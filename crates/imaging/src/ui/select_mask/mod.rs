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
//! under it. The workspace is a [`FullWindowWorkspace`] that the app's dialog host owns as a
//! `HostedWorkspace`: OK answers [`SelectMaskDone`].
//!
//! ```ignore
//! dialogs.open(HostedWorkspace::new(SelectMaskWorkspace::new(open), Arc::new(source)));
//! dialogs.run(ui);
//! if let Some(select_mask) = dialogs.get_mut::<HostedWorkspace<SelectMaskWorkspace>>() {
//!     for event in select_mask.take_events() {
//!         if let SelectMaskEvent::PickColor(request) = event { app.host.request_color(request) }
//!     }
//!     select_mask.set_source(Arc::new(next_source));
//! }
//! if let Some(DialogResult::Ok(done)) = dialogs.take::<HostedWorkspace<SelectMaskWorkspace>>() {
//!     app.run("select.refineEdge", done.command);
//! }
//! ```
//!
//! The workspace remembers its settings under a [`SaveKey`] ([`SelectMaskOpen::remember`]) when OK is pressed.
//!
//! Files: `session` and `edits`; `tools`, `commands` and `bar`; `panel` and this file's
//! `show` and `step`.
//!
//! [`EditHistory`]: craft_behavior::EditHistory
//! [`FullWindowWorkspace`]: craft_ui::dialog::FullWindowWorkspace

pub(crate) mod bar;
mod commands;
mod edits;
pub(crate) mod panel;
mod session;
#[cfg(feature = "suite")]
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

use egui::{Context, Id, Ui};
use serde::Deserialize;

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, EditEvent, EditHistory, GestureId, IconId, Mixed};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::canvas::{NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, ZoomTool, tool_pick};
use craft_ui::canvas_controls::brush::{BrushCursorOptions, TipFalloff};
use craft_ui::contract::{Bind, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, Tracked, clicked};
use craft_ui::dialog::{
    FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, FullWindowWorkspace,
    SideFooter, WorkspaceCx, WorkspacePass, serde_fields, set_serde_field,
};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::Checker;
use craft_ui::rails::{LastUsed, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase};
use craft_ui::runtime::edit::next_gesture_id;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft_ui::runtime::pending::{Job, Pending};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};

use crate::trace::MaskSource;
use crate::ui::mask_view::{MaskViewKeys, MaskViewRecipe, MaskViewState};
use crate::ui::selection::{OutlineSource, SelectionOverlay};
use crate::ui::wide_angle::PreviewDone;

/// What the app gives the workspace to show each pass.
pub trait SelectMaskSource: Send + Sync {
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
    /// Where the settings are remembered, when the app asks for it.
    pub remember: Option<SaveKey>,
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
            remember: None,
        }
    }

    /// Remembers the settings under `key`: OK stores them, and the stored settings apply at open only when their
    /// Remember Settings flag is on (they then replace [`settings`](Self::settings)).
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.remember = Some(key);
        self
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
    /// The Overlay colour swatch was pressed: the app opens its colour picker (the request is keyed
    /// [`OVERLAY_COLOR_KEY`]) and answers with [`SelectMaskWorkspace::set_color`].
    PickColor(ColorRequest),
}

/// What OK answers: the selection command and the settings.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SelectMaskDone {
    /// The session's edits as the command the app runs.
    pub command: SelectMaskCommand,
    /// The settings as they were at OK.
    pub settings: SelectMaskSettings,
}

/// An edit the control requests make of the workspace through [`SelectMaskWorkspace::perform`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SelectMaskOp {
    /// A tool edit.
    Edit(SelectMaskEdit),
    /// A Refine Edge Brush stroke.
    Refine(RefineStroke),
}

/// The stroke being drawn, as the canvas reported it last: what Real-Time Refinement previews.
#[derive(Clone, Debug, PartialEq)]
enum Drawn {
    Edit(SelectMaskEdit),
    Refine(RefineStroke),
}

