//! Index table segments (ST 377-1 §11).

use crate::mxf::Rational;
use crate::mxf::klv::{Cur, int_of, rational_of};

/// One index entry (ST 377-1 §11.2.4), for one edit unit in stored order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IndexEntry {
    /// Display-order position `n` is stored at `n + temporal_offset` (the offset belongs to the
    /// entry at the display position).
    pub temporal_offset: i8,
    /// Stored-order offset from this edit unit to the key frame it depends on (≤ 0).
    pub key_frame_offset: i8,
    /// Bit 7 random access, bit 6 sequence header, bits 5-4 prediction (00 I, 10 P, 11 B).
    pub flags: u8,
    /// Byte offset of the edit unit in the essence container stream.
    pub stream_offset: u64,
}

impl IndexEntry {
    pub fn random_access(&self) -> bool {
        self.flags & 0x80 != 0
    }
    /// Uses backward prediction (a B picture).
    pub fn is_b(&self) -> bool {
        self.flags & 0x30 == 0x30
    }
}

/// An index table segment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexSegment {
    pub edit_rate: Rational,
    pub start_position: i64,
    /// Edit units covered (0 with a constant byte count: the whole container).
    pub duration: i64,
    /// Constant bytes per edit unit (CBR); 0 when the entries give the offsets (VBR).
    pub edit_unit_byte_count: u32,
    pub index_sid: u32,
    pub body_sid: u32,
    pub slice_count: u8,
    pub pos_table_count: u8,
    /// (pos table index, slice, element delta) per element of a content package.
    pub delta_entries: Vec<(i8, u8, u32)>,
    pub entries: Vec<IndexEntry>,
}

impl IndexSegment {
    pub fn parse(v: &[u8]) -> IndexSegment {
        let mut s = IndexSegment::default();
        let mut c = Cur::new(v);
        let mut entry_array: Option<&[u8]> = None;
        while c.remaining() >= 4 {
            let (Some(tag), Some(len)) = (c.u16(), c.u16()) else { break };
            let Some(val) = c.bytes(len as usize) else { break };
            match tag {
                0x3F0B => s.edit_rate = rational_of(val).unwrap_or_default(),
                0x3F0C => s.start_position = int_of(val, true).unwrap_or(0),
                0x3F0D => s.duration = int_of(val, true).unwrap_or(0),
                0x3F05 => s.edit_unit_byte_count = int_of(val, false).unwrap_or(0) as u32,
                0x3F06 => s.index_sid = int_of(val, false).unwrap_or(0) as u32,
                0x3F07 => s.body_sid = int_of(val, false).unwrap_or(0) as u32,
                0x3F08 => s.slice_count = val.first().copied().unwrap_or(0),
                0x3F0E => s.pos_table_count = val.first().copied().unwrap_or(0),
                0x3F09 => {
                    for d in crate::mxf::klv::batch_of(val) {
                        if d.len() >= 6 {
                            s.delta_entries.push((d[0] as i8, d[1], u32::from_be_bytes([d[2], d[3], d[4], d[5]])));
                        }
                    }
                }
                0x3F0A => entry_array = Some(val),
                _ => {}
            }
        }
        if let Some(val) = entry_array {
            for e in crate::mxf::klv::batch_of(val) {
                if e.len() >= 11 {
                    s.entries.push(IndexEntry {
                        temporal_offset: e[0] as i8,
                        key_frame_offset: e[1] as i8,
                        flags: e[2],
                        stream_offset: u64::from_be_bytes(e[3..11].try_into().unwrap_or([0; 8])),
                    });
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vbr_segment() {
        let mut v = Vec::new();
        let mut prop = |tag: u16, val: &[u8]| {
            v.extend_from_slice(&tag.to_be_bytes());
            v.extend_from_slice(&(val.len() as u16).to_be_bytes());
            v.extend_from_slice(val);
        };
        prop(0x3F0B, &[0, 0, 0, 25, 0, 0, 0, 1]);
        prop(0x3F0C, &0i64.to_be_bytes());
        prop(0x3F0D, &2i64.to_be_bytes());
        prop(0x3F06, &2u32.to_be_bytes());
        prop(0x3F07, &1u32.to_be_bytes());
        let mut arr = vec![0, 0, 0, 2, 0, 0, 0, 11];
        arr.extend_from_slice(&[0, 0, 0xC0]);
        arr.extend_from_slice(&0u64.to_be_bytes());
        arr.extend_from_slice(&[0xFF, 0xFF, 0x33]);
        arr.extend_from_slice(&1000u64.to_be_bytes());
        prop(0x3F0A, &arr);
        let s = IndexSegment::parse(&v);
        assert_eq!(s.edit_rate, Rational::new(25, 1));
        assert_eq!(s.duration, 2);
        assert_eq!(s.entries.len(), 2);
        assert!(s.entries[0].random_access());
        assert_eq!(s.entries[1].temporal_offset, -1);
        assert!(s.entries[1].is_b());
        assert_eq!(s.entries[1].stream_offset, 1000);
    }
}
