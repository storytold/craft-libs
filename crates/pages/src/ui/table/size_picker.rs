//! `TableSizePicker`: Insert > Table's hover grid. Hover a rows-by-columns square to size the table, click to insert;
//! the label above reads "rows × columns" or Insert Table, and the page shows a live preview of the hovered table
//! (Word).

use craft_behavior::{Metrics, NavShape, PartKey, PartRef, Roving, Sense};
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, GridPlace, Input, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role,
    Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, Flags, FocusLook, Stateful, StyleColor, StyleCx, SurfaceKind, TextLook, TypeRole};
use egui::{CornerRadius, Rect, Response, Vec2, pos2, vec2};

const CELL: PartKey = PartKey::new("cell");

/// The grid's height in squares: 8 rows (Word and PowerPoint).
pub const ROWS: usize = 8;
/// The grid's width in squares: 10 columns.
pub const COLUMNS: usize = 10;

craft_ui::words! {
    /// The picker's words.
    #[non_exhaustive]
    pub struct TableSizePickerWords {
        /// The size under the pointer: rows, then columns.
        size: template "tableSizePicker.size" = "{rows} × {columns}";
    }
}

/// What the picker did: the hovered `(rows, columns)` for the live preview, and the size clicked (the menu closes
/// then).
#[derive(Debug)]
#[non_exhaustive]
pub struct TableSizePickerOutput {
    /// The picker's response.
    pub response: Response,
    /// The `(rows, columns)` under the pointer or the focused square, for a live preview.
    pub hovered: Option<(usize, usize)>,
    /// The `(rows, columns)` clicked or chosen with Enter or Space.
    pub picked: Option<(usize, usize)>,
}

impl Outcome for TableSizePickerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of a table size picker.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TableSizePickerStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    #[style(text)]
    label: TextLook,
    /// The side of a square.
    cell: f32,
    /// The gap between squares.
    gap: f32,
    /// The squares' corner radius.
    radius: CornerRadius,
    /// A square outside the highlighted rectangle: its fill.
    off: Stateful<StyleColor>,
    /// A square inside it: its fill.
    on: Stateful<StyleColor>,
}

/// The picker inside a menu.
///
/// - The pointer hovers a square only on the square itself; the gap between squares hovers none (WordCraft's picker).
/// - Squares up to the hovered one fill with the selected fill; the others are sunken.
/// - Keys: the grid is one Tab stop; the arrows move the hovered square and stop at the grid's edges, Home and End go
///   to the ends of the grid, Space or Enter inserts the hovered size. The grid previews a new table in the
///   interface's flow: under right-to-left the first column is at the right and the arrows follow the screen.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct TableSizePicker {
    #[common]
    common: Common,
    /// The size shown as hovered while no pointer is over the grid and the grid has no focus: a story's or a test's
    /// stand-in for a pointer. Out-of-range sizes are clamped to the grid.
    #[prop]
    hovered: Option<(usize, usize)>,
    /// The square under the pointer or the focus this pass.
    lead: Option<usize>,
    /// The square that was picked this pass.
    picked: Option<usize>,
}

impl TableSizePicker {
    /// A picker named `label`.
    pub fn new(label: impl Into<Label>) -> Self {
        TableSizePicker { common: Common::new(), hovered: None, lead: None, picked: None }.label(label)
    }

    /// The `(rows, columns)` the square at `index` stands for.
    pub fn size_of(index: usize) -> (usize, usize) {
        (index / COLUMNS + 1, index % COLUMNS + 1)
    }

    fn part(index: usize) -> PartRef {
        CELL.at(index as u64)
    }

    /// The grid under the label row: the rect's bottom part.
    fn grid_rect(rect: Rect, style: &TableSizePickerStyle) -> Rect {
        let step = style.cell + style.gap;
        Rect::from_min_size(
            pos2(rect.min.x, rect.max.y - ROWS as f32 * step + style.gap),
            vec2(COLUMNS as f32 * step - style.gap, ROWS as f32 * step - style.gap),
        )
    }

    fn cell_rect(rect: Rect, index: usize, style: &TableSizePickerStyle, rtl: bool) -> Rect {
        let step = style.cell + style.gap;
        let (row, column) = (index / COLUMNS, index % COLUMNS);
        let grid = Self::grid_rect(rect, style);
        let x = if rtl { grid.max.x - style.cell - column as f32 * step } else { grid.min.x + column as f32 * step };
        Rect::from_min_size(pos2(x, grid.min.y + row as f32 * step), Vec2::splat(style.cell))
    }

