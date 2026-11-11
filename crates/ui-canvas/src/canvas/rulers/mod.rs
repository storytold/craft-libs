//! `Rulers`: the strips along the top and left of a canvas and the corner that holds the zero point, a canvas
//! control (20.6). The strips and the corner are parts at handle priority in the canvas's arena. A drag from a strip
//! reports a guide being placed; a drag from the corner moves the zero point.

#[cfg(feature = "testing")]
mod stories;
mod ticks;

pub use ticks::{INCH_EIGHTHS, INCH_SIXTEENTHS, RulerTick, StepLadder, TickRule};

pub(crate) use self::ticks::LABEL_STEP_MIN;
use super::words::CanvasWords;
use super::{CanvasRulers, CanvasSpace, GuideDrag, RulerText, Snap, canvas_snap, canvas_space, context_menu, readout};
use crate::contract::overlay::Anchor;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, Focus as _, MeasureCx, Name, Outcome, OverlayUi, Overlays as _,
    PaintUi, PartsCx, Pointer as _, Props as _, Role, Styled as _, WordSetCx as _,
};
use crate::menu::Menu;
use crate::primitives::{PaintShape, StrokeWidth, TextRun, Tick, TickEdge, TickRank};
use crate::runtime::env::{EnvView, Label, UnitKind};
use crate::runtime::focus::KeyUse;
use crate::runtime::memory::Remember as _;
use crate::style::{ComponentStyle, RuleKind, StyleColor, StyleCx, TextLook, TypeRole, UiTokens};
use craft_behavior::geometry::{across_axis, coordinate_across, motion_across};
use craft_behavior::kurbo::Point;
use craft_behavior::{
    Axis, CanvasView, CursorIcon, GesturePhase, HitLevel, HitPriority, Metrics, PartKey, PointerButton, PressPhase, RulerUnits, Sense, resize_cursor,
};
use craft_keymap::Mods;
use craft_theme::Stateful;
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::ops::{Deref, Range};

/// The strips: `STRIP.at(0)` along the top, `STRIP.at(1)` along the left.
const STRIP: PartKey = PartKey::new("strip");
const CORNER: PartKey = PartKey::new("corner");

/// The distance an arrow key moves the zero point of a focused corner or a selected guide, in document units; Shift
/// moves it by `NUDGE_FAR`.
pub const NUDGE: f64 = 1.0;
/// The distance Shift moves a nudge by, in document units.
pub const NUDGE_FAR: f64 = 10.0;

/// The zero point is written to this many decimal places' worth of document units.
const ZERO_ROUND: f64 = 1000.0;

/// The part of the rulers a pointer is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RulerPart {
    /// The top ruler, which measures the document's x axis.
    Horizontal,
    /// The left ruler, which measures the document's y axis.
    Vertical,
    /// The square where the rulers meet.
    Corner,
}

impl RulerPart {
    fn part(self) -> craft_behavior::PartRef {
        match self {
            RulerPart::Horizontal => STRIP.at(0),
            RulerPart::Vertical => STRIP.at(1),
            RulerPart::Corner => CORNER.into(),
        }
    }

    /// The axis a guide dragged out of this strip runs along: the top ruler makes horizontal guides.
    fn guide_along(self) -> Axis {
        match self {
            RulerPart::Vertical => Axis::Y,
            _ => Axis::X,
        }
    }
}

