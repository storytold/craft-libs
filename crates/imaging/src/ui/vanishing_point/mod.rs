//! Vanishing Point (Filter › Vanishing Point…): the plane editor and its full-window workspace.
//!
//! The owner's answer (decisions, "Step 7 questions" 2): Vanishing Point opens in its own window, as in Photoshop;
//! PhotoCraft ran it on the main canvas. [`VanishingPointWorkspace`] is a [`FullWindowDialog`] with the title strip
//! "Vanishing Point (layer, 66%)", the full-width options bar ([`VanishingPointBar`]), the tool strip (Edit Plane V,
//! Create Plane C, Stamp S, then Hand H and Zoom Z), the canvas, and a narrow side column with only OK and Cancel at
//! its bottom (Photoshop has no side panel and PhotoCraft's bar ends in check and ban buttons; the owner's answer
//! keeps the column, which also holds the preview's status).
//!
//! [`VanishingPointEditor`] is the plane editor without a frame: the workspace runs it in its canvas, and an app may
//! run it on any canvas through one [`VanishingPointPass`] per pass (its [`planes`](VanishingPointPass::planes)
//! control and its [`tool`](VanishingPointPass::tool)). The session ([`VanishingPointSession`]: planes, torn planes,
//! paste, clone source and strokes) has [`EditHistory`] undo. The app's [`VanishingScene`] holds the perspective
//! maths, its [`VanishingPointSource`] paints the stamp preview, and OK runs `filter.vanishingPoint` with the
//! session.
//!
//! Files: `planes` and `editor` (the state, the tool and the planes control); `draw`, `bar`, `new_plane`, `commands`
//! and this file's `show` and `step`; `style` and `words`.
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`EditHistory`]: craft_behavior::EditHistory

pub(crate) mod bar;
mod command;
mod commands;
mod draw;
pub(crate) mod editor;
mod new_plane;
mod planes;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod style;
mod words;

pub use bar::{VanishingPointBar, VanishingPointBarOutput};
pub use commands::{VanishingPointCommands, VanishingPointStripTool};
pub use editor::{
    PasteImage, PlaneEdit, StampLimits, StampSettings, StampStroke, VanishingPlanes, VanishingPlanesOutput, VanishingPointEditor, VanishingPointPass,
    VanishingPointSession, VanishingPointSettings, VanishingPointTool, VanishingTool,
};
pub use planes::{MAX_GRID_LINES, MAX_PLANES, PlaneCheck, PlaneEdge, TornPlane, VanishingPlane, VanishingScene, fit_torn, slide_edge};
pub use style::VanishingPointStyle;
pub use words::VanishingPointWords;

use std::cell::RefCell;
use std::sync::Arc;

use egui::{Color32, Id, Ui};
use serde_json::Value;

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, EditEvent, EditHistory, Mixed};
use craft_ui::bars::StatusMessage;
use craft_ui::canvas::{HandTool, NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, ZoomTool, tool_pick};
use craft_ui::contract::{Bind, Job, Pending, Remember as _, Show as _, ShowContainer as _, Tracked};
use craft_ui::dialog::{
    FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, FullWindowWorkspace,
    RasterOverlay, WorkspaceCx, WorkspacePass, serde_fields, set_serde_field,
};
use craft_ui::layout_components::Row;
use craft_ui::primitives::Checker;
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::windows::WindowSpec;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};

use command::{command_planes, command_strokes, point, read_paste_at, read_paste_width, read_planes, read_strokes};
use commands::Pick;
use editor::{PASTE_CELLS, is_unset};

pub use command::{VanishingCommandPaste, VanishingCommandPlane, VanishingCommandStroke, VanishingPointCommand};

use crate::ui::wide_angle::PreviewDone;

/// The paste's tint: white at the legacy alpha, so the image shows as it is, a little transparent over the picture.
/// It is data of the paste, not a theme colour.
const PASTE_TINT: Color32 = Color32::from_rgba_premultiplied(230, 230, 230, 230);

