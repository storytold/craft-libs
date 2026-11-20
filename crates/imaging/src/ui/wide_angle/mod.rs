//! The Adaptive Wide Angle workspace (Filter › Adaptive Wide Angle…), in Photoshop's layout.
//!
//! [`WideAngleWorkspace`] is a [`FullWindowDialog`] with the title strip "Adaptive Wide Angle (layer, 66%)", the tool
//! strip (Constraint A, Hand H, Zoom Z; Photoshop's set without Polygon Constraint and Move), the canvas, and a
//! Standard side column with [`WideAnglePanel`] and OK and Cancel.
//!
//! The session ([`WideAngleSession`]) is `filter.adaptiveWideAngle`'s parameters with [`EditHistory`] undo. The
//! app's [`WideAngleGeometry`] curves the constraints by its lens model, [`WideAngleSource`] shows the source and
//! the corrected preview and starts the job that solves and warps it for a session. OK runs the command with the
//! session.
//!
//! Files: everything here belongs to the Wide Angle workspace.
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`EditHistory`]: craft_behavior::EditHistory

mod commands;
pub(crate) mod lines;
pub(crate) mod panel;
mod session;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod tool;
mod words;

pub use commands::WideAngleCommands;
pub use lines::{ConstraintLines, ConstraintLinesOutput, ConstraintLinesStyle};
pub use panel::{WideAnglePanel, WideAnglePanelOutput};
pub use session::{ConstraintOrientation, WideAngleConstraint, WideAngleConstraintEdit, WideAngleGeometry, WideAngleModel, WideAngleSession};
pub use tool::ConstraintTool;
pub use words::WideAngleWords;

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Duration;

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, EditEvent, EditHistory, Mixed};
use craft_ui::canvas::{CanvasTool, HandTool, NavigationOptions, PickStep, RasterSource, ToolHost, ViewCommands, ZoomTool, tool_pick};
use craft_ui::contract::{Bind, Pattern, Remember as _, Show as _, ShowContainer as _, Tracked};
use craft_ui::dialog::{
    FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, FullWindowWorkspace,
    WorkspaceCx, WorkspacePass, serde_fields, set_serde_field,
};
use craft_ui::display::Divider;
use craft_ui::layout_components::Row;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::patterns::form_dialogs::{Param, ParamDialog, ParamKind};
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase, Fired};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::pending::Job;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Id, Ui};
use serde_json::Value;

use crate::ui::live_preview::settings_hash;

/// The tools of the strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui::contract::MemoProps, craft_ui::contract::PlainData)]
#[non_exhaustive]
pub enum WideAngleTool {
    /// A.
    #[default]
    Constraint,
    /// H.
    Hand,
    /// Z.
    Zoom,
}

/// What a finished preview job reports: how long the solve and warp took. The picture stays the app's, shown through
/// [`WideAngleSource::corrected`]. Every full-window workspace's preview job delivers this.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PreviewDone {
    /// How long the solve and warp took.
    pub timing: Duration,
    /// What rendered it ("GPU", "CPU"), shown with the time.
    pub backend: Option<Label>,
}

impl PreviewDone {
    /// Names what rendered the preview.
    #[must_use]
    pub fn backend(mut self, label: impl Into<Label>) -> Self {
        self.backend = Some(label.into());
        self
    }

    /// A finished preview that took `timing`.
    pub fn new(timing: Duration) -> Self {
        Self { timing, backend: None }
    }
}

/// What the app gives the workspace to show each pass.
pub trait WideAngleSource: Send + Sync {
    /// The lens model the constraints curve by.
    fn geometry(&self) -> &dyn WideAngleGeometry;

    /// The layer as it is (a proxy of at most 800 px in PhotoCraft).
    fn source(&self) -> &dyn RasterSource;

    /// The last ready corrected preview; `None` until one is ready.
    fn corrected(&self) -> Option<&dyn RasterSource>;

