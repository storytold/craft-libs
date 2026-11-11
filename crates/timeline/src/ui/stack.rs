//! `TrackStack` and `TrackRow`: the tracks of one pane of a timeline on the collection contract (20.6).
//!
//! A stack is one collection keyed by [`TrackId`]. Its columns are the colour strip, the track name (the header width the
//! app keeps, frozen and resizable), one sweepable toggle column per switch of the [`TrackInfo`] and the lane. A row paints
//! its header in the cells and registers the lane's clips as parts of the lane cell through [`ClipLane`], so a track of
//! any number of clips costs one pipeline run. Switches reach the app as the collection's `Toggle` events, the width of
//! the header as its `Column` events; everything else a row reports is its [`TrackEvent`].

use super::clips::{ClipLane, ClipLaneEvents, ClipLook};
use super::view::{TimelineWords, time_axis};
use super::{DragEvent, LabelPalette, label_palette};
use crate::clips::ClipEdit;
use crate::model::{ClipId, TrackId, TrackInfo, TrackKind};
use crate::time::Tick;
use craft_behavior::{
    ColumnKey, ColumnWidth, CursorIcon, DisplayOrder, EditEvent, EditEvents, HitLevel, HitPriority, IconId, ItemSource, MarqueeSelect, Metrics,
    Mixed, PartKey, PressPhase, Selection, SelectionChange, Sense, SizeHint, SpanLayout,
};
use craft_ui::contract::{
    Access, Bind, BindMut, Collection, CollectionProps, CollectionResponse, ColumnKind, ColumnSpec, Common, Component, ContainerProps, Cx, Edit as _,
    ItemKeys, ItemOutput, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _, Props, Remember as _, Role, Scroll, ScrollAnchor, SelectMode,
    Sizing, Styled as _, ToggleColumn, WordSetCx as _,
};
use craft_ui::primitives::{IconSpec, PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, IconLook, Stateful, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole, UiTokens};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::sync::Arc;

/// The colour strip: a click on it opens the label menu.
pub const STRIP: ColumnKey = ColumnKey::new("strip");
/// The track name.
pub const NAME: ColumnKey = ColumnKey::new("name");
/// The lock switch.
pub const LOCK: ColumnKey = ColumnKey::new("lock");
/// The targeting switch.
pub const TARGET: ColumnKey = ColumnKey::new("target");
/// The sync lock switch.
pub const SYNC_LOCK: ColumnKey = ColumnKey::new("syncLock");
/// The output switch (the eye or the speaker).
pub const OUTPUT: ColumnKey = ColumnKey::new("output");
/// The mute switch.
pub const MUTE: ColumnKey = ColumnKey::new("mute");
/// The solo switch.
pub const SOLO: ColumnKey = ColumnKey::new("solo");
/// The lane with the clips.
pub const LANE: ColumnKey = ColumnKey::new("lane");

const LABEL: PartKey = PartKey::new("label");
const HEADER: PartKey = PartKey::new("header");
const EDGE_BOTTOM: PartKey = PartKey::new("heightBottom");
const EDGE_TOP: PartKey = PartKey::new("heightTop");

/// The switch columns in header order, each with its icon while on and off.
const SWITCHES: [(ColumnKey, &str, &str); 6] = [
    (LOCK, "lock", "lock_open"),
    (TARGET, "target", "target"),
    (SYNC_LOCK, "sync_lock", "sync_lock"),
    (OUTPUT, "eye", "eye_off"),
    (MUTE, "track_mute", "track_mute"),
    (SOLO, "solo", "solo"),
];

/// How strongly a track's colour label tints its header: a share of the label colour (FilmCraft `TRACK_HEADER_TINT`).
const HEADER_LABEL_TINT: f32 = 0.18;

/// The sizes of a timeline's tracks and headers, all from the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrackSizes {
    /// A video track's default height.
    pub video: f32,
    /// An audio or other track's default height.
    pub audio: f32,
    /// A caption track's default height.
    pub caption: f32,
    /// The least a track's height can be dragged to.
    pub least: f32,
    /// The most a track's height can be dragged to.
    pub most: f32,
    /// One control height: the width of a switch column.
    pub unit: f32,
}

