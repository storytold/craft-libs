//! A face's own access facts reach its part's sub-node in the composite that paints it.

use craft_behavior::PartKey;
use craft_theme::Srgba;
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const SWATCH: PartKey = PartKey::new("swatch");

/// A swatch face: it words its colour and reports the colour.
#[derive(craft_ui::contract::Component)]
struct Chip {
    #[common]
    common: Common,
}

impl Component for Chip {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: ChipStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.value("Coral").color(Srgba::rgb(1.0, 0.5, 0.31));
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// One part painted with a `Chip` face, faded to `fade` when set; its own `access` declares no node for the part.
#[derive(craft_ui::contract::Component)]
struct Palette {
    #[common]
    common: Common,
    fade: Option<f32>,
}

impl Component for Palette {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: PaletteStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(60.0, 30.0)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(SWATCH, rect);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        let chip = Chip { common: Common::new() }.label("Swatch");
        match self.fade {
            Some(share) => ui.face_faded(SWATCH, chip, share),
            None => ui.face(SWATCH, chip),
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

// PLAN: the access tree has no public colour reader (runtime/egui/access/tree.rs, outside this set); the colour takes
// the same record, facts and sub-node lines as the value, which the trial reads.
#[trial]
fn a_face_in_a_composite_reports_its_value_and_colour() {
    let root = Rc::new(Cell::new(Id::NULL));
    let shown = Rc::clone(&root);
    let mut fixture = Fixture::new(move |ui| {
        shown.set(Palette { common: Common::new(), fade: None }.label("Palette").show(ui).id);
    });
    fixture.settle();
    let part = fixture.part(root.get(), SWATCH).unwrap_or_else(|| panic!("the face's part has a sub-node"));
    assert_eq!(fixture.tree().value_of(part.node), Some("Coral"), "the part's sub-node carries the value the face's access set");
}

/// A face painted faded (a component fading out where it stood) is a look only: its part gets no sub-node and its
/// name reaches no node. Guards `PaintUi::face_faded` reporting the face as `face` does.
#[trial]
fn a_faded_face_reports_no_sub_node() {
    let root = Rc::new(Cell::new(Id::NULL));
    let shown = Rc::clone(&root);
    let mut fixture = Fixture::new(move |ui| {
        shown.set(Palette { common: Common::new(), fade: Some(0.5) }.label("Palette").show(ui).id);
    });
    fixture.settle();
    assert!(fixture.part(root.get(), SWATCH).is_none(), "a faded face gives its part no sub-node");
    assert!(fixture.by_label("Swatch").is_none(), "no node carries the faded face's name");
}
