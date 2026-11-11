//! The timeline widgets, for egui. The look follows Premiere Pro (FilmCraft); After Effects' (EffectCraft) and Pro
//! Tools' (SoundCraft) differences are options.
//!
//! - [`view`] has [`TimelineView`], the container that owns the time axis and publishes it as [`TimeAxis`] to the
//!   ruler, the tracks, the marker strip and the graph editor.
//! - [`panel`] has the [`Timeline`] panel over a [`TimelineModel`](crate::TimelineModel): track headers, rows, clips,
//!   the ruler and the playhead; edits out as [`TimelineEdit`]s. [`stack`] has its [`TrackStack`].
//! - [`ruler`] has [`TimelineRuler`] (ticks, timecode labels, scrubbing, work area, cache bars).
//! - [`clips`] has the clips of one track with trims and moves.
//! - [`keyframes`] has [`KeyframeLane`], [`KeyframeNavigator`] and [`AnimationToggle`].
//! - [`graph`] has the [`GraphEditor`] (value and speed graphs).
//! - [`transport`] has [`TransportBar`] (a monitor's buttons) and [`MediaControls`] (a compact play / seek bar).
//! - [`timecode`] has [`TimecodeField`] and the [`TimecodePanel`].
//! - [`preview`] has [`FramePreview`], a frame fitted in a panel with a status line.
//! - [`markers`] has [`MarkerStrip`] and the [`MarkerDialog`].
//! - [`overview`] has [`TimelineOverview`], the whole timeline in miniature.
//! - [`render_queue`] has [`RenderQueueList`].
//!
//! The label colours are an app value, [`LabelPalette`]; a component reads them with [`label_palette`].

pub mod clips;
pub mod graph;
pub mod keyframes;
pub mod markers;
pub mod overview;
pub mod panel;
pub mod preview;
pub mod render_queue;
pub mod ruler;
pub mod stack;
pub mod timecode;
pub mod transport;
pub mod view;

pub use clips::{ClipContent, ClipLane, ClipLaneEvents, ClipLook, ThumbnailMode, TimelineTool, thumbnail_tiles};
pub use graph::{
    GraphAction, GraphCurve, GraphEditor, GraphEditorOutput, GraphKey, GraphKeyMove, GraphMode, GraphRange, HandleMove, HandleSide, auto_range,
    nice_step,
};
pub use keyframes::{
    AnimationToggle, KeyframeId, KeyframeInterpolation, KeyframeLane, KeyframeLaneOutput, KeyframeMark, KeyframeNavigator, KeyframeNavigatorOutput,
    KeyframeWords,
};
pub use markers::{
    MARKER_POINT, MarkerDialog, MarkerDialogAction, MarkerDialogResult, MarkerDraft, MarkerEdit, MarkerSpan, MarkerStrip, MarkerStripOutput,
    MarkerWords,
};
pub use overview::{OverviewRun, OverviewRuns, TimelineOverview, TimelineOverviewOutput, overview_runs};
pub use panel::{Timeline, TimelineEdit, TimelineOutput, TimelineToolbar, TrackSwitch};
pub use preview::{FrameFit, FramePreview, frame_rect};
pub use render_queue::{
    RenderItemId, RenderQueueEvent, RenderQueueItem, RenderQueueList, RenderQueueListOutput, RenderQueueWords, RenderStatus, format_eta,
};
pub use ruler::{RenderBarState, RenderSpan, SplitMarks, TimeTick, TimelineRuler, TimelineRulerResponse, ruler_ladder, ruler_ticks};
pub use stack::{TrackRow, TrackStack};
pub use timecode::{
    TimecodeCodec, TimecodeField, TimecodeFieldOutput, TimecodePanel, TimecodePanelOutput, TimecodePanelSettings, TimecodeRow, TimecodeRowKind,
    TimecodeSource, TimecodeText, TimecodeWords,
};
pub use transport::{MediaControls, MediaControlsOutput, TransportBar, TransportBarOutput, TransportBarSettings, TransportButton, TransportWords};
pub use view::{TimeAxis, TimelineView, TimelineWords, time_axis};

use craft_ui::runtime::env::{EnvValue, EnvView};
use craft_ui::style::DataColor;

/// One step of a drag a component reports without a value of its own to edit: a gesture's begin, its proposals, its
/// end, or a value that arrives in one step.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DragEvent<T> {
    /// The gesture began; the value is its first proposal.
    Begin(T),
    /// A proposed value of the running gesture.
    Update(T),
    /// The gesture ended with this value.
    Commit(T),
    /// The gesture ended without a commit.
    Cancel,
    /// A value that arrived as one step.
    Set(T),
}

impl<T> DragEvent<T> {
    /// The value the event carries.
    pub fn value(&self) -> Option<&T> {
        match self {
            DragEvent::Begin(value) | DragEvent::Update(value) | DragEvent::Commit(value) | DragEvent::Set(value) => Some(value),
            DragEvent::Cancel => None,
        }
    }
}

/// The colours of the sixteen labels ([`MarkerColor`](crate::MarkerColor) indices), the app's Label preferences. The
/// root value is Premiere's (FilmCraft `crates/project/src/label.rs:64`); an app sets its own through its app values.
#[derive(Clone, Debug, PartialEq, EnvValue)]
#[non_exhaustive]
pub struct LabelPalette(pub [egui::Color32; 16]);

impl Default for LabelPalette {
    fn default() -> Self {
        let rgb = [
            [0x38, 0x0e, 0xa7],
            [0x1d, 0x4a, 0x64],
            [0x35, 0x54, 0x18],
            [0x6b, 0x1c, 0x82],
            [0x23, 0x53, 0x5a],
            [0x40, 0x4a, 0x11],
            [0x80, 0x18, 0x36],
            [0x80, 0x3f, 0x17],
            [0x59, 0x0d, 0xb0],
            [0x19, 0x2d, 0x94],
            [0x1f, 0x4d, 0x45],
            [0x79, 0x1c, 0x56],
            [0x6c, 0x5b, 0x47],
            [0x29, 0x5c, 0x2d],
            [0x58, 0x3d, 0x14],
            [0x6e, 0x66, 0x28],
        ];
        Self(rgb.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)))
    }
}

impl LabelPalette {
    /// A palette of sixteen label colours, in label order.
    pub fn new(colors: [egui::Color32; 16]) -> Self {
        Self(colors)
    }

    /// The colour of `label`; out-of-range indices wrap.
    pub fn color(&self, label: crate::MarkerColor) -> DataColor {
        DataColor::new(self.0[usize::from(label.0) % self.0.len()])
    }
}

/// The label palette of the app values `env` carries.
pub fn label_palette(env: &EnvView<'_>) -> LabelPalette {
    env.get::<LabelPalette>()
}
