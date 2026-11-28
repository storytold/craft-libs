//! The free bin: the flow of unplaced cards.

use craft_behavior::{DisplayOrder, ItemSource};
use craft_browse::ui::freebin::{FreeBin, FreeBinMove, FreeformCard};
use craft_ui::contract::{Props as _, ShowCollection as _};
use craft_ui::list::ThumbAspect;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use craft_ui::tile_grid::Tile;
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = offset(700.0, 400.0);

/// A position or displacement in the bin's coordinates, named so a trial's geometry reads as inputs.
const fn offset(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}
/// A picture side that keeps cards small enough to leave room between them in `VIEW`.
const ZOOM: f32 = 0.5;

/// Cards keyed by position.
struct Count(usize);

impl DisplayOrder<usize> for Count {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Count {}

/// What the host keeps and where the bin showed each card.
#[derive(Default)]
struct Scene {
    cards: RefCell<Vec<FreeformCard<usize>>>,
    moving: RefCell<Option<FreeBinMove<usize>>>,
    rects: RefCell<Vec<(usize, Rect)>>,
}

impl Scene {
    fn with_cards(cards: Vec<FreeformCard<usize>>) -> Self {
        Scene { cards: RefCell::new(cards), ..Scene::default() }
    }

    fn rect(&self, key: usize) -> Rect {
        self.rects.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the card was shown")
    }
}

fn card(key: usize) -> Tile<'static> {
    Tile::new(format!("Clip {}", key + 1)).thumb(ThumbAspect::new(1, 1), |_, _| {})
}

fn show_bin(ui: &mut Ui, count: usize, scene: &Scene, region: Rect) {
    in_region(ui, region, |ui| {
        let cards = scene.cards.borrow().clone();
        let mut bin = FreeBin::new("Clips", &cards).id_salt("bin").zoom(ZOOM).drop_into(|key| *key == 1);
        if let Some(live) = scene.moving.borrow().clone() {
            bin = bin.moving(live);
        }
        let out = bin.show_items(ui, &Count(count), &mut |key, cx| {
            let mut rects = scene.rects.borrow_mut();
            rects.retain(|(held, _)| held != key);
            rects.push((*key, cx.rect));
            card(*key)
        });
        *scene.moving.borrow_mut() = out.moving;
        for (key, pos) in &out.moved {
            let mut cards = scene.cards.borrow_mut();
            match cards.iter_mut().find(|held| held.key == *key) {
                Some(held) => held.pos = *pos,
                None => cards.push(FreeformCard::new(*key, *pos)),
            }
        }
    });
}

fn bin_region() -> Rect {
    Rect::from_min_size(ORIGIN, VIEW)
}

/// A card without a position never lands on a placed card: it takes the next cell of the flow that is free.
#[trial]
fn an_unplaced_card_skips_the_cells_of_placed_cards() {
    let flowing = Scene::default();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_bin(ui, 3, &flowing, bin_region());
    frames.idle(4, &mut show);
    let first_cell = flowing.rect(0);
    let placed = Scene::with_cards(vec![FreeformCard::new(0, first_cell.min - ORIGIN)]);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_bin(ui, 3, &placed, bin_region());
    frames.idle(4, &mut show);
    assert_eq!(placed.rect(0), first_cell, "the placed card keeps its place");
    assert!(!placed.rect(1).intersects(placed.rect(0)), "the next card does not cover the placed one");
    assert_eq!(placed.rect(1), flowing.rect(1), "it takes the second cell of the flow");
    assert_eq!(placed.rect(2), flowing.rect(2), "the third card follows it");
}
