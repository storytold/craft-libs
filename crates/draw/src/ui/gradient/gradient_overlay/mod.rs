//! [`GradientOverlay`]: a gradient drawn on the canvas as the live Gradient tool draws it, a line between a start and an
//! end handle with the colour stops and midpoint diamonds on it. A canvas control (20.6): every part is in the canvas's
//! arena, so a press on a part stays with the overlay and one elsewhere reaches the canvas's tool.

mod keys;
mod parts;
mod pointer;
#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::kurbo::Point;
use craft_behavior::{EditEvent, Metrics, Mixed};
use craft_keymap::{Chord, Key, Mods};
use craft_ui::canvas::{canvas_snap, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{Dashes, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, HandleLook, StyleColor, StyleCx};
use egui::{Color32, Pos2, Rect, Response, Vec2, vec2};

use super::gradient_line::{at_screen, midpoint_location};
use super::marker_texts;
use super::{GradientExtent, GradientWords, LineGradient, LinePart, Sampler, StopColor, StopRef, StopRow, StopRows, StopRules};
use parts::{END, LINE, MIDPOINT, START, STOP, line_part};

craft_ui::words! {
    /// The names a screen reader reads for the overlay's group and parts.
    pub struct GradientLineWords {
        /// The group.
        group: "gradient.group" = "Gradient";
        /// The line between the handles.
        line: "gradient.line" = "Gradient line";
        /// The start handle.
        start: "gradient.start" = "Start point";
        /// The end handle.
        end: "gradient.end" = "End point";
        /// An end handle's value. `{x}` and `{y}` stand for its coordinates in the canvas's units.
        point_value: template "gradient.pointValue" = "{x}, {y}";
        /// The group's value when the host gives none. `{n}` stands for the number of colour stops.
        stops: count "gradient.lineStops" = "{n} stop" | "{n} stops";
    }
}

pub(super) const ENTER: Chord = Chord::plain(Key::Enter);
pub(super) const SPACE: Chord = Chord::plain(Key::Space);
pub(super) const INSERT: Chord = Chord::plain(Key::Insert);
pub(super) const CMD_ENTER: Chord = Chord::new(Mods::CMD, Key::Enter);

/// Keys the focused overlay keeps from the app's shortcuts: Insert and Cmd/Ctrl+Enter add a stop.
const CLAIMS: &[Chord] = &[INSERT, CMD_ENTER];

/// The style of the gradient overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GradientOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The end handles' look.
    handle: HandleLook,
    /// The line, the extent and the rings.
    line: StyleColor,
    /// The outline beside the line and under the rings.
    halo: StyleColor,
    /// The selected stop's ring, the selected diamond's fill and a picked end's ring.
    selected: StyleColor,
    /// The fill under a stop's colour where the colour has opacity.
    backing: StyleColor,
    /// Length of a dash of the mirrored extent.
    dash: f32,
    /// Length of the gap between dashes of the mirrored extent.
    gap: f32,
    /// Radius of an end handle's ring.
    end_radius: f32,
    /// Radius of a colour stop's disc.
    stop_radius: f32,
    /// Radius of a midpoint's diamond.
    diamond_radius: f32,
    /// Distance from the line past which a dragged stop is torn off.
    tear_off: f32,
}

/// Distance in points from the line at which a dragged stop is torn off.
const TEAR_OFF: f32 = 36.0;