/// The commands of the rulers. They are app commands: the app that owns the zero point and the unit choice handles a
/// key bound to one. Reset Zero Point writes the same zero as the rulers' own reset (a double click on the corner, the
/// units menu's row), which goes through the zero bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "canvasRulers")]
#[non_exhaustive]
pub enum RulerCommands {
    /// Put the zero point back at the document's origin.
    #[command(label = "Reset Zero Point")]
    ResetZero,
    /// Measure the rulers in pixels.
    #[command(label = "Pixels", kind = "radio", group = "rulerUnit")]
    UnitPixels,
    /// Measure the rulers in points.
    #[command(label = "Points", kind = "radio", group = "rulerUnit")]
    UnitPoints,
    /// Measure the rulers in picas.
    #[command(label = "Picas", kind = "radio", group = "rulerUnit")]
    UnitPicas,
    /// Measure the rulers in inches.
    #[command(label = "Inches", kind = "radio", group = "rulerUnit")]
    UnitInches,
    /// Measure the rulers in centimetres.
    #[command(label = "Centimetres", kind = "radio", group = "rulerUnit")]
    UnitCentimetres,
    /// Measure the rulers in millimetres.
    #[command(label = "Millimetres", kind = "radio", group = "rulerUnit")]
    UnitMillimetres,
    /// Measure the rulers in percent.
    #[command(label = "Percent", kind = "radio", group = "rulerUnit")]
    UnitPercent,
}

/// The margin bands of the two strips, as document ranges along the x axis (the top strip) and the y axis (the left
/// strip): a page view's margins, a text frame's insets.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct RulerMargins {
    /// Along the x axis.
    pub horizontal: Vec<Range<f64>>,
    /// Along the y axis.
    pub vertical: Vec<Range<f64>>,
}

impl RulerMargins {
    /// Margins `horizontal` along x and `vertical` along y. A reversed range is read the right way round.
    pub fn new(horizontal: impl IntoIterator<Item = Range<f64>>, vertical: impl IntoIterator<Item = Range<f64>>) -> Self {
        RulerMargins { horizontal: horizontal.into_iter().collect(), vertical: vertical.into_iter().collect() }
    }
}

/// The style of the rulers.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RulersStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The strips' thickness and the corner's side.
    thickness: f32,
    /// Between a tick and its label.
    pad: f32,
    fill: StyleColor,
    margin: StyleColor,
    label: TextLook,
    marker: StyleColor,
}

/// The type role of the tick labels.
const LABEL_ROLE: TypeRole = TypeRole::Caption;

/// A strip is as thick as its tick labels need, and no thinner than a small icon.
fn thickness(tokens: &UiTokens) -> f32 {
    tokens.size.icon.sm.max(tokens.typography.get(LABEL_ROLE).line_height)
}

/// The top strip, the left strip and the corner inside `rect`.
fn strips(rect: Rect, thickness: f32) -> (Rect, Rect, Rect) {
    let top = Rect::from_min_max(Pos2::new(rect.left() + thickness, rect.top()), Pos2::new(rect.right(), rect.top() + thickness));
    let left = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + thickness), Pos2::new(rect.left() + thickness, rect.bottom()));
    (top, left, Rect::from_min_size(rect.min, Vec2::splat(thickness)))
}

/// `p` written to the zero point's precision.
fn round_zero(p: Point) -> Point {
    let round = |v: f64| (v * ZERO_ROUND).round() / ZERO_ROUND;
    Point::new(round(p.x), round(p.y))
}

/// The place the units menu hangs from.
#[derive(Clone, Copy, Default)]
struct MenuAt(Option<Pos2>);

/// What a units menu row does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuRow {
    Unit(UnitKind),
    ResetZero,
}

