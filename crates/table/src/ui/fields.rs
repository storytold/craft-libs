//! The field list pane of a pivot table.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource, Mixed};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    CollectionEvent, ColumnKind, ColumnSpec, Pattern, Props as _, Remember, ReorderMove, ReorderRule, Show as _, ShowCollection as _,
    ShowContainer as _, SweepApply, SweepStep, ToggleColumn, Trigger, clicked,
};
use craft_ui::display::{Divider, Shade, Text};
use craft_ui::list::{List, ListRow, ToggleCell, ToggleState};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::{Panel, Section};
use craft_ui::style::TypeRole;
use egui::{Id, Response, Ui};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The pane's words, English by default.
    #[non_exhaustive]
    pub struct FieldListWords {
        title: "fieldList.title" = "PivotTable Fields";
        choose: "fieldList.choose" = "Choose fields to add to report:";
        areas_hint: "fieldList.areasHint" = "Use a field's menu to move it between areas:";
        filters: "fieldList.filters" = "Filters";
        drop_here: "fieldList.dropHere" = "Drag fields here";
        columns: "fieldList.columns" = "Columns";
        rows: "fieldList.rows" = "Rows";
        values: "fieldList.values" = "Values";
        move_to: template "fieldList.moveTo" = "Move to {area}";
        move_up: "fieldList.moveUp" = "Move Up";
        move_down: "fieldList.moveDown" = "Move Down";
        remove: "fieldList.remove" = "Remove Field";
        refresh: "fieldList.refresh" = "Refresh";
        calculated: "fieldList.calculated" = "Calculated Field…";
        layout: "fieldList.layout" = "Report Layout";
        compact: "fieldList.compact" = "Show in Compact Form";
        outline: "fieldList.outline" = "Show in Outline Form";
        tabular: "fieldList.tabular" = "Show in Tabular Form";
        values_field: "fieldList.valuesField" = "Σ Values";
        samples_separator: "fieldList.samplesSeparator" = ", ";
        summarize_sum: "fieldList.summarizeSum" = "Summarize by Sum";
        summarize_count: "fieldList.summarizeCount" = "Summarize by Count";
        summarize_average: "fieldList.summarizeAverage" = "Summarize by Average";
        summarize_max: "fieldList.summarizeMax" = "Summarize by Max";
        summarize_min: "fieldList.summarizeMin" = "Summarize by Min";
        show_as_normal: "fieldList.showAsNormal" = "Show as No Calculation";
        show_as_percent_of_grand_total: "fieldList.showAsPercentOfGrandTotal" = "Show as % of Grand Total";
        show_as_running_total: "fieldList.showAsRunningTotal" = "Show as Running Total";
        show_as_rank: "fieldList.showAsRank" = "Show as Rank";
        sort_ascending: "fieldList.sortAscending" = "Sort A to Z";
        sort_descending: "fieldList.sortDescending" = "Sort Z to A";
        group_years: "fieldList.groupYears" = "Group by Years";
        group_quarters: "fieldList.groupQuarters" = "Group by Quarters";
        group_months: "fieldList.groupMonths" = "Group by Months";
        ungroup: "fieldList.ungroup" = "Ungroup";
    }
}

/// A pivot area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldArea {
    /// The report filters.
    Filters,
    /// The column fields.
    Columns,
    /// The row fields.
    Rows,
    /// The value fields.
    Values,
}

impl FieldArea {
    /// Excel's order in the pane.
    pub const ALL: [FieldArea; 4] = [FieldArea::Filters, FieldArea::Columns, FieldArea::Rows, FieldArea::Values];
}

/// A source field: its name, whether it holds numbers (ticked, it goes to Values; text to Rows).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PivotField {
    /// The field's name, as the checklist shows it.
    pub name: String,
    /// Whether the field holds numbers.
    pub numeric: bool,
    /// A few of the field's values, shown as the checklist row's tooltip.
    pub samples: Vec<String>,
}

impl PivotField {
    /// A field named `name`, with no samples.
    pub fn new(name: impl Into<String>, numeric: bool) -> Self {
        PivotField { name: name.into(), numeric, samples: Vec::new() }
    }

    /// The values shown as the checklist row's tooltip.
    #[must_use]
    pub fn samples(mut self, samples: Vec<String>) -> Self {
        self.samples = samples;
        self
    }
}

/// Where the "Σ Values" pseudo-field sits: the app passes it when the pivot has two or more value fields, in Rows or
/// Columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ValuesRow {
    /// Rows or Columns.
    pub area: FieldArea,
    /// Its place among that area's fields; past the end puts it last.
    pub at: usize,
}

