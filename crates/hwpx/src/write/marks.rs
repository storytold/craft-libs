//! Which source paragraph each paragraph of an edited document is, for a write back.
//!
//! The file is read again; the reader notes the source paragraph (`section:ordinal`) of every
//! paragraph it makes ([`crate::read::Origins`]). The edited document's stories (the body, each
//! header, footer, note, comment and text box by its part id) are then lined up with the ones
//! read, block by block (tables and the paragraphs in their cells too):
//!
//! - blocks whose [`fingerprint`] is the same as one read, in the same order, are that block
//!   (unchanged);
//! - the blocks between two of those are paired with the ones read between the same two, in
//!   order, each pair counting for itself and for how alike its two blocks are ([`similarity`]):
//!   an edited paragraph is the one it was edited from, the rest are new or deleted;
//! - an unchanged block that is out of order (moved) is still the one it was;
//! - a paragraph read that is gone while its text starts or ends the paragraph its neighbour
//!   became was merged into it: that paragraph is both.
//!
//! A paragraph is known by its address while the documents are borrowed ([`Marks`]); nothing is
//! written into either document.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{HashMap, HashSet};

use craft_wordmodel::kept::{fingerprint, table_fingerprint};
use craft_wordmodel::para::{COLUMN_BREAK, NB_HYPHEN, OBJ, PAGE_BREAK, SOFT_HYPHEN};
use craft_wordmodel::{Block, Blocks, Document, InlineObject, Paragraph, PartKind, Table};

use super::back::{DocPara, Mark};
use crate::read::{Origins, first_para_rev};

/// Deepest table nesting followed.
const MAX_DEPTH: usize = 32;
/// Most pairs (blocks read × blocks edited) weighed one against another between two unchanged
/// blocks; past it they are paired in order (hostile sizes).
const MAX_GAP_CELLS: usize = 1_000_000;
/// Deepest the lining up splits a story at unchanged blocks; past it, what is left is paired as
/// one stretch.
const MAX_SPLITS: usize = 64;

/// The source marks of a document's paragraphs and tables, by address.
#[derive(Default)]
pub(crate) struct Marks {
    paras: HashMap<*const Paragraph, Vec<Mark>>,
    tables: HashMap<*const Table, Mark>,
}

impl Marks {
    /// Paragraph `p`'s marks ("hp" for a source paragraph's first piece, "hq" for the others).
    pub(crate) fn para(&self, p: &Paragraph) -> &[Mark] {
        self.paras.get(&std::ptr::from_ref(p)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The first piece's mark of paragraph `p`.
    pub(crate) fn first(&self, p: &Paragraph) -> Option<Mark> {
        self.para(p).iter().copied().find(|m| m.0 == "hp")
    }

    /// The mark ("ht") of the source paragraph shown only as table `t`.
    pub(crate) fn table(&self, t: &Table) -> Option<Mark> {
        self.tables.get(&std::ptr::from_ref(t)).copied()
    }
}

/// A block of a story, flattened: tables come before the blocks of their cells.
struct Item<'d> {
    block: &'d Block,
    /// In a table's cell.
    nested: bool,
    fp: u64,
    /// A paragraph's text as the write-back compares it ([`para_text`]).
    text: String,
}

impl Item<'_> {
    fn table(&self) -> bool {
        matches!(self.block, Block::Table(_))
    }

    fn kind(&self) -> (bool, bool) {
        (self.table(), self.nested)
    }
}

fn items<'d>(blocks: &'d Blocks, nested: bool, depth: usize, out: &mut Vec<Item<'d>>) {
    for b in blocks {
        match &**b {
            Block::Para(p) => out.push(Item { block: b, nested, fp: fingerprint(p), text: para_text(p).0 }),
            Block::Table(t) => {
                out.push(Item { block: b, nested, fp: table_fingerprint(t), text: String::new() });
                if depth < MAX_DEPTH {
                    for cell in t.rows.iter().flat_map(|r| &r.cells) {
                        items(&cell.blocks, true, depth + 1, out);
                    }
                }
            }
        }
    }
}

/// A paragraph's text as the write-back compares it with the file's (its tracked deletions,
/// breaks, soft hyphens and objects left out, a field showing its result), and its pictures
/// (pictures, drawings and groups, not deleted ones).
pub(crate) fn para_text(p: &Paragraph) -> (String, usize) {
    let mut out = String::new();
    let mut pictures = 0;
    let mut objects = p.objects.iter();
    let mut pos = 0usize;
    for run in &p.runs {
        let end = pos.saturating_add(run.len).min(p.text.len());
        let text = p.text.get(pos..end).unwrap_or("");
        pos = end;
        let deleted = run.props.del.is_some();
        for c in text.chars() {
            match c {
                OBJ => {
                    let o = objects.next();
                    if deleted {
                        continue;
                    }
                    match o {
                        Some(InlineObject::Field { result, .. }) => out.push_str(result),
                        Some(InlineObject::Image { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. }) => pictures += 1,
                        _ => {}
                    }
                }
                _ if deleted => {}
                PAGE_BREAK | COLUMN_BREAK | SOFT_HYPHEN => {}
                NB_HYPHEN => out.push('-'),
                c => out.push(c),
            }
        }
    }
    (out, pictures)
}

