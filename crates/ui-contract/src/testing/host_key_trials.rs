//! Trials of the keys and presses a host's own egui widgets keep while the runtime is installed on the whole window.

use crate::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, ShowContainer as _};
use crate::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandSet as _, EditCommands, NavCommand, Scope, TypingRule};
use crate::runtime::drag::{self, DragPreview};
use crate::runtime::egui::{areas, fixture};
use crate::runtime::env::{Env, RootEnv};
use crate::runtime::focus::{EditSet, FocusPolicy, FocusTarget, KeepKeys, KeyUse, host_canvas, request};
use crate::runtime::frame::{ChromeToggle, take_output};
use crate::runtime::keys::with_window_intents;
use crate::style::{ComponentStyle, StyleCx};
use crate::testing::focus::{Stop, egui_focused, key_release, named_id, pop_scope, push_scope, register_stop, widget};
use crate::testing::input::{key_press, key_reached, slop};
use crate::testing::pointer::{self, moved, press, release};
use crate::testing::sensing::{self, ProbeRoot};
use crate::testing::{Frames, trial};
use craft_behavior::{EditCommand, Input, Intent, Metrics, Sense};
use egui::{Event, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Response, Ui, ViewportId};
use std::cell::{Cell, RefCell};
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

/// A press of `key` and its release, in one frame: a second press with no release between is a repeat.
fn press_key(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), key: Key) {
    frames.frame(vec![key_press(key, Modifiers::NONE), key_release(key, Modifiers::NONE)], &mut *show);
}

#[trial]
fn escape_unused_by_a_focused_control_reaches_the_window() {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(named_id("button")));
    frames.idle(2, &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host).1, 1, "the focused button leaves Escape for the window");
}

