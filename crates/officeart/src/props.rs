//! Shape property tables ([MS-ODRAW] §2.3): an `OfficeArtFOPT` (and its secondary and tertiary
//! tables) holds 6-byte entries, a property id with flags and a 32-bit operand, then the extra
//! bytes of the complex entries in entry order.

use std::collections::HashMap;

use crate::record::{Rec, u16_at, u32_at};

/// Property ids ([MS-ODRAW] §2.3).
pub mod prop {
    pub const ROTATION: u16 = 0x0004;
    pub const TEXT_LEFT: u16 = 0x0081;
    pub const TEXT_TOP: u16 = 0x0082;
    pub const TEXT_RIGHT: u16 = 0x0083;
    pub const TEXT_BOTTOM: u16 = 0x0084;
    pub const WRAP_TEXT: u16 = 0x0085;
    pub const ANCHOR_TEXT: u16 = 0x0087;
    pub const CROP_TOP: u16 = 0x0100;
    pub const CROP_BOTTOM: u16 = 0x0101;
    pub const CROP_LEFT: u16 = 0x0102;
    pub const CROP_RIGHT: u16 = 0x0103;
    pub const PIB: u16 = 0x0104;
    pub const GEO_LEFT: u16 = 0x0140;
    pub const GEO_TOP: u16 = 0x0141;
    pub const GEO_RIGHT: u16 = 0x0142;
    pub const GEO_BOTTOM: u16 = 0x0143;
    pub const SHAPE_PATH: u16 = 0x0144;
    pub const VERTICES: u16 = 0x0145;
    pub const SEGMENT_INFO: u16 = 0x0146;
    pub const ADJUST_VALUE: u16 = 0x0147;
    pub const FILL_TYPE: u16 = 0x0180;
    pub const FILL_COLOR: u16 = 0x0181;
    pub const FILL_OPACITY: u16 = 0x0182;
    pub const FILL_BACK_COLOR: u16 = 0x0183;
    pub const FILL_BACK_OPACITY: u16 = 0x0184;
    pub const FILL_BLIP: u16 = 0x0186;
    pub const FILL_ANGLE: u16 = 0x018B;
    pub const FILL_FOCUS: u16 = 0x018C;
    pub const FILL_BOOLS: u16 = 0x01BF;
    pub const LINE_COLOR: u16 = 0x01C0;
    pub const LINE_OPACITY: u16 = 0x01C1;
    pub const LINE_WIDTH: u16 = 0x01CB;
    pub const LINE_STYLE: u16 = 0x01CD;
    pub const LINE_DASHING: u16 = 0x01CE;
    pub const LINE_START_ARROW: u16 = 0x01D0;
    pub const LINE_END_ARROW: u16 = 0x01D1;
    pub const LINE_START_ARROW_WIDTH: u16 = 0x01D2;
    pub const LINE_START_ARROW_LENGTH: u16 = 0x01D3;
    pub const LINE_END_ARROW_WIDTH: u16 = 0x01D4;
    pub const LINE_END_ARROW_LENGTH: u16 = 0x01D5;
    pub const LINE_JOIN: u16 = 0x01D6;
    pub const LINE_CAP: u16 = 0x01D7;
    pub const LINE_BOOLS: u16 = 0x01FF;
    pub const SHADOW_COLOR: u16 = 0x0201;
    pub const SHADOW_OPACITY: u16 = 0x0204;
    pub const SHADOW_OFFSET_X: u16 = 0x0205;
    pub const SHADOW_OFFSET_Y: u16 = 0x0206;
    pub const SHADOW_BOOLS: u16 = 0x023F;
    pub const HSP_MASTER: u16 = 0x0301;
    pub const NAME: u16 = 0x0380;
    pub const DESCRIPTION: u16 = 0x0381;
    pub const GROUP_BOOLS: u16 = 0x03BF;
}

/// One entry of a property table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry<'a> {
    /// The property id, without its flags.
    pub pid: u16,
    /// The 32-bit operand: for a complex entry, the length of its extra bytes.
    pub op: u32,
    /// Whether the entry is complex (`fComplex`).
    pub is_complex: bool,
    /// A complex entry's extra bytes, clipped to the table; `None` when they start past its end.
    pub complex: Option<&'a [u8]>,
}

