//! The arrow of a select: the stroked chevron by default, the filled down triangle under `DropdownArrow::Triangle`.

use craft_ui::contract::{Bind, Entry, Show as _};
use craft_ui::controls::disclosure::DropdownArrow;
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, trial};
use egui::Shape;

/// Whether a select shown under `arrow` (`None`: the app sets none) paints a filled triangle, and whether it paints a
/// stroked chevron (an open three-point line), inside its rect.
fn arrow_shapes(arrow: Option<DropdownArrow>) -> (bool, bool) {
    let rect = std::rc::Rc::new(std::cell::Cell::new(egui::Rect::NOTHING));
    let seen = rect.clone();
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "One"), Entry::new(1u8, "Two")];
        seen.set(Select::new("Pick", Bind::given(1u8), &entries).show(ui).response.rect);
    });
    if let Some(arrow) = arrow {
        fixture.root_value(arrow);
    }
    fixture.settle();
    let select = rect.get();
    let shapes = fixture.frame_shapes();
    let inside = |path: &egui::epaint::PathShape| path.points.len() == 3 && select.contains_rect(path.visual_bounding_rect());
    let triangle = shapes.iter().any(|shape| matches!(shape, Shape::Path(path) if inside(path) && path.fill != egui::Color32::TRANSPARENT));
    let chevron =
        shapes.iter().any(|shape| matches!(shape, Shape::Path(path) if inside(path) && !path.closed && path.fill == egui::Color32::TRANSPARENT));
    (triangle, chevron)
}

/// Upstream's default: a select ends in the fine stroked chevron.
#[trial]
fn a_select_ends_in_a_chevron_by_default() {
    let (triangle, chevron) = arrow_shapes(None);
    assert!(chevron && !triangle, "the select paints a stroked chevron for its arrow (triangle {triangle}, chevron {chevron})");
}

/// PdfCraft's look (`DropdownArrow::Triangle`): a small filled triangle pointing down, as in a document app's choice
/// boxes, not a stroked chevron.
#[trial]
fn a_select_ends_in_a_filled_down_triangle_when_the_app_asks() {
    let (triangle, chevron) = arrow_shapes(Some(DropdownArrow::Triangle));
    assert!(triangle && !chevron, "the select paints a filled triangle for its arrow (triangle {triangle}, chevron {chevron})");
}
