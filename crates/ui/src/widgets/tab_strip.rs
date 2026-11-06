//! Tab-strip geometry on top of [`crate::tabs`]: placing strip items from the reading-start edge,
//! the overflow chevron's slot at the reading end, and a tab's close button and title x. The fit
//! policy itself ([`fit`]/[`fit_whole`]/[`TabFit`]) is direction-agnostic — it works in widths —
//! and is re-exported here so a strip's caller needs only this module.

use egui::{Rect, pos2, vec2};

use crate::direction::{HasDirection, end_x, is_rtl, start_x, start_x_w};

pub use crate::tabs::{Tab, TabFit, fit, fit_whole};

/// Width of the » overflow button.
pub const CHEVRON_W: f32 = 18.0;

/// Place strip items `(index, width)` in `area` from the reading-start edge with `gap` between
/// them — the left edge left-to-right, the right edge right-to-left — and return the placed rects
/// (same order as `shown`) and the cursor x after the last item.
pub fn place_strip(d: &impl HasDirection, area: Rect, shown: &[(usize, f32)], gap: f32) -> (Vec<(usize, Rect)>, f32) {
    let rtl = is_rtl(d);
    let mut x = start_x(d, area);
    let mut placed = Vec::with_capacity(shown.len());
    for &(i, w) in shown {
        let r = if rtl {
            Rect::from_min_max(pos2(x - w, area.top()), pos2(x, area.bottom()))
        } else {
            Rect::from_min_size(pos2(x, area.top()), vec2(w, area.height()))
        };
        x = if rtl { r.left() - gap } else { r.right() + gap };
        placed.push((i, r));
    }
    (placed, x)
}

/// The overflow chevron's slot right after the last tab placed at cursor `x` (`cw` wide),
/// clamped to the strip's reading-start edge.
pub fn chevron_at(d: &impl HasDirection, area: Rect, x: f32, cw: f32) -> Rect {
    if is_rtl(d) {
        Rect::from_min_max(pos2(area.left(), area.top()), pos2((area.left() + cw).min(x), area.bottom()))
    } else {
        Rect::from_min_size(pos2(x, area.top()), vec2(cw, area.height()))
    }
}

/// A `cw`-wide slot pinned to the strip's reading end (the left edge right-to-left), full height.
pub fn reading_end_rect(d: &impl HasDirection, area: Rect, cw: f32) -> Rect {
    let left = if is_rtl(d) { area.left() } else { (area.right() - cw).max(area.left()) };
    Rect::from_min_size(pos2(left, area.top()), vec2(cw, area.height()))
}

/// A tab's close button: 14×14, centred vertically, 13 pt from the tab's reading-start edge on
/// macOS and from its reading-end edge elsewhere (the platform order, mirrored).
pub fn tab_close_button(d: &impl HasDirection, tab: Rect, mac: bool) -> Rect {
    let x = if mac { start_x(d, tab) + crate::direction::forward(d) * 13.0 } else { end_x(d, tab) - crate::direction::forward(d) * 13.0 };
    Rect::from_center_size(pos2(x, tab.center().y), vec2(14.0, 14.0))
}

