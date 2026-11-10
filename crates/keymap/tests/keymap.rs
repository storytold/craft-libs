use std::time::{Duration, Instant};

use craft_keymap::{
    Active, AssignError, Bindable, Binding, Chord, CommandRef, ConflictKind, DropReason, Key, Keymap, Kind, Lookup, MAX_BINDINGS, MAX_COMMANDS,
    MAX_CONFLICTS, Mods, Overrides, Platform, Policy, Problem, Scope, Scopes, Trigger, TypingRule, reserved,
};
use serde_json::json;

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn scope(name: &str) -> Scope {
    Scope::new(name).unwrap()
}

fn b(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn cmd(id: &str) -> CommandRef {
    CommandRef::new(id)
}

fn app(id: &str, keys: &[&str]) -> Bindable {
    keys.iter().fold(Bindable::new(id, id, Scope::app()), |e, k| e.with_default(b(k)))
}

fn in_scope(id: &str, home: &str, keys: &[&str]) -> Bindable {
    keys.iter().fold(Bindable::new(id, id, scope(home)), |e, k| e.with_default(b(k)))
}

fn run(id: &str) -> Lookup {
    Lookup::Run(cmd(id))
}

fn scopes() -> Scopes {
    Scopes::new().with_panel("Timeline").with_panel("Composition").with_share("Timeline", "Composition").with_context("Text")
}

fn press(km: &Keymap, text: &str, active: &Active) -> Lookup {
    km.lookup(&chord(text), false, active)
}

#[test]
fn command_keys_are_canonical_whatever_the_map_order() {
    let c = CommandRef::with_params("window.panel", json!({"z": [1, "a b"], "panel": "layers"}));
    assert_eq!(c.key(), r#"window.panel {"panel":"layers","z":[1,"a b"]}"#);
    assert_eq!(CommandRef::parse_key(&c.key()), Some(c));
    assert_eq!(CommandRef::with_params("edit.copy", json!({})).key(), "edit.copy");
    assert_eq!(CommandRef::parse_key("x {bad"), None);
    assert_eq!(CommandRef::parse_key(&"x".repeat(300)), None);
}

#[test]
fn lookup_picks_the_nearest_scope() {
    let catalog = vec![
        app("edit.copy", &["Cmd+C"]),
        in_scope("timeline.copy", "Timeline", &["Cmd+C"]),
        app("file.bake", &["Cmd+B"]),
        in_scope("text.bold", "Text", &["Cmd+B", "Cmd+I"]),
        in_scope("timeline.loop", "Timeline", &["L"]),
    ];
    let km = Keymap::new(catalog.clone(), scopes(), Platform::Mac);
    let text = ["Text"];
    assert_eq!(press(&km, "Cmd+C", &Active::NONE), run("edit.copy"));
    assert_eq!(press(&km, "Cmd+C", &Active::panel("Timeline")), run("timeline.copy"));
    assert_eq!(press(&km, "Cmd+C", &Active::panel("Composition")), run("timeline.copy"), "sharing panel");
    assert_eq!(press(&km, "Cmd+C", &Active::panel("Effects")), run("edit.copy"));
    assert_eq!(press(&km, "Cmd+B", &Active::panel("Timeline")), run("file.bake"));
    assert_eq!(press(&km, "Cmd+B", &Active { panel: Some("Timeline"), contexts: &text, typing: false }), run("text.bold"));
    assert_eq!(press(&km, "Cmd+I", &Active::NONE), Lookup::None, "inactive context");
    assert_eq!(press(&km, "L", &Active::panel("Effects")), run("timeline.loop"), "fallthrough");
    assert_eq!(km.lookup(&chord("L"), true, &Active::panel("Timeline")), Lookup::None, "double press is its own trigger");

    let strict = Keymap::new(catalog, scopes().with_panel_fallthrough(false), Platform::Mac);
    assert_eq!(press(&strict, "L", &Active::panel("Effects")), Lookup::None);
    assert_eq!(press(&strict, "L", &Active::panel("Timeline")), run("timeline.loop"));
}

#[test]
fn typing_rules_decide_what_fires_in_a_text_field() {
    let catalog = vec![app("tool.brush", &["B"]), app("edit.copy", &["Cmd+C"]), app("help", &["F1"]), app("view.alt", &["Alt+V"])];
    let typing = Active { panel: None, contexts: &[], typing: true };
    let cases = [
        (TypingRule::PlainKeysWait, [false, true, true, true]),
        (TypingRule::CommandKeysOnly, [false, true, true, false]),
        (TypingRule::AllWait, [false, false, false, false]),
    ];
    for (rule, fires) in cases {
        let km = Keymap::new(catalog.clone(), Scopes::new(), Platform::Windows).with_typing_rule(rule);
        for ((key, id), fire) in [("B", "tool.brush"), ("Cmd+C", "edit.copy"), ("F1", "help"), ("Alt+V", "view.alt")].into_iter().zip(fires) {
            let want = if fire { run(id) } else { Lookup::None };
            assert_eq!(press(&km, key, &typing), want, "{rule:?} {key}");
            assert_eq!(press(&km, key, &Active::NONE), run(id), "{rule:?} {key} not typing");
        }
    }
}

#[test]
fn text_editing_chords_go_to_the_text_field_under_every_rule() {
    let catalog = vec![app("edit.undo", &["Cmd+Z"]), app("file.save", &["Cmd+S"]), app("nav.word", &["Alt+Left"]), app("nav.start", &["Cmd+Home"])];
    let typing = Active { typing: true, ..Active::NONE };
    for rule in [TypingRule::PlainKeysWait, TypingRule::CommandKeysOnly] {
        for platform in [Platform::Mac, Platform::Windows] {
            let km = Keymap::new(catalog.clone(), Scopes::new(), platform).with_typing_rule(rule);
            assert_eq!(press(&km, "Cmd+Z", &typing), Lookup::None, "{rule:?} {platform:?}");
            assert_eq!(press(&km, "Cmd+Z", &Active::NONE), run("edit.undo"));
            assert_eq!(press(&km, "Cmd+S", &typing), run("file.save"), "{rule:?} {platform:?}");
        }
    }
    // Word and line keys follow the platform.
    let mac = Keymap::new(catalog.clone(), Scopes::new(), Platform::Mac);
    let windows = Keymap::new(catalog.clone(), Scopes::new(), Platform::Windows);
    assert_eq!(press(&mac, "Alt+Left", &typing), Lookup::None);
    assert_eq!(press(&windows, "Alt+Left", &typing), run("nav.word"));
    assert_eq!(press(&windows, "Ctrl+Home", &typing), Lookup::None);
    assert_eq!(press(&mac, "Cmd+Home", &typing), run("nav.start"));

    // A binding in an active context takes the chord from the field.
    let catalog = vec![app("edit.undo", &["Cmd+Z"]), app("text.undo", &["Text: Cmd+Z"])];
    let km = Keymap::new(catalog, scopes(), Platform::Windows);
    let editing = Active { contexts: &["Text"], typing: true, ..Active::NONE };
    assert_eq!(press(&km, "Cmd+Z", &editing), run("text.undo"));
    assert_eq!(press(&km, "Cmd+Z", &typing), Lookup::None);
    assert!(!TypingRule::PlainKeysWait.fires(&chord("Ctrl+Z"), Platform::Linux));
}

#[test]
fn bindings_of_an_active_context_fire_while_typing_under_every_rule() {
    let catalog = vec![
        app("text.bold", &["Text: Cmd+B", "Text: B"]),
        app("file.bake", &["Cmd+B"]),
        in_scope("timeline.cut", "Timeline", &["Cmd+K"]),
        app("file.save", &["Cmd+S"]),
    ];
    let editing = Active { panel: Some("Timeline"), contexts: &["Text"], typing: true };
    let elsewhere = Active { contexts: &[], ..editing };
    let km = Keymap::new(catalog, scopes(), Platform::Mac).with_typing_rule(TypingRule::AllWait);
    assert_eq!(press(&km, "Cmd+B", &editing), run("text.bold"));
    assert_eq!(press(&km, "B", &editing), run("text.bold"));
    assert_eq!(press(&km, "Cmd+K", &editing), Lookup::None, "panel bindings wait");
    assert_eq!(press(&km, "Cmd+S", &editing), Lookup::None, "app bindings wait");
    assert_eq!(press(&km, "Cmd+B", &elsewhere), Lookup::None, "the app binding waits when the context is not active");
}

#[test]
fn tools_of_one_group_cycle_and_keep_their_shared_key() {
    let tool = |id: &str, group: &str| app(id, &["B"]).with_kind(Kind::Tool { group: Some(group.into()) });
    let mut km =
        Keymap::new(vec![tool("tool.brush", "brush"), tool("tool.pencil", "brush"), tool("tool.mixer", "brush")], Scopes::new(), Platform::Mac);
    assert_eq!(press(&km, "B", &Active::NONE), Lookup::Cycle(vec![cmd("tool.brush"), cmd("tool.pencil"), cmd("tool.mixer")]));
    assert!(km.conflicts().is_empty());

    km.clear(&cmd("tool.mixer"));
    let done = km.assign(&cmd("tool.mixer"), b("B"), Policy::Steal).unwrap();
    assert!(done.taken_from.is_empty());
    assert_eq!(km.bindings(&cmd("tool.brush")), vec![b("B")]);

    let mixed = Keymap::new(vec![tool("tool.brush", "brush"), app("layer.bake", &["B"]), tool("tool.pencil", "brush")], Scopes::new(), Platform::Mac);
    assert_eq!(press(&mixed, "B", &Active::NONE), run("tool.brush"));
}

#[test]
fn steal_keep_and_refuse() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), app("edit.duplicate", &["Cmd+D"]), in_scope("timeline.copy", "Timeline", &[])];
    let mut km = Keymap::new(catalog, scopes(), Platform::Mac);
    let dup = cmd("edit.duplicate");

    let err = km.assign(&dup, b("Cmd+C"), Policy::Refuse).unwrap_err();
    let AssignError::Taken(holders) = err else { panic!("{err:?}") };
    assert_eq!(holders[0].command, cmd("edit.copy"));
    assert_eq!(km.bindings(&dup), vec![b("Cmd+D")]);

    let done = km.assign(&dup, b("Cmd+C"), Policy::Keep).unwrap();
    assert_eq!(done.clashes.len(), 1);
    assert_eq!(km.bindings(&cmd("edit.copy")), vec![b("Cmd+C")]);
    assert_eq!(km.bindings(&dup), vec![b("Cmd+D"), b("Cmd+C")]);

    km.reset_all();
    let done = km.assign(&dup, b("Cmd+C"), Policy::Steal).unwrap();
    assert_eq!(done.taken_from[0].command, cmd("edit.copy"));
    assert!(km.bindings(&cmd("edit.copy")).is_empty());
    assert_eq!(km.overrides().get("edit.copy"), Some(&[][..]));
    assert_eq!(press(&km, "Cmd+C", &Active::NONE), run("edit.duplicate"));

    // A panel binding shadows the application one; nothing is taken.
    let done = km.assign(&cmd("timeline.copy"), b("Cmd+D"), Policy::Refuse).unwrap();
    assert!(done.taken_from.is_empty() && done.clashes.is_empty());
    assert_eq!(km.assign(&cmd("nope"), b("Cmd+D"), Policy::Steal), Err(AssignError::UnknownCommand));
}

