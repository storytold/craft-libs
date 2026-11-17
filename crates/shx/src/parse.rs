//! Reading the compiled SHX containers (layouts in the module documentation). Every count and
//! offset comes from the file, so each is checked against its length; records that run past
//! the end are dropped and the rest kept.

use super::ShxError;
use std::collections::HashMap;

/// The SHX container kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShxKind {
    Shapes,
    BigFont,
    Unifont,
}
/// Byte ranges of one shape name and pen program in [`ParsedFont::data`].
#[derive(Clone, Copy, Debug)]
pub struct Record {
    pub name: (usize, usize),
    pub program: (usize, usize),
}
/// Decoded container records and metrics; glyph execution remains a caller concern.
pub struct ParsedFont {
    pub kind: ShxKind,
    pub name: String,
    pub header: bool,
    pub above: f64,
    pub below: f64,
    pub dual: bool,
    pub encoding: u8,
    pub char_width: f64,
    pub ranges: Vec<(u16, u16)>,
    pub data: Vec<u8>,
    pub records: HashMap<u16, Record>,
}

pub const SHAPES_10: &[u8] = b"AutoCAD-86 shapes 1.0\r\n\x1a";
pub const SHAPES_11: &[u8] = b"AutoCAD-86 shapes 1.1\r\n\x1a";
pub const BIGFONT: &[u8] = b"AutoCAD-86 bigfont 1.0\r\n\x1a";
pub const UNIFONT: &[u8] = b"AutoCAD-86 unifont 1.0\r\n\x1a";

/// Largest file read (the largest big fonts are a few megabytes).
pub const MAX_FILE_BYTES: usize = 32 << 20;
/// Most escape ranges in a big font (lead bytes: at most 256).
const MAX_RANGES: usize = 256;

/// A checked little-endian reader.
struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ShxError> {
        let end = self.pos.checked_add(n).ok_or(ShxError::Truncated)?;
        let s = self.b.get(self.pos..end).ok_or(ShxError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, ShxError> {
        match self.take(2)? {
            [a, b] => Ok(u16::from_le_bytes([*a, *b])),
            _ => Err(ShxError::Truncated),
        }
    }
    fn u32(&mut self) -> Result<u32, ShxError> {
        match self.take(4)? {
            [a, b, c, d] => Ok(u32::from_le_bytes([*a, *b, *c, *d])),
            _ => Err(ShxError::Truncated),
        }
    }
}

/// A font being read.
struct Font {
    kind: ShxKind,
    name: String,
    header: bool,
    above: f64,
    below: f64,
    dual: bool,
    encoding: u8,
    char_width: f64,
    ranges: Vec<(u16, u16)>,
    records: HashMap<u16, Record>,
}

impl Font {
    fn new(kind: ShxKind) -> Font {
        Font {
            kind,
            name: String::new(),
            header: false,
            above: 0.0,
            below: 0.0,
            dual: false,
            encoding: 0,
            char_width: 0.0,
            ranges: Vec::new(),
            records: HashMap::new(),
        }
    }

    /// Add the record of shape `number` at `start..end` of `data`: a NUL-terminated name, then
    /// the program. Shape 0 is the font record. A record without a name terminator is dropped;
    /// the first record of a number wins.
    fn record(&mut self, data: &[u8], number: u16, start: usize, end: usize) {
        let Some(bytes) = data.get(start..end) else { return };
        let Some(nul) = bytes.iter().position(|b| *b == 0) else { return };
        let name = (start, start + nul);
        let program = (start + nul + 1, end);
        if number == 0 {
            if !self.header {
                self.font_record(data.get(program.0..program.1).unwrap_or_default(), data.get(name.0..name.1).unwrap_or_default());
            }
            return;
        }
        self.records.entry(number).or_insert(Record { name, program });
    }

    /// The font record: above, below, modes (and for a unifont encoding and embedding); an
    /// extended big font's is character height, 0, modes, character width.
    fn font_record(&mut self, info: &[u8], name: &[u8]) {
        self.header = true;
        self.name = name.iter().take(255).map(|c| char::from(*c)).collect();
        let byte = |i: usize| info.get(i).copied().unwrap_or(0);
        self.above = f64::from(byte(0));
        self.below = f64::from(byte(1));
        self.dual = byte(2) == 2;
        if self.kind == ShxKind::Unifont {
            self.encoding = byte(3);
        }
        if self.kind == ShxKind::BigFont && info.len() >= 5 && byte(1) == 0 {
            // Extended: height, 0, modes, width, 0.
            self.below = 0.0;
            self.char_width = f64::from(byte(3));
        }
    }

    fn finish(self, data: Vec<u8>) -> Result<ParsedFont, ShxError> {
        if self.records.is_empty() {
            return Err(ShxError::Empty);
        }
        Ok(ParsedFont {
            kind: self.kind,
            name: self.name,
            header: self.header,
            above: self.above,
            below: self.below,
            dual: self.dual,
            encoding: self.encoding,
            char_width: if self.char_width > 0.0 { self.char_width } else { self.above },
            ranges: self.ranges,
            data,
            records: self.records,
        })
    }
}

pub fn parse(bytes: &[u8]) -> Result<ParsedFont, ShxError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ShxError::TooLarge);
    }
    if bytes.starts_with(SHAPES_10) || bytes.starts_with(SHAPES_11) {
        shapes(bytes)
    } else if bytes.starts_with(BIGFONT) {
        bigfont(bytes)
    } else if bytes.starts_with(UNIFONT) {
        unifont(bytes)
    } else {
        Err(ShxError::NotShx)
    }
}

