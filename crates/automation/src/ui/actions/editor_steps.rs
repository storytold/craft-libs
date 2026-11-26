//! One shared List row per ActionStep; expanded summaries stay inside their owning row.

use craft_behavior::{DisplayOrder, EditEvent, ItemSource, ReorderKeys, Selection};
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnKey, ColumnSpec, Props as _, ReorderRule, SelectMode, ShowCollection as _, Sizing, SpaceAction, SweepStep,
    ToggleColumn,
};
use craft_ui::list::{List, ListRow, ToggleCell, ToggleState};
use craft_ui::runtime::env::Label;
use egui::Ui;
use std::borrow::Cow;

use super::editor::{ActionEditorHost, ActionEditorState, ActionStepMove};
use super::rows::{INCLUDE_COLUMN, NAME_COLUMN};
use super::words::ActionEditorWords;
use crate::actions::{Action, moved_index};

struct Rows<'a> {
    action: &'a Action,
    labels: Vec<String>,
    details: Vec<Option<String>>,
    state: &'a ActionEditorState,
    editable: bool,
}

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.action.steps.len()
    }
    fn key(&self, at: usize) -> usize {
        at
    }
    fn has_children(&self, at: usize) -> bool {
        self.details[at].is_some()
    }
    fn open(&self, at: usize) -> bool {
        self.state.expanded.contains(&at)
    }
    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.labels.get(at).map(|label| Cow::Borrowed(label.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == INCLUDE_COLUMN && self.action.step_enabled(at)
    }
    fn toggle_enabled(&self, at: usize, column: &ColumnKey) -> bool {
        self.editable && *column == INCLUDE_COLUMN && at < self.action.steps.len()
    }
}

pub(super) fn show(
    ui: &mut Ui,
    action: &mut Action,
    state: &mut ActionEditorState,
    host: Option<&dyn ActionEditorHost>,
    words: &ActionEditorWords,
    editable: bool,
) -> Vec<ActionStepMove> {
    let labels = action.steps.iter().map(|step| host.map_or_else(|| step.command.clone(), |host| host.step_label(step))).collect();
    let details = action.steps.iter().map(|step| host.and_then(|host| host.step_summary(step))).collect();
    let lead = state.selected;
    let mut selection = Selection::new(lead.into_iter().collect(), lead, lead);
    let rows = Rows { action, labels, details, state, editable };
    let mut list = List::new(words.steps.clone())
        .id_salt("action-editor-steps")
        .tree(true)
        .select(SelectMode::Single)
        .allow_empty(true)
        .sizing(Sizing::Content)
        .space(SpaceAction::ToggleCell)
        .columns(vec![
            ColumnSpec::toggle(INCLUDE_COLUMN, Label::Empty, ToggleColumn::sweepable()),
            ColumnSpec::new(NAME_COLUMN, Label::Empty, craft_behavior::ColumnWidth::fill(1.0)),
        ])
        .selection(Bind::bound(&mut selection));
    if editable {
        list = list.reorder(ReorderRule::new().keys(ReorderKeys::AltArrows));
    }
    let out = list.show_items(ui, &rows, &mut |step, item| {
        let mut row = ListRow::new(rows.labels[*step].clone()).cells(item.cells()).state(item.row.clone()).toggle(ToggleCell::check(
            INCLUDE_COLUMN,
            ToggleState::of(action.step_enabled(*step)),
            words.include.clone(),
        ));
        if let Some(summary) = &rows.details[*step] {
            row = row.group(rows.open(*step));
            if rows.open(*step) {
                row = row.detail(summary.clone());
            }
        }
        row
    });
    drop(rows);
    if ui.is_sizing_pass() {
        return Vec::new();
    }
    let mut step_moves = Vec::new();
    state.selected = selection.lead.filter(|&step| step < action.steps.len());
    for event in out.events {
        match event {
            CollectionEvent::Open(change) if change.key < action.steps.len() => {
                if change.open {
                    state.expanded.insert(change.key);
                } else {
                    state.expanded.remove(&change.key);
                }
            }
            CollectionEvent::Toggle(edit) if editable => {
                if let Some(SweepStep::Flip(change) | SweepStep::Restore(change)) = state.include.feed(&edit)
                    && change.key < action.steps.len()
                {
                    if change.on {
                        action.disabled.remove(&change.key);
                    } else {
                        action.disabled.insert(change.key);
                    }
                }
            }
            CollectionEvent::Reorder(EditEvent::Set { after, .. })
                if editable && !after.copy && after.parent.is_none() && after.keys == [after.key] =>
            {
                let (from, to) = (after.key, after.to);
                if from != to && from < action.steps.len() && to < action.steps.len() {
                    let moved = action.steps.remove(from);
                    action.steps.insert(to, moved);
                    step_moves.push(ActionStepMove { from, to });
                    action.remap_steps(|step| Some(moved_index(step, from, to)));
                    state.selected = state.selected.map(|step| moved_index(step, from, to));
                    state.expanded = state.expanded.iter().map(|&step| moved_index(step, from, to)).collect();
                }
            }
            _ => {}
        }
    }
    step_moves
}
