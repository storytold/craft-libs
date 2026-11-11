//! `Draw`: what a component paints content with. It takes only `StyleColor` or `DataColor`.

use super::content::DocText;
use super::record::PaintRecord;
use super::rule::RuleKind;
use super::surface::Surface;
use super::text::{TextEnv, TextLayout, TextRun};
use crate::fonts::stack_facts;
use crate::style::{Corners, DataColor, Ink, StyleColor, SurfaceKind, SurfaceStyle, UiTokens, ink_color, rule_look, surface_style};
use craft_i18n::Direction;
use egui::epaint::{CircleShape, RectShape};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureId, Vec2};
use std::cell::RefCell;

/// A shape `Draw::fill` and `Draw::stroke` paint.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaintShape {
    /// A rect with corners.
    Rect(Rect, CornerRadius),
    /// A circle.
    Circle(Pos2, f32),
    /// A convex polygon.
    Polygon(Vec<Pos2>),
    /// An open path.
    Path(Vec<Pos2>),
}

/// A line width from the theme, so content never picks a literal one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StrokeWidth {
    /// One physical pixel.
    Hairline,
    /// `stroke.thin`.
    Thin,
    /// `stroke.medium`.
    Medium,
    /// `stroke.thick`.
    Thick,
    /// `stroke.heavy`.
    Heavy,
}

impl StrokeWidth {
    /// The width in points on a display of `pixels_per_point`: the one place a named width becomes a number.
    #[doc(hidden)]
    pub fn points(self, tokens: &UiTokens, pixels_per_point: f32) -> f32 {
        let stroke = &tokens.stroke;
        match self {
            StrokeWidth::Hairline => 1.0 / pixels_per_point.max(f32::EPSILON),
            StrokeWidth::Thin => stroke.thin,
            StrokeWidth::Medium => stroke.medium,
            StrokeWidth::Thick => stroke.thick,
            StrokeWidth::Heavy => stroke.heavy,
        }
    }
}

/// A line width the data gives (a path's thickness, a pen size), in screen points: the caller maps a document width
/// through its view first. `Draw::stroke` and `Draw::dashed` take one beside the named [`StrokeWidth`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataWidth(f32);

impl DataWidth {
    /// A width of `points`.
    pub const fn points(points: f32) -> Self {
        DataWidth(points)
    }
}

/// The width of a stroke: a named theme width or one from the document.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DataStrokeWidth {
    /// A width the theme names.
    Named(StrokeWidth),
    /// A width the document gives.
    Data(DataWidth),
}

impl From<StrokeWidth> for DataStrokeWidth {
    fn from(width: StrokeWidth) -> Self {
        DataStrokeWidth::Named(width)
    }
}

impl From<DataWidth> for DataStrokeWidth {
    fn from(width: DataWidth) -> Self {
        DataStrokeWidth::Data(width)
    }
}

/// The colour of a stroke: a theme colour or one from the data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum StrokeColor {
    /// A colour the theme gives.
    Theme(StyleColor),
    /// A colour the data gives (a chart series, a pen colour).
    Data(DataColor),
}

impl From<StyleColor> for StrokeColor {
    fn from(color: StyleColor) -> Self {
        StrokeColor::Theme(color)
    }
}

impl From<DataColor> for StrokeColor {
    fn from(color: DataColor) -> Self {
        StrokeColor::Data(color)
    }
}

/// The lengths of a dashed stroke, in points along the path.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Dashes {
    /// The length of one dash.
    pub dash: f32,
    /// The length of the gap after a dash.
    pub gap: f32,
    /// How far along the pattern the path starts (animates marching ants).
    pub phase: f32,
}

impl Dashes {
    /// Dashes of `dash` with gaps of `gap`, starting at the pattern's beginning.
    pub fn new(dash: f32, gap: f32) -> Self {
        Dashes { dash, gap, phase: 0.0 }
    }

    /// Dashes of `length` with gaps of the same length, starting at the pattern's beginning.
    pub fn even(length: f32) -> Self {
        Dashes { dash: length, gap: length, phase: 0.0 }
    }

    /// The pattern advanced by `phase` points.
    pub fn phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }
}

/// What a `Draw` reads from the env of the component it paints for.
#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct DrawEnv<'a> {
    pub tokens: &'a UiTokens,
    pub direction: Direction,
    pub ink: Ink,
}