/// What the app gives the editor to show each pass.
pub trait VanishingPointSource: Send + Sync {
    /// The app's perspective scene: its planes and their geometry.
    fn scene(&self) -> &dyn VanishingScene;

    /// The layer as it is.
    fn image(&self) -> &dyn RasterSource;

    /// The clone strokes painted onto a copy of the layer (the app paints them in batches, lead call 47); `None` while
    /// there are none.
    fn stamp_preview(&self) -> Option<&dyn RasterSource>;

    /// The clipboard's image, for Paste; `None` without one.
    fn paste(&self) -> Option<&dyn RasterSource>;

    /// The job that paints the strokes of `request` onto a copy of the layer from scratch: after an undo, or a plane
    /// change under strokes. The workspace starts it once per change of the session and shows its status in the side
    /// column; the picture stays [`stamp_preview`](Self::stamp_preview).
    fn preview_job(&self, request: &VanishingPreviewRequest) -> Job<PreviewDone, String>;
}

/// What a preview job paints.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VanishingPreviewRequest {
    /// The planes, paste and strokes to paint.
    pub session: VanishingPointSession,
}

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VanishingPointOpen {
    /// The layer's size in document pixels.
    pub size: [u32; 2],
    /// The layer's name: the title's subject.
    pub subject: String,
    /// The planes, paste and strokes to start from.
    pub session: VanishingPointSession,
    /// The options bar's values.
    pub settings: VanishingPointSettings,
    /// The engine's clone-stroke limits.
    pub limits: StampLimits,
    /// The user's navigation options (Photoshop's defaults).
    pub navigation: NavigationOptions,
    /// The user's transparency grid.
    pub checkerboard: Checker,
}

impl VanishingPointOpen {
    /// Opens on a layer of `size` document pixels named `subject`; session and settings start at their defaults.
    pub fn new(size: [u32; 2], subject: impl Into<String>, limits: StampLimits) -> Self {
        Self {
            size,
            subject: subject.into(),
            session: VanishingPointSession::default(),
            settings: VanishingPointSettings::default(),
            limits,
            navigation: NavigationOptions::default(),
            checkerboard: Checker::default(),
        }
    }

    /// Sets the options bar's starting values.
    #[must_use]
    pub fn settings(mut self, settings: VanishingPointSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The planes, paste and strokes to start from.
    #[must_use]
    pub fn session(mut self, session: VanishingPointSession) -> Self {
        self.session = session;
        self
    }

    /// The user's navigation options, for the canvas, Hand and Zoom.
    #[must_use]
    pub fn navigation(mut self, options: NavigationOptions) -> Self {
        self.navigation = options;
        self
    }

    /// The user's transparency grid under the image.
    #[must_use]
    pub fn checkerboard(mut self, checkerboard: Checker) -> Self {
        self.checkerboard = checkerboard;
        self
    }
}

/// What the editor reports, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum VanishingPointEvent {
    /// The planes changed: a node or edge drag (Begin, Update, Commit), a plane made, torn off or deleted (`Set`).
    Planes(EditEvent<PlaneEdit>),
    /// The pasted image moved on its plane, or its Width changed.
    Paste(EditEvent<PasteImage>),
    /// A clone stroke: the app paints its preview from the points as they come.
    Stamp(EditEvent<StampStroke>),
    /// A bar setting changed.
    Settings(VanishingPointSettings),
    /// A press was refused; the message is worded for the app's status bar.
    Refused(StatusMessage),
}

/// What the dialog's body keeps between passes: the view, the strip's memory and the canvas the tools run under.
#[derive(Clone)]
struct VanishingView {
    view: FullWindowView,
    last_used: Tracked<LastUsed<Pick>>,
    slots: Tracked<Vec<ToolSlot<Pick>>>,
    /// The canvas the tools run under, to cancel their gestures outside a pass.
    canvas: Option<Id>,
}