/// A gradient's line over a canvas: the line between its handles, the circle, square or mirrored half its style adds,
/// and the colour stops and midpoint diamonds on it, as the live Gradient tool draws them. The host holds the
/// gradient and passes the line and stops as the value.
///
/// - Look: the line over its halo, the extent in the line's style (the mirrored half dashed, no halo), rings at the ends,
///   a disc in each stop's colour with a ring round it, and a diamond between stops. Points go through the canvas's
///   view, so a rotated, flipped or stretched view draws the line where the document has it. Under
///   [`StopRows::Split`] and [`StopRows::ColorOnly`] a stop's disc is opaque; under [`StopRows::Combined`] its colour
///   shows its opacity over a backing. Nothing mirrors in a right-to-left interface.
/// - Pointer: the end, start, nearest stop, nearest midpoint and the line are parts in that priority. A drag begins past
///   the click slop. An end follows the pointer (Shift: 45 degree steps from [`angle_base`](Self::angle_base) about the
///   other end), the line moves both ends, a stop moves to the location under the pointer and is torn off, removed when
///   the drag ends there, more than the style's tear-off distance from the line while its row is above `rules.min`, a
///   midpoint moves within its segment. Alt drags a copy of a stop with [`alt_copies`](Self::alt_copies). Every update is
///   the whole edit of the gradient at the press. Ends and the line snap through the canvas's `Snap`.
/// - A press on a stop or midpoint selects it and the selection follows a dragged stop. A click on the line adds a colour
///   stop there and selects it. A double-click on a stop, or on an end for the stop there, sets
///   [`color_requested`](GradientOverlayOutput::color_requested). A secondary click sets
///   [`context`](GradientOverlayOutput::context).
/// - Keys (one Tab stop): Page Down and Page Up pick the next and previous part, Home and End the first and last: the
///   line, the start, the stops and midpoints in location order, the end. Picking a stop or midpoint selects it. The
///   arrows move a picked stop or midpoint by 1 %, 10 % with Shift, Left always lowering the location, and a picked end
///   or the line by 1 document unit, 10 with Shift. Delete and Backspace remove a stop, Insert and Cmd/Ctrl+Enter add one
///   halfway after the picked stop. Enter and Space on a colour stop or an end set `color_requested`. Each change is one
///   Set.
/// - With [`shared_selection`](Self::shared_selection) the overlay and a [`GradientEditor`](super::GradientEditor) share one selected stop.
#[derive(Component, Props)]
#[component(canvas_control)]
pub struct GradientOverlay<'a> {
    #[common]
    common: Common,
    value: Bind<'a, LineGradient>,
    /// The row layout and the fewest and most stops and the midpoint range the host gives its gradient editor.
    #[prop]
    rules: StopRules,
    /// Colours a stop added on the line; the default sampler of the rules' rows by default.
    #[prop]
    sampler: Option<&'a Sampler>,
    /// Whether midpoint diamonds show.
    #[prop]
    midpoints: bool,
    /// Whether the gradient runs from the end to the start.
    #[prop(flag)]
    reverse: bool,
    /// The shape the gradient's style adds round the line; decoration only.
    #[prop]
    extent: GradientExtent,
    /// Whether an Alt-drag on a stop drags a copy of it; off, Alt is the tool's.
    #[prop(flag)]
    alt_copies: bool,
    /// The angle in degrees, along the document's axes, that Shift's 45 degree steps of a dragged end start from.
    #[prop(quantity = "degrees")]
    angle_base: f64,
    /// Decimals of a stop's location in its accessible value.
    #[prop(quantity = "decimals")]
    location_decimals: u8,
    /// The selected stop or midpoint, shared with the host's gradient editor. Without it the overlay keeps its own.
    #[prop]
    shared_selection: Option<Bind<'a, Option<StopRef>>>,
    /// The group's accessible value; "{n} stops" by default.
    #[prop(into)]
    value_text: Option<Label>,
    report: Report,
}

/// What the overlay reports beyond the edit.
#[derive(Debug, Default)]
struct Report {
    color_requested: Option<StopColor>,
    picked: Option<LinePart>,
    hovered: Option<LinePart>,
    context: Option<LinePart>,
    selected: Option<StopRef>,
    /// The dragged stop is torn off the line: the drag's release removes it.
    torn: bool,
}

impl<'a> GradientOverlay<'a> {
    /// An overlay named `label` over `value`, the line and stops in document units.
    pub fn new(label: impl Into<Label>, value: Bind<'a, LineGradient>) -> Self {
        GradientOverlay {
            common: Common::new(),
            value,
            rules: StopRules::default(),
            sampler: None,
            midpoints: true,
            reverse: false,
            extent: GradientExtent::None,
            alt_copies: false,
            angle_base: 0.0,
            location_decimals: 0,
            shared_selection: None,
            value_text: None,
            report: Report::default(),
        }
        .label(label)
        .ltr_axis()
    }

    /// The selected stop or midpoint: the shared binding's, or the overlay's own.
    fn selection(&mut self, cx: &mut Cx<'_, Self>) -> Option<StopRef> {
        match self.shared_selection.as_mut() {
            Some(bind) => match bind.reborrow().read(cx) {
                Some(Mixed::Same(selected)) => selected,
                _ => None,
            },
            None => cx.keep::<OwnSelection>().0,
        }
    }

    /// Selects `next` (a stop or midpoint, or nothing) in the shared binding or the overlay's own slot.
    fn select(&mut self, cx: &mut Cx<'_, Self>, next: Option<StopRef>) {
        let before = self.selection(cx);
        if before == next {
            return;
        }
        match self.shared_selection.as_mut() {
            Some(bind) => {
                let label: Arc<str> = Arc::from(cx.words::<GradientLineWords>().group.as_str());
                let event = EditEvent::set(Mixed::Same(before), next, label, None);
                bind.reborrow().write(cx, &event);
            }
            None => cx.keep::<OwnSelection>().0 = next,
        }
    }

