//! `EditPolicy`: a slider and a text field bound to one of two documents under an edit policy, their edit events in a
//! log, and undo and document switches that interrupt a running gesture.

use craft_behavior::{CancelReason, EditEvent, EditEvents, EditPolicy as Policy, GestureId};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Gap, Pad, Pattern, Props, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::edit::cancel_all;
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::{SurfaceKind, TypeRole};
use craft_ui::text::TextField;
use craft_ui::value_drag::Slider;
use egui::Ui;
use std::collections::VecDeque;
use std::fmt::Debug;

/// Events kept in the log.
const LOG_LEN: usize = 20;

/// The slider's top value.
const LEVEL_MAX: f64 = 100.0;

/// The value a document had before an undo step, by field.
#[derive(Clone, Debug, PartialEq)]
enum Before {
    Level(f64),
    Label(String),
}

/// A document: two fields and the values its undo steps restore, newest last.
#[derive(Clone, Debug)]
struct Doc {
    name: &'static str,
    level: f64,
    label: String,
    undo: Vec<Before>,
}

impl Doc {
    fn new(name: &'static str, level: f64, label: &str) -> Self {
        Doc { name, level, label: label.to_owned(), undo: Vec::new() }
    }

    /// Takes back the newest step; false when there is none.
    fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(Before::Level(level)) => self.level = level,
            Some(Before::Label(label)) => self.label = label,
            None => return false,
        }
        true
    }
}

/// What the card keeps between passes.
#[derive(Clone, Debug)]
struct State {
    docs: [Doc; 2],
    current: usize,
    /// The gesture under way, from its `Begin` to its `Commit` or `Cancel`.
    running: Option<GestureId>,
    /// Newest first.
    log: VecDeque<String>,
}

impl Default for State {
    fn default() -> Self {
        State { docs: [Doc::new("A", 40.0, "Layer 1"), Doc::new("B", 75.0, "Background")], current: 0, running: None, log: VecDeque::new() }
    }
}

impl State {
    fn log(&mut self, line: String) {
        self.log.push_front(line);
        self.log.truncate(LOG_LEN);
    }

    /// Logs the events of `control`, follows the gesture under way, and keeps an undo step for each commit or set
    /// that changed the value.
    fn events<T: Clone + Debug + PartialEq>(&mut self, control: &str, events: &EditEvents<T>, step: impl Fn(T) -> Before) {
        for event in events {
            self.log(format!("{control}: {}", describe(event)));
            let before = match event {
                EditEvent::Begin { id, .. } => {
                    self.running = Some(*id);
                    None
                }
                EditEvent::Commit { before, after, .. } => {
                    self.running = None;
                    before.same().filter(|before| *before != after).cloned()
                }
                EditEvent::Cancel { .. } => {
                    self.running = None;
                    None
                }
                EditEvent::Set { before, .. } => before.same().cloned(),
                _ => None,
            };
            if let Some(before) = before {
                self.docs[self.current].undo.push(step(before));
            }
        }
    }

    /// Undo: during a gesture it cancels the gesture instead of taking back a step.
    fn undo(&mut self, ui: &Ui) {
        if self.running.is_some() {
            cancel_all(ui.ctx(), CancelReason::UndoDuringGesture);
            self.log(format!("undo: cancel_all({:?})", CancelReason::UndoDuringGesture));
        } else if self.docs[self.current].undo() {
            self.log("undo: one step".to_owned());
        }
    }

    /// A gesture belongs to the document it began in: it is cancelled before the switch.
    fn switch(&mut self, ui: &Ui) {
        cancel_all(ui.ctx(), CancelReason::HostChanged);
        self.current = (self.current + 1) % self.docs.len();
        let name = self.docs[self.current].name;
        self.log(format!("switched to document {name}: cancel_all({:?})", CancelReason::HostChanged));
    }
}

/// One log line, e.g. `Update #3 42.0`.
fn describe<T: Debug>(event: &EditEvent<T>) -> String {
    match event {
        EditEvent::Begin { id, .. } => format!("Begin {id:?}"),
        EditEvent::Update { id, value, .. } => format!("Update {id:?} {value:?}"),
        EditEvent::Commit { id, after, .. } => format!("Commit {id:?} {after:?}"),
        EditEvent::Cancel { id, reason, .. } => format!("Cancel {id:?} {reason:?}"),
        EditEvent::Set { after, .. } => format!("Set {after:?}"),
        _ => format!("{event:?}"),
    }
}

/// The edit lifecycle under `policy`: a slider and a text field bound to the current one of two documents, the undo
/// and switch buttons, the document's state and the log of every edit event, newest first.
#[derive(Pattern, Props)]
pub struct EditPolicy {
    policy: Policy,
}

impl EditPolicy {
    /// The demo with its controls bound under `policy`.
    pub fn new(policy: Policy) -> Self {
        EditPolicy { policy }
    }

    /// Shows the controls, the buttons, the state and the log one under the other.
    pub fn show(self, ui: &mut Ui) {
        let policy = self.policy;
        Stack::new().gap(Gap::Stack).show(ui, |ui, cx| {
            let state = cx.memory::<State>();
            Row::new().gap(Gap::Item).show(ui, |ui, _| {
                let doc = &mut state.docs[state.current];
                let level = Slider::new(Bind::bound(&mut doc.level).policy(policy), 0.0..=LEVEL_MAX).label("Level").show(ui).events;
                let label = TextField::new("Label", Bind::bound(&mut doc.label).policy(policy)).show(ui).events;
                state.events("slider", &level, Before::Level);
                state.events("field", &label, Before::Label);
            });
            Row::new().gap(Gap::Item).show(ui, |ui, _| {
                if clicked(&Button::new("Undo during gesture").show(ui)) {
                    state.undo(ui);
                }
                if clicked(&Button::new("Switch document").show(ui)) {
                    state.switch(ui);
                }
            });
            let doc = &state.docs[state.current];
            let status = format!("Document {}: level {}, label {:?}, {} undo steps", doc.name, doc.level, doc.label, doc.undo.len());
            Text::new(status).shade(Shade::Muted).show(ui);
            Stack::new().fill_width().surface(SurfaceKind::Sunken).pad(Pad::Control).show(ui, |ui, _| {
                if state.log.is_empty() {
                    Text::new("No events yet").shade(Shade::Faint).show(ui);
                }
                for line in &state.log {
                    Text::new(line.clone()).role(TypeRole::Mono).show(ui);
                }
            });
        });
    }
}
