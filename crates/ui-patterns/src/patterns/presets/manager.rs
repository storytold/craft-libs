//! `PresetManager`: a dialog body for managing the presets of several kinds: a kind select over the presets of that
//! kind, with rename and delete, and the host's own action buttons.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::browser::PresetBrowser;
use super::event::PresetEvent;
use super::footer::PresetFooterOptions;
use super::model::PresetRow;
use super::source::{PresetFns, PresetSource};
use super::state::{PresetState, PresetView};
use super::words::PresetWords;
use crate::contract::{Bind, Entry, MemoProps, NoValue, Outcome, Pattern, Props, Show as _, ShowContainer as _};
use crate::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use crate::layout_components::Stack;
use crate::pickers::Select;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::sections::{DialogRole, FooterButton};
use craft_behavior::{Mixed, Value};
use egui::{Response, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::hash::Hash;

/// The most presets the list shows before it scrolls: the dialog body scrolls too, so the list keeps its own scroll
/// (production's list scrolls past 260 points, about ten rows).
const LIST_ROWS: u16 = 10;

/// What a [`PresetManager`] took from the user in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct PresetManagerTaken<G, K> {
    /// The kind the user picked in this pass.
    pub kind: Option<G>,
    /// The renames, deletes and picks of the rows, in order.
    pub events: Vec<PresetEvent<K>>,
    /// The indices of the action buttons pressed, in order.
    pub actions: Vec<usize>,
}

impl<G, K> Default for PresetManagerTaken<G, K> {
    fn default() -> Self {
        PresetManagerTaken { kind: None, events: Vec::new(), actions: Vec::new() }
    }
}

impl<G, K> PresetManagerTaken<G, K> {
    /// Adds what a later pass took: its kind pick replaces an earlier one, its events and presses follow.
    fn absorb(&mut self, later: PresetManagerTaken<G, K>) {
        let PresetManagerTaken { kind, events, actions } = later;
        if kind.is_some() {
            self.kind = kind;
        }
        self.events.extend(events);
        self.actions.extend(actions);
    }
}

/// What a [`PresetManager`] reports.
#[non_exhaustive]
pub struct PresetManagerOutput<G, K> {
    /// The manager's response.
    pub response: Response,
    /// How the manager closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// What the user did this pass.
    pub taken: PresetManagerTaken<G, K>,
}

impl<G, K> Outcome for PresetManagerOutput<G, K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The preset manager: a select of preset `kinds` over the list of the picked kind's `rows`, with Rename and Delete
/// on each row and buttons for the host's own `actions` (Load, Save, Reset).
///
/// Each kind keeps its own view state in the dialog's memory. The host replaces `rows` when `taken.kind` changes.
#[must_use = "show the manager with `show`"]
#[derive(Pattern, Props)]
#[pattern(probe(PresetManager<'static, u32, u32>))]
pub struct PresetManager<'a, G: Value, K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> {
    title: Label,
    kinds: &'a [Entry<G>],
    kind: Bind<'a, G>,
    source: &'a PresetSource<'a, K>,
    /// The host's buttons: a label and a tooltip each.
    #[prop]
    actions: Vec<(Label, Option<Label>)>,
    /// What the list shows when `rows` is empty.
    #[prop(into)]
    empty: Option<Label>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a, G: Value, K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> PresetManager<'a, G, K> {
    /// A manager titled `title` over the preset `kinds`, with `kind` the picked one and `source` its presets (whose
    /// tests say which presets Rename and Delete apply to).
    pub fn new(title: impl Into<Label>, kinds: &'a [Entry<G>], kind: Bind<'a, G>, source: &'a PresetSource<'a, K>) -> Self {
        PresetManager { title: title.into(), kinds, kind, source, actions: Vec::new(), empty: None, open: None }
    }

    /// Shows the manager.
    pub fn show(self, ui: &mut Ui) -> PresetManagerOutput<G, K> {
        let PresetManager { title, kinds, mut kind, source, actions, empty, open } = self;
        let env = Env::of(ui);
        let words = env.words::<PresetWords>();
        let empty = empty.unwrap_or_else(|| words.manager_empty.clone());

        // The browser's state lives under an id scoped by the picked kind's position, so each kind keeps its own view.
        let picked = match kind.reborrow().get() {
            Some(Mixed::Same(value)) => Some(value),
            _ => None,
        };
        let index = picked.and_then(|value| kinds.iter().position(|entry| entry.value() == Some(&value))).unwrap_or(0);

        let host_buttons = actions.len();
        let buttons = actions
            .iter()
            .map(|(label, tooltip)| {
                let button = FooterButton::new(label.clone(), DialogRole::Other).stays_open(true);
                match tooltip {
                    Some(tooltip) => button.tooltip(tooltip.clone()),
                    None => button,
                }
            })
            .chain(std::iter::once(FooterButton::new(words.done.clone(), DialogRole::Accept)));

        let mut dialog = Dialog::new(title.clone()).buttons(buttons);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let shown = dialog.show(ui, |ui, _| {
            let kind_pick = Select::new(words.manager_kind.clone(), kind.reborrowed(), kinds).show(ui).value().cloned();
            // Each kind's browser keeps its own state: its search, scroll and open groups.
            let events: Vec<PresetEvent<K>> = Stack::new()
                .id_salt(("preset-kind", index))
                .fill_width()
                .show(ui, |ui, _| {
                    PresetBrowser::new(title.clone(), Bind::<PresetState<K>>::kept(), source)
                        .view_lock(PresetView::List)
                        .toolbar(false)
                        .empty(empty.clone())
                        .max_rows(LIST_ROWS)
                        .footer(PresetFooterOptions::new().can_delete(true))
                        .show(ui)
                        .events
                })
                .inner
                .unwrap_or_default();
            (kind_pick, events)
        });

        // The body can run twice in a pass (measure, then paint); `inner` is the last run's value.
        let (kind_pick, events) = shown.inner.unwrap_or_default();
        let actions = shown.output.picked.filter(|picked| *picked < host_buttons).into_iter().collect();
        PresetManagerOutput {
            response: shown.output.response,
            closed: shown.output.closed,
            taken: PresetManagerTaken { kind: kind_pick, events, actions },
        }
    }
}

