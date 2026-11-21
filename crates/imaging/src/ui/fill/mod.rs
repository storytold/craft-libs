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
//! OK with [`FillCommand`]. The workspace is a [`FullWindowWorkspace`] on the app's dialog host.
//! Apply keeps the workspace open: the app fills with the [`FillEvent::Apply`] command, then
//! calls [`ContentAwareFillWorkspace::answer_apply`]. OK is the host's held OK: the app takes the
//! command with `take_ok`, fills and answers with `answer_ok`, and a failed fill keeps the workspace
//! open. Cancel closes, keeping what Apply did.
//!
//! Files: `session`, `lasso` and `tool`; `overlay`, `panel`, `preview_pane`, `bar`, `commands`,
//! `words` and this file's bodies.
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`EditHistory`]: craft_behavior::EditHistory

pub(crate) mod bar;
mod commands;
mod lasso;
mod overlay;
pub(crate) mod panel;
pub(crate) mod preview_pane;
mod session;
#[cfg(feature = "suite")]
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

use egui::{Color32, Context, Id, Ui};
use serde::Deserialize;

use craft_behavior::edit_history::Step;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, EditEvent, EditHistory, IconId, Mixed, PixelSize};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::canvas::{HandTool, NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, ZoomTool, tool_pick};
use craft_ui::canvas_controls::brush::{BrushCursorOptions, TipFalloff};
use craft_ui::canvas_controls::{Marquee, MarqueeLook};
use craft_ui::contract::{Bind, BodyCx, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, Tracked, clicked};
use craft_ui::dialog::{
    DialogResult, DialogWords, FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowTitle, FullWindowView,
    FullWindowWorkspace, RasterOverlay, SideFooter, WorkspaceCx, WorkspacePass, serde_fields, set_serde_field,
};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::{FormDialogWords, ParamDialogWords};
use craft_ui::primitives::{Checker, IconSpec};
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft_ui::runtime::pending::{Job, Pending};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};

use crate::ui::wide_angle::PreviewDone;

/// The key of the colour request for the sampling tint ([`FillEvent::PickColor`],
/// [`ContentAwareFillWorkspace::set_color`]).
pub const OVERLAY_COLOR_KEY: &str = "fill.overlayColor";

/// The name the history gives its steps.
const HISTORY_NAME: &str = "Content-Aware Fill";

/// The footer's OK and Apply buttons, by the index the dialog reports a pick with (Cancel is the third, and closes the
/// dialog by itself).
const PICK_OK: usize = 0;
const PICK_APPLY: usize = 1;

/// What the app gives the workspace to show each pass.
pub trait FillSource: Send + Sync {
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
    /// The Apply button was pressed (the workspace stays open): the app fills with the command and answers with
    /// [`ContentAwareFillWorkspace::answer_apply`].
    Apply(FillCommand),
    /// The Color swatch was pressed: the app opens its colour picker and answers with
    /// [`ContentAwareFillWorkspace::set_color`].
    PickColor(ColorRequest),
    /// A fill setting changed.
    SettingsChanged(FillSettings),
}

/// An edit the control requests make of the workspace through [`ContentAwareFillWorkspace::perform`]. Each carries what
/// the user would have drawn or chosen.
// PLAN: the stub's `FillOp` was a unit enum, which cannot say which stroke or polygon to record; the variants carry
// their value. dialogs-w20's `control/dialogs.rs` and dialogs-w9's re-export read the new shape.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FillOp {
    /// A Sampling Brush stroke.
    Stroke(SamplingStroke),
    /// A lasso edit of the fill area.
    FillArea(FillAreaEdit),
    /// A Reset.
    Reset(FillReset),
    /// A change of the sampling option.
    Sampling(SamplingMode),
}

/// What shows under the panel: the limit a stroke or edit ran into, or the app's message (a failed
/// Apply or OK, an edit that emptied the fill area).
#[derive(Clone, Debug, PartialEq)]
enum FillIssue {
    Limit(FillLimit),
    Message(Label),
}

