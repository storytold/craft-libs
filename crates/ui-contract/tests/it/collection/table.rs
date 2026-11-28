//! Collections with columns and reordering, and the trials of the header's sideways reveal and its bound layout.

use super::kit::{Count, Dismissed, ROW_H, Row};
use craft_behavior::{CanvasTransform, ColumnKey, ColumnWidth, ItemSource, Selection, SpanLayout};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, ColumnEvent, ColumnLayout, ColumnSpec, Common, Component,
    ContainerProps, Cx, Props as _, Role, ShowCollection as _,
};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, input, pointer, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

/// A list of rows at `ROW_H` with the collection props the trial gives, and the column layout it binds.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
pub struct Table<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    column_layout: Option<Bind<'a, ColumnLayout>>,
}

impl<'a> Table<'a> {
    pub fn new(items: CollectionProps) -> Self {
        Table { common: Common::new(), props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill), items, column_layout: None }
    }

    /// The column arrangement the trial owns.
    pub fn column_layout(mut self, layout: Bind<'a, ColumnLayout>) -> Self {
        self.column_layout = Some(layout);
        self
    }

    pub fn transformed(mut self, transform: CanvasTransform) -> Self {
        self.props = self.props.transform(transform);
        self
    }
}

impl Component for Table<'_> {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: TableStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Table<'_> {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = Row;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn column_layout(&mut self) -> Option<BindMut<'_, ColumnLayout>> {
        self.column_layout.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, ROW_H)
    }
}

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(400.0, 240.0);
const COL: f32 = 100.0;
/// Places that differ by less are the same place: the rounding of the layout.
const SAME: f32 = 0.5;

/// Where each row was last shown.
#[derive(Default)]
struct Rows(RefCell<Vec<(usize, Rect)>>);

impl Rows {
    fn note(&self, key: usize, rect: Rect) {
        let mut rows = self.0.borrow_mut();
        rows.retain(|(held, _)| *held != key);
        rows.push((key, rect));
    }

    fn row(&self, key: usize) -> Rect {
        self.0.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    /// The height of the middle of the header row, above the first row.
    fn header_y(&self) -> f32 {
        (ORIGIN.y + self.row(0).min.y) / 2.0
    }
}

fn sorts(events: &[CollectionEvent<usize, Dismissed>]) -> Vec<ColumnKey> {
    events
        .iter()
        .filter_map(|event| if let CollectionEvent::Column(ColumnEvent::Sort { key, .. }) = event { Some(key.clone()) } else { None })
        .collect()
}

fn press(frames: &mut Frames, key: Key, mods: Modifiers, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![input::key_press(key, mods)], &mut *show);
    frames.frame(vec![craft_ui::testing::focus::key_release(key, mods)], &mut *show);
}

/// Six columns wider together than the table: focus on the first header cell and Forward until the last holds focus
/// scrolls the header sideways to show the last cell whole, and the rows by the same offset, so a press where the
/// rows' offset puts the last cell lands on it. Guards keyboard reach to header cells out of view.
#[trial]
fn focus_on_an_out_of_view_header_cell_scrolls_it_in() {
    const NAMES: [&str; 6] = ["a", "b", "c", "d", "e", "f"];
    let last = ColumnKey::new(NAMES[5]);
    let columns = || {
        let specs = NAMES
            .iter()
            .map(|name| {
                let spec = ColumnSpec::new(ColumnKey::new(name), *name, ColumnWidth::Fixed(COL));
                if *name == NAMES[5] { spec.sortable() } else { spec }
            })
            .collect();
        CollectionProps::new().columns(specs)
    };
    let (rows, source) = (Rows::default(), Count(4));
    let events = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let out = Table::new(columns()).id_salt("table").show_items(ui, &source, &mut |key, cx| {
                rows.note(*key, cx.rect);
                Row::new()
            });
            events.borrow_mut().extend(out.events);
        });
    };
    frames.idle(3, &mut show);
    let header_y = rows.header_y();
    // Tab enters the rows; Shift+Tab goes back to the header row, on its first column (a press on a header cell takes
    // no focus).
    press(&mut frames, Key::Tab, Modifiers::NONE, &mut show);
    press(&mut frames, Key::Tab, Modifiers::SHIFT, &mut show);
    for _ in 1..NAMES.len() {
        press(&mut frames, Key::ArrowRight, Modifiers::NONE, &mut show);
    }
    frames.idle(2, &mut show);
    let scrolled = ORIGIN.x - rows.row(0).min.x;
    let cell = (ORIGIN.x + COL * 5.0 - scrolled)..=(ORIGIN.x + COL * 6.0 - scrolled);
    assert!(
        *cell.start() >= ORIGIN.x - SAME && *cell.end() <= ORIGIN.x + VIEW.x + SAME,
        "the last header cell lies whole in the header's view: {cell:?}, rows scrolled by {scrolled}"
    );
    events.borrow_mut().clear();
    pointer::click(&mut frames, Pos2::new((cell.start() + cell.end()) / 2.0, header_y), &mut show);
    assert_eq!(sorts(&events.borrow()), vec![last], "the header cell sits where the rows' sideways offset puts it");
}

