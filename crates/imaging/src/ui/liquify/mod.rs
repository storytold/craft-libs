//! The Liquify workspace (Filter › Liquify…), in Photoshop's layout.
//!
//! [`LiquifyWorkspace`] is a [`FullWindowDialog`] with the title strip "Liquify (layer, 66%)", the
//! tool strip at the start edge (the eleven Liquify tools in three groups, then Hand and Zoom), the
//! canvas, and one Standard [`FullWindowSide`] at the end edge: the Properties panel (Brush Tool
//! Options, Brush Reconstruct Options, Mask Options, View Options) with OK and Cancel at its
//! bottom. No options bar.
//!
//! The library holds the session as plain data: the strokes and whole-field operations in the order
//! the engine replays them ([`LiquifySession`]), with [`EditHistory`] undo. The app owns the
//! displacement field and its preview: it applies each [`LiquifyEvent`] to the field as it comes,
//! replays the list after undo, redo and Restore All, and shows the result through
//! [`LiquifySource`]. The workspace is a [`FullWindowWorkspace`] that the app's dialog host owns as a
//! `HostedWorkspace`: OK answers [`LiquifyDone`] (the steps `filter.liquify` runs and the settings);
//! Cancel changes nothing. With [`LiquifyWorkspace::remember`] the workspace stores its settings itself.
//!
//! ```ignore
//! dialogs.open(HostedWorkspace::new(LiquifyWorkspace::new(open).remember(SETTINGS), Arc::new(source)));
//! dialogs.run(ui);
//! if let Some(liquify) = dialogs.get_mut::<HostedWorkspace<LiquifyWorkspace>>() {
//!     for event in liquify.take_events() {
//!         match event {
//!             LiquifyEvent::Stroke(e) => app.liquify_engine.stroke(e),
//!             LiquifyEvent::Global(op) => app.liquify_engine.global(op),
//!             LiquifyEvent::Replay(steps) => app.liquify_engine.replay(&steps),
//!             LiquifyEvent::BackdropWanted(of) => app.load_backdrop(of),
//!             LiquifyEvent::Preview(on) => app.liquify_engine.show_preview(on),
//!             _ => {}
//!         }
//!     }
//! }
//! if let Some(DialogResult::Ok(done)) = dialogs.take::<HostedWorkspace<LiquifyWorkspace>>() {
//!     app.run("filter.liquify", done.steps);
//! }
//! ```
//!
//! Files: `session` (the steps, the settings), `tools` (the tools and the stroke
//! recorder), `commands` (the workspace keys), `panel` (the Properties panel),
//! `canvas` (the canvas tool, the backdrop, the mask and mesh, the brush resize), this file
//! (`show`, `step`).
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`FullWindowWorkspace`]: craft_ui::dialog::FullWindowWorkspace
//! [`FullWindowSide`]: craft_ui::dialog::FullWindowSide
//! [`EditHistory`]: craft_behavior::EditHistory

pub(crate) mod canvas;
mod commands;
pub(crate) mod panel;
mod session;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod tools;
mod words;

pub use canvas::{
    BackdropLayering, LassoLine, LassoLineStyle, LassoTone, LayeredImage, LiquifyCanvas, LiquifyCanvasTool, LiquifyMesh, LiquifyTiming,
};
pub use commands::LiquifyCommands;
pub use panel::{LiquifyPanel, LiquifyPanelOutput};
pub use session::{BackdropMode, BackdropUse, LiquifyGlobal, LiquifySession, LiquifySettings, LiquifyStep, LiquifyStroke, MeshSize, StrokeEvent};
pub use tools::{LiquifyBrush, LiquifyTool, StrokeRecorder};
pub use words::LiquifyWords;

use std::cell::RefCell;
use std::sync::Arc;

use egui::{Color32, Context, Id, Ui};

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, Cursor, EditEvent, EditHistory, Mixed};
use craft_ui::canvas::{NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, tool_pick};
use craft_ui::canvas_controls::brush::{BrushCursorOptions, TipFalloff};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Pattern, Props as _, Show as _, ShowContainer as _, Tracked};
use craft_ui::dialog::{
    FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowTitle, FullWindowView, FullWindowWorkspace, RasterOverlay,
    WorkspaceCx, WorkspacePass, serde_fields, set_serde_field,
};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::Row;
use craft_ui::primitives::Checker;
use craft_ui::rails::{LastUsed, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};