/// The entries of a property table's body: up to `count` 6-byte entries, ending at the first one
/// the body doesn't hold whole, the complex data starting after `count` entries.
pub fn entries(table: &[u8], count: usize) -> impl Iterator<Item = Entry<'_>> {
    let mut complex_at = count.saturating_mul(6);
    (0..count).map_while(move |i| {
        let id = u16_at(table, i.checked_mul(6)?)?;
        let op = u32_at(table, i.checked_mul(6)?.checked_add(2)?)?;
        let is_complex = id & 0x8000 != 0;
        let complex = if is_complex {
            let len = op as usize;
            let data = table.get(complex_at..complex_at.saturating_add(len).min(table.len()));
            complex_at = complex_at.saturating_add(len);
            data
        } else {
            None
        };
        Some(Entry { pid: id & 0x3FFF, op, is_complex, complex })
    })
}

/// UTF-16 units kept of a string property.
pub const MAX_STRING: usize = 4096;

/// Distinct properties one set keeps. Ids are 14 bits, so a file's tables could name about 16K of
/// them, and every shape copies the drawing defaults' and its master shape's sets into its own:
/// unbounded, that is shapes × 16K. [MS-ODRAW] defines a few hundred properties and a real shape
/// or the drawing defaults set tens of them. Entries past the limit are still read (their complex
/// data keeps its place) but not kept.
pub const MAX_PROPS: usize = 1024;

/// One property value: the 32-bit operand and, for complex properties, the extra bytes.
#[derive(Clone, Copy, Debug)]
pub struct PropVal<'a> {
    /// For a complex property, the length of its extra bytes.
    pub op: u32,
    pub is_complex: bool,
    /// A complex property's extra bytes; `None` when they start past the table's end.
    pub complex: Option<&'a [u8]>,
}

/// A shape's merged property tables (primary first; secondary and tertiary fill gaps).
#[derive(Clone, Debug, Default)]
pub struct Props<'a> {
    map: HashMap<u16, PropVal<'a>>,
}

impl<'a> Props<'a> {
    /// Read one `FOPT` body: `count` 6-byte entries then the complex data in entry order. Entries
    /// already present are kept (the caller adds the most specific table first). Within the table,
    /// a property named twice takes its last entry ([MS-ODRAW], the overview of properties).
    pub fn add_table(&mut self, r: &Rec<'a>) {
        let mut table: Vec<(u16, PropVal<'a>)> = Vec::new();
        let mut at: HashMap<u16, usize> = HashMap::new();
        for e in entries(r.data, usize::from(r.instance)) {
            let v = PropVal { op: e.op, is_complex: e.is_complex, complex: e.complex };
            match at.get(&e.pid) {
                Some(&i) => {
                    if let Some(slot) = table.get_mut(i) {
                        slot.1 = v;
                    }
                }
                None => {
                    at.insert(e.pid, table.len());
                    table.push((e.pid, v));
                }
            }
        }
        for (id, v) in table {
            self.fill_gap(id, v);
        }
    }

    /// Fill gaps from another set (a master shape's or the drawing defaults).
    pub fn inherit(&mut self, other: &Props<'a>) {
        for (k, v) in &other.map {
            self.fill_gap(*k, *v);
        }
    }

    /// Add `v` for `id` where this set doesn't say it already. A boolean property word ([MS-ODRAW]:
    /// the last id of each 64-id group) holds sixteen flags, each with its own `fUse` bit 16
    /// places up, so it is merged flag by flag: a shape that sets only `fNoFillHitTest` still
    /// inherits `fFilled` from its master or the defaults.
    fn fill_gap(&mut self, id: u16, v: PropVal<'a>) {
        let full = self.map.len() >= MAX_PROPS;
        match self.map.get_mut(&id) {
            None if full => {}
            None => {
                self.map.insert(id, v);
            }
            Some(have) if id & 0x3F == 0x3F => {
                let unset = !(have.op >> 16) & 0xFFFF;
                let take = (v.op >> 16) & unset;
                have.op |= take << 16;
                have.op = (have.op & !take) | (v.op & take);
            }
            Some(_) => {}
        }
    }

    /// How many distinct properties the set holds.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Set a simple (non-complex) property. A new one isn't added to a set already holding
    /// [`MAX_PROPS`].
    pub fn set(&mut self, id: u16, op: u32) {
        if self.map.len() < MAX_PROPS || self.map.contains_key(&id) {
            self.map.insert(id, PropVal { op, is_complex: false, complex: None });
        }
    }

    /// The operand: for a complex property, the length of its extra bytes.
    pub fn get(&self, id: u16) -> Option<u32> {
        self.map.get(&id).map(|v| v.op)
    }

    /// The operand of a simple property; `None` for a complex one, whose operand is a length.
    pub fn simple(&self, id: u16) -> Option<u32> {
        self.map.get(&id).filter(|v| !v.is_complex).map(|v| v.op)
    }

    pub fn complex(&self, id: u16) -> Option<&'a [u8]> {
        self.map.get(&id).and_then(|v| v.complex)
    }