/// A track grows to at most this many heights of a large thumbnail.
const MOST_IN_THUMBS: f32 = 4.0;
/// The name column's width until the user drags it.
pub(crate) const NAME_DEFAULT: ColumnWidth = ColumnWidth::Controls(6.0);
/// A switch column is one control wide.
const SWITCH_WIDTH: ColumnWidth = ColumnWidth::Controls(1.0);
/// The colour strip is a quarter of a control wide.
const STRIP_WIDTH: ColumnWidth = ColumnWidth::Controls(0.25);

impl TrackSizes {
    /// The sizes the tokens give.
    pub fn of(tokens: &UiTokens) -> TrackSizes {
        let unit = tokens.size.control.md;
        let tall = tokens.size.thumb.lg;
        TrackSizes { video: tall, audio: tokens.size.thumb.md, caption: unit, least: unit, most: tall * MOST_IN_THUMBS, unit }
    }

    /// The height of a track of `kind` whose own height is `own`: that, or the default of its kind, within the least
    /// and the most.
    pub fn height(&self, kind: TrackKind, own: Option<f32>) -> f32 {
        let default = match kind {
            TrackKind::Video => self.video,
            TrackKind::Caption => self.caption,
            TrackKind::Audio | TrackKind::Other => self.audio,
        };
        own.filter(|height| height.is_finite() && *height > 0.0).unwrap_or(default).clamp(self.least, self.most)
    }

    /// The height of `track`.
    pub fn height_of(&self, track: &TrackInfo<'_>) -> f32 {
        self.height(track.kind, track.height)
    }

    /// The height a double click on a header gives a track of height `now`: the tall one below the middle, else the
    /// short one.
    pub fn toggled(&self, now: f32) -> f32 {
        let (short, tall) = (self.caption, self.video);
        if now < (short + tall) / 2.0 { tall } else { short }
    }

    /// The width of the headers (colour strip, name and the switches) for a name column `name` wide: where the lane
    /// begins. The view reads it before its body has laid the stacks out, so it sums the same column widths the stacks
    /// declare.
    pub fn header_extent(&self, name: Option<ColumnWidth>) -> f32 {
        let fixed = |width: ColumnWidth| match width.in_unit(self.unit) {
            ColumnWidth::Fixed(width) => Some(width),
            _ => None,
        };
        let name = name.and_then(fixed).or_else(|| fixed(NAME_DEFAULT)).unwrap_or(0.0);
        let switches = SWITCHES.len() as f32 * fixed(SWITCH_WIDTH).unwrap_or(0.0);
        fixed(STRIP_WIDTH).unwrap_or(0.0) + name + switches
    }
}

/// The tracks of a pane as a collection source: keyed by track.
#[non_exhaustive]
pub struct Tracks<'a> {
    /// The tracks, top to bottom.
    pub tracks: &'a [TrackInfo<'a>],
}

impl<'a> Tracks<'a> {
    /// The source over `tracks`.
    pub fn new(tracks: &'a [TrackInfo<'a>]) -> Self {
        Tracks { tracks }
    }
}

impl DisplayOrder<TrackId> for Tracks<'_> {
    fn len(&self) -> usize {
        self.tracks.len()
    }

    fn key(&self, at: usize) -> TrackId {
        self.tracks[at].id
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(self.tracks[at].name))
    }
}

impl ItemSource<TrackId> for Tracks<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        let track = &self.tracks[at];
        let on = if *column == LOCK {
            track.locked
        } else if *column == TARGET {
            track.targeted
        } else if *column == SYNC_LOCK {
            track.sync_lock
        } else if *column == OUTPUT {
            track.enabled
        } else if *column == MUTE {
            track.muted
        } else {
            track.solo
        };
        on.unwrap_or(false)
    }

    fn toggle_enabled(&self, at: usize, column: &ColumnKey) -> bool {
        let track = &self.tracks[at];
        let switch = if *column == LOCK {
            track.locked
        } else if *column == TARGET {
            track.targeted
        } else if *column == SYNC_LOCK {
            track.sync_lock
        } else if *column == OUTPUT {
            track.enabled
        } else if *column == MUTE {
            track.muted
        } else {
            track.solo
        };
        switch.is_some()
    }
}

impl TrackEvent {
    /// Whether the row reported nothing in the pass: such a row emits no item event. A field added later counts
    /// without a change here.
    fn is_idle(&self) -> bool {
        *self == Self::default()
    }
}

