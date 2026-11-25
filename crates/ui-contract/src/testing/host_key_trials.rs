//! Trials of the keys and presses a host's own egui widgets keep while the runtime is installed on the whole window.

use crate::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, ShowContainer as _};
use crate::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandInput, CommandSet as _, EditCommands, NavCommand, QuitCommand, Scope, TypingRule,
};
use crate::runtime::drag::{self, DragPreview};
use crate::runtime::egui::{areas, fixture};
use crate::runtime::env::{Env, RootEnv};
use crate::runtime::focus::{EditSet, FocusPolicy, FocusTarget, KeepKeys, KeyUse, host_canvas, request};
use crate::runtime::frame::{ChromeToggle, take_output};
use crate::runtime::ime;
use crate::runtime::keys::with_window_intents;
use crate::style::{ComponentStyle, StyleCx};
use crate::testing::contract::findings;
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

fn host_editor_id() -> Id {
    named_id("host editor")
}

/// A window with the app's own IME editor (`fixture::host_ime_editor`), and the text it holds.
fn host_editor() -> (Rc<RefCell<String>>, impl FnMut(&mut Ui)) {
    let text = Rc::new(RefCell::new(String::new()));
    let held = Rc::clone(&text);
    let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(200.0, 30.0));
    (text, move |ui: &mut Ui| fixture::host_ime_editor(ui, host_editor_id(), rect, &mut held.borrow_mut()))
}

/// A press during the app's own editor's composition interrupts it, in the editor's own IME context, and fences the
/// composition's late deletion and commit, which the one-press rule holds for the next frame, until the reset
/// completes; the events after the completion reach the editor.
#[trial]
fn a_press_during_an_app_editor_s_composition_fences_its_late_ime_events() {
    let (editor, mut show) = host_editor();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    frames.frame(vec![Event::Ime(ImeEvent::Commit("ab".to_owned()))], &mut show);
    frames.frame(vec![Event::Ime(ImeEvent::Preedit { text: "z".to_owned(), active_range_chars: None })], &mut show);
    let at = Pos2::new(150.0, 115.0);
    let late = vec![
        moved(at),
        press(at),
        Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }),
        Event::Ime(ImeEvent::Commit("Q".to_owned())),
    ];
    frames.frame(late, &mut show);
    let ime = frames.ime();
    assert!(ime.is_some(), "the editor writes the pass's IME output");
    let Some(ime) = ime else { return };
    assert!(ime.owner == host_editor_id() && ime.interrupt, "the press interrupts the editor's composition in its own context");
    assert!(ime.reset.is_some(), "the interrupt asks for a reset");
    let Some(reset) = ime.reset else { return };
    frames.frame(vec![release(at)], &mut show);
    frames.frame(Vec::new(), &mut show);
    assert_eq!(*editor.borrow(), "ab", "the composition's late deletion and commit never reach the editor");
    frames.frame(vec![Event::ImeResetCompleted { request_id: reset }, Event::Ime(ImeEvent::Commit("X".to_owned()))], &mut show);
    assert_eq!(*editor.borrow(), "abX", "the commit after the completion reaches the editor");
}

/// The app's own resetting editor at its rect, the show closure that draws it, and the frames after two idle ones,
/// a commit of "ab" and a preedit of "z": the editor composes. `in_output`: the editor writes its reset request into
/// its IME output too, instead of leaving it to egui as EffectCraft's editor does.
fn composing_resetting_editor(in_output: bool) -> (Rc<RefCell<fixture::ResettingEditor>>, impl FnMut(&mut Ui), Frames) {
    let editor = Rc::new(RefCell::new(fixture::ResettingEditor { in_output, ..Default::default() }));
    let held = Rc::clone(&editor);
    let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(200.0, 30.0));
    let mut show = move |ui: &mut Ui| fixture::host_resetting_ime_editor(ui, host_editor_id(), rect, &mut held.borrow_mut());
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    frames.frame(vec![Event::Ime(ImeEvent::Commit("ab".to_owned()))], &mut show);
    frames.frame(vec![Event::Ime(ImeEvent::Preedit { text: "z".to_owned(), active_range_chars: None })], &mut show);
    (editor, show, frames)
}

/// The press at `at` with the composition's late deletion and commit after it, in one frame.
fn press_with_late_ime(at: Pos2) -> Vec<Event> {
    vec![
        moved(at),
        press(at),
        Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }),
        Event::Ime(ImeEvent::Commit("Q".to_owned())),
    ]
}

/// An app editor that asks for its own reset at the press that interrupts its composition keeps that request: the
/// platform is asked to answer the app's id, the late events stay fenced, and the completion reaches the editor, which
/// takes the events after it and composes again. The editor asks through egui alone, or writes the request into its
/// output too.
#[trial]
fn an_app_editor_s_own_reset_at_the_press_reaches_it_and_it_composes_again() {
    for in_output in [false, true] {
        let (editor, mut show, mut frames) = composing_resetting_editor(in_output);
        let at = Pos2::new(150.0, 115.0);
        frames.frame(press_with_late_ime(at), &mut show);
        let own = editor.borrow().reset;
        assert!(own.is_some(), "the editor asked for its own reset at the press");
        let reset = frames.ime().and_then(|ime| ime.interrupt.then_some(ime.reset).flatten());
        assert_eq!(reset, own, "the interrupt asks the platform to answer the editor's own request (in_output {in_output})");
        let Some(reset) = reset else { return };
        frames.frame(vec![release(at)], &mut show);
        frames.frame(Vec::new(), &mut show);
        assert_eq!(editor.borrow().text, "ab", "the composition's late deletion and commit never reach the editor");
        frames.frame(vec![Event::ImeResetCompleted { request_id: reset }, Event::Ime(ImeEvent::Commit("X".to_owned()))], &mut show);
        assert_eq!(editor.borrow().reset, None, "the completion of its own request reaches the editor");
        assert_eq!(editor.borrow().text, "abX", "the commit after it reaches the editor");
        frames.frame(vec![Event::Ime(ImeEvent::Preedit { text: "y".to_owned(), active_range_chars: None })], &mut show);
        assert!(editor.borrow().composing, "the editor composes again");
    }
}

/// A press outside the app's editor fences its composition with the framework's own reset request; a reset the app
/// asks for in a later pass becomes the one the fence waits for. Its completion ends the fence, the commit after it
/// reaches the editor, and the framework's request is never answered.
#[trial]
fn an_app_editor_s_later_reset_ends_the_fence() {
    let (editor, mut show, mut frames) = composing_resetting_editor(false);
    let outside = Pos2::new(500.0, 400.0);
    frames.frame(press_with_late_ime(outside), &mut show);
    let fence = frames.ime().and_then(|ime| ime.interrupt.then_some(ime.reset).flatten());
    assert!(fence.is_some() && editor.borrow().reset.is_none(), "the framework fences with its own request; the editor asks nothing");
    frames.frame(vec![release(outside)], &mut show);
    editor.borrow_mut().ask = true;
    frames.frame(Vec::new(), &mut show);
    let later = editor.borrow().reset;
    assert!(later.is_some() && later != fence, "the editor asks for its own reset later");
    assert_eq!(frames.ime().and_then(|ime| ime.reset), later, "the pass publishes the editor's request");
    let Some(later) = later else { return };
    frames.frame(
        vec![
            Event::Ime(ImeEvent::Commit("Q".to_owned())),
            Event::ImeResetCompleted { request_id: later },
            Event::Ime(ImeEvent::Commit("X".to_owned())),
        ],
        &mut show,
    );
    assert_eq!(editor.borrow().reset, None, "the completion of its later request reaches the editor");
    assert_eq!(editor.borrow().text, "abX", "the late commit is fenced and the commit after the completion reaches the editor");
    frames.frame(vec![Event::Ime(ImeEvent::Preedit { text: "y".to_owned(), active_range_chars: None })], &mut show);
    assert!(editor.borrow().composing, "the editor composes again");
}