use canvas::MaskAtTexel;
use panel::backdrop_id;

use crate::ui::warp::MeshLines;

/// Photoshop's default Mask Color for Liquify's freeze mask: red at half opacity. The mask source is white with the
/// frozen amount as alpha, so the tint carries the colour. It is data the user's Mask Color setting replaces, not a
/// theme token.
const FREEZE_MASK_TINT: Color32 = Color32::from_rgba_premultiplied(0x80, 0x00, 0x00, 0x80);

/// What the app gives the workspace to show each pass: images and names only. The field, the
/// proxy and the backdrop render stay the app's (PhotoCraft's `photocraft_algo::liquify`).
pub trait LiquifySource: Send + Sync {
    /// The liquified layer as the field shows it now (the app's proxy, or its full-resolution crop
    /// above 100 %), in document pixels.
    fn picture(&self) -> &dyn RasterSource;

    /// The freeze mask, white with the frozen amount as alpha, and its texel size in document
    /// pixels. The library tints it with the freeze while Show Mask is on.
    fn freeze_mask(&self) -> Option<(&dyn RasterSource, u32)>;

    /// The displaced mesh for Show Mesh at `size`: the field's nodes moved by the field. `None`
    /// while the app has no mesh to show.
    fn mesh(&self, size: MeshSize) -> Option<LiquifyMesh>;

    /// The backdrop for the current Show Backdrop › Use, once the app has it (it was asked with
    /// [`LiquifyEvent::BackdropWanted`]); `None` until then.
    fn backdrop(&self) -> Option<&dyn RasterSource>;

    /// Show Backdrop › Use's layers, bottom to top, as `(layer id, name)`. "All Layers" is the
    /// library's own first entry.
    fn layers(&self) -> Vec<(u64, String)>;
}

/// What opens the workspace.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct LiquifyOpen {
    /// The document's size in pixels.
    pub size: [u32; 2],
    /// The layer's name: the title's subject.
    pub subject: String,
    /// The settings the workspace opens with; stored settings under [`LiquifyWorkspace::remember`]'s key replace them.
    /// The Preview check box is not among them: it is on at each opening.
    pub settings: LiquifySettings,
    /// The recorded steps of a smart filter opened again; empty for a new Liquify.
    pub steps: Vec<LiquifyStep>,
    /// The user's navigation options (wheel, flick, animated zoom); Photoshop's by default.
    pub navigation: NavigationOptions,
    /// The user's transparency grid under the image.
    pub checkerboard: Checker,
    /// The user's cursor options (the brush outline); Photoshop's by default.
    pub cursor_options: Option<BrushCursorOptions>,
}

impl LiquifyOpen {
    /// Opens on a layer of `size` document pixels named `subject`; every other option takes its default.
    pub fn new(size: [u32; 2], subject: impl Into<String>) -> Self {
        Self { size, subject: subject.into(), ..Self::default() }
    }

    /// The settings the workspace opens with.
    #[must_use]
    pub fn settings(mut self, settings: LiquifySettings) -> Self {
        self.settings = settings;
        self
    }

    /// The steps of a smart filter opened again (PhotoCraft's `open_smart_filter`).
    #[must_use]
    pub fn steps(mut self, steps: Vec<LiquifyStep>) -> Self {
        self.steps = steps;
        self
    }

    /// The user's navigation options (wheel, flick, animated zoom), for the canvas and the Hand
    /// and Zoom tools.
    #[must_use]
    pub fn navigation(mut self, navigation: NavigationOptions) -> Self {
        self.navigation = navigation;
        self
    }

    /// The user's transparency grid under the image.
    #[must_use]
    pub fn checkerboard(mut self, checkerboard: Checker) -> Self {
        self.checkerboard = checkerboard;
        self
    }

    /// The user's cursor options for the brush outline.
    #[must_use]
    pub fn cursor_options(mut self, options: BrushCursorOptions) -> Self {
        self.cursor_options = Some(options);
        self
    }
}

