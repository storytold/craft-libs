//! The ruler tick rule (`Rulers::ticks`): labelled ticks stay at least `LABEL_STEP_MIN` apart at every zoom.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, RulerUnits, Ticks};
use craft_ui::canvas::{Canvas, INCH_EIGHTHS, INCH_SIXTEENTHS, LABEL_STEP_MIN, RulerText, Rulers, StepLadder, TickRule};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::primitives::TickRank;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

/// Screen points per document unit, from far out to deep in.
const ZOOMS: [f64; 10] = [0.01, 0.0625, 0.25, 1.0 / 3.0, 1.0, 3.7, 16.0, 64.0, 320.0, 3200.0];

/// The least distance in points between two consecutive labelled ticks of `rule` at `zoom`, over a ruler 2000 points
/// long.
fn least_label_gap(units: &RulerUnits, rule: TickRule<'_>, zoom: f64) -> f64 {
    let labelled: Vec<f64> = Rulers::ticks(units, rule, zoom, 0.0..=2000.0 / zoom * units.per_doc)
        .into_iter()
        .filter(|tick| tick.rank == TickRank::Major)
        .map(|tick| tick.value)
        .collect();
    assert!(labelled.len() >= 2, "{rule:?} at zoom {zoom} labels at least two ticks");
    labelled.windows(2).map(|pair| (pair[1] - pair[0]) / units.per_doc * zoom).fold(f64::INFINITY, f64::min)
}

#[trial]
fn labelled_ticks_never_touch_at_any_zoom() {
    let pixels = RulerUnits::pixels();
    let centimetres = RulerUnits::new(2.54 / 72.0, Ticks::decimal(false));
    let inches = RulerUnits::new(1.0 / 72.0, Ticks::decimal(false));
    // Frames at 30 per second, one frame per document unit.
    let frames = RulerUnits::new(1.0, Ticks::decimal(true));
    let ladder = StepLadder::new(30, 48.0, 6.0);
    let least = f64::from(LABEL_STEP_MIN) - 1e-9;
    for zoom in ZOOMS {
        for (units, rule) in [
            (&pixels, TickRule::Metric),
            (&centimetres, TickRule::Metric),
            (&inches, TickRule::Inches(&INCH_SIXTEENTHS)),
            (&inches, TickRule::Inches(&INCH_EIGHTHS)),
        ] {
            let gap = least_label_gap(units, rule, zoom);
            assert!(gap >= least, "{rule:?} at zoom {zoom}: labels {gap} points apart");
        }
        // The ladder's coarsest step is an hour of frames; below that the labels may touch by design.
        if 3600.0 * 30.0 * zoom >= f64::from(ladder.label_gap) {
            let gap = least_label_gap(&frames, TickRule::Steps(ladder), zoom);
            assert!(gap >= f64::from(ladder.label_gap) - 1e-9, "the step ladder at zoom {zoom}: labels {gap} points apart");
        }
    }
}

/// A vertical label stacks its characters one label-role size apart; the longest label the metric rule gives at the
/// least step (five characters) and its gap to the tick stay inside that step, so neighbouring labels never touch.
#[trial]
fn a_five_character_vertical_label_fits_the_least_label_step() {
    for mode in [craft_theme::Mode::Light, craft_theme::Mode::Dark] {
        let tokens = craft_ui::testing::style::tokens(mode, craft_theme::Contrast::Standard);
        let pitch = tokens.typography.get(craft_theme::TypeRole::Caption).size;
        assert!(5.0 * pitch + tokens.space.gap.tight < LABEL_STEP_MIN, "{mode:?}: five characters at pitch {pitch}");
    }
}

/// The vertical extents of the tick marks the top strip paints over a canvas whose view is turned by `degrees`.
fn top_tick_extents(degrees: f64) -> BTreeSet<(i32, i32)> {
    let room = Vec2::new(800.0, 600.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, room), Point::new(400.0, 300.0), 1.0).rotation(degrees);
    let text = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
    let content = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&content);
    let mut zero = Point::ZERO;
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).show_rulers(true).rulers(text, text, Point::ZERO).show(ui, |ui, body| {
            seen.set(Rulers::new("Rulers", Bind::bound(&mut zero)).show_at(ui, body.rect).content_rect());
        });
    });
    fixture.room(room);
    fixture.settle();
    let strip_bottom = content.get().min.y;
    fixture
        .frame_shapes()
        .iter()
        .map(|shape| shape.visual_bounding_rect())
        .filter(|bounds| bounds.width() < 3.0 && bounds.height() > 1.0 && bounds.max.y <= strip_bottom + 0.5)
        .map(|bounds| ((bounds.min.y * 10.0).round() as i32, (bounds.max.y * 10.0).round() as i32))
        .collect()
}

#[trial]
fn ticks_stay_level_and_the_zero_label_sits_at_the_zero_point_in_a_turned_view() {
    let level = top_tick_extents(0.0);
    let turned = top_tick_extents(30.0);
    assert!(!turned.is_empty(), "the turned view's top strip has tick marks");
    assert!(turned.is_subset(&level), "tick marks keep the lengths and cross coordinates of the level view: {turned:?} against {level:?}");
}