/// When a released brush stroke's trail stops showing.
#[derive(Clone, Debug, PartialEq)]
enum TrailSince {
    /// A preview under another request key than this one is ready.
    Key(String),
    /// The first preview that is not running, whatever its key (the request already held the stroke).
    NextIdle,
}

impl TrailSince {
    /// Whether a ready preview of the request `key` ends the trail.
    fn is_over(&self, key: &str) -> bool {
        match self {
            TrailSince::Key(since) => since != key,
            TrailSince::NextIdle => true,
        }
    }
}

/// The Select and Mask workspace.
///
/// Keys ([`SelectMaskCommands`], on the dialog's scope; none while a text field edits): W, R, B, L, H, Z pick the
/// tools, a pressed-again W or L (or Shift+W, Shift+L) cycles its group; `[` and `]` the brush size
/// ([`BrushSizes::step`]); `MaskViewKeys` pick the view (O M V A T K Y, F, J, P, and X held hides it); ⌘Z and ⇧⌘Z
/// step the history (a field being edited steps its own text first; an open stroke is cancelled first). Backspace and Delete remove the polygon's last vertex; Enter closes an open polygon first,
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
    last_used: Tracked<LastUsed<SelectMaskTool>>,
    slots: Tracked<Vec<ToolSlot<SelectMaskTool>>>,
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
    trail_since: Option<TrailSince>,
    /// The zoom on screen at the end of the last pass, for the title.
    zoom_percent: f64,
    /// The events the next pass reports first: what the methods between passes queued, then the pass's own.
    events: Vec<SelectMaskEvent>,
    /// Edits `perform` recorded between passes: the next pass reports each as a `Set` (its gesture id needs the
    /// context).
    owed: Vec<SelectMaskOp>,
    /// The stroke being drawn, while the canvas reports it (`Begin`, `Update`).
    drawing: Option<Drawn>,
    /// The last pass's preview job was still running.
    previewing: bool,
    /// The settings as the last pass or method left them: a difference means the app edited them through
    /// `settings_mut` or a control request, and the view and the refinement follow.
    synced: SelectMaskSettings,
    /// Where OK stores the settings.
    remember: Option<SaveKey>,
    /// The stored settings were read (the first pass reads them: the context is not at hand in `new`).
    recalled: bool,
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
            synced: open.settings.clone(),
            settings: open.settings,
            limits: open.limits,
            target_mask: open.target_mask,
            subject: open.subject,
            navigation: open.navigation,
            checkerboard: open.checkerboard,
            view: FullWindowView::new(),
            tool: SelectMaskTool::Quick,
            last_used: Tracked::new(LastUsed::default()),
            slots: Tracked::new(SelectMaskTool::slots()),
            options: SelectMaskToolOptions::default(),
            canvas,
            zoom_tool: ZoomTool::new(open.navigation),
            canvas_id: None,
            error: None,
            panel_open: false,
            request: None,
            trail_since: None,
            zoom_percent: 100.0,
            events: Vec::new(),
            owed: Vec::new(),
            drawing: None,
            previewing: false,
            remember: open.remember,
            recalled: false,
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

    /// The settings, for the control requests that change them. The next pass applies a change to the view and the
    /// refinement values; Real-Time Refinement, High Quality Preview and Remember Settings are read where they are.
    pub fn settings_mut(&mut self) -> &mut SelectMaskSettings {
        &mut self.settings
    }

    /// Records `op` in the session as the user would; the next pass reports it as a `Set` event and records the
    /// history step. `Err` when a limit refuses it: nothing is recorded and the panel shows the limit. The caller asks
    /// for the repaint that runs the next pass (this method has no context).
    pub fn perform(&mut self, op: SelectMaskOp) -> Result<(), LimitRefused> {
        let recorded = match &op {
            SelectMaskOp::Edit(edit) => self.session.add_edit(edit.clone(), &self.limits),
            SelectMaskOp::Refine(stroke) => self.session.add_stroke(stroke.clone(), &self.limits),
        };
        match recorded {
            Ok(()) => {
                self.error = None;
                self.owed.push(op);
                self.settle(false);
                Ok(())
            }
            Err(why) => {
                self.error = Some(why);
                Err(why)
            }
        }
    }

    /// Undoes the last edit, abandoning a stroke still open first. The session changes at once.
    pub fn undo(&mut self) {
        self.step(None, Step::Undo);
    }

    /// Redoes the last undone edit, abandoning a stroke still open first.
    pub fn redo(&mut self) {
        self.step(None, Step::Redo);
    }

    /// Resets the session: no edits, no strokes, the default refinement values, as one history step. The app reads the
    /// session after the next pass.
    pub fn reset(&mut self) {
        self.canvas.drop_polygon();
        self.drawing = None;
        self.owed.clear();
        self.session.reset();
        self.error = None;
        self.follow_session_refine();
        self.settle(false);
    }

    /// Picks the canvas tool.
    pub fn set_tool(&mut self, tool: SelectMaskTool) {
        self.pick(None, tool);
    }

    /// Sets Add or Subtract.
    pub fn set_mode(&mut self, mode: EditMode) {
        self.options.add = mode == EditMode::Add;
    }

    /// Sets Sample All Layers.
    pub fn set_sample_all_layers(&mut self, on: bool) {
        self.options.sample_all_layers = on;
    }

    /// Sets Hide View, as X held does.
    pub fn set_hide_view(&mut self, on: bool) {
        self.mask_view.hide_view = on;
    }

    /// The app's colour picker answered a [`SelectMaskEvent::PickColor`] request. A key this
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
    /// footer's leading end, OK and Cancel) in the dialog's side region. The body, in order: Undo and
    /// Redo, the commands, [`SelectMaskBar`], the tool strip (a pick cancels the old
    /// tool's gesture with `Removed`), the canvas ([`FullWindowCanvas`]) with the source's view (the image while X
    /// is held), the ants as a `SelectionOverlay` control while the recipe has them, the brush resize preview, and
    /// [`SelectMaskCanvasTool`], the Zoom tool or the canvas's Hand as its tool. Commits from the canvas go through
    /// [`SelectMaskSession::add_edit`] and [`SelectMaskSession::add_stroke`]; a refusal sets the error and reports
    /// `Cancel`. Then the history settles (not while the canvas or a slider drags).
    ///
    /// The events of the methods that ran between passes come first. A second pass of a frame reports only what its
    /// own input did, since the first pass took the frame's steps and commands. OK closes an open polygon first, so the
    /// result carries it.
    fn pass(
        &mut self,
        ui: &mut Ui,
        source: &dyn SelectMaskSource,
        revision: u64,
        open: Bind<'_, bool>,
    ) -> WorkspacePass<SelectMaskEvent, SelectMaskDone> {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1812 `show`, :1375 `keys`, :870
        // `enter`, :881 `escape`.
        let env = Env::of(ui);
        let words = env.words::<SelectMaskWords>();
        let common = env.words::<CommonWords>();
        let title = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);
        self.start_pass(ui.ctx());
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
            .opened(open)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .side_footer(SideFooter::bottom().leading(|ui| {
                let button = IconButton::new(IconId::new("undo")).label(words.reset.clone()).id_salt("reset").show(ui);
                reset.set(reset.get() || clicked(&button));
            }))
            .sides(|ui, cx| {
                let mut guard = this.borrow_mut();
                let work: &mut SelectMaskWorkspace = &mut guard;
                let status = cx.pending((revision, key.clone()), || source.preview_job(&request)).clone();
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

                // Before the canvas runs: an open stroke is cancelled first, so its Commit never lands on the restored
                // session.
                while let Some(step) = cx.take_step(work.history.can_undo(), work.history.can_redo()) {
                    work.step(Some(&ctx), step);
                }
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    work.view.apply(command);
                }
                while let Some((command, fired)) = cx.take_command::<MaskViewKeys>() {
                    work.mask_view.run(command, fired.phase);
                }

                while let Some((command, fired)) = cx.take_command::<SelectMaskCommands>() {
                    if let Some(tool) = command.tool() {
                        // A tool key's release picks nothing.
                        if fired.phase == FirePhase::Release {
                            continue;
                        }
                        let slot = work.slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == tool));
                        let tools: Vec<_> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                        let picked = slot.and_then(|slot| {
                            tool_pick(&tools, slot.id, &work.tool, &work.last_used, PickStep::of(&fired), |_| true).map(|entry| (slot.id, entry.id))
                        });
                        if let Some((slot_id, tool)) = picked {
                            work.pick(Some(&ctx), tool);
                            work.last_used.set(slot_id, tool);
                        }
                        continue;
                    }
                    match command {
                        SelectMaskCommands::Smaller | SelectMaskCommands::Larger if work.tool.brushes() => {
                            work.options.sizes.step(work.tool, command == SelectMaskCommands::Larger);
                        }
                        // A polygon in progress keeps the mode it started with (`polygon_add`).
                        SelectMaskCommands::ToggleMode if work.tool.edits() => work.options.add = !work.options.add,
                        _ => {}
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
                        let rail = ToolRail::new(words.tools.clone(), &work.slots, &work.tool, &work.last_used).show_column(ui, |_, _| {});
                        if let Some(next) = rail.last_used {
                            *work.last_used = next;
                        }
                        if let Some(tool) = rail.select {
                            work.pick(Some(&ctx), tool);
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
            if let Some(status) = side_status.take() {
                work.previewing = matches!(status, Pending::Loading { .. });
                if !work.previewing && work.trail_since.as_ref().is_some_and(|since| since.is_over(&key)) {
                    work.canvas.drop_trail();
                    work.trail_since = None;
                }
            }
            if let Some(open) = side_panel_open.take() {
                work.panel_open = open;
            }
            if let Some(request) = side_pick.take() {
                work.events.push(SelectMaskEvent::PickColor(request));
            }
            for edit in [side_edit.take(), subject.take()].into_iter().flatten() {
                work.absorb_edit(&ctx, edit);
            }
            work.canvas_id = *canvas_id;
            work.zoom_percent = *zoom_percent;
            work.absorb_canvas();
            work.settle(work.canvas.busy() || work.panel_open);
            work.settings.take_view(&work.mask_view);
            work.settings.refine = work.session.refine.clone();
            work.synced = work.settings.clone();
        });
        let this = this.into_inner();
        if reset.get() {
            this.reset();
        }
        let result = shown.output.result(|| {
            if this.canvas.close_polygon() {
                this.absorb_canvas();
            }
            this.done(&ctx)
        });
        let events = std::mem::take(&mut this.events);
        WorkspacePass::new(shown.output.response, events).result(result).canvas(this.canvas_id)
    }

    /// The start of a pass: the first pass reads the stored settings, a change the app made to the settings since the
    /// last pass reaches the view and the refinement values, and the edits `perform` recorded are reported.
    fn start_pass(&mut self, ctx: &Context) {
        if !std::mem::replace(&mut self.recalled, true)
            && let Some(key) = &self.remember
        {
            let stored: SelectMaskSettings = saved_value(ctx, key);
            if stored.remember {
                // The stored refinement values are the session's starting point, not a step to undo.
                self.settings = stored;
                self.session.refine = self.settings.refine.clone();
                self.history = EditHistory::new(&self.session);
                self.settled = self.session.clone();
            }
        }
        if self.settings != self.synced {
            let hidden = self.mask_view.hide_view;
            self.mask_view = self.settings.view_state();
            self.mask_view.hide_view = hidden;
            self.session.refine = self.settings.refine.clone();
            self.synced = self.settings.clone();
        }
        for op in std::mem::take(&mut self.owed) {
            let id = next_gesture_id(ctx);
            self.events.push(match op {
                SelectMaskOp::Edit(value) => SelectMaskEvent::Edit(EditPhase::Set { id, value }),
                SelectMaskOp::Refine(value) => SelectMaskEvent::Refine(EditPhase::Set { id, value }),
            });
        }
    }

    /// OK's answer; the settings are stored under the key the app gave, if any.
    fn done(&self, ctx: &Context) -> SelectMaskDone {
        if let Some(key) = &self.remember {
            set_saved_value(ctx, key, self.settings.clone());
        }
        SelectMaskDone { command: self.command(), settings: self.settings.clone() }
    }

    /// Mirrors the session's refinement values into the settings after the session changed outside a pass.
    fn follow_session_refine(&mut self) {
        self.settings.refine = self.session.refine.clone();
        self.synced.refine = self.settings.refine.clone();
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
}

