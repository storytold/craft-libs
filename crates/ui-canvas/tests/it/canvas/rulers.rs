//! The ruler tick rule (`Rulers::ticks`): labelled ticks stay at least `LABEL_STEP_MIN` apart at every zoom.

use craft_behavior::{RulerUnits, Ticks};
use craft_ui::canvas::{INCH_EIGHTHS, INCH_SIXTEENTHS, LABEL_STEP_MIN, Rulers, StepLadder, TickRule};
use craft_ui::primitives::TickRank;
use craft_ui::testing::trial;

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
