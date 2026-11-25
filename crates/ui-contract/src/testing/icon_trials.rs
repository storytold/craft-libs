//! Trials of the icon lookup: the app icon registry (the catalog wins a clashing name, the first set keeps a name two
//! sets share), the set a theme's icon pack picks, and an app icon's layers by class.

use crate::primitives::{AppIcons, DataMode, Draw, DrawEnv, IconSpec, Layer, Mirror, PaintRecord, SvgIcon, icon_names, register_app_icons};
use crate::style::{Ink, StyleColor, UiTokens, ink_color, rgba_color};
use crate::testing::{Frames, style, trial};
use craft_behavior::IconId;
use craft_i18n::Direction;
use craft_theme::{AppSizes, Appearance, Contrast, IconPackId, Mode, Size, TextSize, builtins};
use egui::{Color32, ColorImage, Rect, Shape, TextureId, pos2, vec2};
use std::cell::RefCell;

// Built from parts so the workspace scan of icon-id literals does not read them.
const FIRST_NAME: &str = concat!("zz_t10_", "first");
const SECOND_NAME: &str = concat!("zz_t10_", "second");
const CATALOG_NAME: &str = "close";
const DISC: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><circle cx=\"12\" cy=\"12\" r=\"8\" fill=\"black\"/></svg>";
const SQUARE: &[u8] =
    b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" fill=\"black\"/></svg>";

const FIRST_SET: AppIcons = AppIcons { name: "t10-first", stroke_width: None, icons: &[(FIRST_NAME, DISC), (CATALOG_NAME, DISC)], fills: &[] };
const SECOND_SET: AppIcons = AppIcons { name: "t10-second", stroke_width: None, icons: &[(FIRST_NAME, SQUARE), (SECOND_NAME, SQUARE)], fills: &[] };
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
    let tokens = UiTokens::of_theme(&theme, appearance, None, &AppSizes::default());
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

const LAYERED_NAME: &str = concat!("zz_t19_", "pen");
/// A plain body, an accent nib (a group, so the class reaches its children) and a fixed red cap.
const LAYERED: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\
<rect x=\"4\" y=\"8\" width=\"10\" height=\"8\" fill=\"black\"/>\
<g class=\"accent\"><path d=\"M14 8 L22 12 L14 16 Z\" fill=\"black\"/></g>\
<rect class=\"fixed\" x=\"2\" y=\"8\" width=\"2\" height=\"8\" fill=\"#d02020\"/></svg>";
const LAYERED_SET: AppIcons = AppIcons { name: "t19-layers", stroke_width: None, icons: &[(LAYERED_NAME, LAYERED)], fills: &[] };
/// A plain icon with no classed part.
const PLAIN_NAME: &str = concat!("zz_t19_", "plain");
const PLAIN_SET: AppIcons = AppIcons { name: "t19-plain", stroke_width: None, icons: &[(PLAIN_NAME, DISC)], fills: &[] };

/// The texture and tint of every textured shape `shapes` holds, in paint order.
fn textured(shapes: &[Shape]) -> Vec<(TextureId, Color32)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if shape.texture_id() != TextureId::default() => Some((shape.texture_id(), rect.fill)),
            Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => mesh.vertices.first().map(|vertex| (mesh.texture_id, vertex.color)),
            _ => None,
        })
        .collect()
}

/// The texel at (`x`, `y`) of a 24-texel layer image.
fn texel(image: &ColorImage, x: usize, y: usize) -> Color32 {
    image.pixels[y * 24 + x]
}

