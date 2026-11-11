//! WCAG 2.x contrast and lightness-based contrast repair.

use super::color::{Oklch, Rgba};
use super::theme::Contrast;
use super::values;

/// The least contrast ratio text needs against its background: WCAG AA, and the higher minimum under high contrast.
pub fn text_contrast_min(contrast: Contrast) -> f64 {
    match contrast {
        Contrast::Standard => values::TEXT_MIN.0,
        Contrast::High => values::TEXT_MIN.1,
    }
}

/// WCAG relative luminance, 0 (black) to 1 (white). Alpha is ignored.
pub fn relative_luminance(c: Rgba) -> f64 {
    let [r, g, b] = c.to_linear();
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio between two colours, 1 to 21, in either order. Alpha is ignored.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Returns `fg` unchanged if it already has at least `min` contrast on `bg`. Otherwise moves its
/// OKLCH lightness away from `bg`, keeping hue and chroma (re-gamut-mapped), and returns the
/// closest colour that passes. If neither direction can reach `min`, returns the extreme
/// (black or white) with the higher ratio. Alpha is kept.
pub fn ensure_contrast(fg: Rgba, bg: Rgba, min: f64) -> Rgba {
    if contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let start = fg.to_oklch();
    let alpha = fg.a();
    let black = Rgba::BLACK.with_alpha(alpha);
    let white = Rgba::WHITE.with_alpha(alpha);
    let (rb, rw) = (contrast_ratio(black, bg), contrast_ratio(white, bg));
    let goes_lighter = match (rb >= min, rw >= min) {
        (true, true) => start.l >= bg.to_oklch().l,
        (false, true) => true,
        (true, false) => false,
        (false, false) => return if rw >= rb { white } else { black },
    };
    let target = if goes_lighter { 1.0 } else { 0.0 };
    let at = |l: f64| Oklch { l, ..start }.to_rgba().with_alpha(alpha);
    // Invariant: `lo` fails, `hi` passes (the extreme passes by the check above).
    let (mut lo, mut hi) = (start.l, target);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if contrast_ratio(at(mid), bg) >= min {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    at(hi)
}
