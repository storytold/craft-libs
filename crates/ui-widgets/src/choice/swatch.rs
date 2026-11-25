//! `Swatch`: a colour well. It shows a colour from data, a mixed look for several colours, a before and after pair, the
//! hue wheel, a picture or an empty well, and reports the activation. It edits nothing: the host opens its picker on a
//! click, or on the `ColorRequest` of a `picker` swatch.

use super::radio::RadioKind;
use super::words::ChoiceWords;
use crate::contract::{
    Access, Activation, Common, Component, Cx, DisabledLook, GroupHandle, MeasureCx, Meta as _, Outcome, PaintUi, Pointer as _, Props as _, Role,
    Sizing, Styled as _, WordSetCx as _,
};
use crate::primitives::textures::UNTINTED;
use crate::primitives::{Checker, ContentPainterCx, DataWidth, Draw, FacePaint, IconSpec, PaintShape, StrokeWidth, TextureCache, WellFill};
use crate::runtime::focus::FocusShape;
use crate::runtime::requests::ColorRequest;
use crate::style::{ComponentStyle, DataColor, Flags, FocusLook, State, StateMix, StateSet, StyleColor, StyleCx, UiTokens, resolve_style};
use craft_behavior::{IconId, Metrics, Mixed, Sense, Value};
use craft_theme::{Hsb, Rgba, Srgba, Stateful, States, over};
use egui::{Color32, ColorImage, CornerRadius, Mesh, Pos2, Rect, Response, Shape, Vec2, pos2};
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// Opacity of the black or white ring that stands out from a well's colour.
const INNER_EDGE_ALPHA: u8 = 0x80;

/// The points a hue wheel takes along each edge of its well's outline: enough that the colours between them blend
/// into a smooth wheel at any well size.
const SPECTRUM_STEPS: usize = 18;

/// The clear colour: an empty well shows only the checkerboard behind it.
const CLEAR: Srgba = Srgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// The sizes of a swatch at a control size: what a composition that lays out wells reads.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SwatchMetrics {
    /// The side of a square well: the control height.
    pub side: f32,
    /// The side of a cell of a table of touching swatches.
    pub cell: f32,
    /// The corners of a well: `shape.radius.sm`.
    pub radius: CornerRadius,
}

impl SwatchMetrics {
    /// The metrics at the control size of `cx`.
    pub fn of(cx: &StyleCx<'_>) -> SwatchMetrics {
        let tokens = cx.tokens;
        SwatchMetrics { side: cx.control().height, cell: tokens.size.icon.md, radius: UiTokens::corner(tokens.shape.radius.sm) }
    }
}

/// The style of a swatch.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SwatchStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The side of the square well, or of the cell.
    side: f32,
    /// The well's corners; square in a cell.
    radius: CornerRadius,
    /// The ring on the well's outline, which separates the well from the panel.
    edge: Stateful<StyleColor>,
    /// The ring on the well's outline: the accent for a selected or checked well, the focus colour on the lead cell of
    /// a table (a member runs no focus step of its own, so its outline carries the ring); clear while it is off.
    ring: Stateful<StyleColor>,
    /// The thin edge just inside the outline of a marked well, the ink of the cell marks, and the glyph of an icon cell.
    mark: StyleColor,
    /// The extent of a corner or centre cell mark: `size.icon.sm`.
    mark_size: f32,
    /// The side of an icon cell's glyph: `size.icon.sm`, cut to the well.
    glyph: f32,
    /// The checkerboard behind a translucent colour: the colours of its corners and its other cells.
    checker: [Rgba; 2],
    /// The mixed well's colour, which `inner_edge` contrasts with.
    mixed_rgba: Rgba,
}

impl SwatchStyle {
    /// A ring of black or white at partial alpha that stands out from `color`, or from the mixed well for `None`: for
    /// a composition that paints a well apart from its colours.
    pub fn inner_edge(&self, color: Option<Srgba>) -> DataColor {
        let color = color.map_or(self.mixed_rgba, Srgba::to_rgba);
        let shown = self.checker.map(|behind| over(color, behind));
        let ring = |base: Rgba| base.with_alpha(INNER_EDGE_ALPHA);
        let least = |base: Rgba| shown.iter().map(|&s| craft_theme::contrast_ratio(over(ring(base), s), s)).fold(f64::INFINITY, f64::min);
        let (black, white) = (Srgba::BLACK.to_rgba(), Srgba::WHITE.to_rgba());
        let base = if least(black) >= least(white) { black } else { white };
        let [r, g, b, a] = ring(base).0;
        DataColor::new(Color32::from_rgba_unmultiplied(r, g, b, a))
    }
}