#[test]
fn twins_keep_their_shared_default_key() {
    let mut km = Keymap::new(vec![app("file.close", &["Cmd+W"]), app("sequence.close", &["Cmd+W"])], Scopes::new(), Platform::Mac);
    km.clear(&cmd("sequence.close"));
    let done = km.assign(&cmd("sequence.close"), b("Cmd+W"), Policy::Refuse).unwrap();
    assert!(done.taken_from.is_empty());
    assert_eq!(km.bindings(&cmd("file.close")), vec![b("Cmd+W")]);
    assert!(km.overrides().is_empty(), "back to the defaults");
}

#[test]
fn clearing_a_command_frees_its_panel_copies() {
    // filmcraft#1333: New Bin holds Cmd+B in the application and in the Project panel.
    let catalog = vec![app("project.newBin", &["Cmd+B", "Project: Cmd+B"]), app("clip.bold", &[])];
    let mut km = Keymap::new(catalog, Scopes::new().with_panel("Project"), Platform::Mac);
    assert_eq!(press(&km, "Cmd+B", &Active::panel("Project")), run("project.newBin"));
    assert!(km.clear(&cmd("project.newBin")));
    assert!(km.bindings(&cmd("project.newBin")).is_empty());
    assert_eq!(press(&km, "Cmd+B", &Active::panel("Project")), Lookup::None);
    assert!(km.assign(&cmd("clip.bold"), b("Project: Cmd+B"), Policy::Refuse).is_ok());
}

