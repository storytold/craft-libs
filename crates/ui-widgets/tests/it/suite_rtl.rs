//! The suite's right-to-left case compares a component's own rect between directions: a component that changes its
//! own width under right-to-left fails it, and so does content that does not follow the component's rect when that
//! rect moves.

use craft_ui::contract::{Bind, Common, Component, Cx, PartsCx, Props as _, Role, Show as _};
use craft_ui::runtime::env::{Direction, Env};
use craft_ui::style::StyleCx;
use craft_ui::testing::story::{Story, StoryData, StoryOutput};
use craft_ui::testing::suite::rtl_case;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Layout, Rect, Response, Ui, UiBuilder, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;

const FIELD_WIDTH: f32 = 200.0;
const SHIFT: f32 = 40.0;

/// A search field with text, so that it has a clear part at its end, in the rect starting at `at`.
fn search_field(ui: &mut Ui, at: egui::Pos2) -> StoryOutput {
    let rect = Rect::from_min_size(at, vec2(FIELD_WIDTH, 28.0));
    let mut text = "text".to_owned();
    let builder = UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(egui::Align::Min));
    ui.scope_builder(builder, |ui| TextField::search("Search", Bind::bound(&mut text)).show(ui)).inner.into()
}

/// As above, plus a second field that keeps its place while the first one moves.
fn leaves_content_behind(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shift = if Env::of(ui).view().direction().is_rtl() { SHIFT } else { 0.0 };
    let shown = search_field(ui, pos2(shift, 0.0));
    search_field(ui, pos2(0.0, 40.0));
    shown
}

#[trial]
fn the_rtl_case_fails_content_that_keeps_its_ltr_offsets_while_the_rect_moves() {
    let failed = rtl_case(&Story::new("leaves-content", leaves_content_behind)).expect_err("the second field does not move");
    assert!(failed.contains("not its reflection"), "{failed}");
}

/// A component with a part at the start of its flow; an instance declares `ltr_axis` for a horizontal axis that keeps
/// its left-to-right order.
#[derive(craft_ui::contract::Component)]
struct TimeAxis {
    #[common]
    common: Common,
    start: Rc<Cell<(Rect, Rect)>>,
}

impl Component for TimeAxis {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TimeAxisStyleKey) -> PlainStyle {
        PlainStyle::min(vec2(200.0, 20.0))
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (start, _) = cx.flow.take_start(10.0);
        self.start.set((rect, start));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Of two parts of one column, the one that declares `ltr_axis` flows from the left and the other mirrors.
#[trial]
fn a_part_that_keeps_its_axis_leaves_the_other_parts_of_its_container_mirrored() {
    let seen = [Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING))), Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)))];
    let shown = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        ui.vertical(|ui| {
            TimeAxis { common: Common::new(), start: shown[0].clone() }.ltr_axis().id_salt("kept").show(ui);
            TimeAxis { common: Common::new(), start: shown[1].clone() }.id_salt("mirrored").show(ui);
        });
    });
    fixture.direction(Direction::Rtl);
    fixture.settle();
    let ((kept_rect, kept), (rect, mirrored)) = (seen[0].get(), seen[1].get());
    assert_eq!(kept.min.x, kept_rect.min.x, "the declaring part flows from the left");
    assert_eq!(mirrored.max.x, rect.max.x, "the other part flows from the right");
}
