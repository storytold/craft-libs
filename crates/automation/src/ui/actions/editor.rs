//! A Modern Actions editor over an unpublished shared action. It requests host effects only.

use std::collections::BTreeSet;

use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, Scroll, Show as _, ShowContainer as _, SweepApply};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::TextField;
use egui::{Response, Ui};

use super::editor_picker;
use super::editor_steps;
use super::words::ActionEditorWords;
use crate::actions::{Action, ActionStep};

/// The host's unpublished editor value, independent of its committed library and draft identity.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionEditorValue {
    /// A cloned action, including opaque options and imported metadata.
    pub action: Action,
    /// Raw shortcut text. Legacy shortcuts are retained until the user explicitly edits it.
    pub shortcut: String,
}

impl ActionEditorValue {
    /// An unpublished action and its shortcut. The caller clones a committed action before opening.
    pub fn new(action: Action, shortcut: impl Into<String>) -> Self {
        Self { action, shortcut: shortcut.into() }
    }
}

/// Presentation state only. A host uses a separate state for each open draft.
#[derive(Clone, Debug, Default)]
pub struct ActionEditorState {
    /// The selected step, if still in the current action.
    pub selected: Option<usize>,
    /// Steps whose host-provided settings summary is expanded.
    pub expanded: BTreeSet<usize>,
    pub(super) icons_open: bool,
    pub(super) step_order_revision: u64,
    pub(super) include: SweepApply,
    pub(super) dialog: SweepApply,
}

/// Activity acknowledged by the host, never inferred from a request emitted by the editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionEditorActivity {
    /// The host owns a live capture lease for this draft.
    pub recording: bool,
    /// The host owns a live preview job, including a suspended step dialog.
    pub previewing: bool,
    /// Another host transition prevents editing or beginning effects.
    pub busy: bool,
}

impl ActionEditorActivity {
    pub(super) fn idle(self) -> bool {
        !self.recording && !self.previewing && !self.busy
    }
}

/// App-specific descriptions and glyphs. This interface owns no commands, jobs or persistence.
pub trait ActionEditorHost {
    /// The translated label of a recorded command.
    fn step_label(&self, step: &ActionStep) -> String;

    /// An optional settings summary, shown when its step is expanded.
    fn step_summary(&self, _step: &ActionStep) -> Option<String> {
        None
    }

    /// Whether the app can open a typed settings dialog for this step.
    fn can_edit_step(&self, _step: &ActionStep) -> bool {
        false
    }

    /// Resolve an icon identifier to licensed artwork; unknown names use the shared fallback.
    fn icon(&self, _name: &str) -> Option<IconSpec<'static>> {
        None
    }
}

/// Requests for the host. None commits, closes, starts playback or changes a capture lease.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionEditorEvent {
    /// Preview this unpublished draft from a valid recorded step.
    Preview {
        /// The first step, in bounds at the time of the request.
        from: usize,
    },
    /// Begin capture at a valid insertion boundary in this unpublished draft.
    BeginCapture {
        /// The insertion boundary, including the end of the action.
        at: usize,
    },
    /// Stop the host's capture lease.
    EndCapture,
    /// Ask the host to open its typed settings dialog for this step.
    EditStep {
        /// The selected step.
        step: usize,
    },
    /// Duplicate a selected step through the host, preserving its native occurrence bindings.
    DuplicateStep {
        /// The source step; the clone has not been changed by this request.
        step: usize,
    },
    /// Delete a selected step through the host after its ownership and stale guards.
    DeleteStep {
        /// The source step; the clone has not been changed by this request.
        step: usize,
    },
    /// Ask the host to open its typed stop-message insertion dialog.
    InsertStop {
        /// The insertion boundary.
        at: usize,
    },
    /// Ask the host to validate and save the draft, including any confirmation prompt.
    Save,
    /// Ask the host to discard the draft; the page remains open until acknowledged.
    Discard,
    /// Ask the host to navigate away, resolving unsaved changes and active effects itself.
    Leave,
}

/// One already-applied move of a step occurrence in the editor's unpublished value.
/// Indices address the evolving sequence: apply receipts in their emitted order to native
/// occurrence bindings. Equal command/parameter values never imply interchangeable origins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionStepMove {
    /// The occurrence's index before this move.
    pub from: usize,
    /// Its index after removal and reinsertion, before the next receipt.
    pub to: usize,
}

