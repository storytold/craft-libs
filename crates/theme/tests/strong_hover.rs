//! Primary / accent buttons (`accent.strong`) must show hover and press in every built-in theme,
//! in both modes, without the text on them dropping below the text minimum (wordcraft#7317 C7-14).

use craft_theme::contrast::contrast_ratio;
use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, Variants, builtins, resolve};

/// Least OKLCH lightness change of hover and of press from rest. Below about 0.05 a fill change is
/// hard to see next to a neighbouring control.
const HOVER_MIN: f64 = 0.05;
const PRESSED_MIN: f64 = 0.08;

#[test]
fn strong_accent_hover_and_pressed_are_visible_and_keep_text_contrast() {
    for theme in builtins() {
        // Classic pins all three states to one grey: its bevel, not the fill, shows the state.
        if theme.name == "Classic" {
            continue;
        }
        let modes = match theme.variants {
            Variants::Both => vec![Mode::Light, Mode::Dark],
            Variants::LightOnly => vec![Mode::Light],
            Variants::DarkOnly => vec![Mode::Dark],
        };
        for mode in modes {
            let appearance =
                Appearance { mode, contrast: Contrast::Standard, size: Size::Comfortable, text_size: TextSize::Small, reduced_motion: false };
            let c = resolve(theme, appearance, None).tokens.color;
            let (rest, hover, pressed) = (c.accent.strong, c.accent.strong_hover, c.accent.strong_pressed);
            let l = |x: craft_theme::color::Rgba| x.to_oklch().l;
            let who = format!("{} {mode:?}", theme.name);
            assert!((l(hover) - l(rest)).abs() >= HOVER_MIN, "{who}: hover barely differs from rest");
            assert!((l(pressed) - l(rest)).abs() >= PRESSED_MIN, "{who}: pressed barely differs from rest");
            for (state, fill) in [("rest", rest), ("hover", hover), ("pressed", pressed)] {
                let ratio = contrast_ratio(c.text.on_strong, fill);
                assert!(ratio >= 4.5, "{who}: text.on_strong on {state} is {ratio:.2}");
            }
        }
    }
}

/// A user asking why the hover fill differs from rest is told which token it starts from and how much
/// lightness it moves at least; the floor shown is the one the colour is held to.
#[test]
fn explaining_strong_hover_names_its_base_and_the_lightness_floor() {
    use craft_theme::{ColorSource, ColorToken, explain};
    let theme = builtins().iter().find(|t| t.name != "Classic" && t.variants != Variants::LightOnly).unwrap();
    let appearance =
        Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Comfortable, text_size: TextSize::Small, reduced_motion: false };
    let e = explain(theme, appearance, None);
    let origin = e.color(ColorToken::AccentStrongHover).unwrap();
    let ColorSource::ShiftedFrom { min, .. } = origin.source else { panic!("hover is a shifted colour: {:?}", origin.source) };
    let words = origin.source_words();
    assert!(words.contains(ColorToken::AccentStrong.path()), "names the base token: {words}");
    assert!(words.contains(&min.to_string()), "names the floor {min}: {words}");
}
