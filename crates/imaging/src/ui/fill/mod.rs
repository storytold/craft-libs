//! The Content-Aware Fill workspace (Edit › Content-Aware Fill…), in Photoshop's layout.
//!
//! [`ContentAwareFillWorkspace`] is a [`FullWindowDialog`] with the title strip "Content-Aware Fill
//! (layer, 66%)", the full-width options bar ([`FillBar`]), the tool strip (Sampling Brush, Lasso,
//! Hand, Zoom), the canvas with the sampling tint ([`FillOverlay`]) and the fill area's ants, and
//! two side columns from the end edge inwards: the Content-Aware Fill panel ([`FillPanel`]) with the
//! footer (Reset; OK, Apply, Cancel), and the Preview panel ([`FillPreviewPane`]).
//!
//! PhotoCraft measured the layout and behaviour on Photoshop 25.4 (`caf_ui.rs:1-25`). The session
//! is plain data ([`FillSession`]: the sampling option, the brush strokes, the lasso edits) with
//! [`EditHistory`] undo; the app's engine plans the fill (`content_aware_plan`), gives the
//! sampling and fill-area planes and the preview through [`FillSource`], and fills on Apply and
//! OK with [`FillCommand`]. Apply keeps the workspace open: the app fills, then calls
//! [`ContentAwareFillWorkspace::rebase`]. Cancel closes, keeping what Apply did.
//!
//! Files: `session`, `lasso` and `tool`; `overlay`, `panel`, `preview_pane`, `bar`, `commands`,
//! `words` and this file's bodies.
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`EditHistory`]: craft_behavior::EditHistory

mod bar;
mod commands;
mod lasso;
mod overlay;
mod panel;
mod preview_pane;
mod session;
pub(crate) mod stories;
mod tool;
mod words;

pub use bar::{FillBar, FillBarOutput};
pub use commands::FillCommands;
pub use lasso::FillAreaEdit;
pub use overlay::{FillOverlay, SamplingPlane, fill_area_edges};
pub use panel::{FillPanel, FillPanelOutput};
pub use preview_pane::{FillPreviewPane, PreviewFraming};
pub use session::{
    ColorAdaptation, FillCommand, FillLimit, FillLimits, FillOutputTo, FillPreviewRequest, FillReset, FillSession, FillSettings, FillStrokeEvent,
    RotationAdaptation, SamplingMode, SamplingStroke,
};
pub use tool::{FillCanvas, FillCanvasTool, FillTool, FillToolOptions};
pub use words::FillWords;

use std::cell::RefCell;
use std::sync::Arc;

use egui::{Color32, Context, Id, Response, Ui};

use craft_behavior::edit_history::Step;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, EditEvent, EditHistory, IconId, Mixed, PixelSize};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::canvas::{HandTool, Momentary, NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, ZoomTool, tool_pick};
use craft_ui::canvas_controls::brush::{BrushCursorOptions, TipFalloff};
use craft_ui::canvas_controls::{Marquee, MarqueeLook};
use craft_ui::contract::{Bind, BodyCx, NoValue, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::{DialogWords, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowTitle, FullWindowView, RasterOverlay, SideFooter};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::ParamDialogWords;
use craft_ui::primitives::{Checker, IconSpec};
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::pending::{Job, Pending};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};

use crate::ui::wide_angle::PreviewDone;

/// The key of the colour request for the sampling tint (`FillOutput::pick_color`,
/// [`ContentAwareFillWorkspace::set_color`]).
pub const OVERLAY_COLOR_KEY: &str = "fill.overlayColor";

/// The name the history gives its steps.
const HISTORY_NAME: &str = "Content-Aware Fill";

/// The footer's buttons in the order the workspace gives them, by the index the dialog reports a pick with.
const PICK_OK: usize = 0;
const PICK_APPLY: usize = 1;
const PICK_CANCEL: usize = 2;

