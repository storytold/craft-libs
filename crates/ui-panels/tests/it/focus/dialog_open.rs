//! Where focus goes when a dialog with no text field and no default stop opens.

use super::{control, focused, id};
use craft_behavior::{Input, NavShape, Roving};
use craft_ui::runtime::focus::{FocusPolicy, KeyUse};
use craft_ui::testing::focus::{Group, Stop, begin_body, end_body, pop_scope, push_scope, register_group, register_stop, widget};
use craft_ui::testing::{Frames, trial};
use egui::Ui;

/// The stop a dialog with no field and no default stop focuses when it opens: `body` names its body's stops, `end`
/// the stops after its body (an end region's and a footer's buttons). A name starting with "text" is read-only text
/// that only a press focuses, and one starting with "row" is a child of a collection's row.
fn opens_on(body: &[&str], end: &[&str]) -> Option<egui::Id> {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        push_scope(ui.ctx(), id("dialog"), FocusPolicy::DIALOG, None);
        begin_body(ui.ctx());
        for name in body {
            if name.starts_with("text") {
                widget(ui, id(name));
                register_stop(ui.ctx(), Stop::new(id(name), KeyUse::of(Input::Text)).press_only());
            } else if name.starts_with("row") {
                widget(ui, id(name));
                register_stop(ui.ctx(), Stop::new(id(name), KeyUse::of(Input::Pointer)).in_group(id("row")));
                register_group(ui.ctx(), Group::new(id("row"), Roving::new(NavShape::Flat)).row(id("rows")));
            } else {
                control(ui, name);
            }
        }
        end_body(ui.ctx());
        for name in end {
            control(ui, name);
        }
        pop_scope(ui.ctx());
    };
    frames.idle(3, &mut show);
    focused(&frames.ctx)
}

/// A dialog with no field and no default stop opens on the first body stop that Tab reaches, past read-only text, and
/// leaves the buttons after its body unfocused when the body has none, so Enter answers nothing nobody chose.
#[trial]
fn a_dialog_with_no_field_or_default_opens_on_its_first_reachable_body_stop() {
    assert_eq!(opens_on(&["list", "new"], &["close"]), Some(id("list")), "the body's first stop takes the focus");
    assert_eq!(opens_on(&["text", "list"], &["close"]), Some(id("list")), "read-only text that Tab passes over does not");
    assert_eq!(opens_on(&["row child", "list"], &["close"]), Some(id("list")), "nor does a row's child, which Tab passes over");
    assert_eq!(opens_on(&[], &["action", "close"]), None, "with nothing in the body, no button after it takes the focus");
}
