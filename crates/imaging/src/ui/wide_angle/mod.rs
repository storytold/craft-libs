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
mod lines;
mod panel;
mod session;
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
use craft_ui::contract::{Bind, Pattern, Remember as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView};
use craft_ui::display::Divider;
use craft_ui::layout_components::Row;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::patterns::form_dialogs::{Param, ParamDialog, ParamKind};
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, Fired};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::pending::{Job, Pending};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Id, Rect, Ui};

use crate::ui::live_preview::settings_hash;

/// The tools of the strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
}

impl PreviewDone {
    /// A finished preview that took `timing`.
    pub fn new(timing: Duration) -> Self {
        Self { timing }
    }
}

/// What the app gives the workspace to show each pass.
pub trait WideAngleSource {
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

/// What [`WideAngleWorkspace::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct WideAngleOutput {
    /// What happened this pass, in order.
    pub events: Vec<WideAngleEvent>,
    /// How the dialog closed this pass: OK is [`CloseReason::Activated`] or [`CloseReason::Enter`]; Cancel and Escape
    /// are the other reasons.
    pub closed: Option<CloseReason>,
    /// The workspace's screen rectangle.
    pub rect: Rect,
    /// How long the solve and warp of the preview now shown took; the app may show it in its status bar.
    pub timing: Option<Duration>,
}

impl WideAngleOutput {
    /// Whether the dialog closed with OK.
    pub fn accepted(&self) -> bool {
        matches!(self.closed, Some(CloseReason::Activated | CloseReason::Enter))
    }
}

/// The state the dialog body keeps between passes: what the app does not read.
#[derive(Clone, Default)]
struct Body {
    view: Option<FullWindowView>,
    tool: WideAngleTool,
    last_used: LastUsed<WideAngleTool>,
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
/// OK; Esc Cancel (a drag in progress first). Undo and redo are the app's Edit commands, forwarded to
/// [`step`](Self::step).
#[derive(Pattern)]
pub struct WideAngleWorkspace {
    session: WideAngleSession,
    history: EditHistory<WideAngleSession>,
    subject: String,
    size: [u32; 2],
    /// Preview: the corrected image (on) or the source (off); the source shows while a drag runs.
    preview_on: bool,
    /// What the preview job reported for the session now shown.
    timing: Option<Duration>,
    /// The title's zoom as of the last pass.
    zoom_percent: f64,
    /// The user's navigation options for the canvas, Hand and Zoom.
    navigation: NavigationOptions,
    /// The panel's Add button was pressed and the form is not open yet.
    add_requested: bool,
    steps: Vec<Step>,
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
            timing: None,
            zoom_percent: 100.0,
            navigation: NavigationOptions::default(),
            add_requested: false,
            steps: Vec::new(),
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

    /// Undo or redo from the app's Edit menu; returns whether it will step. The next [`show`](Self::show) ends a drag
    /// in progress first, then steps.
    pub fn step(&mut self, step: Step) -> bool {
        let can = match step {
            Step::Undo => self.history.can_undo(),
            Step::Redo => self.history.can_redo(),
            _ => false,
        };
        // The body ends a drag in its own pass, so a step is queued even when there is nothing to undo.
        self.steps.push(step);
        can
    }