/// The rulers of a canvas: ticks and labels counted from a zero point, level whatever the view's rotation, a pointer
/// marker, the guide drag-out and the zero point's corner.
///
/// - Show it with `show_at` over the canvas's rect inside the `Canvas` body, and lay the view out in
///   [`Rulers::content_rect`] of that rect. The units come from the canvas ([`Canvas::rulers`](super::Canvas::rulers));
///   the rulers hold the zero point's edit.
/// - A drag on the top ruler reports a horizontal guide in [`RulersOutput::guide`], one on the left ruler a vertical
///   guide; Alt turns it, Shift snaps it to the ticks, and a release over a ruler or off the document reports no
///   position, which removes the guide.
/// - A drag out of the corner moves the zero point: cross hairs show where it lands, Shift snaps to the ticks
///   (otherwise the canvas's [`Snap`]), dropped back on the corner it stays, Esc cancels. A double click
///   on the corner and the menu's Reset Zero Point put the zero point back at the document's origin, one edit through the zero bind. The arrow keys move the focused corner. [`origin_locked`](Rulers::origin_locked)
///   turns the drag and the reset off.
/// - A secondary click on a strip or the corner, or the Menu key or Shift+F10 on the focused corner, opens the units
///   menu when [`units`](Rulers::units) lists units, and reports [`RulersOutput::context`] either way. The menu's rows are also the [`RulerCommands`].
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control, content_painter, overlay)]
pub struct Rulers<'a> {
    #[common]
    common: Common,
    zero: Bind<'a, Point>,
    /// Whether the zero point is locked: no drag and no reset.
    #[prop]
    origin_locked: bool,
    /// Whether the rulers take the pointer; `false` paints them only.
    #[prop]
    interactive: bool,
    /// The margin bands of the strips.
    #[prop]
    margins: RulerMargins,
    /// The units the secondary click's menu offers; the unit of the top ruler is checked. Without it the rulers open
    /// no menu.
    #[prop]
    units: Option<&'a [UnitKind]>,
    guide: Option<GuideDrag>,
    picked: Option<UnitKind>,
    context: Option<RulerPart>,
}

