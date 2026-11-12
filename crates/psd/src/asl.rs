//! Photoshop layer style libraries (`.asl`, the Styles panel's Import Styles… / Export Styles…).
//!
//! Layout of a version 3 style library:
//!
//! ```text
//! file    := u16 2, "8BSL", u16 3 (sub-version),
//!            u32 pattern section length, patterns (the `Patt` block record format),
//!            u32 style count, style × count
//! style   := u32 length, then that many bytes: identity, style descriptor, zero padding
//! identity:= versioned descriptor (class `null`): `Nm  ` name, `Idnt` id (Unicode strings)
//! style descriptor := versioned descriptor (class `Styl`): `documentMode`, `Lefx` (the same
//!            tree as a layer's `lfx2` block), optional `blendOptions` (`Md  `, `Opct`,
//!            `fillOpacity`)
//! ```
//!
//! The reader skips up to three padding bytes after each style record whatever its length says,
//! so it reads files whose record length counts the padding and files whose length does not.
//! The writer counts the padding in the record length, which reads correctly either way.
//!
//! Malformed input is an error, never a panic. A pattern section that doesn't decode is reported
//! in [`AslFile::pattern_error`] and skipped (its length is known), so the styles still load.

use crate::descriptor::{Descriptor, UnicodeString, Value, VersionedDescriptor};
use crate::error::{PsdError, Result};
use crate::patterns::{PsdPattern, parse_pattern_block, write_pattern_block};

/// Most styles read from one file.
pub const MAX_STYLES: usize = 10_000;

/// One style of a library.
#[derive(Debug, Clone, PartialEq)]
pub struct AslStyle {
    /// The name shown in the Styles panel (may be empty or repeated).
    pub name: String,
    /// The style's unique id (usually a UUID).
    pub id: String,
    /// The style descriptor (class `Styl`): `Lefx`, `blendOptions`, `documentMode`.
    pub style: Descriptor,
}

/// A parsed style library.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AslFile {
    /// Patterns embedded for the styles' Pattern Overlays, pattern strokes and bevel textures.
    pub patterns: Vec<PsdPattern>,
    /// Why the pattern section was skipped, if it didn't decode (read only).
    pub pattern_error: Option<String>,
    /// Styles in file order.
    pub styles: Vec<AslStyle>,
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(PsdError::UnexpectedEof { offset: self.p, needed: n })?;
        let s = self.b.get(self.p..end).ok_or(PsdError::UnexpectedEof { offset: self.p, needed: n })?;
        self.p = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    /// The next big-endian `u32`.
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// Bytes left to read.
    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }
}

/// One style from an ASL body: its identity descriptor, then the style itself.
fn read_style(body: &[u8]) -> Result<AslStyle> {
    let (identity, used) = VersionedDescriptor::parse_prefix(body)?;
    let rest = body.get(used..).ok_or(PsdError::UnexpectedEof { offset: used, needed: 4 })?;
    let (style, _) = VersionedDescriptor::parse_prefix(rest)?;
    let id = &identity.descriptor;
    Ok(AslStyle { name: id.get_text("Nm  ").unwrap_or_default(), id: id.get_text("Idnt").unwrap_or_default(), style: style.descriptor })
}

