//! The Select and Mask workspace (Select › Select and Mask…), in Photoshop's layout.
//!
//! [`SelectMaskWorkspace`] is a [`FullWindowDialog`] with the title strip "Select and Mask
//! (document, 66%)" (PhotoCraft's document tab is dropped, lead call 3), the full-width options bar
//! ([`SelectMaskBar`]), the tool strip at the start edge (W, R, B, L, H, Z with the shared W and L
//! slots), the canvas, and a side column at the end edge headed "Properties"
//! ([`SelectMaskPanel`]) with Reset at the footer's leading end and OK and Cancel.
//!
//! The library holds the session as typed data ([`SelectMaskSession`]: edits, Refine Edge Brush
//! strokes, refinement values) with [`EditHistory`] undo, draws the tools and the panels, and
//! asks for previews through [`SelectMaskSource::preview_job`]. The app's engine computes the view (the edits and
//! refinement on a reduced copy, composited by the [`MaskViewRecipe`] the workspace gives) and
//! shows it through [`SelectMaskSource`]; OK runs `select.refineEdge` with
//! [`SelectMaskSession::command`]. Cancel changes nothing. The app opens the workspace only with a
//! selection or a layer mask (lead call 26), and closes it when the document or layer changes
//! under it.
//!
//! ```ignore
//! let out = w.show(ui, &PcSelectMaskSource { view: app.sm_preview.view(), image: app.image(), ants: app.sm_preview.ants() });
//! if let Some(request) = out.pick_color { app.host.request_color(request) }
//! match out.closed {
//!     Some(DialogRole::Accept) => app.run("select.refineEdge", w.command()),
//!     Some(_) => app.close_select_mask(),
//!     None => {}
//! }
//! if w.settings().remember { prefs.select_mask = w.settings().clone() }
//! ```
//!
//! Files: `session` and `edits`; `tools`, `commands` and `bar`; `panel` and this file's
//! `show` and `step`.
//!
//! [`EditHistory`]: craft_behavior::EditHistory

mod bar;
mod commands;
mod edits;
mod panel;
mod session;
pub(crate) mod stories;
mod tools;
mod words;

pub use bar::{SelectMaskBar, SelectMaskBarOutput};
pub use commands::SelectMaskCommands;
pub use edits::{EditMode, ObjectTarget, RefineStroke, SelectMaskEdit, round_point};
pub use panel::{OVERLAY_COLOR_KEY, SelectMaskPanel, SelectMaskPanelOutput};
pub use session::{LimitRefused, OutputTo, RefineSettings, SelectMaskCommand, SelectMaskLimits, SelectMaskSession, SelectMaskSettings};
pub use tools::{BrushSizes, EditPhase, SelectMaskCanvas, SelectMaskCanvasTool, SelectMaskTool, SelectMaskToolOptions};
pub use words::SelectMaskWords;

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use egui::{Id, Response, Ui};

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, EditEvent, EditHistory, GestureId, IconId, Mixed};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::canvas::{NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, ZoomTool, tool_pick};
use craft_ui::canvas_controls::brush::{BrushCursorOptions, TipFalloff};
use craft_ui::contract::{Bind, NoValue, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::{FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, SideFooter};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::Checker;
use craft_ui::rails::{LastUsed, ToolRail};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase};
use craft_ui::runtime::edit::next_gesture_id;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::pending::{Job, Pending};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};

use crate::trace::MaskSource;
use crate::ui::mask_view::{MaskViewKeys, MaskViewRecipe, MaskViewState};
use crate::ui::selection::{OutlineSource, SelectionOverlay};
use crate::ui::wide_angle::PreviewDone;

/// What the app gives the workspace to show each pass.
pub trait SelectMaskSource {
    /// The view as the engine composited it for the last ready preview (the recipe of
    /// [`SelectMaskWorkspace::recipe`]), in document pixels.
    fn view(&self) -> &dyn RasterSource;

