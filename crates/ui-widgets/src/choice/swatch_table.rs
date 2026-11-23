//! `SwatchTable`: a table of touching cells selected by index, for a colour table whose entries may repeat a colour.

use super::swatch::hexagon;
use super::{Swatch, SwatchMetrics, SwatchShape};
use crate::contract::{
    ActivateOn, Bind, BindMut, Collection, CollectionEvent, CollectionProps, Common, Component, ContainerProps, Cx, NoValue, Outcome, Props,
    Remember as _, Role, SelectMode, ShowCollection as _, Sizing,
};
use crate::list::RowTest;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{DisplayOrder, ItemLayout, ItemSource, Metrics, NavShape, Preview, ScrollPos, Selection, TileLayout};
use craft_theme::Srgba;
use egui::{Pos2, Rect, Response, Ui, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The entries of a colour table as an item source: keyed by index, named for type-ahead.
#[derive(Clone, Copy, Debug)]
pub struct SwatchRows<'a>(&'a [(Label, Srgba)]);

impl<'a> SwatchRows<'a> {
    /// The entries of `swatches`.
    pub const fn new(swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchRows(swatches)
    }
}

impl DisplayOrder<usize> for SwatchRows<'_> {
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
        self.0.get(at).map(|(name, _)| std::borrow::Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for SwatchRows<'_> {}

/// What a [`SwatchTable`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the selection and the open request"]
pub struct SwatchTableOutput {
    /// The cells.
    pub response: Response,
    /// The entry a click, Space or a screen reader's click selected this pass: the lead of the selection it made.
    pub selected: Option<usize>,
    /// The selected entries after the pass, in selection order.
    pub chosen: Vec<usize>,
    /// The entry a double click or Enter asked to edit this pass.
    pub open: Option<usize>,
    /// What this pass did to the preview of the cell under the pointer: `Shows` in the pass the pointer moves onto a
    /// cell, `Ended` in the pass it leaves the last one. The keyboard lead previews nothing.
    pub preview: Preview<usize>,
}

/// The cell the table last reported as previewed, kept in its shown memory, so a pass reports a preview only when the
/// cell under the pointer changes and a second pass of a frame reports nothing.
#[derive(Clone, Copy, Debug, Default)]
struct Previewed(Option<usize>);

impl Previewed {
    /// Follows the cell under the pointer this pass and reports what changed.
    fn follow(&mut self, hovered: Option<usize>) -> Preview<usize> {
        let preview = match (hovered, self.0) {
            (Some(at), shown) if shown != Some(at) => Preview::Shows(at),
            (None, Some(_)) => Preview::Ended,
            _ => Preview::Unchanged,
        };
        self.0 = hovered;
        preview
    }
}

impl Outcome for SwatchTableOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// How a [`SwatchTable`] lays out its cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwatchArrangement {
    /// Square cells in rows and columns.
    #[default]
    Grid,
    /// Touching hexagon cells; the arrows move between neighbours.
    Honeycomb,
}

/// The cells of a honeycomb: pointy-top hexagons, each row offset by half a cell from its neighbours and stacked at
/// three quarters of the cell height, so every cell touches its six neighbours. The rows grow from `n` cells to `2n - 1`
/// and shrink back, `n` being the smallest side whose full hexagon (`3n(n - 1) + 1` cells) holds the count. A row sits
/// where the full hexagon's row of that index sits, so a count short of a full hexagon leaves the last row short and
/// continuing the lattice from its left, never shifted by a whole cell.
///
/// Arrows: Left and Right step to the neighbour in the row. Down from column `c` goes to column `c` of the next row
/// while it is longer and to column `c - 1` (at least 0) while it is shorter; Up goes the same way, to `c - 1` while the
/// row above is shorter. Both are the lower-left neighbour of the two a hexagon has, as [`column_shift`] says.
///
/// [`column_shift`]: ItemLayout::column_shift
#[derive(Clone, Debug)]
pub struct HoneycombLayout {
    tile: Vec2,
    /// The cells of each row.
    lens: Vec<usize>,
    /// The index of each row's first cell.
    starts: Vec<usize>,
    /// The side of the full hexagon: its longest row has `2 * side - 1` cells.
    side: usize,
}

