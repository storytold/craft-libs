//! Stories of `Timeline`: a 30 s sequence at 29.97 drop-frame over an in-memory model: V1 and V2 with four clips (one
//! selected, one disabled), A1 and A2 with two, a marker, In and Out set and the playhead at 5 s.

use super::{Timeline, TimelineEdit, TopStrip};
use crate::clips::ClipEdit;
use crate::markers::{Marker, MarkerColor, MarkerId};
use crate::model::{ClipId, ClipInfo, TimelineModel, TrackId, TrackInfo, TrackKind};
use crate::rates::FrameRate;
use crate::time::{Tick, TimeRange, TimeView};
use crate::timecode::TimeFormat;
use craft_behavior::{ColumnWidth, Selection, SelectionModel as _};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::{Arc, Mutex};

const RATE: FrameRate = FrameRate::FPS_29_97;
const SEQUENCE_SECONDS: f64 = 30.0;
/// The seconds the panel shows when the story starts.
const SHOWN_SECONDS: f64 = 32.0;

const V1: TrackId = TrackId(1);
const V2: TrackId = TrackId(2);
const A1: TrackId = TrackId(3);
const A2: TrackId = TrackId(4);

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn span(from: f64, to: f64) -> TimeRange {
    TimeRange::from_bounds(secs(from), secs(to))
}

struct TrackData {
    id: TrackId,
    kind: TrackKind,
    name: &'static str,
    locked: bool,
    muted: bool,
}

struct ClipData {
    id: ClipId,
    track: TrackId,
    range: TimeRange,
    name: &'static str,
    label: u8,
    enabled: bool,
}

/// The sample sequence: the model the panel reads and the edits are applied to.
struct Sequence {
    tracks: Vec<TrackData>,
    clips: Vec<ClipData>,
    markers: Vec<Marker>,
    selection: Selection<ClipId>,
    view: TimeView,
    playhead: Tick,
    header_width: Option<ColumnWidth>,
    divider: Option<f32>,
}

impl TimelineModel for Sequence {
    fn duration(&self) -> Tick {
        secs(SEQUENCE_SECONDS)
    }

    fn format(&self) -> TimeFormat {
        TimeFormat::timecode(RATE).drop_frame(true)
    }

    fn playhead(&self) -> Tick {
        self.playhead
    }

    fn view(&mut self) -> &mut TimeView {
        &mut self.view
    }

    fn tracks(&self) -> Vec<TrackInfo<'_>> {
        self.tracks
            .iter()
            .map(|t| {
                let audio = t.kind == TrackKind::Audio;
                TrackInfo {
                    id: t.id,
                    kind: t.kind,
                    name: t.name,
                    height: None,
                    depth: 0,
                    label: None,
                    locked: Some(t.locked),
                    targeted: Some(true),
                    sync_lock: None,
                    enabled: Some(true),
                    muted: audio.then_some(t.muted),
                    solo: audio.then_some(false),
                }
            })
            .collect()
    }

    fn clips(&self, track: TrackId, visible: TimeRange) -> Vec<ClipInfo<'_>> {
        self.clips
            .iter()
            .filter(|c| c.track == track && c.range.overlaps(&visible))
            .map(|c| ClipInfo {
                id: c.id,
                track: c.track,
                range: c.range,
                name: c.name,
                label: Some(MarkerColor(c.label)),
                enabled: c.enabled,
                source: Some(span((c.range.start.seconds() - 3.0).max(0.0), c.range.end().seconds() + 3.0)),
            })
            .collect()
    }

    fn selection(&self) -> &[ClipId] {
        &self.selection.items
    }

    fn markers(&self) -> &[Marker] {
        &self.markers
    }

    fn edit_points(&self) -> Vec<Tick> {
        let mut points: Vec<Tick> = self.clips.iter().flat_map(|c| [c.range.start, c.range.end()]).collect();
        points.sort();
        points.dedup();
        points
    }

    fn in_out(&self) -> (Option<Tick>, Option<Tick>) {
        (Some(secs(2.0)), Some(secs(22.0)))
    }
}

