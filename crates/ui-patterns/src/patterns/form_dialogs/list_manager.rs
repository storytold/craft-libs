//! `ListManager`: a named list with a name box and a column of buttons that run, edit, create or delete entries.

use crate::contract::{
    Bind, CollectionEvent, NoValue, Outcome, Props as _, Remember as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use crate::dialog::{Dialog, DialogOutput};
use crate::display::{Shade, Text};
use crate::form::Field;
use crate::layout_components::Stack;
use crate::list::{List, ListRow};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{DisplayOrder, ItemSource, Selection};
use craft_i18n::Folder;
use egui::{Response, Ui};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

#[cfg(feature = "testing")]
mod stories;

craft_ui::words! {
    /// The standard words of the list manager; the app's own actions bring their labels.
    pub struct ListManagerWords {
        run: "listManager.run" = "Run";
        edit: "listManager.edit" = "Edit";
        create: "listManager.create" = "Create";
        delete: "listManager.delete" = "Delete";
        options: "listManager.options" = "Options…";
        description: "listManager.description" = "Description";
    }
}

/// The id of the standard delete action, which Delete in the list runs.
const DELETE: &str = "delete";

/// One row.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ListManagerEntry {
    /// The app's id for the entry; the answer carries it.
    pub id: String,
    /// The name shown in the row and typed in the name box.
    pub name: String,
    /// After the name in the row, dimmer: a shortcut, "(script, 3 steps)".
    pub detail: Option<Label>,
    /// Shown under the list while the entry is selected.
    pub description: Label,
    /// The app's grouping (macro or script); actions can require one.
    pub kind: u32,
}

impl ListManagerEntry {
    /// An entry with the app's `id` and a `name`, with no detail, no description and kind 0.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        ListManagerEntry { id: id.into(), name: name.into(), detail: None, description: Label::default(), kind: 0 }
    }

    /// Sets the detail after the name.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<Label>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Sets the description under the list.
    #[must_use]
    pub fn description(mut self, description: impl Into<Label>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets the entry's kind.
    #[must_use]
    pub fn kind(mut self, kind: u32) -> Self {
        self.kind = kind;
        self
    }
}

/// When an action's button is enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ActionEnabled {
    /// Always.
    Always,
    /// When the name box names an entry (of `kind`, when given).
    #[non_exhaustive]
    OnEntry {
        /// Only entries of this kind, when given.
        kind: Option<u32>,
    },
    /// When the name box holds a name no entry has (Create).
    OnNewName,
}

/// One button of the column, top to bottom.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ListManagerAction {
    /// The app's id ("run", "stepInto"); the answer carries it.
    pub id: String,
    /// The button's label.
    pub label: Label,
    /// What the button tells while it is disabled, if anything.
    pub reason: Option<Label>,
    /// What the button tells while it is enabled, if anything.
    pub tooltip: Option<Label>,
    /// When the button is enabled.
    pub enabled: ActionEnabled,
    /// Closes the dialog with the answer (Run, Edit, Create); otherwise the dialog stays open and the answer comes
    /// back for the pass of the press (Delete asks first; Options… opens a child dialog).
    pub closes: bool,
}

impl ListManagerAction {
    /// An action with the app's `id`, a `label` and its enabled rule; it closes the dialog.
    pub fn new(id: impl Into<String>, label: impl Into<Label>, enabled: ActionEnabled) -> Self {
        ListManagerAction { id: id.into(), label: label.into(), reason: None, tooltip: None, enabled, closes: true }
    }

    /// Run, enabled on an entry, closing; the default action (Enter, double click).
    pub fn run(words: &ListManagerWords) -> Self {
        Self::new("run", words.run.clone(), ActionEnabled::OnEntry { kind: None })
    }

    /// Edit, enabled on an entry, closing.
    pub fn edit(words: &ListManagerWords) -> Self {
        Self::new("edit", words.edit.clone(), ActionEnabled::OnEntry { kind: None })
    }

    /// Create, enabled when the name box holds a new name, closing.
    pub fn create(words: &ListManagerWords) -> Self {
        Self::new("create", words.create.clone(), ActionEnabled::OnNewName)
    }

    /// Delete, enabled on an entry, keeping the dialog open for the app's confirmation; the Delete key runs it.
    pub fn delete(words: &ListManagerWords) -> Self {
        Self::new(DELETE, words.delete.clone(), ActionEnabled::OnEntry { kind: None }).keeps_open()
    }

