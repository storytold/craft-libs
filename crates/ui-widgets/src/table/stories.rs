//! Stories of `Table`: rows with a text in each cell, a sorted column, a toggle column and a tree.

use super::{Table, sort_rows};
use crate::contract::{CollectionEvent, ColumnEvent, ColumnKind, ColumnSort, ColumnSpec, ShowCollection as _, ToggleColumn};
use crate::list::{ListRow, ToggleCell};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource};
use egui::Ui;

const NAME: ColumnKey = ColumnKey::new("name");
const PAGES: ColumnKey = ColumnKey::new("pages");
const SIZE: ColumnKey = ColumnKey::new("size");
const KEEP: ColumnKey = ColumnKey::new("keep");

/// A file row of the stories.
struct File {
    name: String,
    pages: u32,
    kilobytes: u32,
    keep: bool,
}

fn files(count: usize) -> Vec<File> {
    (0..count)
        .map(|n| File {
            name: format!("Report {}.pdf", n + 1),
            pages: 3 + (n as u32 * 7) % 40,
            kilobytes: 120 + (n as u32 * 53) % 900,
            keep: n % 2 == 0,
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

fn columns(keep: bool, widths: &Widths) -> Vec<ColumnSpec> {
    let mut columns = vec![
        ColumnSpec::new(NAME, "File name", ColumnWidth::fill(2.0)).sortable().resizable().movable(),
        ColumnSpec::new(PAGES, "Pages", width_of(widths, &PAGES, ColumnWidth::Controls(3.0))).numeric().sortable().resizable(),
        ColumnSpec::new(SIZE, "Size (KB)", width_of(widths, &SIZE, ColumnWidth::Controls(4.0))).numeric().sortable().resizable(),
    ];
    if keep {
        columns.insert(0, ColumnSpec::new(KEEP, "Keep", ColumnWidth::Controls(1.5)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())));
    }
    columns
}

fn table(ui: &mut Ui, data: &mut StoryData, mut rows: Vec<File>, sort: Option<ColumnSort>, keep: bool, striped: bool) -> StoryOutput {
    let widths = data.value("widths", Widths::new()).clone();
    sort_rows(&mut rows, sort.as_ref(), |a, b, column| match column {
        c if *c == PAGES => a.pages.cmp(&b.pages),
        c if *c == SIZE => a.kilobytes.cmp(&b.kilobytes),
        _ => a.name.cmp(&b.name),
    });
    let out = Table::<usize>::new("Files", columns(keep, &widths)).striped(striped).sort(sort).empty("No files").show_items(
        ui,
        &Files(&rows),
        &mut |key, icx| {
            let file = &rows[*key];
            let row = ListRow::new(file.name.clone())
                .cells(icx.cells())
                .striped(icx.row.striped)
                .cell(PAGES, file.pages.to_string())
                .cell(SIZE, file.kilobytes.to_string());
            if keep { row.toggle(ToggleCell::check(KEEP, crate::list::ToggleState::of(file.keep), "Keep {row}")) } else { row }
        },
    );
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
    table(ui, data, files(40), Some(ColumnSort::ascending(NAME)), false, true)
}

#[story(Table<'static, usize>, covers = [One], drive = Drive::Never)]
fn table_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, files(1), None, false, false)
}

#[story(Table<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn table_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, Vec::new(), None, false, false)
}

#[story(Table<'static, usize>, covers = [], drive = Drive::Never)]
fn table_sorted_descending_with_toggles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    table(ui, data, files(12), Some(ColumnSort::descending(SIZE)), true, false)
}