impl Default for VanishingView {
    fn default() -> Self {
        VanishingView { view: FullWindowView::default(), last_used: Tracked::default(), slots: Tracked::new(slots()), canvas: None }
    }
}

/// The New Plane form's state.
#[derive(Clone, Default)]
struct NewPlaneState {
    open: bool,
    values: new_plane::NewPlaneCorners,
}

/// The Vanishing Point workspace: the session, its history, the settings and the editor, as one value the app keeps
/// while Vanishing Point is open. The view and the strip's memory live in the dialog's body.
///
/// Keys ([`VanishingPointCommands`], on the dialog's scope): V, C, S, H, Z pick the tools; Delete and Backspace remove
/// the selected plane while the canvas has the keyboard. ⌘Z and ⇧⌘Z step the history (a field being edited steps its own
/// text first); Enter is OK and
/// Esc Cancel (with the canvas focused, Esc drops pending corners first, and during a drag cancels the drag only). A
/// tool switch cancels the old tool's gesture (`CancelReason::Removed`), and so does an undo
/// (`CancelReason::UndoDuringGesture`).
#[derive(craft_ui::contract::Pattern)]
pub struct VanishingPointWorkspace {
    session: VanishingPointSession,
    history: EditHistory<VanishingPointSession>,
    /// The session as the history last saw it.
    recorded: VanishingPointSession,
    /// Counts the sessions the history has seen: the key of the preview job.
    revision: u64,
    settings: VanishingPointSettings,
    /// The settings as the last [`Settings`](VanishingPointEvent::Settings) event reported them.
    reported: VanishingPointSettings,
    /// Events from calls outside `show`; the next pass reports them first.
    queued: Vec<VanishingPointEvent>,
    editor: VanishingPointEditor,
    subject: String,
    size: [u32; 2],
    navigation: NavigationOptions,
    checkerboard: Checker,
    strip: VanishingPointStripTool,
    hand: HandTool,
    zoom: ZoomTool,
    new_plane: NewPlaneState,
    /// The zoom on screen at the end of the last pass, for the title.
    zoom_percent: f64,
}

impl VanishingPointWorkspace {
    /// A workspace on the layer and session that `open` describes.
    pub fn new(open: VanishingPointOpen) -> Self {
        Self {
            history: EditHistory::new(&open.session),
            recorded: open.session.clone(),
            session: open.session,
            revision: 0,
            settings: open.settings,
            reported: open.settings,
            queued: Vec::new(),
            editor: VanishingPointEditor::new(open.limits),
            subject: open.subject,
            size: open.size,
            navigation: open.navigation,
            checkerboard: open.checkerboard,
            strip: VanishingPointStripTool::Editor,
            hand: HandTool::new(open.navigation),
            zoom: ZoomTool::new(open.navigation),
            new_plane: NewPlaneState::default(),
            zoom_percent: 100.0,
        }
    }

    /// Selects plane `index` (none when it is out of range or `None`), for an app or a story that opens on a chosen
    /// plane.
    pub fn selected_plane(&mut self, index: Option<usize>) -> &mut Self {
        self.editor.select(index, &self.session);
        self
    }

    /// Picks the editor's tool, for an app or a story that opens on a chosen tool.
    pub fn tool(&mut self, tool: VanishingTool) -> &mut Self {
        self.strip = VanishingPointStripTool::Editor;
        self.editor.set_tool(tool);
        self
    }

    /// The planes, paste and strokes recorded so far.
    pub fn session(&self) -> &VanishingPointSession {
        &self.session
    }

    /// The editor: the selected tool and plane, and the corners Create Plane has clicked.
    pub fn editor(&self) -> &VanishingPointEditor {
        &self.editor
    }

