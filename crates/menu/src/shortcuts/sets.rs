//! The set row: the set picker, then New Set…, Rename…, Delete, Save, Revert Set, Export, Import…
//! and Summarize… as scoped commands in a bar that overflows into a menu. New Set… and Rename…
//! open an inline form under the row.

use craft_keymap::profiles::{Profiles, set_name};
use craft_ui::bars::{BarItem, BarItems, ItemDisplay};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, BodyCx, CommandSet, Entry, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::{CommandState, Commands as _};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::TextField;
use egui::Ui;

use super::state::{DialogState, Env, SetForm, SetFormKind};

/// The set row's actions. They belong to the dialog that shows them: a chord or the palette reaches the dialog
/// that holds key focus. The dialog is a modal layer, so they fire while a dialog is open (`in_dialogs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "shortcutSets", scope = "dialog.shortcuts")]
pub(crate) enum SetCommand {
    /// Opens the New Set form.
    #[command(label = "New Set…", modal = "in_dialogs")]
    New,
    /// Opens the Rename form.
    #[command(label = "Rename…", modal = "in_dialogs")]
    Rename,
    /// Asks to delete the active set.
    #[command(label = "Delete", modal = "in_dialogs")]
    Delete,
    /// Stores the edits in the active set.
    #[command(label = "Save", modal = "in_dialogs")]
    Save,
    /// Puts the active set's stored shortcuts back.
    #[command(label = "Revert Set", modal = "in_dialogs")]
    Revert,
    /// Exports the active set as craft-keymap JSON.
    #[command(label = "Export", modal = "in_dialogs")]
    Export,
    /// Asks the app for a set file.
    #[command(label = "Import…", modal = "in_dialogs")]
    Import,
    /// Summarizes the shortcuts.
    #[command(label = "Summarize…", modal = "in_dialogs")]
    Summarize,
}

/// Drawn above the header when the app passes sets and the dialog is editable.
pub(crate) fn show(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) {
    if !env.options.editable || env.sets.is_none() {
        return;
    }
    // The row and its form are the instance of the set scope: the dialog itself keeps the scope of the edit commands.
    Stack::new().command_scope(SetCommand::SCOPE).show(ui, |ui, cx| contents(ui, cx, env, state));
}

fn contents(ui: &mut Ui, cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let Some((p, b)) = env.sets.as_ref() else { return };
    let names = p.names(b);
    let active = p.active_name(b).to_string();
    let builtin = Profiles::is_builtin(&active, b);
    let modified = DialogState::is_modified(env);
    let idle = !state.modal();
    let formats = !env.options.export_formats.is_empty();
    publish(cx, env, idle, builtin, modified, formats);

    let words = env.text.words();
    let set_label = words.set.as_str().to_owned();
    let entries: Vec<Entry<String>> = names.iter().map(|n| Entry::new(n.clone(), n.clone())).collect();
    let mut picked = active.clone();
    let mut export_pick = None;
    Row::new().enabled(idle).show(ui, |ui, _| {
        Text::new(set_label.clone()).show(ui);
        Select::new(set_label.clone(), Bind::bound(&mut picked), &entries).show(ui);
        if modified {
            Text::new(words.modified.as_str().to_owned()).show(ui);
        }
        BarItems::new(items(env, formats)).show(ui);
        if formats {
            export_pick = export_menu(ui, env);
        }
    });
    if picked != active {
        state.set_form = None;
        state.switch_set(env, &picked);
    }
    if let Some(at) = export_pick {
        match at.checked_sub(1).and_then(|i| env.options.export_formats.get(i)) {
            Some(format) => state.export_as(env, format.id),
            None => state.export_set(env),
        }
    }
    while let Some((command, _)) = cx.take_command::<SetCommand>() {
        match command {
            SetCommand::New => state.set_form = Some(SetForm { kind: SetFormKind::New { based_on: None }, name: String::new(), focus: true }),
            SetCommand::Rename => state.set_form = Some(SetForm { kind: SetFormKind::Rename, name: active.clone(), focus: true }),
            SetCommand::Delete => state.ask_delete_set(env),
            SetCommand::Save => state.save_set(env),
            SetCommand::Revert => state.revert_set(env),
            SetCommand::Export => state.export_set(env),
            SetCommand::Import => state.request_import(env),
            SetCommand::Summarize => state.summarize(env),
        }
    }
    form(ui, env, state, idle, &names);
}

