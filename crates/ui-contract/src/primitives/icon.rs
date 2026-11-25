//! `IconSpec`: an icon to paint, and the icon sets a theme's icon pack picks among.

use super::svg_set;
use super::textures::TextureCache;
use crate::contract::{MemoKey, MemoProps};
use crate::style::{DataColor, Ink, StyleColor, UiTokens, ink_color, log_once};
use craft_behavior::IconId;
use craft_i18n::Direction;
use craft_theme::{IconPackId, Srgba};
use egui::emath::GuiRounding as _;
use egui::{Color32, Painter, Rect, pos2, vec2};
use std::any::Any;

pub use super::svg_set::IconSet;

/// Whether a directional icon flips under right-to-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum Mirror {
    /// Flips a directional icon under right-to-left.
    #[default]
    Auto,
    /// Never flips.
    Never,
}

/// A value glyph: an icon drawn from a value. It is `Sync` because container props that show one are kept with a
/// body's measurement in layout memory. It is props ([`MemoProps`]): an icon keys a memoized view by the glyph's type,
/// then by what the glyph feeds, so a glyph owns what it draws from (`'static`).
pub trait Glyph: Any + MemoProps + Sync {
    /// Whether the glyph has a reading direction (an arrow) and so flips under right-to-left.
    fn directional(&self) -> bool {
        false
    }

    /// Paints the glyph into `rect` in `color`, with the strokes of `tokens`. `mirrored` is set when a directional
    /// glyph is painted under right-to-left: the glyph draws itself flipped.
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, mirrored: bool);
}

/// Where an icon's drawing comes from.
#[derive(Clone)]
#[non_exhaustive]
pub enum IconSource<'a> {
    /// An SVG-set or tool icon, by its egui-free key.
    Svg(IconId),
    /// A value glyph.
    Glyph(&'a (dyn Glyph + 'static)),
    /// A colour from data, painted as a swatch (a guide colour, a matte colour); `None` paints the empty swatch of a
    /// colour the theme decides.
    Swatch(Option<Srgba>),
    /// A filled circle as wide as the icon's square, in the spec's colour or ink (`IconSpec::data_color` for a colour
    /// from data).
    Disc,
    /// A letter in an outlined box, in the spec's colour or ink: the icon of something that has none, from the first
    /// letter of its name. Painted by [`Draw::icon`](super::Draw::icon) as text, in the theme's fonts.
    Initial(char),
    /// The mark of a cursor badge, painted by the cursor-badge painter in the spec's colour or ink.
    Mark(craft_behavior::CursorBadge),
}

/// By variant, then what it draws: a glyph by its type, then by what it feeds.
impl MemoProps for IconSource<'_> {
    fn key(&self, key: &mut MemoKey) {
        match self {
            IconSource::Svg(id) => {
                key.write_u32(0);
                id.key(key);
            }
            IconSource::Glyph(glyph) => {
                key.write_u32(1);
                let any: &dyn Any = *glyph;
                key.feed_hash(&any.type_id());
                glyph.key(key);
            }
            IconSource::Swatch(color) => {
                key.write_u32(2);
                color.key(key);
            }
            IconSource::Disc => key.write_u32(3),
            IconSource::Initial(letter) => {
                key.write_u32(4);
                letter.key(key);
            }
            IconSource::Mark(badge) => {
                key.write_u32(5);
                key.feed_hash(badge);
            }
        }
    }
}

impl std::fmt::Debug for IconSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IconSource::Svg(id) => f.debug_tuple("Svg").field(id).finish(),
            IconSource::Glyph(_) => f.write_str("Glyph(..)"),
            IconSource::Swatch(color) => f.debug_tuple("Swatch").field(color).finish(),
            IconSource::Disc => f.write_str("Disc"),
            IconSource::Initial(letter) => f.debug_tuple("Initial").field(letter).finish(),
            IconSource::Mark(badge) => f.debug_tuple("Mark").field(badge).finish(),
        }
    }
}

