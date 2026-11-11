//! `MetadataTable`: name and value rows in groups over a [`Table`], with a search field above and an optional
//! check column for the app's column picker.

use super::Table;
use super::words::MetadataWords;
use crate::contract::{
    Bind, CollectionEvent, CollectionResponse, ColumnKind, ColumnSpec, NoValue, Outcome, Pad, Pattern, Props, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, ToggleColumn,
};
use crate::layout_components::Stack;
use crate::list::{ListRow, RowEvent, ToggleCell, ToggleState};
use crate::runtime::env::{Env, Label};
use crate::text::TextField;
use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, ItemSource, Mixed};
use craft_i18n::matcher::Matcher;
use craft_theme::ControlSize;
use egui::{Response, Ui};
use std::borrow::Cow;
use std::sync::Arc;

const SHOW: ColumnKey = ColumnKey::new("show");
const NAME: ColumnKey = ColumnKey::new("name");
const VALUE: ColumnKey = ColumnKey::new("value");
/// The value column is this many times as wide as the name column.
const VALUE_SHARE: f32 = 2.0;

/// One field: its name and its value, as the app formats them.
#[derive(Clone, Debug, PartialEq, Eq, Props)]
#[non_exhaustive]
pub struct MetadataRow {
    /// "Exposure Time", "dc:creator".
    pub name: String,
    /// "1/250 s"; may run to several lines, which wrap.
    pub value: String,
    /// What the column picker stores for this field, when that is not the name (the name may be translated or
    /// renamed).
    #[prop(into)]
    pub key: Option<String>,
    /// The column picker shows the field checked and its check can't be changed.
    #[prop(flag)]
    pub locked: bool,
}

impl MetadataRow {
    /// A row showing `name` and `value`, unlocked and with no storage key.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self { name: name.into(), value: value.into(), key: None, locked: false }
    }

    /// What the column picker stores for this field: its key, else its name.
    pub fn picker_key(&self) -> &str {
        self.key.as_deref().unwrap_or(&self.name)
    }

    /// Whether the name or the value matches `matcher`.
    pub fn matches(&self, matcher: &Matcher) -> bool {
        matcher.score(&self.name).is_some() || matcher.score(&self.value).is_some()
    }
}

/// A titled group of fields ("Camera", "EXIF", "XMP"); a group with an empty name has no title row.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetadataGroup {
    /// The group's title, or empty for a group without a title row.
    pub name: String,
    /// The group's fields, in display order.
    pub rows: Vec<MetadataRow>,
}

impl MetadataGroup {
    /// A group named `name` holding `rows`.
    pub fn new(name: impl Into<String>, rows: Vec<MetadataRow>) -> Self {
        Self { name: name.into(), rows }
    }
}

/// Where an entry of the table sits in the caller's groups; it does not change while the filter does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldKey {
    /// The title row of group `group`.
    Group(usize),
    /// Field `field` of group `group`.
    Field(usize, usize),
}

/// A field's check changed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Checked {
    /// The field's [`picker_key`](MetadataRow::picker_key).
    pub key: String,
    /// Whether it is chosen now.
    pub on: bool,
}

/// What the table reported this pass.
#[non_exhaustive]
pub struct MetadataTableOutput {
    /// The table's response, its events and its visible range.
    pub table: CollectionResponse<FieldKey, RowEvent>,
    /// The search text changed.
    pub filter_changed: bool,
    /// The checks the user changed this pass, in order; a bound chosen list already holds them.
    pub checked: Vec<Checked>,
}

impl MetadataTableOutput {}

impl Outcome for MetadataTableOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.table.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.table.story_facts()
    }
}

/// A key and value table in groups: a search field over the rows keeps the fields whose name or value matches the
/// typed text, and a check column marks the keys the app shows as columns. The search field drives the rows: ArrowDown moves
/// the lead to the first row and Enter activates it, while focus stays in the field.
///
/// ```ignore
/// let out = MetadataTable::new("All metadata", &groups).filter(Bind::bound(&mut dialog.search)).show(ui);
/// ```
#[derive(Pattern, Props)]
pub struct MetadataTable<'a> {
    label: Label,
    groups: &'a [MetadataGroup],
    /// The search text; the search field shows above the rows while it is set.
    #[prop]
    filter: Option<Bind<'a, String>>,
    /// The keys of the chosen fields; a check column shows before each field while it is set. Checking appends the
    /// key and unchecking removes it. A locked field shows checked whatever the list holds.
    #[prop]
    choose: Option<Bind<'a, Vec<String>>>,
}

/// One entry of the table: a title row or a field.
enum Entry<'a> {
    Group(&'a MetadataGroup),
    Field(&'a MetadataRow),
}

/// The visible entries as the table's source.
struct Fields<'a> {
    keys: Vec<FieldKey>,
    entries: Vec<Entry<'a>>,
    chosen: Option<&'a [String]>,
}

impl DisplayOrder<FieldKey> for Fields<'_> {
    fn is_section(&self, at: usize) -> bool {
        matches!(self.entries[at], Entry::Group(_))
    }

    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> FieldKey {
        self.keys[at]
    }

    fn index_of(&self, key: &FieldKey) -> Option<usize> {
        self.keys.iter().position(|candidate| candidate == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(match &self.entries[at] {
            Entry::Group(group) => group.name.as_str(),
            Entry::Field(row) => row.name.as_str(),
        }))
    }
}

