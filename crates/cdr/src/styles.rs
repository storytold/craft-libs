//! The style table in a `LIST stlt` of versions 7 to 15 (R45): the fill and outline records that a
//! style gives the objects that name it and have no fill or outline argument of their own.

use std::collections::{HashMap, HashSet};

use crate::Error;
use crate::bytes::{Rd, u32_at};
use crate::limits::Budget;

/// One style: its parent, and the fill and outline table entries it names (0: none).
#[derive(Clone, Copy)]
struct Style {
    parent: u32,
    fill: u32,
    outline: u32,
}

/// The styles of a drawing: for each style id, the fill and outline records it gives, its
/// parents' included.
#[derive(Default)]
pub(crate) struct Styles {
    fills: HashMap<u32, u32>,
    outlines: HashMap<u32, u32>,
}

impl Styles {
    /// The style table in body `b` of a `LIST stlt` of `version` (R45), or `None` when it doesn't
    /// parse to its end. Its fill and outline entries and its styles count toward the chunks a
    /// read takes.
    pub(crate) fn read(b: &[u8], version: u16, budget: &mut Budget) -> Result<Option<Styles>, Error> {
        let mut limit = None;
        let styles = parse(b, version, &mut |n| match budget.take_chunks(n) {
            Ok(()) => true,
            Err(e) => {
                limit = Some(e);
                false
            }
        });
        match limit {
            Some(e) => Err(e),
            None => Ok(styles),
        }
    }

    /// The fill record that style `id` gives.
    pub(crate) fn fill(&self, id: u32) -> Option<u32> {
        self.fills.get(&id).copied()
    }

    /// The outline record that style `id` gives.
    pub(crate) fn outline(&self, id: u32) -> Option<u32> {
        self.outlines.get(&id).copied()
    }
}

/// The style table in `b`. `take` is asked for each count of entries or styles before they are
/// read; a `false` stops the read.
fn parse(b: &[u8], version: u16, take: &mut dyn FnMut(usize) -> bool) -> Option<Styles> {
    let mut r = Rd::new(b);
    let count = r.u32()? as usize;
    if count == 0 {
        return (r.left() == 0).then(Styles::default);
    }
    // Fill entries carry 48 more bytes from version 13.
    let fills = table(&mut r, if version >= 1300 { 60 } else { 12 }, take)?;
    let outlines = table(&mut r, 12, take)?;
    // The text style tables: fonts, alignments, spacings, two tables of fixed records, bullets,
    // indents, hyphenation and drop caps, and from version 8.01 one more.
    skip_table(&mut r, if version >= 1000 { 60 } else { 44 })?;
    skip_table(&mut r, 12)?;
    skip_table(&mut r, 52)?;
    skip_table(&mut r, 152)?;
    skip_table(&mut r, 784)?;
    let bullets = r.u32()?;
    for _ in 0..bullets {
        bullet(&mut r, version)?;
    }
    skip_table(&mut r, 28)?;
    skip_table(&mut r, if version >= 1300 { 36 } else { 32 })?;
    skip_table(&mut r, 28)?;
    if version >= 801 {
        skip_table(&mut r, 12)?;
    }
    // Each style is at least 32 bytes.
    if count > r.left() / 32 || !take(count) {
        return None;
    }
    let mut styles = HashMap::new();
    for _ in 0..count {
        let (id, s) = style(&mut r, version)?;
        styles.entry(id).or_insert(s);
    }
    if r.left() != 0 {
        return None;
    }
    let mut work = 0;
    Some(Styles { fills: resolve(&styles, &fills, |s| s.fill, &mut work), outlines: resolve(&styles, &outlines, |s| s.outline, &mut work) })
}

/// For each style, the record of `table` that its own entry names, or the nearest parent's where
/// its entry is 0. Each style is resolved once, and each path forgets only the styles on it, so a
/// chain of any length takes time in proportion to the styles; a chain that comes back to a style
/// on it, or reaches a style that isn't there, gives no record. `work` counts the styles visited
/// and forgotten.
fn resolve(styles: &HashMap<u32, Style>, table: &HashMap<u32, u32>, entry: impl Fn(&Style) -> u32, work: &mut usize) -> HashMap<u32, u32> {
    // `None`: resolved to no record.
    let mut done: HashMap<u32, Option<u32>> = HashMap::new();
    let mut path = Vec::new();
    let mut on_path = HashSet::new();
    for &start in styles.keys() {
        let mut id = start;
        let found = loop {
            *work += 1;
            if let Some(&known) = done.get(&id) {
                break known;
            }
            let Some(s) = styles.get(&id) else { break None };
            // Back at a style on this path: a cycle.
            if !on_path.insert(id) {
                break None;
            }
            path.push(id);
            match entry(s) {
                0 => id = s.parent,
                e => break table.get(&e).copied(),
            }
        };
        for id in path.drain(..) {
            *work += 1;
            on_path.remove(&id);
            done.insert(id, found);
        }
    }
    done.into_iter().filter_map(|(id, r)| Some((id, r?))).collect()
}

/// A table of a u32 count and entries of `size` bytes, each a table id, 4 bytes and a record id.
fn table(r: &mut Rd, size: usize, take: &mut dyn FnMut(usize) -> bool) -> Option<HashMap<u32, u32>> {
    let n = r.u32()? as usize;
    if n > r.left() / size || !take(n) {
        return None;
    }
    let mut out = HashMap::new();
    for _ in 0..n {
        let e = r.bytes(size)?;
        out.entry(u32_at(e, 0)?).or_insert(u32_at(e, 8)?);
    }
    Some(out)
}