    /// The job that solves and warps the corrected preview for `session`. The workspace starts it once per session
    /// and shows [`corrected`](Self::corrected) when it is ready.
    fn preview_job(&self, session: &WideAngleSession) -> Job<PreviewDone, String>;
}

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct WideAngleOpen {
    /// The layer's size in document pixels.
    pub size: [u32; 2],
    /// The layer's name: the title's subject.
    pub subject: String,
    /// The settings to start from (the last used, or the command's parameters from the control channel).
    pub session: WideAngleSession,
}

impl WideAngleOpen {
    /// Opens on a layer of `size` document pixels named `subject`, with the default session.
    pub fn new(size: [u32; 2], subject: impl Into<String>) -> Self {
        Self { size, subject: subject.into(), session: WideAngleSession::default() }
    }

    /// Sets the session to start from.
    #[must_use]
    pub fn session(mut self, session: WideAngleSession) -> Self {
        self.session = session;
        self
    }
}

/// What the workspace reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WideAngleEvent {
    /// A constraint added, oriented, deleted or moved; already applied to the session and recorded in its history.
    Constraints(WideAngleConstraintEdit),
}

/// The state the dialog body keeps between passes: what the app does not read.
#[derive(Clone)]
struct Body {
    view: Option<FullWindowView>,
    tool: WideAngleTool,
    last_used: Tracked<LastUsed<WideAngleTool>>,
    slots: Tracked<Vec<ToolSlot<WideAngleTool>>>,
    constraint: ConstraintTool,
    hand: Option<HandTool>,
    zoom: Option<ZoomTool>,
    /// The canvas of the last pass: a gesture is ended through the host that holds it.
    canvas: Option<Id>,
    /// The constraint the context menu is open for.
    menu: Option<usize>,
    /// The Add Constraint form is open.
    adding: bool,
}

impl Default for Body {
    fn default() -> Self {
        let slots = TOOL_SLOT.iter().zip(0..).map(|(&(tool, command), slot)| ToolSlot::new(slot, vec![RailTool::new(tool, command)])).collect();
        Body {
            view: None,
            tool: WideAngleTool::default(),
            last_used: Tracked::default(),
            slots: Tracked::new(slots),
            constraint: ConstraintTool::default(),
            hand: None,
            zoom: None,
            canvas: None,
            menu: None,
            adding: false,
        }
    }
}

/// A change the passes ask of the session, applied after the dialog has been shown.
enum Action {
    /// The panel's correction edit.
    Panel(EditEvent<WideAngleSession>),
    /// A constraint edit from the canvas, the tool, the menu or the form.
    Constraint(WideAngleConstraintEdit),
}

/// The slot of the strip's tools.
const TOOL_SLOT: [(WideAngleTool, WideAngleCommands); 3] = [
    (WideAngleTool::Constraint, WideAngleCommands::Constraint),
    (WideAngleTool::Hand, WideAngleCommands::Hand),
    (WideAngleTool::Zoom, WideAngleCommands::Zoom),
];

/// The Adaptive Wide Angle workspace.
///
/// Keys: A, H, Z (the tool commands); Delete and Backspace the focused constraint; the zoom commands the view; Enter
/// OK; Esc Cancel (a drag in progress first). ⌘Z and ⇧⌘Z step the history, ending a drag in progress first (a field
/// being edited steps its own text first).
#[derive(Pattern)]
pub struct WideAngleWorkspace {
    session: WideAngleSession,
    history: EditHistory<WideAngleSession>,
    subject: String,
    size: [u32; 2],
    /// Preview: the corrected image (on) or the source (off); the source shows while a drag runs.
    preview_on: bool,
    /// The title's zoom as of the last pass.
    zoom_percent: f64,
    /// The user's navigation options for the canvas, Hand and Zoom.
    navigation: NavigationOptions,
    /// The panel's Add button was pressed and the form is not open yet.
    add_requested: bool,
    /// Events from calls outside `show`; the next pass reports them first.
    queued: Vec<WideAngleEvent>,
}

