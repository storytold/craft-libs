#![cfg(feature = "egui")]

use craft_keymap::egui::{Dispatcher, Hit, matches, platform};
use craft_keymap::{Active, Bindable, Binding, Chord, CommandRef, Key, KeyInput, Keymap, Kind, Layout, Mods, Platform, Scope, Scopes, Trigger};
use egui::{Event, InputState, Key as EguiKey, Modifiers};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

#[test]
fn a_browser_on_a_mac_gets_the_mac_keymap() {
    let ctx = egui::Context::default();
    ctx.set_os(egui::os::OperatingSystem::from_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15"));
    assert_eq!(platform(&ctx), Platform::Mac);
    ctx.set_os(egui::os::OperatingSystem::from_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"));
    assert_eq!(platform(&ctx), Platform::Windows);
}

#[test]
fn every_key_converts_both_ways() {
    for &key in Key::ALL {
        assert_eq!(Key::from_egui(key.to_egui()), Some(key), "{key}");
    }
    for k in [EguiKey::Copy, EguiKey::Cut, EguiKey::Paste, EguiKey::BrowserBack, EguiKey::ShiftLeft, EguiKey::IntlBackslash] {
        assert_eq!(Key::from_egui(k), None);
    }
}

#[test]
fn modifiers_and_shortcuts_follow_the_platform() {
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    assert_eq!(Mods::from_egui(&ctrl, Platform::Windows), Mods::CMD);
    assert_eq!(Mods::from_egui(&Modifiers::CTRL, Platform::Mac), Mods::CTRL);
    assert_eq!(Mods::from_egui(&Modifiers::MAC_CMD, Platform::Mac), Mods::CMD);

    assert!(matches(&chord("Ctrl+S"), EguiKey::S, &ctrl, Platform::Windows));
    assert!(matches(&chord("Cmd+S"), EguiKey::S, &ctrl, Platform::Windows));
    assert!(!matches(&chord("Cmd+S"), EguiKey::S, &Modifiers::CTRL, Platform::Mac));

    let shortcut = chord("Cmd+Shift+Z").to_egui_shortcut();
    assert_eq!(shortcut.logical_key, EguiKey::Z);
    assert!(Modifiers { mac_cmd: true, command: true, shift: true, ..Modifiers::NONE }.matches_exact(shortcut.modifiers));
}

#[test]
fn shifted_symbols_match_their_bindings() {
    let p = Platform::Windows;
    let shift = Modifiers::SHIFT;
    let none = Modifiers::NONE;
    let cmd = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let cmd_shift = Modifiers { shift: true, ..cmd };

    // Shift+base matches the shifted key egui reports, but the base key alone does not.
    assert!(matches(&chord("Shift+;"), EguiKey::Colon, &shift, p));
    assert!(!matches(&chord("Shift+;"), EguiKey::Semicolon, &none, p));
    assert!(!matches(&chord(";"), EguiKey::Colon, &shift, p));
    assert!(matches(&chord("Shift+/"), EguiKey::Questionmark, &shift, p));
    assert!(matches(&chord("Cmd+Shift+]"), EguiKey::CloseCurlyBracket, &cmd_shift, p));
    assert!(matches(&chord("Shift+1"), EguiKey::Exclamationmark, &shift, p));
    // A shifted symbol bound directly matches while Shift is held to type it.
    assert!(matches(&chord("?"), EguiKey::Questionmark, &shift, p));
    assert!(matches(&chord(":"), EguiKey::Colon, &none, p));

    // Zoom keys: `=` also matches `+`, with or without Shift; `+` matches Shift+`=`.
    assert!(matches(&chord("Cmd+="), EguiKey::Plus, &cmd, p));
    assert!(matches(&chord("Cmd+="), EguiKey::Plus, &cmd_shift, p));
    assert!(matches(&chord("Cmd+="), EguiKey::Equals, &cmd, p));
    assert!(matches(&chord("Cmd++"), EguiKey::Equals, &cmd_shift, p));
    assert!(matches(&chord("Cmd++"), EguiKey::Plus, &cmd_shift, p));
    assert!(!matches(&chord("Cmd++"), EguiKey::Equals, &cmd, p));
    assert!(!matches(&chord("Cmd+="), EguiKey::Plus, &none, p));
    assert!(!matches(&chord("Cmd+="), EguiKey::Minus, &cmd, p));
}

const CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };

fn dispatch_keymap() -> Keymap {
    let app = |id: &str, keys: &[&str]| keys.iter().fold(Bindable::new(id, id, Scope::app()), |e, k| e.with_default(Binding::parse(k).unwrap()));
    let tool = |id: &str| app(id, &["M"]).with_kind(Kind::Tool { group: Some("select".into()) });
    let catalog = vec![
        app("edit.copy", &["Cmd+C", "Cmd+Insert"]),
        app("edit.paste", &["Cmd+V", "Shift+Insert"]),
        app("edit.pasteInPlace", &["Cmd+Shift+V"]),
        app("edit.copyMerged", &["Cmd+Shift+C"]),
        app("app.quit", &["Cmd+Alt+Q"]),
        app("type.pageNumber", &["Cmd+Alt+Shift+N"]),
        app("view.refresh", &["Cmd+Alt+F5"]),
        app("file.save", &["Cmd+S"]),
        app("view.zoomIn", &["="]),
        app("help.find", &["Shift+;"]),
        app("tool.brush", &["B"]),
        app("app.loop", &["L"]),
        Bindable::new("timeline.loop", "Loop", Scope::new("Timeline").unwrap()).with_default(Binding::parse("L").unwrap()),
        tool("tool.marquee"),
        tool("tool.ellipse"),
        app("reveal.one", &["U"]),
        app("reveal.all", &["U U"]),
    ];
    Keymap::new(catalog, Scopes::new().with_panel("Timeline"), Platform::Windows)
}

fn press(key: EguiKey, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

fn frame(time: f64, modifiers: Modifiers, events: Vec<Event>) -> InputState {
    let mut input = InputState::default();
    input.time = time;
    input.modifiers = modifiers;
    input.events = events;
    input
}

/// Dispatches one frame and returns the ids that ran and the events left.
fn run(d: &mut Dispatcher, km: &Keymap, active: &Active, mut input: InputState) -> (Vec<String>, Vec<Event>) {
    let hits = d.dispatch(&mut input, km, active);
    (hits.into_iter().map(|h| h.command.id).collect(), input.events)
}

#[test]
fn a_bound_press_runs_once_and_is_removed_with_its_text() {
    let km = dispatch_keymap();
    let events = vec![
        press(EguiKey::B, Modifiers::NONE),
        Event::Text("b".into()),
        Event::Key { key: EguiKey::B, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
        press(EguiKey::Q, Modifiers::NONE),
        Event::Text("q".into()),
    ];
    let (ran, left) = run(&mut Dispatcher::new(), &km, &Active::NONE, frame(1.0, Modifiers::NONE, events));
    assert_eq!(ran, ["tool.brush"]);
    // The release and the unbound Q with its text stay.
    assert_eq!(left.len(), 3);
    assert!(matches!(left[0], Event::Key { key: EguiKey::B, pressed: false, .. }));
    assert!(matches!(left[1], Event::Key { key: EguiKey::Q, pressed: true, .. }));
    assert_eq!(left[2], Event::Text("q".into()));
}

#[test]
fn shifted_symbols_and_zoom_keys_reach_their_bindings() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let shift = Modifiers::SHIFT;
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(1.0, shift, vec![press(EguiKey::Colon, shift), Event::Text(":".into())]));
    assert_eq!(ran, ["help.find"]);
    assert!(left.is_empty());
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(2.0, Modifiers::NONE, vec![press(EguiKey::Plus, Modifiers::NONE)]));
    assert_eq!(ran, ["view.zoomIn"]);
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(3.0, shift, vec![press(EguiKey::Plus, shift)]));
    assert_eq!(ran, ["view.zoomIn"]);
}