    /// The engine command OK answers: the session's planes, paste and strokes with the bar's options. The paste is
    /// left out when `has_paste` is false.
    pub fn command(&self, has_paste: bool) -> VanishingPointCommand {
        VanishingPointCommand::new(&self.session, &self.settings, self.editor.selected(), has_paste)
    }

    /// The options bar's current values.
    pub fn settings(&self) -> &VanishingPointSettings {
        &self.settings
    }

    /// Whether a plane the scene calls invalid blocks OK: the corners of a plane that is set must form a convex quad.
    fn blocked(&self, scene: &dyn VanishingScene) -> bool {
        (0..self.session.planes.len()).any(|i| !is_unset(&self.session.planes[i]) && scene.check(&self.session.planes, i) == PlaneCheck::Invalid)
    }

    /// Runs `write` on the session with the history it needs: a gesture in progress is cancelled first, and what the
    /// write changed is one history step named `step`. The events wait for the next pass: the editor's, and a `Set` for
    /// the planes or the strokes when the write changed them, as an edit by pointer reports them.
    fn write_session(&mut self, step: &str, write: impl FnOnce(&mut Self)) {
        self.editor.cancel(&mut self.session, CancelReason::Removed);
        let before = self.session.clone();
        write(self);
        let mut events = self.editor.take_events();
        let label: Arc<str> = Arc::from(step);
        if (before.planes.as_slice(), before.torn.as_slice()) != (self.session.planes.as_slice(), self.session.torn.as_slice()) {
            let was = PlaneEdit::new(before.planes.clone(), before.torn.clone());
            let now = PlaneEdit::new(self.session.planes.clone(), self.session.torn.clone());
            events.push(VanishingPointEvent::Planes(EditEvent::set(Mixed::Same(was), now, label.clone(), None)));
        }
        if before.paste != self.session.paste {
            events.push(VanishingPointEvent::Paste(EditEvent::set(Mixed::Same(before.paste), self.session.paste, label.clone(), None)));
        }
        if before.strokes != self.session.strokes {
            // A stroke is the event's value: the last one, or the last one emptied when the write cleared them.
            let was = before.strokes.last().cloned();
            let now = self.session.strokes.last().cloned().or_else(|| {
                was.clone().map(|mut stroke| {
                    stroke.points.clear();
                    stroke
                })
            });
            if let Some(now) = now {
                events.push(VanishingPointEvent::Stamp(EditEvent::set(was.map_or(Mixed::Mixed, Mixed::Same), now, label, None)));
            }
        }
        self.record(&events, step);
        self.queued.extend(events);
    }
}

/// Why a field value was refused, with what is wrong in `detail`.
fn rejects(field: &str, value: &Value, detail: &str) -> String {
    format!("{} ({detail})", FieldRefusal::Rejects(value).message(field))
}

impl FullWindowWorkspace for VanishingPointWorkspace {
    type Source = dyn VanishingPointSource;
    type Event = VanishingPointEvent;
    type Result = VanishingPointCommand;

    const KEY: &'static str = "filter.vanishingPoint";

    /// Shows the workspace for this pass.
    ///
    /// A [`FullWindowDialog`] in its own window (the main window's full-window layer where the platform has no
    /// viewports), titled by [`FullWindowTitle`] (the subject and the zoom), in the scope of
    /// [`VanishingPointCommands`], with the narrow side column (the preview's status, OK and Cancel at its bottom). The
    /// body, in order: the commands; [`VanishingPointBar`]; the tool strip (a `ToolRail`; a pick cancels the old tool's
    /// gesture with `Removed`); the canvas ([`FullWindowCanvas`]) with the source's image (the stamp preview while
    /// strokes exist), the paste as an overlay mapped through the selected plane, the planes control over it, and the
    /// editor's tool (or Hand, Zoom). The New Plane form shows while open. OK is refused while a plane is invalid and
    /// answers the command for the session as it is after this pass's edits. A second pass in one frame reports only
    /// the events of its own input: the first pass drains the queue of calls made outside a pass.
    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<VanishingPointEvent, VanishingPointCommand> {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1230 `options_bar`, :1045 `keys`, :1068 `escape`;
        // distort_ui.rs:352-368 (the mode on the main canvas, replaced by the frame).
        let WorkspaceCx { source, revision: source_revision, open, .. } = cx;
        let scene = source.scene();
        let env = Env::of(ui);
        let words = env.words::<VanishingPointWords>();
        let common = env.words::<CommonWords>();
        let title = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);
        let status: RefCell<Option<Pending<PreviewDone, String>>> = RefCell::new(None);
        let events = RefCell::new(Vec::<VanishingPointEvent>::new());
        let shown_canvas = std::cell::Cell::new(None);
        let this = RefCell::new(&mut *self);

