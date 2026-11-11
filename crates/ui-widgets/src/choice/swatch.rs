//! `Swatch`: a colour well. It shows a colour from data, a mixed look for several colours, and reports the activation.
//! It edits nothing: the host opens its picker on a click, or on the `ColorRequest` of a `picker` swatch.

use super::radio::RadioKind;
use super::words::ChoiceWords;
use crate::contract::{
    Access, Activation, Common, Component, Cx, GroupHandle, MeasureCx, Meta as _, Outcome, PaintUi, Pointer as _, Props as _, Role, Styled as _,
    WordSetCx as _,
};
use crate::primitives::{Checker, ContentPainterCx, Draw, PaintShape, StrokeWidth};
use crate::runtime::requests::ColorRequest;
use crate::style::{ComponentStyle, DataColor, Flags, FocusLook, State, StateMix, StateSet, StyleColor, StyleCx, UiTokens, resolve_style};
use craft_behavior::{CursorIcon, CursorPriority, Metrics, Mixed, Sense, Value};
use craft_theme::{Rgba, Srgba, Stateful, States, over};
use egui::{Color32, CornerRadius, Rect, Response, Shape, Vec2, pos2};

#[cfg(feature = "testing")]
mod stories;

/// Opacity of the black or white ring that stands out from a well's colour.
const INNER_EDGE_ALPHA: u8 = 0x80;
/// The length of the dash on a mixed well, as a share of the well's width.
const MIXED_DASH_SHARE: f32 = 0.5;

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
    /// The ring inside the outline of a selected or checked well; clear while it is off.
    ring: Stateful<StyleColor>,
    /// The well of a mixed colour.
    mixed_fill: StyleColor,
    /// The dash on a mixed well.
    mixed_mark: StyleColor,
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

/// A colour well that shows a colour from data and reports its activation; it edits nothing.
///
/// The colour fills the well over a checkerboard where it has alpha, inside a thin ring that separates it from the
/// panel. A `Mixed` colour shows a neutral well with a dash and has no colour value. A click reports `clicked` and the
/// `activation`. A swatch with a `picker` key also files a `ColorRequest` for that key, opened at the swatch's colour
/// and titled with its label, as `pick_color`; the host opens its colour picker and answers with the picked colour. A
/// `Mixed` swatch files a request with no colour: the host's picker opens empty and the first pick sets the colour.
/// The host writes the pick to every target.
///
/// AccessKit reports a colour well named by the label, whose value is the colour as `#rrggbb` (`#rrggbbaa` when
/// translucent) and whose colour value is the colour; a mixed well has the value "Mixed" and no colour value.
/// `member` makes the swatch a member of the enclosing `RadioGroup`: a radio button, checked when the group holds its
/// value, whose keys and focus are the group's. `cell` makes it a cell of a table of touching swatches, a list box
/// option. The selected ring is `selected` or a checked radio.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter, probe(Swatch<u8>))]
pub struct Swatch<T: Value + 'static = ()> {
    #[common]
    common: Common,
    color: Mixed<Srgba>,
    /// The value this swatch stands for in its `RadioGroup`.
    member: Option<T>,
    /// A cell of a table of touching swatches: `size.icon.md` square with square corners.
    #[style_key]
    #[prop(flag)]
    cell: bool,
    /// Takes the width the layout offers, at the control height.
    #[prop(flag)]
    fill: bool,
    /// The key of the `ColorRequest` a click files.
    #[prop(into)]
    picker: Option<String>,
    checked: bool,
    activation: Option<Activation>,
}

impl Swatch<()> {
    /// A swatch of `color` named `label`.
    pub fn new(label: impl Into<crate::runtime::env::Label>, color: impl Into<Mixed<Srgba>>) -> Self {
        Swatch { common: Common::new(), color: color.into(), member: None, cell: false, fill: false, picker: None, checked: false, activation: None }
            .label(label)
    }
}

impl<T: Value + 'static> Swatch<T> {
    /// Makes the swatch the member standing for `value` of the enclosing `RadioGroup<U>`.
    pub fn member<U: Value + 'static>(self, value: U) -> Swatch<U> {
        Swatch {
            common: self.common,
            color: self.color,
            member: Some(value),
            cell: self.cell,
            fill: self.fill,
            picker: self.picker,
            checked: false,
            activation: None,
        }
    }
}

