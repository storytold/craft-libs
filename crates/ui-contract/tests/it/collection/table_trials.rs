//! The header row and the toggle column: a header drag resizes or moves a column, and a keyed slice reports toggles by key.

use super::kit::{Count, Dismissed, Row};
use super::table::Table;
use craft_behavior::{ColumnKey, ColumnWidth, ItemSource, keyed};
use craft_ui::contract::{
    CollectionEvent, CollectionProps, CollectionResponse, ColumnEvent, ColumnKind, ColumnSpec, Props as _, ShowCollection as _, ToggleColumn,
};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, input, pointer, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(400.0, 240.0);
const COL: f32 = 100.0;
const NAME: ColumnKey = ColumnKey::new("name");
const SIZE: ColumnKey = ColumnKey::new("size");
const DATE: ColumnKey = ColumnKey::new("date");
const MUTE: ColumnKey = ColumnKey::new("mute");

/// What a collection reported and where its rows were.
#[derive(Default)]
struct Scene {
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
}

impl Scene {
    fn take(&self, out: CollectionResponse<usize, Dismissed>) {
        self.events.borrow_mut().extend(out.events);
    }

    fn note(&self, key: usize, rect: Rect) {
        let mut rows = self.rows.borrow_mut();
        rows.retain(|(held, _)| *held != key);
        rows.push((key, rect));
    }

    fn row(&self, key: usize) -> Rect {
        self.rows.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    /// The centre of header cell `at`, `width` wide columns from the left edge.
    fn header_cell(&self, at: usize, widths: &[f32]) -> Pos2 {
        let top = self.row(0).min.y;
        let before: f32 = widths[..at].iter().sum();
        Pos2::new(ORIGIN.x + before + widths[at] / 2.0, (ORIGIN.y + top) / 2.0)
    }

    fn columns(&self) -> Vec<ColumnEvent> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| if let CollectionEvent::Column(column) = event { Some(column.clone()) } else { None })
            .collect()
    }

    fn sorts(&self) -> Vec<(ColumnKey, bool)> {
        self.columns()
            .into_iter()
            .filter_map(|event| if let ColumnEvent::Sort { key, descending, .. } = event { Some((key, descending)) } else { None })
            .collect()
    }

    fn moves(&self) -> Vec<(ColumnKey, usize)> {
        self.columns().into_iter().filter_map(|event| if let ColumnEvent::Moved { key, to, .. } = event { Some((key, to)) } else { None }).collect()
    }

    fn widths(&self) -> Vec<f32> {
        self.columns().into_iter().filter_map(|event| if let ColumnEvent::Resized { width, .. } = event { Some(width) } else { None }).collect()
    }

    fn toggles(&self) -> Vec<(usize, bool)> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| {
                if let CollectionEvent::Toggle(edit) = event {
                    edit.value()
                        .filter(|_| matches!(edit, craft_behavior::EditEvent::Set { .. } | craft_behavior::EditEvent::Commit { .. }))
                        .map(|toggle| (toggle.key, toggle.on))
                } else {
                    None
                }
            })
            .collect()
    }
}

fn show_table(ui: &mut Ui, props: CollectionProps, source: &dyn ItemSource<usize>, scene: &Scene) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
        let out = Table::new(props).id_salt("table").show_items(ui, source, &mut |key, cx| {
            scene.note(*key, cx.rect);
            Row::new()
        });
        scene.take(out);
    });
}

fn name_column(width: f32) -> ColumnSpec {
    ColumnSpec::new(NAME, "Name", ColumnWidth::Fixed(width)).sortable().resizable().movable()
}

fn file_columns(width: f32) -> CollectionProps {
    CollectionProps::new().columns(vec![
        name_column(width),
        ColumnSpec::new(SIZE, "Size", ColumnWidth::Fixed(COL)).sortable(),
        ColumnSpec::new(DATE, "Date", ColumnWidth::Fixed(COL)).movable(),
    ])
}

/// Dragging a header's edge resizes a column the app owns the width of; dragging the header moves it.
#[trial]
fn a_header_drag_resizes_and_moves_a_column() {
    let (scene, source) = (Scene::default(), Count(4));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let width = scene.widths().last().copied().unwrap_or(COL);
        show_table(ui, file_columns(width), &source, &scene);
    };
    frames.idle(3, &mut show);
    let edge = scene.header_cell(0, &[COL]) + Vec2::new(COL / 2.0 - 1.0, 0.0);
    pointer::drag(&mut frames, edge, Vec2::new(30.0, 0.0), &mut show);
    let width = scene.widths().last().copied().expect("the edge drag reports the new width");
    assert!(width > COL, "the column grew: {width}");
    assert!(scene.moves().is_empty() && scene.sorts().is_empty(), "a resize moves nothing and sorts nothing");

    let widths = [width, COL, COL];
    let from = scene.header_cell(0, &widths);
    let to_second = Vec2::new(width / 2.0 + COL / 2.0, 0.0);
    pointer::drag(&mut frames, from, to_second, &mut show);
    assert_eq!(scene.moves(), vec![(NAME, 1)]);
    assert!(scene.sorts().is_empty(), "a drag is not a click");
}

