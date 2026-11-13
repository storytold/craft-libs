//! DICOM (medical imaging) reader, uncompressed transfer syntaxes only.
//!
//! Supports what a viewer needs from a single-frame image: MONOCHROME1/2 (gray) and RGB or
//! YBR_FULL, 8- and 16-bit samples, interleaved or planar, both byte orders and explicit or
//! implicit VR. Undefined-length sequences outside the pixel data are skipped (clinical files
//! contain them); compressed transfer syntaxes, encapsulated pixel data and other photometric
//! interpretations are refused with a clear error. Read-only in this crate.

use std::collections::HashMap;

use crate::error::CodecError;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Dicom;
const PREAMBLE: usize = 132; // 128 zero bytes + "DICM"
const UNDEFINED: u32 = 0xFFFF_FFFF;

fn bad(msg: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, msg)
}

/// A DICOM file begins with a 128-byte preamble and the `DICM` magic.
pub(crate) fn is_dicom(b: &[u8]) -> bool {
    b.get(128..132) == Some(b"DICM")
}

#[derive(Clone, Copy)]
struct Endian {
    big: bool,
}

/// The transfer syntax: byte order and whether value representations are explicit.
#[derive(Clone, Copy)]
struct Syntax {
    endian: Endian,
    explicit: bool,
}

/// The value representations whose length field is 2 reserved bytes + `u32` in explicit VR.
fn long_vr(vr: &[u8]) -> bool {
    matches!(vr, b"OB" | b"OW" | b"OF" | b"SQ" | b"UT" | b"UN" | b"OD" | b"OL" | b"OV" | b"UC" | b"UR")
}

/// A cursor over the byte stream.
struct Cur<'a> {
    b: &'a [u8],
    pos: usize,
    big: bool,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        let c = self.take(2)?;
        Some(if self.big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
    }
    fn u32(&mut self) -> Option<u32> {
        let c = self.take(4)?;
        Some(if self.big { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) })
    }
    /// Skip `len` bytes, refusing to run past the buffer.
    fn skip(&mut self, len: usize) -> Result<(), CodecError> {
        self.pos = self.pos.checked_add(len).filter(|p| *p <= self.b.len()).ok_or_else(|| bad("a value runs past the end of the file"))?;
        Ok(())
    }
}

/// Read one element's tag and value length, leaving the cursor at the value. Item and delimiter
/// tags (group `FFFE`) carry a `u32` length with no VR even under explicit VR.
fn read_header(c: &mut Cur, syn: &Syntax) -> Result<(u16, u16, [u8; 2], u32), CodecError> {
    let group = c.u16().ok_or_else(|| bad("truncated element tag"))?;
    let elem = c.u16().ok_or_else(|| bad("truncated element tag"))?;
    let (vr, len) = if group == 0xFFFE {
        ([0, 0], c.u32().ok_or_else(|| bad("truncated item length"))?)
    } else if syn.explicit {
        let vr = c.take(2).ok_or_else(|| bad("truncated VR"))?;
        let v = [vr[0], vr[1]];
        let len = if long_vr(&v) {
            c.take(2).ok_or_else(|| bad("truncated VR"))?; // reserved
            c.u32().ok_or_else(|| bad("truncated length"))?
        } else {
            u32::from(c.u16().ok_or_else(|| bad("truncated length"))?)
        };
        (v, len)
    } else {
        ([0, 0], c.u32().ok_or_else(|| bad("truncated length"))?)
    };
    Ok((group, elem, vr, len))
}

/// Skip an undefined-length sequence, ending just after its `FFFE,E0DD` delimiter. Nesting is
/// tracked through undefined-length items and nested sequences; everything else is skipped by
/// its length. Returns the position after the delimiter.
fn skip_undefined(c: &mut Cur, syn: &Syntax) -> Result<(), CodecError> {
    let mut depth = 1u32;
    while depth > 0 {
        let (group, elem, _, len) = read_header(c, syn)?;
        // Item (E00D) and sequence (E0DD) delimiters close one undefined length; they carry no
        // content of their own.
        if group == 0xFFFE && matches!(elem, 0xE00D | 0xE0DD) {
            depth -= 1;
            continue;
        }
        if len == UNDEFINED {
            depth += 1;
        } else {
            c.skip(len as usize)?;
        }
    }
    Ok(())
}