impl HoneycombLayout {
    /// `count` cells of width `cell`.
    fn new(count: usize, cell: f32) -> Self {
        let side = (1usize..).find(|&n| 3 * n * (n - 1) + 1 >= count).unwrap_or(1);
        let mut lens = Vec::new();
        let mut left = count;
        for row in 0..2 * side - 1 {
            let len = (side + row.min(2 * side - 2 - row)).min(left);
            if len == 0 {
                break;
            }
            lens.push(len);
            left -= len;
        }
        let starts = lens
            .iter()
            .scan(0, |start, len| {
                let first = *start;
                *start += len;
                Some(first)
            })
            .collect();
        HoneycombLayout { tile: Vec2::new(cell, cell * 2.0 / 3.0_f32.sqrt()), lens, starts, side }
    }

    /// The cells of the longest row of the full hexagon: the width of the table.
    fn columns(&self) -> usize {
        2 * self.side - 1
    }

    /// The cells of `row` in the full hexagon.
    fn full_len(&self, row: usize) -> usize {
        self.side + row.min((2 * self.side - 2).saturating_sub(row))
    }

    /// The space before `row`, in half cells.
    fn lead(&self, row: usize) -> usize {
        self.columns() - self.full_len(row)
    }

    /// The space before `row`'s first cell.
    fn left(&self, row: usize) -> f32 {
        self.lead(row) as f32 * self.tile.x / 2.0
    }

    /// The distance between the tops of two rows.
    fn pitch(&self) -> f32 {
        self.tile.y * 0.75
    }

    /// The cell `at` in content space.
    fn cell_rect(&self, at: usize) -> Option<Rect> {
        let row = self.starts.partition_point(|&start| start <= at).checked_sub(1)?;
        let column = at - self.starts[row];
        if column >= self.lens[row] {
            return None;
        }
        let left = self.left(row) + column as f32 * self.tile.x;
        Some(Rect::from_min_size(Pos2::new(left, row as f32 * self.pitch()), self.tile))
    }
}

/// Whether `point` is inside the convex `polygon`, its edge included.
fn inside(polygon: &[Pos2], point: Pos2) -> bool {
    let mut side = 0.0_f32;
    for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let cross = (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
        if cross == 0.0 {
            continue;
        }
        if side == 0.0 {
            side = cross.signum();
        } else if side != cross.signum() {
            return false;
        }
    }
    true
}

impl ItemLayout for HoneycombLayout {
    fn content_size(&self) -> ScrollPos {
        let height = if self.lens.is_empty() { 0.0 } else { (self.lens.len() - 1) as f64 * f64::from(self.pitch()) + f64::from(self.tile.y) };
        ScrollPos { x: self.columns() as f64 * f64::from(self.tile.x), y: height }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        for (row, (start, len)) in self.starts.iter().zip(&self.lens).enumerate() {
            let top = f64::from(row as f32 * self.pitch());
            if top + f64::from(self.tile.y) > offset.y && top < offset.y + f64::from(view.y) {
                out.extend(*start..start + len);
            }
        }
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        let rect = self.cell_rect(at).unwrap_or(Rect::ZERO);
        rect.translate(Vec2::new(-(offset.x as f32), -(offset.y as f32)))
    }

    /// The cell whose hexagon holds the point; the rows overlap in height, so the row of the point and the one above
    /// are tried.
    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        let at = Pos2::new(point.x + offset.x as f32, point.y + offset.y as f32);
        if at.x < 0.0 || at.y < 0.0 {
            return None;
        }
        let row = ((at.y / self.pitch()) as usize).min(self.lens.len().checked_sub(1)?);
        [row, row.saturating_sub(1)].into_iter().find_map(|row| {
            let left = self.left(row);
            let column = ((at.x - left) / self.tile.x).floor();
            if column < 0.0 || column as usize >= self.lens[row] {
                return None;
            }
            let index = self.starts[row] + column as usize;
            self.cell_rect(index).filter(|rect| inside(&hexagon(*rect), at)).map(|_| index)
        })
    }

    fn nav(&self) -> NavShape {
        NavShape::RaggedGrid
    }

    fn row_len(&self, row: usize) -> Option<usize> {
        self.lens.get(row).copied()
    }

    /// Half a cell of offset per step between rows, rounded down: a row that starts later (shorter) puts the lower-left
    /// neighbour one column earlier, a row that starts earlier (longer) keeps the column.
    fn column_shift(&self, from: usize, to: usize) -> isize {
        (self.lead(from) as isize - self.lead(to) as isize).div_euclid(2)
    }
}

