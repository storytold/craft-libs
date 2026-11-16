//! Stories of `Table`: rows with a text in each cell, a sorted column, a toggle column and a tree.

use super::{Table, sort_rows};
use crate::contract::{Bind, CollectionEvent, ColumnEvent, ColumnKind, ColumnSort, ColumnSpec, ShowCollection as _, ToggleColumn};
use crate::list::{ListRow, ToggleCell};
use crate::testing::pointer::{moved, secondary};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft_i18n::{AgeStyle, FileSize, SizeUnit};
use egui::Ui;

const NAME: ColumnKey = ColumnKey::new("name");
const PAGES: ColumnKey = ColumnKey::new("pages");
const SIZE: ColumnKey = ColumnKey::new("size");
const MODIFIED: ColumnKey = ColumnKey::new("modified");
const WARNINGS: ColumnKey = ColumnKey::new("warnings");
const KEEP: ColumnKey = ColumnKey::new("keep");

/// Sizes in megabytes with one decimal, as a file list shows them.
const MEGABYTES: FileSize = FileSize::BINARY.smallest(SizeUnit::Mega);

/// A file row of the stories.
struct File {
    name: String,
    pages: u64,
    kilobytes: u64,
    /// Seconds since the file changed.
    age: u64,
    warnings: u64,
    keep: bool,
}

fn files(count: usize) -> Vec<File> {
    (0..count)
        .map(|n| {
            let at = n as u64;
            File {
                name: format!("Report {:03}.pdf", n + 1),
                pages: (at * 7) % 90 + 1,
                kilobytes: (at * 937) % 48_000 + 120,
                age: (at * 7_919) % (86_400 * 400),
                warnings: if n.is_multiple_of(7) { at % 3 + 1 } else { 0 },
                keep: n % 2 == 0,
            }
        })
        .collect()
}

/// The files as a collection source, keyed by position.
struct Files<'a>(&'a [File]);

impl DisplayOrder<usize> for Files<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|file| std::borrow::Cow::Borrowed(file.name.as_str()))
    }
}

impl ItemSource<usize> for Files<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == KEEP && self.0[at].keep
    }
}

/// The widths the user dragged the columns to, which the story applies to the next pass.
type Widths = Vec<(ColumnKey, f32)>;

fn width_of(widths: &Widths, key: &ColumnKey, default: ColumnWidth) -> ColumnWidth {
    widths.iter().find(|(held, _)| held == key).map_or(default, |(_, width)| ColumnWidth::Fixed(*width))
}

/// The most of the table the name column takes: a file list keeps room for the columns beside the names.
const NAME_SHARE: f32 = 1.0 / 3.0;

fn columns(keep: bool, widths: &Widths) -> Vec<ColumnSpec> {
    let mut columns = vec![
        ColumnSpec::new(NAME, "Name", width_of(widths, &NAME, ColumnWidth::Content)).at_most(NAME_SHARE).sortable().resizable().movable(),
        ColumnSpec::new(PAGES, "Pages", width_of(widths, &PAGES, ColumnWidth::Controls(2.0))).numeric().sortable().resizable(),
        ColumnSpec::new(SIZE, "Size", width_of(widths, &SIZE, ColumnWidth::Controls(3.0))).numeric().sortable().resizable().hideable(),
        ColumnSpec::new(MODIFIED, "Modified", width_of(widths, &MODIFIED, ColumnWidth::Content)).sortable().resizable().hideable(),
        ColumnSpec::new(WARNINGS, "Warnings", width_of(widths, &WARNINGS, ColumnWidth::Controls(3.0))).numeric().sortable().resizable(),
    ];
    if keep {
        columns.insert(0, ColumnSpec::new(KEEP, "Keep", ColumnWidth::Controls(1.5)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())));
    }
    columns
}

/// The table of `rows` sorted by `sort`, `selected` the rows picked to begin with.
fn table(
    ui: &mut Ui,
    data: &mut StoryData,
    mut rows: Vec<File>,
    sort: Option<ColumnSort>,
    keep: bool,
    striped: bool,
    selected: Selection<usize>,
) -> StoryOutput {
    let widths = data.value("widths", Widths::new()).clone();
    sort_rows(&mut rows, sort.as_ref(), |a, b, column| match column {
        c if *c == PAGES => a.pages.cmp(&b.pages),
        c if *c == SIZE => a.kilobytes.cmp(&b.kilobytes),
        c if *c == MODIFIED => b.age.cmp(&a.age),
        c if *c == WARNINGS => a.warnings.cmp(&b.warnings),
        _ => a.name.cmp(&b.name),
    });
    let out = Table::<usize>::new("Files", columns(keep, &widths))
        .striped(striped)
        .sort(sort)
        .empty("No files")
        .selection(Bind::bound(data.value("selection", selected)))
        .show_items(ui, &Files(&rows), &mut |key, icx| {
            let file = &rows[*key];
            let (units, format) = (icx.env.units(), icx.env.format());
            let mut row = ListRow::new(file.name.clone())
                .cells(icx.cells())
                .striped(icx.row.striped)
                .cell(PAGES, format.integer(file.pages))
                .cell(SIZE, units.file_size_as(file.kilobytes * 1024, MEGABYTES))
                .cell(MODIFIED, icx.env.relative_age(file.age, AgeStyle::Short));
            if file.warnings > 0 {
                row = row.cell(WARNINGS, format.integer(file.warnings));
            }
            if keep { row.toggle(ToggleCell::check(KEEP, crate::list::ToggleState::of(file.keep), "Keep {row}")) } else { row }
        });
    let widths = data.value("widths", Widths::new());
    for event in &out.events {
        let CollectionEvent::Column(ColumnEvent::Resized { key, width }) = event else { continue };
        match widths.iter_mut().find(|(held, _)| held == key) {
            Some((_, held)) => *held = *width,
            None => widths.push((key.clone(), *width)),
        }
    }
    out.into()
}

#[story(Table<'static, usize>, covers = [Default, Many], drive = Drive::Never)]
fn table_many(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, files(40), Some(ColumnSort::ascending(NAME)), false, true, Selection::single(0))
}

/// A secondary click on the Name header, which opens the header menu.
fn open_header_menu(fixture: &mut Fixture) {
    let Some(header) = fixture.by_label("Name") else { return };
    let at = header.center();
    fixture.frame(vec![moved(at), secondary(at, true)]);
    fixture.frame(vec![secondary(at, false)]).settle();
}

#[story(Table<'static, usize>, covers = [Open], drive = Drive::Custom(open_header_menu))]
fn table_header_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, files(12), Some(ColumnSort::ascending(NAME)), true, false, Selection::default())
}

#[story(Table<'static, usize>, covers = [One], drive = Drive::Never)]
fn table_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, files(1), None, false, false, Selection::default())
}

#[story(Table<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn table_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, Vec::new(), None, false, false, Selection::default())
}

#[story(Table<'static, usize>, covers = [], drive = Drive::Never)]
fn table_sorted_descending_with_toggles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, files(12), Some(ColumnSort::descending(SIZE)), true, false, Selection::default())
}