    /// Shows the workspace for this pass: the title, the strip, the side column ([`WideAnglePanel`], OK and Cancel),
    /// the canvas (the corrected preview while Preview is on and no drag runs, else the source) with the Constraint
    /// tool (or Hand, Zoom) and the [`ConstraintLines`], and the constraint menu while a constraint asked for it.
    /// Edits go through [`WideAngleSession::apply`]; a session the preview job has not seen starts one.
    ///
    /// The canvas is in the source's pixels and the constraints in document pixels (`open.size`); the tool and the
    /// lines convert between them, so the source may be a proxy of the layer.
    pub fn show(&mut self, ui: &mut Ui, geometry: &dyn WideAngleGeometry, source: &dyn WideAngleSource) -> WideAngleOutput {
        let env = Env::of(ui);
        let words = env.words::<WideAngleWords>();
        let title: Label = FullWindowTitle::new(words.name.clone()).subject(self.subject.clone()).zoom(self.zoom_percent).label(&env);

        let snapshot = self.session.clone();
        let size = self.size;
        let navigation = self.navigation;
        let steps = std::mem::take(&mut self.steps);
        let ending = !steps.is_empty();
        let preview_on = Cell::new(self.preview_on);
        let actions = RefCell::new(Vec::<Action>::new());
        let timing = Cell::new(self.timing);
        let zoom_percent = Cell::new(self.zoom_percent);
        let add = Cell::new(self.add_requested);
        let common = env.words::<CommonWords>();
        let ctx = ui.ctx().clone();

        let shown = FullWindowDialog::new(title)
            .command_scope(WideAngleCommands::SCOPE)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .sides(|ui, cx| {
                let mut preview = preview_on.get();
                let status = cx.pending(settings_hash(&[format!("{snapshot:?}").as_bytes()]), || source.preview_job(&snapshot)).clone();
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
                if let Pending::Ready(done) = &status {
                    timing.set(Some(done.timing));
                }
            })
            .show(ui, |ui, cx| {
                let mut body = cx.keep::<Body>().clone();
                if add.replace(false) {
                    body.adding = true;
                }
                if ending {
                    end_gesture(&ctx, &mut body, CancelReason::UndoDuringGesture);
                }

                let slots: Vec<ToolSlot<WideAngleTool>> =
                    TOOL_SLOT.iter().zip(0..).map(|(&(tool, command), slot)| ToolSlot::new(slot, vec![RailTool::new(tool, command)])).collect();
                let mut next = None;
                while let Some((command, fired)) = cx.take_command::<WideAngleCommands>() {
                    next = pick(&slots, command, &fired, &body).or(next);
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
                    let rail = ToolRail::new(words.tools.clone(), &slots, &body.tool, &body.last_used).show_rail(ui, |_, _| {});
                    next = rail.select.or(next);
                    if let Some(last_used) = rail.last_used {
                        body.last_used = last_used;
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
        self.timing = timing.get();
        self.zoom_percent = zoom_percent.get();
        for step in steps {
            self.history.step(&mut self.session, step);
        }
        let mut events = Vec::new();
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
                Action::Constraint(edit) => {
                    let before = self.session.clone();
                    if self.session.apply(edit.clone()) {
                        let name = Arc::<str>::from(edit_name(&words, &edit));
                        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name, None));
                        events.push(WideAngleEvent::Constraints(edit));
                    }
                }
            }
        }
        WideAngleOutput { events, closed: shown.output.closed, rect: shown.output.response.rect, timing: self.timing }
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

/// The tool a fired tool command picks, through its slot's pick rule.
fn pick(slots: &[ToolSlot<WideAngleTool>], command: WideAngleCommands, fired: &Fired, body: &Body) -> Option<WideAngleTool> {
    let tool = TOOL_SLOT.iter().find(|(_, c)| *c == command)?.0;
    let slot = slots.iter().find(|slot| slot.tools.iter().any(|t| t.id == tool))?;
    let visible: Vec<&RailTool<WideAngleTool>> = slot.tools.iter().collect();
    tool_pick(&visible, slot.id, &body.tool, &body.last_used, PickStep::of(fired), |_| true).map(|t| t.id)
}

/// Ends the gesture the active tool holds on the canvas, through the host that runs it, before the tool is replaced or
/// the history steps.
fn end_gesture(ctx: &egui::Context, body: &mut Body, reason: CancelReason) {
    let Some(canvas) = body.canvas else { return };
    match body.tool {
        WideAngleTool::Constraint => ToolHost::cancel(ctx, canvas, &mut body.constraint, reason),
        WideAngleTool::Hand => body.hand.as_mut().is_some_and(|hand| ToolHost::cancel(ctx, canvas, hand, reason)),
        WideAngleTool::Zoom => body.zoom.as_mut().is_some_and(|zoom| ToolHost::cancel(ctx, canvas, zoom, reason)),
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
