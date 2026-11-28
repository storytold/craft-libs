//! `.3dl` save and reopen through the public API, compared on the rendered output
//! (photocraft#11581 review, #11452): a lookup written with `write_3dl` and read back with
//! `parse_3dl` renders the same colours, shaper included.

use craft_cms::lutfile::{LutFile, parse_3dl, sample, write_3dl};
use std::sync::Arc;

/// Colours every check renders: black, mid grey, white, and a few off-axis ones.
const SAMPLES: [[f32; 3]; 8] =
    [[0.0, 0.0, 0.0], [0.5, 0.5, 0.5], [1.0, 1.0, 1.0], [0.25, 0.25, 0.25], [0.75, 0.75, 0.75], [1.0, 0.0, 0.0], [0.2, 0.6, 0.9], [0.9, 0.35, 0.05]];

/// A table that is not the identity, so a shaper that moved shows in the render.
fn warped(size: usize) -> LutFile {
    LutFile::from_fn("warped", size, |c| [c[0] * c[0], 0.5 * c[1] + 0.25 * c[0], 1.0 - c[2]]).unwrap()
}

fn with_shaper(mut l: LutFile, knots: &[f32]) -> LutFile {
    l.shaper = Some(Arc::new(knots.to_vec()));
    l
}

/// Saves `l` as `.3dl`, reopens it and asserts every sample renders within `tol`.
fn assert_reopens_the_same(l: &LutFile, tol: f32) -> LutFile {
    let text = write_3dl(l).unwrap_or_else(|e| panic!("write_3dl refused {:?} on {}: {e}", l.shaper, l.size));
    let back = parse_3dl(&text).unwrap_or_else(|e| panic!("reopen failed: {e}\n{text}"));
    for c in SAMPLES {
        let (a, b) = (sample(l, c), sample(&back, c));
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() <= tol, "shaper {:?} on {}³ at {c:?}: {a:?} saved, {b:?} reopened", l.shaper, l.size);
        }
    }
    back
}

/// Finding 1: a low-range shaper must come back at the scale it was written at. Writing every
/// knot at 65535 made `[0, 1/1023, 2/1023]` reopen as `[0, 64/1023, 128/1023]` (white 0.001955
/// became 0.125122).
#[test]
fn three_dl_low_and_full_range_shapers_render_the_same_after_save_and_reopen() {
    let shapers: [&[f32]; 3] = [&[0.0, 1.0 / 1023.0, 2.0 / 1023.0], &[0.0, 0.02, 0.05, 0.1, 0.2], &[0.0, 0.3, 0.7, 1.0]];
    for knots in shapers {
        for size in [3, 17] {
            assert_reopens_the_same(&with_shaper(warped(size), knots), 3e-5);
        }
    }
    // The reviewer's example on an identity table: white is the shaper's top knot.
    let l = with_shaper(LutFile::identity(3).unwrap(), &[0.0, 1.0 / 1023.0, 2.0 / 1023.0]);
    let back = assert_reopens_the_same(&l, 2e-5);
    let white = sample(&back, [1.0; 3]);
    assert!(white.iter().all(|v| (v - 2.0 / 1023.0).abs() < 2e-5), "white reopened as {white:?}, was 0.001955");
}

/// Finding 2: a shaped 2-node table is saved as `.3dl` with its look kept, not refused into a
/// `.cube` fallback that drops the curve.
#[test]
fn three_dl_shaped_two_node_table_renders_the_same_after_save_and_reopen() {
    let shapers: [&[f32]; 4] = [&[0.0, 0.25, 1.0], &[0.0, 0.5], &[0.0, 1.0 / 1023.0, 2.0 / 1023.0], &[0.0, 0.1, 0.9, 1.0]];
    for knots in shapers {
        let l = with_shaper(warped(2), knots);
        let back = assert_reopens_the_same(&l, 3e-5);
        assert!(back.shaper.is_some(), "{knots:?}: the curve survives");
    }
    // Without a shaper, too.
    assert_reopens_the_same(&warped(2), 3e-5);
}

/// What `.3dl` really cannot hold is still an error the caller can surface, not a silent fallback.
#[test]
fn three_dl_write_still_refuses_what_it_cannot_declare() {
    let bent = [0.0, 0.25, 0.5, 1.0];
    assert!(write_3dl(&with_shaper(warped(4), &bent)).is_err(), "a 4-node edge is not 2^bits + 1");
    let mut d = with_shaper(warped(2), &bent);
    d.domain_min = [0.25; 3];
    assert!(write_3dl(&d).is_err(), "a non-default domain");
}