/// The geometry of a [`SwatchTable`], by its [`SwatchArrangement`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SwatchLayout {
    /// Square cells in rows and columns.
    Grid(TileLayout),
    /// Touching hexagon cells.
    Honeycomb(HoneycombLayout),
}

impl ItemLayout for SwatchLayout {
    fn content_size(&self) -> ScrollPos {
        match self {
            SwatchLayout::Grid(tiles) => tiles.content_size(),
            SwatchLayout::Honeycomb(cells) => cells.content_size(),
        }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        match self {
            SwatchLayout::Grid(tiles) => tiles.visible(offset, view, out),
            SwatchLayout::Honeycomb(cells) => cells.visible(offset, view, out),
        }
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        match self {
            SwatchLayout::Grid(tiles) => tiles.rect(at, offset),
            SwatchLayout::Honeycomb(cells) => cells.rect(at, offset),
        }
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        match self {
            SwatchLayout::Grid(tiles) => tiles.hit(point, offset),
            SwatchLayout::Honeycomb(cells) => cells.hit(point, offset),
        }
    }

    fn nav(&self) -> NavShape {
        match self {
            SwatchLayout::Grid(tiles) => tiles.nav(),
            SwatchLayout::Honeycomb(cells) => cells.nav(),
        }
    }

    fn row_len(&self, row: usize) -> Option<usize> {
        match self {
            SwatchLayout::Grid(tiles) => tiles.row_len(row),
            SwatchLayout::Honeycomb(cells) => cells.row_len(row),
        }
    }

    fn column_shift(&self, from: usize, to: usize) -> isize {
        match self {
            SwatchLayout::Grid(tiles) => tiles.column_shift(from, to),
            SwatchLayout::Honeycomb(cells) => cells.column_shift(from, to),
        }
    }
}

/// A table of touching colour cells, one per entry, wrapping to the width or to a fixed count per row. Entries are
/// told apart by index, so duplicates select one cell. With [`span`](Self::span) and a count per row, the cells share
/// the table's width edge to edge; [`marked`](Self::marked) cells draw the swatch mark.
///
/// - A click or Space selects the cell; a double click or Enter reports `open` and the Enter is the table's, so a
///   dialog around it does not also answer OK.
/// - The table is one Tab stop; the arrows move through the cells as a grid.
/// - The cell under the pointer is previewed ([`SwatchTableOutput::preview`]); the selection stays.
/// - AccessKit: a list box named by the label; each cell a list box option named by its entry, with the hex colour as
///   its value.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection)]
pub struct SwatchTable<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    swatches: &'a [(Label, Srgba)],
    /// The selection the caller owns.
    #[prop]
    selection: Option<Bind<'a, Selection<usize>>>,
    /// A fixed number of cells to a row instead of wrapping to the width.
    #[prop]
    per_row: Option<usize>,
    /// With `per_row`, the cells of a row divide the table's width among them, at the cell height.
    #[prop]
    span: bool,
    /// The cells that draw the swatch mark, by index.
    #[prop(func = "Fn(&usize) -> bool")]
    marked: Option<RowTest<'a, usize>>,
    /// Whether each cell's name is its tooltip.
    #[prop]
    tips: bool,
    /// Whether the cells are wells at the control height, as a palette's swatches are, instead of small touching cells.
    #[prop]
    wells: bool,
    /// How the cells lie: a square grid, or touching hexagons. A honeycomb ignores `per_row`, `span` and `wells`: its
    /// rows follow the count of entries and its cells are `size.icon.md` wide.
    #[prop]
    arrangement: SwatchArrangement,
}

impl<'a> SwatchTable<'a> {
    /// The side of a cell: a well at the control height, or a small touching cell.
    fn cell(&self, cx: &StyleCx<'_>) -> f32 {
        let metrics = SwatchMetrics::of(cx);
        if self.wells { metrics.side } else { metrics.cell }
    }

