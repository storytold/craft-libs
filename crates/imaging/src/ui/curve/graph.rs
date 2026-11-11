//! `CurveGraph`: the plot of a curve with a part per point, and the paint the read-only preview shares.

use super::model::{Curve, CurveRules, follow_corners};
use super::ramp::ToneRamp;
use super::words::CurveWords;
use crate::ui::histogram::{ChannelColor, HistogramData, HistogramInks, HistogramLook, PlotFrame, paint_bins, paint_frame};
use craft_behavior::{HitLevel, HitPriority, Input, Metrics, NavMove, PartKey, PointerButton, PressPhase, Proposal, Sense};
use craft_keymap::{Chord, Key, Mods};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::runtime::access::Name;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, Mix as _, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2, pos2, vec2};
use std::ops::Deref;
use std::rc::Rc;

/// The plot of the graph: the area inside the border, where levels `0..=levels` map to.
pub(super) const PLOT: PartKey = PartKey::new("plot");
/// A point: one accessible node each, `POINT.at(i)`.
const POINT: PartKey = PartKey::new("point");

/// The keys a focused graph keeps from the shortcut dispatcher while it shows points. `+` also arrives as Shift with `=`.
const PLUS: [Chord; 3] = [Chord::plain(Key::Plus), Chord::plain(Key::Equals), Chord::new(Mods::SHIFT, Key::Equals)];
const MINUS: Chord = Chord::plain(Key::Minus);
const INSERT: [Chord; 3] = [Chord::plain(Key::Insert), Chord::new(Mods::CMD, Key::Enter), Chord::new(Mods::CTRL, Key::Enter)];
const DELETE: [Chord; 2] = [Chord::plain(Key::Delete), Chord::plain(Key::Backspace)];

/// Levels one arrow key moves a point, and with Shift.
const NUDGE: f32 = 1.0;
const NUDGE_LARGE: f32 = 10.0;

/// One channel of a curve dialog: its name and colour.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveChannel {
    label: Label,
    color: ChannelColor,
}

impl CurveChannel {
    /// A channel called `label`, drawn in `color`.
    pub fn new(label: impl Into<Label>, color: ChannelColor) -> Self {
        CurveChannel { label: label.into(), color }
    }

    /// The channel's colour.
    pub fn color(&self) -> ChannelColor {
        self.color
    }
}

type CurveSamplerFn<'a> = Rc<dyn Fn(&[[f32; 2]]) -> Vec<f32> + 'a>;

/// A host's interpolation: the output level of every input level `0..=levels` through the points. The default is a
/// monotone cubic ([`Curve::monotone`]).
#[derive(Clone)]
pub struct CurveSampler<'a>(CurveSamplerFn<'a>);

impl<'a> CurveSampler<'a> {
    /// The interpolation `sample`.
    pub fn new(sample: impl Fn(&[[f32; 2]]) -> Vec<f32> + 'a) -> Self {
        CurveSampler(Rc::new(sample))
    }

    /// The monotone cubic over `levels + 1` outputs.
    pub fn monotone(levels: u32) -> CurveSampler<'static> {
        let levels = levels.max(1) as f32;
        CurveSampler(Rc::new(move |points| Curve::monotone(points, levels)))
    }
}

/// The output of every input level of `curve`, through its sampler or the monotone cubic; a drawn curve as it is.
pub(super) fn outputs(curve: &Curve, sampler: Option<&CurveSampler<'_>>, levels: f32) -> Vec<f32> {
    match curve {
        Curve::Points { points, corners } => samples(points, corners, sampler, levels),
        Curve::Map(outputs) => outputs.clone(),
    }
}

fn samples(points: &[[f32; 2]], corners: &[bool], sampler: Option<&CurveSampler<'_>>, levels: f32) -> Vec<f32> {
    Curve::through(points, corners, |points| match sampler {
        Some(sampler) => (sampler.0)(points),
        None => Curve::monotone(points, levels),
    })
}