/// Two sources are equal when they name the same SVG icon, hold the same glyph object or the same swatch colour.
impl PartialEq for IconSource<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (IconSource::Svg(a), IconSource::Svg(b)) => a == b,
            (IconSource::Glyph(a), IconSource::Glyph(b)) => std::ptr::addr_eq(*a, *b),
            (IconSource::Swatch(a), IconSource::Swatch(b)) => a == b,
            (IconSource::Disc, IconSource::Disc) => true,
            (IconSource::Initial(a), IconSource::Initial(b)) => a == b,
            (IconSource::Mark(a), IconSource::Mark(b)) => a == b,
            _ => false,
        }
    }
}

/// An icon to paint.
#[derive(Clone, Debug, PartialEq, crate::contract::MemoProps)]
pub struct IconSpec<'a> {
    /// The drawing.
    pub source: IconSource<'a>,
    /// The ink the icon reads in.
    pub ink: Ink,
    /// A colour a style resolved from the state set (hover, disabled, tone); it replaces the ink's colour.
    pub color: Option<StyleColor>,
    /// Mirroring under right-to-left.
    pub mirror: Mirror,
    /// A data-colour mark on the icon: a dot at the bottom-trailing corner (see [`badge`](Self::badge)) or a bar across
    /// the bottom (see [`with_mark`](Self::with_mark)).
    pub badge: Option<IconMark>,
}

/// A data-colour mark on an icon.
#[derive(Clone, Copy, Debug, PartialEq, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum IconMark {
    /// A dot at the icon's bottom-trailing corner, ringed in the icon's colour.
    Dot(DataColor),
    /// A bar across the bottom of the icon square, inside it; `weight` is its thickness in points, else `StrokeWidth::Thick`.
    #[non_exhaustive]
    Bar {
        /// The bar's colour.
        color: DataColor,
        /// The thickness in points; `None` takes `StrokeWidth::Thick`.
        weight: Option<f32>,
    },
}

impl IconMark {
    /// A bar of `color` with the default thickness.
    pub const fn bar(color: DataColor) -> Self {
        IconMark::Bar { color, weight: None }
    }

    /// A bar of `color` `weight` points thick.
    pub const fn weighted_bar(color: DataColor, weight: f32) -> Self {
        IconMark::Bar { color, weight: Some(weight) }
    }
}

impl<'a> IconSpec<'a> {
    /// An icon of the SVG set or a tool icon, in the panel ink.
    pub fn svg(id: IconId) -> Self {
        IconSpec { source: IconSource::Svg(id), ink: Ink::OnPanel, color: None, mirror: Mirror::Auto, badge: None }
    }

