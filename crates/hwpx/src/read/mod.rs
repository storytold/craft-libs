//! HWPX sections into a `craft-wordmodel` document.
//!
//! The layout rules here were found by comparing Hancom's viewer with an app laying out the
//! same model on real documents; each one says what was observed. Hancom lays text out in its
//! own way (line heights, column breaks, page ends): the reader keeps what its layout cache
//! (`hp:linesegarray`) says where the model has a place for it.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

mod background;
mod drawing;
mod media;
mod place;
mod props;
mod section;
mod table;
mod text;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use craft_wordmodel::para::NoteKind;
use craft_wordmodel::props::{CharProps, LineSpacing, ParaProps};
use craft_wordmodel::{Block, Blocks, Comment, Document, InlineObject, Paragraph, PartKind, Run, SectionProps, Table};

use crate::error::Result;
use crate::header::{Header, ParaPr, num, num_or, truthy};
use crate::package::Package;
use crate::units::tw;
use crate::xml::{Element, ORDINAL};
use props::Spacing;
use section::{Hf, HideFirst, Section};
use text::{
    circled, click_here_guide, floor_div, hyperlink_url, iso_from_filetime, no_spaces, split_spaces, split_unescaped, squeeze_spaces,
    unescape_backslash,
};

pub(crate) const A4: (i64, i64) = (59528, 84188); // HWPUNIT
pub(crate) const FORMS: &[&str] = &["hp:btn", "hp:radioBtn", "hp:checkBtn", "hp:comboBox", "hp:listBox", "hp:edit"];
pub(crate) const SHAPES: &[&str] = &["hp:rect", "hp:ellipse", "hp:polygon", "hp:curve", "hp:arc", "hp:line", "hp:connectLine"];
/// Inline markers inside hp:t that carry no text of their own.
const SILENT_TEXT_MARKS: &[&str] = &["hp:markpenBegin", "hp:markpenEnd", "hp:titleMark", "hp:insertBegin", "hp:insertEnd"];
/// Deepest nesting of tables, text boxes and notes followed (hostile files).
const MAX_NESTING: usize = 64;

/// An offset attribute; HWPX writes negative ones as unsigned 32-bit numbers.
pub(crate) fn signed32(v: Option<&str>) -> i64 {
    let n = num(v);
    if n >= 1 << 31 { n - (1 << 32) } else { n }
}

/// One piece of a paragraph being made: text in one formatting, or an inline object.
#[derive(Clone, Debug)]
pub(crate) enum Item {
    Text(String, CharProps),
    Obj(Box<InlineObject>, CharProps),
}

/// A paragraph being made.
#[derive(Clone, Debug, Default)]
pub(crate) struct P {
    pub props: ParaProps,
    /// The paragraph mark's formatting (its last run's).
    pub mark: CharProps,
    pub items: Vec<Item>,
    /// Space before and after, twips: kept apart until the paragraphs around it are known
    /// ([`merge_spacing`]). `None`: the paragraph doesn't say (its style does).
    pub spacing: Option<Spacing>,
    /// HWPUNIT of line spacing after the last line (see the table cells).
    pub tail: i64,
    /// The page number paragraph added for hp:pageNum.
    pub page_num: bool,
    /// Ends a section: its properties.
    pub section: Option<SectionProps>,
    /// The source paragraph it was made of (see [`crate::KEPT_FORMAT`]).
    pub origin: Option<Arc<str>>,
}

impl P {
    /// A paragraph that takes (almost) no height: separates adjacent tables and closes a section
    /// or document that ends with a table, which an app otherwise pads with a full-height empty
    /// paragraph.
    pub(crate) fn tiny() -> P {
        P {
            props: ParaProps { line_spacing: Some(LineSpacing::Exactly(1.0)), ..ParaProps::default() },
            mark: CharProps { size: Some(1.0), ..CharProps::default() },
            spacing: Some(Spacing::default()),
            ..P::default()
        }
    }

    /// An empty paragraph exactly `hu` HWPUNIT tall.
    pub(crate) fn spacer(hu: i64) -> P {
        let line = crate::units::round_even(hu as f64 / 5.0).max(20);
        P { props: ParaProps { line_spacing: Some(LineSpacing::Exactly(props::pt(line))), ..P::tiny().props }, ..P::tiny() }
    }

    fn shows_text(&self) -> bool {
        self.items.iter().any(|i| match i {
            Item::Text(t, _) => !t.trim().is_empty() || t.contains(['\t', '\n']),
            Item::Obj(o, _) => matches!(**o, InlineObject::Field { .. } | InlineObject::FieldStart { .. }),
        })
    }
}

/// A block being made.
#[derive(Clone, Debug)]
pub(crate) enum Blk {
    P(Box<P>),
    Table(Box<Table>),
}

/// How one paragraph's lines are laid out.
pub(crate) struct ParaCtx {
    pp: ParaPr,
    style_id: Option<String>,
    /// HWPUNIT height the line spacing is measured from.
    font_h: i64,
    page_break: bool,
    /// The paragraph mark's run properties (its last run's).
    mark: CharProps,
    /// No line spacing after the (one) line: a cell's or page's last.
    trim: bool,
    text_only: bool,
    /// Its tables go on the text line, in text boxes.
    inline_tables: bool,
}

/// A paragraph being converted: the pieces made so far.
#[derive(Default)]
pub(crate) struct ParaOut {
    blocks: Vec<Blk>,
    cap_above: Vec<Blk>,
    cap_below: Vec<Blk>,
    items: Vec<Item>,
    /// Nothing written yet (the page break goes on the first piece).
    first: bool,
    click_start: Option<usize>,
    para_top: Option<i64>,
}