/// Levels to screen and back, over the value area.
#[derive(Clone, Copy, Debug)]
pub(super) struct Map {
    pub area: Rect,
    pub levels: f32,
}

impl Map {
    fn screen(&self, q: [f32; 2]) -> Pos2 {
        let a = self.area;
        pos2(a.left() + q[0] / self.levels * a.width(), a.bottom() - q[1] / self.levels * a.height())
    }

    /// The levels under `p`, clamped to the curve's range.
    fn value_at(&self, p: Pos2) -> [f32; 2] {
        let a = self.area;
        let x = (p.x - a.left()) / a.width().max(1.0) * self.levels;
        let y = (a.bottom() - p.y) / a.height().max(1.0) * self.levels;
        [x.clamp(0.0, self.levels), y.clamp(0.0, self.levels)]
    }

    /// The point of `points` nearest `pos` within `radius`.
    fn hit(&self, points: &[[f32; 2]], pos: Pos2, radius: f32) -> Option<usize> {
        points
            .iter()
            .enumerate()
            .map(|(i, &q)| (i, self.screen(q).distance(pos)))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

/// The look of a plot: the frame, the histogram inks, the grid and the curves per channel. The graph and the preview
/// have one.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotLook {
    frame: PlotFrame,
    inks: HistogramInks,
    grid: StyleColor,
    baseline: StyleColor,
    /// The curve in each channel's colour, in `ChannelColor::ALL` order.
    curves: [StyleColor; 7],
    /// The curves of the other channels: each colour faded towards the plot.
    others: [StyleColor; 7],
    /// How far in from the border the value area starts.
    pad: f32,
}

/// Where a channel colour sits in the per-channel arrays.
fn slot(color: ChannelColor) -> usize {
    ChannelColor::ALL.iter().position(|c| *c == color).unwrap_or(0)
}

/// Share of the plot colour the other channels' curves take on.
const OTHER_FADE: f32 = 0.2;

impl PlotLook {
    pub(super) fn new(cx: &StyleCx<'_>) -> Self {
        let colors = &cx.tokens.color;
        let plot = cx.color(colors.surface.sunken);
        let curves = ChannelColor::ALL.map(|c| c.curve(cx));
        PlotLook {
            frame: PlotFrame::new(cx),
            inks: HistogramInks::new(cx),
            grid: cx.color(colors.line.divider),
            baseline: cx.color(colors.line.control_edge),
            curves,
            others: curves.map(|c| c.mix(plot, OTHER_FADE)),
            pad: cx.tokens.space.gap.inline,
        }
    }

    pub(super) fn pad(&self) -> f32 {
        self.pad
    }

    pub(super) fn curve(&self, color: ChannelColor) -> StyleColor {
        self.curves[slot(color)]
    }

