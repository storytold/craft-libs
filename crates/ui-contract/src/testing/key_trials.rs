//! Trials of the bare-Tab chrome toggle: when a Tab press toggles a window's panels and when it moves focus.

use crate::runtime::commands::{ChromeCommand, CommandInput, CommandSet as _, EditCommands};
use crate::runtime::env::RootEnv;
use crate::runtime::focus::{FocusTarget, KeyUse, request};
use crate::runtime::frame::{ChromeToggle, take_output};
use crate::runtime::keys::{focus_holds, intents_of, take_shortcuts};
use crate::testing::focus::{Stop, egui_focused, key_release, named_id, press_shape, register_stop, widget};
use crate::testing::input::{key_press, key_repeat};
use crate::testing::pointer::click;
use crate::testing::{Frames, trial};
use craft_behavior::{Input, Intent, Sense};
use craft_keymap::{Chord, Overrides};
use egui::{Id, Key, Modifiers, Rect, Ui, ViewportId};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

fn id(name: &str) -> Id {
    named_id(name)
}

fn stop(ui: &mut Ui, name: &str, keys: KeyUse) {
    widget(ui, id(name));
    register_stop(ui.ctx(), Stop::new(id(name), keys));
}

/// A window with a document canvas, a button and a text field.
fn with_canvas(ui: &mut Ui) {
    stop(ui, "button", KeyUse::of(Input::Pointer));
    stop(ui, "canvas", KeyUse::CANVAS);
    stop(ui, "field", KeyUse::of(Input::Text));
}

/// A window without a canvas: a button and a slider.
fn without_canvas(ui: &mut Ui) {
    stop(ui, "button", KeyUse::of(Input::Pointer));
    stop(ui, "slider", KeyUse::of(Input::Pointer).with_value(true));
}

/// Frames with the app's switch `enabled`, past the first frame that registers the stops.
fn shown(enabled: bool, show: &mut impl FnMut(&mut Ui)) -> Frames {
    let mut frames = Frames::new();
    frames.root({
        let mut root = RootEnv::default();
        root.tab_toggles_chrome = enabled;
        root
    });
    frames.frame(Vec::new(), &mut *show);
    take_output(&frames.ctx);
    frames
}

/// A press of `key` with `modifiers` and its release, in one frame: a second press with no release between is a repeat.
fn press(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), key: Key, modifiers: Modifiers) -> Vec<(ViewportId, ChromeToggle)> {
    frames.frame(vec![key_press(key, modifiers), key_release(key, modifiers)], &mut *show);
    take_output(&frames.ctx).chrome_toggles
}

fn focus_on(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), name: &str) {
    request(&frames.ctx, FocusTarget::Id(id(name)));
    frames.frame(Vec::new(), &mut *show);
    take_output(&frames.ctx);
}

#[trial]
fn tab_in_a_window_with_a_canvas_toggles_that_window_only() {
    let mut show = with_canvas;
    let mut frames = shown(true, &mut show);
    let child = ViewportId::from_hash_of("child window");
    frames.frame_in_viewport(child, Vec::new(), without_canvas);
    take_output(&frames.ctx);
    assert_eq!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE), [(ViewportId::ROOT, ChromeToggle::Panels)], "nothing focused");
    assert_eq!(egui_focused(&frames.ctx), None, "the press moved no focus");
    focus_on(&mut frames, &mut show, "canvas");
    assert_eq!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE), [(ViewportId::ROOT, ChromeToggle::Panels)], "canvas focused");
    assert_eq!(egui_focused(&frames.ctx), Some(id("canvas")));
}

#[trial]
fn tab_in_a_window_with_a_canvas_moves_focus_off_a_control() {
    let mut show = with_canvas;
    let mut frames = shown(true, &mut show);
    focus_on(&mut frames, &mut show, "button");
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty());
    assert_eq!(egui_focused(&frames.ctx), Some(id("canvas")));
    focus_on(&mut frames, &mut show, "field");
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty(), "a text field never loses Tab");
}