#[test]
fn reserved_chords_are_refused() {
    let catalog = vec![app("view.next", &[])];
    let mut mac = Keymap::new(catalog.clone(), Scopes::new(), Platform::Mac).with_reserved(reserved::system(Platform::Mac));
    let err = mac.assign(&cmd("view.next"), b("Cmd+Tab"), Policy::Steal).unwrap_err();
    assert_eq!(err, AssignError::Reserved(Trigger::parse("Cmd+Tab").unwrap()));

    let mut win = Keymap::new(catalog.clone(), Scopes::new(), Platform::Windows).with_reserved(reserved::system(Platform::Windows));
    assert!(matches!(win.assign(&cmd("view.next"), b("Ctrl+Alt+Delete"), Policy::Steal), Err(AssignError::Reserved(_))));
    assert!(reserved::system(Platform::Linux).contains(&chord("Cmd+Alt+F12")));

    let mut o = Overrides::new();
    o.insert("view.next", vec!["Alt+F4".into()]);
    let linux = Keymap::new(catalog, Scopes::new(), Platform::Linux).with_reserved(reserved::system(Platform::Linux)).with_overrides(o);
    assert!(linux.bindings(&cmd("view.next")).is_empty());
    assert_eq!(linux.problems(), vec![Problem::Reserved { key: "view.next".into(), text: "Alt+F4".into() }]);
}

