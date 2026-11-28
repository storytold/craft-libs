//! The list of the Layer Comps panel: its columns and rows, the mapping of the list's events to
//! `LayerCompsEvent`s, the row menu and the footer.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource, Mixed, Selection, SelectionChange};
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, ColumnSpec, Props as _, ReorderRule, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
    ToggleColumn,
};
use craft_ui::display::{Shade, Text};
use craft_ui::list::{List, ListRow, RowEvent, ToggleCell, ToggleFace, ToggleState, TrailingFace, TrailingPart};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::{Id, Response, Ui};

use super::{CompOption, LayerCompRow, LayerCompsEvent, LayerCompsWords, Session};
use crate::ui::sweep::{SweepOut, sweep_events};

/// The apply marker column: one cell is on, the applied comp's or the Last Document State row's.
const APPLY: ColumnKey = ColumnKey::new("apply");
/// The name column, the one a double click renames in.
const NAME: ColumnKey = ColumnKey::new("name");
const VISIBILITY: ColumnKey = ColumnKey::new("visibility");
const POSITION: ColumnKey = ColumnKey::new("position");
const APPEARANCE: ColumnKey = ColumnKey::new("appearance");

/// The trailing warning part of a comp that lost layers.
const WARNING: u16 = 0;

// The footer's actions in the order given: previous and next at the start edge, then delete, new and update from the
// end edge, which puts update, new, delete left to right as Photoshop does.
const PREVIOUS: usize = 0;
const NEXT: usize = 1;
const DELETE: usize = 2;
const NEW: usize = 3;
const UPDATE: usize = 4;
const AT_START: usize = 2;

/// The option columns, in the order they show.
const OPTIONS: [CompOption; 3] = [CompOption::Visibility, CompOption::Position, CompOption::Appearance];

/// The column of `option`.
fn column_of(option: CompOption) -> ColumnKey {
    match option {
        CompOption::Visibility => VISIBILITY,
        CompOption::Position => POSITION,
        CompOption::Appearance => APPEARANCE,
    }
}

/// The option whose column is `column`.
fn option_of(column: &ColumnKey) -> Option<CompOption> {
    OPTIONS.into_iter().find(|option| column_of(*option) == *column)
}

/// The icons of `option`'s cell, on and off.
fn icons_of(option: CompOption) -> (&'static str, &'static str) {
    match option {
        CompOption::Visibility => ("eye", "eye_off"),
        CompOption::Position => ("move", "move"),
        CompOption::Appearance => ("sparkles", "sparkles"),
    }
}

/// The column name and the name pattern of the cells of `option`.
fn words_of(words: &LayerCompsWords, option: CompOption) -> (&Label, &Label) {
    match option {
        CompOption::Visibility => (&words.visibility, &words.visibility_cell),
        CompOption::Position => (&words.position, &words.position_cell),
        CompOption::Appearance => (&words.appearance, &words.appearance_cell),
    }
}

/// Whether `comp` records `option`.
fn records(comp: &LayerCompRow, option: CompOption) -> bool {
    match option {
        CompOption::Visibility => comp.visibility,
        CompOption::Position => comp.position,
        CompOption::Appearance => comp.appearance,
    }
}

/// The key of a row: the fixed Last Document State row or a comp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CompKey {
    LastState,
    Comp(u64),
}

/// What the panel's pass shares: the list's id, the comps and the words.
pub(super) struct ListCx<'a> {
    pub(super) list: Id,
    pub(super) label: &'a Label,
    pub(super) comps: &'a [LayerCompRow],
    pub(super) last_state_applied: bool,
    pub(super) words: &'a LayerCompsWords,
    pub(super) env: &'a Env,
}

/// The comp shown at row `at`; `None` for the Last Document State row at 0.
fn comp_at(comps: &[LayerCompRow], at: usize) -> Option<&LayerCompRow> {
    at.checked_sub(1).and_then(|index| comps.get(index))
}

/// The rows: Last Document State, then the comps.
struct CompRows<'a> {
    comps: &'a [LayerCompRow],
    last_state: &'a str,
    last_state_applied: bool,
}

impl DisplayOrder<CompKey> for CompRows<'_> {
    fn len(&self) -> usize {
        self.comps.len() + 1
    }

    fn key(&self, at: usize) -> CompKey {
        comp_at(self.comps, at).map_or(CompKey::LastState, |comp| CompKey::Comp(comp.id))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(comp_at(self.comps, at).map_or(self.last_state, |comp| comp.name.as_str())))
    }
}

