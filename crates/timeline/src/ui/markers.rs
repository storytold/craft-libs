//! Markers on a ruler or a clip bar, and the Marker dialog (N1.12).

mod dialog;
#[cfg(feature = "screen-testing")]
mod stories;

pub use dialog::{MarkerDialog, MarkerDialogAction, MarkerDialogResult, MarkerDraft};

use std::sync::Arc;

use craft_behavior::{
    Arrows, CancelReason, CursorIcon, EditEvent, GestureId, Granularity, HitLevel, HitPriority, Input, Intent, Metrics, Mixed, NavShape, PartKey,
    PointerButton, PressCancel, PressPhase, Roving, Sense,
};
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role,
    SetPosition, WordSetCx as _, clicked,
};
use craft_ui::primitives::{IconSpec, PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::edit::next_gesture_id;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2, pos2};

use super::clips::context_menu;
use super::{TimeAxis, label_palette, time_axis};
use crate::clips::{SnapTargets, snap_time};
use crate::markers::{Marker, MarkerColor, MarkerId};
use crate::time::Tick;
use crate::timecode::format_time;

/// The fill of a range marker's duration bar, as a share of the label colour (EffectCraft `markers_ui.rs:296`).
const DURATION_TINT: f32 = 0.5;
/// Where a marker's point sits, as a share of the strip's height (EffectCraft `markers_ui.rs:274`). A proportion of
/// the strip, not a size.
pub const MARKER_POINT: f32 = 0.6;

craft_ui::words! {
    /// The marker strip's and the Marker dialog's words, English by default.
    #[non_exhaustive]
    pub struct MarkerWords {
        bin: "markerStrip.bin" = "Marker bin";
        bin_hint: "markerStrip.binHint" = "Marker bin: drag a marker to the time ruler";
        title: "markerDialog.title" = "Marker";
        name: "markerDialog.name" = "Name:";
        comment: "markerDialog.comment" = "Comments:";
        time: "markerDialog.time" = "Time:";
        duration: "markerDialog.duration" = "Duration:";
        color: "markerDialog.color" = "Marker Color:";
        delete: "markerDialog.delete" = "Delete";
        previous: "markerDialog.previous" = "Previous";
        next: "markerDialog.next" = "Next";
    }
}

/// What a marker gesture asks of the app, besides the drag the strip's value carries.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarkerEdit {
    /// Cmd-click (Ctrl-click on Windows and Linux), or Delete with the marker picked.
    Delete(MarkerId),
    /// Double-click or Enter: open the Marker dialog on it.
    Open(MarkerId),
    /// A marker dragged out of the marker bin and dropped at `at`, or Add Marker (M).
    #[non_exhaustive]
    Add {
        /// Where the new marker starts.
        at: Tick,
    },
}

/// A dragged marker's place: the strip's value. A drag moves `start`; an Alt-drag sets `duration`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MarkerSpan {
    /// The marker that moves.
    pub id: MarkerId,
    /// Where it starts.
    pub start: Tick,
    /// How long it lasts; zero for a point marker.
    pub duration: Tick,
}

impl MarkerSpan {
    /// The place of marker `id`.
    pub const fn new(id: MarkerId, start: Tick, duration: Tick) -> Self {
        MarkerSpan { id, start, duration }
    }
}

/// The part of marker `MARKER.at(id)`, and the marker bin.
const MARKER: PartKey = PartKey::new("marker");
const BIN: PartKey = PartKey::new("bin");

/// The style of the marker strip.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MarkerStripStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The marker's name.
    name: StyleColor,
    /// The box behind a name.
    name_fill: StyleColor,
    /// The outline of that box.
    name_edge: StyleColor,
    /// The bin's glyph, idle and while a marker is dragged out of it.
    bin_idle: StyleColor,
    bin_hot: StyleColor,
    /// The flag's width and the strip's height.
    flag_width: f32,
    height: f32,
    /// How far from a marker's time a press still takes it, and how near a snap target a dragged time snaps.
    reach: f32,
    /// The room around a name, and the gap before a range marker's end cap.
    pad: f32,
    cap_gap: f32,
    /// The width of a range marker's end cap.
    cap: f32,
    radius: CornerRadius,
}

