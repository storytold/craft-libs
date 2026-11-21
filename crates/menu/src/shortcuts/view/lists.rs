//! The `Layout::CommandLists` body: Word's Customize Keyboard.

use craft_behavior::{ColumnWidth, Selection, SelectionChange};
use craft_keymap::CommandRef;
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, Props as _, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::runtime::motion::Animate as _;
use craft_ui::text::TextField;
use egui::Ui;

use super::Titles;
use super::detail::{binding_text, label_of, new_shortcut};
use super::table::stamp_of;
use crate::shortcuts::rows::Line;
use crate::shortcuts::state::{DialogState, Env};

/// The lists' share of the width beside the command's keys.
const LISTS_SHARE: f32 = 0.5;
/// The detail column's share of the width beside the lists.
const DETAIL_SHARE: f32 = 0.5;
const PANES: [ColumnWidth; 2] = [ColumnWidth::fill(LISTS_SHARE), ColumnWidth::fill(DETAIL_SHARE)];

/// One frame of the command-lists body: a search field, the Categories list beside the Commands
/// list, then the command's keys, the New Shortcut recorder, the holder line and the Assign and
/// Remove buttons.
pub(super) fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let now = cx.now();
    search(ui, env, state);
    Grid::new(&PANES).fill().show(ui, |ui, _| {
        Stack::new().fill().show(ui, |ui, _| {
            categories(ui, env, state);
            commands(ui, env, state);
        });
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| detail(ui, now, env, state));
    });
}

/// The search field over the commands, the same query the table's search edits.
fn search(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    let label = env.text.words().search.clone();
    let mut query = state.query.clone();
    Field::new(label.clone()).id_salt("shortcuts.search").sizing(Sizing::Content).show(ui, |ui, _| {
        TextField::search(label.clone(), Bind::bound(&mut query)).id_salt("search").show(ui);
    });
    if query != state.query {
        state.set_query(query);
    }
}

/// The Categories list over the dialog's areas.
fn categories(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    let words = env.text.words();
    Text::new(words.categories.clone()).show(ui);
    let labels: Vec<String> = state.areas.iter().map(|area| area.label.clone()).collect();
    let at = state.areas.iter().position(|area| area.id == state.area);
    let mut selection = Selection::new(at.into_iter().collect(), at, at);
    let out = List::<usize>::new(words.categories.clone())
        .id_salt("categories")
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &Titles(&labels), &mut |key, _| ListRow::new(labels[*key].clone()));
    for event in out.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(area) = keys.last().and_then(|key| state.areas.get(*key))
        {
            let id = area.id.clone();
            state.set_area(id);
        }
    }
}

/// The Commands list over the rows of the selected category and the search.
fn commands(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    let words = env.text.words();
    Text::new(words.commands.clone()).show(ui);
    let (commands, labels): (Vec<CommandRef>, Vec<String>) = state
        .lines(env)
        .lines
        .iter()
        .filter_map(|line| match line {
            Line::Row(row) => Some((row.command.clone(), row.label.clone())),
            _ => None,
        })
        .unzip();
    let at = state.selected.as_ref().and_then(|selected| commands.iter().position(|command| command == selected));
    let mut selection = Selection::new(at.into_iter().collect(), at, at);
    let stamp = stamp_of(&state.area, &state.query, &format!("{:?}", state.filter));
    let out = List::<usize>::new(words.commands.clone())
        .id_salt("commands")
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut selection))
        .empty(words.no_matching_commands.clone())
        .rows_stamp(stamp)
        .show_items(ui, &Titles(&labels), &mut |key, _| ListRow::new(labels[*key].clone()));
    for event in out.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(command) = keys.last().and_then(|key| commands.get(*key))
        {
            state.select_row(command);
        }
    }
}

/// The selected command's keys, the New Shortcut field, who holds it, and Assign and Remove.
fn detail(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState) {
    let words = env.text.words();
    let selected = state.selected.clone().filter(|command| env.keymap.bindable(command).is_some());
    let Some(command) = selected else {
        Text::new(words.select_a_command_to_see_and.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let Some(home) = env.keymap.bindable(&command).map(|b| b.scope.clone()) else { return };

    Text::new(words.current_shortcuts.clone()).show(ui);
    let texts: Vec<String> = env.keymap.bindings(&command).iter().map(|binding| binding_text(env, binding, &home)).collect();
    let pick = state.keys_pick.filter(|at| *at < texts.len());
    let mut selection = Selection::new(pick.into_iter().collect(), pick, pick);
    let out = List::<usize>::new(words.current_shortcuts.clone())
        .id_salt("keys")
        .select(SelectMode::Single)
        .sizing(Sizing::Content)
        .natural_height(true)
        .selection(Bind::bound(&mut selection))
        .empty(words.none.clone())
        .show_items(ui, &Titles(&texts), &mut |key, _| ListRow::new(texts[*key].clone()));
    for event in out.events {
        match event {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => state.keys_pick = keys.last().copied(),
            CollectionEvent::Selection { change: SelectionChange::Toggle(key), .. } => {
                state.keys_pick = (state.keys_pick != Some(key)).then_some(key)
            }
            CollectionEvent::Selection { change: SelectionChange::Clear, .. } => state.keys_pick = None,
            _ => {}
        }
    }
    if !env.options.editable {
        return;
    }

    new_shortcut(ui, now, env, state, &command, &home);
    if let Some(field) = &state.field {
        let line = match field.holders.first() {
            Some(holder) => words.now_used_by_command.fill(&[("command", &label_of(env, &holder.command))]),
            None => words.not_used_by_any_command.as_str().to_owned(),
        };
        Text::new(line).shade(Shade::Muted).show(ui);
    }
    // A chord the command already holds is not assigned twice.
    let assignable = state.field.as_ref().is_some_and(|field| !env.keymap.bindings(&command).contains(&field.binding));
    Row::new().show(ui, |ui, _| {
        if clicked(&Button::new(words.assign.clone()).id_salt("assign").enabled(assignable).show(ui)) {
            state.assign_field(env);
        }
        if clicked(&Button::new(words.remove.clone()).id_salt("remove").enabled(pick.is_some()).show(ui)) {
            state.remove_picked(env);
        }
    });
}