/// In a window with a canvas, a component a press focused leaves Tab to the panels' toggle, as clicking a panel's row and
/// pressing Tab hides the panels in Photoshop; one the keyboard or a request focused moves focus on Tab. `focus_holds`
/// names the Tab as the component's exactly when it moves focus.
#[trial]
fn tab_toggles_from_a_pressed_component_and_moves_focus_from_a_requested_one() {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut show = move |ui: &mut Ui| {
        let rect = widget(ui, id("button"));
        register_stop(ui.ctx(), Stop::new(id("button"), KeyUse::of(Input::Pointer)));
        press_shape(ui, rect, Sense::CLICK | Sense::TAKE_FOCUS, id("button"));
        seen.set(rect);
        stop(ui, "other", KeyUse::of(Input::Pointer));
        stop(ui, "canvas", KeyUse::CANVAS);
    };
    let mut frames = shown(true, &mut show);
    click(&mut frames, at.get().center(), &mut show);
    take_output(&frames.ctx);
    assert_eq!(egui_focused(&frames.ctx), Some(id("button")), "the press focused the button");
    let tab = Chord::plain(craft_keymap::Key::Tab);
    assert!(!focus_holds(&frames.ctx, tab), "a pressed component leaves Tab to the toggle");
    assert_eq!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE), [(ViewportId::ROOT, ChromeToggle::Panels)], "a pressed component");
    assert_eq!(egui_focused(&frames.ctx), Some(id("button")), "the toggle moved no focus");
    focus_on(&mut frames, &mut show, "other");
    assert!(focus_holds(&frames.ctx, tab), "a requested component's Tab moves focus");
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty(), "a requested component");
    assert_ne!(egui_focused(&frames.ctx), Some(id("other")), "focus left the component");
}

/// A text field keeps Tab for going to the next field however it got the focus: clicking into a value, typing and
/// pressing Tab commits it and moves on, as in Photoshop.
#[trial]
fn tab_moves_focus_from_a_pressed_text_field() {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut show = move |ui: &mut Ui| {
        let rect = widget(ui, id("field"));
        register_stop(ui.ctx(), Stop::new(id("field"), KeyUse::of(Input::Text)));
        press_shape(ui, rect, Sense::CLICK | Sense::TAKE_FOCUS, id("field"));
        seen.set(rect);
        stop(ui, "other", KeyUse::of(Input::Pointer));
        stop(ui, "canvas", KeyUse::CANVAS);
    };
    let mut frames = shown(true, &mut show);
    click(&mut frames, at.get().center(), &mut show);
    take_output(&frames.ctx);
    assert_eq!(egui_focused(&frames.ctx), Some(id("field")), "the press focused the field");
    assert!(focus_holds(&frames.ctx, Chord::plain(craft_keymap::Key::Tab)), "the field's Tab moves focus");
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty(), "a pressed text field toggles nothing");
    assert_ne!(egui_focused(&frames.ctx), Some(id("field")), "focus left the field");
}

/// Read-only text a press focused to select from (a label's selection part, a stop only a press reaches) is no text
/// field: Tab toggles the panels from it, as from any pressed control. Guards against a toggle that read every stop of
/// the text class as a field that keeps Tab for the next field.
#[trial]
fn tab_toggles_from_pressed_read_only_text() {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut show = move |ui: &mut Ui| {
        let rect = widget(ui, id("label"));
        register_stop(ui.ctx(), Stop::new(id("label"), KeyUse::of(Input::Text)).press_only());
        press_shape(ui, rect, Sense::CLICK | Sense::TAKE_FOCUS, id("label"));
        seen.set(rect);
        stop(ui, "canvas", KeyUse::CANVAS);
    };
    let mut frames = shown(true, &mut show);
    click(&mut frames, at.get().center(), &mut show);
    take_output(&frames.ctx);
    assert_eq!(egui_focused(&frames.ctx), Some(id("label")), "the press focused the text");
    assert_eq!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE), [(ViewportId::ROOT, ChromeToggle::Panels)], "pressed read-only text");
    assert_eq!(egui_focused(&frames.ctx), Some(id("label")), "the toggle moved no focus");
}

#[trial]
fn tab_in_a_window_without_a_canvas_moves_focus() {
    let mut show = without_canvas;
    let mut frames = shown(true, &mut show);
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty(), "nothing focused");
    assert_eq!(egui_focused(&frames.ctx), Some(id("button")));
    focus_on(&mut frames, &mut show, "slider");
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty(), "a slider");
    assert_ne!(egui_focused(&frames.ctx), Some(id("slider")), "focus left the slider");
}

#[trial]
fn a_repeated_tab_toggles_nothing_and_moves_no_focus() {
    let mut show = with_canvas;
    let mut frames = shown(true, &mut show);
    frames.frame(vec![key_repeat(Key::Tab, Modifiers::NONE)], &mut show);
    assert!(take_output(&frames.ctx).chrome_toggles.is_empty());
    assert_eq!(egui_focused(&frames.ctx), None);
}

