//! Tables: cells and their merges, borders and fills, margins, row heights, captions.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_wordmodel::para::Anchor;
use craft_wordmodel::props::{Align, Border, BorderStyle, Borders, CellProps, HeightRule, RowProps, TableFloat, TableProps, VAlign, VMerge};
use craft_wordmodel::{Block, Cell, Paragraph, Row, Table};

use super::place::dist_pt;
use super::props::{border, border_style, pt, rgb};
use super::text::floor_div;
use super::{Blk, Converter, P};
use crate::header::{BorderFill, num, num_or, truthy};
use crate::units::{round_even, tw};
use crate::xml::Element;

/// The most table rows read (the document model's limit).
const MAX_ROWS: i64 = 32_767;
const MAX_COLS: i64 = 63;
/// Smaller outer margins of tables are left out (see [`Converter::table`]).
const SPACER_MIN: i64 = 1000;

/// A table cell being read.
#[derive(Clone, Debug)]
pub(crate) struct CellInfo<'e> {
    pub el: &'e Element,
    pub row: i64,
    pub col: i64,
    pub rs: i64,
    pub cs: i64,
    pub w: i64,
    pub h: i64,
    pub bf: Option<BorderFill>,
    /// top, left, bottom, right
    pub tmar: Option<[i64; 4]>,
}

/// Each row: its cells, the content height for the row (HWPUNIT), the full row height, header row.
type RowData = (Vec<Cell>, i64, i64, bool);

/// A cell side that isn't drawn, as a DOCX reader takes `w:val="nil"`.
fn no_border() -> Border {
    Border { style: BorderStyle::from_ooxml("nil"), width: 0.0, color: None, space: 0.0 }
}

fn cell_border(b: &crate::header::Border) -> Border {
    border(b).unwrap_or_else(no_border)
}

fn empty_cell(width_twips: i64, span: i64) -> Cell {
    Cell {
        props: CellProps { width: (width_twips > 0).then(|| pt(width_twips)), span: span.clamp(1, 63) as u32, ..CellProps::default() },
        blocks: vec![Arc::new(Block::Para(Paragraph::new()))],
        controls: Default::default(),
    }
}