/// A table of a u32 count and entries of `size` bytes, passed over.
fn skip_table(r: &mut Rd, size: usize) -> Option<()> {
    let n = r.u32()? as usize;
    r.skip(n.checked_mul(size)?)
}

/// A bullet record, passed over: its length depends on a u32 inside it.
fn bullet(r: &mut Rd, version: u16) -> Option<()> {
    r.skip(40)?;
    if version >= 1300 {
        if version >= 1400 {
            r.skip(4)?;
        }
        let more = r.u32()?;
        return r.skip(if more != 0 { 68 } else { 12 });
    }
    r.skip(if version >= 1000 { 28 } else { 20 })?;
    let more = r.u32()?;
    if more != 0 {
        r.skip(8)?;
    }
    r.skip(8)
}

/// A style record: its kind (u32), id, parent, 8 bytes, a name length and the name (bytes before
/// version 12, UTF-16 units from it), the fill and outline entries, then the entries of text
/// styles (kinds 2 and 3).
fn style(r: &mut Rd, version: u16) -> Option<(u32, Style)> {
    let kind = r.u32()?;
    let (id, parent) = (r.u32()?, r.u32()?);
    r.skip(8)?;
    let len = r.u32()? as usize;
    r.skip(len.checked_mul(if version >= 1200 { 2 } else { 1 })?)?;
    let (fill, outline) = (r.u32()?, r.u32()?);
    if kind > 1 {
        r.skip(if version >= 801 { 20 } else { 16 })?;
    }
    if kind > 2 {
        r.skip(20)?;
    }
    Some((id, Style { parent, fill, outline }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
    use crate::synth::{self, V};

    fn read(b: &[u8], version: u16) -> Option<Styles> {
        Styles::read(b, version, &mut Budget::new(Limits::default())).unwrap()
    }

    #[test]
    fn styles_that_name_each_other_as_parents_end() {
        let looped = [synth::Style { id: 0x10, parent: 0x11, fill: 0, outline: 0 }, synth::Style { id: 0x11, parent: 0x10, fill: 0, outline: 0 }];
        let s = read(&synth::stlt_body(V(700), &[(0x100, 1)], &[], &looped, false), 700).unwrap();
        assert_eq!((s.fill(0x10), s.outline(0x11), s.fill(0x12)), (None, None, None));
    }

    #[test]
    fn a_long_chain_of_parents_reaches_the_style_that_names_an_entry() {
        let mut chain = vec![synth::Style { id: 1, parent: 0, fill: 0x100, outline: 0 }];
        chain.extend((2..=500).map(|id| synth::Style { id, parent: id - 1, fill: 0, outline: 0 }));
        let s = read(&synth::stlt_body(V(1100), &[(0x100, 7)], &[], &chain, false), 1100).unwrap();
        assert_eq!((s.fill(500), s.fill(250), s.outline(500)), (Some(7), Some(7), None));
    }

    /// A loop of 2,000 styles, which every walk into it follows around, leaves the walks of 20,000
    /// other styles no more work than the styles they visit.
    #[test]
    fn one_long_walk_does_not_slow_the_walks_after_it() {
        let mut styles: HashMap<u32, Style> = (1..=2000).map(|id| (id, Style { parent: id % 2000 + 1, fill: 0, outline: 0 })).collect();
        styles.extend((10_001..=30_000).map(|id| (id, Style { parent: 0, fill: 0x100, outline: 0 })));
        let table = HashMap::from([(0x100, 7)]);
        let mut work = 0;
        let fills = resolve(&styles, &table, |s| s.fill, &mut work);
        assert_eq!((fills.get(&1), fills.get(&10_001), fills.len()), (None, Some(&7), 20_000));
        assert!(work <= 4 * styles.len(), "{work}");
    }

    #[test]
    fn an_entry_missing_from_its_table_names_no_record() {
        let styles = [synth::Style { id: 0x10, parent: 0, fill: 0x101, outline: 0x200 }];
        let s = read(&synth::stlt_body(V(1300), &[(0x100, 1)], &[(0x200, 2)], &styles, false), 1300).unwrap();
        assert_eq!((s.fill(0x10), s.outline(0x10)), (None, Some(2)));
    }

    #[test]
    fn counts_past_the_body_do_not_parse() {
        let mut b = synth::stlt_body(V(900), &[], &[], &[], false);
        b[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&b, 900).is_none());
        let empty = 0u32.to_le_bytes();
        assert!(read(&empty, 900).is_some());
        assert!(read(&[], 900).is_none());
    }

    #[test]
    fn entries_and_styles_count_toward_the_chunk_limit() {
        let styles: Vec<_> = (1..=10).map(|id| synth::Style { id, parent: 0, fill: 0x100, outline: 0 }).collect();
        let b = synth::stlt_body(V(1000), &[(0x100, 1)], &[], &styles, false);
        let mut budget = Budget::new(Limits { chunks: 10, ..Limits::default() });
        assert_eq!(Styles::read(&b, 1000, &mut budget).err(), Some(Error::Limit("chunk count")));
        let mut budget = Budget::new(Limits { chunks: 11, ..Limits::default() });
        assert!(Styles::read(&b, 1000, &mut budget).is_ok_and(|s| s.is_some()));
    }
}