/// What the workspace reports to the app, in the order it happened this pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LiquifyEvent {
    /// A brush stroke or a Freeze Lasso polygon. `Begin` carries the stroke with its first point,
    /// `Update` the stroke's tool and brush with only the points added since the last event (the
    /// app applies them to its field), `Commit` the whole stroke, now recorded in the session
    /// (the app applies the points it has not seen: at least the release point); `Cancel` (Esc, a
    /// lost pointer, a tool switch or an undo during a stroke) is followed by a
    /// [`Replay`](Self::Replay), since the app's field already took the dabs. A lasso reports only
    /// `Commit`.
    Stroke(StrokeEvent),
    /// Reconstruct, Mask All, None, Invert All (the panel's buttons, ⌘I, ⌘D): recorded in the
    /// session, then reported for the app to apply.
    Global(LiquifyGlobal),
    /// The field must be rebuilt from these steps: after undo, redo, Restore All and a cancelled
    /// stroke.
    Replay(Vec<LiquifyStep>),
    /// Show Backdrop turned on, or its Use changed: the app loads that backdrop (`None`: All
    /// Layers) and shows it through [`LiquifySource::backdrop`].
    BackdropWanted(Option<u64>),
    /// The settings differ from the last ones reported.
    SettingsChanged(LiquifySettings),
    /// The Preview checkbox or the `P` key turned the preview on or off.
    Preview(bool),
}

/// What OK answers: the recorded steps and the settings.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LiquifyDone {
    /// The steps recorded in the session.
    pub steps: Vec<LiquifyStep>,
    /// The settings as they were at OK.
    pub settings: LiquifySettings,
}

/// What the workspace keeps between passes besides the session: the view, the strip's memory and the tool in progress. None of it is
/// the app's, and none of it is in the history.
#[derive(Clone)]
struct LiquifyView {
    view: FullWindowView,
    last_used: Tracked<LastUsed<LiquifyTool>>,
    slots: Tracked<Vec<ToolSlot<LiquifyTool>>>,
    canvas: LiquifyCanvas,
    /// The canvas the tools run under, to cancel their gestures outside a pass.
    canvas_id: Option<Id>,
}

impl Default for LiquifyView {
    fn default() -> Self {
        LiquifyView {
            view: FullWindowView::default(),
            last_used: Tracked::default(),
            slots: Tracked::new(LiquifyTool::slots()),
            canvas: LiquifyCanvas::default(),
            canvas_id: None,
        }
    }
}

/// The Liquify workspace: the session and its history and the settings, as one value the app keeps while Liquify
/// is open, with the view, the strip's memory and the stroke in progress.
///
/// Keys ([`LiquifyCommands`], on the scope of the dialog's body): W R E C S B O F D L pick the Liquify tools and H, Z the Hand
/// and Zoom tools; `[` and `]` scale the brush, ⌘H shows or hides the mask, ⌘I inverts the mask, ⌘D clears it. The
/// dialog applies the zoom commands to its view. ⌘Z and ⇧⌘Z step the workspace's history (a field being edited
/// steps its own text first); Enter is OK and Esc Cancel (Esc during a stroke cancels the stroke only). A tool switch or
/// an undo during a stroke cancels it first (`CancelReason::Removed`, `CancelReason::UndoDuringGesture`); the app's
/// field is rebuilt by the [`LiquifyEvent::Replay`] that follows.
///
/// The side column's header is "Properties", as Photoshop's Liquify panel is headed (lead call 89: the one editor
/// with a header besides Select and Mask and Content-Aware Fill).
#[derive(Clone, Pattern)]
pub struct LiquifyWorkspace {
    edit: Edit,
    /// The view, the strip's memory and the tool in progress.
    view: LiquifyView,
    subject: String,
    navigation: NavigationOptions,
    checkerboard: Checker,
    /// The backdrop the app was last asked for, while Show Backdrop is on.
    asked: Option<Option<u64>>,
    /// The zoom on screen at the end of the last pass, for the title.
    zoom_percent: f64,
    /// The timing of the latest stroke, as of the last pass.
    timing: Option<LiquifyTiming>,
    /// The cursor the canvas tool asked for in the last pass.
    cursor: Option<Cursor>,
    /// The Preview checkbox: on at each opening, never remembered.
    preview: bool,
    /// The settings as the app last heard them, so a change made between passes (a control request, a colour answer)
    /// is reported once by the next pass.
    reported: LiquifySettings,
    /// Where the settings are remembered ([`remember`](LiquifyWorkspace::remember)).
    remember: Option<SaveKey>,
    /// The first pass has read the remembered settings.
    recalled: bool,
}

/// The parts of the workspace the canvas tool's gestures and the history change together: kept apart from the view so a
/// method on them can run beside a borrow of the view.
#[derive(Clone)]
struct Edit {
    session: LiquifySession,
    history: EditHistory<LiquifySession>,
    settings: LiquifySettings,
    cursor_options: BrushCursorOptions,
    /// The events the next pass reports first: what the methods between passes queued, then the pass's own.
    events: Vec<LiquifyEvent>,
}

