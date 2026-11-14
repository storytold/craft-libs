//! Progress geometry: where a bar's accent fill sits, and where the indeterminate segment
//! travels. Both hug the reading-start edge in a right-to-left interface, so a caller passes the
//! fill fraction only, never a pre-mirrored boolean.

use egui::{Rect, pos2, vec2};

use crate::direction::{HasDirection, is_rtl, start_x_w};

/// The accent fill of a progress bar `rect` at `frac` (clamped to 0..=1), at least `min_w` wide
/// (a bar's fill is never thinner than its height, a tab underline's can shrink to nothing): a
/// `w`-wide rect hugging the reading-start edge, i.e. the left edge left-to-right and the right
/// edge right-to-left.
pub fn progress_fill(d: &impl HasDirection, rect: Rect, frac: f32, min_w: f32) -> Rect {
    let w = (rect.width() * frac.clamp(0.0, 1.0)).max(min_w);
    let x = start_x_w(d, rect, 0.0, w);
    Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()))
}

/// The indeterminate segment of a progress bar `rect` at `time` (seconds): a 30%-wide segment
/// sliding across the bar, travelling from the reading start; mirrored in a right-to-left
/// interface and clamped inside the bar.
pub fn progress_travel(d: &impl HasDirection, rect: Rect, time: f32) -> Rect {
    let w = rect.width() * 0.3;
    let x = rect.left() + ((time * 0.8).fract() * (rect.width() + w)) - w;
    let x = if is_rtl(d) { rect.left() + rect.right() - x - w } else { x };
    Rect::from_min_max(pos2(x.max(rect.left()), rect.top()), pos2((x + w).min(rect.right()), rect.bottom()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use egui::vec2;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 8.0))
    }

    #[test]
    fn fill_hugs_the_reading_start_and_clamps_frac() {
        let r = rect();
        let f = progress_fill(&Direction::Ltr, r, 0.25, 0.0);
        assert!((f.left() - r.left()).abs() < 1e-4 && (f.width() - 25.0).abs() < 1e-4, "{f:?}");
        assert!((progress_fill(&Direction::Ltr, r, 2.0, 0.0).width() - r.width()).abs() < 1e-4, "frac clamps to 1");
        assert!((progress_fill(&Direction::Ltr, r, -1.0, 0.0).width()).abs() < 1e-4, "frac clamps to 0");
        let f = progress_fill(&Direction::Ltr, r, 0.05, r.height());
        assert!((f.width() - r.height()).abs() < 1e-4, "min_w wins over a tiny frac: {f:?}");
        let f = progress_fill(&Direction::Rtl, r, 0.25, 0.0);
        assert!((f.right() - r.right()).abs() < 1e-4 && (f.width() - 25.0).abs() < 1e-4, "rtl fill hugs the right edge: {f:?}");
    }

    #[test]
    fn travel_is_mirrored_and_stays_inside_the_bar() {
        let r = rect();
        for time in [0.0, 0.3, 1.7, 12.9] {
            let f = progress_travel(&Direction::Ltr, r, time);
            assert!(f.left() >= r.left() - 1e-4 && f.right() <= r.right() + 1e-4, "{time}: {f:?} in {r:?}");
        }
        for time in [0.3, 1.7, 12.9] {
            let f = progress_travel(&Direction::Ltr, r, time);
            assert!((f.width() - r.width() * 0.3).abs() < 1e-4, "{time}: {f:?}");
        }
        let ltr_mid = progress_travel(&Direction::Ltr, r, 0.9);
        let rtl_mid = progress_travel(&Direction::Rtl, r, 0.9);
        assert!((rtl_mid.left() + rtl_mid.width() - (r.left() + r.right() - ltr_mid.left())).abs() < 1e-4, "mirrored: {ltr_mid:?} vs {rtl_mid:?}");
        for time in [0.0, 0.3, 1.7, 12.9] {
            let f = progress_travel(&Direction::Rtl, r, time);
            assert!(f.left() >= r.left() - 1e-4 && f.right() <= r.right() + 1e-4, "rtl {time}: {f:?} in {r:?}");
        }
    }
}
