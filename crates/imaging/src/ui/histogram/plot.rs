//! `Histogram`: a plot of bins, one tone channel or overlapping ribbons.

use super::data::{HistogramData, HistogramLook, LevelRange, Plot};
use super::paint::{
    HistogramInks, PlotFrame, level_at, level_x, marker_shapes, outline_shapes, paint_bins, paint_edge, paint_frame, selection_band, selection_edges,
    selection_shapes, sum,
};
use super::words::HistogramWords;
use craft_behavior::{AccessAction, CancelReason, CursorIcon, Flow, IconId, Intent, Metrics, NavMove, PartKey, PressPhase, Sense};
use craft_theme::{Stateful, TypeRole};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Numeric, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{IconSpec, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, IconLook, StyleCx, TextLook};
use egui::{Color32, Pos2, Rect, Response, Vec2};

/// The plot's height in control heights when it stands alone.
const STANDARD_CONTROLS: f32 = 4.0;
/// The plot's height in control heights in a tight arrangement of plots, and the least a plot that fills its room
/// keeps.
const SHORT_CONTROLS: f32 = 2.0;

/// The plot of the whole histogram.
pub(super) const PLOT: PartKey = PartKey::new("plot");
/// The start edge of the selection.
const START: PartKey = PartKey::new("start");
/// The end edge of the selection.
const END: PartKey = PartKey::new("end");
/// The stale mark.
const STALE: PartKey = PartKey::new("stale");
/// A marker's caption.
const CAPTION: PartKey = PartKey::new("caption");

/// How tall the plot is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlotHeight {
    /// Four control heights.
    #[default]
    Standard,
    /// Two control heights: the plots of an arrangement of several, such as All Channels.
    Short,
    /// The room the parent gives, at least a short plot: Levels and Threshold.
    Fill,
}

/// The mark that says the data is older than the image: a warning icon at the top trailing corner, with its text as
/// the tooltip and the accessible name.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StaleMark {
    /// Only the mark. An empty label takes the word "Cached data".
    Mark(Label),
    /// A button that asks for new data; the label names it ("Recalculate"). It is a Tab stop and its press is
    /// [`HistogramOutput::refresh`].
    Button(Label),
}

impl StaleMark {
    fn label(&self) -> &Label {
        match self {
            StaleMark::Mark(label) | StaleMark::Button(label) => label,
        }
    }

    fn is_button(&self) -> bool {
        matches!(self, StaleMark::Button(_))
    }
}

/// The style of a histogram plot.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct HistogramStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The marker captions.
    #[style(text)]
    caption: TextLook,
    /// The stale mark.
    #[style(icon)]
    stale: IconLook,
    /// The colours of the bars.
    inks: HistogramInks,
    /// The plot's well and edge.
    frame: PlotFrame,
    /// The selected range's band: `data.selection`.
    selection: Color32,
    /// The ink of the markers and their ticks: `text.normal`, or the sum of the ribbons on the scope.
    marker: Color32,
    /// Width of a marker tick and of the selection's edges: `stroke.medium`.
    marker_width: f32,
    /// Height of a marker tick, from the bottom of the plot: `size.icon.sm`.
    marker_height: f32,
    /// The room between a tick and its caption, and between the stale mark and the plot's corner.
    gap: f32,
}

/// The sizes of the marks on the plot, copied out of the style so `parts` can place them while it declares.
#[derive(Clone, Copy)]
struct Geometry {
    marker_width: f32,
    marker_height: f32,
    gap: f32,
    side: f32,
}

impl Geometry {
    fn of(style: &HistogramStyle) -> Self {
        Self { marker_width: style.marker_width, marker_height: style.marker_height, gap: style.gap, side: style.stale.side }
    }
}

/// What a histogram reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct HistogramOutput {
    /// What the user did to the selection: a range set by dragging, the keys or a screen reader, and `None` set by a
    /// click or Escape. The host stores the value and passes it back.
    pub selection: EditResponse<Option<LevelRange>>,
    /// The whole level under the pointer while it is over the plot or dragging from it, clamped to the bins.
    pub hovered_level: Option<f32>,
    /// The stale button was pressed; only with [`StaleMark::Button`].
    pub refresh: bool,
    /// The plot was double-clicked; what it means (the scope panel recounts) is the host's.
    pub double_click: bool,
    /// The plot's response.
    pub response: Response,
}