        let shown = FullWindowDialog::new(title)
            .own_window(WindowSpec::dialog())
            .opened(open)
            .command_scope(VanishingPointCommands::SCOPE)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .settle_buttons(|_, buttons| {
                // Read after this pass's edits: OK is disabled while a plane is invalid.
                let blocked = this.borrow().blocked(scene);
                for button in buttons.iter_mut().filter(|button| button.role() == DialogRole::Accept) {
                    button.set_enabled(!blocked);
                }
            })
            .sides(|ui, _| {
                FullWindowSide::new(words.planes.clone()).headerless().show(ui, |ui, _| {
                    if let Some(status) = status.borrow().as_ref() {
                        FullWindowStatus::of(status).show(ui);
                    }
                });
            })
            .show(ui, |ui, cx| {
                let ctx = ui.ctx().clone();
                let mut guard = this.borrow_mut();
                let work: &mut VanishingPointWorkspace = &mut guard;
                let mut state = std::mem::take(cx.keep::<VanishingView>());
                let mut out = Vec::new();

                // An open drag is cancelled first, so the step finds the session as the history has it.
                while let Some(step) = cx.take_step(work.history.can_undo(), work.history.can_redo()) {
                    // pc: crates/ui-egui/src/dialogs/vp_ui.rs:383 `step`.
                    work.cancel_gesture(&ctx, &state, CancelReason::UndoDuringGesture, &mut out);
                    work.step_now(step);
                }
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    state.view.apply(command);
                }
                while let Some((command, fired)) = cx.take_command::<VanishingPointCommands>() {
                    if command == VanishingPointCommands::NewPlane {
                        work.new_plane.open = true;
                        continue;
                    }
                    // A tool key's release picks nothing.
                    let Some(pick) = command.pick().filter(|_| fired.phase != FirePhase::Release) else { continue };
                    let slot = state.slots.iter().find(|slot| slot.tools.iter().any(|entry| entry.id == pick));
                    let tools: Vec<_> = slot.into_iter().flat_map(|slot| slot.tools.iter()).collect();
                    let current = Pick::of(work.strip, work.editor.tool());
                    let picked = slot.and_then(|slot| {
                        tool_pick(&tools, slot.id, &current, &state.last_used, PickStep::of(&fired), |_| true).map(|entry| (slot.id, entry.id))
                    });
                    if let Some((slot_id, pick)) = picked {
                        work.pick(&ctx, &state, pick, &mut out);
                        state.last_used.set(slot_id, pick);
                    }
                }

                let bar = VanishingPointBar::new(work.editor.tool(), &work.settings, work.session.paste, source.paste().is_some())
                    .has_strokes(!work.session.strokes.is_empty())
                    .show(ui);
                if let Some(settings) = bar.edit.value() {
                    work.settings = *settings;
                }
                for event in bar.paste_width {
                    let at = work.session.paste.at;
                    if let Some(width) = event.value() {
                        work.session.paste.width = *width;
                    }
                    out.push(VanishingPointEvent::Paste(event.map(|width| PasteImage::new(at, width))));
                }

                let strokes = !work.session.strokes.is_empty();
                let preview = strokes.then(|| {
                    let request = VanishingPreviewRequest { session: work.session.clone() };
                    cx.pending((work.revision, source_revision, "vanishing-point-preview"), || source.preview_job(&request)).clone()
                });
                *status.borrow_mut() = preview;

                let image: &dyn RasterSource = match source.stamp_preview().filter(|_| strokes) {
                    Some(preview) => preview,
                    None => source.image(),
                };
                let current = Pick::of(work.strip, work.editor.tool());
                let document = [f64::from(work.size[0]), f64::from(work.size[1])];
                let mut chosen = None;
                let VanishingPointWorkspace { editor, session, settings, hand, zoom, checkerboard, navigation, strip, .. } = &mut *work;
                let (checkerboard, navigation, strip) = (*checkerboard, *navigation, *strip);
                let pass = editor.pass(&ctx, session, settings, scene, source);
                let paste = source.paste().filter(|_| !strokes || source.stamp_preview().is_none()).zip(pass.paste_placement());

                let shown = Row::new()
                    .fill()
                    .show(ui, |ui, _| {
                        let rail = ToolRail::new(words.tools.clone(), &state.slots, &current, &state.last_used).show_column(ui, |_, _| {});
                        if let Some(next) = rail.last_used {
                            *state.last_used = next;
                        }
                        chosen = rail.select.filter(|pick| *pick != current);
                        let mut canvas = FullWindowCanvas::new(Bind::bound(&mut state.view), image)
                            .checkerboard(checkerboard)
                            .navigation(navigation)
                            .controls(|ui, rect| {
                                pass.planes().show_at(ui, rect);
                            });
                        if let Some((paste, (frame, placed))) = paste {
                            canvas = canvas.overlay(RasterOverlay::new(paste).mapped(frame, PASTE_CELLS).placed(placed).tint(PASTE_TINT));
                        }
                        match strip {
                            VanishingPointStripTool::Hand => canvas.tool(&mut *hand).show(ui),
                            VanishingPointStripTool::Zoom => canvas.tool(&mut *zoom).show(ui),
                            _ => canvas.tool(&mut pass.tool()).show(ui),
                        }
                    })
                    .inner;
                if let Some(pick) = chosen {
                    work.pick(&ctx, &state, pick, &mut out);
                }
                if work.new_plane.open {
                    let NewPlaneState { open, values } = &mut work.new_plane;
                    if let Some(corners) = new_plane::show(ui, document, Bind::bound(values), Bind::bound(open)) {
                        let VanishingPointWorkspace { editor, session, .. } = &mut *work;
                        editor.add_plane(session, scene, corners, words.new_plane.as_str());
                    }
                }
                state.canvas = shown.as_ref().map(|canvas| canvas.response.id);
                shown_canvas.set(state.canvas);
                work.zoom_percent = state.view.zoom_percent();
                out.extend(work.editor.take_events());
                events.borrow_mut().extend(out);
                *cx.keep::<VanishingView>() = state;
            });
        let this = this.into_inner();
        let passed = events.into_inner();
        this.record(&passed, words.source_step.as_str());
        let mut events = std::mem::take(&mut this.queued);
        events.extend(passed);
        if this.settings != this.reported {
            this.reported = this.settings;
            events.push(VanishingPointEvent::Settings(this.settings));
        }
        events.extend(this.editor.take_status().map(VanishingPointEvent::Refused));
        let result = shown.output.result(|| this.command(source.paste().is_some()));
        WorkspacePass::new(shown.output.response.clone(), events).result(result).canvas(shown_canvas.get())
    }

    fn json_fields(&self, _source: &Self::Source) -> Option<Value> {
        let mut fields = serde_fields(&self.settings)?;
        let map = fields.as_object_mut()?;
        let selected = self.editor.selected().map_or(Value::Null, Value::from);
        let source = self.session.source.map_or(Value::Null, |at| Value::from(at.to_vec()));
        map.insert("tool".to_owned(), serde_json::to_value(self.editor.tool()).ok()?);
        map.insert("select".to_owned(), selected);
        map.insert("planes".to_owned(), serde_json::to_value(command_planes(&self.session)).ok()?);
        map.insert("pasteAt".to_owned(), Value::from(self.session.paste.at.to_vec()));
        map.insert("pasteWidth".to_owned(), Value::from(self.session.paste.width));
        map.insert("source".to_owned(), source);
        map.insert("clone".to_owned(), serde_json::to_value(command_strokes(&self.session)).ok()?);
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &egui::Context, source: &Self::Source, field: &str, value: Value) -> Result<(), String> {
        let words = Env::of_root(ctx).words::<VanishingPointWords>();
        match field {
            "tool" => {
                let tool = serde_json::from_value::<VanishingTool>(value.clone()).map_err(|_| rejects(field, &value, "edit|create|stamp"))?;
                self.tool(tool);
            }
            "select" => {
                let index = match &value {
                    Value::Null => None,
                    other => Some(other.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| rejects(field, &value, "a plane index"))?),
                };
                self.editor.select(index, &self.session);
            }
            "planes" => {
                let (planes, torn) = read_planes(&value).map_err(|detail| rejects(field, &value, &detail))?;
                self.write_session(words.edit_planes_step.as_str(), |work| {
                    work.session.planes = if planes.is_empty() { planes } else { source.scene().resolve(&planes, &torn) };
                    work.session.torn = torn;
                    work.editor.select(work.session.planes.len().checked_sub(1), &work.session);
                });
            }
            "pasteAt" => {
                let at = read_paste_at(&value).ok_or_else(|| rejects(field, &value, "[x, y]"))?;
                self.write_session(words.paste_step.as_str(), |work| work.session.paste.at = at);
            }
            "pasteWidth" => {
                let width = read_paste_width(&value).ok_or_else(|| rejects(field, &value, "a number"))?;
                self.write_session(words.paste_width_step.as_str(), |work| work.session.paste.width = width);
            }
            "source" => {
                let at = match &value {
                    Value::Null => None,
                    other => Some(point(other).ok_or_else(|| rejects(field, &value, "[x, y]"))?),
                };
                self.write_session(words.source_step.as_str(), |work| {
                    work.session.source = at;
                    work.session.aligned_from = None;
                });
            }
            "clone" => {
                let strokes = read_strokes(&value, self.editor.limits).map_err(|detail| rejects(field, &value, &detail))?;
                self.write_session(words.stamp_step.as_str(), |work| work.session.strokes = strokes);
            }
            _ => {
                // The stamp options merge into the current ones, as the bar edits one at a time.
                let value = match (field == "stamp", value) {
                    (true, Value::Object(patch)) => {
                        let mut merged = serde_json::to_value(self.settings.stamp).map_err(|e| e.to_string())?;
                        if let Some(map) = merged.as_object_mut() {
                            map.extend(patch);
                        }
                        merged
                    }
                    (_, value) => value,
                };
                let mut next = self.settings;
                set_serde_field(&mut next, field, value)?;
                if next != self.settings {
                    self.settings = next;
                    self.reported = next;
                    self.queued.push(VanishingPointEvent::Settings(next));
                }
            }
        }
        Ok(())
    }

    fn confirm(&mut self, _ctx: &egui::Context, source: &Self::Source) -> Option<VanishingPointCommand> {
        if self.blocked(source.scene()) {
            return None;
        }
        Some(self.command(source.paste().is_some()))
    }
}