#[test]
fn clipboard_events_stay_with_the_app_by_default() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    // egui-winit sends these in place of the Ctrl+C and Ctrl+V presses.
    let events = vec![Event::Copy, Event::Paste("text".into())];
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(1.0, CTRL, events.clone()));
    assert!(ran.is_empty(), "{ran:?}");
    assert_eq!(left, events);
    // The web build sends the press and then the event: both stay.
    let events = vec![press(EguiKey::C, CTRL), Event::Copy, press(EguiKey::S, CTRL)];
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(2.0, CTRL, events.clone()));
    assert_eq!(ran, ["file.save"]);
    assert_eq!(left, events[..2]);
    // A clipboard press with no clipboard event after it is an ordinary press.
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(3.0, CTRL, vec![press(EguiKey::Insert, CTRL)]));
    assert_eq!(ran, ["edit.copy"]);
}

#[test]
fn clipboard_chords_with_more_modifiers_run_their_bindings() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let typing = Active { typing: true, ..Active::NONE };
    let ctrl_shift = Modifiers { shift: true, ..CTRL };
    // The desktop: egui-winit sends the Paste event in place of Ctrl+Shift+V.
    let mut input = frame(1.0, ctrl_shift, vec![Event::Paste("text".into())]);
    let hits = d.dispatch(&mut input, &km, &Active::NONE);
    let ran: Vec<_> = hits.iter().map(|h| (h.command.id.as_str(), h.pasted.as_deref())).collect();
    assert_eq!(ran, [("edit.pasteInPlace", Some("text"))]);
    assert!(input.events.is_empty());
    let (ran, left) = run(&mut d, &km, &typing, frame(2.0, ctrl_shift, vec![Event::Copy]));
    assert_eq!(ran, ["edit.copyMerged"]);
    assert!(left.is_empty());
    // Ctrl+Shift+X has no binding: the event stays.
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(3.0, ctrl_shift, vec![Event::Cut]));
    assert!(ran.is_empty());
    assert_eq!(left, [Event::Cut]);

    // The web: the press decides for itself and its event, also while typing.
    for (time, active) in [(4.0, &Active::NONE), (5.0, &typing)] {
        let events = vec![press(EguiKey::C, ctrl_shift), Event::Copy];
        let (ran, left) = run(&mut d, &km, active, frame(time, ctrl_shift, events));
        assert_eq!(ran, ["edit.copyMerged"]);
        assert!(left.is_empty(), "{left:?}");
    }
    let events = vec![press(EguiKey::X, ctrl_shift), Event::Cut];
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(6.0, ctrl_shift, events.clone()));
    assert!(ran.is_empty());
    assert_eq!(left, events);
}