/// How alike two blocks are, 0 to 1000: the same, or the share of the longer paragraph's text
/// that both start or end with.
fn similarity(b: &Item, a: &Item) -> u64 {
    if b.fp == a.fp {
        return 1000;
    }
    if b.table() {
        return 500;
    }
    let (x, y): (Vec<char>, Vec<char>) = (b.text.chars().collect(), a.text.chars().collect());
    let longest = x.len().max(y.len());
    if longest == 0 {
        return 1000;
    }
    let head = x.iter().zip(&y).take_while(|(p, q)| p == q).count();
    let tail = x.iter().rev().zip(y.iter().rev()).take(x.len().min(y.len()) - head).take_while(|(p, q)| p == q).count();
    ((head + tail) * 1000 / longest) as u64
}

/// For each edited block, the blocks read it is ([`Item`] indexes, in the order read): one, or
/// several merged.
fn line_up(before: &[Item], after: &[Item]) -> Vec<Vec<usize>> {
    let paired = pair_up(before, after);
    let mut out: Vec<Vec<usize>> = paired.iter().map(|i| i.iter().copied().collect()).collect();
    // where each block read went
    let mut went: HashMap<usize, usize> = paired.iter().enumerate().filter_map(|(k, i)| Some(((*i)?, k))).collect();
    // gone paragraphs merged into a neighbour's: its text starts with theirs (they came before)
    // or ends with it (after); a run of them merged one by one
    loop {
        let mut more = false;
        for (i, b) in before.iter().enumerate() {
            if went.contains_key(&i) || b.table() || b.text.is_empty() {
                continue;
            }
            let into = |j: usize, at_end: bool| {
                let k = *went.get(&j)?;
                let n = before.get(j)?;
                let a = after.get(k)?;
                let fits = n.kind() == b.kind() && if at_end { a.text.ends_with(&b.text) } else { a.text.starts_with(&b.text) };
                fits.then_some(k)
            };
            let k = i.checked_sub(1).and_then(|j| into(j, true)).or_else(|| into(i + 1, false));
            if let Some(k) = k
                && let Some(list) = out.get_mut(k)
            {
                list.push(i);
                list.sort_unstable();
                went.insert(i, k);
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    out
}

/// For each edited block, the block read it is (see [`line_up`]).
fn pair_up(before: &[Item], after: &[Item]) -> Vec<Option<usize>> {
    let mut out = vec![None; after.len()];
    split(before, after, (0, before.len()), (0, after.len()), &mut out, 0);
    // unchanged blocks out of order: moved
    let paired: HashSet<usize> = out.iter().flatten().copied().collect();
    let mut left: HashMap<(u64, bool), Vec<usize>> = HashMap::new();
    for (i, b) in before.iter().enumerate().rev().filter(|(i, _)| !paired.contains(i)) {
        if b.table() || !b.text.is_empty() {
            left.entry((b.fp, b.nested)).or_default().push(i);
        }
    }
    for (k, a) in after.iter().enumerate() {
        if out.get(k).is_some_and(Option::is_none)
            && let Some(i) = left.get_mut(&(a.fp, a.nested)).and_then(Vec::pop)
            && let Some(slot) = out.get_mut(k)
        {
            *slot = Some(i);
        }
    }
    out
}

/// Line up `before[b.0..b.1]` with `after[a.0..a.1]`: their common start and end, then the
/// blocks found once in each, in order (patience), the stretches between them paired.
fn split(before: &[Item], after: &[Item], b: (usize, usize), a: (usize, usize), out: &mut [Option<usize>], depth: usize) {
    let (mut bl, mut bh, mut al, mut ah) = (b.0, b.1, a.0, a.1);
    let same = |i: usize, k: usize| before.get(i).zip(after.get(k)).is_some_and(|(x, y)| x.fp == y.fp && x.kind() == y.kind());
    while bl < bh && al < ah && same(bl, al) {
        if let Some(s) = out.get_mut(al) {
            *s = Some(bl);
        }
        bl += 1;
        al += 1;
    }
    while bl < bh && al < ah && same(bh - 1, ah - 1) {
        bh -= 1;
        ah -= 1;
        if let Some(s) = out.get_mut(ah) {
            *s = Some(bh);
        }
    }
    if bl == bh || al == ah {
        return;
    }
    // blocks found once on each side: their order in both is the longest run that agrees
    // by fingerprint and kind: how often read, where last; how often edited, where last
    let mut seen: HashMap<(u64, (bool, bool)), [usize; 4]> = HashMap::new();
    for (i, x) in before.iter().enumerate().take(bh).skip(bl) {
        let e = seen.entry((x.fp, x.kind())).or_insert([0, i, 0, 0]);
        e[0] += 1;
        e[1] = i;
    }
    for (k, y) in after.iter().enumerate().take(ah).skip(al) {
        if let Some(e) = seen.get_mut(&(y.fp, y.kind())) {
            e[2] += 1;
            e[3] = k;
        }
    }
    let mut once: Vec<(usize, usize)> = seen.values().filter(|e| e[0] == 1 && e[2] == 1).map(|e| (e[1], e[3])).collect();
    once.sort_by_key(|&(_, k)| k);
    let anchors = increasing(&once);
    if anchors.is_empty() || depth >= MAX_SPLITS {
        return pair(before, after, (bl, bh), (al, ah), out);
    }
    let (mut pb, mut pa) = (bl, al);
    for (i, k) in anchors {
        split(before, after, (pb, i), (pa, k), out, depth + 1);
        if let Some(s) = out.get_mut(k) {
            *s = Some(i);
        }
        (pb, pa) = (i + 1, k + 1);
    }
    split(before, after, (pb, bh), (pa, ah), out, depth + 1);
}

/// The longest run of `pairs` (sorted by their second) whose first increases too.
fn increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    // patience sorting: the smallest last first for each length, and each one's predecessor
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; pairs.len()];
    for (j, &(i, _)) in pairs.iter().enumerate() {
        let at = tails.partition_point(|&t| pairs.get(t).is_some_and(|p| p.0 < i));
        if let Some(slot) = prev.get_mut(j) {
            *slot = at.checked_sub(1).and_then(|a| tails.get(a).copied());
        }
        if at == tails.len() {
            tails.push(j);
        } else if let Some(t) = tails.get_mut(at) {
            *t = j;
        }
    }
    let mut run = Vec::new();
    let mut at = tails.last().copied();
    while let Some(j) = at {
        if let Some(&p) = pairs.get(j) {
            run.push(p);
        }
        at = prev.get(j).copied().flatten();
    }
    run.reverse();
    run
}

/// Pair the blocks of a stretch between unchanged ones (a block with one of its kind, in order),
/// for the most pairs and likeness together: two pairs of nearly the same text outweigh three of
/// unrelated text, so a deleted paragraph doesn't shift every edited one after it onto its
/// neighbour.
fn pair(before: &[Item], after: &[Item], b: (usize, usize), a: (usize, usize), out: &mut [Option<usize>]) {
    let (nb, na) = (b.1 - b.0, a.1 - a.0);
    let fits = |i: usize, k: usize| before.get(i).zip(after.get(k)).is_some_and(|(x, y)| x.kind() == y.kind());
    if nb.saturating_mul(na) > MAX_GAP_CELLS {
        // in order, each edited block with the next read one of its kind
        let mut i = b.0;
        for k in a.0..a.1 {
            let Some(j) = (i..b.1).find(|&j| fits(j, k)) else { continue };
            if let Some(s) = out.get_mut(k) {
                *s = Some(j);
            }
            i = j + 1;
        }
        return;
    }
    // score[i][k]: the best for before[b.0 + i..] and after[a.0 + k..]; a pair counts 1000 and
    // its likeness (0 to 1000)
    let w = na + 1;
    let mut score = vec![0u64; (nb + 1) * w];
    let at = |i: usize, k: usize| i * w + k;
    for i in (0..nb).rev() {
        for k in (0..na).rev() {
            let skip = score.get(at(i + 1, k)).copied().unwrap_or(0).max(score.get(at(i, k + 1)).copied().unwrap_or(0));
            let take = if fits(b.0 + i, a.0 + k) {
                let like = before.get(b.0 + i).zip(after.get(a.0 + k)).map_or(0, |(x, y)| similarity(x, y));
                score.get(at(i + 1, k + 1)).copied().unwrap_or(0) + 1000 + like
            } else {
                0
            };
            if let Some(s) = score.get_mut(at(i, k)) {
                *s = skip.max(take);
            }
        }
    }
    let (mut i, mut k) = (0, 0);
    while i < nb && k < na {
        let here = score.get(at(i, k)).copied().unwrap_or(0);
        if here == score.get(at(i + 1, k)).copied().unwrap_or(0) {
            i += 1;
        } else if here == score.get(at(i, k + 1)).copied().unwrap_or(0) {
            k += 1;
        } else {
            if let Some(s) = out.get_mut(a.0 + k) {
                *s = Some(b.0 + i);
            }
            i += 1;
            k += 1;
        }
    }
}

/// `section:ordinal:piece` as a mark: piece 0 "hp", a further piece "hq", `t` (a table) "ht".
fn origin_mark(origin: &str) -> Option<Mark> {
    let mut parts = origin.split(':');
    let (s, o, piece) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?);
    let kind = match piece {
        "0" => "hp",
        "t" => "ht",
        _ => "hq",
    };
    Some((kind, s, o))
}

