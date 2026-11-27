//! `Draw`: what a component paints content with. It takes only `StyleColor` or `DataColor`.

use super::content::{DocRun, DocText, FaceRun};
use super::record::PaintRecord;
use super::rule::{RectSide, RuleKind};
use super::surface::Surface;
use super::text::{TextEnv, TextLayout, TextRun};
use crate::fonts::document::DocumentFaces;
use crate::fonts::stack_facts;
use crate::fonts::{FaceBytes, FaceState, FontRef};
use crate::style::{Corners, DataColor, Ink, StyleColor, SurfaceKind, SurfaceStyle, TypeRole, UiTokens, ink_color, rule_look, surface_style};
use craft_i18n::Direction;
use craft_theme::Srgba;
use egui::epaint::{CircleShape, RectShape};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureId, Vec2};
use std::cell::RefCell;

/// The "no colour" diagonal of a well stops this share of the well's height short of its edges.
const NONE_DIAGONAL_INSET: f32 = 0.15;

/// What a swatch well shows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WellFill {
    /// A colour.
    Color(Srgba),
    /// The mixed mark: the values differ.
    Mixed,
    /// The sunken "no colour" diagonal in `status.bad`.
    None,
    /// The sunken well with no mark: the base of a swatch icon.
    Empty,
}

impl From<Option<Srgba>> for WellFill {
    /// A colour, or the mixed mark for no colour of its own.
    fn from(color: Option<Srgba>) -> WellFill {
        color.map_or(WellFill::Mixed, WellFill::Color)
    }
}

/// A shape `Draw::fill` and `Draw::stroke` paint.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaintShape {
    /// A rect with corners.
    Rect(Rect, CornerRadius),
    /// A circle.
    Circle(Pos2, f32),
    /// A closed polygon, convex or not, whose edges don't cross.
    Polygon(Vec<Pos2>),
    /// An open path.
    Path(Vec<Pos2>),
}

impl PaintShape {
    /// Where `Draw::stroke` puts the shape's stroke.
    fn stroke_place(&self) -> StrokePlace {
        match self {
            PaintShape::Rect(..) => StrokePlace::Inside,
            PaintShape::Circle(..) => StrokePlace::Outside,
            PaintShape::Polygon(_) | PaintShape::Path(_) => StrokePlace::Centre,
        }
    }
}

/// Where a stroke lies against the edge of a closed shape (`Rect`, `Circle`, `Polygon`); an open `Path` always strokes
/// on its centre line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokePlace {
    /// Wholly inside the edge, so the shape keeps its outer size.
    Inside,
    /// Centred on the edge.
    Centre,
    /// Wholly outside the edge, so the stroke covers nothing the shape covers.
    Outside,
}

/// A line width from the theme, so content never picks a literal one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, crate::contract::MemoProps)]
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

impl DataStrokeWidth {
    /// The width with a document width multiplied by `scale` (a view's zoom); a theme width stays as the theme names
    /// it.
    pub fn scaled(self, scale: f32) -> Self {
        match self {
            DataStrokeWidth::Data(DataWidth(points)) => DataStrokeWidth::Data(DataWidth(points * scale)),
            named @ DataStrokeWidth::Named(_) => named,
        }
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

/// How a `Draw` paints colours from data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DataMode {
    /// As they are.
    Full,
    /// At `Opacity::DISABLED`, for a component that paints its Disabled state: the data can't be used now but is
    /// still what it is, so each colour keeps its hue.
    Dimmed,
    /// In the theme's disabled fill, for an inactive part (a switched-off clip): the data is off.
    Inactive,
}

impl DataMode {
    /// The mode of a component painting in `state`: `Dimmed` in its Disabled state, which step 12 gives a disabled
    /// component that is not faded as a whole.
    pub(crate) fn of(state: &crate::style::StateSet) -> Self {
        if state.is_disabled() { DataMode::Dimmed } else { DataMode::Full }
    }

    /// A colour from data as this mode paints it.
    pub(crate) fn color(self, tokens: &UiTokens, color: Color32) -> Color32 {
        self.map(crate::style::rgba_color(tokens.color.state.disabled_fill), color)
    }

    /// Maps every colour of `shape` (fills, strokes, mesh vertices, glyphs) as [`Self::color`] maps one.
    pub(crate) fn shape(self, tokens: &UiTokens, shape: &mut Shape) {
        if self == DataMode::Full {
            return;
        }
        let inactive = crate::style::rgba_color(tokens.color.state.disabled_fill);
        egui::epaint::shape_transform::adjust_colors(shape, move |color| *color = self.map(inactive, *color));
    }