#[test]
fn clipboard_events_run_their_chords_outside_text_fields_when_turned_on() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new().with_clipboard_events(true);
    let mut input = frame(1.0, CTRL, vec![Event::Copy, Event::Paste("text".into())]);
    let hits = d.dispatch(&mut input, &km, &Active::NONE);
    let ran: Vec<_> = hits.iter().map(|h| (h.command.id.as_str(), h.pasted.as_deref())).collect();
    assert_eq!(ran, [("edit.copy", None), ("edit.paste", Some("text"))]);
    assert!(input.events.is_empty());

    let typing = Active { typing: true, ..Active::NONE };
    let (ran, left) = run(&mut d, &km, &typing, frame(2.0, CTRL, vec![Event::Copy]));
    assert!(ran.is_empty());
    assert_eq!(left, [Event::Copy]);

    // The web build sends the key press and the clipboard event it causes: one run each.
    for (time, key, mods, event, id) in [
        (3.0, EguiKey::C, CTRL, Event::Copy, "edit.copy"),
        (4.0, EguiKey::Insert, CTRL, Event::Copy, "edit.copy"),
        (5.0, EguiKey::Insert, Modifiers::SHIFT, Event::Paste("text".into()), "edit.paste"),
    ] {
        let (ran, _) = run(&mut d, &km, &Active::NONE, frame(time, mods, vec![press(key, mods), event]));
        assert_eq!(ran, [id], "{key:?}");
    }
}