/// One pass of the editor. The host stages changes through its own draft/revision guard before effects.
#[derive(Debug)]
#[must_use = "apply occurrence receipts, stage changed values and handle host requests"]
pub struct ActionEditorOutput {
    /// The editor's area.
    pub response: Response,
    /// The borrowed unpublished value changed this pass. An occurrence move between equal values
    /// may leave this false; always consume `step_moves` independently.
    pub changed: bool,
    /// Moves already applied to the borrowed action, in order. Apply them only to app-native
    /// occurrence bindings before staging the remaining metadata; do not move the UI clone again.
    /// Sizing passes return no receipts.
    pub step_moves: Vec<ActionStepMove>,
    /// Host requests, in display order.
    pub events: Vec<ActionEditorEvent>,
}

impl Outcome for ActionEditorOutput {
    type Value = NoValue;
    fn response(&self) -> &Response {
        &self.response
    }
}

/// A page body, suitable for an app's workspace or typed dialog body. It owns no dialog lifetime.
/// The step list uses Up/Down for selection, Left/Right for summaries, Space for Include and
/// Alt+Up/Down to reorder; its standard controls are real focus stops.
#[must_use = "show the editor with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ActionEditor<'a> {
    state: &'a mut ActionEditorState,
    value: &'a mut ActionEditorValue,
    host: Option<&'a dyn ActionEditorHost>,
    activity: ActionEditorActivity,
    refusal: Option<Label>,
}

impl<'a> ActionEditor<'a> {
    /// Edit the host's unpublished value. Do not bind a committed library action directly.
    pub fn new(state: &'a mut ActionEditorState, value: &'a mut ActionEditorValue) -> Self {
        Self { state, value, host: None, activity: ActionEditorActivity::default(), refusal: None }
    }

    /// The host that can answer requests and supply command labels.
    pub fn host(mut self, host: &'a dyn ActionEditorHost) -> Self {
        self.host = Some(host);
        self
    }

    /// The current acknowledged activity of this draft.
    pub fn activity(mut self, activity: ActionEditorActivity) -> Self {
        self.activity = activity;
        self
    }

    /// A validation or stale-draft refusal from the host, shown without discarding the local value.
    pub fn refusal(mut self, refusal: impl Into<Label>) -> Self {
        self.refusal = Some(refusal.into());
        self
    }

