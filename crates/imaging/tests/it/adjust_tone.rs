//! The Levels editor and panel, the Curves panel's point helper and the tone pickers, driven through the public
//! widgets as a host would show them.

use craft_imaging::ui::adjust::{CurvesPanel, Levels, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::Curve;
use craft_theme::Srgba;
use craft_ui::testing::trial;

fn grey(v: f32) -> Srgba {
    Srgba { r: v, g: v, b: v, a: 1.0 }
}

fn rgb() -> ToneChannels {
    ToneChannels::of(ToneSpace::Rgb, false)
}

/// Auto clips a tenth of a percent at both ends, back to gamma 1, outputs kept.
#[trial]
fn auto_follows_the_clip_rule_at_both_ends() {
    let start = Levels::new(3.0, 0.5, 100.0, 7.0, 9.0);
    let empty = start.auto(&[0; 256]);
    assert_eq!((empty.in_black, empty.in_white, empty.gamma), (253.0, 255.0, 1.0));
    assert_eq!((empty.out_black, empty.out_white), (7.0, 9.0), "the outputs stay");

    let mut counts = [0u32; 256];
    counts[0] = 20;
    counts[1] = 1;
    counts[255] = 12;
    counts[254] = 8;
    counts[253] = 1;
    counts[128] = 20000 - 42;
    let auto = start.auto(&counts);
    assert_eq!((auto.in_black, auto.in_white), (1.0, 253.0));
}

/// The output at the sampled input is the curve's as drawn: each run between corner points is sampled on its own.
#[trial]
fn a_sample_reads_each_corner_run_on_its_own() {
    // A sampler that draws the chord from a run's first to its last point.
    let chord = |points: &[[f32; 2]]| {
        let (a, b) = (points[0], points[points.len() - 1]);
        (0..=255).map(|x| if x as f32 <= a[0] { a[1] } else { a[1] + (b[1] - a[1]) * ((x as f32 - a[0]) / (b[0] - a[0])).min(1.0) }).collect()
    };
    let mut curves = ToneCurves::identity(&rgb());
    curves.curves[0] = Curve::from_parts(vec![[0.0, 0.0], [100.0, 200.0], [255.0, 255.0]], vec![false, true, false]);
    let at = CurvesPanel::add_point_from_sample(&mut curves, &rgb(), 0, grey(0.5), false, &chord);
    assert_eq!(at, Some(2));
    let Some(points) = curves.curves[0].points() else { panic!("still a point curve") };
    assert_eq!(points[2], [128.0, 210.0], "the chord of the run from the corner at 100, not of the whole curve");
}