#[test]
fn a_web_paste_carries_its_text_and_takes_its_event() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new().with_clipboard_events(true);
    // The web build sends the Cmd+V press and then the Paste event with the clipboard text.
    let events = vec![press(EguiKey::V, CTRL), Event::Paste("text".into()), press(EguiKey::C, CTRL), Event::Copy];
    let mut input = frame(1.0, CTRL, events.clone());
    let mut seen = Vec::new();
    let hits = d.dispatch_filtered(&mut input, &km, &Active::NONE, |hit| {
        seen.push(hit.pasted.clone());
        true
    });
    let ran: Vec<_> = hits.iter().map(|h| (h.command.id.as_str(), h.pasted.as_deref())).collect();
    assert_eq!(ran, [("edit.paste", Some("text")), ("edit.copy", None)]);
    assert_eq!(seen, [Some("text".to_string()), None], "the filter sees the pasted text");
    assert!(input.events.is_empty(), "{:?}", input.events);

    // A declined press leaves the press and its event in place, and the event does not run again.
    let mut input = frame(2.0, CTRL, events.clone());
    let hits = d.dispatch_filtered(&mut input, &km, &Active::NONE, |hit| hit.command.id != "edit.paste");
    let ran: Vec<_> = hits.iter().map(|h| h.command.id.as_str()).collect();
    assert_eq!(ran, ["edit.copy"]);
    assert_eq!(input.events, events[..2]);
}

#[test]
fn a_held_web_paste_runs_once_when_repeats_are_off() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new().with_clipboard_events(true);
    let repeat = Event::Key { key: EguiKey::V, physical_key: Some(EguiKey::V), pressed: true, repeat: true, modifiers: CTRL };
    let paste = || Event::Paste("text".into());
    let events = vec![press(EguiKey::V, CTRL), paste(), repeat.clone(), paste()];
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(1.0, CTRL, events));
    assert_eq!(ran, ["edit.paste"]);
    assert_eq!(left, [repeat, paste()], "the repeat and its event stay in egui's input");
}

#[test]
fn declined_hits_leave_their_events_in_order() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let shift = Modifiers::SHIFT;
    let events = vec![
        press(EguiKey::B, Modifiers::NONE),
        Event::Text("b".into()),
        press(EguiKey::S, CTRL),
        press(EguiKey::Colon, shift),
        Event::Text(":".into()),
        press(EguiKey::Equals, Modifiers::NONE),
    ];
    let mut input = frame(1.0, Modifiers::NONE, events.clone());
    let mut seen = Vec::new();
    let hits = d.dispatch_filtered(&mut input, &km, &Active::NONE, |hit| {
        seen.push((hit.command.id.clone(), hit.trigger.chord.to_string(), hit.exact));
        hit.command.id != "file.save" && hit.command.id != "tool.brush"
    });
    let s = |id: &str, chord: &str, exact| (id.to_string(), chord.to_string(), exact);
    assert_eq!(seen, [s("tool.brush", "B", true), s("file.save", "Cmd+S", true), s("help.find", "Shift+;", false), s("view.zoomIn", "=", true)]);
    let ran: Vec<_> = hits.iter().map(|h| h.command.id.as_str()).collect();
    assert_eq!(ran, ["help.find", "view.zoomIn"]);
    assert_eq!(input.events, events[..3], "the declined B keeps its text, and Ctrl+S stays in its place");
}

#[test]
fn clipboard_presses_stay_with_the_text_field() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let typing = Active { typing: true, ..Active::NONE };
    // The web build's Cmd+C press and its Copy event both reach the text field.
    let events = vec![press(EguiKey::C, CTRL), Event::Copy, press(EguiKey::Insert, CTRL)];
    let (ran, left) = run(&mut d, &km, &typing, frame(1.0, CTRL, events.clone()));
    assert!(ran.is_empty(), "{ran:?}");
    assert_eq!(left, events);
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(2.0, CTRL, vec![press(EguiKey::Insert, CTRL)]));
    assert_eq!(ran, ["edit.copy"]);
}

