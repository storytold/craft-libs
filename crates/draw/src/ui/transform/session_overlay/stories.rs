//! Stories of the transform overlay: the open session's box, run live under its options bar.

use super::{TransformOverlay, TransformTool};
use crate::ui::transform::{TransformBar, TransformEdit, TransformPhase, TransformRequest, TransformSession, TransformSettings};
use craft_behavior::CanvasView;
use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::{Line, Rect as DocRect};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::pointer;
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, paint_shapes};
use craft_ui::testing::{EditKind, StoryData, StoryOutput, ToolStory, story, story_tool, tool_story};
use egui::{Color32, Ui};

craft_ui::words! {
    /// The status line of the transform session story.
    #[non_exhaustive]
    pub(crate) struct TransformStoryWords {
        /// The session's phase, then how the last session ended.
        status: template "transformStory.status" = "{phase} · {note}";
        /// The Box phase.
        box_phase: "transformStory.box" = "Box";
        /// The Warp phase.
        warp: "transformStory.warp" = "Warp";
        /// The last session applied its transform.
        applied: "transformStory.applied" = "Applied";
        /// The last session closed with nothing changed.
        closed: "transformStory.closed" = "Closed without changes";
        /// The last session was discarded.
        discarded: "transformStory.discarded" = "Discarded";
    }
}

/// The layer's source frame, in document units: the story shows the layer as a box in the accent with a diagonal.
const FRAME: DocRect = DocRect::new(200.0, 120.0, 440.0, 280.0);

/// How the last session ended.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ended {
    Applied,
    Closed,
    Discarded,
}

/// The open session, kept across passes, and how the last one ended.
struct TransformKept {
    session: TransformSession<()>,
    ended: Option<Ended>,
}

impl TransformKept {
    /// The first session, on the layer's own frame.
    fn new(settings: &TransformSettings) -> Self {
        TransformKept { session: open(BoxState::from_rect(FRAME), settings), ended: None }
    }
}

/// The persisted settings the story starts from: the defaults with Show Reference Point on, so the point shows.
fn sample_settings() -> TransformSettings {
    TransformSettings { show_reference_point: true, ..TransformSettings::default() }
}

/// A session on the layer, its box at `state`.
fn open(state: BoxState, settings: &TransformSettings) -> TransformSession<()> {
    TransformSession::with_state(FRAME, state).settings(settings)
}

/// Free Transform live over the layer, under its options bar: the box and the bar's fields edit the session; Enter, a
/// click outside or ✓ applies it, Esc or ⊘ discards it, and a new session opens on the applied box. The applied box is
/// the story's document.
#[story(TransformOverlay<'static, ()>, covers = [Default])]
fn transform_phase(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<TransformStoryWords>();
    let first = data.value("settings", sample_settings()).clone();
    let kept = story_tool(data, || TransformKept::new(&first));
    let settings = data.value("settings", sample_settings());
    // The bar shows before the canvas, as the app's options bar does: its edits reach the session on this pass.
    let bar = TransformBar::new("Free Transform", &kept.borrow().session, Bind::bound(settings)).show(ui);
    let settings = settings.clone();
    let document = StoryDocument::STANDARD;
    let line = accent(ui, 1.0);
    let layer = [
        StoryShape::filled(&FRAME, accent(ui, 0.5)).outlined(OUTLINE, line),
        StoryShape::line(&Line::new(FRAME.origin(), (FRAME.x1, FRAME.y1)), OUTLINE, line),
    ];
    let size = document.size();
    let picture = StoryPicture::new(size.width, size.height, move |p| paint_shapes(&layer, Color32::TRANSPARENT, p));
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, kept: &mut TransformKept, _: &BoxState| {
        TransformOverlay::new("Free Transform", &kept.session, &settings).show_at(ui, at)
    };
    let frame = pointer::frame(ui.ctx());
    let story = ToolStory::new("Document", &picture);
    tool_story(
        ui,
        data,
        story,
        || TransformKept::new(&settings),
        BoxState::from_rect(FRAME),
        body,
        |run| {
            let (applied, kept) = (run.document, run.tool);
            let mut tool = TransformTool::new(&kept.session);
            let _ = run.host.run(&mut tool);
            let (key_apply, key_discard) = tool.take_keys();
            let (edits, apply, discard) = match run.shown {
                Some(shown) => (shown.edits, shown.apply, shown.discard),
                None => (Vec::new(), false, false),
            };
            let edits: Vec<TransformEdit<()>> = edits.into_iter().chain(bar.edits).collect();
            let output = StoryOutput {
                edits: edits
                    .iter()
                    .filter_map(|edit| match edit {
                        TransformEdit::Box(event) => EditKind::of(event),
                        TransformEdit::Fields(event) => EditKind::of(event),
                        _ => None,
                    })
                    .collect(),
                clicked: bar.apply || bar.discard,
                ..StoryOutput::default()
            };
            let mut requests: Vec<TransformRequest<()>> = edits.into_iter().filter_map(|edit| kept.session.apply(edit, frame.time)).collect();
            if apply || bar.apply || key_apply {
                requests.push(kept.session.commit());
            } else if discard || bar.discard || key_discard {
                requests.push(kept.session.discard());
            }
            for request in requests {
                let ended = match request {
                    TransformRequest::Apply(result) => {
                        *applied = result.state;
                        Ended::Applied
                    }
                    TransformRequest::CloseUntouched => Ended::Closed,
                    TransformRequest::Discard => Ended::Discarded,
                    TransformRequest::NeedWarp { .. } => {
                        let _ = kept.session.apply(TransformEdit::Warp(()), frame.time);
                        continue;
                    }
                };
                kept.ended = Some(ended);
                kept.session = open(*applied, &settings);
            }
            kept.session.settle(frame.time, !frame.buttons.is_empty());
            let phase = match kept.session.phase() {
                TransformPhase::Warp => words.warp.as_str(),
                TransformPhase::Box => words.box_phase.as_str(),
            };
            let note = kept.ended.map(|ended| match ended {
                Ended::Applied => words.applied.as_str(),
                Ended::Closed => words.closed.as_str(),
                Ended::Discarded => words.discarded.as_str(),
            });
            let readout = note.map_or_else(|| phase.to_owned(), |note| craft_i18n::fmt(words.status.as_str(), &[("phase", phase), ("note", note)]));
            output.readout(readout)
        },
    )
}