impl ItemSource<FieldKey> for Fields<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        match (&self.entries[at], self.chosen) {
            (Entry::Field(row), Some(chosen)) if *column == SHOW => row.locked || chosen.iter().any(|key| key == row.picker_key()),
            _ => false,
        }
    }

    fn toggle_enabled(&self, at: usize, _: &ColumnKey) -> bool {
        matches!(&self.entries[at], Entry::Field(row) if !row.locked)
    }
}

impl<'a> MetadataTable<'a> {
    /// A table of `groups` named `label`, with no search field and no check column.
    pub fn new(label: impl Into<Label>, groups: &'a [MetadataGroup]) -> Self {
        MetadataTable { label: label.into(), groups, filter: None, choose: None }
    }

    /// Shows the table and returns what the user did this pass.
    pub fn show(self, ui: &mut Ui) -> MetadataTableOutput {
        let MetadataTable { label, groups, filter, mut choose } = self;
        let env = Env::of(ui);
        let words = env.words::<MetadataWords>();
        let mut query = String::new();
        let mut filter_changed = false;
        let rows = ui.scope_id().with("rows");
        if let Some(text) = filter {
            // The field starts where the table's text does: one cell inset (`space.pad.control`) from the edge.
            let mut text = text;
            let edit = Stack::new()
                .pad(Pad::Control)
                .show(ui, |ui, _| TextField::search(words.filter.clone(), text.reborrowed()).placeholder(words.filter.clone()).drives(rows).show(ui));
            if let Some(edit) = edit.inner {
                filter_changed = !edit.events.is_empty();
                if let Some(Mixed::Same(shown)) = &edit.shown {
                    query.clone_from(&**shown);
                }
            }
        }
        let query = query.trim();
        let matcher = (!query.is_empty()).then(|| Matcher::new(&env.folder(), query));

        let mut keys = Vec::new();
        let mut entries = Vec::new();
        for (at, group) in groups.iter().enumerate() {
            let kept: Vec<usize> =
                (0..group.rows.len()).filter(|&row| matcher.as_ref().is_none_or(|matcher| group.rows[row].matches(matcher))).collect();
            if kept.is_empty() {
                continue;
            }
            if !group.name.is_empty() {
                keys.push(FieldKey::Group(at));
                entries.push(Entry::Group(group));
            }
            for row in kept {
                keys.push(FieldKey::Field(at, row));
                entries.push(Entry::Field(&group.rows[row]));
            }
        }
        let empty = if groups.iter().all(|group| group.rows.is_empty()) { words.none.clone() } else { words.no_match.clone() };

        let mut columns = vec![
            ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0)),
            ColumnSpec::new(VALUE, words.value.clone(), ColumnWidth::fill(VALUE_SHARE)),
        ];
        if choose.is_some() {
            columns
                .insert(0, ColumnSpec::new(SHOW, words.show.clone(), ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())));
        }
        let held: Option<Vec<String>> = choose.as_mut().map(|bind| match bind.reborrow().get() {
            Some(Mixed::Same(keys)) => keys,
            _ => Vec::new(),
        });
        let source = Fields { keys, entries, chosen: held.as_deref() };
        let checks = choose.is_some();
        let table = Table::<FieldKey>::new(label.clone(), columns)
            .id(rows)
            .select(SelectMode::Single)
            .size(ControlSize::Sm)
            .striped(true)
            .header(false)
            .rows_stamp(query_stamp(query))
            .empty(empty)
            .show_items(ui, &source, &mut |_, icx| match &source.entries[icx.at] {
                Entry::Group(group) => ListRow::new(group.name.clone()).cells(icx.cells()).heading().space_above(icx.at > 0),
                Entry::Field(row) => {
                    let line = ListRow::new(row.name.clone())
                        .cells(icx.cells())
                        .striped(icx.row.striped)
                        .cell_muted(NAME)
                        .cell_lines(VALUE, row.value.clone());
                    if checks {
                        line.toggle(ToggleCell::check(SHOW, ToggleState::of(source.toggled(icx.at, &SHOW)), words.show_field.clone()))
                    } else {
                        line
                    }
                }
            });
        drop(source);

        let mut checked = Vec::new();
        if let (Some(mut choose), Some(before)) = (choose, held) {
            let mut chosen = before.clone();
            for event in &table.events {
                let CollectionEvent::Toggle(edit) = event else { continue };
                let Some(toggle) = edit.value() else { continue };
                let FieldKey::Field(group, row) = toggle.key else { continue };
                let row = &groups[group].rows[row];
                let key = row.picker_key();
                let at = chosen.iter().position(|candidate| candidate == key);
                if row.locked || at.is_some() == toggle.on {
                    continue;
                }
                match at {
                    Some(at) => {
                        chosen.remove(at);
                    }
                    None => chosen.push(key.to_owned()),
                }
                checked.push(Checked { key: key.to_owned(), on: toggle.on });
            }
            if chosen != before {
                let name: Arc<str> = Arc::from(label.as_str());
                choose.reborrow().apply(&EditEvent::set(Mixed::Same(before), chosen, name, None));
            }
        }
        MetadataTableOutput { table, filter_changed, checked }
    }
}

#[cfg(feature = "testing")]
mod stories;

/// The rows stamp of a filter query: the same query keeps the layout, another one rebuilds it.
fn query_stamp(query: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    query.hash(&mut hasher);
    hasher.finish()
}