/// Walk the elements from `start`, calling `f` with each `(group, element, value)`. An
/// undefined-length sequence outside the pixel data is skipped; encapsulated (undefined-length)
/// pixel data is refused. Returns the number of bytes walked.
fn elements(b: &[u8], start: usize, syn: &Syntax, mut f: impl FnMut(u16, u16, &Elem) -> Result<(), CodecError>) -> Result<usize, CodecError> {
    let mut c = Cur { b, pos: start, big: syn.endian.big };
    loop {
        if c.pos >= b.len() {
            return Ok(c.pos - start);
        }
        let (group, elem, vr, len) = read_header(&mut c, syn)?;
        if len == UNDEFINED {
            if (group, elem) == (0x7FE0, 0x0010) {
                return Err(CodecError::unsupported(F, "encapsulated (compressed) pixel data"));
            }
            skip_undefined(&mut c, syn)?;
            continue;
        }
        let len = len as usize;
        let start = c.pos;
        if b.len() < start.saturating_add(len) {
            return Err(bad("an element value runs past the end of the file"));
        }
        f(group, elem, &Elem { vr, start, len })?;
        c.pos = start + len;
    }
}

/// A parsed element: its VR and the byte range of its value.
struct Elem {
    vr: [u8; 2],
    start: usize,
    len: usize,
}