    /// The colour a part asks for: a colour stop's, or the one at an end handle (stop 0 or the last, swapped under
    /// `reverse`).
    fn color_request(&self, value: &LineGradient, part: LinePart) -> Option<StopColor> {
        let len = value.stops.color.len();
        let index = match part {
            LinePart::Stop(i) => i,
            LinePart::Start | LinePart::End if len > 0 => {
                if (part == LinePart::Start) != self.reverse {
                    0
                } else {
                    len - 1
                }
            }
            _ => return None,
        };
        let stop = value.stops.color.get(index)?;
        let request = craft_ui::runtime::requests::ColorRequest::new("gradient.stop", stop.color);
        Some(StopColor { index, tag: stop.tag, request })
    }

    /// The group's accessible value.
    fn group_value(&self, cx: &Cx<'_, Self>, value: &LineGradient) -> String {
        match &self.value_text {
            Some(text) => cx.env.resolve(text).as_str().to_owned(),
            None => {
                let n = value.stops.color.len() as u64;
                cx.words::<GradientLineWords>().stops.text(n, &cx.format().integer(n))
            }
        }
    }
}

/// The selection the overlay keeps when the host binds none.
#[derive(Clone, Copy, Default)]
struct OwnSelection(Option<StopRef>);

/// What the overlay reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct GradientOverlayOutput {
    /// The edit of the line and stops: a drag is one `Begin` to `Commit`; a key, or a click on the line that adds a
    /// stop, is a `Set`.
    pub edit: EditResponse<LineGradient>,
    /// A double-click, Enter or Space asked for the colour of a colour stop.
    pub color_requested: Option<StopColor>,
    /// The part the keys act on.
    pub picked: Option<LinePart>,
    /// The part under the pointer.
    pub hovered: Option<LinePart>,
    /// A secondary click on a part, for the host's context menu.
    pub context: Option<LinePart>,
    /// The selected stop or midpoint after this pass.
    pub selected: Option<StopRef>,
}

impl Outcome for GradientOverlayOutput {
    type Value = LineGradient;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

impl Component for GradientOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS.with_deletes().with_chrome_key(false).with_claims(CLAIMS);
    type Style = GradientOverlayStyle;
    type Output = GradientOverlayOutput;

    fn style(cx: &StyleCx<'_>, _: GradientOverlayStyleKey) -> GradientOverlayStyle {
        let (overlay, size, stroke) = (&cx.tokens.color.overlay, &cx.tokens.size, &cx.tokens.stroke);
        GradientOverlayStyle {
            metrics: Metrics::new().min_size(Vec2::splat(size.handle.md)),
            handle: cx.handle(),
            line: cx.color(overlay.handle_fill),
            halo: cx.color(overlay.halo),
            selected: cx.color(overlay.handle_stroke),
            backing: cx.color(overlay.handle_fill),
            dash: stroke.dash,
            gap: stroke.dash_gap,
            end_radius: size.handle.md,
            stop_radius: size.handle.sm,
            diamond_radius: size.handle.sm,
            tear_off: TEAR_OFF,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.handle.side)
    }