/// What the app gives the workspace to show each pass.
pub trait FillSource {
    /// The document as the canvas shows it (the visible layers, so an Apply to a new layer shows).
    fn image(&self) -> &dyn RasterSource;

    /// The sampling area after the strokes so far (the stroke in progress included), for the tint.
    fn sampling(&self) -> Option<&SamplingPlane>;

    /// The fill area (the selection after the lasso edits), for the ants; never tinted.
    fn fill_area(&self) -> Option<&SamplingPlane>;

    /// The sampling window in document pixels: the Preview panel opens framed on it.
    fn sampling_window(&self) -> Option<Rect>;

    /// The last ready preview of the fill, for the Preview panel.
    fn preview(&self) -> Option<&dyn RasterSource>;

    /// The job that fills `request` for the preview. The workspace starts one when the request changes and no
    /// stroke, lasso or slider drag is under way, shows its progress and failure, and cancels it when a newer request
    /// replaces it. The picture the job makes is the app's: it shows through [`preview`](Self::preview).
    fn preview_job(&self, request: &FillPreviewRequest) -> Job<PreviewDone, String>;
}

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FillOpen {
    /// The layer's size in document pixels.
    pub size: [u32; 2],
    /// The layer's name: the title's subject.
    pub subject: String,
    /// The fill settings the workspace starts with.
    pub settings: FillSettings,
    /// The limits on strokes, edits and points.
    pub limits: FillLimits,
    /// The Sampling Brush's first size: a tenth of the sampling window's side.
    pub brush_size: f64,
}

impl FillOpen {
    /// The Sampling Brush's first size in document pixels when the app sets none.
    pub const BRUSH_SIZE: f64 = 20.0;

    /// Opens on a layer of `size` document pixels named `subject`; the settings start at their defaults.
    pub fn new(size: [u32; 2], subject: impl Into<String>, limits: FillLimits) -> Self {
        Self { size, subject: subject.into(), settings: FillSettings::default(), limits, brush_size: Self::BRUSH_SIZE }
    }

    /// Sets the starting fill settings.
    #[must_use]
    pub fn settings(mut self, settings: FillSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Sets the Sampling Brush's first size in document pixels.
    #[must_use]
    pub fn brush_size(mut self, brush_size: f64) -> Self {
        self.brush_size = brush_size;
        self
    }
}

/// What the workspace reports, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FillEvent {
    /// A Sampling Brush stroke: `Begin`, `Update` with each new point (the app paints its sampling
    /// plane), `Commit` (recorded), or `Cancel`.
    Stroke(FillStrokeEvent),
    /// A lasso polygon, Expand or Contract, recorded in the session: the app re-plans the fill
    /// area and the window. If the edit empties the fill area, the app calls
    /// [`ContentAwareFillWorkspace::refuse_fill_edit`].
    FillArea(FillAreaEdit),
    /// A Reset or a new sampling option changed the session: the app re-plans.
    Reset(FillReset),
    /// A stroke or edit the limits refused, with the limit (worded under the panel,
    /// [`FillLimit::message`]).
    Refused(FillLimit),
}

/// What [`ContentAwareFillWorkspace::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FillOutput {
    /// What happened this pass, in order.
    pub events: Vec<FillEvent>,
    /// OK (`Accept`) or Cancel this pass.
    pub closed: Option<DialogRole>,
    /// Apply was pressed (the workspace stays open): the app fills with [`ContentAwareFillWorkspace::command`],
    /// then calls [`rebase`](ContentAwareFillWorkspace::rebase).
    pub apply: bool,
    /// The dialog's response: its rect is the workspace's.
    pub response: Response,
    /// A fill setting changed this pass.
    pub settings_changed: bool,
    /// The Color swatch was pressed: the app opens its colour picker and answers with
    /// [`ContentAwareFillWorkspace::set_color`].
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for FillOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What shows under the panel: the limit a stroke or edit ran into, or the app's message (a failed
/// Apply, an edit that emptied the fill area).
#[derive(Clone, Debug, PartialEq)]
enum FillIssue {
    Limit(FillLimit),
    Message(String),
}