/// The shape of a swatch's well.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwatchShape {
    /// A square well.
    #[default]
    Square,
    /// A hexagon well, for a honeycomb of touching cells.
    Hexagon,
    /// A round well: a circle in the square, a pill when the swatch fills its width. The ranges of Hue/Saturation.
    Round,
}

/// What fills a swatch's well.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SwatchFill {
    /// A colour over a checkerboard where it is translucent; `Mixed` shows the mixed well with its dash.
    Color(Mixed<Srgba>),
    /// A colour before and after a change: the first fills the half at the start edge, the second the half at the end
    /// edge.
    Split(Srgba, Srgba),
    /// The hue wheel, red at the top and the hues going clockwise: every colour, as a Master range.
    Spectrum,
    /// A picture filling the well over the checkerboard, cut to its middle where its aspect differs.
    Image(SwatchImage),
    /// Nothing: the checkerboard alone, as a clean brush.
    Empty,
    /// No colour: the sunken well with the "no colour" diagonal, read as "None" (an annotation with no fill).
    None,
}

impl SwatchFill {
    /// The fill of a colour that may be "no colour": `None` is [`SwatchFill::None`].
    pub fn optional(color: Mixed<Option<Srgba>>) -> SwatchFill {
        match color {
            Mixed::Same(Some(color)) => SwatchFill::Color(Mixed::Same(color)),
            Mixed::Same(None) => SwatchFill::None,
            Mixed::Mixed => SwatchFill::Color(Mixed::Mixed),
        }
    }
}

impl From<Srgba> for SwatchFill {
    fn from(color: Srgba) -> Self {
        SwatchFill::Color(Mixed::Same(color))
    }
}

impl From<Mixed<Srgba>> for SwatchFill {
    fn from(color: Mixed<Srgba>) -> Self {
        SwatchFill::Color(color)
    }
}

/// A picture for a swatch's well. [`new`](Self::new) hashes the pixels once into the key the texture cache keeps the
/// picture's texture under, so a well showing the same picture uploads it once and a new picture uploads again.
#[derive(Clone, crate::contract::MemoProps)]
pub struct SwatchImage {
    key: u64,
    #[memo_props(covered_by = "key", reason = "the key is the hash of the pixels")]
    pixels: Arc<ColorImage>,
}

impl SwatchImage {
    /// The picture of `pixels`.
    pub fn new(pixels: ColorImage) -> Self {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::hash::DefaultHasher::new();
        pixels.size.hash(&mut hasher);
        pixels.pixels.hash(&mut hasher);
        SwatchImage { key: hasher.finish(), pixels: Arc::new(pixels) }
    }

    /// The pixels.
    pub fn pixels(&self) -> &ColorImage {
        &self.pixels
    }
}

impl PartialEq for SwatchImage {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl std::fmt::Debug for SwatchImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwatchImage").field("key", &self.key).field("size", &self.pixels.size).finish()
    }
}

/// The texture cache's key of a swatch picture.
#[derive(Hash)]
struct SwatchImageKey(u64);

/// What a swatch reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SwatchOutput {
    /// The swatch's response.
    pub response: Response,
    /// A primary click, Space, Enter or an AccessKit click reached the enabled swatch.
    pub clicked: bool,
    /// How the enabled swatch was activated: by key or click, with the modifiers held (Cmd or Ctrl-click and
    /// Alt-click reach the palette that owns the swatch).
    pub activation: Option<Activation>,
    /// The request for the host's colour picker, set on a click of a `picker` swatch; it has no colour while the swatch is `Mixed`.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for SwatchOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A swatch in a collection, such as a cell of a colour table, reports no item event.
impl<E> crate::contract::ItemOutput<E> for SwatchOutput {
    fn into_event(self) -> Option<E> {
        None
    }
}

