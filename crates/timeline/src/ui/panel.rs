//! The timeline panel: the large timecode and the toolbar, the marker strip and the ruler over the time area, and the
//! caption, video and audio panes as three [`TrackStack`]s in one [`TimelineView`], over the app's [`TimelineModel`].
//! Every change the user makes comes out as a [`TimelineEdit`].

use super::clips::{ClipContent, ClipLane, TimelineTool};
use super::markers::{MarkerEdit, MarkerSpan, MarkerStrip};
use super::ruler::TimelineRuler;
use super::stack::{
    LANE, LOCK, MUTE, MovePointer, MoveTarget, NAME, NAME_DEFAULT, OUTPUT, RowGeom, SOLO, SYNC_LOCK, Side, TARGET, TrackEvent, TrackRow, TrackStack,
    Tracks, resolve_move, set_event,
};
use super::view::{Overlay, TimelineView, TimelineWords, time_axis};
use super::{DragEvent, LabelPalette, TimecodeField, label_palette};
use crate::clips::{ClipEdit, SnapTargets};
use crate::clock::FollowMode;
use crate::markers::{Marker, MarkerColor};
use crate::model::{ClipId, ClipInfo, TimelineModel, TrackId, TrackInfo, TrackKind};
use crate::time::{Tick, TimeRange};
use craft_behavior::{
    Axis, ColumnKey, ColumnWidth, CursorIcon, CursorPriority, EditEvent, EditEvents, GestureId, IconId, MarqueeSelect, Metrics, Mixed,
    SelectionChange,
};
use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::contract::{
    Bind, BodyLayout, CollectionEvent, CollectionResponse, ColumnEvent, Common, Component, Container, ContainerProps, Cx, EditResponse, NoValue,
    Outcome, Pattern, Props, Remember as _, Role, Show as _, ShowCollection as _, ShowContainer as _, ShownRow, SizeLimits, Sizing, SweepApply,
    SweepStep,
};
use craft_ui::display::Text;
use craft_ui::docking::SplitArea;
use craft_ui::docking::SplitFramesCx as _;
use craft_ui::docking::tree::FrameRules;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Pos2, Rect, Response, Ui};

/// A track header switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TrackSwitch {
    /// The track is locked against edits.
    Locked,
    /// The track is targeted by paste and insert.
    Targeted,
    /// The track moves with ripple edits on other tracks.
    SyncLock,
    /// The track's output is on (the eye or the speaker).
    Enabled,
    /// The track is muted.
    Muted,
    /// The track is soloed.
    Solo,
}

/// What the panel asks of the app. Clip and playhead edits come as `EditEvent` gestures: the app previews updates and
/// applies the commit as one undo step.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum TimelineEdit {
    /// A clip drag; see [`ClipEdit`].
    Clip(EditEvent<ClipEdit>),
    /// A playhead drag or click, as a new time.
    Playhead(EditEvent<Tick>),
    /// A drag of the work area's range, or the span a double click sets.
    WorkArea(DragEvent<TimeRange>),
    /// A marker was dragged: where it is, as the strip's gesture.
    MarkerSpan(EditEvent<MarkerSpan>),
    /// A marker gesture.
    Marker(MarkerEdit),
    /// A header switch was clicked; `value` is the new state. Alt-click on Solo or Mute (exclusive) arrives with
    /// `exclusive`.
    #[non_exhaustive]
    Switch {
        /// The track whose switch was clicked.
        track: TrackId,
        /// Which switch was clicked.
        switch: TrackSwitch,
        /// The switch's new state.
        value: bool,
        /// Alt was held: apply to every other track too.
        exclusive: bool,
    },
    /// The colour strip's menu.
    #[non_exhaustive]
    Label {
        /// The track whose label changed.
        track: TrackId,
        /// The new label, or `None` to clear it.
        label: Option<MarkerColor>,
    },
    /// A row's edge dragged, or a double click on its header: the new height in points, within the least and most
    /// of the track sizes, as one gesture (Esc cancels).
    #[non_exhaustive]
    TrackHeight {
        /// The track that is resized.
        track: TrackId,
        /// The new height, in points.
        height: DragEvent<f32>,
    },
    /// The name column dragged: its new width. A view setting the app keeps beside the `TimeView`, never in
    /// undo.
    HeaderWidth(EditEvent<ColumnWidth>),
    /// A click on a clip or empty track space, or a marquee: Premiere's picks (a click replaces, Shift+click toggles
    /// the clip, a Shift marquee adds; a click on empty space clears). `unlinked` with Cmd or Ctrl: the app leaves the
    /// clip's linked partners out. The app expands linked partners when Linked Selection is on, and keeps the
    /// selection ([`TimelineModel::selection`]).
    #[non_exhaustive]
    Select {
        /// The change of the clip selection.
        change: SelectionChange<ClipId>,
        /// Cmd or Ctrl was held: leave linked partners out.
        unlinked: bool,
    },
    /// The divider between the video and audio tracks dragged or moved by its keys: the video side's new share of the
    /// room below the captions (of what the divider leaves), as one gesture. A view setting the app keeps and hands
    /// back through [`Timeline::divider`], never in undo.
    Divider(EditEvent<f32>),
    /// A secondary click, or the Menu key or Shift+F10 with the time area focused: the app opens its menu. `anchor` is
    /// the clicked clip's or track's rect, `pointer` the click position, `None` for a key.
    #[non_exhaustive]
    ContextMenu {
        /// The track under the pointer, if any.
        track: Option<TrackId>,
        /// The clip under the pointer or focused, if any.
        clip: Option<ClipId>,
        /// The clicked clip's or track's rect, to anchor the menu.
        anchor: Rect,
        /// The click position; `None` for a key.
        pointer: Option<Pos2>,
    },
    /// A double click on a clip (Premiere opens it in the Source monitor).
    OpenClip(ClipId),
}

