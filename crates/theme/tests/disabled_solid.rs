//! A disabled solid button (primary, danger) keeps a readable label and stays clearly unlike the enabled one in every
//! built-in theme and contrast setting (wordcraft#7978).

use craft_theme::contrast::contrast_ratio;
use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, Variant, Variants, builtins, resolve};

/// Least contrast ratio between the disabled fill and the enabled one.
const FILL_SEPARATION: f64 = 1.5;

#[test]
fn disabled_solid_label_reads_and_the_button_is_not_the_enabled_one() {
    for theme in builtins() {
        // Classic draws a disabled button with its bevel; its pinned greys are not this look.
        if theme.name == "Classic" {
            continue;
        }
        let modes = match theme.variants {
            Variants::Both => vec![Mode::Light, Mode::Dark],
            Variants::LightOnly => vec![Mode::Light],
            Variants::DarkOnly => vec![Mode::Dark],
        };
        for mode in modes {
            for contrast in [Contrast::Standard, Contrast::High] {
                let appearance = Appearance { mode, contrast, size: Size::Comfortable, text_size: TextSize::Small, reduced_motion: false };
                let tokens = resolve(theme, appearance, None).tokens;
                for variant in [Variant::PRIMARY, Variant::DANGER] {
                    let look = variant.resolve(&tokens);
                    let who = format!("{} {mode:?} {contrast:?} {variant:?}", theme.name);
                    let label = contrast_ratio(look.text.disabled, look.fill.disabled);
                    assert!(label >= 4.5, "{who}: disabled label is {label:.2}:1");
                    let apart = contrast_ratio(look.fill.disabled, look.fill.rest);
                    assert!(apart >= FILL_SEPARATION, "{who}: disabled fill is {apart:.2}:1 from the enabled one");
                }
            }
        }
    }
}