    fn bind(&mut self) -> Option<BindMut<'_, LineGradient>> {
        Some(self.value.reborrow())
    }

    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.declare_parts(cx);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(value)) = (canvas_space(&cx.env), cx.value().cloned()) else { return };
        let snap = canvas_snap(&cx.env);
        self.run_pointer(cx, &space, snap.as_ref(), &value);
        self.run_keys(cx, &value);
        self.report.selected = self.selection(cx);
        self.report.picked = cx.active_part().and_then(line_part);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(value)) = (canvas_space(&cx.env), cx.value()) else { return };
        self.paint_gradient(ui, cx, &space.view, value);
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(value)) = (canvas_space(&cx.env), cx.value()) else { return };
        let words = cx.words::<GradientLineWords>();
        let group = self.group_value(cx, value);
        access.value(group.clone());
        let line = access.node(LINE, Role::BUTTON);
        line.name = Name::Text(words.line.as_str().to_owned());
        line.value(group);
        let units = cx.env.units();
        let rulers = space.rulers();
        let end_value = |p: Point| {
            let (x, y) = (rulers.x.format_value(&units, p.x, rulers.zero.x), rulers.y.format_value(&units, p.y, rulers.zero.y));
            words.point_value.fill(&[("x", x.as_str()), ("y", y.as_str())])
        };
        for (key, name, point) in [(START, &words.start, value.line.start), (END, &words.end, value.line.end)] {
            let node = access.node(key, Role::BUTTON);
            node.name = Name::Text(name.as_str().to_owned());
            node.value(end_value(point));
        }
        let gradient_words = cx.words::<GradientWords>();
        let units = cx.env.units();
        let text =
            |marker: StopRef| marker_texts(&gradient_words, &units, self.rules.rows, &value.stops, marker, usize::from(self.location_decimals));
        for i in 0..value.stops.color.len() {
            let (name, shown) = text(StopRef::Color(i));
            let node = access.node(STOP.at(i as u64), Role::BUTTON);
            node.name = Name::Text(name);
            node.value(shown);
            if self.midpoints && midpoint_location(&value.stops, i).is_some() {
                let (name, shown) = text(StopRef::Midpoint(StopRow::Color, i));
                let node = access.node(MIDPOINT.at(i as u64), Role::BUTTON);
                node.name = Name::Text(name);
                node.value(shown);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GradientOverlayOutput {
        let Report { color_requested, picked, hovered, context, selected, .. } = self.report;
        GradientOverlayOutput { edit: cx.edit_response(), color_requested, picked, hovered, context, selected }
    }
}

/// A screen point as the line has it.
pub(super) fn screen_line(view: &craft_behavior::CanvasView, value: &LineGradient) -> (Pos2, Pos2) {
    (view.to_screen(value.line.start), view.to_screen(value.line.end))
}

impl GradientOverlay<'_> {
    fn paint_gradient(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, view: &craft_behavior::CanvasView, value: &LineGradient) {
        let style = &cx.style;
        let (start, end) = screen_line(view, value);
        let marked =
            |key, part: LinePart| cx.part(key).press.phase.is_drag() || (cx.is_focused() && cx.active_part().and_then(line_part) == Some(part));
        let selected = self.report.selected;
        let draw = ui.draw();
        draw.clipped(cx.rect, |draw| {
            let outlined = |shape: &PaintShape| {
                draw.stroke(shape, StrokeWidth::Thick, style.halo);
                draw.stroke(shape, StrokeWidth::Medium, style.line);
            };
            let d = end - start;
            match self.extent {
                GradientExtent::None => {}
                GradientExtent::Circle => outlined(&PaintShape::Circle(start, d.length())),
                GradientExtent::Square => {
                    let turned = vec2(-d.y, d.x);
                    outlined(&PaintShape::Polygon(vec![start + d, start + turned, start - d, start - turned]));
                }
                GradientExtent::Mirror => draw.dashed(&[start, start - d], Dashes::new(style.dash, style.gap), StrokeWidth::Medium, style.line),
            }
            outlined(&PaintShape::Path(vec![start, end]));
            if self.midpoints {
                for i in 0..value.stops.color.len() {
                    let Some(t) = midpoint_location(&value.stops, i) else { continue };
                    let at = at_screen(start, end, t, self.reverse);
                    let r = style.diamond_radius;
                    let diamond = PaintShape::Polygon(vec![at + vec2(r, 0.0), at + vec2(0.0, r), at - vec2(r, 0.0), at - vec2(0.0, r)]);
                    let fill = if selected == Some(StopRef::Midpoint(StopRow::Color, i)) {
                        style.selected
                    } else if cx.part_if_declared(MIDPOINT.at(i as u64)).is_some() {
                        cx.resolve_part(MIDPOINT.at(i as u64), &style.handle.fill)
                    } else {
                        cx.resolve(&style.handle.fill)
                    };
                    draw.fill(&diamond, fill);
                    draw.stroke(&diamond, StrokeWidth::Thin, style.halo);
                }
            }
            for (at, key, part) in [(start, START, LinePart::Start), (end, END, LinePart::End)] {
                let ring = PaintShape::Circle(at, style.end_radius);
                draw.stroke(&ring, StrokeWidth::Thick, style.halo);
                draw.stroke(&ring, StrokeWidth::Medium, if marked(key, part) { style.selected } else { style.line });
            }
            for (i, stop) in value.stops.color.iter().enumerate() {
                if self.report.torn && cx.part(STOP.at(i as u64)).press.phase.is_drag() {
                    continue;
                }
                let at = at_screen(start, end, stop.location, self.reverse);
                let color = Color32::from(stop.color);
                let disc = PaintShape::Circle(at, style.stop_radius);
                draw.stroke(&PaintShape::Circle(at, style.stop_radius), StrokeWidth::Thick, style.halo);
                match self.rules.rows {
                    StopRows::Combined => {
                        draw.fill(&disc, style.backing);
                        draw.data_fill(&disc, DataColor::new(color));
                    }
                    _ => draw.data_fill(&disc, DataColor::new(Color32::from_rgb(color.r(), color.g(), color.b()))),
                }
                let ring = if selected == Some(StopRef::Color(i)) { style.selected } else { style.line };
                draw.stroke(&disc, StrokeWidth::Medium, ring);
            }
        });
    }
}
