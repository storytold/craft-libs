//! `LayersPanel` eye boxes: a hidden layer's box stays empty, also under the pointer (photocraft#16321).

use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::LayersPanel;
use craft_layers::ui::tree::{LayerInfo, LayerKind};
use craft_ui::contract::Tracked;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A visible layer over a hidden one.
fn rig() -> Fixture {
    let mut tree = FakeLayers::default();
    tree.add(None, LayerInfo::new("Shown", LayerKind::Pixel), false);
    let mut hidden = LayerInfo::new("Hidden", LayerKind::Pixel);
    hidden.visible = false;
    tree.add(None, hidden, false);
    let tree = Rc::new(RefCell::new(Tracked::new(tree)));
    let mut fixture = Fixture::new(move |ui| {
        let tree = tree.borrow();
        let _ = LayersPanel::new(&tree).show(ui);
    });
    fixture.room(Vec2::new(420.0, 560.0));
    fixture.settle();
    fixture
}

/// How many shapes the frame paints wholly inside the eye box `cell`: its icon, not the row's fill around it.
fn drawn_in(fixture: &mut Fixture, cell: Rect) -> usize {
    fixture.frame_shapes().iter().filter(|shape| cell.contains_rect(shape.visual_bounding_rect())).count()
}

/// The pointer over a hidden layer's eye box draws nothing there (no crossed-out eye), while a visible layer's box
/// under the pointer draws its eye.
#[trial]
fn a_hidden_layers_eye_box_stays_empty_under_the_pointer() {
    let mut fixture = rig();
    let shown = fixture.by_label("Visibility Shown").unwrap_or_else(|| panic!("no eye box for Shown")).rect;
    let hidden = fixture.by_label("Visibility Hidden").unwrap_or_else(|| panic!("no eye box for Hidden")).rect;

    fixture.frame(vec![pointer::moved(shown.center())]).idle(2);
    assert!(drawn_in(&mut fixture, shown) > 0, "the visible layer's eye is drawn");

    fixture.frame(vec![pointer::moved(hidden.center())]).idle(2);
    assert_eq!(drawn_in(&mut fixture, hidden), 0, "the hidden layer's box stays empty under the pointer");
}
