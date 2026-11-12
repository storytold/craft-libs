//! `LayersPanel` rows the app reads back: the hovered layer, a Smart Filter row selected on its own, its Blending
//! Options button, and a frame's masks after its content thumbnail: linked and dragged by their own index.

use craft_behavior::{EditEvent, SelectionChange};
use craft_layers::testing;
use craft_layers::ui::mock::{FakeLayers, sub_row};
use craft_layers::ui::panel::{ActivateOn, LayerDrop, LayersEvent, LayersPanel, MaskMove};
use craft_layers::ui::tree::{LayerInfo, LayerKind, LayerPart, LayerRowKey, LayerSelection, MaskInfo, SubRow, SubRowGroup, SubRows};
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The panel's room: wide enough for a row's parts and inside the 800 by 600 screen of a fixture.
const PANEL: Vec2 = Vec2::new(420.0, 560.0);

/// The smart object of [`tree`].
const SMART: u32 = 2;

/// The app around the panel: the tree it shows, what it reported and the layer it last called hovered.
struct App {
    tree: FakeLayers,
    events: Vec<LayersEvent<u32>>,
    hovered: Option<u32>,
}

type Shared = Rc<RefCell<App>>;

/// A type layer, a smart object with two Smart Filters shown, each with its Blending Options button, and the
/// Background.
fn tree() -> FakeLayers {
    let mut tree = FakeLayers::default();
    tree.add(None, LayerInfo::new("Title", LayerKind::Text), false);
    let mut smart = LayerInfo::new("Smart Object", LayerKind::SmartObject);
    smart.has_smart_filters = true;
    let id = tree.add(None, smart, false);
    let layer = tree.layer_mut(id);
    layer.effects_open = true;
    let filter = |label: &str| {
        let mut row = sub_row(label);
        row.options = true;
        row
    };
    layer.smart_filters = Some(SubRows::new(SubRow::new("Smart Filters"), vec![filter("Gaussian Blur"), filter("Sharpen")]));
    tree.add(None, LayerInfo::new("Background", LayerKind::Pixel), false);
    tree.selection = LayerSelection::new(vec![1], Some(1), LayerPart::Pixels);
    tree
}

/// Applies a selection change as an app that keeps a selected sub-row does.
fn apply(tree: &mut FakeLayers, event: &LayersEvent<u32>) {
    if let LayersEvent::Select(select) = event
        && let SelectionChange::Set(layers) = &select.change
    {
        tree.selection = LayerSelection::new(layers.clone(), layers.last().copied(), select.part.unwrap_or(LayerPart::Pixels));
        tree.selection.sub_row = select.row.clone();
    }
}

fn rig(tree: FakeLayers) -> (Fixture, Shared) {
    let app = Rc::new(RefCell::new(App { tree, events: Vec::new(), hovered: None }));
    let shared = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shared.borrow_mut();
        let App { tree, events, hovered } = &mut *app;
        let out = LayersPanel::new(&*tree).show(ui);
        for event in &out.events {
            apply(tree, event);
        }
        events.extend(out.events);
        *hovered = out.hovered;
    });
    fixture.room(PANEL);
    fixture.settle();
    (fixture, app)
}

fn take(app: &Shared) -> Vec<LayersEvent<u32>> {
    std::mem::take(&mut app.borrow_mut().events)
}

fn node(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| panic!("no node named {label:?}"))
}

/// The sub-rows the selections among `events` name.
fn picked_rows(events: &[LayersEvent<u32>]) -> Vec<Option<LayerRowKey<u32>>> {
    events.iter().filter_map(|event| if let LayersEvent::Select(select) = event { Some(select.row.clone()) } else { None }).collect()
}

/// The layer row under the pointer is reported as hovered; an effects or Smart Filter row and the room away from the rows
/// are not.
#[trial]
fn the_hovered_layer_row_is_reported_and_a_sub_row_is_not() {
    let (mut fixture, app) = rig(tree());
    let [title, smart, filter] = ["Title", "Smart Object", "Gaussian Blur"].map(|label| node(&fixture, label).center());
    let mut hovered = |at| {
        fixture.frame(vec![pointer::moved(at)]).idle(1);
        app.borrow().hovered
    };
    assert_eq!(hovered(title), Some(1), "over a layer row");
    assert_eq!(hovered(smart), Some(SMART), "over the next layer row");
    assert_eq!(hovered(filter), None, "over a Smart Filter row");
    fixture.frame(vec![pointer::gone()]).idle(1);
    assert_eq!(app.borrow().hovered, None, "with the pointer gone");
}