impl ValuesRow {
    /// The row at place `at` of `area`.
    pub fn new(area: FieldArea, at: usize) -> Self {
        ValuesRow { area, at }
    }
}

/// How a value field is summarized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Summary {
    /// The sum of the values.
    Sum,
    /// How many values there are.
    Count,
    /// The mean of the values.
    Average,
    /// The largest value.
    Max,
    /// The smallest value.
    Min,
}

/// How a value field is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShowAs {
    /// Show the plain value.
    NoCalculation,
    /// Show the value as a share of the grand total.
    PercentOfGrandTotal,
    /// Show a running total down the items.
    RunningTotal,
    /// Show the value's rank among the items.
    Rank,
}

/// A change of a value field's settings: its summary or how it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueSetting {
    /// Change how the value is summarized.
    Summarize(Summary),
    /// Change how the value is shown.
    ShowAs(ShowAs),
}

/// The order a field's items sort in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldSort {
    /// A to Z.
    Ascending,
    /// Z to A.
    Descending,
}

/// How a date field groups its items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldGroup {
    /// By years.
    Years,
    /// By quarters.
    Quarters,
    /// By months.
    Months,
    /// Remove the grouping.
    Ungroup,
}

/// The report layouts of the Report Layout menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReportLayout {
    /// Compact form.
    Compact,
    /// Outline form.
    Outline,
    /// Tabular form.
    Tabular,
}

/// Fields moving to a place of an area.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FieldMove {
    /// The moved fields, in the order they were listed.
    pub fields: Vec<String>,
    /// The place of the first moved field among the area's fields once the moved ones are taken out of it.
    pub position: usize,
}

/// What the pane asks of the app.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FieldListEdit {
    /// Ticked: add it to its default area. A tick sweep reports one `Add` or `Remove` per row it changes.
    #[non_exhaustive]
    Add {
        /// Name of the source field.
        field: String,
    },
    /// Unticked, or Remove Field from an area's menu or the Delete key.
    #[non_exhaustive]
    Remove {
        /// Name of the source field.
        field: String,
        /// The area the field was removed from; `None` for a tick of the checklist, which removes it from every area.
        from: Option<FieldArea>,
    },
    /// A field's menu (Move to …, Move Up, Move Down), or a drag between or inside the areas. A drag is one gesture:
    /// the app previews the updates and applies the commit.
    #[non_exhaustive]
    Move {
        /// The area the fields move to.
        to: FieldArea,
        /// The gesture of the move.
        change: EditEvent<FieldMove>,
    },
    /// The "Σ Values" row moved to Rows or Columns, by its menu or a drop. The app places it.
    #[non_exhaustive]
    MoveValues {
        /// The area the Values row moves to.
        to: FieldArea,
    },
    /// A value field's Summarize by and Show as entries.
    #[non_exhaustive]
    ValueSettings {
        /// Name of the value field.
        field: String,
        /// The new setting.
        setting: ValueSetting,
    },
    /// A field's Sort A to Z and Z to A entries.
    #[non_exhaustive]
    Sort {
        /// Name of the source field.
        field: String,
        /// The order to sort in.
        order: FieldSort,
    },
    /// A field's Group by and Ungroup entries.
    #[non_exhaustive]
    Group {
        /// Name of the date field.
        field: String,
        /// How to group its items.
        by: FieldGroup,
    },
    /// Refresh the pivot table from its source.
    Refresh,
    /// Open the calculated-field dialog.
    CalculatedField,
    /// The Report Layout menu.
    Layout(ReportLayout),
}

/// What the pane did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct FieldListPaneOutput {
    /// The pane's response.
    pub response: Response,
    /// This pass's edits, in order.
    pub edits: Vec<FieldListEdit>,
}