    /// A table named `label` of `swatches`, each a name and a colour.
    pub fn new(label: impl Into<Label>, swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchTable {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::FillWidth),
            items: CollectionProps::new().select(SelectMode::Single).activate(ActivateOn::DoubleClick).arrows_select(false).natural_height(true),
            swatches,
            selection: None,
            per_row: None,
            span: false,
            marked: None,
            tips: false,
            wells: false,
            arrangement: SwatchArrangement::default(),
        }
        .label(label)
    }

    /// Whether Shift or Cmd adds cells to the selection.
    #[must_use]
    pub fn multiple(mut self, on: bool) -> Self {
        self.items = self.items.select(if on { SelectMode::Multi } else { SelectMode::Single });
        self
    }

    /// Shows the table.
    pub fn show_all(mut self, ui: &mut Ui) -> SwatchTableOutput {
        let (swatches, tips, wells) = (self.swatches, self.tips, self.wells);
        let shape = if self.arrangement == SwatchArrangement::Honeycomb { SwatchShape::Hexagon } else { SwatchShape::Square };
        let marked = self.marked.take();
        self.show_items(ui, &SwatchRows::new(swatches), &mut |key, _| {
            let (name, color) = &swatches[*key];
            let cell = Swatch::new(name.clone(), *color).cell().shape(shape);
            let cell = if wells { cell.well() } else { cell };
            let cell = if marked.as_ref().is_some_and(|test| test.passes(key)) { cell.marked() } else { cell };
            if tips { cell.tooltip(name.clone()) } else { cell }
        })
    }
}

/// The style of a table: it has no look of its own; its cells are swatch faces.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SwatchTableStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for SwatchTable<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::LIST_BOX;
    const PARTS: bool = false;
    type Style = SwatchTableStyle;
    type Output = SwatchTableOutput;

    fn style(_: &StyleCx<'_>, _: SwatchTableStyleKey) -> SwatchTableStyle {
        SwatchTableStyle { metrics: Metrics::new() }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> SwatchTableOutput {
        let selection = cx.collection_selection().cloned().unwrap_or_default();
        let response = cx.collection_response();
        let open = response.events.iter().find_map(|event| match event {
            CollectionEvent::Activate(key) => Some(*key),
            _ => None,
        });
        let changed = response.events.iter().any(|event| matches!(event, CollectionEvent::Selection(_)));
        let hovered = response.shown.iter().find(|row| row.hovered).map(|row| row.key);
        let preview = cx.memory::<Previewed>().follow(hovered);
        SwatchTableOutput { response: response.response, selected: selection.lead.filter(|_| changed), chosen: selection.items, open, preview }
    }
}

impl Collection for SwatchTable<'_> {
    type Key = usize;
    type ItemLayout = SwatchLayout;
    type Selection = Selection<usize>;
    type Item<'r> = Swatch;
    type ItemEvent = std::convert::Infallible;
    const MEMBERS: bool = true;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    /// Where the table takes its width from its content it asks for every cell in one row, or its count per row: the
    /// room it is offered then wraps the cells, as it does where it is given the width.
    fn ideal_width(&self, _item: Option<Vec2>, _rows: usize, cx: &StyleCx<'_>) -> f32 {
        if self.arrangement == SwatchArrangement::Honeycomb {
            return HoneycombLayout::new(self.swatches.len(), self.cell(cx)).columns() as f32 * self.cell(cx) + 0.5;
        }
        let columns = self.per_row.map_or(self.swatches.len(), |columns| columns.max(1).min(self.swatches.len()));
        // Half a point over, so the division that counts the columns back never rounds below `columns`.
        columns as f32 * self.cell(cx) + 0.5
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> SwatchLayout {
        let cell = self.cell(cx);
        if self.arrangement == SwatchArrangement::Honeycomb {
            return SwatchLayout::Honeycomb(HoneycombLayout::new(source.len(), cell));
        }
        SwatchLayout::Grid(match self.per_row {
            Some(columns) => {
                let columns = columns.max(1);
                let spans = self.span && width.is_finite() && width > 0.0;
                let tile = if spans { Vec2::new(width / columns as f32, cell) } else { Vec2::splat(cell) };
                TileLayout::new(source, width, tile, 0.0).in_columns(columns)
            }
            None => TileLayout::new(source, width, Vec2::splat(cell), 0.0),
        })
    }
}