#[test]
fn conflicts_tell_clashes_from_shadows() {
    let catalog = vec![
        app("edit.copy", &["Cmd+C"]),
        app("color.copy", &["Cmd+C"]),
        in_scope("timeline.copy", "Timeline", &["Cmd+C"]),
        in_scope("text.cut", "Text", &["Cmd+X"]),
        app("edit.cut", &["Cmd+X"]),
        app("edit.find", &["Cmd+F"]),
        in_scope("tab.find", "Effects", &["Cmd+F"]),
        in_scope("render.find", "Render", &["Cmd+R"]),
        in_scope("effects.render", "Effects", &["Cmd+R"]),
    ];
    let mut km = Keymap::new(catalog, scopes(), Platform::Mac);
    let summary = |km: &Keymap| -> Vec<(String, String, String, ConflictKind, bool)> {
        km.conflicts().into_iter().map(|c| (c.chord.to_string(), c.a.0.id, c.b.0.id, c.kind, c.in_defaults)).collect()
    };
    let s = |a: &str, x: &str, y: &str, k, d| (a.to_string(), x.to_string(), y.to_string(), k, d);
    let mut want = vec![
        s("Cmd+C", "edit.copy", "color.copy", ConflictKind::Clash, true),
        s("Cmd+C", "edit.copy", "timeline.copy", ConflictKind::Shadow, true),
        s("Cmd+C", "color.copy", "timeline.copy", ConflictKind::Shadow, true),
        s("Cmd+X", "text.cut", "edit.cut", ConflictKind::Shadow, true),
        s("Cmd+F", "edit.find", "tab.find", ConflictKind::Shadow, true),
        // Fallthrough: in a third panel the two tie.
        s("Cmd+R", "render.find", "effects.render", ConflictKind::Shadow, true),
    ];
    let sort = |mut v: Vec<(String, String, String, ConflictKind, bool)>| {
        v.sort();
        v
    };
    assert_eq!(sort(summary(&km)), sort(want.clone()));

    km.assign(&cmd("edit.find"), b("Cmd+D"), Policy::Steal).unwrap();
    km.assign(&cmd("edit.cut"), b("Cmd+D"), Policy::Keep).unwrap();
    want.push(s("Cmd+D", "edit.cut", "edit.find", ConflictKind::Clash, false));
    assert_eq!(sort(summary(&km)), sort(want.clone()));

    let order: Vec<_> = summary(&km).into_iter().map(|(c, a, b, ..)| (c, a, b)).collect();
    let chords: Vec<Trigger> = km.conflicts().iter().map(|c| c.chord).collect();
    assert!(chords.is_sorted(), "{order:?}");
    let c = |x: &str, y: &str| order.iter().position(|(_, a, b)| a == x && b == y);
    assert!(c("edit.copy", "color.copy") < c("edit.copy", "timeline.copy"), "{order:?}");
    assert!(c("edit.copy", "timeline.copy") < c("color.copy", "timeline.copy"), "{order:?}");

    // One command's conflicts, with that command as `a`.
    let with = |id: &str| -> Vec<(String, String, ConflictKind)> {
        km.conflicts_with(&cmd(id)).into_iter().map(|c| (c.chord.to_string(), c.b.0.id, c.kind)).collect()
    };
    let w = |chord: &str, other: &str, kind| (chord.to_string(), other.to_string(), kind);
    assert_eq!(with("edit.copy"), [w("Cmd+C", "color.copy", ConflictKind::Clash), w("Cmd+C", "timeline.copy", ConflictKind::Shadow)]);
    assert_eq!(with("edit.cut"), [w("Cmd+X", "text.cut", ConflictKind::Shadow), w("Cmd+D", "edit.find", ConflictKind::Clash)]);
    assert!(with("nope").is_empty());
    for entry in km.catalog() {
        let id = entry.command.id.as_str();
        let mut direct: Vec<_> =
            km.conflicts_with(&entry.command).into_iter().map(|c| (c.chord, c.a.0.id, c.b.0.id, c.kind, c.in_defaults)).collect();
        let mut filtered: Vec<_> = km
            .conflicts()
            .into_iter()
            .filter(|c| c.a.0.id == id || c.b.0.id == id)
            .map(|c| {
                if c.a.0.id == id {
                    (c.chord, c.a.0.id, c.b.0.id, c.kind, c.in_defaults)
                } else {
                    (c.chord, c.b.0.id, c.a.0.id, c.kind, c.in_defaults)
                }
            })
            .collect();
        direct.sort();
        filtered.sort();
        assert_eq!(direct, filtered, "{id}");
    }

    let strict = Keymap::new(km.catalog().to_vec(), scopes().with_panel_fallthrough(false), Platform::Mac);
    assert!(!strict.conflicts().iter().any(|c| c.chord.to_string() == "Cmd+R"));
}

/// `commands` commands, each with 16 bindings on one chord in 16 panels.
fn crowded(commands: usize, scopes: Scopes, kind: Kind) -> (Keymap, Overrides) {
    let catalog: Vec<Bindable> = (0..commands).map(|n| app(&format!("c.{n}"), &[]).with_kind(kind.clone())).collect();
    let mut o = Overrides::new();
    for n in 0..commands {
        assert!(o.insert(format!("c.{n}"), (1..=MAX_BINDINGS).map(|p| format!("P{p}: K")).collect()));
    }
    (Keymap::new(catalog, scopes, Platform::Mac), o)
}

fn panels(n: usize) -> Scopes {
    (1..=n).fold(Scopes::new(), |s, p| s.with_panel(&format!("P{p}")))
}

#[test]
fn crowded_chords_are_capped() {
    // Panels the app never registered are not bound.
    let (km, o) = crowded(2, Scopes::new(), Kind::Command);
    let km = km.with_overrides(o);
    assert!(km.conflicts().is_empty());
    assert!(km.bindings(&cmd("c.0")).is_empty());
    assert_eq!(km.overrides().get("c.0").map(<[String]>::len), Some(MAX_BINDINGS), "kept verbatim");
    assert!(km.problems().contains(&Problem::UnknownScope { key: "c.0".into(), text: "P1: K".into() }));

    // Registered panels that never share: 32 commands make 16 × 496 pairs, capped.
    let (km, o) = crowded(32, panels(MAX_BINDINGS).with_panel_fallthrough(false), Kind::Command);
    assert_eq!(km.with_overrides(o).conflicts().len(), MAX_CONFLICTS);

    // Tools of one group: no conflicts.
    let (km, o) = crowded(32, panels(MAX_BINDINGS), Kind::Tool { group: None });
    assert!(km.with_overrides(o).conflicts().is_empty());
}