/// What a track row reports beyond the collection's own events, in one pass.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TrackEvent {
    /// A clip gesture of the lane.
    pub clip: EditEvents<ClipEdit>,
    /// A change of the clip selection.
    pub select: Option<SelectionChange<ClipId>>,
    /// Cmd or Ctrl was held at the pick: the app leaves linked partners out.
    pub unlinked: bool,
    /// A double click on a clip.
    pub open: Option<ClipId>,
    /// A context menu on a clip, with the clip's rect and the pointer's position when a secondary click asked for it.
    pub context: Option<(ClipId, Rect, Option<Pos2>)>,
    /// The colour strip was clicked: the app shows the label menu.
    pub label_menu: bool,
    /// A drag of the row's edge, or a double click on the header.
    pub height: Option<DragEvent<f32>>,
    /// Where a clip move drag's pointer is over this row.
    pub pointer: Option<MovePointer>,
    /// Where a lane drag snapped.
    pub snapped_at: Option<Tick>,
    /// The marquee a drag on the lane's empty space sweeps.
    pub marquee: Option<MarqueeSelect<ClipId>>,
    /// The marquee was released in this pass.
    pub marquee_ended: bool,
}

/// The pointer of a clip move drag over the row that holds the dragged clips: the panel works the track change out
/// from it and the geometry of every row.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MovePointer {
    /// The pointer's y.
    pub y: f32,
    /// The row's rect.
    pub row: Rect,
}

/// The stack has no look of its own: its rows and the view's overlay draw everything.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TrackStackStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The tracks of one pane. The video pane keeps its last track nearest the divider by anchoring its scroll at the end.
#[derive(Component, Props)]
#[component(collection)]
pub struct TrackStack {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    /// The kind and own height of each track, in order.
    heights: Vec<(TrackKind, Option<f32>)>,
}

impl TrackStack {
    /// A stack named `label` whose name column is `name` wide (the default until the user drags it).
    pub fn new(label: impl Into<Label>, name: Option<ColumnWidth>, words: &TimelineWords) -> Self {
        let switch_name = |key: &ColumnKey| match () {
            _ if *key == LOCK => words.lock.clone(),
            _ if *key == TARGET => words.target.clone(),
            _ if *key == SYNC_LOCK => words.sync_lock.clone(),
            _ if *key == OUTPUT => words.output.clone(),
            _ if *key == MUTE => words.mute.clone(),
            _ => words.solo.clone(),
        };
        let mut columns = vec![
            ColumnSpec::new(STRIP, words.track_color.clone(), STRIP_WIDTH),
            ColumnSpec::new(NAME, "", name.unwrap_or(NAME_DEFAULT)).resizable().frozen(),
        ];
        for (key, _, _) in SWITCHES {
            let header = switch_name(&key);
            columns.push(ColumnSpec::new(key, header, SWITCH_WIDTH).kind(ColumnKind::Toggle(ToggleColumn::sweepable())));
        }
        columns.push(ColumnSpec::new(LANE, "", ColumnWidth::fill(1.0)));
        TrackStack {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill).scroll(Scroll::vertical()),
            items: CollectionProps::new().select(SelectMode::None).columns(columns).header(false).item_keys(ItemKeys::Owner),
            heights: Vec::new(),
        }
        .label(label)
    }

    /// The stack takes the height its rows need, up to the room, instead of all the room: a pane that sits above others.
    #[must_use]
    pub fn sized_to_rows(mut self) -> Self {
        self.props = self.props.sizing(Sizing::FillWidth);
        self
    }

    /// The stack stays scrolled to its end when its content grows: the video pane, whose first track is nearest the
    /// divider.
    #[must_use]
    pub fn anchored_at_end(mut self) -> Self {
        let mut scroll = Scroll::vertical();
        scroll.anchor = ScrollAnchor::End;
        self.props = self.props.scroll(scroll);
        self
    }

    /// The tracks the rows show: their heights follow the kind and the own height of each.
    #[must_use]
    pub fn tracks(mut self, tracks: &[TrackInfo<'_>]) -> Self {
        self.heights = tracks.iter().map(|track| (track.kind, track.height)).collect();
        self
    }
}

impl Component for TrackStack {
    const ROLE: Role = Role::LIST;
    const OVERLAYS: bool = false;
    type Style = TrackStackStyle;
    type Output = CollectionResponse<TrackId, TrackEvent>;

