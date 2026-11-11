//! Clip edges, trims and snapping: which edge a press grabs, which trim it starts, and the edit
//! a drag reports. The edit rules (ripple, linked partners, overwrite) stay the app's: the library
//! reports the gesture, the app applies it.

use std::cmp::Ordering;

use crate::model::ClipId;
use crate::time::{Tick, TimeRange, TimeView};

/// The hit sizes of clip edges, in points: the widgets pass the edge and roll zones of their look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeZones {
    /// The widest an edge zone gets on each side of a clip edge; at most a third of the clip's
    /// width (FilmCraft `timeline_hit.rs:21` `EDGE_PX`).
    pub edge: f32,
    /// Within this distance of a cut with a clip on its other side, the Selection tool rolls
    /// instead of rippling when its no-modifier trims are on (FilmCraft `timeline_hit.rs:18`
    /// `ROLL_PX`).
    pub roll: f32,
}

/// A clip's start (In) or end (Out).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ClipEdge {
    /// The clip's start edge.
    In,
    /// The clip's end edge.
    Out,
}

/// What a drag at a clip edge or body does (Premiere's trim tools).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrimKind {
    /// The edge moves, opening or covering a gap.
    Trim,
    /// The edge moves and later clips follow.
    Ripple,
    /// The cut between two touching clips moves.
    Roll,
    /// The clip's media moves inside it; its place stays.
    Slip,
    /// The clip moves; its neighbours' touching edges follow.
    Slide,
    /// The clip's speed changes to fill its new length (Rate Stretch).
    Stretch,
}

/// The clip edge a pointer grabs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeHit {
    /// The clip whose edge is hit.
    pub clip: ClipId,
    /// Which edge of `clip` is hit.
    pub edge: ClipEdge,
    /// The pointer is inside the clip (not in the zone past its edge).
    pub inside: bool,
    /// Points from the edge.
    pub distance: f32,
}

/// The edge zone of a clip `width` points wide: `zones.edge`, at most a third of the width;
/// `None` for a clip without a finite positive width.
// filmcraft: crates/ui-egui/src/panels/timeline_hit.rs:195 `zone`.
pub fn edge_zone(width: f32, zones: &EdgeZones) -> Option<f32> {
    (width.is_finite() && width > 0.0).then(|| zones.edge.min(width / 3.0))
}

/// A clip edge within reach of the pointer.
struct Reach {
    clip: ClipId,
    edge: ClipEdge,
    inside: bool,
    distance: f32,
}

impl Reach {
    /// Inside beats outside, then the nearer edge; an exact tie (right on a cut) goes to the
    /// later clip's In edge.
    fn beats(&self, other: &Reach) -> bool {
        if self.inside != other.inside {
            return self.inside;
        }
        match self.distance.partial_cmp(&other.distance) {
            Some(Ordering::Less) => true,
            Some(Ordering::Greater) => false,
            _ => self.edge == ClipEdge::In && other.edge != ClipEdge::In,
        }
    }
}

/// The edge of `clips` (one track's, in any order) a pointer at `x` grabs. Of several in reach,
/// the one the pointer is inside of beats one it is outside of, then the nearer (FilmCraft
/// #259: two touching clips share the cut, and the press goes to the clip it is over).
// filmcraft: crates/ui-egui/src/panels/timeline_hit.rs:200 `edge_at`, `:167` `Reach::beats`.
pub fn edge_at(clips: &[(ClipId, TimeRange)], view: &TimeView, left: f32, x: f32, zones: &EdgeZones) -> Option<EdgeHit> {
    let mut best: Option<Reach> = None;
    for (clip, range) in clips {
        let (x0, x1) = (view.x_of(range.start, left), view.x_of(range.end(), left));
        let Some(zone) = edge_zone(x1 - x0, zones) else { continue };
        for (edge, into) in [(ClipEdge::In, x - x0), (ClipEdge::Out, x1 - x)] {
            if !(-zone..=zone).contains(&into) {
                continue;
            }
            let reach = Reach { clip: *clip, edge, inside: into >= 0.0, distance: into.abs() };
            if best.as_ref().is_none_or(|b| reach.beats(b)) {
                best = Some(reach);
            }
        }
    }
    best.map(|r| EdgeHit { clip: r.clip, edge: r.edge, inside: r.inside, distance: r.distance })
}

