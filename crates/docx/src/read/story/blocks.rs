use std::collections::HashSet;

use super::perms::{Grids, RowGrid, column_marker_waits};
use super::{
    Arc, Block, Blocks, Cell, CharProps, El, InlineObject, MAX_COLS, MAX_ROWS, MAX_TABLE_DEPTH, Node, NumChange, PB, Paragraph, Phase, PropChange,
    Reader, Rels, RevisionKind, Row, Run, RunCtx, StoryCtx, Table, TableFrame, children_of_choice, clean_text, collect, last_para_mut, para_block,
    placeholder_of, sectpr, tcpr, trpr, u32_of,
};

impl Reader<'_> {
    /// Read a story's blocks from a container element (`w:body`, `w:hdr`, `w:tc`, `w:txbxContent`…).
    pub fn read_blocks(&mut self, sc: &mut StoryCtx, parent: &El, rels: &Rels, out: &mut Blocks, depth: usize) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        for e in parent.els() {
            match e.name.as_str() {
                "w:p" => {
                    let p = self.read_para(sc, e, rels, depth);
                    // A framed drop-cap paragraph merges into the paragraph it starts.
                    if let Some(Block::Para(prev)) = out.last().map(|b| &**b)
                        && prev.props.drop_cap.is_some()
                        && prev.section.is_none()
                        && prev.text.len() <= 16
                        && p.props.drop_cap.is_none()
                        && let Some(Block::Para(mut first)) = out.pop().map(|b| (*b).clone())
                    {
                        let mut props = p.props.clone();
                        props.drop_cap = first.props.drop_cap;
                        props.drop_cap_margin = first.props.drop_cap_margin;
                        props.drop_cap_distance = first.props.drop_cap_distance;
                        // A letter in a font of its own: the drop cap's font, the letter itself
                        // in the text's (so taking the drop cap away gives the letter its font back).
                        let cap_font = first.runs.first().and_then(|r| r.props.font.clone());
                        let text_font = p.runs.first().and_then(|r| r.props.font.clone());
                        if cap_font.is_some() && cap_font != text_font {
                            for r in first.runs.iter_mut().filter(|r| r.props.font == cap_font) {
                                r.props.font = text_font.clone();
                            }
                            props.drop_cap_font = cap_font;
                        }
                        first.append(p);
                        first.props = props;
                        out.push(para_block(first));
                        continue;
                    }
                    out.push(para_block(p));
                }
                "w:tbl" => {
                    if depth / 4 >= MAX_TABLE_DEPTH {
                        out.push(para_block(Paragraph::with_text(&clean_text(&e.deep_text()), CharProps::default())));
                    } else if let Some(t) = self.read_table(sc, e, rels, depth) {
                        out.push(Arc::new(Block::Table(t)));
                    }
                }
                "w:sdt" => self.read_block_sdt(sc, e, rels, out, depth),
                "w:altChunk" => self.read_alt_chunk(sc, e, rels, out, depth),
                "w:customXml" | "w:smartTag" | "w:ins" | "w:moveTo" => self.read_blocks(sc, e, rels, out, depth + 1),
                // Math is allowed straight in a story (EG_BlockLevelElts takes EG_RunLevelElements,
                // which takes EG_MathContent): it gets a paragraph of its own (#1273).
                "m:oMathPara" | "m:oMath" => {
                    let wrapped = El { name: "w:p".into(), attrs: Vec::new(), kids: vec![Node::El(e.clone())] };
                    let p = self.read_para(sc, &wrapped, rels, depth);
                    out.push(para_block(p));
                }
                "mc:AlternateContent" => {
                    if let Some(c) = children_of_choice(e) {
                        self.read_blocks(sc, c, rels, out, depth + 1);
                    }
                }
                "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd" | "w:permStart" | "w:permEnd" => {
                    if let Some(o) = self.marker(sc, e, false) {
                        sc.pending.push(o);
                    }
                }
                "w:moveFromRangeStart" | "w:moveFromRangeEnd" | "w:moveToRangeStart" | "w:moveToRangeEnd" => self.move_range(e),
                _ => {}
            }
        }
    }

    /// A block-level content control: its content's blocks, between a start marker at the start
    /// of the first paragraph and an end marker at the end of the last (a table at either end
    /// carries the control instead).
    fn read_block_sdt(&mut self, sc: &mut StoryCtx, e: &El, rels: &Rels, out: &mut Blocks, depth: usize) {
        let Some(content) = e.child("w:sdtContent") else { return };
        let mut ctl = crate::read::sdt::sdt_pr(e.child("w:sdtPr"), true);
        ctl.end_pr_xml = crate::read::sdt::sdt_end_pr(e.child("w:sdtEndPr"));
        ctl.run_props = e.child("w:sdtPr").and_then(|pr| pr.child("w:rPr")).map(|r| self.pc.rpr(r));
        let mut inner = Blocks::new();
        self.read_blocks(sc, content, rels, &mut inner, depth + 1);
        // A table of contents' control is handed to the TOC later, when the writer puts it back
        // (`toc_control_to_toc`).
        if inner.is_empty() {
            out.extend(inner);
            return;
        }
        if ctl.showing_placeholder
            && let Some(Block::Para(p)) = inner.first().map(|b| &**b)
        {
            ctl.placeholder_text = placeholder_of(&p.plain_text());
        }
        if let Some(first) = inner.first_mut() {
            match Arc::make_mut(first) {
                Block::Para(p) => {
                    let props = p.props_of_char(0).clone();
                    let _ = p.insert_object(0, InlineObject::ControlStart { control: Box::new(ctl) }, &props);
                }
                Block::Table(t) => t.controls.open.insert(0, ctl),
            }
        }
        if let Some(last) = inner.last_mut() {
            match Arc::make_mut(last) {
                Block::Para(p) => {
                    let (at, props) = (p.len(), p.runs.last().map(|r| r.props.clone()).unwrap_or_else(|| p.mark.clone()));
                    let _ = p.insert_object(at, InlineObject::ControlEnd, &props);
                }
                Block::Table(t) => t.controls.close = t.controls.close.saturating_add(1),
            }
        }
        out.extend(inner);
    }

    /// Markers left over at the end of a story go to its last paragraph (after a final table:
    /// the last paragraph of its last cell, so a bookmark around the table keeps its end).
    /// A range limited to columns keeps its markers out of a final table (whose columns they
    /// don't name): they get the empty paragraph a save gives a list that ends with a table.
    pub fn flush_pending(&mut self, sc: &mut StoryCtx, out: &mut Blocks) {
        if sc.pending.is_empty() {
            return;
        }
        if out.last().is_some_and(|b| matches!(**b, Block::Table(_))) && column_marker_waits(sc) {
            out.push(para_block(Paragraph::new()));
        }
        if let Some(p) = last_para_mut(out, 0) {
            for o in sc.pending.drain(..) {
                let props = p.runs.last().map(|r| r.props.clone()).unwrap_or_default();
                p.text.push(craft_wordmodel::para::OBJ);
                p.objects.push(o);
                p.runs.push(Run { len: craft_wordmodel::para::OBJ.len_utf8(), props });
            }
            p.normalize();
        }
        sc.pending.clear();
    }

    /// Markers read before content placed whole (a chunk's `next`) stay before it: at the end of
    /// the paragraph before, or after a table or with none, at the start of the content's first
    /// paragraph, as a paragraph read there would take them (with a table first, the next
    /// paragraph read takes them).
    pub fn flush_pending_before(&mut self, sc: &mut StoryCtx, out: &mut Blocks, next: &mut [Block]) {
        if out.last().is_some_and(|b| matches!(**b, Block::Para(_))) {
            return self.flush_pending(sc, out);
        }
        if let Some(Block::Para(p)) = next.first_mut() {
            for o in sc.pending.drain(..).rev() {
                let _ = p.insert_object(0, o, &CharProps::default());
            }
        }
    }

    /// A range marker read in a paragraph or between blocks, or (`structural`) in a table's
    /// structure (`w:tbl`, `w:tr` or a wrapper there; see [`Self::read_table`]).
    pub(super) fn marker(&mut self, sc: &mut StoryCtx, e: &El, structural: bool) -> Option<InlineObject> {
        let id = e.attr("w:id").map(str::trim).unwrap_or("").to_string();
        match e.name.as_str() {
            "w:bookmarkStart" => {
                if self.bookmarks.len() >= 100_000 {
                    return None;
                }
                let name = e.attr("w:name").map(str::trim).unwrap_or("").to_string();
                if name.is_empty() || name.len() > 256 {
                    return None;
                }
                self.bookmarks.insert(id, name.clone());
                Some(InlineObject::BookmarkStart { name })
            }
            "w:bookmarkEnd" => self
                .bookmarks
                .get(&id)
                .cloned()
                .or_else(|| e.attr("w:name").map(str::trim).filter(|n| !n.is_empty() && n.len() <= 256).map(str::to_string))
                .map(|name| InlineObject::BookmarkEnd { name }),
            "w:commentRangeStart" => self.comment_id(&id).map(|id| InlineObject::CommentStart { id }),
            "w:permStart" => self.perm_start(sc, e, id, structural),
            "w:permEnd" => self.perm_end(sc, e, &id, structural),
            "w:commentRangeEnd" => {
                let cid = self.comment_id(&id)?;
                self.comments_ended.insert(cid);
                Some(InlineObject::CommentEnd { id: cid })
            }
            _ => None,
        }
    }

    pub(super) fn comment_id(&self, file_id: &str) -> Option<u32> {
        self.comment_map.get(file_id).copied()
    }

    fn read_para(&mut self, sc: &mut StoryCtx, e: &El, rels: &Rels, depth: usize) -> Paragraph {
        let mut pb = PB::default();
        let mut para = Paragraph::new();
        let mut mark_moves: [Option<&El>; 2] = [None, None];
        self.moves.closed_here = [None, None];
        for o in std::mem::take(&mut sc.pending) {
            pb.push_obj(o, &CharProps::default());
        }
        if let Some(ppr) = e.child("w:pPr") {
            let (mut props, mark) = self.pc.ppr(ppr);
            crate::read::props::resolve_level_only(&self.doc.styles, &mut props, "Normal");
            para.props = props;
            para.mark = mark;
            // A tracked paragraph mark (ECMA-376 §17.13.5.16 / §17.13.5.15): `w:ins` / `w:del`
            // in the mark's `w:rPr`.
            if let Some(rpr) = ppr.child("w:rPr") {
                for k in rpr.els() {
                    match k.name.as_str() {
                        "w:ins" => para.mark.ins = Some(self.revision(RevisionKind::Insert, k)),
                        "w:del" => para.mark.del = Some(self.revision(RevisionKind::Delete, k)),
                        // A moved paragraph mark (§17.13.5.21, §17.13.5.24): its move range
                        // usually starts in the paragraph's content, so it is read after that.
                        "w:moveTo" => mark_moves[1] = Some(k),
                        "w:moveFrom" => mark_moves[0] = Some(k),
                        _ => {}
                    }
                }
                para.mark.fmt_change = self.rpr_change(rpr);
            }
            // Tracked formatting changes (§17.13.5.29, §17.13.5.19): the paragraph properties
            // before the change, and a change of the list numbering.
            if let Some(ch) = ppr.child("w:pPrChange") {
                let old = ch
                    .child("w:pPr")
                    .map(|o| {
                        let mut old = self.pc.ppr(o).0;
                        crate::read::props::resolve_level_only(&self.doc.styles, &mut old, "Normal");
                        old.formatting()
                    })
                    .unwrap_or_default();
                para.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
            if let Some(ch) = ppr.child("w:numPr").and_then(|n| n.child("w:numberingChange")) {
                // Multi-level numbering caches (`w:original`) routinely exceed 256 chars while staying
                // bounded against hostile attributes.
                const MAX_CHANGE_ORIGINAL: usize = 4096;
                let original = ch.attr("w:original").unwrap_or("").chars().take(MAX_CHANGE_ORIGINAL).collect();
                para.props.num_change = Some(Box::new(NumChange { rev: self.revision(RevisionKind::Format, ch), original }));
            }
            if let Some(s) = ppr.child("w:sectPr") {
                para.section = Some(Box::new(self.read_section(s, rels)));
            }
        }
        let ctx = RunCtx::default();
        self.read_inline_children(sc, &mut pb, e, rels, &ctx, depth + 1);
        self.spill_fields(sc, &mut pb);
        if let Some(k) = mark_moves[0] {
            para.mark.del = Some(self.move_revision(false, k, true));
        }
        if let Some(k) = mark_moves[1] {
            para.mark.ins = Some(self.move_revision(true, k, true));
        }
        para.text = pb.text;
        para.runs = pb.runs;
        para.objects = pb.objects;
        // Paragraph boundaries inside an instruction are hidden with the field code.
        // The result paragraph still takes a line, even when its cached text is empty.
        if sc.fields.iter().any(|f| f.phase == Phase::Instr) && para.plain_text().is_empty() {
            para.mark.hidden = Some(true);
        }
        // A cell's paragraphs are marked once their table is read: its style counts.
        if !sc.in_cell {
            self.doc.styles.mark_runs_as_word_draws_them(&mut para, None);
        }
        para.normalize();
        para
    }

    /// A tracked run formatting change in `rpr` (`w:rPrChange`, §17.13.5.31): the formatting
    /// before it. A change nested inside it is ignored.
    pub(super) fn rpr_change(&mut self, rpr: &El) -> Option<Box<PropChange<CharProps>>> {
        let ch = rpr.child("w:rPrChange")?;
        let old = ch.child("w:rPr").map(|o| self.pc.rpr(o).formatting()).unwrap_or_default();
        PropChange::boxed(self.revision(RevisionKind::Format, ch), old)
    }

    pub fn read_section(&mut self, s: &El, rels: &Rels) -> craft_wordmodel::SectionProps {
        let (mut sp, refs) = sectpr(&self.pc, s);
        // §17.13.5.32: the section's properties before a tracked change (no header references).
        if let Some(ch) = s.child("w:sectPrChange") {
            let old = ch.child("w:sectPr").map(|o| sectpr(&self.pc, o).0).unwrap_or_default();
            sp.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
        }
        for r in refs {
            let Some(rel) = rels.by_id(&r.rid) else { continue };
            if rel.external {
                continue;
            }
            let target = rel.target.clone();
            let Some(id) = self.load_header_footer(&target, r.footer) else { continue };
            let set = if r.footer { &mut sp.footers } else { &mut sp.headers };
            match r.kind.as_str() {
                "first" => set.first = Some(id),
                "even" => set.even = Some(id),
                _ => set.default = Some(id),
            }
        }
        sp
    }

    // ---- tables ----

    /// Word gives a table no cell margins when the file has no default table style and neither the
    /// table nor its style sets them; saving may write a default Normal Table, so the zeros are
    /// kept as the table's own. shortcut: they stay direct formatting (a table style applied later
    /// keeps them), until the model knows the file's default table style. A `w:tblCellMar` that sets
    /// only some sides keeps them: only the sides it omits become 0.
    fn no_default_margins(&self, props: &mut craft_wordmodel::props::TableProps) {
        if self.no_default_table_style
            && props.cell_margins.is_none()
            && props.style.as_deref().and_then(|id| self.doc.styles.table_style(id)).is_none_or(|s| s.parts.cell_margins.is_none())
        {
            props.cell_margins = Some([0.0; 4]);
        }
    }

    /// Omitted table-margin sides inherit from that property set's style.
    fn table_margin_base(&self, style: Option<&str>) -> [f32; 4] {
        style.and_then(|id| self.doc.styles.table_style(id)).and_then(|st| st.parts.cell_margins).unwrap_or(if self.no_default_table_style {
            [0.0; 4]
        } else {
            crate::read::props::WORD_CELL_MARGINS
        })
    }

    /// Markers pending at the end of a cell's content end it, in its last paragraph (see
    /// [`Self::flush_pending`]). A cell with no paragraph there to take them (none at all, or a
    /// table last whose last cell has none) gets an empty one at its end, as saving gives it.
    fn end_cell(&mut self, sc: &mut StoryCtx, blocks: &mut Blocks) {
        if !sc.pending.is_empty() && last_para_mut(blocks, 0).is_none() {
            blocks.push(para_block(Paragraph::new()));
        }
        self.flush_pending(sc, blocks);
    }

    /// A table. A start of a range limited to columns, waiting for a paragraph, stays out of it:
    /// its columns name the table it was read in (or none, read before a table at a story's own
    /// level). It waits, with any end of its own waiting after it, for the next paragraph at its
    /// own level. Other markers keep their place in the text: the table's first paragraph takes
    /// them.
    fn read_table(&mut self, sc: &mut StoryCtx, t: &El, rels: &Rels, depth: usize) -> Option<Table> {
        let held = take_column_starts(&mut sc.pending);
        for o in &held {
            if let InlineObject::PermStart { id, .. } = o {
                self.relocated_perms.insert(*id);
            }
        }
        let table = self.read_table_body(sc, t, rels, depth);
        if !held.is_empty() {
            let after = std::mem::replace(&mut sc.pending, held);
            sc.pending.extend(after);
        }
        table
    }

    fn read_table_body(&mut self, sc: &mut StoryCtx, t: &El, rels: &Rels, depth: usize) -> Option<Table> {
        let mut table = Table::default();
        if let Some(p) = t.child("w:tblPr") {
            table.props = self.pc.tblpr(p);
            // Sides `w:tblCellMar` leaves out keep the style's margins; without a style, Word's
            // 0.08" left and right, or none in a file with no default table style.
            if let Some(m) = p.child("w:tblCellMar") {
                let base = self.table_margin_base(table.props.style.as_deref());
                table.props.cell_margins = Some(crate::read::props::margins_over(m, base));
            }
            if let Some(ch) = p.child("w:tblPrChange") {
                let mut old = ch
                    .child("w:tblPr")
                    .map(|o| {
                        let mut props = self.pc.tblpr(o);
                        if let Some(m) = o.child("w:tblCellMar") {
                            let base = self.table_margin_base(props.style.as_deref());
                            props.cell_margins = Some(crate::read::props::margins_over(m, base));
                        }
                        props
                    })
                    .unwrap_or_default();
                // Rejecting the change brings back these properties: with the same margins.
                self.no_default_margins(&mut old);
                table.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
        }
        self.no_default_margins(&mut table.props);
        if let Some(g) = t.child("w:tblGrid") {
            table.grid = g
                .children("w:gridCol")
                .take(MAX_COLS)
                .map(|c| crate::units::tw(c, "w:w").unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT))
                .collect();
        }
        let number = sc.tables_read;
        sc.tables_read = number.saturating_add(1);
        sc.open_tables.push(TableFrame::new(number));
        // The columns the file gives the table, before the reader's limit: its grid's, or its
        // widest row's.
        let mut grids = Grids { rows: Vec::new(), file_cols: t.child("w:tblGrid").map_or(0, |g| g.children("w:gridCol").count() as u64) };
        let kept = self.read_rows(sc, t, rels, depth, &mut table, &mut grids);
        sc.open_tables.pop();
        if !kept {
            self.check_columns(sc, number, None, &grids);
            return None;
        }
        if table.grid.is_empty() {
            // A file with an empty `w:tblGrid` keeps the inch per column it always had.
            table.grid = if t.child("w:tblGrid").is_none() { grid_from_cells(&table) } else { vec![72.0; table.cols().clamp(1, MAX_COLS)] };
        }
        self.check_columns(sc, number, Some(&mut table), &grids);
        // The cells' paragraphs, with the run formatting the table style gives each cell (a
        // table inside has marked its own).
        let chrs = self.doc.styles.table_cells_chr(&table);
        for (ri, row) in table.rows.iter_mut().enumerate() {
            for (ci, cell) in row.cells.iter_mut().enumerate() {
                let chr = chrs.as_ref().and_then(|c| c.get(ri)?.get(ci));
                for b in &mut cell.blocks {
                    if let Block::Para(p) = Arc::make_mut(b) {
                        self.doc.styles.mark_runs_as_word_draws_them(p, chr);
                        p.normalize();
                    }
                }
            }
        }
        Some(table)
    }

    /// A table's rows, and the comment and permission range markers between them and between a
    /// row's cells, in document order: those before a cell go to its first paragraph, as a
    /// paragraph takes the markers before it; those after a row's last cell, or after the last
    /// row, end that cell; a permission range's end between rows ends the row before
    /// ([`Self::send_back`]). The grids of rows read once a column range has started go to
    /// `grids`. Whether a row was kept: with none, the markers go on to what follows the table.
    fn read_rows(&mut self, sc: &mut StoryCtx, t: &El, rels: &Rels, depth: usize, table: &mut Table, grids: &mut Grids) -> bool {
        let mut rows = Vec::new();
        collect(t, "w:tr", &mut rows, &mut Vec::new(), 0);
        let mut lead = Vec::new();
        let (mut row_count, mut file_row) = (0, 0);
        for (tr, row_controls) in rows {
            if tr.name != "w:tr" {
                set_at(sc, 2 * file_row);
                lead.extend(self.marker(sc, tr, true));
                continue;
            }
            self.send_back(sc, &mut lead, table);
            set_at(sc, 2 * file_row + 1);
            let file = file_row;
            file_row += 1;
            if row_count >= MAX_ROWS {
                continue;
            }
            row_count += 1;
            let mut row = Row { props: tr.child("w:trPr").map(trpr).unwrap_or_default(), cells: Vec::new(), controls: row_controls };
            row.props.exceptions = tr.child("w:tblPrEx").map(|e| Box::new(self.pc.tblpr(e)));
            if let Some(ch) = tr.child("w:trPr").and_then(|p| p.child("w:trPrChange")) {
                let old = ch.child("w:trPr").map(trpr).unwrap_or_default();
                row.props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
            let mut grid = RowGrid { file, kept: table.rows.len(), cells: Vec::new(), kept_cells: Vec::new() };
            self.read_cells(sc, tr, rels, depth, &mut lead, (&mut row, &mut grid, &mut grids.file_cols));
            // A row with no cells isn't kept: its markers wait for the next cell.
            let Some(last) = row.cells.last_mut() else { continue };
            sc.pending.append(&mut lead);
            self.end_cell(sc, &mut last.blocks);
            table.rows.push(row);
            if sc.open_tables.last().is_some_and(|f| f.columns) {
                grids.rows.push(grid);
            }
        }
        set_at(sc, 2 * file_row);
        // Markers after the last row end the table's last cell; with no row kept, they go on to
        // what follows the table.
        sc.pending.append(&mut lead);
        let Some(last) = table.rows.last_mut().and_then(|r| r.cells.last_mut()) else { return false };
        self.end_cell(sc, &mut last.blocks);
        true
    }

    /// A row's cells, the markers among them going to `lead`; with its grid as the file gives it
    /// (`grid`), and the widest the file's rows are so far (`file_cols`).
    fn read_cells(
        &mut self,
        sc: &mut StoryCtx,
        tr: &El,
        rels: &Rels,
        depth: usize,
        lead: &mut Vec<InlineObject>,
        into: (&mut Row, &mut RowGrid, &mut u64),
    ) {
        let (row, grid, file_cols) = into;
        let mut tcs = Vec::new();
        collect(tr, "w:tc", &mut tcs, &mut Vec::new(), 0);
        let tr_pr = tr.child("w:trPr");
        let before = file_count(tr_pr, "w:gridBefore", 0);
        // Where the next cell starts, and how wide the row is, as the file grids it.
        let mut at = before;
        let mut row_cols = before.saturating_add(file_count(tr_pr, "w:gridAfter", 0));
        for (tc, cell_controls) in tcs {
            if tc.name != "w:tc" {
                lead.extend(self.marker(sc, tc, true));
                continue;
            }
            let span = file_count(tc.child("w:tcPr"), "w:gridSpan", 1).max(1);
            let first = at;
            at = at.saturating_add(span);
            row_cols = row_cols.saturating_add(span);
            *file_cols = (*file_cols).max(row_cols);
            let ord = grid.cells.len();
            if ord >= MAX_COLS {
                continue;
            }
            grid.cells.push((first, span));
            let (mut props, hcont) = tc
                .child("w:tcPr")
                .map(|p| tcpr(&self.pc, p))
                .unwrap_or_else(|| (craft_wordmodel::props::CellProps { span: 1, ..Default::default() }, false));
            if let Some(ch) = tc.child("w:tcPr").and_then(|p| p.child("w:tcPrChange")) {
                let old = ch.child("w:tcPr").map(|o| tcpr(&self.pc, o).0).unwrap_or_default();
                props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
            }
            // Markers before a skipped cell wait for the next one, or end the row. In the file, too,
            // the cell is part of the cell before it.
            if hcont && let Some(prev) = row.cells.last_mut() {
                prev.props.span = (prev.props.span + props.span.max(1)).min(63);
                if let Some(merged) = grid.kept_cells.last().and_then(|c| grid.cells.get_mut(*c)) {
                    merged.1 = merged.1.saturating_add(span);
                }
                continue;
            }
            let mut blocks = Blocks::new();
            let outer = std::mem::replace(&mut sc.in_cell, true);
            sc.pending.append(lead);
            self.read_blocks(sc, tc, rels, &mut blocks, depth + 4);
            self.end_cell(sc, &mut blocks);
            sc.in_cell = outer;
            row.cells.push(Cell { props, blocks, controls: cell_controls });
            grid.kept_cells.push(ord);
        }
    }

    /// The permission ranges' ends waiting in `lead` between rows, or in a row with no cells, end
    /// the row before, at the end of its last cell: that row is the last the range covers in the
    /// file, and the next row's first cell would give a range limited to columns a row more. An
    /// end whose own start waits with it waits on with it (the range covers no row). The text
    /// each range covers is the same either way.
    fn send_back(&mut self, sc: &mut StoryCtx, lead: &mut Vec<InlineObject>, table: &mut Table) {
        if !lead.iter().any(|o| matches!(o, InlineObject::PermEnd { .. })) {
            return;
        }
        let Some(last) = table.rows.last_mut().and_then(|r| r.cells.last_mut()) else { return };
        let mut starts = HashSet::new();
        let (back, wait): (Vec<_>, Vec<_>) = std::mem::take(lead).into_iter().partition(|o| match o {
            InlineObject::PermStart { id, .. } => {
                starts.insert(*id);
                false
            }
            InlineObject::PermEnd { id } => !starts.contains(id),
            _ => false,
        });
        *lead = wait;
        let pending = std::mem::replace(&mut sc.pending, back);
        self.end_cell(sc, &mut last.blocks);
        sc.pending = pending;
    }
}

/// Where the reader is among the rows of the table it reads (see [`TableFrame::at`]).
fn set_at(sc: &mut StoryCtx, at: usize) {
    if let Some(frame) = sc.open_tables.last_mut() {
        frame.at = at;
    }
}

/// The narrowest column Word rebuilds for a table without `w:tblGrid`: 26 twips, for a cell of
/// 10 twips and for 10 twips left over by a spanning cell alike (measured: Word re-saves both as
/// `w:gridCol w:w="26"`). Content still widens a column when laid out.
const MIN_GRID_COL: f32 = 1.3;

/// The column grid of a table without `w:tblGrid` (a file written by a template library), the
/// way Word rebuilds it on opening: each column as wide as the widest preferred width
/// (`w:tcW` in twips) among the cells that occupy it alone, a spanning cell's width shared out
/// among its columns that no cell sets, what it still lacks going to its last column, and an inch
/// for a column no cell sizes at all; never narrower than [`MIN_GRID_COL`]. Spanning cells are
/// taken from the left, whatever row they are in (measured: Word gives the same grid in any row
/// order, and never narrows a column for a spanning cell).
fn grid_from_cells(table: &Table) -> Vec<f32> {
    let cols = table.cols().clamp(1, MAX_COLS);
    let mut grid: Vec<Option<f32>> = vec![None; cols];
    let mut spanning = Vec::new();
    for row in &table.rows {
        // After `w:gridBefore` the row's first cell is not in the first column.
        let mut g = row.first_grid();
        for cell in &row.cells {
            let span = cell.span();
            if let Some(w) = cell.props.width.filter(|w| w.is_finite() && *w > 0.0) {
                // The narrowest column counts before a span shares out what it lacks.
                let w = w.max(MIN_GRID_COL);
                if span == 1 {
                    if let Some(slot) = grid.get_mut(g) {
                        *slot = Some(slot.map_or(w, |cur| cur.max(w)));
                    }
                } else {
                    spanning.push((g, span, w));
                }
            }
            g += span;
        }
    }
    spanning.sort_by(|a: &(usize, usize, f32), b| (a.0 + a.1, a.0).cmp(&(b.0 + b.1, b.0)).then(a.2.total_cmp(&b.2)));
    for (g, span, w) in spanning {
        let end = (g + span).min(cols);
        let Some(cs) = grid.get_mut(g..end) else { continue };
        // A span cut off at the last column the reader keeps sizes only its kept columns, by
        // their share of it, and has no last column to give the rest to.
        let clipped = end < g + span;
        let w = if clipped { w * cs.len() as f32 / span as f32 } else { w };
        let known: f32 = cs.iter().flatten().sum();
        let unknown = cs.iter().filter(|c| c.is_none()).count();
        if unknown > 0 {
            let each = ((w - known) / unknown as f32).max(MIN_GRID_COL);
            for c in cs.iter_mut().filter(|c| c.is_none()) {
                *c = Some(each);
            }
        }
        let lacking = w - cs.iter().flatten().sum::<f32>();
        if lacking > 0.0
            && !clipped
            && let Some(Some(last)) = cs.last_mut()
        {
            *last += lacking;
        }
    }
    grid.into_iter().map(|w| w.unwrap_or(72.0).clamp(MIN_GRID_COL, crate::units::MAX_LEN_PT)).collect()
}

/// A count in an element's `w:val` (`w:gridSpan`, `w:gridBefore`, `w:gridAfter`) as the file gives
/// it, before the reader's limits (the last such element counts, as the reader takes it):
/// `default` when it's not there or not a count.
fn file_count(parent: Option<&El>, name: &str, default: u64) -> u64 {
    parent.and_then(|p| p.children(name).last()).and_then(|e| e.attr("w:val")).and_then(u32_of).map_or(default, u64::from)
}

/// The starts of ranges limited to columns waiting in `pending`, and any end waiting after its own
/// start, taken out in order (see [`Reader::read_table`]).
fn take_column_starts(pending: &mut Vec<InlineObject>) -> Vec<InlineObject> {
    if !pending.iter().any(|o| matches!(o, InlineObject::PermStart { cols: Some(_), .. })) {
        return Vec::new();
    }
    let mut ids = HashSet::new();
    let (held, rest) = std::mem::take(pending).into_iter().partition(|o| match o {
        InlineObject::PermStart { id, cols: Some(_), .. } => {
            ids.insert(*id);
            true
        }
        InlineObject::PermEnd { id } => ids.contains(id),
        _ => false,
    });
    *pending = rest;
    held
}