pub(crate) struct Converter<'a> {
    pub pkg: &'a Package,
    pub h: Header,
    pub doc: Document,
    pub sec: Section,
    /// What sections pass on to the next (headers and footers, numbering of parts).
    pub carry: section::Carry,
    pub unsupported: BTreeMap<String, i64>,
    char_cache: HashMap<i64, CharProps>,
    num_ids: Vec<((&'static str, i64), u32)>,
    /// The kind and part id of the note being read, and whether its number was written.
    note: Option<(NoteKind, u32)>,
    note_ref_done: bool,
    /// Open fields by id: the hyperlink each one sets, if any.
    fields: HashMap<String, Option<String>>,
    /// The hyperlink the text now gets (an open hyperlink field).
    link: Option<String>,
    page_last: bool,
    object_numbers: BTreeMap<String, i64>,
    /// Twips, inside a table cell (None: the body).
    pub text_width: Option<i64>,
    /// Column count and gap of the current region.
    pub columns: (i64, i64),
    memos: HashMap<String, Element>,
    memo_fields: HashMap<String, u32>,
    /// Layout-cache position of the next top-level paragraph.
    para_top: Option<i64>,
    /// ... of the table being converted.
    pub table_top: Option<i64>,
    /// The alignment of the paragraph holding the table being converted: a table on a text line
    /// sits where that paragraph aligns it.
    pub table_para_align: Option<String>,
    click_fields: HashMap<String, String>,
    click_open: Option<String>,
    click_close: Option<String>,
    in_delete: bool,
    bm_n: i64,
    /// Reading a story other than the body (a header, a note, a cell's text box…).
    in_story: bool,
    depth: usize,
    /// Table cells the file doesn't hold that may still be made, for the whole document.
    pub made_cells_left: i64,
    /// Drawing ids handed out so far (shapes keep theirs).
    pub pic_n: u32,
    /// Pictures stored so far, by package entry: their media key.
    pub media_keys: HashMap<String, String>,
    /// The section being read (0-based).
    pub sec_index: usize,
    /// Made paragraphs by `rev`: the source paragraph each was made of.
    pub origins: HashMap<u64, Arc<str>>,
}

/// Read an opened package.
pub(crate) fn read(pkg: &Package) -> Result<Document> {
    let head = match &pkg.header_name {
        Some(n) => Some(pkg.xml(n)?),
        None => None,
    };
    let mut c = Converter {
        pkg,
        h: Header::new(head.as_ref()),
        doc: Document::new(),
        sec: Section::default(),
        carry: section::Carry::default(),
        unsupported: BTreeMap::new(),
        char_cache: HashMap::new(),
        num_ids: Vec::new(),
        note: None,
        note_ref_done: false,
        fields: HashMap::new(),
        link: None,
        page_last: false,
        object_numbers: BTreeMap::new(),
        text_width: None,
        columns: (1, 0),
        memos: HashMap::new(),
        memo_fields: HashMap::new(),
        para_top: None,
        table_top: None,
        table_para_align: None,
        click_fields: HashMap::new(),
        click_open: None,
        click_close: None,
        in_delete: false,
        bm_n: 0,
        in_story: false,
        depth: 0,
        made_cells_left: section::MAX_MADE_CELLS,
        pic_n: 0,
        media_keys: HashMap::new(),
        sec_index: 0,
        origins: HashMap::new(),
    };
    c.doc.body.clear();
    c.convert()?;
    c.keep_origins();
    Ok(c.doc)
}

impl Converter<'_> {
    /// Files the paragraphs' origins for the writer: those still as read once reading ends.
    fn keep_origins(&mut self) {
        let mut bytes = 0;
        let mut pending = HashMap::new();
        for (rev, o) in std::mem::take(&mut self.origins) {
            if bytes + o.len() > craft_wordmodel::kept::MAX_KEPT_BYTES {
                break;
            }
            bytes += o.len();
            pending.insert(rev, o);
        }
        self.doc.kept = craft_wordmodel::kept::Kept::new(craft_wordmodel::kept::KeptXml {
            format: crate::KEPT_FORMAT.into(),
            pending,
            bytes,
            ..Default::default()
        });
        self.doc.seal_kept();
    }

    pub(crate) fn missing(&mut self, key: &str) {
        *self.unsupported.entry(key.to_string()).or_insert(0) += 1;
    }

    fn convert(&mut self) -> Result<()> {
        let names = self.pkg.section_names.clone();
        let mut body: Vec<Blk> = Vec::new();
        let n_sections = names.len();
        for (i, name) in names.iter().enumerate() {
            self.sec_index = i;
            self.sec =
                if i > 0 { Section { outline: self.sec.outline, tab_stop: self.sec.tab_stop, ..Section::default() } } else { Section::default() };
            self.sec.first_section = i == 0;
            let mut root = self.pkg.xml(name)?;
            self.memos = root.findall("hp:memo").filter_map(|m| Some((m.get("id")?.to_string(), m.find("hp:subList")?.clone()))).collect();
            let mut blocks: Vec<Blk> = Vec::new();
            // Column regions: (index into blocks where the region starts, (count, gap)). Hancom
            // can change the column count mid-section (a two-column table of contents); the
            // model needs a continuous section break for each change.
            let mut regions: Vec<(usize, (i64, i64))> = vec![(0, (1, 0))];
            let (mut last_pos, mut region_col) = (-1i64, 0i64);
            let idx: Vec<usize> = root.children.iter().enumerate().filter(|(_, c)| c.tag == "hp:p").map(|(k, _)| k).collect();
            let starts: Vec<Option<i64>> = idx
                .iter()
                .map(|&k| {
                    root.children.get(k).and_then(|p| p.find("hp:linesegarray")).and_then(|s| s.children.first()).map(|s0| num(s0.get("vertpos")))
                })
                .collect();
            for (j, &k) in idx.iter().enumerate() {
                let Some(p) = root.children.get(k) else { continue };
                // Last paragraph on its page (or column): the next one restarts the cache.
                let segs_len = p.find("hp:linesegarray").map(|s| s.children.len());
                let first_vert = p.find("hp:linesegarray").and_then(|s| s.children.first()).map(|s| num(s.get("vertpos")));
                let last_vert = p.find("hp:linesegarray").and_then(|s| s.children.last()).map(|s| num(s.get("vertpos")));
                let nxt = if j + 1 < idx.len() { starts.get(j + 1).copied().flatten() } else { Some(0) };
                self.para_top = starts.get(j).copied().flatten();
                let col = direct_col_pr(p);
                if let Some(col) = col {
                    if blocks.is_empty() {
                        regions[0] = (0, col);
                    } else if regions.last().map(|r| r.1) != Some(col) {
                        regions.push((blocks.len(), col));
                    }
                }
                self.columns = regions.last().map(|r| r.1).unwrap_or((1, 0));
                // (One column only, and not before a new column region: the cache restarts at
                // each column and region too, and an app balances columns its own way.)
                let next_has_col = idx.get(j + 1).and_then(|&n| root.children.get(n)).map(|q| direct_col_pr(q).is_some()).unwrap_or(false);
                self.page_last =
                    segs_len == Some(1) && matches!((nxt, first_vert), (Some(n), Some(f)) if n <= f) && self.columns.0 == 1 && !next_has_col;
                // Hancom's layout cache restarts line positions at each page, so a drop in
                // vertpos marks a new page (an explicit page break without a cache, too). It
                // restarts at each column and each new column region as well: in a region of n
                // columns every n-th restart is a page, the others are column breaks, written as
                // such so the columns split where Hancom split them.
                let mut set_column_break = None;
                match (first_vert, last_vert) {
                    (Some(first), Some(last)) => {
                        if col.is_some() {
                            // A new column definition starts again in the first column; Hancom
                            // ignores a column break on that paragraph (viewer).
                            region_col = 0;
                            set_column_break = Some("0");
                        } else if 0 <= first && first < last_pos {
                            region_col += 1;
                            let cols = self.columns.0;
                            if cols > 1 && region_col % cols != 0 && !truthy(p.get("pageBreak")) {
                                set_column_break = Some("1");
                            } else {
                                self.sec.page_index += 1;
                            }
                        }
                        last_pos = last;
                    }
                    _ => {
                        if truthy(p.get("pageBreak")) && !blocks.is_empty() {
                            self.sec.page_index += 1;
                        }
                    }
                }
                if let Some(v) = set_column_break
                    && let Some(pm) = root.children.get_mut(k)
                {
                    pm.set("columnBreak", v);
                }
                let Some(p) = root.children.get(k) else { continue };
                let made = self.paragraph(p, false);
                blocks.extend(made);
            }
            self.finish_section();
            let n_regions = regions.len();
            for r in 0..n_regions {
                let Some(&(start, (cols, gap))) = regions.get(r) else { continue };
                let end = regions.get(r + 1).map(|x| x.0).unwrap_or(blocks.len()).min(blocks.len());
                let start = start.min(end);
                let mut part: Vec<Blk> = blocks.get(start..end).map(<[Blk]>::to_vec).unwrap_or_default();
                // A column region that starts on a new page: the page break is the section
                // break's (a page break inside a continuous break is not honoured).
                let mut new_page = false;
                if r > 0
                    && let Some(Blk::P(p0)) = part.first_mut()
                    && p0.props.page_break_before == Some(true)
                {
                    p0.props.page_break_before = None;
                    new_page = true;
                }
                let sect = self.section_props(cols, gap, r == 0, new_page);
                if r == n_regions - 1 && i == n_sections - 1 {
                    // The document's last section is the document's own.
                    let ends_with_table = matches!(part.last(), Some(Blk::Table(_)));
                    body.extend(part);
                    if ends_with_table {
                        body.push(Blk::P(Box::new(P::tiny())));
                    }
                    self.doc.last_section = sect;
                } else {
                    merge_spacing(&mut part);
                    match part.last_mut() {
                        // Any other section ends on the last paragraph of its region.
                        Some(Blk::P(lastp)) => lastp.section = Some(sect),
                        Some(Blk::Table(_)) => {
                            part.push(Blk::P(Box::new(P { section: Some(sect), ..P::tiny() })));
                        }
                        None => part.push(Blk::P(Box::new(P { section: Some(sect), ..P::default() }))),
                    }
                    body.extend(part);
                }
            }
        }
        self.doc.body = blocks(body, false, &mut self.origins);
        self.doc.styles = Arc::new(self.style_sheet());
        self.doc.numbering = Arc::new(self.numbering());
        self.doc.settings.default_tab = props::pt(tw(self.sec.tab_stop).max(1));
        self.doc.settings.even_odd_headers = self.carry.even_odd;
        let get = |k: &str| self.pkg.metadata.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        self.doc.core.title = get("title");
        self.doc.core.creator = get("creator");
        for (what, n) in &self.unsupported {
            log::warn!("HWPX: {n} × {what} not read");
        }
        Ok(())
    }

    // ==== blocks / paragraphs ===========================================================
    pub(crate) fn blocks<'e>(&mut self, paras: impl Iterator<Item = &'e Element>, in_cell: bool) -> Vec<Blk> {
        let paras: Vec<&Element> = paras.collect();
        if self.depth >= MAX_NESTING {
            self.missing("nesting too deep");
            return Vec::new();
        }
        self.depth += 1;
        let mut out = Vec::new();
        let n = paras.len();
        for (i, p) in paras.into_iter().enumerate() {
            out.extend(self.paragraph(p, in_cell && i + 1 == n));
        }
        self.depth -= 1;
        out
    }

    /// The paragraphs of a story other than the body (a header, a note, a memo).
    pub(crate) fn story<'e>(&mut self, paras: impl Iterator<Item = &'e Element>, in_cell: bool) -> Vec<Blk> {
        let saved = std::mem::replace(&mut self.in_story, true);
        let out = self.apart(|c| c.blocks(paras, in_cell));
        self.in_story = saved;
        out
    }

    /// Reads a story of its own (a note, a header, a text box): a hyperlink open around where it
    /// is anchored doesn't reach into it, and comes back after it.
    pub(crate) fn apart<T>(&mut self, read: impl FnOnce(&mut Self) -> T) -> T {
        let link = self.link.take();
        let out = read(self);
        self.link = link;
        out
    }

    fn style_height(&self, style_id: Option<&str>) -> i64 {
        let cp = self.h.style(num(style_id)).map(|s| s.char_pr).unwrap_or(0);
        self.h.char_pr_id(cp).height
    }

    pub(crate) fn paragraph(&mut self, p: &Element, cell_last: bool) -> Vec<Blk> {
        let page_last = std::mem::replace(&mut self.page_last, false); // nested (cell) paragraphs: false
        let runs_el: Vec<&Element> = p.findall("hp:run").collect();
        let ctx = self.para_layout(p, &runs_el, cell_last, page_last);
        let mut acc = ParaOut { para_top: self.para_top.take(), first: true, ..Default::default() };
        if truthy(p.get("columnBreak")) {
            acc.items.push(Item::Text(craft_wordmodel::para::COLUMN_BREAK.to_string(), CharProps::default()));
        }
        for run in &runs_el {
            let props = self.run_props(run.get("charPrIDRef"));
            for child in &run.children {
                self.para_child(child, &props, &ctx, &mut acc);
            }
        }
        let mut blocks = std::mem::take(&mut acc.blocks);
        if !acc.items.is_empty() || blocks.is_empty() {
            let page_break = ctx.page_break && acc.first;
            blocks.push(Blk::P(Box::new(self.para_with(&ctx, page_break, std::mem::take(&mut acc.items)))));
        }
        if cell_last
            && ctx.text_only
            && !ctx.trim
            && let Some(Blk::P(last)) = blocks.last_mut()
        {
            let v = ctx.pp.ls_value;
            last.tail = match ctx.pp.ls_type.as_str() {
                "PERCENT" => floor_div(ctx.font_h.saturating_mul((v - 100).max(0)), 100),
                "BETWEEN_LINES" => v.max(0),
                _ => 0,
            };
        }
        // Korean words wrap as the paragraph shape says, on each paragraph made of it; each
        // piece notes where it came from
        if let Some(ord) = p.get(ORDINAL) {
            let mut piece = 0;
            for b in blocks.iter_mut() {
                if let Blk::P(bp) = b {
                    bp.props.keep_hangul_words = Some(ctx.pp.keep_words);
                    if bp.origin.is_none() {
                        bp.origin = Some(Arc::from(format!("{}:{ord}:{piece}", self.sec_index)));
                        piece += 1;
                    }
                }
            }
        }
        let mut out = acc.cap_above;
        out.extend(blocks);
        out.extend(acc.cap_below);
        out
    }

    /// A paragraph of `ctx`'s shape holding `items`.
    fn para_with(&mut self, ctx: &ParaCtx, page_break: bool, items: Vec<Item>) -> P {
        let (mut props, spacing) = self.para_props(&ctx.pp, ctx.font_h, page_break, true, ctx.trim, ctx.text_only);
        if let Some(sid) = ctx.style_id.as_deref()
            && self.h.style(num(Some(sid))).is_some()
        {
            props.style = Some(props::style_id(num(Some(sid))));
        }
        P { props, mark: ctx.mark.clone(), items, spacing: Some(spacing), ..P::default() }
    }

    /// How a paragraph's lines are laid out (see [`ParaCtx`]).
    fn para_layout(&mut self, p: &Element, runs_el: &[&Element], cell_last: bool, page_last: bool) -> ParaCtx {
        // The section's page, given in its first paragraph, is read first: the tables below are
        // measured against its line width (a later read of the same settings does nothing).
        if let Some(sp) = runs_el.iter().flat_map(|r| r.children.iter()).find(|c| c.tag == "hp:secPr") {
            self.sec_pr(sp);
        }
        let pp = self.h.para_pr(p.get("paraPrIDRef"));
        let style_id = p.get("styleIDRef").map(str::to_string);
        let mut heights: Vec<i64> = runs_el.iter().map(|r| self.h.char_pr(r.get("charPrIDRef")).height).collect();
        if heights.is_empty() {
            heights.push(self.style_height(style_id.as_deref()));
        }
        let max_h = heights.iter().copied().max().unwrap_or(1000);
        let mut font_h = main_height(runs_el, &heights, max_h);
        let mark = runs_el.last().map(|r| self.run_props(r.get("charPrIDRef"))).unwrap_or_default();
        // Hancom lays text lines at exactly font * percent; an app would stretch a line to the
        // font's natural height, so text-only paragraphs get an exact line height. Paragraphs
        // with objects keep a minimum height so pictures are never clipped. Inside a cell, a
        // one-line last paragraph is exactly one font height tall: Hancom adds no line spacing
        // after a cell's last line (its layout cache, hp:lineseg). Tables set "like a
        // character" that share their line with text or other tables (boxes for a member
        // number) stay on the line inside inline text boxes; a table of the model always
        // starts a block of its own.
        let tables: Vec<&Element> = runs_el.iter().flat_map(|r| r.children.iter()).filter(|c| c.tag == "hp:tbl").collect();
        let mut inline_tables = !tables.is_empty()
            && tables.iter().all(|t| t.find("hp:pos").is_some_and(|pos| truthy(pos.get("treatAsChar"))))
            && (tables.len() > 1 || runs_el.iter().flat_map(|r| r.findall("hp:t")).any(|t| !t.text.trim().is_empty()));
        if inline_tables {
            // Only tables that share a line: wider ones go one under the other in Hancom too.
            let widths: i64 = tables.iter().map(|t| t.find("hp:sz").map(|s| num(s.get("width"))).unwrap_or(0)).sum();
            inline_tables = tw(widths) <= self.line_width();
        }
        let text_only = !inline_tables
            && !runs_el.iter().flat_map(|r| r.children.iter()).any(|c| {
                c.tag == "hp:pic"
                    || SHAPES.contains(&c.tag.as_str())
                    || FORMS.contains(&c.tag.as_str())
                    || matches!(c.tag.as_str(), "hp:container" | "hp:equation" | "hp:ole")
            });
        let segs = p.find("hp:linesegarray");
        let segs_len = segs.map(|s| s.children.len());
        if segs_len == Some(1) {
            // One line holds every character, the tallest included, and a bullet or number in
            // its own character shape: Hancom's layout cache has the result.
            font_h = max_h;
            let cached = segs.and_then(|s| s.children.first()).map(|s| num(s.get("vertsize"))).unwrap_or(0);
            if text_only && font_h <= cached && cached <= floor_div(font_h * 3, 2) {
                font_h = cached;
            }
        }
        // The line spacing after a page's last line takes no room in Hancom either.
        let mut one_line = segs_len == Some(1);
        if segs.is_none()
            && cell_last
            && let Some(text_width) = self.text_width.filter(|w| *w != 0)
        {
            // No layout cache (some generators leave it out): one line if the text is clearly
            // narrower than the cell (full-width characters 1 em, others 0.6 em).
            let text: String = runs_el
                .iter()
                .flat_map(|r| r.findall("hp:t"))
                .map(|t| format!("{}{}", t.text, t.children.iter().map(|c| c.tail.as_str()).collect::<String>()))
                .collect();
            let em = font_h as f64 / 5.0;
            let width: f64 = text.chars().map(|ch| if ch as u32 >= 0x1100 { em } else { em * 0.6 }).sum();
            one_line = !text.contains('\n') && width <= text_width as f64 * 0.9;
        }
        let trim = (cell_last || page_last) && text_only && one_line;
        ParaCtx { pp, style_id, font_h, page_break: truthy(p.get("pageBreak")), mark, trim, text_only, inline_tables }
    }

    /// One child of a paragraph's run.
    fn para_child(&mut self, child: &Element, props: &CharProps, ctx: &ParaCtx, acc: &mut ParaOut) {
        let tag = child.tag.as_str();
        match tag {
            "hp:t" => {
                let t = self.text(child, props);
                acc.items.extend(t);
            }
            "hp:ctrl" => {
                let c = self.ctrl(child, props);
                acc.items.extend(c);
                if self.click_open.take().is_some() {
                    acc.click_start = Some(acc.items.len());
                }
                if let Some(guide) = self.click_close.take() {
                    // An unfilled input field (Hancom: 누름틀) shows its guide text only while editing in Hancom (the
                    // viewer shows nothing); hide it, keeping the text.
                    if let Some(tail) = acc.click_start.and_then(|cs| acc.items.get_mut(cs..)) {
                        let shown: String = tail.iter().filter_map(|i| if let Item::Text(t, _) = i { Some(t.as_str()) } else { None }).collect();
                        if no_spaces(&shown) == no_spaces(&guide) {
                            for i in tail.iter_mut() {
                                if let Item::Text(_, p) = i {
                                    p.hidden = Some(true);
                                }
                            }
                        }
                    }
                    acc.click_start = None;
                }
            }
            "hp:secPr" => self.sec_pr(child),
            "hp:tbl" => self.para_table(child, props, ctx, acc),
            "hp:compose" => {
                let c = self.compose(child, props);
                acc.items.extend(c);
            }
            _ => self.para_object(child, props, acc),
        }
    }

    /// Pictures, drawings, equations and form objects in a run.
    fn para_object(&mut self, child: &Element, props: &CharProps, acc: &mut ParaOut) {
        let tag = child.tag.as_str();
        match tag {
            "hp:equation" => acc.items.extend(self.equation(child, props)),
            _ if tag == "hp:pic" || SHAPES.contains(&tag) || tag == "hp:container" => {
                acc.items.extend(self.drawing(child, props));
                // The caption of an object on a text line: its own paragraphs above or below
                // that line.
                if child.find("hp:pos").is_some_and(|x| truthy(x.get("treatAsChar")))
                    && let Some((side, cap)) = self.caption(child)
                    && !cap.is_empty()
                {
                    if side == "TOP" { acc.cap_above.extend(cap) } else { acc.cap_below.extend(cap) }
                }
            }
            _ if FORMS.contains(&tag) => {
                let f = self.form(child, props);
                acc.items.extend(f);
            }
            _ => self.missing(tag),
        }
    }

    fn para_table(&mut self, child: &Element, props: &CharProps, ctx: &ParaCtx, acc: &mut ParaOut) {
        let pos = child.find("hp:pos");
        let over = matches!(child.get("textWrap"), Some("BEHIND_TEXT" | "IN_FRONT_OF_TEXT")) && pos.is_some_and(|x| !truthy(x.get("treatAsChar")));
        if ctx.inline_tables || over {
            let r = self.inline_table(child, props);
            acc.items.extend(r);
            return;
        }
        self.table_top = acc.para_top.take(); // only the first table's is known
        self.table_para_align = Some(ctx.pp.align.clone());
        let blocks = self.table(child, false);
        if blocks.is_empty() {
            return;
        }
        // flush the text before the table
        if !acc.items.is_empty() {
            let page_break = ctx.page_break && acc.first;
            let items = std::mem::take(&mut acc.items);
            // Floating objects alone before the table are placed from their paragraph's top,
            // which in Hancom is the table's line: a paragraph of (almost) no height keeps them
            // there instead of a line higher.
            let floating_only =
                items.iter().any(|i| matches!(i, Item::Obj(o, _) if o.is_floating())) && !P { items: items.clone(), ..P::default() }.shows_text();
            let p = if floating_only && !page_break && !ctx.pp.page_break_before {
                P { items, ..P::tiny() }
            } else {
                self.para_with(ctx, page_break, items)
            };
            acc.blocks.push(Blk::P(Box::new(p)));
            acc.first = false;
        }
        if acc.first && (ctx.page_break || ctx.pp.page_break_before) {
            // The paragraph starts with a table: a table cannot carry the page break, so a
            // paragraph of (almost) no height does.
            let mut p = P::tiny();
            p.props.page_break_before = Some(true);
            acc.blocks.push(Blk::P(Box::new(p)));
        }
        acc.blocks.extend(blocks);
        acc.first = false;
    }

    // ==== text ===========================================================================
    /// Text in `props` (with the open hyperlink, if any).
    fn text_item(&self, s: &str, props: &CharProps) -> Item {
        let mut p = props.clone();
        if let Some(l) = &self.link {
            p.link = Some(l.clone());
        }
        Item::Text(s.to_string(), p)
    }

    /// The pieces of `t` in `props`.
    fn text(&mut self, t: &Element, props: &CharProps) -> Vec<Item> {
        let mut out: Vec<Item> = Vec::new();
        if !t.text.is_empty() && !self.in_delete {
            for piece in split_spaces(&t.text) {
                out.push(self.text_item(piece, props));
            }
        }
        for c in &t.children {
            let tag = c.tag.as_str();
            if self.in_delete && tag != "hp:deleteEnd" {
                // inside a tracked deletion
            } else {
                match tag {
                    "hp:tab" => out.push(self.text_item("\t", props)),
                    "hp:lineBreak" => out.push(self.text_item(&craft_wordmodel::para::LINE_BREAK.to_string(), props)),
                    "hp:nbSpace" => out.push(self.text_item("\u{a0}", props)),
                    "hp:fwSpace" => out.push(self.text_item("\u{3000}", props)),
                    "hp:hypen" => out.push(self.text_item("-", props)),
                    "hp:deleteBegin" => self.in_delete = true,
                    "hp:deleteEnd" => self.in_delete = false,
                    _ if SILENT_TEXT_MARKS.contains(&tag) => {}
                    _ => self.missing(tag),
                }
            }
            if !c.tail.is_empty() && !self.in_delete {
                for piece in split_spaces(&c.tail) {
                    out.push(self.text_item(piece, props));
                }
            }
        }
        out
    }

    /// Overlapped characters (Hancom: 글자 겹치기), used in documents mostly for circled numbers. Circled
    /// numbers above 18 come as a tens glyph (U+F0288 + tens) and a units glyph (U+F0292 +
    /// units) from Hancom's private-use range; inferred from consecutive clause numbers in real
    /// documents and checked against the Hancom Viewer.
    fn compose(&mut self, el: &Element, props: &CharProps) -> Vec<Item> {
        let mut text = el.get("composeText").unwrap_or("").to_string();
        let codes: Vec<u32> = text.chars().map(|c| c as u32).collect();
        let private = |c: u32| (0xE000..=0xF8FF).contains(&c) || c >= 0xF0000;
        if let [a, b] = codes.as_slice()
            && (0xF0288..=0xF0291).contains(a)
            && (0xF0292..=0xF029B).contains(b)
            && el.get("circleType").unwrap_or("CHAR") == "CHAR"
        {
            text = circled(i64::from(a - 0xF0288) * 10 + i64::from(b - 0xF0292));
            return if text.is_empty() || self.in_delete { vec![] } else { vec![self.text_item(&text, props)] };
        }
        if codes.iter().any(|&c| private(c)) {
            self.missing("hp:compose (unknown glyphs)");
            text = text.chars().filter(|c| !private(*c as u32)).collect();
        }
        if text.is_empty() || self.in_delete { vec![] } else { vec![self.text_item(&text, props)] }
    }

    // ==== controls =======================================================================
    fn ctrl(&mut self, el: &Element, props: &CharProps) -> Vec<Item> {
        let mut out = Vec::new();
        for c in &el.children {
            let tag = c.tag.as_str();
            match tag {
                "hp:colPr" => {} // handled per column region, see direct_col_pr()
                "hp:header" | "hp:footer" => self.header_footer(c, if tag == "hp:header" { "header" } else { "footer" }),
                "hp:footNote" | "hp:endNote" => {
                    let r = self.note(c, if tag == "hp:footNote" { NoteKind::Footnote } else { NoteKind::Endnote });
                    out.push(r);
                }
                "hp:fieldBegin" => {
                    let r = self.field_begin(c);
                    out.extend(r);
                }
                "hp:fieldEnd" => {
                    for k in [c.get("beginIDRef").unwrap_or(""), c.get("fieldid").unwrap_or("")] {
                        if let Some(g) = self.click_fields.remove(k) {
                            self.click_close = Some(g);
                        }
                        if let Some(cid) = self.memo_fields.remove(k) {
                            out.push(Item::Obj(Box::new(InlineObject::CommentEnd { id: cid }), CharProps::default()));
                        }
                    }
                    let a = self.fields.remove(c.get("beginIDRef").unwrap_or("")).flatten();
                    let b = a.or_else(|| self.fields.remove(c.get("fieldid").unwrap_or("")).flatten());
                    if b.is_some() {
                        self.link = None;
                    }
                }
                "hp:bookmark" => {
                    self.bm_n += 1;
                    let raw = c.get("name").map(str::to_string).unwrap_or_else(|| format!("bm{}", self.bm_n));
                    let name: String = squeeze_spaces(&raw, "_").chars().take(40).collect();
                    out.push(Item::Obj(Box::new(InlineObject::BookmarkStart { name: name.clone() }), CharProps::default()));
                    out.push(Item::Obj(Box::new(InlineObject::BookmarkEnd { name }), CharProps::default()));
                }
                "hp:autoNum" => {
                    let r = self.auto_num(c, props);
                    out.extend(r);
                }
                "hp:pageNum" => {
                    self.sec.page_num = Some((
                        c.get("pos").unwrap_or("BOTTOM_CENTER").to_string(),
                        c.get("formatType").unwrap_or("DIGIT").to_string(),
                        c.get("sideChar").unwrap_or("").to_string(),
                    ));
                }
                "hp:newNum" if c.get("numType") == Some("PAGE") && self.on_first_page() => {
                    self.sec.start_page = num_or(c.get("num"), 1).max(1);
                }
                "hp:pageHiding" if self.on_first_page() => {
                    self.sec.hide_first = Some(HideFirst {
                        header: truthy(c.get("hideHeader")),
                        footer: truthy(c.get("hideFooter")),
                        pagenum: truthy(c.get("hidePageNum")),
                    });
                }
                _ => self.missing(tag),
            }
        }
        out
    }

    /// Page controls on a section's first page have a place in the section (title page, page
    /// number start).
    fn on_first_page(&self) -> bool {
        !self.in_story && self.sec.page_index == 0
    }

    fn auto_num(&mut self, el: &Element, props: &CharProps) -> Vec<Item> {
        let t = el.get("numType").unwrap_or("");
        match t {
            "PAGE" => vec![field("PAGE", props)],
            "TOTAL_PAGE" => vec![field("NUMPAGES", props)],
            "FOOTNOTE" | "ENDNOTE" if self.note.is_some() => {
                self.note_ref_done = true;
                let Some((kind, id)) = self.note else { return vec![] };
                vec![Item::Obj(Box::new(InlineObject::NoteRef { kind, id, custom: String::new() }), superscript())]
            }
            "PICTURE" | "TABLE" | "EQUATION" => {
                // Caption numbers count each kind through the document (written as text).
                let n = self.object_numbers.entry(t.to_string()).or_insert(0);
                *n += 1;
                let s = n.to_string();
                vec![self.text_item(&s, props)]
            }
            _ => {
                self.missing(&format!("autoNum:{t}"));
                vec![]
            }
        }
    }

    fn field_begin(&mut self, el: &Element) -> Vec<Item> {
        let ids: Vec<String> = [el.get("id"), el.get("fieldid")].into_iter().flatten().filter(|k| !k.is_empty()).map(str::to_string).collect();
        let url = if el.get("type") == Some("HYPERLINK") { hyperlink_url(el) } else { None };
        for k in &ids {
            self.fields.insert(k.clone(), url.clone());
        }
        if el.get("type") == Some("CLICK_HERE") {
            let guide = click_here_guide(el);
            if !guide.is_empty() {
                for k in &ids {
                    self.click_fields.insert(k.clone(), guide.clone());
                }
                self.click_open = Some(guide);
            }
        }
        if let Some(memo) = self.memo_comment(el) {
            for k in &ids {
                self.memo_fields.insert(k.clone(), memo);
            }
            return vec![Item::Obj(Box::new(InlineObject::CommentStart { id: memo }), CharProps::default())];
        }
        match url {
            Some(url) => self.link = Some(url),
            None => {
                if el.get("type") != Some("CLICK_HERE") {
                    // an input field keeps its text; nothing is lost
                    self.missing(&format!("field:{}", el.get("type").unwrap_or("")));
                }
            }
        }
        vec![]
    }

    /// A memo field (Hancom: 메모; "MEMO/<shape>/<number>/.../<author>/...") -> a comment; the
    /// memo's paragraphs (hp:memo, or in HWPX the field's hp:subList) are its text.
    fn memo_comment(&mut self, el: &Element) -> Option<u32> {
        let cmd = el.iter("hp:stringParam").find(|sp| sp.get("name") == Some("Command")).map(|sp| sp.text.clone()).unwrap_or_default();
        let parts = split_unescaped(&cmd, '/');
        if !matches!(el.get("type"), Some("MEMO" | "UNKNOWN")) || !cmd.starts_with("MEMO/") || parts.len() < 3 {
            return None;
        }
        // HWP keeps the memo's paragraphs apart (hp:memo, by number); HWPX inside the field
        let sub = parts.get(2).and_then(|k| self.memos.get(*k)).or_else(|| el.find("hp:subList")).cloned();
        let made = match &sub {
            Some(s) => self.story(s.findall("hp:p"), true),
            None => vec![],
        };
        let author = parts.get(5).map(|a| unescape_backslash(a).trim().to_string()).unwrap_or_default();
        let author = if author.is_empty() { "HWP".to_string() } else { author };
        // the memo's time: a FILETIME in two halves (low, high), UTC (viewer-checked)
        let half = |k: usize| parts.get(k).and_then(|v| v.trim().parse::<u32>().ok());
        let when = half(3).zip(half(4)).and_then(|(lo, hi)| iso_from_filetime((u64::from(hi) << 32) | u64::from(lo)));
        let part = self.doc.add_part(PartKind::Comment, blocks(made, true, &mut self.origins));
        let cid = self.doc.comments.len() as u32;
        let initials: String = author.chars().take(2).collect();
        self.doc.comments.insert(cid, Comment { author, initials, date: when.unwrap_or_default(), part, ..Comment::default() });
        Some(cid)
    }

    fn note(&mut self, el: &Element, kind: NoteKind) -> Item {
        let part_kind = if kind == NoteKind::Footnote { PartKind::Footnote } else { PartKind::Endnote };
        let id = self.doc.add_part(part_kind, Vec::new());
        let saved = (self.note, self.note_ref_done);
        self.note = Some((kind, id));
        self.note_ref_done = false;
        let mut made = match el.find("hp:subList") {
            Some(sub) => self.story(sub.findall("hp:p"), false),
            None => vec![],
        };
        if !self.note_ref_done {
            let r = Item::Obj(Box::new(InlineObject::NoteRef { kind, id, custom: String::new() }), superscript());
            match made.first_mut() {
                Some(Blk::P(p0)) => p0.items.insert(0, r),
                _ => made.insert(0, Blk::P(Box::new(P { items: vec![r], ..P::default() }))),
            }
        }
        (self.note, self.note_ref_done) = saved;
        if let Some(part) = self.doc.parts.get_mut(&id) {
            part.blocks = blocks(made, true, &mut self.origins);
        }
        Item::Obj(Box::new(InlineObject::NoteRef { kind, id, custom: String::new() }), superscript())
    }

    fn header_footer(&mut self, el: &Element, kind: &'static str) {
        let made = match el.find("hp:subList") {
            Some(sub) => self.story(sub.findall("hp:p"), false),
            None => vec![],
        };
        let apply = el.get("applyPageType").unwrap_or("BOTH");
        let which = if apply == "EVEN" { "even" } else { "default" };
        if apply == "EVEN" {
            self.carry.even_odd = true;
        }
        self.carry.hf_n += 1;
        let name = format!("{kind}{}", self.carry.hf_n);
        self.sec.hf.push(Hf { kind, which, name, blocks: Some(made) });
    }
}