    /// An icon whose glyph is the mark of the cursor `badge` (the plus or minus drawn at the cursor), painted by the
    /// one cursor-badge painter.
    pub fn mark(badge: craft_behavior::CursorBadge) -> IconSpec<'a> {
        IconSpec { source: IconSource::Mark(badge), ink: Ink::OnPanel, color: None, mirror: Mirror::Auto, badge: None }
    }

    /// An icon of `glyph` in the panel ink.
    pub fn glyph(glyph: &'a (dyn Glyph + 'static)) -> Self {
        IconSpec { source: IconSource::Glyph(glyph), ink: Ink::OnPanel, color: None, mirror: Mirror::Auto, badge: None }
    }

    /// A swatch of `color`, painted as [`Draw::swatch_well`](super::Draw::swatch_well) paints a well.
    pub fn swatch(color: Srgba) -> Self {
        IconSpec { source: IconSource::Swatch(Some(color)), ink: Ink::OnPanel, color: None, mirror: Mirror::Never, badge: None }
    }

    /// The empty swatch of a colour the theme decides (a "Default" pasteboard colour): it keeps a swatch's place, so
    /// the labels after a column of swatches line up.
    pub fn empty_swatch() -> Self {
        IconSpec { source: IconSource::Swatch(None), ..IconSpec::swatch(Srgba::rgb(0.0, 0.0, 0.0)) }
    }

    /// The first letter of `name` in capitals, in an outlined box, in the panel ink: the icon of something that has
    /// none. A blank name shows an empty box.
    pub fn initial(name: &str) -> Self {
        let letter = name.trim_start().chars().next().map_or(' ', |first| first.to_uppercase().next().unwrap_or(first));
        IconSpec { source: IconSource::Initial(letter), ink: Ink::OnPanel, color: None, mirror: Mirror::Never, badge: None }
    }

    /// A filled circle in the panel ink, or in the colour [`color`](Self::color) or [`data_color`](Self::data_color)
    /// sets (a status dot, a person's dot).
    pub fn disc() -> Self {
        IconSpec { source: IconSource::Disc, ink: Ink::OnPanel, color: None, mirror: Mirror::Never, badge: None }
    }

    /// Sets the ink.
    pub fn ink(mut self, ink: Ink) -> Self {
        self.ink = ink;
        self
    }

    /// Sets the colour, in place of the ink's.
    pub fn color(mut self, color: StyleColor) -> Self {
        self.color = Some(color);
        self
    }

    /// Sets the colour from data, in place of the ink's: an icon drawn in document content (an annotation pictogram),
    /// whose colour belongs to the document and not to a state. It fills the colour slot of [`color`](Self::color), so
    /// a style that resolves a colour for an icon without one leaves it.
    pub fn data_color(mut self, color: DataColor) -> Self {
        self.color = Some(StyleColor(color.0));
        self
    }

    /// Sets the mirroring.
    pub fn mirror(mut self, mirror: Mirror) -> Self {
        self.mirror = mirror;
        self
    }

    /// Adds a dot of `color` at the bottom-trailing corner of an SVG or glyph icon, ringed in the icon's colour: the
    /// tone a point picker sets. The dot is a data colour (`Draw::data_fill`); the ring follows the icon's colour.
    pub fn badge(mut self, color: DataColor) -> Self {
        self.badge = Some(IconMark::Dot(color));
        self
    }

    /// Puts `mark` on the icon in place of any mark it has.
    pub fn with_mark(mut self, mark: IconMark) -> Self {
        self.badge = Some(mark);
        self
    }
}

impl IconSpec<'_> {
    /// The colour the spec names, else its ink's colour: what the glyph, a swatch's edge and the badge's ring paint in.
    pub(crate) fn color_in(&self, tokens: &UiTokens) -> Color32 {
        self.color.map_or_else(|| ink_color(tokens, self.ink), StyleColor::egui)
    }
}

impl From<IconId> for IconSpec<'_> {
    fn from(id: IconId) -> Self {
        IconSpec::svg(id)
    }
}

/// Whether the built-in catalog or a registered app icon set has an icon named `id`.
pub fn has_icon(id: &IconId) -> bool {
    svg_set::lookup(id.name()).is_some()
}

/// The names of the built-in catalog's icons, sorted; app icon sets registered with [`register_app_icons`](super::register_app_icons) are not
/// listed.
pub fn icon_names() -> &'static [&'static str] {
    svg_set::names()
}

/// Paints `spec` centred in `rect` as components do, so a [`Glyph`] (which gets a raw `Painter`) can draw a set icon
/// inside its own drawing: in its enabled inks, on the surface its own `ink` names. `Draw::icon` adds the badge.
pub fn paint_icon(painter: &Painter, tokens: &UiTokens, direction: Direction, rect: Rect, spec: &IconSpec<'_>) {
    paint(painter, tokens, direction, rect, spec, false, false);
}