    /// The frame, the backdrop, the quarter grid and the diagonal of the plot `plot` (the value area is inset by the pad).
    pub(super) fn backdrop(
        &self,
        ui: &PaintUi<'_>,
        painter: &craft_ui::primitives::ContentPainterCx<'_>,
        plot: Rect,
        histogram: Option<&HistogramData>,
    ) {
        paint_frame(painter, plot, &self.frame, true);
        let area = plot.shrink(self.pad);
        if let Some(data) = histogram {
            paint_bins(painter, area, data, HistogramLook::Backdrop, &self.inks);
        }
        let draw = ui.draw();
        for i in 1..4 {
            let f = i as f32 / 4.0;
            let x = area.left() + f * area.width();
            let y = area.top() + f * area.height();
            line(&draw, [pos2(x, area.top()), pos2(x, area.bottom())], self.grid);
            line(&draw, [pos2(area.left(), y), pos2(area.right(), y)], self.grid);
        }
        line(&draw, [area.left_bottom(), area.right_top()], self.baseline);
    }
}

fn line(draw: &Draw<'_>, ends: [Pos2; 2], color: StyleColor) {
    draw.stroke(&PaintShape::Path(ends.to_vec()), StrokeWidth::Thin, color);
}

/// The curve through `outputs` (the output at each input level, evenly spaced over the range) over the value area.
pub(super) fn paint_line(draw: &Draw<'_>, outputs: &[f32], map: Map, color: StyleColor) {
    let last = outputs.len().saturating_sub(1).max(1) as f32;
    let path: Vec<Pos2> = outputs.iter().enumerate().map(|(k, &o)| map.screen([k as f32 / last * map.levels, o])).collect();
    if path.len() >= 2 {
        draw.stroke(&PaintShape::Path(path), StrokeWidth::Medium, color);
    }
}

/// The style of the graph.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CurveGraphStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    look: PlotLook,
    /// The least and the greatest side of the graph, tone bars included; between them it takes the room it is given.
    least: f32,
    most: f32,
    /// Thickness of the tone bars and the gap between them and the plot.
    bar: f32,
    bar_gap: f32,
    /// Radius around a point that takes a press.
    hit: f32,
    /// How far outside the plot a dragged interior point is removed.
    drag_off: f32,
    /// The side of a point's handle.
    handle: f32,
    point_fill: StyleColor,
    point_edge: StyleColor,
    chosen_fill: StyleColor,
    halo: StyleColor,
    /// The light end of the tone bars per channel, in `ChannelColor::ALL` order. Image data, so a data colour.
    bar_ends: [DataColor; 7],
}

fn bar_end(color: ChannelColor, cx: &StyleCx<'_>) -> DataColor {
    let data = &cx.tokens.color.data;
    let rgba = match color {
        ChannelColor::Red => data.red,
        ChannelColor::Green => data.green,
        ChannelColor::Blue => data.blue,
        ChannelColor::Cyan => data.cyan,
        ChannelColor::Magenta => data.magenta,
        ChannelColor::Yellow => data.yellow,
        _ => return DataColor::new(Color32::from_gray(u8::MAX)),
    };
    let [r, g, b, a] = rgba.0;
    DataColor::new(Color32::from_rgba_unmultiplied(r, g, b, a))
}

/// The squares the graph is laid out in.
#[derive(Clone, Copy, Debug)]
struct Frame {
    plot: Rect,
    map: Map,
    under: Rect,
    left: Rect,
}

/// The pointer's hold on a point.
#[derive(Clone, Copy, Debug, Default)]
struct Drag {
    /// The point dragged; `None` while it is off the graph.
    index: Option<usize>,
    /// From the pointer to the point, in levels, so a press doesn't make the point jump.
    offset: [f32; 2],
    /// The selection before the press, restored when the gesture is cancelled.
    before: Option<usize>,
    /// Whether the point is a corner point, kept while it is off the graph so it comes back as one.
    corner: bool,
    /// Whether the gesture has begun.
    begun: bool,
}

/// What the graph keeps between passes.
#[derive(Clone, Debug, Default)]
struct Held {
    drag: Option<Drag>,
    /// The last level a pencil stroke under way drew to.
    stroke: Option<[f32; 2]>,
    /// The outputs a pencil stroke under way draws into.
    outputs: Vec<f32>,
    /// Where the previous pencil stroke ended and the curve it left: Shift-press continues from it.
    end: Option<([f32; 2], Curve)>,
}

/// The graph's output: the curve's edit and the selection after the pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct CurveGraphOutput {
    /// The curve's edit.
    pub edit: EditResponse<Curve>,
    /// The selected point after the pass.
    pub selected: Option<usize>,
}