    /// The one mapping, with `inactive` the theme's disabled fill. `Color32::PLACEHOLDER` passes through, so a
    /// galley's fallback ink still resolves when it is painted.
    fn map(self, inactive: Color32, color: Color32) -> Color32 {
        match self {
            _ if color == Color32::PLACEHOLDER => color,
            DataMode::Full => color,
            DataMode::Dimmed => color.gamma_multiply(craft_theme::Opacity::DISABLED),
            DataMode::Inactive => inactive,
        }
    }
}

/// The content-paint API `PaintUi::draw` gives.
pub struct Draw<'a> {
    painter: &'a Painter,
    record: &'a RefCell<PaintRecord>,
    env: DrawEnv<'a>,
    data: DataMode,
    /// The paragraph direction of text: the env's direction unless the caller sets the interface's.
    text_direction: Direction,
}

impl<'a> Draw<'a> {
    #[doc(hidden)]
    pub fn new(painter: &'a Painter, record: &'a RefCell<PaintRecord>, env: DrawEnv<'a>) -> Self {
        Draw { painter, record, env, data: DataMode::Full, text_direction: env.direction }
    }

    /// The same painter with `direction` as the paragraph direction of its text, while `env.direction` stays the
    /// flow that icons and surfaces mirror with.
    pub(crate) fn text_direction(mut self, direction: Direction) -> Self {
        self.text_direction = direction;
        self
    }

    /// The same painter painting data colours in `data`.
    pub(crate) fn data(mut self, data: DataMode) -> Self {
        self.data = data;
        self
    }

    /// The egui painter and the tokens this `Draw` paints with: what [`Glyph::paint`](super::Glyph::paint) takes.
    pub(crate) fn painter_and_tokens(&self) -> (&'a Painter, &'a UiTokens) {
        (self.painter, self.env.tokens)
    }

