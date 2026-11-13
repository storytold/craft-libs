//! Trials of the keys and presses a host's own egui widgets keep while the runtime is installed on the whole window.

use crate::runtime::commands::{CommandInput, CommandSet as _, EditCommands, NavCommand};
use crate::runtime::drag::{self, DragPreview};
use crate::runtime::egui::fixture;
use crate::runtime::env::RootEnv;
use crate::runtime::focus::{EditSet, FocusTarget, KeepKeys, KeyUse, host_canvas, request};
use crate::runtime::frame::{ChromeToggle, take_output};
use crate::runtime::keys::with_window_intents;
use crate::testing::focus::{Stop, egui_focused, named_id, register_stop, widget};
use crate::testing::input::{key_press, key_reached, slop};
use crate::testing::pointer::{self, moved, press, release};
use crate::testing::sensing::{self, ProbeRoot};
use crate::testing::{Frames, trial};
use craft_behavior::{EditCommand, Input, Intent, Sense};
use egui::{Event, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Ui, ViewportId};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn field() -> Id {
    named_id("host field")
}

/// What the host's own UI saw in its passes.
struct Host {
    text: String,
    rect: Rect,
    /// The keys of `WATCHED` that reached egui's input.
    reached: Vec<Key>,
    /// The window `Cancel` intents the host took.
    cancels: usize,
    /// Start a framework drag session in the next pass.
    start_drag: bool,
}

const WATCHED: [Key; 3] = [Key::Tab, Key::Escape, Key::F6];

/// What the host saw since the last call: the keys that reached egui and the window cancels it took.
fn seen(host: &RefCell<Host>) -> (Vec<Key>, usize) {
    let mut host = host.borrow_mut();
    (std::mem::take(&mut host.reached), std::mem::take(&mut host.cancels))
}

fn press_key(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), key: Key) {
    frames.frame(vec![key_press(key, Modifiers::NONE)], &mut *show);
}

/// A window with a framework button and the host's own egui text field, with what the host saw.
fn host_window() -> (Rc<RefCell<Host>>, impl FnMut(&mut Ui)) {
    let host = Rc::new(RefCell::new(Host { text: String::new(), rect: Rect::NOTHING, reached: Vec::new(), cancels: 0, start_drag: false }));
    let shared = Rc::clone(&host);
    let show = move |ui: &mut Ui| {
        let mut host = shared.borrow_mut();
        let host = &mut *host;
        host.reached.extend(WATCHED.into_iter().filter(|key| key_reached(ui.ctx(), *key)));
        widget(ui, named_id("button"));
        register_stop(ui.ctx(), Stop::new(named_id("button"), KeyUse::of(Input::Pointer)));
        host.rect = fixture::host_field(ui, field(), &mut host.text);
        if std::mem::take(&mut host.start_drag) {
            drag::start(ui.ctx(), named_id("button"), Arc::new(()), None, DragPreview::None);
        }
        if with_window_intents(ui.ctx(), false, |intents| intents.take(Intent::Cancel)) {
            host.cancels += 1;
        }
    };
    (host, show)
}

/// A click at `at`, with the pointer moved there first; returns egui's focus after the press and after the release.
fn click_focus(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), at: Pos2) -> (Option<Id>, Option<Id>) {
    frames.frame(vec![moved(at)], &mut *show);
    frames.frame(vec![press(at)], &mut *show);
    let pressed = egui_focused(&frames.ctx);
    frames.frame(vec![release(at)], &mut *show);
    frames.frame(Vec::new(), &mut *show);
    (pressed, egui_focused(&frames.ctx))
}

