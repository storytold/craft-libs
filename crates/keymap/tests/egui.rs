#![cfg(feature = "egui")]

use craft_keymap::egui::{Dispatcher, Hit, matches};
use craft_keymap::{Active, Bindable, Binding, Chord, CommandRef, Key, Keymap, Kind, Mods, Platform, Scope, Scopes};
use egui::{Event, InputState, Key as EguiKey, Modifiers};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
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
    let catalogue = vec![
        app("edit.copy", &["Cmd+C", "Cmd+Insert"]),
        app("app.quit", &["Cmd+Alt+Q"]),
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
    Keymap::new(catalogue, Scopes::new().with_panel("Timeline"), Platform::Windows)
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
fn clipboard_events_run_their_chords_outside_text_fields() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(1.0, CTRL, vec![Event::Copy]));
    assert_eq!(ran, ["edit.copy"]);
    assert!(left.is_empty());

    let typing = Active { typing: true, ..Active::NONE };
    let (ran, left) = run(&mut d, &km, &typing, frame(2.0, CTRL, vec![Event::Copy]));
    assert!(ran.is_empty());
    assert_eq!(left, [Event::Copy]);

    // The web build sends the key press and the Copy event it causes: one copy.
    let (ran, _) = run(&mut d, &km, &Active::NONE, frame(3.0, CTRL, vec![press(EguiKey::C, CTRL), Event::Copy]));
    assert_eq!(ran, ["edit.copy"]);
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
fn ctrl_alt_on_a_printable_key_is_left_to_the_text_field() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let typing = Active { typing: true, ..Active::NONE };
    // AltGr+Q on a German layout types `@`; Windows reports it as Ctrl+Alt+Q, and egui-winit sends
    // no Text event while Ctrl is held.
    let alt_gr = Modifiers { alt: true, ..CTRL };
    let events = vec![press(EguiKey::Q, alt_gr)];
    let (ran, left) = run(&mut d, &km, &typing, frame(1.0, alt_gr, events.clone()));
    assert!(ran.is_empty(), "{ran:?}");
    assert_eq!(left, events);
    let (ran, _) = run(&mut d, &km, &typing, frame(2.0, alt_gr, vec![press(EguiKey::F5, alt_gr)]));
    assert_eq!(ran, ["view.refresh"], "a key that types nothing");
    let (ran, left) = run(&mut d, &km, &Active::NONE, frame(3.0, alt_gr, events));
    assert_eq!(ran, ["app.quit"], "outside a text field");
    assert!(left.is_empty());

    // macOS has no AltGr: Cmd+Option+Q is a command chord while typing.
    let mac = Keymap::new(
        vec![Bindable::new("app.quit", "Quit", Scope::app()).with_default(Binding::parse("Cmd+Alt+Q").unwrap())],
        Scopes::new(),
        Platform::Mac,
    );
    let cmd_opt = Modifiers { alt: true, mac_cmd: true, command: true, ..Modifiers::NONE };
    let (ran, _) = run(&mut Dispatcher::new(), &mac, &typing, frame(4.0, cmd_opt, vec![press(EguiKey::Q, cmd_opt)]));
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
fn the_focused_panel_wins_and_tool_groups_cycle() {
    let km = dispatch_keymap();
    let mut d = Dispatcher::new();
    let l = || frame(1.0, Modifiers::NONE, vec![press(EguiKey::L, Modifiers::NONE)]);
    assert_eq!(run(&mut d, &km, &Active::panel("Timeline"), l()).0, ["timeline.loop"]);
    assert_eq!(run(&mut d, &km, &Active::NONE, l()).0, ["app.loop"]);

    let mut input = frame(2.0, Modifiers::NONE, vec![press(EguiKey::M, Modifiers::NONE)]);
    let hits = d.dispatch(&mut input, &km, &Active::NONE);
    let tools = vec![CommandRef::new("tool.marquee"), CommandRef::new("tool.ellipse")];
    assert_eq!(hits, [Hit { command: tools[0].clone(), cycle: tools }]);
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