#[trial]
fn a_canvas_cannot_take_escape_before_an_unrelated_focused_control() {
    let (host, mut base) = host_window();
    let stolen = Rc::new(std::cell::Cell::new(false));
    let canvas_seen = Rc::clone(&stolen);
    let mut show = move |ui: &mut Ui| {
        if crate::runtime::keys::with_canvas_window_intents(ui.ctx(), false, |intents| intents.take(Intent::Cancel)) {
            canvas_seen.set(true);
        }
        base(ui);
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(named_id("button")));
    frames.idle(2, &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert!(!stolen.get());
    assert_eq!(seen(&host).1, 1);
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
            drag::start(ui.ctx(), named_id("button"), &Env::of(ui).view(), Arc::new(()), Vec::new(), DragPreview::None);
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
        areas::host_popup(ui, popup, Pos2::ZERO);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    areas::open_host_popup(&frames.ctx, popup);
    frames.frame(Vec::new(), &mut show);
    assert!(areas::host_popup_open(&frames.ctx, popup));
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (vec![Key::Escape], 0), "the popup's Escape is egui's");
    assert!(!areas::host_popup_open(&frames.ctx, popup), "egui closed the popup");
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
    if matches!(before, Before::FocusedUnstopped)
        && let Some(id) = unstopped.get()
    {
        fixture::host_focus(&frames.ctx, id);
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

/// Which of the arrow keys, Home and End reached egui's input, one press each, in a window whose host declares a canvas;
/// with `focus_field` the host's text field has the focus; and whether Left moved the field's caret.
fn navigation_keys_with_canvas(focus_field: bool) -> (Vec<Key>, bool) {
    const KEYS: [Key; 6] = [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End];
    let canvas = named_id("host canvas");
    let reached = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&reached);
    let (host, mut base) = host_window();
    host.borrow_mut().text = "abc".to_owned();
    let mut show = move |ui: &mut Ui| {
        seen.borrow_mut().extend(KEYS.into_iter().filter(|key| key_reached(ui.ctx(), *key)));
        base(ui);
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    if focus_field {
        fixture::host_focus(&frames.ctx, field());
        frames.frame(Vec::new(), &mut show);
        frames.frame(vec![key_press(Key::End, Modifiers::NONE), key_release(Key::End, Modifiers::NONE)], &mut show);
    }
    reached.borrow_mut().clear();
    let caret = |frames: &Frames| {
        egui::TextEdit::load_state(&frames.ctx, field()).and_then(|state| state.cursor.char_range()).map(|range| range.primary.index)
    };
    let at_end = caret(&frames);
    let mut moved = false;
    for key in KEYS {
        press_key(&mut frames, &mut show, key);
        if key == Key::ArrowLeft {
            moved = caret(&frames) != at_end;
        }
    }
    let mut reached = reached.take();
    reached.dedup();
    (reached, moved)
}

#[trial]
fn a_host_canvas_leaves_navigation_keys_in_egui_for_the_app_s_widget() {
    const KEYS: [Key; 6] = [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End];
    let (reached, caret) = navigation_keys_with_canvas(true);
    assert_eq!(reached, KEYS, "a focused app field reads the arrows, Home and End from egui");
    assert!(caret, "the field's caret moved left from the end");
    assert_eq!(navigation_keys_with_canvas(false).0, KEYS, "with nothing focused, the host canvas's own keys stay in egui");
}

/// A clipboard with a picture and no text sends a paste with no text: it fires the app's Paste as the chord does, and
/// `textless_paste` reads it in that pass only.
#[trial]
fn a_paste_with_no_text_is_the_paste_chord_and_reads_as_textless() {
    let picture = Event::PasteImage(Arc::new(crate::testing::pictures::sample_picture(1, [1, 1])));
    assert!(!clipboard_with_canvas(KeyUse::CANVAS, picture.clone()).1.is_empty(), "the picture's paste fires the app's Paste");
    let read = Rc::new(Cell::new(false));
    let seen = Rc::clone(&read);
    let (_, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        seen.set(crate::runtime::keys::textless_paste(ui.ctx()));
        base(ui);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    frames.frame(vec![Event::Paste("x".to_owned())], &mut show);
    assert!(!read.get(), "a paste with text");
    frames.frame(vec![picture], &mut show);
    assert!(read.get(), "a paste with no text");
    frames.frame(Vec::new(), &mut show);
    assert!(!read.get(), "the pass after it");
}

/// A pasted image is the same Paste as pasted text: in a window whose canvas does not take it, the app's Paste runs,
/// and the app reads the image; a canvas that lists Paste reads it from egui, as it does text.
#[trial]
fn a_pasted_image_runs_the_paste_a_pasted_text_runs() {
    // The pixels are image data the trial never reads; any grey stands for them.
    let image = Event::PasteImage(Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::from_gray(128))));
    assert_eq!(clipboard_with_canvas(KeyUse::CANVAS, image.clone()).1, [EditCommands::Paste.id()], "the app's Paste runs");
    let typing = KeyUse::CANVAS.with_edits(EditSet::NONE.with(EditCommand::Paste));
    assert_eq!(clipboard_with_canvas(typing, image), (true, Vec::new()), "the canvas's paste stays in egui and fires nothing");
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

/// What a press of Escape with `mods` did with nothing focused, under `overrides`: the window cancels it raised and the
/// ids of the commands it fired.
fn escape_under(overrides: Option<craft_keymap::Overrides>, mods: Modifiers) -> (usize, Vec<String>) {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    if let Some(overrides) = overrides {
        frames.commands(CommandInput::new().overrides(Arc::new(overrides)));
    }
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    seen(&host);
    frames.frame(vec![key_press(Key::Escape, mods), key_release(Key::Escape, mods)], &mut show);
    frames.frame(Vec::new(), &mut show);
    let fired = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    (seen(&host).1, fired)
}

/// Tb-8 (3): Shift+Escape bound to a command runs it, as production's keymap ran it under a loaded place cursor; the
/// key stage read every Escape without Cmd or Ctrl as the scope's escape, so the binding never fired. Plain Escape keeps
/// its meaning, and an unbound Shift+Escape is still an escape.
#[trial]
fn a_command_bound_to_shift_escape_runs_and_plain_escape_still_cancels() {
    let delete = EditCommands::Delete.id().to_owned();
    assert_eq!(escape_under(None, Modifiers::SHIFT), (1, Vec::new()), "an unbound Shift+Escape is the window's Cancel");
    let mut bound = craft_keymap::Overrides::default();
    bound.insert(EditCommands::Delete.id(), vec!["Shift+Escape".to_owned()]);
    assert_eq!(escape_under(Some(bound.clone()), Modifiers::SHIFT), (0, vec![delete]), "the bound Shift+Escape runs its command");
    assert_eq!(escape_under(Some(bound), Modifiers::NONE), (1, Vec::new()), "a plain Escape is still the window's Cancel");
}

/// The panes the pane trials show, in order: the scope's name, its stop and its label.
const PANES: [(&str, &str, &str); 2] = [("left pane", "left", "Left"), ("right pane", "right", "Right")];

/// Shows the two panes of `PANES`, each a pane scope holding one button stop.
fn show_panes(ui: &mut Ui) {
    for (pane, stop, label) in PANES {
        push_scope(ui.ctx(), named_id(pane), FocusPolicy::PANE, Some(Arc::from(label)));
        widget(ui, named_id(stop));
        register_stop(ui.ctx(), Stop::new(named_id(stop), KeyUse::of(Input::Pointer)));
        pop_scope(ui.ctx());
    }
}

/// A press of F6 with `mods` and the frame after it: where egui focus is then, and what the move announced.
fn pane_key(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), mods: Modifiers) -> (Option<Id>, Vec<String>) {
    take_output(&frames.ctx);
    frames.frame(vec![key_press(Key::F6, mods)], &mut *show);
    frames.frame(Vec::new(), &mut *show);
    (egui_focused(&frames.ctx), take_output(&frames.ctx).announcements)
}

/// Focuses the stop `id` and settles one frame.
fn focus_stop(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), id: Id) {
    request(&frames.ctx, FocusTarget::Id(id));
    frames.frame(Vec::new(), &mut *show);
}