impl WideAngleWorkspace {
    /// A workspace on the layer and session that `open` describes.
    pub fn new(open: WideAngleOpen) -> Self {
        Self {
            history: EditHistory::new(&open.session),
            session: open.session,
            subject: open.subject,
            size: open.size,
            preview_on: true,
            zoom_percent: 100.0,
            navigation: NavigationOptions::default(),
            add_requested: false,
            queued: Vec::new(),
        }
    }

    /// Sets the Preview check: the corrected picture (on, the default) or the source.
    pub fn preview(&mut self, on: bool) {
        self.preview_on = on;
    }

    /// The view's zoom in percent as of the last pass.
    pub fn zoom_percent(&self) -> f64 {
        self.zoom_percent
    }

    /// The user's navigation options (wheel, flick, animated zoom); Photoshop's by default.
    pub fn navigation(&mut self, navigation: NavigationOptions) {
        self.navigation = navigation;
    }

    /// The correction settings as they are now.
    pub fn session(&self) -> &WideAngleSession {
        &self.session
    }

    /// Undoes the last step; `false` when there is none.
    pub fn undo(&mut self) -> bool {
        self.history.step(&mut self.session, Step::Undo)
    }

    /// Redoes the last undone step; `false` when there is none.
    pub fn redo(&mut self) -> bool {
        self.history.step(&mut self.session, Step::Redo)
    }

    /// Applies a constraint edit to the session and records it as one history step named by `edit_name`. `None` when
    /// the edit changed nothing (an index past the list, an orientation it already has).
    fn edit_constraints(&mut self, words: &WideAngleWords, edit: WideAngleConstraintEdit) -> Option<WideAngleEvent> {
        let before = self.session.clone();
        if !self.session.apply(edit.clone()) {
            return None;
        }
        let name = Arc::<str>::from(edit_name(words, &edit));
        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name, None));
        Some(WideAngleEvent::Constraints(edit))
    }
}

/// An index a json value holds.
fn json_index(value: &Value) -> Option<usize> {
    value.as_u64().and_then(|index| usize::try_from(index).ok())
}

/// The constraint edit the action key `field` names, from its json `value`; `None` when the value does not fit.
fn json_edit(field: &str, value: &Value) -> Option<WideAngleConstraintEdit> {
    match field {
        "add" => serde_json::from_value(value.clone()).ok().map(WideAngleConstraintEdit::Add),
        "remove" => json_index(value).map(WideAngleConstraintEdit::Delete),
        "orient" => {
            let index = json_index(value.get("index")?)?;
            let orientation = serde_json::from_value(value.get("orientation")?.clone()).ok()?;
            Some(WideAngleConstraintEdit::Orient { index, orientation })
        }
        _ => None,
    }
}

impl FullWindowWorkspace for WideAngleWorkspace {
    type Source = dyn WideAngleSource;
    type Event = WideAngleEvent;
    type Result = WideAngleSession;

    const KEY: &'static str = "filter.adaptiveWideAngle";

    /// Shows the workspace for this pass: the title, the strip, the side column ([`WideAnglePanel`], OK and Cancel),
    /// the canvas (the corrected preview while Preview is on and no drag runs, else the source) with the Constraint
    /// tool (or Hand, Zoom) and the [`ConstraintLines`], and the constraint menu while a constraint asked for it.
    /// Edits go through [`WideAngleSession::apply`]; a session the preview job has not seen starts one. OK answers
    /// the session as it is after this pass's edits.
    ///
    /// The canvas is in the source's pixels and the constraints in document pixels (`open.size`); the tool and the
    /// lines convert between them, so the source may be a proxy of the layer. A second pass in one frame reports
    /// only the events of its own input: the first pass drains the queue of calls made outside a pass.
    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<WideAngleEvent, WideAngleSession> {
        let WorkspaceCx { source, revision, open, .. } = cx;
        let geometry = source.geometry();
        let env = Env::of(ui);
        let words = env.words::<WideAngleWords>();
        let title: Label = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);