    /// The document's image as it is, in document pixels, with no view applied: the workspace
    /// draws it while X is held ([`MaskViewRecipe::hidden`]), without waiting for a preview.
    fn image(&self) -> &dyn RasterSource;

    /// The refined selection for Marching Ants and its reduction factor (document pixels per mask
    /// pixel; 1 with High Quality Preview), drawn by `SelectionOverlay` with `reduction(k)`.
    /// `None` while there is none; PhotoCraft caps the ants at 200 000 edges and 4 000 000
    /// pixels.
    fn ants(&self) -> Option<(&dyn MaskSource, u32)>;

    /// The job that computes the view for `request`. The workspace starts it once per request: a change of the
    /// session, of the recipe (not X held), of High Quality Preview or of Sample All Layers asks for a new one, and
    /// a stroke or slider drag in progress holds the request back until it ends.
    fn preview_job(&self, request: &SelectMaskPreviewRequest) -> Job<PreviewDone, String>;
}

/// What a preview shows: everything that changes the engine's view.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SelectMaskPreviewRequest {
    /// The edits, strokes and refinement values.
    pub session: SelectMaskSession,
    /// What the engine composites, with `hidden` off (X held shows the image without a new preview).
    pub recipe: MaskViewRecipe,
    /// High Quality Preview: the preview runs at full size.
    pub high_quality: bool,
    /// Sample All Layers.
    pub sample_all_layers: bool,
}

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SelectMaskOpen {
    /// The document's size in pixels.
    pub size: [u32; 2],
    /// The document's name: the title's subject.
    pub subject: String,
    /// The remembered settings when Remember Settings was on, else the defaults.
    pub settings: SelectMaskSettings,
    /// The limits on edits, strokes and points.
    pub limits: SelectMaskLimits,
    /// The targeted layer mask is refined (`"target": "mask"`), not the selection.
    pub target_mask: bool,
    /// The user's wheel, flick and animated-zoom options.
    pub navigation: NavigationOptions,
    /// The user's transparency grid under the image.
    pub checkerboard: Checker,
    /// Preferences › Cursors, which shape the brush outline; Photoshop's by default.
    pub cursor_options: Option<BrushCursorOptions>,
}

impl SelectMaskOpen {
    /// Opens on a document of `size` pixels named `subject`; settings and options take their defaults.
    pub fn new(size: [u32; 2], subject: impl Into<String>, limits: SelectMaskLimits) -> Self {
        Self {
            size,
            subject: subject.into(),
            settings: SelectMaskSettings::default(),
            limits,
            target_mask: false,
            navigation: NavigationOptions::default(),
            checkerboard: Checker::default(),
            cursor_options: None,
        }
    }

    /// Sets the starting settings.
    #[must_use]
    pub fn settings(mut self, settings: SelectMaskSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Sets whether the layer mask is refined instead of the selection.
    #[must_use]
    pub fn target_mask(mut self, target_mask: bool) -> Self {
        self.target_mask = target_mask;
        self
    }

    /// The user's navigation options; Photoshop's by default.
    #[must_use]
    pub fn navigation(mut self, navigation: NavigationOptions) -> Self {
        self.navigation = navigation;
        self
    }

    /// The user's transparency grid.
    #[must_use]
    pub fn checkerboard(mut self, checkerboard: Checker) -> Self {
        self.checkerboard = checkerboard;
        self
    }

    /// The user's Preferences › Cursors.
    #[must_use]
    pub fn cursor_options(mut self, cursor_options: BrushCursorOptions) -> Self {
        self.cursor_options = Some(cursor_options);
        self
    }
}

/// What the workspace reports, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SelectMaskEvent {
    /// A tool edit: `Begin`, `Update` with the edit so far (only with Real-Time Refinement on, so
    /// the preview can show the stroke as it is drawn), then `Commit` with the edit, already
    /// recorded in the session, or `Cancel` (refused by the limits, Esc, a lost pointer). Select
    /// Subject, Clear Selection and Invert are `Set`.
    Edit(EditPhase<SelectMaskEdit>),
    /// A Refine Edge Brush stroke, the same way.
    Refine(EditPhase<RefineStroke>),
}

