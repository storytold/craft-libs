//! `SwatchTable`: a table of touching cells selected by index, for a colour table whose entries may repeat a colour.

use super::{Swatch, SwatchMetrics};
use crate::contract::{
    ActivateOn, Bind, BindMut, Collection, CollectionEvent, CollectionProps, Common, Component, ContainerProps, Cx, LeadOnOpen, NoValue, Outcome,
    Props, Role, SelectMode, ShowCollection as _, Sizing,
};
use crate::list::RowTest;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{DisplayOrder, IconId, ItemSource, Metrics, Selection, TileLayout};
use craft_theme::Srgba;
use egui::{Response, Ui, Vec2};

#[cfg(feature = "testing")]
mod stories;

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
}

impl Outcome for SwatchTableOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A table of touching colour cells, one per entry, wrapping to the width or to a fixed count per row. Entries are
/// told apart by index, so duplicates select one cell. With [`span`](Self::span) and a count per row, the cells share
/// the table's width edge to edge; [`marked`](Self::marked) cells draw the swatch mark.
///
/// - A click or Space selects the cell; a double click or Enter reports `open` and the Enter is the table's, so a
///   dialog around it does not also answer OK.
/// - The table is one Tab stop; the arrows move through the cells as a grid.
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
                .natural_height(true),
            swatches,
            selection: None,
            per_row: None,
            span: false,
            marked: None,
            tips: false,
            wells: false,
            trailing: None,
        }
        .label(label)
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

    /// Whether Shift or Cmd adds cells to the selection.
    #[must_use]
    pub fn multiple(mut self, on: bool) -> Self {
        self.items = self.items.select(if on { SelectMode::Multi } else { SelectMode::Single });
        self
    }

    /// Shows the table.
    pub fn show_all(mut self, ui: &mut Ui) -> SwatchTableOutput {
        let (swatches, tips, wells) = (self.swatches, self.tips, self.wells);
        let marked = self.marked.take();
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
            let cell = cell.cell();
            let cell = if wells { cell.well() } else { cell };
            let cell = if marked.as_ref().is_some_and(|test| test.passes(key)) { cell.marked() } else { cell };
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
        let changed = response.events.iter().any(|event| matches!(event, CollectionEvent::Selection(_)));
        SwatchTableOutput { response: response.response, selected: selection.lead.filter(|_| changed), chosen: selection.items, open }
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
        let cells = self.cells();
        let columns = self.per_row.map_or(cells, |columns| columns.max(1).min(cells));
        // Half a point over, so the division that counts the columns back never rounds below `columns`.
        columns as f32 * self.cell(cx) + 0.5
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> TileLayout {
        let cell = self.cell(cx);
        match self.per_row {
            Some(columns) => {
                let columns = columns.max(1);
                let spans = self.span && width.is_finite() && width > 0.0;
                let tile = if spans { Vec2::new(width / columns as f32, cell) } else { Vec2::splat(cell) };
                TileLayout::new(source, width, tile, 0.0).in_columns(columns)
            }
            None => TileLayout::new(source, width, Vec2::splat(cell), 0.0),
        }
    }
}
