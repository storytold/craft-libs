//! `IconSpec`: an icon to paint, and the SVG set it can name.

use super::svg_set;
use super::textures::TextureCache;
use crate::style::{DataColor, Ink, StyleColor, UiTokens, ink_color};
use craft_behavior::IconId;
use craft_i18n::Direction;
use craft_theme::Srgba;
use egui::emath::GuiRounding as _;
use egui::{Color32, Painter, Rect, pos2, vec2};

pub use super::svg_set::IconSet;

/// Whether a directional icon flips under right-to-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mirror {
    /// Flips a directional icon under right-to-left.
    #[default]
    Auto,
    /// Never flips.
    Never,
}

/// A value glyph: an icon drawn from a value. It is `Sync` because container props that show one are kept with a
/// body's measurement in layout memory.
pub trait Glyph: Sync {
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
    Glyph(&'a dyn Glyph),
    /// A colour from data, painted as a swatch (a guide colour, a matte colour); `None` paints the empty swatch of a
    /// colour the theme decides.
    Swatch(Option<Srgba>),
    /// A filled circle as wide as the icon's square, in the spec's colour or ink (`IconSpec::data_color` for a colour
    /// from data).
    Disc,
    /// A letter in an outlined box, in the spec's colour or ink: the icon of something that has none, from the first
    /// letter of its name. Painted by [`Draw::icon`](super::Draw::icon) as text, in the theme's fonts.
    Initial(char),
}

impl std::fmt::Debug for IconSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IconSource::Svg(id) => f.debug_tuple("Svg").field(id).finish(),
            IconSource::Glyph(_) => f.write_str("Glyph(..)"),
            IconSource::Swatch(color) => f.debug_tuple("Swatch").field(color).finish(),
            IconSource::Disc => f.write_str("Disc"),
            IconSource::Initial(letter) => f.debug_tuple("Initial").field(letter).finish(),
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
            _ => false,
        }
    }
}

/// An icon to paint.
#[derive(Clone, Debug, PartialEq)]
pub struct IconSpec<'a> {
    /// The drawing.
    pub source: IconSource<'a>,
    /// The ink the icon reads in.
    pub ink: Ink,
    /// A colour a style resolved from the state set (hover, disabled, tone); it replaces the ink's colour.
    pub color: Option<StyleColor>,
    /// Mirroring under right-to-left.
    pub mirror: Mirror,
    /// The set an SVG icon is drawn from.
    pub set: IconSet,
    /// A dot of a colour from data at the icon's bottom-trailing corner; see [`badge`](Self::badge).
    pub badge: Option<DataColor>,
}

impl<'a> IconSpec<'a> {
    /// An icon of the SVG set or a tool icon, in the panel ink.
    pub fn svg(id: IconId) -> Self {
        IconSpec { source: IconSource::Svg(id), ink: Ink::OnPanel, color: None, mirror: Mirror::Auto, set: IconSet::default(), badge: None }
    }

    /// An icon of `glyph` in the panel ink.
    pub fn glyph(glyph: &'a dyn Glyph) -> Self {
        IconSpec { source: IconSource::Glyph(glyph), ink: Ink::OnPanel, color: None, mirror: Mirror::Auto, set: IconSet::default(), badge: None }
    }

    /// A swatch of `color`, painted as [`Draw::swatch_well`](super::Draw::swatch_well) paints a well.
    pub fn swatch(color: Srgba) -> Self {
        IconSpec {
            source: IconSource::Swatch(Some(color)),
            ink: Ink::OnPanel,
            color: None,
            mirror: Mirror::Never,
            set: IconSet::default(),
            badge: None,
        }
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
        IconSpec { source: IconSource::Initial(letter), ink: Ink::OnPanel, color: None, mirror: Mirror::Never, set: IconSet::default(), badge: None }
    }

    /// A filled circle in the panel ink, or in the colour [`color`](Self::color) or [`data_color`](Self::data_color)
    /// sets (a status dot, a person's dot).
    pub fn disc() -> Self {
        IconSpec { source: IconSource::Disc, ink: Ink::OnPanel, color: None, mirror: Mirror::Never, set: IconSet::default(), badge: None }
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

    /// Sets the set an SVG icon is drawn from.
    pub fn set(mut self, set: IconSet) -> Self {
        self.set = set;
        self
    }

    /// Adds a dot of `color` at the bottom-trailing corner of an SVG or glyph icon, ringed in the icon's colour: the
    /// tone a point picker sets. The dot is a data colour (`Draw::data_fill`); the ring follows the icon's colour.
    pub fn badge(mut self, color: DataColor) -> Self {
        self.badge = Some(color);
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

/// Whether the catalog has an icon named `id`.
pub fn has_icon(id: &IconId) -> bool {
    svg_set::find(id.name()).is_some()
}

/// The names of the catalog's icons, sorted.
pub fn icon_names() -> &'static [&'static str] {
    svg_set::names()
}

/// Paints `spec`'s glyph centred in the square of `rect`'s shorter side; `Draw::icon` adds the badge.
pub(crate) fn paint(painter: &Painter, tokens: &UiTokens, direction: Direction, rect: Rect, spec: &IconSpec<'_>) {
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
            let Some(index) = svg_set::find(id.name()) else {
                log::error!("icon {:?} is not in the catalog", id.name());
                return;
            };
            let mirrored = rtl && svg_set::directional(index) && spec.mirror == Mirror::Auto;
            svg(painter, tokens, square, index, spec.set, mirrored, color);
        }
    }
}

fn svg(painter: &Painter, tokens: &UiTokens, square: Rect, index: usize, set: IconSet, mirrored: bool, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (square.width() * ppp).round().max(1.0) as u32;
    // Fill sets ignore the stroke, so all their requests share one entry.
    let stroke_px = if set.stroke_width().is_some() { (stroke(tokens, square.width()) * ppp * 64.0).round() as u32 } else { 0 };
    let key = (index, set, px, stroke_px, mirrored);
    let texture = TextureCache::texture(ctx, key, || svg_set::rasterise(index, set, px, stroke_px as f32 / 64.0, mirrored));
    let side = px as f32 / ppp;
    let min = (square.center() - vec2(side, side) / 2.0).round_to_pixels(ppp);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    crate::runtime::egui::textures::paint(painter, texture, Rect::from_min_size(min, vec2(side, side)), uv, color, egui::CornerRadius::ZERO);
}

/// The stroke of a stroke-set icon `side` wide: `stroke.thin` below `size.icon.md`, `stroke.medium` from it up, so a
/// glyph inside a control keeps about the share of its side that a standard icon's stroke has.
fn stroke(tokens: &UiTokens, side: f32) -> f32 {
    if side < tokens.size.icon.md { tokens.stroke.thin } else { tokens.stroke.medium }
}