#[trial]
fn an_app_widget_keeps_or_surrenders_focus_by_egui_s_rule() {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    assert!(fixture::app_surrenders_on_click(&frames.ctx), "the runtime leaves the app's surrender option alone");
    let rect = host.borrow().rect;
    let outside = rect.left_bottom() + egui::vec2(0.0, rect.height() * 3.0);
    fixture::host_focus(&frames.ctx, field());
    frames.frame(Vec::new(), &mut show);
    assert_eq!(click_focus(&mut frames, &mut show, rect.center()), (Some(field()), Some(field())), "a click inside the field keeps its focus");
    assert_eq!(
        click_focus(&mut frames, &mut show, outside),
        (Some(field()), None),
        "a click no widget takes ends the field's focus by egui's rule: at the release, not at the press"
    );
    fixture::set_app_surrender(&frames.ctx, true);
    fixture::host_focus(&frames.ctx, field());
    frames.frame(Vec::new(), &mut show);
    assert_eq!(click_focus(&mut frames, &mut show, outside), (Some(field()), Some(field())), "the app's rule keeps it");
}

#[trial]
fn the_framework_s_focus_moves_only_by_its_press_rule() {
    let probe_rect = Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(160.0, 40.0));
    let (_, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        sensing::show(ui, &[ProbeRoot::new("probe", probe_rect, Sense::CLICK)]);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    assert!(fixture::app_surrenders_on_click(&frames.ctx));
    let button = named_id("button");
    request(&frames.ctx, FocusTarget::Id(button));
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(button));
    assert_eq!(
        click_focus(&mut frames, &mut show, probe_rect.center()),
        (Some(button), Some(button)),
        "a click on a control that takes no focus leaves the framework's focus, whatever egui's option"
    );
    let empty = probe_rect.left_bottom() + egui::vec2(0.0, 100.0);
    assert_eq!(click_focus(&mut frames, &mut show, empty).0, None, "a press no component takes drops the framework's focus at the press");
}

#[trial]
fn a_focused_egui_field_takes_the_ime() {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    fixture::host_focus(&frames.ctx, field());
    frames.frame(Vec::new(), &mut show);
    frames.frame(vec![Event::Ime(ImeEvent::Preedit { text: "世".to_owned(), active_range_chars: None })], &mut show);
    assert_eq!(frames.ime().map(|ime| ime.owner), Some(field()), "the field writes the pass's IME output");
    frames.frame(vec![Event::Ime(ImeEvent::Commit("世".to_owned()))], &mut show);
    assert_eq!(host.borrow().text, "世", "the commit reaches the field");
}

#[trial]
fn a_focused_egui_field_keeps_tab_escape_and_the_clipboard() {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (Vec::new(), 1), "with nothing focused, Escape is the window's Cancel");
    fixture::host_focus(&frames.ctx, field());
    frames.frame(Vec::new(), &mut show);
    frames.frame(vec![Event::Paste("x".to_owned())], &mut show);
    assert_eq!(host.borrow().text, "x", "the paste reaches the focused field");
    press_key(&mut frames, &mut show, Key::Tab);
    assert_eq!(seen(&host), (vec![Key::Tab], 0), "the field's Tab is egui's");
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(named_id("button")), "egui's Tab moves focus on to the next widget");
    fixture::host_focus(&frames.ctx, field());
    frames.frame(Vec::new(), &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (vec![Key::Escape], 0), "the field's Escape is egui's, and no window Cancel");
}

#[trial]
fn an_open_egui_popup_keeps_escape() {
    let popup = named_id("host popup");
    let (host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        fixture::host_popup(ui, popup, Pos2::ZERO);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    fixture::open_host_popup(&frames.ctx, popup);
    frames.frame(Vec::new(), &mut show);
    assert!(fixture::host_popup_open(&frames.ctx, popup));
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (vec![Key::Escape], 0), "the popup's Escape is egui's");
    assert!(!fixture::host_popup_open(&frames.ctx, popup), "egui closed the popup");
    frames.frame(Vec::new(), &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (Vec::new(), 1), "with the popup closed, Escape is the window's Cancel again");
}