/// A table bound to a layout that moves a column and hides another shows the bound order, and a resize by a header
/// edge drag writes the width into the binding in the pass that reports it. Guards restoring a saved layout.
#[trial]
fn a_bound_column_layout_is_restored_and_receives_changes() {
    const NAME: ColumnKey = ColumnKey::new("name");
    const SIZE: ColumnKey = ColumnKey::new("size");
    const DATE: ColumnKey = ColumnKey::new("date");
    let columns = || {
        CollectionProps::new().columns(vec![
            ColumnSpec::new(NAME, "Name", ColumnWidth::fill(1.0)).sortable().resizable().movable(),
            ColumnSpec::new(SIZE, "Size", ColumnWidth::Fixed(COL)).sortable().movable(),
            ColumnSpec::new(DATE, "Date", ColumnWidth::Fixed(COL)).sortable().hideable(),
        ])
    };
    let saved = RefCell::new(ColumnLayout { order: vec![SIZE, NAME, DATE], widths: Vec::new(), hidden: vec![DATE] });
    let (rows, source) = (Rows::default(), Count(4));
    let events = RefCell::new(Vec::new());
    // Widths a pass reported whose binding did not hold them at the end of that pass.
    let unsaved = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = saved.borrow().clone();
        let mut got = Vec::new();
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            got = Table::new(columns())
                .column_layout(Bind::bound(&mut held))
                .id_salt("table")
                .show_items(ui, &source, &mut |key, cx| {
                    rows.note(*key, cx.rect);
                    Row::new()
                })
                .events;
        });
        for event in &got {
            if let CollectionEvent::Column(ColumnEvent::Resized { key, width, .. }) = event
                && !held.widths.contains(&(key.clone(), *width))
            {
                unsaved.borrow_mut().push(*width);
            }
        }
        *saved.borrow_mut() = held;
        events.borrow_mut().extend(got);
    };
    frames.idle(2, &mut show);
    let header_y = rows.header_y();
    // Size first at its fixed width, Name filling the rest, Date hidden.
    pointer::click(&mut frames, Pos2::new(ORIGIN.x + COL / 2.0, header_y), &mut show);
    pointer::click(&mut frames, Pos2::new(ORIGIN.x + (COL + VIEW.x) / 2.0, header_y), &mut show);
    assert_eq!(sorts(&events.borrow()), vec![SIZE, NAME], "the header shows the bound order without the hidden column");

    events.borrow_mut().clear();
    let edge = Pos2::new(ORIGIN.x + VIEW.x - 1.0, header_y);
    pointer::drag(&mut frames, edge, Vec2::new(-30.0, 0.0), &mut show);
    let widths: Vec<f32> = events
        .borrow()
        .iter()
        .filter_map(|event| {
            if let CollectionEvent::Column(ColumnEvent::Resized { key, width, .. }) = event { (*key == NAME).then_some(*width) } else { None }
        })
        .collect();
    let width = widths.last().copied().expect("the edge drag resizes the Name column");
    assert!(width < VIEW.x - COL, "the column narrowed: {width}");
    assert!(unsaved.borrow().is_empty(), "every reported width was in the binding in its own pass: {:?}", unsaved.borrow());
    let layout = saved.borrow().clone();
    assert_eq!(layout.widths, vec![(NAME, width)], "the binding holds the new width");
    assert_eq!((layout.order, layout.hidden), (vec![SIZE, NAME, DATE], vec![DATE]), "the resize keeps the bound order and hidden column");
}