/// A slice shown with `keyed(..).with_toggled(..)` after its first item was removed: the toggle cells show the rows'
/// own state, and presses and sweeps report the rows' keys, not their positions.
#[trial]
fn a_keyed_slice_reports_toggle_presses_and_sweeps_by_key() {
    let mut rows: Vec<(usize, bool)> = vec![(10, false), (11, false), (12, false), (13, true)];
    rows.remove(0);
    let props = || {
        CollectionProps::new().columns(vec![
            ColumnSpec::new(NAME, "Name", ColumnWidth::Fixed(COL)),
            ColumnSpec::new(MUTE, "Mute", ColumnWidth::Fixed(COL)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
        ])
    };
    for press in [false, true] {
        let scene = Scene::default();
        let mut frames = Frames::new();
        let mut show = |ui: &mut Ui| {
            let source = keyed(&rows, |row| row.0).with_toggled(|row, column| *column == MUTE && row.1);
            show_table(ui, props(), &source, &scene);
        };
        frames.idle(3, &mut show);
        let cell = |key: usize| Pos2::new(ORIGIN.x + COL * 1.5, scene.row(key).center().y);
        if press {
            pointer::click(&mut frames, cell(13), &mut show);
            assert_eq!(scene.toggles(), vec![(13, false)], "the cell shows the row's own state, so a press turns it off");
        } else {
            pointer::drag(&mut frames, cell(11), cell(12) - cell(11), &mut show);
            assert_eq!(scene.toggles(), vec![(11, true), (12, true)], "the sweep reports the rows' keys");
        }
    }
}

/// A toggle column's key flips that column on every selected row as one gesture, while Space keeps flipping the first
/// toggle column.
#[trial]
fn a_toggle_column_key_flips_the_column_on_the_selected_rows() {
    const SOLO: ColumnKey = ColumnKey::new("solo");
    let rows: Vec<(usize, bool)> = vec![(10, false), (11, false), (12, false)];
    let props = || {
        CollectionProps::new().select(craft_ui::contract::SelectMode::Multi).space(craft_ui::contract::SpaceAction::ToggleCell).columns(vec![
            ColumnSpec::new(NAME, "Name", ColumnWidth::Fixed(COL)),
            ColumnSpec::new(SOLO, "Solo", ColumnWidth::Fixed(COL)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
            ColumnSpec::new(MUTE, "Mute", ColumnWidth::Fixed(COL)).kind(ColumnKind::Toggle(
                ToggleColumn::sweepable().key(craft_keymap::Chord::new(craft_keymap::Mods::SHIFT, craft_keymap::Key::Space)),
            )),
        ])
    };
    let scene = Scene::default();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let source = keyed(&rows, |row| row.0).with_toggled(|row, column| *column == MUTE && row.1);
        show_table(ui, props(), &source, &scene);
    };
    frames.idle(3, &mut show);
    let name = |key: usize| Pos2::new(ORIGIN.x + COL / 2.0, scene.row(key).center().y);
    pointer::click(&mut frames, name(11), &mut show);
    let mut press = |key: Key, mods: Modifiers| {
        frames.frame(vec![input::key_press(key, mods)], &mut show);
        frames.frame(vec![craft_ui::testing::focus::key_release(key, mods)], &mut show);
    };
    press(Key::ArrowDown, Modifiers::SHIFT);
    scene.events.borrow_mut().clear();
    press(Key::Space, Modifiers::SHIFT);
    let columns = |scene: &Scene| -> Vec<(usize, ColumnKey)> {
        scene
            .events
            .borrow()
            .iter()
            .filter_map(
                |event| if let CollectionEvent::Toggle(edit) = event { edit.value().map(|toggle| (toggle.key, toggle.column.clone())) } else { None },
            )
            .collect()
    };
    let mut flipped = columns(&scene);
    flipped.sort_unstable_by_key(|(key, _)| *key);
    flipped.dedup();
    assert_eq!(flipped, vec![(11, MUTE), (12, MUTE)], "Shift+Space flips Mute on both selected rows");
    scene.events.borrow_mut().clear();
    press(Key::Space, Modifiers::NONE);
    assert!(columns(&scene).iter().all(|(_, column)| *column == SOLO) && !columns(&scene).is_empty(), "Space flips the first toggle column");
}