/// A field showing `instr`'s result ("1" until the app updates it).
fn field(instr: &str, props: &CharProps) -> Item {
    Item::Obj(
        Box::new(InlineObject::Field { instr: instr.to_string(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() }),
        props.clone(),
    )
}

fn superscript() -> CharProps {
    CharProps { vert_align: Some(craft_wordmodel::props::VertAlign::Superscript), ..CharProps::default() }
}

/// Hancom sizes each line by its own tallest character, so a short bigger run (a citation
/// "[1]") lifts one line only; use the height that carries most text.
fn main_height(runs_el: &[&Element], heights: &[i64], max_h: i64) -> i64 {
    let mut weight: Vec<(i64, i64)> = Vec::new();
    for (r, hgt) in runs_el.iter().zip(heights.iter()) {
        let n: i64 =
            r.findall("hp:t").map(|t| (t.text.chars().count() + t.children.iter().map(|c| c.tail.chars().count()).sum::<usize>()) as i64).sum();
        match weight.iter_mut().find(|(k, _)| k == hgt) {
            Some(w) => w.1 += n,
            None => weight.push((*hgt, n)),
        }
    }
    if weight.iter().any(|(_, w)| *w > 0)
        && let Some(&(main, _)) = weight.iter().max_by_key(|(k, w)| (*w, *k))
        && max_h * 5 <= main * 6
    {
        return main; // much bigger text keeps its room (exact lines clip)
    }
    max_h
}

/// (count, gap) of a column definition placed directly in this paragraph.
pub(crate) fn direct_col_pr(p: &Element) -> Option<(i64, i64)> {
    for r in p.findall("hp:run") {
        for ctl in r.findall("hp:ctrl") {
            if let Some(c) = ctl.find("hp:colPr") {
                return Some((num_or(c.get("colCount"), 1).clamp(1, 255), num(c.get("sameGap"))));
            }
        }
    }
    None
}

/// Hancom (and Word) add one paragraph's space after to the next one's space before; an app
/// that keeps only the larger of the two (measured: 20pt + 20pt -> 20pt) shows them apart only
/// when the space after moves into the next paragraph's space before, which gives the sum in
/// both.
pub(crate) fn merge_spacing(blocks: &mut [Blk]) {
    for i in 1..blocks.len() {
        let (head, tail) = blocks.split_at_mut(i);
        let (Some(Blk::P(a)), Some(Blk::P(b))) = (head.last_mut(), tail.first_mut()) else { continue };
        let (Some(sa), Some(sb)) = (a.spacing.as_mut(), b.spacing.as_mut()) else { continue };
        if sa.after == 0 {
            continue;
        }
        sb.before = sa.after.saturating_add(sb.before);
        sa.after = 0;
    }
}

/// The finished blocks of a story: spacing merged, adjacent tables kept apart, and an empty
/// paragraph at the end of a story that needs one (`need_final_p`: a header, a note, a cell).
pub(crate) fn blocks(mut made: Vec<Blk>, need_final_p: bool, origins: &mut HashMap<u64, Arc<str>>) -> Blocks {
    merge_spacing(&mut made);
    let mut out: Blocks = Vec::new();
    let mut prev_table = false;
    let n = made.len();
    for b in made {
        match b {
            Blk::Table(t) => {
                if prev_table {
                    // Adjacent tables would merge into one; keep them apart.
                    out.push(Arc::new(Block::Para(paragraph(P::tiny()))));
                }
                out.push(Arc::new(Block::Table(*t)));
                prev_table = true;
            }
            Blk::P(mut p) => {
                let origin = p.origin.take();
                let para = paragraph(*p);
                if let Some(o) = origin {
                    origins.insert(para.rev, o);
                }
                out.push(Arc::new(Block::Para(para)));
                prev_table = false;
            }
        }
    }
    if need_final_p && (n == 0 || prev_table) {
        out.push(Arc::new(Block::Para(Paragraph::new())));
    }
    out
}

/// A made paragraph as the model's: its pieces as text, runs and objects.
pub(crate) fn paragraph(p: P) -> Paragraph {
    let mut para = Paragraph::new();
    let mut props = p.props;
    if let Some(s) = p.spacing {
        props.space_before = Some(props::pt(s.before));
        props.space_after = Some(props::pt(s.after));
    }
    para.props = props;
    para.mark = p.mark;
    para.section = p.section.map(Box::new);
    let push = |para: &mut Paragraph, text: &str, props: CharProps| {
        if text.is_empty() {
            return;
        }
        match para.runs.last_mut() {
            Some(r) if r.props == props => r.len += text.len(),
            _ => para.runs.push(Run { len: text.len(), props }),
        }
        para.text.push_str(text);
    };
    for item in p.items {
        match item {
            Item::Text(t, props) => push(&mut para, &t, props),
            Item::Obj(o, props) => {
                push(&mut para, &craft_wordmodel::para::OBJ.to_string(), props);
                para.objects.push(*o);
            }
        }
    }
    para.normalize();
    para
}

#[cfg(test)]
mod tests;