#[trial]
fn escape_cancels_a_framework_drag_while_an_egui_field_has_focus() {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    fixture::host_focus(&frames.ctx, field());
    let inside = host.borrow().rect.center();
    frames.frame(vec![moved(inside)], &mut show);
    frames.frame(vec![press(inside)], &mut show);
    host.borrow_mut().start_drag = true;
    frames.frame(Vec::new(), &mut show);
    assert!(drag::running(&frames.ctx), "the drag runs while the button is held");
    seen(&host);
    press_key(&mut frames, &mut show, Key::Escape);
    assert!(!drag::running(&frames.ctx), "Escape cancelled the drag");
    assert_eq!(seen(&host), (Vec::new(), 0), "the Escape went to the drag alone");
    frames.frame(vec![release(inside)], &mut show);
}

#[trial]
fn arrows_and_tab_move_egui_focus_among_host_buttons() {
    let (left, right) = (named_id("host left"), named_id("host right"));
    let (_host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        fixture::host_button(ui, left, Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(80.0, 30.0)));
        fixture::host_button(ui, right, Rect::from_min_size(Pos2::new(420.0, 300.0), egui::vec2(80.0, 30.0)));
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    fixture::host_focus(&frames.ctx, left);
    frames.idle(2, &mut show);
    press_key(&mut frames, &mut show, Key::ArrowRight);
    assert_eq!(egui_focused(&frames.ctx), Some(right), "ArrowRight moves egui focus to the host's next button");
    press_key(&mut frames, &mut show, Key::ArrowLeft);
    assert_eq!(egui_focused(&frames.ctx), Some(left), "ArrowLeft moves it back");
    press_key(&mut frames, &mut show, Key::Tab);
    assert_eq!(egui_focused(&frames.ctx), Some(right), "Tab moves egui focus on");
    frames.frame(vec![key_press(Key::Tab, Modifiers::SHIFT)], &mut show);
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(left), "Shift+Tab moves it back");

    request(&frames.ctx, FocusTarget::Id(named_id("button")));
    frames.idle(2, &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(named_id("button")));
    press_key(&mut frames, &mut show, Key::ArrowRight);
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(named_id("button")), "egui's arrows leave a framework stop's focus alone");
}

/// What the window shows before the app's own egui buttons.
#[derive(Clone, Copy)]
enum Before {
    Nothing,
    /// A framework focus stop.
    Stop,
    /// A component's egui widget that registered no stop, as a disabled component's.
    Unstopped,
    /// The same widget, holding egui focus.
    FocusedUnstopped,
}

/// A window of the app's own egui buttons after `before`: where egui focus is after a bare Tab or Shift+Tab from
/// nothing focused, and whether the press reached egui's input.
fn tab_from_nothing(before: Before, shift: bool) -> (Option<Id>, bool) {
    let (first, last) = (named_id("host first"), named_id("host last"));
    let reached = Rc::new(RefCell::new(false));
    let seen_tab = Rc::clone(&reached);
    let unstopped = Rc::new(std::cell::Cell::new(None));
    let shown = Rc::clone(&unstopped);
    let mut show = move |ui: &mut Ui| {
        *seen_tab.borrow_mut() |= key_reached(ui.ctx(), Key::Tab);
        match before {
            Before::Nothing => {}
            Before::Stop => {
                widget(ui, named_id("button"));
                register_stop(ui.ctx(), Stop::new(named_id("button"), KeyUse::of(Input::Pointer)));
            }
            Before::Unstopped | Before::FocusedUnstopped => {
                let root = ProbeRoot::new("unstopped", Rect::from_min_size(Pos2::new(100.0, 300.0), egui::vec2(80.0, 30.0)), Sense::CLICK);
                shown.set(sensing::show(ui, &[root]).first().map(|report| report.root.id));
            }
        }
        fixture::host_button(ui, first, Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(80.0, 30.0)));
        fixture::host_button(ui, last, Rect::from_min_size(Pos2::new(420.0, 300.0), egui::vec2(80.0, 30.0)));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    if matches!(before, Before::FocusedUnstopped) {
        fixture::host_focus(&frames.ctx, unstopped.get().expect("the widget was shown"));
        frames.idle(2, &mut show);
    }
    assert_eq!(egui_focused(&frames.ctx), unstopped.get().filter(|_| matches!(before, Before::FocusedUnstopped)));
    let mods = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
    frames.frame(vec![key_press(Key::Tab, mods)], &mut show);
    let reached = *reached.borrow();
    frames.frame(Vec::new(), &mut show);
    (egui_focused(&frames.ctx), reached)
}

