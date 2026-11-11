//! `SwatchPalette`: a wrapping grid of colour cells that chooses one colour. `SwatchTable`: a table of touching cells
//! selected by index, for a colour table whose entries may repeat a colour.

use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Metrics, Mixed, PartKey, Selection, Sense, TileLayout};
use craft_theme::Srgba;
use craft_ui::choice::{Swatch, SwatchMetrics};
use craft_ui::contract::{
    Access, ActivateOn, Bind, BindMut, Collection, CollectionEvent, CollectionProps, Common, Component, ContainerProps, Cx, Held, MeasureCx, NoValue,
    Outcome, PaintUi, PartsCx, Pattern, Props, Role, SelectMode, ShowCollection as _, ShowContainer as _, Sizing, Styled as _,
};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, Flags, FocusLook, StyleCx};
use egui::{Rect, Response, Ui, Vec2};
use std::sync::Arc;

#[cfg(feature = "screen-testing")]
mod stories;

/// The part of a cell that shows the colour.
const WELL: PartKey = PartKey::new("well");

/// What a [`SwatchPalette`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host reads the pick"]
pub struct SwatchPaletteOutput {
    /// The palette's response.
    pub response: Response,
    /// The pick of this pass as one `Set` of the swatch's colour; `None` when nothing was picked. The palette has
    /// already written it to its binding.
    pub edit: Option<EditEvent<Srgba>>,
}

impl Outcome for SwatchPaletteOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A wrapping grid of colour cells from the host's list of named colours that chooses one colour.
///
/// - The swatch equal to the colour (compared in 8 bits, the first when several are) is selected; the name is its
///   tooltip.
/// - A click, Space or Enter on a swatch, or a screen reader's click, sets its colour as one `Set`.
/// - The palette is one Tab stop: Tab enters on the selected swatch (the first when none is) and the next Tab leaves.
///   Arrows move the focus through the cells as the rows lie, Up and Down to the cell above or below; they choose
///   nothing.
/// - AccessKit: a list box named by the label; each cell a list box option named by its name, with the hex colour as
///   its value.
#[must_use = "show the palette with `show`"]
#[derive(Pattern)]
pub struct SwatchPalette<'a> {
    label: Label,
    color: Bind<'a, Srgba>,
    swatches: &'a [(Label, Srgba)],
}

impl<'a> SwatchPalette<'a> {
    /// A palette named `label` of `swatches`, each a name and a colour, choosing the colour bound to `color`.
    pub fn new(label: impl Into<Label>, color: Bind<'a, Srgba>, swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchPalette { label: label.into(), color, swatches }
    }

    /// Shows the palette.
    pub fn show(self, ui: &mut Ui) -> SwatchPaletteOutput {
        let SwatchPalette { label, color, swatches } = self;
        let mut held = Held::new(color);
        let name = label.clone();
        let mut shown = Stack::new().show(ui, |ui, cx| {
            let before = held.read(cx)?;
            let selected = swatches.iter().position(|(_, c)| c.to_rgba() == before.to_rgba());
            let mut current = selected.map_or_else(Selection::default, Selection::single);
            let table = SwatchTable::new(label.clone(), swatches).tips(true).selection(Bind::bound(&mut current)).show_all(ui);
            let picked = table.selected.or(table.open)?;
            let after = swatches.get(picked)?.1;
            if after.to_rgba() == before.to_rgba() {
                return None;
            }
            Some((before, after))
        });
        // Once per pass, after the real run: the picked colour is stored and reported.
        let edit = shown
            .settle(|cx, picked| {
                let (before, after) = picked.take()?;
                held.write(cx, &before, after, name.as_str());
                Some(EditEvent::set(Mixed::Same(before), after, Arc::from(name.as_str()), None))
            })
            .flatten();
        SwatchPaletteOutput { response: shown.output, edit }
    }
}

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

/// The style of a table cell: the swatch's own look is painted as the cell's face.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SwatchCellStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The side of the cell.
    side: f32,
}

/// One cell of a [`SwatchTable`]: a touching, square swatch named by its entry and valued by its hex colour.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct SwatchCell {
    #[common]
    common: Common,
    color: Srgba,
}

impl SwatchCell {
    /// A cell of `color` named `name`.
    pub fn new(name: impl Into<Label>, color: Srgba) -> Self {
        SwatchCell { common: Common::new(), color }.label(name)
    }
}

impl Component for SwatchCell {
    const ROLE: Role = Role::LIST_BOX_OPTION;
    const OVERLAYS: bool = false;
    type Style = SwatchCellStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SwatchCellStyleKey) -> SwatchCellStyle {
        let side = SwatchMetrics::of(cx).cell;
        SwatchCellStyle { metrics: Metrics::new(), focus: cx.focus_look(), side }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let _ = cx.part(WELL, rect);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.value(self.color.to_hex());
        access.color(self.color);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let selected = cx.resolved.selected == Some(true) || cx.state().flags.contains(Flags::SELECTED);
        ui.face(WELL, Swatch::new(cx.resolved.label.to_owned(), self.color).cell().selected(selected));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
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
}

impl Outcome for SwatchTableOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A table of touching colour cells, one per entry, wrapping to the width or to a fixed count per row. Entries are
/// told apart by index, so duplicates select one cell.
///
/// - A click or Space selects the cell; a double click or Enter reports `open` and the Enter is the table's, so a
///   dialog around it does not also answer OK.
/// - The table is one Tab stop; the arrows move through the cells as a grid.
/// - AccessKit: a list box named by the label; each cell a list box option named by its entry, with the hex colour as
///   its value.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
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
    /// Whether each cell's name is its tooltip.
    #[prop]
    tips: bool,
}

impl<'a> SwatchTable<'a> {
    /// A table named `label` of `swatches`, each a name and a colour.
    pub fn new(label: impl Into<Label>, swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchTable {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::FillWidth),
            items: CollectionProps::new().select(SelectMode::Single).activate(ActivateOn::DoubleClick).arrows_select(false).natural_height(true),
            swatches,
            selection: None,
            per_row: None,
            tips: false,
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
    pub fn show_all(self, ui: &mut Ui) -> SwatchTableOutput {
        let (swatches, tips) = (self.swatches, self.tips);
        self.show_items(ui, &SwatchRows::new(swatches), &mut |key, _| {
            let (name, color) = &swatches[*key];
            let cell = SwatchCell::new(name.clone(), *color);
            if tips { cell.tooltip(name.clone()) } else { cell }
        })
    }
}

/// The style of a table: it has no look of its own; its cells paint the swatches.
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
    type Item<'r> = SwatchCell;
    type ItemEvent = std::convert::Infallible;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> TileLayout {
        let cell = SwatchMetrics::of(cx).cell;
        let layout = TileLayout::new(source, width, Vec2::splat(cell), 0.0);
        match self.per_row {
            Some(columns) => layout.in_columns(columns),
            None => layout,
        }
    }
}