impl ItemSource<CompKey> for CompRows<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        let Some(comp) = comp_at(self.comps, at) else { return *column == APPLY && self.last_state_applied };
        if *column == APPLY { comp.applied } else { option_of(column).is_some_and(|option| records(comp, option)) }
    }
}

/// The apply marker cell; the cell fills the `{row}` of its name pattern with the row's label.
fn apply_cell(words: &LayerCompsWords, on: bool) -> ToggleCell {
    ToggleCell::new(APPLY, ToggleFace::Check, ToggleState::of(on), words.apply_cell.clone())
}

/// The cell of `option`; the cell fills the `{row}` of its name pattern with the row's label.
fn option_cell(words: &LayerCompsWords, option: CompOption, on: bool) -> ToggleCell {
    let (column, cell) = words_of(words, option);
    let (on_icon, off_icon) = icons_of(option);
    ToggleCell::new(column_of(option), ToggleFace::icons(IconId::new(on_icon), IconId::new(off_icon)), ToggleState::of(on), cell.clone())
        .tooltip(column.clone())
        .always_visible()
}

/// The row at `at`.
fn build_row(cx: &ListCx<'_>, at: usize) -> ListRow<'static> {
    let words = cx.words;
    let Some(comp) = comp_at(cx.comps, at) else {
        return ListRow::new(words.last_state.clone()).toggle(apply_cell(words, cx.last_state_applied));
    };
    let mut row = ListRow::new(comp.name.clone()).toggle(apply_cell(words, comp.applied));
    for option in OPTIONS {
        row = row.toggle(option_cell(words, option, records(comp, option)));
    }
    if comp.missing > 0 {
        let count = comp.missing as u64;
        let tip = words.missing.text(count, &cx.env.format().integer(count));

        row = row.trailing_part(TrailingPart::new(WARNING), TrailingFace::Icon(IconSpec::svg(IconId::new("warning"))), Label::from(tip));
    }
    row
}

/// One step of the pass: an event, or the cells of a sweep that later cells of the same gesture join.
enum Step {
    Event(LayerCompsEvent),
    Toggle { rows: Vec<CompKey>, option: CompOption, edit: EditEvent<bool> },
}

/// The pass's events in order. The sweep helper writes its toggles here with the list's row keys; they become comp
/// ids when the pass ends.
#[derive(Default)]
struct Steps(Vec<Step>);

impl Steps {
    fn push(&mut self, event: LayerCompsEvent) {
        self.0.push(Step::Event(event));
    }

    /// Restores the last document state once per pass, and only while it is not the state shown.
    fn restore(&mut self, cx: &ListCx<'_>) {
        let reported = self.0.iter().any(|step| matches!(step, Step::Event(LayerCompsEvent::RestoreLastDocumentState)));
        if !cx.last_state_applied && !reported {
            self.push(LayerCompsEvent::RestoreLastDocumentState);
        }
    }

    /// The comp the pass selected last.
    fn selected(&self) -> Option<u64> {
        self.0.iter().rev().find_map(|step| match step {
            Step::Event(LayerCompsEvent::Select(id)) => Some(*id),
            _ => None,
        })
    }

    fn into_events(self) -> Vec<LayerCompsEvent> {
        self.0
            .into_iter()
            .map(|step| match step {
                Step::Event(event) => event,
                Step::Toggle { rows, option, edit } => {
                    let comps = rows.into_iter().filter_map(|key| if let CompKey::Comp(id) = key { Some(id) } else { None }).collect();
                    LayerCompsEvent::SetOption { comps, option, edit }
                }
            })
            .collect()
    }
}

impl SweepOut<CompKey, CompOption> for Steps {
    fn last_toggle(&mut self) -> Option<(CompOption, &EditEvent<bool>, &mut Vec<CompKey>)> {
        match self.0.last_mut() {
            Some(Step::Toggle { rows, option, edit }) => Some((*option, &*edit, rows)),
            _ => None,
        }
    }

    fn push_toggle(&mut self, rows: Vec<CompKey>, column: CompOption, edit: EditEvent<bool>) {
        self.0.push(Step::Toggle { rows, option: column, edit });
    }
}

/// The comp the selection binding holds, if it is one of `comps`.
fn held(bind: &mut Bind<'_, Option<u64>>, cx: &mut BodyCx<'_>, comps: &[LayerCompRow]) -> Option<u64> {
    match bind.reborrow().read(cx) {
        Some(Mixed::Same(Some(id))) if comps.iter().any(|comp| comp.id == id) => Some(id),
        _ => None,
    }
}

/// The comp the footer's Update and Delete act on: the selected one, else the applied one.
fn target(comps: &[LayerCompRow], selected: Option<u64>) -> Option<u64> {
    selected.or_else(|| comps.iter().find(|comp| comp.applied).map(|comp| comp.id))
}

/// Shows the rows, the row menu and the hint of an empty list, and returns what the user did.
pub(super) fn show_list(ui: &mut Ui, stack: &mut BodyCx<'_>, selected: &RefCell<Bind<'_, Option<u64>>>, cx: &ListCx<'_>) -> Vec<LayerCompsEvent> {
    let words = cx.words;
    let before = held(&mut selected.borrow_mut(), stack, cx.comps);
    let selection = before.map_or_else(Selection::default, |id| Selection::single(CompKey::Comp(id)));
    let source = CompRows { comps: cx.comps, last_state: words.last_state.as_str(), last_state_applied: cx.last_state_applied };

    let mut columns = vec![
        ColumnSpec::toggle(APPLY, words.apply.clone(), ToggleColumn::exclusive()),
        ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0)),
    ];
    for option in OPTIONS {
        let (name, _) = words_of(words, option);
        columns.push(ColumnSpec::toggle(column_of(option), name.clone(), ToggleColumn::sweepable()).icon(IconId::new(icons_of(option).0)));
    }
    // No `accepts` rule: a drop before Last Document State lands at index 0 and the row stays first, so every
    // spot is a drop target and `can_drag` keeps the fixed row out of the drag.
    let mut list = List::new(cx.label.clone())
        .id(cx.list)
        .select(SelectMode::Single)
        .allow_empty(true)
        .reorder(ReorderRule::new())
        .rename(true)
        .rename_column(NAME)
        .columns(columns)
        .selection(Bind::given(selection))
        .can_rename(|key| matches!(key, CompKey::Comp(_)))
        .can_drag(|key| matches!(key, CompKey::Comp(_)));
    if cx.comps.is_empty() {
        // The hint shows under the one row.
        list = list.sizing(Sizing::Content);
    }
    let out = list.show_items(ui, &source, &mut |_, item| {
        if let Some(comp) = comp_at(cx.comps, item.at)
            && !comp.comment.is_empty()
        {
            let comment = comp.comment.clone();
            item.tip(move |ui| {
                let _ = Text::new(comment).show(ui);
            });
        }
        build_row(cx, item.at)
    });

    let mut steps = Steps::default();
    let menu = {
        let session = stack.memory::<Session>();
        map_events(cx, session, out.events, &mut steps);
        session.menu
    };
    if show_menu(ui, &out.response, cx, menu, &mut steps) {
        stack.memory::<Session>().menu = None;
    }
    if cx.comps.is_empty() {
        let _ = Text::new(words.hint.clone()).shade(Shade::Muted).show(ui);
    }
    // A bound selection takes the pick; a given one leaves it to the app, which reads `Select`.
    if let Some(id) = steps.selected().filter(|id| Some(*id) != before) {
        let edit = EditEvent::set(Mixed::Same(before), Some(id), Arc::from(cx.label.as_str()), None);
        selected.borrow_mut().reborrow().write(stack, &edit);
    }
    steps.into_events()
}