/// A [`PresetManager`] the [`DialogHost`](crate::dialog::DialogHost) owns, over the names of the picked kind's
/// presets, each keyed by its index. The app reads what the user did with [`take`](HostedPresetManager::take).
pub struct HostedPresetManager<G> {
    id: DialogId,
    title: Label,
    kinds: Vec<Entry<G>>,
    kind: G,
    names: Vec<String>,
    actions: Vec<(Label, Option<Label>)>,
    queued: PresetManagerTaken<G, usize>,
}

/// One preset row per name, keyed by its index.
fn name_rows(names: &&Vec<String>, _: &Env) -> Vec<PresetRow<usize>> {
    names.iter().enumerate().map(|(index, name)| PresetRow::item(index, name.clone(), 0)).collect()
}

impl<G: Value> HostedPresetManager<G> {
    /// A hosted manager titled `title` over the preset `kinds`, opening on `kind`.
    pub fn new(title: impl Into<Label>, kinds: Vec<Entry<G>>, kind: G) -> Self {
        HostedPresetManager {
            id: dialog_id("preset-manager"),
            title: title.into(),
            kinds,
            kind,
            names: Vec::new(),
            actions: Vec::new(),
            queued: PresetManagerTaken::default(),
        }
    }

    /// Sets the names of the picked kind's presets.
    pub fn names(&mut self, names: Vec<String>) {
        self.names = names;
    }

    /// Sets the host's footer buttons: a label and a tooltip each. A press is reported by its index in
    /// [`PresetManagerTaken::actions`].
    pub fn actions(&mut self, actions: Vec<(Label, Option<Label>)>) {
        self.actions = actions;
    }

    /// The kind the manager shows.
    pub fn kind(&self) -> &G {
        &self.kind
    }

    /// What the user did since the last call.
    pub fn take(&mut self) -> PresetManagerTaken<G, usize> {
        std::mem::take(&mut self.queued)
    }
}

impl<G: Value + Send + Serialize + DeserializeOwned + 'static> StackDialog for HostedPresetManager<G> {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let HostedPresetManager { title, kinds, kind, names, actions, queued, .. } = self;
        let source = PresetSource::new(&*names, PresetFns::new(name_rows));
        let out = PresetManager::new(title.clone(), kinds, Bind::bound(kind), &source).actions(actions.clone()).show(ui);
        queued.absorb(out.taken);
        match out.closed? {
            CloseReason::Enter | CloseReason::Activated => Some(DialogResult::Ok(())),
            _ => Some(DialogResult::Cancel),
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        Some(())
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "kind": serde_json::to_value(&self.kind).ok()?, "rows": self.names }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        if field != "kind" {
            return Err(format!("unknown field `{field}`"));
        }
        let kind = serde_json::from_value::<G>(value.clone()).ok().filter(|kind| self.kinds.iter().any(|entry| entry.value() == Some(kind)));
        let Some(kind) = kind else {
            let accepted: Vec<serde_json::Value> =
                self.kinds.iter().filter_map(|entry| entry.value()).filter_map(|kind| serde_json::to_value(kind).ok()).collect();
            return Err(format!("`kind` can't take {value}: the manager shows {}", serde_json::Value::from(accepted)));
        };
        self.kind = kind.clone();
        self.queued.kind = Some(kind);
        Ok(())
    }
}