impl<T: Value + 'static> Component for Swatch<T> {
    const ROLE: Role = Role::COLOR_WELL;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::RADIO_BUTTON, Role::LIST_BOX_OPTION];
    type Style = SwatchStyle;
    type Output = SwatchOutput;

    fn style(cx: &StyleCx<'_>, key: SwatchStyleKey<T>) -> SwatchStyle {
        let colors = &cx.tokens.color;
        let metrics = SwatchMetrics::of(cx);
        let clear = cx.color(Rgba::TRANSPARENT);
        let rest = cx.color(colors.line.control_edge);
        let accent = cx.color(colors.accent.solid);
        let disabled = cx.color(colors.text.disabled);
        SwatchStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            side: if key.cell { metrics.cell } else { metrics.side },
            radius: if key.cell { CornerRadius::ZERO } else { metrics.radius },
            edge: Stateful::new(rest).off(States::new(rest, rest, rest, disabled)),
            ring: Stateful::new(clear).on(States::new(accent, accent, accent, disabled)),
            mixed_fill: cx.color(colors.surface.sunken),
            mixed_mark: cx.color(colors.text.muted),
            checker: [colors.overlay.checker_light, colors.overlay.checker_dark],
            mixed_rgba: colors.surface.sunken,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let side = cx.style.side;
        let width = if self.fill { cx.room.filter(|room| room.is_finite()).map_or(side, |room| room.max(side)) } else { side };
        Vec2::new(width, side)
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        self.fill.then_some(craft_behavior::Axis::X)
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
        self.activation = cx.activation().filter(|_| cx.resolved.enabled);
        if cx.resolved.enabled && cx.hit() {
            cx.cursor(CursorIcon::PointingHand, CursorPriority::Part);
        }
    }

    fn flags(&self) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, self.checked);
        flags
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if self.member.is_some() {
            access.role(Role::RADIO_BUTTON);
        } else if self.cell {
            access.role(Role::LIST_BOX_OPTION);
        }
        match self.color.same() {
            Some(color) => {
                access.value(color.to_hex());
                access.color(*color);
            }
            None => {
                access.value(cx.words::<ChoiceWords>().swatch_mixed.as_str());
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        paint_face(&ui.draw(), &ui.env.tokens, cx.rect, cx.style, cx.state(), self.color);
    }

    fn finish(self, cx: Cx<'_, Self>) -> SwatchOutput {
        let pick_color =
            self.picker.filter(|_| self.activation.is_some()).map(|key| ColorRequest::new(key, self.color.same().copied()).title(cx.label()));
        SwatchOutput { response: cx.response.clone(), clicked: self.activation.is_some(), activation: self.activation, pick_color }
    }
}

/// The one swatch face painter: the well of `color` in `well` (a checkerboard behind a translucent colour, the dash
/// of a mixed one), then the outline ring and the selection ring, in `state`.
fn paint_face(draw: &Draw<'_>, tokens: &UiTokens, well: Rect, style: &SwatchStyle, state: &StateSet, color: Mixed<Srgba>) {
    let shape = PaintShape::Rect(well, style.radius);
    match color.same() {
        None => {
            draw.fill(&shape, style.mixed_fill);
            let (center, half) = (well.center(), well.width() * MIXED_DASH_SHARE / 2.0);
            draw.stroke(
                &PaintShape::Path(vec![pos2(center.x - half, center.y), pos2(center.x + half, center.y)]),
                StrokeWidth::Medium,
                style.mixed_mark,
            );
        }
        Some(color) => {
            if color.a < 1.0 {
                for cell in rounded(Checker::new().shapes(tokens, well, well), well, style.radius) {
                    if let Shape::Rect(cell) = cell {
                        draw.data_fill(&PaintShape::Rect(cell.rect, cell.corner_radius), DataColor::new(cell.fill));
                    }
                }
            }
            draw.data_fill(&shape, DataColor::new(Color32::from(*color)));
        }
    }
    draw.stroke(&shape, StrokeWidth::Thin, state.resolve(&style.edge));
    draw.stroke(&shape, StrokeWidth::Thick, state.resolve(&style.ring));
}

/// The face of a swatch outside a `Swatch`: a preset thumbnail of a colour (`PresetBrowser::thumb`). It paints
/// what a non-cell `Swatch` of the same colour paints, from the style that `Swatch` resolves, with the selected
/// ring when `selected`.
#[derive(Clone, Copy, Debug)]
pub struct SwatchFace {
    color: Mixed<Srgba>,
    selected: bool,
}

impl SwatchFace {
    /// The face of `color`, not selected.
    pub fn new(color: impl Into<Mixed<Srgba>>) -> Self {
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
        let key = Swatch::new("", self.color).style_key();
        let scope = cx.style_cx();
        let style = resolve_style(&scope, key, cx.painter.ctx(), Swatch::<()>::style);
        let (flags, on) = if self.selected { (Flags::SELECTED, 1.0) } else { (Flags::empty(), 0.0) };
        let state = StateSet::new(State::Rest, flags, StateMix::new(0.0, 0.0, on));
        cx.with_draw(|draw| paint_face(draw, scope.tokens, rect, &style, &state, self.color));
    }
}

/// `shapes` of a checkerboard over `well` with the corners of `radius` where a cell meets a corner of the well.
fn rounded(mut shapes: Vec<Shape>, well: Rect, radius: CornerRadius) -> Vec<Shape> {
    for shape in &mut shapes {
        if let Shape::Rect(cell) = shape {
            let round = |touches: bool, r: u8| if touches { r } else { 0 };
            cell.corner_radius = CornerRadius {
                nw: round(cell.rect.min == well.min, radius.nw),
                ne: round(cell.rect.right_top() == well.right_top(), radius.ne),
                sw: round(cell.rect.left_bottom() == well.left_bottom(), radius.sw),
                se: round(cell.rect.max == well.max, radius.se),
            };
        }
    }
    shapes
}
