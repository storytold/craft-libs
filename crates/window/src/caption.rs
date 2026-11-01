//! Caption-button placement on an app-drawn title bar, as pure `f32` math: each button's hit
//! extent and face centre, and the width the groups reserve at the bar's ends.
//!
//! [`Layout`] names the buttons on the bar's logical ends (reading start / reading end — GTK and
//! KWin decoration strings are logical too). Left to right the reading start is the physical
//! left; in a right-to-left interface the whole arrangement reflects about the bar's centre
//! line: each group takes the opposite physical end and its internal order reverses, so Windows'
//! `[Minimize, Maximize, Close]` renders Close-in-the-corner at the physical left, as in Arabic
//! Windows and right-to-left GTK/KDE. Glyphs never flip — only positions move.

use crate::{Button, Layout};

/// A platform's caption-button metrics: button width, the gap between buttons, and the gap at
/// the bar's outer edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub size: f32,
    pub gap: f32,
    pub end: f32,
}

impl Metrics {
    /// Windows 10/11: full-height buttons 46 pt wide, no gaps.
    pub const WINDOWS: Self = Self { size: 46.0, gap: 0.0, end: 0.0 };
    /// GNOME (libadwaita): 24 pt buttons, 6.5 pt apart, 10 pt from the edge.
    pub const ADWAITA: Self = Self { size: 24.0, gap: 6.5, end: 10.0 };
}

/// Each button's horizontal extent `(left, right)` and face centre x on one side, in `side`'s
/// order, laid out from `from`. `outer_left` puts the edge gap on the first button (a bar's left
/// end) instead of the last (its right end). Direction-agnostic: the mirror lives only in
/// [`reserved`] and [`layout_slots`].
pub fn slots(side: &[Button], m: Metrics, from: f32, outer_left: bool) -> Vec<(Button, (f32, f32), f32)> {
    let mut x = from;
    let n = side.len();
    side.iter()
        .enumerate()
        .map(|(i, &b)| {
            let before = if outer_left && i == 0 { m.end } else { m.gap };
            let after = if !outer_left && i + 1 == n { m.end } else { m.gap };
            let slot = (x, x + before + m.size + after);
            let centre = x + before + m.size / 2.0;
            x = slot.1;
            (b, slot, centre)
        })
        .collect()
}

/// The width the buttons take at the bar's physical `(left, right)` ends, already mirrored for
/// `rtl`: right to left each logical group takes the opposite end, so the tuple swaps.
pub fn reserved(layout: &Layout, m: Metrics, rtl: bool) -> (f32, f32) {
    let width = |side: &[Button]| slots(side, m, 0.0, true).last().map_or(0.0, |(_, r, _)| r.1);
    let (left, right) = (width(&layout.left), width(&layout.right));
    if rtl { (right, left) } else { (left, right) }
}