#[test]
#[ignore = "worst-case timing; run with --ignored"]
fn crowded_chords_stay_fast() {
    let limit = Duration::from_secs(8);

    // Panels the app never registered are not bound.
    let (km, o) = crowded(MAX_COMMANDS, Scopes::new(), Kind::Command);
    let km = km.with_overrides(o);
    let t = Instant::now();
    assert!(km.conflicts().is_empty());
    assert!(t.elapsed() < limit, "{:?}", t.elapsed());
    assert!(km.bindings(&cmd("c.0")).is_empty());
    assert_eq!(km.overrides().get("c.0").map(<[String]>::len), Some(MAX_BINDINGS), "kept verbatim");
    assert!(km.problems().contains(&Problem::UnknownScope { key: "c.0".into(), text: "P1: K".into() }));

    // Registered panels that never share: many conflicts, capped.
    let (km, o) = crowded(MAX_COMMANDS, panels(MAX_BINDINGS).with_panel_fallthrough(false), Kind::Command);
    let km = km.with_overrides(o);
    let t = Instant::now();
    assert_eq!(km.conflicts().len(), MAX_CONFLICTS);
    assert!(t.elapsed() < limit, "{:?}", t.elapsed());

    // Tools of one group: no conflicts, and no pair is compared.
    let (km, o) = crowded(MAX_COMMANDS, panels(MAX_BINDINGS), Kind::Tool { group: None });
    let km = km.with_overrides(o);
    let t = Instant::now();
    assert!(km.conflicts().is_empty());
    assert!(t.elapsed() < limit, "{:?}", t.elapsed());
}

#[test]
fn assigning_many_commands_stays_fast() {
    let catalog: Vec<Bindable> = (0..MAX_COMMANDS).map(|n| app(&format!("c.{n}"), &[])).collect();
    let mut km = Keymap::new(catalog, panels(MAX_BINDINGS), Platform::Mac);
    let list: Vec<Binding> = (1..=MAX_BINDINGS).map(|p| b(&format!("P{p}: K"))).collect();
    let t = Instant::now();
    for n in 0..MAX_COMMANDS {
        let done = km.set_bindings(&cmd(&format!("c.{n}")), list.clone(), Policy::Steal).unwrap();
        assert_eq!(done.taken_from.len(), if n == 0 { 0 } else { MAX_BINDINGS });
    }
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    assert_eq!(km.bindings(&cmd(&format!("c.{}", MAX_COMMANDS - 1))), list);
    assert!(km.bindings(&cmd("c.0")).is_empty());
    assert_eq!(press(&km, "K", &Active::panel("P3")), run(&format!("c.{}", MAX_COMMANDS - 1)));
}

#[test]
fn binding_text_round_trips_for_every_key() {
    let panel = scope("Timeline");
    for &key in Key::ALL {
        for mods in [Mods::NONE, Mods { cmd: true, ctrl: true, alt: true, shift: true }] {
            let chord = Chord::new(mods, key);
            for trigger in [Trigger::single(chord), Trigger::double(chord)] {
                for binding in [Binding::new(trigger), Binding::in_scope(trigger, panel.clone())] {
                    let text = binding.to_string();
                    assert_eq!(Binding::parse(&text).as_ref(), Ok(&binding), "{text}");
                }
            }
        }
    }
    assert_eq!(Binding::parse(": :"), Ok(Binding::new(Trigger::double(Chord::plain(Key::Colon)))));
}

#[test]
fn scope_names_with_outer_whitespace_are_refused() {
    for name in [" Timeline", "Timeline ", "\u{a0}Timeline", ""] {
        assert_eq!(Scope::new(name), None, "{name:?}");
    }
    assert_eq!(Scope::new("Time line").map(|s| s.to_string()).as_deref(), Some("Time line"));
    assert_eq!(Binding::parse(" Timeline :  K").map(|b| b.scope), Ok(Some(scope("Timeline"))));
}

#[test]
fn edits_are_refused_when_the_overrides_are_full() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), app("edit.duplicate", &["Cmd+D"]), app("edit.cut", &[])];
    let mut o = Overrides::new();
    o.insert("edit.cut", vec!["Cmd+X".into()]);
    for n in 1..MAX_COMMANDS {
        o.insert(format!("junk.{n}"), vec![]);
    }
    assert_eq!(o.len(), MAX_COMMANDS);
    let mut km = Keymap::new(catalog, Scopes::new(), Platform::Mac).with_overrides(o.clone());
    let (copy, dup, cut) = (cmd("edit.copy"), cmd("edit.duplicate"), cmd("edit.cut"));

    assert_eq!(km.assign(&dup, b("Cmd+K"), Policy::Steal), Err(AssignError::Full));
    assert_eq!(km.set_bindings(&dup, vec![], Policy::Steal), Err(AssignError::Full));
    assert!(!km.clear(&copy));
    assert!(!km.remove(&copy, &b("Cmd+C")));
    // Taking Cmd+C from edit.copy would need an entry for it too.
    assert_eq!(km.assign(&cut, b("Cmd+C"), Policy::Steal), Err(AssignError::Full));
    assert_eq!(km.bindings(&copy), vec![b("Cmd+C")]);
    assert_eq!(km.bindings(&cut), vec![b("Cmd+X")]);
    assert_eq!(km.overrides(), &o);

    // Commands that have an entry still change, and back to the defaults frees one.
    assert!(km.assign(&cut, b("Cmd+Shift+X"), Policy::Steal).is_ok());
    assert!(km.reset(&cut));
    assert!(km.assign(&dup, b("Cmd+K"), Policy::Steal).is_ok());
}