impl FillIssue {
    fn text(&self, env: &Env) -> String {
        match self {
            FillIssue::Limit(limit) => limit.message(env),
            FillIssue::Message(message) => message.clone(),
        }
    }
}

type Ants = (SamplingPlane, Arc<Vec<(Point, Point)>>);

/// The Content-Aware Fill workspace.
///
/// Keys ([`FillCommands`], on the body's scope, none while a text field edits): B, L, H, Z pick the tools; `[` and
/// `]` the brush size ([`FillToolOptions::step_size`]); ⌘H hides or shows the tint. The zoom chords are the
/// dialog's view commands. The app routes ⌘Z and ⇧⌘Z (`EditCommands::Undo`, `Redo`) to [`step`](Self::step); Enter
/// is OK and Esc Cancel (Esc during a stroke or lasso cancels that only). A tool switch or an undo during a stroke
/// cancels it first (`CancelReason::Removed`, `CancelReason::UndoDuringGesture`).
#[derive(Pattern)]
pub struct ContentAwareFillWorkspace {
    session: FillSession,
    history: EditHistory<FillSession>,
    settings: FillSettings,
    limits: FillLimits,
    subject: String,
    size: PixelSize,
    open: bool,
    // View state outside the history.
    view: FullWindowView,
    preview_view: FullWindowView,
    framing: PreviewFraming,
    tool: FillTool,
    last_used: LastUsed<FillTool>,
    options: FillToolOptions,
    canvas: FillCanvas,
    hand: HandTool,
    zoom: ZoomTool,
    momentary: Momentary,
    canvas_id: Option<Id>,
    /// The user's navigation options and transparency grid, the app's.
    navigation: NavigationOptions,
    checkerboard: Checker,
    cursor_options: BrushCursorOptions,
    /// ⌘H: the tint hidden without changing Show Sampling Area.
    tint_hidden: bool,
    /// The fill the preview job is for, and the number its key carries.
    job: Option<FillPreviewRequest>,
    job_id: u64,
    preview: Pending<PreviewDone, String>,
    error: Option<FillIssue>,
    /// A setting is being dragged (the Opacity slider): the preview waits for its end.
    settings_gesture: bool,
    /// [`set_color`](Self::set_color) changed the color since the last pass: the next
    /// [`FillOutput::settings_changed`] says so.
    color_owed: bool,
    /// The outline of the fill area's covered cells, kept for as long as the plane is the same.
    ants: Option<Ants>,
    /// Steps the app asked for while a stroke was open; the next `show` cancels the stroke, then steps.
    requested: Vec<Step>,
    /// A stroke, a lasso or a resize was open at the end of the last pass.
    busy: bool,
    /// This pass's events and colour request.
    events: Vec<FillEvent>,
    pick_color: Option<ColorRequest>,
}

impl ContentAwareFillWorkspace {
    /// A workspace over `open`; its first pass asks for a preview.
    pub fn new(open: FillOpen) -> Self {
        let session = FillSession { sampling: open.settings.sampling, ..FillSession::default() };
        let navigation = NavigationOptions::default();
        let [width, height] = open.size;
        Self {
            history: EditHistory::new(&session),
            options: FillToolOptions {
                brush_add: open.settings.sampling.brush_adds(),
                brush_size: open.brush_size,
                lasso: SelectionOperation::New,
                grow_by: FillToolOptions::GROW_BY,
            },
            session,
            settings: open.settings,
            limits: open.limits,
            subject: open.subject,
            size: PixelSize::new(width, height),
            open: true,
            view: FullWindowView::new(),
            preview_view: FullWindowView::new(),
            framing: PreviewFraming::default(),
            tool: FillTool::SamplingBrush,
            last_used: LastUsed::default(),
            canvas: FillCanvas::new(),
            hand: HandTool::new(navigation),
            zoom: ZoomTool::new(navigation),
            momentary: Momentary::new(navigation),
            canvas_id: None,
            navigation,
            checkerboard: Checker::default(),
            cursor_options: tool::default_cursor_options(),
            tint_hidden: false,
            job: None,
            job_id: 0,
            preview: Pending::default(),
            error: None,
            settings_gesture: false,
            color_owed: false,
            ants: None,
            requested: Vec::new(),
            busy: false,
            events: Vec::new(),
            pick_color: None,
        }
    }

