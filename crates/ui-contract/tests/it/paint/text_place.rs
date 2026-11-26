//! `Draw::text` places a run in its rect by the run's alignment, mirrored under right-to-left.

use craft_ui::contract::{Common, Component, Cx, PaintUi, Role, Show as _};
use craft_ui::primitives::{HAlign, TextRun};
use craft_ui::runtime::env::Direction;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response};

const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(310.0, 40.0) };

/// Paints one short run across its whole rect.
#[derive(craft_ui::contract::Component)]
struct Placed {
    #[common]
    common: Common,
    align: HAlign,
}

impl Component for Placed {
    const ROLE: Role = Role::LABEL;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PlacedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().text(cx.rect, &TextRun::new("ab").align(self.align));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The room before and after the painted text, along the rect's width.
fn gaps(align: HAlign, direction: Direction) -> (f32, f32) {
    let mut fixture = Fixture::new(move |ui| {
        Placed { common: Common::new(), align }.show_at(ui, AT);
    });
    fixture.direction(direction).settle();
    let text = fixture.shape_bounds().iter().filter(|shape| AT.contains_rect(**shape)).fold(Rect::NOTHING, |all, shape| all.union(*shape));
    assert!(text.is_positive(), "the run painted inside its rect");
    (text.left() - AT.left(), AT.right() - text.right())
}

#[trial]
fn a_run_sits_in_its_rect_by_its_alignment() {
    let (before, after) = gaps(HAlign::Start, Direction::Ltr);
    assert!(before < after, "Start sits at the left: {before} < {after}");
    let (before, after) = gaps(HAlign::Center, Direction::Ltr);
    assert!((before - after).abs() <= 1.5, "Center sits in the middle: {before} ~ {after}");
    let (before, after) = gaps(HAlign::End, Direction::Ltr);
    assert!(after < before, "End sits at the right: {after} < {before}");
    let (before, after) = gaps(HAlign::Start, Direction::Rtl);
    assert!(after < before, "Start sits at the right under right-to-left: {after} < {before}");
    let (before, after) = gaps(HAlign::End, Direction::Rtl);
    assert!(before < after, "End sits at the left under right-to-left: {before} < {after}");
}