#[test]
fn bindings_in_unknown_scopes_are_refused() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), in_scope("timeline.mark", "Timeline", &["M"])];
    let mut km = Keymap::new(catalog, Scopes::new().with_context("Text"), Platform::Mac);
    let copy = cmd("edit.copy");
    assert_eq!(km.assign(&copy, b("Nowhere: Cmd+K"), Policy::Steal), Err(AssignError::UnknownScope(scope("Nowhere"))));
    assert!(km.assign(&copy, b("Timeline: Cmd+K"), Policy::Steal).is_ok(), "a catalog scope");
    assert!(km.assign(&copy, b("Text: Cmd+K"), Policy::Steal).is_ok(), "a registered scope");

    let mut o = Overrides::new();
    o.insert("edit.copy", vec!["Nowhere: Cmd+K".into(), "Cmd+J".into()]);
    km.set_overrides(o);
    assert_eq!(km.bindings(&copy), vec![b("Cmd+J")]);
    assert_eq!(km.problems(), vec![Problem::UnknownScope { key: "edit.copy".into(), text: "Nowhere: Cmd+K".into() }]);
}

#[test]
fn unreadable_entries_keep_the_defaults() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), app("edit.paste", &["Cmd+V"])];
    let mut o = Overrides::new();
    o.insert("edit.copy", vec!["Cmd+Nope".into()]);
    o.insert("edit.paste", vec![]);
    let mut km = Keymap::new(catalog, Scopes::new(), Platform::Mac).with_overrides(o);
    assert_eq!(km.bindings(&cmd("edit.copy")), vec![b("Cmd+C")]);
    assert!(km.bindings(&cmd("edit.paste")).is_empty());
    assert_eq!(km.overrides().get("edit.copy"), Some(&["Cmd+Nope".to_string()][..]), "verbatim");
    assert!(km.reset(&cmd("edit.copy")));
    assert_eq!(km.overrides().get("edit.copy"), None);
}

#[test]
fn entries_with_nothing_usable_keep_the_defaults() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), app("edit.cut", &["Cmd+X"]), app("edit.paste", &["Cmd+V"]), app("edit.undo", &["Cmd+Z"])];
    let mut o = Overrides::new();
    o.insert("edit.copy", vec!["Nowhere: Cmd+K".into()]);
    o.insert("edit.cut", vec!["Alt+F4".into()]);
    o.insert("edit.paste", vec!["Nowhere: Cmd+K".into(), "Alt+F4".into(), "Cmd+J".into()]);
    o.insert("edit.undo", vec![]);
    let km = Keymap::new(catalog, Scopes::new(), Platform::Linux).with_reserved(reserved::system(Platform::Linux)).with_overrides(o);
    assert_eq!(km.bindings(&cmd("edit.copy")), vec![b("Cmd+C")], "only an unknown scope");
    assert_eq!(km.bindings(&cmd("edit.cut")), vec![b("Cmd+X")], "only a reserved chord");
    assert_eq!(km.bindings(&cmd("edit.paste")), vec![b("Cmd+J")]);
    assert!(km.bindings(&cmd("edit.undo")).is_empty());
}

#[test]
fn overrides_hold_only_what_differs_from_the_defaults() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), in_scope("timeline.mark", "Timeline", &["M"])];
    let mut km = Keymap::new(catalog, scopes(), Platform::Mac);
    let copy = cmd("edit.copy");
    km.assign(&copy, b("Timeline: Cmd+K"), Policy::Steal).unwrap();
    assert_eq!(km.overrides().get("edit.copy"), Some(&["Cmd+C".to_string(), "Timeline: Cmd+K".to_string()][..]));
    assert_eq!(km.bindings(&copy)[1], Binding::in_scope(Trigger::parse("Cmd+K").unwrap(), scope("Timeline")));
    assert!(km.remove(&copy, &b("Timeline: Cmd+K")));
    assert!(km.overrides().is_empty());

    km.set_bindings(&cmd("timeline.mark"), vec![b("Timeline: M")], Policy::Steal).unwrap();
    assert!(km.overrides().is_empty(), "the home scope written out is the same binding");
    km.set_bindings(&copy, vec![b("Cmd+Shift+C")], Policy::Steal).unwrap();
    assert!(km.reset(&copy));
    assert!(km.overrides().is_empty());
}