/// Parses an `.asl` style library.
pub fn parse(data: &[u8]) -> Result<AslFile> {
    let mut r = Rd { b: data, p: 0 };
    let version = r.u16()?;
    let sig = r.take(4)?;
    if sig != b"8BSL" {
        return Err(PsdError::InvalidSignature { expected: "8BSL", found: [sig[0], sig[1], sig[2], sig[3]] });
    }
    let sub = r.u16()?;
    if version != 2 || sub != 3 {
        return Err(PsdError::Unsupported(format!("style library version {version}.{sub} (only 2.3, Photoshop 6 and later)")));
    }
    let plen = r.u32()? as usize;
    let section = r.take(plen)?;
    let (patterns, pattern_error) = match parse_pattern_block(section) {
        Ok(p) => (p, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    let count = r.u32()? as usize;
    // Every style needs at least its own length word.
    if count > r.remaining() / 4 {
        return Err(PsdError::invalid(format!("{count} styles cannot fit in the remaining {} bytes", r.remaining())));
    }
    if count > MAX_STYLES {
        return Err(PsdError::LimitExceeded("too many styles in one library"));
    }
    let mut styles = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        styles.push(read_style(body)?);
        let pad = (4 - len % 4) % 4;
        r.p = r.p.saturating_add(pad).min(data.len());
    }
    Ok(AslFile { patterns, pattern_error: pattern_error.filter(|_| !section.is_empty()), styles })
}

/// Writes a version 3 `.asl` style library.
pub fn write(file: &AslFile) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(b"8BSL");
    out.extend_from_slice(&3u16.to_be_bytes());
    let patterns = write_pattern_block(&file.patterns)?;
    let plen = u32::try_from(patterns.len()).map_err(|_| PsdError::LimitExceeded("pattern section larger than 4 GB"))?;
    out.extend_from_slice(&plen.to_be_bytes());
    out.extend_from_slice(&patterns);
    let count = u32::try_from(file.styles.len()).map_err(|_| PsdError::LimitExceeded("too many styles"))?;
    out.extend_from_slice(&count.to_be_bytes());
    for s in &file.styles {
        let identity = Descriptor::new("null")
            .with("Nm  ", Value::Text(UnicodeString::new_nul(&s.name)))
            .with("Idnt", Value::Text(UnicodeString::new_nul(&s.id)));
        let mut body = VersionedDescriptor::new(identity).to_bytes();
        body.extend(VersionedDescriptor::new(s.style.clone()).to_bytes());
        body.resize(body.len().div_ceil(4) * 4, 0);
        let len = u32::try_from(body.len()).map_err(|_| PsdError::LimitExceeded("style larger than 4 GB"))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&body);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Id;

    fn pattern() -> PsdPattern {
        PsdPattern {
            mode: 3,
            width: 3,
            height: 2,
            name: "Dots".into(),
            id: "a1b2".into(),
            palette: None,
            depth: 8,
            channels: vec![vec![1, 2, 3, 4, 5, 6], vec![9; 6], vec![200; 6]],
            alpha: None,
        }
    }

    fn style(name: &str) -> AslStyle {
        let lefx = Descriptor::new("null").with("masterFXSwitch", Value::Boolean(true)).with(
            "SoFi",
            Value::Descriptor(
                Descriptor::new("SoFi").with("enab", Value::Boolean(true)).with("Opct", Value::UnitFloat { unit: *b"#Prc", value: 50.0 }),
            ),
        );
        let blend = Descriptor::new("blendOptions")
            .with("Md  ", Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new("Mltp") })
            .with("fillOpacity", Value::UnitFloat { unit: *b"#Prc", value: 25.0 });
        AslStyle {
            name: name.into(),
            id: format!("id-{name}"),
            style: Descriptor::new("Styl")
                .with("documentMode", Value::Descriptor(Descriptor::new("documentMode")))
                .with("Lefx", Value::Descriptor(lefx))
                .with("blendOptions", Value::Descriptor(blend)),
        }
    }

    fn sample() -> AslFile {
        AslFile { patterns: vec![pattern()], pattern_error: None, styles: vec![style("Gold"), style(""), style("Ünïcode ✓")] }
    }

    #[test]
    fn round_trip() {
        let f = sample();
        let b = write(&f).unwrap();
        assert_eq!(&b[..8], b"\0\x028BSL\0\x03");
        assert_eq!(b.len() % 4, 0);
        assert_eq!(parse(&b).unwrap(), f);
        let empty = AslFile::default();
        assert_eq!(parse(&write(&empty).unwrap()).unwrap(), empty);
    }

    #[test]
    fn record_lengths_without_padding_also_read() {
        // A writer that leaves the padding out of each record length.
        let f = sample();
        let mut out = write(&AslFile { styles: vec![], ..f.clone() }).unwrap();
        out.truncate(out.len() - 4);
        out.extend_from_slice(&(f.styles.len() as u32).to_be_bytes());
        for s in &f.styles {
            let identity = Descriptor::new("null")
                .with("Nm  ", Value::Text(UnicodeString::new_nul(&s.name)))
                .with("Idnt", Value::Text(UnicodeString::new_nul(&s.id)));
            let mut body = VersionedDescriptor::new(identity).to_bytes();
            body.extend(VersionedDescriptor::new(s.style.clone()).to_bytes());
            body.push(7); // odd length
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            let pad = (4 - body.len() % 4) % 4;
            out.extend_from_slice(&body);
            out.extend(std::iter::repeat_n(0, pad));
        }
        assert_eq!(parse(&out).unwrap().styles, f.styles);
    }

    #[test]
    fn bad_pattern_section_is_skipped() {
        let mut b = write(&AslFile { patterns: vec![], pattern_error: None, styles: vec![style("A")] }).unwrap();
        // Replace the empty pattern section with 8 bytes of garbage.
        let tail = b.split_off(12);
        b.truncate(8);
        b.extend_from_slice(&8u32.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0, 4, 0xff, 0xff, 0xff, 0xff]);
        b.extend(tail);
        let f = parse(&b).unwrap();
        assert!(f.pattern_error.is_some());
        assert_eq!(f.styles.len(), 1);
    }

    #[test]
    fn hostile_input_errors() {
        assert!(parse(b"").is_err());
        assert!(parse(b"\0\x028BSL\0\x02").is_err());
        assert!(parse(b"\0\x028BPS\0\x03").is_err());
        // A count far beyond the data.
        let mut b = b"\0\x028BSL\0\x03\0\0\0\0".to_vec();
        b.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&b).is_err());
        // A pattern length beyond the data.
        assert!(parse(b"\0\x028BSL\0\x03\xff\xff\xff\xff").is_err());
        let b = write(&sample()).unwrap();
        for cut in 0..b.len() {
            assert!(parse(&b[..cut]).is_err(), "cut at {cut}");
        }
        for i in 0..b.len() {
            for x in [0x5a, 0xff, 0x01] {
                let mut c = b.clone();
                c[i] ^= x;
                let _ = parse(&c);
            }
        }
        // Seeded random noise after a valid header.
        let mut seed = 0x1234_5678u32;
        for _ in 0..500 {
            let mut c = b"\0\x028BSL\0\x03".to_vec();
            for _ in 0..(seed % 200) {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                c.push(seed as u8);
            }
            let _ = parse(&c);
        }
    }
}