/// Paints `spec`'s glyph centred in the square of `rect`'s shorter side; `Draw::icon` adds the badge. `single_ink` paints
/// every layer of an app icon in the one colour: the icon of a component in its disabled state. `on_dark` is whether
/// the icon stands on a strong fill or scrim (the surface's ink is `Ink::OnDark`): its accent parts then paint in the
/// icon's colour, since `accent.solid` would vanish on the brand fill.
pub(crate) fn paint(painter: &Painter, tokens: &UiTokens, direction: Direction, rect: Rect, spec: &IconSpec<'_>, single_ink: bool, on_dark: bool) {
    let color = spec.color_in(tokens);
    let rtl = direction == Direction::Rtl;
    let side = rect.width().min(rect.height());
    let square = Rect::from_center_size(rect.center(), vec2(side, side));
    match &spec.source {
        // `Draw::icon` paints swatches with the data-colour fill and initials as text.
        IconSource::Swatch(_) | IconSource::Initial(_) => {}
        IconSource::Disc => {
            painter.circle_filled(square.center(), side / 2.0, color);
        }
        IconSource::Glyph(glyph) => glyph.paint(painter, tokens, square, color, rtl && glyph.directional() && spec.mirror == Mirror::Auto),
        IconSource::Svg(id) => {
            if let Some(icon) = SvgIcon::of(tokens, direction, side, id, spec.mirror) {
                icon.paint_layers(painter, square, LayerInks::of(tokens, color, single_ink, on_dark || spec.ink == Ink::OnDark));
            }
        }
        IconSource::Mark(badge) => super::pointer_art::paint_badge(painter, tokens, direction, square, *badge, color),
    }
}

impl IconSet {
    /// The built-in set a theme's `icon_pack` names: `craft.` and the set's [`id`](IconSet::id), such as
    /// `craft.tabler`; `None` for any other id.
    pub fn of_pack(id: &IconPackId) -> Option<IconSet> {
        let name = id.as_str().strip_prefix(craft_theme::RESERVED_PREFIX)?;
        IconSet::ALL.into_iter().find(|set| set.id() == name)
    }
}

/// A catalog or app icon as it is drawn: its entry and set, whether it is mirrored, and its stroke width in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SvgIcon {
    entry: svg_set::Entry,
    set: IconSet,
    mirrored: bool,
    stroke: f32,
}

impl SvgIcon {
    /// The icon `id` in the set `tokens`' theme draws, at `side` points under `direction` and `mirror`; `None`, with a
    /// log line the first time, for a name the catalog and the registered app sets lack.
    pub(crate) fn of(tokens: &UiTokens, direction: Direction, side: f32, id: &IconId, mirror: Mirror) -> Option<SvgIcon> {
        let Some(entry) = svg_set::lookup(id.name()) else {
            let name = id.name();
            log_once(format!("icon:{name}"), || log::error!("icon {name:?} is not in the catalog"));
            return None;
        };
        let mirrored = direction == Direction::Rtl && svg_set::directional(entry) && mirror == Mirror::Auto;
        Some(SvgIcon { entry, set: tokens.icons(), mirrored, stroke: stroke(tokens, side) })
    }

    /// The image's side in pixels and its stroke in 64ths of a pixel for a square of `side` points at `ppp`: the
    /// texture key, so requests that draw the same pixels share an entry. A document drawn with fills ignores the
    /// stroke, so all its requests share one.
    fn pixels(&self, side: f32, ppp: f32) -> (u32, u32) {
        let px = (side * ppp).round().max(1.0) as u32;
        let stroke_px = if svg_set::entry_stroke_width(self.entry, self.set).is_some() { (self.stroke * ppp * 64.0).round() as u32 } else { 0 };
        (px, stroke_px)
    }

    /// The icon's ink layer in white for a square of `side` points at `ppp`, its alpha from the drawing.
    pub(crate) fn mask(&self, side: f32, ppp: f32) -> egui::ColorImage {
        self.layer(side, ppp, svg_set::Layer::Ink)
    }

    /// The icon's `layer` for a square of `side` points at `ppp`.
    pub(crate) fn layer(&self, side: f32, ppp: f32, layer: svg_set::Layer) -> egui::ColorImage {
        let (px, stroke_px) = self.pixels(side, ppp);
        svg_set::rasterise_entry(self.entry, self.set, px, stroke_px as f32 / 64.0, self.mirrored, layer)
    }

    /// Paints every layer of the icon in `color` centred in `square`, on whole pixels: a one-ink silhouette (a
    /// cursor's icon).
    pub(crate) fn paint(&self, painter: &Painter, square: Rect, color: Color32) {
        self.paint_layers(painter, square, LayerInks::single(color));
    }

