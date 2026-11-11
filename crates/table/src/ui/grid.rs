//! The spreadsheet cell grid: addresses in A1 notation, the sheet's geometry and the grid component.
//!
//! The grid is a collection on `SheetLayout`: one item per visible row ([`GridRow`]), cells as parts of the row, the
//! cell selection and the cell rules of `craft_behavior`, and the in-cell editor on the editor contract. The column
//! header strip, the row headers, the corner, the header edges, the fill handle and the range border are the grid's own
//! parts and paint.

pub use craft_behavior::{CellAdvance, CellAt, CellDirection, CellRange, CellSelection, CellSpace, LineIndex, Positions, SHEET_ZOOM};
use craft_behavior::{DisplayOrder, ItemSource, Metrics, SheetLayout};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, ContainerProps, ContextMenuRequest, Cx, Outcome,
    PaintUi, Props as _, Remember as _, Role, SelectMode, Sizing,
};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{EndKey, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, Stateful, StyleColor, StyleCx, SurfaceLook};
use egui::{Rect, Response, Vec2};

use self::geometry::ROW_HEADER_LOOKAHEAD;
use super::filter::FilterButtonEvent;
use super::outline::{OutlineAction, OutlineAxis};

mod editor;
mod geometry;
mod input;
mod model;
mod paint;
mod reference;
mod row;
#[cfg(feature = "screen-testing")]
mod stories;

pub use geometry::{GridLayout, HEADER_MIN_ZOOM, SheetGeometry, SheetMetrics};
pub use model::{CellEdit, CellProblem, ColumnFilter, GridEdit, GridMenuTarget, GridModel, GridState};
pub use reference::{SHEET_COLUMNS, SHEET_ROWS, absolute_range_reference, absolute_reference, column_letters};
pub use row::GridRow;

pub(crate) use geometry::{digits_of, header_insets};
pub(crate) use reference::edges;

/// The rows of a sheet as the collection's source: one key per row, equal to its position.
struct SheetRows(Positions);

impl DisplayOrder<u32> for SheetRows {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> u32 {
        self.0.key(at)
    }

    fn index_of(&self, key: &u32) -> Option<usize> {
        self.0.index_of(key)
    }
}

impl ItemSource<u32> for SheetRows {}

/// What the grid returns.
#[non_exhaustive]
pub struct GridOutput {
    /// The collection's response and events.
    pub collection: CollectionResponse<u32, ()>,
    /// This pass's geometry, for the app's objects and overlays.
    pub geometry: SheetGeometry,
    /// What the grid asks of the app this pass, in order.
    pub edits: Vec<GridEdit>,
    /// The draft committed this pass, for the app to write into the cell.
    pub committed: Option<CellEdit>,
    /// The draft of the open cell, for the app's formula bar; `None` while no cell is open.
    pub draft: Option<String>,
    /// Whether Cmd/Ctrl was held at a fill commit this pass (copy and series swap).
    pub fill_swapped: bool,
    /// The filter button pressed this pass: its column and event.
    pub filter: Option<(u32, FilterButtonEvent)>,
    /// The fix chosen from the error tag's menu, by index.
    pub fix: Option<usize>,
    /// The outline buttons pressed this pass.
    pub outline: Vec<OutlineAction>,
}

impl Outcome for GridOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.collection.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.collection.story_facts()
    }
}

/// What one pass of the grid collected for its output.
#[derive(Default)]
pub(crate) struct PassOutput {
    pub(crate) edits: Vec<GridEdit>,
    pub(crate) committed: Option<CellEdit>,
    pub(crate) draft: Option<String>,
    pub(crate) fill_swapped: bool,
    pub(crate) filter: Option<(u32, FilterButtonEvent)>,
    pub(crate) fix: Option<usize>,
    pub(crate) outline: Vec<OutlineAction>,
}

/// The style of the grid: the sheet look, the header and selection colours and the sizes the geometry reads.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CellGridStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    pub(crate) sheet: SheetMetrics,
    pub(crate) bands: Vec2,
    pub(crate) background: StyleColor,
    pub(crate) header: Stateful<StyleColor>,
    pub(crate) header_text: Stateful<StyleColor>,
    pub(crate) gridline: StyleColor,
    pub(crate) header_line: StyleColor,
    pub(crate) accent: StyleColor,
    pub(crate) preview: StyleColor,
    pub(crate) selection: StyleColor,
    pub(crate) handle_fill: StyleColor,
    pub(crate) handle_ring: StyleColor,
    pub(crate) edit_fill: StyleColor,
    pub(crate) edit_edge: StyleColor,
    pub(crate) handle: f32,
    pub(crate) mark: f32,
    pub(crate) pad: f32,
    pub(crate) border_reach: f32,
    pub(crate) side: f32,
    pub(crate) gap: f32,
}