/// A click on a Smart Filter row selects its layer and names the row; once the app keeps that row selected it holds the
/// lead, so Down moves to the next filter. A click on a layer row names no sub-row.
#[trial]
fn a_smart_filter_row_is_selected_on_its_own_and_holds_the_lead() {
    let (mut fixture, app) = rig(tree());
    take(&app);
    fixture.click(&node(&fixture, "Gaussian Blur")).idle(2);
    let events = take(&app);
    assert!(
        events.iter().any(|event| matches!(event, LayersEvent::Select(select) if select.change == SelectionChange::Set(vec![SMART]))),
        "the click selects the smart object: {events:?}"
    );
    assert_eq!(picked_rows(&events), [Some(LayerRowKey::Filters(SMART, Some(0)))], "and names the filter row");
    fixture.key(Key::ArrowDown, Modifiers::NONE).idle(2);
    assert_eq!(picked_rows(&take(&app)), [Some(LayerRowKey::Filters(SMART, Some(1)))], "Down moves from the selected filter row");
    fixture.click(&node(&fixture, "Title")).idle(2);
    assert_eq!(picked_rows(&take(&app)), [None], "a layer row names no sub-row");
}

/// The Blending Options button of a Smart Filter row opens on a double click, not on one click, and leaves the selection.
#[trial]
fn the_blending_options_button_opens_on_a_double_click() {
    let (mut fixture, app) = rig(tree());
    let button = node(&fixture, "Blending Options (Gaussian Blur)").center();
    let options = |events: &[LayersEvent<u32>]| -> Vec<LayerRowKey<u32>> {
        events
            .iter()
            .filter_map(|event| match event {
                LayersEvent::Activate { row, on: ActivateOn::Options, .. } => Some(row.clone()),
                _ => None,
            })
            .collect()
    };
    take(&app);
    fixture.click_at(button).settle();
    let once = take(&app);
    assert!(options(&once).is_empty(), "one click opens nothing: {once:?}");
    fixture.click_at(button).settle();
    let twice = take(&app);
    assert_eq!(options(&twice), [LayerRowKey::Filters(SMART, Some(0))], "the double click opens the filter's options");
    assert!(!once.iter().chain(&twice).any(|event| matches!(event, LayersEvent::Select(_))), "the button leaves the selection");
}

/// A frame's masks come after its content thumbnail: the chain before its layer mask links that mask.
#[trial]
fn a_frames_mask_chain_links_the_mask_after_the_content() {
    let mut frame = LayerInfo::new("Frame 1", LayerKind::Frame);
    frame.masks.push(MaskInfo::new(LayerPart::LayerMask, true, true));
    let mut tree = FakeLayers::default();
    tree.add(None, frame, false);
    let (mut fixture, app) = rig(tree);
    take(&app);
    fixture.click(&node(&fixture, "Unlink layer mask of Frame 1")).idle(2);
    let links: Vec<_> = take(&app)
        .into_iter()
        .filter_map(|event| if let LayersEvent::MaskLink { layer, part, linked, .. } = event { Some((layer, part, linked)) } else { None })
        .collect();
    assert_eq!(links, [(1, LayerPart::LayerMask, false)]);
}

/// A frame's layer mask thumbnail, after its content thumbnail, drags the mask: dropped on another layer it moves there.
#[trial]
fn a_frames_mask_thumbnail_drags_the_mask() {
    let mut frame = LayerInfo::new("Frame 1", LayerKind::Frame);
    frame.masks.push(MaskInfo::new(LayerPart::LayerMask, true, true));
    let mut tree = FakeLayers::default();
    tree.add(None, LayerInfo::new("Title", LayerKind::Text), false);
    let id = tree.add(None, frame.clone(), false);
    let (mut fixture, app) = rig(tree);
    take(&app);
    let row = node(&fixture, "Frame 1").node.into();
    let from = testing::thumb_rect(fixture.ctx(), row, &frame, LayerPart::LayerMask).expect("the frame's layer mask thumbnail").center();
    let to = node(&fixture, "Title").center();
    fixture.drag_hold_at(from, to - from).release_at(to).idle(2);
    let moves: Vec<_> = take(&app)
        .into_iter()
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Mask(mv), .. }) => Some(mv),
            _ => None,
        })
        .collect();
    assert_eq!(moves, [MaskMove::new(id, LayerPart::LayerMask, 1, false)]);
}

/// The Activate events on a thumbnail among `events`: the layer and the part.
fn thumb_activations(events: &[LayersEvent<u32>]) -> Vec<(LayerRowKey<u32>, LayerPart)> {
    events
        .iter()
        .filter_map(|event| match event {
            LayersEvent::Activate { row, on: ActivateOn::Thumbnail(part), .. } => Some((row.clone(), *part)),
            _ => None,
        })
        .collect()
}