    /// Draw the editor, changing only its borrowed draft and reporting requests to its host.
    pub fn show(self, ui: &mut Ui) -> ActionEditorOutput {
        let Self { state, value, host, activity, refusal } = self;
        let before = value.clone();
        let words = Env::of(ui).words::<ActionEditorWords>();
        let idle = activity.idle();
        let hosted = host.is_some();
        state.selected = state.selected.filter(|&step| step < value.action.steps.len());
        state.expanded.retain(|&step| step < value.action.steps.len());
        let mut events = Vec::new();
        let mut step_moves = Vec::new();
        let shown = Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            TextField::new(words.name.clone(), Bind::bound(&mut value.action.name)).enabled(idle).show(ui);
            TextField::new(words.description.clone(), Bind::bound(&mut value.action.description)).lines(3).enabled(idle).show(ui);
            editor_picker::show(ui, value, state, host, &words, idle);
            TextField::new(words.shortcut.clone(), Bind::bound(&mut value.shortcut)).enabled(idle).show(ui);
            Text::new(words.steps.clone()).show(ui);
            step_moves.extend(editor_steps::show(ui, &mut value.action, state, host, &words, idle));
            let selected = state.selected.filter(|&step| step < value.action.steps.len());
            let at = selected.map_or(value.action.steps.len(), |step| step + 1);
            let can_preview = hosted && !activity.busy && !activity.previewing;
            Row::new().show(ui, |ui, _| {
                if Button::new(words.preview.clone()).enabled(can_preview && !value.action.steps.is_empty()).show(ui).clicked() {
                    events.push(ActionEditorEvent::Preview { from: 0 });
                }
                if Button::new(words.preview_from.clone()).enabled(can_preview && selected.is_some()).show(ui).clicked()
                    && let Some(from) = selected
                {
                    events.push(ActionEditorEvent::Preview { from });
                }
                let label = if activity.recording { &words.stop_capture } else { &words.capture };
                if Button::new(label.clone()).enabled(hosted && !activity.busy && !activity.previewing).show(ui).clicked() {
                    events.push(if activity.recording {
                        ActionEditorEvent::EndCapture
                    } else {
                        ActionEditorEvent::BeginCapture { at: value.action.steps.len() }
                    });
                }
            });
            Row::new().show(ui, |ui, _| {
                let editable =
                    selected.and_then(|step| value.action.steps.get(step)).is_some_and(|step| host.is_some_and(|host| host.can_edit_step(step)));
                if Button::new(words.edit_step.clone()).enabled(idle && editable).show(ui).clicked()
                    && let Some(step) = selected
                {
                    events.push(ActionEditorEvent::EditStep { step });
                }
                for (label, duplicate) in [(words.duplicate_step.clone(), true), (words.delete_step.clone(), false)] {
                    if Button::new(label).enabled(hosted && idle && selected.is_some()).show(ui).clicked()
                        && let Some(step) = selected
                    {
                        events.push(if duplicate { ActionEditorEvent::DuplicateStep { step } } else { ActionEditorEvent::DeleteStep { step } });
                    }
                }
                if Button::new(words.insert_stop.clone()).enabled(hosted && idle).show(ui).clicked() {
                    events.push(ActionEditorEvent::InsertStop { at });
                }
            });
            if let Some(refusal) = &refusal {
                Notice::new(Tone::Bad, refusal.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            Row::new().show(ui, |ui, _| {
                for (label, event, enabled) in [
                    (words.save.clone(), ActionEditorEvent::Save, !value.action.name.trim().is_empty()),
                    (words.discard.clone(), ActionEditorEvent::Discard, true),
                    (words.leave.clone(), ActionEditorEvent::Leave, true),
                ] {
                    if Button::new(label).enabled(hosted && idle && enabled).show(ui).clicked() {
                        events.push(event);
                    }
                }
            });
        });
        let sizing = ui.is_sizing_pass();
        if sizing {
            *value = before.clone();
            events.clear();
            step_moves.clear();
        }
        ActionEditorOutput { response: shown.output, changed: !sizing && *value != before, step_moves, events }
    }
}

#[cfg(feature = "screen-testing")]
pub(crate) mod stories {
    use super::*;
    use crate::actions::ActionColor;
    use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
    use serde_json::json;

    struct Host;

    impl ActionEditorHost for Host {
        fn step_label(&self, step: &ActionStep) -> String {
            step.command.clone()
        }

        fn step_summary(&self, step: &ActionStep) -> Option<String> {
            Some(format!("Settings: {}", step.params))
        }

        fn can_edit_step(&self, _: &ActionStep) -> bool {
            true
        }
    }

    fn draft() -> ActionEditorValue {
        let mut action =
            Action::new("Retouch draft", ["Levels", "Curves", "Blur"].into_iter().map(|name| ActionStep::new(name, json!({"amount": 2}))).collect());
        action.description = "Adjust the tonal range.\nThen soften the result.".to_owned();
        action.color = ActionColor::Teal;
        action.disabled.insert(1);
        action.dialogs.insert(2);
        action.options.insert("imported-option".to_owned(), json!({"kept": true}));
        ActionEditorValue::new(action, "Shift+F2")
    }

    /// A persistent unpublished draft; the story reports host requests without starting app effects.
    #[story(ActionEditor<'static>, covers = [Default], drive = Drive::Never)]
    fn action_editor_draft(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
        // StoryData lends one slot at a time, so copy the value out and write it back after showing.
        let mut value = data.value("draft", draft()).clone();
        let state =
            data.note("editor-state", ActionEditorState { selected: Some(0), expanded: [0].into_iter().collect(), ..ActionEditorState::default() });
        let output = ActionEditor::new(state, &mut value).host(&Host).show(ui);
        *data.value("draft", draft()) = value;
        if output.changed || !output.step_moves.is_empty() || !output.events.is_empty() {
            *data.note("last-request", String::new()) = format!(
                "Draft changed: {}. Step moves: {:?}. Host requests awaiting acknowledgement: {:?}.",
                output.changed, output.step_moves, output.events
            );
        }
        let readout = data.note("last-request", "Unpublished draft; host requests are shown here, not performed.".to_owned()).clone();
        StoryOutput::from(output).readout(readout)
    }
}