    /// The label above the grid: the hovered size, or the picker's own name while none is.
    fn text(&self, words: &TableSizePickerWords, cx: &Cx<'_, Self>) -> String {
        match self.lead {
            Some(index) => {
                let (rows, columns) = Self::size_of(index);
                let format = cx.env.format();
                words.size.fill(&[("rows", &format.integer(rows as u64)), ("columns", &format.integer(columns as u64))])
            }
            None => cx.env.resolve(&cx.label()).as_str().to_owned(),
        }
    }

    /// The square the preset size names.
    fn preset_index(&self) -> Option<usize> {
        self.hovered.map(|(rows, columns)| (rows.clamp(1, ROWS) - 1) * COLUMNS + columns.clamp(1, COLUMNS) - 1)
    }
}

impl Component for TableSizePicker {
    const ROLE: Role = Role::GRID;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Both);
    type Style = TableSizePickerStyle;
    type Output = TableSizePickerOutput;

    fn style(cx: &StyleCx<'_>, _: TableSizePickerStyleKey) -> TableSizePickerStyle {
        let colors = &cx.tokens.color;
        let sunken = cx.surface(SurfaceKind::Sunken);
        TableSizePickerStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            label: TextLook::new(TypeRole::Body, Stateful::new(cx.color(colors.text.normal))),
            cell: cx.tokens.size.icon.md,
            gap: cx.tokens.space.gap.tight,
            radius: sunken.radius,
            off: Stateful::new(cx.color(colors.surface.sunken)),
            on: Stateful::new(cx.color(colors.state.selected)),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let step = cx.style.cell + cx.style.gap;
        let label = cx.layout(&TextRun::new(cx.env.resolve(&cx.label()).as_str()).role(cx.style.label.role)).size().y;
        vec2(COLUMNS as f32 * step - cx.style.gap, label + cx.style.gap + ROWS as f32 * step - cx.style.gap)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        for index in 0..ROWS * COLUMNS {
            let part =
                cx.part(Self::part(index), Self::cell_rect(rect, index, cx.style, cx.flow.is_rtl())).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if self.lead == Some(index) {
                part.flags(Flags::CHECKED);
            }
        }
        cx.roving(Roving::new(NavShape::bounded_grid(COLUMNS)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let pointer = (0..ROWS * COLUMNS).find(|&index| cx.hover_pos(Self::part(index)).is_some());
        let focused = cx.active_part().and_then(|part| (0..ROWS * COLUMNS).find(|&index| Self::part(index) == part)).filter(|_| cx.focus_visible());
        self.lead = pointer.or(focused).or_else(|| self.preset_index());
        self.picked = (0..ROWS * COLUMNS).find(|&index| cx.part_activated(Self::part(index)).is_some());
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<TableSizePickerWords>();
        access.value(self.text(&words, cx));
        access.grid(GridPlace::grid(ROWS, COLUMNS));
        for index in 0..ROWS * COLUMNS {
            let (rows, columns) = Self::size_of(index);
            let node = access.node(Self::part(index), Role::GRID_CELL);
            let format = cx.env.format();
            node.name = Name::Text(words.size.fill(&[("rows", &format.integer(rows as u64)), ("columns", &format.integer(columns as u64))]));
            node.grid = GridPlace::cell(rows - 1, columns - 1);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<TableSizePickerWords>();
        let draw = ui.draw();
        let lit = self.lead.map(|index| (index / COLUMNS, index % COLUMNS));
        for index in 0..ROWS * COLUMNS {
            let (row, column) = (index / COLUMNS, index % COLUMNS);
            let chosen = lit.is_some_and(|(r, c)| row <= r && column <= c);
            let look = if chosen { &cx.style.on } else { &cx.style.off };
            draw.fill(&PaintShape::Rect(Self::cell_rect(cx.rect, index, cx.style, cx.flow.is_rtl()), cx.style.radius), cx.resolve(look));
        }
        let label = Rect::from_min_max(cx.rect.min, pos2(cx.rect.max.x, Self::grid_rect(cx.rect, cx.style).min.y));
        draw.text(label, &TextRun::new(self.text(&words, cx)).role(cx.style.label.role).color(cx.resolve(&cx.style.label.color)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> TableSizePickerOutput {
        TableSizePickerOutput { response: cx.response.clone(), hovered: self.lead.map(Self::size_of), picked: self.picked.map(Self::size_of) }
    }
}