    /// The layer's size in document pixels.
    pub fn size(&self) -> PixelSize {
        self.size
    }

    /// The user's navigation options (wheel, flick, animated zoom) for the canvas and the Preview
    /// panel; Photoshop's by default.
    pub fn navigation(&mut self, navigation: NavigationOptions) {
        self.navigation = navigation;
        self.hand.options = navigation;
        self.zoom.options = navigation;
        self.momentary.set_options(navigation);
    }

    /// The user's transparency grid under the image and the preview.
    pub fn checkerboard(&mut self, checkerboard: Checker) {
        self.checkerboard = checkerboard;
    }

    /// The user's cursor preferences (the Sampling Brush's outline).
    pub fn cursor_options(&mut self, cursor_options: BrushCursorOptions) {
        self.cursor_options = cursor_options;
    }

    /// Shows the workspace for this pass: the title, the keys, [`FillBar`], the strip, the two
    /// side columns ([`FillPanel`] with OK, Apply, Cancel at the footer and Reset at its leading end;
    /// then [`FillPreviewPane`]), and the canvas with the source's image, [`FillOverlay`] and
    /// [`FillCanvas`]'s tool (or Hand, Zoom). Commits go into the session within the limits; a
    /// sampling option or Reset from the panel goes through [`FillSession::set_sampling`] and
    /// [`FillSession::reset`] (Custom makes the brush add, the others subtract). Then a change of the
    /// fill, once no stroke, lasso or slider drag is under way, starts a preview job through
    /// [`FillSource::preview_job`]. An Apply or OK the app reports failing ([`Self::apply_failed`])
    /// keeps the workspace open.
    pub fn show(&mut self, ui: &mut Ui, source: &dyn FillSource) -> FillOutput {
        // pc: crates/ui-egui/src/dialogs/caf_ui/view.rs:265 `show`; caf_ui/controls.rs:151 `keys`.
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let dialog = env.words::<DialogWords>();
        let params = env.words::<ParamDialogWords>();
        let title =
            FullWindowTitle::new(env.words::<FillWords>().name.clone()).subject(self.subject.clone()).zoom(self.view.zoom_percent()).label(&env);
        let settings_before = self.settings.clone();
        let asked_steps = std::mem::take(&mut self.requested);
        self.pick_color = None;
        let slots = FillTool::slots();
        let mut open = self.open;
        let mut reset_all = false;
        let this = RefCell::new(&mut *self);

        let shown = FullWindowDialog::new(title)
            .command_scope(FillCommands::SCOPE)
            .opened(Bind::bound(&mut open))
            .buttons([
                FooterButton::new(common.ok.clone(), DialogRole::Accept),
                FooterButton::new(dialog.apply.clone(), DialogRole::Other).stays_open(true),
                FooterButton::new(common.cancel.clone(), DialogRole::Cancel),
            ])
            .side_footer(SideFooter::bottom().leading(|ui| {
                let icon = IconSpec::svg(IconId::new("undo"));
                reset_all = clicked(&IconButton::new(icon).label(params.reset.clone()).id_salt("reset-all").show(ui));
            }))
            .sides(|ui, _| {
                let mut guard = this.borrow_mut();
                guard.show_sides(ui, &env, source);
            })
            .show(ui, |ui, cx| {
                let mut guard = this.borrow_mut();
                guard.show_body(ui, cx, &env, &slots, source, &asked_steps);
            });

        let work = this.into_inner();
        work.open = open;
        if reset_all {
            // The footer's Reset sets the session and every setting back, whether or not the session was already
            // at its defaults.
            let before = work.session.clone();
            work.session.reset(FillReset::All, &mut work.settings);
            work.synced();
            work.record_history(before);
            work.events.push(FillEvent::Reset(FillReset::All));
        }
        let picked = shown.output.picked;
        let closed = match picked {
            Some(PICK_OK) => Some(DialogRole::Accept),
            Some(PICK_CANCEL) => Some(DialogRole::Cancel),
            _ => shown.output.closed.map(|_| DialogRole::Cancel),
        };
        let color_owed = std::mem::take(&mut work.color_owed);
        FillOutput {
            events: std::mem::take(&mut work.events),
            closed,
            apply: picked == Some(PICK_APPLY),
            response: shown.output.response.clone(),
            settings_changed: work.settings != settings_before || color_owed,
            pick_color: work.pick_color.take(),
        }
    }

