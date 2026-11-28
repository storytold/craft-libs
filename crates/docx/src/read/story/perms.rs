//! Permission ranges (Restrict Editing exceptions, ECMA-376-1:2016 §17.13.7.1–2) as a story is
//! read. The markers pair as the file pairs them ([`PermPairs`]), whatever the reader moves or
//! drops, and a range limited to table columns keeps its columns only while they authorise the
//! cells they authorise in the file ([`Reader::check_columns`]), so that what the reader moves and
//! drops doesn't make a range cover text or cells its pair of markers doesn't cover in the file.

use std::collections::{BTreeSet, HashMap, HashSet};

use craft_wordmodel::walk::{each_para, each_para_mut};

use super::{Arc, Block, Blocks, El, InlineObject, Reader, StoryCtx, Table, children_of_choice};

/// Most permission markers one story's pairing takes in; later ones are dropped when read.
const MAX_MARKS: usize = 200_000;

/// Most model ids one read gives out: to starts, and to ends that close none. Later starts are
/// dropped, and so are later ends that close none.
const MAX_PERMS: u32 = 100_000;

/// Most cells one read looks at to check its column ranges ([`Reader::check_columns`]); a range
/// past them is dropped.
pub(crate) const MAX_COLUMN_CHECKS: usize = 1 << 24;

/// The file's own pairing of a story's permission markers, made before the story is read: in
/// document order, an end closes the latest start before it with its `w:id` that no end has
/// closed yet, as [`craft_wordmodel::Document::perm_ranges`] pairs a model's. A text box's markers
/// are its own story's; of alternate content, only the branch the reader reads counts.
///
/// The reader gives each start an id of its own, and an end the id of the start the file pairs it
/// with, or one no start has. So in the model an end closes no start but its own, however the
/// reader moved or dropped the markers around it.
#[derive(Default)]
pub struct PermPairs {
    /// Each marker's place in document order, by its element's address in the parsed part (the
    /// tree doesn't change while its story is read).
    places: HashMap<usize, u32>,
    marks: Vec<Mark>,
}

/// A permission marker in [`PermPairs`].
#[derive(Clone, Copy, Default)]
struct Mark {
    /// The marker the file pairs it with: a start's end, an end's start.
    partner: Option<u32>,
    /// A start read while another start with its `w:id` is open, or open while another is read.
    contested: bool,
    /// A start the reader read: the model id it gave it.
    id: Option<u32>,
    /// A start the reader kept (not dropped for naming no editor and no group).
    kept: bool,
    /// A start the reader dropped with its end.
    dropped: bool,
}

impl PermPairs {
    /// The pairing of the markers under `root`, the element whose blocks are the story's.
    pub fn of(root: &El) -> PermPairs {
        let mut pairs = PermPairs::default();
        pairs.walk(root, &mut HashMap::new(), 0);
        pairs
    }

    fn walk<'a>(&mut self, e: &'a El, open: &mut HashMap<&'a str, Vec<u32>>, depth: usize) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        for k in e.els() {
            match k.name.as_str() {
                "w:permStart" | "w:permEnd" => self.mark(k, open),
                "w:txbxContent" => {}
                "mc:AlternateContent" => {
                    if let Some(c) = children_of_choice(k) {
                        self.walk(c, open, depth + 1);
                    }
                }
                _ => self.walk(k, open, depth + 1),
            }
        }
    }

    fn mark<'a>(&mut self, e: &'a El, open: &mut HashMap<&'a str, Vec<u32>>) {
        let Some(at) = u32::try_from(self.marks.len()).ok().filter(|_| self.marks.len() < MAX_MARKS) else { return };
        let stack = open.entry(e.attr("w:id").map(str::trim).unwrap_or("")).or_default();
        let mut mark = Mark::default();
        if e.name == "w:permStart" {
            // With two or more open, each is contested already.
            if let [only] = stack.as_slice()
                && let Some(m) = self.marks.get_mut(*only as usize)
            {
                m.contested = true;
            }
            mark.contested = !stack.is_empty();
            stack.push(at);
        } else if let Some(start) = stack.pop() {
            mark.partner = Some(start);
            if let Some(m) = self.marks.get_mut(start as usize) {
                m.partner = Some(at);
            }
        }
        self.places.insert(address(e), at);
        self.marks.push(mark);
    }

    fn place(&self, e: &El) -> Option<u32> {
        self.places.get(&address(e)).copied()
    }

    fn get(&self, at: u32) -> Option<Mark> {
        self.marks.get(at as usize).copied()
    }

    fn update(&mut self, at: u32, f: impl FnOnce(&mut Mark)) {
        if let Some(m) = self.marks.get_mut(at as usize) {
            f(m);
        }
    }
}