impl LiquifyWorkspace {
    /// A workspace over `open`, fitted on its first pass.
    pub fn new(open: LiquifyOpen) -> Self {
        let session = LiquifySession::from_steps(open.steps);
        Self {
            reported: open.settings.clone(),
            edit: Edit {
                history: EditHistory::new(&session),
                session,
                settings: open.settings,
                cursor_options: open.cursor_options.unwrap_or_else(canvas::default_cursor_options),
                events: Vec::new(),
            },
            view: LiquifyView::default(),
            subject: open.subject,
            navigation: open.navigation,
            checkerboard: open.checkerboard,
            asked: None,
            zoom_percent: 100.0,
            timing: None,
            cursor: None,
            preview: true,
            remember: None,
            recalled: false,
        }
    }

    /// Remembers the settings under `key` (PhotoCraft's Liquify settings, kept across openings and restarts): the first
    /// pass opens with the stored settings when there are any, in place of [`LiquifyOpen::settings`], and every
    /// settings change is stored, as production stored them on every change.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.remember = Some(key);
        self
    }

    /// How many steps Redo can go forward.
    pub fn redo_len(&self) -> usize {
        self.edit.history.redo_len()
    }

    /// The cursor the canvas tool asked for in the last pass.
    pub fn cursor(&self) -> Option<&Cursor> {
        self.cursor.as_ref()
    }

    /// The timing of the latest stroke, as of the last pass.
    pub fn timing(&self) -> Option<LiquifyTiming> {
        self.timing
    }

    /// Whether the Preview checkbox is on.
    pub fn preview(&self) -> bool {
        self.preview
    }

    /// Turns the preview on or off; the next pass reports [`LiquifyEvent::Preview`] when it changed.
    pub fn set_preview(&mut self, on: bool) {
        if self.preview != on {
            self.preview = on;
            self.edit.events.push(LiquifyEvent::Preview(on));
        }
    }

    /// The settings, for the control requests that change them. The next pass reports a change as
    /// [`LiquifyEvent::SettingsChanged`].
    pub fn settings_mut(&mut self) -> &mut LiquifySettings {
        &mut self.edit.settings
    }

    /// Records `step` in the session and queues its events for the next pass. A stroke reports only its `Commit`
    /// with every point, as a Freeze Lasso does; a stroke the session does not record (no points, a lasso of fewer
    /// than three) reports nothing. The caller asks for the repaint that runs the next pass (this method has no
    /// context).
    pub fn perform(&mut self, step: LiquifyStep) {
        match step {
            LiquifyStep::Stroke(stroke) => {
                let (steps, events) = (self.edit.session.len(), self.edit.events.len());
                self.edit.absorb(LiquifyEvent::Stroke(StrokeEvent::Commit(stroke)));
                if self.edit.session.len() == steps {
                    self.edit.events.truncate(events);
                }
            }
            LiquifyStep::Global(op) => self.edit.record_global(None, op),
        }
    }

    /// Undoes the last step; its events come out of the next pass. A stroke still open is abandoned first.
    pub fn undo(&mut self) {
        self.edit.step(None, &mut self.view, Step::Undo);
    }

    /// Redoes the last undone step; its events come out of the next pass. A stroke still open is abandoned first.
    pub fn redo(&mut self) {
        self.edit.step(None, &mut self.view, Step::Redo);
    }

    /// Restores every step; its events come out of the next pass.
    pub fn restore_all(&mut self) {
        self.edit.restore_all();
    }

    /// The result OK answers.
    fn done(&self) -> LiquifyDone {
        LiquifyDone { steps: self.edit.session.steps().to_vec(), settings: self.edit.settings.clone() }
    }

    /// The user's navigation options (wheel, flick, animated zoom); Photoshop's by default.
    pub fn navigation(&mut self, navigation: NavigationOptions) {
        self.navigation = navigation;
    }

    /// The user's transparency grid under the image.
    pub fn checkerboard(&mut self, checkerboard: Checker) {
        self.checkerboard = checkerboard;
    }

    /// The user's cursor options for the brush outline.
    pub fn cursor_options(&mut self, options: BrushCursorOptions) {
        self.edit.cursor_options = options;
    }

    /// Shows the workspace for this pass.
    ///
    /// A [`FullWindowDialog`] titled by [`FullWindowTitle`] (the subject and the zoom), with the Properties column ([`LiquifyPanel`], OK and Cancel at its bottom) in the dialog's
    /// side region. The body, in order: Undo and Redo; the commands, in the scope of
    /// [`LiquifyCommands`]; the tool strip (a `ToolRail` over [`LiquifyTool::slots`]; a pick
    /// cancels the old tool's gesture with `Removed`); the canvas ([`FullWindowCanvas`]) with the source's image under
    /// [`BackdropLayering`], the freeze mask as a `RasterOverlay` tinted red while Show Mask is on, the mesh while
    /// Show Mesh is on, the lasso and the resize preview as canvas controls, and [`LiquifyCanvasTool`] as its tool.
    /// Undo and redo report [`LiquifyEvent::Replay`] in the same pass.
    ///
    /// The events of the methods that ran between passes come first. A second pass of a frame reports only what its
    /// own input did, since the first pass took the frame's steps and commands.
    fn pass(&mut self, ui: &mut Ui, source: &dyn LiquifySource, open: Bind<'_, bool>) -> WorkspacePass<LiquifyEvent, LiquifyDone> {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:950 `show` (title, strip, panel, footer,
        // preview, brush circle); :730 `history_keys`; :740 `keys`.
        if !std::mem::replace(&mut self.recalled, true)
            && let Some(key) = &self.remember
            && let Some(stored) = saved_value::<Option<LiquifySettings>>(ui.ctx(), key)
        {
            // The pass reports the change as `SettingsChanged`, as it does for any other change before it.
            self.edit.settings = stored;
        }
        let env = Env::of(ui);
        let words = env.words::<LiquifyWords>();
        let common = env.words::<CommonWords>();
        let layers = source.layers();
        let title = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);
        if !self.edit.settings.show_backdrop {
            self.asked = None;
        } else if self.asked != Some(backdrop_id(self.edit.settings.backdrop)) {
            // Show Backdrop was on when the workspace opened, or a control request turned it on or changed its Use
            // between passes.
            let of = backdrop_id(self.edit.settings.backdrop);
            self.edit.events.push(LiquifyEvent::BackdropWanted(of));
            self.asked = Some(of);
        }
        let preview_label = words.preview.clone();
        let this = RefCell::new(&mut *self);

        let mut shown = FullWindowDialog::new(title)
            .command_scope(LiquifyCommands::SCOPE)
            .opened(open)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .footer(|ui, _| {
                let _ = Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
                    let mut guard = this.borrow_mut();
                    let work: &mut LiquifyWorkspace = &mut guard;
                    let events = Checkbox::new(Bind::given(work.preview)).label(preview_label.clone()).show(ui).events;
                    if let Some(on) = events.value().copied() {
                        work.set_preview(on);
                    }
                });
            })
            .sides(|ui, _| {
                let mut guard = this.borrow_mut();
                let work: &mut LiquifyWorkspace = &mut guard;
                let Some(panel) = FullWindowSide::new(words.properties.clone())
                    .show(ui, |ui, _| LiquifyPanel::new(Bind::bound(&mut work.edit.settings)).layers(&layers).show(ui))
                    .inner
                else {
                    return;
                };
                if let Some(op) = panel.global {
                    work.edit.record_global(None, op);
                }
                if panel.restore_all {
                    work.edit.restore_all();
                }
                if let Some(of) = panel.backdrop_wanted {
                    work.edit.events.push(LiquifyEvent::BackdropWanted(of));
                    work.asked = Some(of);
                }
            })
            .show(ui, |ui, cx| {
                let ctx = ui.ctx().clone();
                let mut guard = this.borrow_mut();
                let work: &mut LiquifyWorkspace = &mut guard;
                // Before the canvas runs, so an undo during a releasing stroke cancels it instead of undoing.
                while let Some(step) = cx.take_step(work.edit.history.can_undo(), work.edit.history.can_redo()) {
                    work.edit.step(Some(&ctx), &mut work.view, step);
                }
                work.view.canvas.navigate(work.navigation);
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    work.view.view.apply(command);
                }

                {
                    while let Some((command, fired)) = cx.take_command::<LiquifyCommands>() {
                        if let Some(tool) = command.tool() {
                            // A tool key's release picks nothing.
                            if fired.phase == FirePhase::Release {
                                continue;
                            }
                            let slot = work.view.slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == tool));
                            let tools: Vec<_> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                            let picked = slot.and_then(|slot| {
                                tool_pick(&tools, slot.id, &work.edit.settings.tool, &work.view.last_used, PickStep::of(&fired), |_| true)
                                    .map(|entry| (slot.id, entry.id))
                            });
                            if let Some((slot_id, tool)) = picked {
                                work.edit.pick(Some(&ctx), &mut work.view, tool);
                                work.view.last_used.set(slot_id, tool);
                            }
                            continue;
                        }
                        match command {
                            LiquifyCommands::Smaller => work.edit.settings.brush = work.edit.settings.brush.smaller(),
                            LiquifyCommands::Larger => work.edit.settings.brush = work.edit.settings.brush.larger(),
                            LiquifyCommands::Preview => work.set_preview(!work.preview),
                            LiquifyCommands::ShowMask => work.edit.settings.show_mask = !work.edit.settings.show_mask,
                            LiquifyCommands::InvertMask => work.edit.record_global(Some(&mut work.view), LiquifyGlobal::InvertAll),
                            LiquifyCommands::ClearMask => work.edit.record_global(Some(&mut work.view), LiquifyGlobal::MaskNone),
                            _ => {}
                        }
                    }
                    let state = &mut work.view;

                    let backdrop = source.backdrop().filter(|_| work.edit.settings.show_backdrop);
                    let layering = BackdropLayering::of(backdrop.is_some(), work.edit.settings.backdrop_mode, work.edit.settings.backdrop_opacity);
                    let layered = backdrop.filter(|_| !layering.is_plain()).map(|backdrop| LayeredImage::new(source.picture(), backdrop, layering));
                    let image: &dyn RasterSource = match &layered {
                        Some(layered) => layered,
                        None => source.picture(),
                    };
                    let mask = source.freeze_mask().filter(|_| work.edit.settings.show_mask).map(|(mask, texel)| MaskAtTexel {
                        mask,
                        texel,
                        document: image.size(),
                    });
                    let edges = source.mesh(work.edit.settings.mesh_size).filter(|_| work.edit.settings.show_mesh).map(|mesh| mesh.edges());
                    let lasso = state.canvas.lasso_line();
                    let falloff = state.canvas.resize().dimensions().map(|dims| TipFalloff::from_hardness(dims.diameter / 2.0, dims.level));
                    let preview = falloff.as_ref().and_then(|falloff| state.canvas.resize().preview(falloff));

                    let shown = Row::new()
                        .fill()
                        .show(ui, |ui, _| {
                            let rail = ToolRail::new(words.tools.clone(), &state.slots, &work.edit.settings.tool, &state.last_used)
                                .show_column(ui, |_, _| {});
                            if let Some(next) = rail.last_used {
                                *state.last_used = next;
                            }
                            if let Some(tool) = rail.select {
                                work.edit.pick(Some(&ctx), state, tool);
                            }
                            let LiquifyView { view, canvas: tools, .. } = &mut *state;
                            let mut canvas = FullWindowCanvas::new(Bind::bound(view), image)
                                .checkerboard(work.checkerboard)
                                .navigation(work.navigation)
                                .controls(|ui, rect| {
                                    if let Some(edges) = &edges {
                                        MeshLines::new(edges).show_at(ui, rect);
                                    }
                                    if let Some((points, tone)) = &lasso {
                                        LassoLine::new(points).tone(*tone).show_at(ui, rect);
                                    }
                                    if let Some(preview) = &preview {
                                        preview.clone().show_at(ui, rect);
                                    }
                                });
                            if let Some(mask) = &mask {
                                canvas = canvas.overlay(RasterOverlay::new(mask).tint(FREEZE_MASK_TINT));
                            }
                            match work.edit.settings.tool {
                                LiquifyTool::Hand => canvas.tool(&mut tools.hand).show(ui),
                                LiquifyTool::Zoom => canvas.tool(&mut tools.zoom).show(ui),
                                _ => {
                                    let mut tool = tools.tool(&mut work.edit.settings, &work.edit.cursor_options);
                                    canvas.tool(&mut tool).show(ui)
                                }
                            }
                        })
                        .inner;
                    shown.map(|shown| (shown.response.id, shown.tool.cursor.clone()))
                }
            });
        // Once per pass, after the real run: what the canvas reported is stored and its events absorbed.
        shown.settle(|_, canvas| {
            let mut guard = this.borrow_mut();
            let work: &mut LiquifyWorkspace = &mut guard;
            if let Some((id, cursor)) = canvas.take() {
                work.view.canvas_id = Some(id);
                work.cursor = cursor;
            }
            work.zoom_percent = work.view.view.zoom_percent();
            work.edit.absorb_canvas_events(&mut work.view);
            work.timing = work.view.canvas.timing();
        });
        let this = this.into_inner();
        if this.edit.settings != this.reported {
            this.reported = this.edit.settings.clone();
            this.edit.events.push(LiquifyEvent::SettingsChanged(this.reported.clone()));
            if let Some(key) = &this.remember {
                set_saved_value(ui.ctx(), key, Some(this.reported.clone()));
            }
        }
        let result = shown.output.result(|| this.done());
        let events = std::mem::take(&mut this.edit.events);
        WorkspacePass::new(shown.output.response, events).result(result).canvas(this.view.canvas_id)
    }

    /// What OK applies.
    pub fn session(&self) -> &LiquifySession {
        &self.edit.session
    }

    /// The current settings.
    pub fn settings(&self) -> &LiquifySettings {
        &self.edit.settings
    }
}