fn sample_sequence(points_per_second: f64) -> Sequence {
    let track = |id, kind, name| TrackData { id, kind, name, locked: false, muted: false };
    let clip = |id: u64, track, name, from, to, label: u8, enabled| ClipData { id: ClipId(id), track, range: span(from, to), name, label, enabled };
    Sequence {
        tracks: vec![
            track(V2, TrackKind::Video, "V2"),
            track(V1, TrackKind::Video, "V1"),
            track(A1, TrackKind::Audio, "A1"),
            track(A2, TrackKind::Audio, "A2"),
        ],
        clips: vec![
            clip(1, V1, "Interview A-cam", 0.0, 12.0, 1, true),
            clip(2, V1, "City b-roll", 12.0, 26.0, 5, true),
            clip(3, V2, "Title card", 2.0, 9.0, 7, true),
            clip(4, V2, "Logo sting", 14.0, 20.0, 3, false),
            clip(5, A1, "Voice-over", 0.0, 22.0, 2, true),
            clip(6, A2, "Music bed", 1.0, 29.0, 10, true),
        ],
        markers: vec![Marker {
            id: MarkerId(1),
            start: secs(12.0),
            duration: Tick::ZERO,
            name: "Check this cut".into(),
            comment: String::new(),
            color: MarkerColor(0),
        }],
        selection: Selection::single(ClipId(2)),
        view: TimeView::new(Tick::ZERO, points_per_second),
        playhead: secs(5.0),
        header_width: None,
        divider: None,
    }
}

impl Sequence {
    /// Applies an edit as an app would: the settled value of a gesture, a switch, a width, a share.
    fn apply(&mut self, edit: &TimelineEdit) {
        match edit {
            TimelineEdit::Playhead(event) => {
                if let Some(&t) = event.value() {
                    self.playhead = t.clamp(Tick::ZERO, secs(SEQUENCE_SECONDS));
                }
            }
            TimelineEdit::Select { change, .. } => self.selection.apply(change.clone()),
            TimelineEdit::HeaderWidth(event) => self.header_width = event.value().copied(),
            TimelineEdit::Divider(event) => self.divider = event.value().copied(),
            TimelineEdit::Clip(event) => {
                if let Some(ClipEdit::Move { clips, delta, .. }) = event.value() {
                    for id in clips {
                        if let Some(clip) = self.clips.iter_mut().find(|c| c.id == *id) {
                            clip.range.start = (clip.range.start + *delta).max(Tick::ZERO);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The sequence as story data, compared by identity.
#[derive(Clone)]
struct Shared(Arc<Mutex<Sequence>>);

impl PartialEq for Shared {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[story(Timeline<'static, super::EmptySequence>, covers = [Default, Many], drive = Drive::Never)]
fn timeline_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let width = f64::from(ui.available_width());
    let shared = data.value("sequence", Shared(Arc::new(Mutex::new(sample_sequence(width / SHOWN_SECONDS))))).clone();
    let mut sequence = shared.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (header_width, divider) = (sequence.header_width, sequence.divider);
    let mut timeline = Timeline::new("Timeline", &mut *sequence);
    if let Some(width) = header_width {
        timeline = timeline.header_width(width);
    }
    if let Some(share) = divider {
        timeline = timeline.divider(share);
    }
    let out = timeline.show(ui);
    for edit in &out.edits {
        sequence.apply(edit);
    }
    out.into()
}

/// The strip over the headers and the time area: the head block in the first column, the ruler row in the second.
#[story(TopStrip, covers = [Default], drive = Drive::Never)]
fn top_strip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let out = TopStrip::new().show(ui, |ui, _| {
        Text::new("Head").show(ui);
        Text::new("Ruler").show(ui);
    });
    out.output.into()
}
