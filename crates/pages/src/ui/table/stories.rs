//! Stories of the document table widgets.

use craft_behavior::{CellAt, CellRange, CellSelection, Mixed};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, Validity};
use craft_ui::runtime::env::{Message, UnitKind};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

use super::{
    BorderEdges, CellAlign, CellSelectionOverlay, EdgeKey, EdgePicker, RowHeight, TableGeometry, TableProperties, TableSizePicker, TableValues,
    TextDirection,
};

fn mixed_edges() -> BorderEdges {
    let mut edges = BorderEdges::default();
    for key in [EdgeKey::Top, EdgeKey::Left, EdgeKey::InsideHorizontal] {
        edges.set(key, true);
    }
    edges
}

fn click_top_border(fixture: &mut Fixture) {
    if let Some(top) = fixture.by_label("Top Border") {
        fixture.click(&top);
    }
}

#[story(EdgePicker<'static>, covers = [Default], drive = Drive::Custom(click_top_border))]
fn edge_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    EdgePicker::new("Borders", Bind::bound(data.value("edges", mixed_edges()))).show(ui).into()
}

#[story(EdgePicker<'static>, covers = [Mixed], drive = Drive::Never)]
fn edge_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    EdgePicker::new("Borders", Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).show(ui).into()
}

#[story(EdgePicker<'static>, covers = [Invalid], drive = Drive::Never)]
fn edge_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_none =
        |edges: &BorderEdges| if *edges == BorderEdges::default() { Validity::Refuse(Message::new("Choose a line")) } else { Validity::Ok };
    EdgePicker::new("Borders", Bind::bound(data.value("none", BorderEdges::default())).check(refuse_none)).show(ui).into()
}

#[story(EdgePicker<'static>, covers = [], drive = Drive::Never)]
fn edge_picker_paragraph(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    EdgePicker::new("Paragraph borders", Bind::bound(data.value("paragraph", mixed_edges()))).paragraph().show(ui).into()
}

#[story(TableSizePicker, covers = [Default], drive = Drive::Never)]
fn table_size_picker(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TableSizePicker::new("Insert Table").hovered((3, 4)).show(ui).into()
}

fn values() -> TableValues {
    TableValues {
        rows: 4,
        columns: 3,
        row_height_mode: Some(RowHeight::AtLeast),
        row_height: Some(18.0),
        column_width: None,
        cell_align: Some(CellAlign::Top),
        insets: [Some(4.0), Some(4.0), None, Some(4.0)],
        direction: TextDirection::LeftToRight,
        alternating_fills: true,
        repeat_header: false,
        header_rows: 1,
        footer_rows: 0,
        several_cells: true,
        merged: false,
    }
}

#[story(TableProperties<'static>, covers = [], drive = Drive::Never)]
fn table_properties(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = values();
    TableProperties::new(&values, UnitKind::Points).show(ui).into()
}

/// A 3 by 3 table with its top row and the first two cells of its middle row selected.
#[story(CellSelectionOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn cell_selection_overlay(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let origin = ui.available_rect_before_wrap().min;
    let geometry = TableGeometry::new((0..4).map(|i| origin.y + 24.0 * i as f32).collect(), (0..4).map(|i| origin.x + 60.0 * i as f32).collect());
    let ranges = vec![CellRange::cells(CellAt::new(0, 0), CellAt::new(0, 2)), CellRange::cells(CellAt::new(1, 0), CellAt::new(1, 1))];
    let selection = CellSelection::from_ranges(ranges, CellAt::new(1, 1)).unwrap_or_default();
    let mut overlay = Some(CellSelectionOverlay::new(&geometry, &selection));
    let stage = craft_ui::testing::stage(ui);
    let shown = Canvas::new("Table").show_at(ui, stage, |ui, body| overlay.take().map(|overlay| overlay.show_at(ui, body.rect)));
    shown.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}