impl Edit {
    /// Records the session as one history step when it differs from `before`.
    fn record_history(&mut self, before: LiquifySession) {
        if before == self.session {
            return;
        }
        let name: Arc<str> = Arc::from(HISTORY_NAME);
        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name, None));
    }

    /// Puts `event` in this pass's events, recording a committed stroke in the session first (the lasso's closing
    /// point included) and following a cancelled one with the replay that rebuilds the app's field.
    fn absorb(&mut self, event: LiquifyEvent) {
        match event {
            LiquifyEvent::Stroke(StrokeEvent::Commit(stroke)) => {
                let before = self.session.clone();
                self.session.record(stroke.clone());
                let recorded = match self.session.steps().last() {
                    Some(LiquifyStep::Stroke(recorded)) if self.session.len() > before.len() => recorded.clone(),
                    _ => stroke,
                };
                self.record_history(before);
                self.events.push(LiquifyEvent::Stroke(StrokeEvent::Commit(recorded)));
            }
            LiquifyEvent::Stroke(cancel @ StrokeEvent::Cancel(_)) => {
                self.events.push(LiquifyEvent::Stroke(cancel));
                self.events.push(LiquifyEvent::Replay(self.session.steps().to_vec()));
            }
            other => self.events.push(other),
        }
    }

    /// Takes the events the canvas tool collected.
    fn absorb_canvas_events(&mut self, state: &mut LiquifyView) {
        for event in state.canvas.take_events() {
            self.absorb(event);
        }
    }

    /// Records a whole-field operation after the stroke in progress, if `state` has one.
    fn record_global(&mut self, state: Option<&mut LiquifyView>, op: LiquifyGlobal) {
        if let Some(state) = state
            && let Some(event) = state.canvas.end_stroke()
        {
            self.absorb(event);
        }
        let before = self.session.clone();
        self.session.record_global(op);
        self.record_history(before);
        self.events.push(LiquifyEvent::Global(op));
    }

    /// Restore All: drops every step as one history step; the app rebuilds its field from the empty replay.
    fn restore_all(&mut self) {
        let before = self.session.clone();
        self.session.restore_all();
        self.record_history(before);
        self.events.push(LiquifyEvent::Replay(Vec::new()));
    }

    /// Cancels the gesture the current tool holds (a Liquify tool's stroke, lasso or resize, or the Hand's or Zoom's
    /// drag) through the host, and the tool's own state if the host held none. Without a context (a request between
    /// passes) only the tool's own state is dropped.
    fn cancel_gesture(&mut self, ctx: Option<&Context>, state: &mut LiquifyView, reason: CancelReason) {
        let cursor = self.cursor_options;
        let LiquifyView { canvas, canvas_id, .. } = state;
        if let (Some(ctx), Some(id)) = (ctx, *canvas_id) {
            match self.settings.tool {
                LiquifyTool::Hand => ToolHost::new().cancel(ctx, id, &mut canvas.hand, reason),
                LiquifyTool::Zoom => ToolHost::new().cancel(ctx, id, &mut canvas.zoom, reason),
                _ => ToolHost::new().cancel(ctx, id, &mut canvas.tool(&mut self.settings, &cursor), reason),
            };
        }
        if canvas.busy() {
            canvas.tool(&mut self.settings, &cursor).abandon(reason);
        }
        self.absorb_canvas_events(state);
    }

    /// Undoes or redoes, cancelling an open stroke first (`UndoDuringGesture`); the replay of the step supersedes the
    /// one the cancel reports.
    fn step(&mut self, ctx: Option<&Context>, state: &mut LiquifyView, step: Step) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:340 `undo`, :351 `redo`.
        self.cancel_gesture(ctx, state, CancelReason::UndoDuringGesture);
        if self.history.step(&mut self.session, step) {
            if matches!(self.events.last(), Some(LiquifyEvent::Replay(_))) {
                self.events.pop();
            }
            self.events.push(LiquifyEvent::Replay(self.session.steps().to_vec()));
        }
    }

    /// Switches to `tool`, cancelling the old tool's gesture first.
    fn pick(&mut self, ctx: Option<&Context>, state: &mut LiquifyView, tool: LiquifyTool) {
        if tool != self.settings.tool {
            self.cancel_gesture(ctx, state, CancelReason::Removed);
            self.settings.tool = tool;
        }
    }
}