impl Converter<'_> {
    pub(crate) fn table(&mut self, el: &Element, boxed: bool) -> Vec<Blk> {
        let mut top = self.table_top.take(); // before the cells' paragraphs run
        if self.depth >= super::MAX_NESTING {
            self.missing("nesting too deep");
            return vec![];
        }
        let mut cells: Vec<CellInfo> = Vec::new();
        for tr in el.findall("hp:tr") {
            for tc in tr.findall("hp:tc") {
                let (a, sp, sz) = (tc.find("hp:cellAddr"), tc.find("hp:cellSpan"), tc.find("hp:cellSz"));
                cells.push(CellInfo {
                    el: tc,
                    row: a.map(|a| num(a.get("rowAddr"))).unwrap_or(0).clamp(0, MAX_ROWS),
                    col: a.map(|a| num(a.get("colAddr"))).unwrap_or(0).clamp(0, MAX_COLS),
                    rs: sp.map(|s| num_or(s.get("rowSpan"), 1).max(1)).unwrap_or(1).min(MAX_ROWS),
                    cs: sp.map(|s| num_or(s.get("colSpan"), 1).max(1)).unwrap_or(1).min(MAX_COLS),
                    w: sz.map(|s| num(s.get("width"))).unwrap_or(0),
                    h: sz.map(|s| num(s.get("height"))).unwrap_or(0),
                    bf: None,
                    tmar: None,
                });
            }
        }
        if cells.is_empty() {
            return vec![];
        }
        let rows = num(el.get("rowCnt")).max(cells.iter().map(|c| c.row + c.rs).max().unwrap_or(0)).clamp(0, MAX_ROWS);
        let cols = num(el.get("colCnt")).max(cells.iter().map(|c| c.col + c.cs).max().unwrap_or(0)).clamp(0, MAX_COLS);
        let sz = el.find("hp:sz");
        let table_w = sz.map(|s| num(s.get("width"))).unwrap_or(0);
        let widths = col_widths(&cells, cols, table_w);
        let im = el.find("hp:inMargin");
        let tmar = im.map(|im| [num(im.get("top")), num(im.get("left")), num(im.get("bottom")), num(im.get("right"))]);
        for i in 0..cells.len() {
            let bf = cells.get(i).and_then(|c| self.cell_border_fill(c, el));
            if let Some(c) = cells.get_mut(i) {
                c.bf = bf;
                c.tmar = tmar;
            }
        }
        let origin: HashMap<(i64, i64), usize> = cells.iter().enumerate().map(|(i, c)| ((c.row, c.col), i)).collect();
        let mut covered: HashMap<(i64, i64), usize> = HashMap::new();
        for (i, c) in cells.iter().enumerate() {
            for r in (c.row + 1)..(c.row + c.rs).min(MAX_ROWS) {
                if self.made_cells_left <= 0 {
                    break;
                }
                self.made_cells_left -= 1;
                covered.insert((r, c.col), i);
            }
        }
        let mut props = TableProps { fixed: true, ..TableProps::default() };
        // A table states its own cell margins, or has none (no default table style).
        props.cell_margins = Some(match tmar {
            Some([t, l, b, r]) => [t, l, b, r].map(|v| pt(tw(v)).clamp(0.0, 1584.0)),
            None => [0.0; 4],
        });
        let pos = el.find("hp:pos");
        let para_align = self.table_para_align.take(); // before the cells' paragraphs run
        let mut floating = false;
        match pos.filter(|p| !truthy(p.get("treatAsChar")) && !boxed) {
            Some(pos) => {
                let (align, indent, float) = self.table_position(el, pos, &widths, sz);
                props.align = align;
                props.indent = indent;
                floating = float.is_some();
                props.float = float;
            }
            // A table on a text line goes where its paragraph aligns it (a right-aligned one at
            // the right of the cell or column), as in Hancom's viewer. The model's tables don't
            // follow their paragraph's alignment, so it becomes the table's.
            None if !boxed => {
                props.align = match para_align.as_deref() {
                    Some("RIGHT") => Some(Align::Right),
                    Some("CENTER") => Some(Align::Center),
                    _ => None,
                }
            }
            None => {}
        }
        let repeat = truthy(el.get("repeatHeader"));
        let mut row_data = self.table_rows(&cells, &origin, &covered, &widths, rows, cols);
        // Hancom keeps a table that is a hair taller than the page body on one page (its line
        // runs into the bottom margin); an app would push a sliver onto a new page. Measured
        // on full-page forms: up to 0.5mm over. Trim the tallest row to fit, keeping 700
        // HWPUNIT (2.5mm) free: the smallest slack that kept such forms from spilling their last
        // row (an app rounds rows and draws borders a bit taller).
        let total: i64 = row_data.iter().map(|r| r.2).sum();
        // A table further down the page has only the rest of the body (from the layout
        // cache's position of its paragraph, plus its own offset from that paragraph).
        if let (Some(t), Some(pos)) = (top, pos)
            && pos.get("vertRelTo").unwrap_or("PARA") == "PARA"
        {
            top = Some(t + if !truthy(pos.get("treatAsChar")) { super::signed32(pos.get("vertOffset")).max(0) } else { 0 });
        }
        let limit = self.body_height() - 700 - top.unwrap_or(0);
        fit_rows(&mut row_data, total, limit);
        let mut table =
            Table { props, grid: widths.iter().map(|w| pt(*w).clamp(0.0, 22_000.0)).collect(), rows: Vec::new(), controls: Default::default() };
        let sum: i64 = widths.iter().sum();
        if sum > 0 {
            table.props.width = Some(pt(sum));
        }
        for (cells, height, _, is_header) in row_data {
            let mut rp = RowProps::default();
            if height != 0 {
                rp.height = Some(pt(tw(height)).abs());
                rp.height_rule = HeightRule::AtLeast;
            }
            rp.header = repeat && is_header;
            if !cells.is_empty() {
                table.rows.push(Row { props: rp, cells, controls: Default::default() });
            }
        }
        if table.rows.is_empty() {
            return vec![];
        }
        let cap = self.caption(el);
        let mut out: Vec<Blk> = match cap {
            Some(("TOP", mut c)) => {
                c.push(Blk::Table(Box::new(table)));
                c
            }
            Some((_, c)) => {
                let mut v = vec![Blk::Table(Box::new(table))];
                v.extend(c);
                v
            }
            None => vec![Blk::Table(Box::new(table))],
        };
        if let Some(pos) = pos.filter(|p| !boxed && !floating && !truthy(p.get("treatAsChar"))) {
            // A table in the text flow takes its outer margins (and a downward offset) above
            // and below it (layout cache: next line = offset + top + height + bottom).
            let om = el.find("hp:outMargin");
            let above = super::signed32(pos.get("vertOffset")).max(0) + om.map(|o| num(o.get("top"))).unwrap_or(0);
            let below = om.map(|o| num(o.get("bottom"))).unwrap_or(0);
            let body = self.body_height();
            if (SPACER_MIN..body).contains(&above) {
                out.insert(0, Blk::P(Box::new(P::spacer(above))));
            }
            if (SPACER_MIN..body).contains(&below) {
                out.push(Blk::P(Box::new(P::spacer(below))));
            }
        }
        out
    }

    /// A table's place: its alignment and indent in the text flow, or where it floats.
    fn table_position(
        &mut self,
        el: &Element,
        pos: &Element,
        widths: &[i64],
        sz: Option<&Element>,
    ) -> (Option<Align>, Option<f32>, Option<TableFloat>) {
        let wrap = el.get("textWrap").unwrap_or("TOP_AND_BOTTOM");
        if wrap != "TOP_AND_BOTTOM" || matches!(pos.get("vertRelTo"), Some("PAPER" | "PAGE")) {
            // A table placed on the page (text beside it, or a paper/page position) floats. An
            // app lets text flow beside any floating table, so a "top and bottom" table
            // relative to its paragraph stays in the text flow instead.
            let p = self.placement(el);
            let w: i64 = widths.iter().sum::<i64>() * 5;
            let h = sz.map(|s| num(s.get("height"))).unwrap_or(0);
            let (x, y) = self.offsets(&p, w as f64, h as f64);
            let text = |a: Anchor, flow: Anchor| if matches!(a, Anchor::Page | Anchor::Margin) { a } else { flow };
            let d = p.dist.map(dist_pt);
            let float = TableFloat {
                h_rel: text(p.h_rel, Anchor::Column),
                v_rel: text(p.v_rel, Anchor::Paragraph),
                x: pt(tw(x)).clamp(-31_680.0, 31_680.0),
                y: pt(tw(y)).clamp(-31_680.0, 31_680.0),
                h_align: None,
                v_align: None,
                dist: [d[0], d[2], d[1], d[3]].map(|v| v.clamp(0.0, 1584.0)),
                overlap: true,
            };
            return (None, None, Some(float));
        }
        let align = match pos.get("horzAlign").unwrap_or("") {
            "CENTER" => Some(Align::Center),
            "RIGHT" => Some(Align::Right),
            _ => None,
        };
        let indent = (align.is_none() && num(pos.get("horzOffset")) != 0).then(|| pt(tw(num(pos.get("horzOffset")))));
        (align, indent, None)
    }

    fn table_rows(
        &mut self,
        cells: &[CellInfo],
        origin: &HashMap<(i64, i64), usize>,
        covered: &HashMap<(i64, i64), usize>,
        widths: &[i64],
        rows: i64,
        cols: i64,
    ) -> Vec<RowData> {
        let mut row_data: Vec<RowData> = Vec::new();
        let held: HashSet<i64> = origin.keys().chain(covered.keys()).map(|&(r, _)| r).collect();
        self.depth += 1;
        for r in 0..rows {
            // out of made-up cells: a declared row the file holds no cell of is left out
            if self.made_cells_left <= 0 && !held.contains(&r) {
                self.missing("table rows left out (too many empty cells)");
                continue;
            }
            let (mut tcs, mut c, mut height, mut vpad, mut rule, mut is_header) = (Vec::new(), 0i64, 0i64, 0i64, 0i64, false);
            while c < cols {
                if let Some(&i) = origin.get(&(r, c)) {
                    let Some(cell) = cells.get(i).cloned() else { break };
                    let (made, cell_pad) = self.cell(&cell, widths, if cell.rs > 1 { VMerge::Restart } else { VMerge::None });
                    tcs.push(made);
                    if cell.rs == 1 {
                        height = height.max(cell.h);
                        vpad = vpad.max(cell_pad);
                        rule = rule.max(cell_rule(&cell, r == rows - 1));
                    }
                    is_header = is_header || truthy(cell.el.get("header"));
                    c += cell.cs;
                } else if let Some(&i) = covered.get(&(r, c)) {
                    let Some(cell) = cells.get(i).cloned() else { break };
                    tcs.push(self.cell(&cell, widths, VMerge::Continue).0);
                    c += cell.cs;
                } else {
                    if self.made_cells_left <= 0 {
                        // Out of made-up cells: a gap before a cell the file holds becomes one
                        // empty cell spanning it, so that cell keeps its text and its place (at
                        // most one per real cell); with none after it, the row stays short.
                        let Some(next) = (c + 1..cols).find(|&k| origin.contains_key(&(r, k)) || covered.contains_key(&(r, k))) else { break };
                        let w: i64 = (c..next).map(|k| widths.get(k as usize).copied().unwrap_or(20)).sum();
                        tcs.push(empty_cell(w, next - c));
                        c = next;
                        continue;
                    }
                    self.made_cells_left -= 1;
                    tcs.push(empty_cell(widths.get(c as usize).copied().unwrap_or(20), 1));
                    c += 1;
                }
            }
            // HWP cell heights include the cell's top/bottom margins and borders, but an app
            // adds both on top of the row height (measured), so it gets the content height only.
            row_data.push((tcs, (height - vpad - rule).max(0), height, is_header));
        }
        self.depth -= 1;
        row_data
    }

    /// The cell's own border/fill, overlaid by any hp:cellzone covering it. A zone sets only
    /// what it defines (borders whose type is not NONE, diagonals, fill); measured on a Hancom
    /// document whose header-cell diagonal comes from a zone.
    fn cell_border_fill(&self, c: &CellInfo, tbl: &Element) -> Option<BorderFill> {
        let mut bf = self.h.border_fills.get(&num(c.el.get("borderFillIDRef"))).cloned();
        if let Some(zl) = tbl.find("hp:cellzoneList") {
            for z in zl.findall("hp:cellzone") {
                let inside = num(z.get("startRowAddr")) <= c.row
                    && c.row <= num(z.get("endRowAddr"))
                    && num(z.get("startColAddr")) <= c.col
                    && c.col <= num(z.get("endColAddr"));
                if !inside {
                    continue;
                }
                let Some(zb) = self.h.border_fills.get(&num(z.get("borderFillIDRef"))) else { continue };
                let base = bf.clone().unwrap_or_default();
                let mut n = base.clone();
                if zb.left.visible() {
                    n.left = zb.left.clone();
                }
                if zb.right.visible() {
                    n.right = zb.right.clone();
                }
                if zb.top.visible() {
                    n.top = zb.top.clone();
                }
                if zb.bottom.visible() {
                    n.bottom = zb.bottom.clone();
                }
                n.slash = base.slash || zb.slash;
                n.back_slash = base.back_slash || zb.back_slash;
                n.diagonal = if zb.slash || zb.back_slash { zb.diagonal.clone() } else { base.diagonal.clone() };
                n.fill = zb.fill.clone().or(base.fill.clone());
                bf = Some(n);
            }
        }
        bf
    }

    /// (the cell, its top + bottom margin as written, HWPUNIT)
    fn cell(&mut self, c: &CellInfo, widths: &[i64], vmerge: VMerge) -> (Cell, i64) {
        let tc = c.el;
        let lo = (c.col.max(0) as usize).min(widths.len());
        let hi = ((c.col + c.cs).max(0) as usize).min(widths.len()).max(lo);
        let width: i64 = widths.get(lo..hi).map(|s| s.iter().sum()).unwrap_or(0);
        let mut props = CellProps { width: (width > 0).then(|| pt(width)), span: c.cs.clamp(1, 63) as u32, vmerge, ..CellProps::default() };
        if let Some(bf) = &c.bf {
            props.borders = Some(Borders {
                top: Some(cell_border(&bf.top)),
                left: Some(cell_border(&bf.left)),
                bottom: Some(cell_border(&bf.bottom)),
                right: Some(cell_border(&bf.right)),
                ..Borders::default()
            });
            if let Some(f) = bf.fill.as_deref().filter(|f| *f != "FFFFFF").and_then(rgb) {
                props.shading = Some(f);
            }
        }
        let cm = tc.find("hp:cellMargin");
        let own = cm.is_some() && truthy(tc.get("hasMargin"));
        // top, left, bottom, right
        let mut mar = match (own, cm, c.tmar) {
            (true, Some(cm), _) => [num(cm.get("top")), num(cm.get("left")), num(cm.get("bottom")), num(cm.get("right"))],
            (_, _, Some(t)) => t,
            _ => [0, 0, 0, 0],
        };
        let sub = tc.find("hp:subList");
        let mut made: Vec<Blk> = Vec::new();
        if vmerge != VMerge::Continue {
            let saved = self.text_width.replace((width - tw(mar[1] + mar[3])).max(1));
            made = match sub {
                Some(s) => self.blocks(s.findall("hp:p"), true),
                None => vec![],
            };
            self.text_width = saved;
        }
        // Hancom adds no line spacing after a cell's last line; the model can't drop it from one
        // line, so a multi-line last paragraph gives it back through the bottom margin.
        let tail = match made.last() {
            Some(Blk::P(p)) => p.tail,
            _ => 0,
        };
        if tail != 0 {
            mar[2] = (mar[2] - tail).max(0);
        }
        if own || tail != 0 {
            props.margins = Some(mar.map(|v| pt(tw(v)).clamp(0.0, 1584.0)));
        }
        if let Some(v) = sub.and_then(|s| s.get("vertAlign")) {
            props.valign = match v {
                "TOP" => Some(VAlign::Top),
                "CENTER" => Some(VAlign::Center),
                "BOTTOM" => Some(VAlign::Bottom),
                _ => None,
            };
        }
        let blocks =
            if vmerge == VMerge::Continue { vec![Arc::new(Block::Para(Paragraph::new()))] } else { super::blocks(made, true, &mut self.origins) };
        (Cell { props, blocks, controls: Default::default() }, mar[0] + mar[2])
    }

    /// An object's caption: (side, its paragraphs).
    pub(crate) fn caption(&mut self, el: &Element) -> Option<(&'static str, Vec<Blk>)> {
        let cap = el.find("hp:caption")?;
        let made = match cap.find("hp:subList") {
            Some(sub) => self.blocks(sub.findall("hp:p"), false),
            None => vec![],
        };
        Some((if cap.get("side") == Some("TOP") { "TOP" } else { "BOTTOM" }, made))
    }
}

