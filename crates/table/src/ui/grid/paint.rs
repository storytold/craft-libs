//! The grid's style, its own parts, and the paint over the rows: headers, the range outline, the fill handle, the copy
//! marks, the in-cell editor and the chrome that sits on the sheet.

use craft_behavior::{
    CursorIcon, HitLevel, HitPriority, Metrics, PartKey, Positions, Proposal, RowAt, ScrollPos, SelectionModel as _, Sense, TextPos,
};
use craft_theme::ControlSize;
use craft_ui::contract::{Access, Cx, Focus as _, Name, PaintUi, PartsCx, Remember as _, Role, Styled as _};
use craft_ui::primitives::{
    Dashes, Draw, EditorLine, EditorPurpose, EditorRun, EditorState, HAlign, PaintShape, RectSide, RuleKind, StrokeWidth, TextRun,
};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::motion::Animate as _;
use craft_ui::style::{Flags, State, StateMix, StateSet, Stateful, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Pos2, Rect, Vec2};

use super::editor::{EditState, Mode};
use super::row::CellPaint;
use super::{
    CellAt, CellGrid, CellGridStyle, CellGridStyleKey, CellGridWords, CellRange, CellSelection, ColumnFilter, GridLayout, SheetGeometry, SheetMetrics,
};
use super::{edges, header_insets};
use crate::ui::error_tag::ErrorTag;
use crate::ui::filter::FilterButton;
use crate::ui::outline::{self, OutlineBand};

pub(super) const EDITOR: PartKey = PartKey::new("editor");
pub(super) const COLUMN: PartKey = PartKey::new("column_header");
pub(super) const ROW: PartKey = PartKey::new("row_header");
pub(super) const CORNER: PartKey = PartKey::new("corner");
pub(super) const COLUMN_EDGE: PartKey = PartKey::new("column_edge");
pub(super) const ROW_EDGE: PartKey = PartKey::new("row_edge");
pub(super) const FILL: PartKey = PartKey::new("fill_handle");
pub(super) const BORDER: PartKey = PartKey::new("range_border");

/// Seconds the marching ants take for one cycle, and how often they step.
const ANT_CYCLE: f64 = 1.0;
const ANT_STEP: f64 = 0.12;

/// What `parts` worked out for the pass: the geometry at the scroll the item step uses, and the lines it shows.
#[derive(Clone)]
pub(super) struct Frame {
    pub(super) geometry: SheetGeometry,
    pub(super) columns: Vec<u32>,
    pub(super) rows: Vec<u32>,
    chrome: Chrome,
}

/// Where the chrome over the cells shows this pass: each filter button in view (its index in the grid's filters, its
/// rect and the pane its header cell is in) and the error tag.
#[derive(Clone, Default)]
struct Chrome {
    filters: Vec<(usize, Rect, Rect)>,
    tag: Option<Rect>,
}

pub(super) fn style(cx: &StyleCx<'_>, key: CellGridStyleKey) -> CellGridStyle {
    let control = cx.control();
    // Headers are a small control high at every density, as a spreadsheet's are.
    let sheet = SheetMetrics::new(cx.control_of(ControlSize::Sm).height, cx.text_width(TypeRole::Label, "0"), control.pad_x);
    let zoom = f32::from(key.percent) / 100.0;
    let bands = outline::bands(key.levels, key.headings, zoom, cx.tokens.size.icon.sm, cx.tokens.space.gap.tight);
    let c = &cx.tokens.color;
    CellGridStyle {
        surface: cx.text_well(),
        metrics: Metrics::new().padding(header_insets(sheet, key.headings, zoom, u32::from(key.digits), bands)),
        sheet,
        bands,
        background: cx.color(c.surface.background),
        header: Stateful::new(cx.color(c.surface.chrome)).when(Flags::SELECTED, cx.color(c.state.selected)),
        header_text: Stateful::new(cx.color(c.text.muted)).when(Flags::SELECTED, cx.color(c.text.on_selected)),
        freeze_line: cx.color(c.line.border),
        accent: cx.color(c.accent.solid),
        accent_bar: cx.tokens.stroke.thick,
        preview: cx.color(c.text.muted),
        tint: super::row::cell_tint(cx),
        handle_fill: cx.color(c.accent.solid),
        handle_ring: cx.color(c.surface.background),
        edit_fill: cx.color(c.surface.raised),
        edit_edge: cx.color(c.line.focus),
        handle: control.pad_x,
        mark: control.gap,
        pad: control.pad_x,
        border_reach: cx.tokens.size.hit.slop,
        inset: cx.tokens.space.gap.tight,
        gap: control.gap,
    }
}