impl Outcome for CurveGraphOutput {
    type Value = Curve;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl Deref for CurveGraphOutput {
    type Target = EditResponse<Curve>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// The graph of a curve editor: the plot with its grid, the diagonal, the curve, the other channels' curves, the
/// histogram backdrop, the tone bars and the points. A press on a point drags it, a press on the plot adds a point
/// there, and the keys select, nudge, add and delete points. Pencil mode draws one output per input level. The
/// selection comes in as a prop and goes out in the output.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct CurveGraph<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Curve>,
    /// The dialog's channels.
    #[prop]
    channels: &'a [CurveChannel],
    /// The active channel, an index into the channels.
    #[prop]
    channel: usize,
    levels: f32,
    /// The most points the curve holds.
    #[prop(quantity = "points")]
    max_points: usize,
    /// The interpolation of the curve through its points.
    #[prop]
    sampler: Option<CurveSampler<'a>>,
    /// Whether the tone bars show under the plot and beside it.
    #[prop]
    bars: bool,
    /// Pencil mode: a drag draws one output per input level.
    #[prop(flag)]
    pencil: bool,
    /// Points can be corner points.
    #[prop(flag)]
    corners: bool,
    /// Accessible values in percent of the levels.
    #[prop(flag)]
    percent: bool,
    /// The active channel's histogram, drawn behind the curve.
    #[prop]
    histogram: Option<&'a HistogramData>,
    /// The curve of every channel, in the order of the channels.
    #[prop]
    overlays: &'a [Curve],
    /// The selected point.
    #[prop]
    selection: Option<usize>,
    /// The selection after `update`, for the output.
    chosen: Option<usize>,
}

impl<'a> CurveGraph<'a> {
    /// A graph named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Curve>) -> Self {
        CurveGraph {
            common: Common::new(),
            value,
            channels: &[],
            channel: 0,
            levels: 255.0,
            max_points: 16,
            sampler: None,
            bars: true,
            pencil: false,
            corners: false,
            percent: false,
            histogram: None,
            overlays: &[],
            selection: None,
            chosen: None,
        }
        .label(label)
    }

    /// The top level of input and output.
    #[must_use]
    pub fn levels(mut self, levels: u32) -> Self {
        self.levels = levels.max(1) as f32;
        self
    }

    fn rules(&self) -> CurveRules {
        CurveRules { levels: self.levels, max_points: self.max_points.max(2) }
    }

    fn samples(&self, points: &[[f32; 2]], corners: &[bool]) -> Vec<f32> {
        samples(points, corners, self.sampler.as_ref(), self.levels)
    }

    fn color(&self) -> ChannelColor {
        self.channels.get(self.channel).map_or(ChannelColor::Neutral, CurveChannel::color)
    }

    /// The channels whose curves draw behind the active one: every channel with an overlay curve, unless it is the identity.
    fn other_channels(&self) -> Vec<usize> {
        (0..self.overlays.len().min(self.channels.len())).filter(|&i| i != self.channel && !self.overlays[i].is_identity(self.levels)).collect()
    }

    /// The squares of the graph in `rect`.
    fn frame(&self, rect: Rect, style: &CurveGraphStyle) -> Frame {
        let edge = if self.bars { style.bar + style.bar_gap } else { 0.0 };
        let plot = Rect::from_min_size(rect.min + vec2(edge, 0.0), (rect.size() - vec2(edge, edge)).max(Vec2::ZERO));
        let area = plot.shrink(style.look.pad);
        let under =
            Rect::from_min_max(pos2(area.left(), plot.bottom() + style.bar_gap), pos2(area.right(), plot.bottom() + style.bar_gap + style.bar));
        let left = Rect::from_min_max(pos2(plot.left() - style.bar_gap - style.bar, area.top()), pos2(plot.left() - style.bar_gap, area.bottom()));
        Frame { plot, map: Map { area, levels: self.levels }, under, left }
    }

    /// Whether the graph shows points: a point curve outside pencil mode.
    fn points_mode(&self, curve: Option<&Curve>) -> bool {
        !self.pencil && curve.is_some_and(|curve| curve.points().is_some())
    }

    /// The value text of a level as the fields write it: whole levels, or a whole percent.
    fn level_text(&self, cx: &Cx<'_, Self>, words: &CurveWords, level: f32) -> String {
        match self.percent {
            true => {
                let percent = (level / self.levels * 100.0).round();
                words.percent.fill(&[("n", cx.format().number(f64::from(percent), 0).as_str())])
            }
            false => cx.format().number(f64::from(level), 0),
        }
    }
}

