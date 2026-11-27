use std::collections::HashSet;

use craft_wordmodel::walk::{each_para, each_para_mut};

use super::{
    Arc, Block, Blocks, Cell, CharProps, El, InlineObject, MAX_COLS, MAX_ROWS, MAX_TABLE_DEPTH, Node, NumChange, OpenPerm, PB, Paragraph, Phase,
    PropChange, Reader, Rels, RevisionKind, Row, Run, RunCtx, StoryCtx, Table, children_of_choice, clean_text, collect, last_para_mut, para_block,
    placeholder_of, sectpr, tcpr, trpr,
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
    pub fn flush_pending(&mut self, sc: &mut StoryCtx, out: &mut Blocks) {
        if sc.pending.is_empty() {
            return;
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
            "w:permEnd" => self.perm_end(sc, &id, structural),
            "w:commentRangeEnd" => {
                let cid = self.comment_id(&id)?;
                self.comments_ended.insert(cid);
                Some(InlineObject::CommentEnd { id: cid })
            }
            _ => None,
        }
    }

    /// A `w:permStart`. Each start gets an id of its own, and the story's open starts are kept by
    /// the file's `w:id`, so that an end closes the latest of them, as
    /// [`craft_wordmodel::Document::perm_ranges`] pairs them. Dropped, so that nothing of it can
    /// widen what is editable:
    /// - a start whose columns are there but aren't column numbers ([`perm_cols`]), and its end;
    /// - one in a table's structure while another start with its `w:id` is open: which end closes
    ///   which is unclear. An open start there is dropped too, when the read ends, as is one there
    ///   that no end closes ([`Self::finish_perms`]). Such starts were dropped before markers there
    ///   were read; a start in a paragraph keeps the end it had then.
    fn perm_start(&mut self, sc: &mut StoryCtx, e: &El, id: String, structural: bool) -> Option<InlineObject> {
        let open = sc.perms_open.entry(id.clone()).or_default();
        // A start in a table's structure is only ever pushed onto an empty list, so it is first.
        if let Some(OpenPerm { id: Some(first), structural: true }) = open.first() {
            self.perms_dropped.insert(*first);
        }
        if structural && !open.is_empty() {
            return None;
        }
        let Ok(cols) = perm_cols(e) else {
            open.push(OpenPerm { id: None, structural });
            return None;
        };
        // Bounded: a hostile file can't grow the map without end.
        if self.perms.len() >= 100_000 || (structural && self.table_perms.len() >= 100_000) {
            return None;
        }
        let n = self.next_perm;
        self.next_perm = n.saturating_add(1);
        self.perms.insert(id, n);
        let text = |a: &str| e.attr(a).filter(|v| v.len() <= 256).unwrap_or("").to_string();
        let (group, editor) = (text("w:edGrp"), text("w:ed"));
        if group.is_empty() && editor.is_empty() {
            return None;
        }
        if structural {
            self.table_perms.insert(n);
        }
        open.push(OpenPerm { id: Some(n), structural });
        Some(InlineObject::PermStart { id: n, group, editor, cols })
    }

    /// A `w:permEnd`: it closes the latest open start of its `w:id` in this story. With none open,
    /// one in a paragraph or between blocks keeps the id it had before (it opens nothing); one in
    /// a table's structure, never read before, is dropped.
    fn perm_end(&mut self, sc: &mut StoryCtx, id: &str, structural: bool) -> Option<InlineObject> {
        match sc.perms_open.get_mut(id).and_then(Vec::pop) {
            Some(OpenPerm { id, .. }) => id.map(|id| InlineObject::PermEnd { id }),
            None if structural => None,
            None => self.perms.get(id).map(|id| InlineObject::PermEnd { id: *id }),
        }
    }

    /// When the read ends, drop, start and end, the permission ranges [`Self::perm_start`] and
    /// [`Self::read_table`] marked, and the starts read in a table's structure that no end after
    /// them in their story closes.
    pub(crate) fn finish_perms(&mut self) {
        if self.table_perms.is_empty() && self.perms_dropped.is_empty() {
            return;
        }
        let mut closed = HashSet::new();
        let table_perms = &self.table_perms;
        let mut visit = |blocks: &Blocks| {
            let mut started = HashSet::new();
            for b in blocks {
                each_para(b, 0, &mut |p| {
                    for o in &p.objects {
                        match o {
                            InlineObject::PermStart { id, .. } if table_perms.contains(id) => {
                                started.insert(*id);
                            }
                            InlineObject::PermEnd { id } if started.contains(id) => {
                                closed.insert(*id);
                            }
                            _ => {}
                        }
                    }
                });
            }
        };
        visit(&self.doc.body);
        self.doc.parts.values().for_each(|part| visit(&part.blocks));
        self.doc.glossary.iter().for_each(|entry| visit(&entry.blocks));
        let mut gone = std::mem::take(&mut self.perms_dropped);
        gone.extend(self.table_perms.iter().filter(|id| !closed.contains(*id)));
        if gone.is_empty() {
            return;
        }
        let drop_in = |blocks: &mut Blocks| {
            for b in blocks.iter_mut() {
                each_para_mut(Arc::make_mut(b), 0, &mut |p| {
                    let at: Vec<usize> = p
                        .objects_with_offsets()
                        .filter(|(_, o)| matches!(o, InlineObject::PermStart { id, .. } | InlineObject::PermEnd { id } if gone.contains(id)))
                        .map(|(off, _)| off)
                        .collect();
                    for off in at.into_iter().rev() {
                        let _ = p.delete(off, off + craft_wordmodel::para::OBJ.len_utf8());
                    }
                });
            }
        };
        drop_in(&mut self.doc.body);
        self.doc.parts.values_mut().for_each(|part| drop_in(&mut part.blocks));
        self.doc.glossary.iter_mut().for_each(|entry| drop_in(&mut entry.blocks));
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
    /// table last whose last cell has none) gets an empty one at its end, as saving gives it; so
    /// does one that ends with a table when a permission start waits, which stays in its own
    /// table (see [`Self::read_table`]).
    fn end_cell(&mut self, sc: &mut StoryCtx, blocks: &mut Blocks) {
        let table_last = blocks.last().is_some_and(|b| matches!(**b, Block::Table(_)));
        let start = sc.pending.iter().any(|o| matches!(o, InlineObject::PermStart { .. }));
        if !sc.pending.is_empty() && (last_para_mut(blocks, 0).is_none() || (table_last && start)) {
            blocks.push(para_block(Paragraph::new()));
        }
        self.flush_pending(sc, blocks);
    }

    /// A table. A permission start waiting for a paragraph in a cell stays in that cell's table,
    /// whose columns it names: a table nested in the cell doesn't take it, and it waits, with any
    /// end of its own waiting after it, for the cell's own next paragraph.
    fn read_table(&mut self, sc: &mut StoryCtx, t: &El, rels: &Rels, depth: usize) -> Option<Table> {
        let held = if sc.in_cell { take_perm_starts(&mut sc.pending) } else { Vec::new() };
        let table = self.read_table_body(sc, t, rels, depth);
        if !held.is_empty() {
            let after = std::mem::replace(&mut sc.pending, held);
            sc.pending.extend(after);
        }
        table
    }

    /// A table the reader cut down to `kept` columns, fewer than the file gives it: a permission
    /// range of its own (started in one of its cells' paragraphs; a nested table's ranges are that
    /// table's) that names a column past them is cut to them, or dropped, start and end, when it
    /// names none of them. Otherwise it would name columns the table no longer has, and the
    /// standard would apply it to the entire table (§17.13.7.2).
    fn fit_perm_cols(&mut self, table: &mut Table, kept: usize) {
        let last = u32::try_from(kept.saturating_sub(1)).unwrap_or(u32::MAX);
        for b in table.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).flat_map(|c| c.blocks.iter_mut()) {
            if !matches!(**b, Block::Para(_)) {
                continue;
            }
            let Block::Para(p) = Arc::make_mut(b) else { continue };
            for o in &mut p.objects {
                if let InlineObject::PermStart { id, cols: Some([first, end]), .. } = o
                    && (*first).max(*end) > last
                {
                    if (*first).min(*end) > last {
                        self.perms_dropped.insert(*id);
                    } else {
                        *first = (*first).min(last);
                        *end = (*end).min(last);
                    }
                }
            }
        }
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
        // The columns the file gives the table, before the reader's limit: its grid's, or its
        // widest row's.
        let mut file_cols = t.child("w:tblGrid").map_or(0, |g| g.children("w:gridCol").count());
        let mut rows = Vec::new();
        collect(t, "w:tr", &mut rows, &mut Vec::new(), 0);
        // Comment and permission range markers between the rows and between a row's cells, in
        // document order: those before a cell go to its first paragraph, as a paragraph takes the
        // markers before it; those after a row's last cell, or after the last row, end that cell.
        let mut lead = Vec::new();
        let mut row_count = 0;
        for (tr, row_controls) in rows {
            if tr.name != "w:tr" {
                lead.extend(self.marker(sc, tr, true));
                continue;
            }
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
            let mut tcs = Vec::new();
            collect(tr, "w:tc", &mut tcs, &mut Vec::new(), 0);
            let tr_pr = tr.child("w:trPr");
            let mut row_cols = file_count(tr_pr, "w:gridBefore", 0).saturating_add(file_count(tr_pr, "w:gridAfter", 0));
            let mut cell_count = 0;
            for (tc, cell_controls) in tcs {
                if tc.name != "w:tc" {
                    lead.extend(self.marker(sc, tc, true));
                    continue;
                }
                row_cols = row_cols.saturating_add(file_count(tc.child("w:tcPr"), "w:gridSpan", 1).max(1));
                file_cols = file_cols.max(row_cols);
                if cell_count >= MAX_COLS {
                    continue;
                }
                cell_count += 1;
                let (mut props, hcont) = tc
                    .child("w:tcPr")
                    .map(|p| tcpr(&self.pc, p))
                    .unwrap_or_else(|| (craft_wordmodel::props::CellProps { span: 1, ..Default::default() }, false));
                if let Some(ch) = tc.child("w:tcPr").and_then(|p| p.child("w:tcPrChange")) {
                    let old = ch.child("w:tcPr").map(|o| tcpr(&self.pc, o).0).unwrap_or_default();
                    props.fmt_change = PropChange::boxed(self.revision(RevisionKind::Format, ch), old);
                }
                // Markers before a skipped cell wait for the next one, or end the row.
                if hcont && let Some(prev) = row.cells.last_mut() {
                    prev.props.span = (prev.props.span + props.span.max(1)).min(63);
                    continue;
                }
                let mut blocks = Blocks::new();
                let outer = std::mem::replace(&mut sc.in_cell, true);
                sc.pending.append(&mut lead);
                self.read_blocks(sc, tc, rels, &mut blocks, depth + 4);
                self.end_cell(sc, &mut blocks);
                sc.in_cell = outer;
                row.cells.push(Cell { props, blocks, controls: cell_controls });
            }
            // A row with no cells isn't kept: its markers wait for the next cell.
            let Some(last) = row.cells.last_mut() else { continue };
            sc.pending.append(&mut lead);
            self.end_cell(sc, &mut last.blocks);
            table.rows.push(row);
        }
        // Markers after the last row end the table's last cell; with no row kept, they go on to
        // what follows the table.
        sc.pending.append(&mut lead);
        let last = table.rows.last_mut().and_then(|r| r.cells.last_mut())?;
        self.end_cell(sc, &mut last.blocks);
        if table.grid.is_empty() {
            // A file with an empty `w:tblGrid` keeps the inch per column it always had.
            table.grid = if t.child("w:tblGrid").is_none() { grid_from_cells(&table) } else { vec![72.0; table.cols().clamp(1, MAX_COLS)] };
        }
        let kept = table.cols();
        if file_cols > kept {
            self.fit_perm_cols(&mut table, kept);
        }
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

/// A permission range's table columns (`w:colFirst`, `w:colLast`, ECMA-376-1:2016 §17.13.7.2),
/// zero-based, as the file gives them, so a save writes the same ones back: a pair in the wrong
/// order or past the table's last column stays as it is, for the code that enforces the range.
/// One of the two alone limits the range to that one column. `Err` when either is there but isn't
/// a column number the model holds: `ST_DecimalNumber` is an `xsd:integer`, and a negative one, or
/// one past `u32::MAX`, names no column. The range is then dropped, rather than read as one without
/// columns (which, in a table, applies to the entire table), and the other value isn't used alone.
fn perm_cols(e: &El) -> Result<Option<[u32; 2]>, ()> {
    let col = |a: &str| match e.attr(a) {
        None => Ok(None),
        Some(v) => v.trim_matches([' ', '\t', '\r', '\n']).parse::<i64>().ok().and_then(|n| u32::try_from(n).ok()).map(Some).ok_or(()),
    };
    Ok(match (col("w:colFirst")?, col("w:colLast")?) {
        (Some(first), Some(last)) => Some([first, last]),
        (Some(one), None) | (None, Some(one)) => Some([one, one]),
        (None, None) => None,
    })
}

/// A count in an element's `w:val` (`w:gridSpan`, `w:gridBefore`, `w:gridAfter`) as the file gives
/// it, before the reader's limits: `default` when it's not there or not a count.
fn file_count(parent: Option<&El>, name: &str, default: usize) -> usize {
    parent.and_then(|p| p.child(name)).and_then(|e| e.attr("w:val")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(default)
}

/// The permission starts waiting in `pending`, and any end waiting after its own start, taken out
/// in order (see [`Reader::read_table`]).
fn take_perm_starts(pending: &mut Vec<InlineObject>) -> Vec<InlineObject> {
    if !pending.iter().any(|o| matches!(o, InlineObject::PermStart { .. })) {
        return Vec::new();
    }
    let mut ids = HashSet::new();
    let (held, rest) = std::mem::take(pending).into_iter().partition(|o| match o {
        InlineObject::PermStart { id, .. } => {
            ids.insert(*id);
            true
        }
        InlineObject::PermEnd { id } => ids.contains(id),
        _ => false,
    });
    *pending = rest;
    held
}
