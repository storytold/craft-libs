//! A face paints and reports its own inputs as a component does: a face given `selected(true)` is a selected node.

use craft_behavior::PartKey;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Rect, Vec2, pos2};
use std::cell::Cell;
use std::rc::Rc;

const FACE: PartKey = PartKey::new("face");

/// The face: a plain leaf.
#[derive(craft_ui::contract::Component)]
struct Mark {
    #[common]
    common: Common,
}

impl Component for Mark {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: MarkStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// Two parts, each painted with a `Mark` face; the first face is selected.
#[derive(craft_ui::contract::Component)]
struct Pair {
    #[common]
    common: Common,
}

impl Component for Pair {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: PairStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(100.0, 30.0)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let half = rect.width() / 2.0;
        for at in 0..2u64 {
            let left = rect.min.x + half * at as f32;
            cx.part(FACE.at(at), Rect::from_min_max(pos2(left, rect.min.y), pos2(left + half, rect.max.y)));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        for at in 0..2u64 {
            ui.face(FACE.at(at), Mark { common: Common::new() }.label("Mark").selected(at == 0));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

#[trial]
fn a_selected_face_reports_a_selected_node() {
    let root = Rc::new(Cell::new(Id::NULL));
    let shown = root.clone();
    let mut fixture = Fixture::new(move |ui| {
        shown.set(Pair { common: Common::new() }.label("Pair").show(ui).id);
    });
    fixture.settle();
    let tree = fixture.tree();
    let selected = |at: u64| fixture.part(root.get(), FACE.at(at)).map(|part| tree.selected_of(part.node));
    assert_eq!(selected(0), Some(Some(true)), "the face given selected(true) is a selected node");
    assert_ne!(selected(1), Some(Some(true)), "the other face is not");
}