#[test]
fn edits_keep_the_texts_this_build_cannot_use() {
    let catalog = vec![app("file.quit", &["Cmd+Q"]), app("file.close", &["Cmd+W"]), app("file.save", &["Cmd+S"])];
    let mut o = Overrides::new();
    o.insert("file.quit", vec!["Cmd+Q".into(), "Cmd+FutureKey".into(), "Nowhere: Cmd+J".into()]);
    let mut km = Keymap::new(catalog.clone(), scopes(), Platform::Mac).with_overrides(o);
    let quit = cmd("file.quit");
    let texts = |km: &Keymap| km.overrides().get("file.quit").map(<[String]>::to_vec);
    let strings = |t: &[&str]| Some(t.iter().map(|s| s.to_string()).collect::<Vec<_>>());

    km.assign(&quit, b("Cmd+W"), Policy::Steal).unwrap();
    assert_eq!(texts(&km), strings(&["Cmd+Q", "Cmd+W", "Cmd+FutureKey", "Nowhere: Cmd+J"]));
    km.assign(&cmd("file.save"), b("Cmd+Q"), Policy::Steal).unwrap();
    assert_eq!(texts(&km), strings(&["Cmd+W", "Cmd+FutureKey", "Nowhere: Cmd+J"]), "taken by a Steal");
    let reloaded = Keymap::new(catalog, scopes(), Platform::Mac).with_overrides(km.overrides().clone());
    assert_eq!(reloaded.bindings(&quit), vec![b("Cmd+W")]);

    // An entry of only such texts would load as the defaults, so a cleared command drops them.
    assert!(km.clear(&quit));
    assert_eq!(texts(&km), strings(&[]));
    assert!(km.reset(&quit));
    assert_eq!(texts(&km), None);
}

#[test]
fn an_override_equal_to_the_defaults_on_this_platform_is_not_stored() {
    let mut km = Keymap::new(vec![app("file.save", &["Cmd+S"])], Scopes::new(), Platform::Windows);
    km.set_bindings(&cmd("file.save"), vec![b("Ctrl+S")], Policy::Steal).unwrap();
    assert!(km.overrides().is_empty());
    assert_eq!(press(&km, "Ctrl+S", &Active::NONE), run("file.save"));
}

