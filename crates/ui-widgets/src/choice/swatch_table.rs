//! `SwatchTable`: a table of touching cells selected by index, for a colour table whose entries may repeat a colour.

use super::swatch::hexagon;
use super::{CellMarks, Swatch, SwatchMetrics, SwatchShape};
use crate::contract::{
    ActivateOn, Bind, BindMut, Collection, CollectionEvent, CollectionProps, Common, Component, ContainerProps, Cx, LeadOnOpen, ListDrag, NoValue,
    Outcome, Props, Remember as _, Role, SelectMode, ShowCollection as _, Sizing,
};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{DisplayOrder, IconId, ItemLayout, ItemSource, Metrics, NavShape, Preview, ScrollPos, Selection, TileLayout};
use craft_theme::Srgba;
use egui::{Pos2, Rect, Response, Ui, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The entries of a colour table as an item source: keyed by index, named for type-ahead.
#[derive(Clone, Copy, Debug)]
pub struct SwatchRows<'a> {
    swatches: &'a [(Label, Srgba)],
    /// The name of the one cell after the entries, keyed `swatches.len()`.
    trailing: Option<&'a Label>,
}

impl<'a> SwatchRows<'a> {
    /// The entries of `swatches`.
    pub const fn new(swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchRows { swatches, trailing: None }
    }

    /// The entries with one more cell named `label` after them, keyed `swatches.len()`.
    #[must_use]
    pub const fn trailing(mut self, label: &'a Label) -> Self {
        self.trailing = Some(label);
        self
    }
}

impl DisplayOrder<usize> for SwatchRows<'_> {
    fn len(&self) -> usize {
        self.swatches.len() + usize::from(self.trailing.is_some())
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        match self.swatches.get(at) {
            Some((name, _)) => Some(std::borrow::Cow::Borrowed(name.as_str())),
            None => self.trailing.filter(|_| at == self.swatches.len()).map(|name| std::borrow::Cow::Borrowed(name.as_str())),
        }
    }
}

impl ItemSource<usize> for SwatchRows<'_> {}

/// How the cells of a [`SwatchTable`] sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwatchSpacing {
    /// The cells touch.
    #[default]
    Touching,
    /// The theme's item gap (`space.gap.item`) lies between the cells.
    Spaced,
}

/// The marks of the cell of a key: `SwatchTable::marks`.
pub struct RowMarks<'a, K>(Box<dyn Fn(&K) -> CellMarks + 'a>);

impl<'a, K> RowMarks<'a, K> {
    /// The marks `marks` gives.
    pub fn new(marks: impl Fn(&K) -> CellMarks + 'a) -> Self {
        RowMarks(Box::new(marks))
    }