    /// The app's colour picker answered a [`FillOutput::pick_color`] request. A key this workspace
    /// did not ask for is ignored.
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        if key == OVERLAY_COLOR_KEY {
            let [r, g, b, _] = Color32::from(color).to_array();
            if self.settings.color != [r, g, b] {
                self.settings.color = [r, g, b];
                self.color_owed = true;
            }
        }
    }

    /// The strokes and fill-area edits recorded so far.
    pub fn session(&self) -> &FillSession {
        &self.session
    }

    /// The current fill settings.
    pub fn settings(&self) -> &FillSettings {
        &self.settings
    }

    /// Apply's and OK's parameters.
    pub fn command(&self) -> FillCommand {
        FillCommand::new(&self.session, &self.settings)
    }

    /// The app applied the fill: the session's history clears (Apply is one document step), the
    /// strokes and edits stay, and a preview is asked for over the new document.
    pub fn rebase(&mut self) {
        // pc: crates/ui-egui/src/dialogs/caf_ui/lifecycle.rs:80 `apply` (refresh), caf_ui/preview.rs:135
        // `rebase`.
        self.history = EditHistory::new(&self.session);
        self.error = None;
        self.job = None;
    }

    /// The app's Apply or OK failed: the message shows under the panel and the workspace stays (it opens again if
    /// OK had closed it).
    pub fn apply_failed(&mut self, message: String) {
        self.error = Some(FillIssue::Message(message));
        self.open = true;
    }

    /// The app found that the last fill-area edit empties the fill area: the edit is taken back
    /// and `message` shows under the panel. The history takes the step back without a redo entry.
    pub fn refuse_fill_edit(&mut self, message: String) {
        // pc: crates/ui-egui/src/dialogs/caf_ui/lifecycle.rs:211-219.
        match self.history.take_back() {
            Some(before) => self.session = before,
            None => {
                self.session.fill_edits.pop();
            }
        }
        self.synced();
        self.error = Some(FillIssue::Message(message));
    }

    /// Undo or redo from the app's Edit menu; returns whether it stepped, or with a stroke open whether it will:
    /// the next [`show`](Self::show) cancels the stroke (`UndoDuringGesture`) first, then steps.
    pub fn step(&mut self, step: Step) -> bool {
        // pc: crates/ui-egui/src/dialogs/caf_ui/controls.rs:151-159 (⌘Z, ⇧⌘Z).
        if self.busy {
            self.requested.push(step);
            return match step {
                Step::Undo => self.history.can_undo(),
                _ => self.history.can_redo(),
            };
        }
        self.step_now(step)
    }
}