impl craft_ui::contract::Outcome for FieldListPaneOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The PivotTable Fields task pane: a field checklist, then the four areas each listing its fields, with Refresh,
/// Calculated Field… and the Report Layout menu in the footer.
///
/// - The checklist is a `List` whose rows carry a check toggle column (ticked: [`FieldListEdit::Add`], unticked:
///   [`FieldListEdit::Remove`]); a row's tooltip lists the field's samples.
/// - Each area is a `Section` holding a `List` with a reorder rule, so fields move inside an area by drag or Alt+arrow
///   and between areas by drag. Dropped onto a field of another area, the moved fields land before it. A row's context
///   menu (and the Menu key) has Move to each other area, Move Up, Move Down, for a value field Summarize by and
///   Show as, for another field Sort and Group, then Remove Field. Delete removes the lead field. The "Σ Values" row
///   moves between Rows and Columns only. Moves arrive as [`FieldListEdit::Move`].
/// - Keyboard: each list is one Tab stop, then the footer buttons.
#[derive(Pattern)]
pub struct FieldListPane<'a> {
    fields: &'a [PivotField],
    areas: &'a [(FieldArea, Vec<String>)],
    values_row: Option<ValuesRow>,
}

impl<'a> FieldListPane<'a> {
    /// A pane listing the source `fields`, with `areas` giving the field names placed in each pivot area.
    pub fn new(fields: &'a [PivotField], areas: &'a [(FieldArea, Vec<String>)]) -> Self {
        FieldListPane { fields, areas, values_row: None }
    }

    /// Shows the "Σ Values" row at `row`; the app positions it and passes it only while the pivot has two or more
    /// value fields.
    #[must_use]
    pub fn values_row(mut self, row: Option<ValuesRow>) -> Self {
        self.values_row = row;
        self
    }

    /// Shows the pane across the room offered and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> FieldListPaneOutput {
        let FieldListPane { fields, areas, values_row } = self;
        let words = Env::of(ui).words::<FieldListWords>();
        let edits = RefCell::new(Vec::new());
        let ids = FieldArea::ALL.map(|area| ui.scope_id().with(("fields-area", area)));
        let model = Model { fields, areas, values_row, words: &words };
        let shown = Panel::new(words.title.clone()).footer(|ui, _| model.footer(ui, &edits)).show(ui, |ui, cx| {
            model.checklist(ui, cx, &edits);
            Divider::horizontal().show(ui);
            Text::new(words.areas_hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            for area in FieldArea::ALL {
                model.area(ui, area, &ids, &edits);
            }
        });
        FieldListPaneOutput { response: shown.output.response, edits: edits.into_inner() }
    }
}

/// A row of an area's list: a field, or the "Σ Values" pseudo-field.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Slot {
    Field(String),
    Values,
}

impl Slot {
    fn field(&self) -> Option<&String> {
        match self {
            Slot::Field(name) => Some(name),
            Slot::Values => None,
        }
    }
}

/// The rows of one area's list.
struct Slots<'a> {
    slots: &'a [Slot],
    values: &'a str,
}

impl DisplayOrder<Slot> for Slots<'_> {
    fn len(&self) -> usize {
        self.slots.len()
    }

    fn key(&self, at: usize) -> Slot {
        self.slots[at].clone()
    }

    fn index_of(&self, key: &Slot) -> Option<usize> {
        self.slots.iter().position(|slot| slot == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.slots.get(at).map(|slot| Cow::Borrowed(slot.field().map_or(self.values, String::as_str)))
    }
}

impl ItemSource<Slot> for Slots<'_> {}

/// The rows of the checklist: every source field, with whether it is placed in an area.
struct Checklist<'a> {
    fields: &'a [PivotField],
    ticked: Vec<bool>,
}

const CHECK: ColumnKey = ColumnKey::new("check");

impl DisplayOrder<String> for Checklist<'_> {
    fn len(&self) -> usize {
        self.fields.len()
    }

    fn key(&self, at: usize) -> String {
        self.fields[at].name.clone()
    }

    fn index_of(&self, key: &String) -> Option<usize> {
        self.fields.iter().position(|field| field.name == *key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.fields.get(at).map(|field| Cow::Borrowed(field.name.as_str()))
    }
}

impl ItemSource<String> for Checklist<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == CHECK && self.ticked.get(at).copied().unwrap_or(false)
    }
}

/// The area whose menu is open and the row it was asked for, kept in the area's section.
#[derive(Clone, Default)]
struct OpenMenu {
    target: Option<Slot>,
}

/// What a pick of a field's menu stands for.
#[derive(Clone, Copy)]
enum Pick {
    MoveTo(FieldArea),
    Up,
    Down,
    Summarize(Summary),
    ShowAs(ShowAs),
    Sort(FieldSort),
    Group(FieldGroup),
    Remove,
}

/// What every part of a pass reads.
struct Model<'a> {
    fields: &'a [PivotField],
    areas: &'a [(FieldArea, Vec<String>)],
    values_row: Option<ValuesRow>,
    words: &'a FieldListWords,
}