        let snapshot = self.session.clone();
        let size = self.size;
        let navigation = self.navigation;
        let can_step = (self.history.can_undo(), self.history.can_redo());
        let steps = RefCell::new(Vec::<Step>::new());
        let preview_on = Cell::new(self.preview_on);
        let actions = RefCell::new(Vec::<Action>::new());
        let zoom_percent = Cell::new(self.zoom_percent);
        let shown_canvas = Cell::new(None);
        let add = Cell::new(self.add_requested);
        let common = env.words::<CommonWords>();
        let ctx = ui.ctx().clone();

        let shown = FullWindowDialog::new(title)
            .opened(open)
            .command_scope(WideAngleCommands::SCOPE)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .sides(|ui, cx| {
                let mut preview = preview_on.get();
                let key = settings_hash(&[format!("{snapshot:?}").as_bytes(), &revision.to_le_bytes()]);
                let status = cx.pending(key, || source.preview_job(&snapshot)).clone();
                FullWindowSide::new(words.settings.clone()).show(ui, |ui, _| {
                    let panel = WideAnglePanel::new(&snapshot, Bind::bound(&mut preview)).show(ui);
                    let mut queued = actions.borrow_mut();
                    queued.extend(panel.edit.into_iter().map(Action::Panel));
                    queued.extend(panel.constraint.map(Action::Constraint));
                    if panel.add {
                        add.set(true);
                    }
                    FullWindowStatus::of(&status).show(ui);
                });
                preview_on.set(preview);
            })
            .show(ui, |ui, cx| {
                let mut body = cx.keep::<Body>().clone();
                if add.replace(false) {
                    body.adding = true;
                }
                // A drag in progress ends first; the steps run after the dialog, before this pass's edits.
                let mut stepping = false;
                while let Some(step) = cx.take_step(can_step.0, can_step.1) {
                    steps.borrow_mut().push(step);
                    stepping = true;
                }
                if stepping {
                    end_gesture(&ctx, &mut body, CancelReason::UndoDuringGesture);
                }

                let mut next = None;
                while let Some((command, fired)) = cx.take_command::<WideAngleCommands>() {
                    next = pick(command, &fired, &body).or(next);
                }

                let image = source.source();
                let corrected = source.corrected().filter(|_| preview_on.get() && !body.constraint.dragging());
                let shown_source: &dyn RasterSource = corrected.unwrap_or(image);
                let canvas_size = shown_source.size();
                let pixels = [canvas_size.width, canvas_size.height];
                body.constraint.picture(pixels, size);
                let draft = body.constraint.draft();
                let mut view = body.view.take().unwrap_or_else(FullWindowView::new);
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    view.apply(command);
                }

                Row::new().fill().show(ui, |ui, _| {
                    let rail = ToolRail::new(words.tools.clone(), &body.slots, &body.tool, &body.last_used).show_column(ui, |_, _| {});
                    next = rail.select.or(next);
                    if let Some(last_used) = rail.last_used {
                        *body.last_used = last_used;
                    }
                    if let Some(tool) = next.take().filter(|tool| *tool != body.tool) {
                        end_gesture(&ctx, &mut body, CancelReason::Removed);
                        body.tool = tool;
                    }

                    let nav_hand = navigation;
                    let active: &mut dyn CanvasTool = match body.tool {
                        WideAngleTool::Constraint => &mut body.constraint,
                        WideAngleTool::Hand => {
                            let tool = body.hand.get_or_insert_with(|| HandTool::new(nav_hand));
                            tool.options = nav_hand;
                            tool
                        }
                        WideAngleTool::Zoom => {
                            let tool = body.zoom.get_or_insert_with(|| ZoomTool::new(nav_hand));
                            tool.options = nav_hand;
                            tool
                        }
                    };
                    let mut lines_out = None;
                    let canvas =
                        FullWindowCanvas::new(Bind::bound(&mut view), shown_source).navigation(navigation).tool(active).controls(|ui, rect| {
                            let mut lines = ConstraintLines::new(words.constraint_lines.clone(), &snapshot, geometry)
                                .picture(pixels, size)
                                .corrected(corrected.is_some());
                            if let Some(draft) = draft {
                                lines = lines.draft(draft);
                            }
                            lines_out = Some(lines.show_at(ui, rect));
                        });
                    let out = canvas.show(ui);
                    zoom_percent.set(view.zoom_percent());
                    body.canvas = Some(out.response.id);
                    shown_canvas.set(body.canvas);

                    let mut queued = actions.borrow_mut();
                    queued.extend(body.constraint.take_edits().into_iter().map(Action::Constraint));
                    if let Some(lines) = lines_out {
                        queued.extend(lines.edit.map(Action::Constraint));
                        if lines.context.is_some() {
                            body.menu = lines.context;
                        }
                    }
                    drop(queued);

                    if let Some(index) = body.menu.filter(|&i| i < snapshot.constraints.len()) {
                        let current = snapshot.constraints[index].orientation;
                        let mut open = true;
                        let mut chosen = None;
                        let menu = Menu::context(words.constraint_lines.clone(), &out.response).open(Bind::bound(&mut open)).show(ui, |ui, _| {
                            for (orientation, label) in [
                                (ConstraintOrientation::Free, &words.free),
                                (ConstraintOrientation::Horizontal, &words.horizontal),
                                (ConstraintOrientation::Vertical, &words.vertical),
                            ] {
                                if MenuItem::new(label.clone()).radio_mark(orientation == current).show(ui).ran {
                                    chosen = Some(WideAngleConstraintEdit::Orient { index, orientation });
                                }
                            }
                            Divider::horizontal().show(ui);
                            if MenuItem::new(words.delete.clone()).show(ui).ran {
                                chosen = Some(WideAngleConstraintEdit::Delete(index));
                            }
                        });
                        let picked = chosen.is_some();
                        if let Some(edit) = chosen {
                            actions.borrow_mut().push(Action::Constraint(edit));
                        }
                        if picked || !open || menu.output.closed.is_some() {
                            body.menu = None;
                        }
                    }
                });

                if body.adding {
                    let params = add_constraint_params(&words);
                    let mut open = true;
                    let form =
                        ParamDialog::new("wideAngle.addConstraint", words.add_constraint.clone(), params).opened(Bind::bound(&mut open)).show(ui);
                    if let Some(result) = form.result {
                        let point = |key: &str| {
                            let [x, y] = result.fields.point(key).unwrap_or([0.0, 0.0]);
                            [x / 100.0 * f64::from(size[0]), y / 100.0 * f64::from(size[1])]
                        };
                        let orientation = match result.fields.text("orientation") {
                            Some("horizontal") => ConstraintOrientation::Horizontal,
                            Some("vertical") => ConstraintOrientation::Vertical,
                            _ => ConstraintOrientation::Free,
                        };
                        let constraint = WideAngleConstraint::new(point("start"), point("end"), orientation);
                        actions.borrow_mut().push(Action::Constraint(WideAngleConstraintEdit::Add(constraint)));
                    }
                    if !open || form.closed.is_some() {
                        body.adding = false;
                    }
                }

                body.view = Some(view);
                *cx.keep::<Body>() = body;
            });

        self.preview_on = preview_on.get();
        self.add_requested = add.get();
        self.zoom_percent = zoom_percent.get();
        for step in steps.into_inner() {
            match step {
                Step::Undo => self.undo(),
                Step::Redo => self.redo(),
                _ => false,
            };
        }
        let mut events = std::mem::take(&mut self.queued);
        let queued = actions.into_inner();
        if !queued.is_empty() {
            ui.ctx().request_repaint();
        }
        for action in queued {
            match action {
                Action::Panel(event) => {
                    self.history.record(&event);
                    match event {
                        EditEvent::Update { value, .. } | EditEvent::Set { after: value, .. } | EditEvent::Commit { after: value, .. } => {
                            self.session = value;
                        }
                        EditEvent::Cancel { before: Some(Mixed::Same(before)), .. } => self.session = before,
                        _ => {}
                    }
                }
                Action::Constraint(edit) => events.extend(self.edit_constraints(&words, edit)),
            }
        }
        let result = shown.output.result(|| self.session.clone());
        WorkspacePass::new(shown.output.response.clone(), events).result(result).canvas(shown_canvas.get())
    }

    fn json_fields(&self, _source: &Self::Source) -> Option<Value> {
        let mut fields = serde_fields(&self.session)?;
        fields.as_object_mut()?.insert("preview".to_owned(), Value::Bool(self.preview_on));
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &egui::Context, _source: &Self::Source, field: &str, value: Value) -> Result<(), String> {
        let words = Env::of_root(ctx).words::<WideAngleWords>();
        match field {
            "preview" => {
                let on = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.preview(on);
            }
            "undo" | "redo" => {
                if value != Value::Bool(true) {
                    return Err(FieldRefusal::Rejects(&value).message(field));
                }
                if field == "undo" {
                    self.undo();
                } else {
                    self.redo();
                }
            }
            "add" | "orient" | "remove" => {
                let edit = json_edit(field, &value).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                let event = self.edit_constraints(&words, edit).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.queued.push(event);
            }
            _ => {
                let mut next = self.session.clone();
                set_serde_field(&mut next, field, value)?;
                if next != self.session {
                    let name = Arc::<str>::from(words.settings.as_str());
                    self.history.record(&EditEvent::set(Mixed::Same(self.session.clone()), next.clone(), name, None));
                    self.session = next;
                }
            }
        }
        Ok(())
    }

    fn confirm(&mut self, _ctx: &egui::Context, _source: &Self::Source) -> Option<WideAngleSession> {
        Some(self.session.clone())
    }
}