/// The share the video side gets when the app keeps none.
const DIVIDER_SHARE_DEFAULT: f32 = 0.5;

/// What the panel did this pass.
#[must_use]
#[non_exhaustive]
pub struct TimelineOutput {
    /// The panel's response, for hover and focus.
    pub response: Response,
    /// This pass's edits, in the order the user made them.
    pub edits: Vec<TimelineEdit>,
    /// The toolbar buttons' outputs; the app reacts to their clicks. `None` in a pass that did not run the body.
    pub toolbar: Option<TimelineToolbar>,
}

impl Outcome for TimelineOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The toolbar buttons' outputs: the Snap and Linked Selection toggles (the app flips its settings on a change), Add
/// Marker (the app adds the marker at the playhead; the panel reports no edit for it) and the display settings.
#[non_exhaustive]
pub struct TimelineToolbar {
    /// The Snap toggle button.
    pub snap: EditResponse<bool>,
    /// The Linked Selection toggle button.
    pub linked_selection: EditResponse<bool>,
    /// The Add Marker button; it is disabled while a marker sits at the playhead.
    pub add_marker: Response,
    /// The display settings button.
    pub settings: Response,
}

/// The Timeline panel (Premiere's Timeline; EffectCraft's layer timeline and SoundCraft's Edit window as options).
///
/// Layout, top to bottom: the large [`TimecodeField`] and the toolbar over the headers beside the [`MarkerStrip`] and the
/// [`TimelineRuler`]; then the caption tracks, the video tracks (V1 nearest the divider), the divider, the audio tracks
/// and the Mix row when the model has a master volume; the bar of the [`TimelineView`] at the bottom. The headers are the
/// header columns of each [`TrackStack`], `header_width` wide at the name column and resizable
/// ([`TimelineEdit::HeaderWidth`]); the lanes paint their clips' content through [`content`](Self::content).
///
/// Input: the wheel, the pinch and the keys of [`TimelineView`]; the colour strip of a header opens the label menu; the
/// Menu key and Shift+F10 report [`TimelineEdit::ContextMenu`] for the selection's last clip, else the first targeted
/// track. AccessKit: the time area is a `Group` named by [`TimelineWords::panel`], each track a `Row`, each height edge a
/// `Splitter`.
#[must_use = "show the panel with `show`"]
#[derive(Pattern, Props)]
#[pattern(probe(Timeline<'static, EmptySequence>))]
pub struct Timeline<'a, M: TimelineModel> {
    label: Label,
    model: &'a mut M,
    /// The tool that decides what a press on a clip does.
    #[prop]
    tool: TimelineTool,
    /// Snapping (Premiere's S); targets are clip edges, the playhead, markers, In and Out.
    #[prop]
    snap: bool,
    /// Whether Linked Selection is on, for its toggle's look; the app owns the setting.
    #[prop]
    linked_selection: bool,
    /// Allow the Selection tool to choose Roll and Ripple trims without a modifier key.
    #[prop]
    no_modifier_trims: bool,
    follow: FollowMode,
    playing: bool,
    /// The name column's width the user dragged to ([`TimelineEdit::HeaderWidth`]), kept by the app.
    #[prop(into)]
    header_width: Option<ColumnWidth>,
    /// The video side's share of the room below the captions that the user dragged the divider to
    /// ([`TimelineEdit::Divider`]), kept by the app.
    #[prop(into, quantity = "fraction")]
    divider: Option<f32>,
    /// Paints each clip's content into its body rect (waveforms, thumbnails).
    #[prop(func = "Fn(&craft_ui::primitives::ContentPainterCx<'_>, egui::Rect, egui::Rect, &ClipInfo<'_>)")]
    content: Option<ClipContent<'a>>,
}

