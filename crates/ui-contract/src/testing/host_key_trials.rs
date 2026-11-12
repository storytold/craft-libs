//! Trials of the keys and presses a host's own egui widgets keep while the runtime is installed on the whole window.

use crate::runtime::drag::{self, DragPreview};
use crate::runtime::egui::fixture;
use crate::runtime::focus::KeyUse;
use crate::runtime::keys::with_window_intents;
use crate::testing::focus::{Stop, egui_focused, named_id, register_stop, widget};
use crate::testing::input::{key_press, key_reached};
use crate::testing::pointer::{moved, press, release};
use crate::testing::{Frames, trial};
use craft_behavior::{Input, Intent};
use egui::{Event, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Ui};
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

const WATCHED: [Key; 2] = [Key::Tab, Key::Escape];

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

#[trial]
fn a_press_on_the_focused_egui_field_keeps_its_focus() {
    let (host, mut show) = host_window();
    let mut frames = Frames::new();
    frames.frame(Vec::new(), &mut show);
    let inside = host.borrow().rect.center();
    fixture::host_focus(&frames.ctx, field());
    frames.frame(vec![moved(inside)], &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(field()));
    frames.frame(vec![press(inside)], &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(field()), "a press inside the focused field keeps its focus");
    frames.frame(vec![release(inside)], &mut show);
    let rect = host.borrow().rect;
    let outside = rect.left_bottom() + egui::vec2(0.0, rect.height() * 3.0);
    frames.frame(vec![moved(outside)], &mut show);
    frames.frame(vec![press(outside)], &mut show);
    assert_eq!(egui_focused(&frames.ctx), None, "a press no widget takes surrenders the focus");
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
