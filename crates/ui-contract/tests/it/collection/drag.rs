//! Drags that start on empty space: the marquee.

use super::kit::{Count, Dismissed, Row};
use super::table::Table;
use craft_behavior::{ItemSource, Selection, SelectionModel};
use craft_ui::contract::{CollectionEvent, CollectionProps, Props as _, SelectMode, ShowCollection as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(400.0, 240.0);
const COL: f32 = 100.0;

/// What a collection reported and where its rows were.
#[derive(Default)]
struct Scene {
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
}

impl Scene {
    fn row(&self, key: usize) -> Rect {
        self.rows.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    /// The selection the reported changes make, applied in order.
    fn selected(&self) -> Vec<usize> {
        let mut selection = Selection::<usize>::default();
        for event in self.events.borrow().iter() {
            if let CollectionEvent::Selection(change) = event {
                selection.apply(change.clone());
            }
        }
        selection.items
    }
}

fn show_table(ui: &mut Ui, props: CollectionProps, source: &dyn ItemSource<usize>, scene: &Scene) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
        let out = Table::new(props).id_salt("table").show_items(ui, source, &mut |key, cx| {
            let mut rows = scene.rows.borrow_mut();
            rows.retain(|(held, _)| held != key);
            rows.push((*key, cx.rect));
            Row::new()
        });
        scene.events.borrow_mut().extend(out.events);
    });
}

/// A drag from the empty space under the rows up to the middle of row 2 of five.
fn sweep_up(frames: &mut Frames, scene: &Scene, show: &mut impl FnMut(&mut Ui)) {
    let x = ORIGIN.x + COL / 2.0;
    let from = Pos2::new(x, scene.row(4).max.y + 40.0);
    let to = Pos2::new(x, scene.row(2).center().y);
    pointer::drag(frames, from, to - from, show);
}

/// A drag from empty space selects the rows its rectangle touches; with Shift they join the selection the drag found.
#[trial]
fn a_marquee_from_empty_space_selects_the_rows_it_touches() {
    let (scene, source) = (Scene::default(), Count(5));
    let props = || CollectionProps::new().select(SelectMode::Multi).marquee(true);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_table(ui, props(), &source, &scene);
    frames.idle(3, &mut show);
    sweep_up(&mut frames, &scene, &mut show);
    assert_eq!(scene.selected(), vec![2, 3, 4]);

    pointer::click(&mut frames, scene.row(0).center(), &mut show);
    assert_eq!(scene.selected(), vec![0]);
    frames.hold(Modifiers::SHIFT);
    sweep_up(&mut frames, &scene, &mut show);
    assert_eq!(scene.selected(), vec![0, 2, 3, 4]);
}