impl<'a, M: TimelineModel> Timeline<'a, M> {
    /// The panel named `label` over `model`.
    pub fn new(label: impl Into<Label>, model: &'a mut M) -> Self {
        Timeline {
            label: label.into(),
            model,
            tool: TimelineTool::Selection,
            snap: true,
            linked_selection: true,
            no_modifier_trims: false,
            follow: FollowMode::Page,
            playing: false,
            header_width: None,
            divider: None,
            content: None,
        }
    }

    /// How the view follows the playhead while `playing`.
    pub fn follow(mut self, mode: FollowMode, playing: bool) -> Self {
        self.follow = mode;
        self.playing = playing;
        self
    }
}

/// What the panel keeps between passes, in its root container's slot.
#[derive(Clone, Debug, Default)]
struct Memory {
    /// The track whose label menu is open.
    label_menu: Option<TrackId>,
    /// Whether that menu shows.
    label_open: bool,
    /// The sweep reader of each switch column that has had one.
    sweeps: Vec<(ColumnKey, SweepApply)>,
    /// The Mute or Solo state of the pane's tracks from before an Alt sweep showed only one, which Escape puts back.
    before_show_only: Vec<(TrackId, bool)>,
    /// The running clip gesture's last reported Update, with the track fields the panel gave it.
    moving: Option<(GestureId, ClipEdit)>,
}

/// What the panes reported in a pass that the panel works out across all of them.
#[derive(Default)]
struct Reports {
    /// The row a clip move drag runs in, with the pointer over it.
    drag: Option<(TrackId, MovePointer)>,
    /// The marquee under way, and whether it ended in this pass.
    marquee: Option<(MarqueeSelect<ClipId>, bool)>,
    /// Where a drag snapped.
    snapped: Option<Tick>,
    /// The rows the collections showed this pass.
    shown: Vec<ShownRow<TrackId>>,
}

/// A track's row in the time area.
struct PlacedRow {
    /// The track's position in the model's tracks.
    track: usize,
    geom: RowGeom,
    /// Where the collection showed the row; none for a row it did not build.
    shown: Option<ShownRow<TrackId>>,
}

/// What the panes share in a pass.
struct Panes<'p, 'a> {
    words: &'p TimelineWords,
    name: Option<ColumnWidth>,
    selection: &'p [ClipId],
    targets: Option<&'p SnapTargets>,
    tool: TimelineTool,
    no_modifier_trims: bool,
    content: Option<&'p ClipContent<'a>>,
    clips: &'p [Vec<ClipInfo<'p>>],
}

impl Panes<'_, '_> {
    /// Shows one pane: the tracks `indices` of the model, as `pane`.
    fn show(&self, ui: &mut Ui, side: Side, pane: &[TrackInfo<'_>], indices: &[usize]) -> CollectionResponse<TrackId, TrackEvent> {
        let label = match side {
            Side::Caption => "Captions",
            Side::Upper => "Video",
            Side::Lower => "Audio",
        };
        let mut stack = TrackStack::new(label, self.name, self.words).tracks(pane);
        match side {
            Side::Upper => stack = stack.anchored_at_end(),
            // The caption tracks take the height their rows need, so the video and audio panes keep the rest.
            Side::Caption => stack = stack.sized_to_rows(),
            Side::Lower => {}
        }
        let source = Tracks::new(pane);
        stack.show_items(ui, &source, &mut |key, icx| {
            let at = pane.iter().position(|track| track.id == *key).unwrap_or(0);
            let track = &pane[at];
            let lane = ClipLane::new(&self.clips[indices[at]])
                .tool(self.tool)
                .no_modifier_trims(self.no_modifier_trims)
                .snap(self.targets)
                .locked(track.locked.unwrap_or(false))
                .selection(self.selection)
                .track_label(track.label);
            let lane = match self.content {
                Some(content) => lane.content(content.clone()),
                None => lane,
            };
            TrackRow::new(track, icx.cells().to_vec(), lane, side == Side::Upper, side == Side::Upper && at == 0)
        })
    }
}

/// What turns a pane's events into edits.
struct Mapping<'p> {
    pane: &'p [TrackInfo<'p>],
    words: &'p TimelineWords,
    name: Option<ColumnWidth>,
    palette: &'p LabelPalette,
}