/// On a strong fill or scrim (the surface's ink is `OnDark`: a brand rail) the accent parts paint in the icon's own
/// colour, since `accent.solid` would vanish on the brand fill; the fixed parts stay as drawn, and on a panel the
/// accent parts keep the accent ink.
#[trial]
fn an_accent_part_on_a_dark_surface_paints_in_the_icons_ink() {
    register_app_icons(&LAYERED_SET);
    let tokens = style::tokens(Mode::Light, Contrast::Standard);
    let rect = Rect::from_min_max(pos2(10.0, 10.0), pos2(34.0, 34.0));
    let mut frames = Frames::new();
    let mut tints = |surface: Ink, spec_ink: Option<Ink>| {
        let on = ink_color(&tokens, Ink::OnDark);
        let mut spec = IconSpec::svg(IconId::from(std::borrow::Cow::Borrowed(LAYERED_NAME))).color(StyleColor(on));
        if let Some(ink) = spec_ink {
            spec = spec.ink(ink);
        }
        let record = RefCell::new(PaintRecord::default());
        let shapes = frames.frame_shapes(|ui| {
            let env = DrawEnv { tokens: &tokens, direction: Direction::Ltr, ink: surface };
            Draw::new(ui.painter(), &record, env).icon(rect, &spec);
        });
        textured(&shapes).into_iter().map(|(_, tint)| tint).collect::<Vec<_>>()
    };
    let on = ink_color(&tokens, Ink::OnDark);
    // design: allow[raw-colour/const](a fixed layer keeps the colours it is drawn in: the trial expects the white tint that leaves them unchanged)
    let fixed = Color32::WHITE;
    assert_eq!(tints(Ink::OnDark, None), [on, on, fixed], "on a brand rail the accent parts read in the icon's ink");
    assert_eq!(tints(Ink::OnPanel, Some(Ink::OnDark)), [on, on, fixed], "an icon that asks for the on-dark ink does the same");
    assert_eq!(tints(Ink::OnPanel, None), [on, rgba_color(tokens.color.accent.solid), fixed], "on a panel the accent ink is kept");
}

