//! The Levels editor and panel, the Curves panel's point helper and the tone pickers, driven through the public
//! widgets as a host would show them.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::ui::adjust::{CurvesPanel, Levels, LevelsEditor, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::Curve;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

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

/// A curves point from a sample goes on the shown channel, or with Shift on each of Red, Green and Blue at that
/// channel's own level, leaving the composite alone.
#[trial]
fn a_curves_point_from_a_sample_goes_on_the_shown_channel_or_on_each_colour() {
    let identity = |_: &[[f32; 2]]| (0..=255).map(|x| x as f32).collect::<Vec<f32>>();
    let sample = Srgba { r: 0.2, g: 0.6, b: 0.4, a: 1.0 };
    let xs = |curves: &ToneCurves, channel: usize| -> Vec<f32> {
        curves.curves[channel].points().map(|points| points.iter().map(|point| point[0]).collect()).unwrap_or_default()
    };

    let mut curves = ToneCurves::identity(&rgb());
    let shown = CurvesPanel::add_point_from_sample(&mut curves, &rgb(), 0, sample, true, &identity);
    assert_eq!(shown, None, "the composite is not one of the three, so no point of it is selected");
    assert_eq!(xs(&curves, 0), [0.0, 255.0], "the composite keeps its two points");
    assert_eq!(xs(&curves, 1), [0.0, 51.0, 255.0], "Red takes its own level");
    assert_eq!(xs(&curves, 2), [0.0, 153.0, 255.0], "Green takes its own level");
    assert_eq!(xs(&curves, 3), [0.0, 102.0, 255.0], "Blue takes its own level");

    let mut curves = ToneCurves::identity(&rgb());
    let shown = CurvesPanel::add_point_from_sample(&mut curves, &rgb(), 2, sample, false, &identity);
    assert_eq!(shown, Some(1), "the point is selected by its index on the curve, not by the channel number");
    assert_eq!(xs(&curves, 2), [0.0, 153.0, 255.0]);
    assert_eq!((xs(&curves, 1).len(), xs(&curves, 3).len()), (2, 2), "the other colours stay as they were");
}

/// Every value field of the editor holds its value whole: the gamma at both ends of its range and the levels at both
/// ends of theirs lie inside the field, not cut by its edge.
#[trial]
fn the_levels_fields_hold_their_values_whole() {
    for (levels, shown) in
        [(Levels::new(0.0, 1.0, 255.0, 0.0, 255.0), ["0", "1.00", "255"]), (Levels::new(255.0, 9.99, 255.0, 255.0, 255.0), ["255", "9.99", "255"])]
    {
        let state = Rc::new(RefCell::new(levels));
        let held = Rc::clone(&state);
        let mut fixture = Fixture::new(move |ui| {
            let _ = LevelsEditor::new("Levels", Bind::bound(&mut *held.borrow_mut())).show(ui);
        });
        fixture.room(Vec2::new(800.0, 600.0));
        fixture.settle();
        for value in shown {
            let painted: Vec<_> = fixture.texts().iter().filter(|text| text.text == value).collect();
            assert!(!painted.is_empty(), "{value} is painted");
            for text in painted {
                assert!(!text.elided, "{value} is not elided");
                assert!(text.clip.contains_rect(text.rect), "{value} {:?} lies inside its field's clip {:?}", text.rect, text.clip);
            }
        }
    }
}