/// What [`SelectMaskWorkspace::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SelectMaskOutput {
    /// What happened this pass, in order.
    pub events: Vec<SelectMaskEvent>,
    /// OK (`Accept`) or Cancel, once chosen.
    pub closed: Option<DialogRole>,
    /// The dialog's response: its rect is the workspace's.
    pub response: Response,
    /// A setting changed this pass.
    pub settings_changed: bool,
    /// The Overlay colour swatch was pressed: open the app's colour picker with this request
    /// (keyed [`OVERLAY_COLOR_KEY`]) and answer with [`SelectMaskWorkspace::set_color`].
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for SelectMaskOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Select and Mask workspace.
///
/// Keys ([`SelectMaskCommands`], on the dialog's scope; none while a text field edits): W, R, B, L, H, Z pick the
/// tools, a pressed-again W or L (or Shift+W, Shift+L) cycles its group; `[` and `]` the brush size
/// ([`BrushSizes::step`]); the app forwards the fired `MaskViewKeys` (O M V A T K Y, F, J, P and X held) to
/// [`mask_view`](Self::mask_view) and ⌘Z, ⇧⌘Z to
/// [`step`](Self::step). Backspace and Delete remove the polygon's last vertex; Enter closes an open polygon first,
/// else is OK; Esc drops it first, else is Cancel.
#[derive(Clone, Pattern)]
pub struct SelectMaskWorkspace {
    session: SelectMaskSession,
    history: EditHistory<SelectMaskSession>,
    /// The session as of the latest history step.
    settled: SelectMaskSession,
    settings: SelectMaskSettings,
    limits: SelectMaskLimits,
    target_mask: bool,
    subject: String,
    navigation: NavigationOptions,
    checkerboard: Checker,
    // View state outside the history.
    view: FullWindowView,
    mask_view: MaskViewState,
    tool: SelectMaskTool,
    last_used: LastUsed<SelectMaskTool>,
    options: SelectMaskToolOptions,
    canvas: SelectMaskCanvas,
    zoom_tool: ZoomTool,
    canvas_id: Option<Id>,
    /// The limit that refused an edit, until the next accepted edit; worded each pass.
    error: Option<LimitRefused>,
    /// A refinement slider's gesture is open.
    panel_open: bool,
    /// The request of the last pass that had no gesture open.
    request: Option<SelectMaskPreviewRequest>,
    /// The key of the request a released brush stroke was made under: the stroke stays drawn until another key's
    /// preview is ready.
    trail_since: Option<String>,
    /// Steps the app asked for while a stroke was open; the next `show` cancels the stroke, then steps.
    requested: Vec<Step>,
    /// The zoom on screen at the end of the last pass, for the title.
    zoom_percent: f64,
    /// This pass's events.
    events: Vec<SelectMaskEvent>,
    pick_color: Option<ColorRequest>,
}

impl SelectMaskWorkspace {
    /// A workspace over `open`.
    pub fn new(open: SelectMaskOpen) -> Self {
        let session = SelectMaskSession { refine: open.settings.refine.clone(), ..SelectMaskSession::default() };
        let mut canvas = SelectMaskCanvas::new();
        if let Some(options) = open.cursor_options {
            canvas.cursor_options(options);
        }
        Self {
            history: EditHistory::new(&session),
            settled: session.clone(),
            session,
            mask_view: open.settings.view_state(),
            settings: open.settings,
            limits: open.limits,
            target_mask: open.target_mask,
            subject: open.subject,
            navigation: open.navigation,
            checkerboard: open.checkerboard,
            view: FullWindowView::new(),
            tool: SelectMaskTool::Quick,
            last_used: LastUsed::default(),
            options: SelectMaskToolOptions::default(),
            canvas,
            zoom_tool: ZoomTool::new(open.navigation),
            canvas_id: None,
            error: None,
            panel_open: false,
            request: None,
            trail_since: None,
            requested: Vec::new(),
            zoom_percent: 100.0,
            events: Vec::new(),
            pick_color: None,
        }
    }

