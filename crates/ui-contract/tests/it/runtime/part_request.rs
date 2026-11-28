//! A request for a part lands on that part: one declared in the pass its owner asks for it (from `update`, before the
//! stops register) waits for its stop instead of landing on another of the owner's parts.

use craft_behavior::{Input, PartKey, PartRef};
use craft_ui::contract::{Common, Component, Cx, Focus as _, MeasureCx, PartsCx, Props as _, Role, Show as _};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Rect, Vec2, pos2};
use std::cell::Cell;
use std::rc::Rc;

const CELL: PartKey = PartKey::new("cell");

/// Cells 0 and 2 always, cell 1 once `late` is set; a focused row asks for cell 1 from `update` in the pass it shows.
#[derive(craft_ui::contract::Component)]
struct Row {
    #[common]
    common: Common,
    late: bool,
}

impl Component for Row {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn keys(&self) -> KeyUse {
        KeyUse::of(Input::Pointer)
    }

    fn style(_: &StyleCx<'_>, _: RowStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(150.0, 30.0)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let third = rect.width() / 3.0;
        let cell = |at: f32| Rect::from_min_max(pos2(rect.min.x + third * at, rect.min.y), pos2(rect.min.x + third * (at + 1.0), rect.max.y));
        cx.part(CELL.at(0), cell(0.0)).focusable();
        if self.late {
            cx.part(CELL.at(1), cell(1.0)).focusable();
        }
        cx.part(CELL.at(2), cell(2.0)).focusable();
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.late && cx.is_focused() && cx.focus_arrived().is_none() {
            cx.focus_part(CELL.at(1));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

#[trial]
fn a_part_asked_for_in_the_pass_it_appears_takes_the_focus_not_a_sibling() {
    let late = Rc::new(Cell::new(false));
    let root = Rc::new(Cell::new(Id::NULL));
    let (shown, seen) = (Rc::clone(&late), Rc::clone(&root));
    let mut fixture = Fixture::new(move |ui| {
        let response = Row { common: Common::new(), late: shown.get() }.label("Row").show(ui);
        seen.set(response.id);
    });
    fixture.settle();
    let id = root.get();
    request(fixture.ctx(), FocusTarget::Part(id, CELL.at(2)));
    fixture.settle();
    assert_eq!(fixture.focused_stop(), Some((id, Some(CELL.at(2)))), "the last cell has the focus");
    late.set(true);
    fixture.settle();
    let middle: PartRef = CELL.at(1);
    assert_eq!(fixture.focused_stop(), Some((id, Some(middle))), "the cell asked for takes the focus, not the first cell");
}
