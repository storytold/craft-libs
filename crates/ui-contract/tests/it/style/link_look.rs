//! The link look reads at the text minimum on the surfaces links are drawn on.

use craft_theme::{Contrast, ControlSize, Rgba, States, Variant, contrast_ratio};
use craft_ui::style::SurfaceKind;
use craft_ui::testing::style::{themes_at, with_cx};
use craft_ui::testing::trial;

/// The 4.5:1 text minimum.
const TEXT_MIN: f64 = 4.5;

/// Link text at rest reaches the text minimum on the panel, chrome, raised and sunken surfaces of every built-in theme
/// in each of its modes, at both contrasts.
#[trial]
fn link_text_reaches_the_text_minimum_on_every_surface_it_sits_on() {
    let ctx = egui::Context::default();
    let mut short = Vec::new();
    for contrast in [Contrast::Standard, Contrast::High] {
        for (theme, tokens) in themes_at(contrast) {
            with_cx(&ctx, &tokens, Variant::SECONDARY, ControlSize::Md, |cx| {
                let States { rest, .. } = cx.link_look(true).color.off;
                for kind in [SurfaceKind::Panel, SurfaceKind::Chrome, SurfaceKind::Raised, SurfaceKind::Sunken] {
                    let ratio = contrast_ratio(Rgba::from(rest.0), Rgba::from(cx.surface(kind).fill));
                    if ratio < TEXT_MIN {
                        short.push(format!("{theme} {contrast:?} on {kind:?}: {ratio:.2}:1"));
                    }
                }
            });
        }
    }
    assert!(short.is_empty(), "link text below {TEXT_MIN}:1:\n{}", short.join("\n"));
}