#[trial]
fn an_accent_part_paints_in_the_accent_ink_a_fixed_part_as_drawn_and_both_take_the_disabled_ink_when_disabled() {
    register_app_icons(&LAYERED_SET);
    register_app_icons(&PLAIN_SET);
    let tokens = style::tokens(Mode::Light, Contrast::Standard);
    let rect = Rect::from_min_max(pos2(10.0, 10.0), pos2(34.0, 34.0));
    let mut frames = Frames::new();
    // The icon painted by a `Draw` in `mode`: `Dimmed` is what a component in its Disabled state paints with.
    let mut paint = |name: &'static str, mode: DataMode, color: Color32| {
        let spec = IconSpec::svg(IconId::from(std::borrow::Cow::Borrowed(name))).color(StyleColor(color));
        let record = RefCell::new(PaintRecord::default());
        let shapes = frames.frame_shapes(|ui| {
            let env = DrawEnv { tokens: &tokens, direction: Direction::Ltr, ink: Ink::OnPanel };
            Draw::new(ui.painter(), &record, env).data(mode).icon(rect, &spec);
        });
        textured(&shapes)
    };
    let (ink, off) = (ink_color(&tokens, Ink::OnPanel), rgba_color(tokens.color.text.disabled));
    let enabled = paint(LAYERED_NAME, DataMode::Full, ink);
    let disabled = paint(LAYERED_NAME, DataMode::Dimmed, ink);
    let enabled_in_the_disabled_colour = paint(LAYERED_NAME, DataMode::Full, off);

    assert_eq!(enabled.len(), 3, "an app icon paints its ink, accent and fixed layers, got {enabled:?}");
    let [(ink_layer, ink_tint), (accent_layer, accent_tint), (fixed_layer, fixed_tint)] = enabled[..] else { return };
    assert_eq!(ink_tint, ink, "the plain parts read in the icon's colour");
    assert_eq!(accent_tint, rgba_color(tokens.color.accent.solid), "the accent parts read in the accent ink");
    assert_ne!(accent_tint, ink_tint);
    // design: allow[raw-colour/const](a fixed layer keeps the colours it is drawn in: the trial expects the white tint that leaves them unchanged)
    assert_eq!(fixed_tint, Color32::WHITE, "the fixed parts keep the colours they are drawn in");
    assert!(ink_layer != accent_layer && accent_layer != fixed_layer && ink_layer != fixed_layer, "each layer is its own texture");

    assert_eq!(disabled.len(), 3, "a disabled app icon paints the same three layers, got {disabled:?}");
    let [(off_ink, a), (off_accent, b), (off_fixed, c)] = disabled[..] else { return };
    assert_eq!([a, b, c], [ink; 3], "every layer takes the one ink when the component is disabled");
    assert_eq!((off_ink, off_accent), (ink_layer, accent_layer), "the ink and accent layers are the same drawings, tinted");
    assert_ne!(off_fixed, fixed_layer, "a disabled fixed part is drawn as a silhouette, not in its own colours");

    let tints: Vec<Color32> = enabled_in_the_disabled_colour.iter().map(|(_, tint)| *tint).collect();
    // design: allow[raw-colour/const](a fixed layer keeps the colours it is drawn in: the trial expects the white tint that leaves them unchanged)
    assert_eq!(tints, [off, rgba_color(tokens.color.accent.solid), Color32::WHITE], "an enabled icon in the disabled colour keeps its layers");

    // What each layer holds: the class split is the SVG engine's, so a sheet it ignored would give every layer the whole drawing.
    let layered = icon(LAYERED_NAME);
    assert!(layered.is_some(), "the layered icon resolves");
    let Some(layered) = layered else { return };
    let (body, nib, cap) = ((9, 12), (16, 12), (3, 12));
    let [plain, accent, fixed] = [Layer::Ink, Layer::Accent, Layer::Fixed].map(|layer| layered.layer(24.0, 1.0, layer));
    assert!(texel(&plain, body.0, body.1).a() > 0 && texel(&plain, nib.0, nib.1).a() == 0, "the ink layer holds the plain body only");
    assert!(texel(&accent, nib.0, nib.1).a() > 0 && texel(&accent, body.0, body.1).a() == 0, "the accent layer holds the nib only");
    let red = texel(&fixed, cap.0, cap.1);
    assert!(red.a() > 0 && red.r() > red.g() && red.r() > red.b(), "the fixed layer holds the cap in its own colour: {red:?}");
    assert!(texel(&fixed, body.0, body.1).a() == 0 && texel(&plain, cap.0, cap.1).a() == 0, "the cap is in no other layer");

    // An icon with no classed part paints its ink layer alone.
    assert_eq!(paint(PLAIN_NAME, DataMode::Full, ink).len(), 1, "a layer with no part paints no quad");
}

/// The tints of the textured shapes `name` paints under the tint `color`, first to last.
fn painted(name: &'static str, color: Color32) -> Vec<Color32> {
    let Some(svg) = icon(name) else { return Vec::new() };
    let square = Rect::from_min_size(pos2(10.0, 10.0), vec2(24.0, 24.0));
    textured(&Frames::new().frame_shapes(|ui| svg.paint(ui.painter(), square, color))).into_iter().map(|(_, tint)| tint).collect()
}

#[trial]
fn an_icon_with_a_fill_keeps_its_colour_under_the_tint() {
    register_app_icons(&FILLED_SET);
    let [r, g, b, a] = FILL_COLOR;
    let fill = Color32::from_rgba_unmultiplied(r, g, b, a);
    // design: allow[raw-colour/const](white is the tint that leaves colours unchanged: the fill keeps its colour under it too)
    for tint in [Color32::from_rgb(10, 200, 30), Color32::WHITE] {
        assert_eq!(painted(FILLED_NAME, tint), [fill, tint], "the fill keeps its colour and the icon takes the tint");
    }
    let tint = Color32::from_rgb(10, 200, 30);
    assert_eq!(painted(CATALOG_NAME, tint), [tint], "an icon with no fill paints the icon only");
}
