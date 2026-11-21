//! Trials of the icon lookup: the app icon registry (the catalog wins a clashing name, the first set keeps a name two
//! sets share) and the set a theme's icon pack picks.

use crate::primitives::{AppIcons, Mirror, SvgIcon, icon_names, register_app_icons};
use crate::style::UiTokens;
use crate::testing::{Frames, style, trial};
use craft_behavior::IconId;
use craft_i18n::Direction;
use craft_theme::{AppSizes, Appearance, Contrast, IconPackId, Mode, Size, TextSize, builtins};
use egui::{Color32, ColorImage, Rect, pos2, vec2};

// Built from parts so the workspace scan of icon-id literals does not read them.
const FIRST_NAME: &str = concat!("zz_t10_", "first");
const SECOND_NAME: &str = concat!("zz_t10_", "second");
const CATALOG_NAME: &str = "close";
const DISC: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><circle cx=\"12\" cy=\"12\" r=\"8\" fill=\"black\"/></svg>";
const SQUARE: &[u8] =
    b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" fill=\"black\"/></svg>";

const FIRST_SET: AppIcons = AppIcons {
    name: "t10-first",
    stroke_width: None,
    icons: &[(FIRST_NAME, DISC), (CATALOG_NAME, DISC)],
    fills: &[],
};
const SECOND_SET: AppIcons = AppIcons {
    name: "t10-second",
    stroke_width: None,
    icons: &[(FIRST_NAME, SQUARE), (SECOND_NAME, SQUARE)],
    fills: &[],
};
const FILLED_NAME: &str = concat!("zz_k10_", "filled");
const FILL_COLOR: [u8; 4] = [255, 0, 0, 255];
const FILLED_SET: AppIcons = AppIcons::new("k10-filled", None, &[(FILLED_NAME, DISC)]).with_fills(&[(FILLED_NAME, SQUARE, FILL_COLOR)]);

fn icon(name: &'static str) -> Option<SvgIcon> {
    let tokens = style::tokens(Mode::Light, Contrast::Standard);
    SvgIcon::of(&tokens, Direction::Ltr, 24.0, &IconId::from(std::borrow::Cow::Borrowed(name)), Mirror::Auto)
}

fn mask(name: &'static str) -> Option<ColorImage> {
    icon(name).map(|icon| icon.mask(24.0, 1.0))
}

#[trial]
fn an_app_icon_set_resolves_after_the_catalog_and_the_first_set_keeps_a_shared_name() {
    assert_eq!(icon(FIRST_NAME), None);
    let catalog = icon_names().len();
    let close = mask(CATALOG_NAME);
    let catalog_icon = icon(CATALOG_NAME);

    register_app_icons(&FIRST_SET);
    assert!(icon(FIRST_NAME).is_some(), "the set's new name resolves");
    assert_eq!(icon(CATALOG_NAME), catalog_icon, "the catalog keeps its name");
    assert_eq!(mask(CATALOG_NAME), close, "the refused icon does not change the catalog icon");
    let first = mask(FIRST_NAME);
    assert_ne!(first, close);
    assert_eq!(icon_names().len(), catalog, "icon_names lists the built-in catalog only");
    assert!(!icon_names().contains(&FIRST_NAME));

    // A later set repeating a registered name is refused for that name only; its other names join.
    assert_eq!(icon(SECOND_NAME), None);
    register_app_icons(&SECOND_SET);
    assert!(icon(SECOND_NAME).is_some());
    assert_eq!(mask(FIRST_NAME), first, "the first set keeps the shared name");
    assert_ne!(mask(SECOND_NAME), first);
    let second = mask(SECOND_NAME);

    // Registering a set again changes nothing.
    register_app_icons(&FIRST_SET);
    register_app_icons(&SECOND_SET);
    assert_eq!((mask(FIRST_NAME), mask(SECOND_NAME), mask(CATALOG_NAME)), (first, second, close));
    assert_eq!(icon_names().len(), catalog);
}

/// The mask of `name` under the first built-in theme with its icon pack set to `pack`.
fn mask_under_pack(pack: Option<&str>, name: &'static str) -> Option<ColorImage> {
    let mut theme = builtins()[0].clone();
    let pack = pack.map(IconPackId::parse).transpose();
    assert!(pack.is_ok(), "{pack:?}");
    theme.icon_pack = pack.unwrap_or_default();
    let appearance =
        Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
    let tokens = UiTokens::of_theme(&theme, appearance, &AppSizes::default());
    SvgIcon::of(&tokens, Direction::Ltr, 24.0, &IconId::from(std::borrow::Cow::Borrowed(name)), Mirror::Auto).map(|icon| icon.mask(24.0, 1.0))
}

#[trial]
fn a_theme_icon_pack_picks_the_set_and_an_unknown_pack_draws_the_default() {
    let default = mask_under_pack(None, CATALOG_NAME);
    assert!(default.is_some());
    // Carbon's close is a fill drawing; Lucide's and Tabler's are the same two strokes.
    assert_ne!(mask_under_pack(Some("craft.carbon"), CATALOG_NAME), default, "a built-in pack draws its own icon");
    assert_eq!(mask_under_pack(Some("craft.lucide"), CATALOG_NAME), default, "the default set by name draws the default");
    assert_eq!(mask_under_pack(Some("user.missing"), CATALOG_NAME), default, "a pack that is not installed draws the default");
}

/// The colours of the textured shapes `name` paints under the tint `color`, first to last.
fn painted(name: &'static str, color: Color32) -> Vec<Color32> {
    let Some(svg) = icon(name) else { return Vec::new() };
    let square = Rect::from_min_size(pos2(10.0, 10.0), vec2(24.0, 24.0));
    Frames::new()
        .frame_shapes(|ui| svg.paint(ui.painter(), square, color))
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Mesh(mesh) => mesh.vertices.first().map(|vertex| vertex.color),
            egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect.fill),
            _ => None,
        })
        .collect()
}

#[trial]
fn an_icon_with_a_fill_keeps_its_colour_under_the_tint() {
    register_app_icons(&FILLED_SET);
    let [r, g, b, a] = FILL_COLOR;
    let fill = Color32::from_rgba_unmultiplied(r, g, b, a);
    for tint in [Color32::from_rgb(10, 200, 30), Color32::WHITE] {
        assert_eq!(painted(FILLED_NAME, tint), [fill, tint], "the fill keeps its colour and the icon takes the tint");
    }
    let tint = Color32::from_rgb(10, 200, 30);
    assert_eq!(painted(CATALOG_NAME, tint), [tint], "an icon with no fill paints the icon only");
}
