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
//! [`LiquifySource`]. OK runs `filter.liquify` with [`LiquifySession::steps`]; Cancel changes
//! nothing.
//!
//! ```ignore
//! let out = app.liquify.show(ui, &PcLiquifySource { proxy: &app.liquify_proxy, doc: &doc });
//! for event in out.events {
//!     match event {
//!         LiquifyEvent::Stroke(e) => app.liquify_engine.stroke(e),
//!         LiquifyEvent::Global(op) => app.liquify_engine.global(op),
//!         LiquifyEvent::Replay(steps) => app.liquify_engine.replay(&steps),
//!         LiquifyEvent::BackdropWanted(of) => app.load_backdrop(of),
//!     }
//! }
//! match out.closed {
//!     Some(DialogRole::Accept) => app.run("filter.liquify", app.liquify.session().steps()),
//!     Some(DialogRole::Cancel) => app.close_liquify(),
//!     _ => {}
//! }
//! app.prefs.liquify = app.liquify.settings().clone();
//! ```
//!
//! Files: `session` (the steps, the settings), `tools` (the tools and the stroke
//! recorder), `commands` (the workspace keys), `panel` (the Properties panel),
//! `canvas` (the canvas tool, the backdrop, the mask and mesh, the brush resize), this file
//! (`show`, `step`).
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`FullWindowSide`]: craft_ui::dialog::FullWindowSide
//! [`EditHistory`]: craft_behavior::EditHistory

mod canvas;
mod commands;
mod panel;
mod session;
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

use egui::{Color32, Id, Response, Ui};

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, Cursor, EditEvent, EditHistory, Mixed};
use craft_ui::canvas::{NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, tool_pick};
use craft_ui::canvas_controls::brush::{BrushCursorOptions, TipFalloff};
use craft_ui::contract::{Bind, NoValue, Outcome, Pattern, Show as _, ShowContainer as _};
use craft_ui::dialog::{FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowTitle, FullWindowView, RasterOverlay};
use craft_ui::layout_components::Row;
use craft_ui::primitives::Checker;
use craft_ui::rails::{LastUsed, ToolRail};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _};
use craft_ui::runtime::env::Env;
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
pub trait LiquifySource {
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
    /// The remembered settings (PhotoCraft: `prefs.dialogs["filter.liquify"]`).
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

    /// The remembered settings.
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
}

/// What [`LiquifyWorkspace::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LiquifyOutput {
    /// What happened this pass, in order.
    pub events: Vec<LiquifyEvent>,
    /// OK (`Accept`) or Cancel this pass.
    pub closed: Option<DialogRole>,
    /// The dialog's response: its rect is the workspace's.
    pub response: Response,
    /// The settings changed this pass (compared before and after, whatever changed them: the
    /// panel, the keys, the strip or a brush resize); the app may persist them.
    pub settings_changed: bool,
    /// The cursor the canvas tool asked for this pass: the brush outline over the canvas, none off it.
    pub cursor: Option<Cursor>,
    /// The timing of the latest brush stroke, for the app's diagnostics (PhotoCraft's stats line
    /// is not drawn; lead call 14); `None` before the first stroke.
    pub timing: Option<LiquifyTiming>,
}

impl Outcome for LiquifyOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the workspace keeps between passes besides the session: the view, the strip's memory and the tool in progress. None of it is
/// the app's, and none of it is in the history.
#[derive(Clone, Default)]
struct LiquifyView {
    view: FullWindowView,
    last_used: LastUsed<LiquifyTool>,
    canvas: LiquifyCanvas,
    /// The canvas the tools run under, to cancel their gestures outside a pass.
    canvas_id: Option<Id>,
}

/// The Liquify workspace: the session and its history and the settings, as one value the app keeps while Liquify
/// is open, with the view, the strip's memory and the stroke in progress.
///
/// Keys ([`LiquifyCommands`], on the scope of the dialog's body): W R E C S B O F D L pick the Liquify tools and H, Z the Hand
/// and Zoom tools; `[` and `]` scale the brush, ⌘H shows or hides the mask, ⌘I inverts the mask, ⌘D clears it. The
/// dialog applies the zoom commands to its view. The app forwards ⌘Z and ⇧⌘Z (`EditCommands::Undo`, `Redo`) to
/// [`step`](Self::step); Enter is OK and Esc Cancel (Esc during a stroke cancels the stroke only). A tool switch or
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
    /// Events that happened between passes ([`step`](Self::step)), reported by the next `show`.
    queued: Vec<LiquifyEvent>,
    /// A step the app asked for while a stroke was open; the next `show` cancels the stroke, then steps.
    requested: Vec<Step>,
    /// A stroke, a lasso or a resize was open at the end of the last pass.
    busy: bool,
    /// The zoom on screen at the end of the last pass, for the title.
    zoom_percent: f64,
    /// The timing of the latest stroke, as of the last pass.
    timing: Option<LiquifyTiming>,
    /// The cursor the canvas tool asked for in the last pass.
    cursor: Option<Cursor>,
}