#[trial]
fn tab_from_nothing_focused_in_a_window_without_stops_moves_egui_focus() {
    let (first, last) = (Some(named_id("host first")), Some(named_id("host last")));
    assert_eq!(tab_from_nothing(Before::Nothing, false), (first, true), "Tab gives egui focus to the first host button");
    assert_eq!(tab_from_nothing(Before::Nothing, true), (last, true), "Shift+Tab to the last");
    assert_eq!(tab_from_nothing(Before::Stop, false), (Some(named_id("button")), false), "with a framework stop, Tab is the framework's move");
    assert_eq!(tab_from_nothing(Before::Unstopped, false), (None, true), "egui's Tab gives no focus to a component's widget without a stop");
    let (focused, reached) = tab_from_nothing(Before::FocusedUnstopped, false);
    assert!(focused.is_some() && focused != first && !reached, "egui focus on that widget is focus: the Tab is not egui's and moves nothing");
}

#[trial]
fn escape_ends_an_egui_drag_on_a_host_widget() {
    let area = named_id("host drag area");
    let rect = Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(120.0, 120.0));
    let drag = Rc::new(RefCell::new((false, false)));
    let (host, mut base) = host_window();
    let shared = Rc::clone(&drag);
    let mut show = move |ui: &mut Ui| {
        base(ui);
        *shared.borrow_mut() = fixture::host_drag_area(ui, area, rect);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    let start = rect.center();
    frames.frame(vec![moved(start)], &mut show);
    frames.frame(vec![press(start)], &mut show);
    // Far past any click distance, so egui's drag has started.
    let far = start + egui::vec2(rect.width() / 3.0, 0.0);
    frames.frame(vec![moved(far)], &mut show);
    frames.frame(vec![moved(far + egui::vec2(1.0, 0.0))], &mut show);
    assert_eq!(*drag.borrow(), (true, false), "the host's egui drag runs");
    seen(&host);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(*drag.borrow(), (false, true), "the Escape ended egui's drag in its own pass");
    assert_eq!(seen(&host), (Vec::new(), 1), "the window took the Escape as its Cancel");
    frames.frame(vec![moved(far - egui::vec2(rect.width() / 3.0, 0.0))], &mut show);
    assert!(!drag.borrow().0, "the held button does not restart the drag");
    frames.frame(vec![release(far)], &mut show);
}

/// A press at `at`, held `held` seconds and released `by` points to the right: whether the host's egui button and the
/// framework root reported a click.
fn click_gesture(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), clicks: &RefCell<(bool, bool)>, at: Pos2, by: f32, held: f64) -> (bool, bool) {
    *clicks.borrow_mut() = (false, false);
    let to = at + egui::vec2(by, 0.0);
    frames.frame(vec![moved(at)], &mut *show);
    frames.frame(vec![press(at)], &mut *show);
    frames.advance(held);
    frames.frame(vec![moved(to)], &mut *show);
    frames.frame(vec![release(to)], &mut *show);
    frames.frame(Vec::new(), &mut *show);
    *clicks.borrow()
}