/// Maps the list's events to the panel's, in order.
fn map_events(cx: &ListCx<'_>, session: &mut Session, list: Vec<CollectionEvent<CompKey, RowEvent>>, steps: &mut Steps) {
    let mut label: Arc<str> = Arc::from("");
    for event in list {
        match event {
            CollectionEvent::Selection { change, .. } => {
                let key = match change {
                    SelectionChange::Set(keys) => keys.last().copied(),
                    SelectionChange::Toggle(key) => Some(key),
                    _ => None,
                };
                match key {
                    Some(CompKey::Comp(id)) => steps.push(LayerCompsEvent::Select(id)),
                    Some(CompKey::LastState) => steps.restore(cx),
                    None => {}
                }
            }
            CollectionEvent::Activate(CompKey::Comp(id)) => steps.push(LayerCompsEvent::Apply(id)),
            CollectionEvent::Activate(CompKey::LastState) => steps.restore(cx),
            CollectionEvent::Reorder(edit) => {
                // The move counts the remaining rows with Last Document State at 0, the comp index is one less.
                if let EditEvent::Commit { after: moved, .. } | EditEvent::Set { after: moved, .. } = edit
                    && let CompKey::Comp(comp) = moved.key
                {
                    steps.push(LayerCompsEvent::Reorder { comp, index: moved.to.saturating_sub(1) });
                }
            }
            CollectionEvent::Rename(edit) => {
                if let EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } = edit
                    && let CompKey::Comp(comp) = after.key
                    && let Some(shown) = cx.comps.iter().find(|row| row.id == comp)
                {
                    let name = after.name.trim();
                    if !name.is_empty() && name != shown.name {
                        steps.push(LayerCompsEvent::Rename { comp, name: name.to_owned() });
                    }
                }
            }
            CollectionEvent::ContextMenu(request) => {
                session.menu = match request.key {
                    CompKey::Comp(comp) => Some(comp),
                    CompKey::LastState => None,
                };
            }
            CollectionEvent::Item(CompKey::Comp(comp), RowEvent::Trailing { part, .. }) if part == TrailingPart::new(WARNING) => {
                steps.push(LayerCompsEvent::ClearWarning(comp));
            }
            CollectionEvent::Toggle(edit) => {
                if let EditEvent::Begin { label: named, .. } = &edit {
                    label = named.clone();
                }
                let column = match &edit {
                    EditEvent::Update { value: cell, .. } | EditEvent::Set { after: cell, .. } | EditEvent::Commit { after: cell, .. } => {
                        cell.column.clone()
                    }
                    EditEvent::Begin { before: Mixed::Same(cell), .. } | EditEvent::Cancel { before: Some(Mixed::Same(cell)), .. } => {
                        cell.column.clone()
                    }
                    _ => continue,
                };
                if column == APPLY {
                    if let EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } = &edit
                        && after.on
                    {
                        match after.key {
                            CompKey::Comp(id) => steps.push(LayerCompsEvent::Apply(id)),
                            CompKey::LastState => steps.restore(cx),
                        }
                    }
                } else if let Some(option) = option_of(&column) {
                    sweep_events(session.sweeps.of(option), option, edit, &mut label, steps);
                }
            }
            _ => {}
        }
    }
}