impl SelectMaskWorkspace {
    /// The request for the preview now: the state as of this pass. With Real-Time Refinement on, a stroke being drawn
    /// joins the session of the request, so drawing and idle share one preview path; a request made while the
    /// preview of the last one still runs waits for it (the last request stands). With it off, an open stroke holds the
    /// last idle request, as does a slider drag.
    fn request_now(&mut self) -> SelectMaskPreviewRequest {
        let drawing = self.drawing.clone().filter(|_| self.settings.real_time);
        let hold = match drawing {
            Some(_) => self.panel_open || self.previewing,
            None => self.canvas.busy() || self.panel_open,
        };
        if hold && let Some(held) = &self.request {
            return held.clone();
        }
        let mut session = self.session.clone();
        match drawing {
            Some(Drawn::Edit(edit)) => session.edits.push(edit),
            Some(Drawn::Refine(stroke)) => session.strokes.push(stroke),
            None => {}
        }
        let request = SelectMaskPreviewRequest {
            session,
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

    /// Undoes or redoes, cancelling an open stroke (`UndoDuringGesture`) and dropping an open polygon first.
    fn step(&mut self, ctx: Option<&Context>, step: Step) {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:644 `undo`, :672 `redo`.
        self.cancel_gesture(ctx, CancelReason::UndoDuringGesture);
        self.canvas.drop_polygon();
        if self.history.step(&mut self.session, step) {
            self.error = None;
            self.follow_session_refine();
            self.settled = self.session.clone();
        }
    }

    /// Records a button's edit (Select Subject, Clear Selection, Invert) as a `Set`.
    fn absorb_edit(&mut self, ctx: &Context, edit: SelectMaskEdit) {
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
            self.track_drawing(&event);
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

    /// Keeps the stroke being drawn for Real-Time Refinement's request: what `Begin` and `Update` carry, until the
    /// gesture ends.
    fn track_drawing(&mut self, event: &SelectMaskEvent) {
        match event {
            SelectMaskEvent::Edit(EditPhase::Begin { value, .. }) => self.drawing = value.clone().map(Drawn::Edit),
            SelectMaskEvent::Edit(EditPhase::Update { value, .. }) => self.drawing = Some(Drawn::Edit(value.clone())),
            SelectMaskEvent::Refine(EditPhase::Begin { value, .. }) => self.drawing = value.clone().map(Drawn::Refine),
            SelectMaskEvent::Refine(EditPhase::Update { value, .. }) => self.drawing = Some(Drawn::Refine(value.clone())),
            SelectMaskEvent::Edit(_) | SelectMaskEvent::Refine(_) => self.drawing = None,
            SelectMaskEvent::PickColor(_) => {}
        }
    }

    /// `Commit` when it was recorded, else the refusal as a `Cancel`. The stroke stays drawn until the preview of the
    /// session that holds it is ready: with Real-Time Refinement on, the request already held the stroke, so the
    /// trail waits for the first preview that is not running, whatever its key.
    fn settle_event<T>(&mut self, recorded: bool, id: GestureId, value: T) -> EditPhase<T> {
        if recorded {
            self.error = None;
            if self.canvas.trail_shown() {
                self.trail_since = if self.settings.real_time {
                    Some(TrailSince::NextIdle)
                } else {
                    self.request.as_ref().map(|request| TrailSince::Key(format!("{request:?}")))
                };
            }
            EditPhase::Commit { id, value }
        } else {
            EditPhase::Cancel { id, reason: CancelReason::Refused }
        }
    }

    /// Cancels the gesture the current tool holds (a brush stroke, an outline, a resize or a Zoom drag) through the
    /// host, and the tool's own state if the host held none. Without a context (a request between passes) only the
    /// tool's own state is dropped.
    fn cancel_gesture(&mut self, ctx: Option<&Context>, reason: CancelReason) {
        let tool = self.tool;
        let real_time = self.settings.real_time;
        if let (Some(ctx), Some(id)) = (ctx, self.canvas_id) {
            match tool {
                SelectMaskTool::Hand => false,
                SelectMaskTool::Zoom => ToolHost::new().cancel(ctx, id, &mut self.zoom_tool, reason),
                _ => ToolHost::new().cancel(ctx, id, &mut self.canvas.tool(tool, &mut self.options, real_time), reason),
            };
        }
        if self.canvas.busy() && tool.edits() {
            self.canvas.tool(tool, &mut self.options, real_time).abandon(reason);
        }
        self.absorb_canvas();
    }

    /// Switches to `tool`, cancelling the old tool's gesture and dropping an open polygon first.
    fn pick(&mut self, ctx: Option<&Context>, tool: SelectMaskTool) {
        if tool != self.tool {
            self.cancel_gesture(ctx, CancelReason::Removed);
            self.canvas.drop_polygon();
            self.tool = tool;
        }
    }
}

/// The name the history gives its steps.
const HISTORY_NAME: &str = "Select and Mask";

/// The name the automation surface gives `tool`.
fn tool_key(tool: SelectMaskTool) -> &'static str {
    match tool {
        SelectMaskTool::Quick => "quick",
        SelectMaskTool::RefineEdge => "refineEdge",
        SelectMaskTool::Brush => "brush",
        SelectMaskTool::Object => "object",
        SelectMaskTool::Lasso => "lasso",
        SelectMaskTool::PolygonalLasso => "polygonalLasso",
        SelectMaskTool::Hand => "hand",
        SelectMaskTool::Zoom => "zoom",
    }
}

/// The value of the `stroke` action key.
#[derive(Deserialize)]
struct StrokeArg {
    points: Vec<[f64; 2]>,
    #[serde(default)]
    alt: bool,
}

/// The workspace on the app's dialog host. OK answers at once with the command and the settings.
impl FullWindowWorkspace for SelectMaskWorkspace {
    type Source = dyn SelectMaskSource;
    type Event = SelectMaskEvent;
    type Result = SelectMaskDone;

    const KEY: &'static str = "select.selectAndMask";

    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<Self::Event, Self::Result> {
        let WorkspaceCx { source, revision, open, .. } = cx;
        self.pass(ui, source, revision, open)
    }

    /// The settings, and the bar's `tool`, `mode` (`"add"`, `"subtract"`), `size` (of the current tool), `hardness`,
    /// `sampleAllLayers`, and `hideView`.
    fn json_fields(&self, _source: &Self::Source) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.settings)?;
        let bar = fields.as_object_mut()?;
        let mode = if self.options.add { "add" } else { "subtract" };
        bar.insert("tool".to_owned(), tool_key(self.tool).into());
        bar.insert("mode".to_owned(), mode.into());
        bar.insert("size".to_owned(), self.options.sizes.get(self.tool).into());
        bar.insert("hardness".to_owned(), self.options.hardness.into());
        bar.insert("sampleAllLayers".to_owned(), self.options.sample_all_layers.into());
        bar.insert("hideView".to_owned(), self.mask_view.hide_view.into());
        Some(fields)
    }