/// A dialog's focus trap (`FocusPolicy::DIALOG`, the scope a modal dialog's layer pushes) holds F6: the pane move would
/// carry focus out of the dialog, so F6 and Shift+F6 move nothing and announce nothing while the trap is shown. The trap
/// is the focus scope alone, with no modal layer, so only the pane move's own guard can keep the focus in it.
#[trial]
fn f6_inside_a_modal_dialog_keeps_the_focus_in_it() {
    let open = Rc::new(Cell::new(false));
    let shown = Rc::clone(&open);
    let ok = named_id("dialog ok");
    let mut show = move |ui: &mut Ui| {
        show_panes(ui);
        if shown.get() {
            push_scope(ui.ctx(), named_id("dialog"), FocusPolicy::DIALOG, Some(Arc::from("Dialog")));
            widget(ui, ok);
            register_stop(ui.ctx(), Stop::new(ok, KeyUse::of(Input::Pointer)));
            pop_scope(ui.ctx());
        }
    };
    let (left, right) = (named_id("left"), named_id("right"));
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    focus_stop(&mut frames, &mut show, left);
    let (at, said) = pane_key(&mut frames, &mut show, Modifiers::NONE);
    assert_eq!(at, Some(right), "outside a dialog F6 moves to the next pane");
    assert!(said.iter().any(|text| text == "Right"), "and names it: {said:?}");

    open.set(true);
    frames.frame(Vec::new(), &mut show);
    focus_stop(&mut frames, &mut show, ok);
    assert_eq!(egui_focused(&frames.ctx), Some(ok), "the dialog's button has the focus");
    assert_eq!(pane_key(&mut frames, &mut show, Modifiers::NONE), (Some(ok), Vec::new()), "F6 keeps the focus in the dialog and says nothing");
    assert_eq!(pane_key(&mut frames, &mut show, Modifiers::SHIFT), (Some(ok), Vec::new()), "so does Shift+F6");

    open.set(false);
    frames.frame(Vec::new(), &mut show);
    focus_stop(&mut frames, &mut show, left);
    assert_eq!(pane_key(&mut frames, &mut show, Modifiers::NONE).0, Some(right), "with the dialog gone F6 moves panes again");
}

/// The app command the routing trial presses, from the app's catalog so that no other trial's keymap binds its key.
const REPLACE: &str = "hostKeyTrial.replace";

/// An app catalog binding `REPLACE` to the plain key Q. Its modal rule is the default one, `Blocked`: a modal dialog
/// with the keys keeps it from running. `None` when the definition is refused.
fn replace_catalog() -> Option<AppCatalog> {
    let binding = Binding::parse("Q").ok()?;
    AppCatalog::new(vec![CommandDef::new(REPLACE, "Replace", Scope::app()).with_default(binding)], 1).ok()
}