/// The pointer's effect on the points while a point is dragged: the curve after the pointer moved to `pos`, or `None`
/// when nothing changed.
fn follow(rules: &CurveRules, map: Map, reach: f32, curve: &Curve, drag: &mut Drag, pos: Pos2, selected: &mut Option<usize>) -> Option<Curve> {
    let mut points = curve.points()?.to_vec();
    let mut flags = curve.corner_flags();
    let before = points.clone();
    let outside = !map.area.expand(reach).contains(pos);
    let at = map.value_at(pos);
    let target = [at[0] + drag.offset[0], at[1] + drag.offset[1]];
    let was = drag.index;
    let changed = match was {
        Some(i) if outside && CurveRules::can_delete(&points, i) => {
            drag.corner = flags.get(i).copied().unwrap_or(false);
            CurveRules::delete(&mut points, i);
            drag.index = None;
            *selected = None;
            true
        }
        Some(i) => rules.move_to(&mut points, i, target),
        None if !outside => {
            let count = points.len();
            let index = rules.insert(&mut points, target);
            *selected = index;
            drag.index = index;
            points.len() != count
        }
        None => false,
    };
    follow_corners(&before, &points, &mut flags);
    if was.is_none()
        && let Some(i) = drag.index
        && drag.corner
        && changed
        && let Some(flag) = flags.get_mut(i)
    {
        *flag = true;
    }
    changed.then(|| Curve::from_parts(points, flags))
}