impl Outcome for HistogramOutput {
    type Value = Option<LevelRange>;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.selection.story_facts()
    }
}

/// A marker's caption as laid out: its level, its text and the room it takes.
type Caption = (f32, String, Vec2);

/// A plot of bins: one tone channel (Levels, the Histogram panel) or overlapping ribbons (Camera Raw's red, green and
/// blue, a print's inks).
///
/// - **Bins:** any count, drawn across the plot width, each bin centred in its slot.
/// - **Tone look:** a filled area on the plot's well, in the channel's data token (`text.muted` for neutral), with an
///   edge only when [`bordered`](Histogram::bordered).
/// - **RGB look** ([`HistogramData::rgb`], [`HistogramData::channels`]): the bins smoothed for display with the
///   five-bin binomial kernel (the two edge bins pinned), one shared scale and a `stroke.thin` outline per channel.
///   Fills are opaque: `rgb` adds the channel colours where fills overlap (saturating, as light), `channels` takes
///   their product (as inks). The plot sits on `data.scope`.
/// - **Outline:** the original histogram as a `stroke.thin` line over the plot, on the plot's scale in the tone look
///   (a taller original is clamped to the top of the plot) and on its own scale in the RGB look.
/// - **Marker, markers:** a tick at a level, standing on the bottom of the plot; markers add a caption beside the
///   tick, level with its top.
/// - **Selection** (Photoshop's Histogram panel): a press and drag over the plot sets a band over whole bin slots,
///   a click or Escape clears it. With the plot focused the Left and Right arrows move the end edge and Shift with
///   them the start edge; with no selection they make one at the marker, else the middle. Levels are bin positions.
/// - **Stale:** a warning icon at the top trailing corner with a tooltip; as a [`StaleMark::Button`] it is a button.
/// - **AccessKit:** one image named by the label, valued by [`value`](Histogram::value); the stale mark and each
///   marker caption are named nodes, and a selection is two sliders, its start and its end, beneath the plot's node,
///   with Increment and Decrement.
///
/// The plot is a Tab stop only with a [`selection`](Histogram::selection) or a stale button.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct Histogram<'a> {
    #[common]
    common: Common,
    data: &'a HistogramData,
    /// What a screen reader reads after the label, such as "Red channel".
    #[prop(into)]
    value: Option<Label>,
    /// The original histogram, outlined over the current one (a tone data).
    #[prop]
    outline: Option<&'a HistogramData>,
    /// A tick at this level, the sampled pixel's tone.
    #[prop(quantity = "level")]
    marker: Option<f32>,
    /// Labelled ticks, each `(level, caption)`: the black point, white point and mid of a levels adjustment.
    #[prop]
    markers: &'a [(f32, Label)],
    /// The selected range of levels, `None` for no selection.
    #[prop]
    selection: Option<Bind<'a, Option<LevelRange>>>,
    /// The mark that says the data is older than the image.
    #[prop]
    stale: Option<StaleMark>,
    /// The edge round the plot. The Histogram panel has none; a levels editor turns it on.
    #[prop(flag)]
    bordered: bool,
    /// How tall the plot is.
    #[style_key]
    #[prop]
    height: PlotHeight,
    /// The plot draws ribbons on the scope.
    #[style_key]
    ribbons: bool,
    shaped: Vec<Caption>,
    stale_text: Option<Label>,
    hovered: Option<f32>,
    refresh: bool,
    double_click: bool,
}

impl<'a> Histogram<'a> {
    /// A plot of `data`, named `label`.
    pub fn new(label: impl Into<Label>, data: &'a HistogramData) -> Self {
        Histogram {
            common: Common::new(),
            data,
            value: None,
            outline: None,
            marker: None,
            markers: &[],
            selection: None,
            stale: None,
            bordered: false,
            height: PlotHeight::Standard,
            ribbons: data.is_ribbons(),
            shaped: Vec::new(),
            stale_text: None,
            hovered: None,
            refresh: false,
            double_click: false,
        }
        .label(label)
        .ltr_axis()
    }