/// The cell grid: a sheet of cells with headers, frozen panes, merged cells, a selection of ranges, an in-cell editor,
/// a fill handle and copy marks.
///
/// - Pointer: click selects, drag extends with auto-scroll, Shift+click extends from the anchor, Cmd/Ctrl+click adds a
///   range on an unselected cell and removes a selected one; a header click selects the line, a drag over headers
///   several; the corner selects all. A header edge drags a resize, a double-click autofits (or unhides a hidden
///   line); the fill handle on the current range's corner drags a [`GridEdit::Fill`], the range's border a
///   [`GridEdit::MoveCells`] (Esc cancels either).
/// - Keys: arrows move (Shift extends; Cmd/Ctrl jumps to [`GridModel::data_edge`]), Page Up and Down a screen, Home the
///   row start, Cmd/Ctrl+Home the first cell after the frozen panes, Cmd/Ctrl+End [`GridModel::last_used`], Enter and
///   Tab advance inside a selection (Shift back), Cmd/Ctrl+A selects all, Cmd/Ctrl+Space the column, Shift+Space the
///   row, F2 edits, Delete clears, Backspace clears and edits, typed text starts an edit with it. Esc and Enter with a
///   copy mark report [`GridEdit::ClearCopyMark`] and [`GridEdit::PasteAtActive`].
/// - The editor: typing opens the active cell in Enter mode (the arrows move to another cell and commit), F2 in Edit
///   mode (the arrows move the caret), `=` and an operator in point mode (the arrows move a reference cursor).
///
/// The scroll lives in the collection.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(collection)]
pub struct CellGrid<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    /// Whether the row and column headings show.
    #[style_key]
    #[prop]
    headings: bool,
    #[style_key]
    digits: u8,
    #[style_key]
    percent: u16,
    #[style_key]
    levels: (u8, u8),
    model: &'a dyn GridModel,
    selection: Bind<'a, CellSelection<u32>>,
    /// The selection as the app held it when the pass began, for the parts that declare before the item step runs.
    snapshot: CellSelection<u32>,
    zoom: f32,
    /// The ranges the copy marks surround.
    #[prop]
    copy_mark: &'a [CellRange<u32>],
    /// The columns that show a filter button, with its state.
    #[prop]
    filters: &'a [ColumnFilter],
    /// The cell whose problem shows an error tag, with its fixes.
    #[prop]
    error_tag: Option<CellProblem<'a>>,
    outline: Option<(&'a OutlineAxis, &'a OutlineAxis)>,
    selectable: Box<dyn Fn(CellAt<u32>) -> bool + 'a>,
    frame: Option<paint::Frame>,
    /// The row the keys of this pass moved the active cell to.
    reveal: Option<u32>,
    pass: PassOutput,
}

impl<'a> CellGrid<'a> {
    /// A grid named `label` over `model` that keeps its selection and zoom in `state`.
    pub fn new(label: impl Into<Label>, model: &'a dyn GridModel, state: &'a mut GridState) -> Self {
        let GridState { selection, zoom } = state;
        let zoom = if zoom.is_finite() { zoom.clamp(*SHEET_ZOOM.start(), *SHEET_ZOOM.end()) } else { 1.0 };
        let digits = digits_of(model.last_used().row.saturating_add(ROW_HEADER_LOOKAHEAD as u32)) as u8;
        CellGrid {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Cells),
            headings: true,
            digits,
            percent: (zoom * 100.0).round() as u16,
            levels: (0, 0),
            model,
            snapshot: selection.clone(),
            selection: Bind::bound(selection),
            zoom,
            copy_mark: &[],
            filters: &[],
            error_tag: None,
            outline: None,
            selectable: Box::new(move |cell| model.selectable(cell)),
            frame: None,
            reveal: None,
            pass: PassOutput::default(),
        }
        .label(label)
    }

    /// Reserves the outline bands for the groups of `rows` and `columns` inside the grid's rect and shows one band over
    /// them. Zero levels reserve nothing.
    #[must_use]
    pub fn outline(mut self, rows: &'a OutlineAxis, columns: &'a OutlineAxis) -> Self {
        self.levels = (rows.levels, columns.levels);
        self.outline = Some((rows, columns));
        self
    }

    /// Shows the grid and reports what the user did. The rows are the model's, one item each, drawn by [`GridRow`].
    pub fn show_sheet(self, ui: &mut egui::Ui) -> GridOutput {
        use craft_ui::contract::ShowCollection as _;
        let model: &'a dyn GridModel = self.model;
        let rows = SheetRows(Positions::new(model.rows().count()));
        let zoom = self.zoom;
        self.show_items(ui, &rows, &mut |key, icx| GridRow::new(model, *key, zoom, icx.cells().to_vec()))
    }
}