    /// A settings key, a bar key of [`json_fields`](Self::json_fields), or an action: `stroke` (`{points, alt?}`: the
    /// current tool's edit, with Alt flipping the mode), `selectSubject`, `clearSelection`, `invert`, `reset`, `undo`
    /// and `redo` (each `true`). An edit goes through the limits like the canvas's; one a limit refuses is rejected
    /// and the panel shows the limit.
    fn set_json_field(&mut self, ctx: &Context, _source: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        let rejects = |value: &serde_json::Value| FieldRefusal::Rejects(value).message(field);
        let on = value == serde_json::Value::Bool(true);
        match field {
            "tool" => {
                let tool = value.as_str().and_then(|key| SelectMaskTool::ALL.into_iter().find(|tool| tool_key(*tool) == key));
                self.pick(Some(ctx), tool.ok_or_else(|| rejects(&value))?);
            }
            "mode" => self.set_mode(serde_json::from_value::<EditMode>(value.clone()).map_err(|_| rejects(&value))?),
            "size" => self.options.sizes.set(self.tool, value.as_f64().ok_or_else(|| rejects(&value))?),
            "hardness" => self.options.hardness = value.as_f64().ok_or_else(|| rejects(&value))?.clamp(0.0, 100.0),
            "sampleAllLayers" => self.set_sample_all_layers(value.as_bool().ok_or_else(|| rejects(&value))?),
            "hideView" => self.set_hide_view(value.as_bool().ok_or_else(|| rejects(&value))?),
            "stroke" => {
                let arg = serde_json::from_value::<StrokeArg>(value.clone()).map_err(|_| rejects(&value))?;
                if arg.points.is_empty() || !arg.points.iter().flatten().all(|coordinate| coordinate.is_finite()) {
                    return Err(rejects(&value));
                }
                let points: Vec<[f64; 2]> = arg.points.into_iter().map(round_point).collect();
                let mode = if self.options.add != arg.alt { EditMode::Add } else { EditMode::Subtract };
                let (sizes, sample_all_layers) = (self.options.sizes, self.options.sample_all_layers);
                let op = match self.tool {
                    SelectMaskTool::Quick => SelectMaskOp::Edit(SelectMaskEdit::quick(points, sizes.quick, mode, sample_all_layers)),
                    SelectMaskTool::Brush => SelectMaskOp::Edit(SelectMaskEdit::brush(points, sizes.brush, self.options.hardness, mode)),
                    SelectMaskTool::Lasso | SelectMaskTool::PolygonalLasso if points.len() >= 3 => {
                        SelectMaskOp::Edit(SelectMaskEdit::lasso(points, mode))
                    }
                    SelectMaskTool::RefineEdge => SelectMaskOp::Refine(RefineStroke::new(points, sizes.refine, mode)),
                    _ => return Err(rejects(&value)),
                };
                self.perform(op).map_err(|why| why.message(&Env::of_root(ctx)))?;
            }
            "selectSubject" | "clearSelection" | "invert" | "reset" | "undo" | "redo" => {
                if !on {
                    return Err(rejects(&value));
                }
                let edit = match field {
                    "selectSubject" => Some(SelectMaskEdit::subject(self.options.sample_all_layers)),
                    "clearSelection" => Some(SelectMaskEdit::Clear),
                    "invert" => Some(SelectMaskEdit::Invert),
                    _ => None,
                };
                match (edit, field) {
                    (Some(edit), _) => self.perform(SelectMaskOp::Edit(edit)).map_err(|why| why.message(&Env::of_root(ctx)))?,
                    (None, "reset") => self.reset(),
                    (None, "undo") => self.undo(),
                    (None, _) => self.redo(),
                }
            }
            _ => set_serde_field(&mut self.settings, field, value)?,
        }
        Ok(())
    }

    fn confirm(&mut self, ctx: &Context, _source: &Self::Source) -> Option<Self::Result> {
        if self.canvas.close_polygon() {
            self.absorb_canvas();
        }
        Some(self.done(ctx))
    }
}