    fn bins(&self) -> usize {
        self.data.bin_count()
    }

    /// Whether the plot takes the pointer and the keys for its selection.
    fn selectable(&self) -> bool {
        self.selection.is_some() && self.bins() > 0
    }

    /// The selection as the pass shows it: a running drag's proposal, else the binding's.
    fn current(cx: &Cx<'_, Self>) -> Option<LevelRange> {
        cx.value().copied().flatten()
    }

    /// The tone data an outline draws from: its bins and channel.
    fn outline_bins(&self) -> Option<(&[f32], super::data::ChannelColor)> {
        match &self.outline?.plot {
            Plot::Tone { bins, channel } => Some((bins, *channel)),
            Plot::Channels { .. } => None,
        }
    }

    /// The caption's rect beside the tick of `level`: its bottom level with the tick's top, on the side with room.
    fn caption_rect(rect: Rect, bins: usize, level: f32, size: Vec2, style: Geometry) -> Option<Rect> {
        let x = level_x(rect, bins, level);
        if !super::paint::usable(rect) || !x.is_finite() {
            return None;
        }
        let (before, after) = (x - style.marker_width / 2.0 - style.gap, x + style.marker_width / 2.0 + style.gap);
        let left = if after + size.x <= rect.right() { after } else { (before - size.x).max(rect.left()) };
        let at = Pos2::new(left, rect.bottom() - style.marker_height - size.y);
        Some(Rect::from_min_size(at, size).intersect(rect))
    }

    /// The stale mark's rect at the top trailing corner.
    fn stale_rect(rect: Rect, flow: Flow, style: Geometry) -> Rect {
        let side = style.side;
        let toward = if flow.is_rtl() { 1.0 } else { -1.0 };
        let x = flow.end_x() + toward * (style.gap + side / 2.0);
        Rect::from_center_size(Pos2::new(x, rect.top() + style.gap + side / 2.0), Vec2::splat(side))
    }
}