#[test]
fn a_press_that_types_text_stays_with_the_text_field() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let typing = Active { typing: true, ..Active::NONE };
    // A press the web build's text agent follows with the text it typed.
    let events = vec![press(EguiKey::B, Modifiers::NONE), Event::Text("b".into())];
    let (ran, left) = run(&mut d, &km, &typing, frame(1.0, Modifiers::NONE, events.clone()));
    assert!(ran.is_empty(), "{ran:?}");
    assert_eq!(left, events);
}

#[test]
fn ctrl_alt_is_left_to_the_text_field_only_when_it_types() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let typing = Active { typing: true, ..Active::NONE };
    // AltGr+Q on a German layout types `@`: the browser on Windows reports Ctrl+Alt+Q and the text
    // agent sends the text after it.
    let alt_gr = Modifiers { alt: true, ..CTRL };
    let events = vec![press(EguiKey::Q, alt_gr), Event::Text("@".into())];
    let (ran, left) = run(&mut d, &km, &typing, frame(1.0, alt_gr, events.clone()));
    assert!(ran.is_empty(), "{ran:?}");
    assert_eq!(left, events);
    // A real Ctrl+Alt chord types nothing, so it runs while typing.
    let all = Modifiers { shift: true, ..alt_gr };
    let (ran, left) = run(&mut d, &km, &typing, frame(2.0, all, vec![press(EguiKey::N, all)]));
    assert_eq!(ran, ["type.pageNumber"]);
    assert!(left.is_empty());
    let (ran, _) = run(&mut d, &km, &typing, frame(3.0, alt_gr, vec![press(EguiKey::Q, alt_gr)]));
    assert_eq!(ran, ["app.quit"]);
}

#[test]
fn typing_blocks_plain_keys_but_not_command_chords() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let typing = Active { typing: true, ..Active::NONE };
    let events = vec![press(EguiKey::B, Modifiers::NONE), Event::Text("b".into()), press(EguiKey::S, CTRL)];
    let (ran, left) = run(&mut d, &km, &typing, frame(1.0, CTRL, events));
    assert_eq!(ran, ["file.save"]);
    assert_eq!(left.len(), 2);
    assert_eq!(left[1], Event::Text("b".into()));
}

#[test]
fn undo_goes_to_the_text_field_and_save_still_runs() {
    let km = Keymap::new(
        vec![
            Bindable::new("edit.undo", "Undo", Scope::app()).with_default(Binding::parse("Cmd+Z").unwrap()),
            Bindable::new("file.save", "Save", Scope::app()).with_default(Binding::parse("Cmd+S").unwrap()),
        ],
        Scopes::new(),
        Platform::Windows,
    );
    let typing = Active { typing: true, ..Active::NONE };
    let events = vec![press(EguiKey::Z, CTRL), press(EguiKey::S, CTRL)];
    let (ran, left) = run(&mut Dispatcher::new(), &km, &typing, frame(1.0, CTRL, events));
    assert_eq!(ran, ["file.save"]);
    assert_eq!(left, [press(EguiKey::Z, CTRL)]);
}

#[test]
fn the_focused_panel_wins_and_tool_groups_cycle() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let l = || frame(1.0, Modifiers::NONE, vec![press(EguiKey::L, Modifiers::NONE)]);
    assert_eq!(run(&mut d, &km, &Active::panel("Timeline"), l()).0, ["timeline.loop"]);
    assert_eq!(run(&mut d, &km, &Active::NONE, l()).0, ["app.loop"]);

    let mut input = frame(2.0, Modifiers::NONE, vec![press(EguiKey::M, Modifiers::NONE)]);
    let hits = d.dispatch(&mut input, &km, &Active::NONE);
    let tools = vec![CommandRef::new("tool.marquee"), CommandRef::new("tool.ellipse")];
    let trigger = Trigger::single(chord("M"));
    let got: Vec<_> = hits.iter().map(|h: &Hit| (&h.command, &h.cycle, h.trigger, h.exact, h.pasted.as_deref())).collect();
    assert_eq!(got, [(&tools[0], &tools, trigger, true, None)]);
}