#[test]
fn an_empty_text_in_the_map_form_removes_the_shortcut() {
    let catalog = vec![app("edit.copy", &["Cmd+C"]), app("edit.paste", &["Cmd+V"]), app("edit.cut", &["Cmd+X"])];
    let o: Overrides = serde_json::from_str(r#"{"edit.copy": "", "edit.paste": [""], "edit.cut": "Cmd+K"}"#).unwrap();
    assert!(o.load_notes().is_empty(), "{:?}", o.load_notes());
    let km = Keymap::new(catalog.clone(), Scopes::new(), Platform::Mac).with_overrides(o);
    assert!(km.bindings(&cmd("edit.copy")).is_empty(), "a removed default stays removed");
    assert!(km.bindings(&cmd("edit.paste")).is_empty());
    assert_eq!(km.bindings(&cmd("edit.cut")), vec![b("Cmd+K")]);
    assert!(km.problems().is_empty());

    let saved = serde_json::to_string(km.overrides()).unwrap();
    let again = Keymap::new(catalog, Scopes::new(), Platform::Mac).with_overrides(serde_json::from_str(&saved).unwrap());
    assert!(again.bindings(&cmd("edit.copy")).is_empty(), "{saved}");
}

#[test]
fn overrides_write_the_map_form_unless_a_command_has_several_keys() {
    let mut o = Overrides::new();
    o.insert("edit.copy", vec!["Cmd+K".into()]);
    o.insert("edit.paste", vec![]);
    let text = serde_json::to_string(&o).unwrap();
    assert_eq!(text, r#"{"edit.copy":"Cmd+K","edit.paste":""}"#, "older builds read one text per command");
    assert_eq!(serde_json::from_str::<Overrides>(&text).unwrap(), o);

    o.insert("edit.cut", vec!["Cmd+X".into(), "Shift+Delete".into()]);
    let text = serde_json::to_string(&o).unwrap();
    assert!(text.starts_with(r#"[{"command":"edit.copy","keys":["Cmd+K"]},"#), "{text}");
    assert_eq!(serde_json::from_str::<Overrides>(&text).unwrap(), o);

    // A blank text is kept verbatim (it loads as the defaults), which the map form cannot say.
    let mut blank = Overrides::new();
    blank.insert("edit.copy", vec![" ".into()]);
    let text = serde_json::to_string(&blank).unwrap();
    assert!(text.starts_with('['), "{text}");
    assert_eq!(serde_json::from_str::<Overrides>(&text).unwrap(), blank);

    #[derive(serde::Serialize)]
    struct Prefs {
        #[serde(with = "craft_keymap::keymap::overrides_list")]
        shortcuts: Overrides,
    }
    let mut single = Overrides::new();
    single.insert("edit.copy", vec!["Cmd+K".into()]);
    let text = serde_json::to_string(&Prefs { shortcuts: single }).unwrap();
    assert_eq!(text, r#"{"shortcuts":[{"command":"edit.copy","keys":["Cmd+K"]}]}"#);
}

#[test]
fn ctrl_folds_into_cmd_off_macos() {
    let catalog = vec![app("edit.cut", &["Cmd+X"]), app("mac.cut", &["Ctrl+X"])];
    let win = Keymap::new(catalog.clone(), Scopes::new(), Platform::Windows);
    let mac = Keymap::new(catalog, Scopes::new(), Platform::Mac);
    let conflicts = win.conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!((conflicts[0].kind, conflicts[0].chord.to_string()), (ConflictKind::Clash, "Cmd+X".to_string()));
    assert!(mac.conflicts().is_empty());
    assert_eq!(press(&win, "Ctrl+X", &Active::NONE), run("edit.cut"));
    assert_eq!(press(&mac, "Ctrl+X", &Active::NONE), run("mac.cut"));
}

#[test]
fn hostile_overrides_load_without_panic_and_report_problems() {
    let mut doc = serde_json::Map::new();
    for i in 0..(MAX_COMMANDS + 100) {
        doc.insert(format!("junk.{i}"), json!(["Cmd+Z"]));
    }
    doc.insert("edit.copy".into(), json!(["Cmd+Nope", "x".repeat(10_000), 7, "Shift+C"]));
    doc.insert("a".into(), json!(["Shift+A"]));
    doc.insert("é".repeat(300), json!([]));
    doc.insert("b".into(), json!({"deep": [[[[1]]]]}));
    let o: Overrides = serde_json::from_value(serde_json::Value::Object(doc)).unwrap();
    assert!(o.len() <= MAX_COMMANDS);

    let km = Keymap::new(vec![app("edit.copy", &["Cmd+C"]), app("a", &[])], Scopes::new(), Platform::Linux).with_overrides(o.clone());
    assert_eq!(km.bindings(&cmd("edit.copy")), vec![b("Shift+C")]);
    assert_eq!(km.overrides().get("edit.copy"), Some(&["Cmd+Nope".to_string(), "Shift+C".to_string()][..]), "verbatim");
    let problems = km.problems();
    assert!(problems.iter().any(|p| matches!(p, Problem::TooManyCommands { .. })));
    assert!(problems.contains(&Problem::Dropped { key: "edit.copy".into(), reason: DropReason::TextTooLong }));
    assert!(problems.contains(&Problem::Dropped { key: "edit.copy".into(), reason: DropReason::NotText }));
    assert!(problems.iter().any(|p| matches!(p, Problem::Dropped { reason: DropReason::KeyTooLong, .. })));
    assert!(problems.iter().any(|p| matches!(p, Problem::BadBinding { text, .. } if text == "Cmd+Nope")));
    assert!(problems.iter().any(|p| matches!(p, Problem::UnknownCommand { key } if key == "junk.0")));

    let text = serde_json::to_string(&o).unwrap();
    let again: Overrides = serde_json::from_str(&text).unwrap();
    assert_eq!(again, o);
    assert!(text.starts_with(r#"[{"command":"a","keys":["Shift+A"]},"#), "{}", &text[..80]);
    assert!(serde_json::from_str::<Overrides>(r#"{"x": {"y": 1}, "z": true, "w": null, "v": "Cmd+V"}"#).is_ok());
    let items: Overrides =
        serde_json::from_str(r#"[{"command": "v", "keys": "Cmd+V"}, {"command": "x"}, {"keys": []}, 3, [], {"command": "y", "keys": [1]}]"#).unwrap();
    assert_eq!(items.iter().collect::<Vec<_>>(), [("v", &["Cmd+V".to_string()][..]), ("y", &[][..])]);
    assert_eq!(items.load_notes().len(), 5);

    let twice: Overrides = serde_json::from_str(r#"[{"command": "v", "keys": ["Cmd+V"]}, {"command": "v", "keys": []}]"#).unwrap();
    assert_eq!(twice.get("v"), Some(&["Cmd+V".to_string()][..]), "the first entry stays");
    assert_eq!(twice.load_notes(), [Problem::DuplicateKey { key: "v".into() }]);
    for text in [r#""x""#, "5", "true", "null"] {
        let o: Overrides = serde_json::from_str(text).unwrap();
        assert!(o.is_empty(), "{text}");
        assert_eq!(o.load_notes(), [Problem::Dropped { key: String::new(), reason: DropReason::NotAList }], "{text}");
    }
}

#[test]
fn reset_removes_hand_written_variants_of_a_key() {
    let layers = CommandRef::with_params("window.panel", json!({"panel": "layers"}));
    let catalog = vec![Bindable::new(layers.clone(), "Layers", Scope::app()).with_default(b("F7"))];
    let load = || {
        let mut o = Overrides::new();
        o.insert(r#"window.panel {"panel":"layers"}"#, vec!["Cmd+L".into()]);
        o.insert(r#"window.panel { "panel": "layers" }"#, vec!["Cmd+K".into()]);
        Keymap::new(catalog.clone(), Scopes::new(), Platform::Mac).with_overrides(o)
    };
    let mut km = load();
    assert_eq!(km.bindings(&layers), vec![b("Cmd+L")], "the canonical entry wins");
    km.reset(&layers);
    assert!(km.overrides().is_empty(), "{:?}", km.overrides());

    // Setting the defaults drops the entries too.
    let mut km = load();
    km.set_bindings(&layers, vec![b("F7")], Policy::Steal).unwrap();
    assert!(km.overrides().is_empty(), "{:?}", km.overrides());
    let again = Keymap::new(catalog.clone(), Scopes::new(), Platform::Mac).with_overrides(km.overrides().clone());
    assert_eq!(again.bindings(&layers), vec![b("F7")]);
}
