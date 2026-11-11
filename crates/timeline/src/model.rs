//! [`TimelineModel`]: what the [`Timeline`](crate::ui::Timeline) panel reads from the app's
//! sequence. The app implements it on its document view; the panel never owns document state and
//! reports every change as an edit.

use crate::markers::{Marker, MarkerColor};
use crate::time::{Tick, TimeRange, TimeView};
use crate::timecode::TimeFormat;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
/// Identifies a track within a sequence.
pub struct TrackId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
/// Identifies a clip within a sequence.
pub struct ClipId(pub u64);

/// What a track holds; the panel groups video tracks above the divider (stacked upward from it,
/// as Premiere does), audio below, and caption tracks in their own area above the video tracks
/// (FilmCraft `timeline.rs:293-295`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrackKind {
    #[default]
    /// A picture track.
    Video,
    /// A sound track.
    Audio,
    /// Captions, in their own area above the video tracks.
    Caption,
    /// An app row with its own content: an After Effects layer, a property row, an automation
    /// lane. Shown in the order the model gives.
    Other,
}

/// One track row as the panel shows it. The `Option<bool>` switches show in the header only
/// when `Some` (an EffectCraft layer has no sync lock).
#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo<'a> {
    /// The track's identity.
    pub id: TrackId,
    /// What the track holds.
    pub kind: TrackKind,
    /// "V1", "A2", a layer name.
    pub name: &'a str,
    /// Points, clamped to `style.track_height_min`…`style.track_height_max`; the user resizes it
    /// at the row's edge. `None` is the style's default for the kind.
    pub height: Option<f32>,
    /// Nesting under the track before it (EffectCraft property rows, SoundCraft folders): the
    /// header's name is indented by it.
    pub depth: u8,
    /// The track's colour label, if it has one.
    pub label: Option<MarkerColor>,
    /// Whether the track is locked against edits.
    pub locked: Option<bool>,
    /// Whether the track is targeted by paste and insert.
    pub targeted: Option<bool>,
    /// Whether the track moves with ripple edits on other tracks.
    pub sync_lock: Option<bool>,
    /// Track output (the eye on video, the speaker on audio).
    pub enabled: Option<bool>,
    /// Whether the track is muted (audio).
    pub muted: Option<bool>,
    /// Whether the track is soloed (audio).
    pub solo: Option<bool>,
}

/// One clip on a track, within the visible time.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo<'a> {
    /// The clip's identity.
    pub id: ClipId,
    /// The track that holds the clip.
    pub track: TrackId,
    /// The clip's place on the timeline.
    pub range: TimeRange,
    /// The clip's display name.
    pub name: &'a str,
    /// The clip's colour label, if it has one.
    pub label: Option<MarkerColor>,
    /// Disabled clips are drawn dimmed (Premiere's Enable off).
    pub enabled: bool,
    /// The whole media under the clip, on the timeline: a slip shows it as a ghost, and a trim
    /// stops at it.
    pub source: Option<TimeRange>,
}

/// The sequence the timeline panel shows.
///
/// ```ignore
/// impl TimelineModel for SequenceView<'_> {
///     fn duration(&self) -> Tick { self.seq.duration() }
///     fn format(&self) -> TimeFormat { TimeFormat::timecode(self.seq.rate).drop_frame(self.seq.drop_frame) }
///     fn playhead(&self) -> Tick { self.session.playhead() }
///     fn view(&mut self) -> &mut TimeView { &mut self.ui.views.entry(self.id).or_default().time }
///     fn tracks(&self) -> Vec<TrackInfo<'_>> { /* V3, V2, V1, A1, A2 */ }
///     fn clips(&self, track: TrackId, visible: TimeRange) -> Vec<ClipInfo<'_>> { /* items overlapping `visible` */ }
/// }
/// ```
pub trait TimelineModel {
    /// The end of the content.
    fn duration(&self) -> Tick;

    /// How times are shown and typed (rate, drop-frame, display).
    fn format(&self) -> TimeFormat;

    /// The playhead position.
    fn playhead(&self) -> Tick;

    /// The view (zoom and scroll) the panel shows and changes directly: view changes are not
    /// document edits and never undo steps.
    fn view(&mut self) -> &mut TimeView;

    /// Top to bottom.
    fn tracks(&self) -> Vec<TrackInfo<'_>>;

    /// The clips of `track` that overlap `visible`.
    fn clips(&self, track: TrackId, visible: TimeRange) -> Vec<ClipInfo<'_>>;

    /// The selected clips; the panel draws them selected and reports changes as
    /// `TimelineEdit::Select` (in `craft_timeline::ui`).
    fn selection(&self) -> &[ClipId];

    /// Sequence markers, in the ruler.
    fn markers(&self) -> &[Marker] {
        &[]
    }

    /// The edit points (cuts on targeted tracks) Up and Down move the playhead to, in time
    /// order; Premiere's Go to Previous / Next Edit Point.
    fn edit_points(&self) -> Vec<Tick> {
        Vec::new()
    }

    /// Mark In and Mark Out.
    fn in_out(&self) -> (Option<Tick>, Option<Tick>) {
        (None, None)
    }

    /// The work area (After Effects; Premiere's work area bar when shown).
    fn work_area(&self) -> Option<TimeRange> {
        None
    }

    /// The master volume in dB. With `Some`, the panel draws the Mix row at the bottom of the
    /// rows, naming it and showing the value (FilmCraft `timeline.rs:387-396`).
    fn master_volume_db(&self) -> Option<f32> {
        None
    }
}