/// A marker drag under way: which marker, whether it sets the duration, and the marker's start and duration when it
/// was grabbed.
#[derive(Clone, Copy, Debug, Default)]
struct Grab {
    id: GestureId,
    marker: MarkerId,
    length_drag: bool,
    start: Tick,
    length: Tick,
    /// The place the drag has reached, as drawn.
    now: (Tick, Tick),
}

/// What the strip's paint reads of the pass: the marker being dragged out of the bin.
#[derive(Clone, Debug, Default)]
struct Pulled {
    at: Option<Tick>,
    /// The marker being dragged and where the drag has put it.
    dragged: Option<MarkerSpan>,
}

/// What the strip reports besides the drag.
#[derive(Clone, Debug, Default)]
struct Report {
    span: Option<EditEvent<MarkerSpan>>,
    edits: Vec<MarkerEdit>,
    context: Option<MarkerId>,
}

fn cancel_reason(cancel: PressCancel) -> CancelReason {
    match cancel {
        PressCancel::Escape => CancelReason::Escape,
        PressCancel::Removed => CancelReason::Removed,
        _ => CancelReason::Blur,
    }
}

/// A row of markers in the rect it is shown at (the ruler's marker band, a clip or layer bar): a flag pointing down
/// at its time (the point at [`MARKER_POINT`] of the height), coloured by its label, a duration bar for range markers,
/// the name beside it when there is room.
///
/// After Effects' gestures (EffectCraft `markers_ui.rs:1`): drag moves (snapping to [`snap`](Self::snap)), Alt-drag
/// sets the duration, Cmd-click (Ctrl-click on Windows and Linux) deletes, a double-click opens the dialog, a secondary
/// click is reported for the app's menu (Settings, Convert, Delete). The drag is the strip's value, a [`MarkerSpan`]:
/// one gesture from press to release, Esc cancels. With [`bin`](Self::bin) a marker dragged out of the bin at the
/// right end adds one where it is dropped. Tooltips show name and comment. Keyboard: the markers are one roving
/// group, so the strip is one Tab stop; Left / Right pick the previous / next marker, Enter opens it, Delete and
/// Backspace delete it. AccessKit: a list box named by the strip's label, holding the markers named by name and time.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct MarkerStrip<'a> {
    #[common]
    common: Common,
    markers: &'a [Marker],
    /// The snap targets for drags; none turns snapping off.
    #[prop]
    snap: Option<&'a SnapTargets>,
    /// Shows the marker bin at the right end.
    #[prop(flag)]
    bin: bool,
    report: Report,
}

impl<'a> MarkerStrip<'a> {
    /// A strip named `label` for `markers`.
    pub fn new(label: impl Into<Label>, markers: &'a [Marker]) -> Self {
        MarkerStrip { common: Common::new(), markers, snap: None, bin: false, report: Report::default() }.label(label).ltr_axis()
    }

    fn sorted(&self) -> Vec<&'a Marker> {
        let mut order: Vec<&Marker> = self.markers.iter().collect();
        order.sort_by_key(|marker| marker.start);
        order
    }

    /// Where a dragged time lands: on a snap target other than the marker's own place.
    fn settle(&self, axis: &TimeAxis, reach: f32, t: Tick, own: Option<Tick>) -> Tick {
        match self.snap {
            Some(targets) => {
                let mut others = SnapTargets::new();
                others.extend(targets.times().iter().copied().filter(|target| Some(*target) != own));
                snap_time(&others, t.max(Tick::ZERO), &axis.view, reach).0
            }
            None => t.max(Tick::ZERO),
        }
    }

    /// The marker's start and duration as drawn: the dragged marker's, while it is dragged.
    fn shown(marker: &Marker, dragged: Option<&MarkerSpan>) -> (Tick, Tick) {
        match dragged {
            Some(span) if span.id == marker.id => (span.start, span.duration),
            _ => (marker.start, marker.duration),
        }
    }

    /// The bin's rect at the right end of `rect`.
    fn bin_rect(rect: Rect, height: f32) -> Rect {
        Rect::from_min_size(pos2(rect.right() - height, rect.top()), Vec2::splat(height))
    }

    /// The name box's left edge: after the flag, after a range marker's end cap when that is in the way, else before
    /// the flag; `None` when it fits nowhere in `clip` (EffectCraft `markers_ui.rs:380` `name_box_x`).
    fn name_box_x(x: f32, width: f32, end: Option<f32>, clip: Rect, gap: f32, end_gap: f32) -> Option<f32> {
        let start = x + gap;
        let right = match end {
            Some(end) if end > start && end < start + width => end + end_gap,
            _ => start,
        };
        if right + width <= clip.right() {
            return Some(right);
        }
        let left = x - gap - width;
        (left >= clip.left()).then_some(left)
    }

    /// The flag's outline: a tab `width` wide pointing down at `x`, `height` tall from `top`.
    fn flag(x: f32, top: f32, width: f32, height: f32) -> Vec<Pos2> {
        let half = width / 2.0;
        let point = top + height * MARKER_POINT;
        vec![pos2(x - half, top), pos2(x + half, top), pos2(x + half, point), pos2(x, top + height), pos2(x - half, point)]
    }
}