impl ContentAwareFillWorkspace {
    /// The two side columns: the panel with the footer's content, and the Preview.
    fn show_sides(&mut self, ui: &mut Ui, env: &Env, source: &dyn FillSource) {
        let words = env.words::<FillWords>();
        let params = env.words::<ParamDialogWords>();
        let error = self.error.as_ref().map(|issue| issue.text(env));
        let sampling = self.session.sampling;
        let panel = FullWindowSide::new(words.name.clone())
            .show(ui, |ui, _| FillPanel::new(Bind::bound(&mut self.settings), sampling).error(error.as_deref()).show(ui))
            .inner;
        if let Some(panel) = panel {
            self.absorb_panel(panel);
        }
        FullWindowSide::new(params.preview.clone()).unscrolled().show(ui, |ui, _| {
            let image = source.preview().unwrap_or_else(|| source.image());
            FillPreviewPane::new(&mut self.preview_view, &mut self.framing, &self.preview, image)
                .frame(source.sampling_window())
                .navigation(self.navigation)
                .checkerboard(self.checkerboard)
                .show(ui);
        });
    }

    /// What the panel reported: a slider's gesture, a sampling option, a Reset, the colour request.
    fn absorb_panel(&mut self, panel: FillPanelOutput) {
        match panel.edit.last() {
            Some(EditEvent::Begin { .. } | EditEvent::Update { .. }) => self.settings_gesture = true,
            Some(_) => self.settings_gesture = false,
            None => {}
        }
        if let Some(mode) = panel.sampling {
            let before = self.session.clone();
            if self.session.set_sampling(mode) {
                self.synced();
                self.record_history(before);
                self.events.push(FillEvent::Reset(FillReset::Sampling));
            }
        }
        if let Some(what) = panel.reset {
            let before = self.session.clone();
            if self.session.reset(what, &mut self.settings) || what == FillReset::Fill {
                self.synced();
                self.record_history(before);
                self.events.push(FillEvent::Reset(what));
            }
        }
        if panel.pick_color.is_some() {
            self.pick_color = panel.pick_color;
        }
    }

    /// The body: the steps and keys, the options bar, the tool strip and the canvas; then the preview job.
    fn show_body(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, env: &Env, slots: &[ToolSlot<FillTool>], source: &dyn FillSource, asked: &[Step]) {
        let ctx = ui.ctx().clone();
        let words = env.words::<FillWords>();
        while let Some((command, _)) = cx.take_command::<ViewCommands>() {
            self.view.apply(command);
        }
        // An open stroke is cancelled first; the step follows.
        for step in asked {
            self.cancel_gesture(&ctx, CancelReason::UndoDuringGesture);
            self.step_now(*step);
        }
        {
            while let Some((command, fired)) = cx.take_command::<FillCommands>() {
                if let Some(tool) = command.tool() {
                    let slot = slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == tool));
                    let tools: Vec<&RailTool<FillTool>> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                    let picked = slot.and_then(|slot| {
                        tool_pick(&tools, slot.id, &self.tool, &self.last_used, PickStep::of(&fired), |_| true).map(|entry| (slot.id, entry.id))
                    });
                    if let Some((slot_id, tool)) = picked {
                        self.pick(&ctx, tool);
                        self.last_used.set(slot_id, tool);
                    }
                    continue;
                }
                match command {
                    FillCommands::Smaller => self.options.step_size(false),
                    FillCommands::Larger => self.options.step_size(true),
                    FillCommands::HideTint => self.tint_hidden = !self.tint_hidden,
                    _ => {}
                }
            }

            let bar = FillBar::new(self.tool, &mut self.options).show(ui);
            if let Some(edit) = bar.edit {
                self.add_edit(edit);
            }
            if bar.reset_fill_area {
                let before = self.session.clone();
                if self.session.reset(FillReset::FillArea, &mut self.settings) {
                    self.error = None;
                    self.record_history(before);
                    self.events.push(FillEvent::Reset(FillReset::FillArea));
                }
            }

