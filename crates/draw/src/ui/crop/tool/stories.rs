//! Stories of the Crop tool run live over a landscape: the default frame and leaving the tool, the options bar's ratio
//! and size with the crop's target size, and the box's own Undo and Redo. The frame is the CropOverlay the tool shows.
//! The story plays the app: it stores the tool's settings, routes the bar to the tool and accepts every crop without
//! cropping the picture.

use super::super::{CropBar, CropDocKey, CropDocument, CropEvent, CropInput, CropSettings, CropTool};
use craft_behavior::CanvasView;
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{BezPath, Circle, Point, Rect, Size};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::{CropOverlay, RasterSource as _};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::runtime::frame;
use craft_ui::testing::canvas_story::{StoryDocument, StoryPicture, StoryShape, paint_shapes};
use craft_ui::testing::{Drive, StoryButton, StoryData, StoryOutput, ToolStory, story, story_tool, tool_story};
use egui::{Color32, Ui};

craft_ui::words! {
    /// The status lines of the Crop tool stories.
    #[non_exhaustive]
    pub(crate) struct CropStoryWords {
        /// The frame's size and turn.
        frame: template "cropStory.frame" = "{w} × {h} px · {angle}°";
        /// A status line: what the story shows, then the last event.
        status: template "cropStory.status" = "{shown}. {event}";
        /// No event has been handled yet.
        no_event: "cropStory.noEvent" = "No event yet";
        /// The crop the app was asked to make.
        apply: template "cropStory.apply" = "Apply {w} × {h} px at {angle}°";
        /// The crop the app was asked to make, resampled to the target size.
        apply_to: template "cropStory.applyTo" = "Apply {w} × {h} px at {angle}° to {target_w} × {target_h} px";
        /// The frame was discarded.
        discard: "cropStory.discard" = "Discard";
        /// A Straighten line turned the frame.
        straightened: template "cropStory.straightened" = "Straighten {angle}°";
        /// The story picked another tool.
        left: "cropStory.left" = "Left the tool";
        /// The size the crop resamples to.
        target: template "cropStory.target" = "Target {w} × {h} px";
        /// The size the crop resamples to, with the resolution typed.
        target_ppi: template "cropStory.targetPpi" = "Target {w} × {h} px at {ppi} ppi";
        /// The crop keeps the frame's pixels.
        free_crop: "cropStory.freeCrop" = "Free crop";
        /// The target, then the frame.
        ratio: template "cropStory.ratio" = "{target} · {frame}";
    }
}

/// The document the tool crops: the story document's size.
const SHOWN: StoryDocument = StoryDocument::STANDARD;
/// The selection of the Default frame story, in document units: the default frame is its bounds.
const SELECTION: Rect = Rect::new(160.0, 100.0, 480.0, 300.0);
/// The document's resolution in px/in.
const RESOLUTION: f64 = 300.0;

const PICK_OTHER: &str = "Pick another tool";
const PICK_CROP: &str = "Pick Crop tool";
const UNDO: &str = "Undo";
const REDO: &str = "Redo";

/// The picture: sky, a sun, the ground below a level horizon and a hill.
fn landscape() -> StoryPicture<impl Fn(Point) -> Color32> {
    let size = SHOWN.size();
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    let polygon = |points: &[Point]| {
        let mut path = BezPath::new();
        path.move_to(points[0]);
        for point in &points[1..] {
            path.line_to(*point);
        }
        path.close_path();
        path
    };
    let horizon = height * 0.6;
    let shapes = [
        StoryShape::filled(&Circle::new(Point::new(width * 0.75, height * 0.22), height * 0.1), Color32::from_rgb(250, 220, 110)),
        StoryShape::filled(
            &polygon(&[Point::new(0.0, horizon), Point::new(width, horizon), Point::new(width, height), Point::new(0.0, height)]),
            Color32::from_rgb(96, 150, 90),
        ),
        StoryShape::filled(
            &polygon(&[Point::new(width * 0.1, height), Point::new(width * 0.3, height * 0.62), Point::new(width * 0.55, height)]),
            Color32::from_rgb(70, 120, 70),
        ),
    ];
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(&shapes, Color32::from_rgb(150, 200, 235), p))
}

/// The document of key `key` the tool crops, at `resolution`.
fn document(key: u64, resolution: f64) -> CropDocument {
    let size = SHOWN.size();
    CropDocument::new(CropDocKey::new(key), Size::new(f64::from(size.width), f64::from(size.height)), resolution)
}

/// What the tool reported in a pass of the canvas body: it shows the frame.
fn show_frame(ui: &mut Ui, at: egui::Rect, tool: &mut CropTool, settings: &CropSettings, document: &CropDocument) -> Vec<CropEvent> {
    let ctx = ui.ctx().clone();
    let mut pass = tool.pass(CropInput::new(&ctx, settings, document));
    pass.show_frame(ui, at);
    pass.finish().events
}