/// The name the history gives its steps.
const HISTORY_NAME: &str = "Liquify";

/// The workspace on the app's dialog host. OK answers at once (no held OK): the steps and settings are the whole
/// answer. The key `filter.liquify` is the command the workspace answers for.
impl FullWindowWorkspace for LiquifyWorkspace {
    type Source = dyn LiquifySource;
    type Event = LiquifyEvent;
    type Result = LiquifyDone;

    const KEY: &'static str = "filter.liquify";

    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<Self::Event, Self::Result> {
        let WorkspaceCx { source, open, .. } = cx;
        self.pass(ui, source, open)
    }

    /// The settings (the brush keys at the top level) and `preview`.
    fn json_fields(&self, _source: &Self::Source) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.edit.settings)?;
        fields.as_object_mut()?.insert("preview".to_owned(), serde_json::Value::Bool(self.preview));
        Some(fields)
    }

    /// A settings key, `preview`, or an action: `stroke` (a [`LiquifyStroke`]), `reconstruct` (the amount),
    /// `maskAll`, `maskNone`, `invertMask`, `restoreAll`, `undo`, `redo` (each `true`). An action runs the way the
    /// canvas and the panel run it; its events come out of the next pass.
    fn set_json_field(&mut self, ctx: &Context, _source: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        let rejects = |value: &serde_json::Value| FieldRefusal::Rejects(value).message(field);
        match field {
            "stroke" => {
                let stroke = serde_json::from_value::<LiquifyStroke>(value.clone()).map_err(|_| rejects(&value))?;
                if stroke.points.is_empty() {
                    return Err(rejects(&value));
                }
                self.perform(LiquifyStep::Stroke(stroke));
            }
            "reconstruct" => {
                let amount = value.as_f64().filter(|amount| amount.is_finite()).ok_or_else(|| rejects(&value))?;
                self.perform(LiquifyStep::Global(LiquifyGlobal::reconstruct(amount.clamp(0.0, 100.0))));
            }
            "maskAll" | "maskNone" | "invertMask" | "restoreAll" | "undo" | "redo" => {
                if value != serde_json::Value::Bool(true) {
                    return Err(rejects(&value));
                }
                match field {
                    "maskAll" => self.perform(LiquifyStep::Global(LiquifyGlobal::MaskAll)),
                    "maskNone" => self.perform(LiquifyStep::Global(LiquifyGlobal::MaskNone)),
                    "invertMask" => self.perform(LiquifyStep::Global(LiquifyGlobal::InvertAll)),
                    "restoreAll" => self.restore_all(),
                    "undo" => self.undo(),
                    _ => self.redo(),
                }
            }
            "preview" => self.set_preview(value.as_bool().ok_or_else(|| rejects(&value))?),
            _ => {
                let before = self.edit.settings.clone();
                set_serde_field(&mut self.edit.settings, field, value)?;
                // A number out of range is clamped as a loaded record is; a tool switch cancels the old tool's gesture.
                let mut settings = self.edit.settings.clone().clamped(&before);
                let tool = std::mem::replace(&mut settings.tool, before.tool);
                self.edit.settings = settings;
                self.edit.pick(Some(ctx), &mut self.view, tool);
            }
        }
        Ok(())
    }

    fn confirm(&mut self, _ctx: &Context, _source: &Self::Source) -> Option<Self::Result> {
        Some(self.done())
    }
}
