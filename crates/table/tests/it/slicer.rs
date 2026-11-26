//! `Slicer`: its own height holds every tile.

use std::cell::Cell;
use std::rc::Rc;

use craft_table::ui::Slicer;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, vec2};

const VALUES: [&str; 7] = ["North", "South", "East", "West", "Central", "Export", "Import"];

/// Shown at the height it measures at a width, a slicer shows every tile inside its card, none scrolled out.
#[trial]
fn a_slicer_at_its_own_height_shows_every_tile() {
    let items: Vec<(String, bool)> = VALUES.iter().enumerate().map(|(at, name)| ((*name).to_owned(), at % 2 == 0)).collect();
    let card = Rc::new(Cell::new(Rect::NOTHING));
    let seen = card.clone();
    let mut fixture = Fixture::new(move |ui| {
        let slicer = Slicer::new("Region", &items).columns(2);
        let width = ui.available_width() / 2.0;
        let rect = Rect::from_min_size(ui.max_rect().min, vec2(width, slicer.height_for(ui, width)));
        seen.set(slicer.show(ui, rect).response.rect);
    });
    fixture.settle();
    let card = card.get();
    for value in VALUES {
        let tile = fixture.by_label(value).unwrap_or_else(|| panic!("the tile {value} has a node"));
        assert!(card.contains_rect(tile.rect), "{value} at {:?} lies inside the card {card:?}", tile.rect);
    }
}
