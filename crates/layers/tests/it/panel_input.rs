//! `LayersPanel` input: an eye sweep as one undo step, and where a dragged layer lands.

use craft_behavior::edit_history::Step;
use craft_behavior::{EditEvent, EditHistory, Mixed, SelectionChange};
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayerColumn, LayerDrop, LayerPosition, LayerToggle, LayersEvent, LayersPanel};
use craft_layers::ui::tree::{LayerKind, LayerPart, LayerSelection};
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Pos2, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The app around the panel: the tree it shows and what it reported.
struct App {
    tree: FakeLayers,
    events: Vec<LayersEvent<u32>>,
}

/// A size of `width` by `height` points.
const fn by(width: f32, height: f32) -> Vec2 {
    Vec2::new(width, height)
}

/// The room of the panel: wide enough for a row's parts, tall enough for the sample tree and the footer, and inside the
/// 800 by 600 screen of a fixture.
const PANEL: Vec2 = by(420.0, 560.0);

/// A pointer move that starts a drag.
const DRAG_STEP: Vec2 = by(0.0, 8.0);

type Shared = Rc<RefCell<App>>;

/// Applies what the panel reported to the tree, as an app would.
fn apply(tree: &mut FakeLayers, event: &LayersEvent<u32>) {
    match event {
        LayersEvent::Select(select) => {
            let part = select.part.unwrap_or(LayerPart::Pixels);
            match &select.change {
                SelectionChange::Set(layers) => tree.selection = LayerSelection::new(layers.clone(), layers.last().copied(), part),
                SelectionChange::Toggle(layer) => {
                    if tree.selection.layers.contains(layer) {
                        tree.selection.layers.retain(|held| held != layer);
                    } else {
                        tree.selection.layers.push(*layer);
                    }
                    tree.selection.active = Some(*layer);
                }
                SelectionChange::Clear => tree.selection = LayerSelection::default(),
                _ => {}
            }
        }
        LayersEvent::Open(change) => {
            for layer in tree.layers.iter_mut().filter(|layer| layer.id == change.key || (change.all && layer.info.kind == LayerKind::Group)) {
                layer.open = change.open;
            }
        }
        LayersEvent::FxOpen { layer, open, all, .. } => {
            for held in tree.layers.iter_mut().filter(|held| held.id == *layer || *all) {
                held.effects_open = *open;
            }
        }
        _ => {}
    }
}

fn show(ui: &mut Ui, app: &mut App) {
    let App { tree, events } = app;
    let out = LayersPanel::new(&*tree).show(ui);
    for event in &out.events {
        apply(tree, event);
    }
    events.extend(out.events);
}

/// The panel over `tree`, in a room tall enough for every row.
fn rig(tree: FakeLayers) -> (Fixture, Shared) {
    let app = Rc::new(RefCell::new(App { tree, events: Vec::new() }));
    let shared = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| show(ui, &mut shared.borrow_mut()));
    fixture.room(PANEL);
    fixture.settle();
    (fixture, app)
}

fn take(app: &Shared) -> Vec<LayersEvent<u32>> {
    std::mem::take(&mut app.borrow_mut().events)
}

/// The node named `label`.
fn node(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| {
        let names: Vec<_> = fixture.tree().keys().into_iter().filter_map(|key| fixture.tree().name_of(key)).collect();
        panic!("no node named {label:?}; the tree has {names:?}")
    })
}

/// The list row named `name`.
fn row(fixture: &Fixture, name: &str) -> Target {
    node(fixture, name)
}

/// The point `share` of the way down `target`, in the middle of its width.
fn at(target: &Target, share: f32) -> Pos2 {
    Pos2::new(target.rect.center().x, target.rect.top() + target.rect.height() * share)
}

/// Press at `from`, move past the drag threshold and on to `to`, release there.
fn drag_to(fixture: &mut Fixture, from: Pos2, to: Pos2) {
    fixture.idle(6);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(from + DRAG_STEP)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::release(to)]);
    fixture.idle(2);
}

fn toggles(events: &[LayersEvent<u32>]) -> Vec<&LayerToggle<u32>> {
    events.iter().filter_map(|event| if let LayersEvent::Toggle(toggle) = event { Some(toggle) } else { None }).collect()
}

/// The layers, target, place and copy flag of the first layer drop reported.
fn layer_drop(events: &[LayersEvent<u32>]) -> Option<(Vec<u32>, u32, LayerPosition, bool)> {
    events.iter().find_map(|event| match event {
        LayersEvent::Drop(EditEvent::Set { after, .. } | EditEvent::Commit { after, .. }) => match after {
            LayerDrop::Layers(mv) => Some((mv.layers.clone(), mv.target, mv.position, mv.copy)),
            _ => None,
        },
        _ => None,
    })
}