    /// The user's navigation options (wheel, flick, animated zoom); Photoshop's by default.
    pub fn navigation(&mut self, navigation: NavigationOptions) {
        self.navigation = navigation;
    }

    /// The user's transparency grid under the image.
    pub fn checkerboard(&mut self, checkerboard: Checker) {
        self.checkerboard = checkerboard;
    }

    /// The user's Preferences › Cursors, which shape the brush outline.
    pub fn cursor_options(&mut self, cursor_options: BrushCursorOptions) {
        self.canvas.cursor_options(cursor_options);
    }

    /// The app's colour picker answered a [`SelectMaskOutput::pick_color`] request. A key this
    /// workspace did not ask for is ignored.
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        if key == OVERLAY_COLOR_KEY && self.mask_view.overlay_color != color {
            self.mask_view.overlay_color = color;
            self.settings.take_view(&self.mask_view);
        }
    }

    /// Shows the workspace for this pass.
    ///
    /// A [`FullWindowDialog`] titled by [`FullWindowTitle`] (the document and the zoom), in the scope of
    /// [`SelectMaskCommands`], with the Properties column ([`SelectMaskPanel`], the preview's status, Reset at the
    /// footer's leading end, OK and Cancel) in the dialog's side region. The body, in order: the steps
    /// [`step`](Self::step) held back, the commands, [`SelectMaskBar`], the tool strip (a pick cancels the old
    /// tool's gesture with `Removed`), the canvas ([`FullWindowCanvas`]) with the source's view (the image while X
    /// is held), the ants as a `SelectionOverlay` control while the recipe has them, the brush resize preview, and
    /// [`SelectMaskCanvasTool`], the Zoom tool or the canvas's Hand as its tool. Commits from the canvas go through
    /// [`SelectMaskSession::add_edit`] and [`SelectMaskSession::add_stroke`]; a refusal sets the error and reports
    /// `Cancel`. Then the history settles (not while the canvas or a slider drags).
    pub fn show(&mut self, ui: &mut Ui, source: &dyn SelectMaskSource) -> SelectMaskOutput {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1812 `show`, :1375 `keys`, :870
        // `enter`, :881 `escape`.
        let env = Env::of(ui);
        let words = env.words::<SelectMaskWords>();
        let common = env.words::<CommonWords>();
        let title = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);
        let settings_before = self.settings.clone();
        // Steps asked while a stroke was open run before the canvas: an open stroke is cancelled first, so its
        // Commit never lands on the restored session.
        for step in std::mem::take(&mut self.requested) {
            self.cancel_gesture(ui.ctx(), CancelReason::UndoDuringGesture);
            self.canvas.drop_polygon();
            self.step_now(step);
        }
        let slots = SelectMaskTool::slots();
        let request = self.request_now();
        let key = format!("{request:?}");
        let reset = Cell::new(false);
        // What the Properties column decided this run (the last run wins); `settle` applies it.
        let side_status = RefCell::new(None);
        let side_panel_open = Cell::new(None);
        let side_pick = RefCell::new(None);
        let side_edit = RefCell::new(None);
        let ctx = ui.ctx().clone();
        let this = RefCell::new(&mut *self);

        let mut shown = FullWindowDialog::new(title)
            .command_scope(SelectMaskCommands::SCOPE)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .side_footer(SideFooter::bottom().leading(|ui| {
                let button = IconButton::new(IconId::new("undo")).label(words.reset.clone()).id_salt("reset").show(ui);
                reset.set(reset.get() || clicked(&button));
            }))
            .sides(|ui, cx| {
                let mut guard = this.borrow_mut();
                let work: &mut SelectMaskWorkspace = &mut guard;
                let status = cx.pending(key.clone(), || source.preview_job(&request)).clone();
                *side_status.borrow_mut() = Some(status.clone());
                side_panel_open.set(None);
                *side_pick.borrow_mut() = None;
                *side_edit.borrow_mut() = None;
                FullWindowSide::new(words.properties.clone()).show(ui, |ui, _| {
                    let message = work.error.map(|why| why.message(&env));
                    let SelectMaskWorkspace { settings, session, mask_view, .. } = &mut *work;
                    let panel = SelectMaskPanel::new(settings, &mut session.refine, mask_view).error(message.as_deref()).show(ui);
                    side_panel_open.set(match panel.edit.last() {
                        Some(EditEvent::Begin { .. } | EditEvent::Update { .. }) => Some(true),
                        Some(_) => Some(false),
                        None => None,
                    });
                    *side_pick.borrow_mut() = panel.pick_color;
                    *side_edit.borrow_mut() = panel.selection_edit;
                    FullWindowStatus::of(&status).show(ui);
                });
            })
            .show(ui, |ui, cx| {
                let mut guard = this.borrow_mut();
                let work: &mut SelectMaskWorkspace = &mut guard;
                work.zoom_tool.options = work.navigation;

                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    work.view.apply(command);
                }

                while let Some((command, fired)) = cx.take_command::<SelectMaskCommands>() {
                    if let Some(tool) = command.tool() {
                        let slot = slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == tool));
                        let tools: Vec<_> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                        let picked = slot.and_then(|slot| {
                            tool_pick(&tools, slot.id, &work.tool, &work.last_used, PickStep::of(&fired), |_| true).map(|entry| (slot.id, entry.id))
                        });
                        if let Some((slot_id, tool)) = picked {
                            work.pick(&ctx, tool);
                            work.last_used.set(slot_id, tool);
                        }
                        continue;
                    }
                    let larger = command == SelectMaskCommands::Larger;
                    if work.tool.brushes() {
                        work.options.sizes.step(work.tool, larger);
                    }
                }

                let mut subject = None;
                let mut canvas_id = None;
                Stack::new().fill().show(ui, |ui, _| {
                    let bar = SelectMaskBar::new(work.tool, &mut work.options).show(ui);
                    if bar.select_subject {
                        subject = Some(SelectMaskEdit::Subject { sample_all_layers: work.options.sample_all_layers });
                    }
                    let recipe = work.mask_view.recipe();
                    let image: &dyn RasterSource = if recipe.hidden { source.image() } else { source.view() };
                    let ants = source.ants().filter(|_| recipe.ants && !recipe.hidden);
                    let falloff = work.canvas.resize().dimensions().map(|dims| {
                        let radius = dims.diameter / 2.0;
                        if work.tool == SelectMaskTool::Brush { TipFalloff::from_hardness(radius, dims.level) } else { TipFalloff::hard(radius) }
                    });
                    let preview = falloff.as_ref().and_then(|falloff| work.canvas.resize().preview(falloff));

                    let shown = Row::new().fill().show(ui, |ui, _| {
                        let rail = ToolRail::new(words.tools.clone(), &slots, &work.tool, &work.last_used).show_rail(ui, |_, _| {});
                        if let Some(next) = rail.last_used {
                            work.last_used = next;
                        }
                        if let Some(tool) = rail.select {
                            work.pick(&ctx, tool);
                        }
                        let canvas = FullWindowCanvas::new(Bind::bound(&mut work.view), image)
                            .checkerboard(work.checkerboard)
                            .navigation(work.navigation)
                            .controls(|ui, rect| {
                                if let Some((mask, reduction)) = ants {
                                    SelectionOverlay::new("Selection", OutlineSource::Mask(mask)).reduction(reduction).show_at(ui, rect);
                                }
                                if let Some(preview) = &preview {
                                    preview.clone().show_at(ui, rect);
                                }
                            });
                        match work.tool {
                            SelectMaskTool::Hand => canvas.show(ui),
                            SelectMaskTool::Zoom => canvas.tool(&mut work.zoom_tool).show(ui),
                            tool => {
                                let mut brush = work.canvas.tool(tool, &mut work.options, work.settings.real_time);
                                canvas.tool(&mut brush).show(ui)
                            }
                        }
                    });
                    canvas_id = shown.inner.as_ref().map(|canvas| canvas.response.id);
                });
                (subject, canvas_id, work.view.zoom_percent())
            });
        // Once per pass, after the real run: what the columns and the canvas decided is stored and the canvas's
        // events are recorded.
        shown.settle(|_, (subject, canvas_id, zoom_percent)| {
            let work: &mut SelectMaskWorkspace = &mut this.borrow_mut();
            if let Some(status) = side_status.take()
                && work.trail_since.as_ref().is_some_and(|since| *since != key)
                && !matches!(status, Pending::Loading { .. })
            {
                work.canvas.drop_trail();
                work.trail_since = None;
            }
            if let Some(open) = side_panel_open.take() {
                work.panel_open = open;
            }
            work.pick_color = side_pick.take();
            for edit in [side_edit.take(), subject.take()].into_iter().flatten() {
                work.absorb_edit(&ctx, edit);
            }
            work.canvas_id = *canvas_id;
            work.zoom_percent = *zoom_percent;
            work.absorb_canvas();
            work.settle(work.canvas.busy() || work.panel_open);
            work.settings.take_view(&work.mask_view);
            work.settings.refine = work.session.refine.clone();
        });
        let this = this.into_inner();
        let picked = shown.output.picked.and_then(|at| [DialogRole::Accept, DialogRole::Cancel].get(at).copied());
        let closed = picked.or(shown.output.closed.map(|_| DialogRole::Cancel));
        if closed == Some(DialogRole::Accept) && this.canvas.close_polygon() {
            this.absorb_canvas();
        }
        if reset.get() {
            this.canvas.drop_polygon();
            this.session.reset();
            this.error = None;
            this.settle(false);
        }
        SelectMaskOutput {
            events: std::mem::take(&mut this.events),
            closed,
            response: shown.output.response,
            settings_changed: this.settings != settings_before,
            pick_color: this.pick_color.take(),
        }
    }

    /// The edits and strokes recorded so far.
    pub fn session(&self) -> &SelectMaskSession {
        &self.session
    }

    /// The current settings.
    pub fn settings(&self) -> &SelectMaskSettings {
        &self.settings
    }

    /// OK's parameters: [`SelectMaskSession::command`] with the bar's Sample All Layers and the
    /// mask target.
    pub fn command(&self) -> SelectMaskCommand {
        self.session.command(self.options.sample_all_layers, self.target_mask)
    }

    /// What the engine composites for the current view (`MaskViewState::recipe`).
    pub fn recipe(&self) -> MaskViewRecipe {
        self.mask_view.recipe()
    }

    /// Undo or redo from the app's Edit menu (`EditCommands::Undo`, `Redo`). Returns whether it stepped. With a stroke
    /// open the step waits for the next pass, which cancels the stroke (`UndoDuringGesture`) first.
    pub fn step(&mut self, step: Step) -> bool {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:644 `undo`, :672 `redo`.
        if self.canvas.busy() {
            self.requested.push(step);
            return match step {
                Step::Undo => self.history.can_undo(),
                _ => self.history.can_redo(),
            };
        }
        self.canvas.drop_polygon();
        self.step_now(step)
    }

    /// Runs a fired view command (`MaskViewKeys`: the views O M V A T K Y, F, J, P, and X held) on the view state.
    /// Returns whether it changed the view. The app calls it with each fired command and its phase.
    pub fn mask_view(&mut self, command: MaskViewKeys, phase: FirePhase) -> bool {
        self.mask_view.run(command, phase).is_some()
    }
}