#[test]
fn a_second_press_runs_the_double_binding() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let key = |d: &mut Dispatcher, k: EguiKey, t: f64| run(d, &km, &Active::NONE, frame(t, Modifiers::NONE, vec![press(k, Modifiers::NONE)])).0;
    let u = |d: &mut Dispatcher, t: f64| key(d, EguiKey::U, t);
    assert_eq!(u(&mut d, 1.0), ["reveal.one"]);
    assert_eq!(u(&mut d, 1.3), ["reveal.all"]);
    // The press after a double starts over.
    assert_eq!(u(&mut d, 1.5), ["reveal.one"]);
    assert_eq!(u(&mut d, 2.5), ["reveal.one"]);
    // B in between breaks the pair.
    key(&mut d, EguiKey::B, 2.6);
    assert_eq!(u(&mut d, 2.7), ["reveal.one"]);
}

#[test]
fn a_double_press_needs_both_presses_in_one_panel() {
    let km = Keymap::new(
        vec![
            Bindable::new("x.one", "One", Scope::new("X").unwrap()).with_default(Binding::parse("A").unwrap()),
            Bindable::new("y.one", "One", Scope::new("Y").unwrap()).with_default(Binding::parse("A").unwrap()),
            Bindable::new("y.two", "Two", Scope::new("Y").unwrap()).with_default(Binding::parse("A A").unwrap()),
        ],
        Scopes::new().with_panel("X").with_panel("Y").with_panel_fallthrough(false),
        Platform::Windows,
    );
    let mut d = Dispatcher::new();
    let a = |d: &mut Dispatcher, panel, t| run(d, &km, &Active::panel(panel), frame(t, Modifiers::NONE, vec![press(EguiKey::A, Modifiers::NONE)])).0;
    assert_eq!(a(&mut d, "X", 1.0), ["x.one"]);
    assert_eq!(a(&mut d, "Y", 1.2), ["y.one"]);
    assert_eq!(a(&mut d, "Y", 1.4), ["y.two"]);
}

#[test]
fn two_presses_in_one_frame_make_a_double() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let u = press(EguiKey::U, Modifiers::NONE);
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(4.0, Modifiers::NONE, vec![u.clone(), u]));
    assert_eq!(ran, ["reveal.one", "reveal.all"]);
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn key_repeats_run_only_when_turned_on() {
    let km = Keymap::new(
        vec![
            Bindable::new("nudge.right", "Nudge", Scope::app()).with_default(Binding::parse("Right").unwrap()),
            Bindable::new("jump.right", "Jump", Scope::app()).with_default(Binding::parse("Right Right").unwrap()),
        ],
        Scopes::new(),
        Platform::Windows,
    );
    let held = |repeat: bool| Event::Key { key: EguiKey::ArrowRight, physical_key: None, pressed: true, repeat, modifiers: Modifiers::NONE };
    let events = || vec![held(false), held(true), held(true)];

    let mut on = Dispatcher::new().with_repeat(true);
    let (ran, left) = run(&mut on, &km, &Active::NONE, frame(1.0, Modifiers::NONE, events()));
    assert_eq!(ran, ["nudge.right", "nudge.right", "nudge.right"]);
    assert!(left.is_empty());
    // Repeats neither made nor broke a double: the next press within the window is one.
    let (ran, _) = run(&mut on, &km, &Active::NONE, frame(1.3, Modifiers::NONE, vec![held(false)]));
    assert_eq!(ran, ["jump.right"]);

    let (ran, left) = run(&mut Dispatcher::new(), &km, &Active::NONE, frame(1.0, Modifiers::NONE, events()));
    assert_eq!(ran, ["nudge.right"]);
    assert_eq!(left, [held(true), held(true)]);
}