    /// The marks of the cell of `key`.
    pub fn of(&self, key: &K) -> CellMarks {
        (self.0)(key)
    }
}

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
    /// The keys Delete or Backspace deleted this pass (`CollectionEvent::Delete`).
    pub delete: Vec<usize>,
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
/// the table's width edge to edge; [`marks`](Self::marks) gives each cell its marks.
///
/// - A click or Space selects the cell; a double click or Enter reports `open` and the Enter is the table's, so a
///   dialog around it does not also answer OK.
/// - The table is one Tab stop; the arrows move through the cells as a grid.
/// - The cell under the pointer is previewed ([`SwatchTableOutput::preview`]); the selection stays.
/// - AccessKit: a list box named by the label; each cell a list box option named by its entry, with the hex colour as
///   its value.
/// - A [`trailing`](Self::trailing) cell follows the entries at key `swatches.len()`: a well with a glyph, selected
///   and opened by that key as an entry is, with no colour value.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection)]
pub struct SwatchTable<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    // `max_rows` is the table's own prop below: it caps the cells, where the collection's scrolls the rows.
    #[collection(
        select,
        reorder,
        rename,
        rename_column,
        slow_click_rename,
        columns,
        sort,
        activate,
        striped,
        allow_empty,
        lead_on_open,
        hover_lead,
        arrows_select,
        arrows_wrap,
        type_ahead,
        rows_stamp,
        focus_on_press,
        empty,
        space,
        header,
        digit_keys,
        menu_on_click,
        item_keys,
        marquee,
        deletable,
        drag_count,
        natural_height,
        min_rows,
        row_height,
        notch,
        command_click,
        current
    )]
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
    /// Whether each cell's name is its tooltip.
    #[prop]
    tips: bool,
    /// Whether the cells are wells at the control height, as a palette's swatches are, instead of small touching cells.
    #[prop]
    wells: bool,
    /// How the cells sit: touching, or the theme's item gap apart.
    #[prop]
    spacing: SwatchSpacing,
    /// The marks of each cell, by index, given to the cell's `Swatch::marks`.
    #[prop(func = "Fn(&usize) -> CellMarks")]
    marks: Option<RowMarks<'a, usize>>,
    /// Only the cells of the first `n` rows at the laid-out column count are items; the rest are not shown.
    #[prop]
    max_rows: Option<usize>,
    /// How the cells lie: a square grid, or touching hexagons. A honeycomb ignores `per_row`, `span`, `wells`,
    /// `spacing` and `max_rows`: its rows follow the count of entries and its cells are `size.icon.md` wide.
    #[prop]
    arrangement: SwatchArrangement,
    /// The icon cell after the entries: its name and glyph.
    trailing: Option<(Label, IconId)>,
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
            items: CollectionProps::new()
                .select(SelectMode::Single)
                .activate(ActivateOn::DoubleClick)
                .arrows_select(false)
                .lead_on_open(LeadOnOpen::First)
                .natural_height(true)
                .deletable(true),
            swatches,
            selection: None,
            per_row: None,
            span: false,
            tips: false,
            wells: false,
            spacing: SwatchSpacing::Touching,
            marks: None,
            max_rows: None,
            arrangement: SwatchArrangement::default(),
            trailing: None,
        }
        .label(label)
    }

    /// Hands the drag of the cells to the host as `map` gives its payload, as `TileGrid::drag_payload` does.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<usize>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
        self
    }

    /// One icon cell after the entries, at key `swatches.len()`, in the same Tab stop and arrow walk: the Other… cell
    /// of a palette.
    #[must_use]
    pub fn trailing(mut self, label: impl Into<Label>, icon: IconId) -> Self {
        self.trailing = Some((label.into(), icon));
        self
    }

    /// The number of cells: the entries and the trailing cell.
    fn cells(&self) -> usize {
        self.swatches.len() + usize::from(self.trailing.is_some())
    }

    /// The gap between two cells.
    fn gap(&self, cx: &StyleCx<'_>) -> f32 {
        match self.spacing {
            SwatchSpacing::Spaced => cx.tokens.space.gap.item,
            _ => 0.0,
        }
    }

    /// The layout of the cells of `source` in `width`.
    fn tiles(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> TileLayout {
        let (cell, gap) = (self.cell(cx), self.gap(cx));
        match self.per_row {
            Some(columns) => {
                let columns = columns.max(1);
                let spans = self.span && width.is_finite() && width > 0.0;
                let tile = if spans { Vec2::new((width - gap * (columns - 1) as f32) / columns as f32, cell) } else { Vec2::splat(cell) };
                TileLayout::new(source, width, tile, gap).in_columns(columns)
            }
            None => TileLayout::new(source, width, Vec2::splat(cell), gap),
        }
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
        let marks = self.marks.take();
        // A clone, not a take: `ideal_width` reads the trailing cell while the table shows.
        let trailing = self.trailing.clone();
        let rows = match &trailing {
            Some((name, _)) => SwatchRows::new(swatches).trailing(name),
            None => SwatchRows::new(swatches),
        };
        self.show_items(ui, &rows, &mut |key, _| {
            let (name, cell) = match (swatches.get(*key), &trailing) {
                (Some((name, color)), _) => (name.clone(), Swatch::new(name.clone(), *color)),
                (None, Some((name, icon))) => (name.clone(), Swatch::icon(name.clone(), icon.clone())),
                // The rows name no key past the trailing cell; a mixed well keeps the closure total without a panic.
                (None, None) => (Label::Empty, Swatch::new(Label::Empty, craft_behavior::Mixed::<Srgba>::Mixed)),
            };
            let cell = cell.cell().shape(shape);
            let cell = if wells { cell.well() } else { cell };
            let cell = match &marks {
                Some(marks) => cell.marks(marks.of(key)),
                None => cell,
            };
            if tips { cell.tooltip(name) } else { cell }
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
        let changed = response.events.iter().any(|event| matches!(event, CollectionEvent::Selection { .. }));
        let delete = response
            .events
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Delete(keys) => Some(keys.iter().copied()),
                _ => None,
            })
            .flatten()
            .collect();
        let hovered = response.shown.iter().find(|row| row.hovered).map(|row| row.key);
        let preview = cx.memory::<Previewed>().follow(hovered);
        SwatchTableOutput {
            response: response.response,
            selected: selection.lead.filter(|_| changed),
            chosen: selection.items,
            open,
            delete,
            preview,
        }
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
        let cells = self.cells();
        if self.arrangement == SwatchArrangement::Honeycomb {
            return HoneycombLayout::new(cells, self.cell(cx)).columns() as f32 * self.cell(cx) + 0.5;
        }
        let columns = self.per_row.map_or(cells, |columns| columns.max(1).min(cells));
        // Half a point over, so the division that counts the columns back never rounds below `columns`.
        columns as f32 * self.cell(cx) + columns.saturating_sub(1) as f32 * self.gap(cx) + 0.5
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> SwatchLayout {
        if self.arrangement == SwatchArrangement::Honeycomb {
            return SwatchLayout::Honeycomb(HoneycombLayout::new(source.len(), self.cell(cx)));
        }
        let layout = self.tiles(source, width, cx);
        let Some(rows) = self.max_rows else { return SwatchLayout::Grid(layout) };
        // The cap is a count of the cells the room lays out in `rows` rows, so it follows the layout's columns.
        let keep = rows.saturating_mul(layout.columns()).min(self.swatches.len());
        SwatchLayout::Grid(self.tiles(&SwatchRows::new(&self.swatches[..keep]), width, cx))
    }
}
