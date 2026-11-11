//! Focus on press (8.3): the kinds of a primary press and the draft that commits in the press pass.

use super::{focus, focused, id};
use craft_behavior::Input;
use craft_behavior::Sense;
use craft_ui::runtime::focus::{FocusPolicy, KeyUse};
use craft_ui::testing::focus::{Stop, commit_now, pop_scope, press_shape, primary_button, push_scope, register_stop, widget};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Pos2, Rect, Ui};
use std::cell::{Cell, RefCell};

struct Rects {
    field: Rect,
    palette: Rect,
    button: Rect,
}

/// A text field that takes focus on a press, a palette that keeps it, a control that does neither and the pane
/// the field's focus moves to.
fn layout(ui: &mut Ui, rects: &Cell<Option<(Rect, Rect, Rect)>>, commits: &RefCell<Vec<Option<egui::Id>>>) {
    widget(ui, id("pane"));
    push_scope(ui.ctx(), id("pane"), FocusPolicy::PANE, None);
    let field = widget(ui, id("field"));
    register_stop(ui.ctx(), Stop::new(id("field"), KeyUse::of(Input::Text)).text());
    pop_scope(ui.ctx());
    let palette = widget(ui, id("palette"));
    let button = widget(ui, id("button"));
    let ctx = ui.ctx().clone();
    press_shape(ui, field, Sense::CLICK | Sense::TAKE_FOCUS, id("field"));
    press_shape(ui, palette, Sense::CLICK | Sense::KEEP_FOCUS, id("palette"));
    press_shape(ui, button, Sense::CLICK, id("button"));
    rects.set(Some((field, palette, button)));
    commits.borrow_mut().push(commit_now(&ctx));
}

fn press(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![Event::PointerMoved(at), primary_button(at, true)], &mut *show);
    frames.frame(vec![primary_button(at, false)], &mut *show);
    frames.idle(2, show);
}

/// Skeleton `structure_keep_focus` (both tests) and `frames_state::press_nobody_claims_surrenders_focus_and_
/// commits_the_draft`: a press on a text field takes focus (kind 1); a press on a keep-focus shape leaves
/// focus and the draft (kind 2); a press on another control moves focus to the pane and commits (kind 3); a
/// press nobody claims commits and surrenders focus (kind 4).
#[trial]
fn a_press_takes_keeps_or_releases_key_focus_by_the_winning_shape() {
    let mut frames = Frames::new();
    let rects = Cell::new(None);
    let commits = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| layout(ui, &rects, &commits);
    frames.idle(3, &mut show);
    let (field, palette, button) = rects.get().expect("laid out");
    let rects = Rects { field, palette, button };
    assert_eq!(focused(&frames.ctx), None);
    press(&mut frames, rects.field.center(), &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("field")), "kind 1: a press on a text field takes key focus");
    assert!(commits.borrow().iter().all(Option::is_none), "and commits nothing");
    press(&mut frames, rects.palette.center(), &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("field")), "kind 2: a keep-focus shape leaves focus where it is");
    assert!(commits.borrow().iter().all(Option::is_none), "and leaves the draft");
    press(&mut frames, rects.button.center(), &mut show);
    assert_eq!(commits.borrow().iter().flatten().copied().collect::<Vec<_>>(), [id("field")], "kind 3: the draft commits in the press pass");
    assert_eq!(focused(&frames.ctx), Some(id("pane")), "and key focus moves to the enclosing pane");
    commits.borrow_mut().clear();
    focus(&mut frames, "field", &mut show);
    commits.borrow_mut().clear();
    press(&mut frames, Pos2::new(700.0, 500.0), &mut show);
    assert_eq!(commits.borrow().iter().flatten().copied().collect::<Vec<_>>(), [id("field")], "kind 4: a press nobody claims commits the draft");
    assert_eq!(focused(&frames.ctx), None, "and the end hook surrendered key focus");
}