impl craft_ui::contract::Component for CellGrid<'_> {
    const ROLE: Role = Role::GRID;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::of(craft_ui::contract::Input::Editor).with_keep(KeepKeys::None).with_end(EndKey::EndMode);
    type Style = CellGridStyle;
    type Output = GridOutput;

    fn style(cx: &StyleCx<'_>, key: CellGridStyleKey) -> CellGridStyle {
        paint::style(cx, key)
    }

    fn parts(&mut self, rect: Rect, cx: &mut craft_ui::contract::PartsCx<'_, Self>) {
        self.declare_parts(rect, cx);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.run_input(cx);
    }

    fn access(&self, access: &mut craft_ui::contract::Access, cx: &Cx<'_, Self>) {
        self.describe(access, cx);
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.paint_sheet(ui, cx);
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> GridOutput {
        let mut collection = cx.collection_response();
        let geometry = self.frame.as_ref().map(|frame| frame.geometry.clone());
        let mut pass = self.pass;
        if collection.events.iter().any(|event| matches!(event, CollectionEvent::Activate(_)))
            && let Some(cell) = cx.collection_selection().and_then(|selection| selection.active().copied())
        {
            let state = cx.memory::<editor::EditState>();
            if state.mode == editor::Mode::Ready {
                state.open(cell, editor::Mode::Edit, &self.model.cell_text(cell));
                pass.draft = Some(state.draft.text().to_owned());
                pass.edits.push(GridEdit::BeginEdit { typed: None });
            }
        }
        collection.events.retain(|event| !matches!(event, CollectionEvent::Activate(_)));
        if let Some(geometry) = &geometry {
            for event in &collection.events {
                if let CollectionEvent::ContextMenu(ContextMenuRequest { at, .. }) = event {
                    let cell = geometry.cell_at(geometry.mirror_pos(*at));
                    pass.edits.push(GridEdit::ContextMenu {
                        anchor: geometry.mirror_rect(geometry.cell_rect(cell)),
                        pointer: Some(*at),
                        target: GridMenuTarget::Cells,
                    });
                }
            }
        }
        collection.events.retain(|event| !matches!(event, CollectionEvent::ContextMenu(_)));
        let geometry = geometry.unwrap_or_else(|| {
            let layout = GridLayout::new(cx.rect, self.model.rows().clone(), self.model.columns().clone()).bands(cx.style.bands);
            SheetGeometry::new(layout, cx.style.sheet)
        });
        GridOutput {
            collection,
            geometry,
            edits: pass.edits,
            committed: pass.committed,
            draft: pass.draft,
            fill_swapped: pass.fill_swapped,
            filter: pass.filter,
            fix: pass.fix,
            outline: pass.outline,
        }
    }
}

impl Collection for CellGrid<'_> {
    type Key = u32;
    type ItemLayout = SheetLayout;
    type Selection = CellSelection<u32>;
    type Item<'r> = GridRow<'r>;
    type ItemEvent = ();

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, CellSelection<u32>>> {
        Some(self.selection.reborrow())
    }

    fn reveal(&self) -> Option<&u32> {
        self.reveal.as_ref()
    }

    fn cell_space<'s>(&'s self, space: CellSpace<'s, u32>) -> CellSpace<'s, u32> {
        space.merges(self.model.merges()).selectable(&*self.selectable)
    }

    fn item_layout(&self, _: &dyn ItemSource<u32>, _: f32, _: &StyleCx<'_>) -> SheetLayout {
        self.sheet_layout()
    }
}

impl CellGrid<'_> {
    /// The layout of the sheet: its frozen lines say where Ctrl+Home lands.
    fn sheet_layout(&self) -> SheetLayout {
        let (rows, columns) = self.model.frozen();
        SheetLayout::lines(self.model.rows().clone(), self.model.columns().clone()).frozen(columns as usize).frozen_rows(rows).zoom(self.zoom)
    }
}

craft_ui::words! {
    /// The grid's words, English by default.
    #[non_exhaustive]
    pub struct CellGridWords {
        select: "cellGrid.select" = "Select Cells";
        resize_column: "cellGrid.resizeColumn" = "Resize Column";
        resize_row: "cellGrid.resizeRow" = "Resize Row";
        fill: "cellGrid.fill" = "Fill";
        move_cells: "cellGrid.moveCells" = "Move Cells";
        column_header: template "cellGrid.columnHeader" = "Column {name}";
        row_header: template "cellGrid.rowHeader" = "Row {n}";
        corner: "cellGrid.corner" = "Select All";
        outline: "cellGrid.outline" = "Outline";
    }
}