/// The row menu of the comp the last context request named: Move Up and Move Down, each disabled at its end. Returns
/// whether the menu is over (it closed this pass, or its comp is gone), so the caller forgets the target.
fn show_menu(ui: &mut Ui, opener: &Response, cx: &ListCx<'_>, comp: Option<u64>, steps: &mut Steps) -> bool {
    let Some(comp) = comp else { return false };
    let Some(at) = cx.comps.iter().position(|row| row.id == comp) else { return true };
    let words = cx.words;
    let shown = Menu::context(words.row_menu.clone(), opener).show(ui, |ui, _| {
        let up = MenuItem::new(words.move_up.clone()).enabled(at > 0).show(ui).ran;
        let down = MenuItem::new(words.move_down.clone()).enabled(at + 1 < cx.comps.len()).show(ui).ran;
        if up {
            at.checked_sub(1)
        } else if down {
            Some(at + 1)
        } else {
            None
        }
    });
    let closed = shown.output.closed.is_some();
    if let Some(index) = shown.inner.flatten() {
        steps.push(LayerCompsEvent::Reorder { comp, index });
    }
    closed
}

/// Shows the footer: previous and next at the start, then update, new and delete at the end.
pub(super) fn show_footer(ui: &mut Ui, stack: &mut BodyCx<'_>, selected: &RefCell<Bind<'_, Option<u64>>>, cx: &ListCx<'_>) -> Vec<LayerCompsEvent> {
    let words = cx.words;
    let chosen = held(&mut selected.borrow_mut(), stack, cx.comps);
    let target = target(cx.comps, chosen);
    let any = !cx.comps.is_empty();
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    let actions = [
        FooterAction::new(icon("chevrons_left"), words.previous.clone()).enabled(any),
        FooterAction::new(icon("chevrons_right"), words.next.clone()).enabled(any),
        FooterAction::new(icon("trash"), words.delete.clone()).enabled(target.is_some()),
        FooterAction::new(icon("plus"), words.new_comp.clone()),
        FooterAction::new(icon("rotate_cw"), words.update.clone()).enabled(target.is_some()),
    ];
    let out = FooterBar::actions(actions).at_start(AT_START).show_footer(ui);
    let event = match out.picked {
        Some(PREVIOUS) => Some(LayerCompsEvent::Previous),
        Some(NEXT) => Some(LayerCompsEvent::Next),
        Some(DELETE) => target.map(LayerCompsEvent::Delete),
        Some(NEW) => Some(LayerCompsEvent::New),
        Some(UPDATE) => target.map(LayerCompsEvent::Update),
        _ => None,
    };
    event.into_iter().collect()
}