/// A colour well that shows a colour from data and reports its activation; it edits nothing.
///
/// The colour fills the well over a checkerboard where it has alpha, inside a thin ring that separates it from the
/// panel. A `Mixed` colour shows a neutral well with a dash and has no colour value. `with_fill` shows any
/// [`SwatchFill`]: a before and after pair, the hue wheel, a picture or an empty well. An [`optional`](Swatch::optional)
/// swatch of no colour shows the sunken well with a diagonal and reads "None". An [`icon`](Swatch::icon) swatch shows
/// its glyph on the empty well, as a palette's Other… cell does. A click reports `clicked` and the
/// `activation`. A swatch with a `picker` key also files a `ColorRequest` for that key, opened at the swatch's colour
/// and titled with its label, as `pick_color`; the host opens its colour picker and answers with the picked colour. A
/// `Mixed` swatch files a request with no colour: the host's picker opens empty and the first pick sets the colour.
/// The host writes the pick to every target.
///
/// AccessKit reports a colour well named by the label, whose value is the colour as `#rrggbb` (`#rrggbbaa` when
/// translucent) and whose colour value is the colour; a mixed well has the value "Mixed" and no colour value. A split
/// well's value is both colours, "#before / #after", and its colour value the after colour; an empty well's value is
/// "Empty"; the hue wheel and a picture have no value, their label names them.
/// `member` makes the swatch a member of the enclosing `RadioGroup`: a radio button, checked when the group holds its
/// value, whose keys and focus are the group's. `cell` makes it a cell of a table of touching swatches, a list box
/// option. The selected ring is `selected` or a checked radio. `marks` draws the cell marks (`CellMarks`): a thin edge
/// in the text colour just inside the outline for a cell that stands out from its row, such as the harmony colour among
/// its variations, and the locked, web-safe, transparent and source marks. `stroke` shows the colour as a ring and
/// `face` replaces the fill with a host painter; both are drawn on a square or round well. `shape` makes the well
/// round, or a hexagon, a cell of a honeycomb.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter, probe(Swatch<u8>))]
pub struct Swatch<T: Value + 'static = ()> {
    #[common]
    common: Common,
    fill: SwatchFill,
    /// The value this swatch stands for in its `RadioGroup`.
    member: Option<T>,
    /// A cell of a table of touching swatches: `size.icon.md` square with square corners.
    #[style_key]
    #[prop(flag)]
    cell: bool,
    /// A cell shown as a well, at the control height with the well's corners: a palette's swatches.
    #[style_key]
    #[prop(flag)]
    well: bool,
    /// Paints these marks on the well: the edge mark, a locked corner square, a web-safe centre diamond, the
    /// transparent checker half and the source-colour corner triangle, each from tokens.
    #[prop]
    marks: CellMarks,
    /// The well shows its colour as a ring: a band of the colour round the edge, open in the middle.
    #[style_key]
    #[prop(flag)]
    stroke: bool,
    /// The face the well shows over its checker instead of the colour; ring, mark and mixed look unchanged.
    #[prop]
    face: Option<FacePaint>,
    /// The shape of the well.
    #[prop]
    shape: SwatchShape,
    /// The key of the `ColorRequest` a click files.
    #[prop(into)]
    picker: Option<String>,
    /// The glyph an icon cell paints over its empty well, in place of a colour.
    icon: Option<IconId>,
    checked: bool,
    activation: Option<Activation>,
}

impl Swatch<()> {
    /// A swatch of `color` named `label`.
    pub fn new(label: impl Into<crate::runtime::env::Label>, color: impl Into<Mixed<Srgba>>) -> Self {
        Swatch::with_fill(label, SwatchFill::Color(color.into()))
    }

    /// A swatch whose well shows `fill`, named `label`.
    pub fn with_fill(label: impl Into<crate::runtime::env::Label>, fill: SwatchFill) -> Self {
        Swatch {
            common: Common::new(),
            fill,
            member: None,
            cell: false,
            well: false,
            marks: CellMarks::default(),
            stroke: false,
            face: None,
            shape: SwatchShape::default(),
            picker: None,
            icon: None,
            checked: false,
            activation: None,
        }
        .label(label)
    }