/// The strip: Edit Plane (V), Create Plane (C), Stamp (S), a gap, Hand (H) and Zoom (Z); each slot runs the tool's
/// command, which has its label, icon and letter.
fn slots() -> Vec<ToolSlot<Pick>> {
    [Pick::EditPlane, Pick::CreatePlane, Pick::Stamp, Pick::Hand, Pick::Zoom]
        .into_iter()
        .zip(0_u32..)
        .map(|(pick, id)| {
            let tools = vec![RailTool::new(pick, VanishingPointCommands::of(pick))];
            if pick == Pick::Stamp { ToolSlot::ending_group(id, tools) } else { ToolSlot::new(id, tools) }
        })
        .collect()
}

impl VanishingPointWorkspace {
    /// Steps the history now; the editor drops what the old session left (pending corners, a stale selection).
    fn step_now(&mut self, step: Step) {
        if self.history.step(&mut self.session, step) {
            self.editor.after_step(&self.session);
            self.recorded = self.session.clone();
            self.revision += 1;
        }
    }

    /// Cancels the gesture the strip's tool holds with `reason`: the editor puts back what the press found (the
    /// Hand's and Zoom's drags go through the host).
    fn cancel_gesture(&mut self, ctx: &egui::Context, state: &VanishingView, reason: CancelReason, out: &mut Vec<VanishingPointEvent>) {
        match (self.strip, state.canvas) {
            (VanishingPointStripTool::Hand, Some(canvas)) => {
                ToolHost::new().cancel(ctx, canvas, &mut self.hand, reason);
            }
            (VanishingPointStripTool::Zoom, Some(canvas)) => {
                ToolHost::new().cancel(ctx, canvas, &mut self.zoom, reason);
            }
            _ => self.editor.cancel(&mut self.session, reason),
        }
        out.extend(self.editor.take_events());
    }

