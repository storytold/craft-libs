//! `NewWorkspaceDialog` and `DeleteWorkspaceDialog`: the questions `run_workspace` returns as `WorkspaceAsk::Name`
//! and `WorkspaceAsk::Which`.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::words::WorkspaceDialogWords;
use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use crate::display::{Shade, Text};
use crate::form::{Field, Help};
use crate::patterns::form_dialogs::FormDialog;
use crate::pickers::Select;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::sections::GroupBox;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::dock::{WorkspaceError, WorkspaceKeeps, WorkspaceSet};
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use std::hash::Hash;

/// What New Workspace answers: the name to pass to `WorkspaceSet::save` and what the workspace captures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewWorkspace {
    /// The name as typed; `WorkspaceSet::save` trims it.
    pub name: String,
    /// The Capture boxes that were on.
    pub keeps: WorkspaceKeeps,
}

/// What [`NewWorkspaceDialog`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NewWorkspaceDialogOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The name and captures, the pass the dialog closed as OK.
    pub accepted: Option<NewWorkspace>,
}

impl Outcome for NewWorkspaceDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Window › Workspace › New Workspace…: a name for the live layout and the Capture boxes (Keyboard Shortcuts,
/// Menus, Toolbar).
///
/// - The name is checked with `WorkspaceSet::check_name`: OK is refused while it is empty, and a built-in's name
///   says why under the field. A saved workspace's name says that OK replaces it.
/// - The value starts empty, with every box off, each time the dialog opens. Enter is OK and Escape cancels.
/// - The host saves the answer with `WorkspaceSet::save`.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(NewWorkspaceDialog<'static, u32>))]
pub struct NewWorkspaceDialog<'a, P> {
    set: &'a WorkspaceSet<P>,
    /// The Capture boxes the dialog shows: the default shows all three; an app without menu or toolbar
    /// customizations leaves those out.
    #[prop]
    offered: WorkspaceKeeps,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Binds the name and captures to `value`.
    #[prop(name = "value")]
    bound: Option<Bind<'a, NewWorkspace>>,
}