#[trial]
fn a_rebound_command_fires_on_its_new_key_and_tab_then_moves_focus() {
    let mut overrides = Overrides::default();
    overrides.insert(ChromeCommand::TogglePanels.id(), vec!["F9".to_owned()]);
    let mut show = with_canvas;
    let mut frames = Frames::new();
    frames.root({
        let mut root = RootEnv::default();
        root.tab_toggles_chrome = true;
        root
    });
    frames.commands(CommandInput::new().overrides(Arc::new(overrides)));
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    assert_eq!(press(&mut frames, &mut show, Key::F9, Modifiers::NONE), [(ViewportId::ROOT, ChromeToggle::Panels)]);
    assert!(press(&mut frames, &mut show, Key::Tab, Modifiers::NONE).is_empty());
    assert_eq!(egui_focused(&frames.ctx), Some(id("button")), "Tab moves focus once its command is rebound");
}

/// What a press of Ctrl+Shift+K gave the stop `name`: the chords it took, and the shortcut events left to the
/// application's commands.
fn press_k(name: &str) -> (Vec<craft_keymap::Chord>, usize) {
    let taken = std::rc::Rc::new(std::cell::RefCell::new((Vec::new(), 0)));
    let sink = std::rc::Rc::clone(&taken);
    let stop_id = id(name);
    let mut show = move |ui: &mut Ui| {
        stop(ui, "button", KeyUse::of(Input::Pointer));
        stop(ui, "recorder", KeyUse::RECORDER);
        let mut intents = intents_of(ui.ctx(), Some(stop_id), false);
        let chords = intents.take_chords();
        if !chords.is_empty() {
            let shortcuts = take_shortcuts(ui.ctx()).keys.len();
            *sink.borrow_mut() = (chords, shortcuts);
        }
    };
    let mut frames = shown(false, &mut show);
    focus_on(&mut frames, &mut show, name);
    frames.frame(vec![key_press(Key::K, Modifiers::COMMAND | Modifiers::SHIFT)], &mut show);

    taken.borrow().clone()
}

#[trial]
fn a_recorder_reads_a_press_as_one_chord_and_no_command_runs() {
    let chord = craft_keymap::Chord { mods: craft_keymap::Mods { shift: true, ..craft_keymap::Mods::CMD }, key: craft_keymap::Key::K };
    assert_eq!(press_k("recorder"), (vec![chord], 0), "the recorder owns the press");
    assert_eq!(press_k("button").0, Vec::new(), "no other role reads chords");
}

/// With nothing focused, the window's document canvas takes the arrows' nudge and an unbound Page Up, as while it has
/// the focus; the press is its alone, so no app shortcut runs with it as well (a page view would turn twice). A page key
/// the app binds to a command that runs is the app's (Page Down to the next artboard). Without a canvas every press is
/// the app's.
#[trial]
fn an_unfocused_press_the_window_canvas_takes_reaches_no_app_shortcut() {
    let paged = Rc::new(Cell::new(0));
    let canvas = Rc::new(Cell::new(true));
    let (sink, shows) = (Rc::clone(&paged), Rc::clone(&canvas));
    let mut show = move |ui: &mut Ui| {
        stop(ui, "button", KeyUse::of(Input::Pointer));
        if shows.get() {
            stop(ui, "canvas", KeyUse::CANVAS);
        }
        let mut intents = intents_of(ui.ctx(), Some(id("canvas")), false);
        for page in [craft_behavior::NavMove::PageDown, craft_behavior::NavMove::PageUp] {
            if intents.take(Intent::Move(page, craft_behavior::MoveMods::PLAIN)) {
                sink.set(sink.get() + 1);
            }
        }
    };
    let mut bound = Overrides::default();
    bound.insert(EditCommands::Delete.id(), vec!["PageDown".to_owned()]);
    bound.insert(EditCommands::SelectAll.id(), vec!["Right".to_owned()]);
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().overrides(Arc::new(bound)));
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    let fired = |frames: &mut Frames, show: &mut dyn FnMut(&mut Ui), key: Key| {
        frames.frame(vec![key_press(key, Modifiers::NONE), key_release(key, Modifiers::NONE)], &mut *show);
        frames.frame(Vec::new(), &mut *show);
        take_output(&frames.ctx).fired.len()
    };
    assert_eq!(fired(&mut frames, &mut show, Key::ArrowRight), 0, "the arrow is the canvas's nudge");
    assert_eq!(fired(&mut frames, &mut show, Key::PageUp), 0, "the unbound Page Up is the canvas's");
    assert_eq!(paged.get(), 1, "the canvas pages");
    assert_eq!(fired(&mut frames, &mut show, Key::PageDown), 1, "the bound Page Down is the app's");
    assert_eq!(paged.get(), 1, "the canvas does not page with it");
    canvas.set(false);
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    let presses = [Key::PageDown, Key::ArrowRight].map(|key| fired(&mut frames, &mut show, key));
    assert_eq!(presses, [1, 1], "without a canvas both presses are the app's");
}

