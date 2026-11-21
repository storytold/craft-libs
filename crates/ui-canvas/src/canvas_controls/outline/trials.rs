//! Trials of the mask tint: what the host's `tint_opacity` and `edge_width` do to the tint.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::{EDGE_ALPHA, FILL_ALPHA, HoverMask, MaskLook, tint};
use crate::primitives::Serials;
use crate::testing::trial;
use craft_behavior::kurbo::Rect;
use egui::{Color32, ColorImage};

/// The alpha a covered edge texel carries, as the image stores it.
const EDGE_A8: u8 = (EDGE_ALPHA * 255.0).round() as u8;
/// The alpha a fully covered texel carries at the framework's own fill alpha.
const FILL_A8: u8 = (FILL_ALPHA * 255.0).round() as u8;

/// A 6 x 6 mask with a 4 x 4 covered block at (1, 1), one texel per document unit: the block's
/// ring sits one texel from an uncovered texel, its middle does not.
fn block() -> HoverMask {
    HoverMask::sampled(&mut Serials::default(), Rect::new(0.0, 0.0, 6.0, 6.0), |x, y| {
        if (1.0..5.0).contains(&x) && (1.0..5.0).contains(&y) { 1.0 } else { 0.0 }
    })
}

fn alpha(image: &ColorImage, x: usize, y: usize) -> u8 {
    image.pixels[y * image.size[0] + x].a()
}

/// The look of a mask over the block: `opacity` and `edge` as a host would pass them.
fn look(opacity: Option<f32>, edge: Option<f32>) -> MaskLook {
    MaskLook::of(&block(), opacity, edge)
}

/// How many texels of the image carry the edge's alpha.
fn edged(image: &ColorImage) -> usize {
    (0..6).flat_map(|y| (0..6).map(move |x| (x, y))).filter(|(x, y)| alpha(image, *x, *y) == EDGE_A8).count()
}

/// The colour the trials tint with: only the alpha the tint writes is asserted, the hue is irrelevant.
fn ink() -> Color32 {
    Color32::from_rgb(0xB4, 0x1E, 0x1E)
}

#[trial]
fn a_host_that_gives_nothing_gets_the_frameworks_own_tint() {
    let image = tint(&block(), ink(), &look(None, None));
    assert!(alpha(&image, 1, 1) > alpha(&image, 2, 2), "the one-texel edge stands out over the fill");
    assert_eq!(alpha(&image, 1, 1), EDGE_A8, "the ring is the edge");
    assert_eq!(alpha(&image, 2, 2), FILL_A8, "the middle is the fill");
    assert_eq!(alpha(&image, 0, 0), 0, "an uncovered texel tints nothing");
}

#[trial]
fn the_host_opacity_moves_the_fill_and_leaves_the_edge_and_the_emptiness_alone() {
    let weak = tint(&block(), ink(), &look(Some(0.1), None));
    let full = tint(&block(), ink(), &look(Some(1.0), None));
    assert!(alpha(&weak, 2, 2) < alpha(&full, 2, 2), "more opacity, more fill");
    assert_eq!(alpha(&weak, 1, 1), alpha(&full, 1, 1), "the edge keeps its own alpha");
    for image in [&weak, &full] {
        assert_eq!(alpha(image, 0, 0), 0, "opacity still tints nothing where nothing is covered");
    }
    // Past the ends of its range the share is clamped; a value that is not a number keeps the
    // framework's own fill.
    assert_eq!(alpha(&tint(&block(), ink(), &look(Some(7.0), None)), 2, 2), 255, "above 1 reads as 1");
    assert_eq!(alpha(&tint(&block(), ink(), &look(Some(-3.0), None)), 2, 2), 0, "below 0 reads as 0");
    let nan = tint(&block(), ink(), &look(Some(f32::NAN), None));
    assert_eq!(alpha(&nan, 2, 2), alpha(&tint(&block(), ink(), &look(None, None)), 2, 2), "not a number tints as the framework does");
}

#[trial]
fn the_host_edge_replaces_the_one_texel_edge_and_a_deeper_outline_reaches_deeper() {
    let default = tint(&block(), ink(), &look(None, None));
    let none = tint(&block(), ink(), &look(None, Some(0.0)));
    let deep = tint(&block(), ink(), &look(None, Some(3.0)));
    assert_eq!(edged(&none), 0, "0 width asks for no band at all");
    assert!(edged(&deep) > edged(&default), "a wider outline reaches further in");
    assert_eq!(alpha(&none, 1, 1), FILL_A8, "with no band the ring is plain fill");
    assert_eq!(alpha(&deep, 2, 2), EDGE_A8, "the block's middle joins a 3-unit band");
    assert_eq!(alpha(&deep, 0, 0), 0, "the band stays inside the covered area");
    let nan = tint(&block(), ink(), &look(None, Some(f32::NAN)));
    assert_eq!(edged(&nan), edged(&default), "a width that is not a number keeps the framework's edge");
}

#[trial]
fn a_width_under_one_texel_still_gets_that_texel_and_an_empty_mask_nothing() {
    let thin = tint(&block(), ink(), &look(None, Some(0.4)));
    let one = tint(&block(), ink(), &look(None, Some(1.0)));
    assert_eq!(edged(&thin), edged(&one), "the grid's own texel is the narrowest band");
    let empty = HoverMask::sampled(&mut Serials::default(), Rect::new(0.0, 0.0, 6.0, 6.0), |_, _| 0.0);
    let image = tint(&empty, ink(), &MaskLook::of(&empty, Some(1.0), Some(30.0)));
    assert!(image.pixels.iter().all(|p| p.a() == 0), "an object with no coverage draws nothing");
}