impl Component for Histogram<'_> {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither).with_moves();
    type Style = HistogramStyle;
    type Output = HistogramOutput;

    fn style(cx: &StyleCx<'_>, key: HistogramStyleKey) -> HistogramStyle {
        let colors = &cx.tokens.color;
        let tall = cx.control().height
            * match key.height {
                PlotHeight::Standard => STANDARD_CONTROLS,
                PlotHeight::Short | PlotHeight::Fill => SHORT_CONTROLS,
            };
        let inks = HistogramInks::new(cx);
        let marker = if key.ribbons { sum(&inks.ribbons(), 0b111) } else { super::data::color32(colors.text.normal) };
        let accent = super::data::color32(colors.data.selection);
        let warn = cx.tone(craft_theme::variant::Tone::Warn);
        HistogramStyle {
            metrics: Metrics::new().min_size(Vec2::new(cx.tokens.size.plot.width, tall)),
            focus: cx.focus_look(),
            caption: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(colors.text.normal))),
            stale: IconLook::new(cx.tokens.size.icon.sm, Stateful::new(warn)),
            frame: PlotFrame::new(cx),
            inks,
            selection: accent,
            marker,
            marker_width: cx.tokens.stroke.medium,
            marker_height: cx.tokens.size.icon.sm,
            gap: cx.tokens.space.gap.inline,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        (self.height == PlotHeight::Fill).then_some(craft_behavior::Axis::Y)
    }

    fn sense(&self) -> Sense {
        if self.selectable() { Sense::FOCUS } else { Sense::empty() }
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let bins = self.bins();
        let selectable = self.selectable();
        let (geometry, role) = (Geometry::of(cx.style), cx.style.caption.role);
        if bins > 0 {
            if selectable {
                cx.part(PLOT, rect)
                    .sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS)
                    .cursor(CursorIcon::Crosshair)
                    .value_part()
                    .pointer_only("the arrow keys move the selection edges");
            } else {
                cx.part(PLOT, rect);
            }
            let current = cx.value().copied().flatten();
            if let Some(band) = current.and_then(|range| selection_band(rect, bins, range)) {
                let [start, end] = selection_edges(band, geometry.marker_width);
                cx.part(START, start);
                cx.part(END, end);
            }
        }
        let markers = self.markers;
        self.shaped = markers
            .iter()
            .map(|(level, label)| {
                let text = label.as_str().to_owned();
                let size = cx.layout(&TextRun::new(text.clone()).role(role)).size();
                (*level, text, size)
            })
            .collect();
        for (i, (level, _, size)) in self.shaped.iter().enumerate() {
            if let Some(area) = Self::caption_rect(rect, bins, *level, *size, geometry) {
                cx.part(CAPTION.at(i as u64), area);
            }
        }
        self.stale_text = self.stale.as_ref().map(|mark| match mark.label().as_str() {
            "" => cx.words::<HistogramWords>().cached_data.clone(),
            _ => mark.label().clone(),
        });
        if let (Some(mark), Some(text)) = (&self.stale, &self.stale_text) {
            let area = Self::stale_rect(rect, cx.flow, geometry);
            let part = cx.part(STALE, area).tooltip(text.clone());
            if mark.is_button() {
                part.sense(Sense::CLICK | Sense::TAKE_FOCUS).focusable();
            }
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<LevelRange>>> {
        self.selection.as_mut().map(|bind| bind.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let bins = self.bins();
        let rect = cx.rect;
        let press = cx.part(PLOT).press;
        let pointer = match press.phase {
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => Some(pos),
            _ => cx.hover_pos(PLOT),
        };
        self.hovered = pointer.filter(|_| bins > 0).map(|at| level_at(rect, bins, at.x));
        self.double_click = press.double_pressed();
        self.refresh = self.stale.as_ref().is_some_and(|mark| mark.is_button() && cx.part_activated(STALE).is_some());
        if !self.selectable() {
            return;
        }
        let current = Self::current(cx);
        let last = bins as f32 - 1.0;
        let dragged = |origin: f32, at: f32| LevelRange::new(level_at(rect, bins, origin), level_at(rect, bins, at));
        match press.phase {
            PressPhase::DragStart { start, pos, .. } => {
                cx.begin();
                cx.propose(Some(dragged(start.origin.x, pos.x)));
            }
            PressPhase::Drag { start, pos, .. } => cx.propose(Some(dragged(start.origin.x, pos.x))),
            PressPhase::Release { start, pos, click, .. } => {
                if cx.gesture().is_some() {
                    cx.propose(Some(dragged(start.origin.x, pos.x)));
                    cx.commit();
                } else if click && current.is_some() {
                    cx.set(None);
                }
            }
            _ => {}
        }
        let running = cx.gesture().is_some();
        if cx.intents().take(Intent::Cancel) {
            if running {
                cx.cancel(CancelReason::Escape);
            } else if current.is_some() {
                cx.set(None);
            }
        }
        if running {
            return;
        }
        let rtl = cx.flow.is_rtl();
        let middle = self.marker.unwrap_or(last / 2.0).round();
        while let Some((mv, mods)) = cx.intents().take_move() {
            // The plot is physical: Right is toward the last bin whatever the reading direction.
            let reading = match mv {
                NavMove::Forward => 1.0,
                NavMove::Back => -1.0,
                _ => continue,
            };
            let step = if rtl { -reading } else { reading };
            let range = Self::current(cx).unwrap_or(LevelRange::new(middle, middle));
            let moved = |level: f32| (level + step).clamp(0.0, last);
            let next = if mods.extend { LevelRange::new(moved(range.start), range.end) } else { LevelRange::new(range.start, moved(range.end)) };
            cx.set(Some(next));
        }
        // A screen reader steps or sets one edge; a start is held at the end and an end at the start.
        while let Some(request) = cx.intents().take_access() {
            let Some(target) = request.target.filter(|target| target.key == START || target.key == END) else { continue };
            let Some(range) = Self::current(cx) else { continue };
            let start = target.key == START;
            let edge = if start { range.start } else { range.end };
            let wanted = match request.action {
                AccessAction::Increment => edge + 1.0,
                AccessAction::Decrement => edge - 1.0,
                _ => continue,
            };
            cx.set(Some(Self::edge_to(range, start, wanted, last)));
        }
        for start in [true, false] {
            if let (Some(value), Some(range)) = (cx.intents().take_set_value_at(if start { START } else { END }), Self::current(cx)) {
                cx.set(Some(Self::edge_to(range, start, value as f32, last)));
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(value) = &self.value {
            access.value(value.as_str());
        }
        let words = cx.words::<HistogramWords>();
        let label = cx.resolved.label;
        if let Some(range) = Self::current(cx).filter(|_| self.bins() > 0) {
            let last = self.bins() as f32 - 1.0;
            for (word, start) in [(&words.selection_start, true), (&words.selection_end, false)] {
                let node = access.node(if start { START } else { END }, Role::SLIDER);
                node.name = if label.is_empty() {
                    Name::Text(word.as_str().to_owned())
                } else {
                    Name::Joined(vec![label.to_owned(), word.as_str().to_owned()])
                };
                let (value, low, high) = if start { (range.start, 0.0, range.end) } else { (range.end, range.start, last) };
                node.numeric = Some(Numeric::new(f64::from(value), f64::from(low)..=f64::from(high)).step(1.0));
            }
        }
        for (i, (_, text, _)) in self.shaped.iter().enumerate() {
            access.node(CAPTION.at(i as u64), Role::IMAGE).name = Name::Text(text.clone());
        }
        if let (Some(mark), Some(text)) = (&self.stale, &self.stale_text) {
            let node = access.node(STALE, if mark.is_button() { Role::BUTTON } else { Role::IMAGE });
            node.name = Name::Text(text.as_str().to_owned());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (style, rect, bins) = (cx.style, cx.rect, self.bins());
        let painter = ui.content_painter(cx);
        let frame = if self.ribbons { style.frame.on(style.inks.scope()) } else { style.frame };
        paint_frame(&painter, rect, &frame, false);
        if bins > 0 {
            let look = if self.ribbons { HistogramLook::Rgb } else { HistogramLook::Tone };
            paint_bins(&painter, rect, self.data, look, &style.inks);
            if let Some((original, channel)) = self.outline_bins() {
                painter.painter.extend(outline_shapes(rect, self.data, original, channel, &style.inks));
            }
            if let Some(range) = Self::current(cx) {
                painter.painter.extend(selection_shapes(rect, bins, range, style.marker_width, style.selection));
            }
            let tick = (style.marker_width, style.marker_height);
            for level in self.marker.into_iter().chain(self.shaped.iter().map(|(level, ..)| *level)) {
                painter.painter.extend(marker_shapes(rect, bins, level, tick, style.marker));
            }
        }
        if self.bordered {
            paint_edge(&painter, rect, &frame);
        }
        let (draw, geometry) = (ui.draw(), Geometry::of(style));
        for (level, text, size) in &self.shaped {
            if let Some(area) = Self::caption_rect(rect, bins, *level, *size, geometry) {
                let _ = draw.text(area, &TextRun::new(text.as_str()).role(style.caption.role).color(cx.resolve(&style.caption.color)));
            }
        }
        if self.stale.is_some() {
            let area = Self::stale_rect(rect, cx.flow, geometry);
            draw.icon(area, &IconSpec::svg(IconId::new("warning")).color(cx.resolve(&style.stale.color)));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> HistogramOutput {
        HistogramOutput {
            selection: cx.edit_response(),
            hovered_level: self.hovered,
            refresh: self.refresh,
            double_click: self.double_click,
            response: cx.response.clone(),
        }
    }
}

impl Histogram<'_> {
    /// `range` with one edge moved to `wanted`, whole, within the plot; the start is held at the end and the end at the
    /// start.
    fn edge_to(range: LevelRange, start: bool, wanted: f32, last: f32) -> LevelRange {
        let wanted = wanted.round().clamp(0.0, last);
        if start { LevelRange { start: wanted.min(range.end), ..range } } else { LevelRange { end: wanted.max(range.start), ..range } }
    }
}