/// The x a tab title `title_w` wide starts at: 26 pt from the reading-start edge on macOS (the ×
/// leads), 16 pt elsewhere.
pub fn tab_title_x(d: &impl HasDirection, tab: Rect, title_w: f32, mac: bool) -> f32 {
    start_x_w(d, tab, if mac { 26.0 } else { 16.0 }, title_w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    #[test]
    fn place_strip_places_from_the_reading_start() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(300.0, 24.0));
        let shown = [(0usize, 60.0f32), (1, 70.0), (2, 50.0)];
        let (placed, x) = place_strip(&Direction::Ltr, area, &shown, 4.0);
        assert_eq!(placed.len(), 3);
        assert!((placed[0].1.left() - 100.0).abs() < 1e-4 && (placed[0].1.width() - 60.0).abs() < 1e-4, "{:?}", placed[0].1);
        assert!((placed[1].1.left() - 164.0).abs() < 1e-4, "gap after the first tab: {:?}", placed[1].1);
        assert!((x - 292.0).abs() < 1e-4, "cursor after the last tab: {x}");
        let (placed, x) = place_strip(&Direction::Rtl, area, &shown, 4.0);
        assert!(
            (placed[0].1.right() - 400.0).abs() < 1e-4 && (placed[0].1.width() - 60.0).abs() < 1e-4,
            "first tab hugs the right edge: {:?}",
            placed[0].1
        );
        assert!((placed[1].1.right() - 336.0).abs() < 1e-4, "gap on the reading side: {:?}", placed[1].1);
        assert!((x - 208.0).abs() < 1e-4, "cursor past the last tab: {x}");
    }

    /// The chevron slot follows the last tab left-to-right and clamps to the strip's left edge
    /// right-to-left; `reading_end_rect` pins to the reading end in both directions.
    #[test]
    fn chevron_and_reading_end_slots() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(300.0, 24.0));
        let c = chevron_at(&Direction::Ltr, area, 380.0, CHEVRON_W);
        assert!((c.left() - 380.0).abs() < 1e-4 && (c.width() - CHEVRON_W).abs() < 1e-4, "{c:?}");
        let e = reading_end_rect(&Direction::Ltr, area, CHEVRON_W);
        assert!((e.right() - 400.0).abs() < 1e-4, "ltr end slot at the right: {e:?}");
        let c = chevron_at(&Direction::Rtl, area, 110.0, CHEVRON_W);
        assert!((c.left() - 100.0).abs() < 1e-4 && (c.right() - 110.0).abs() < 1e-4, "clamped to the cursor: {c:?}");
        let c = chevron_at(&Direction::Rtl, area, 150.0, CHEVRON_W);
        assert!((c.left() - 100.0).abs() < 1e-4 && (c.width() - CHEVRON_W).abs() < 1e-4, "full slot at the reading end: {c:?}");
        let e = reading_end_rect(&Direction::Rtl, area, CHEVRON_W);
        assert!((e.left() - 100.0).abs() < 1e-4, "rtl end slot at the left: {e:?}");
    }

    /// `tab_close_button`/`tab_title_x` reproduce the platform x table in both directions.
    #[test]
    fn tab_close_and_title_x() {
        let tab = Rect::from_min_size(pos2(100.0, 50.0), vec2(120.0, 26.0));
        assert!((tab_close_button(&Direction::Ltr, tab, true).center().x - 113.0).abs() < 1e-4, "mac ltr");
        assert!((tab_close_button(&Direction::Ltr, tab, false).center().x - 207.0).abs() < 1e-4, "win ltr");
        assert!((tab_title_x(&Direction::Ltr, tab, 40.0, true) - 126.0).abs() < 1e-4, "mac ltr");
        assert!((tab_title_x(&Direction::Ltr, tab, 40.0, false) - 116.0).abs() < 1e-4, "win ltr");
        assert!((tab_close_button(&Direction::Rtl, tab, true).center().x - 207.0).abs() < 1e-4, "mac rtl");
        assert!((tab_close_button(&Direction::Rtl, tab, false).center().x - 113.0).abs() < 1e-4, "win rtl");
        assert!((tab_title_x(&Direction::Rtl, tab, 40.0, true) - 154.0).abs() < 1e-4, "mac rtl");
        assert!((tab_title_x(&Direction::Rtl, tab, 40.0, false) - 164.0).abs() < 1e-4, "win rtl");
    }

    /// The fit policy itself is direction-agnostic and hostile-input safe.
    #[test]
    fn fit_is_bounded_and_keeps_the_selected_tab() {
        let f = fit(&[60.0, 70.0, 80.0], 0, 400.0, 40.0, 18.0);
        assert_eq!(f.shown, vec![(0, 60.0), (1, 70.0), (2, 80.0)]);
        assert!(f.overflow.is_empty());
        for avail in [0.0, -10.0, 5.0, f32::NAN, f32::INFINITY] {
            for natural in [&[][..], &[f32::NAN, 50.0][..], &[1e9, -5.0, 30.0][..]] {
                for sel in [0, 1, 99] {
                    let f = fit(natural, sel, avail, 40.0, 18.0);
                    assert!(f.shown.iter().all(|(_, w)| w.is_finite() && *w >= 0.0), "{avail} {natural:?} {f:?}");
                    assert_eq!(f.shown.len() + f.overflow.len(), natural.len());
                }
            }
        }
    }
}