/// The name an edit has in the history.
fn edit_name<'a>(words: &'a WideAngleWords, edit: &WideAngleConstraintEdit) -> &'a str {
    match edit {
        WideAngleConstraintEdit::Add(_) => words.add_constraint.as_str(),
        WideAngleConstraintEdit::Orient { .. } => words.orientation.as_str(),
        WideAngleConstraintEdit::Delete(_) => words.delete.as_str(),
        _ => words.constraint_lines.as_str(),
    }
}

/// The tool a fired tool command picks, through its slot's pick rule; a tool key's release picks nothing.
fn pick(command: WideAngleCommands, fired: &Fired, body: &Body) -> Option<WideAngleTool> {
    if fired.phase == FirePhase::Release {
        return None;
    }
    let tool = TOOL_SLOT.iter().find(|(_, c)| *c == command)?.0;
    let slot = body.slots.iter().find(|slot| slot.tools.iter().any(|t| t.id == tool))?;
    let visible: Vec<&RailTool<WideAngleTool>> = slot.tools.iter().collect();
    tool_pick(&visible, slot.id, &body.tool, &body.last_used, PickStep::of(fired), |_| true).map(|t| t.id)
}

/// Ends the gesture the active tool holds on the canvas, through the host that runs it, before the tool is replaced or
/// the history steps.
fn end_gesture(ctx: &egui::Context, body: &mut Body, reason: CancelReason) {
    let Some(canvas) = body.canvas else { return };
    match body.tool {
        WideAngleTool::Constraint => ToolHost::new().cancel(ctx, canvas, &mut body.constraint, reason),
        WideAngleTool::Hand => body.hand.as_mut().is_some_and(|hand| ToolHost::new().cancel(ctx, canvas, hand, reason)),
        WideAngleTool::Zoom => body.zoom.as_mut().is_some_and(|zoom| ToolHost::new().cancel(ctx, canvas, zoom, reason)),
    };
}

/// The rows of the Add Constraint form: the two end points (percent of the image) and the orientation.
fn add_constraint_params(words: &WideAngleWords) -> Vec<Param> {
    vec![
        Param::new("start", words.start.clone(), ParamKind::point([25.0, 50.0])),
        Param::new("end", words.end.clone(), ParamKind::point([75.0, 50.0])),
        Param::new(
            "orientation",
            words.orientation.clone(),
            ParamKind::choice(
                vec![
                    ("free".to_owned(), words.free.clone()),
                    ("horizontal".to_owned(), words.horizontal.clone()),
                    ("vertical".to_owned(), words.vertical.clone()),
                ],
                Some("free".to_owned()),
            ),
        ),
    ]
}