impl Mapping<'_> {
    /// Maps what a pane reported to edits; a track whose colour strip was clicked opens the label menu.
    fn collect(&self, out: &CollectionResponse<TrackId, TrackEvent>, edits: &mut Vec<TimelineEdit>, memory: &mut Memory, reports: &mut Reports) {
        reports.shown.extend(out.shown.iter().cloned());
        let mut with_clip: Vec<TrackId> = Vec::new();
        for event in &out.events {
            match event {
                CollectionEvent::Toggle(edit) => {
                    let column = match edit {
                        EditEvent::Set { after: cell, .. } | EditEvent::Update { value: cell, .. } => Some(&cell.column),
                        EditEvent::Begin { before: Mixed::Same(cell), .. } | EditEvent::Cancel { before: Some(Mixed::Same(cell)), .. } => {
                            Some(&cell.column)
                        }
                        _ => None,
                    };
                    let step = match column {
                        Some(column) => {
                            let at = memory.sweeps.iter().position(|(key, _)| key == column).unwrap_or_else(|| {
                                let sweep = if *column == MUTE || *column == SOLO { SweepApply::show_only() } else { SweepApply::new() };
                                memory.sweeps.push((column.clone(), sweep));
                                memory.sweeps.len() - 1
                            });
                            memory.sweeps[at].1.feed(edit)
                        }
                        None => {
                            // A release ends the sweep of whichever column ran.
                            memory.sweeps.iter_mut().for_each(|(_, sweep)| _ = sweep.feed(edit));
                            None
                        }
                    };
                    let Some(step) = step else { continue };
                    let (toggled, exclusive) = match step {
                        SweepStep::Flip(cell) | SweepStep::Restore(cell) => (cell, false),
                        SweepStep::ShowOnly(cell) => (cell, true),
                        SweepStep::UndoShowOnly(cell) => {
                            // Every track the Alt sweep changed goes back, not only the one it started on.
                            let switch = if cell.column == SOLO { TrackSwitch::Solo } else { TrackSwitch::Muted };
                            for (track, value) in std::mem::take(&mut memory.before_show_only) {
                                edits.push(TimelineEdit::Switch { track, switch, value, exclusive: false });
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    let switch = match &toggled.column {
                        column if *column == LOCK => TrackSwitch::Locked,
                        column if *column == TARGET => TrackSwitch::Targeted,
                        column if *column == SYNC_LOCK => TrackSwitch::SyncLock,
                        column if *column == OUTPUT => TrackSwitch::Enabled,
                        column if *column == MUTE => TrackSwitch::Muted,
                        column if *column == SOLO => TrackSwitch::Solo,
                        _ => continue,
                    };
                    if exclusive {
                        let state = |track: &TrackInfo<'_>| if switch == TrackSwitch::Solo { track.solo } else { track.muted };
                        memory.before_show_only = self.pane.iter().filter_map(|track| Some((track.id, state(track)?))).collect();
                    }
                    let exclusive = exclusive && matches!(switch, TrackSwitch::Muted | TrackSwitch::Solo);
                    edits.push(TimelineEdit::Switch { track: toggled.key, switch, value: toggled.on, exclusive });
                }
                CollectionEvent::Column(ColumnEvent::Resized { key, width, .. }) if *key == NAME => {
                    edits.push(TimelineEdit::HeaderWidth(set_event(
                        self.name.unwrap_or(NAME_DEFAULT),
                        ColumnWidth::Fixed(*width),
                        self.words.header_width.as_str(),
                    )));
                }
                CollectionEvent::Item(track, row) => {
                    edits.extend(row.clip.iter().cloned().map(TimelineEdit::Clip));
                    if let Some(change) = row.select.clone() {
                        edits.push(TimelineEdit::Select { change, unlinked: row.unlinked });
                    }
                    edits.extend(row.open.map(TimelineEdit::OpenClip));
                    if let Some((clip, anchor, pointer)) = row.context {
                        with_clip.push(*track);
                        edits.push(TimelineEdit::ContextMenu { track: Some(*track), clip: Some(clip), anchor, pointer });
                    }
                    edits.extend(row.height.clone().map(|height| TimelineEdit::TrackHeight { track: *track, height }));
                    if row.label_menu {
                        memory.label_menu = Some(*track);
                        memory.label_open = true;
                    }
                    if let Some(pointer) = row.pointer {
                        reports.drag = Some((*track, pointer));
                    }
                    reports.snapped = reports.snapped.or(row.snapped_at);
                    if let Some(marquee) = row.marquee.clone() {
                        reports.marquee = Some((marquee, row.marquee_ended));
                    }
                }
                _ => {}
            }
        }
        for event in &out.events {
            let CollectionEvent::ContextMenu(request) = event else { continue };
            if !with_clip.contains(&request.key) && self.pane.iter().any(|track| track.id == request.key) {
                let Some(anchor) = out.shown.iter().find(|row| row.key == request.key).map(|row| row.rect) else { continue };
                edits.push(TimelineEdit::ContextMenu { track: Some(request.key), clip: None, anchor, pointer: Some(request.at) });
            }
        }
    }

    /// The label menu of the track in this pane whose strip was clicked: None, then the sixteen names.
    fn label_menu(&self, ui: &mut Ui, anchor: &Response, memory: &mut Memory, edits: &mut Vec<TimelineEdit>) {
        let Some(track) = memory.label_menu.and_then(|id| self.pane.iter().find(|track| track.id == id)) else { return };
        let (chosen, closed) = label_menu(ui, self.words, self.palette, track.label, anchor, &mut memory.label_open);
        if let Some(label) = chosen {
            edits.push(TimelineEdit::Label { track: track.id, label });
        }
        if closed {
            memory.label_menu = None;
            memory.label_open = false;
        }
    }
}

/// The menu of a track's colour label, open while `open`: the label chosen (`Some(None)` for no label) and whether the
/// menu closed in this pass.
fn label_menu(
    ui: &mut Ui,
    words: &TimelineWords,
    palette: &LabelPalette,
    current: Option<MarkerColor>,
    anchor: &Response,
    open: &mut bool,
) -> (Option<Option<MarkerColor>>, bool) {
    let menu = Menu::context(words.track_color.clone(), anchor).open(Bind::bound(open)).show(ui, |ui, _| {
        let mut chosen = None;
        if MenuItem::new(words.no_label.clone()).check(current.is_none()).show(ui).ran {
            chosen = Some(None);
        }
        for n in 0..=15 {
            let label = MarkerColor(n);
            let swatch = IconSpec::swatch(craft_theme::Srgba::from(palette.0[usize::from(n) % palette.0.len()]));
            if MenuItem::new(words.label_name(label).to_owned()).icon(swatch).check(current == Some(label)).show(ui).ran {
                chosen = Some(Some(label));
            }
        }
        chosen
    });
    (menu.inner.flatten(), menu.output.closed.is_some())
}

impl<M: TimelineModel> Timeline<'_, M> {
    /// Shows the panel and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> TimelineOutput {
        let Timeline { label, model, tool, snap, linked_selection, no_modifier_trims, follow, playing, header_width, divider, content } = self;
        let env = Env::of(ui);
        let words = env.words::<TimelineWords>();
        let mut edits: Vec<TimelineEdit> = Vec::new();
        let mut toolbar: Option<TimelineToolbar> = None;
        let view = *model.view();

        let mut selected_clip: Option<(TrackId, Rect)> = None;
        let output = {
            let model: &M = &*model;
            let tracks = model.tracks();
            let format = model.format();
            let (duration, playhead) = (model.duration(), model.playhead());
            let selection = model.selection().to_vec();
            let points = model.edit_points();
            let markers = model.markers();
            let (mark_in, mark_out) = model.in_out();
            let work_area = model.work_area();
            let visible = TimeRange::from_bounds(view.start, view.time_at(ui.available_width(), 0.0));
            let clips: Vec<Vec<ClipInfo<'_>>> = tracks.iter().map(|track| model.clips(track.id, visible)).collect();
            let targets = snap.then(|| {
                let mut targets = SnapTargets::new();
                targets.ranges(clips.iter().flatten().map(|clip| clip.range));
                targets.push(playhead);
                targets.extend(markers.iter().map(|marker| marker.start));
                targets.extend(mark_in.into_iter().chain(mark_out));
                targets.ranges(work_area);
                targets
            });
            let of_kind = |keep: fn(TrackKind) -> bool| -> Vec<usize> { (0..tracks.len()).filter(|&i| keep(tracks[i].kind)).collect() };
            let panes = [
                (Side::Caption, of_kind(|kind| kind == TrackKind::Caption)),
                (Side::Upper, of_kind(|kind| kind == TrackKind::Video)),
                (Side::Lower, of_kind(|kind| !matches!(kind, TrackKind::Caption | TrackKind::Video))),
            ];
            let shared = Panes {
                words: &words,
                name: header_width,
                selection: &selection,
                targets: targets.as_ref(),
                tool,
                no_modifier_trims,
                content: content.as_ref(),
                clips: &clips,
            };
            let palette = label_palette(&env.view());
            let mut timecode_edit = EditEvents::new();
            let mut ruler_edits: Vec<TimelineEdit> = Vec::new();
            let mut ruler_snapped: Option<Tick> = None;

            let overlay = Overlay::default();
            let area = TimelineView::new(label, Bind::given(view), format, duration)
                .headers(true)
                .follow(follow, playing, playhead)
                .edit_points(&points)
                .overlay(&overlay);
            let area = match header_width {
                Some(width) => area.header_width(width),
                None => area,
            };
            let shown = area.show(ui, |ui, cx| {
                TopStrip::new().show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        let field = TimecodeField::new(words.panel.clone(), Bind::given(playhead), format).large().show(ui);
                        timecode_edit = field.edit.events;
                        toolbar = toolbar_row(ui, &words, snap, linked_selection, markers, playhead);
                    });
                    Stack::new().show(ui, |ui, _| {
                        let strip = MarkerStrip::new(words.overview.clone(), markers);
                        let strip = match shared.targets {
                            Some(targets) => strip.snap(targets),
                            None => strip,
                        };
                        let strip = strip.show(ui);
                        ruler_edits.extend(strip.span.map(TimelineEdit::MarkerSpan));
                        ruler_edits.extend(strip.edits.into_iter().map(TimelineEdit::Marker));
                        let ruler = TimelineRuler::new(words.overview.clone(), Bind::given(playhead)).in_out((mark_in, mark_out));
                        let ruler = match work_area {
                            Some(range) => ruler.work_area(range),
                            None => ruler,
                        };
                        let ruler = ruler.show(ui);
                        ruler_edits.extend(ruler.playhead.events.into_iter().map(TimelineEdit::Playhead));
                        ruler_edits.extend(ruler.work_area.map(TimelineEdit::WorkArea));
                        ruler_snapped = ruler.snapped_at;
                    });
                });
                let mut memory = std::mem::take(cx.keep::<Memory>());
                let mut reports = Reports { snapped: ruler_snapped, ..Reports::default() };
                let mut pane_rects: Vec<Rect> = Vec::new();
                let both = !panes[1].1.is_empty() && !panes[2].1.is_empty();
                let share = divider.filter(|share| share.is_finite()).map_or(DIVIDER_SHARE_DEFAULT, |share| share.clamp(0.0, 1.0));
                let mut show_pane = |ui: &mut Ui, side: Side, indices: &[usize]| {
                    let pane: Vec<TrackInfo<'_>> = indices.iter().map(|&i| tracks[i].clone()).collect();
                    let out = shared.show(ui, side, &pane, indices);
                    pane_rects.push(out.response.rect);
                    let mapping = Mapping { pane: &pane, words: &words, name: header_width, palette: &palette };
                    mapping.collect(&out, &mut edits, &mut memory, &mut reports);
                    mapping.label_menu(ui, &out.response, &mut memory, &mut edits);
                };
                // The video and audio panes are the two bodies of one split area: its divider shares the room.
                let limits = |_: &Side| SizeLimits::default();
                let stripless = |_: &Side| true;
                let rules = FrameRules::new(&limits).stripless(&stripless);
                for (side, indices) in &panes {
                    if indices.is_empty() || (both && matches!(side, Side::Upper | Side::Lower)) {
                        continue;
                    }
                    show_pane(ui, *side, indices);
                }
                if both {
                    let split = SplitArea::pair(words.tracks.clone(), Axis::Y, (Side::Upper, Side::Lower), share, rules)
                        .divider_label(words.divider.clone())
                        .show(ui, |ui, body| {
                            for (side, rect) in body.group_frames::<Side>() {
                                let Some((_, indices)) = panes.iter().find(|(pane, _)| *pane == side) else { continue };
                                Stack::new().show_at(ui, rect, |ui, _| show_pane(ui, side, indices));
                            }
                        });
                    edits.extend(split.output.share().into_iter().map(TimelineEdit::Divider));
                }
                let placed = placed_rows(&panes, &tracks, &reports.shown);
                if let Some((dragged, pointer)) = reports.drag {
                    let bounds = pane_rects
                        .iter()
                        .fold((f32::INFINITY, f32::NEG_INFINITY), |(top, bottom), rect| (top.min(rect.top()), bottom.max(rect.bottom())));
                    let moved = edits
                        .iter()
                        .find_map(|edit| match edit {
                            TimelineEdit::Clip(event) => match event.value() {
                                Some(ClipEdit::Move { clips, .. }) => Some(clips.clone()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .or_else(|| match &memory.moving {
                            Some((_, ClipEdit::Move { clips, .. })) => Some(clips.clone()),
                            _ => None,
                        });
                    if let Some(moved) = moved {
                        let everything = TimeRange::from_bounds(Tick::ZERO, Tick::MAX);
                        let own = |row: &PlacedRow| tracks[row.track].id == dragged;
                        let holds = |row: &PlacedRow| model.clips(tracks[row.track].id, everything).iter().any(|clip| moved.contains(&clip.id));
                        let sources: Vec<RowGeom> = placed
                            .iter()
                            .filter(|row| own(row))
                            .chain(placed.iter().filter(|row| !own(row) && holds(row)))
                            .map(|row| row.geom)
                            .collect();
                        let rows: Vec<RowGeom> = placed.iter().map(|row| row.geom).collect();
                        let target = resolve_move(&rows, &sources, pointer.y, bounds);
                        for edit in &mut edits {
                            if let TimelineEdit::Clip(event) = edit {
                                retarget(event, target);
                            }
                        }
                        // The lane reports a move only when its time shift changes, so a drag that only crosses rows
                        // reaches here with no event: the panel reports the new track fields itself.
                        let reported = edits.iter().any(|edit| matches!(edit, TimelineEdit::Clip(_)));
                        if !reported && let Some((id, last)) = &memory.moving {
                            let mut event = EditEvent::update(*id, last.clone());
                            retarget(&mut event, target);
                            if event.value() != Some(last) {
                                edits.push(TimelineEdit::Clip(event));
                            }
                        }
                        if target.refused {
                            cx.cursor(CursorIcon::NotAllowed, CursorPriority::Drag);
                        }
                    }
                }
                for edit in &edits {
                    match edit {
                        TimelineEdit::Clip(EditEvent::Update { id, value, .. }) => memory.moving = Some((*id, value.clone())),
                        TimelineEdit::Clip(_) => memory.moving = None,
                        _ => {}
                    }
                }
                let mut marquee_rect = None;
                if let Some((marquee, ended)) = &reports.marquee {
                    let inner = Env::of(ui);
                    if let Some(axis) = time_axis(&inner.view()) {
                        let axis = &axis;
                        let items =
                            placed.iter().filter_map(|row| Some((row, row.shown.as_ref()?, row.lane_left()?))).flat_map(|(row, shown, lane_left)| {
                                let (top, bottom) = (shown.rect.top(), shown.rect.bottom());
                                clips[row.track].iter().map(move |clip| {
                                    let (x0, x1) = (axis.x_of(clip.range.start, lane_left), axis.x_of(clip.range.end(), lane_left));
                                    (clip.id, Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, bottom)))
                                })
                            });
                        let keys = marquee.selection(items);
                        if keys != selection {
                            let change = if keys.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(keys) };
                            edits.push(TimelineEdit::Select { change, unlinked: false });
                        }
                    }
                    marquee_rect = (!ended).then(|| marquee.rect());
                }
                if let (Some(last), Some(axis)) = (selection.last(), time_axis(&Env::of(ui).view())) {
                    selected_clip = placed.iter().find_map(|row| {
                        let (shown, lane_left) = (row.shown.as_ref()?, row.lane_left()?);
                        let clip = clips[row.track].iter().find(|clip| clip.id == *last)?;
                        let (x0, x1) = (axis.x_of(clip.range.start, lane_left), axis.x_of(clip.range.end(), lane_left));
                        Some((tracks[row.track].id, Rect::from_min_max(Pos2::new(x0, shown.rect.top()), Pos2::new(x1, shown.rect.bottom()))))
                    });
                }
                overlay.snapped(reports.snapped);
                overlay.marquee(marquee_rect);
                let lane_left = reports.shown.iter().find_map(lane_left_of);
                if let (Some(lane_left), Some(panes)) = (lane_left, pane_rects.iter().copied().reduce(Rect::union)) {
                    overlay.panes(panes, lane_left);
                }
                if let Some(db) = model.master_volume_db() {
                    mix_row(ui, &words, &env.view().format().number(f64::from(db), 1));
                }
                if tracks.is_empty() {
                    Text::new(words.empty.clone()).show(ui);
                }
                *cx.keep::<Memory>() = memory;
            });
            edits.extend(timecode_edit.into_iter().map(TimelineEdit::Playhead));
            edits.extend(ruler_edits);
            shown.output
        };
        edits.extend(output.playhead.clone().map(TimelineEdit::Playhead));
        if let Some(next) = output.view.events.value() {
            *model.view() = *next;
        }
        if output.context_menu {
            let tracks = model.tracks();
            let clip = model.selection().last().copied();
            let (track, anchor) = match selected_clip {
                Some((track, rect)) => (Some(track), rect),
                None => {
                    let targeted = tracks.iter().find(|track| track.targeted == Some(true)).or_else(|| tracks.first());
                    (targeted.map(|track| track.id), output.view.response.rect)
                }
            };
            edits.push(TimelineEdit::ContextMenu { track, clip, anchor, pointer: None });
        }
        TimelineOutput { response: output.view.response, edits, toolbar }
    }
}