/// The stories of a document: the body, then its parts by id.
fn stories(doc: &Document) -> Vec<(Option<(u32, PartKind)>, &Blocks)> {
    let mut out = vec![(None, &doc.body)];
    out.extend(doc.parts.iter().map(|(&id, p)| (Some((id, p.kind)), &p.blocks)));
    out
}

/// The marks of the document read again (`before`, with the reader's `origins`) and of the
/// edited one (`after`, lined up with it).
pub(crate) fn mark(before: &Document, origins: &Origins, after: &Document) -> (Marks, Marks) {
    let (mut mb, mut ma) = (Marks::default(), Marks::default());
    let after_stories: HashMap<Option<(u32, PartKind)>, &Blocks> = stories(after).into_iter().collect();
    for (key, blocks) in stories(before) {
        let mut bi = Vec::new();
        items(blocks, false, 0, &mut bi);
        // the marks read
        let marks: Vec<Vec<Mark>> = bi
            .iter()
            .map(|it| match it.block {
                Block::Para(p) => origins.paras.get(&p.rev).and_then(|o| origin_mark(o)).into_iter().collect(),
                Block::Table(t) => first_para_rev(t, 0).and_then(|r| origins.tables.get(&r)).and_then(|o| origin_mark(o)).into_iter().collect(),
            })
            .collect();
        for (it, m) in bi.iter().zip(&marks) {
            put(&mut mb, it.block, m);
        }
        let Some(edited) = after_stories.get(&key) else { continue };
        let mut ai = Vec::new();
        items(edited, false, 0, &mut ai);
        for (k, read) in line_up(&bi, &ai).into_iter().enumerate() {
            let m: Vec<Mark> = read.iter().filter_map(|&i| marks.get(i)).flatten().copied().collect();
            if let Some(it) = ai.get(k) {
                put(&mut ma, it.block, &m);
            }
        }
    }
    (mb, ma)
}