#[trial]
fn the_app_keeps_its_click_options_and_a_framework_root_clicks_by_its_own() {
    let host_rect = Rect::from_min_size(Pos2::new(300.0, 300.0), egui::vec2(160.0, 40.0));
    let probe_rect = Rect::from_min_size(Pos2::new(300.0, 400.0), egui::vec2(160.0, 40.0));
    let clicks = Rc::new(RefCell::new((false, false)));
    let shared = Rc::clone(&clicks);
    let mut show = move |ui: &mut Ui| {
        let host = fixture::host_button(ui, named_id("host button"), host_rect);
        let reports = sensing::show(ui, &[ProbeRoot::new("probe", probe_rect, Sense::CLICK)]);
        let mut seen = shared.borrow_mut();
        seen.0 |= host;
        seen.1 |= reports.first().is_some_and(|report| report.root.clicked);
    };
    let mut frames = Frames::new();
    let reach = slop(&frames.ctx);
    // The app's own rule for its widgets: a longer reach than the framework's slop and a short press.
    let (distance, seconds) = (4.0 * reach, 0.3);
    fixture::set_click_options(&frames.ctx, distance, seconds);
    frames.frame(Vec::new(), &mut show);
    assert_eq!(fixture::click_options(&frames.ctx), (distance, seconds), "the runtime leaves the app's click options alone");

    let long = 2.0 * seconds;
    assert_eq!(click_gesture(&mut frames, &mut show, &clicks, host_rect.center(), 0.0, 0.0), (true, false), "a quick press clicks the host button");
    assert_eq!(click_gesture(&mut frames, &mut show, &clicks, probe_rect.center(), 0.0, 0.0), (false, true), "a quick press clicks the root");
    assert!(!click_gesture(&mut frames, &mut show, &clicks, host_rect.center(), 0.0, long).0, "a long press is no click by the app's rule");
    assert!(click_gesture(&mut frames, &mut show, &clicks, probe_rect.center(), 0.0, long).1, "the root clicks whatever the press's duration");
    let between = 2.0 * reach;
    assert!(click_gesture(&mut frames, &mut show, &clicks, host_rect.center(), between, 0.0).0, "the app's reach is its own");
    assert!(!click_gesture(&mut frames, &mut show, &clicks, probe_rect.center(), between, 0.0).1, "the root's reach is the framework's slop");
    assert_eq!(fixture::click_options(&frames.ctx), (distance, seconds), "and stays so");
}

#[trial]
fn copy_with_a_label_selection_copies_the_label() {
    const TEXT: &str = "host label text";
    let label = Rc::new(std::cell::Cell::new(Rect::NOTHING));
    let at = Rc::clone(&label);
    let (_, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        at.set(fixture::host_label(ui, TEXT));
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    frames.frame(vec![Event::Copy], &mut show);
    assert!(frames.copied().is_empty(), "with no selection, the copy is the app's chord");
    let rect = label.get();
    pointer::drag(&mut frames, rect.left_center() + egui::vec2(1.0, 0.0), egui::vec2(rect.width() - 2.0, 0.0), &mut show);
    frames.frame(vec![Event::Copy], &mut show);
    assert!(frames.copied().iter().any(|copied| !copied.is_empty() && TEXT.contains(copied.as_str())), "egui copied the selection");
}

/// What a bare Tab did in a window whose host declares its canvas with `keys` (`None`: no canvas), with the canvas
/// focused or not: the chrome toggles it asked for, and whether it reached egui.
fn tab_with_canvas(keys: Option<KeyUse>, focused: bool) -> (Vec<(ViewportId, ChromeToggle)>, bool) {
    let canvas = named_id("host canvas");
    let (host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        widget(ui, canvas);
        if let Some(keys) = keys {
            host_canvas(ui.ctx(), canvas, keys);
        }
    };
    let mut frames = Frames::new();
    frames.root({
        let mut root = RootEnv::default();
        root.tab_toggles_chrome = true;
        root
    });
    frames.frame(Vec::new(), &mut show);
    if focused {
        request(&frames.ctx, FocusTarget::Id(canvas));
        frames.frame(Vec::new(), &mut show);
    }
    take_output(&frames.ctx);
    seen(&host);
    let before = egui_focused(&frames.ctx);
    press_key(&mut frames, &mut show, Key::Tab);
    let reached = seen(&host).0.contains(&Key::Tab);
    if reached {
        frames.frame(Vec::new(), &mut show);
        assert_eq!(egui_focused(&frames.ctx), before, "a Tab the canvas keeps moves no egui focus");
    }
    (take_output(&frames.ctx).chrome_toggles, reached)
}

#[trial]
fn a_host_canvas_marks_the_window_and_tab_toggles_its_panels() {
    assert_eq!(tab_with_canvas(None, false), (Vec::new(), false), "no canvas: Tab moves focus");
    assert_eq!(tab_with_canvas(Some(KeyUse::CANVAS), false), (vec![(ViewportId::ROOT, ChromeToggle::Panels)], false), "nothing focused");
    assert_eq!(tab_with_canvas(Some(KeyUse::CANVAS), true), (vec![(ViewportId::ROOT, ChromeToggle::Panels)], false), "the canvas focused");
    assert_eq!(
        tab_with_canvas(Some(KeyUse::CANVAS.with_keep(KeepKeys::Tab)), true),
        (Vec::new(), true),
        "a focused canvas that keeps Tab gets it from egui"
    );
    assert_eq!(
        tab_with_canvas(Some(KeyUse::CANVAS.with_keep(KeepKeys::Tab)), false),
        (Vec::new(), true),
        "with nothing focused, a canvas that keeps Tab gets it from egui and nothing toggles"
    );
}

/// What `event` did with nothing focused in a window whose host declares its canvas with `keys`: whether the event
/// reached egui's input, and the ids of the commands it fired.
fn clipboard_with_canvas(keys: KeyUse, event: Event) -> (bool, Vec<String>) {
    let canvas = named_id("host canvas");
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&events);
    let (_, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        seen.borrow_mut().extend(ui.input(|input| input.events.clone()));
        base(ui);
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, keys);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    events.borrow_mut().clear();
    frames.frame(vec![event.clone()], &mut show);
    frames.frame(Vec::new(), &mut show);
    let fired = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    (events.borrow().contains(&event), fired)
}