fn address(e: &El) -> usize {
    std::ptr::from_ref(e).addr()
}

/// A table being read (see [`StoryCtx::open_tables`]).
pub struct TableFrame {
    /// Its number among the story's tables.
    number: u32,
    /// Where the reader is among the rows the file gives it: `2k` before row `k` (between rows,
    /// or before the first), `2k + 1` in row `k`.
    pub at: usize,
    /// A column range started in it: its rows' grids are kept ([`RowGrid`]).
    pub columns: bool,
}

impl TableFrame {
    pub fn new(number: u32) -> TableFrame {
        TableFrame { number, at: 0, columns: false }
    }
}

/// A range limited to columns, as the file gives it: its columns, the table its start was read in
/// (none outside tables), and the first and last of that table's rows it covers.
pub struct ColumnRange {
    cols: [u32; 2],
    table: Option<u32>,
    /// The row its start was read in, or the row after it when read between rows.
    from: usize,
    /// When its end was read in that table: the row it was read in, or the row before it when
    /// read between rows (`None`: before the first row).
    to: Option<Option<usize>>,
}

/// A row of a table with a column range: as the file grids it, and as the reader keeps it.
pub struct RowGrid {
    /// Its place among the file's rows, and among the rows kept.
    pub file: usize,
    pub kept: usize,
    /// Each cell the reader takes in, by its place among the row's cells: the first grid column
    /// and the number of columns the file gives it.
    pub cells: Vec<(u64, u64)>,
    /// The file cell (its place in `cells`) of each cell kept.
    pub kept_cells: Vec<usize>,
}

/// The rows of a table with a column range, and the columns the file gives the table: its grid's,
/// or its widest row's.
#[derive(Default)]
pub struct Grids {
    pub rows: Vec<RowGrid>,
    pub file_cols: u64,
}

/// What becomes of a column range when its table is read.
enum Verdict {
    Keep,
    Cols([u32; 2]),
    Drop,
}