impl FillIssue {
    fn text(&self, env: &Env) -> String {
        match self {
            FillIssue::Limit(limit) => limit.message(env),
            FillIssue::Message(message) => env.view().resolve(message).as_str().to_owned(),
        }
    }
}

type Ants = (SamplingPlane, Arc<Vec<(Point, Point)>>);

/// The Content-Aware Fill workspace.
///
/// Keys ([`FillCommands`], on the body's scope, none while a text field edits): B, L, H, Z pick the tools; `[` and
/// `]` the brush size ([`FillToolOptions::step_size`]); ⌘H hides or shows the tint. The zoom chords are the
/// dialog's view commands. ⌘Z and ⇧⌘Z step the history (a field being edited steps its own text first); Enter
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
    // View state outside the history.
    view: FullWindowView,
    preview_view: FullWindowView,
    framing: PreviewFraming,
    tool: FillTool,
    last_used: Tracked<LastUsed<FillTool>>,
    slots: Tracked<Vec<ToolSlot<FillTool>>>,
    options: FillToolOptions,
    canvas: FillCanvas,
    hand: HandTool,
    zoom: ZoomTool,
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
    /// The settings as the app last heard them, so a change made between passes (a control request, a colour answer)
    /// is reported once by the next pass.
    reported: FillSettings,
    /// The outline of the fill area's covered cells, kept for as long as the plane is the same.
    ants: Option<Ants>,
    /// The events the next pass reports first: what the methods between passes queued, then the pass's own.
    events: Vec<FillEvent>,
    /// Where the settings are remembered ([`remember`](ContentAwareFillWorkspace::remember)).
    remember: Option<SaveKey>,
    /// The first pass has read the remembered settings.
    recalled: bool,
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
            reported: open.settings.clone(),
            settings: open.settings,
            limits: open.limits,
            subject: open.subject,
            size: PixelSize::new(width, height),
            view: FullWindowView::new(),
            preview_view: FullWindowView::new(),
            framing: PreviewFraming::default(),
            tool: FillTool::SamplingBrush,
            last_used: Tracked::new(LastUsed::default()),
            slots: Tracked::new(FillTool::slots()),
            canvas: FillCanvas::new(),
            hand: HandTool::new(navigation),
            zoom: ZoomTool::new(navigation),
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
            ants: None,
            events: Vec::new(),
            remember: None,
            recalled: false,
        }
    }

    /// Remembers the fill settings under `key`: the first pass opens with the stored settings when there are any, in
    /// place of [`FillOpen::settings`], and every close of the workspace stores them
    /// ([`closed`](FullWindowWorkspace::closed)).
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.remember = Some(key);
        self
    }

    /// The first pass: the stored settings replace the opening ones, with the sampling option they carry as the
    /// session's starting point (not a step to undo). The pass reports the change as `SettingsChanged`.
    fn recall(&mut self, ctx: &Context) {
        if std::mem::replace(&mut self.recalled, true) {
            return;
        }
        let Some(key) = &self.remember else { return };
        let Some(stored) = saved_value::<Option<FillSettings>>(ctx, key) else { return };
        self.settings = stored;
        self.session.sampling = self.settings.sampling;
        self.options.brush_add = self.settings.sampling.brush_adds();
        self.history = EditHistory::new(&self.session);
    }

    /// Stores the settings under the remembered key, if any.
    fn store(&self, ctx: &Context) {
        if let Some(key) = &self.remember {
            set_saved_value(ctx, key, Some(self.settings.clone()));
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
    /// [`FillSource::preview_job`]. OK is the host's held OK when the app asks for one (`hold_ok`): the pass reports the
    /// command as `held`, and the app answers it. A second pass of a frame reports only what its own input did: the
    /// frame's steps and commands went to the first pass.
    fn pass(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, dyn FillSource>) -> WorkspacePass<FillEvent, FillCommand> {
        // pc: crates/ui-egui/src/dialogs/caf_ui/view.rs:265 `show`; caf_ui/controls.rs:151 `keys`.
        let WorkspaceCx { source, open, hold_ok, ok_waits, .. } = cx;
        self.recall(ui.ctx());
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let dialog = env.words::<DialogWords>();
        let params = env.words::<ParamDialogWords>();
        let title =
            FullWindowTitle::new(env.words::<FillWords>().name.clone()).subject(self.subject.clone()).zoom(self.view.zoom_percent()).label(&env);
        let mut reset_all = false;
        let this = RefCell::new(&mut *self);

        let shown = FullWindowDialog::new(title)
            .command_scope(FillCommands::SCOPE)
            .opened(open)
            .buttons([
                FooterButton::new(common.ok.clone(), DialogRole::Accept).stays_open(hold_ok).enabled(!ok_waits),
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
                guard.show_body(ui, cx, &env, source);
            });

        let work = this.into_inner();
        if reset_all {
            work.apply_reset(FillReset::All);
        }
        let picked = shown.output.picked;
        if picked == Some(PICK_APPLY) {
            work.events.push(FillEvent::Apply(work.command()));
        }
        if work.settings != work.reported {
            work.reported = work.settings.clone();
            work.events.push(FillEvent::SettingsChanged(work.reported.clone()));
        }
        // A held OK files the command and keeps the dialog open; Esc and Cancel close it either way.
        let (result, held) = if hold_ok {
            (shown.output.closed.map(|_| DialogResult::Cancel), (picked == Some(PICK_OK)).then(|| work.command()))
        } else {
            (shown.output.result(|| work.command()), None)
        };
        let events = std::mem::take(&mut work.events);
        WorkspacePass::new(shown.output.response, events).held(held).result(result).canvas(work.canvas_id)
    }

    /// The app's colour picker answered a [`FillEvent::PickColor`] request. A key this workspace
    /// did not ask for is ignored. The next pass reports the new colour as [`FillEvent::SettingsChanged`].
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        if key == OVERLAY_COLOR_KEY {
            let [r, g, b, _] = Color32::from(color).to_array();
            self.settings.color = [r, g, b];
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

    /// Answers the [`FillEvent::Apply`]: `Ok` means the app filled, so the session's history clears (Apply is one
    /// document step), the strokes and edits stay, and a preview is asked for over the new document. `Err` shows the
    /// message under the panel; OK stays enabled either way.
    pub fn answer_apply(&mut self, answer: Result<(), String>) {
        // pc: crates/ui-egui/src/dialogs/caf_ui/lifecycle.rs:80 `apply` (refresh), caf_ui/preview.rs:135 `rebase`.
        match answer {
            Ok(()) => {
                self.history = EditHistory::new(&self.session);
                self.error = None;
                self.job = None;
            }
            Err(message) => self.error = Some(FillIssue::Message(message.into())),
        }
    }

    /// The fill settings, for the control requests that change them. The next pass reports a change as
    /// [`FillEvent::SettingsChanged`].
    pub fn settings_mut(&mut self) -> &mut FillSettings {
        &mut self.settings
    }

    /// Runs `op` as the user would; its events come out of the next pass. `Err` when a limit refuses it: nothing is
    /// recorded, the panel shows the limit and the next pass reports [`FillEvent::Refused`]. The caller asks for the
    /// repaint that runs the next pass (this method has no context).
    pub fn perform(&mut self, op: FillOp) -> Result<(), FillLimit> {
        match op {
            FillOp::Stroke(stroke) => self.record_stroke(stroke),
            FillOp::FillArea(edit) => self.add_edit(edit),
            FillOp::Reset(what) => {
                self.apply_reset(what);
                Ok(())
            }
            FillOp::Sampling(mode) => {
                self.apply_sampling(mode);
                Ok(())
            }
        }
    }

    /// Undoes the last edit, abandoning a stroke still open first. The session changes at once; the app reads it
    /// after its next pass.
    pub fn undo(&mut self) {
        self.cancel_gesture(None, CancelReason::UndoDuringGesture);
        self.step_now(Step::Undo);
    }

    /// Redoes the last undone edit, abandoning a stroke still open first.
    pub fn redo(&mut self) {
        self.cancel_gesture(None, CancelReason::UndoDuringGesture);
        self.step_now(Step::Redo);
    }

    /// Picks the canvas tool.
    pub fn set_tool(&mut self, tool: FillTool) {
        self.pick(None, tool);
    }

    /// Sets the Sampling Brush size, in 1..=[`FillToolOptions::SIZE_MAX`] pixels; a non-finite size changes nothing.
    pub fn set_brush(&mut self, size: f64) {
        if size.is_finite() {
            self.options.brush_size = size.round().clamp(1.0, FillToolOptions::SIZE_MAX);
        }
    }

    /// Apply's and OK's parameters.
    pub fn command(&self) -> FillCommand {
        FillCommand::new(&self.session, &self.settings)
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
        self.error = Some(FillIssue::Message(message.into()));
    }
}

impl ContentAwareFillWorkspace {
    /// The two side columns: the panel with the footer's content, and the Preview.
    fn show_sides(&mut self, ui: &mut Ui, env: &Env, source: &dyn FillSource) {
        let words = env.words::<FillWords>();
        let preview = env.words::<FormDialogWords>().preview.clone();
        let error = self.error.as_ref().map(|issue| issue.text(env));
        let sampling = self.session.sampling;
        let panel = FullWindowSide::new(words.name.clone())
            .show(ui, |ui, _| FillPanel::new(Bind::bound(&mut self.settings), sampling).error(error.as_deref()).show(ui))
            .inner;
        if let Some(panel) = panel {
            self.absorb_panel(panel);
        }
        FullWindowSide::new(preview).unscrolled().show(ui, |ui, _| {
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
            self.apply_sampling(mode);
        }
        if let Some(what) = panel.reset {
            self.apply_reset(what);
        }
        if let Some(request) = panel.pick_color {
            self.events.push(FillEvent::PickColor(request));
        }
    }

    /// A Reset: the session and the settings it covers go back, the step is recorded and the app hears of it. A Reset of
    /// the fill settings or of everything is reported whether or not the session was already at its defaults.
    fn apply_reset(&mut self, what: FillReset) {
        let before = self.session.clone();
        let changed = self.session.reset(what, &mut self.settings);
        if changed || matches!(what, FillReset::Fill | FillReset::All) {
            self.synced();
            self.record_history(before);
            self.error = None;
            self.events.push(FillEvent::Reset(what));
        }
    }

    /// A new sampling option: the strokes start over, the step is recorded and the app hears of it. The same option
    /// with no strokes changes nothing.
    fn apply_sampling(&mut self, mode: SamplingMode) {
        let before = self.session.clone();
        if self.session.set_sampling(mode) {
            self.synced();
            self.record_history(before);
            self.events.push(FillEvent::Reset(FillReset::Sampling));
        }
    }

    /// The body: the steps and keys, the options bar, the tool strip and the canvas; then the preview job.
    fn show_body(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, env: &Env, source: &dyn FillSource) {
        let ctx = ui.ctx().clone();
        let words = env.words::<FillWords>();
        while let Some((command, _)) = cx.take_command::<ViewCommands>() {
            self.view.apply(command);
        }
        // An open stroke is cancelled first; the step follows.
        while let Some(step) = cx.take_step(self.history.can_undo(), self.history.can_redo()) {
            // pc: crates/ui-egui/src/dialogs/caf_ui/controls.rs:151-159 (⌘Z, ⇧⌘Z).
            self.cancel_gesture(Some(&ctx), CancelReason::UndoDuringGesture);
            self.step_now(step);
        }
        {
            while let Some((command, fired)) = cx.take_command::<FillCommands>() {
                if let Some(tool) = command.tool() {
                    // A tool key's release picks nothing.
                    if fired.phase == FirePhase::Release {
                        continue;
                    }
                    let slot = self.slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == tool));
                    let tools: Vec<&RailTool<FillTool>> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                    let picked = slot.and_then(|slot| {
                        tool_pick(&tools, slot.id, &self.tool, &self.last_used, PickStep::of(&fired), |_| true).map(|entry| (slot.id, entry.id))
                    });
                    if let Some((slot_id, tool)) = picked {
                        self.pick(Some(&ctx), tool);
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
                // A refusal is already shown and reported by `add_edit`.
                let _ = self.add_edit(edit);
            }
            if bar.reset_fill_area {
                self.apply_reset(FillReset::FillArea);
            }

            Row::new().fill().show(ui, |ui, _| {
                let rail = ToolRail::new(words.tools.clone(), &self.slots, &self.tool, &self.last_used).show_column(ui, |_, _| {});
                if let Some(next) = rail.last_used {
                    *self.last_used = next;
                }
                if let Some(tool) = rail.select {
                    self.pick(Some(&ctx), tool);
                }
                self.show_canvas(ui, source, &words);
            });
        }
        self.absorb_canvas_events();
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
                // A refusal is already shown and reported by `add_edit`.
                let _ = self.add_edit(edit);
            }
            // The tool checked the limits as the stroke grew.
            FillEvent::Stroke(FillStrokeEvent::Commit(stroke)) => self.commit_stroke(stroke),
            FillEvent::Refused(limit) => {
                self.error = Some(FillIssue::Limit(limit));
                self.events.push(FillEvent::Refused(limit));
            }
            other => self.events.push(other),
        }
    }

    /// Records a finished stroke in the session and reports it.
    fn commit_stroke(&mut self, stroke: SamplingStroke) {
        let before = self.session.clone();
        self.session.add_stroke(stroke.clone());
        self.error = None;
        self.record_history(before);
        self.events.push(FillEvent::Stroke(FillStrokeEvent::Commit(stroke)));
    }

    /// Records a whole stroke that did not come from the canvas (a control request) within the limits: the stroke
    /// limit and the points limit, a total equal to the limit passing, as the brush and `add_fill_edit` count them. A
    /// refusal records nothing, shows under the panel and is reported.
    fn record_stroke(&mut self, stroke: SamplingStroke) -> Result<(), FillLimit> {
        let fits = match self.session.may_begin_stroke(&self.limits) {
            Ok(()) if self.session.points_used(Some(&stroke)) > self.limits.points => Err(FillLimit::Vertices(self.limits.points)),
            other => other,
        };
        match fits {
            Ok(()) => {
                self.commit_stroke(stroke);
                Ok(())
            }
            Err(limit) => Err(self.refuse(limit)),
        }
    }

    /// Records a fill-area edit within the limits, or refuses it.
    fn add_edit(&mut self, edit: FillAreaEdit) -> Result<(), FillLimit> {
        let before = self.session.clone();
        self.session.add_fill_edit(edit.clone(), &self.limits).map_err(|limit| self.refuse(limit))?;
        self.error = None;
        self.record_history(before);
        self.events.push(FillEvent::FillArea(edit));
        Ok(())
    }

    /// Shows `limit` under the panel and reports it; the limit, for the caller to return.
    fn refuse(&mut self, limit: FillLimit) -> FillLimit {
        self.error = Some(FillIssue::Limit(limit));
        self.events.push(FillEvent::Refused(limit));
        limit
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

    fn step_now(&mut self, step: Step) {
        if self.history.step(&mut self.session, step) {
            self.synced();
        }
    }

    /// Switches to `tool`, cancelling the old tool's gesture first.
    fn pick(&mut self, ctx: Option<&Context>, tool: FillTool) {
        if tool != self.tool {
            self.cancel_gesture(ctx, CancelReason::Removed);
            self.tool = tool;
        }
    }

    /// Cancels the gesture the current tool holds (a stroke, a lasso or a resize, or the Hand's or Zoom's drag)
    /// through the host, and the canvas's own state if the host held none. Without a context (a request between
    /// passes) only the canvas's own state is dropped.
    fn cancel_gesture(&mut self, ctx: Option<&Context>, reason: CancelReason) {
        if let (Some(ctx), Some(id)) = (ctx, self.canvas_id) {
            match self.tool {
                FillTool::Hand => ToolHost::new().cancel(ctx, id, &mut self.hand, reason),
                FillTool::Zoom => ToolHost::new().cancel(ctx, id, &mut self.zoom, reason),
                FillTool::SamplingBrush | FillTool::Lasso => {
                    let mut tool = self.canvas.tool(self.tool, &mut self.options, &self.session, &self.limits, &self.cursor_options);
                    ToolHost::new().cancel(ctx, id, &mut tool, reason)
                }
            };
        }
        if self.canvas.busy() {
            self.canvas.abandon(reason);
        }
        self.absorb_canvas_events();
    }
}

/// The names the automation surface gives the tools.
const TOOL_KEYS: [(&str, FillTool); 4] =
    [("samplingBrush", FillTool::SamplingBrush), ("lasso", FillTool::Lasso), ("hand", FillTool::Hand), ("zoom", FillTool::Zoom)];

/// The names the automation surface gives the lasso's operations.
const LASSO_KEYS: [(&str, SelectionOperation); 4] = [
    ("new", SelectionOperation::New),
    ("add", SelectionOperation::Add),
    ("subtract", SelectionOperation::Subtract),
    ("intersect", SelectionOperation::Intersect),
];

/// The name `table` gives `item`; empty for an item it does not list.
fn key_of<T: Copy + PartialEq>(table: &[(&'static str, T)], item: T) -> &'static str {
    table.iter().find(|(_, listed)| *listed == item).map_or("", |(key, _)| *key)
}

/// The item `table` lists under `key`.
fn item_of<T: Copy>(table: &[(&str, T)], key: &str) -> Option<T> {
    table.iter().find(|(listed, _)| *listed == key).map(|(_, item)| *item)
}

/// The value of the `stroke` action key.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StrokeArg {
    points: Vec<[f64; 2]>,
    size: Option<f64>,
    #[serde(default)]
    alt: bool,
}

/// The value of the `lasso` action key.
#[derive(Deserialize)]
struct LassoArg {
    points: Vec<[f64; 2]>,
    op: String,
}

/// Whether every coordinate is a finite number.
fn finite(points: &[[f64; 2]]) -> bool {
    points.iter().flatten().all(|coordinate| coordinate.is_finite())
}

/// The workspace on the app's dialog host. OK is the host's held OK (`HostedWorkspace::held_ok`): the app takes the
/// command with `take_ok`, fills and answers with `answer_ok`. Apply is the footer's own button, answered by
/// [`ContentAwareFillWorkspace::answer_apply`].
impl FullWindowWorkspace for ContentAwareFillWorkspace {
    type Source = dyn FillSource;
    type Event = FillEvent;
    type Result = FillCommand;

    const KEY: &'static str = "edit.contentAwareFill";

    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<Self::Event, Self::Result> {
        self.pass(ui, cx)
    }

    /// The settings, and the bar's `tool`, `brushSize`, `brushMode` (`"add"`, `"subtract"`), `lassoMode` (`"new"`,
    /// `"add"`, `"subtract"`, `"intersect"`) and `expandBy`.
    fn json_fields(&self, _source: &Self::Source) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.settings)?;
        let bar = fields.as_object_mut()?;
        let brush_mode = if self.options.brush_add { "add" } else { "subtract" };
        bar.insert("tool".to_owned(), key_of(&TOOL_KEYS, self.tool).into());
        bar.insert("brushSize".to_owned(), self.options.brush_size.into());
        bar.insert("brushMode".to_owned(), brush_mode.into());
        bar.insert("lassoMode".to_owned(), key_of(&LASSO_KEYS, self.options.lasso).into());
        bar.insert("expandBy".to_owned(), self.options.grow_by.into());
        Some(fields)
    }

    /// A settings key, a bar key of [`json_fields`](Self::json_fields), or an action: `stroke` (`{points, size?, alt?}`),
    /// `lasso` (`{points, op}`), `expand` and `contract` (pixels, 1..=100: the fill area grows or shrinks by that), `reset` (`"all"`, `"fillArea"`, `"sampling"`), `undo`, `redo` and `apply` (each `true`).
    /// A stroke or lasso edit goes through the limits like the canvas's; one a limit refuses is rejected and the panel
    /// shows the limit.
    fn set_json_field(&mut self, ctx: &Context, _source: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        let rejects = |value: &serde_json::Value| FieldRefusal::Rejects(value).message(field);
        match field {
            "tool" => {
                let tool = value.as_str().and_then(|key| item_of(&TOOL_KEYS, key)).ok_or_else(|| rejects(&value))?;
                self.pick(Some(ctx), tool);
            }
            "brushSize" => self.set_brush(value.as_f64().ok_or_else(|| rejects(&value))?),
            "brushMode" => {
                self.options.brush_add = match value.as_str() {
                    Some("add") => true,
                    Some("subtract") => false,
                    _ => return Err(rejects(&value)),
                };
            }
            "lassoMode" => {
                self.options.lasso = value.as_str().and_then(|key| item_of(&LASSO_KEYS, key)).ok_or_else(|| rejects(&value))?;
            }
            "expandBy" => {
                let by = value.as_f64().ok_or_else(|| rejects(&value))?;
                self.options.grow_by = by.clamp(FillAreaEdit::GROW_RANGE.0, FillAreaEdit::GROW_RANGE.1);
            }
            "expand" | "contract" => {
                let by = value.as_f64().filter(|by| by.is_finite()).ok_or_else(|| rejects(&value))?;
                self.perform(FillOp::FillArea(FillAreaEdit::grow(field == "expand", by))).map_err(|limit| limit.message(&Env::of_root(ctx)))?;
            }
            "sampling" => {
                let mode = serde_json::from_value::<SamplingMode>(value.clone()).map_err(|_| rejects(&value))?;
                self.perform(FillOp::Sampling(mode)).map_err(|limit| limit.message(&Env::of_root(ctx)))?;
            }
            "stroke" => {
                let arg = serde_json::from_value::<StrokeArg>(value.clone()).map_err(|_| rejects(&value))?;
                if arg.points.is_empty() || !finite(&arg.points) {
                    return Err(rejects(&value));
                }
                let size = arg.size.filter(|size| size.is_finite()).unwrap_or(self.options.brush_size).clamp(1.0, FillToolOptions::SIZE_MAX);
                let mode = FillToolOptions::stroke_mode(self.options.brush_add, arg.alt);
                self.perform(FillOp::Stroke(SamplingStroke::new(mode, size, arg.points))).map_err(|limit| limit.message(&Env::of_root(ctx)))?;
            }
            "lasso" => {
                let arg = serde_json::from_value::<LassoArg>(value.clone()).map_err(|_| rejects(&value))?;
                let operation = item_of(&LASSO_KEYS, &arg.op).ok_or_else(|| rejects(&value))?;
                if !finite(&arg.points) {
                    return Err(rejects(&value));
                }
                let edit = FillAreaEdit::lasso(operation, arg.points).ok_or_else(|| rejects(&value))?;
                self.perform(FillOp::FillArea(edit)).map_err(|limit| limit.message(&Env::of_root(ctx)))?;
            }
            "reset" => {
                let what = match value.as_str() {
                    Some("all") => FillReset::All,
                    Some("fillArea") => FillReset::FillArea,
                    Some("sampling") => FillReset::Sampling,
                    _ => return Err(rejects(&value)),
                };
                self.perform(FillOp::Reset(what)).map_err(|limit| limit.message(&Env::of_root(ctx)))?;
            }
            "undo" | "redo" | "apply" => {
                if value != serde_json::Value::Bool(true) {
                    return Err(rejects(&value));
                }
                match field {
                    "undo" => self.undo(),
                    "redo" => self.redo(),
                    _ => {
                        let command = self.command();
                        self.events.push(FillEvent::Apply(command));
                    }
                }
            }
            _ => set_serde_field(&mut self.settings, field, value)?,
        }
        Ok(())
    }

    fn confirm(&mut self, _ctx: &Context, _source: &Self::Source) -> Option<Self::Result> {
        Some(self.command())
    }

    /// The app's fill failed: the reason shows under the panel and the workspace stays open.
    fn ok_failed(&mut self, reason: Label) {
        self.error = Some(FillIssue::Message(reason));
    }

    fn closed(&mut self, ctx: &Context) {
        self.store(ctx);
    }
}