    /// A swatch that can be "no colour": `None` paints the sunken well with the diagonal and reads as "None".
    pub fn optional(label: impl Into<crate::runtime::env::Label>, color: impl Into<Mixed<Option<Srgba>>>) -> Swatch<()> {
        Swatch::with_fill(label, SwatchFill::optional(color.into()))
    }

    /// A well with the catalogue glyph `icon` on top; its access node is an option named `label` with no colour value.
    pub fn icon(label: impl Into<crate::runtime::env::Label>, icon: IconId) -> Swatch<()> {
        Swatch { icon: Some(icon), ..Swatch::with_fill(label, SwatchFill::Color(Mixed::Mixed)) }
    }
}

impl<T: Value + 'static> Swatch<T> {
    /// Makes the swatch the member standing for `value` of the enclosing `RadioGroup<U>`.
    pub fn member<U: Value + 'static>(self, value: U) -> Swatch<U> {
        Swatch {
            common: self.common,
            fill: self.fill,
            member: Some(value),
            cell: self.cell,
            well: self.well,
            marks: self.marks,
            stroke: self.stroke,
            face: self.face,
            shape: self.shape,
            picker: self.picker,
            icon: self.icon,
            checked: false,
            activation: None,
        }
    }
}

/// The marks of a swatch cell: each is painted from tokens. Other crates build them with the builders, as
/// `#[non_exhaustive]` forbids a struct literal outside this crate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CellMarks {
    /// A thin edge in the text colour just inside the outline.
    pub edge: bool,
    /// A locked corner square.
    pub locked: bool,
    /// A web-safe centre diamond.
    pub web_safe: bool,
    /// The transparent checker half.
    pub transparent: bool,
    /// The source-colour corner triangle.
    pub source: Option<Srgba>,
}

impl CellMarks {
    /// Sets the edge mark.
    #[must_use]
    pub fn edge(self, on: bool) -> Self {
        Self { edge: on, ..self }
    }

    /// Sets the locked mark.
    #[must_use]
    pub fn locked(self, on: bool) -> Self {
        Self { locked: on, ..self }
    }

    /// Sets the web-safe mark.
    #[must_use]
    pub fn web_safe(self, on: bool) -> Self {
        Self { web_safe: on, ..self }
    }

    /// Sets the transparent mark.
    #[must_use]
    pub fn transparent(self, on: bool) -> Self {
        Self { transparent: on, ..self }
    }

    /// Sets the source-colour mark.
    #[must_use]
    pub fn source(self, color: Srgba) -> Self {
        Self { source: Some(color), ..self }
    }
}

/// The share of a ring well's width its band of colour takes.
pub const RING_SHARE: f32 = 0.3;

/// Paints `color` as a ring in `well`: a band of the colour round the edge, open in the middle, each side with the
/// edge and the inner edge (`SwatchStyle::inner_edge`) that stand out from the colour, in `state`.
pub fn paint_ring(draw: &Draw<'_>, well: Rect, color: Srgba, style: &SwatchStyle, state: &StateSet, radius: CornerRadius) {
    let band = well.width() * RING_SHARE;
    let line = draw.width(StrokeWidth::Thin);
    let contrast = style.inner_edge(Some(color));
    let edge = state.resolve(&style.edge);
    let shape = PaintShape::Rect(well, radius);
    draw.stroke(&shape, DataWidth::points(band), DataColor::new(Color32::from(color)));
    draw.stroke(&shape, StrokeWidth::Thin, contrast);
    draw.stroke(&PaintShape::Rect(well.expand(line), UiTokens::corner(f32::from(radius.nw) + line)), StrokeWidth::Thin, edge);
    let hole = well.shrink(band);
    // The hole's corners are concentric with the well's: square for a square well, round for a round one.
    let corner = |grow: f32| UiTokens::corner((f32::from(radius.nw) - band + grow).max(0.0));
    draw.stroke(&PaintShape::Rect(hole.expand(2.0 * line), corner(2.0 * line)), StrokeWidth::Thin, contrast);
    draw.stroke(&PaintShape::Rect(hole.expand(line), corner(line)), StrokeWidth::Thin, edge);
}