type Edits = RefCell<Vec<FieldListEdit>>;

impl Model<'_> {
    fn fields_in(&self, area: FieldArea) -> &[String] {
        self.areas.iter().find(|(a, _)| *a == area).map_or(&[], |(_, fields)| fields.as_slice())
    }

    /// The rows of an area's list: its fields, with the "Σ Values" row where the app put it.
    fn slots(&self, area: FieldArea) -> Vec<Slot> {
        let mut slots: Vec<Slot> = self.fields_in(area).iter().cloned().map(Slot::Field).collect();
        if let Some(row) = self.values_row.filter(|row| row.area == area) {
            slots.insert(row.at.min(slots.len()), Slot::Values);
        }
        slots
    }

    fn name(&self, area: FieldArea) -> Label {
        match area {
            FieldArea::Filters => self.words.filters.clone(),
            FieldArea::Columns => self.words.columns.clone(),
            FieldArea::Rows => self.words.rows.clone(),
            FieldArea::Values => self.words.values.clone(),
        }
    }

    fn ticked(&self, field: &str) -> bool {
        self.areas.iter().any(|(_, fields)| fields.iter().any(|f| f == field))
    }

    /// The edit that sets `field` ticked or unticked.
    fn tick_edit(field: String, on: bool) -> FieldListEdit {
        if on { FieldListEdit::Add { field } } else { FieldListEdit::Remove { field, from: None } }
    }

    /// The checklist of source fields.
    fn checklist(&self, ui: &mut Ui, cx: &mut impl Remember, edits: &Edits) {
        let source = Checklist { fields: self.fields, ticked: self.fields.iter().map(|field| self.ticked(&field.name)).collect() };
        let columns = vec![ColumnSpec::new(CHECK, Label::Empty, ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable()))];
        let separator = self.words.samples_separator.as_str();
        let out = List::<String>::new(self.words.choose.clone()).natural_height(true).columns(columns).header(false).show_items(
            ui,
            &source,
            &mut |key, icx| {
                let state = ToggleState::of(source.ticked[icx.at]);
                let row = ListRow::new(key.clone()).cells(icx.cells()).toggle(ToggleCell::check(CHECK, state, "{row}"));
                match self.fields.get(icx.at).filter(|field| !field.samples.is_empty()) {
                    Some(field) => row.tooltip(field.samples.join(separator)),
                    None => row,
                }
            },
        );
        let mut seen = HashSet::new();
        for event in out.events {
            let CollectionEvent::Toggle(edit) = event else { continue };
            let step = cx.keep::<SweepApply>().feed(&edit);
            let Some(SweepStep::Flip(toggle) | SweepStep::Restore(toggle)) = step else {
                continue;
            };
            if self.ticked(&toggle.key) != toggle.on && seen.insert(toggle.key.clone()) {
                edits.borrow_mut().push(Self::tick_edit(toggle.key.clone(), toggle.on));
            }
        }
    }

    /// One area: a section named by the area over a list of its fields.
    fn area(&self, ui: &mut Ui, area: FieldArea, ids: &[Id; 4], edits: &Edits) {
        let name = self.name(area);
        let slots = self.slots(area);
        let at = FieldArea::ALL.iter().position(|other| *other == area).unwrap_or_default();
        let others: Vec<Id> = ids.iter().enumerate().filter(|(n, _)| *n != at).map(|(_, id)| *id).collect();
        // The "Σ Values" row goes only to Rows and Columns.
        let values_here = matches!(area, FieldArea::Rows | FieldArea::Columns);
        Section::fixed(name.clone()).show(ui, |ui, cx| {
            let source = Slots { slots: &slots, values: self.words.values_field.as_str() };
            let others = others.clone();
            let out = List::<Slot>::new(name.clone())
                .id(ids[at])
                .natural_height(true)
                .empty(self.words.drop_here.clone())
                .reorder(ReorderRule::new())
                .deletable(true)
                .accept_from(move |from| others.contains(&from))
                .accepts(move |drag, _, _| values_here || !drag.keys.contains(&Slot::Values))
                .show_items(ui, &source, &mut |key, _| match key {
                    Slot::Field(field) => ListRow::new(field.clone()),
                    Slot::Values => ListRow::new(self.words.values_field.clone()),
                });
            for event in out.events {
                match event {
                    CollectionEvent::Reorder(change) => edits.borrow_mut().extend(self.reordered(&slots, area, change)),
                    CollectionEvent::Dropped { keys, insert, .. } => edits.borrow_mut().push(self.dropped(&slots, area, insert, &keys)),
                    CollectionEvent::ContextMenu(request) => cx.keep::<OpenMenu>().target = Some(request.key),
                    CollectionEvent::Delete(keys) => {
                        let removed =
                            keys.iter().filter_map(Slot::field).map(|field| FieldListEdit::Remove { field: field.clone(), from: Some(area) });
                        edits.borrow_mut().extend(removed);
                    }
                    _ => {}
                }
            }
            let target = cx.keep::<OpenMenu>().target.clone().filter(|slot| slots.contains(slot));
            if let Some(slot) = target {
                let mut picks = Vec::new();
                let menu = Menu::context(name.clone(), &out.response).show(ui, |ui, _| self.menu_rows(ui, area, &slot, &mut picks));
                if menu.output.closed.is_some() {
                    cx.keep::<OpenMenu>().target = None;
                }
                edits.borrow_mut().extend(picks.into_iter().filter_map(|pick| self.pick_edit(area, &slot, pick)));
            }
        });
    }

    /// The edit a move of an area's rows stands for: a [`FieldListEdit::Move`] of its fields, or
    /// [`FieldListEdit::MoveValues`] when only the "Σ Values" row moved (reported when the move ends).
    fn reordered(&self, slots: &[Slot], area: FieldArea, change: EditEvent<ReorderMove<Slot>>) -> Option<FieldListEdit> {
        let only_values = change.value().is_some_and(|mv| mv.keys.iter().all(|slot| slot.field().is_none()));
        if only_values {
            return matches!(change, EditEvent::Commit { .. } | EditEvent::Set { .. }).then_some(FieldListEdit::MoveValues { to: area });
        }
        Some(FieldListEdit::Move { to: area, change: change.map(|mv| field_move(slots, &mv)) })
    }

    /// The edit a drop of rows of another area at slot index `insert` stands for: the fields land before the slot
    /// there, or at the end when `insert` is past the last slot (an empty area has one spot).
    fn dropped(&self, slots: &[Slot], area: FieldArea, insert: usize, keys: &[Slot]) -> FieldListEdit {
        let fields: Vec<String> = keys.iter().filter_map(Slot::field).cloned().collect();
        if fields.is_empty() {
            return FieldListEdit::MoveValues { to: area };
        }
        let at = insert.min(slots.len());
        let position = slots[..at].iter().filter(|slot| slot.field().is_some()).count();
        FieldListEdit::Move { to: area, change: Self::set(FieldMove { fields, position }) }
    }

    /// A move that happens at once, as a menu item makes it.
    fn set(mv: FieldMove) -> EditEvent<FieldMove> {
        EditEvent::set(Mixed::Mixed, mv, Arc::from("Move Field"), None)
    }

    /// A field's menu: Move to each other area, Move Up and Move Down, then for a value field Summarize by and Show
    /// as, for any other Sort and Group, then Remove Field. The "Σ Values" row's menu has only Move to Rows and
    /// Columns.
    fn menu_rows(&self, ui: &mut Ui, area: FieldArea, slot: &Slot, picks: &mut Vec<Pick>) {
        let words = self.words;
        let mut item = |ui: &mut Ui, label: Label, pick: Pick, enabled: bool| {
            if MenuItem::new(label).enabled(enabled).show(ui).ran {
                picks.push(pick);
            }
        };
        let move_to = |target: FieldArea| words.move_to.fill(&[("area", self.name(target).as_str())]);
        match slot {
            Slot::Values => {
                for target in [FieldArea::Rows, FieldArea::Columns].into_iter().filter(|target| *target != area) {
                    item(ui, Label::from(move_to(target)), Pick::MoveTo(target), true);
                }
            }
            Slot::Field(field) => {
                let names = self.fields_in(area);
                let position = names.iter().position(|name| name == field);
                for target in FieldArea::ALL.into_iter().filter(|target| *target != area) {
                    item(ui, Label::from(move_to(target)), Pick::MoveTo(target), true);
                }
                Divider::horizontal().show(ui);
                item(ui, words.move_up.clone(), Pick::Up, position.is_some_and(|at| at > 0));
                item(ui, words.move_down.clone(), Pick::Down, position.is_some_and(|at| at + 1 < names.len()));
                Divider::horizontal().show(ui);
                let settings: Vec<(Label, Pick)> = if area == FieldArea::Values {
                    vec![
                        (words.summarize_sum.clone(), Pick::Summarize(Summary::Sum)),
                        (words.summarize_count.clone(), Pick::Summarize(Summary::Count)),
                        (words.summarize_average.clone(), Pick::Summarize(Summary::Average)),
                        (words.summarize_max.clone(), Pick::Summarize(Summary::Max)),
                        (words.summarize_min.clone(), Pick::Summarize(Summary::Min)),
                        (words.show_as_normal.clone(), Pick::ShowAs(ShowAs::NoCalculation)),
                        (words.show_as_percent_of_grand_total.clone(), Pick::ShowAs(ShowAs::PercentOfGrandTotal)),
                        (words.show_as_running_total.clone(), Pick::ShowAs(ShowAs::RunningTotal)),
                        (words.show_as_rank.clone(), Pick::ShowAs(ShowAs::Rank)),
                    ]
                } else {
                    vec![
                        (words.sort_ascending.clone(), Pick::Sort(FieldSort::Ascending)),
                        (words.sort_descending.clone(), Pick::Sort(FieldSort::Descending)),
                        (words.group_years.clone(), Pick::Group(FieldGroup::Years)),
                        (words.group_quarters.clone(), Pick::Group(FieldGroup::Quarters)),
                        (words.group_months.clone(), Pick::Group(FieldGroup::Months)),
                        (words.ungroup.clone(), Pick::Group(FieldGroup::Ungroup)),
                    ]
                };
                for (label, pick) in settings {
                    item(ui, label, pick, true);
                }
                Divider::horizontal().show(ui);
                item(ui, words.remove.clone(), Pick::Remove, true);
            }
        }
    }

    /// The edit a pick of the menu of `slot` in `area` stands for.
    fn pick_edit(&self, area: FieldArea, slot: &Slot, pick: Pick) -> Option<FieldListEdit> {
        let Slot::Field(field) = slot else {
            return match pick {
                Pick::MoveTo(to) => Some(FieldListEdit::MoveValues { to }),
                _ => None,
            };
        };
        let field = field.clone();
        let position = self.fields_in(area).iter().position(|name| *name == field).unwrap_or_default();
        let step =
            |to: FieldArea, position: usize| FieldListEdit::Move { to, change: Self::set(FieldMove { fields: vec![field.clone()], position }) };
        Some(match pick {
            Pick::MoveTo(to) => step(to, self.fields_in(to).len()),
            Pick::Up => step(area, position.saturating_sub(1)),
            Pick::Down => step(area, position + 1),
            Pick::Summarize(summary) => FieldListEdit::ValueSettings { field, setting: ValueSetting::Summarize(summary) },
            Pick::ShowAs(show) => FieldListEdit::ValueSettings { field, setting: ValueSetting::ShowAs(show) },
            Pick::Sort(order) => FieldListEdit::Sort { field, order },
            Pick::Group(by) => FieldListEdit::Group { field, by },
            Pick::Remove => FieldListEdit::Remove { field, from: Some(area) },
        })
    }

    /// Refresh, Calculated Field… and the Report Layout menu.
    fn footer(&self, ui: &mut Ui, edits: &Edits) {
        let words = self.words;
        if clicked(&Button::new(words.refresh.clone()).show(ui)) {
            edits.borrow_mut().push(FieldListEdit::Refresh);
        }
        if clicked(&Button::new(words.calculated.clone()).show(ui)) {
            edits.borrow_mut().push(FieldListEdit::CalculatedField);
        }
        let button = Button::new(words.layout.clone()).trailing_icon(IconSpec::svg(IconId::new("chevron_down"))).show(ui);
        let layouts = [(&words.compact, ReportLayout::Compact), (&words.outline, ReportLayout::Outline), (&words.tabular, ReportLayout::Tabular)];
        Menu::new(words.layout.clone()).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
            for (label, layout) in layouts {
                if MenuItem::new(label.clone()).show(ui).ran {
                    edits.borrow_mut().push(FieldListEdit::Layout(layout));
                }
            }
        });
    }
}

/// `mv` as fields moving to a place among the fields of the area.
fn field_move(slots: &[Slot], mv: &ReorderMove<Slot>) -> FieldMove {
    let fields: Vec<String> = mv.keys.iter().filter_map(Slot::field).cloned().collect();
    let remaining: Vec<&Slot> = slots.iter().filter(|slot| !mv.keys.contains(slot)).collect();
    let to = mv.to.min(remaining.len());
    let position = remaining[..to].iter().filter(|slot| slot.field().is_some()).count();
    FieldMove { fields, position }
}