impl<'a> Rulers<'a> {
    /// Rulers named `label` that edit the zero point `zero` (document units). The units and the zero they count from
    /// are the canvas's ([`Canvas::rulers`](super::Canvas::rulers)); the app passes the same zero to both.
    pub fn new(label: impl Into<Label>, zero: Bind<'a, Point>) -> Self {
        Rulers {
            common: Common::new(),
            zero,
            origin_locked: false,
            interactive: true,
            margins: RulerMargins::default(),
            units: None,
            guide: None,
            picked: None,
            context: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// The part of the canvas's rect `full` inside the strips: where the view lies.
    pub fn content_rect(cx: &StyleCx<'_>, full: Rect) -> Rect {
        Self::content_rect_of(full, thickness(cx.tokens))
    }

    /// The ticks of one ruler over `values` (ruler units from the zero point): the one tick rule of every ruler, so a
    /// text ruler or a page view's ruler shows the same ticks as the canvas's. One labelled step beyond each end is
    /// included. `zoom` is screen points per document unit along the axis. Empty when the units are unusable.
    pub fn ticks(units: &RulerUnits, rule: TickRule<'_>, zoom: f64, values: std::ops::RangeInclusive<f64>) -> Vec<RulerTick> {
        ticks::ticks(units, rule, zoom, values)
    }

    fn zero_of(value: Option<&Point>) -> Point {
        value.copied().unwrap_or(Point::ZERO)
    }

    /// The units the canvas published, whatever the pass.
    fn published(env: &EnvView<'_>) -> CanvasRulers {
        canvas_space(env).map(|space| space.rulers()).unwrap_or_default()
    }

    /// Where the zero point lands for the pointer at `pos`: on the nearest tick of the rulers with Shift, else where
    /// the canvas's snap puts it.
    fn landing(space: &CanvasSpace, snap: Option<&Snap>, pos: Pos2, mods: Mods, zero: Point) -> Point {
        let doc = space.view.to_doc(pos);
        let rulers = space.rulers();
        let (x, y) = if mods.shift {
            (
                rulers.x.units.snap_to_tick(doc.x, zero.x, space.view.zoom_x(), LABEL_STEP_MIN),
                rulers.y.units.snap_to_tick(doc.y, zero.y, space.view.zoom, LABEL_STEP_MIN),
            )
        } else if let Some(snap) = snap {
            (snap.apply(Axis::Y, doc.x, mods), snap.apply(Axis::X, doc.y, mods))
        } else {
            (doc.x, doc.y)
        };
        round_zero(Point::new(x, y))
    }

    /// The guide a drag from `part` reports in this pass, if it runs one.
    fn drag_out(&self, part: RulerPart, cx: &Cx<'_, Self>, space: &CanvasSpace, zero: Point, content: Rect) -> Option<GuideDrag> {
        let press = cx.part(part.part()).press;
        let start = press.phase.start().filter(|start| start.button == PointerButton::Primary)?;
        let (phase, pos) = match press.phase {
            PressPhase::DragStart { pos, .. } => (GesturePhase::Begin, pos),
            PressPhase::Drag { pos, .. } => (GesturePhase::Update, pos),
            PressPhase::Release { pos, click: false, .. } => (GesturePhase::Commit, pos),
            PressPhase::Cancel { .. } => (GesturePhase::Cancel, start.origin),
            _ => return None,
        };
        let along = if press.mods.alt { across_axis(part.guide_along()) } else { part.guide_along() };
        let at = (phase != GesturePhase::Cancel && content.contains(pos)).then(|| {
            let across = coordinate_across(along, space.view.to_doc(pos));
            if press.mods.shift {
                let (text, _) = space.rulers().across(along);
                let (origin, zoom) = match along {
                    Axis::X => (zero.y, space.view.zoom),
                    _ => (zero.x, space.view.zoom_x()),
                };
                text.units.snap_to_tick(across, origin, zoom, LABEL_STEP_MIN)
            } else {
                across
            }
        });
        Some(GuideDrag::new_guide(along, phase, at, press.mods))
    }

    /// The ticks of one ruler along `strip`, each with its screen coordinate along the strip.
    fn axis_ticks(text: &RulerText, level: &CanvasView, strip: Rect, part: RulerPart, zero: Point) -> Vec<(RulerTick, f32)> {
        let vertical = part == RulerPart::Vertical;
        let units = &text.units;
        if !(units.per_doc.is_finite() && units.per_doc > 0.0) {
            return Vec::new();
        }
        let value_at = |pos: Pos2| {
            let doc = level.to_doc(pos);
            (if vertical { doc.y - zero.y } else { doc.x - zero.x }) * units.per_doc
        };
        let screen_at = |value: f64| {
            let along = value / units.per_doc;
            if vertical { level.to_screen(Point::new(zero.x, zero.y + along)).y } else { level.to_screen(Point::new(zero.x + along, zero.y)).x }
        };
        let (a, b) = if vertical { (strip.left_top(), strip.left_bottom()) } else { (strip.left_top(), strip.right_top()) };
        let (v0, v1) = (value_at(a), value_at(b));
        let zoom = if vertical { level.zoom } else { level.zoom_x() };
        ticks::ticks(units, TickRule::Metric, zoom, v0.min(v1)..=v0.max(v1)).into_iter().map(|tick| (tick, screen_at(tick.value))).collect()
    }
}

impl Component for Rulers<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = RulersStyle;
    type Output = RulersOutput;

    fn style(cx: &StyleCx<'_>, _: RulersStyleKey) -> RulersStyle {
        let c = &cx.tokens.color;
        RulersStyle {
            metrics: Metrics::new(),
            thickness: thickness(cx.tokens),
            pad: cx.tokens.space.gap.tight,
            fill: cx.color(c.surface.chrome),
            margin: cx.color(c.surface.sunken),
            label: TextLook::new(LABEL_ROLE, Stateful::new(cx.color(c.text.muted))),
            marker: cx.color(c.text.normal),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.thickness)
    }

    fn bind(&mut self) -> Option<BindMut<'_, Point>> {
        Some(self.zero.reborrow())
    }