/// The style of a `TrialDialog`: its size comes from its content.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct TrialDialogStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A modal dialog layer (`Overlay::dialog`), whose body the trial fills.
#[derive(Component)]
#[component(container, unregistered)]
struct TrialDialog {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for TrialDialog {
    const ROLE: Role = Role::GROUP;
    type Style = TrialDialogStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TrialDialogStyleKey) -> TrialDialogStyle {
        TrialDialogStyle { metrics: Metrics::new() }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for TrialDialog {}

/// S-X7: a window shortcut runs while the keyboard is on a pane's title, as after F6, and runs neither from a button of
/// an open modal dialog (the dialog owns the keys, and the command's modal rule blocks it) nor from a text field (a plain
/// key there is typing).
#[trial]
fn a_window_shortcut_runs_from_a_pane_title_and_not_from_a_modal_dialog_or_a_text_field() {
    let (title, field, button) = (named_id("pane title"), named_id("pane field"), named_id("dialog button"));
    let open = Rc::new(Cell::new(false));
    let shown = Rc::clone(&open);
    let mut show = move |ui: &mut Ui| {
        push_scope(ui.ctx(), named_id("pane"), FocusPolicy::PANE, Some(Arc::from("Pane")));
        widget(ui, title);
        register_stop(ui.ctx(), Stop::new(title, KeyUse::of(Input::Pointer)));
        widget(ui, field);
        register_stop(ui.ctx(), Stop::new(field, KeyUse::of(Input::Text)));
        pop_scope(ui.ctx());
        if shown.get() {
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::dialog()));
            TrialDialog { common: Common::new(), props }.id_salt("trial dialog").show(ui, |ui, _| {
                widget(ui, button);
                register_stop(ui.ctx(), Stop::new(button, KeyUse::of(Input::Pointer)));
            });
        }
    };
    // The ids of the commands a press of the shortcut's key fired.
    let fired = |frames: &mut Frames, show: &mut dyn FnMut(&mut Ui)| -> Vec<String> {
        take_output(&frames.ctx);
        frames.frame(vec![key_press(Key::Q, Modifiers::NONE), key_release(Key::Q, Modifiers::NONE)], &mut *show);
        frames.frame(Vec::new(), &mut *show);
        take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect()
    };
    let replace = vec![REPLACE.to_owned()];
    let catalog = replace_catalog();
    assert!(catalog.is_some(), "the trial's catalog is valid");
    let mut frames = Frames::new();
    if let Some(catalog) = catalog {
        frames.commands(CommandInput::new().catalog(catalog));
    }
    frames.frame(Vec::new(), &mut show);

    focus_stop(&mut frames, &mut show, title);
    assert_eq!(egui_focused(&frames.ctx), Some(title));
    assert_eq!(fired(&mut frames, &mut show), replace, "the pane's title leaves the shortcut to the window");

    focus_stop(&mut frames, &mut show, field);
    assert_eq!(egui_focused(&frames.ctx), Some(field));
    assert_eq!(fired(&mut frames, &mut show), Vec::<String>::new(), "a text field keeps the plain key for its typing");

    focus_stop(&mut frames, &mut show, title);
    open.set(true);
    frames.idle(2, &mut show);
    focus_stop(&mut frames, &mut show, button);
    assert_eq!(egui_focused(&frames.ctx), Some(button), "the dialog's button has the keyboard");
    assert_eq!(fired(&mut frames, &mut show), Vec::<String>::new(), "a modal dialog keeps the window's shortcut from running");

    open.set(false);
    frames.idle(2, &mut show);
    focus_stop(&mut frames, &mut show, title);
    assert_eq!(fired(&mut frames, &mut show), replace, "with the dialog closed the title runs it again");
}

/// The app command the typing-rule trial presses: Alt+Q, as a word processor's search.
const SEARCH: &str = "hostKeyTrial.search";

/// The ids of the commands an Alt+`key` press, bound to `SEARCH`, fired while a text stop has the keyboard, with the
/// text the platform sends with it, under the app catalog's typing rule `rule` (`None`: the keymap's default).
fn alt_while_typing(key: Key, rule: Option<TypingRule>) -> Vec<String> {
    let field = named_id("document");
    let mut show = move |ui: &mut Ui| {
        widget(ui, field);
        register_stop(ui.ctx(), Stop::new(field, KeyUse::of(Input::Text)));
    };
    let Ok(binding) = Binding::parse(&format!("Alt+{}", key.name())) else { return vec!["the trial's chord does not parse".to_owned()] };
    let Ok(catalog) = AppCatalog::new(vec![CommandDef::new(SEARCH, "Search", Scope::app()).with_default(binding)], 1) else {
        return vec!["the trial's catalog is refused".to_owned()];
    };
    let catalog = match rule {
        Some(rule) => catalog.typing_rule(rule),
        None => catalog,
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog));
    frames.frame(Vec::new(), &mut show);
    focus_stop(&mut frames, &mut show, field);
    assert_eq!(egui_focused(&frames.ctx), Some(field), "the text stop has the keyboard");
    take_output(&frames.ctx);
    frames.frame(vec![key_press(key, Modifiers::ALT), Event::Text("x".into())], &mut show);
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect()
}