/// The parts of the workspace the canvas tool's gestures and the history change together: kept apart from the view so a
/// method on them can run beside a borrow of the view.
#[derive(Clone)]
struct Edit {
    session: LiquifySession,
    history: EditHistory<LiquifySession>,
    settings: LiquifySettings,
    cursor_options: BrushCursorOptions,
    /// This pass's events.
    events: Vec<LiquifyEvent>,
}

impl LiquifyWorkspace {
    /// A workspace over `open`, fitted on its first pass.
    pub fn new(open: LiquifyOpen) -> Self {
        let session = LiquifySession::from_steps(open.steps);
        Self {
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
            queued: Vec::new(),
            requested: Vec::new(),
            busy: false,
            zoom_percent: 100.0,
            timing: None,
            cursor: None,
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

    /// The user's cursor options for the brush outline.
    pub fn cursor_options(&mut self, options: BrushCursorOptions) {
        self.edit.cursor_options = options;
    }

    /// Shows the workspace for this pass.
    ///
    /// A [`FullWindowDialog`] titled by [`FullWindowTitle`] (the subject and the zoom), with the Properties column ([`LiquifyPanel`], OK and Cancel at its bottom) in the dialog's
    /// side region. The body, in order: the events [`step`](Self::step) queued; the commands, in the scope of
    /// [`LiquifyCommands`]; the tool strip (a `ToolRail` over [`LiquifyTool::slots`]; a pick
    /// cancels the old tool's gesture with `Removed`); the canvas ([`FullWindowCanvas`]) with the source's image under
    /// [`BackdropLayering`], the freeze mask as a `RasterOverlay` tinted red while Show Mask is on, the mesh while
    /// Show Mesh is on, the lasso and the resize preview as canvas controls, and [`LiquifyCanvasTool`] as its tool.
    /// Undo and redo report [`LiquifyEvent::Replay`] in the same pass.
    pub fn show(&mut self, ui: &mut Ui, source: &dyn LiquifySource) -> LiquifyOutput {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:950 `show` (title, strip, panel, footer,
        // preview, brush circle); :730 `history_keys`; :740 `keys`.
        let env = Env::of(ui);
        let words = env.words::<LiquifyWords>();
        let common = env.words::<CommonWords>();
        let layers = source.layers();
        let title = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);
        let settings_before = self.edit.settings.clone();
        let mut asked_steps = std::mem::take(&mut self.requested);
        self.edit.events = std::mem::take(&mut self.queued);
        if !self.edit.settings.show_backdrop {
            self.asked = None;
        } else if self.asked.is_none() {
            // Show Backdrop was already on when the workspace opened.
            let of = backdrop_id(self.edit.settings.backdrop);
            self.edit.events.push(LiquifyEvent::BackdropWanted(of));
            self.asked = Some(of);
        }
        // An open stroke is cancelled first; the replay of the step supersedes the one the cancel reports. This runs before
        // the container, so an undo during a releasing stroke cancels it instead of undoing.
        for step in asked_steps.drain(..) {
            self.edit.cancel_gesture(&ui.ctx().clone(), &mut self.view, CancelReason::UndoDuringGesture);
            if self.edit.history.step(&mut self.edit.session, step) {
                if matches!(self.edit.events.last(), Some(LiquifyEvent::Replay(_))) {
                    self.edit.events.pop();
                }
                self.edit.events.push(LiquifyEvent::Replay(self.edit.session.steps().to_vec()));
            }
        }
        let slots = LiquifyTool::slots();
        let this = RefCell::new(&mut *self);

        let mut shown = FullWindowDialog::new(title)
            .command_scope(LiquifyCommands::SCOPE)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
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
                    let before = work.edit.session.clone();
                    work.edit.session.restore_all();
                    work.edit.record_history(before);
                    work.edit.events.push(LiquifyEvent::Replay(Vec::new()));
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
                let state = &mut work.view;
                state.canvas.navigate(work.navigation);
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    state.view.apply(command);
                }

                {
                    while let Some((command, fired)) = cx.take_command::<LiquifyCommands>() {
                        if let Some(tool) = command.tool() {
                            let slot = slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == tool));
                            let tools: Vec<_> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                            let picked = slot.and_then(|slot| {
                                tool_pick(&tools, slot.id, &work.edit.settings.tool, &state.last_used, PickStep::of(&fired), |_| true)
                                    .map(|entry| (slot.id, entry.id))
                            });
                            if let Some((slot_id, tool)) = picked {
                                work.edit.pick(&ctx, state, tool);
                                state.last_used.set(slot_id, tool);
                            }
                            continue;
                        }
                        match command {
                            LiquifyCommands::Smaller => work.edit.settings.brush = work.edit.settings.brush.smaller(),
                            LiquifyCommands::Larger => work.edit.settings.brush = work.edit.settings.brush.larger(),
                            LiquifyCommands::ShowMask => work.edit.settings.show_mask = !work.edit.settings.show_mask,
                            LiquifyCommands::InvertMask => work.edit.record_global(Some(&mut *state), LiquifyGlobal::InvertAll),
                            LiquifyCommands::ClearMask => work.edit.record_global(Some(&mut *state), LiquifyGlobal::MaskNone),
                            _ => {}
                        }
                    }

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
                            let rail =
                                ToolRail::new(words.tools.clone(), &slots, &work.edit.settings.tool, &state.last_used).show_rail(ui, |_, _| {});
                            if let Some(next) = rail.last_used {
                                state.last_used = next;
                            }
                            if let Some(tool) = rail.select {
                                work.edit.pick(&ctx, state, tool);
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
            work.busy = work.view.canvas.busy();
            work.timing = work.view.canvas.timing();
        });
        let this = this.into_inner();
        let timing = this.timing;
        let picked = shown.output.picked.and_then(|at| [DialogRole::Accept, DialogRole::Cancel].get(at).copied());
        let closed = picked.or(shown.output.closed.map(|_| DialogRole::Cancel));
        LiquifyOutput {
            events: std::mem::take(&mut this.edit.events),
            closed,
            response: shown.output.response,
            settings_changed: this.edit.settings != settings_before,
            cursor: this.cursor.take(),
            timing,
        }
    }

    /// What OK applies.
    pub fn session(&self) -> &LiquifySession {
        &self.edit.session
    }

    /// What the app persists (always: PhotoCraft remembers Liquify's settings on every close).
    pub fn settings(&self) -> &LiquifySettings {
        &self.edit.settings
    }

    /// Undo or redo from the app's Edit menu. Returns whether a step will happen; its
    /// [`LiquifyEvent::Replay`] comes with the next [`show`](Self::show)'s events. With a stroke
    /// open the step waits for that pass, which cancels the stroke (`UndoDuringGesture`) first.
    pub fn step(&mut self, step: Step) -> bool {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:340 `undo`, :351 `redo`.
        if self.busy {
            self.requested.push(step);
            return match step {
                Step::Undo => self.edit.history.can_undo(),
                Step::Redo => self.edit.history.can_redo(),
                _ => false,
            };
        }
        let stepped = self.edit.history.step(&mut self.edit.session, step);
        if stepped {
            self.queued.push(LiquifyEvent::Replay(self.edit.session.steps().to_vec()));
        }
        stepped
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

    /// Cancels the gesture the current tool holds (a Liquify tool's stroke, lasso or resize, or the Hand's or Zoom's
    /// drag) through the host, and the tool's own state if the host held none.
    fn cancel_gesture(&mut self, ctx: &egui::Context, state: &mut LiquifyView, reason: CancelReason) {
        let cursor = self.cursor_options;
        let LiquifyView { canvas, canvas_id, .. } = state;
        if let Some(id) = *canvas_id {
            match self.settings.tool {
                LiquifyTool::Hand => ToolHost::cancel(ctx, id, &mut canvas.hand, reason),
                LiquifyTool::Zoom => ToolHost::cancel(ctx, id, &mut canvas.zoom, reason),
                _ => ToolHost::cancel(ctx, id, &mut canvas.tool(&mut self.settings, &cursor), reason),
            };
        }
        if canvas.busy() {
            canvas.tool(&mut self.settings, &cursor).abandon(reason);
        }
        self.absorb_canvas_events(state);
    }

    /// Switches to `tool`, cancelling the old tool's gesture first.
    fn pick(&mut self, ctx: &egui::Context, state: &mut LiquifyView, tool: LiquifyTool) {
        if tool != self.settings.tool {
            self.cancel_gesture(ctx, state, CancelReason::Removed);
            self.settings.tool = tool;
        }
    }
}

/// The name the history gives its steps.
const HISTORY_NAME: &str = "Liquify";