impl<T: Value + 'static> Component for Swatch<T> {
    const ROLE: Role = Role::COLOR_WELL;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::RADIO_BUTTON, Role::LIST_BOX_OPTION];
    const DISABLED: DisabledLook = DisabledLook::Faded;
    type Style = SwatchStyle;
    type Output = SwatchOutput;

    fn style(cx: &StyleCx<'_>, key: SwatchStyleKey<T>) -> SwatchStyle {
        let colors = &cx.tokens.color;
        let metrics = SwatchMetrics::of(cx);
        let clear = cx.color(Rgba::TRANSPARENT);
        let rest = cx.color(colors.line.control_edge);
        let accent = cx.color(colors.accent.solid);
        SwatchStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            side: if key.cell && !key.well { metrics.cell } else { metrics.side },
            radius: if key.cell && !key.well { CornerRadius::ZERO } else { metrics.radius },
            edge: Stateful::new(rest),
            ring: Stateful::new(clear).on(States::new(accent, accent, accent, accent)).when(Flags::LEAD, cx.color(colors.line.focus)),
            mark: cx.color(colors.text.normal),
            mark_size: cx.tokens.size.icon.sm,
            glyph: cx.tokens.size.icon.sm,
            checker: [colors.overlay.checker_light, colors.overlay.checker_dark],
            mixed_rgba: colors.surface.sunken,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let side = cx.style.side;
        let width = if cx.resolved.sizing() == Some(Sizing::FillWidth) {
            cx.room.filter(|room| room.is_finite()).map_or(side, |room| room.max(side))
        } else {
            side
        };
        Vec2::new(width, side)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    /// Registers with the enclosing group when it is a member; reports the click.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(value) = &self.member {
            let group = cx.env.get::<GroupHandle<RadioKind<T>>>();
            group.register(value.clone(), cx.id, cx.resolved.enabled);
            cx.join_roving();
            self.checked = group.is(value);
        }
        match self.shape {
            SwatchShape::Square => {}
            SwatchShape::Round => cx.focus_shape(FocusShape::Circle),
            SwatchShape::Hexagon => cx.focus_shape(FocusShape::Polygon(hexagon(cx.rect))),
        }
        self.activation = cx.activation().filter(|_| cx.resolved.enabled);
    }

    fn flags(&self) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, self.checked);
        flags
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if self.member.is_some() {
            access.role(Role::RADIO_BUTTON);
        } else if self.cell || self.icon.is_some() {
            access.role(Role::LIST_BOX_OPTION);
        }
        if self.icon.is_some() {
            return;
        }
        match &self.fill {
            SwatchFill::Color(Mixed::Same(color)) => {
                access.value(color.to_hex());
                access.color(*color);
            }
            SwatchFill::Color(Mixed::Mixed) => {
                access.value(cx.words::<ChoiceWords>().swatch_mixed.as_str());
            }
            SwatchFill::Split(before, after) => {
                let (from, to) = (before.to_hex(), after.to_hex());
                access.value(cx.words::<ChoiceWords>().swatch_split.fill(&[("before", &from), ("after", &to)]));
                access.color(*after);
            }
            SwatchFill::Empty => {
                access.value(cx.words::<ChoiceWords>().swatch_empty.as_str());
            }
            SwatchFill::None => {
                access.value(cx.words::<ChoiceWords>().swatch_none.as_str());
            }
            SwatchFill::Spectrum | SwatchFill::Image(_) => {}
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let face = Face {
            style: cx.style,
            state: cx.state(),
            fill: &self.fill,
            face: self.face.as_ref(),
            stroke: self.stroke,
            marks: self.marks,
            glyph: self.icon.as_ref(),
            shape: self.shape,
            rtl: cx.flow.is_rtl(),
        };
        face.paint(&ui.content_painter(cx), cx.rect);
    }

    fn finish(self, cx: Cx<'_, Self>) -> SwatchOutput {
        let color = match &self.fill {
            SwatchFill::Color(color) => color.same().copied(),
            _ => None,
        };
        let pick_color = self.picker.filter(|_| self.activation.is_some()).map(|key| ColorRequest::new(key, color).title(cx.label()));
        SwatchOutput { response: cx.response.clone(), clicked: self.activation.is_some(), activation: self.activation, pick_color }
    }
}

