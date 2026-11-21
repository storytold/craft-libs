//! `SwatchTable`: a table of touching cells selected by index, for a colour table whose entries may repeat a colour.

use super::{CellMarks, Swatch, SwatchMetrics};
use crate::contract::{
    ActivateOn, Bind, BindMut, Collection, CollectionEvent, CollectionProps, Common, Component, ContainerProps, Cx, ListDrag, NoValue, Outcome,
    Props, Role, SelectMode, ShowCollection as _, Sizing,
};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{DisplayOrder, ItemSource, Metrics, Selection, TileLayout};
use craft_theme::Srgba;
use egui::{Response, Ui, Vec2};

#[cfg(feature = "testing")]
mod stories;

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
}

impl Outcome for SwatchTableOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A table of touching colour cells, one per entry, wrapping to the width or to a fixed count per row. Entries are
/// told apart by index, so duplicates select one cell. With [`span`](Self::span) and a count per row, the cells share
/// the table's width edge to edge; [`marks`](Self::marks) gives each cell its marks.
///
/// - A click or Space selects the cell; a double click or Enter reports `open` and the Enter is the table's, so a
///   dialog around it does not also answer OK.
/// - The table is one Tab stop; the arrows move through the cells as a grid.
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
        }
        .label(label)
    }

    /// Hands the drag of the cells to the host as `map` gives its payload, as `TileGrid::drag_payload` does.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(self, map: impl Fn(&ListDrag<usize>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
        self
    }

    /// What activates a cell: a double click by default.
    #[must_use]
    pub fn activate(mut self, on: ActivateOn) -> Self {
        self.items = self.items.activate(on);
        self
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
        let marks = self.marks.take();
        self.show_items(ui, &SwatchRows::new(swatches), &mut |key, _| {
            let (name, color) = &swatches[*key];
            let cell = Swatch::new(name.clone(), *color).cell();
            let cell = if wells { cell.well() } else { cell };
            let cell = match &marks {
                Some(marks) => cell.marks(marks.of(key)),
                None => cell,
            };
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
        let changed = response.events.iter().any(|event| matches!(event, CollectionEvent::Selection { .. })); // P3(mechanical): dp.C12
        let delete = response
            .events
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Delete(keys) => Some(keys.iter().copied()),
                _ => None,
            })
            .flatten()
            .collect();
        SwatchTableOutput { response: response.response, selected: selection.lead.filter(|_| changed), chosen: selection.items, open, delete }
    }
}

impl Collection for SwatchTable<'_> {
    type Key = usize;
    type ItemLayout = TileLayout;
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
    fn ideal_width(&self, _item: Option<Vec2>, cx: &StyleCx<'_>) -> f32 {
        let columns = self.per_row.map_or(self.swatches.len(), |columns| columns.max(1).min(self.swatches.len()));
        // Half a point over, so the division that counts the columns back never rounds below `columns`.
        columns as f32 * self.cell(cx) + columns.saturating_sub(1) as f32 * self.gap(cx) + 0.5
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> TileLayout {
        let layout = self.tiles(source, width, cx);
        let Some(rows) = self.max_rows else { return layout };
        // The cap is a count of the cells the room lays out in `rows` rows, so it follows the layout's columns.
        let keep = rows.saturating_mul(layout.columns()).min(self.swatches.len());
        self.tiles(&SwatchRows::new(&self.swatches[..keep]), width, cx)
    }
}