/// Publishes the label and the enabled state of each action for this pass.
fn publish(cx: &mut BodyCx<'_>, env: &Env<'_>, idle: bool, builtin: bool, modified: bool, formats: bool) {
    let words = env.text.words();
    let export = if formats { words.export.as_str() } else { words.export_more.as_str() };
    let rows = [
        (SetCommand::New, words.new_set_more.as_str(), idle),
        (SetCommand::Rename, words.rename_more.as_str(), idle && !builtin),
        (SetCommand::Delete, words.delete.as_str(), idle && !builtin),
        (SetCommand::Save, words.save.as_str(), idle && modified),
        (SetCommand::Revert, words.revert_set.as_str(), idle && modified),
        (SetCommand::Export, export, idle),
        (SetCommand::Import, words.import_more.as_str(), idle),
        (SetCommand::Summarize, words.summarize_more.as_str(), idle && env.options.summary),
    ];
    for (command, title, enabled) in rows {
        cx.publish(command, CommandState::new().enabled(enabled).title(title));
    }
}

/// The strip's items. An app that writes other formats gets its own Export menu in place of the Export item.
fn items(env: &Env<'_>, formats: bool) -> Vec<BarItem> {
    let item = |command: SetCommand| BarItem::new(command).display(ItemDisplay::Text);
    let mut items = vec![item(SetCommand::New), item(SetCommand::Rename), item(SetCommand::Delete), item(SetCommand::Save), item(SetCommand::Revert)];
    if !formats {
        items.push(item(SetCommand::Export));
    }
    items.push(item(SetCommand::Import));
    if env.options.summary {
        items.push(item(SetCommand::Summarize));
    }
    items
}

/// The Export menu of an app that writes other formats: craft-keymap JSON first, then the app's. The pick is 0 for
/// the JSON and the format's position plus one for the others.
fn export_menu(ui: &mut Ui, env: &Env<'_>) -> Option<usize> {
    let words = env.text.words();
    let opener = Button::new(words.export.as_str().to_owned()).show(ui);
    let mut entries = vec![Entry::new(0usize, words.keyboard_shortcuts_json.as_str().to_owned())];
    entries.extend(env.options.export_formats.iter().enumerate().map(|(i, f)| Entry::new(i + 1, f.label)));
    Menu::new(words.export.as_str().to_owned()).values().opened_by(&opener, Trigger::Click).pick(ui, &entries).picked
}

/// New Set… (Name, Based on) or Rename… (Name), under the set row.
fn form(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState, idle: bool, names: &[String]) {
    let Some(mut form) = state.set_form.clone() else { return };
    let words = env.text.words();
    let name_label = words.name.as_str().to_owned();
    let (mut submit, mut cancel) = (false, false);
    let ok = match form.kind {
        SetFormKind::New { .. } => words.create.as_str().to_owned(),
        SetFormKind::Rename => words.rename.as_str().to_owned(),
    };
    let current = words.current_shortcuts_heading.as_str().to_owned();
    let based_label = words.based_on.as_str().to_owned();
    let cancel_label = words.cancel.as_str().to_owned();
    Row::new().enabled(idle).show(ui, |ui, _| {
        Text::new(name_label.clone()).show(ui);
        let field = TextField::new(name_label.clone(), Bind::bound(&mut form.name)).show(ui);
        if std::mem::take(&mut form.focus) {
            focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
        }
        submit |= field.submitted;
        cancel |= field.escaped;
        if let SetFormKind::New { based_on } = &mut form.kind {
            Text::new(based_label.clone()).show(ui);
            let mut entries = vec![Entry::new(None, current.clone())];
            entries.extend(names.iter().map(|n| Entry::new(Some(n.clone()), n.clone())));
            Select::new(based_label.clone(), Bind::bound(based_on), &entries).show(ui);
        }
        let valid = set_name(&form.name).is_some();
        submit |= clicked(&Button::new(ok.clone()).enabled(valid).show(ui));
        cancel |= clicked(&Button::new(cancel_label.clone()).show(ui));
    });
    if cancel {
        state.set_form = None;
        return;
    }
    if !submit || set_name(&form.name).is_none() {
        state.set_form = Some(form);
        return;
    }
    state.message = None;
    match &form.kind {
        SetFormKind::New { based_on } => state.new_set(env, &form.name, based_on.as_deref()),
        SetFormKind::Rename => state.rename_set(env, &form.name),
    }
    // A refused name keeps the form open for another try.
    let refused = state.message.as_ref().is_some_and(|m| m.refused);
    state.set_form = refused.then_some(form);
}
