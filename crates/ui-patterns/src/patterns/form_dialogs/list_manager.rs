//! `ListManager`: a named list with a name box and a column of buttons that run, edit, create or delete entries.

use crate::contract::{
    Bind, BodyCx, CollectionEvent, NoValue, Outcome, Props as _, Remember as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use crate::dialog::{ButtonPlace, Dialog, DialogId, DialogOutput, DialogResult, DialogStack, StackDialog};
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
use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Mixed, Selection};
use craft_i18n::Folder;
use egui::{Response, Ui};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

type UnderList<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

#[cfg(feature = "suite")]
pub(crate) mod stories;

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
    /// When the name box holds a name `name_check` accepts.
    OnValidName,
    /// Never: the button is shown disabled.
    Never,
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

impl ListManagerChoice {
    /// The action id of an in-place rename ([`ListManager::renames`]). An app's own action must not use this id.
    pub const RENAME: &'static str = "rename";

    /// Whether the choice is an in-place rename.
    pub fn is_rename(&self) -> bool {
        self.action == Self::RENAME
    }
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

/// A [`ListManager`] the [`DialogHost`](crate::dialog::DialogHost) owns, over its entries and actions. It answers with
/// the [`ListManagerChoice`] of the action that closed it, as `HostedNewWorkspace` answers with its value; the choice
/// of an action that keeps it open (Delete, a rename) waits for [`take_choice`](HostedListManager::take_choice).
pub struct HostedListManager {
    id: DialogId,
    key: String,
    title: Label,
    name_label: Label,
    entries: Vec<ListManagerEntry>,
    actions: Vec<ListManagerAction>,
    renames: Option<u32>,
    /// The name box's text, kept by the dialog.
    name: String,
    open: bool,
    /// The choice of an action that kept the dialog open, until the host takes it.
    choice: Option<ListManagerChoice>,
}

impl HostedListManager {
    /// A hosted list manager for the command `key`, titled `title` over a name box labelled `name_label`, with the
    /// buttons `actions`.
    pub fn new(key: impl Into<String>, title: impl Into<Label>, name_label: impl Into<Label>, actions: Vec<ListManagerAction>) -> Self {
        let key = key.into();
        HostedListManager {
            id: crate::dialog::dialog_id(&key),
            key,
            title: title.into(),
            name_label: name_label.into(),
            entries: Vec::new(),
            actions,
            renames: None,
            name: String::new(),
            open: true,
            choice: None,
        }
    }

    /// The entries the list opens with.
    #[must_use]
    pub fn entries(mut self, entries: Vec<ListManagerEntry>) -> Self {
        self.entries = entries;
        self
    }

    /// Entries of `kind` rename in place, as [`ListManager`]'s `renames`: a commit is a choice that keeps the dialog
    /// open.
    #[must_use]
    pub fn renames(mut self, kind: u32) -> Self {
        self.renames = Some(kind);
        self
    }

    /// Replaces the entries of an open dialog; the name box keeps its text.
    pub fn set_entries(&mut self, entries: Vec<ListManagerEntry>) {
        self.entries = entries;
    }