impl Component for MarkerStrip<'_> {
    const ROLE: Role = Role::LIST_BOX;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_deletes();
    const SINGLE_VALUE: bool = true;
    type Style = MarkerStripStyle;
    type Output = MarkerStripOutput;

    fn style(cx: &StyleCx<'_>, _: MarkerStripStyleKey) -> MarkerStripStyle {
        let tokens = cx.tokens;
        MarkerStripStyle {
            metrics: Metrics::new(),
            name: cx.color(tokens.color.text.normal),
            name_fill: cx.color(tokens.color.surface.sunken),
            name_edge: cx.color(tokens.color.line.control_edge),
            bin_idle: cx.color(tokens.color.text.muted),
            bin_hot: cx.color(tokens.color.accent.solid),
            flag_width: tokens.size.handle.md,
            height: cx.control().height,
            reach: tokens.size.hit.target,
            pad: tokens.space.gap.inline,
            cap_gap: tokens.space.gap.tight,
            cap: tokens.stroke.medium,
            radius: cx.field().radius,
        }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(0.0, cx.style.height)
    }

    /// One focusable part per marker in time order, in one roving group; the bin is pointer-only.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (height, half) = (cx.style.height.min(rect.height()), cx.style.reach / 2.0);
        let mut any = false;
        for marker in self.sorted() {
            let x = axis.x_of(marker.start, rect.left());
            if x < rect.left() - half || x > rect.right() + half {
                continue;
            }
            let hit = Rect::from_min_max(pos2(x - half, rect.top()), pos2(x + half, rect.top() + height));
            let tip = [marker.name.as_str(), marker.comment.as_str()].iter().filter(|text| !text.is_empty()).copied().collect::<Vec<_>>().join("\n");
            let part = cx
                .part(MARKER.at(marker.id.0), hit)
                .sense(Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focusable();
            if !tip.is_empty() {
                part.tooltip(tip);
            }
            any = true;
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).arrows(Arrows::Horizontal));
        }
        if self.bin {
            let hint = cx.words::<MarkerWords>().bin_hint.clone();
            cx.part(BIN, Self::bin_rect(rect, height))
                .sense(Sense::DRAG)
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .tooltip(hint)
                .pointer_only("Add Marker adds a marker at the playhead");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (reach, strip) = (cx.style.reach, cx.rect);
        let mut report = Report::default();
        for marker in self.sorted() {
            let part = MARKER.at(marker.id.0);
            let Some(result) = cx.part_if_declared(part) else { continue };
            let press = result.press;
            if press.double_pressed() {
                report.edits.push(MarkerEdit::Open(marker.id));
            }
            if context_menu(cx, part).is_some() {
                report.context = Some(marker.id);
            }
            if cx.part_activated(part) == Some(craft_ui::runtime::commands::FireSource::Key) {
                report.edits.push(MarkerEdit::Open(marker.id));
            }
            if press.phase.start().is_none_or(|start| start.button != PointerButton::Primary) {
                continue;
            }
            match press.phase {
                PressPhase::DragStart { .. } => {
                    let id = next_gesture_id(&cx.response.ctx);
                    let grab = Grab {
                        id,
                        marker: marker.id,
                        length_drag: press.mods.alt,
                        start: marker.start,
                        length: marker.duration,
                        now: (marker.start, marker.duration),
                    };
                    *cx.press_slot::<Grab>() = Some(grab);
                    let before = MarkerSpan::new(marker.id, marker.start, marker.duration);
                    report.span = Some(EditEvent::begin(id, Mixed::Same(before), Arc::from(cx.resolved.label.to_owned()), None));
                }
                PressPhase::Drag { pos, .. } => {
                    if let Some(grab) = *cx.press_slot::<Grab>() {
                        let at = self.settle(&axis, reach, axis.tick_at(pos.x, strip.left()), Some(grab.start));
                        let span = if grab.length_drag {
                            MarkerSpan::new(marker.id, grab.start, at.max(grab.start) - grab.start)
                        } else {
                            MarkerSpan::new(marker.id, at, grab.length)
                        };
                        if let Some(held) = cx.press_slot::<Grab>().as_mut() {
                            held.now = (span.start, span.duration);
                        }
                        report.span = Some(EditEvent::update(grab.id, span));
                    }
                }
                PressPhase::Release { click: false, .. } => {
                    if let Some(grab) = cx.press_slot::<Grab>().take() {
                        let before = MarkerSpan::new(marker.id, grab.start, grab.length);
                        let after = MarkerSpan::new(marker.id, grab.now.0, grab.now.1);
                        report.span = Some(EditEvent::commit(grab.id, Mixed::Same(before), after));
                    }
                }
                PressPhase::Cancel { reason, .. } => {
                    if let Some(grab) = cx.press_slot::<Grab>().take() {
                        let before = MarkerSpan::new(marker.id, grab.start, grab.length);
                        report.span = Some(EditEvent::cancel(grab.id, cancel_reason(reason), Some(Mixed::Same(before))));
                    }
                }
                PressPhase::Release { click: true, .. } if press.mods.cmd => report.edits.push(MarkerEdit::Delete(marker.id)),
                _ => {}
            }
        }
        // Delete and Backspace delete the focused marker.
        if let Some(index) = cx.active_part().filter(|part| part.key == MARKER).and_then(|part| part.index) {
            let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
            let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
            if forward || backward {
                report.edits.push(MarkerEdit::Delete(MarkerId(index)));
            }
        }
        // The marker bin: a marker dragged out of it follows the pointer along the strip.
        let mut pulled = Pulled::default();
        if self.bin
            && let Some(result) = cx.part_if_declared(BIN)
        {
            let phase = result.press.phase;
            match phase {
                PressPhase::Drag { pos, .. } | PressPhase::DragStart { pos, .. } if strip.x_range().contains(pos.x) => {
                    pulled.at = Some(self.settle(&axis, reach, axis.tick_at(pos.x, strip.left()), None));
                }
                PressPhase::Release { pos, click: false, .. } if strip.x_range().contains(pos.x) => {
                    let at = self.settle(&axis, reach, axis.tick_at(pos.x, strip.left()), None);
                    report.edits.push(MarkerEdit::Add { at });
                }
                _ => {}
            }
        }
        pulled.dragged = cx.press_slot::<Grab>().as_ref().map(|grab| MarkerSpan::new(grab.marker, grab.now.0, grab.now.1));
        *cx.memory::<Pulled>() = pulled;
        self.report = report;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (style, rect) = (cx.style, cx.rect);
        let palette = label_palette(&cx.env);
        let tint = |color: MarkerColor| palette.0[usize::from(color.0) % palette.0.len()];
        let dragged = cx.recall::<Pulled>().dragged;
        let height = rect.height().min(style.height);
        let draw = ui.draw();
        let placed: Vec<(&Marker, Tick, Tick)> = self
            .sorted()
            .into_iter()
            .map(|marker| {
                let (start, length) = Self::shown(marker, dragged.as_ref());
                (marker, start, length)
            })
            .collect();
        // The duration bars and names first, then every flag above them.
        for &(marker, start, length) in &placed {
            let x = axis.x_of(start, rect.left());
            let end = (length > Tick::ZERO).then(|| axis.x_of(start + length, rect.left()));
            if let Some(end) = end {
                let bar = Rect::from_min_max(pos2(x, rect.top()), pos2(end, rect.top() + height));
                draw.data_fill(&PaintShape::Rect(bar, CornerRadius::ZERO), DataColor::new(tint(marker.color).gamma_multiply(DURATION_TINT)));
            }
            if marker.name.is_empty() {
                continue;
            }
            let run = TextRun::new(marker.name.as_str()).role(TypeRole::Caption).color(style.name);
            let layout = draw.layout(&run, f32::INFINITY);
            let width = layout.size().x + 2.0 * style.pad;
            if let Some(left) = Self::name_box_x(x, width, end, rect, style.pad, style.cap_gap) {
                let name_box = Rect::from_min_size(pos2(left, rect.top()), Vec2::new(width, height.max(layout.size().y)));
                let shape = PaintShape::Rect(name_box, style.radius);
                draw.fill(&shape, style.name_fill);
                draw.stroke(&shape, StrokeWidth::Hairline, style.name_edge);
                draw.galley(pos2(name_box.left() + style.pad, name_box.center().y - layout.size().y / 2.0), &layout);
            }
        }
        for &(marker, start, length) in &placed {
            let x = axis.x_of(start, rect.left());
            let color = DataColor::new(tint(marker.color));
            draw.data_fill(&PaintShape::Polygon(Self::flag(x, rect.top(), style.flag_width, height)), color);
            if length > Tick::ZERO {
                let end = axis.x_of(start + length, rect.left());
                let cap = Rect::from_min_max(pos2(end - style.cap / 2.0, rect.top()), pos2(end + style.cap / 2.0, rect.top() + height));
                draw.data_fill(&PaintShape::Rect(cap, CornerRadius::ZERO), color);
            }
        }
        if self.bin {
            let pulled = cx.recall::<Pulled>();
            let hot = pulled.at.is_some() || cx.part_if_declared(BIN).is_some_and(|result| result.hovered);
            let bin = Self::bin_rect(rect, height);
            draw.icon(bin, &IconSpec::svg(craft_behavior::IconId::new("flag")).color(if hot { style.bin_hot } else { style.bin_idle }));
            if let Some(at) = pulled.at {
                let x = axis.x_of(at, rect.left());
                draw.data_fill(
                    &PaintShape::Polygon(Self::flag(x, rect.top(), style.flag_width, height)),
                    DataColor::new(tint(MarkerColor::default())),
                );
            }
        }
    }

    /// A list box of the markers, each named by name and time.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let sorted = self.sorted();
        for (at, marker) in sorted.iter().enumerate() {
            let node = access.node(MARKER.at(marker.id.0), Role::LIST_BOX_OPTION);
            node.name = Name::Text(format!("{}, {}", marker.name, format_time(marker.start, &axis.format)));
            node.position = Some(SetPosition::new(at).of(sorted.len()));
        }
        if self.bin {
            access.node(BIN, Role::BUTTON).name = Name::Text(cx.words::<MarkerWords>().bin.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MarkerStripOutput {
        let Report { span, edits, context } = self.report;
        MarkerStripOutput { response: cx.response.clone(), span, edits, context }
    }
}

/// What the strip did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct MarkerStripOutput {
    /// The strip's response.
    pub response: Response,
    /// The marker being dragged and where it is: the app previews each `Update` and applies the `Commit`.
    pub span: Option<EditEvent<MarkerSpan>>,
    /// This pass's edits, in order.
    pub edits: Vec<MarkerEdit>,
    /// The marker that asked for its menu; the app opens it.
    pub context: Option<MarkerId>,
}

impl Outcome for MarkerStripOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        let mut facts = craft_ui::testing::StoryOutput::default();
        facts.id = Some(self.response.id);
        facts.clicked = clicked(&self.response);
        facts.edits = self.span.iter().filter_map(craft_ui::testing::EditKind::of).collect();
        facts
    }
}