impl Component for CurveGraph<'_> {
    const ROLE: Role = Role::GROUP;
    const INPUT: Input = Input::Pointer;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Neither).with_moves();
    type Style = CurveGraphStyle;
    type Output = CurveGraphOutput;

    fn style(cx: &StyleCx<'_>, _: CurveGraphStyleKey) -> CurveGraphStyle {
        let colors = &cx.tokens.color;
        let tokens = &cx.tokens;
        let least = tokens.size.plot.width.max(tokens.size.plot.height);
        CurveGraphStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            look: PlotLook::new(cx),
            least,
            most: tokens.size.plot.most.max(least),
            bar: tokens.size.icon.sm,
            bar_gap: tokens.space.gap.inline,
            hit: tokens.size.hit.target,
            drag_off: tokens.size.icon.sm + tokens.space.gap.inline,
            handle: tokens.size.handle.md,
            point_fill: cx.color(colors.surface.sunken),
            point_edge: cx.color(colors.text.normal),
            chosen_fill: cx.color(colors.text.normal),
            halo: cx.color(colors.overlay.halo),
            bar_ends: ChannelColor::ALL.map(|c| bar_end(c, cx)),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let CurveGraphStyle { least, most, .. } = *cx.style;
        let room = cx.column_room().filter(|room| room.is_finite() && cx.proposal != Proposal::Min);
        Vec2::splat(room.map_or(least, |room| room.clamp(least, most)))
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let frame = self.frame(rect, cx.style);
        cx.part(PLOT, frame.plot)
            .sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS)
            .value_part()
            .pointer_only("the point keys and the arrow keys edit the points");
        let shown = cx.value().cloned();
        if let (true, Some(points)) = (self.points_mode(shown.as_ref()), shown.as_ref().and_then(Curve::points)) {
            for (i, p) in points.iter().enumerate() {
                let reach = Rect::from_center_size(frame.map.screen(*p), Vec2::splat(cx.style.hit));
                cx.part(POINT.at(i as u64), reach).sense(Sense::empty()).priority(HitPriority::new(HitLevel::Body, 0));
            }
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Curve>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(mut curve) = cx.value().cloned() else {
            self.chosen = None;
            return;
        };
        let frame = self.frame(cx.rect, cx.style);
        let mut selected = self.selection;
        if self.pencil {
            self.pencil_update(cx, &frame, &curve);
            self.chosen = None;
            return;
        }
        let Some(count) = curve.points().map(<[_]>::len) else {
            self.chosen = None;
            return;
        };
        selected = selected.filter(|&i| i < count);
        let rules = self.rules();
        let reach = cx.style.look.pad + cx.style.drag_off;
        let press = cx.part(PLOT).press;
        let running = cx.gesture().is_some();
        let mut held = cx.memory::<Held>().clone();
        if let Some(drag) = held.drag
            && drag.begun
            && !running
            && !matches!(press.phase, PressPhase::Down { .. })
        {
            // Cancelled (Escape, an undo, an external change): the selection goes back too.
            selected = drag.before;
            held.drag = None;
        }
        match press.phase {
            PressPhase::Down { start, .. } => {
                let origin = start.origin;
                let secondary = start.button != PointerButton::Primary;
                let command = press.mods.cmd || press.mods.ctrl;
                let points = curve.points().map(<[_]>::to_vec).unwrap_or_default();
                let before = selected;
                match frame.map.hit(&points, origin, cx.style.hit) {
                    Some(i) if secondary || command => {
                        if let Some(next) = CurveRules::deleted(&curve, i) {
                            cx.set(next);
                        }
                        selected = None;
                    }
                    Some(i) => {
                        let at = frame.map.value_at(origin);
                        let p = points[i];
                        selected = Some(i);
                        held.drag = Some(Drag { index: Some(i), offset: [p[0] - at[0], p[1] - at[1]], before, corner: false, begun: false });
                    }
                    None if secondary => selected = None,
                    // A press within two levels of a point's input takes that point to the press.
                    None if !command => {
                        let mut points = points.clone();
                        let mut flags = curve.corner_flags();
                        let was = points.clone();
                        if let Some(i) = rules.insert(&mut points, frame.map.value_at(origin)) {
                            follow_corners(&was, &points, &mut flags);
                            selected = Some(i);
                            let mut drag = Drag { index: Some(i), offset: [0.0, 0.0], before, corner: false, begun: false };
                            if points.len() != was.len() {
                                cx.begin();
                                cx.propose(Curve::from_parts(points, flags));
                                drag.begun = true;
                            }
                            held.drag = Some(drag);
                        }
                    }
                    None => {}
                }
            }
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } | PressPhase::Release { pos, .. } => {
                if let Some(mut drag) = held.drag {
                    if let Some(next) = follow(&rules, frame.map, reach, &curve, &mut drag, pos, &mut selected) {
                        if !drag.begun {
                            cx.begin();
                            drag.begun = true;
                        }
                        cx.propose(next);
                    }
                    held.drag = Some(drag);
                    if matches!(press.phase, PressPhase::Release { .. }) {
                        if drag.begun {
                            cx.commit();
                        }
                        held.drag = None;
                    }
                }
            }
            PressPhase::Cancel { .. } => {
                if let Some(drag) = held.drag.take() {
                    selected = drag.before;
                }
            }
            _ => {}
        }
        // Keys: selection, nudges, additions and deletes while the graph has focus and no drag is under way.
        let dragging = held.drag.is_some() || cx.gesture().is_some();
        if let Some(shown) = cx.value().cloned() {
            curve = shown;
        }
        if !dragging {
            selected = self.keys(cx, &rules, &mut curve, selected);
        }
        *cx.memory::<Held>() = held;
        for chord in PLUS.into_iter().chain([MINUS]).chain(INSERT).chain(DELETE) {
            cx.claim(chord);
        }
        self.chosen = selected;
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<CurveWords>();
        let curve = cx.value();
        let channel = self.channels.get(self.channel).map(|c| c.label.as_str().to_owned());
        let drawn = self.pencil || curve.is_some_and(|curve| curve.points().is_none());
        let n = curve.and_then(Curve::points).map_or(0, <[_]>::len);
        let value = match (channel, drawn) {
            (Some(channel), true) => words.graph_drawn.fill(&[("channel", channel.as_str())]),
            (None, true) => words.drawn.as_str().to_owned(),
            (channel, false) => {
                let points = words.graph_points.text(n as u64, &cx.format().integer(n as u64));
                match channel {
                    Some(channel) => words.graph_value.fill(&[("channel", channel.as_str()), ("points", points.as_str())]),
                    None => points,
                }
            }
        };
        access.value(value);
        let Some(points) = curve.and_then(Curve::points).filter(|_| !self.pencil) else { return };
        let count = cx.format().integer(n as u64);
        for (i, p) in points.iter().enumerate() {
            let name = words.point.fill(&[("i", cx.format().integer(i as u64 + 1).as_str()), ("n", count.as_str())]);
            let template = if curve.is_some_and(|curve| curve.is_corner(i)) { &words.corner_value } else { &words.point_value };
            let value = template.fill(&[("x", self.level_text(cx, &words, p[0]).as_str()), ("y", self.level_text(cx, &words, p[1]).as_str())]);
            let node = access.node(POINT.at(i as u64), Role::GRAPHICS_OBJECT);
            node.name = Name::Text(name);
            node.description = Some(value);
            node.current(self.selection == Some(i));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let frame = self.frame(cx.rect, style);
        let curve = cx.value();
        let color = self.color();
        {
            let painter = ui.content_painter(cx);
            style.look.backdrop(ui, &painter, frame.plot, self.histogram);
            let draw = ui.draw();
            for i in self.other_channels() {
                paint_line(
                    &draw,
                    &outputs(&self.overlays[i], self.sampler.as_ref(), self.levels),
                    frame.map,
                    style.look.others[slot(self.channels[i].color)],
                );
            }
            if let Some(curve) = curve {
                paint_line(&draw, &outputs(curve, self.sampler.as_ref(), self.levels), frame.map, style.look.curve(color));
                if let (true, Some(points)) = (self.points_mode(Some(curve)), curve.points()) {
                    for (i, p) in points.iter().enumerate() {
                        let smooth = self.corners && !curve.is_corner(i);
                        point(&draw, style, frame.map.screen(*p), smooth, self.selection == Some(i));
                    }
                }
            }
        }
        if self.bars {
            let stops = [DataColor::new(Color32::from_gray(0)), style.bar_ends[slot(color)]];
            ui.child("under", frame.under, ToneRamp::new(&stops));
            ui.child("left", frame.left, ToneRamp::new(&stops).vertical());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CurveGraphOutput {
        CurveGraphOutput { selected: self.chosen, edit: cx.edit_response() }
    }
}

/// A point's handle: round for a smooth point when corners are on, square otherwise; the selected one filled.
fn point(draw: &Draw<'_>, style: &CurveGraphStyle, center: Pos2, smooth: bool, selected: bool) {
    let side = Vec2::splat(style.handle);
    let shape = |grow: f32| match smooth {
        true => PaintShape::Circle(center, style.handle / 2.0 + grow),
        false => PaintShape::Rect(Rect::from_center_size(center, side + Vec2::splat(2.0 * grow)), CornerRadius::ZERO),
    };
    if selected {
        draw.stroke(&shape(0.0), StrokeWidth::Thin, style.halo);
        draw.fill(&shape(0.0), style.chosen_fill);
    } else {
        draw.fill(&shape(0.0), style.point_fill);
        draw.stroke(&shape(0.0), StrokeWidth::Thin, style.point_edge);
    }
}

impl CurveGraph<'_> {
    /// Selection, nudges, additions and deletes from the keys. Returns the selection after them.
    fn keys(&self, cx: &mut Cx<'_, Self>, rules: &CurveRules, curve: &mut Curve, mut selected: Option<usize>) -> Option<usize> {
        let n = curve.points().map_or(0, <[_]>::len);
        if n == 0 {
            return selected;
        }
        let mut plus = 0;
        for chord in PLUS {
            while cx.take_claimed(chord) {
                plus += 1;
            }
        }
        for _ in 0..plus {
            selected = Some(selected.map_or(0, |i| (i + 1).min(n - 1)));
        }
        while cx.take_claimed(MINUS) {
            selected = Some(selected.map_or(n - 1, |i| i.saturating_sub(1)));
        }
        let mut insert = false;
        for chord in INSERT {
            while cx.take_claimed(chord) {
                insert = true;
            }
        }
        if insert {
            // A gap too narrow for a point adds nothing and keeps the selection.
            let drawn = curve.points().map(|points| self.samples(points, &curve.corner_flags())).unwrap_or_default();
            if let Some((next, i)) = rules.inserted(curve, selected, &drawn) {
                cx.set(next.clone());
                *curve = next;
                selected = Some(i);
            }
        }
        let mut delete = false;
        for chord in DELETE {
            while cx.take_claimed(chord) {
                delete = true;
            }
        }
        let rtl = cx.flow.is_rtl();
        let mut by = [0.0f32; 2];
        while let Some((mv, mods)) = cx.intents().take_move() {
            // Alt and Cmd/Ctrl with an arrow stay the app's.
            if mods.fine || mods.lead {
                continue;
            }
            let steps = if mods.extend { NUDGE_LARGE } else { NUDGE };
            let (dx, dy) = match mv {
                NavMove::Forward => (if rtl { -1.0 } else { 1.0 }, 0.0),
                NavMove::Back => (if rtl { 1.0 } else { -1.0 }, 0.0),
                NavMove::Up => (0.0, 1.0),
                NavMove::Down => (0.0, -1.0),
                _ => continue,
            };
            by = [by[0] + dx * steps, by[1] + dy * steps];
        }
        let Some(at) = selected else { return selected };
        if delete {
            // An end point stays, and so does the selection on it.
            let Some(next) = CurveRules::deleted(curve, at) else { return selected };
            cx.set(next.clone());
            *curve = next;
            return None;
        }
        if by != [0.0, 0.0]
            && let Some(next) = rules.nudged(curve, at, by)
        {
            cx.set(next.clone());
            *curve = next;
        }
        selected
    }

    /// One frame of the pencil: press, drag and release are one gesture drawing straight segments into the outputs. A
    /// Shift press continues from where the previous stroke ended.
    fn pencil_update(&self, cx: &mut Cx<'_, Self>, frame: &Frame, curve: &Curve) {
        let press = cx.part(PLOT).press;
        let mut held = cx.memory::<Held>().clone();
        if cx.gesture().is_none() && held.end.as_ref().is_some_and(|(_, left)| left != curve) {
            held.end = None;
        }
        let draw = |held: &mut Held, cx: &mut Cx<'_, Self>, from: [f32; 2], to: [f32; 2]| {
            // Any stroke makes the curve a drawn map, even one that leaves every output as it was.
            let changed = Curve::pencil_line(&mut held.outputs, from, to, self.levels);
            if changed || curve.points().is_some() {
                cx.propose(Curve::Map(held.outputs.clone()));
            }
        };
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                let at = frame.map.value_at(start.origin);
                let from = if press.mods.shift { held.end.as_ref().map_or(at, |(end, _)| *end) } else { at };
                held.outputs = Curve::drawn(curve, |points, corners| self.samples(points, corners), self.levels);
                cx.begin();
                held.stroke = Some(at);
                draw(&mut held, cx, from, at);
            }
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } | PressPhase::Release { pos, .. } => {
                if let Some(last) = held.stroke {
                    let at = frame.map.value_at(pos);
                    held.stroke = Some(at);
                    draw(&mut held, cx, last, at);
                    if matches!(press.phase, PressPhase::Release { .. }) {
                        held.stroke = None;
                        cx.commit();
                        held.end = Some((at, Curve::Map(held.outputs.clone())));
                    }
                }
            }
            PressPhase::Cancel { .. } => held.stroke = None,
            _ => {}
        }
        *cx.memory::<Held>() = held;
    }
}