/// Runs the tool on the canvas and returns what it reported.
fn run_tool(
    ctx: &egui::Context,
    host: &craft_ui::testing::StoryHost<'_>,
    tool: &mut CropTool,
    settings: &CropSettings,
    document: &CropDocument,
) -> Vec<CropEvent> {
    let mut pass = tool.pass(CropInput::new(ctx, settings, document));
    let _ = host.run(&mut pass);
    pass.finish().events
}

/// The app's part: stores the settings the tool changed and accepts every crop. Returns the event in story words,
/// for the status line.
fn handle(words: &CropStoryWords, units: &Units, tool: &mut CropTool, settings: &mut CropSettings, event: CropEvent) -> Option<String> {
    let n = |value: f64| units.number(value, 0..=1);
    match event {
        CropEvent::Settings(next) => {
            *settings = next;
            None
        }
        CropEvent::Apply(commit) => {
            tool.applied(true);
            let rect = commit.frame.rect;
            let (w, h, angle) = (n(rect.width()), n(rect.height()), n(commit.frame.angle));
            Some(match commit.target {
                Some(target) => craft_i18n::fmt(
                    words.apply_to.as_str(),
                    &[
                        ("w", w.as_str()),
                        ("h", h.as_str()),
                        ("angle", angle.as_str()),
                        ("target_w", n(target.width).as_str()),
                        ("target_h", n(target.height).as_str()),
                    ],
                ),
                None => craft_i18n::fmt(words.apply.as_str(), &[("w", w.as_str()), ("h", h.as_str()), ("angle", angle.as_str())]),
            })
        }
        CropEvent::Discard => Some(words.discard.as_str().to_owned()),
        CropEvent::Straightened(angle) => Some(craft_i18n::fmt(words.straightened.as_str(), &[("angle", n(angle).as_str())])),
        _ => None,
    }
}

/// The frame as the status line shows it: size and turn.
fn frame_line(words: &CropStoryWords, units: &Units, frame: Option<CropBox>) -> String {
    frame.map_or_else(String::new, |frame| {
        let n = |value: f64| units.number(value, 0..=1);
        craft_i18n::fmt(
            words.frame.as_str(),
            &[("w", n(frame.rect.width()).as_str()), ("h", n(frame.rect.height()).as_str()), ("angle", n(frame.angle).as_str())],
        )
    })
}

/// `shown`, then the last event the app handled.
fn status(words: &CropStoryWords, shown: &str, last: Option<&str>) -> String {
    craft_i18n::fmt(words.status.as_str(), &[("shown", shown), ("event", last.unwrap_or(words.no_event.as_str()))])
}

/// The tool picked: a frame is there at once, the selection's bounds or the page. A drag inside the untouched default
/// frame draws a new one, a drag inside a drawn frame moves it. "Pick another tool" drops an untouched default frame
/// and applies a drawn one; "Pick Crop tool" picks it again.
#[story(CropOverlay<'static>, covers = [], drive = Drive::Never)]
fn default_frame(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (words, units) = (env.words::<CropStoryWords>(), env.units());
    let selection = *data.value("selection", true);
    let document = if selection { document(2, RESOLUTION).selection(Some(SELECTION)) } else { document(1, RESOLUTION) };
    let mut picked = *data.note("picked", true);
    let mut last = data.note::<Option<String>>("last", None).clone();
    let picture = landscape();
    let buttons = [StoryButton::new(if picked { PICK_OTHER } else { PICK_CROP })];
    let showing = picked;
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, tool: &mut CropTool, settings: &CropSettings| {
        if showing { show_frame(ui, at, tool, settings, &document) } else { Vec::new() }
    };
    let ctx = ui.ctx().clone();
    let output = tool_story(ui, data, ToolStory::new("Document", &picture).buttons(&buttons), CropTool::new, CropSettings::default(), body, |run| {
        let mut events = run.shown.unwrap_or_default();
        if showing {
            events.extend(run_tool(&ctx, &run.host, run.tool, run.document, &document));
        }
        match run.pressed {
            Some(PICK_OTHER) => {
                events.extend(run.tool.leave(run.document, &document, &units, false));
                picked = false;
                last = Some(words.left.as_str().to_owned());
            }
            Some(PICK_CROP) => picked = true,
            _ => {}
        }
        for event in events {
            if let Some(text) = handle(&words, &units, run.tool, run.document, event) {
                last = Some(text);
            }
        }
        let frame = if picked { frame_line(&words, &units, run.tool.frame()) } else { String::new() };
        let mut output = StoryOutput::default().readout(status(&words, &frame, last.as_deref()));
        output.clicked = run.pressed.is_some();
        output
    });
    *data.note("picked", true) = picked;
    *data.note::<Option<String>>("last", None) = last;
    output
}