/// What the one swatch face painter paints: the well in `shape` (the empty sunken well under the `glyph` of an icon
/// cell, the host's face over a checkerboard, a ring of the colour, or `fill`), then the outline, the cell marks and
/// the selection ring, in `state`. A split well's before half is at the start edge, the transparent mark's checker half
/// and the corner marks follow the flow, mirrored when `rtl`.
struct Face<'a> {
    style: &'a SwatchStyle,
    state: &'a StateSet,
    fill: &'a SwatchFill,
    face: Option<&'a FacePaint>,
    stroke: bool,
    marks: CellMarks,
    glyph: Option<&'a IconId>,
    shape: SwatchShape,
    rtl: bool,
}

impl Face<'_> {
    fn paint(&self, painter: &ContentPainterCx<'_>, well: Rect) {
        let style = self.style;
        let edge = self.outline(well);
        let ring = match self.fill {
            SwatchFill::Color(Mixed::Same(color)) if self.stroke && self.face.is_none() && self.glyph.is_none() => Some(*color),
            _ => None,
        };
        if let Some(glyph) = self.glyph {
            self.well(painter, well, WellFill::Empty);
            painter.with_draw(|draw| {
                let side = style.glyph.min(well.width()).min(well.height());
                draw.icon(Rect::from_center_size(well.center(), Vec2::splat(side)), &IconSpec::svg(glyph.clone()).color(style.mark));
            });
        } else {
            match (self.face, ring) {
                (Some(face), _) => {
                    painter.checker_with(&Checker::new(), well, self.radius(well));
                    face.paint(painter, well);
                }
                (None, Some(_)) => {}
                (None, None) => self.paint_fill(painter, well),
            }
        }
        if self.marks.transparent {
            let half = if self.rtl {
                Rect::from_min_max(well.min, pos2(well.center().x, well.max.y))
            } else {
                Rect::from_min_max(pos2(well.center().x, well.min.y), well.max)
            };
            painter.checker_with(&Checker::new(), half, CornerRadius::ZERO);
        }
        painter.with_draw(|draw| {
            match ring {
                // The ring's outline sits outside its well, so the well is taken in by one line to keep the outline in the rect.
                Some(color) => {
                    let inner = well.shrink(draw.width(StrokeWidth::Thin));
                    paint_ring(draw, inner, color, style, self.state, self.radius(inner));
                }
                None => draw.stroke(&edge, StrokeWidth::Thin, self.state.resolve(&style.edge)),
            }
            self.paint_marks(draw, well);
            draw.stroke(&edge, StrokeWidth::Thick, self.state.resolve(&style.ring));
        });
    }

    /// The cell marks that `Draw` paints, with the corner marks at the start-top and end-bottom corners of the flow.
    fn paint_marks(&self, draw: &Draw<'_>, well: Rect) {
        let (marks, rtl, side, style) = (&self.marks, self.rtl, self.style.mark_size, self.style);
        if marks.edge {
            let inside = self.outline(well.shrink(draw.width(StrokeWidth::Thin)));
            draw.stroke(&inside, StrokeWidth::Thin, style.mark);
        }
        if marks.locked {
            let x = if rtl { well.max.x - side } else { well.min.x };
            let square = Rect::from_min_size(pos2(x, well.min.y), Vec2::splat(side));
            draw.fill(&PaintShape::Rect(square, CornerRadius::ZERO), style.mark);
        }
        if marks.web_safe {
            let (c, h) = (well.center(), side / 2.0);
            let diamond = vec![pos2(c.x, c.y - h), pos2(c.x + h, c.y), pos2(c.x, c.y + h), pos2(c.x - h, c.y)];
            draw.stroke(&PaintShape::Polygon(diamond), StrokeWidth::Thin, style.mark);
        }
        if let Some(color) = marks.source {
            let (x, step) = if rtl { (well.min.x, side) } else { (well.max.x, -side) };
            let triangle = vec![pos2(x, well.max.y), pos2(x + step, well.max.y), pos2(x, well.max.y - side)];
            draw.data_fill(&PaintShape::Polygon(triangle), DataColor::new(Color32::from(color)));
        }
    }

    /// The corners of a well in `rect`: the style's for a square, half the shorter side for a round one (a circle,
    /// or a pill when wider than tall).
    fn radius(&self, rect: Rect) -> CornerRadius {
        match self.shape {
            SwatchShape::Round => UiTokens::corner(rect.width().min(rect.height()) / 2.0),
            SwatchShape::Square | SwatchShape::Hexagon => self.style.radius,
        }
    }

    /// The outline of a well in `rect`.
    fn outline(&self, rect: Rect) -> PaintShape {
        match self.shape {
            SwatchShape::Square | SwatchShape::Round => PaintShape::Rect(rect, self.radius(rect)),
            SwatchShape::Hexagon => PaintShape::Polygon(hexagon(rect)),
        }
    }

    fn paint_fill(&self, painter: &ContentPainterCx<'_>, well: Rect) {
        match self.fill {
            SwatchFill::Color(color) => self.color_well(painter, well, color.same().copied()),
            SwatchFill::Empty => self.color_well(painter, well, Some(CLEAR)),
            SwatchFill::None => self.well(painter, well, WellFill::None),
            SwatchFill::Split(before, after) => {
                self.color_well(painter, well, Some(*before));
                let middle = well.center().x;
                let end = if self.rtl {
                    Rect::from_x_y_ranges(well.left()..=middle, well.y_range())
                } else {
                    Rect::from_x_y_ranges(middle..=well.right(), well.y_range())
                };
                painter.clipped(end, |painter| self.color_well(painter, well, Some(*after)));
            }
            SwatchFill::Spectrum => {
                let mesh = spectrum_mesh(well.center(), &self.corners(well));
                match self.shape {
                    SwatchShape::Hexagon => painter.data_shapes([Shape::mesh(mesh)]),
                    SwatchShape::Square | SwatchShape::Round => painter.mesh_in(well, self.radius(well), mesh),
                }
            }
            SwatchFill::Image(image) => {
                self.color_well(painter, well, Some(CLEAR));
                let texture = TextureCache::texture(painter.painter.ctx(), SwatchImageKey(image.key), || image.pixels().clone());
                let uv = cover_uv(image.pixels().size, well.size());
                match self.shape {
                    SwatchShape::Hexagon => {
                        let corners = hexagon(well);
                        let mut mesh = Mesh::with_texture(texture);
                        for &pos in &corners {
                            let at = (pos - well.min) / well.size();
                            mesh.vertices.push(egui::epaint::Vertex { pos, uv: uv.lerp_inside(at), color: UNTINTED });
                        }
                        for k in 1..corners.len() as u32 - 1 {
                            mesh.add_triangle(0, k, k + 1);
                        }
                        painter.data_shapes([Shape::mesh(mesh)]);
                    }
                    SwatchShape::Square | SwatchShape::Round => painter.picture(texture, well, uv, f32::from(self.radius(well).nw)),
                }
            }
        }
    }

    /// The well of `color` (the checkerboard behind a translucent one, the dash of `None`) over `well`.
    fn color_well(&self, painter: &ContentPainterCx<'_>, well: Rect, color: Option<Srgba>) {
        self.well(painter, well, WellFill::from(color));
    }

    /// The well `fill` over `well`, in the swatch's shape.
    fn well(&self, painter: &ContentPainterCx<'_>, well: Rect, fill: WellFill) {
        painter.with_draw(|draw| match self.shape {
            SwatchShape::Hexagon => draw.swatch_polygon_well(&hexagon(well), fill),
            SwatchShape::Square | SwatchShape::Round => draw.swatch_well(well, self.radius(well), fill),
        });
    }

    /// The corners of the polygon a hue wheel is laid over: the hexagon, or the rect, whose rounded corners
    /// `mesh_in` pulls the wheel inside.
    fn corners(&self, well: Rect) -> Vec<Pos2> {
        match self.shape {
            SwatchShape::Hexagon => hexagon(well),
            SwatchShape::Square | SwatchShape::Round => vec![well.left_top(), well.right_top(), well.right_bottom(), well.left_bottom()],
        }
    }
}