/// A framework text field of `id`, as a text component's part draws it: a text stop that takes the IME edits meant for
/// it and notes its caret.
fn framework_field(ui: &mut Ui, id: Id) {
    let rect = widget(ui, id);
    register_stop(ui.ctx(), Stop::new(id, KeyUse::of(Input::Text)).text());
    let _ = ime::take(ui.ctx(), id, false);
    ime::note_caret(ui.ctx(), id, rect, rect, false);
}

/// The framework field of `framework_field` and the app's resetting editor beside it at its rect, the show closure that
/// draws both, and the frames after the field took the focus and composed "k".
fn framework_field_composing_beside_an_app_editor() -> (Id, Rect, Rc<RefCell<fixture::ResettingEditor>>, impl FnMut(&mut Ui), Frames) {
    let editor = Rc::new(RefCell::new(fixture::ResettingEditor::default()));
    let held = Rc::clone(&editor);
    let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(200.0, 30.0));
    let framework = named_id("framework field");
    let mut show = move |ui: &mut Ui| {
        framework_field(ui, framework);
        fixture::host_resetting_ime_editor(ui, host_editor_id(), rect, &mut held.borrow_mut());
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(framework));
    frames.frame(Vec::new(), &mut show);
    frames.frame(vec![Event::Ime(ImeEvent::Preedit { text: "k".to_owned(), active_range_chars: None })], &mut show);
    assert_eq!(frames.ime().map(|ime| ime.owner), Some(framework), "the framework field composes");
    (framework, rect, editor, show, frames)
}

/// A press on the app's editor takes the focus from a composing framework field, and the editor asks for its own reset
/// in the pass the field loses the IME, as EffectCraft's editor does when its caret moves after the press. The interrupt
/// of the field's composition publishes the editor's request, so the platform's answer in the next frame reaches the
/// editor, the field's late commit stays fenced until then, and the commit after it reaches the editor.
#[trial]
fn an_app_editor_s_reset_asked_as_it_takes_the_focus_from_a_composing_framework_field_completes() {
    let (framework, rect, editor, mut show, mut frames) = framework_field_composing_beside_an_app_editor();
    let at = rect.center();
    frames.frame(vec![moved(at), press(at)], &mut show);
    assert!(!frames.ime().is_some_and(|ime| ime.interrupt), "the field keeps the IME through the press pass");
    editor.borrow_mut().ask = true;
    frames.frame(vec![release(at)], &mut show);
    let own = editor.borrow().reset;
    assert!(own.is_some(), "the editor asked for its own reset");
    let ime = frames.ime().cloned();
    assert!(ime.as_ref().is_some_and(|ime| ime.owner == framework && ime.interrupt), "the pass interrupts the field's composition");
    assert_eq!(ime.and_then(|ime| ime.reset), own, "the interrupt publishes the editor's request, not a new one");
    let Some(own) = own else { return };
    frames.frame(
        vec![
            Event::Ime(ImeEvent::Commit("Q".to_owned())),
            Event::ImeResetCompleted { request_id: own },
            Event::Ime(ImeEvent::Commit("X".to_owned())),
        ],
        &mut show,
    );
    assert_eq!(editor.borrow().reset, None, "the completion of its request reaches the editor");
    assert_eq!(editor.borrow().text, "X", "the field's late commit is fenced and the commit after the completion reaches the editor");
}

/// The app's editor asks for its own reset at the press that takes the focus from a composing framework field, while
/// the field still owns the IME. egui publishes that request on the field's output as an interrupt; the engine takes it
/// as its interrupt of the field and fences on it. The field loses the IME in the next pass with no second interrupt,
/// the platform's answer in that frame reaches the editor, the field's late commit stays fenced, and the editor takes
/// the commits after it.
#[trial]
fn an_app_editor_s_reset_asked_at_the_press_on_it_while_a_framework_field_composes_completes() {
    let (framework, rect, editor, mut show, mut frames) = framework_field_composing_beside_an_app_editor();
    let at = rect.center();
    editor.borrow_mut().ask = true;
    frames.frame(vec![moved(at), press(at)], &mut show);
    let own = editor.borrow().reset;
    assert!(own.is_some(), "the editor asked for its own reset at the press");
    let ime = frames.ime().cloned();
    assert!(ime.as_ref().is_some_and(|ime| ime.owner == framework && ime.interrupt), "the request interrupts the field's composition");
    assert_eq!(ime.and_then(|ime| ime.reset), own, "the field's output carries the editor's request");
    let Some(own) = own else { return };
    frames.frame(vec![release(at), Event::Ime(ImeEvent::Commit("Q".to_owned())), Event::ImeResetCompleted { request_id: own }], &mut show);
    assert!(!frames.ime().is_some_and(|ime| ime.interrupt), "the field loses the IME without a second interrupt");
    assert_eq!(editor.borrow().reset, None, "the completion of its request reaches the editor");
    frames.frame(vec![Event::Ime(ImeEvent::Commit("X".to_owned()))], &mut show);
    assert_eq!(editor.borrow().text, "X", "the field's late commit is fenced and the editor takes the commit after the completion");
}

#[trial]
fn a_focused_egui_field_keeps_escape_and_the_clipboard_and_tabs_on_through_the_one_order() {
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
    assert_eq!(seen(&host), (Vec::new(), 0), "a single-line field's Tab is the framework's move, never egui's");
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(named_id("button")), "the field is the last stop: Tab wraps to the framework's button");
    fixture::host_focus(&frames.ctx, field());
    frames.frame(Vec::new(), &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (vec![Key::Escape], 0), "the field's Escape is egui's, and no window Cancel");
}

