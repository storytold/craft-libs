//! `NewWorkspaceDialog` and `DeleteWorkspaceDialog`: the questions `run_workspace` returns as `WorkspaceAsk::Name`
//! and `WorkspaceAsk::Which`.

#[cfg(feature = "testing")]
mod stories;

use super::words::WorkspaceDialogWords;
use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
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
}

impl<'a, P: Clone + Eq + Hash> NewWorkspaceDialog<'a, P> {
    /// New Workspace over `set`, whose names it checks.
    pub fn new(set: &'a WorkspaceSet<P>) -> Self {
        NewWorkspaceDialog { set, offered: WorkspaceKeeps::ALL, open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NewWorkspaceDialogOutput {
        let NewWorkspaceDialog { set, offered, open } = self;
        let words = Env::of(ui).words::<WorkspaceDialogWords>();
        let mut form = FormDialog::<NewWorkspace>::own(words.new_title.clone());
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
}

impl<'a, P: Clone + Eq + Hash> DeleteWorkspaceDialog<'a, P> {
    /// Delete Workspace over the saved workspaces of `set`.
    pub fn new(set: &'a WorkspaceSet<P>) -> Self {
        DeleteWorkspaceDialog { set, open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> DeleteWorkspaceDialogOutput {
        let DeleteWorkspaceDialog { set, open } = self;
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
        let mut form = FormDialog::<String>::own(words.delete_title.clone());
        if let Some(open) = open {
            form = form.opened(open);
        }
        let label: Label = words.workspace.clone();
        let shown = form.show(ui, |ui, cx, value| {
            if cx.first()
                && let Some(first) = set.saved.iter().find(|w| deletable(&w.id))
            {
                value.clone_from(&first.id);
            }
            Field::new(label.clone()).show(ui, |ui, _| {
                if let Some(picked) = Select::new(label.clone(), Bind::given(value.clone()), &entries).show(ui).value() {
                    value.clone_from(picked);
                }
            });
            set.saved.iter().any(|w| w.id == *value && deletable(&w.id))
        });
        DeleteWorkspaceDialogOutput { response: shown.response, closed: shown.closed, accepted: shown.accepted }
    }
}