    /// Switches to `pick`, cancelling the old tool's gesture first.
    fn pick(&mut self, ctx: &egui::Context, state: &VanishingView, pick: Pick, out: &mut Vec<VanishingPointEvent>) {
        if pick == Pick::of(self.strip, self.editor.tool()) {
            return;
        }
        self.cancel_gesture(ctx, state, CancelReason::Removed, out);
        let (strip, tool) = pick.picks();
        self.strip = strip;
        if let Some(tool) = tool {
            self.editor.set_tool(tool);
        }
    }

    /// Takes this pass's events into the history: a gesture's Begin opens a step, its Commit closes it, a Set is a
    /// step of its own, and a change with no event (Alt-click sets the clone source) is a step by itself.
    fn record(&mut self, events: &[VanishingPointEvent], source_step: &str) {
        for event in events {
            match event {
                VanishingPointEvent::Planes(event) => self.record_event(event),
                VanishingPointEvent::Paste(event) => self.record_event(event),
                VanishingPointEvent::Stamp(event) => self.record_event(event),
                // Settings and refusals are not history steps.
                VanishingPointEvent::Settings(_) | VanishingPointEvent::Refused(_) => {}
            }
        }
        if !self.history.is_open() {
            if self.session != self.recorded {
                let name: Arc<str> = Arc::from(source_step);
                self.history.record(&EditEvent::set(Mixed::Same(self.recorded.clone()), self.session.clone(), name, None));
                self.revision += 1;
            }
            self.recorded = self.session.clone();
        }
    }

    fn record_event<T>(&mut self, event: &EditEvent<T>) {
        let before = Mixed::Same(self.recorded.clone());
        match event {
            EditEvent::Begin { id, label, .. } => self.history.record(&EditEvent::begin(*id, before, label.clone(), None)),
            EditEvent::Commit { id, .. } => {
                self.history.record(&EditEvent::commit(*id, before, self.session.clone()));
                self.recorded = self.session.clone();
                self.revision += 1;
            }
            EditEvent::Cancel { id, reason, .. } => self.history.record(&EditEvent::cancel(*id, *reason, None)),
            EditEvent::Set { label, .. } => {
                self.history.record(&EditEvent::set(before, self.session.clone(), label.clone(), None));
                self.recorded = self.session.clone();
                self.revision += 1;
            }
            _ => {}
        }
    }
}
