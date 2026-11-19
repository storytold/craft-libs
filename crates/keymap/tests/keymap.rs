//! Integration tests of craft-keymap: keymap.

use craft_keymap::{
    Active, Binding, Chord, CommandDef, CommandRef, DropReason, Key, KeyInput, Keymap, Layout, Lookup, MAX_COMMANDS, Matcher, Mods, Overrides,
    Platform, Policy, Press, Problem, Scope, Scopes,
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

fn app(id: &str, keys: &[&str]) -> CommandDef {
    keys.iter().fold(CommandDef::new(id, id, Scope::app()), |e, k| e.with_default(b(k)))
}

fn in_scope(id: &str, home: &str, keys: &[&str]) -> CommandDef {
    keys.iter().fold(CommandDef::new(id, id, scope(home)), |e, k| e.with_default(b(k)))
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

/// A command shows a chord only where the lookup runs it: of two application commands on one chord the catalog-first
/// one shows it and is its holder, the other shows its next key; a panel or context binding shows where it applies,
/// and a panel-only chord's holder is the panel's command.
#[test]
fn a_command_shows_only_the_chords_the_lookup_runs_it_for() {
    let catalog = vec![
        app("layer.hide", &["Cmd+,"]),
        app("edit.settings", &["Cmd+,", "Cmd+Alt+,"]),
        app("view.extras", &["Cmd+H"]),
        in_scope("timeline.copy", "Timeline", &["Cmd+C"]),
        app("edit.copy", &["Cmd+C"]),
        in_scope("text.bold", "Text", &["Cmd+B"]),
        in_scope("timeline.loop", "Timeline", &["Cmd+L"]),
    ];
    let km = Keymap::new(catalog, scopes().with_panel_fallthrough(false), Platform::Mac);
    assert_eq!(km.shown_chord(&cmd("layer.hide")), Some(chord("Cmd+,")));
    assert_eq!(km.shown_chord(&cmd("edit.settings")), Some(chord("Cmd+Alt+,")), "the shadowed key is not shown");
    assert_eq!(km.holder(&chord("Cmd+,")), Some(cmd("layer.hide")));
    assert_eq!(km.shown_chord(&cmd("timeline.copy")), Some(chord("Cmd+C")), "in its panel the panel's command runs");
    assert_eq!(km.shown_chord(&cmd("edit.copy")), Some(chord("Cmd+C")));
    assert_eq!(km.holder(&chord("Cmd+C")), Some(cmd("edit.copy")), "with no panel focused");
    assert_eq!(km.shown_chord(&cmd("text.bold")), Some(chord("Cmd+B")));
    assert_eq!(km.holder(&chord("Cmd+L")), Some(cmd("timeline.loop")), "a panel-only chord");
    assert_eq!(km.holder(&chord("Cmd+K")), None);

    let alone = Keymap::new(vec![app("layer.hide", &["Cmd+,"]), app("edit.settings", &["Cmd+,"])], Scopes::new(), Platform::Mac);
    assert_eq!(alone.shown_chord(&cmd("edit.settings")), None, "every key shadowed");
}

/// The shown chords are worked out when the bindings change, not per call, and follow every change: a key freed by one
/// command shows on the command it no longer shadows, and a stolen key leaves the command it was taken from.
#[test]
fn shown_chords_follow_binding_changes_and_reads_work_out_nothing() {
    let catalog = vec![app("layer.hide", &["Cmd+,"]), app("edit.settings", &["Cmd+,", "Cmd+Alt+,"]), app("view.extras", &["Cmd+H"])];
    let mut km = Keymap::new(catalog, Scopes::new(), Platform::Mac);
    let built = km.shown_builds();
    for _ in 0..100 {
        assert_eq!(km.shown_chord(&cmd("edit.settings")), Some(chord("Cmd+Alt+,")));
    }
    assert_eq!(km.shown_builds(), built, "reads work out no chord");

    assert!(km.clear(&cmd("layer.hide")));
    assert_eq!(km.shown_chord(&cmd("edit.settings")), Some(chord("Cmd+,")), "the freed key shows on the command it no longer shadows");
    km.assign(&cmd("view.extras"), b("Cmd+,"), Policy::Steal).expect("steal");
    assert_eq!(km.shown_chord(&cmd("view.extras")), Some(chord("Cmd+H")));
    assert_eq!(km.shown_chord(&cmd("edit.settings")), Some(chord("Cmd+Alt+,")), "a stolen key leaves the command it was taken from");
    km.reset_all();
    assert_eq!(km.shown_chord(&cmd("layer.hide")), Some(chord("Cmd+,")));
    assert_eq!(km.shown_chord(&cmd("edit.settings")), Some(chord("Cmd+Alt+,")));
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

/// A command that opts in to the Alt variant runs on its bound chord with Alt added, wherever its key is rebound to,
/// unless a binding owns the Alt chord itself.
#[test]
fn the_alt_variant_follows_the_commands_key_and_yields_to_a_bound_alt_chord() {
    let alt = |key: Key, cmd: bool, shift: bool| KeyInput::new(Mods { cmd, ctrl: false, alt: true, shift }).with_physical(key);
    let hit = |km: &Keymap, input: &KeyInput| {
        let press = Press { input, repeat: false, now: 0.0, active: &Active::NONE, outer: &[] };
        Matcher::new().press(&Layout::new(), km, &press)
    };
    let new_layer = || app("layer.new.layer", &["F2"]).with_alt_variant();
    let km = Keymap::new(vec![new_layer()], Scopes::new(), Platform::Mac);
    let found = hit(&km, &alt(Key::F2, false, false)).expect("Alt+F2 runs the command bound to F2");
    assert_eq!((found.command, found.alt_variant), (cmd("layer.new.layer"), true));

    let owner = app("layer.new.copy", &["Alt+F2"]);
    let km = Keymap::new(vec![new_layer(), owner], Scopes::new(), Platform::Mac);
    let found = hit(&km, &alt(Key::F2, false, false)).expect("Alt+F2 is bound");
    assert_eq!((found.command, found.alt_variant), (cmd("layer.new.copy"), false), "the bound Alt chord runs its own command");

    let km = Keymap::new(vec![app("layer.new.layer", &["F2"])], Scopes::new(), Platform::Mac);
    assert!(hit(&km, &alt(Key::F2, false, false)).is_none(), "a command that did not opt in has no Alt variant");
}

/// Tools that share a chord: within one group a press cycles through them, so each shows the chord; across groups the
/// first in catalog order runs, so only it shows the chord. `shown_chord` decides on catalog positions (#15493) and
/// must answer as the lookup does.
#[test]
fn tools_sharing_a_chord_show_it_as_the_lookup_runs_them() {
    use craft_keymap::CommandKind;
    let tool = |id: &str, group: &str, keys: &[&str]| app(id, keys).with_kind(CommandKind::Tool { group: Some(group.into()) });
    let catalog = vec![
        tool("tool.marquee", "select", &["M"]),
        tool("tool.ellipse", "select", &["M"]),
        tool("tool.move", "move", &["V"]),
        tool("tool.artboard", "board", &["V"]),
    ];
    let km = Keymap::new(catalog, scopes(), Platform::Mac);
    assert_eq!(press(&km, "M", &Active::NONE), Lookup::Cycle(vec![cmd("tool.marquee"), cmd("tool.ellipse")]));
    assert_eq!(km.shown_chord(&cmd("tool.marquee")), Some(chord("M")), "a cycle runs its first tool");
    assert_eq!(km.shown_chord(&cmd("tool.ellipse")), Some(chord("M")), "and its other tools");
    assert_eq!(press(&km, "V", &Active::NONE), run("tool.move"));
    assert_eq!(km.shown_chord(&cmd("tool.move")), Some(chord("V")), "of two groups the first runs");
    assert_eq!(km.shown_chord(&cmd("tool.artboard")), None, "the other group's tool never runs on V");
    // A command with empty parameters is the same command.
    assert_eq!(km.shown_chord(&CommandRef::with_params("tool.move", serde_json::json!({}))), Some(chord("V")));
}
