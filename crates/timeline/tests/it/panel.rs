//! The track stack: resizing rows.

use std::cell::RefCell;
use std::rc::Rc;

use craft_timeline::ui::{Timeline, TimelineEdit};
use craft_timeline::{ClipId, ClipInfo, TimeRange, TimelineModel, TrackId, TrackInfo};
use craft_timeline::{FrameRate, Tick, TimeFormat, TimeView, TrackKind};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Rect, Vec2};

const RATE: FrameRate = FrameRate::FPS_29_97;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

// The Timeline panel over a model of tracks, one clip in V1 and one in A1.

const V1: TrackId = TrackId(1);
const V2: TrackId = TrackId(2);
const A1: TrackId = TrackId(3);
/// The panel's room.
const PANEL_ROOM: Vec2 = Vec2::new(900.0, 500.0);
/// The time the panel's view shows per second of room.
const PANEL_POINTS_PER_SECOND: f64 = 20.0;

struct Sequence {
    tracks: Vec<(TrackId, TrackKind, String)>,
    master: Option<f32>,
    view: TimeView,
    playhead: Tick,
    selection: Vec<ClipId>,
    /// The clips that are switched off.
    disabled: Vec<ClipId>,
    edits: Vec<TimelineEdit>,
}

impl TimelineModel for Sequence {
    fn duration(&self) -> Tick {
        secs(30.0)
    }

    fn format(&self) -> TimeFormat {
        TimeFormat::timecode(RATE)
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
            .map(|(id, kind, name)| TrackInfo {
                id: *id,
                kind: *kind,
                name,
                height: None,
                depth: 0,
                label: None,
                locked: Some(false),
                targeted: Some(true),
                sync_lock: None,
                enabled: Some(true),
                muted: (*kind == TrackKind::Audio).then_some(false),
                solo: (*kind == TrackKind::Audio).then_some(false),
            })
            .collect()
    }

    fn clips(&self, track: TrackId, _: TimeRange) -> Vec<ClipInfo<'_>> {
        let clip = |id: u64| ClipInfo {
            id: ClipId(id),
            track,
            range: TimeRange::from_bounds(secs(2.0), secs(6.0)),
            name: "Clip",
            label: None,
            enabled: !self.disabled.contains(&ClipId(id)),
            source: None,
        };
        match track {
            V1 => vec![clip(1)],
            A1 => vec![clip(2)],
            _ => Vec::new(),
        }
    }

    fn selection(&self) -> &[ClipId] {
        &self.selection
    }

    fn master_volume_db(&self) -> Option<f32> {
        self.master
    }
}

struct Panel {
    fixture: Fixture,
    sequence: Rc<RefCell<Sequence>>,
}

fn panel_with(tracks: Vec<(TrackId, TrackKind, String)>, master: Option<f32>) -> Panel {
    let sequence = Rc::new(RefCell::new(Sequence {
        tracks,
        master,
        view: TimeView::new(Tick::ZERO, PANEL_POINTS_PER_SECOND),
        playhead: Tick::ZERO,
        selection: Vec::new(),
        disabled: Vec::new(),
        edits: Vec::new(),
    }));
    let shown = sequence.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut sequence = shown.borrow_mut();
        let out = Timeline::new("Timeline", &mut *sequence).show(ui);
        sequence.edits.extend(out.edits);
    });
    fixture.room(PANEL_ROOM);
    fixture.settle();
    Panel { fixture, sequence }
}

fn named(tracks: &[(TrackId, TrackKind, &str)]) -> Vec<(TrackId, TrackKind, String)> {
    tracks.iter().map(|(id, kind, name)| (*id, *kind, (*name).to_owned())).collect()
}

/// V2 and V1 over A1.
fn panel() -> Panel {
    panel_with(named(&[(V2, TrackKind::Video, "V2"), (V1, TrackKind::Video, "V1"), (A1, TrackKind::Audio, "A1")]), None)
}

impl Panel {
    /// The row of the track called `name`: of the nodes with that name, the widest (the row spans the header and the lane).
    fn row(&self, name: &str) -> Rect {
        let tree = self.fixture.tree();
        tree.keys()
            .into_iter()
            .filter(|key| tree.name_of(*key) == Some(name))
            .filter_map(|key| self.fixture.target(key))
            .map(|target| target.rect)
            .max_by(|a, b| a.width().total_cmp(&b.width()))
            .unwrap_or_else(|| panic!("no row named {name}"))
    }

    /// The node whose name contains `name` and which lies inside `row`.
    fn in_row(&self, name: &str, row: Rect) -> Target {
        let tree = self.fixture.tree();
        tree.keys()
            .into_iter()
            .filter(|key| tree.name_of(*key).is_some_and(|label| label.contains(name)))
            .filter_map(|key| self.fixture.target(key))
            .find(|target| row.contains(target.center()))
            .unwrap_or_else(|| panic!("no node named {name} in {row:?}"))
    }

    fn edits(&self) -> Vec<TimelineEdit> {
        self.sequence.borrow().edits.clone()
    }

    fn heights(&self) -> Vec<(TrackId, f32)> {
        self.edits()
            .into_iter()
            .filter_map(|edit| match edit {
                TimelineEdit::TrackHeight { track, height, .. } => height.value().map(|height| (track, *height)),
                _ => None,
            })
            .collect()
    }
}

/// A row resizes from its own height: video rows grow upward, so an edge dragged up adds height, and the top video row
/// has a top edge.
#[trial]
fn rows_resize_from_their_height_and_video_rows_grow_upward() {
    let mut panel = panel();
    let a1 = panel.row("A1");
    let edge = panel.in_row("Drag to change the height of A1", a1);
    panel.fixture.drag(&edge, Vec2::new(0.0, RESIZE_TRAVEL));
    let got = panel.heights();
    assert_eq!(got.last().map(|(track, _)| *track), Some(A1), "{got:?}");
    assert!((got.last().map_or(0.0, |(_, height)| *height) - (a1.height() + RESIZE_TRAVEL)).abs() < SLACK, "{got:?} from {}", a1.height());

    let mut panel = self::panel();
    let v1 = panel.row("V1");
    let edge = panel.in_row("Drag to change the height of V1", v1);
    panel.fixture.drag(&edge, Vec2::new(0.0, -RESIZE_TRAVEL));
    let got = panel.heights();
    assert!((got.last().map_or(0.0, |(_, height)| *height) - (v1.height() + RESIZE_TRAVEL)).abs() < SLACK, "{got:?} from {}", v1.height());

    let mut panel = self::panel();
    let v2 = panel.row("V2");
    let top = panel.in_row("Drag the top edge to change the height of V2", v2);
    panel.fixture.drag(&top, Vec2::new(0.0, -RESIZE_TRAVEL));
    let got = panel.heights();
    assert_eq!(got.last().map(|(track, _)| *track), Some(V2), "{got:?}");
    assert!((got.last().map_or(0.0, |(_, height)| *height) - (v2.height() + RESIZE_TRAVEL)).abs() < SLACK, "{got:?} from {}", v2.height());
}

/// The distance a height edge is dragged in the resize trial.
const RESIZE_TRAVEL: f32 = 10.0;

/// The rounding slack of a measured point.
const SLACK: f32 = 0.5;