    /// A 16.16 fixed-point value.
    pub fn fixed(&self, id: u16) -> Option<f64> {
        self.get(id).map(|v| f64::from(v as i32) / 65536.0)
    }

    /// A boolean from a bool-property word: bit `bit` holds the value, bit `bit + 16` says it is set.
    pub fn flag(&self, id: u16, bit: u32) -> Option<bool> {
        let v = self.get(id)?;
        (v & (1 << (bit + 16)) != 0).then_some(v & (1 << bit) != 0)
    }

    /// A UTF-16 string property (names, alt text), at most [`MAX_STRING`] units: a master shape's
    /// name is copied into every shape that inherits it.
    pub fn string(&self, id: u16) -> Option<String> {
        let b = self.complex(id)?;
        let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take(MAX_STRING).take_while(|&u| u != 0).collect();
        Some(String::from_utf16_lossy(&units)).filter(|s| !s.is_empty())
    }

    /// An `IMsoArray` property: element size and the elements' bytes.
    pub fn array(&self, id: u16) -> Option<(usize, &'a [u8])> {
        let b = self.complex(id)?;
        let n = usize::from(u16_at(b, 0)?);
        let size = u16_at(b, 4)?;
        // 0xFFF0 marks 4-byte elements stored as two 2-byte halves.
        let size = if size == 0xFFF0 { 4 } else { usize::from(size) };
        if size == 0 {
            return None;
        }
        let body = b.get(6..)?;
        let len = n.saturating_mul(size).min(body.len() / size * size);
        Some((size, body.get(..len)?))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::record::build::*;
    use crate::rt;

    pub fn fopt(entries: &[(u16, u32)], complex: &[u8]) -> Vec<u8> {
        let mut b = vec![];
        for (id, op) in entries {
            b.extend_from_slice(&id.to_le_bytes());
            b.extend_from_slice(&op.to_le_bytes());
        }
        b.extend_from_slice(complex);
        atom(rt::FOPT, entries.len() as u16, &b)
    }

    #[test]
    fn reads_properties_and_complex_strings() {
        let name: Vec<u8> = "Logo\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let t = fopt(&[(prop::FILL_COLOR, 0x0011_2233), (0x8000 | prop::NAME, name.len() as u32), (prop::FILL_BOOLS, 0x0010_0000)], &name);
        let mut p = Props::default();
        p.add_table(&Rec::at(&t, 0).unwrap());
        assert_eq!(p.get(prop::FILL_COLOR), Some(0x0011_2233));
        assert_eq!(p.string(prop::NAME).as_deref(), Some("Logo"));
        assert_eq!(p.flag(prop::FILL_BOOLS, 4), Some(false));
        assert_eq!(p.flag(prop::LINE_BOOLS, 3), None);
    }

    /// A boolean word inherits the flags it doesn't set (storytold/deckcraft#1137 review): a text box that sets only
    /// `fNoFillHitTest` (bit 4 is `fFilled`, bit 0 `fNoFillHitTest`) still gets "not filled"
    /// from below, and its own flags win over the inherited ones.
    #[test]
    fn boolean_properties_merge_flag_by_flag() {
        let mut p = Props::default();
        p.set(prop::FILL_BOOLS, 0x0001_0001); // fNoFillHitTest = true; fFilled not said
        p.set(prop::LINE_BOOLS, 0x0008_0008); // fLine = true
        let mut below = Props::default();
        below.set(prop::FILL_BOOLS, 0x0011_0000); // fFilled = false, fNoFillHitTest = false
        below.set(prop::LINE_BOOLS, 0x0008_0000); // fLine = false
        p.inherit(&below);
        assert_eq!(p.flag(prop::FILL_BOOLS, 4), Some(false));
        assert_eq!(p.flag(prop::FILL_BOOLS, 0), Some(true));
        assert_eq!(p.flag(prop::LINE_BOOLS, 3), Some(true));
        // The same merge applies between a shape's own tables.
        let t = fopt(&[(prop::FILL_BOOLS, 0x0001_0001)], &[]);
        let t2 = fopt(&[(prop::FILL_BOOLS, 0x0010_0010)], &[]);
        let mut q = Props::default();
        q.add_table(&Rec::at(&t, 0).unwrap());
        q.add_table(&Rec::at(&t2, 0).unwrap());
        assert_eq!(q.flag(prop::FILL_BOOLS, 4), Some(true));
        assert_eq!(q.flag(prop::FILL_BOOLS, 0), Some(true));
        // Non-boolean properties still keep the first value.
        let mut r = Props::default();
        r.set(prop::FILL_COLOR, 1);
        let mut s = Props::default();
        s.set(prop::FILL_COLOR, 2);
        r.inherit(&s);
        assert_eq!(r.get(prop::FILL_COLOR), Some(1));
    }

    /// A string property longer than [`MAX_STRING`] units is cut there.
    #[test]
    fn a_long_string_property_is_capped() {
        let name: Vec<u8> = "x".repeat(10_000).encode_utf16().flat_map(u16::to_le_bytes).collect();
        let t = fopt(&[(0x8000 | prop::NAME, name.len() as u32)], &name);
        let mut p = Props::default();
        p.add_table(&Rec::at(&t, 0).unwrap());
        assert_eq!(p.string(prop::NAME).map(|s| s.len()), Some(MAX_STRING));
    }

    #[test]
    fn a_complex_length_past_the_record_is_clipped() {
        let t = fopt(&[(0x8000 | prop::NAME, 0xFFFF_FFFF), (0x8000 | prop::DESCRIPTION, 10)], b"A\0");
        let mut p = Props::default();
        p.add_table(&Rec::at(&t, 0).unwrap());
        assert_eq!(p.string(prop::NAME).as_deref(), Some("A"));
        assert_eq!(p.complex(prop::DESCRIPTION), None);
    }

    /// Within one table a property named twice takes its last entry ([MS-ODRAW]); between tables
    /// the first one added still wins.
    #[test]
    fn the_last_entry_in_a_table_wins() {
        let t = fopt(&[(prop::FILL_COLOR, 1), (prop::LINE_COLOR, 5), (prop::FILL_COLOR, 2)], &[]);
        let t2 = fopt(&[(prop::FILL_COLOR, 3), (prop::LINE_COLOR, 6)], &[]);
        let mut p = Props::default();
        p.add_table(&Rec::at(&t, 0).unwrap());
        p.add_table(&Rec::at(&t2, 0).unwrap());
        assert_eq!((p.get(prop::FILL_COLOR), p.get(prop::LINE_COLOR)), (Some(2), Some(5)));
    }

    /// A complex property's operand is the length of its bytes: `simple` doesn't give it as a
    /// value (a complex `pib` is no store index).
    #[test]
    fn a_complex_operand_is_not_a_simple_value() {
        let t = fopt(&[(0x8000 | prop::PIB, 2), (prop::FILL_BLIP, 7)], b"AB");
        let mut p = Props::default();
        p.add_table(&Rec::at(&t, 0).unwrap());
        assert_eq!((p.get(prop::PIB), p.simple(prop::PIB)), (Some(2), None));
        assert_eq!(p.simple(prop::FILL_BLIP), Some(7));
    }

    /// `set` keeps the set within [`MAX_PROPS`]: a new id is refused at the limit, an existing one
    /// still changes.
    #[test]
    fn set_keeps_at_most_max_props() {
        let mut p = Props::default();
        for id in 0..MAX_PROPS as u16 {
            p.set(id, 1);
        }
        p.set(MAX_PROPS as u16, 1);
        p.set(0, 2);
        assert_eq!((p.len(), p.get(MAX_PROPS as u16), p.get(0)), (MAX_PROPS, None, Some(2)));
    }

    /// A table declaring more entries than it holds ends at the last whole one, and its complex
    /// data is looked for after the entries it declares.
    #[test]
    fn a_short_table_ends_at_its_last_whole_entry() {
        let mut t = fopt(&[(prop::FILL_COLOR, 1), (prop::LINE_COLOR, 2), (0x8000 | prop::NAME, 2)], b"A\0");
        // Cut into the third entry: two whole ones are left, and no complex data.
        t.truncate(8 + 6 * 2 + 3);
        let r = Rec::at(&t, 0).unwrap();
        let got: Vec<_> = entries(r.data, usize::from(r.instance)).map(|e| (e.pid, e.op)).collect();
        assert_eq!(got, [(prop::FILL_COLOR, 1), (prop::LINE_COLOR, 2)]);
    }
}