/// Where each pane's rows are, as the collections laid them out. A row the collection did not build (scrolled out of
/// the pane) has no rect: it still ranks and may be locked, but no pointer is over it.
fn placed_rows(panes: &[(Side, Vec<usize>); 3], tracks: &[TrackInfo<'_>], shown: &[ShownRow<TrackId>]) -> Vec<PlacedRow> {
    let mut placed = Vec::new();
    for (side, indices) in panes {
        for (at, &track) in indices.iter().enumerate() {
            let kind = tracks[track].kind;
            let same = |other: &usize| tracks[indices[*other]].kind == kind;
            // Ranks count away from the divider: the video side is stacked upward from it.
            let rank = if *side == Side::Upper { (at + 1..indices.len()).filter(same).count() } else { (0..at).filter(same).count() };
            let shown = shown.iter().find(|row| row.key == tracks[track].id).cloned();
            let mut geom = match &shown {
                Some(row) => RowGeom::new(kind, *side, rank as i32, row.rect.top(), row.rect.bottom()),
                None => RowGeom::unbuilt(kind, *side, rank as i32),
            };
            if tracks[track].locked == Some(true) {
                geom = geom.locked();
            }
            placed.push(PlacedRow { track, geom, shown });
        }
    }
    placed
}

/// The left edge of a shown row's lane cell.
fn lane_left_of(row: &ShownRow<TrackId>) -> Option<f32> {
    row.cells.iter().find(|(key, _)| *key == LANE).map(|(_, rect)| rect.left())
}

impl PlacedRow {
    fn lane_left(&self) -> Option<f32> {
        self.shown.as_ref().and_then(lane_left_of)
    }
}

/// Sets the track fields of a clip move event.
fn retarget(event: &mut EditEvent<ClipEdit>, target: MoveTarget) {
    let value = match event {
        EditEvent::Update { value, .. } => value,
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => after,
        _ => return,
    };
    if let ClipEdit::Move { track_delta, new_track, .. } = value {
        *track_delta = target.track_delta;
        *new_track = target.new_track;
    }
}

/// The toolbar under the timecode: Snap, Linked Selection, Add Marker and the display settings.
fn toolbar_row(
    ui: &mut Ui,
    words: &TimelineWords,
    snap: bool,
    linked_selection: bool,
    markers: &[Marker],
    playhead: Tick,
) -> Option<TimelineToolbar> {
    let shown = Row::new().show(ui, |ui, _| {
        let snap = ToggleButton::new(words.snap.clone(), Bind::given(snap)).icon(IconSpec::svg(IconId::new("snap"))).icon_only().show(ui);
        let linked = ToggleButton::new(words.linked_selection.clone(), Bind::given(linked_selection))
            .icon(IconSpec::svg(IconId::new("linked_selection")))
            .icon_only()
            .show(ui);
        let refused = markers.iter().any(|marker| marker.start == playhead);
        let add_marker = IconButton::new(IconSpec::svg(IconId::new("flag"))).label(words.add_marker.clone()).enabled(!refused).show(ui).response;
        let settings = IconButton::new(IconSpec::svg(IconId::new("settings"))).label(words.settings.clone()).show(ui).response;
        (snap, linked, add_marker, settings)
    });
    shown.inner.map(|(snap, linked_selection, add_marker, settings)| TimelineToolbar { snap, linked_selection, add_marker, settings })
}

/// The Mix row: its name and the master volume in dB.
fn mix_row(ui: &mut Ui, words: &TimelineWords, value: &str) {
    Row::new().show(ui, |ui, _| {
        Text::new(words.mix.clone()).show(ui);
        Text::new(value.to_owned()).show(ui);
    });
}

/// The top strip: the head block over the headers, then the marker strip and the ruler over the time area.
#[derive(Component)]
#[component(container)]
struct TopStrip {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl TopStrip {
    fn new() -> Self {
        const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::fill(1.0)];
        TopStrip { common: Common::new(), props: ContainerProps::new().sizing(Sizing::FillWidth).layout(BodyLayout::grid(COLUMNS)) }
    }
}

impl Component for TopStrip {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    type Style = TopStripStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TopStripStyleKey) -> TopStripStyle {
        TopStripStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for TopStrip {}

/// The top strip has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TopStripStyle {
    #[style(metrics)]
    metrics: Metrics,
}

#[cfg(feature = "screen-testing")]
mod stories;

/// The model of the pattern's registration probe: no tracks.
struct EmptySequence(crate::time::TimeView);

impl TimelineModel for EmptySequence {
    fn duration(&self) -> Tick {
        Tick::ZERO
    }

    fn format(&self) -> crate::timecode::TimeFormat {
        crate::timecode::TimeFormat::timecode(crate::rates::FrameRate::FPS_29_97)
    }

    fn playhead(&self) -> Tick {
        Tick::ZERO
    }

    fn view(&mut self) -> &mut crate::time::TimeView {
        &mut self.0
    }

    fn tracks(&self) -> Vec<TrackInfo<'_>> {
        Vec::new()
    }

    fn clips(&self, _: TrackId, _: TimeRange) -> Vec<ClipInfo<'_>> {
        Vec::new()
    }

    fn selection(&self) -> &[ClipId] {
        &[]
    }
}