/// The content-paint API `PaintUi::draw` gives.
pub struct Draw<'a> {
    painter: &'a Painter,
    record: &'a RefCell<PaintRecord>,
    env: DrawEnv<'a>,
    /// Painting for an inactive part: data fills and strokes take the theme's disabled fill.
    inactive: bool,
    /// The paragraph direction of text: the env's direction unless the caller sets the interface's.
    text_direction: Direction,
}

impl<'a> Draw<'a> {
    #[doc(hidden)]
    pub fn new(painter: &'a Painter, record: &'a RefCell<PaintRecord>, env: DrawEnv<'a>) -> Self {
        Draw { painter, record, env, inactive: false, text_direction: env.direction }
    }

    /// The same painter with `direction` as the paragraph direction of its text, while `env.direction` stays the
    /// flow that icons and surfaces mirror with.
    pub(crate) fn text_direction(mut self, direction: Direction) -> Self {
        self.text_direction = direction;
        self
    }

    /// The same painter for a part that is `inactive`.
    pub(crate) fn for_part(mut self, inactive: bool) -> Self {
        self.inactive = inactive;
        self
    }

    fn text_env(&self) -> TextEnv<'a> {
        TextEnv {
            tokens: self.env.tokens,
            direction: self.text_direction,
            ink: ink_color(self.env.tokens, self.env.ink),
            fonts: stack_facts(self.painter.ctx()),
        }
    }

    #[doc(hidden)]
    pub fn width(&self, width: StrokeWidth) -> f32 {
        width.points(self.env.tokens, self.painter.pixels_per_point())
    }

    /// Lays `run` out in `rect`'s width and paints it at the reading start of `rect`. A single line is centred
    /// vertically in a taller `rect`, so a label in a control or a row sits at the same place whatever the room; a
    /// wrapped run starts at the top. Records the elision and the first baseline.
    pub fn text(&self, rect: Rect, run: &TextRun<'_>) -> TextLayout {
        let layout = run.layout(self.painter.ctx(), &self.text_env(), rect.width());
        let at_end = self.text_direction == Direction::Rtl && !run.ltr;
        let x = if at_end { rect.right() - layout.size().x } else { rect.left() };
        let spare = if layout.galley.rows.len() <= 1 { (rect.height() - layout.size().y).max(0.0) } else { 0.0 };
        self.galley(Pos2::new(x, rect.top() + spare / 2.0), &layout);
        layout
    }

    /// Paints a one-line `run` as `text` does, with the middle of its lowercase letters (the x-height) on the middle
    /// of `rect`'s height. A chevron or icon centred on the same rect then sits on the text's centre line.
    pub fn text_on_centre_line(&self, rect: Rect, run: &TextRun<'_>) -> TextLayout {
        let layout = run.layout(self.painter.ctx(), &self.text_env(), rect.width());
        let letters = TextRun { spans: vec![super::text::Span::new("x")], rotation: 0.0, ..run.clone() }.layout(
            self.painter.ctx(),
            &self.text_env(),
            f32::INFINITY,
        );
        let at_end = self.text_direction == Direction::Rtl && !run.ltr;
        let x = if at_end { rect.right() - layout.size().x } else { rect.left() };
        self.galley(Pos2::new(x, rect.center().y - letters.galley.mesh_bounds.center().y), &layout);
        layout
    }

    /// Lays `text` out in `rect`'s width and paints it top aligned, its block placed in `rect` by `text.align` (Start
    /// and End swap sides under right-to-left). Each run keeps its own font, size and colour.
    pub fn doc(&self, rect: Rect, text: &DocText<'_>) -> TextLayout {
        let text = DocText { wrap: Some(rect.width()), ..*text };
        let layout = text.layout(self.painter.ctx(), self.env.tokens, self.text_direction);
        let layout = TextLayout { galley: super::text::anchored_at_start(layout.galley.clone()), ..layout };
        let share = super::text::free_room_before(text.align, self.text_direction == Direction::Rtl);
        let x = rect.left() + (rect.width() - layout.size().x).max(0.0) * share;
        self.galley(Pos2::new(x, rect.top()), &layout);
        layout
    }

    /// Lays `run` out in `max_width` without painting it: the one text layout path for painters that measure before
    /// they paint (the same layout `text` paints with).
    pub fn layout(&self, run: &TextRun<'_>, max_width: f32) -> TextLayout {
        run.layout(self.painter.ctx(), &self.text_env(), max_width)
    }

    /// Paints a laid-out run at `pos`. Records the first baseline and, when the layout cut text, the full text, as
    /// `text` does.
    pub fn galley(&self, pos: Pos2, layout: &TextLayout) {
        let mut record = self.record.borrow_mut();
        record.baseline.get_or_insert(pos.y + layout.first_baseline);
        if layout.elided && record.elided.is_none() {
            record.elided = Some(layout.full.clone());
        }
        drop(record);
        super::text::note_painted(self.painter.ctx(), layout);
        // A run cut to nothing (an elided label in no room) has no glyphs, and egui gives it NaN bounds.
        if !layout.galley.mesh_bounds.is_finite() {
            return;
        }
        let ink = ink_color(self.env.tokens, self.env.ink);
        if layout.rotation == 0.0 {
            self.painter.galley(pos, layout.galley.clone(), ink);
        } else {
            let shape = egui::epaint::TextShape::new(pos, layout.galley.clone(), ink).with_angle_and_anchor(layout.rotation, egui::Align2::LEFT_TOP);
            self.painter.add(shape);
        }
    }

    /// Paints `icon` centred in `rect`, in the colour or ink the spec names, mirrored under right-to-left when it is
    /// directional.
    pub fn icon(&self, rect: Rect, icon: &super::IconSpec<'_>) {
        if let super::IconSource::Swatch(color) = &icon.source {
            let side = rect.width().min(rect.height());
            let shape = PaintShape::Rect(Rect::from_center_size(rect.center(), Vec2::splat(side)), CornerRadius::ZERO);
            let edge = icon.color.map_or_else(|| ink_color(self.env.tokens, icon.ink), StyleColor::egui);
            self.data_fill(&shape, DataColor::new(Color32::from(*color)));
            self.paint(&shape, None, Some(Stroke::new(self.width(StrokeWidth::Thin), edge)));
            return;
        }
        super::icon::paint(self.painter, self.env.tokens, self.env.direction, rect, icon);
    }

    /// Paints the reset dot of `look` centred on `center`.
    pub fn reset_mark(&self, look: &crate::style::ResetMarkLook, center: Pos2) {
        self.painter.extend(super::ResetMark::shapes(look, center));
    }

    /// `v` moved to the centre of the physical pixel it lies in: a line one physical pixel wide placed there fills that
    /// pixel, and on a pixel edge it would split into two paler ones.
    pub fn pixel_centre(&self, v: f32) -> f32 {
        let ppp = self.painter.pixels_per_point();
        ((v * ppp).floor() + 0.5) / ppp
    }

    /// Paints the line of `kind` from `from` to `to`.
    pub fn rule(&self, kind: RuleKind, from: Pos2, to: Pos2) {
        let look = rule_look(self.env.tokens, kind);
        self.painter.extend(super::rule::Rule::shapes(&look, from, to, self.painter.pixels_per_point()));
    }

    /// Paints a line of theme width `width` and `color` from `from` to `to`, snapped to physical pixels as `rule` is.
    #[doc(hidden)]
    pub fn line_snapped(&self, width: StrokeWidth, color: Color32, from: Pos2, to: Pos2) {
        let look = crate::style::RuleLook { color, width: self.width(width) };
        self.painter.extend(super::rule::Rule::shapes(&look, from, to, self.painter.pixels_per_point()));
    }

    /// Paints a surface of `kind` (tracks, plot wells).
    pub fn surface(&self, rect: Rect, kind: SurfaceKind, corners: Corners) {
        let style = surface_style(self.env.tokens, kind);
        self.painter.extend(Surface::shapes(&style, None, rect, corners, self.env.direction, self.painter.pixels_per_point()));
    }

    /// Paints a surface of an already resolved `style` (a kind's look with one field changed, as a knob's round radius).
    pub fn surface_styled(&self, rect: Rect, style: &SurfaceStyle) {
        self.painter.extend(Surface::shapes(style, None, rect, Corners::All, self.env.direction, self.painter.pixels_per_point()));
    }

    /// Fills `shape`.
    pub fn fill(&self, shape: &PaintShape, color: StyleColor) {
        self.paint(shape, Some(color.egui()), None);
    }

    /// Strokes `shape` at a theme width or a width from the document, in a theme colour or a colour from data (a
    /// chart series, a curve). A `Draw` for an inactive part strokes a data colour with the theme's disabled fill
    /// instead.
    pub fn stroke(&self, shape: &PaintShape, width: impl Into<DataStrokeWidth>, color: impl Into<StrokeColor>) {
        self.paint(shape, None, Some(self.line(width.into(), color.into())));
    }

    /// Fills `shape` with a colour from data (a swatch well, a layer label dot, an avatar). A `Draw` for an inactive
    /// part (`PaintUi::draw_part`) fills with the theme's disabled fill instead.
    pub fn data_fill(&self, shape: &PaintShape, color: DataColor) {
        let fill = self.data_color(color);
        self.paint(shape, Some(fill), None);
    }

    /// Paints the part `uv` of `texture` (from `TextureCache` or a `TextureStore`) into `rect`, untinted. A `Draw`
    /// for an inactive part fills `rect` with the theme's disabled fill instead, as `data_fill` does.
    pub fn texture(&self, rect: Rect, texture: TextureId, uv: Rect) {
        if self.inactive {
            self.data_fill(&PaintShape::Rect(rect, CornerRadius::ZERO), DataColor::new(Color32::TRANSPARENT));
        } else {
            self.painter.image(texture, rect, uv, super::textures::UNTINTED);
        }
    }

    fn data_color(&self, color: DataColor) -> Color32 {
        if self.inactive { crate::style::rgba_color(self.env.tokens.color.state.disabled_fill) } else { color.0 }
    }

    /// Strokes the open path `path` as dashes (copy marks, drag previews, reference lines), with the width and colour
    /// [`Self::stroke`] takes. [`kit::dashed`] is the one implementation; the kit painter's `dashed` calls it too.
    pub fn dashed(&self, path: &[Pos2], pattern: Dashes, width: impl Into<DataStrokeWidth>, color: impl Into<StrokeColor>) {
        let line = self.line(width.into(), color.into());
        self.painter.extend(super::kit::dashed(path, pattern, line.width, line.color));
    }

    /// The width in points and colour a stroke of `width` and `color` paints with.
    fn line(&self, width: DataStrokeWidth, color: StrokeColor) -> Stroke {
        let points = match width {
            DataStrokeWidth::Named(width) => self.width(width),
            DataStrokeWidth::Data(DataWidth(points)) => points,
        };
        let color = match color {
            StrokeColor::Theme(color) => color.egui(),
            StrokeColor::Data(color) => self.data_color(color),
        };
        Stroke::new(points, color)
    }

    /// Runs `paint` with a `Draw` whose painting is cut to `rect` (inside the clip already in force): scrolled content
    /// under a frozen band, a bracket cut at a lane's edge.
    pub fn clipped(&self, rect: Rect, paint: impl FnOnce(&Draw<'_>)) {
        let painter = self.painter.with_clip_rect(rect);
        paint(&Draw { painter: &painter, record: self.record, env: self.env, inactive: self.inactive, text_direction: self.text_direction });
    }

    /// Runs `paint` with a `Draw` whose fills, strokes, text runs, icons and pictures are all `share` as opaque as
    /// they would be (1 paints as `self` does, 0 paints nothing). Fades stack: a faded `Draw` faded again multiplies
    /// the shares. The share is the app's runtime value, such as an animation's progress; a value outside 0 to 1 is
    /// clamped and a non-finite one leaves the opacity as it was.
    pub fn faded(&self, share: f32, paint: impl FnOnce(&Draw<'_>)) {
        let painter = faded_painter(self.painter, share);
        paint(&Draw { painter: &painter, record: self.record, env: self.env, inactive: self.inactive, text_direction: self.text_direction });
    }

    fn paint(&self, shape: &PaintShape, fill: Option<Color32>, stroke: Option<Stroke>) {
        let fill_color = fill.unwrap_or(Color32::TRANSPARENT);
        let stroke = stroke.unwrap_or(Stroke::NONE);
        let shape = match shape {
            PaintShape::Rect(rect, radius) => Shape::Rect(RectShape::new(*rect, *radius, fill_color, stroke, StrokeKind::Inside)),
            PaintShape::Circle(center, radius) => Shape::Circle(CircleShape { center: *center, radius: *radius, fill: fill_color, stroke }),
            PaintShape::Polygon(points) => Shape::convex_polygon(points.clone(), fill_color, stroke),
            PaintShape::Path(points) => Shape::line(points.clone(), stroke),
        };
        self.painter.add(shape);
    }
}

/// `painter` with its opacity multiplied by `share`: the one fade `Draw::faded` and `ContentPainterCx::faded` share.
pub(super) fn faded_painter(painter: &Painter, share: f32) -> Painter {
    let mut painter = painter.clone();
    painter.multiply_opacity(share);
    painter
}