/// The trim the Selection tool starts at an edge: Cmd+Shift rolls, Cmd ripples, else a regular
/// trim. With `no_modifier` (Premiere's Trim setting "Allow Selection tool to choose Roll and
/// Ripple trims without modifier key") no modifier is needed: on the cut (within
/// `zones.roll`, a clip on the other side) it rolls, elsewhere on the edge it ripples.
// filmcraft: crates/ui-egui/src/panels/timeline_hit.rs:26 `selection_trim_kind`.
pub fn selection_trim(command: bool, shift: bool, no_modifier: bool, distance: f32, has_neighbour: bool, zones: &EdgeZones) -> TrimKind {
    if command && shift {
        TrimKind::Roll
    } else if command {
        TrimKind::Ripple
    } else if no_modifier {
        if has_neighbour && distance <= zones.roll { TrimKind::Roll } else { TrimKind::Ripple }
    } else {
        TrimKind::Trim
    }
}

/// An edit a clip drag reports, as the value of an `EditEvent` gesture: the app previews each
/// update and applies the commit (one undo step).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ClipEdit {
    /// Clips moved by `delta` and `track_delta` tracks. `new_track`: dropped in the empty space
    /// past the last track of their kind (FilmCraft #1104), the app adds a track there. `copy`:
    /// Alt was held on the last drag frame, the drop copies. A `ClipLane`
    /// knows only its own row, so it reports `track_delta: 0` and `new_track: false`; the panel
    /// that holds the rows works both out from the pointer and sets them before the edit reaches
    /// the app.
    Move {
        /// The clips that move together.
        clips: Vec<ClipId>,
        /// The shift in time; negative moves earlier.
        delta: Tick,
        /// The shift in tracks; negative moves up.
        track_delta: i32,
        /// Dropped past the last track of its kind.
        new_track: bool,
        /// The drop copies the clips instead of moving them.
        copy: bool,
    },
    /// An edge moved by `delta` with `kind` ([`TrimKind::Trim`], `Ripple` or `Stretch`).
    /// `linked`: linked partners trim along (false when Alt was held as the drag started).
    Trim {
        /// The clip whose edge moves.
        clip: ClipId,
        /// The edge that moves.
        edge: ClipEdge,
        /// The kind of trim.
        kind: TrimKind,
        /// The edge's shift in time.
        delta: Tick,
        /// Linked partners trim along.
        linked: bool,
    },
    /// The cut between `left` and `right` moved by `delta`.
    Roll {
        /// The clip before the cut.
        left: ClipId,
        /// The clip after the cut.
        right: ClipId,
        /// The cut's shift in time.
        delta: Tick,
    },
    /// The clip's media moved by `delta` inside it.
    Slip {
        /// The clip whose media moves.
        clip: ClipId,
        /// The media's shift in time inside the clip.
        delta: Tick,
    },
    /// The clip moved by `delta`, its neighbours' edges with it.
    Slide {
        /// The clip that moves.
        clip: ClipId,
        /// The clip's shift in time.
        delta: Tick,
    },
    /// The Razor tool cut `clip` at `at` (snapped to a frame).
    Cut {
        /// The clip that is cut.
        clip: ClipId,
        /// The cut time.
        at: Tick,
    },
    /// Box selection: the clips the marquee touches; `add` keeps the selection from before
    /// (Shift or Cmd).
    Select {
        /// The clips the marquee touches.
        clips: Vec<ClipId>,
        /// Keep the earlier selection.
        add: bool,
    },
}

/// Times a drag snaps to: clip starts and ends (but the dragged clips'), the playhead (but not
/// for the playhead's own drag), markers, In and Out, work-area ends, keyframes. The app fills
/// it per gesture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapTargets {
    times: Vec<Tick>,
}

impl SnapTargets {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one snap time.
    pub fn push(&mut self, t: Tick) {
        self.times.push(t);
    }

    /// Both ends of each range.
    pub fn ranges(&mut self, ranges: impl IntoIterator<Item = TimeRange>) {
        for r in ranges {
            self.times.push(r.start);
            self.times.push(r.end());
        }
    }

    /// The snap times in the order they were added.
    pub fn times(&self) -> &[Tick] {
        &self.times
    }
}

impl Extend<Tick> for SnapTargets {
    fn extend<I: IntoIterator<Item = Tick>>(&mut self, iter: I) {
        self.times.extend(iter);
    }
}

/// `t` snapped to the nearest target within `tolerance` points at this zoom (the
/// `timeline.snap_distance` token), and the target it
/// snapped to (the timeline draws the snap line there); `t` and `None` when none is in reach.
// filmcraft: crates/ui-egui/src/panels/timeline/snap.rs:15 `snap_to` (distance in screen
// points, nearest wins).
pub fn snap_time(targets: &SnapTargets, t: Tick, view: &TimeView, tolerance: f32) -> (Tick, Option<Tick>) {
    let x = view.x_of(t, 0.0);
    let mut best: Option<(f32, Tick)> = None;
    for &target in targets.times() {
        let distance = (view.x_of(target, 0.0) - x).abs();
        if distance < tolerance && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, target));
        }
    }
    best.map_or((t, None), |(_, target)| (target, Some(target)))
}