#[test]
fn the_dispatcher_reads_presses_by_the_layout() {
    let app = |id: &str, key: &str| Bindable::new(id, id, Scope::app()).with_default(Binding::parse(key).unwrap());
    let km = Keymap::new(
        vec![app("edit.duplicate", "Cmd+Shift+D"), app("tab.next", "Cmd+Shift+]"), app("edit.comment", "Cmd+/")],
        Scopes::new(),
        Platform::Windows,
    );
    let at = |key, physical, modifiers| Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers };
    let ctrl_shift = Modifiers { shift: true, ..CTRL };

    // Arabic 101: typing ي on the D key shows the layout is not Latin, so ⌃⇧D (which types ]) is
    // Ctrl+Shift+D.
    let mut d = Dispatcher::new();
    let typed = vec![at(EguiKey::D, EguiKey::D, Modifiers::NONE), Event::Text("ي".into())];
    run(&mut d, &km, &Active::NONE, frame(1.0, Modifiers::NONE, typed));
    assert!(!d.layout().is_latin());
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(2.0, ctrl_shift, vec![at(EguiKey::CloseBracket, EguiKey::D, ctrl_shift)]));
    assert_eq!(ran, ["edit.duplicate"]);

    // German: ⇧7 types /, so Ctrl+Shift+7 runs Ctrl+/; the ü key's label is learned from typing.
    let mut d = Dispatcher::new();
    let typed = vec![at(EguiKey::OpenBracket, EguiKey::OpenBracket, Modifiers::NONE), Event::Text("ü".into())];
    run(&mut d, &km, &Active::NONE, frame(1.0, Modifiers::NONE, typed));
    assert_eq!(chord("Cmd+[").display_for(Platform::Windows, d.layout()), "Ctrl+Ü");
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(2.0, ctrl_shift, vec![at(EguiKey::Slash, EguiKey::Num7, ctrl_shift)]));
    assert_eq!(ran, ["edit.comment"]);

    // German on egui-winit: AltGr+M types µ and AltGr+] types ~ with no modifiers reported, so
    // presses while right Alt is held teach nothing and Ctrl+Z stays Ctrl+Z on the Y position.
    let mut d = Dispatcher::new();
    let typed = vec![at(EguiKey::Plus, EguiKey::CloseBracket, Modifiers::NONE), Event::Text("+".into())];
    run(&mut d, &km, &Active::NONE, frame(1.0, Modifiers::NONE, typed));
    let right_alt =
        |pressed| Event::Key { key: EguiKey::AltRight, physical_key: Some(EguiKey::AltRight), pressed, repeat: false, modifiers: Modifiers::NONE };
    run(&mut d, &km, &Active::NONE, frame(2.0, Modifiers::NONE, vec![right_alt(true)]));
    let alt_graph = vec![
        at(EguiKey::M, EguiKey::M, Modifiers::NONE),
        Event::Text("µ".into()),
        at(EguiKey::CloseBracket, EguiKey::CloseBracket, Modifiers::NONE),
        Event::Text("~".into()),
        right_alt(false),
    ];
    run(&mut d, &km, &Active::NONE, frame(3.0, Modifiers::NONE, alt_graph));
    assert!(d.layout().is_latin());
    assert_eq!(d.layout().char_at(Key::CloseBracket), Some('+'));
    assert_eq!(d.layout().resolve(&KeyInput::new(Mods::CMD).with_typed(Key::Z).with_physical(Key::Y)), Some(chord("Cmd+Z")));

    // A layout the platform reports applies from the first press.
    let mut d = Dispatcher::new().with_layout(Layout::new().with_latin(false));
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(1.0, ctrl_shift, vec![at(EguiKey::CloseBracket, EguiKey::D, ctrl_shift)]));
    assert_eq!(ran, ["edit.duplicate"]);
}