/// The lines `first..=last` that show, hidden ones left out.
fn shown(lines: &craft_behavior::LineIndex, first: u32, last: u32) -> impl Iterator<Item = u32> + '_ {
    (first..=last.min(lines.count().saturating_sub(1))).filter(|&line| lines.shown(line))
}

/// Which lines the ranges of a selection mark, and which they mark whole.
#[derive(Default)]
struct Marks {
    columns: Vec<(u32, u32, bool)>,
    rows: Vec<(u32, u32, bool)>,
}

impl Marks {
    fn of(selection: &CellSelection<u32>, rows: u32, columns: u32) -> Marks {
        let mut marks = Marks::default();
        for range in selection.ranges() {
            let (top, bottom, left, right) = edges(range);
            marks.columns.push((left, right, top == 0 && bottom + 1 >= rows));
            marks.rows.push((top, bottom, left == 0 && right + 1 >= columns));
        }
        marks
    }

    /// `None` for an unmarked line, `Some(true)` for one inside a range that selects every line across.
    fn of_line(marks: &[(u32, u32, bool)], line: u32) -> Option<bool> {
        marks.iter().filter(|&&(first, last, _)| (first..=last).contains(&line)).map(|&(.., whole)| whole).reduce(|a, b| a || b)
    }
}

impl CellGrid<'_> {
    /// The geometry at this pass's scroll.
    fn geometry_for(&self, rect: Rect, offset: ScrollPos, rtl: bool, style: &CellGridStyle) -> SheetGeometry {
        let zoom = f64::from(self.zoom);
        let scroll = Vec2::new((offset.x / zoom) as f32, (offset.y / zoom) as f32);
        let (rows, columns) = self.model.frozen();
        let layout = GridLayout::new(rect, self.model.rows().clone(), self.model.columns().clone())
            .zoom(self.zoom)
            .frozen(rows, columns)
            .scroll(scroll)
            .headings(self.headings)
            .bands(style.bands)
            .right_to_left(rtl)
            .digits(u32::from(self.digits));
        SheetGeometry::new(layout, style.sheet)
    }

    /// The column header cell of `column` in screen coordinates, kept to the columns' side of the frozen split.
    pub(super) fn column_header(geometry: &SheetGeometry, column: u32) -> Rect {
        let cells = geometry.cells();
        let x = geometry.x(column);
        let width = geometry.columns().size(column) as f32 * geometry.zoom();
        let floor = if column < geometry.frozen().1 { cells.left() } else { geometry.split().x };
        let left = x.max(floor);
        let right = (x + width).min(cells.right()).max(left);
        let strip = Rect::from_min_max(Pos2::new(left, cells.top() - geometry.header_size().y), Pos2::new(right, cells.top()));
        geometry.mirror_rect(strip)
    }

    /// The row header cell of `row` in screen coordinates, kept to the rows' side of the frozen split.
    pub(super) fn row_header(geometry: &SheetGeometry, row: u32) -> Rect {
        let cells = geometry.cells();
        let y = geometry.y(row);
        let height = geometry.rows().size(row) as f32 * geometry.zoom();
        let floor = if row < geometry.frozen().0 { cells.top() } else { geometry.split().y };
        let top = y.max(floor);
        let bottom = (y + height).min(cells.bottom()).max(top);
        let strip = Rect::from_min_max(Pos2::new(cells.left() - geometry.header_size().x, top), Pos2::new(cells.left(), bottom));
        geometry.mirror_rect(strip)
    }

    /// The columns and rows the geometry shows, hidden lines left out.
    fn visible_lines(geometry: &SheetGeometry) -> (Vec<u32>, Vec<u32>) {
        let (frozen_rows, frozen_columns) = geometry.frozen();
        let ((first_row, last_row), (first_column, last_column)) = geometry.visible();
        let frozen =
            |lines: &craft_behavior::LineIndex, count: u32| shown(lines, 0, count.saturating_sub(1)).filter(move |_| count > 0).collect::<Vec<_>>();
        let mut columns = frozen(geometry.columns(), frozen_columns);
        columns.extend(shown(geometry.columns(), first_column, last_column));
        let mut rows = frozen(geometry.rows(), frozen_rows);
        rows.extend(shown(geometry.rows(), first_row, last_row));
        (columns, rows)
    }

    /// The fill handle's rect at the end-bottom corner of `outline` (screen coordinates).
    pub(super) fn handle_rect(geometry: &SheetGeometry, outline: Rect, size: f32) -> Rect {
        let corner = if geometry.right_to_left() { outline.left_bottom() } else { outline.right_bottom() };
        Rect::from_center_size(corner, Vec2::splat(size))
    }

    /// Declares the grid's own parts: header cells, edges, corner, fill handle, range border and the editor.
    pub(super) fn declare_parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let geometry = self.geometry_for(rect, cx.offset(), cx.flow.is_rtl(), cx.style);
        let (columns, rows) = Self::visible_lines(&geometry);
        let sense = Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK | Sense::TAKE_FOCUS;
        let reach = craft_behavior::hit::LINE_REACH;
        let rtl = geometry.right_to_left();
        let marks = Marks::of(&self.snapshot, geometry.rows().count(), geometry.columns().count());
        let flags = |held: Option<bool>| if held.is_some() { Flags::SELECTED } else { Flags::empty() };
        if self.headings {
            let (cells, header) = (geometry.cells(), geometry.header_size());
            let corner = Rect::from_min_max(Pos2::new(cells.left() - header.x, cells.top() - header.y), cells.min);
            cx.part(CORNER, geometry.mirror_rect(corner)).sense(Sense::CLICK | Sense::TAKE_FOCUS).over().pointer_only("Select all is Cmd+A");
            for &column in &columns {
                let header = Self::column_header(&geometry, column);
                cx.part(COLUMN.at(u64::from(column)), header)
                    .sense(sense)
                    .over()
                    .flags(flags(Marks::of_line(&marks.columns, column)))
                    .pointer_only("Select the column is Cmd+Space");
                let end = if rtl { header.left_center() } else { header.right_center() };
                let edge = Rect::from_center_size(end, Vec2::new(reach * 2.0, header.height()));
                cx.part(COLUMN_EDGE.at(u64::from(column)), edge)
                    .sense(Sense::CLICK | Sense::DRAG)
                    .priority(HitPriority::new(HitLevel::Body, 1))
                    .cursor(CursorIcon::ResizeColumn)
                    .over()
                    .pointer_only("Column width is set from the app's menu");
            }
            for &row in &rows {
                let header = Self::row_header(&geometry, row);
                cx.part(ROW.at(u64::from(row)), header)
                    .sense(sense)
                    .flags(flags(Marks::of_line(&marks.rows, row)))
                    .over()
                    .pointer_only("Select the row is Shift+Space");
                let edge = Rect::from_center_size(header.center_bottom(), Vec2::new(header.width(), reach * 2.0));
                cx.part(ROW_EDGE.at(u64::from(row)), edge)
                    .sense(Sense::CLICK | Sense::DRAG)
                    .priority(HitPriority::new(HitLevel::Body, 1))
                    .cursor(CursorIcon::ResizeVertical)
                    .over()
                    .pointer_only("Row height is set from the app's menu");
            }
        }
        let selection = self.snapshot.clone();
        if let Some(range) = selection.current() {
            let outline = geometry.mirror_rect(geometry.range_rect(range));
            let handle = Self::handle_rect(&geometry, outline, cx.style.handle);
            if geometry.mirror_rect(geometry.cells()).contains_rect(handle) {
                cx.part(FILL, handle)
                    .sense(Sense::DRAG)
                    .priority(HitPriority::new(HitLevel::Body, 2))
                    .cursor(CursorIcon::Crosshair)
                    .over()
                    .pointer_only("Fill is Cmd+D and Cmd+R in the app's menu");
            }
            let slop = cx.style.border_reach;
            let sides = [
                Rect::from_min_max(Pos2::new(outline.left(), outline.top() - slop), Pos2::new(outline.right(), outline.top() + slop)),
                Rect::from_min_max(Pos2::new(outline.left(), outline.bottom() - slop), Pos2::new(outline.right(), outline.bottom() + slop)),
                Rect::from_min_max(Pos2::new(outline.left() - slop, outline.top()), Pos2::new(outline.left() + slop, outline.bottom())),
                Rect::from_min_max(Pos2::new(outline.right() - slop, outline.top()), Pos2::new(outline.right() + slop, outline.bottom())),
            ];
            for (at, side) in sides.into_iter().enumerate() {
                cx.part(BORDER.at(at as u64), side)
                    .sense(Sense::CLICK | Sense::DRAG)
                    .priority(HitPriority::new(HitLevel::Body, 1))
                    .cursor(CursorIcon::Move)
                    .over()
                    .pointer_only("Moving cells is Cut and Paste");
            }
        }
        let lead = selection.active().copied().unwrap_or(CellAt::new(0, 0));
        let mut at = geometry.mirror_rect(geometry.cell_rect(lead));
        if let Some(row) = cx.rect_of(&lead.row) {
            at = Rect::from_x_y_ranges(at.x_range(), row.y_range());
        }
        cx.editor(EDITOR, at, EditorPurpose::Text);
        let chrome = self.place_chrome(&geometry, &columns, &rows, cx);
        self.frame = Some(Frame { geometry, columns, rows, chrome });
    }

    /// The filter button of `filter`, named by its column's header text.
    fn filter_button(&self, filter: &ColumnFilter) -> FilterButton {
        let button = FilterButton::new(self.model.cell_text(filter.cell)).filtered(filter.filtered);
        match filter.sort.clone() {
            Some(sort) => button.sort(sort),
            None => button,
        }
    }

    /// Places the chrome at its own sizes: each filter button in view at the end of its header cell, `style.inset` in
    /// and centred on the cell's height (no taller than the cell), and the error tag `style.inset` before its cell, or
    /// after it where the tag would leave the cells.
    fn place_chrome(&self, geometry: &SheetGeometry, columns: &[u32], rows: &[u32], cx: &PartsCx<'_, Self>) -> Chrome {
        let (rtl, inset) = (geometry.right_to_left(), cx.style.inset);
        let mut chrome = Chrome::default();
        for (at, filter) in self.filters.iter().enumerate() {
            let cell = filter.cell;
            if !rows.contains(&cell.row) || !columns.contains(&cell.column) {
                continue;
            }
            let Some(pane) = geometry.range_panes(&CellRange::cell(cell)).first().map(|pane| geometry.mirror_rect(*pane)) else { continue };
            let rect = geometry.mirror_rect(geometry.cell_rect(cell));
            let size = cx.child(filter_key(cell), &mut self.filter_button(filter), Proposal::Ideal);
            let size = Vec2::new(size.x, size.y.min(rect.height()));
            let x = if rtl { rect.left() + inset } else { rect.right() - inset - size.x };
            chrome.filters.push((at, Rect::from_min_size(Pos2::new(x, rect.center().y - size.y / 2.0), size), pane));
        }
        if let Some(problem) = &self.error_tag {
            let size = cx.child(TAG_KEY, &mut ErrorTag::new(problem.problem.clone(), problem.fixes), Proposal::Ideal);
            let cell = geometry.cell_rect(problem.cell);
            let before = cell.left() - size.x - inset;
            let left = if before >= geometry.cells().left() { before } else { cell.right() + inset };
            chrome.tag = Some(geometry.mirror_rect(Rect::from_min_size(Pos2::new(left, cell.top()), size)));
        }
        chrome
    }

    /// The facts AccessKit gets: the sheet's full size and the headers as sub-nodes.
    pub(super) fn describe(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(frame) = &self.frame else { return };
        let words = cx.env.words::<CellGridWords>();
        access.grid(craft_ui::contract::GridPlace::grid(self.model.rows().count() as usize, self.model.columns().count() as usize));
        for &column in &frame.columns {
            let name = words.column_header.fill(&[("name", &self.model.column_label(column))]);
            access.node(COLUMN.at(u64::from(column)), Role::COLUMN_HEADER).name = Name::Text(name);
        }
        for &row in &frame.rows {
            let name = words.row_header.fill(&[("n", &(row + 1).to_string())]);
            access.node(ROW.at(u64::from(row)), Role::ROW_HEADER).name = Name::Text(name);
        }
    }

    /// Paints headers, the range outline and handle, the copy marks and the open cell, and shows the chrome.
    pub(super) fn paint_sheet(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(frame) = self.frame.clone() else { return };
        let geometry = &frame.geometry;
        let style = cx.style;
        let selection = cx.collection_selection().cloned().unwrap_or_else(|| self.snapshot.clone());
        let editing = cx.recall::<EditState>();
        let open = editing.mode != Mode::Ready;
        let preview = cx.recall::<super::input::Preview>();
        self.paint_merges(ui, geometry, &selection, cx);
        {
            let draw = ui.draw();
            let rect = |shape: Rect| PaintShape::Rect(shape, CornerRadius::ZERO);
            let outline = |range: &CellRange<u32>| geometry.mirror_rect(geometry.range_rect(range));
            if let Some(current) = selection.current() {
                draw.stroke(&rect(outline(current)), StrokeWidth::Medium, style.accent);
                if selection.ranges().len() > 1
                    && let Some(active) = selection.active()
                {
                    draw.stroke(&rect(outline(&CellRange::cell(*active))), StrokeWidth::Thin, style.accent);
                }
                if !open {
                    let handle = Self::handle_rect(geometry, outline(current), style.handle);
                    draw.fill(&rect(handle), style.handle_fill);
                    draw.stroke(&rect(handle), StrokeWidth::Hairline, style.handle_ring);
                }
            }
            for range in self.copy_mark {
                match cx.phase(ANT_CYCLE, Some(ANT_STEP)) {
                    Some(phase) => {
                        let pattern = Dashes::even(style.mark).phase(phase * style.mark * 2.0);
                        draw.dashed(&corners(outline(range)), pattern, StrokeWidth::Medium, style.accent);
                    }
                    None => draw.stroke(&rect(outline(range)), StrokeWidth::Thin, style.accent),
                }
            }
            if let Some(range) = preview.range() {
                draw.dashed(&corners(outline(&range)), Dashes::even(style.mark), StrokeWidth::Medium, style.preview);
            }
            let (frozen_rows, frozen_columns) = geometry.frozen();
            let (cells, split) = (geometry.cells(), geometry.split());
            if frozen_columns > 0 {
                let x = geometry.mirror_x(split.x);
                draw.stroke(&PaintShape::Path(vec![Pos2::new(x, cells.top()), Pos2::new(x, cells.bottom())]), StrokeWidth::Medium, style.freeze_line);
            }
            if frozen_rows > 0 {
                draw.stroke(
                    &PaintShape::Path(vec![Pos2::new(cells.left(), split.y), Pos2::new(cells.right(), split.y)]),
                    StrokeWidth::Medium,
                    style.freeze_line,
                );
            }
            if self.headings {
                self.paint_headers(&draw, &frame, &selection, cx);
            }
            match editing.cell {
                Some(cell) if open => self.paint_editor(&draw, cx, &editing, geometry.mirror_rect(geometry.cell_rect(cell))),
                _ => {
                    let lead = selection.active().map_or(Rect::NOTHING, |cell| geometry.mirror_rect(geometry.cell_rect(*cell)));
                    cx.editor_state(EditorState::new(Rect::from_min_size(lead.min, Vec2::new(0.0, lead.height())), &[], TextPos::new(0, 0), 0));
                }
            }
        }
        self.show_chrome(ui, &frame, cx.env.words::<CellGridWords>().outline.clone());
    }

    /// Paints the merged cells in view, each whole over the rows it spans and cut to each pane it shows in, as a row
    /// paints its cells. A merged cell takes the flags the item step gives a row's cells: selected when the selection
    /// holds its first cell, lead when it holds the active cell while the grid has focus.
    fn paint_merges(&self, ui: &mut PaintUi<'_>, geometry: &SheetGeometry, selection: &CellSelection<u32>, cx: &Cx<'_, Self>) {
        let cells =
            CellPaint { model: self.model, background: cx.style.background, tint: &cx.style.tint, zoom: self.zoom, rtl: geometry.right_to_left() };
        let focused = cx.is_focused();
        let order = Positions::new(self.model.rows().count());
        let shown = geometry.mirror_rect(geometry.cells());
        for merge in self.model.merges() {
            let rect = geometry.mirror_rect(geometry.range_rect(merge));
            if !rect.intersects(shown) {
                continue;
            }
            let (top, bottom, left, right) = edges(merge);
            let held = |cell: &CellAt<u32>| (top..=bottom).contains(&cell.row) && (left..=right).contains(&cell.column);
            let mut flags = Flags::empty();
            flags.set(Flags::SELECTED, selection.contains_cell(RowAt::new(&top, top as usize, &order), left));
            flags.set(Flags::LEAD, focused && selection.active().is_some_and(held));
            let state = StateSet::new(State::Rest, flags, StateMix::REST);
            for pane in geometry.range_panes(merge) {
                ui.clipped(geometry.mirror_rect(pane), |ui| cells.paint(ui, CellAt::new(top, left), rect, &state));
            }
        }
    }

    fn paint_headers(&self, draw: &Draw<'_>, frame: &Frame, selection: &CellSelection<u32>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let geometry = &frame.geometry;
        let rect = |shape: Rect| PaintShape::Rect(shape, CornerRadius::ZERO);
        let (cells, header) = (geometry.cells(), geometry.header_size());
        let top = Rect::from_min_max(Pos2::new(cells.left(), cells.top() - header.y), Pos2::new(cells.right(), cells.top()));
        let left = Rect::from_min_max(Pos2::new(cells.left() - header.x, cells.top()), Pos2::new(cells.left(), cells.bottom()));
        let corner = Rect::from_min_max(Pos2::new(cells.left() - header.x, cells.top() - header.y), cells.min);
        let plain = cx.resolve_part(CORNER, &style.header);
        for area in [top, left, corner] {
            draw.fill(&rect(geometry.mirror_rect(area)), plain);
        }
        let marks = Marks::of(selection, geometry.rows().count(), geometry.columns().count());
        let rtl = geometry.right_to_left();
        let line = |from: Pos2, to: Pos2| draw.rule(RuleKind::Edge, from, to);
        let bar = style.accent_bar;
        let look = |part: craft_behavior::PartRef| (cx.resolve_part(part, &style.header), cx.resolve_part(part, &style.header_text));
        for &column in &frame.columns {
            let at = Self::column_header(geometry, column);
            let state = Marks::of_line(&marks.columns, column);
            let (fill, text) = look(COLUMN.at(u64::from(column)));
            draw.fill(&rect(at), fill);
            // A column cut by the cells' edge leaves a strip narrower than its letters; they then stay unpainted, as a
            // cut row's number does.
            let letters = TextRun::new(self.model.column_label(column)).color(text).align(HAlign::Center).elide();
            if draw.layout(&letters, f32::INFINITY).size().x <= at.width() {
                draw.text(at, &letters);
            }
            if state.is_some() {
                draw.fill(&rect(Rect::from_x_y_ranges(at.x_range(), at.bottom() - bar..=at.bottom())), style.accent);
            }
            draw.edge_rule(RuleKind::Edge, at, RectSide::end(rtl));
            let end = if rtl { at.left() } else { at.right() };
            if column + 1 < geometry.columns().count() && !geometry.columns().shown(column + 1) {
                let mark = if rtl { end - style.gap } else { end + style.gap };
                line(Pos2::new(mark, at.top()), Pos2::new(mark, at.bottom()));
            }
        }
        for &row in &frame.rows {
            let at = Self::row_header(geometry, row);
            let state = Marks::of_line(&marks.rows, row);
            let (fill, text) = look(ROW.at(u64::from(row)));
            draw.fill(&rect(at), fill);
            // A row cut by the cells' edge leaves a strip shorter than its number; the number then stays unpainted
            // instead of running past the grid (the row header keeps its access name).
            let number = TextRun::new((row + 1).to_string()).color(text).align(HAlign::Center).elide();
            if draw.layout(&number, at.width()).size().y <= at.height() {
                draw.text(at, &number);
            }
            if state.is_some() {
                let across = if rtl { at.left()..=at.left() + bar } else { at.right() - bar..=at.right() };
                draw.fill(&rect(Rect::from_x_y_ranges(across, at.y_range())), style.accent);
            }
            draw.edge_rule(RuleKind::Edge, at, RectSide::Bottom);
            if row + 1 < geometry.rows().count() && !geometry.rows().shown(row + 1) {
                line(Pos2::new(at.left(), at.bottom() + style.gap), Pos2::new(at.right(), at.bottom() + style.gap));
            }
        }
        // The headers' edges toward the cells, across the corner too.
        draw.edge_rule(RuleKind::Edge, geometry.mirror_rect(top.union(corner)), RectSide::Bottom);
        draw.edge_rule(RuleKind::Edge, geometry.mirror_rect(left.union(corner)), RectSide::end(rtl));
        Self::paint_corner_mark(draw, geometry.mirror_rect(corner), style.gap, rtl, cx.resolve_part(CORNER, &style.header_text));
    }

    /// The select-all mark: a right triangle in the end-bottom of the corner `corner`, `inset` in from its edges.
    fn paint_corner_mark(draw: &Draw<'_>, corner: Rect, inset: f32, rtl: bool, color: StyleColor) {
        let mark = corner.shrink(inset);
        let side = mark.width().min(mark.height());
        if side <= 0.0 {
            return;
        }
        let (end, start) = if rtl { (mark.left(), mark.left() + side) } else { (mark.right(), mark.right() - side) };
        // Clockwise on screen, as a filled polygon wants.
        let mut points = vec![Pos2::new(end, mark.bottom() - side), Pos2::new(end, mark.bottom()), Pos2::new(start, mark.bottom())];
        if rtl {
            points.reverse();
        }
        draw.fill(&PaintShape::Polygon(points), color);
    }

    /// The open cell: its fill, edge, draft text and caret, and the editor part's report.
    fn paint_editor(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, editing: &EditState, cell: Rect) {
        let style = cx.style;
        let shape = PaintShape::Rect(cell, CornerRadius::ZERO);
        draw.fill(&shape, style.edit_fill);
        draw.stroke(&shape, StrokeWidth::Medium, style.edit_edge);
        let inner = craft_behavior::Flow::new(cell, cx.env.direction().is_rtl()).inset(craft_behavior::Insets::horizontal(style.pad));
        let text = editing.draft.shown();
        let layout = draw.layout(&TextRun::new(text.clone()), inner.width());
        let line = Rect::from_min_size(inner.min, Vec2::new(inner.width(), layout.size().y));
        draw.galley(line.min, &layout);
        let runs = layout.editor_runs();
        let at = TextPos::new(0, editing.draft.caret_shown());
        let top = line.min + Vec2::new(caret_x(&runs, &text, at.byte), 0.0);
        if cx.is_focused() {
            draw.stroke(&PaintShape::Path(vec![top, top + Vec2::new(0.0, line.height())]), StrokeWidth::Medium, style.edit_edge);
        }
        let lines = [EditorLine::new(&text, line, line.top() + layout.first_baseline, &runs)];
        let mut state = EditorState::new(Rect::from_min_size(top, Vec2::new(0.0, line.height())), &lines, at, editing.draft.revision());
        state.selection = editing.draft.selection();
        cx.editor_state(state);
    }

    /// Shows the filter buttons, the error tag and the outline band over the sheet.
    fn show_chrome(&mut self, ui: &mut PaintUi<'_>, frame: &Frame, outline: Label) {
        let (geometry, filters) = (&frame.geometry, self.filters);
        for &(at, rect, pane) in &frame.chrome.filters {
            let filter = &filters[at];
            let button = self.filter_button(filter);
            let out = ui.clipped(pane, |ui| ui.child(filter_key(filter.cell), rect, button));
            if let Some(event) = out.event {
                self.pass.filter = Some((filter.cell, event));
            }
        }
        if let (Some(problem), Some(tag)) = (&self.error_tag, frame.chrome.tag)
            && geometry.mirror_rect(geometry.cells()).intersects(tag)
        {
            let out = ui.child(TAG_KEY, tag, ErrorTag::new(problem.problem.clone(), problem.fixes));
            self.pass.fix = out.fix.or(self.pass.fix);
        }
        if let Some((rows, columns)) = self.outline
            && self.levels != (0, 0)
        {
            let out = ui.child("outline", OutlineBand::area(geometry), OutlineBand::new(outline, rows, columns, geometry));
            self.pass.outline.extend(out.actions);
        }
    }
}

/// The child key of the filter button in the header cell `cell`.
fn filter_key(cell: CellAt<u32>) -> (&'static str, u32, u32) {
    ("filter", cell.row, cell.column)
}

/// The child key of the error tag.
const TAG_KEY: &str = "error_tag";

/// The closed outline of `rect`, as a path that starts and ends at its top-left corner.
fn corners(rect: Rect) -> [Pos2; 5] {
    [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()]
}

/// The x offset of byte `at` of `text` in the laid-out runs.
fn caret_x(runs: &[EditorRun<'_>], text: &str, at: usize) -> f32 {
    for run in runs {
        if run.bytes.contains(&at) || run.bytes.end == at {
            let index = text[run.bytes.start..at.min(text.len())].chars().count();
            return run.positions.get(index).copied().unwrap_or_else(|| run.positions.last().zip(run.widths.last()).map_or(0.0, |(p, w)| p + w));
        }
    }
    0.0
}
