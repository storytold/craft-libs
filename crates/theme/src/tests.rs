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

#[test]
fn non_text_contrast_min_matches_theme_values() {
    assert_eq!(non_text_contrast_min(Contrast::Standard), 3.0);
    assert_eq!(non_text_contrast_min(Contrast::High), 4.5);
}

/// A user's accent over each built-in theme, in each mode and contrast and for each app the theme overrides, gives
/// the accent fill its own colour even where the theme fixes the fill, and fails no contrast pair the theme alone
/// passes.
#[test]
fn an_accent_over_a_theme_takes_its_fill_and_fails_no_pair_the_theme_passes() {
    use std::collections::BTreeSet;
    let failing = |resolved: &Resolved| resolved.warnings.iter().map(|check| (check.fg, check.bg)).collect::<BTreeSet<_>>();
    let seeds = [Rgba::rgb(0xff, 0xd8, 0x00), Rgba::rgb(0x12, 0x18, 0x60), Rgba::rgb(0xd0, 0x20, 0x30)];
    for theme in builtins() {
        for app in std::iter::once(None).chain(theme.apps.keys().map(Some)) {
            for (mode, contrast) in
                [(Mode::Light, Contrast::Standard), (Mode::Dark, Contrast::Standard), (Mode::Light, Contrast::High), (Mode::Dark, Contrast::High)]
            {
                let appearance = Appearance { mode, contrast, size: Size::default(), text_size: TextSize::default(), reduced_motion: false };
                let own = failing(&resolve(theme, appearance, app));
                let fills: Vec<Rgba> = seeds
                    .iter()
                    .map(|&seed| {
                        let accented = resolve(&theme.with_accent(seed), appearance, app);
                        let new: Vec<_> = failing(&accented).difference(&own).copied().collect();
                        assert!(new.is_empty(), "{} {app:?} {mode:?} {contrast:?} with accent {seed:?} fails {new:?}", theme.id.as_str());
                        accented.tokens.color(ColorToken::AccentSolid)
                    })
                    .collect();
                assert!(fills[0] != fills[1], "{} {app:?} {mode:?} {contrast:?}: the fill follows the accent: {fills:?}", theme.id.as_str());
            }
        }
    }
}