/// The hue wheel over the convex polygon `corners` round `centre`: full colours, each point along an edge the hue of
/// its direction from the centre, red straight up and the hues going clockwise. Each slice has its own centre vertex
/// in the slice's middle hue, so the hues meet at the centre without a blend towards one colour.
fn spectrum_mesh(centre: Pos2, corners: &[Pos2]) -> Mesh {
    let hue = |pos: Pos2| {
        let d = pos - centre;
        let degrees = d.x.atan2(-d.y).to_degrees().rem_euclid(360.0);
        Color32::from(Srgba::from_hsb(Hsb { h: degrees, s: 1.0, b: 1.0 }))
    };
    let rim: Vec<Pos2> = corners
        .iter()
        .enumerate()
        .flat_map(|(k, &from)| {
            let to = corners[(k + 1) % corners.len()];
            (0..SPECTRUM_STEPS).map(move |step| from.lerp(to, step as f32 / SPECTRUM_STEPS as f32))
        })
        .collect();
    let mut mesh = Mesh::default();
    for (k, &from) in rim.iter().enumerate() {
        let to = rim[(k + 1) % rim.len()];
        let at = mesh.vertices.len() as u32;
        mesh.colored_vertex(centre, hue(from.lerp(to, 0.5)));
        mesh.colored_vertex(from, hue(from));
        mesh.colored_vertex(to, hue(to));
        mesh.add_triangle(at, at + 1, at + 2);
    }
    mesh
}