#[trial]
fn a_host_canvas_reads_the_clipboard_of_its_edit_set_with_nothing_focused() {
    let edits = EditSet::NONE.with(EditCommand::Cut).with(EditCommand::Copy).with(EditCommand::Paste);
    let typing = KeyUse::CANVAS.with_edits(edits);
    let paste = Event::Paste("x".to_owned());
    assert_eq!(clipboard_with_canvas(typing, paste.clone()), (true, Vec::new()), "the canvas's paste stays in egui and fires nothing");
    assert_eq!(clipboard_with_canvas(typing, Event::Cut), (true, Vec::new()), "and its cut");
    assert!(!clipboard_with_canvas(KeyUse::CANVAS, paste).0, "a canvas without the command: the paste is the app's chord");
    let chord = key_press(Key::V, Modifiers::COMMAND);
    assert_eq!(clipboard_with_canvas(typing, chord.clone()), (true, Vec::new()), "the canvas's chord stays in egui and fires nothing");
    assert!(!clipboard_with_canvas(KeyUse::CANVAS, chord).1.is_empty(), "a canvas without the command: the chord fires the app's Paste");
}

/// What an F6 press did under `overrides`: whether it reached egui, and the ids of the commands it fired.
fn f6_under(overrides: Option<craft_keymap::Overrides>) -> (bool, Vec<String>) {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    if let Some(overrides) = overrides {
        frames.commands(CommandInput::new().overrides(Arc::new(overrides)));
    }
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    seen(&host);
    press_key(&mut frames, &mut show, Key::F6);
    let fired = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    (seen(&host).0.contains(&Key::F6), fired)
}

#[trial]
fn an_app_binding_on_f6_beats_the_pane_move() {
    assert_eq!(f6_under(None), (false, Vec::new()), "by default F6 is the window's pane move");
    let mut bound = craft_keymap::Overrides::default();
    bound.insert(EditCommands::Delete.id(), vec!["F6".to_owned()]);
    assert_eq!(f6_under(Some(bound)), (true, vec![EditCommands::Delete.id().to_owned()]), "a command bound to F6 takes it");
    let mut unbound = craft_keymap::Overrides::default();
    unbound.insert(NavCommand::NextPane.id(), Vec::new());
    assert_eq!(f6_under(Some(unbound)), (true, Vec::new()), "with the pane move unbound, F6 is egui's");
}