fn col_widths(cells: &[CellInfo], cols: i64, table_w: i64) -> Vec<i64> {
    let n = cols.max(0) as usize;
    let mut w: Vec<Option<i64>> = vec![None; n];
    for c in cells {
        if c.cs == 1
            && c.col < cols
            && c.w > 0
            && let Some(slot) = w.get_mut(c.col as usize)
            && slot.is_none()
        {
            *slot = Some(c.w);
        }
    }
    let mut changed = true;
    let mut guard = 0;
    while changed && guard < 10_000 {
        changed = false;
        guard += 1;
        for c in cells {
            let span = (c.col.max(0) as usize)..((c.col + c.cs).min(cols).max(0) as usize);
            let unknown: Vec<usize> = span.clone().filter(|i| w.get(*i).is_some_and(|x| x.is_none())).collect();
            if unknown.len() == 1 && c.w > 0 {
                let known: i64 = span.filter_map(|i| w.get(i).copied().flatten()).sum();
                if let Some(slot) = unknown.first().and_then(|&u| w.get_mut(u)) {
                    *slot = Some((c.w - known).max(200));
                    changed = true;
                }
            }
        }
    }
    let rest: Vec<usize> = w.iter().enumerate().filter(|(_, x)| x.is_none()).map(|(i, _)| i).collect();
    if !rest.is_empty() {
        let known: i64 = w.iter().flatten().sum();
        let each = if table_w != 0 { floor_div(table_w - known, rest.len() as i64).max(2000) } else { 5000 };
        for i in rest {
            if let Some(slot) = w.get_mut(i) {
                *slot = Some(each);
            }
        }
    }
    w.into_iter().map(|x| tw(x.unwrap_or(0)).max(20)).collect()
}

/// Trim the first of the tallest rows by what the table is over `limit` (see table()).
fn fit_rows(row_data: &mut [RowData], total: i64, limit: i64) {
    if !(0 < total - limit && (total - limit) as f64 <= total as f64 * 0.05) {
        return;
    }
    // the first of the tallest rows
    let mut best: Option<usize> = None;
    for (i, rd) in row_data.iter().enumerate() {
        if best.and_then(|b| row_data.get(b)).is_none_or(|b| rd.1 > b.1) {
            best = Some(i);
        }
    }
    if let Some(rd) = best.and_then(|b| row_data.get_mut(b)) {
        rd.1 = (rd.1 - (total - limit)).max(0);
    }
}

/// HWPUNIT thickness of the borders an app adds to a row: top (+ bottom on the last row).
fn cell_rule(c: &CellInfo, last_row: bool) -> i64 {
    let Some(bf) = &c.bf else { return 0 };
    let mut sides = vec![&bf.top];
    if last_row {
        sides.push(&bf.bottom);
    }
    let mm: f64 = sides.iter().filter(|b| border_style(&b.kind).is_some()).map(|b| b.width_mm).sum();
    round_even(mm * 7200.0 / 25.4)
}
