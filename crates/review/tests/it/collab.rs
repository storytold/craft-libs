//! Following a collaborator: the order Escape is taken in.

use std::cell::RefCell;
use std::rc::Rc;

use craft_review::collab::{PEOPLE_COLORS, ParticipantId};
use craft_ui::canvas::{Canvas, ToolHost};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Fixture, trial};

/// A canvas tool with an edit under way: Escape cancels the edit first.
struct Editing(Rc<std::cell::Cell<bool>>);

impl craft_ui::canvas::CanvasTool for Editing {
    fn press(&mut self, _: &craft_ui::canvas::ToolPointer, _: &mut craft_ui::canvas::ToolContext<'_>) -> craft_ui::canvas::ToolAction {
        craft_ui::canvas::ToolAction::Ignored
    }

    fn drag(&mut self, _: &craft_ui::canvas::ToolPointer, _: &mut craft_ui::canvas::ToolContext<'_>) {}

    fn release(&mut self, _: &craft_ui::canvas::ToolPointer, _: &mut craft_ui::canvas::ToolContext<'_>) {}

    fn cancel(&mut self, _: craft_behavior::CancelReason, _: &mut craft_ui::canvas::ToolContext<'_>) {}

    fn key(&mut self, key: &craft_ui::canvas::ToolKey, _: &mut craft_ui::canvas::ToolContext<'_>) -> bool {
        matches!(key, craft_ui::canvas::ToolKey::Cancel) && self.0.replace(false)
    }

    fn paint(&self, _: &craft_ui::primitives::Draw<'_>, _: &craft_ui::canvas::ToolContext<'_>) {}

    fn command_form(&self) -> craft_ui::canvas::ToolPath {
        craft_ui::canvas::ToolPath::Freehand("the test tool has no keyboard path")
    }
}

/// With nothing focused, Escape cancels a tool's pending edit first and stops following the second time.
#[trial]
fn escape_cancels_a_pending_edit_before_it_stops_following() {
    let following = Rc::new(RefCell::new(Some(ParticipantId(2))));
    let shared = following.clone();
    let pending = Rc::new(std::cell::Cell::new(true));
    let edit = pending.clone();
    let mut fixture = Fixture::new(move |ui| {
        let area = ui.available_rect_before_wrap();
        let view = craft_behavior::CanvasView::new(area, kurbo::Point::new(0.0, 0.0), 1.0);
        let mut shared = shared.borrow_mut();
        if shared.is_none() {
            return;
        }
        let stopped = craft_review::ui::FollowFrame::new("Following", PEOPLE_COLORS[1], "Following Bo Chen")
            .show(ui, |ui, _| {
                let shown = Canvas::new("Document").view(view).show(ui, |_, _| {});
                let _ = ToolHost::new().run(ui, &shown.output, &mut Editing(edit.clone()));
            })
            .output
            .stopped;
        if stopped {
            *shared = None;
        }
    });
    fixture.settle();
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
    fixture.settle();
    assert!(!pending.get(), "the first Escape cancels the edit");
    assert!(following.borrow().is_some(), "the first Escape leaves following on");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
    fixture.settle();
    assert!(following.borrow().is_none(), "the second Escape stops following");
}