    /// The choice of the last action that kept the dialog open (Delete, a rename), once.
    pub fn take_choice(&mut self) -> Option<ListManagerChoice> {
        self.choice.take()
    }
}

impl StackDialog for HostedListManager {
    type Result = ListManagerChoice;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ListManagerChoice>> {
        let list = ListManager::new(self.title.clone(), self.name_label.clone(), &self.entries[..], &self.actions[..])
            .name_box(Bind::bound(&mut self.name))
            .opened(Bind::bound(&mut self.open));
        let list = match self.renames {
            Some(kind) => list.renames(kind),
            None => list,
        };
        let shown = list.show(ui);
        match (shown.closed, shown.choice) {
            (Some(_), Some(choice)) => Some(DialogResult::Ok(choice)),
            (Some(_), None) => Some(DialogResult::Cancel),
            (None, choice) => {
                if choice.is_some() {
                    self.choice = choice;
                }
                None
            }
        }
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }
}

/// The name box's text in `bind`: empty while a kept binding holds none.
fn text_of(bind: &mut Bind<'_, String>, cx: &mut BodyCx<'_>) -> String {
    match bind.reborrow().read(cx) {
        Some(Mixed::Same(text)) => text,
        _ => String::new(),
    }
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

/// The action's button state for the name box's text. `OnValidName` follows `name_check` on the trimmed name, and is
/// "not empty" without one, so it holds for a name an entry already has (unlike `OnNewName`).
fn is_enabled(action: &ListManagerAction, entries: &[ListManagerEntry], folder: &Folder, name: &str, name_check: Option<fn(&str) -> bool>) -> bool {
    match action.enabled {
        ActionEnabled::OnEntry { .. } => entries.iter().any(|entry| is_named(entry, folder, name) && accepts(action, entry)),
        ActionEnabled::OnNewName => !name.trim().is_empty() && !entries.iter().any(|entry| is_named(entry, folder, name)),
        ActionEnabled::OnValidName => match name_check {
            Some(check) => check(name.trim()),
            None => !name.trim().is_empty(),
        },
        ActionEnabled::Never => false,
        _ => true,
    }
}

/// A named list with a name box and a column of action buttons (after Excel's Macros dialog).
///
/// - Name box: typing selects the entry with that name; picking a row puts its name in the box. With `searchable` the
///   box also filters the rows to the names that match it.
/// - Keys: Up and Down in the box or the list move the selection, Enter and a double click run the first action when
///   it is enabled, Delete in the list runs the `delete` action, Escape cancels.
/// - Actions are the buttons in a column beside the list: the first action, Cancel, then the other actions in the
///   given order. The app replaces `entries` between passes; the name box keeps its text.
/// - The name box's text is one binding: the caller's with [`name_box`](ListManager::name_box), else the dialog's
///   memory, which each open starts from the `name` prop.
/// - Rename: with `renames`, F2 on an entry of that kind edits its name in the row. A commit puts the name in the name
///   box, reports a choice whose action is [`ListManagerChoice::RENAME`] and leaves the dialog open; Escape reports
///   nothing.
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
    /// The name the box opens with, with its entry selected, while the dialog keeps the text (no
    /// [`name_box`](ListManager::name_box)).
    #[prop(into)]
    name: Option<String>,
    /// Binds the name box's text to `bind`: the caller reads what the user typed and sets the text, which the box
    /// shows on the next pass. The `name` prop does not apply then.
    #[prop]
    name_box: Option<Bind<'a, String>>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Entries of this kind rename in place: a commit reports a [`ListManagerChoice`] with the `RENAME` action, the
    /// entry's id and the trimmed name, and the dialog stays open.
    #[prop]
    renames: Option<u32>,
    /// Whether a name is valid, for [`ActionEnabled::OnValidName`].
    #[prop]
    name_check: Option<fn(&str) -> bool>,
    /// The second button's label when it is not Cancel.
    #[prop(into)]
    cancel_label: Option<Label>,
    /// Draws `content` under the list.
    #[prop(boxed = "FnMut(&mut Ui) + 'a", name = "under_list")]
    under: Option<UnderList<'a>>,
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
            name_box: None,
            open: None,
            renames: None,
            name_check: None,
            cancel_label: None,
            under: None,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ListManagerOutput {
        let ListManager {
            title,
            name_label,
            entries,
            actions,
            empty,
            note,
            searchable,
            name,
            name_box,
            open,
            renames,
            name_check,
            cancel_label,
            under,
        } = self;
        let under = RefCell::new(under);
        // The dialog keeps the text unless the caller binds it; a kept text starts each open from `name`.
        let kept = name_box.is_none();
        let name_box = Rc::new(RefCell::new(name_box.unwrap_or_else(Bind::kept)));
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
        buttons.push(FooterButton::new(cancel_label.unwrap_or_else(|| common.cancel.clone()), DialogRole::Cancel));
        let settled = Rc::clone(&name_box);
        let mut dialog =
            Dialog::new(title.clone()).collection_body().close_on_enter(true).button_place(ButtonPlace::Side).buttons(buttons).settle_buttons(
                move |cx, buttons| {
                    let folder = cx.env.folder();
                    let typed = text_of(&mut settled.borrow_mut(), cx);
                    for (button, action) in buttons.iter_mut().zip(actions) {
                        button.set_enabled(is_enabled(action, entries, &folder, &typed, name_check));
                    }
                },
            );
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }

        let folder = env.folder();
        let activated = Cell::new(None::<usize>);
        let typed = RefCell::new(String::new());
        let renamed = RefCell::new(None::<(usize, String)>);
        let shown = dialog.show(ui, |ui, cx| {
            let mut bind = name_box.borrow_mut();
            let before = text_of(&mut bind, cx);
            let opened = if kept && cx.appeared() { name.clone().unwrap_or_default() } else { before.clone() };
            let text = RefCell::new(opened);
            Stack::new()
                .header(|ui, _| {
                    let mut text = text.borrow_mut();
                    Field::new(name_label.clone()).show(ui, |ui, _| {
                        TextField::new(name_label.clone(), Bind::bound(&mut *text)).select_on_focus().drives(list_id).show(ui);
                    });
                })
                .footer(|ui, _| {
                    let text = text.borrow();
                    if let Some(under) = under.borrow_mut().as_mut() {
                        under(ui);
                    }
                    if let Some(entry) = entries.iter().find(|entry| is_named(entry, &folder, &text)) {
                        Text::new(words.description.clone()).show(ui);
                        Text::new(entry.description.clone()).shade(Shade::Muted).show(ui);
                    }
                    if let Some(note) = &note {
                        Text::new(note.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    }
                })
                .show(ui, |ui, _| {
                    let mut text = text.borrow_mut();
                    let current = entries.iter().position(|entry| is_named(entry, &folder, &text));
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
                        list = list.filter(text.as_str(), |at| vec![Label::from(entries[*at].name.clone())]);
                    }
                    if let Some(kind) = renames {
                        list = list.rename(true).can_rename(move |at| entries[*at].kind == kind);
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
                        text.clone_from(&entries[at].name);
                    }
                    for event in events {
                        match event {
                            CollectionEvent::Activate(at) => {
                                // A lead row left unpicked by a name typed after it runs the first action on that name.
                                if picked == Some(at) {
                                    text.clone_from(&entries[at].name);
                                }
                                let first = actions.first().filter(|action| is_enabled(action, entries, &folder, &text, name_check));
                                activated.set(first.map(|_| 0));
                            }
                            CollectionEvent::Delete(rows) => {
                                if let Some(at) = rows.first() {
                                    text.clone_from(&entries[*at].name);
                                }
                                let delete = actions.iter().position(|action| action.id == DELETE);
                                activated.set(delete.filter(|at| is_enabled(&actions[*at], entries, &folder, &text, name_check)));
                            }
                            CollectionEvent::Rename(EditEvent::Commit { after: rename, .. }) => {
                                let name = rename.name.trim();
                                if !name.is_empty() {
                                    *text = name.to_owned();
                                    *renamed.borrow_mut() = Some((rename.key, text.clone()));
                                }
                            }
                            _ => {}
                        }
                    }
                });
            let text = text.into_inner();
            if text != before {
                let event = EditEvent::set(Mixed::Same(before), text.clone(), Arc::from(name_label.as_str()), None);
                let _ = bind.reborrow().write(cx, &event);
            }
            typed.borrow_mut().clone_from(&text);
            if let Some(at) = activated.get()
                && actions[at].closes
            {
                cx.request_close_as(CloseReason::Activated);
            }
        });

        let DialogOutput { response, closed, picked } = shown.output;
        let at = picked.filter(|at| *at < actions.len()).or(activated.get());
        let choice = at.map(|at| {
            let typed = typed.take();
            let entry = entries.iter().find(|entry| is_named(entry, &folder, &typed) && accepts(&actions[at], entry));
            ListManagerChoice { action: actions[at].id.clone(), entry: entry.map(|entry| entry.id.clone()), name: typed.trim().to_owned() }
        });
        let rename = renamed.take().and_then(|(at, name)| {
            entries.get(at).map(|entry| ListManagerChoice { action: ListManagerChoice::RENAME.to_owned(), entry: Some(entry.id.clone()), name })
        });
        ListManagerOutput { response, closed, choice: choice.or(rename) }
    }
}