fn shapes(bytes: &[u8]) -> Result<ParsedFont, ShxError> {
    let mut r = Rd { b: bytes, pos: SHAPES_10.len() };
    let _first = r.u16()?;
    let _last = r.u16()?;
    let count = usize::from(r.u16()?);
    // The whole index must be there before anything is read from it.
    let index = r.take(count.checked_mul(4).ok_or(ShxError::TooLarge)?)?;
    let mut f = Font::new(ShxKind::Shapes);
    let mut pos = r.pos;
    for [a, b, c, d] in index.as_chunks::<4>().0 {
        let (number, len) = (u16::from_le_bytes([*a, *b]), usize::from(u16::from_le_bytes([*c, *d])));
        let Some(end) = pos.checked_add(len).filter(|e| *e <= bytes.len()) else { break };
        f.record(bytes, number, pos, end);
        pos = end;
    }
    f.finish(bytes.to_vec())
}

fn bigfont(bytes: &[u8]) -> Result<ParsedFont, ShxError> {
    let mut r = Rd { b: bytes, pos: BIGFONT.len() };
    let index_size = usize::from(r.u16()?);
    let count = usize::from(r.u16()?);
    let nranges = usize::from(r.u16()?);
    if nranges > MAX_RANGES {
        return Err(ShxError::TooLarge);
    }
    let mut f = Font::new(ShxKind::BigFont);
    for _ in 0..nranges {
        let (lo, hi) = (r.u16()?, r.u16()?);
        f.ranges.push((lo.min(hi), lo.max(hi)));
    }
    // The index has `count` entries of 8 bytes; a file whose count doesn't fit but whose index
    // size does is read by that.
    let fits = |n: usize| n.checked_mul(8).and_then(|l| r.pos.checked_add(l)).is_some_and(|e| e <= bytes.len());
    let n = if fits(count) {
        count
    } else if fits(index_size) {
        index_size
    } else {
        return Err(ShxError::Truncated);
    };
    let index = r.take(n.checked_mul(8).ok_or(ShxError::TooLarge)?)?;
    for [a, b, c, d, o0, o1, o2, o3] in index.as_chunks::<8>().0 {
        let number = u16::from_le_bytes([*a, *b]);
        let len = usize::from(u16::from_le_bytes([*c, *d]));
        let Ok(off) = usize::try_from(u32::from_le_bytes([*o0, *o1, *o2, *o3])) else { continue };
        if len == 0 {
            continue;
        }
        let Some(end) = off.checked_add(len).filter(|e| *e <= bytes.len()) else { continue };
        f.record(bytes, number, off, end);
    }
    f.finish(bytes.to_vec())
}

fn unifont(bytes: &[u8]) -> Result<ParsedFont, ShxError> {
    let mut r = Rd { b: bytes, pos: UNIFONT.len() };
    let count = r.u32()?;
    let info_len = usize::from(r.u16()?);
    let info = r.take(info_len)?;
    let mut f = Font::new(ShxKind::Unifont);
    let nul = info.iter().position(|b| *b == 0).unwrap_or(info.len());
    f.font_record(info.get(nul.saturating_add(1)..).unwrap_or_default(), info.get(..nul).unwrap_or_default());
    // `count` includes the font record; never trusted beyond the data that is there.
    let mut left = count.saturating_sub(1);
    while left > 0 {
        left -= 1;
        let Ok(number) = r.u16() else { break };
        let Ok(len) = r.u16().map(usize::from) else { break };
        let start = r.pos;
        let Some(end) = start.checked_add(len).filter(|e| *e <= bytes.len()) else { break };
        r.pos = end;
        f.record(bytes, number, start, end);
    }
    f.finish(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn reads_shapes_bigfont_and_unifont_records() {
        for (source, kind, shape) in [
            ("*0,4,font\n6,2,2,0\n*65,4,A\n1,064,0\n", ShxKind::Shapes, 65),
            ("*BIGFONT 2,1,081,09F\n*0,4,big\n8,0,0,0\n*082A0,4,square\n1,080,0\n", ShxKind::BigFont, 0x82a0),
            ("*UNIFONT,6,uni\n6,2,0,0,0,0\n*00041,4,a\n1,064,0\n", ShxKind::Unifont, 65),
        ] {
            let bytes = crate::compile(source).unwrap();
            let font = parse(&bytes).unwrap();
            assert_eq!(font.kind, kind);
            assert!(font.header);
            let record = font.records.get(&shape).unwrap();
            assert_eq!(font.data.get(record.program.1 - 1), Some(&0));
        }
    }
    #[test]
    fn truncation_limits_and_unknown_signatures_are_errors() {
        assert!(matches!(parse(b"not a font"), Err(ShxError::NotShx)));
        assert!(matches!(parse(&vec![0; MAX_FILE_BYTES + 1]), Err(ShxError::TooLarge)));
        let mut shapes = SHAPES_10.to_vec();
        shapes.extend([1, 0, 255, 255, 255, 255]);
        assert!(matches!(parse(&shapes), Err(ShxError::Truncated)));
        let mut big = BIGFONT.to_vec();
        big.extend([255, 255, 255, 255, 255, 255]);
        assert!(matches!(parse(&big), Err(ShxError::TooLarge)));
    }
}