fn put(marks: &mut Marks, block: &Block, m: &[Mark]) {
    match block {
        Block::Para(p) if !m.is_empty() => {
            marks.paras.insert(std::ptr::from_ref(p), m.to_vec());
        }
        Block::Table(t) => {
            if let Some(&first) = m.first() {
                marks.tables.insert(std::ptr::from_ref(t), first);
            }
        }
        Block::Para(_) => {}
    }
}

/// Every paragraph of a document's stories, in order, with its container (the body, a cell, a
/// part), as the write-back compares them; a source paragraph shown only as a table is an entry
/// without text before the table.
pub(crate) fn doc_paragraphs(doc: &Document, marks: &Marks) -> Vec<DocPara> {
    let mut out = Vec::new();
    for (key, blocks) in stories(doc) {
        let name = match key {
            None => "word/document.xml".to_string(),
            Some((id, PartKind::Header)) => format!("word/header{id}.xml"),
            Some((id, PartKind::Footer)) => format!("word/footer{id}.xml"),
            Some((id, PartKind::Footnote)) => format!("word/footnotes.xml#{id}"),
            Some((id, PartKind::Endnote)) => format!("word/endnotes.xml#{id}"),
            Some((id, PartKind::Comment)) => format!("word/comments.xml#{id}"),
            Some((id, PartKind::TextBox)) => format!("word/document.xml#box{id}"),
        };
        let mut counter = 0;
        walk(blocks, &name, &name, marks, &mut counter, &mut out, 0);
    }
    out
}

fn walk(blocks: &Blocks, root: &str, container: &str, marks: &Marks, counter: &mut usize, out: &mut Vec<DocPara>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for b in blocks {
        match &**b {
            Block::Para(p) => {
                let (text, pictures) = para_text(p);
                let own = marks.para(p).to_vec();
                out.push(DocPara { container: container.to_string(), text: Some(text), marks: own, pictures });
            }
            Block::Table(t) => {
                if let Some(m) = marks.table(t) {
                    out.push(DocPara { container: container.to_string(), text: None, marks: vec![m], pictures: 0 });
                }
                for cell in t.rows.iter().flat_map(|r| &r.cells) {
                    *counter += 1;
                    let inner = format!("{root}#{counter}");
                    walk(&cell.blocks, root, &inner, marks, counter, out, depth + 1);
                }
            }
        }
    }
}