/// The options bar over the tool: a ratio preset or typed sides holds every gesture to the ratio and refits the frame;
/// W x H x Resolution sets the size the crop resamples to, shown as the target. ✓ applies the frame, ⊘ discards it.
#[story(CropOverlay<'static>, covers = [], drive = Drive::Never)]
fn ratio_and_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (words, units) = (env.words::<CropStoryWords>(), env.units());
    let resolution = *data.ranged("resolution", RESOLUTION, 72.0..=600.0);
    let document = document(1, resolution);
    let mut last = data.note::<Option<String>>("last", None).clone();
    // The bar edits the settings the tool runs with, which are the story's document.
    let mut settings = data.value("document", CropSettings::default()).clone();
    let mut bar = None;
    let shown = Bar::new("Options", BarKind::Options)
        .commit_cancel("Commit", "Cancel")
        .show(ui, |ui, _| bar = Some(CropBar::new(&mut settings, &document).show(ui)));
    *data.value("document", CropSettings::default()) = settings.clone();
    let (committed, cancelled) = (shown.committed, shown.cancelled);
    let (ratio, straighten) = bar.map_or((None, false), |bar| (bar.ratio, bar.straighten));
    let picture = landscape();
    let body =
        |ui: &mut Ui, at: egui::Rect, _: &CanvasView, tool: &mut CropTool, settings: &CropSettings| show_frame(ui, at, tool, settings, &document);
    let ctx = ui.ctx().clone();
    let output = tool_story(ui, data, ToolStory::new("Document", &picture), CropTool::new, settings, body, |run| {
        let mut events = run.shown.unwrap_or_default();
        if let Some(change) = ratio {
            events.extend(run.tool.refit(run.document, &document, &units, change.editing));
        }
        if straighten {
            run.tool.toggle_straighten();
        }
        if committed {
            events.extend(run.tool.apply(run.document, &document, &units));
        }
        if cancelled {
            events.extend(run.tool.discard());
        }
        if ratio.is_some() || committed || cancelled {
            // The bar showed before the frame: the frame shows its answer on the next pass.
            frame::request_repaint(&ctx);
        }
        events.extend(run_tool(&ctx, &run.host, run.tool, run.document, &document));
        for event in events {
            if let Some(text) = handle(&words, &units, run.tool, run.document, event) {
                last = Some(text);
            }
        }
        let n = |value: f64| units.number(value, 0..=1);
        let target = match run.document.target(&document, &units) {
            Some(target) => match target.resolution {
                Some(ppi) => craft_i18n::fmt(
                    words.target_ppi.as_str(),
                    &[("w", n(target.width).as_str()), ("h", n(target.height).as_str()), ("ppi", units.number(ppi, 0..=2).as_str())],
                ),
                None => craft_i18n::fmt(words.target.as_str(), &[("w", n(target.width).as_str()), ("h", n(target.height).as_str())]),
            },
            None => words.free_crop.as_str().to_owned(),
        };
        let frame = frame_line(&words, &units, run.tool.frame());
        let shown = craft_i18n::fmt(words.ratio.as_str(), &[("target", target.as_str()), ("frame", frame.as_str())]);
        StoryOutput::default().readout(status(&words, &shown, last.as_deref()))
    });
    *data.note::<Option<String>>("last", None) = last;
    output
}

/// Edit › Undo and Redo, as the app routes them to the tool while the box owns Undo: each draw, move, resize, turn or
/// key step of the frame is one box step. The buttons are disabled when there is no step to take.
#[story(CropOverlay<'static>, covers = [], drive = Drive::Never)]
fn box_undo(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (words, units) = (env.words::<CropStoryWords>(), env.units());
    let document = document(1, RESOLUTION);
    let steps = |tool: &CropTool| {
        let session = tool.session();
        (session.owns_undo() && session.can_undo(), session.owns_undo() && session.can_redo())
    };
    let declared = steps(&story_tool(data, CropTool::new).borrow());
    let buttons = [StoryButton::new(UNDO).enabled(declared.0), StoryButton::new(REDO).enabled(declared.1)];
    let picture = landscape();
    let body =
        |ui: &mut Ui, at: egui::Rect, _: &CanvasView, tool: &mut CropTool, settings: &CropSettings| show_frame(ui, at, tool, settings, &document);
    let ctx = ui.ctx().clone();
    tool_story(ui, data, ToolStory::new("Document", &picture).buttons(&buttons), CropTool::new, CropSettings::default(), body, |run| {
        let mut events = run.shown.unwrap_or_default();
        match run.pressed {
            Some(UNDO) => {
                let _ = run.tool.undo();
            }
            Some(REDO) => {
                let _ = run.tool.redo();
            }
            _ => {}
        }
        events.extend(run_tool(&ctx, &run.host, run.tool, run.document, &document));
        for event in events {
            let _ = handle(&words, &units, run.tool, run.document, event);
        }
        if steps(run.tool) != declared {
            // The buttons showed before this pass's step: they show what Undo and Redo can do on the next pass.
            frame::request_repaint(&ctx);
        }
        let mut output = StoryOutput::default().readout(frame_line(&words, &units, run.tool.frame()));
        output.clicked = run.pressed.is_some();
        output
    })
}