/// The Enter presses in egui's input and the window Submits taken after one Enter, with the host's field focused or
/// nothing focused.
fn enter_with(field_focused: bool) -> (usize, usize) {
    let counts = Rc::new(Cell::new((0usize, 0usize)));
    let counted = Rc::clone(&counts);
    let (_host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        let enters = ui.input(|input| input.events.iter().filter(|event| matches!(event, Event::Key { key: Key::Enter, pressed: true, .. })).count());
        let submit = with_window_intents(ui.ctx(), false, |intents| intents.take(Intent::Submit));
        let (seen, taken) = counted.get();
        counted.set((seen + enters, taken + usize::from(submit)));
        base(ui);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    if field_focused {
        fixture::host_focus(&frames.ctx, field());
        frames.frame(Vec::new(), &mut show);
        assert_eq!(egui_focused(&frames.ctx), Some(field()), "the host's field holds egui focus");
    }
    counts.set((0, 0));
    press_key(&mut frames, &mut show, Key::Enter);
    frames.idle(2, &mut show);
    counts.get()
}

/// An app's own focused widget keeps the plain Enter as it keeps Escape: it reads it from egui once, and no window
/// Submit comes of it, so an app that turns the window's Submit back into an Enter does not get two. With nothing
/// focused the plain Enter is the window's Submit and stays in egui's input too.
#[trial]
fn a_focused_egui_field_keeps_enter_and_the_window_gets_no_submit() {
    assert_eq!(enter_with(true), (1, 0), "the field reads one Enter and the window takes no Submit");
    assert_eq!(enter_with(false), (1, 1), "with nothing focused, Enter is the window's Submit and stays in egui");
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

/// The app's own buttons drawn after the window's framework button and host field: egui's arrows move focus among
/// them, and Tab and Shift+Tab move it through the window's one order, which holds them.
#[trial]
fn arrows_move_egui_focus_among_host_buttons_and_tab_moves_through_the_one_order() {
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
    assert_eq!(egui_focused(&frames.ctx), Some(right), "Tab moves focus on to the next stop");
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

/// Arrows from an entry of the app's own popup list stay in the list: the app's button under the list, a Tab stop as
/// every focusable widget of the app is, is no arrow target while the list covers it, though it lies nearer below the
/// first entry than the second entry does.
#[trial]
fn arrows_in_an_app_popup_list_skip_the_widget_it_covers() {
    let (button, popup, first, second) = (named_id("host tool"), named_id("host popup"), named_id("host entry 1"), named_id("host entry 2"));
    let mut show = move |ui: &mut Ui| {
        fixture::host_button(ui, button, Rect::from_min_max(Pos2::new(228.0, 31.0), Pos2::new(254.0, 57.0)));
        areas::host_popup_list(ui.ctx(), popup, Pos2::new(201.0, 18.0), &[first, second]);
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    fixture::host_focus(&frames.ctx, first);
    frames.idle(2, &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(first));
    press_key(&mut frames, &mut show, Key::ArrowDown);
    assert_eq!(egui_focused(&frames.ctx), Some(second), "ArrowDown goes to the list's next entry, not to the button under it");
    press_key(&mut frames, &mut show, Key::ArrowUp);
    assert_eq!(egui_focused(&frames.ctx), Some(first), "ArrowUp comes back to the first entry");
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

/// Tab is the framework's move whatever the window holds: from nothing focused it reaches the app's own widgets as its
/// stops, and never egui's input. A component's widget that registered no stop is no stop of the app's own.
#[trial]
fn tab_from_nothing_reaches_the_app_s_widgets_as_the_framework_s_move() {
    let (first, last) = (Some(named_id("host first")), Some(named_id("host last")));
    assert_eq!(tab_from_nothing(Before::Nothing, false), (first, false), "Tab focuses the first host button");
    assert_eq!(tab_from_nothing(Before::Nothing, true), (last, false), "Shift+Tab the last");
    assert_eq!(tab_from_nothing(Before::Stop, false), (Some(named_id("button")), false), "the framework stop drawn first comes first");
    assert_eq!(tab_from_nothing(Before::Unstopped, false), (first, false), "a component's widget without a stop gets no focus");
    assert_eq!(tab_from_nothing(Before::FocusedUnstopped, false), (first, false), "from that widget holding focus, Tab goes to the first stop");
}

/// A window with a framework stop, two of the app's own buttons and the app's canvas, which keeps Tab, unfocused: Tab
/// from nothing is the framework's move over its one order, which holds the app's buttons beside the framework's
/// stop. The unfocused canvas does not keep the Tab; it is the last entry, and once Tab focuses it, it keeps Tab.
#[trial]
fn tab_from_nothing_goes_through_the_framework_order_with_the_app_s_widgets() {
    let (stop, first, second, canvas) = (named_id("button"), named_id("host first"), named_id("host second"), named_id("host canvas"));
    let reached = Rc::new(Cell::new(false));
    let seen_tab = Rc::clone(&reached);
    let mut show = move |ui: &mut Ui| {
        seen_tab.set(seen_tab.get() || key_reached(ui.ctx(), Key::Tab));
        widget(ui, stop);
        register_stop(ui.ctx(), Stop::new(stop, KeyUse::of(Input::Pointer)));
        for (id, x) in [(first, 300.0), (second, 420.0)] {
            fixture::host_button(ui, id, Rect::from_min_size(Pos2::new(x, 300.0), egui::vec2(80.0, 30.0)));
        }
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, KeyUse::CANVAS.with_keep(KeepKeys::Tab));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let mut tab = |frames: &mut Frames| {
        reached.set(false);
        press_key(frames, &mut show, Key::Tab);
        frames.frame(Vec::new(), &mut show);
        (egui_focused(&frames.ctx), reached.get())
    };
    assert_eq!(tab(&mut frames), (Some(stop), false), "Tab from nothing goes to the first stop, not to the unfocused canvas");
    assert_eq!(tab(&mut frames), (Some(first), false), "then to the app's first button");
    assert_eq!(tab(&mut frames), (Some(second), false), "and its second, in the order the pass registered them");
    assert_eq!(tab(&mut frames), (Some(canvas), false), "then to the canvas");
    assert_eq!(tab(&mut frames), (Some(canvas), true), "the focused canvas keeps Tab and reads it from egui");
}

/// A window of the app's own canvas between two of the app's own buttons, with no framework stop: a bare Tab moves
/// focus through them in the order the pass drew them, from nothing focused and from the canvas alike, and never
/// reaches egui; a canvas that keeps Tab keeps it.
#[trial]
fn tab_moves_through_the_app_s_widgets_and_canvas_in_a_window_without_framework_stops() {
    let (first, canvas, last) = (named_id("host first"), named_id("host canvas"), named_id("host last"));
    let keeps = Rc::new(Cell::new(false));
    let reached = Rc::new(Cell::new(false));
    let (keep, seen) = (Rc::clone(&keeps), Rc::clone(&reached));
    let mut show = move |ui: &mut Ui| {
        seen.set(seen.get() || key_reached(ui.ctx(), Key::Tab));
        fixture::host_button(ui, first, Rect::from_min_size(Pos2::new(100.0, 300.0), egui::vec2(80.0, 30.0)));
        fixture::host_button(ui, canvas, Rect::from_min_size(Pos2::new(220.0, 300.0), egui::vec2(80.0, 30.0)));
        host_canvas(ui.ctx(), canvas, if keep.get() { KeyUse::CANVAS.with_keep(KeepKeys::Tab) } else { KeyUse::CANVAS });
        fixture::host_button(ui, last, Rect::from_min_size(Pos2::new(340.0, 300.0), egui::vec2(80.0, 30.0)));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let tab = |frames: &mut Frames, show: &mut dyn FnMut(&mut Ui)| {
        reached.set(false);
        frames.frame(vec![key_press(Key::Tab, Modifiers::NONE), key_release(Key::Tab, Modifiers::NONE)], &mut *show);
        frames.frame(Vec::new(), &mut *show);
        (egui_focused(&frames.ctx), reached.get())
    };
    assert_eq!(tab(&mut frames, &mut show), (Some(first), false), "Tab from nothing focuses the first widget drawn");
    assert_eq!(tab(&mut frames, &mut show), (Some(canvas), false), "the next Tab reaches the canvas");
    assert_eq!(tab(&mut frames, &mut show), (Some(last), false), "Tab from the canvas moves on");
    keeps.set(true);
    request(&frames.ctx, FocusTarget::Id(canvas));
    frames.idle(2, &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(canvas));
    assert_eq!(tab(&mut frames, &mut show), (Some(canvas), true), "a canvas that keeps Tab gets it from egui and keeps the focus");
}

/// The window the one-order trials show: the app's own button, a framework stop, the app's own single-line text field
/// inside a framework scope, a framework stop and the app's own button, in that order.
const MIXED: [&str; 5] = ["app first", "framework one", "app field", "framework two", "app last"];

fn mixed_window() -> impl FnMut(&mut Ui) {
    let mut text = String::new();
    move |ui: &mut Ui| {
        let [first, one, field, two, last] = MIXED.map(named_id);
        fixture::host_button(ui, first, Rect::from_min_size(Pos2::new(20.0, 300.0), egui::vec2(80.0, 30.0)));
        widget(ui, one);
        register_stop(ui.ctx(), Stop::new(one, KeyUse::of(Input::Pointer)));
        push_scope(ui.ctx(), named_id("group"), FocusPolicy::NONE, None);
        fixture::host_field(ui, field, &mut text);
        pop_scope(ui.ctx());
        widget(ui, two);
        register_stop(ui.ctx(), Stop::new(two, KeyUse::of(Input::Pointer)));
        fixture::host_button(ui, last, Rect::from_min_size(Pos2::new(420.0, 300.0), egui::vec2(80.0, 30.0)));
    }
}

/// A Tab press with `mods`, released in the same frame, and the frame after it: where egui focus is then.
fn tab_with(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), mods: Modifiers) -> Option<Id> {
    frames.frame(vec![key_press(Key::Tab, mods), key_release(Key::Tab, mods)], &mut *show);
    frames.frame(Vec::new(), &mut *show);
    egui_focused(&frames.ctx)
}

/// Every focusable egui widget the app draws is a Tab stop of the window's one order, where the pass drew it among the
/// framework's stops, with no registration of its own: Tab from nothing visits the five in order and wraps, and
/// Shift+Tab walks back.
#[trial]
fn tab_reaches_the_app_s_egui_widgets_beside_the_framework_stops_in_registration_order() {
    let mut show = mixed_window();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let order = MIXED.map(|name| Some(named_id(name)));
    let forward: Vec<Option<Id>> = (0..6).map(|_| tab_with(&mut frames, &mut show, Modifiers::NONE)).collect();
    assert_eq!(forward, [&order[..], &order[..1]].concat(), "Tab visits the five in the order the pass drew them, then wraps");
    let back: Vec<Option<Id>> = (0..5).map(|_| tab_with(&mut frames, &mut show, Modifiers::SHIFT)).collect();
    let reversed: Vec<Option<Id>> = order.iter().rev().copied().collect();
    assert_eq!(back, reversed, "Shift+Tab walks back from the first, wrapping to the last");
}

/// F6 lands on the app's own widget in a pane whose only content it is.
#[trial]
fn f6_lands_on_an_app_widget_in_a_pane() {
    let button = named_id("app pane button");
    let mut show = move |ui: &mut Ui| {
        show_panes(ui);
        push_scope(ui.ctx(), named_id("app pane"), FocusPolicy::PANE, Some(Arc::from("App")));
        fixture::host_button(ui, button, Rect::from_min_size(Pos2::new(420.0, 300.0), egui::vec2(80.0, 30.0)));
        pop_scope(ui.ctx());
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    focus_stop(&mut frames, &mut show, named_id("right"));
    let (focused, said) = pane_key(&mut frames, &mut show, Modifiers::NONE);
    assert_eq!(focused, Some(button), "F6 from the right pane goes to the app's pane and its button");
    assert!(said.contains(&"App".to_owned()), "and announces the pane: {said:?}");
}

/// A focused multi-line text field of the app's own whose egui key filter keeps Tab (it locks focus) gets the Tab in
/// egui and types it; a single-line field of the app's own moves on to the next stop.
#[trial]
fn an_app_field_that_keeps_tab_types_it_and_a_single_line_field_tabs_on() {
    let (code, line, after) = (named_id("app code"), named_id("app line"), named_id("framework after"));
    let texts = Rc::new(RefCell::new((String::new(), String::new())));
    let shared = Rc::clone(&texts);
    let mut show = move |ui: &mut Ui| {
        let mut texts = shared.borrow_mut();
        let (code_text, line_text) = &mut *texts;
        fixture::host_code_field(ui, code, code_text);
        fixture::host_field(ui, line, line_text);
        widget(ui, after);
        register_stop(ui.ctx(), Stop::new(after, KeyUse::of(Input::Pointer)));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    fixture::host_focus(&frames.ctx, code);
    frames.idle(2, &mut show);
    assert_eq!(tab_with(&mut frames, &mut show, Modifiers::NONE), Some(code), "the field that keeps Tab keeps the focus");
    assert!(texts.borrow().0.contains('\t'), "and types the Tab: {:?}", texts.borrow().0);
    fixture::host_focus(&frames.ctx, line);
    frames.idle(2, &mut show);
    assert_eq!(tab_with(&mut frames, &mut show, Modifiers::NONE), Some(after), "the single-line field's Tab moves on");
    assert_eq!(texts.borrow().1, "", "and types nothing");
}

/// How the focus of the app's own widget `id` in `mixed_window`, given by Tab (`tab`) or by the app's own egui request,
/// ends.
#[derive(Debug, PartialEq, Eq)]
struct FocusEnds {
    /// egui's focus after the press and after the release of a click elsewhere.
    click: (Option<Id>, Option<Id>),
    /// Whether the widget lost focus by egui's word, in each pass from the press on.
    lost: Vec<bool>,
    /// egui's focus after Escape, the focus given again.
    escape: Option<Id>,
    /// egui's focus after a click elsewhere while the app's option is never to surrender it, the focus given again.
    kept: Option<Id>,
}

fn focus_ends(id: Id, tab: bool) -> FocusEnds {
    let lost = Rc::new(RefCell::new(Vec::new()));
    let losses = Rc::clone(&lost);
    let mut base = mixed_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        losses.borrow_mut().push(ui.ctx().read_response(id).is_some_and(|response| response.lost_focus()));
    };
    let give = |frames: &mut Frames, show: &mut dyn FnMut(&mut Ui)| {
        for _ in 0..MIXED.len() {
            if tab && egui_focused(&frames.ctx) != Some(id) {
                tab_with(frames, &mut &mut *show, Modifiers::NONE);
            }
        }
        if !tab {
            fixture::host_focus(&frames.ctx, id);
        }
        frames.idle(2, &mut *show);
        assert_eq!(egui_focused(&frames.ctx), Some(id), "the widget has the focus");
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let empty = Pos2::new(300.0, 450.0);
    give(&mut frames, &mut show);
    lost.borrow_mut().clear();
    let click = click_focus(&mut frames, &mut show, empty);
    frames.idle(2, &mut show);
    let lost = lost.take();
    give(&mut frames, &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    let escape = egui_focused(&frames.ctx);
    fixture::set_app_surrender(&frames.ctx, true);
    give(&mut frames, &mut show);
    let (_, kept) = click_focus(&mut frames, &mut show, empty);
    FocusEnds { click, lost, escape, kept }
}

/// Focus that Tab gave an app's own button or text field ends as the focus the app's own request gave it: a click
/// elsewhere and Escape release it once, by egui's rule and the app's surrender option.
#[trial]
fn tab_focus_on_an_app_widget_ends_as_the_focus_the_app_gave_it() {
    for id in ["app first", "app field"].map(named_id) {
        let ends = focus_ends(id, true);
        assert_eq!(ends.click, (Some(id), None), "a click elsewhere releases it, at the release by egui's option");
        assert!(ends.lost.contains(&true), "and the widget sees its focus lost");
        assert_eq!(ends.escape, None, "Escape releases it");
        assert_eq!(ends.kept, Some(id), "the app's option keeps it over a click elsewhere");
        assert_eq!(ends, focus_ends(id, false), "as the focus the app gave it does, seen lost in the same passes");
    }
}

/// A component reached with Tab keeps its focus over Escape and drops it at a press no component takes.
#[trial]
fn tab_focus_on_a_component_ends_by_the_framework_s_press_rule() {
    let mut show = mixed_window();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    tab_with(&mut frames, &mut show, Modifiers::NONE);
    let one = Some(named_id("framework one"));
    assert_eq!(tab_with(&mut frames, &mut show, Modifiers::NONE), one);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(egui_focused(&frames.ctx), one, "Escape leaves the component's focus");
    let empty = Pos2::new(300.0, 450.0);
    assert_eq!(click_focus(&mut frames, &mut show, empty).0, None, "a press no component takes drops it at the press");
}

/// A press on a component that takes focus moves it there from the app's field that Tab focused, and the field sees its
/// focus lost once.
#[trial]
fn a_press_on_a_component_takes_the_focus_tab_gave_an_app_field() {
    let field = named_id("app field");
    let rect = Rect::from_min_size(Pos2::new(300.0, 400.0), egui::vec2(160.0, 40.0));
    let lost = Rc::new(Cell::new(0));
    let losses = Rc::clone(&lost);
    let mut base = mixed_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        sensing::show(ui, &[ProbeRoot::new("probe", rect, Sense::CLICK | Sense::TAKE_FOCUS)]);
        register_stop(ui.ctx(), Stop::new(ProbeRoot::id("probe"), KeyUse::of(Input::Pointer)));
        losses.set(losses.get() + usize::from(ui.ctx().read_response(field).is_some_and(|response| response.lost_focus())));
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    for _ in 0..3 {
        tab_with(&mut frames, &mut show, Modifiers::NONE);
    }
    assert_eq!(egui_focused(&frames.ctx), Some(field));
    lost.set(0);
    let probe = Some(ProbeRoot::id("probe"));
    assert_eq!(click_focus(&mut frames, &mut show, rect.center()), (probe, probe), "the press moves the focus to the component");
    frames.idle(2, &mut show);
    assert_eq!(lost.get(), 1, "the field sees its focus lost once");
}

/// What gives an app's field the focus during a click elsewhere.
#[derive(Clone, Copy, Debug)]
enum GivenAt {
    /// The press closes a popup, which gives the focus back to the field it opened from.
    PopupPress,
    /// The app's request at the press.
    Press,
    /// The app's request at the release, in a pass that draws the field later.
    Release,
}

/// Focus the runtime gives an app's own widget during a click elsewhere stands: neither the press nor its release takes
/// it back. The next click elsewhere does, by egui's rule.
#[trial]
fn a_click_that_gives_an_app_field_the_focus_does_not_take_it_back() {
    for at in [GivenAt::PopupPress, GivenAt::Press, GivenAt::Release] {
        let field = named_id("app field");
        let (asking, popup) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        let (ask, open) = (Rc::clone(&asking), Rc::clone(&popup));
        let mut base = mixed_window();
        let mut show = move |ui: &mut Ui| {
            let (pressed, clicked) = ui.input(|input| (input.pointer.primary_pressed(), input.pointer.any_click()));
            let now = if matches!(at, GivenAt::Release) { clicked } else { pressed };
            if now && ask.take() {
                match at {
                    GivenAt::PopupPress => open.set(false),
                    GivenAt::Press | GivenAt::Release => _ = request(ui.ctx(), FocusTarget::Id(field)),
                }
            }
            base(ui);
            if open.get() {
                push_scope(ui.ctx(), named_id("popup"), FocusPolicy::POPUP, None);
                widget(ui, named_id("popup button"));
                register_stop(ui.ctx(), Stop::new(named_id("popup button"), KeyUse::of(Input::Pointer)));
                pop_scope(ui.ctx());
            }
        };
        let mut frames = Frames::new();
        frames.idle(2, &mut show);
        if matches!(at, GivenAt::PopupPress) {
            fixture::host_focus(&frames.ctx, field);
            frames.idle(1, &mut show);
            popup.set(true);
            frames.idle(2, &mut show);
            assert_eq!(egui_focused(&frames.ctx), Some(named_id("popup button")), "the popup takes the focus");
        }
        asking.set(true);
        let empty = Pos2::new(300.0, 450.0);
        assert_eq!(click_focus(&mut frames, &mut show, empty).1, Some(field), "{at:?}: the focus stands over the release");
        assert_eq!(click_focus(&mut frames, &mut show, empty).1, None, "{at:?}: the next click elsewhere takes it");
    }
}

/// A canvas declared after a component registered behind it is first taken as one of the app's widgets: focus the runtime
/// gave it there follows the canvas's rules, which keep it over Escape.
#[trial]
fn focus_given_to_a_canvas_before_its_declaration_is_the_canvas_s() {
    let canvas = named_id("host canvas");
    let mut show = move |ui: &mut Ui| {
        fixture::host_button(ui, canvas, Rect::from_min_size(Pos2::new(100.0, 300.0), egui::vec2(80.0, 30.0)));
        widget(ui, named_id("button"));
        register_stop(ui.ctx(), Stop::new(named_id("button"), KeyUse::of(Input::Pointer)));
        host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), |_| {});
    request(&frames.ctx, FocusTarget::Id(canvas));
    frames.idle(2, &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(canvas));
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(egui_focused(&frames.ctx), Some(canvas), "egui keeps the focus on the canvas over its Escape");
}

/// A window of the app's own document canvas, declared while `declared` holds, beside the app's field of `host_window`, a
/// button and an area of the app's own that take no focus, and the component `probe`, which takes it.
struct CanvasWindow {
    declared: Rc<Cell<bool>>,
    host: Rc<RefCell<Host>>,
    /// egui's focus in each pass, at its start and after the canvas was drawn.
    in_pass: Rc<RefCell<Vec<Option<Id>>>>,
}

impl CanvasWindow {
    const BUTTON: Rect = Rect { min: Pos2::new(300.0, 100.0), max: Pos2::new(380.0, 130.0) };
    const AREA: Rect = Rect { min: Pos2::new(300.0, 200.0), max: Pos2::new(380.0, 230.0) };
    const PROBE: Rect = Rect { min: Pos2::new(300.0, 300.0), max: Pos2::new(380.0, 330.0) };
    const CANVAS: Rect = Rect { min: Pos2::new(100.0, 400.0), max: Pos2::new(180.0, 430.0) };
    const EMPTY: Pos2 = Pos2::new(500.0, 450.0);

    fn new() -> (Self, impl FnMut(&mut Ui)) {
        let (host, mut base) = host_window();
        let window = Self { declared: Rc::new(Cell::new(true)), host, in_pass: Rc::default() };
        let (declared, in_pass) = (Rc::clone(&window.declared), Rc::clone(&window.in_pass));
        let show = move |ui: &mut Ui| {
            in_pass.borrow_mut().push(egui_focused(ui.ctx()));
            base(ui);
            fixture::host_button(ui, named_id("app button"), Self::BUTTON);
            fixture::host_unfocusable_area(ui, named_id("app area"), Self::AREA);
            sensing::show(ui, &[ProbeRoot::new("probe", Self::PROBE, Sense::CLICK | Sense::TAKE_FOCUS)]);
            register_stop(ui.ctx(), Stop::new(ProbeRoot::id("probe"), KeyUse::of(Input::Pointer)));
            fixture::host_button(ui, Self::canvas(), Self::CANVAS);
            if declared.get() {
                host_canvas(ui.ctx(), Self::canvas(), KeyUse::CANVAS);
            }
            in_pass.borrow_mut().push(egui_focused(ui.ctx()));
        };
        (window, show)
    }

    fn canvas() -> Id {
        named_id("host canvas")
    }

    /// Gives the canvas the focus, by the app's own request or the framework's.
    fn focus(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), app: bool) {
        if app {
            fixture::host_focus(&frames.ctx, Self::canvas());
        } else {
            request(&frames.ctx, FocusTarget::Id(Self::canvas()));
        }
        frames.idle(2, &mut *show);
        assert_eq!(egui_focused(&frames.ctx), Some(Self::canvas()));
    }
}

/// A click that nothing takes leaves the app's document canvas focused in every pass, whoever gave it the focus: by the
/// framework's press rule and by egui's. A canvas the app no longer declares follows egui's rule.
#[trial]
fn a_click_nothing_takes_leaves_the_app_s_canvas_focused() {
    for app in [true, false] {
        let (window, mut show) = CanvasWindow::new();
        let mut frames = Frames::new();
        frames.idle(2, &mut show);
        CanvasWindow::focus(&mut frames, &mut show, app);
        window.in_pass.take();
        for at in [CanvasWindow::BUTTON.center(), CanvasWindow::AREA.center(), CanvasWindow::EMPTY] {
            click_focus(&mut frames, &mut show, at);
            let passes = window.in_pass.take();
            assert!(!passes.is_empty() && passes.iter().all(|focused| *focused == Some(CanvasWindow::canvas())), "app {app}, {at:?}: {passes:?}");
        }
        window.declared.set(false);
        frames.idle(2, &mut show);
        let (pressed, clicked) = click_focus(&mut frames, &mut show, CanvasWindow::EMPTY);
        assert_eq!((pressed, clicked), (Some(CanvasWindow::canvas()), None), "app {app}: an app's plain widget loses it at the click");
    }
}

/// A press on a component that takes focus, or on the app's own field, takes it from the app's document canvas.
#[trial]
fn a_press_that_takes_focus_takes_it_from_the_app_s_canvas() {
    for app in [true, false] {
        let (window, mut show) = CanvasWindow::new();
        let mut frames = Frames::new();
        frames.idle(2, &mut show);
        CanvasWindow::focus(&mut frames, &mut show, app);
        let probe = Some(ProbeRoot::id("probe"));
        assert_eq!(click_focus(&mut frames, &mut show, CanvasWindow::PROBE.center()), (probe, probe), "app {app}: the component");
        CanvasWindow::focus(&mut frames, &mut show, app);
        let at = window.host.borrow().rect.center();
        assert_eq!(click_focus(&mut frames, &mut show, at).1, Some(field()), "app {app}: the app's field");
    }
}

/// Where Tab reaches only text fields (the Mac's default), it reaches the app's own text field and skips the app's
/// own buttons, as it skips the framework's.
#[trial]
fn tab_that_reaches_only_text_fields_reaches_the_app_s_field_and_skips_its_buttons() {
    let mut show = mixed_window();
    let mut frames = Frames::new();
    crate::runtime::windows::set_os(&frames.ctx, craft_host::target::Os::Mac);
    frames.root({
        let mut root = RootEnv::default();
        root.tab_all_controls = false;
        root
    });
    frames.idle(2, &mut show);
    let field = Some(named_id("app field"));
    assert_eq!(tab_with(&mut frames, &mut show, Modifiers::NONE), field, "Tab from nothing goes to the app's field");
    assert_eq!(tab_with(&mut frames, &mut show, Modifiers::NONE), field, "the field is the one entry: the next Tab stays on it");
}

/// While an app's own egui popup is open, Tab is egui's, as Escape is: the framework moves no focus with it.
#[trial]
fn an_open_app_popup_keeps_tab() {
    let popup = named_id("host popup");
    let (host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        areas::host_popup(ui, popup, Pos2::ZERO);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    request(&frames.ctx, FocusTarget::Id(named_id("button")));
    frames.idle(2, &mut show);
    areas::open_host_popup(&frames.ctx, popup);
    frames.frame(Vec::new(), &mut show);
    assert!(areas::host_popup_open(&frames.ctx, popup));
    seen(&host);
    press_key(&mut frames, &mut show, Key::Tab);
    assert_eq!(seen(&host).0, vec![Key::Tab], "the popup's Tab is egui's");
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(named_id("button")), "the framework moved no focus");
}

/// The scan that makes the app's egui widgets Tab stops classifies each focus candidate of a pass once, however many
/// registrations take candidates in between: the pass's lookups grow with its widgets, one per app widget.
#[trial]
fn each_focus_candidate_of_a_pass_is_classified_once() {
    let lookups = |n: usize| {
        let mut show = move |ui: &mut Ui| {
            for at in 0..n {
                fixture::host_button(
                    ui,
                    crate::runtime::egui::memory::unique(("app", at)),
                    Rect::from_min_size(Pos2::new(10.0, 30.0 * at as f32), egui::vec2(60.0, 20.0)),
                );
                widget(ui, crate::runtime::egui::memory::unique(("framework", at)));
                register_stop(ui.ctx(), Stop::new(crate::runtime::egui::memory::unique(("framework", at)), KeyUse::of(Input::Pointer)));
            }
        };
        let mut frames = Frames::new();
        frames.idle(2, &mut show);
        crate::runtime::pointer::made_lookups(&frames.ctx, ViewportId::ROOT) as usize
    };
    // The framework widget is the root of the stop registered after it, so only the app's widgets need a lookup.
    let (small, large) = (lookups(8), lookups(16));
    assert!(small >= 8, "every app widget is looked up: {small}");
    assert_eq!(large - small, 8, "eight more app widgets cost one lookup each: {small} then {large}");
}

/// A Tab in a window with no stop and nothing focused moves no focus, so it stays in egui's input for the app's own
/// binding (EffectCraft's Tab for the timeline's mini flowchart), from the window's first pass on. Once the app draws a
/// focusable widget, that widget is a stop and the Tab is the framework's move to it.
#[trial]
fn an_unowned_tab_with_no_stops_reaches_the_app() {
    let button = named_id("app button");
    let (reached, draw) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(false)));
    let mut show = {
        let (reached, draw) = (Rc::clone(&reached), Rc::clone(&draw));
        move |ui: &mut Ui| {
            if key_reached(ui.ctx(), Key::Tab) {
                reached.set(reached.get() + 1);
            }
            if draw.get() {
                fixture::host_button(ui, button, Rect::from_min_size(Pos2::new(40.0, 40.0), egui::vec2(80.0, 30.0)));
            }
        }
    };
    let mut frames = Frames::new();
    press_key(&mut frames, &mut show, Key::Tab);
    assert_eq!(reached.take(), 1, "the window's first pass has no stop: its Tab is the app's");
    frames.idle(2, &mut show);
    frames.frame(vec![key_press(Key::Tab, Modifiers::SHIFT), key_release(Key::Tab, Modifiers::SHIFT)], &mut show);
    assert_eq!(reached.take(), 1, "with nothing drawn, Shift+Tab is the app's too");
    assert_eq!(egui_focused(&frames.ctx), None);
    draw.set(true);
    frames.idle(2, &mut show);
    press_key(&mut frames, &mut show, Key::Tab);
    frames.frame(Vec::new(), &mut show);
    assert_eq!(reached.take(), 0, "the app's button is a stop: the Tab is the framework's move");
    assert_eq!(egui_focused(&frames.ctx), Some(button), "and the Tab moved focus to the button");
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
        (vec![(ViewportId::ROOT, ChromeToggle::Panels)], false),
        "a canvas keeps Tab only while focused: with nothing focused, Tab toggles the panels"
    );
}

/// Focus that Tab moves to the app's canvas in a layer returns to the stop it came from when the layer goes away, also
/// when the group the canvas sits in first appears in the pass of the Tab: declaring the canvas enters nothing.
#[trial]
fn declaring_a_focused_canvas_keeps_the_opener_of_its_layer() {
    let (opener, canvas) = (named_id("opener"), named_id("host canvas"));
    let layer = Rc::new(Cell::new(true));
    let grouped = Rc::new(Cell::new(false));
    let (shown, group) = (Rc::clone(&layer), Rc::clone(&grouped));
    let mut show = move |ui: &mut Ui| {
        widget(ui, opener);
        register_stop(ui.ctx(), Stop::new(opener, KeyUse::of(Input::Pointer)));
        if !shown.get() {
            return;
        }
        push_scope(ui.ctx(), named_id("layer"), FocusPolicy::TAB_LAYER, None);
        if group.get() {
            push_scope(ui.ctx(), named_id("group"), FocusPolicy::NONE, None);
        }
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
        if group.get() {
            pop_scope(ui.ctx());
        }
        pop_scope(ui.ctx());
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    focus_stop(&mut frames, &mut show, opener);
    grouped.set(true);
    assert_eq!(tab_with(&mut frames, &mut show, Modifiers::NONE), Some(canvas), "Tab goes from the stop into the layer");
    layer.set(false);
    frames.idle(2, &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(opener), "the layer gives the focus back to the stop it came from");
}

/// How an app's window shows the canvas of `host_canvas_findings`.
#[derive(Clone, Copy, Debug)]
enum Shown {
    /// Neither declared nor drawn.
    Hidden,
    /// Declared, and drawn with a focusable widget that senses clicks or drags.
    Click,
    Drag,
    /// Declared, and drawn with a focusable widget in an egui window, whose first pass sizes it invisibly.
    Window,
    /// Declared, and drawn with a widget that senses drags and is not focusable.
    Unfocusable,
    /// Declared, and drawn in a disabled `Ui`.
    Disabled,
    /// Declared and not drawn.
    Undrawn,
}

/// Whether the canvas holds egui focus after a Tab from nothing in a window that shows it as `shown`, declared before it
/// is drawn, and the subjects of the findings on it.
fn host_canvas_findings(shown: Shown) -> (bool, Vec<String>) {
    let canvas = named_id("host canvas");
    let mut show = move |ui: &mut Ui| {
        if !matches!(shown, Shown::Hidden) {
            host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
        }
        let rect = Rect::from_min_size(Pos2::new(100.0, 300.0), egui::vec2(80.0, 30.0));
        match shown {
            Shown::Hidden | Shown::Undrawn => {}
            Shown::Click => _ = fixture::host_button(ui, canvas, rect),
            Shown::Drag => _ = fixture::host_drag_area(ui, canvas, rect),
            Shown::Window => areas::host_egui_window(ui.ctx(), named_id("host window"), |ui| _ = widget(ui, canvas)),
            Shown::Unfocusable => fixture::host_unfocusable_area(ui, canvas, rect),
            Shown::Disabled => fixture::host_disabled_button(ui, canvas, rect),
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    press_key(&mut frames, &mut show, Key::Tab);
    frames.idle(2, &mut show);
    let found = findings(&frames.ctx).into_iter().filter(|finding| finding.rule == "canvas-without-widget");
    (egui_focused(&frames.ctx) == Some(canvas), found.map(|finding| finding.subject).collect())
}

/// Guards the finding that tells an app its canvas stop takes Tab and loses the focus: `host_canvas` named an id that no
/// focusable egui widget of the pass has.
#[trial]
fn a_host_canvas_no_focusable_widget_draws_is_a_finding() {
    for shown in [Shown::Click, Shown::Drag, Shown::Window] {
        assert_eq!(host_canvas_findings(shown), (true, Vec::new()), "{shown:?}: Tab stays at a canvas with a focusable widget");
    }
    assert_eq!(host_canvas_findings(Shown::Hidden).1, Vec::<String>::new(), "a hidden canvas makes no call");
    for (shown, says) in [(Shown::Undrawn, "no egui widget"), (Shown::Unfocusable, "sense"), (Shown::Disabled, "disabled")] {
        let (focused, found) = host_canvas_findings(shown);
        assert!(!focused, "{shown:?}: egui drops the focus Tab gave the canvas");
        assert!(matches!(found.as_slice(), [only] if only.contains(says)), "{shown:?}: {found:?}");
    }
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

/// What a host canvas with GridCraft's keys saw of a copy and an Escape while it held egui focus: whether the copy
/// reached egui's input with no command fired, the keys that reached egui and the window cancels, and egui's focus
/// after the Escape. `app_focus` gives it egui focus by the app's own request, else by the framework's `request`.
fn focused_host_canvas_keys(app_focus: bool) -> (bool, (Vec<Key>, usize), Option<Id>) {
    const KEYS: KeyUse = KeyUse::CANVAS.with_keep(KeepKeys::Tab).with_edits(EditSet::ALL);
    let canvas = named_id("host canvas");
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen_events = Rc::clone(&events);
    let (host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        seen_events.borrow_mut().extend(ui.input(|input| input.events.clone()));
        base(ui);
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, KEYS);
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    if app_focus {
        fixture::host_focus(&frames.ctx, canvas);
    } else {
        request(&frames.ctx, FocusTarget::Id(canvas));
    }
    frames.frame(Vec::new(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(canvas), "the canvas holds egui focus");
    take_output(&frames.ctx);
    seen(&host);
    events.borrow_mut().clear();
    frames.frame(vec![Event::Copy], &mut show);
    frames.frame(Vec::new(), &mut show);
    let copied = events.borrow().contains(&Event::Copy) && take_output(&frames.ctx).fired.is_empty();
    press_key(&mut frames, &mut show, Key::Escape);
    let escape = seen(&host);
    frames.frame(Vec::new(), &mut show);
    (copied, escape, egui_focused(&frames.ctx))
}

/// A host canvas holding egui focus is the app's own focused widget: its clipboard events and Escape stay in egui's
/// input, and no window cancel comes of the Escape. With nothing focused, Escape is the window's cancel.
#[trial]
fn a_host_canvas_with_egui_focus_reads_escape_and_the_clipboard_from_egui() {
    let canvas = named_id("host canvas");
    let (copied, escape, focused) = focused_host_canvas_keys(false);
    assert!(copied, "the copy of a canvas the framework focused stays in egui and fires nothing");
    assert_eq!(escape, (vec![Key::Escape], 0), "its Escape is egui's, and no window cancel");
    assert_eq!(focused, Some(canvas), "egui keeps the framework's focus on the canvas over its Escape");
    let (copied, escape, _) = focused_host_canvas_keys(true);
    assert!(copied, "the copy of a canvas the app focused stays in egui and fires nothing");
    assert_eq!(escape, (vec![Key::Escape], 0), "and its Escape");
    let (host, mut base) = host_window();
    let mut show = move |ui: &mut Ui| {
        base(ui);
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, KeyUse::CANVAS.with_keep(KeepKeys::Tab).with_edits(EditSet::ALL));
    };
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    press_key(&mut frames, &mut show, Key::Escape);
    assert_eq!(seen(&host), (Vec::new(), 1), "with nothing focused, Escape is the window's cancel");
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
        // design: allow[contract/egui-widget](the trial reads the caret of the app's own egui TextEdit, which is what it checks)
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
    let (host, show) = host_window();
    f6_in(&host, show, overrides)
}

/// What an F6 press did under `overrides` in the window `show` draws, whose host records what reached egui in `host`.
fn f6_in(host: &RefCell<Host>, mut show: impl FnMut(&mut Ui), overrides: Option<craft_keymap::Overrides>) -> (bool, Vec<String>) {
    let mut frames = Frames::new();
    if let Some(overrides) = overrides {
        frames.commands(CommandInput::new().overrides(Arc::new(overrides)));
    }
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    seen(host);
    press_key(&mut frames, &mut show, Key::F6);
    let fired = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    (seen(host).0.contains(&Key::F6), fired)
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

/// What a ⌘Q (Ctrl+Q off the Mac) did in the host window under `overrides`: how many frames sent a close command, the
/// app taking each frame's output before the next as `embed::before_input` does, and the ids of the commands it fired.
fn quit_under(overrides: Option<craft_keymap::Overrides>) -> (usize, Vec<String>) {
    let (_, mut show) = host_window();
    let mut frames = Frames::new();
    if let Some(overrides) = overrides {
        frames.commands(CommandInput::new().overrides(Arc::new(overrides)));
    }
    frames.frame(Vec::new(), &mut show);
    take_output(&frames.ctx);
    let mut closes = usize::from(frames.frame_closing(vec![key_press(Key::Q, Modifiers::COMMAND)], &mut show));
    let mut fired = Vec::new();
    for _ in 0..3 {
        fired.extend(take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id));
        closes += usize::from(frames.frame_closing(Vec::new(), &mut show));
    }
    (closes, fired)
}

/// egui's own quit shortcut is cleared at install; the runtime's Quit command takes its place, through the root window's
/// close path, and an app's own binding of the chord replaces it.
#[trial]
fn quit_asks_the_root_window_to_close_once_and_an_app_binding_replaces_it() {
    assert_eq!(quit_under(None), (1, vec![QuitCommand::Quit.id().to_owned()]), "the quit chord asks the root window to close once");
    let mut bound = craft_keymap::Overrides::default();
    bound.insert(EditCommands::Delete.id(), vec!["Cmd+Q".to_owned()]);
    assert_eq!(quit_under(Some(bound)), (0, vec![EditCommands::Delete.id().to_owned()]), "a command bound to the chord takes it");
    let mut unbound = craft_keymap::Overrides::default();
    unbound.insert(QuitCommand::Quit.id(), Vec::new());
    assert_eq!(quit_under(Some(unbound)), (0, Vec::new()), "with Quit unbound, nothing closes the window");
}

/// The Alt+digit chords the hosted canvas of `an_unbound_f6_reaches_a_host_canvas_with_claims` claims.
const CANVAS_CLAIMS: [craft_keymap::Chord; 2] = [
    craft_keymap::Chord::new(craft_keymap::Mods::ALT, craft_keymap::Key::Num1),
    craft_keymap::Chord::new(craft_keymap::Mods::ALT, craft_keymap::Key::Num2),
];

/// A hosted app whose own egui canvas is the window's key target, with Tab, the edit chords and Alt+digit claims kept
/// (SoundCraft's session view), and whose shortcut table uses F6: with both pane moves unbound, F6 reaches egui.
#[trial]
fn an_unbound_f6_reaches_a_host_canvas_with_claims() {
    const KEYS: KeyUse = KeyUse::CANVAS.with_keep(KeepKeys::Tab).with_edits(EditSet::ALL).with_claims(&CANVAS_CLAIMS);
    let canvas = named_id("host canvas");
    let (host, mut base) = host_window();
    let show = move |ui: &mut Ui| {
        base(ui);
        widget(ui, canvas);
        host_canvas(ui.ctx(), canvas, KEYS);
    };
    let mut unbound = craft_keymap::Overrides::default();
    unbound.insert(NavCommand::NextPane.id(), Vec::new());
    unbound.insert(NavCommand::PreviousPane.id(), Vec::new());
    assert_eq!(f6_in(&host, show, Some(unbound)), (true, Vec::new()), "the canvas's app reads F6 from egui");
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
