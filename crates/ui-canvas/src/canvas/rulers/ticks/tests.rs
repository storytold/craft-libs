use super::{INCH_EIGHTHS, INCH_SIXTEENTHS, LABEL_STEP_MIN, StepLadder, TickRule, ticks};
use crate::primitives::TickRank;
use craft_behavior::{RulerUnits, Ticks};

/// Screen points per document unit, from far out to deep in.
const ZOOMS: [f64; 10] = [0.01, 0.0625, 0.25, 1.0 / 3.0, 1.0, 3.7, 16.0, 64.0, 320.0, 3200.0];

/// The least distance in points between two consecutive labelled ticks of `rule` at `zoom`, over a ruler 2000 points
/// long.
fn least_label_gap(units: &RulerUnits, rule: TickRule<'_>, zoom: f64) -> f64 {
    let labelled: Vec<f64> = ticks(units, rule, zoom, 0.0..=2000.0 / zoom * units.per_doc)
        .into_iter()
        .filter(|tick| tick.rank == TickRank::Major)
        .map(|tick| tick.value)
        .collect();
    assert!(labelled.len() >= 2, "{rule:?} at zoom {zoom} labels at least two ticks");
    labelled.windows(2).map(|pair| (pair[1] - pair[0]) / units.per_doc * zoom).fold(f64::INFINITY, f64::min)
}

#[test]
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
