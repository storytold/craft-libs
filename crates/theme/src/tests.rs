#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

/// A lightness search stops where the contrast test flips, which for a colour with any residual
/// tint lies between the points where its channels round to their next 8-bit value.
#[test]
fn contrast_moves_keep_greys_neutral() {
    let is_grey = |c: Rgba| c.0[0] == c.0[1] && c.0[1] == c.0[2];
    for fg in (0..=255u8).step_by(3) {
        for bg in [0x00, 0x28, 0x42, 0x53, 0x80, 0xd6, 0xe2, 0xf0, 0xff] {
            let (fg, bg) = (Rgba::rgb(fg, fg, fg), Rgba::rgb(bg, bg, bg));
            for min in [3.0, 4.5, 7.0] {
                let out = ensure_contrast(fg, bg, min);
                assert!(is_grey(out), "{fg:?} on {bg:?} at {min}: {out:?}");
            }
        }
    }
}