#[test]
fn letters_follow_the_latin_layout_from_egui_winit_to_the_dispatcher() {
    let app = |id: &str, key: &str| Bindable::new(id, id, Scope::app()).with_default(Binding::parse(key).unwrap());
    let km = Keymap::new(vec![app("d", "Cmd+D"), app("e", "Cmd+E")], Scopes::new(), Platform::Linux);
    let at = |key, modifiers| Event::Key { key, physical_key: Some(EguiKey::D), pressed: true, repeat: false, modifiers };

    // The QWERTY D key, as the suite's egui-winit reports it: on a layout that is not Latin, the
    // logical key is the Latin layout's letter (Dvorak types e there), and the text is Cyrillic.
    // (layout, logical key, text typed, platform report, command Ctrl runs)
    let cases = [
        ("Dvorak + Russian", EguiKey::E, "в", Some(false), "e"),
        ("QWERTY + Russian", EguiKey::D, "в", Some(false), "d"),
        ("Dvorak", EguiKey::E, "e", Some(true), "e"),
        ("QWERTY", EguiKey::D, "d", Some(true), "d"),
    ];
    for (name, key, text, latin, expected) in cases {
        // The layout learned from typing.
        let mut d = Dispatcher::new();
        run(&mut d, &km, &Active::NONE, frame(1.0, Modifiers::NONE, vec![at(key, Modifiers::NONE), Event::Text(text.into())]));
        assert_eq!(Some(d.layout().is_latin()), latin, "{name}");
        let (ran, _) = run(&mut d, &km, &Active::NONE, frame(2.0, CTRL, vec![at(key, CTRL)]));
        assert_eq!(ran, [expected], "{name}, learned");

        // The layout the platform reports, before anything is typed.
        let mut d = Dispatcher::new().with_layout(Layout::new().with_latin(latin == Some(true)));
        let (ran, _) = run(&mut d, &km, &Active::NONE, frame(1.0, CTRL, vec![at(key, CTRL)]));
        assert_eq!(ran, [expected], "{name}, reported");
    }
}

/// `input` with the Latin layout characters the suite's egui-winit reports per physical key, set
/// through serde because crates.io egui has no such field; `None` there.
fn with_latin_layout_chars(mut input: InputState, chars: &[(EguiKey, char)]) -> Option<InputState> {
    let mut raw = serde_json::to_value(&input.raw).unwrap();
    raw.get("latin_layout_chars")?;
    raw["latin_layout_chars"] = serde_json::to_value(chars).unwrap();
    input.raw = serde_json::from_value(raw).unwrap();
    Some(input)
}

#[test]
fn a_key_without_a_latin_letter_runs_no_letter_binding() {
    let app = |id: &str, key: &str| Bindable::new(id, id, Scope::app()).with_default(Binding::parse(key).unwrap());
    let km = Keymap::new(vec![app("quit", "Cmd+Q"), app("quote", "Cmd+'"), app("j", "Cmd+J")], Scopes::new(), Platform::Linux);
    let at = |key, physical| Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers: CTRL };

    // Dvorak + Russian. The QWERTY Q key types й, and Dvorak ' there: egui-winit reports the key
    // as ', and egui without a key for the character as its position, Q. The QWERTY C key types
    // с, and Dvorak j.
    let latin = [(EguiKey::Q, '\''), (EguiKey::C, 'j')];
    let cases = [(at(EguiKey::Quote, EguiKey::Q), "quote"), (at(EguiKey::Q, EguiKey::Q), "quote"), (at(EguiKey::J, EguiKey::C), "j")];
    for (press, expected) in cases {
        let mut d = Dispatcher::new().with_layout(Layout::new().with_latin(false));
        let Some(input) = with_latin_layout_chars(frame(1.0, CTRL, vec![press.clone()]), &latin) else {
            // crates.io egui reports no Latin layout, so the position decides: Ctrl+Q.
            let (ran, _) = run(&mut d, &km, &Active::NONE, frame(1.0, CTRL, vec![at(EguiKey::Q, EguiKey::Q)]));
            assert_eq!(ran, ["quit"]);
            return;
        };
        let (ran, _) = run(&mut d, &km, &Active::NONE, input);
        assert_eq!(ran, [expected], "{press:?}");
    }
}