fn u16_val(b: &[u8], e: &Elem, big: bool) -> Option<u16> {
    let c = b.get(e.start..e.start.checked_add(2)?)?;
    Some(if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
}

/// The transfer syntax (byte order and explicit/implicit VR) and the data-set start. The file
/// meta group is always explicit VR little-endian; the data set follows its `(0002,0010)`
/// TransferSyntaxUID.
fn transfer_syntax(b: &[u8]) -> Result<(Syntax, usize), CodecError> {
    let meta = Syntax { endian: Endian { big: false }, explicit: true };
    let mut ts: Option<String> = None;
    let mut data_start = b.len();
    let mut c = Cur { b, pos: PREAMBLE, big: false };
    while c.pos + 8 <= b.len() {
        let save = c.pos;
        let (group, elem, _, len) = read_header(&mut c, &meta)?;
        if group != 0x0002 {
            // A non-meta group begins the data set.
            data_start = save;
            break;
        }
        if len == UNDEFINED {
            return Err(CodecError::unsupported(F, "undefined-length meta element"));
        }
        let start = c.pos;
        if b.len() < start.saturating_add(len as usize) {
            return Err(bad("a meta element runs past the end of the file"));
        }
        if elem == 0x0010 {
            ts = Some(String::from_utf8_lossy(&b[start..start + len as usize]).trim_end_matches(['\0', ' ']).to_string());
        }
        c.pos = start + len as usize;
    }
    let syn = match ts.as_deref() {
        Some("1.2.840.10008.1.2") => Syntax { endian: Endian { big: false }, explicit: false },
        Some("1.2.840.10008.1.2.1") => Syntax { endian: Endian { big: false }, explicit: true },
        Some("1.2.840.10008.1.2.2") => Syntax { endian: Endian { big: true }, explicit: true },
        Some(other) => return Err(CodecError::unsupported(F, format!("transfer syntax {other} (only uncompressed is read)"))),
        // No meta: the historical default is implicit VR little-endian.
        None => Syntax { endian: Endian { big: false }, explicit: false },
    };
    Ok((syn, data_start))
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if !is_dicom(b) {
        return Err(bad("not a DICOM file (no DICM magic)"));
    }
    let (syn, start) = transfer_syntax(b)?;
    let mut tags: HashMap<(u16, u16), Elem> = HashMap::new();
    elements(b, start, &syn, |g, e, el| {
        // Later duplicates must not silently override the first (DICOM forbids duplicates).
        tags.entry((g, e)).or_insert_with(|| Elem { vr: el.vr, start: el.start, len: el.len });
        Ok(())
    })?;
    let big = syn.endian.big;
    let get_u16 = |tag: (u16, u16)| tags.get(&tag).and_then(|e| u16_val(b, e, big));
    let rows = u32::from(get_u16((0x0028, 0x0010)).ok_or_else(|| bad("missing Rows (0028,0010)"))?);
    let cols = u32::from(get_u16((0x0028, 0x0011)).ok_or_else(|| bad("missing Columns (0028,0011)"))?);
    let samples = u32::from(get_u16((0x0028, 0x0002)).unwrap_or(1));
    let bits = get_u16((0x0028, 0x0100)).ok_or_else(|| bad("missing BitsAllocated (0028,0100)"))?;
    let bits_stored = get_u16((0x0028, 0x0101)).unwrap_or(bits);
    let high_bit = get_u16((0x0028, 0x0102)).unwrap_or(bits_stored.saturating_sub(1));
    if u32::from(high_bit) >= u32::from(bits) {
        return Err(bad("HighBit is outside the allocated bits"));
    }
    let signed = get_u16((0x0028, 0x0103)).unwrap_or(0) == 1;
    let planar = get_u16((0x0028, 0x0006)).unwrap_or(0);
    let photometric = tags
        .get(&(0x0028, 0x0004))
        .and_then(|e| b.get(e.start..e.start.checked_add(e.len)?))
        .map(|v| String::from_utf8_lossy(v).trim_end_matches(['\0', ' ']).to_string())
        .ok_or_else(|| bad("missing PhotometricInterpretation (0028,0004)"))?;
    let pixel = tags.get(&(0x7FE0, 0x0010)).ok_or_else(|| CodecError::unsupported(F, "no pixel data (7FE0,0010)"))?;
    if !matches!(bits, 8 | 16) || !matches!(bits_stored, 1..=16) {
        return Err(CodecError::unsupported(F, format!("{bits}-bit allocated / {bits_stored}-bit stored samples")));
    }
    let mono = matches!(photometric.as_str(), "MONOCHROME1" | "MONOCHROME2");
    let rgb = photometric == "RGB";
    let ybr = photometric == "YBR_FULL";
    if !(mono || rgb || ybr) {
        return Err(CodecError::unsupported(F, format!("photometric interpretation {photometric}")));
    }
    // Mono stores one sample; RGB and YBR_FULL store three. Any other count would index past the
    // pixel data, so it is refused rather than misread.
    let want_samples = if mono { 1 } else { 3 };
    if samples != want_samples {
        return Err(CodecError::unsupported(F, format!("{samples} samples per pixel with {photometric}")));
    }
    if ybr && bits != 8 {
        return Err(CodecError::unsupported(F, format!("{bits}-bit {photometric} (8-bit only)")));
    }
    let layout = if mono { ChannelLayout::Gray } else { ChannelLayout::Rgb };
    let out_sample = if bits == 8 { SampleType::U8 } else { SampleType::U16 };
    limits.check(cols, rows, layout, out_sample)?;

    let pixels = (cols as usize).checked_mul(rows as usize).ok_or_else(|| CodecError::LimitExceeded("image too large".into()))?;
    let n = pixels.checked_mul(samples as usize).ok_or_else(|| CodecError::LimitExceeded("image too large".into()))?;
    let bytes_per = if bits == 16 { 2usize } else { 1 };
    let need = n.checked_mul(bytes_per).ok_or_else(|| CodecError::LimitExceeded("pixel data too large".into()))?;
    let data = b
        .get(pixel.start..pixel.start.checked_add(need).ok_or_else(|| CodecError::LimitExceeded("pixel data too large".into()))?)
        .ok_or_else(|| bad("pixel data is truncated"))?;
    let out_channels = layout.channels();
    let out_len = pixels.checked_mul(out_channels).and_then(|k| k.checked_mul(bytes_per)).ok_or_else(|| CodecError::LimitExceeded("image too large".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(out_len).map_err(|_| CodecError::LimitExceeded("cannot allocate the image".into()))?;
    // Raw sample of channel `ch` at pixel `p`, honouring the planar configuration.
    let sample_at = |p: usize, ch: usize| -> Option<u32> {
        let idx = if planar == 1 { ch.checked_mul(pixels)?.checked_add(p)? } else { p.checked_mul(samples as usize)?.checked_add(ch)? };
        let off = idx.checked_mul(bytes_per)?;
        if bits == 16 {
            let c = data.get(off..off.checked_add(2)?)?;
            Some(if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }.into())
        } else {
            Some(u32::from(*data.get(off)?))
        }
    };
    let stored_max = f64::from((1u32 << bits_stored) - 1);
    // Extract the `bits_stored` samples (the least significant one at `high_bit`) and scale to
    // [0, 1]; signed values are shifted up by half a range first. `high_bit` is 1-based here.
    let shift = (u32::from(high_bit) + 1).saturating_sub(u32::from(bits_stored));
    let mask = (1u32 << bits_stored) - 1;
    let half = 1u32 << (bits_stored - 1);
    let norm = move |raw: u32| -> f64 {
        let v = (raw >> shift) & mask;
        // Signed: flip the sign bit to map [-2^(b-1), 2^(b-1)-1] onto [0, 2^b-1].
        let v = if signed { v ^ half } else { v };
        (f64::from(v) / stored_max).clamp(0.0, 1.0)
    };
    let q = |x: f64| -> u16 { (x * 65535.0).round() as u16 };
    let push = |out: &mut Vec<u8>, x: f64| {
        if bits == 8 {
            out.push((x * 255.0).round() as u8);
        } else {
            out.extend_from_slice(&q(x).to_ne_bytes());
        }
    };
    for p in 0..pixels {
        let s = |ch: usize| sample_at(p, ch).ok_or_else(|| bad("pixel data is shorter than the image"));
        if mono {
            let x = norm(s(0)?);
            push(&mut out, if photometric == "MONOCHROME1" { 1.0 - x } else { x });
        } else if rgb {
            for ch in 0..3 {
                push(&mut out, norm(s(ch)?));
            }
        } else {
            // YBR_FULL (8-bit): BT.601 full-range to RGB.
            let y = f64::from(s(0)?);
            let cb = f64::from(s(1)?) - 128.0;
            let cr = f64::from(s(2)?) - 128.0;
            let r = (y + 1.402 * cr).clamp(0.0, 255.0);
            let g = (y - 0.344_136 * cb - 0.714_136 * cr).clamp(0.0, 255.0);
            let bl = (y + 1.772 * cb).clamp(0.0, 255.0);
            for v in [r, g, bl] {
                push(&mut out, v / 255.0);
            }
        }
    }
    Image::from_raw(cols, rows, layout, out_sample, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the file meta group (explicit VR little-endian), including the given extra
    /// `(element, ascii)` pairs before the TransferSyntaxUID.
    fn meta(b: &mut Vec<u8>, ts: &str, extra: &[(u16, &str)]) {
        let ui = |e: u16, v: &str, b: &mut Vec<u8>| {
            let mut s = v.as_bytes().to_vec();
            if s.len() % 2 == 1 {
                s.push(0);
            }
            b.extend_from_slice(&0x0002u16.to_le_bytes());
            b.extend_from_slice(&e.to_le_bytes());
            b.extend_from_slice(b"UI");
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
            b.extend_from_slice(&s);
        };
        for (e, v) in extra {
            ui(*e, v, b);
        }
        ui(0x0010, ts, b);
    }

    fn us(b: &mut Vec<u8>, g: u16, e: u16, v: u16, explicit: bool) {
        b.extend_from_slice(&g.to_le_bytes());
        b.extend_from_slice(&e.to_le_bytes());
        if explicit {
            b.extend_from_slice(b"US");
            b.extend_from_slice(&2u16.to_le_bytes());
        } else {
            b.extend_from_slice(&2u32.to_le_bytes());
        }
        b.extend_from_slice(&v.to_le_bytes());
    }

    fn cs(b: &mut Vec<u8>, g: u16, e: u16, v: &str, explicit: bool) {
        let mut s = v.as_bytes().to_vec();
        if s.len() % 2 == 1 {
            s.push(0);
        }
        b.extend_from_slice(&g.to_le_bytes());
        b.extend_from_slice(&e.to_le_bytes());
        if explicit {
            b.extend_from_slice(b"CS");
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
        } else {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
        }
        b.extend_from_slice(&s);
    }

    /// A DICOM with the given transfer syntax UID and data-set encoding.
    #[allow(clippy::too_many_arguments)]
    fn synth(ts: &str, explicit: bool, big: bool, planar: u16, rows: u16, cols: u16, samples: u16, bits: u16, photometric: &str, pixels: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b.extend_from_slice(b"DICM");
        // One extra meta element before the syntax, so a reader that only looks at the last
        // group-0002 element takes the wrong one.
        meta(&mut b, ts, &[(0x0002, "1.2.840.10008.5.1.4.1.1.7")]);
        let _ = big;
        us(&mut b, 0x0028, 0x0010, rows, explicit);
        us(&mut b, 0x0028, 0x0011, cols, explicit);
        us(&mut b, 0x0028, 0x0002, samples, explicit);
        us(&mut b, 0x0028, 0x0006, planar, explicit);
        cs(&mut b, 0x0028, 0x0004, photometric, explicit);
        us(&mut b, 0x0028, 0x0100, bits, explicit);
        us(&mut b, 0x0028, 0x0101, bits, explicit);
        us(&mut b, 0x0028, 0x0102, bits - 1, explicit);
        us(&mut b, 0x0028, 0x0103, 0, explicit);
        b.extend_from_slice(&0x7FE0u16.to_le_bytes());
        b.extend_from_slice(&0x0010u16.to_le_bytes());
        if explicit {
            b.extend_from_slice(b"OB");
            b.extend_from_slice(&0u16.to_le_bytes());
        }
        b.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        b.extend_from_slice(pixels);
        b
    }

    #[test]
    fn decodes_8bit_monochrome() {
        let b = synth("1.2.840.10008.1.2.1", true, false, 0, 2, 2, 1, 8, "MONOCHROME2", &[0, 51, 102, 255]);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (2, 2, ChannelLayout::Gray, SampleType::U8));
        assert_eq!(img.data(), &[0, 51, 102, 255]);
    }

    #[test]
    fn decodes_16bit_monochrome_and_scales() {
        let px: Vec<u8> = [0u16, 32768, 65535].iter().flat_map(|v| v.to_le_bytes()).collect();
        let b = synth("1.2.840.10008.1.2.1", true, false, 0, 3, 1, 1, 16, "MONOCHROME2", &px);
        let img = decode(&b, &Limits::default()).unwrap();
        let v = img.to_u16_samples().unwrap();
        assert_eq!(v[0], 0);
        assert!((i32::from(v[1]) - 32768).abs() <= 1);
        assert_eq!(v[2], 65535);
    }

    #[test]
    fn monochrome1_is_inverted() {
        let b = synth("1.2.840.10008.1.2.1", true, false, 0, 2, 1, 1, 8, "MONOCHROME1", &[0, 255]);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[255, 0]);
    }

    #[test]
    fn decodes_rgb_interleaved_and_planar() {
        // 2 pixels: (10,20,30) and (40,50,60).
        let interleaved = synth("1.2.840.10008.1.2.1", true, false, 0, 1, 2, 3, 8, "RGB", &[10, 20, 30, 40, 50, 60]);
        let img = decode(&interleaved, &Limits::default()).unwrap();
        assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgb, &[10u8, 20, 30, 40, 50, 60][..]));
        // Planar 0: R plane, then G, then B.
        let planar = synth("1.2.840.10008.1.2.1", true, false, 1, 1, 2, 3, 8, "RGB", &[10, 40, 20, 50, 30, 60]);
        let img = decode(&planar, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn decodes_ybr_full_to_rgb() {
        // White (Y=128, Cb=Cr=128) stays neutral grey.
        let b = synth("1.2.840.10008.1.2.1", true, false, 0, 1, 1, 3, 8, "YBR_FULL", &[128, 128, 128]);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.layout(), ChannelLayout::Rgb);
        let d = img.data();
        assert!(d[0].abs_diff(d[1]) <= 1 && d[1].abs_diff(d[2]) <= 1, "{d:?}");
    }

    #[test]
    fn decodes_implicit_vr() {
        let b = synth("1.2.840.10008.1.2", false, false, 0, 2, 2, 1, 8, "MONOCHROME2", &[0, 51, 102, 255]);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.layout(), ChannelLayout::Gray);
        assert_eq!(img.data(), &[0, 51, 102, 255]);
    }

    #[test]
    fn skips_an_undefined_length_sequence_outside_the_pixel_data() {
        // Insert an undefined-length SQ (0008,2218) with one item, then a defined-length one,
        // before the pixel data: the reader must skip both and still decode the image.
        let mut b = vec![0u8; 128];
        b.extend_from_slice(b"DICM");
        meta(&mut b, "1.2.840.10008.1.2.1", &[]);
        for (g, e, v) in [
            (0x0028, 0x0010, 1u16),
            (0x0028, 0x0011, 1),
            (0x0028, 0x0002, 1),
            (0x0028, 0x0100, 8),
            (0x0028, 0x0101, 8),
            (0x0028, 0x0102, 7),
            (0x0028, 0x0103, 0),
        ] {
            us(&mut b, g, e, v, true);
        }
        cs(&mut b, 0x0028, 0x0004, "MONOCHROME2", true);
        // Undefined-length sequence (0008,2211), one undefined-length item containing a nested
        // defined-length sequence, then the delimiters.
        b.extend_from_slice(&[0x08, 0x00, 0x11, 0x22, b'S', b'Q', 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.extend_from_slice(&[0xFE, 0xFF, 0x00, 0xE0, 0xFF, 0xFF, 0xFF, 0xFF]); // item, undefined
        b.extend_from_slice(&[0x08, 0x00, 0x55, 0x22, b'S', b'Q', 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 1, 2, 3, 4]); // nested, defined
        b.extend_from_slice(&[0xFE, 0xFF, 0x0D, 0xE0, 0x00, 0x00, 0x00, 0x00]); // item delimitation
        b.extend_from_slice(&[0xFE, 0xFF, 0xDD, 0xE0, 0x00, 0x00, 0x00, 0x00]); // sequence delimitation
        // Pixel data (defined length).
        b.extend_from_slice(&0x7FE0u16.to_le_bytes());
        b.extend_from_slice(&0x0010u16.to_le_bytes());
        b.extend_from_slice(b"OB");
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(200);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[200]);
    }

    #[test]
    fn truncated_and_bad_is_error_not_panic() {
        let b = synth("1.2.840.10008.1.2.1", true, false, 0, 2, 2, 1, 8, "MONOCHROME2", &[0, 1, 2, 3]);
        for cut in [0, 4, 128, 132, 140, b.len() - 1] {
            assert!(decode(&b[..cut.min(b.len())], &Limits::default()).is_err(), "cut {cut}");
        }
        assert!(decode(b"not dicom", &Limits::default()).is_err());
    }

    #[test]
    fn high_bit_outside_the_allocated_bits_is_an_error_not_a_panic() {
        let mut b = synth("1.2.840.10008.1.2.1", true, false, 0, 1, 1, 1, 8, "MONOCHROME2", &[7]);
        let tag = [0x28, 0x00, 0x02, 0x01, b'U', b'S', 0x02, 0x00];
        let at = b.windows(tag.len()).position(|w| w == tag).unwrap() + tag.len();
        b[at..at + 2].copy_from_slice(&200u16.to_le_bytes());
        assert!(decode(&b, &Limits::default()).is_err());
    }

    #[test]
    fn rejects_compressed_transfer_syntax() {
        let mut b = synth("1.2.840.10008.1.2.1", true, false, 0, 1, 1, 1, 8, "MONOCHROME2", &[0]);
        let uncompressed = b"1.2.840.10008.1.2.1\0";
        let compressed = b"1.2.840.10008.1.2.4\0"; // same length, a compressed family
        let at = b.windows(uncompressed.len()).position(|w| w == uncompressed).expect("transfer syntax in the fixture");
        b[at..at + compressed.len()].copy_from_slice(compressed);
        assert!(matches!(decode(&b, &Limits::default()), Err(CodecError::Unsupported { .. })));
    }
}