            Row::new().fill().show(ui, |ui, _| {
                let rail = ToolRail::new(words.tools.clone(), slots, &self.tool, &self.last_used).show_rail(ui, |_, _| {});
                if let Some(next) = rail.last_used {
                    self.last_used = next;
                }
                if let Some(tool) = rail.select {
                    self.pick(&ctx, tool);
                }
                self.show_canvas(ui, source, &words);
            });
        }
        self.absorb_canvas_events();
        self.busy = self.canvas.busy();
        self.ask_for_preview(cx, source);
    }

    /// The canvas with the picture, the sampling tint, the fill area's ants, the lasso being drawn, the brush
    /// resize preview and the tool.
    fn show_canvas(&mut self, ui: &mut Ui, source: &dyn FillSource, words: &FillWords) {
        let image = source.image();
        let [r, g, b] = self.settings.color;
        let alpha = (self.settings.opacity.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
        let tint = Color32::from_rgba_unmultiplied(r, g, b, alpha);
        let overlay = source
            .sampling()
            .filter(|_| self.settings.show_overlay && !self.tint_hidden)
            .map(|plane| FillOverlay::new(plane, source.fill_area(), image.size()).excluded(self.settings.excluded));
        let ants = source.fill_area().map(|plane| self.ants_of(plane));
        let lasso: Option<Vec<Point>> = self.canvas.lasso_points().map(<[Point]>::to_vec);
        let falloff = self.canvas.resize().dimensions().map(|dimensions| TipFalloff::hard(dimensions.diameter / 2.0));
        let resize = falloff.as_ref().and_then(|falloff| self.canvas.resize().preview(falloff));
        let fill_area_name = words.fill_area.clone();

        let mut canvas = FullWindowCanvas::new(Bind::bound(&mut self.view), image)
            .checkerboard(self.checkerboard)
            .navigation(self.navigation)
            .momentary(&mut self.momentary)
            .controls(|ui, rect| {
                if let Some(edges) = &ants {
                    Marquee::segments(fill_area_name.clone(), edges.iter().copied()).look(MarqueeLook::Ants).show_at(ui, rect);
                }
                if let Some(points) = &lasso {
                    Marquee::polyline(Label::Empty, points.iter().copied()).look(MarqueeLook::Drawing).show_at(ui, rect);
                }
                if let Some(preview) = &resize {
                    preview.clone().show_at(ui, rect);
                }
            });
        if let Some(overlay) = &overlay {
            canvas = canvas.overlay(RasterOverlay::new(overlay).tint(tint));
        }
        let shown = match self.tool {
            FillTool::Hand => canvas.tool(&mut self.hand).show(ui),
            FillTool::Zoom => canvas.tool(&mut self.zoom).show(ui),
            FillTool::SamplingBrush | FillTool::Lasso => {
                let mut tool = self.canvas.tool(self.tool, &mut self.options, &self.session, &self.limits, &self.cursor_options);
                canvas.tool(&mut tool).show(ui)
            }
        };
        self.canvas_id = Some(shown.response.id);
    }

    /// The fill area's outline for `plane`, computed again only when the plane changed.
    fn ants_of(&mut self, plane: &SamplingPlane) -> Arc<Vec<(Point, Point)>> {
        match &self.ants {
            Some((kept, edges)) if kept == plane => Arc::clone(edges),
            _ => {
                let edges = Arc::new(fill_area_edges(plane));
                self.ants = Some((plane.clone(), Arc::clone(&edges)));
                edges
            }
        }
    }

    /// Asks the source for a preview job when the fill changed and nothing is being drawn or dragged, and reads
    /// the job's state. The job's key is the request's number; a newer request cancels the older job.
    fn ask_for_preview(&mut self, cx: &mut BodyCx<'_>, source: &dyn FillSource) {
        let texel = source.sampling().map_or(1, |plane| plane.texel.max(1));
        let waiting = self.canvas.busy() || self.settings_gesture;
        if !waiting && self.job.as_ref().is_none_or(|job| job.differs_from(&self.session, &self.settings, texel)) {
            self.job = Some(FillPreviewRequest::new(&self.session, &self.settings, texel));
            self.job_id += 1;
        }
        if let Some(request) = self.job.as_ref() {
            self.preview = cx.pending(self.job_id, || source.preview_job(request)).clone();
        }
    }

    /// Takes the events the canvas tool collected.
    fn absorb_canvas_events(&mut self) {
        for event in self.canvas.take_events() {
            self.absorb(event);
        }
    }

    /// Puts a canvas event in this pass's events, recording a committed stroke and a lasso edit in the session.
    fn absorb(&mut self, event: FillEvent) {
        match event {
            FillEvent::FillArea(edit) => {
                self.add_edit(edit);
            }
            FillEvent::Stroke(FillStrokeEvent::Commit(stroke)) => {
                let before = self.session.clone();
                self.session.add_stroke(stroke.clone());
                self.error = None;
                self.record_history(before);
                self.events.push(FillEvent::Stroke(FillStrokeEvent::Commit(stroke)));
            }
            FillEvent::Refused(limit) => {
                self.error = Some(FillIssue::Limit(limit));
                self.events.push(FillEvent::Refused(limit));
            }
            other => self.events.push(other),
        }
    }

    /// Records a fill-area edit within the limits, or sets the reason as the error and reports it. Whether it
    /// was recorded.
    fn add_edit(&mut self, edit: FillAreaEdit) -> bool {
        let before = self.session.clone();
        match self.session.add_fill_edit(edit.clone(), &self.limits) {
            Ok(()) => {
                self.error = None;
                self.record_history(before);
                self.events.push(FillEvent::FillArea(edit));
                true
            }
            Err(limit) => {
                self.error = Some(FillIssue::Limit(limit));
                self.events.push(FillEvent::Refused(limit));
                false
            }
        }
    }

    /// Records the session as one history step when it differs from `before`.
    fn record_history(&mut self, before: FillSession) {
        if before == self.session {
            return;
        }
        let name: Arc<str> = Arc::from(HISTORY_NAME);
        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name, None));
    }

    /// What the session's sampling option sets in the settings and the bar after undo, redo or a
    /// reset: the option is remembered and Custom makes the brush add. A stale error goes.
    fn synced(&mut self) {
        self.settings.sampling = self.session.sampling;
        self.options.brush_add = self.session.sampling.brush_adds();
        self.error = None;
    }

    fn step_now(&mut self, step: Step) -> bool {
        let stepped = self.history.step(&mut self.session, step);
        if stepped {
            self.synced();
        }
        stepped
    }

    /// Switches to `tool`, cancelling the old tool's gesture first.
    fn pick(&mut self, ctx: &Context, tool: FillTool) {
        if tool != self.tool {
            self.cancel_gesture(ctx, CancelReason::Removed);
            self.tool = tool;
        }
    }

    /// Cancels the gesture the current tool holds (a stroke, a lasso or a resize, or the Hand's or Zoom's drag)
    /// through the host, and the canvas's own state if the host held none.
    fn cancel_gesture(&mut self, ctx: &Context, reason: CancelReason) {
        if let Some(id) = self.canvas_id {
            match self.tool {
                FillTool::Hand => ToolHost::cancel(ctx, id, &mut self.hand, reason),
                FillTool::Zoom => ToolHost::cancel(ctx, id, &mut self.zoom, reason),
                FillTool::SamplingBrush | FillTool::Lasso => {
                    let mut tool = self.canvas.tool(self.tool, &mut self.options, &self.session, &self.limits, &self.cursor_options);
                    ToolHost::cancel(ctx, id, &mut tool, reason)
                }
            };
        }
        if self.canvas.busy() {
            self.canvas.abandon(reason);
        }
        self.absorb_canvas_events();
    }
}
