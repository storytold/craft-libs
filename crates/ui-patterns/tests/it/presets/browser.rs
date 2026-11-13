//! `PresetBrowser::drop_zone`: a preset dragged from the browser and released over the zone reports `DropAt`.

use craft_ui::contract::{Bind, DropZone};
use craft_ui::patterns::presets::{PresetBrowser, PresetEvent, PresetRow, PresetState, PresetView};
use craft_ui::runtime::drag;
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Pos2, Rect, Sense, UiBuilder, pos2, vec2};
use std::cell::RefCell;
use std::rc::Rc;

const GROUP: u32 = 100;
const ALPHA: u32 = 1;
const BROWSER: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(200.0, 300.0) };
const ZONE: Rect = Rect { min: Pos2::new(260.0, 20.0), max: Pos2::new(460.0, 280.0) };
/// An area over the zone's lower part: a release there is not over the zone.
const COVER: Rect = Rect { min: Pos2::new(260.0, 200.0), max: Pos2::new(460.0, 280.0) };

/// The browser in the left column, a list of one group holding Alpha and Beta, with the zone to its right.
fn rig() -> (Fixture, Rc<RefCell<Vec<PresetEvent<u32>>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let rows = vec![PresetRow::group(GROUP, "Group", 0), PresetRow::item(ALPHA, "Alpha", 1), PresetRow::item(2, "Beta", 1)];
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        let zone = DropZone::new(ui.layer_id(), ZONE);
        ui.scope_builder(UiBuilder::new().max_rect(BROWSER), |ui| {
            let out = PresetBrowser::new("Patterns", Bind::bound(&mut state), &rows).view_lock(PresetView::List).drop_zone(zone).show(ui);
            seen.borrow_mut().extend(out.events);
        });
        egui::Area::new(Id::unique("cover")).fixed_pos(COVER.min).show(ui.ctx(), |ui| ui.allocate_exact_size(COVER.size(), Sense::click()));
    });
    fixture.room(vec2(480.0, 320.0)).settle();
    (fixture, events)
}

#[trial]
fn a_preset_released_over_the_zone_reports_drop_at_and_nothing_else_does() {
    // (row grabbed, release point, the drop reported)
    let cases: [(&str, Pos2, Option<(u32, Pos2)>); 4] = [
        ("Alpha", pos2(360.0, 100.0), Some((ALPHA, pos2(360.0, 100.0)))),
        ("Alpha", pos2(360.0, 240.0), None),
        ("Alpha", pos2(230.0, 100.0), None),
        ("Group", pos2(360.0, 100.0), None),
    ];
    for (row, at, expected) in cases {
        let (mut fixture, events) = rig();
        let from = fixture.by_label(row).expect("the row is shown").center();
        let mut marked = false;
        fixture.drag_at_pausing(from, at - from, |fixture| marked = drag::mark(fixture.ctx()).is_some());
        let drops: Vec<(u32, Pos2)> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                PresetEvent::DropAt { key, at, .. } => Some((*key, *at)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, expected.into_iter().collect::<Vec<_>>(), "{row} released at {at:?}");
        assert_eq!(marked, expected.is_some(), "{row} over {at:?} shows the zone's mark only where it drops");
    }
}