    fn style(_: &StyleCx<'_>, _: TrackStackStyleKey) -> TrackStackStyle {
        TrackStackStyle { metrics: Metrics::new() }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

/// The tracks' source with the heights the tokens resolve.
struct Heights<'s> {
    source: &'s dyn ItemSource<TrackId>,
    heights: Vec<f32>,
}

impl DisplayOrder<TrackId> for Heights<'_> {
    fn len(&self) -> usize {
        self.source.len()
    }

    fn key(&self, at: usize) -> TrackId {
        self.source.key(at)
    }
}

impl ItemSource<TrackId> for Heights<'_> {
    fn size_hint(&self, at: usize) -> SizeHint {
        self.heights.get(at).map_or(SizeHint::Auto, |height| SizeHint::Height(*height))
    }
}

impl Collection for TrackStack {
    type Key = TrackId;
    type ItemLayout = SpanLayout;
    type Selection = Selection<TrackId>;
    type Item<'r> = TrackRow<'r>;
    type ItemEvent = TrackEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    /// The column widths come from the specs; a row is as tall as its track says.
    fn item_layout(&self, source: &dyn ItemSource<TrackId>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        let sizes = TrackSizes::of(cx.tokens);
        let heights = self.heights.iter().map(|(kind, own)| sizes.height(*kind, *own)).collect();
        SpanLayout::sized(&Heights { source, heights }, width, cx.control().height)
    }
}

/// The style of a track row: the header's inks and fills.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TrackRowStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The track name.
    #[style(text)]
    name: TextLook,
    /// A switch while on.
    #[style(icon)]
    on: IconLook,
    /// A switch while off.
    #[style(icon)]
    off: IconLook,
    /// The line under the header.
    divider: StyleColor,
    /// The colour strip while the track has no label and the pointer is over it.
    hover: StyleColor,
    /// Between the strip and the name, and the indent of one level of depth.
    pad: f32,
    /// The reach of a height edge each side of the row's boundary.
    edge: f32,
    sizes: TrackSizes,
    /// The clips' fills and strokes.
    clip: ClipLook,
}

/// One track: its header in the header cells, its clips as parts of the lane cell.
#[derive(Component)]
#[component(content_painter, unregistered)]
pub struct TrackRow<'r> {
    #[common]
    common: Common,
    track: &'r TrackInfo<'r>,
    cells: Vec<(ColumnKey, Rect)>,
    /// The lane's clip gesture: the row's value is a [`ClipEdit`], and the idle value is an edit of no clips.
    edit: Bind<'r, ClipEdit>,
    lane: ClipLane<'r>,
    above_divider: bool,
    top_edge: bool,
    event: TrackEvent,
}

impl<'r> TrackRow<'r> {
    /// A row of `track` in `cells`, the rects the item context lists, with the lane's clips. `above_divider` rows grow upward; the top video row also has a top edge.
    pub(crate) fn new(track: &'r TrackInfo<'r>, cells: Vec<(ColumnKey, Rect)>, lane: ClipLane<'r>, above_divider: bool, top_edge: bool) -> Self {
        TrackRow {
            common: Common::new(),
            track,
            cells,
            edit: Bind::given(ClipEdit::Select { clips: Vec::new(), add: false }),
            lane,
            above_divider,
            top_edge,
            event: TrackEvent::default(),
        }
        .label(track.name.to_owned())
        .ltr_axis()
    }

    fn cell(&self, column: &ColumnKey) -> Option<Rect> {
        self.cells.iter().find(|(key, _)| key == column).map(|(_, rect)| *rect)
    }

    fn row(&self) -> Rect {
        self.cells.iter().fold(Rect::NOTHING, |all, (_, rect)| all.union(*rect))
    }

    fn headers(&self) -> Rect {
        self.cells.iter().filter(|(key, _)| *key != LANE).fold(Rect::NOTHING, |all, (_, rect)| all.union(*rect))
    }
}