/// An app sets the typing rule through its catalog: by default an Alt+letter chord waits while a text component has
/// the keyboard (Option types characters on the Mac), and under `PlainKeysWait` it fires from there, as Word's Alt+Q
/// opens search from the page, though the platform sends the key's text with it.
#[trial]
fn the_app_catalog_s_typing_rule_lets_an_alt_chord_fire_while_typing() {
    assert_eq!(alt_while_typing(Key::Q, None), Vec::<String>::new(), "by default the Alt chord stays with the text");
    assert_eq!(alt_while_typing(Key::Q, Some(TypingRule::CommandKeysOnly)), Vec::<String>::new(), "so it does under the default rule named");
    assert_eq!(alt_while_typing(Key::Q, Some(TypingRule::PlainKeysWait)), vec![SEARCH.to_owned()], "under PlainKeysWait it fires");
}

/// The text a platform sends with a bound Alt chord is withdrawn and is not the text field's typing, so the field does
/// not hold the press: an Alt+function-key chord, which fires while typing under every rule but `AllWait`, runs its
/// command from a text field (Word's Alt+F7, next misspelling, from the page).
#[trial]
fn a_bound_alt_chord_s_text_leaves_the_press_to_its_command() {
    assert_eq!(alt_while_typing(Key::F7, None), vec![SEARCH.to_owned()], "Alt+F7 fires from the text field");
}

/// The app command on Alt+Shift+- in the disabled-chord trial: a word processor's Collapse, which works in Outline
/// view only.
const COLLAPSE: &str = "hostKeyTrial.collapse";

/// What Alt+Shift+- does from a text stop, with the em dash the platform types for it in the same frame, while the
/// command bound to it is `enabled`: the commands it fired and the text the stop was given.
fn alt_shift_minus(enabled: bool) -> (Vec<String>, String) {
    let field = named_id("document");
    let typed = RefCell::new(String::new());
    let mut show = |ui: &mut Ui| {
        widget(ui, field);
        register_stop(ui.ctx(), Stop::new(field, KeyUse::of(Input::Text)));
        let mut intents = crate::testing::focus::intents_of(ui.ctx(), Some(field), false);
        while let Some(text) = intents.take_text() {
            typed.borrow_mut().push_str(&text);
        }
    };
    let Ok(binding) = Binding::parse("Alt+Shift+-") else { return (vec!["the trial's chord does not parse".to_owned()], String::new()) };
    let Ok(catalog) = AppCatalog::new(vec![CommandDef::new(COLLAPSE, "Collapse", Scope::app()).with_default(binding)], 1) else {
        return (vec!["the trial's catalog is refused".to_owned()], String::new());
    };
    let state = crate::runtime::commands::CommandState::new().enabled(enabled);
    let mut frames = Frames::new();
    let mut states = crate::runtime::commands::CommandStates::new();
    states.set(COLLAPSE, state);
    frames.commands(CommandInput::new().catalog(catalog.typing_rule(TypingRule::PlainKeysWait)).states(&crate::contract::Tracked::new(states)));
    frames.frame(Vec::new(), &mut show);
    focus_stop(&mut frames, &mut show, field);
    take_output(&frames.ctx);
    typed.borrow_mut().clear();
    let alt_shift = Modifiers { alt: true, shift: true, ..Modifiers::NONE };
    frames.frame(vec![key_press(Key::Minus, alt_shift), Event::Text("\u{2014}".into())], &mut show);
    frames.frame(Vec::new(), &mut show);
    let fired = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    (fired, typed.take())
}

/// fwstub-1 (4): the text of a bound Alt chord is withheld only when its command runs (S-L177, as production's
/// `shortcuts.rs` returned unhandled for a disabled Alt command): with the command disabled the shortcut does nothing
/// and the character is typed, as Word types Option+Shift+-'s em dash outside Outline view; enabled, the command runs
/// and nothing is typed.
#[trial]
fn a_disabled_alt_chord_leaves_its_text_typed() {
    assert_eq!(alt_shift_minus(false), (Vec::new(), "\u{2014}".to_owned()), "disabled: the em dash is typed");
    assert_eq!(alt_shift_minus(true), (vec![COLLAPSE.to_owned()], String::new()), "enabled: the command runs and types nothing");
}