impl Reader<'_> {
    /// A `w:permStart`: it gets an id of its own. Dropped, so that nothing of it can widen what
    /// is editable:
    /// - one whose columns are there but aren't column numbers ([`perm_cols`]), and its end;
    /// - one read in a table's structure (`w:tbl`, `w:tr` or a wrapper there) that the file gives
    ///   no end, or while another start with its `w:id` is open (which end closes which is then
    ///   unclear), and its end. Such starts were dropped before markers there were read.
    pub(super) fn perm_start(&mut self, sc: &mut StoryCtx, e: &El, file_id: String, structural: bool) -> Option<InlineObject> {
        let at = sc.pairs.place(e)?;
        let mark = sc.pairs.get(at)?;
        let cols = match perm_cols(e) {
            Ok(cols) if !structural || (mark.partner.is_some() && !mark.contested) => cols,
            _ => {
                sc.pairs.update(at, |m| m.dropped = true);
                return None;
            }
        };
        if self.next_perm >= MAX_PERMS {
            return None;
        }
        let n = self.next_perm;
        self.next_perm += 1;
        self.perm_ids.insert(file_id);
        sc.pairs.update(at, |m| m.id = Some(n));
        let text = |a: &str| e.attr(a).filter(|v| v.len() <= 256).unwrap_or("").to_string();
        let (group, editor) = (text("w:edGrp"), text("w:ed"));
        if group.is_empty() && editor.is_empty() {
            return None;
        }
        sc.pairs.update(at, |m| m.kept = true);
        if structural {
            self.relocated_perms.insert(n);
        }
        if let Some(cols) = cols {
            let frame = sc.open_tables.last_mut();
            let (table, from) = frame.map_or((None, 0), |f| {
                f.columns = true;
                (Some(f.number), f.at / 2)
            });
            sc.columns.insert(n, ColumnRange { cols, table, from, to: None });
        }
        Some(InlineObject::PermStart { id: n, group, editor, cols })
    }

    /// A `w:permEnd`: the id of the start the file pairs it with, when the reader read that start
    /// (and didn't drop it with its end); then it closes that start and no other. One that closes
    /// no start read: in a table's structure, where nothing was read before, it is dropped; in a
    /// paragraph or between blocks it is kept as before, when the reader has read a start with its
    /// `w:id`, but under an id no start has, so it can't close one.
    pub(super) fn perm_end(&mut self, sc: &mut StoryCtx, e: &El, file_id: &str, structural: bool) -> Option<InlineObject> {
        let at = sc.pairs.place(e)?;
        let start = sc.pairs.get(at).and_then(|m| m.partner).and_then(|p| sc.pairs.get(p));
        match start {
            Some(Mark { dropped: true, .. }) => None,
            Some(Mark { id: Some(id), kept, .. }) if kept || !structural => {
                if let Some(range) = sc.columns.get_mut(&id) {
                    range.to = sc.open_tables.iter().find(|f| Some(f.number) == range.table).map(|f| (f.at / 2).checked_sub(1 - f.at % 2));
                }
                Some(InlineObject::PermEnd { id })
            }
            _ if structural || !self.perm_ids.contains(file_id) || self.next_perm >= MAX_PERMS => None,
            _ => {
                let n = self.next_perm;
                self.next_perm += 1;
                Some(InlineObject::PermEnd { id: n })
            }
        }
    }

    /// When the read ends, drop, start and end, the permission ranges [`Self::perm_start`] and
    /// [`Self::check_columns`] marked, and the starts the reader read in a table's structure or
    /// held past a table that no end after them in their story closes.
    pub(crate) fn finish_perms(&mut self) {
        if self.relocated_perms.is_empty() && self.perms_dropped.is_empty() {
            return;
        }
        let mut closed = HashSet::new();
        let relocated = &self.relocated_perms;
        let mut visit = |blocks: &Blocks| {
            let mut started = HashSet::new();
            for b in blocks {
                each_para(b, 0, &mut |p| {
                    for o in &p.objects {
                        match o {
                            InlineObject::PermStart { id, .. } if relocated.contains(id) => {
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
        gone.extend(self.relocated_perms.iter().filter(|id| !closed.contains(*id)));
        if gone.is_empty() {
            return;
        }
        let drop_in = |blocks: &mut Blocks| {
            for b in blocks.iter_mut() {
                let mut any = false;
                each_para(b, 0, &mut |p| any |= p.objects.iter().any(|o| is_perm_of(o, &gone)));
                if !any {
                    continue;
                }
                each_para_mut(Arc::make_mut(b), 0, &mut |p| {
                    let at: Vec<usize> = p.objects_with_offsets().filter(|(_, o)| is_perm_of(o, &gone)).map(|(off, _)| off).collect();
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

    /// When table `number` is read (`table`; `None` when it kept no row): each column range that
    /// started in it keeps its columns only while they authorise, in the model, the cells they
    /// authorise in the file that the reader kept (see [`ColumnRange`]), in the same rows, and
    /// name columns the table has exactly when they did in the file. The reader cuts rows, cells
    /// and columns past its limits, and spans and `w:gridBefore` with them, which moves cells to
    /// other columns. Failing that, the range takes the columns that authorise exactly those
    /// cells, when there are such; otherwise it is dropped, start and end. It is dropped too when
    /// its start isn't in one of the table's own cells, or its end is in the table when it wasn't
    /// in the file, or the other way round, or before its start.
    pub(super) fn check_columns(&mut self, sc: &mut StoryCtx, number: u32, table: Option<&mut Table>, grids: &Grids) {
        let mut ids: Vec<u32> = sc.columns.iter().filter(|(_, r)| r.table == Some(number)).map(|(id, _)| *id).collect();
        if ids.is_empty() {
            return;
        }
        ids.sort_unstable();
        let Some(table) = table else {
            self.perms_dropped.extend(ids);
            return;
        };
        let wanted: HashSet<u32> = ids.iter().copied().collect();
        let found = markers_in(table, &wanted);
        for id in ids {
            let Some(range) = sc.columns.get(&id) else { continue };
            let [start, end] = found.get(&id).copied().unwrap_or([None, None]);
            match column_verdict(table, grids, range, start, end, &mut self.column_checks) {
                Verdict::Keep => {}
                Verdict::Cols(cols) => set_cols(table, id, cols),
                Verdict::Drop => {
                    self.perms_dropped.insert(id);
                }
            }
        }
    }
}

fn is_perm_of(o: &InlineObject, ids: &HashSet<u32>) -> bool {
    matches!(o, InlineObject::PermStart { id, .. } | InlineObject::PermEnd { id } if ids.contains(id))
}

/// See [`Reader::check_columns`]. `start` and `end`: where the range's markers are in the table,
/// as (row, place in document order), the start in one of its cells' own paragraphs.
fn column_verdict(
    table: &Table,
    grids: &Grids,
    range: &ColumnRange,
    start: Option<(usize, usize)>,
    end: Option<(usize, usize)>,
    budget: &mut usize,
) -> Verdict {
    let Some((start_row, start_at)) = start else { return Verdict::Drop };
    let (to, end_row) = match (range.to, end) {
        (None, None) => return Verdict::Keep,
        (Some(to), Some((end_row, end_at))) if end_at > start_at => (to, end_row),
        _ => return Verdict::Drop,
    };
    let want = match to {
        Some(to) if to >= range.from => file_cells(grids, range.from, to, range.cols, budget),
        _ => Some(BTreeSet::new()),
    };
    let (Some(want), Some(got)) = (want, model_cells(table, start_row, end_row, range.cols, budget)) else { return Verdict::Drop };
    let [a, b] = range.cols;
    let names = |cols: u64| a <= b && u64::from(b) < cols;
    if got == want && names(grids.file_cols) == names(table.cols() as u64) {
        return Verdict::Keep;
    }
    let edges = want.iter().filter_map(|&(i, j)| {
        let c = table.rows.get(i)?.cells.get(j)?;
        let g = table.grid_col(i, j);
        Some((g, g + c.span() - 1))
    });
    let (Some(lo), Some(hi)) = edges
        .fold((None, None), |(lo, hi): (Option<usize>, Option<usize>), (g, e)| (Some(lo.map_or(g, |x| x.min(g))), Some(hi.map_or(e, |x| x.max(e)))))
    else {
        return Verdict::Drop;
    };
    let (Ok(lo), Ok(hi)) = (u32::try_from(lo), u32::try_from(hi)) else { return Verdict::Drop };
    match model_cells(table, start_row, end_row, [lo, hi], budget) {
        Some(got) if got == want => Verdict::Cols([lo, hi]),
        _ => Verdict::Drop,
    }
}

/// Whether a cell from grid column `g` over `span` columns is wholly inside columns `a` to `b`.
fn inside(g: u64, span: u64, [a, b]: [u32; 2]) -> bool {
    u64::from(a) <= g && g.saturating_add(span.max(1)) <= u64::from(b) + 1
}

/// The cells a range over columns `cols` authorises in the file's rows `from..=to`, as the file
/// grids them, that the reader kept: as (row, cell) in the model. `None` past the budget.
fn file_cells(grids: &Grids, from: usize, to: usize, cols: [u32; 2], budget: &mut usize) -> Option<BTreeSet<(usize, usize)>> {
    let mut out = BTreeSet::new();
    for row in grids.rows.iter().filter(|r| (from..=to).contains(&r.file)) {
        *budget = budget.checked_sub(row.kept_cells.len())?;
        for (j, c) in row.kept_cells.iter().enumerate() {
            if row.cells.get(*c).is_some_and(|&(g, span)| inside(g, span, cols)) {
                out.insert((row.kept, j));
            }
        }
    }
    Some(out)
}

/// The cells a range over columns `cols` authorises in the model's rows `from..=to`, as the model
/// grids them, from the row of the start to the row of the end. `None` past the budget.
fn model_cells(table: &Table, from: usize, to: usize, cols: [u32; 2], budget: &mut usize) -> Option<BTreeSet<(usize, usize)>> {
    let mut out = BTreeSet::new();
    for (i, row) in table.rows.iter().enumerate().take(to.saturating_add(1)).skip(from) {
        *budget = budget.checked_sub(row.cells.len())?;
        let mut g = row.first_grid();
        for (j, c) in row.cells.iter().enumerate() {
            if inside(g as u64, c.span() as u64, cols) {
                out.insert((i, j));
            }
            g += c.span();
        }
    }
    Some(out)
}

/// Where the starts and ends of the ranges `ids` are in `table`: a start in one of its cells' own
/// paragraphs (not in a table inside a cell), an end anywhere in it; as (row, place in document
/// order).
fn markers_in(table: &Table, ids: &HashSet<u32>) -> HashMap<u32, [Option<(usize, usize)>; 2]> {
    let mut out: HashMap<u32, [Option<(usize, usize)>; 2]> = HashMap::new();
    let mut at = 0;
    for (i, row) in table.rows.iter().enumerate() {
        for b in row.cells.iter().flat_map(|c| c.blocks.iter()) {
            let own = matches!(**b, Block::Para(_));
            each_para(b, 0, &mut |p| {
                for o in &p.objects {
                    at += 1;
                    match o {
                        InlineObject::PermStart { id, .. } if own && ids.contains(id) => out.entry(*id).or_default()[0] = Some((i, at)),
                        InlineObject::PermEnd { id } if ids.contains(id) => out.entry(*id).or_default()[1] = Some((i, at)),
                        _ => {}
                    }
                }
            });
        }
    }
    out
}

/// Give range `id`'s start, in one of `table`'s cells' own paragraphs, the columns `cols`.
fn set_cols(table: &mut Table, id: u32, cols: [u32; 2]) {
    let mine = |o: &InlineObject| matches!(o, InlineObject::PermStart { id: x, .. } if *x == id);
    for b in table.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).flat_map(|c| c.blocks.iter_mut()) {
        if !matches!(&**b, Block::Para(p) if p.objects.iter().any(mine)) {
            continue;
        }
        if let Block::Para(p) = Arc::make_mut(b) {
            for o in p.objects.iter_mut().filter(|o| mine(o)) {
                if let InlineObject::PermStart { cols: c, .. } = o {
                    *c = Some(cols);
                }
            }
            p.touch();
        }
        return;
    }
}

/// Whether a permission marker waiting in `sc.pending` is of a range limited to columns: its start,
/// or the end of one read.
pub(super) fn column_marker_waits(sc: &StoryCtx) -> bool {
    sc.pending.iter().any(|o| match o {
        InlineObject::PermStart { cols, .. } => cols.is_some(),
        InlineObject::PermEnd { id } => sc.columns.contains_key(id),
        _ => false,
    })
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