impl Component for TrackRow<'_> {
    const ROLE: Role = Role::ROW;
    const OVERLAYS: bool = false;
    type Style = TrackRowStyle;
    type Output = TrackOut;

    fn style(cx: &StyleCx<'_>, _: TrackRowStyleKey) -> TrackRowStyle {
        let color = &cx.tokens.color;
        TrackRowStyle {
            surface: SurfaceLook::new(Stateful::new(cx.color(color.surface.panel))),
            metrics: Metrics::new(),
            name: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(color.text.normal))),
            on: IconLook::new(cx.control().icon, Stateful::new(cx.color(color.text.normal))),
            off: IconLook::new(cx.control().icon, Stateful::new(cx.color(color.text.faint))),
            divider: cx.color(color.line.divider),
            hover: cx.color(color.state.hover),
            pad: cx.tokens.space.pad.regular,
            edge: cx.tokens.size.hit.target / 2.0,
            clip: ClipLook::of(cx),
            sizes: TrackSizes::of(cx.tokens),
        }
    }

    /// The row has no shape of its own: the collection's takes a press on the row and gives the row the lead and the
    /// focus, and the lane's backdrop part takes the presses on empty lane space.
    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.row().size().max(Vec2::ZERO)
    }

    fn bind(&mut self) -> Option<BindMut<'_, ClipEdit>> {
        Some(self.edit.reborrow())
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        if let Some(strip) = self.cell(&STRIP) {
            cx.part(LABEL, strip).sense(Sense::CLICK).pointer_only("the track's context menu sets its colour label");
        }
        if let Some(name) = self.cell(&NAME) {
            cx.part(HEADER, name).sense(Sense::CLICK).pointer_only("the host's track height command");
        }
        let row = self.row();
        let reach = cx.style.edge;
        // The edge of a video row beside the divider reaches into the row only, so the divider keeps its own hit area.
        let beyond = if self.above_divider { 0.0 } else { reach };
        let drag = |part: PartKey, y: f32, beyond: f32, cx: &mut PartsCx<'_, Self>| {
            let grab = Rect::from_min_max(Pos2::new(row.left(), y - reach), Pos2::new(row.right(), y + beyond));
            cx.part(part, grab)
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Edge, 0))
                .cursor(CursorIcon::ResizeVertical)
                .pointer_only("the host's track height command");
        };
        drag(EDGE_BOTTOM, row.bottom(), beyond, cx);
        if self.top_edge {
            drag(EDGE_TOP, row.top(), reach, cx);
        }
        if let (Some(lane), Some(axis)) = (self.cell(&LANE), time_axis(&cx.env)) {
            let look = cx.style.clip.clone();
            self.lane.parts(lane, &axis, &look, cx);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<TimelineWords>();
        for (part, template) in [(EDGE_BOTTOM, &words.track_height), (EDGE_TOP, &words.track_height_top)] {
            if part == EDGE_TOP && !self.top_edge {
                continue;
            }
            let node = access.node(part, Role::SPLITTER);
            node.name = Name::Text(template.fill(&[("track", self.track.name)]));
        }
        let node = access.node(LABEL, Role::BUTTON);
        node.name = Name::Text(words.track_color.as_str().to_owned());
        if self.cell(&NAME).is_some() {
            let node = access.node(HEADER, Role::BUTTON);
            node.name = Name::Text(self.track.name.to_owned());
            node.description = Some(words.track_expand.fill(&[("track", self.track.name)]));
        }
        if let (Some(lane), Some(axis)) = (self.cell(&LANE), time_axis(&cx.env)) {
            self.lane.access(access, lane, &axis, &cx.style.clip);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let height = sizes.height_of(self.track);
        let sign = if self.above_divider { -1.0 } else { 1.0 };
        for part in [EDGE_BOTTOM, EDGE_TOP] {
            let Some(result) = cx.part_if_declared(part) else { continue };
            let phase = result.press.phase;
            let height_at = |held: f32, start_y: f32, y: f32| (held + sign * (y - start_y)).clamp(sizes.least, sizes.most);
            match phase {
                PressPhase::DragStart { .. } => {
                    *cx.press_slot::<f32>() = Some(height);
                    self.event.height = Some(DragEvent::Begin(height));
                }
                PressPhase::Drag { start, pos, .. } => {
                    let held = cx.press_slot::<f32>().unwrap_or(height);
                    self.event.height = Some(DragEvent::Update(height_at(held, start.origin.y, pos.y)));
                }
                PressPhase::Release { start, pos, click: false, .. } => {
                    let held = cx.press_slot::<f32>().take().unwrap_or(height);
                    self.event.height = Some(DragEvent::Commit(height_at(held, start.origin.y, pos.y)));
                }
                PressPhase::Cancel { .. } => {
                    *cx.press_slot::<f32>() = None;
                    self.event.height = Some(DragEvent::Cancel);
                }
                _ => {}
            }
        }
        if cx.part_activated(LABEL).is_some() {
            self.event.label_menu = true;
        }
        // A double click on the header, or AccessKit's click on its node, toggles the height between the two named heights.
        let double = matches!(cx.part(HEADER).press.phase, PressPhase::Release { start, click: true, .. } if start.clicks == 2);
        if double || matches!(cx.part_activated(HEADER), Some(FireSource::Access)) {
            self.event.height = Some(DragEvent::Set(sizes.toggled(height)));
        }
        if let (Some(lane), Some(axis)) = (self.cell(&LANE), time_axis(&cx.env)) {
            let look = cx.style.clip.clone();
            let events: ClipLaneEvents = self.lane.update(lane, &axis, &look, cx);
            self.event.select = events.select;
            self.event.unlinked = events.unlinked;
            self.event.open = events.open;
            self.event.context = events.context;
            self.event.snapped_at = events.snapped_at;
            self.event.marquee = events.marquee;
            self.event.marquee_ended = events.marquee_ended;
            self.event.pointer = events.pointer_y.map(|y| MovePointer { y, row: self.row() });
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rtl = cx.flow.is_rtl();
        let style = cx.style;
        let draw = ui.draw();
        let headers = self.headers();
        if let Some(label) = self.track.label {
            let palette: LabelPalette = label_palette(&cx.env);
            let tint = palette.0[usize::from(label.0) % palette.0.len()];
            draw.data_fill(&PaintShape::Rect(headers, CornerRadius::ZERO), DataColor::new(tint.gamma_multiply(HEADER_LABEL_TINT)));
            if let Some(strip) = self.cell(&STRIP) {
                draw.data_fill(&PaintShape::Rect(strip, CornerRadius::ZERO), DataColor::new(tint));
            }
        } else if let Some(strip) = self.cell(&STRIP).filter(|_| cx.part(LABEL).hovered) {
            draw.fill(&PaintShape::Rect(strip, CornerRadius::ZERO), style.hover);
        }
        let divider = [Pos2::new(headers.left(), headers.bottom()), Pos2::new(headers.right(), headers.bottom())];
        draw.stroke(&PaintShape::Path(divider.to_vec()), StrokeWidth::Hairline, style.divider);
        if let Some(cell) = self.cell(&NAME) {
            let indent = f32::from(self.track.depth) * style.pad;
            let flow = craft_behavior::Flow::new(cell, rtl);
            let text = flow.inset(craft_behavior::Insets::horizontal(style.pad + indent));
            draw.text(text, &craft_ui::primitives::TextRun::new(self.track.name).elide());
        }
        for (key, on_icon, off_icon) in SWITCHES {
            let Some(cell) = self.cell(&key) else { continue };
            let Some(on) = self.switch(&key) else { continue };
            let look = if on { &style.on } else { &style.off };
            let color = cx.resolve(&look.color);
            let icon = IconSpec::svg(IconId::new(if on { on_icon } else { off_icon })).color(color);
            draw.icon(Rect::from_center_size(cell.center(), Vec2::splat(look.side)), &icon);
        }
        if let (Some(lane), Some(axis)) = (self.cell(&LANE), time_axis(&cx.env)) {
            self.lane.paint(ui, lane, &axis, &style.clip, cx);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TrackOut {
        let mut event = self.event;
        event.clip = cx.edit_response().events;
        let event = (!event.is_idle()).then_some(event);
        TrackOut { response: cx.response.clone(), event }
    }
}

impl TrackRow<'_> {
    /// Whether the track's switch in `column` is on; `None` when the track has no such switch.
    fn switch(&self, column: &ColumnKey) -> Option<bool> {
        let track = self.track;
        if *column == LOCK {
            track.locked
        } else if *column == TARGET {
            track.targeted
        } else if *column == SYNC_LOCK {
            track.sync_lock
        } else if *column == OUTPUT {
            track.enabled
        } else if *column == MUTE {
            track.muted
        } else {
            track.solo
        }
    }
}

/// What a track row returns.
#[non_exhaustive]
pub struct TrackOut {
    /// The row's response.
    pub response: Response,
    /// What the row reported this pass.
    pub event: Option<TrackEvent>,
}

impl Outcome for TrackOut {
    type Value = ClipEdit;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<TrackEvent> for TrackOut {
    fn into_event(self) -> Option<TrackEvent> {
        self.event
    }
}

/// The kind of pane a track belongs to: where it sits against the divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Side {
    /// The caption tracks at the top.
    Caption,
    /// The video tracks above the divider, stacked upward from it.
    Upper,
    /// The audio and other tracks below the divider.
    Lower,
}

/// A track row's place for a clip move: `kind_index` counts away from the divider within the kind (V1 is 0, V2 is 1;
/// A1 is 0), `span` is its screen rows, `None` for a row the collection did not build (it still ranks and may be
/// locked, but no pointer is over it).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowGeom {
    /// The track's kind.
    pub kind: TrackKind,
    /// The pane the row is in.
    pub side: Side,
    /// The row's rank in its kind, counted from the divider.
    pub kind_index: i32,
    /// The track is locked.
    pub locked: bool,
    /// The row's top and bottom edges; `None` for a row that is not shown.
    pub span: Option<(f32, f32)>,
}

impl RowGeom {
    /// A row of kind `kind` in `side`, ranked `kind_index`, between `top` and `bottom`.
    pub fn new(kind: TrackKind, side: Side, kind_index: i32, top: f32, bottom: f32) -> Self {
        RowGeom { kind, side, kind_index, locked: false, span: Some((top, bottom)) }
    }

    /// A row of kind `kind` in `side`, ranked `kind_index`, that is not shown.
    pub fn unbuilt(kind: TrackKind, side: Side, kind_index: i32) -> Self {
        RowGeom { kind, side, kind_index, locked: false, span: None }
    }

    /// Whether the row lies over `y`.
    fn holds(&self, y: f32) -> bool {
        self.span.is_some_and(|(top, bottom)| top <= y && y < bottom)
    }

    /// The same row, locked.
    #[must_use]
    pub fn locked(mut self) -> Self {
        self.locked = true;
        self
    }
}

/// Where a clip move is headed: how many tracks it changes, whether it adds one, and whether it is refused because a
/// locked track is involved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MoveTarget {
    /// How many rows of the same kind the move changes.
    pub track_delta: i32,
    /// The pointer is past the last track of the kind, where a drop adds one.
    pub new_track: bool,
    /// The move starts on or reaches a locked track.
    pub refused: bool,
}