/// A recorder holds every press: the app reads none from egui's input, and `focus_holds` answers so before the press.
#[trial]
fn a_recorder_holds_its_presses_from_the_app() {
    let saw = std::cell::Cell::new(false);
    let mut show = |ui: &mut Ui| {
        stop(ui, "button", KeyUse::of(Input::Pointer));
        stop(ui, "recorder", KeyUse::RECORDER);
        saw.set(saw.get() || ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: Key::K, pressed: true, .. }))));
    };
    let mut frames = shown(false, &mut show);
    focus_on(&mut frames, &mut show, "recorder");
    let chord = craft_keymap::Chord { mods: craft_keymap::Mods { shift: true, ..craft_keymap::Mods::CMD }, key: craft_keymap::Key::K };
    assert!(crate::runtime::keys::focus_holds(&frames.ctx, chord), "the recorder names the chord as its own");
    frames.frame(vec![key_press(Key::K, Modifiers::COMMAND | Modifiers::SHIFT)], &mut show);
    assert!(!saw.get(), "the app reads no press the recorder holds");
}

// A dialog opened in a pass takes that pass's Escape, also when the press that opened it moved focus off the stop the
// Escape was routed to: the stop it left gets no Cancel.
#[trial]
fn a_dialog_opened_in_a_pass_takes_its_escape_after_focus_moved() {
    let opens = std::rc::Rc::new(std::cell::Cell::new(false));
    let cancels = std::rc::Rc::new(std::cell::Cell::new(0));
    let (open, seen) = (std::rc::Rc::clone(&opens), std::rc::Rc::clone(&cancels));
    let mut show = move |ui: &mut Ui| {
        stop(ui, "button", KeyUse::of(Input::Pointer));
        stop(ui, "canvas", KeyUse::CANVAS);
        if open.get() {
            // The press on the button that opens the dialog takes focus first.
            request(ui.ctx(), FocusTarget::Id(id("button")));
            crate::runtime::overlay::take_keys_for_new_dialog(ui.ctx());
        }
        if intents_of(ui.ctx(), Some(id("canvas")), false).untaken(&Intent::Cancel) {
            seen.set(seen.get() + 1);
        }
    };
    let mut frames = shown(false, &mut show);
    focus_on(&mut frames, &mut show, "canvas");
    press(&mut frames, &mut show, Key::Escape, Modifiers::NONE);
    assert_eq!(cancels.get(), 1, "with no dialog opening, the focused canvas gets the Escape");
    focus_on(&mut frames, &mut show, "canvas");
    opens.set(true);
    press(&mut frames, &mut show, Key::Escape, Modifiers::NONE);
    assert_eq!(cancels.get(), 1, "the dialog opened in the pass takes it");
}

/// A bare Tab that an app command took from Toggle Panels (the window shortcut yields its default chord to the app's
/// binding) runs that command where Tab would toggle the panels, and still moves focus from a text field. Guards
/// against a Tab gate that only took chrome commands, so an app command bound to Tab never ran.
#[trial]
fn tab_runs_the_app_command_bound_to_it_where_it_would_toggle_the_panels() {
    use crate::runtime::commands::{AppCatalog, Binding, CommandDef, Scope};
    let catalog = Binding::parse("Tab")
        .ok()
        .and_then(|binding| AppCatalog::new(vec![CommandDef::new("trialTab.dock", "Dock", Scope::app()).with_default(binding)], 1).ok());
    assert!(catalog.is_some(), "the trial's catalog is valid");
    let mut show = with_canvas;
    let mut frames = Frames::new();
    frames.root({
        let mut root = RootEnv::default();
        root.tab_toggles_chrome = true;
        root
    });
    if let Some(catalog) = catalog {
        frames.commands(CommandInput::new().catalog(catalog));
    }
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    let mut tab = |frames: &mut Frames| {
        frames.frame(vec![key_press(Key::Tab, Modifiers::NONE), key_release(Key::Tab, Modifiers::NONE)], &mut show);
        let out = take_output(&frames.ctx);
        (out.chrome_toggles, out.fired.into_iter().map(|fired| fired.command.id).collect::<Vec<_>>())
    };
    assert_eq!(tab(&mut frames), (Vec::new(), vec!["trialTab.dock".to_owned()]), "nothing focused: the app's command runs");
    assert_eq!(egui_focused(&frames.ctx), None, "and the press moved no focus");
    focus_on(&mut frames, &mut with_canvas, "field");
    assert!(tab(&mut frames).1.is_empty(), "a text field keeps Tab as its focus move");
}
