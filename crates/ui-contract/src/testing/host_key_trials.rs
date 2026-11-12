//! Trials of the keys and presses a host's own egui widgets keep while the runtime is installed on the whole window.

use crate::runtime::egui::fixture;
use crate::runtime::focus::KeyUse;
use crate::testing::focus::{Stop, egui_focused, named_id, register_stop, widget};
use crate::testing::pointer::{moved, press, release};
use crate::testing::{Frames, trial};
use craft_behavior::Input;
use egui::{Id, Rect, Ui};
use std::cell::RefCell;
use std::rc::Rc;

fn field() -> Id {
    named_id("host field")
}

/// What the host's own UI saw in its passes.
struct Host {
    text: String,
    rect: Rect,
}

/// A window with a framework button and the host's own egui text field, with what the host saw.
fn host_window() -> (Rc<RefCell<Host>>, impl FnMut(&mut Ui)) {
    let host = Rc::new(RefCell::new(Host { text: String::new(), rect: Rect::NOTHING }));
    let seen = Rc::clone(&host);
    let show = move |ui: &mut Ui| {
        widget(ui, named_id("button"));
        register_stop(ui.ctx(), Stop::new(named_id("button"), KeyUse::of(Input::Pointer)));
        let mut host = seen.borrow_mut();
        let host = &mut *host;
        host.rect = fixture::host_field(ui, field(), &mut host.text);
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