impl MoveTarget {
    /// A move that stays on its track.
    pub const STAY: MoveTarget = MoveTarget { track_delta: 0, new_track: false, refused: false };
}

/// What a drag at `y` does to a move from `sources` (the lane's own row first, then the rows of the other moved clips):
/// `track_delta` counts the rows the pointer's row is away from the lane's when it is of the same kind and 0 off every
/// row; `new_track` is a separate flag for the empty space past the last track of the kind, where the drop adds a
/// track. A move that starts on or reaches a locked track is refused and stays where it was.
pub fn resolve_move(rows: &[RowGeom], sources: &[RowGeom], y: f32, bounds: (f32, f32)) -> MoveTarget {
    let Some(source) = sources.first() else { return MoveTarget::STAY };
    if !(bounds.0..=bounds.1).contains(&y) {
        return MoveTarget::STAY;
    }
    let (track_delta, new_track) = match rows.iter().find(|row| row.holds(y)) {
        Some(row) if row.kind == source.kind => (row.kind_index - source.kind_index, false),
        Some(_) => (0, false),
        None if source.side == Side::Caption => (0, false),
        None => {
            let shown: Vec<(f32, f32)> = rows.iter().filter(|row| row.kind == source.kind).filter_map(|row| row.span).collect();
            let before_all = |(top, _): &(f32, f32)| y < *top;
            let after_all = |(_, bottom): &(f32, f32)| y >= *bottom;
            let past = !shown.is_empty() && if source.kind == TrackKind::Video { shown.iter().all(before_all) } else { shown.iter().all(after_all) };
            (0, past)
        }
    };
    let reaches_locked = |from: &RowGeom| {
        let count = rows.iter().filter(|row| row.kind == from.kind).count() as i32;
        let target = (from.kind_index + track_delta).clamp(0, (count - 1).max(0));
        rows.iter().any(|row| row.kind == from.kind && row.kind_index == target && row.locked)
    };
    if sources.iter().any(|from| from.locked || (track_delta != 0 && reaches_locked(from))) {
        return MoveTarget { track_delta: 0, new_track: false, refused: true };
    }
    MoveTarget { track_delta, new_track, refused: false }
}

/// The one-step event a header gesture without a drag reports (a width a column resize gave): a `Set`.
pub(crate) fn set_event<T>(before: T, after: T, label: &str) -> EditEvent<T> {
    EditEvent::set(Mixed::Same(before), after, Arc::from(label), None)
}

#[cfg(feature = "screen-testing")]
mod stories;