    /// Paints the icon centred in `square`, on whole pixels, each layer in its ink of `inks`. A catalog icon has only
    /// the ink layer; an app icon also paints its accent and fixed parts over it. A layer with no part is not painted.
    /// An app icon with a fill (`AppIcons::with_fills`) paints it under every layer, in the fill's own colour whatever
    /// the inks.
    pub(crate) fn paint_layers(&self, painter: &Painter, square: Rect, inks: LayerInks) {
        let ctx = painter.ctx();
        let ppp = ctx.pixels_per_point();
        let (px, stroke_px) = self.pixels(square.width(), ppp);
        let side = px as f32 / ppp;
        let min = (square.center() - vec2(side, side) / 2.0).round_to_pixels(ppp);
        let rect = Rect::from_min_size(min, vec2(side, side));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        if let Some((svg, [r, g, b, a])) = svg_set::fill_of(self.entry) {
            // The fill's key type keeps its texture apart from the layers'.
            let key = (self.entry, self.set, px, stroke_px, self.mirrored, true);
            let texture = TextureCache::texture(ctx, key, || svg_set::rasterise_fill(svg, px));
            let tint = Color32::from_rgba_unmultiplied(r, g, b, a);
            crate::runtime::egui::textures::paint(painter, texture, rect, uv, tint, egui::CornerRadius::ZERO);
        }
        let fixed = match inks.fixed {
            Some(color) => (svg_set::Layer::FixedMask, color),
            // design: allow[raw-colour/const](a fixed layer keeps the colours it is drawn in: a white tint leaves them unchanged)
            None => (svg_set::Layer::Fixed, Color32::WHITE),
        };
        let layers = [(svg_set::Layer::Ink, inks.ink), (svg_set::Layer::Accent, inks.accent), fixed];
        let count = if matches!(self.entry, svg_set::Entry::Catalog(_)) { 1 } else { layers.len() };
        for (layer, color) in layers.into_iter().take(count) {
            // Each layer is its own texture, so a disabled icon's one-ink fixed layer and the enabled one as drawn
            // are both kept.
            let key = (self.entry, self.set, px, stroke_px, self.mirrored, layer);
            let drawn = || {
                let image = self.layer(square.width(), ppp, layer);
                (!image.pixels.iter().all(|pixel| pixel.a() == 0)).then_some(image)
            };
            let Some(texture) = TextureCache::texture_if(ctx, key, drawn) else {
                continue;
            };
            crate::runtime::egui::textures::paint(painter, texture, rect, uv, color, egui::CornerRadius::ZERO);
        }
    }
}

/// The inks an icon's layers paint in: the ink layer, the accent layer, and the fixed layer (`None` paints it as
/// drawn).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayerInks {
    ink: Color32,
    accent: Color32,
    fixed: Option<Color32>,
}

impl LayerInks {
    /// Every layer in `color`.
    pub(crate) fn single(color: Color32) -> Self {
        LayerInks { ink: color, accent: color, fixed: Some(color) }
    }

    /// The inks of an icon painted in `color`: the accent parts in `colors.accent.solid` and the fixed parts as drawn,
    /// except that with `single_ink` (a disabled component) every layer paints in `color`, and with `on_dark` (a strong
    /// fill or scrim, where the accent colour would vanish) the accent parts paint in `color` and the fixed parts stay
    /// as drawn.
    pub(crate) fn of(tokens: &UiTokens, color: Color32, single_ink: bool, on_dark: bool) -> Self {
        if on_dark && !single_ink {
            return LayerInks { ink: color, accent: color, fixed: None };
        }
        if single_ink {
            return LayerInks::single(color);
        }
        LayerInks { ink: color, accent: crate::style::rgba_color(tokens.color.accent.solid), fixed: None }
    }
}

/// The stroke of a stroke-set icon `side` wide: `stroke.icon` of its side, so every glyph has the same weight.
fn stroke(tokens: &UiTokens, side: f32) -> f32 {
    tokens.stroke.icon * side
}
