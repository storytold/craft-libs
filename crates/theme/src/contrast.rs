//! WCAG 2 contrast on the colour pairs apps actually paint.
//!
//! Diagnostics are advisory: nothing here changes a colour or refuses a theme. Built-in themes
//! have none (a test enforces it); user themes show theirs and stay usable.

use std::sync::LazyLock;

use Paint::{Derived as D, Role as R};
use Role::{Accent, Background, Bad, Focus, Good, OnAccent, Raised, Selection, Surface, Text, TextMuted, Warn};

use crate::{CoreRoles, Derived, Rgba, Role, SuiteTheme, Variant, derive};

/// The contrast ratio of `fg` painted on `bg`, 1.0 to 21.0.
///
/// A translucent `fg` is composited over `bg` first; `bg` counts as opaque.
pub fn ratio(fg: Rgba, bg: Rgba) -> f32 {
    let bg = Rgba([bg.r(), bg.g(), bg.b(), 0xFF]);
    let a = fg.over(bg).luminance();
    let b = bg.luminance();
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The minimum ratio a pair needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Need {
    /// 4.5:1, text (WCAG 2.2 AA).
    Text,
    /// 3.5:1, secondary text on the hover, pressed and selected state backgrounds, which it
    /// sits on only while the pointer or the selection is there.
    MutedText,
    /// 3:1, focus rings, control edges, status marks and other graphics.
    NonText,
}

impl Need {
    pub fn min_ratio(self) -> f32 {
        match self {
            Need::Text => 4.5,
            Need::MutedText => 3.5,
            Need::NonText => 3.0,
        }
    }

    pub fn is_met(self, ratio: f32) -> bool {
        ratio >= self.min_ratio()
    }
}

/// A colour in a resolved theme: a core role or a [`Derived`] field by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Paint {
    Role(Role),
    Derived(&'static str),
}

impl Paint {
    pub fn color(self, roles: &CoreRoles, derived: &Derived) -> Option<Rgba> {
        match self {
            Paint::Role(r) => Some(roles.get(r)),
            Paint::Derived(name) => derived.get(name),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pair {
    pub fg: Paint,
    pub bg: Paint,
    pub need: Need,
}

/// A painted pair below its requirement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Diagnostic {
    pub variant: Variant,
    pub fg: Paint,
    pub bg: Paint,
    pub ratio: f32,
    pub need: Need,
}

type Row = (&'static [Paint], &'static [Paint], Need);

/// The table in docs/suite-themes.md: every foreground of a row on every background of it.
const ROWS: &[Row] = &[
    (&[R(Text)], &[R(Background), R(Surface), R(Raised), D("hover"), D("pressed"), R(Selection), D("selected_hover")], Need::Text),
    (&[R(TextMuted)], &[R(Background), R(Surface), R(Raised)], Need::Text),
    (&[R(TextMuted)], &[D("hover"), D("pressed"), R(Selection), D("selected_hover")], Need::MutedText),
    (&[D("text_faint")], &[R(Background), R(Surface), R(Raised)], Need::Text),
    (&[R(OnAccent)], &[R(Accent), D("accent_hover"), D("accent_pressed")], Need::Text),
    (&[D("accent_text")], &[R(Background), R(Surface), R(Raised)], Need::Text),
    (&[R(Focus), D("control_edge"), R(Accent)], &[R(Surface), R(Raised), R(Background)], Need::NonText),
    (&[R(Good), R(Warn), R(Bad)], &[R(Background), R(Surface)], Need::NonText),
];

static PAINTED: LazyLock<Vec<Pair>> = LazyLock::new(|| {
    ROWS.iter().flat_map(|&(fgs, bgs, need)| fgs.iter().flat_map(move |&fg| bgs.iter().map(move |&bg| Pair { fg, bg, need }))).collect()
});

/// Every pair the suite's apps paint from the core roles and derived states, with its need.
///
/// `line` is a decorative divider and has no requirement; edges that must be seen use
/// `control_edge`.
pub fn painted_pairs() -> &'static [Pair] {
    &PAINTED
}

/// Every painted pair of every variant of `t` that misses its need, with derived states from
/// [`derive`].
pub fn diagnose(t: &SuiteTheme) -> Vec<Diagnostic> {
    t.variants().flat_map(|(v, roles)| diagnose_variant(v, roles, &derive(roles, v))).collect()
}

/// [`diagnose`] for one variant, with derived states an app may have overwritten.
pub fn diagnose_variant(variant: Variant, roles: &CoreRoles, derived: &Derived) -> Vec<Diagnostic> {
    PAINTED
        .iter()
        .filter_map(|p| {
            let ratio = ratio(p.fg.color(roles, derived)?, p.bg.color(roles, derived)?);
            (!p.need.is_met(ratio)).then_some(Diagnostic { variant, fg: p.fg, bg: p.bg, ratio, need: p.need })
        })
        .collect()
}

/// Checks pairs an app paints outside the table: `(label, fg, bg, need)`. Returns the failing
/// labels with their ratios.
pub fn check(pairs: &[(&'static str, Rgba, Rgba, Need)]) -> Vec<(&'static str, f32)> {
    pairs
        .iter()
        .filter_map(|&(label, fg, bg, need)| {
            let r = ratio(fg, bg);
            (!need.is_met(r)).then_some((label, r))
        })
        .collect()
}

/// The lowest `ratio / need` over the painted pairs that have `name` on one side and a core role
/// on the other; at least 1.0 means all of them pass. Used to guard derived states.
pub(crate) fn margin(name: &str, candidate: Rgba, roles: &CoreRoles) -> f32 {
    PAINTED
        .iter()
        .filter_map(|p| {
            let (fg, bg) = match (p.fg, p.bg) {
                (Paint::Derived(n), Paint::Role(r)) if n == name => (candidate, roles.get(r)),
                (Paint::Role(r), Paint::Derived(n)) if n == name => (roles.get(r), candidate),
                _ => return None,
            };
            Some(ratio(fg, bg) / p.need.min_ratio())
        })
        .fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ratios() {
        assert!((ratio(Rgba::BLACK, Rgba::WHITE) - 21.0).abs() < 1e-3);
        assert!((ratio(Rgba::WHITE, Rgba::BLACK) - 21.0).abs() < 1e-3);
        assert!((ratio(Rgba::from_rgb_u32(0x777777), Rgba::WHITE) - 4.48).abs() < 0.01);
        // Half-transparent white on black lands on mid grey.
        let mid = ratio(Rgba([255, 255, 255, 128]), Rgba::BLACK);
        assert!((mid - ratio(Rgba::from_rgb_u32(0x808080), Rgba::BLACK)).abs() < 1e-3);
        // White on the plan's example accent.
        assert!((ratio(Rgba::WHITE, Rgba::from_rgb_u32(0x4B8BF5)) - 3.33).abs() < 0.01);
    }

    #[test]
    fn check_reports_only_failures() {
        let failing = check(&[
            ("ok", Rgba::BLACK, Rgba::WHITE, Need::Text),
            ("faint", Rgba::from_rgb_u32(0xAAAAAA), Rgba::WHITE, Need::Text),
            ("edge", Rgba::from_rgb_u32(0x949494), Rgba::WHITE, Need::NonText),
        ]);
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].0, "faint");
    }

    #[test]
    fn table_names_only_real_derived_fields() {
        for p in painted_pairs() {
            for side in [p.fg, p.bg] {
                if let Paint::Derived(name) = side {
                    assert!(Derived::KEYS.contains(&name), "{name}");
                }
            }
        }
    }
}