/// The part of a picture of `size` pixels that covers a well of `well` points at the picture's aspect: all of it
/// along the side that fits, the middle along the other.
fn cover_uv(size: [usize; 2], well: Vec2) -> Rect {
    let (width, height) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let picture = width / height;
    let room = if well.y > 0.0 { well.x / well.y } else { 1.0 };
    if picture > room {
        let share = room / picture;
        Rect::from_x_y_ranges((0.5 - share / 2.0)..=(0.5 + share / 2.0), 0.0..=1.0)
    } else {
        let share = picture / room;
        Rect::from_x_y_ranges(0.0..=1.0, (0.5 - share / 2.0)..=(0.5 + share / 2.0))
    }
}

/// The corners of the pointy-top hexagon inscribed in `rect`: the vertices at the middle of the top and bottom edges,
/// the vertical sides spanning the middle half of the height. The honeycomb layout hit-tests against the same points.
pub(super) fn hexagon(rect: Rect) -> Vec<Pos2> {
    let quarter = rect.height() / 4.0;
    let middle = rect.center().x;
    vec![
        pos2(middle, rect.top()),
        pos2(rect.right(), rect.top() + quarter),
        pos2(rect.right(), rect.bottom() - quarter),
        pos2(middle, rect.bottom()),
        pos2(rect.left(), rect.bottom() - quarter),
        pos2(rect.left(), rect.top() + quarter),
    ]
}

/// The face of a swatch outside a `Swatch`: a preset thumbnail of a colour (`PresetFns::thumb`). It paints
/// what a non-cell `Swatch` of the same colour paints, from the style that `Swatch` resolves, with the selected
/// ring when `selected`.
#[derive(Clone, Copy, Debug)]
pub struct SwatchFace {
    color: Mixed<Option<Srgba>>,
    selected: bool,
}

impl SwatchFace {
    /// The face of `color`, not selected.
    pub fn new(color: impl Into<Mixed<Srgba>>) -> Self {
        let color = match color.into() {
            Mixed::Same(color) => Mixed::Same(Some(color)),
            Mixed::Mixed => Mixed::Mixed,
        };
        SwatchFace { color, selected: false }
    }

    /// The face of a colour that can be "no colour": `None` paints the sunken well with the diagonal.
    pub fn optional(color: impl Into<Mixed<Option<Srgba>>>) -> SwatchFace {
        SwatchFace { color: color.into(), selected: false }
    }

    /// Whether the selected ring shows.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Paints the face filling `rect`.
    pub fn paint(&self, cx: &ContentPainterCx<'_>, rect: Rect) {
        let key = Swatch::optional("", self.color).style_key();
        let scope = cx.style_cx();
        let style = resolve_style(&scope, key, cx.painter.ctx(), Swatch::<()>::style);
        let (flags, on) = if self.selected { (Flags::SELECTED, 1.0) } else { (Flags::empty(), 0.0) };
        let state = StateSet::new(State::Rest, flags, StateMix::new(0.0, 0.0, on));
        let fill = SwatchFill::optional(self.color);
        let face = Face {
            style: &style,
            state: &state,
            fill: &fill,
            face: None,
            stroke: false,
            marks: CellMarks::default(),
            glyph: None,
            shape: SwatchShape::Square,
            rtl: false,
        };
        face.paint(cx, rect);
    }
}