    /// Tells `reason` while the button is disabled.
    #[must_use]
    pub fn reason(mut self, reason: impl Into<Label>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Tells `tooltip` while the button is enabled.
    #[must_use]
    pub fn tooltip(mut self, tooltip: impl Into<Label>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Enabled only on entries of `kind`.
    #[must_use]
    pub fn kind(mut self, kind: u32) -> Self {
        if let ActionEnabled::OnEntry { kind: wanted } = &mut self.enabled {
            *wanted = Some(kind);
        }
        self
    }

    /// Keeps the dialog open when pressed; the answer comes back for the pass of the press.
    #[must_use]
    pub fn keeps_open(mut self) -> Self {
        self.closes = false;
        self
    }
}

/// A button pressed: which action, on which entry (`None` for a new name) and the name box's text, trimmed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListManagerChoice {
    /// The pressed action's id.
    pub action: String,
    /// The id of the selected entry; `None` for a new name.
    pub entry: Option<String>,
    /// The name box's text, trimmed.
    pub name: String,
}

/// What a list manager reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ListManagerOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The action run in this pass. The dialog closed with it when `closed` is set; an action that keeps the dialog
    /// open reports it with `closed` empty.
    pub choice: Option<ListManagerChoice>,
}

impl Outcome for ListManagerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the dialog keeps while it is open: the text of the name box.
#[derive(Clone, Default)]
struct Session {
    name: String,
}

/// The rows as the list's item source: the position in the entries is the key.
struct Rows<'a>(&'a [ListManagerEntry]);

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0[at].name.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

/// Whether the name box names `entry`: its trimmed text equals the entry's name, case and accents ignored in the
/// scope's language.
fn is_named(entry: &ListManagerEntry, folder: &Folder, name: &str) -> bool {
    folder.fold(&entry.name).as_str() == folder.fold(name.trim()).as_str()
}

/// Whether `action` takes `entry` as its target.
fn accepts(action: &ListManagerAction, entry: &ListManagerEntry) -> bool {
    match action.enabled {
        ActionEnabled::OnEntry { kind, .. } => kind.is_none_or(|wanted| wanted == entry.kind),
        _ => true,
    }
}

/// The action's button state for the name box's text.
fn is_enabled(action: &ListManagerAction, entries: &[ListManagerEntry], folder: &Folder, name: &str) -> bool {
    match action.enabled {
        ActionEnabled::OnEntry { .. } => entries.iter().any(|entry| is_named(entry, folder, name) && accepts(action, entry)),
        ActionEnabled::OnNewName => !name.trim().is_empty() && !entries.iter().any(|entry| is_named(entry, folder, name)),
        _ => true,
    }
}