    /// The strips and the corner: parts at handle priority. The corner is a Tab stop that the arrow keys move; a
    /// strip's keyboard path is the guide dialog.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (top, left, corner) = strips(rect, cx.style.thickness);
        let space = canvas_space(&cx.env);
        let handle = HitPriority::new(HitLevel::Handle, 0);
        for (part, strip) in [(RulerPart::Horizontal, top), (RulerPart::Vertical, left)] {
            let declared = cx.part(part.part(), strip).priority(handle).pointer_only("the New Guide... command places a guide by number");
            if self.interactive {
                let declared = declared.sense(Sense::DRAG | Sense::CLICK);
                if let Some(space) = space {
                    declared.cursor(resize_cursor(&space.view, motion_across(part.guide_along())));
                }
            }
        }
        let declared = cx.part(CORNER, corner).priority(handle);
        if self.interactive {
            let sense = if self.origin_locked { Sense::CLICK | Sense::CONTEXT_CLICK } else { Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS };
            let declared = declared.sense(sense);
            if self.origin_locked {
                declared.pointer_only("the zero point is locked");
            } else {
                declared.cursor(CursorIcon::Crosshair).focusable();
            }
        } else {
            declared.pointer_only("the rulers only show");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let (_, _, corner) = strips(cx.rect, cx.style.thickness);
        let content = Rulers::content_rect_of(cx.rect, cx.style.thickness);
        let zero = Self::zero_of(cx.value());
        let popup = cx.popup("menu");

        for part in [RulerPart::Horizontal, RulerPart::Vertical] {
            if let Some(drag) = self.drag_out(part, cx, &space, zero, content) {
                self.guide = Some(drag);
            }
        }

        let press = cx.part(CORNER).press;
        let started = press.phase.start().filter(|start| start.button == PointerButton::Primary);
        if started.is_some() && !self.origin_locked {
            match press.phase {
                PressPhase::DragStart { .. } => {
                    *cx.press_slot::<Point>() = Some(zero);
                    cx.begin();
                }
                PressPhase::Drag { pos, .. } => {
                    let start = cx.press_slot::<Point>().unwrap_or(zero);
                    let at = if corner.contains(pos) { start } else { Self::landing(&space, canvas_snap(&cx.env).as_ref(), pos, press.mods, zero) };
                    cx.propose(at);
                }
                PressPhase::Release { click: false, .. } => cx.commit(),
                _ => {}
            }
            if press.double_pressed() {
                cx.set(Point::ZERO);
            }
        }
        let focused = cx.active_part().is_some_and(|part| part.key == CORNER);
        if focused
            && !self.origin_locked
            && self.interactive
            && let Some((direction, mods)) = cx.intents().take_nudge()
        {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            cx.set(round_zero(Point::new(zero.x + f64::from(direction.x) * step, zero.y + f64::from(direction.y) * step)));
        }

        let opened = [RulerPart::Horizontal, RulerPart::Vertical, RulerPart::Corner]
            .into_iter()
            .find_map(|part| context_menu(cx, part.part()).map(|at| (part, at)));
        if let Some((part, at)) = opened.filter(|_| self.interactive) {
            self.context = Some(part);
            if self.units.is_some() {
                cx.memory::<MenuAt>().0 = Some(at);
                popup.open(cx);
            }
        }
    }