/// A double click on a layer's thumbnail activates that thumbnail once; a single click only selects, and a double click on
/// the name activates no thumbnail.
#[trial]
fn a_double_click_on_a_thumbnail_activates_it_once() {
    let info = LayerInfo::new("Title", LayerKind::Text);
    let (mut fixture, app) = rig(tree());
    take(&app);
    let row = node(&fixture, "Title").node.into();
    let thumb = testing::thumb_rect(fixture.ctx(), row, &info, LayerPart::Pixels).expect("the layer's thumbnail").center();
    fixture.click_at(thumb).settle();
    let once = take(&app);
    assert!(thumb_activations(&once).is_empty(), "one click activates nothing: {once:?}");
    assert!(once.iter().any(|event| matches!(event, LayersEvent::Select(_))), "one click selects: {once:?}");
    fixture.click_at(thumb).settle();
    let twice = take(&app);
    assert_eq!(thumb_activations(&twice), [(LayerRowKey::Layer(1), LayerPart::Pixels)], "the double click: {twice:?}");
    assert!(
        !twice.iter().any(|event| matches!(event, LayersEvent::Activate { on: ActivateOn::Row, .. })),
        "the thumbnail is not also a row activation: {twice:?}"
    );
    let name = node(&fixture, "Smart Object").center();
    fixture.click_at(name).settle();
    fixture.click_at(name).settle();
    let named = take(&app);
    assert!(thumb_activations(&named).is_empty(), "a double click on the name is no thumbnail activation: {named:?}");
}

/// A double click whose two clicks arrive in one frame, on a Smart Filter row of a smart object that is not active,
/// selects the row and then activates it, as two clicks in two frames do.
#[trial]
fn a_double_click_in_one_frame_selects_a_smart_filter_row_and_activates_it() {
    let (mut fixture, app) = rig(tree());
    take(&app);
    let filter = node(&fixture, "Gaussian Blur").center();
    fixture.frame(vec![pointer::moved(filter)]).idle(1);
    fixture.frame(vec![pointer::press(filter), pointer::release(filter), pointer::press(filter), pointer::release(filter)]).settle();
    let row = LayerRowKey::Filters(SMART, Some(0));
    let events = take(&app);
    let selected = events.iter().position(|event| matches!(event, LayersEvent::Select(select) if select.row.as_ref() == Some(&row)));
    let activated = events.iter().position(|event| matches!(event, LayersEvent::Activate { row: on, on: ActivateOn::Row, .. } if *on == row));
    assert!(selected.is_some() && activated.is_some() && selected < activated, "selected, then activated: {events:?}");
}

/// A Smart Filter row dragged onto another smart object's row moves the filter there, with the active layer changing to
/// the filter's smart object when the press selects its row. With the filter row selected beside its layer, a drag of
/// the layer's row carries the layer alone.
#[trial]
fn a_smart_filter_row_dragged_onto_another_smart_object_moves_there() {
    let mut tree = tree();
    let other = tree.add(None, LayerInfo::new("Other Smart Object", LayerKind::SmartObject), false);
    let (mut fixture, app) = rig(tree);
    take(&app);
    let from = node(&fixture, "Gaussian Blur").center();
    let onto = node(&fixture, "Other Smart Object").rect;
    let to = egui::pos2(onto.center().x, onto.bottom() - onto.height() / 4.0);
    fixture.drag_hold_at(from, to - from).release_at(to).idle(2);
    let events = take(&app);
    let transfers: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Transfer(t), .. }) => Some((t.rows.clone(), t.to, t.copy)),
            _ => None,
        })
        .collect();
    assert_eq!(transfers, [(vec![LayerRowKey::Filters(SMART, Some(0))], other, false)], "{events:?}");
    // The rig applies selections only, so the filter stays on its layer and its row stays selected for this drag.
    let from = node(&fixture, "Smart Object").center();
    fixture.drag_hold_at(from, to - from).release_at(to).idle(2);
    let events = take(&app);
    let moved: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Layers(mv), .. }) => Some(mv.layers.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(moved, [vec![SMART]], "{events:?}");
}

/// A stacking key on a selected Smart Filter row, selected beside its layer, moves that filter within its stack.
#[trial]
fn a_stacking_key_on_a_selected_smart_filter_row_moves_the_filter() {
    let (mut fixture, app) = rig(tree());
    fixture.click(&node(&fixture, "Gaussian Blur")).idle(2);
    take(&app);
    fixture.key(Key::OpenBracket, Modifiers::COMMAND).idle(2);
    let events = take(&app);
    let moves: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::SubRows(mv), .. }) => Some((mv.layer, mv.group, mv.from, mv.to)),
            _ => None,
        })
        .collect();
    assert_eq!(moves, [(SMART, SubRowGroup::SmartFilters, 0, 1)], "{events:?}");
}