/// The eyes of layers 4 to 6 as a host keeps them, with the history that records the toggles it is given.
struct Eyes {
    now: Vec<bool>,
    history: EditHistory<Vec<bool>>,
    /// The state at the open gesture's `Begin`.
    begun: Vec<bool>,
}

impl Eyes {
    fn new() -> Self {
        Eyes { now: vec![true; 3], history: EditHistory::new(&vec![true; 3]), begun: vec![true; 3] }
    }

    /// Applies the toggles among `events`, as an app would.
    fn take(&mut self, events: &[LayersEvent<u32>]) {
        for toggle in toggles(events) {
            let rows: Vec<usize> = toggle.rows.iter().map(|row| *row.layer() as usize - 4).collect();
            match &toggle.edit {
                EditEvent::Begin { id, label, .. } => {
                    self.begun = self.now.clone();
                    self.history.record(&EditEvent::begin(*id, Mixed::Same(self.now.clone()), label.clone(), None));
                }
                EditEvent::Update { value, .. } => rows.iter().for_each(|row| self.now[*row] = *value),
                EditEvent::Set { after, label, .. } => {
                    let before = self.now.clone();
                    rows.iter().for_each(|row| self.now[*row] = *after);
                    self.history.record(&EditEvent::set(Mixed::Same(before), self.now.clone(), label.clone(), None));
                }
                EditEvent::Commit { id, .. } => self.history.record(&EditEvent::commit(*id, Mixed::Same(self.begun.clone()), self.now.clone())),
                EditEvent::Cancel { id, reason, before: Some(Mixed::Same(before)), .. } => {
                    rows.iter().for_each(|row| self.now[*row] = *before);
                    self.history.record(&EditEvent::cancel(*id, *reason, None));
                }
                _ => {}
            }
        }
    }
}

/// A sweep down the eyes is one undo step: one Undo through the host's history restores every row it crossed.
#[trial]
fn a_sweep_down_the_eyes_is_one_undo_step() {
    let (mut fixture, app) = rig(FakeLayers::sample());
    let (start, end) = (node(&fixture, "Visibility Title").center(), node(&fixture, "Visibility Smart Object").center());
    let mut eyes = Eyes::new();
    fixture.frame(vec![pointer::moved(start)]);
    fixture.frame(vec![pointer::press(start)]);
    eyes.take(&take(&app));
    for step in [0.3, 0.6, 1.0] {
        fixture.frame(vec![pointer::moved(start + (end - start) * step)]);
        eyes.take(&take(&app));
    }
    fixture.frame(vec![pointer::release(end)]);
    fixture.idle(2);
    let events = take(&app);
    let sweep = toggles(&events);
    assert!(sweep.iter().all(|toggle| toggle.column == LayerColumn::Visibility), "{events:?}");
    eyes.take(&events);
    assert_eq!(eyes.now, vec![false; 3], "every crossed row is off after the sweep");
    assert!(eyes.history.can_undo(), "the sweep made a step");
    let mut state = eyes.now.clone();
    assert!(eyes.history.step(&mut state, Step::Undo));
    assert_eq!(state, vec![true; 3], "one Undo restores every crossed row");
    assert!(!eyes.history.can_undo(), "the sweep was one step");
}

/// Where a layer dragged from `from` and released at `to` lands.
fn dropped(fixture: &mut Fixture, app: &Shared, from: Pos2, to: Pos2) -> Option<(Vec<u32>, u32, LayerPosition, bool)> {
    take(app);
    drag_to(fixture, from, to);
    layer_drop(&take(app))
}

/// A drop is reported one way: into an open group at its top, else before the row it lands above, and after the last row only
/// when nothing follows. The middle band of a group row and the lower part of its header both land at the top of the group.
#[trial]
fn drop_zones_into_before_and_after_the_last_row() {
    let (mut fixture, app) = rig(FakeLayers::sample());
    let from = row(&fixture, "Hidden").center();
    let group = row(&fixture, "Group 1");
    for (share, target, position) in [(0.5, 1, LayerPosition::Into), (0.15, 1, LayerPosition::Before), (0.85, 1, LayerPosition::Into)] {
        let to = at(&group, share);
        assert_eq!(dropped(&mut fixture, &app, from, to), Some((vec![8], target, position, false)), "Group 1 at {share}");
    }
    let title = row(&fixture, "Title");
    let to = at(&title, 0.75);
    assert_eq!(
        dropped(&mut fixture, &app, from, to),
        Some((vec![8], 5, LayerPosition::Before, false)),
        "the lower half of Title lands above the next row"
    );
    let last = row(&fixture, "Background");
    let below = Pos2::new(last.rect.center().x, last.rect.bottom() + 20.0);
    assert_eq!(dropped(&mut fixture, &app, from, below), Some((vec![8], 9, LayerPosition::After, false)), "the space below the last row");
}