    /// The units menu: the units with the top ruler's checked, then Reset Zero Point.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let Some(units) = self.units else { return };
        let popup = cx.popup("menu");
        let numbers = cx.env.units();
        let words = cx.words::<CanvasWords>();
        let shown = Self::published(&cx.env).x.kind;
        let mut entries: Vec<Entry<MenuRow>> =
            units.iter().map(|&kind| Entry::new(MenuRow::Unit(kind), Label::from(numbers.symbol(kind))).checked(kind == shown)).collect();
        entries.push(Entry::separator());
        entries.push(Entry::new(MenuRow::ResetZero, Label::from(words.ruler_reset_zero.as_str().to_owned())).enabled(!self.origin_locked));
        let name = Label::from(words.ruler_menu.as_str().to_owned());
        let at = cx.memory::<MenuAt>().0.unwrap_or(cx.rect.min);
        let layer = crate::menu::layer(false).anchor(Anchor::Point(at));
        if let Some(pick) = ui.show(popup, layer, |ui, _| Menu::inline(name.clone()).pick(ui, &entries)) {
            match pick.picked {
                Some(MenuRow::Unit(kind)) => self.picked = Some(kind),
                Some(MenuRow::ResetZero) => cx.set(Point::ZERO),
                None => {}
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let (top, left, corner) = strips(cx.rect, style.thickness);
        let zero = Self::zero_of(cx.value());
        let space = canvas_space(&cx.env);
        let draw = ui.draw();
        let rect = |rect: Rect| PaintShape::Rect(rect, CornerRadius::ZERO);
        let dragging = matches!(cx.gesture(), Some(GesturePhase::Begin | GesturePhase::Update));

        if let Some(space) = space.filter(|_| dragging) {
            let visible = space.view.visible();
            for (from, to) in
                [(Point::new(zero.x, visible.y0), Point::new(zero.x, visible.y1)), (Point::new(visible.x0, zero.y), Point::new(visible.x1, zero.y))]
            {
                draw.stroke(&PaintShape::Path(vec![space.view.to_screen(from), space.view.to_screen(to)]), StrokeWidth::Thin, style.marker);
            }
            if let Some(at) = crate::runtime::pointer::frame(&cx.core.ctx).pos {
                let numbers = cx.env.units();
                let rulers = space.rulers();
                let rows = [
                    (Label::from("X"), rulers.x.format_value(&numbers, zero.x, 0.0)),
                    (Label::from("Y"), rulers.y.format_value(&numbers, zero.y, 0.0)),
                ];
                readout(cx, at, &rows);
            }
        }

        draw.fill(&rect(top), style.fill);
        draw.fill(&rect(left), style.fill);

        let Some(space) = space else {
            draw.fill(&rect(corner), style.fill);
            return;
        };
        let level = space.view.level(zero);
        let rulers = space.rulers();
        let text_of = |part: RulerPart| if part == RulerPart::Vertical { rulers.y } else { rulers.x };
        for (part, strip, spans) in [(RulerPart::Horizontal, top, &self.margins.horizontal), (RulerPart::Vertical, left, &self.margins.vertical)] {
            let vertical = part == RulerPart::Vertical;
            for span in spans {
                let at = |along: f64| {
                    let p = if vertical { Point::new(zero.x, along) } else { Point::new(along, zero.y) };
                    let s = level.to_screen(p);
                    if vertical { s.y } else { s.x }
                };
                let (a, b) = (at(span.start), at(span.end));
                let (lo, hi) = (a.min(b), a.max(b));
                let band = if vertical {
                    Rect::from_min_max(Pos2::new(strip.left(), lo), Pos2::new(strip.right(), hi))
                } else {
                    Rect::from_min_max(Pos2::new(lo, strip.top()), Pos2::new(hi, strip.bottom()))
                };
                let band = band.intersect(strip);
                if band.is_positive() {
                    draw.fill(&rect(band), style.margin);
                }
            }
        }

        let painter = ui.content_painter(cx);
        let numbers = cx.env.units();
        let label = &style.label;
        let color = cx.resolve(&label.color);
        // A label is painted where the strip shows it: it overlaps the strip and stays inside the rulers' rect, so a
        // label that hangs off an end of the strip is left out.
        let shows = |strip: Rect, text: Rect| text.intersects(strip) && cx.rect.contains_rect(text);
        for (part, strip, edge) in [(RulerPart::Horizontal, top, TickEdge::Bottom), (RulerPart::Vertical, left, TickEdge::Right)] {
            let vertical = part == RulerPart::Vertical;
            let ticks = Self::axis_ticks(&text_of(part), &level, strip, part, zero);
            let marks: Vec<Tick> =
                ticks.iter().map(|(tick, at)| Tick::new(*at - if vertical { strip.top() } else { strip.left() }, tick.rank)).collect();
            painter.ticks(strip, edge, &marks);
            for (tick, at) in ticks.iter().filter(|(tick, _)| tick.rank == TickRank::Major) {
                let text = text_of(part).tick_label(&numbers, tick.value, tick.step);
                if vertical {
                    // One character per line, centred in the strip: a narrow "1" lines up with a "0".
                    let mut y = at + style.pad;
                    for ch in text.chars() {
                        let layout = draw.layout(&TextRun::new(ch.to_string()).role(label.role).color(color), f32::INFINITY);
                        let at = Pos2::new(strip.center().x - layout.size().x / 2.0, y);
                        if shows(strip, Rect::from_min_size(at, layout.size())) {
                            draw.galley(at, &layout);
                        }
                        y += layout.size().y;
                    }
                } else {
                    let layout = draw.layout(&TextRun::new(text).role(label.role).color(color), f32::INFINITY);
                    let at = Pos2::new(at + style.pad, strip.top());
                    if shows(strip, Rect::from_min_size(at, layout.size())) {
                        draw.galley(at, &layout);
                    }
                }
            }
        }

        if let Some(at) = crate::runtime::pointer::frame(&cx.core.ctx).pos.filter(|at| cx.rect.contains(*at)) {
            for marker in [[Pos2::new(at.x, top.top()), Pos2::new(at.x, top.bottom())], [Pos2::new(left.left(), at.y), Pos2::new(left.right(), at.y)]]
            {
                draw.stroke(&PaintShape::Path(marker.to_vec()), StrokeWidth::Thin, style.marker);
            }
        }

        draw.fill(&rect(corner), style.fill);
        draw.rule(RuleKind::Divider, Pos2::new(top.left(), top.bottom()), Pos2::new(top.right(), top.bottom()));
        draw.rule(RuleKind::Divider, Pos2::new(left.right(), left.top()), Pos2::new(left.right(), left.bottom()));
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<CanvasWords>();
        let names = [
            (RulerPart::Horizontal, Role::GROUP, &words.ruler_horizontal),
            (RulerPart::Vertical, Role::GROUP, &words.ruler_vertical),
            (RulerPart::Corner, Role::BUTTON, &words.ruler_origin),
        ];
        for (part, role, name) in names {
            access.node(part.part(), role).name = Name::Text(name.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RulersOutput {
        let zero = cx.edit_response();
        let content = Rulers::content_rect_of(cx.rect, cx.style.thickness);
        RulersOutput { zero, guide: self.guide, unit_picked: self.picked, context: self.context, content }
    }
}

impl Rulers<'_> {
    fn content_rect_of(rect: Rect, thickness: f32) -> Rect {
        Rect::from_min_max(rect.min + Vec2::splat(thickness), rect.max)
    }
}

/// What the rulers report in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct RulersOutput {
    /// The zero point (document units): a corner drag's gesture, a nudge of the focused corner, or the reset to the
    /// document's origin (a double click on the corner, the menu's Reset Zero Point).
    pub zero: EditResponse<Point>,
    /// The guide a drag from a strip is placing in this pass. The app forwards it to the guides control; it ends
    /// with a `Commit` or a `Cancel`, and a position of `None` removes the guide.
    pub guide: Option<GuideDrag>,
    /// A unit picked from the rulers' own menu ([`Rulers::units`]): the app keeps it as the ruler unit.
    pub unit_picked: Option<UnitKind>,
    /// A strip or the corner was clicked with the secondary button, or the focused corner asked for its menu (the Menu
    /// key, Shift+F10).
    pub context: Option<RulerPart>,
    content: Rect,
}

impl RulersOutput {
    /// The part of the canvas's rect inside the strips: where the view lies.
    pub fn content_rect(&self) -> Rect {
        self.content
    }
}

impl Deref for RulersOutput {
    type Target = EditResponse<Point>;

    fn deref(&self) -> &Self::Target {
        &self.zero
    }
}

impl Outcome for RulersOutput {
    type Value = Point;

    fn response(&self) -> &Response {
        &self.zero.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.zero.story_facts()
    }
}