impl SelectMaskWorkspace {
    /// The request for the preview now: the state as of this pass, or the last idle one while a gesture is open.
    fn request_now(&mut self) -> SelectMaskPreviewRequest {
        let busy = self.canvas.busy() || self.panel_open;
        if busy && let Some(held) = &self.request {
            return held.clone();
        }
        let request = SelectMaskPreviewRequest {
            session: self.session.clone(),
            recipe: MaskViewRecipe { hidden: false, ..self.mask_view.recipe() },
            high_quality: self.settings.high_quality,
            sample_all_layers: self.options.sample_all_layers,
        };
        self.request = Some(request.clone());
        request
    }

    /// Records the session as one history step when it differs from the last step and no gesture is open.
    fn settle(&mut self, busy: bool) {
        if busy || self.session == self.settled {
            return;
        }
        let name: Arc<str> = Arc::from(HISTORY_NAME);
        let before = std::mem::replace(&mut self.settled, self.session.clone());
        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name, None));
    }

    /// Steps the history now.
    fn step_now(&mut self, step: Step) -> bool {
        let moved = self.history.step(&mut self.session, step);
        if moved {
            self.error = None;
            self.settings.refine = self.session.refine.clone();
            self.settled = self.session.clone();
        }
        moved
    }

    /// Records a button's edit (Select Subject, Clear Selection, Invert) as a `Set`.
    fn absorb_edit(&mut self, ctx: &egui::Context, edit: SelectMaskEdit) {
        match self.session.add_edit(edit.clone(), &self.limits) {
            Ok(()) => {
                self.error = None;
                let id = next_gesture_id(ctx);
                self.events.push(SelectMaskEvent::Edit(EditPhase::Set { id, value: edit }));
            }
            Err(why) => self.error = Some(why),
        }
    }

    /// Takes the canvas's events: a commit is recorded in the session within the limits, or refused (the message
    /// shows under the panel, and the event becomes a `Cancel`).
    fn absorb_canvas(&mut self) {
        for event in self.canvas.take_events() {
            let event = match event {
                SelectMaskEvent::Edit(EditPhase::Commit { id, value }) => {
                    let recorded = self.session.add_edit(value.clone(), &self.limits).map_err(|why| self.error = Some(why)).is_ok();
                    SelectMaskEvent::Edit(self.settle_event(recorded, id, value))
                }
                SelectMaskEvent::Refine(EditPhase::Commit { id, value }) => {
                    let recorded = self.session.add_stroke(value.clone(), &self.limits).map_err(|why| self.error = Some(why)).is_ok();
                    SelectMaskEvent::Refine(self.settle_event(recorded, id, value))
                }
                other => other,
            };
            self.events.push(event);
        }
    }

    /// `Commit` when it was recorded, else the refusal as a `Cancel`.
    fn settle_event<T>(&mut self, recorded: bool, id: GestureId, value: T) -> EditPhase<T> {
        if recorded {
            self.error = None;
            if self.canvas.trail_shown() {
                self.trail_since = self.request.as_ref().map(|request| format!("{request:?}"));
            }
            EditPhase::Commit { id, value }
        } else {
            EditPhase::Cancel { id, reason: CancelReason::Refused }
        }
    }

    /// Cancels the gesture the current tool holds (a brush stroke, an outline, a resize or a Zoom drag) through the
    /// host, and the tool's own state if the host held none.
    fn cancel_gesture(&mut self, ctx: &egui::Context, reason: CancelReason) {
        let tool = self.tool;
        let real_time = self.settings.real_time;
        if let Some(id) = self.canvas_id {
            match tool {
                SelectMaskTool::Hand => false,
                SelectMaskTool::Zoom => ToolHost::cancel(ctx, id, &mut self.zoom_tool, reason),
                _ => ToolHost::cancel(ctx, id, &mut self.canvas.tool(tool, &mut self.options, real_time), reason),
            };
        }
        if self.canvas.busy() && tool.edits() {
            self.canvas.tool(tool, &mut self.options, real_time).abandon(reason);
        }
        self.absorb_canvas();
    }

    /// Switches to `tool`, cancelling the old tool's gesture and dropping an open polygon first.
    fn pick(&mut self, ctx: &egui::Context, tool: SelectMaskTool) {
        if tool != self.tool {
            self.cancel_gesture(ctx, CancelReason::Removed);
            self.canvas.drop_polygon();
            self.tool = tool;
        }
    }
}

/// The name the history gives its steps.
const HISTORY_NAME: &str = "Select and Mask";
