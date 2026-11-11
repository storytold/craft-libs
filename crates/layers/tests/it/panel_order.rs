//! `LayersPanel`: the order of the rows, a filter over them and where a Smart Filter drop lands.

use craft_behavior::EditEvent;
use craft_layers::ui::filter::{FilterKind, LayerFilter};
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayerDrop, LayerPosition, LayersEvent, LayersPanel};
use craft_layers::ui::tree::{LayerInfo, LayerKind, LayerRowKey, LayerTree, SubRowGroup};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A size of `width` by `height` points.
const fn by(width: f32, height: f32) -> Vec2 {
    Vec2::new(width, height)
}

/// One panel's room: tall enough for the sample tree and the footer, and inside the 800 by 600 screen of a fixture.
const ONE_PANEL: Vec2 = by(400.0, 560.0);

type Seen = Rc<RefCell<Vec<LayersEvent<u32>>>>;
type Shared<T> = Rc<RefCell<T>>;

/// What a rig holds: the fixture, the events its panel reported, the tree and the filter it shows.
struct Rig<T> {
    fixture: Fixture,
    seen: Seen,
    tree: Shared<T>,
    filter: Shared<LayerFilter>,
}

fn rig_of<T: LayerTree<Id = u32> + 'static>(tree: T) -> Rig<T> {
    let seen: Seen = Rc::default();
    let tree = Rc::new(RefCell::new(tree));
    let filter: Shared<LayerFilter> = Rc::default();
    let (sink, shown, by) = (Rc::clone(&seen), Rc::clone(&tree), Rc::clone(&filter));
    let mut fixture = Fixture::new(move |ui| {
        let tree = shown.borrow();
        let mut filter = by.borrow_mut();
        let out = LayersPanel::new(&*tree).filter(Bind::bound(&mut *filter)).show(ui);
        sink.borrow_mut().extend(out.events);
    });
    fixture.room(ONE_PANEL);
    fixture.settle();
    Rig { fixture, seen, tree, filter }
}

fn rig() -> Rig<FakeLayers> {
    rig_of(FakeLayers::sample())
}

impl<T> Rig<T> {
    fn row(&self, label: &str) -> Target {
        self.fixture.by_label(label).unwrap_or_else(|| panic!("no row named {label}"))
    }

    /// The labels of `labels` that are on screen, top to bottom.
    fn order(&self, labels: &[&'static str]) -> Vec<&'static str> {
        let mut shown: Vec<(f32, &str)> = labels.iter().filter_map(|label| self.fixture.by_label(label).map(|t| (t.rect.top(), *label))).collect();
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        shown.into_iter().map(|(_, label)| label).collect()
    }

    /// Drags from `from` to `to` in small steps.
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.seen.borrow_mut().clear();
        let f = &mut self.fixture;
        f.frame(vec![pointer::moved(from)]);
        f.frame(vec![pointer::press(from)]);
        for step in 1..=6 {
            f.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 6.0))]);
        }
        f.frame(vec![pointer::moved(to)]);
        f.frame(vec![pointer::release(to)]);
        f.idle(2);
    }

    /// Drags the row `from` onto the upper or the lower quarter of the row `onto`.
    fn drop_on(&mut self, from: &str, onto: &str, at: Place) {
        let (source, target) = (self.row(from).center(), self.row(onto).rect);
        let y = match at {
            Place::Above => target.top() + target.height() * 0.2,
            Place::Below => target.top() + target.height() * 0.8,
        };
        self.drag(source, Pos2::new(target.center().x, y));
    }

    fn drops(&self) -> Vec<Landing> {
        self.seen
            .borrow()
            .iter()
            .filter_map(|event| match event {
                LayersEvent::Drop(EditEvent::Set { after, .. }) => Some(match after {
                    LayerDrop::Layers(m) => Landing::Layers(m.layers.clone(), m.target, m.position, m.copy),
                    LayerDrop::Transfer(t) => Landing::Transfer(t.rows.clone(), t.to, t.copy),
                    LayerDrop::SubRows(m) => Landing::SubRows(m.layer, m.group, m.from, m.to, m.copy),
                    _ => Landing::Other,
                }),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
enum Place {
    Above,
    Below,
}

/// What a drop event says, in plain values.
#[derive(Debug, PartialEq)]
enum Landing {
    Layers(Vec<u32>, u32, LayerPosition, bool),
    Transfer(Vec<LayerRowKey<u32>>, u32, bool),
    SubRows(u32, SubRowGroup, usize, usize, bool),
    Other,
}

const SAMPLE_ORDER: [&str; 9] = ["Group 1", "Clipped", "Masked", "Title", "Levels 1", "Smart Object", "Styled", "Hidden", "Background"];

/// The rows run top first, a closed group hides its layers, and open effects and Smart Filters list under their layer.
#[trial]
fn rows_are_listed_top_first_and_closed_groups_hide_their_layers() {
    let mut rig = rig();
    assert_eq!(rig.order(&SAMPLE_ORDER), SAMPLE_ORDER);

    rig.tree.borrow_mut().layers[0].open = false;
    rig.fixture.idle(3);
    assert_eq!(rig.order(&SAMPLE_ORDER), ["Group 1", "Title", "Levels 1", "Smart Object", "Styled", "Hidden", "Background"]);

    {
        let mut tree = rig.tree.borrow_mut();
        tree.layers[5].effects_open = true;
        tree.layers[6].effects_open = true;
    }
    rig.fixture.idle(3);
    let all = ["Smart Object", "Gaussian Blur", "Sharpen", "Styled", "Effects", "Drop Shadow", "Stroke", "Hidden"];
    assert_eq!(rig.order(&all), all);
}

/// A filter reads fields the tree revision does not cover, so it applies even while the revision stands.
#[trial]
fn a_filter_applies_even_while_the_tree_revision_stands() {
    let mut rig = rig();
    rig.tree.borrow_mut().revision = Some(1);
    rig.filter.borrow_mut().kinds = vec![FilterKind::Pixel];
    rig.fixture.idle(3);
    assert!(rig.fixture.by_label("Clipped").is_some());
    rig.tree.borrow_mut().layers[1].info = LayerInfo::new("Clipped", LayerKind::Text);
    rig.fixture.idle(3);
    assert!(rig.fixture.by_label("Clipped").is_none(), "a kind change shows at the same revision");
}

/// A Smart Filter moves within its own object, with the slot counted after taking it out; effects keep their order.
#[trial]
fn smart_filters_reorder_within_their_object_and_effects_keep_their_order() {
    let mut rig = rig();
    {
        let mut tree = rig.tree.borrow_mut();
        tree.layers[5].effects_open = true;
        tree.layers[6].effects_open = true;
    }
    rig.fixture.idle(3);
    let moved = |from, to| Landing::SubRows(6, SubRowGroup::SmartFilters, from, to, false);
    rig.drop_on("Gaussian Blur", "Sharpen", Place::Below);
    assert_eq!(rig.drops(), [moved(0, 1)]);
    rig.drop_on("Sharpen", "Gaussian Blur", Place::Above);
    assert_eq!(rig.drops(), [moved(1, 0)]);
    rig.drop_on("Gaussian Blur", "Gaussian Blur", Place::Above);
    assert_eq!(rig.drops(), [], "where it already is");
    rig.drop_on("Gaussian Blur", "Stroke", Place::Below);
    assert_eq!(rig.drops(), [], "another layer's filters");
    rig.drop_on("Drop Shadow", "Stroke", Place::Below);
    assert_eq!(rig.drops(), [], "effects keep their order");
}