/// Every caption button's extent `(left, right)` and face centre x for a bar spanning
/// `bar_left..bar_right`. Right to left, every slot and centre reflects about the bar's centre
/// line (`span = bar_left + bar_right`): one reflection relocates each group to the opposite end
/// AND reverses its internal order.
pub fn layout_slots(layout: &Layout, m: Metrics, bar_left: f32, bar_right: f32, rtl: bool) -> Vec<(Button, (f32, f32), f32)> {
    let lefts = slots(&layout.left, m, bar_left, true);
    let width = slots(&layout.right, m, 0.0, false).last().map_or(0.0, |(_, r, _)| r.1);
    let rights = slots(&layout.right, m, bar_right - width, false);
    let mut out: Vec<(Button, (f32, f32), f32)> = lefts.into_iter().chain(rights).collect();
    if rtl {
        let span = bar_left + bar_right;
        for (_, slot, centre) in &mut out {
            *slot = (span - slot.1, span - slot.0);
            *centre = span - *centre;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_layout() -> Layout {
        Layout { left: Vec::new(), right: vec![Button::Minimize, Button::Maximize, Button::Close] }
    }

    #[test]
    fn reserved_reports_physical_widths_and_swaps_them_under_rtl() {
        let l = windows_layout();
        assert_eq!(reserved(&l, Metrics::WINDOWS, false), (0.0, 3.0 * 46.0));
        assert_eq!(reserved(&l, Metrics::WINDOWS, true), (3.0 * 46.0, 0.0));
        let both = Layout { left: vec![Button::Close], right: vec![Button::Minimize, Button::Maximize] };
        assert_eq!(reserved(&both, Metrics::WINDOWS, false), (46.0, 2.0 * 46.0));
        assert_eq!(reserved(&both, Metrics::WINDOWS, true), (2.0 * 46.0, 46.0));
    }

    #[test]
    fn slots_stay_direction_agnostic() {
        let s = slots(&[Button::Minimize, Button::Close], Metrics::ADWAITA, 10.0, true);
        assert_eq!(s.len(), 2);
        assert!(
            (s[0].1.0 - 10.0).abs() < 1e-6 && (s[0].1.1 - (10.0 + 10.0 + 24.0 + 6.5)).abs() < 1e-6,
            "outer edge gap on the first button: {:?}",
            s[0].1
        );
        assert!((s[1].1.1 - (10.0 + 10.0 + 24.0 + 6.5 + 6.5 + 24.0 + 6.5)).abs() < 1e-6, "{:?}", s[1].1);
        assert!((s[0].2 - (10.0 + 10.0 + 12.0)).abs() < 1e-6, "face centre: {}", s[0].2);
    }

    #[test]
    fn layout_reflects_every_slot_about_the_bar_centre_under_rtl() {
        let l = windows_layout();
        let (bar_left, bar_right) = (0.0, 800.0);
        let ltr = layout_slots(&l, Metrics::WINDOWS, bar_left, bar_right, false);
        assert_eq!(ltr.last().map(|(b, _, _)| *b), Some(Button::Close));
        assert!((ltr.last().unwrap().1.1 - bar_right).abs() < 1e-6, "Close flush right in LTR");

        let rtl = layout_slots(&l, Metrics::WINDOWS, bar_left, bar_right, true);
        // One reflection: Close in the physical left corner, internal order reversed.
        assert_eq!(rtl.last().map(|(b, _, _)| *b), Some(Button::Close));
        assert!((rtl.last().unwrap().1.0 - bar_left).abs() < 1e-6, "Close flush left in RTL: {:?}", rtl.last().unwrap().1);
        assert_eq!(rtl.first().map(|(b, _, _)| *b), Some(Button::Minimize));

        let span = bar_left + bar_right;
        // The reflection maps each slot in place (the vec order is unchanged; the visual order
        // reverses because the slots themselves moved).
        for (a, b) in ltr.iter().zip(rtl.iter()) {
            assert_eq!(a.0, b.0, "glyphs never flip, only positions move");
            assert!((span - a.1.0 - b.1.1).abs() < 1e-4 && (span - a.1.1 - b.1.0).abs() < 1e-4, "slots reflect");
            assert!((span - a.2 - b.2).abs() < 1e-4, "centres reflect too");
        }
        // Glyphs never flip: the same multiset of buttons, each once.
        let names = |v: &[(Button, (f32, f32), f32)]| {
            let mut n: Vec<Button> = v.iter().map(|(b, _, _)| *b).collect();
            n.sort_by_key(|b| *b as u8);
            n
        };
        assert_eq!(names(&ltr), names(&rtl));
    }

    #[test]
    fn a_logical_start_group_lands_on_the_physical_right_under_rtl() {
        let l = Layout { left: vec![Button::Close], right: Vec::new() };
        let ltr = layout_slots(&l, Metrics::WINDOWS, 0.0, 800.0, false);
        assert!((ltr[0].1.0 - 0.0).abs() < 1e-6);
        let rtl = layout_slots(&l, Metrics::WINDOWS, 0.0, 800.0, true);
        assert_eq!(rtl[0].0, Button::Close);
        assert!((rtl[0].1.1 - 800.0).abs() < 1e-6);
    }
}