    fn text_env(&self) -> TextEnv<'a> {
        TextEnv {
            tokens: self.env.tokens,
            direction: self.text_direction,
            ink: ink_color(self.env.tokens, self.env.ink),
            fonts: stack_facts(self.painter.ctx()),
        }
    }

    /// A colour role of the scope's theme, for content that paints a state with no style of its own: a selected row in
    /// `draw.role(|c| c.state.selected)`, its text in `|c| c.text.on_selected`. It is the colour a style function
    /// gets from `cx.color(cx.tokens.color.…)` for the same role.
    pub fn role(&self, pick: impl FnOnce(&craft_theme::ColorTokens) -> craft_theme::Rgba) -> StyleColor {
        StyleColor(crate::style::rgba_color(pick(&self.env.tokens.color)))
    }

    #[doc(hidden)]
    pub fn width(&self, width: StrokeWidth) -> f32 {
        width.points(self.env.tokens, self.painter.pixels_per_point())
    }

    /// Lays `run` out in `rect`'s width and paints it placed in `rect`'s width by the run's alignment (Start at the
    /// reading start). A single line is centred vertically in a taller `rect`, so a label in a control or a row sits at
    /// the same place whatever the room; a wrapped run starts at the top. Records the elision and the first baseline.
    pub fn text(&self, rect: Rect, run: &TextRun<'_>) -> TextLayout {
        let (layout, at) = self.place_text(rect, run);
        self.galley(at, &layout);
        layout
    }

    /// `run` laid out in `rect`'s width, and where `text` paints it.
    pub(crate) fn place_text(&self, rect: Rect, run: &TextRun<'_>) -> (TextLayout, Pos2) {
        let layout = run.layout(self.painter.ctx(), &self.text_env(), rect.width());
        let x = self.text_x(rect, run, &layout);
        let spare = if layout.galley.rows.len() <= 1 { (rect.height() - layout.size().y).max(0.0) } else { 0.0 };
        (layout, Pos2::new(x, rect.top() + spare / 2.0))
    }

    /// Where `layout` of `run` starts in `rect`'s width: the free room shared out by the run's alignment, the reading
    /// start being the right edge in a right-to-left paragraph.
    pub fn text_x(&self, rect: Rect, run: &TextRun<'_>, layout: &TextLayout) -> f32 {
        let rtl = self.text_direction == Direction::Rtl && !run.ltr;
        rect.left() + (rect.width() - layout.size().x).max(0.0) * super::text::free_room_before(run.align, rtl)
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
        let x = self.text_x(rect, run, &layout);
        self.galley(Pos2::new(x, rect.center().y - letters.galley.mesh_bounds.center().y), &layout);
        layout
    }

    /// Paints a one-line `run` as `text` does, on the baseline of a line of `line` centred on the middle of `rect`'s
    /// height ([`line_baseline`](Self::line_baseline)): a field's unit, in a smaller role than the field's text, sits
    /// on the text's baseline.
    pub fn text_on_line(&self, rect: Rect, run: &TextRun<'_>, line: TypeRole) -> TextLayout {
        let layout = run.layout(self.painter.ctx(), &self.text_env(), rect.width());
        let x = self.text_x(rect, run, &layout);
        let baseline = self.line_baseline(rect.center().y, line);
        self.galley(Pos2::new(x, baseline - layout.first_baseline), &layout);
        layout
    }

    /// The baseline of a control's line of `role` whose centre line lies at `centre`: every text in the line sits on
    /// it, whatever its face or size (`text::line_baseline`).
    pub fn line_baseline(&self, centre: f32, role: TypeRole) -> f32 {
        super::text::line_baseline(self.painter.ctx(), &self.text_env(), role, centre)
    }

    /// Lays `text` out in `rect`'s width and paints it top aligned, its block placed in `rect` by `text.align` (Start
    /// and End swap sides under right-to-left). Each run keeps its own font, size and colour; a disabled component's
    /// runs paint at `Opacity::DISABLED`, as `data_fill` does.
    pub fn doc(&self, rect: Rect, text: &DocText<'_>) -> TextLayout {
        let text = DocText { wrap: Some(rect.width()), ..*text };
        let layout = text.layout(self.painter.ctx(), self.env.tokens, self.text_direction);
        let anchored = super::text::anchored_at_start(layout.galley.clone());
        let layout = layout.with_galley(anchored);
        let share = super::text::free_room_before(text.align, self.text_direction == Direction::Rtl);
        let x = rect.left() + (rect.width() - layout.size().x).max(0.0) * share;
        // The runs' colours are data.
        if self.data == DataMode::Dimmed {
            self.faded(craft_theme::Opacity::DISABLED, |draw| draw.galley(Pos2::new(x, rect.top()), &layout));
        } else {
            self.galley(Pos2::new(x, rect.top()), &layout);
        }
        layout
    }

    /// Paints `run` in its document face with the start of its first baseline at `start`, and returns its layout, whose
    /// top lies at `start.y - layout.first_baseline` ([`FaceRun`] says how the face, its fallback and the size are
    /// chosen). `load` gives the face's bytes and index, which the caller holds, the first time `run.key` is drawn while
    /// the face is kept; a `None` leaves the run in the interface face. The paragraph is left to right. A disabled
    /// component's run paints at `Opacity::DISABLED`, as `doc` does.
    pub fn face_run(&self, start: Pos2, run: &FaceRun<'_>, load: impl FnOnce() -> Option<FaceBytes>) -> TextLayout {
        let ctx = self.painter.ctx();
        let font = match DocumentFaces::over_interface(ctx, run.key, load) {
            FaceState::Ready(font) => font,
            _ => FontRef::proportional(),
        };
        let runs = [DocRun::new(run.text, font, run.drawn_size(ctx), run.color)];
        let layout = DocText::new(&runs).layout(ctx, self.env.tokens, Direction::Ltr);
        let at = Pos2::new(start.x, start.y - layout.first_baseline);
        if self.data == DataMode::Dimmed {
            self.faded(craft_theme::Opacity::DISABLED, |draw| draw.galley(at, &layout));
        } else {
            self.galley(at, &layout);
        }
        layout
    }

    /// Lays `run` out in `max_width` without painting it: the one text layout path for painters that measure before
    /// they paint (the same layout `text` paints with).
    pub fn layout(&self, run: &TextRun<'_>, max_width: f32) -> TextLayout {
        run.layout(self.painter.ctx(), &self.text_env(), max_width)
    }

    /// Records that the face dropped its label, so the component's tooltip text does not count it.
    pub fn drop_label(&self) {
        self.record.borrow_mut().label_dropped = true;
    }

    /// Paints a laid-out run at `pos`. Records the first baseline and, when the layout cut text, the full text, as
    /// `text` does.
    pub fn galley(&self, pos: Pos2, layout: &TextLayout) {
        let mut record = self.record.borrow_mut();
        record.baseline.get_or_insert(pos.y + layout.first_baseline);
        if layout.elided && record.elided.is_none() {
            record.elided = Some(layout.full().to_owned());
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
    /// directional, with its badge at the bottom-trailing corner.
    pub fn icon(&self, rect: Rect, icon: &super::IconSpec<'_>) {
        let color = icon.color_in(self.env.tokens);
        let side = rect.width().min(rect.height());
        let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
        match icon.source {
            super::IconSource::Swatch(swatch) => {
                // The icon's `None` is its own empty swatch (a mixed or theme-decided colour), so it keeps the dash.
                self.swatch_well(square, CornerRadius::ZERO, swatch.map_or(WellFill::Mixed, WellFill::Color));
                self.paint(&PaintShape::Rect(square, CornerRadius::ZERO), None, Some(Stroke::new(self.width(StrokeWidth::Thin), color)));
                return;
            }
            super::IconSource::Initial(letter) => {
                self.initial(rect, letter, StyleColor(color));
                return;
            }
            super::IconSource::Glyph(glyph) => glyph.paint_draw(self, square, color, super::icon::glyph_mirrored(glyph, icon, self.env.direction)),
            _ => super::icon::paint(
                self.painter,
                self.env.tokens,
                self.env.direction,
                rect,
                icon,
                self.data != DataMode::Full,
                self.env.ink == Ink::OnDark,
            ),
        }
        match icon.badge {
            Some(super::IconMark::Dot(badge)) => {
                let radius = self.env.tokens.size.handle.sm / 2.0;
                let centre = if self.env.direction == Direction::Rtl {
                    square.left_bottom() + Vec2::new(radius, -radius)
                } else {
                    square.right_bottom() - Vec2::splat(radius)
                };
                let ring = Stroke::new(self.width(StrokeWidth::Thin), color);
                self.paint(&PaintShape::Circle(centre, radius), Some(self.data_color(badge)), Some(ring));
            }
            Some(super::IconMark::Bar { color: bar, weight }) => {
                // The bar lies inside the square at its bottom, so it never leaves the icon's box.
                let thickness = weight.unwrap_or_else(|| self.width(StrokeWidth::Thick)).clamp(0.0, side);
                let strip = Rect::from_min_max(Pos2::new(square.min.x, square.max.y - thickness), square.max);
                self.data_fill(&PaintShape::Rect(strip, CornerRadius::ZERO), bar);
            }
            None => {}
        }
    }

    /// Paints `icon` as `icon` does, in the data colour `color` in place of the icon's own colour or ink: an icon drawn in
    /// document content. The colour paints as `data_fill` paints one (dimmed or inactive with the component).
    pub fn data_icon(&self, rect: Rect, icon: &super::IconSpec<'_>, color: DataColor) {
        self.icon(rect, &icon.clone().data_color(DataColor::new(self.data_color(color))));
    }

    /// `letter` in caption text centred in an outlined box that fills the square of `rect`'s shorter side.
    fn initial(&self, rect: Rect, letter: char, color: StyleColor) {
        let side = rect.width().min(rect.height());
        let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
        let radius = UiTokens::corner(self.env.tokens.shape.radius.sm);
        let width = self.width(StrokeWidth::Thin);
        self.paint(&PaintShape::Rect(square.shrink(width / 2.0), radius), None, Some(Stroke::new(width, color.egui())));
        let run = TextRun::new(letter.to_string()).role(TypeRole::Caption).color(color);
        let layout = self.layout(&run, f32::INFINITY);
        self.galley(square.center() - layout.size() / 2.0, &layout);
    }

    /// Paints the auto-scroll mark of `look` at `anchor`.
    pub fn auto_scroll_mark(&self, look: &crate::style::AutoScrollMarkLook, anchor: Pos2) {
        self.painter.extend(super::AutoScrollMark::shapes(look, anchor));
    }

    /// Paints the reset mark of `look` centred on `center`: the dot while the value `differs`, else the ring.
    pub fn reset_mark(&self, look: &crate::style::ResetMarkLook, center: Pos2, differs: bool) {
        self.painter.extend(super::ResetMark::shapes(look, center, differs));
    }

    /// Paints `marker` in `look`. A filled marker's fill is a data colour, painted as `data_fill` paints it.
    pub fn marker(&self, look: &crate::style::MarkerLook, marker: &super::Marker) {
        let marker = match marker.fill() {
            Some(color) => marker.with_fill(self.data_color(DataColor::new(color))),
            None => *marker,
        };
        self.painter.extend(marker.shapes(look));
    }

    /// Paints the line of `kind` from `from` to `to`.
    pub fn rule(&self, kind: RuleKind, from: Pos2, to: Pos2) {
        let look = rule_look(self.env.tokens, kind);
        self.painter.extend(super::rule::Rule::shapes(&look, from, to, self.painter.pixels_per_point()));
    }

    /// Paints the line of `kind` along the `side` edge of `rect`, on the physical pixels just inside it
    /// (`Rule::edge`): a row's bottom rule, a cell's gridline, a strip's edge toward its neighbour.
    pub fn edge_rule(&self, kind: RuleKind, rect: Rect, side: RectSide) {
        let look = rule_look(self.env.tokens, kind);
        self.painter.extend(super::rule::Rule::edge(&look, rect, side, self.painter.pixels_per_point()));
    }

    /// Paints the scroll bar in `track` along `axis` for `span`, in the bar's `state` (`ScrollBar::shapes`;
    /// `CxCore::scroll_bar_state` gives it).
    pub fn scroll_bar(
        &self,
        look: &crate::style::ScrollBarLook,
        track: Rect,
        axis: craft_behavior::Axis,
        rtl: bool,
        span: &craft_behavior::ScrollSpan,
        state: &crate::style::StateSet,
    ) {
        self.painter.extend(super::ScrollBar::shapes(look, track, axis, rtl, span, state));
    }

    /// The width in points of the line `rule` paints for `kind`.
    pub fn rule_width(&self, kind: RuleKind) -> f32 {
        rule_look(self.env.tokens, kind).width.points(self.painter.pixels_per_point())
    }

    /// Paints the line `look` from `from` to `to`, snapped to physical pixels as `rule` is: a rule in a component's own
    /// colour or width.
    pub fn rule_with(&self, look: &crate::style::RuleLook, from: Pos2, to: Pos2) {
        self.painter.extend(super::rule::Rule::shapes(look, from, to, self.painter.pixels_per_point()));
    }

    /// Paints the line `look` from `from` to `to` as dots `gap` points apart (`Rule::dotted`): a dotted line.
    pub fn dotted_rule_with(&self, look: &crate::style::RuleLook, from: Pos2, to: Pos2, gap: f32) {
        self.painter.extend(super::rule::Rule::dotted(look, from, to, gap, self.painter.pixels_per_point()));
    }

    /// Paints the line `look` from `from` to `to` as ticks `across` points long, `gap` points apart (`Rule::ticks`):
    /// a grip's row of marks.
    pub fn ticked_rule_with(&self, look: &crate::style::RuleLook, from: Pos2, to: Pos2, gap: f32, across: f32) {
        self.painter.extend(super::rule::Rule::ticks(look, from, to, gap, across, self.painter.pixels_per_point()));
    }

    /// Paints `count` upright ticks of `look` centred on `centre`, side by side on a level line, `gap` points apart and
    /// `across` points tall (`Rule::ticks`): a grip's row of marks, `count` of them at every display scale.
    pub fn grip_ticks(&self, look: &crate::style::RuleLook, centre: Pos2, count: u16, gap: f32, across: f32) {
        let ppp = self.painter.pixels_per_point().max(f32::EPSILON);
        let (side, gap_px) = (look.width.pixels(ppp), (gap * ppp).round().max(1.0));
        // The run `Rule::ticks` fits exactly `count` marks into: their sides, the gaps between them and the one free
        // pixel it keeps.
        let length = f32::from(count) * side + f32::from(count.saturating_sub(1)) * gap_px + 1.0;
        let half = egui::vec2(length / ppp / 2.0, 0.0);
        self.ticked_rule_with(look, centre - half, centre + half, gap, across);
    }

    /// Paints a surface of `kind` (tracks, plot wells).
    pub fn surface(&self, rect: Rect, kind: SurfaceKind, corners: Corners) {
        let style = surface_style(self.env.tokens, kind);
        let mut shapes = Surface::shapes(&style, None, rect, corners, self.env.direction, self.painter.pixels_per_point());
        super::fit_to_frame(self.painter.ctx(), self.painter.layer_id(), &mut shapes);
        self.painter.extend(shapes);
    }

    /// Paints the edge alone of a surface of `kind` over `rect`, with no shadow or fill: a floating window draws its
    /// edge again over its content, which covers it where it reaches the window's sides.
    pub fn surface_edge(&self, rect: Rect, kind: SurfaceKind) {
        let mut style = surface_style(self.env.tokens, kind);
        style.shadow = None;
        style.fill = egui::Color32::TRANSPARENT;
        self.painter.extend(Surface::shapes(&style, None, rect, Corners::All, self.env.direction, self.painter.pixels_per_point()));
    }

    /// Paints a surface of an already resolved `style` (a kind's look with one field changed, as a knob's round radius).
    pub fn surface_styled(&self, rect: Rect, style: &SurfaceStyle) {
        self.painter.extend(Surface::shapes(style, None, rect, Corners::All, self.env.direction, self.painter.pixels_per_point()));
    }

    /// Paints `look` over `rect` in `state`, as the paint step paints a component's surface: a part's own surface (an
    /// indicator's well) keeps the kind's edge and bevel.
    pub fn surface_look(&self, rect: Rect, look: &crate::style::SurfaceLook, state: &crate::style::StateSet) {
        self.painter.extend(surface_look_shapes(self.env.tokens, look, state, rect, self.env.direction, self.painter.pixels_per_point()));
    }

    /// Fills `shape`.
    pub fn fill(&self, shape: &PaintShape, color: StyleColor) {
        self.paint(shape, Some(color.egui()), None);
    }

    /// Strokes `shape` at a theme width or a width from the document, in a theme colour or a colour from data (a
    /// chart series, a curve). A data colour paints as `data_fill` paints it. A rect strokes inside its edge, a circle
    /// outside it and a polygon on it; [`stroke_placed`](Self::stroke_placed) picks the place.
    pub fn stroke(&self, shape: &PaintShape, width: impl Into<DataStrokeWidth>, color: impl Into<StrokeColor>) {
        self.stroke_placed(shape, shape.stroke_place(), width, color);
    }

    /// Strokes `shape` as [`stroke`](Self::stroke) does, with the stroke `place`d against the shape's edge: an outline
    /// around a selected item that must not cover it strokes `Outside`.
    pub fn stroke_placed(&self, shape: &PaintShape, place: StrokePlace, width: impl Into<DataStrokeWidth>, color: impl Into<StrokeColor>) {
        self.paint_placed(shape, None, Some(self.line(width.into(), color.into())), place);
    }

    /// Fills `shape` with a colour from data (a swatch well, a layer label dot, an avatar). A component that paints its
    /// Disabled state fills at `Opacity::DISABLED`, keeping the colour's hue; a `Draw` for an inactive part
    /// (`PaintUi::draw_part`) fills with the theme's disabled fill instead.
    pub fn data_fill(&self, shape: &PaintShape, color: DataColor) {
        let fill = self.data_color(color);
        self.paint(shape, Some(fill), None);
    }

    /// Fills the polygons `pieces`, which tile one area edge to edge, as one mesh with no feathered edges, in a colour
    /// from data as `data_fill` paints it. Feathered pieces would each fade out along the edges they share, so a
    /// translucent fill would show a seam of what lies beneath along every shared edge; unfeathered triangles meet
    /// with no gap and no overlap.
    pub fn data_fill_tiled(&self, pieces: &[Vec<Pos2>], color: DataColor) {
        let fill = self.data_color(color);
        let mut mesh = egui::Mesh::default();
        for piece in pieces {
            mesh.append(super::kit::fill_polygon(piece, fill, 0.0));
        }
        if !mesh.is_empty() {
            self.painter.add(Shape::mesh(mesh));
        }
    }

    /// Paints the part `uv` of `texture` (from `TextureCache` or a `TextureStore`) into `rect`, untinted. The picture
    /// is data: a disabled component paints it at `Opacity::DISABLED`, and a `Draw` for an inactive part fills `rect`
    /// with the theme's disabled fill instead, as `data_fill` does.
    pub fn texture(&self, rect: Rect, texture: TextureId, uv: Rect) {
        data_picture(self.painter, self.env.tokens, self.data, texture, rect, uv, CornerRadius::ZERO);
    }

    /// Paints `uv` of `texture` onto the clockwise quad `corners` (from `uv`'s top left), untinted, as data: a
    /// disabled component paints it at `Opacity::DISABLED`, and for an inactive part the quad is filled with the
    /// theme's disabled fill. A quad has square corners.
    pub fn texture_quad(&self, corners: [Pos2; 4], texture: TextureId, uv: Rect) {
        data_quad(self.painter, self.env.tokens, self.data, texture, corners, uv);
    }

    /// The texture for `key` from the `TextureCache` of the context this painter paints in, for [`texture`](Self::texture):
    /// `image()` is built on the first request only (see `TextureCache::texture`). A painter with no `egui::Context`
    /// of its own, such as a canvas tool's paint call, loads its pictures through here.
    pub fn cached_texture<K: std::hash::Hash + 'static>(&self, key: K, image: impl FnOnce() -> egui::ColorImage) -> TextureId {
        super::TextureCache::texture(self.painter.ctx(), key, image)
    }

    /// Paints `checker` over `rect` with corners `radius`, a light cell at the rect's top left. It shows through data,
    /// so it paints as the data over it: at `Opacity::DISABLED` for a disabled component, and for an inactive part as
    /// the theme's disabled fill, as `data_fill` does.
    pub fn checker(&self, checker: &super::Checker, rect: Rect, radius: CornerRadius) {
        data_checker(self.painter, self.env.tokens, self.data, checker, rect, radius);
    }

    /// Paints the well of a colour swatch over `well` with corners `radius`: the colour, over a checkerboard where it is
    /// translucent; else the empty well in `surface.sunken`, with a `text.muted` dash for `Mixed`, a `status.bad`
    /// diagonal from the bottom start to the top end for `None`, and nothing for `Empty`. An `Option<Srgba>` is a
    /// colour or, for `None` (no colour of its own, or a mixed one), the dash. `Swatch`, swatch icons and the paint
    /// chip paint their wells with it and add their own edges.
    pub fn swatch_well(&self, well: Rect, radius: CornerRadius, fill: impl Into<WellFill>) {
        self.well_of(&PaintShape::Rect(well, radius), well, fill.into(), |draw| draw.checker(&super::Checker::new(), well, radius));
    }

    /// [`Self::swatch_well`] over the convex polygon `corners` (a honeycomb cell): the same parts, the checkerboard and
    /// the marks cut to the polygon.
    pub fn swatch_polygon_well(&self, corners: &[Pos2], fill: impl Into<WellFill>) {
        let bounds = Rect::from_points(corners);
        self.well_of(&PaintShape::Polygon(corners.to_vec()), bounds, fill.into(), |draw| {
            data_polygon_checker(draw.painter, draw.env.tokens, draw.data, &super::Checker::new(), corners, bounds.min);
        });
    }

    /// The parts of a swatch well in `shape` within `bounds`: the colour over the checkerboard `checker` paints where it
    /// is translucent, or the empty well with its mark.
    fn well_of(&self, shape: &PaintShape, bounds: Rect, fill: WellFill, checker: impl FnOnce(&Self)) {
        let colors = &self.env.tokens.color;
        let sunken = Some(crate::style::rgba_color(colors.surface.sunken));
        match fill {
            WellFill::Color(color) => {
                if color.a < 1.0 {
                    checker(self);
                }
                self.data_fill(shape, DataColor::new(Color32::from(color)));
            }
            WellFill::Mixed => {
                self.paint(shape, sunken, None);
                let dash = Stroke::new(self.width(StrokeWidth::Medium), crate::style::rgba_color(colors.text.muted));
                self.paint(&super::glyph::dash(bounds), None, Some(dash));
            }
            WellFill::None => {
                self.paint(shape, sunken, None);
                let inner = bounds.shrink(bounds.height() * NONE_DIAGONAL_INSET);
                let (from, to) = if self.env.direction == Direction::Rtl {
                    (inner.right_bottom(), inner.left_top())
                } else {
                    (inner.left_bottom(), inner.right_top())
                };
                let mark = Stroke::new(self.width(StrokeWidth::Medium), crate::style::rgba_color(colors.status.bad.solid));
                self.paint(&PaintShape::Path(vec![from, to]), None, Some(mark));
            }
            WellFill::Empty => self.paint(shape, sunken, None),
        }
    }

    /// A data colour as `self.data` paints it.
    fn data_color(&self, color: DataColor) -> Color32 {
        self.data.color(self.env.tokens, color.0)
    }

    /// Strokes the open path `path` as dashes (copy marks, drag previews, reference lines), with the width and colour
    /// [`Self::stroke`] takes. `kit::dashed` is the one implementation; the kit painter's `dashed` calls it too.
    pub fn dashed(&self, path: &[Pos2], pattern: Dashes, width: impl Into<DataStrokeWidth>, color: impl Into<StrokeColor>) {
        let line = self.line(width.into(), color.into());
        self.painter.extend(super::kit::dashed(path, pattern, line.width, line.color));
    }

    /// The theme's dash pattern: `stroke.dash` dashes with `stroke.dash_gap` gaps.
    pub fn dashes(&self) -> Dashes {
        let stroke = &self.env.tokens.stroke;
        Dashes::new(stroke.dash, stroke.dash_gap)
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
        paint(&Draw { painter: &painter, record: self.record, env: self.env, data: self.data, text_direction: self.text_direction });
    }

    /// Runs `paint` with a `Draw` whose fills, strokes, text runs, icons and pictures are all `share` as opaque as
    /// they would be (1 paints as `self` does, 0 paints nothing). Fades stack: a faded `Draw` faded again multiplies
    /// the shares. The share is the app's runtime value, such as an animation's progress; a value outside 0 to 1 is
    /// clamped and a non-finite one leaves the opacity as it was.
    pub fn faded(&self, share: f32, paint: impl FnOnce(&Draw<'_>)) {
        let painter = faded_painter(self.painter, share);
        paint(&Draw { painter: &painter, record: self.record, env: self.env, data: self.data, text_direction: self.text_direction });
    }

    /// Paints `shape` with its stroke where [`stroke`](Self::stroke) puts it.
    fn paint(&self, shape: &PaintShape, fill: Option<Color32>, stroke: Option<Stroke>) {
        self.paint_placed(shape, fill, stroke, shape.stroke_place());
    }

    fn paint_placed(&self, shape: &PaintShape, fill: Option<Color32>, stroke: Option<Stroke>, place: StrokePlace) {
        let fill_color = fill.unwrap_or(Color32::TRANSPARENT);
        let stroke = stroke.unwrap_or(Stroke::NONE);
        let kind = match place {
            StrokePlace::Inside => StrokeKind::Inside,
            StrokePlace::Centre => StrokeKind::Middle,
            StrokePlace::Outside => StrokeKind::Outside,
        };
        let shape = match shape {
            PaintShape::Rect(rect, radius) => Shape::Rect(RectShape::new(*rect, *radius, fill_color, stroke, kind)),
            PaintShape::Circle(center, radius) => {
                // egui strokes a circle outside its radius, so the fill and the stroke paint as two circles when the
                // stroke moves in.
                let offset = match place {
                    StrokePlace::Inside => -stroke.width,
                    StrokePlace::Centre => -stroke.width / 2.0,
                    StrokePlace::Outside => 0.0,
                };
                if offset != 0.0 && fill.is_some() {
                    self.painter.add(Shape::Circle(CircleShape { center: *center, radius: *radius, fill: fill_color, stroke: Stroke::NONE }));
                }
                let fill_color = if offset == 0.0 { fill_color } else { Color32::TRANSPARENT };
                let radius = (*radius + offset).max(0.0);
                Shape::Circle(CircleShape { center: *center, radius, fill: fill_color, stroke })
            }
            PaintShape::Polygon(points) => {
                if fill.is_some() {
                    let feather = self.painter.ctx().tessellation_options(|options| {
                        if options.feathering { options.feathering_size_in_pixels / self.painter.pixels_per_point() } else { 0.0 }
                    });
                    let mesh = super::kit::fill_polygon(points, fill_color, feather);
                    if !mesh.is_empty() {
                        self.painter.add(Shape::mesh(mesh));
                    }
                }
                if stroke.is_empty() {
                    return;
                }
                let mut line = egui::epaint::PathStroke::from(stroke);
                line.kind = kind;
                Shape::closed_line(points.clone(), line)
            }
            PaintShape::Path(points) => Shape::line(points.clone(), stroke),
        };
        self.painter.add(shape);
    }
}

/// Paints the part `uv` of `texture` into `rect` with corners `radius` as data in `mode`: tinted as `mode` maps a
/// colour, or for an inactive part `rect` in the theme's disabled fill. `Draw::texture` and `ContentPainterCx::picture`
/// paint through it.
pub(super) fn data_picture(painter: &Painter, tokens: &UiTokens, mode: DataMode, texture: TextureId, rect: Rect, uv: Rect, radius: CornerRadius) {
    if mode == DataMode::Inactive {
        painter.rect_filled(rect, radius, mode.color(tokens, Color32::TRANSPARENT));
    } else {
        crate::runtime::egui::textures::paint(painter, texture, rect, uv, mode.color(tokens, super::textures::UNTINTED), radius);
    }
}

/// Paints `uv` of `texture` onto the clockwise quad `corners` as the data in `mode`: untinted through `paint_quad`,
/// or for an inactive part the quad filled with the disabled fill, as `data_picture` fills its rect.
fn data_quad(painter: &Painter, tokens: &UiTokens, mode: DataMode, texture: TextureId, corners: [Pos2; 4], uv: Rect) {
    if mode == DataMode::Inactive {
        let fill = mode.color(tokens, Color32::TRANSPARENT);
        painter.add(Shape::convex_polygon(corners.to_vec(), fill, Stroke::NONE));
    } else {
        crate::runtime::egui::textures::paint_quad(painter, texture, corners, uv, mode.color(tokens, super::textures::UNTINTED));
    }
}

/// Paints `checker` over `rect` with corners `radius`, a light cell at the rect's top left, as the data over it in
/// `mode`: mapped as `mode` maps a colour, or for an inactive part `rect` in the theme's disabled fill. `Draw::checker`
/// and `ContentPainterCx::checker_with` paint through it.
pub(super) fn data_checker(painter: &Painter, tokens: &UiTokens, mode: DataMode, checker: &super::Checker, rect: Rect, radius: CornerRadius) {
    if mode == DataMode::Inactive {
        painter.rect_filled(rect, radius, mode.color(tokens, Color32::TRANSPARENT));
    } else {
        let mut shape = checker.shape(painter.ctx(), tokens, super::CheckerArea::Rect(rect, radius), rect.min);
        mode.shape(tokens, &mut shape);
        painter.add(shape);
    }
}

/// The checkerboard over the convex polygon `corners`, as [`data_checker`] paints a rect: for an inactive `mode` the
/// polygon in the theme's disabled fill.
fn data_polygon_checker(painter: &Painter, tokens: &UiTokens, mode: DataMode, checker: &super::Checker, corners: &[Pos2], origin: Pos2) {
    if mode == DataMode::Inactive {
        painter.add(Shape::convex_polygon(corners.to_vec(), mode.color(tokens, Color32::TRANSPARENT), Stroke::NONE));
    } else {
        let mut shape = checker.polygon_shape(painter.ctx(), tokens, corners, origin);
        mode.shape(tokens, &mut shape);
        painter.add(shape);
    }
}

/// `painter` with its opacity multiplied by `share`: the one fade `Draw::faded` and `ContentPainterCx::faded` share.
pub(super) fn faded_painter(painter: &Painter, share: f32) -> Painter {
    let mut painter = painter.clone();
    painter.multiply_opacity(share);
    painter
}

/// The shapes of `look` over `rect` in `state`: the kind's surface (its edge and bevel) with the state fill, or the
/// fill alone without a kind, then the state border.
pub(crate) fn surface_look_shapes(
    tokens: &UiTokens,
    look: &crate::style::SurfaceLook,
    state: &crate::style::StateSet,
    rect: Rect,
    direction: Direction,
    ppp: f32,
) -> Vec<egui::Shape> {
    let fill = state.resolve(&look.fill);
    let radius = look.corners.apply(look.radius, direction);
    let mut shapes = match look.kind_in(state) {
        Some(kind) => {
            let mut style = surface_style(tokens, kind);
            // A look that names its own radius (a capsule button) shapes the kind's fill and edge too, so no plate with the kind's corners shows outside it.
            if look.radius != egui::CornerRadius::ZERO {
                style.radius = look.radius;
            }
            if look.border_replaces_edge && matches!(style.edge, crate::style::SurfaceEdge::Border(_)) {
                style.edge = crate::style::SurfaceEdge::None;
            }
            Surface::shapes(&style, Some(fill.egui()), rect, look.corners, direction, ppp)
        }
        None => super::StateLayer::shapes(super::StateLayerKind::Fill, fill, rect, radius, 0.0),
    };
    if let Some(border) = &look.border {
        shapes.extend(super::StateLayer::shapes(super::StateLayerKind::Outline, state.resolve(border), rect, radius, look.border_width));
    }
    if let Some(inner) = look.inner.as_ref().map(|inner| state.resolve(inner)).filter(|inner| inner.egui().a() > 0) {
        let inset = look.border_width;
        let shrink = |r: u8| (f32::from(r) - inset).max(0.0).round() as u8;
        let radius = CornerRadius { nw: shrink(radius.nw), ne: shrink(radius.ne), sw: shrink(radius.sw), se: shrink(radius.se) };
        shapes.extend(super::StateLayer::shapes(super::StateLayerKind::Outline, inner, rect.shrink(inset), radius, look.border_width));
    }
    shapes
}