impl<'a, P: Clone + Eq + Hash> NewWorkspaceDialog<'a, P> {
    /// New Workspace over `set`, whose names it checks.
    pub fn new(set: &'a WorkspaceSet<P>) -> Self {
        NewWorkspaceDialog { set, offered: WorkspaceKeeps::ALL, open: None, bound: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NewWorkspaceDialogOutput {
        let NewWorkspaceDialog { set, offered, open, bound } = self;
        let words = Env::of(ui).words::<WorkspaceDialogWords>();
        let mut form = match bound {
            Some(bound) => FormDialog::new(words.new_title.clone(), bound),
            None => FormDialog::<NewWorkspace>::own(words.new_title.clone()),
        };
        if let Some(open) = open {
            form = form.opened(open);
        }
        let shown = form.show(ui, |ui, _, value| {
            // Ok(true) when the name replaces a saved workspace.
            let check = set.check_name(&value.name).map(|(_, replaced)| replaced.is_some());
            let help = match check {
                Err(WorkspaceError::NameTaken) => Some(Help::Status(Tone::Bad, words.built_in.clone())),
                Ok(true) => Some(Help::Status(Tone::Warn, words.replaces.clone())),
                _ => None,
            };
            let mut field = Field::new(words.name.clone());
            if let Some(help) = help {
                field = field.help(help);
            }
            field.show(ui, |ui, _| {
                TextField::new(words.name.clone(), Bind::bound(&mut value.name)).show(ui);
            });
            if offered != WorkspaceKeeps::NONE {
                let keeps = &mut value.keeps;
                GroupBox::new(words.capture.clone(), None).show(ui, |ui, _| {
                    Text::new(words.capture_note.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    let boxes = [
                        (offered.shortcuts, &words.shortcuts, &mut keeps.shortcuts),
                        (offered.menus, &words.menus, &mut keeps.menus),
                        (offered.toolbar, &words.toolbar, &mut keeps.toolbar),
                    ];
                    for (shown, label, on) in boxes {
                        if shown {
                            Checkbox::new(Bind::bound(on)).label(label.clone()).show(ui);
                        }
                    }
                });
            }
            check.is_ok()
        });
        NewWorkspaceDialogOutput { response: shown.response, closed: shown.closed, accepted: shown.accepted }
    }
}

/// What [`DeleteWorkspaceDialog`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct DeleteWorkspaceDialogOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The id of the saved workspace to delete, the pass the dialog closed as OK.
    pub accepted: Option<String>,
}

impl Outcome for DeleteWorkspaceDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Window › Workspace › Delete Workspace…: one of the saved workspaces.
///
/// - The list holds the saved workspaces; the current one shows disabled with its reason, since
///   `WorkspaceSet::delete` refuses it. The pick starts on the first saved workspace that is not current.
/// - OK is refused while nothing deletable is picked. Enter is OK and Escape cancels.
/// - The host deletes the answer with `WorkspaceSet::delete`.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(DeleteWorkspaceDialog<'static, u32>))]
pub struct DeleteWorkspaceDialog<'a, P> {
    set: &'a WorkspaceSet<P>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Binds the picked workspace's id to `value`.
    #[prop(name = "value")]
    bound: Option<Bind<'a, String>>,
}

impl<'a, P: Clone + Eq + Hash> DeleteWorkspaceDialog<'a, P> {
    /// Delete Workspace over the saved workspaces of `set`.
    pub fn new(set: &'a WorkspaceSet<P>) -> Self {
        DeleteWorkspaceDialog { set, open: None, bound: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> DeleteWorkspaceDialogOutput {
        let DeleteWorkspaceDialog { set, open, bound } = self;
        let words = Env::of(ui).words::<WorkspaceDialogWords>();
        let deletable = |id: &str| id != set.current;
        let entries: Vec<Entry<String>> = set
            .saved
            .iter()
            .map(|w| {
                let entry = Entry::new(w.id.clone(), w.name.clone());
                if deletable(&w.id) { entry } else { entry.enabled(false).disabled_reason(words.current.clone()) }
            })
            .collect();
        // A host that binds the value seeds it; only the dialog's own value starts on the first deletable workspace.
        let owns = bound.is_none();
        let mut form = match bound {
            Some(bound) => FormDialog::new(words.delete_title.clone(), bound),
            None => FormDialog::<String>::own(words.delete_title.clone()),
        };
        if let Some(open) = open {
            form = form.opened(open);
        }
        let label: Label = words.workspace.clone();
        let shown = form.show(ui, |ui, cx, value| {
            if owns
                && cx.first()
                && let Some(first) = first_deletable(set)
            {
                value.clone_from(first);
            }
            Field::new(label.clone()).show(ui, |ui, _| {
                if let Some(picked) = Select::new(label.clone(), Bind::given(value.clone()), &entries).show(ui).value() {
                    value.clone_from(picked);
                }
            });
            can_delete(set, value)
        });
        DeleteWorkspaceDialogOutput { response: shown.response, closed: shown.closed, accepted: shown.accepted }
    }
}

/// A [`NewWorkspaceDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns, over a copy of the workspace set. It
/// answers with the [`NewWorkspace`] at OK. It opens with its [`value`](HostedNewWorkspace::value): an empty name with
/// every box off unless the app gives one.
pub struct HostedNewWorkspace<P> {
    id: DialogId,
    set: WorkspaceSet<P>,
    offered: WorkspaceKeeps,
    value: NewWorkspace,
    open: bool,
}

/// The id of the first saved workspace that is not current: where Delete Workspace starts.
fn first_deletable<P>(set: &WorkspaceSet<P>) -> Option<&String> {
    set.saved.iter().map(|w| &w.id).find(|id| **id != set.current)
}

/// Whether `id` names a saved workspace other than the current one: what `WorkspaceSet::delete` accepts.
fn can_delete<P>(set: &WorkspaceSet<P>, id: &str) -> bool {
    id != set.current && set.saved.iter().any(|w| w.id == id)
}

/// The `Rejects` message of `field` for `value`.
fn rejected(field: &str, value: &serde_json::Value) -> String {
    FieldRefusal::Rejects(value).message(field)
}

impl<P: Clone> HostedNewWorkspace<P> {
    /// New Workspace over a copy of `set`, whose names it checks.
    pub fn new(set: &WorkspaceSet<P>) -> Self {
        HostedNewWorkspace { id: dialog_id(NEW_KEY), set: set.clone(), offered: WorkspaceKeeps::ALL, value: NewWorkspace::default(), open: true }
    }

    /// The Capture boxes the dialog shows; the default shows all three.
    #[must_use]
    pub fn offered(mut self, keeps: WorkspaceKeeps) -> Self {
        self.offered = keeps;
        self
    }

    /// The name and captures the dialog opens with; without it, `NewWorkspace::default()`.
    #[must_use]
    pub fn value(mut self, value: NewWorkspace) -> Self {
        self.value = value;
        self
    }
}

/// The command id of `WorkspaceCommands::New`, which the dialog answers for.
const NEW_KEY: &str = "workspace.new";
/// The command id of `WorkspaceCommands::Delete`, which the dialog answers for.
const DELETE_KEY: &str = "workspace.delete";

impl<P: Clone + Eq + Hash + Send + 'static> StackDialog for HostedNewWorkspace<P> {
    type Result = NewWorkspace;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<NewWorkspace>> {
        let shown =
            NewWorkspaceDialog::new(&self.set).offered(self.offered).value(Bind::bound(&mut self.value)).opened(Bind::bound(&mut self.open)).show(ui);
        match (shown.accepted, shown.closed) {
            (Some(value), _) => Some(DialogResult::Ok(value)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(NEW_KEY)
    }

    /// The answer OK gives: the value while its name passes the check the drawn dialog's OK applies.
    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        self.set.check_name(&self.value.name).is_ok().then(|| self.value.clone())
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        let keeps = &self.value.keeps;
        Some(serde_json::json!({
            "name": self.value.name,
            "keyboardShortcuts": keeps.shortcuts,
            "menus": keeps.menus,
            "toolbar": keeps.toolbar,
        }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let offered = self.offered;
        let keeps = &mut self.value.keeps;
        let (shown, on) = match field {
            "name" => {
                let serde_json::Value::String(name) = &value else { return Err(rejected(field, &value)) };
                self.value.name.clone_from(name);
                return Ok(());
            }
            "keyboardShortcuts" => (offered.shortcuts, &mut keeps.shortcuts),
            "menus" => (offered.menus, &mut keeps.menus),
            "toolbar" => (offered.toolbar, &mut keeps.toolbar),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        };
        if !shown {
            return Err(FieldRefusal::Inactive.message(field));
        }
        *on = value.as_bool().ok_or_else(|| rejected(field, &value))?;
        Ok(())
    }
}

/// A [`DeleteWorkspaceDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns, over a copy of the workspace set. It
/// answers with the id of the picked workspace at OK.
pub struct HostedDeleteWorkspace<P> {
    id: DialogId,
    set: WorkspaceSet<P>,
    value: String,
    open: bool,
}

impl<P: Clone> HostedDeleteWorkspace<P> {
    /// Delete Workspace over a copy of the saved workspaces of `set`.
    pub fn new(set: &WorkspaceSet<P>) -> Self {
        HostedDeleteWorkspace { id: dialog_id(DELETE_KEY), value: first_deletable(set).cloned().unwrap_or_default(), set: set.clone(), open: true }
    }
}

impl<P: Clone + Eq + Hash + Send + 'static> StackDialog for HostedDeleteWorkspace<P> {
    type Result = String;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<String>> {
        let shown = DeleteWorkspaceDialog::new(&self.set).value(Bind::bound(&mut self.value)).opened(Bind::bound(&mut self.open)).show(ui);
        match (shown.accepted, shown.closed) {
            (Some(id), _) => Some(DialogResult::Ok(id)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(DELETE_KEY)
    }

    /// The answer OK gives: the picked id while it is a saved workspace other than the current one.
    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        can_delete(&self.set, &self.value).then(|| self.value.clone())
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        let name = self.set.saved.iter().find(|w| w.id == self.value).map_or(self.value.as_str(), |w| w.name.as_str());
        Some(serde_json::json!({ "name": name }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        if field != "name" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        let serde_json::Value::String(wanted) = &value else { return Err(rejected(field, &value)) };
        let picked = self.set.saved.iter().find(|w| w.id == *wanted || w.name == *wanted);
        match picked {
            Some(w) if can_delete(&self.set, &w.id) => {
                self.value.clone_from(&w.id);
                Ok(())
            }
            _ => Err(rejected(field, &value)),
        }
    }
}
