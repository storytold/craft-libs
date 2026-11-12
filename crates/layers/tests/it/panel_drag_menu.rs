//! `LayersPanel` drags and menus: a mask thumbnail dropped on another layer, what a drag carries for the app's own
//! drop targets, and the row menu's Label Color submenu.

use craft_behavior::EditEvent;
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayerDrop, LayersDrag, LayersEvent, LayersPanel, MaskMove};
use craft_layers::ui::tree::LayerPart;
use craft_theme::LabelColor;
use craft_ui::contract::Entry;
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Modifiers, Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<LayersEvent<u32>>>>;

/// The room of the panel: wide enough for a row's parts, tall enough for the sample tree and the footer.
const PANEL: Vec2 = Vec2::new(420.0, 560.0);

/// The panel over the sample tree, with a row menu of one entry, and the events it reports.
fn rig() -> (Fixture, Seen) {
    let seen: Seen = Rc::default();
    let sink = Rc::clone(&seen);
    let tree = FakeLayers::sample();
    let mut fixture = Fixture::new(move |ui| {
        let out = LayersPanel::new(&tree).row_menu(|_| vec![Entry::new(String::from("rename"), "Rename Layer…")]).show(ui);
        sink.borrow_mut().extend(out.events);
    });
    fixture.room(PANEL);
    fixture.settle();
    (fixture, seen)
}

fn node(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| panic!("no node named {label:?}"))
}

/// A point on the layer mask thumbnail of `layer`, which follows its link chain.
fn mask_thumb(fixture: &Fixture, layer: &str) -> Pos2 {
    let link = node(fixture, &format!("Unlink layer mask of {layer}"));
    Pos2::new(link.rect.right() + link.rect.width() * 0.75, link.rect.center().y)
}

/// Drags from `from` and releases on the middle of the row named `onto`.
fn drag_onto(fixture: &mut Fixture, from: Pos2, onto: &str) {
    let to = node(fixture, onto).center();
    fixture.drag_hold_at(from, to - from).release_at(to).idle(2);
}

/// The mask drops among the events reported since the last call.
fn mask_drops(seen: &Seen) -> Vec<MaskMove<u32>> {
    seen.borrow_mut()
        .drain(..)
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Mask(mv), .. }) => Some(mv),
            _ => None,
        })
        .collect()
}

/// A layer mask's thumbnail dropped on another layer's row moves the mask there and Alt copies it; a layer that has a
/// layer mask already takes none.
#[trial]
fn a_mask_thumbnail_dropped_on_another_layer_moves_the_mask_there() {
    let (mut fixture, seen) = rig();
    let from = mask_thumb(&fixture, "Masked");
    drag_onto(&mut fixture, from, "Title");
    assert_eq!(mask_drops(&seen), [MaskMove::new(3, LayerPart::LayerMask, 4, false)], "onto Title");
    fixture.hold(Modifiers::ALT);
    drag_onto(&mut fixture, from, "Title");
    fixture.hold(Modifiers::NONE);
    assert_eq!(mask_drops(&seen), [MaskMove::new(3, LayerPart::LayerMask, 4, true)], "Alt copies");
    drag_onto(&mut fixture, from, "Levels 1");
    assert_eq!(mask_drops(&seen), [], "Levels 1 has a layer mask already");
}

/// While layer rows are dragged, the drag session carries their layers for the app's targets outside the panel (a
/// document tab); a mask's drag carries no layers.
#[trial]
fn a_row_drag_carries_its_layers_for_targets_outside_the_panel() {
    let (mut fixture, _) = rig();
    let held = |fixture: &Fixture| craft_ui::runtime::drag::active::<LayersDrag<u32>>(fixture.ctx()).expect("a drag of the panel is held");
    let title = node(&fixture, "Title").center();
    let away = Vec2::new(0.0, 40.0);
    fixture.drag_hold_at(title, away);
    assert_eq!(held(&fixture).layers(), [&4], "the dragged row's layer");
    fixture.release_at(title + away).idle(2);
    let mask = mask_thumb(&fixture, "Masked");
    fixture.drag_hold_at(mask, away);
    assert!(held(&fixture).layers().is_empty(), "a mask is no layer");
    fixture.release_at(mask + away).idle(2);
}

/// The row menu ends with the Label Color submenu, whose pick colours the row's layer as the eye's menu does.
#[trial]
fn the_row_menus_label_color_submenu_colours_the_layer() {
    let (mut fixture, seen) = rig();
    let title = node(&fixture, "Title").center();
    fixture.frame(vec![pointer::moved(title)]);
    fixture.frame(vec![pointer::secondary(title, true)]);
    fixture.frame(vec![pointer::secondary(title, false)]);
    fixture.idle(3);
    let submenu = node(&fixture, "Label Color");
    fixture.click(&submenu).idle(3);
    let red = node(&fixture, "Red");
    fixture.click(&red).idle(2);
    let picked: Vec<(Vec<u32>, Option<LabelColor>)> = seen
        .borrow()
        .iter()
        .filter_map(|event| match event {
            LayersEvent::LabelColor { rows, color, .. } => Some((rows.clone(), *color)),
            _ => None,
        })
        .collect();
    assert_eq!(picked, [(vec![4], Some(LabelColor::Red))]);
}