/// A named list with a name box and a column of action buttons (after Excel's Macros dialog).
///
/// - Name box: typing selects the entry with that name; picking a row puts its name in the box. With `searchable` the
///   box also filters the rows to the names that match it.
/// - Keys: Up and Down in the box or the list move the selection, Enter and a double click run the first action when
///   it is enabled, Delete in the list runs the `delete` action, Escape cancels.
/// - Actions are the footer's buttons, in the given order, with Cancel last. The app replaces `entries` between
///   passes; the name box keeps its text.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct ListManager<'a> {
    title: Label,
    name_label: Label,
    entries: Cow<'a, [ListManagerEntry]>,
    actions: Cow<'a, [ListManagerAction]>,
    /// Shown in the list, dim, when it has no rows.
    #[prop]
    empty: Option<Label>,
    /// A dim line at the bottom.
    #[prop]
    note: Option<Label>,
    /// The name box also filters the rows to the names that match it.
    #[prop]
    searchable: bool,
    /// The name the box opens with, with its entry selected.
    #[prop(into)]
    name: Option<String>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> ListManager<'a> {
    /// A list manager titled `title` whose name box is labelled `name_label` ("Macro name:").
    pub fn new(
        title: impl Into<Label>,
        name_label: impl Into<Label>,
        entries: impl Into<Cow<'a, [ListManagerEntry]>>,
        actions: impl Into<Cow<'a, [ListManagerAction]>>,
    ) -> Self {
        ListManager {
            title: title.into(),
            name_label: name_label.into(),
            entries: entries.into(),
            actions: actions.into(),
            empty: None,
            note: None,
            searchable: false,
            name: None,
            open: None,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ListManagerOutput {
        let ListManager { title, name_label, entries, actions, empty, note, searchable, name, open } = self;
        let env = Env::of(ui);
        let words = env.words::<ListManagerWords>();
        let common = env.words::<CommonWords>();
        let list_id = ui.scope_id().with("list-manager-rows");
        let entries: &[ListManagerEntry] = &entries;
        let actions: &[ListManagerAction] = &actions;

        let mut buttons: Vec<FooterButton> = actions
            .iter()
            .enumerate()
            .map(|(at, action)| {
                let role = if at == 0 { DialogRole::Accept } else { DialogRole::Other };
                let mut button = FooterButton::new(action.label.clone(), role).stays_open(!action.closes);
                if let Some(reason) = &action.reason {
                    button = button.disabled_reason(reason.clone());
                }
                if let Some(tooltip) = &action.tooltip {
                    button = button.tooltip(tooltip.clone());
                }
                button
            })
            .collect();
        buttons.push(FooterButton::new(common.cancel.clone(), DialogRole::Cancel));
        let mut dialog = Dialog::new(title.clone()).collection_body().close_on_enter(false).buttons(buttons).settle_buttons(move |cx, buttons| {
            let folder = cx.env.folder();
            let typed = cx.memory::<Session>().name.clone();
            for (button, action) in buttons.iter_mut().zip(actions) {
                button.set_enabled(is_enabled(action, entries, &folder, &typed));
            }
        });
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }

        let folder = env.folder();
        let activated = Cell::new(None::<usize>);
        let typed = RefCell::new(String::new());
        let shown = dialog.show(ui, |ui, cx| {
            let mut session = cx.memory::<Session>().clone();
            if cx.appeared()
                && let Some(name) = &name
            {
                session.name.clone_from(name);
            }
            let session = RefCell::new(session);
            Stack::new()
                .header(|ui, _| {
                    let mut session = session.borrow_mut();
                    Field::new(name_label.clone()).show(ui, |ui, _| {
                        TextField::new(name_label.clone(), Bind::bound(&mut session.name)).select_on_focus().drives(list_id).show(ui);
                    });
                })
                .footer(|ui, _| {
                    let session = session.borrow();
                    if let Some(entry) = entries.iter().find(|entry| is_named(entry, &folder, &session.name)) {
                        Text::new(words.description.clone()).show(ui);
                        Text::new(entry.description.clone()).shade(Shade::Muted).show(ui);
                    }
                    if let Some(note) = &note {
                        Text::new(note.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    }
                })
                .show(ui, |ui, _| {
                    let mut session = session.borrow_mut();
                    let current = entries.iter().position(|entry| is_named(entry, &folder, &session.name));
                    let mut selection = current.map_or_else(Selection::default, Selection::single);
                    let source = Rows(entries);
                    let mut list = List::new(title.clone())
                        .id(list_id)
                        .select(SelectMode::Single)
                        .allow_empty(true)
                        .deletable(true)
                        .selection(Bind::bound(&mut selection));
                    if let Some(empty) = &empty {
                        list = list.empty(empty.clone());
                    }
                    if searchable {
                        list = list.filter(&session.name, |at| Label::from(entries[*at].name.clone()));
                    }
                    let events = list
                        .show_items(ui, &source, &mut |at, _| {
                            let entry = &entries[*at];
                            let row = ListRow::new(entry.name.clone());
                            match &entry.detail {
                                Some(detail) => row.trailing(detail.clone()).description(detail.as_str()),
                                None => row,
                            }
                        })
                        .events;
                    let picked = selection.items.first().copied();
                    if picked != current
                        && let Some(at) = picked
                    {
                        session.name.clone_from(&entries[at].name);
                    }
                    for event in events {
                        match event {
                            CollectionEvent::Activate(at) => {
                                session.name.clone_from(&entries[at].name);
                                let first = actions.first().filter(|action| is_enabled(action, entries, &folder, &session.name));
                                activated.set(first.map(|_| 0));
                            }
                            CollectionEvent::Delete(rows) => {
                                if let Some(at) = rows.first() {
                                    session.name.clone_from(&entries[*at].name);
                                }
                                let delete = actions.iter().position(|action| action.id == DELETE);
                                activated.set(delete.filter(|at| is_enabled(&actions[*at], entries, &folder, &session.name)));
                            }
                            _ => {}
                        }
                    }
                });
            let session = session.into_inner();
            typed.borrow_mut().clone_from(&session.name);
            if let Some(at) = activated.get()
                && actions[at].closes
            {
                cx.request_close_as(CloseReason::Activated);
            }
            *cx.memory::<Session>() = session;
        });

        let DialogOutput { response, closed, picked } = shown.output;
        let at = picked.filter(|at| *at < actions.len()).or(activated.get());
        let choice = at.map(|at| {
            let typed = typed.take();
            let entry = entries.iter().find(|entry| is_named(entry, &folder, &typed) && accepts(&actions[at], entry));
            ListManagerChoice { action: actions[at].id.clone(), entry: entry.map(|entry| entry.id.clone()), name: typed.trim().to_owned() }
        });
        ListManagerOutput { response, closed, choice }
    }
}
