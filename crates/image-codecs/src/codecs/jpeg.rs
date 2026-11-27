//! JPEG: decode via `zune-jpeg` (gray, YCbCr, CMYK, YCCK; arithmetic-coded and 12-bit files
//! via [`craft_jpeg`]), encode via
//! `jpeg-encoder` (gray, RGB, CMYK). Metadata (APP0 JFIF density, APP1
//! EXIF/XMP, APP2 multi-segment ICC, APP13 Photoshop resources, APP14 Adobe) is
//! parsed by our own marker scanner so behaviour does not depend on decoder internals.
//!
//! **Resolution** (#1691) can be recorded four times; like Photoshop the first usable one of
//! these wins: the APP13 Photoshop ResolutionInfo, the XMP `tiff:` resolution, the EXIF IFD0
//! resolution, and last the JFIF density (a unit-0 JFIF density is only an aspect ratio and
//! never counts). Cameras record their resolution in EXIF only, and many tools write a default
//! 72 into JFIF, so JFIF can't outrank EXIF (see [`crate::resolution`]). On export RGB and gray
//! images carry density in JFIF; CMYK omits JFIF APP0, which is invalid for four components, and
//! carries an available density in EXIF instead.

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, JpegFormat, Limits};
use crate::resolution::{exif_resolution, export_exif, export_xmp, photoshop_resolution, xmp_resolution};

const F: Format = Format::Jpeg;
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const ICC_HEADER: &[u8] = b"ICC_PROFILE\0";
const PHOTOSHOP_HEADER: &[u8] = b"Photoshop 3.0\0";

#[derive(Default, Debug)]
pub(crate) struct JpegMeta {
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<String>,
    pub dpi: Option<(f32, f32)>,
    /// The APP13 Photoshop image resource blocks (`8BIM` ...), segments joined, without the header.
    pub photoshop: Option<Vec<u8>>,
    /// Whether an Adobe APP14 marker identifies the four-component samples as inverted CMYK.
    pub adobe_marker: bool,
    pub adobe_transform: Option<u8>,
    /// (width, height, components) from the first SOFn.
    pub frame: Option<(u32, u32, u8)>,
    /// [`crate::JpegInfo::held_coefficients`] of the first SOFn and the first scan.
    pub held_coefficients: u64,
}

/// Validate a JPEG by decoding the 8-bit RGBA output used by renderers.
///
/// This deliberately uses the existing zune backend's default decode policy.
/// Unlike general JPEG import it does not fall back to additional JPEG modes,
/// apply orientation, or retain pixels. The caller supplies the admitted pixel count.
pub fn validate_jpeg_rgba8(bytes: &[u8], max_pixels: u64) -> Result<(), CodecError> {
    validate_jpeg_rgba8_with_panic_policy(bytes, max_pixels, cfg!(panic = "abort"))
}

fn validate_jpeg_rgba8_with_panic_policy(bytes: &[u8], max_pixels: u64, panic_aborts: bool) -> Result<(), CodecError> {
    // The decoder can assert while upsampling these frames. On abort builds the
    // unwind guard below cannot turn that assertion into an image omission.
    if panic_aborts && first_component_subsampled(bytes) {
        return Err(CodecError::malformed(F, "JPEG first component is subsampled below the frame maximum"));
    }
    let decode = || {
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
        decoder.decode_headers().map_err(|error| CodecError::malformed(F, format_args!("{error:?}")))?;
        let info = decoder.info().ok_or_else(|| CodecError::malformed(F, "missing frame dimensions"))?;
        let pixels = u64::from(info.width) * u64::from(info.height);
        if pixels == 0 {
            return Err(CodecError::malformed(F, "empty frame"));
        }
        if pixels > max_pixels {
            return Err(CodecError::LimitExceeded("JPEG exceeds the admitted pixel count".into()));
        }
        let length = usize::try_from(pixels * 4).map_err(|_| CodecError::LimitExceeded("JPEG RGBA output is too large".into()))?;
        if decoder.output_colorspace() != Some(ColorSpace::RGBA) || decoder.output_buffer_size() != Some(length) {
            return Err(CodecError::malformed(F, "unexpected RGBA output"));
        }
        let mut output = Vec::new();
        output.try_reserve_exact(length).map_err(|_| CodecError::LimitExceeded("could not allocate JPEG RGBA output".into()))?;
        output.resize(length, 0);
        decoder.decode_into(&mut output).map_err(|error| CodecError::malformed(F, format_args!("{error:?}")))
    };
    std::panic::catch_unwind(decode).map_err(|_| CodecError::malformed(F, "JPEG decoder failed on this frame"))?
}

/// The first frame of `b` as [`crate::JpegInfo`], from its markers alone (see [`crate::jpeg_info`]).
pub(crate) fn info(b: &[u8]) -> Option<crate::JpegInfo> {
    let m = scan_metadata(b);
    let (width, height, components) = m.frame?;
    let layout = match components {
        1 => ChannelLayout::Gray,
        3 => ChannelLayout::Rgb,
        4 => ChannelLayout::Cmyk,
        _ => return None,
    };
    let held_coefficients = m.held_coefficients;
    Some(crate::JpegInfo { width, height, layout, whole_coefficients: uses_craft_jpeg(b), held_coefficients })
}

/// Bytes of the coefficients `zune-jpeg` holds beside its output for a frame it can't turn into
/// rows as it reads (see [`crate::JpegInfo::held_coefficients`]), from the frame's header
/// segment `seg` and the number of components in its first scan: per component, a 64-value
/// block of `i16`s for each of its H × V blocks in every MCU (A.2), the MCUs covering the frame
/// at the largest sampling factors. 0 for any other frame.
fn held_coefficients(marker: u8, seg: &[u8], scan_components: Option<u8>) -> u64 {
    let &[8, h0, h1, w0, w1, n, ref comps @ ..] = seg else { return 0 };
    // Progressive scans refine every block in turn; a sequential first scan without every
    // component leaves its rows unfinished until the scans that bring the others.
    let held = match marker {
        0xC2 => true,
        0xC0 | 0xC1 => scan_components.is_some_and(|s| s < n),
        _ => false,
    };
    if !held {
        return 0;
    }
    let (h, w) = (u64::from(u16::from_be_bytes([h0, h1])), u64::from(u16::from_be_bytes([w0, w1])));
    let factors: Vec<(u64, u64)> =
        comps.as_chunks::<3>().0.iter().take(usize::from(n)).map(|&[_, f, _]| (u64::from(f >> 4), u64::from(f & 15))).collect();
    let hmax = factors.iter().map(|f| f.0).max().unwrap_or(1).max(1);
    let vmax = factors.iter().map(|f| f.1).max().unwrap_or(1).max(1);
    let mcus = w.div_ceil(8 * hmax) * h.div_ceil(8 * vmax);
    factors.iter().map(|&(fh, fv)| mcus * fh * fv * 64 * 2).sum()
}

/// Bytes of DCT coefficients to budget for before decoding hostile `b`: the larger of
/// [`crate::JpegInfo::held_coefficients`]'s rule (first frame; SOF2, or SOF0/SOF1 whose first
/// scan lacks components; 8-bit only; sampling factors as written) and a coarser one that
/// keeps a progressive JPEG's coefficients for the whole picture:
/// - every SOF2, SOF6, SOF10 and SOF14 header before the first scan counts, at any precision, and
///   the largest wins; each charges 2 bytes a coefficient: per component, `2 * 64 * h * v` per
///   MCU, with `h` and `v` clamped to 1..=4 and the MCUs covering the frame at the largest `h`, `v`;
/// - a header that can't be followed (a segment that is cut, a length under 2, a frame header
///   without its components, no `0xFF` where a marker belongs) charges the costliest frame,
///   four components sampled 4 × 4, at the first frame's size (as `scan_metadata` reads it), and 0 without one.
///
/// Never less than either rule. Frames `craft-jpeg` decodes (arithmetic, lossless, 12-bit)
/// enforce `max_alloc` on their own buffers, and SOF6/SOF14 (hierarchical) are not decoded at all,
/// so charging them is only a conservative budget. Arithmetic saturates; the scan is linear.
pub(crate) fn worst_case_coefficients(b: &[u8]) -> u64 {
    let m = scan_metadata(b);
    let mut at = 2usize;
    let mut most = 0u64;
    let first: Option<(u64, u64)> = m.frame.map(|(w, h, _)| (u64::from(w), u64::from(h)));
    let mut broken = false;
    while !broken {
        if b.get(at) != Some(&0xFF) {
            broken = true;
            break;
        }
        while b.get(at) == Some(&0xFF) {
            at += 1;
        }
        let Some(&code) = b.get(at) else {
            broken = true;
            break;
        };
        at += 1;
        match code {
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => break,
            _ => {}
        }
        let len = match (b.get(at), b.get(at + 1)) {
            (Some(&hi), Some(&lo)) => usize::from(u16::from_be_bytes([hi, lo])),
            _ => 0,
        };
        let Some(seg) = (len >= 2).then(|| b.get(at + 2..at + len)).flatten() else {
            broken = true;
            break;
        };
        if matches!(code, 0xC2 | 0xC6 | 0xCA | 0xCE) {
            match progressive_frame_coefficients(seg) {
                Some(held) => most = most.max(held),
                None => {
                    broken = true;
                    break;
                }
            }
        }
        at += len;
    }
    if broken && let Some((w, h)) = first {
        let mcus = w.div_ceil(32).saturating_mul(h.div_ceil(32));
        most = most.max(mcus.saturating_mul(2 * 64 * 16 * 4));
    }
    most.max(m.held_coefficients)
}

/// [`worst_case_coefficients`]'s charge for one frame header `sof` (after its length).
fn progressive_frame_coefficients(sof: &[u8]) -> Option<u64> {
    let height = u64::from(u16::from_be_bytes([*sof.get(1)?, *sof.get(2)?]));
    let width = u64::from(u16::from_be_bytes([*sof.get(3)?, *sof.get(4)?]));
    let n = usize::from(*sof.get(5)?);
    let comps = sof.get(6..6usize.checked_add(n.checked_mul(3)?)?)?;
    let factors: Vec<(u64, u64)> =
        comps.as_chunks::<3>().0.iter().map(|&[_, f, _]| (u64::from(f >> 4).clamp(1, 4), u64::from(f & 15).clamp(1, 4))).collect();
    let (hmax, vmax) = factors.iter().fold((1, 1), |(h, v), &(a, b)| (h.max(a), v.max(b)));
    let mcus = width.div_ceil(8 * hmax).saturating_mul(height.div_ceil(8 * vmax));
    Some(factors.iter().fold(0u64, |sum, &(h, v)| sum.saturating_add(mcus.saturating_mul(2 * 64 * h * v))))
}

/// Walk the marker segments up to SOS and collect metadata.
pub(crate) fn scan_metadata(b: &[u8]) -> JpegMeta {
    let mut m = JpegMeta::default();
    let mut icc_parts: Vec<(u8, u8, &[u8])> = Vec::new();
    // Photoshop splits a large resource block over consecutive APP13 segments.
    let mut photoshop: Vec<u8> = Vec::new();
    let mut jfif_dpi = None;
    // The first frame header and the number of components in the first scan.
    let mut sof: Option<(u8, &[u8])> = None;
    let mut scan_components = None;
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            break;
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            scan_components = (marker == 0xDA).then(|| b.get(i + 4).copied()).flatten();
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 || i + 2 + len > b.len() {
            break;
        }
        let seg = &b[i + 4..i + 2 + len];
        match marker {
            0xE0 if seg.len() >= 12 && seg.starts_with(b"JFIF\0") && jfif_dpi.is_none() => {
                let unit = seg[7];
                let x = u16::from_be_bytes([seg[8], seg[9]]) as f32;
                let y = u16::from_be_bytes([seg[10], seg[11]]) as f32;
                if x > 0.0 && y > 0.0 {
                    jfif_dpi = match unit {
                        1 => Some((x, y)),
                        2 => Some((x * 2.54, y * 2.54)),
                        _ => None,
                    };
                }
            }
            0xE1 if seg.starts_with(EXIF_HEADER) && m.exif.is_none() => {
                m.exif = Some(seg[EXIF_HEADER.len()..].to_vec());
            }
            0xE1 if seg.starts_with(XMP_HEADER) && m.xmp.is_none() => {
                m.xmp = std::str::from_utf8(&seg[XMP_HEADER.len()..]).ok().map(|s| s.trim_end_matches('\0').to_owned());
            }
            0xED if seg.starts_with(PHOTOSHOP_HEADER) && photoshop.len() < 1 << 24 => {
                photoshop.extend_from_slice(&seg[PHOTOSHOP_HEADER.len()..]);
            }
            0xE2 if seg.len() >= 14 && seg.starts_with(ICC_HEADER) => {
                icc_parts.push((seg[12], seg[13], &seg[14..]));
            }
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) && seg.len() >= 6 && m.frame.is_none() => {
                let h = u16::from_be_bytes([seg[1], seg[2]]) as u32;
                let w = u16::from_be_bytes([seg[3], seg[4]]) as u32;
                m.frame = Some((w, h, seg[5]));
                sof = Some((marker, seg));
            }
            0xEE if seg.starts_with(b"Adobe") => {
                m.adobe_marker = true;
                if seg.len() >= 12 {
                    m.adobe_transform = Some(seg[11]);
                }
            }
            _ => {}
        }
        i += 2 + len;
    }
    m.held_coefficients = sof.map_or(0, |(marker, seg)| held_coefficients(marker, seg, scan_components));
    m.dpi = photoshop_resolution(&photoshop)
        .filter(|&d| crate::resolution_probe::usable(d))
        .or_else(|| m.xmp.as_deref().and_then(xmp_resolution).filter(|&d| crate::resolution_probe::usable(d)))
        .or_else(|| m.exif.as_deref().and_then(exif_resolution).filter(|&d| crate::resolution_probe::usable(d)))
        .or_else(|| jfif_dpi.filter(|&d| crate::resolution_probe::usable(d)));
    m.photoshop = Some(photoshop).filter(|p| !p.is_empty());
    if !icc_parts.is_empty() {
        icc_parts.sort_by_key(|p| p.0);
        let total = icc_parts[0].1 as usize;
        let seqs_ok = icc_parts.len() == total && icc_parts.iter().enumerate().all(|(k, p)| p.0 as usize == k + 1 && p.1 as usize == total);
        if seqs_ok || icc_parts.len() == 1 {
            m.icc = Some(icc_parts.iter().flat_map(|p| p.2.iter().copied()).collect());
        }
    }
    m
}

/// How a JPEG's data ends, seen from its marker structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataEnd {
    /// An end-of-image marker follows the scans (or the structure is too odd to tell).
    Complete,
    /// The file ends inside the image data, before the end-of-image marker.
    Truncated,
    /// The file ends before the first scan holds any data: nothing can be decoded.
    Empty,
}

/// Walks the marker segments (skipped by their length) and the entropy-coded scan data
/// (skipped up to the next marker that isn't a stuffed `FF 00`, a restart or a fill byte),
/// so an embedded thumbnail's end marker or data after the end of the image don't count.
/// Also returns how many scan-data bytes the walk crossed: the truncated-input budget is
/// measured on these, not on the file length, which APP padding can inflate without
/// adding image data.
fn data_end(b: &[u8]) -> (DataEnd, u64) {
    data_end_mode(b, false)
}

/// Whether JPEG `b` holds too little scan data for a `w` × `h` frame to be worth decoding: no scan
/// data at all, or under one byte for every [`MAX_PIXELS_PER_BYTE`] pixels (counted on the scan
/// data the marker walk crosses, so APP padding doesn't count). `zune-jpeg` allocates the whole
/// output before it reads a scan, so a few bytes could otherwise ask for a frame of any size.
pub(crate) fn scan_too_short(b: &[u8], w: u32, h: u32) -> bool {
    let (end, scan_bytes) = data_end(b);
    end == DataEnd::Empty || u64::from(w) * u64::from(h) / MAX_PIXELS_PER_BYTE > scan_bytes
}

/// Whether the marker walk actually reaches the first EOI, with malformed segment lengths refused.
pub(crate) fn reaches_eoi(b: &[u8]) -> bool {
    data_end_mode(b, true).0 == DataEnd::Complete
}

fn find_ff(b: &[u8]) -> Option<usize> {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let (words, tail) = b.as_chunks::<8>();
    for (i, word) in words.iter().enumerate() {
        let x = !u64::from_ne_bytes(*word);
        if x.wrapping_sub(ONES) & !x & (ONES << 7) != 0 {
            return word.iter().position(|&v| v == 0xff).map(|j| i * 8 + j);
        }
    }
    tail.iter().position(|&v| v == 0xff).map(|j| words.len() * 8 + j)
}

fn data_end_mode(b: &[u8], strict: bool) -> (DataEnd, u64) {
    // Where the bytes run out: inside the image data once a scan has held some.
    let cut = |scan_data: bool, scanned: u64| (if scan_data { DataEnd::Truncated } else { DataEnd::Empty }, scanned);
    let mut i = 2; // past SOI
    let mut scan_data = false;
    let mut scanned = 0u64;
    loop {
        let Some(&byte) = b.get(i) else { return cut(scan_data, scanned) };
        if byte != 0xFF {
            // Not where a marker should be: skip to the next one, as a lenient decoder does, so
            // segments after the stray bytes (padding among them) are still recognised. Stray
            // bytes are not scan data, so they never count (junk would buy budget as padding
            // did); with no marker left, the image data ends here.
            let Some(p) = b.get(i..).and_then(find_ff) else { return cut(scan_data, scanned) };
            i += p;
            continue;
        }
        let Some(&code) = b.get(i + 1) else { return cut(scan_data, scanned) };
        match code {
            0xD9 => return (DataEnd::Complete, scanned),
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            0x00 if strict => i += 2,
            _ => {
                let Some(len) = b.get(i + 2..i + 4).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes) else {
                    return cut(scan_data, scanned);
                };
                if strict && len < 2 {
                    return cut(scan_data, scanned);
                }
                i = i.saturating_add(2 + usize::from(len));
                if code == 0xDA {
                    let start = i;
                    loop {
                        let Some(p) = b.get(i..).and_then(find_ff) else {
                            // EOF inside this scan: everything from the scan start to the end
                            // of the file is scan data (inside a declared segment it may not
                            // be, so only count what remains).
                            return cut(scan_data || b.len() > start, scanned + u64::try_from(b.len().saturating_sub(start)).unwrap_or(0));
                        };
                        i += p;
                        match b.get(i + 1) {
                            Some(0x00 | 0xD0..=0xD7) => i += 2,
                            Some(0xFF) => i += 1,
                            _ => break,
                        }
                    }
                    scan_data |= i > start;
                    scanned += u64::try_from(i - start).unwrap_or(0);
                }
            }
        }
    }
}

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

const EXTENDED_XMP_HEADER: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

/// The file without its Extended XMP APP1 segments (`None` when it has none), for `zune-jpeg`.
///
/// zune-jpeg keeps every Extended XMP fragment until a GUID's set is complete and re-sorts and
/// re-checks all pending fragments after every marker it parses, so many small, consistent but
/// incomplete fragments (or a few followed by many other markers) cost quadratic time before a
/// pixel is decoded. Nothing here reads its reassembled Extended XMP ([`scan_metadata`] works on
/// the original bytes), so the fragments are left out of what it parses. The walk stops at the
/// first scan, as the header parse does, and skips stray bytes to the next `FF` as it does.
fn without_extended_xmp(b: &[u8]) -> Option<Vec<u8>> {
    let mut drop: Vec<std::ops::Range<usize>> = Vec::new();
    let mut i = 2; // past SOI
    while let Some(&byte) = b.get(i) {
        if byte != 0xFF {
            i += b.get(i..)?.iter().position(|&v| v == 0xFF)?;
            continue;
        }
        let Some(&code) = b.get(i + 1) else { break };
        match code {
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            0xD9 | 0xDA => break,
            _ => {
                let Some(len) = b.get(i + 2..i + 4).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes) else { break };
                let end = i.saturating_add(2 + usize::from(len));
                if code == 0xE1 && b.get(i + 4..).is_some_and(|p| p.starts_with(EXTENDED_XMP_HEADER)) {
                    drop.push(i..end.min(b.len()));
                }
                i = end;
            }
        }
    }
    if drop.is_empty() {
        return None;
    }
    // Reserve what is kept, not the input: the dropped fragments may be most of it.
    let kept = b.len().saturating_sub(drop.iter().map(|r| r.end - r.start).sum::<usize>());
    let mut out = Vec::with_capacity(kept);
    let mut at = 0;
    for r in drop {
        out.extend_from_slice(b.get(at..r.start)?);
        at = r.end;
    }
    out.extend_from_slice(b.get(at..)?);
    Some(out)
}

/// Whether the first frame header of `b` samples its first component below the largest
/// horizontal or vertical sampling factor of the frame. zune-jpeg 0.5 sizes its upsampler's
/// scratch row from the first component and asserts on some such frames (jpeg-encoder writes
/// CMYK that way: C, M and Y at 1×1, K larger). `false` when no frame header can be read.
fn first_component_subsampled(b: &[u8]) -> bool {
    if b.get(..2) != Some(&[0xFF, 0xD8]) {
        return false;
    }
    let mut i = 2;
    while let Some(&prefix) = b.get(i) {
        // The decoder searches forward between segments. Stray bytes and
        // stuffed FF 00 pairs must not hide a frame from the abort-build guard.
        if prefix != 0xFF {
            i += 1;
            continue;
        }
        let Some(&m) = b.get(i + 1) else { return false };
        match m {
            0xFF => i += 1,
            0x00 | 0x01 | 0xD0..=0xD8 => i += 2,
            0xD9 | 0xDA => return false,
            _ => {
                let Some(len) = b.get(i + 2..i + 4).map(|l| usize::from(u16::from_be_bytes([l[0], l[1]]))) else { return false };
                if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    let Some(seg) = b.get(i + 4..i + 2 + len.max(2)) else { return false };
                    let n = usize::from(seg.get(5).copied().unwrap_or(0));
                    let factors: Vec<(u8, u8)> = (0..n).filter_map(|c| seg.get(6 + c * 3 + 1)).map(|f| (f >> 4, f & 0x0F)).collect();
                    let (Some(&(h0, v0)), Some(hmax), Some(vmax)) =
                        (factors.first(), factors.iter().map(|f| f.0).max(), factors.iter().map(|f| f.1).max())
                    else {
                        return false;
                    };
                    return h0 < hmax || v0 < vmax;
                }
                i += 2 + len;
            }
        }
    }
    false
}

/// Whether `craft-jpeg` (rather than `zune-jpeg`) decodes the file: arithmetic coding
/// (SOF9–SOF11), lossless (SOF3), or a DCT frame whose precision isn't 8 bits (#4934, #5003).
/// This includes 12-bit progressive Huffman (SOF2), which `craft-jpeg` supports.
fn uses_craft_jpeg(bytes: &[u8]) -> bool {
    match craft_jpeg::frame(bytes) {
        Some(f) if (0xC9..=0xCB).contains(&f.marker) || f.marker == 0xC3 => true,
        Some(f) if matches!(f.marker, 0xC0..=0xC2) => f.precision != 8,
        _ => false,
    }
}

fn craft_jpeg_error(e: craft_jpeg::Error) -> CodecError {
    match e {
        craft_jpeg::Error::Malformed(m) => err(m),
        craft_jpeg::Error::Unsupported(m) => CodecError::unsupported(F, m),
        craft_jpeg::Error::LimitExceeded(m) => CodecError::LimitExceeded(m),
        e => err(e),
    }
}

fn undo_craft_cmyk_inversion_u8(samples: &mut [u8], enabled: bool) {
    if enabled {
        samples.iter_mut().for_each(|v| *v = 255 - *v);
    }
}

fn undo_craft_cmyk_inversion_u16(samples: &mut [u16], enabled: bool) {
    if enabled {
        samples.iter_mut().for_each(|v| *v = 65535 - *v);
    }
}

/// Decode a file [`uses_craft_jpeg`] routes away from `zune-jpeg`; `damaged` says entropy-coded
/// data was cut short or corrupt.
fn decode_craft_jpeg(bytes: &[u8], limits: &Limits, adobe_transform: Option<u8>, adobe_marker: bool) -> Result<(Image, bool), CodecError> {
    let mut lim = craft_jpeg::Limits::default();
    lim.max_pixels = limits.max_pixels;
    lim.max_alloc = limits.max_alloc;
    let d = craft_jpeg::decode(bytes, &lim, adobe_transform).map_err(craft_jpeg_error)?;
    let layout = match d.components {
        1 => ChannelLayout::Gray,
        3 => ChannelLayout::Rgb,
        4 => ChannelLayout::Cmyk,
        n => return Err(err(format!("{n} components"))),
    };
    // craft-jpeg returns non-YCC CMYK samples with Adobe's inversion already undone,
    // regardless of whether the file actually has an Adobe marker. Unmarked four-component
    // streams are also found with direct ink samples, so restore those when using this backend.
    let restore_unmarked_cmyk = !adobe_marker && matches!(layout, ChannelLayout::Cmyk);
    let img = match d.samples {
        craft_jpeg::Samples::U8(mut px) => {
            undo_craft_cmyk_inversion_u8(&mut px, restore_unmarked_cmyk);
            Image::from_raw(d.width, d.height, layout, SampleType::U8, px)?
        }
        craft_jpeg::Samples::U16(mut px) => {
            undo_craft_cmyk_inversion_u16(&mut px, restore_unmarked_cmyk);
            Image::from_u16(d.width, d.height, layout, &px)?
        }
    };
    Ok((img, d.damaged))
}

/// Most pixels one byte of a JPEG may account for. Every 8×8 block of a frame needs at least a
/// one-bit DC code (512 pixels a byte for a flat image under ideal Huffman tables), and real files,
/// truncated or not, are far above this.
const MAX_PIXELS_PER_BYTE: u64 = 2048;

/// Decode with craft-jpeg: the files [`uses_craft_jpeg`] routes away from `zune-jpeg`, and the
/// ones `zune-jpeg` panics on.
fn decode_whole(bytes: &[u8], limits: &Limits, meta: JpegMeta, end: DataEnd, scan_bytes: u64) -> Result<Image, CodecError> {
    // craft-jpeg sizes its buffers from the frame header before it reads a scan.
    if scan_bytes == 0 {
        return Err(err("the file has no scan data"));
    }
    if let Some((w, h, _)) = meta.frame
        && end == DataEnd::Truncated
        // Only truncated scans: arithmetic coding (SOF9–SOF11) can spend well under a bit on
        // a flat block, so a complete scan may be shorter than this bound.
        && u64::from(w) * u64::from(h) / MAX_PIXELS_PER_BYTE > scan_bytes
    {
        return Err(err("the truncated scan is too short for the frame it declares"));
    }
    let (mut img, damaged) = decode_craft_jpeg(bytes, limits, meta.adobe_transform, meta.adobe_marker)?;
    img.icc = meta.icc;
    img.meta = Metadata { exif: meta.exif, xmp: meta.xmp, dpi: meta.dpi, photoshop_resources: meta.photoshop, ..Default::default() };
    if end == DataEnd::Truncated || damaged {
        img.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(img)
}

/// The stored size and EXIF block [`decode`] would find, from the marker segments before the
/// first scan: the first frame header's size, checked against `limits`, and the APP1 EXIF block.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<(u32, u32, Option<Vec<u8>>), CodecError> {
    let meta = scan_metadata(bytes);
    let (w, h, _) = meta.frame.ok_or_else(|| err("no frame header before the image data"))?;
    limits.check_size(w, h)?;
    Ok((w, h, meta.exif))
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let (end, scan_bytes) = data_end(bytes);
    if end == DataEnd::Empty {
        return Err(err("the file ends before any image data"));
    }
    let meta = scan_metadata(bytes);
    if let Some((w, h, nc)) = meta.frame {
        limits.check_bytes(w, h, u64::from(nc.max(1)))?;
    }
    // Where a panic aborts (wasm32), the frames zune-jpeg may panic on can't be caught below.
    if uses_craft_jpeg(bytes) || (cfg!(panic = "abort") && first_component_subsampled(bytes)) {
        return decode_whole(bytes, limits, meta, end, scan_bytes);
    }
    let options = DecoderOptions::default().set_strict_mode(false).set_max_width(65535).set_max_height(65535);
    let stripped = without_extended_xmp(bytes);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(stripped.as_deref().unwrap_or(bytes)), options);
    dec.decode_headers().map_err(err)?;
    let (w, h) = dec.dimensions().ok_or_else(|| err("no dimensions"))?;
    let (w, h) = (w as u32, h as u32);
    let input = dec.input_colorspace().ok_or_else(|| err("no colorspace"))?;
    let (out_cs, layout) = match input {
        ColorSpace::Luma | ColorSpace::LumaA => (ColorSpace::Luma, ChannelLayout::Gray),
        ColorSpace::CMYK => (ColorSpace::CMYK, ChannelLayout::Cmyk),
        ColorSpace::YCCK => (ColorSpace::YCCK, ChannelLayout::Cmyk),
        _ => (ColorSpace::RGB, ChannelLayout::Rgb),
    };
    limits.check(w, h, layout, SampleType::U8)?;
    // The backend fills whatever the scan no longer describes, and allocates the whole output
    // once a scan holds a byte, so a short scan under a large frame is a gigabyte of synthesized
    // output from a few kilobytes of file (#8597), truncated or not. The budget is the scan data
    // itself, not `bytes.len()`, which APP padding can inflate without describing one more pixel;
    // where the walk stops early on an odd layout, what it couldn't read counts as scan data.
    if u64::from(w) * u64::from(h) / MAX_PIXELS_PER_BYTE > scan_bytes {
        let what = if end == DataEnd::Truncated { "the truncated scan" } else { "the scan data" };
        return Err(err(format!("{what} is too short for the frame it declares")));
    }
    dec.set_options(dec.options().jpeg_set_out_colorspace(out_cs));
    // zune-jpeg 0.5 asserts on some subsampled four-component frames whose first component isn't
    // the largest (its upsampler's scratch row is sized from that component): craft-jpeg reads them.
    let Ok(decoded) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dec.decode())) else {
        return decode_whole(bytes, limits, meta, end, scan_bytes);
    };
    let mut px = decoded.map_err(err)?;
    let expected = w as usize * h as usize * layout.channels();
    if px.len() < expected {
        return Err(err("short pixel buffer"));
    }
    px.truncate(expected);
    match input {
        // Adobe-style CMYK is stored inverted (255 = no ink). Four-component JPEGs without
        // APP14 are also encountered with their CMYK samples stored as ink values directly.
        ColorSpace::CMYK if meta.adobe_marker => px.iter_mut().for_each(|v| *v = 255 - *v),
        ColorSpace::YCCK => {
            for p in px.as_chunks_mut::<4>().0 {
                let (y, cb, cr) = (p[0] as f32, p[1] as f32 - 128.0, p[2] as f32 - 128.0);
                let c = y + 1.402 * cr;
                let m = y - 0.344_136 * cb - 0.714_136 * cr;
                let yy = y + 1.772 * cb;
                p[0] = c.round().clamp(0.0, 255.0) as u8;
                p[1] = m.round().clamp(0.0, 255.0) as u8;
                p[2] = yy.round().clamp(0.0, 255.0) as u8;
                p[3] = 255 - p[3];
            }
        }
        _ => {}
    }
    let mut img = Image::from_raw(w, h, layout, SampleType::U8, px)?;
    img.icc = meta.icc;
    img.meta = Metadata { exif: meta.exif, xmp: meta.xmp, dpi: meta.dpi, photoshop_resources: meta.photoshop, ..Default::default() };
    if end == DataEnd::Truncated {
        img.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(img)
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    let (w, h) = img.dimensions();
    let (w16, h16) = match (u16::try_from(w), u16::try_from(h)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return Err(CodecError::encode(F, "JPEG dimensions are limited to 65535"));
        }
    };
    let ct = match img.layout() {
        ChannelLayout::Gray => jpeg_encoder::ColorType::Luma,
        ChannelLayout::Rgb => jpeg_encoder::ColorType::Rgb,
        ChannelLayout::Cmyk => jpeg_encoder::ColorType::Cmyk,
        l => return Err(CodecError::encode(F, format!("unsupported layout {l:?}"))),
    };
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, opts.jpeg_quality.clamp(1, 100));
    // CMYK is always written 4:4:4: subsampled 4-component JPEGs are not
    // decoded consistently across implementations.
    let subsampled = opts.jpeg_chroma_subsampling && img.layout() == ChannelLayout::Rgb;
    let sampling = if subsampled { jpeg_encoder::SamplingFactor::R_4_2_0 } else { jpeg_encoder::SamplingFactor::R_4_4_4 };
    enc.set_sampling_factor(sampling);
    // JPEG Options › Format Options. Fitted Huffman tables make a baseline scan non-interleaved
    // (one scan per component), which zune-jpeg 0.5.15 decodes wrongly with 4:2:0 chroma
    // (etemesi254/zune-image#419; see `web::encode_jpeg_rgb8`), so a subsampled Baseline
    // Optimized keeps the standard tables until that decoder is updated. Progressive is fine.
    let progressive = opts.jpeg_format == JpegFormat::Progressive;
    enc.set_progressive(progressive);
    if progressive && let Some(scans) = opts.jpeg_progressive_scans {
        enc.set_progressive_scans(scans);
    }
    let optimized = opts.jpeg_format == JpegFormat::Optimized && !subsampled;
    enc.set_optimized_huffman_tables(progressive || optimized);
    let e = |e: jpeg_encoder::EncodingError| CodecError::encode(F, e);
    if opts.embed_metadata {
        if img.layout() != ChannelLayout::Cmyk
            && let Some((x, y)) = img.meta.dpi
            && x >= 1.0
            && y >= 1.0
        {
            let dpi_x = x.round().min(65535.0) as u16;
            let dpi_y = y.round().min(65535.0) as u16;
            enc.set_density(jpeg_encoder::PixelDensity { density: (dpi_x, dpi_y), unit: jpeg_encoder::PixelDensityUnit::Inches });
        }
        // Metadata too large for its one segment is left out (and reported by the fidelity
        // warnings) rather than failing the whole export.
        let exif = if img.layout() == ChannelLayout::Cmyk {
            match img.meta.exif.as_deref() {
                Some(exif) => exif_segment_with_resolution(exif, img.meta.dpi),
                None if usable_dpi(img.meta.dpi) => exif_segment_with_resolution(&[], img.meta.dpi),
                None => None,
            }
        } else {
            img.meta.exif.as_deref().and_then(|x| exif_segment(x, img.meta.dpi))
        };
        if let Some(seg) = exif {
            // Complete APP1 payload includes Exif\0\0; add_exif_metadata would duplicate it.
            enc.add_app_segment(1, seg).map_err(e)?;
        }
        if let Some(seg) = img.meta.xmp.as_deref().and_then(|x| xmp_segment(x, img.meta.dpi)) {
            enc.add_app_segment(1, seg).map_err(e)?;
        }
    }
    if opts.embed_icc
        && let Some(icc) = &img.icc
    {
        enc.add_icc_profile(icc).map_err(e)?;
    }
    enc.encode(img.data(), w16, h16, ct).map_err(e)?;
    if img.layout() == ChannelLayout::Cmyk {
        strip_jfif_app0(&mut out);
    }
    Ok(out)
}

/// JFIF APP0 requires one or three image components. A four-component CMYK JPEG must not carry
/// that marker; keep Adobe APP14, which identifies the CMYK encoding instead.
fn strip_jfif_app0(bytes: &mut Vec<u8>) {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return;
    }
    let mut i = 2usize;
    loop {
        if bytes.get(i) != Some(&0xff) {
            return;
        }
        let Some(&marker) = bytes.get(i.saturating_add(1)) else { return };
        if matches!(marker, 0xda | 0xd9) {
            return;
        }
        if marker == 0xff {
            i = i.saturating_add(1);
            continue;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            i = i.saturating_add(2);
            continue;
        }
        let Some(length_bytes) = i.checked_add(2).and_then(|at| bytes.get(at..at.checked_add(2)?)) else { return };
        let Ok(length_bytes) = <[u8; 2]>::try_from(length_bytes) else { return };
        let length = usize::from(u16::from_be_bytes(length_bytes));
        if length < 2 {
            return;
        }
        let Some(end) = i.checked_add(2).and_then(|at| at.checked_add(length)) else { return };
        if end > bytes.len() {
            return;
        }
        if marker == 0xe0 && bytes.get(i.saturating_add(4)..i.saturating_add(9)) == Some(b"JFIF\0") {
            bytes.drain(i..end);
        } else {
            i = end;
        }
    }
}

/// Return whether both resolution values are finite and within the supported metadata range.
fn usable_dpi(dpi: Option<(f32, f32)>) -> bool {
    dpi.is_some_and(|(x, y)| x.is_finite() && y.is_finite() && (1.0..=1_000_000.0).contains(&x) && (1.0..=1_000_000.0).contains(&y))
}

/// The most data one APP segment holds (its 16-bit length counts its own two bytes).
const MAX_SEGMENT: usize = 65533;

/// XMP properties (local names, any namespace prefix) that only describe the layered document:
/// every document ever placed in it, the text of every type layer. A flat image doesn't need them,
/// and they alone can outgrow a segment.
const LAYERED_ONLY_XMP: [&str; 2] = ["DocumentAncestors", "TextLayers"];

/// The EXIF APP1 segment, or `None` when it doesn't fit in one segment.
pub(crate) fn exif_segment(exif: &[u8], dpi: Option<(f32, f32)>) -> Option<Vec<u8>> {
    // The pixels are written as they are shown: never let a viewer rotate them again; and store
    // the output resolution, not the source's (#1691).
    let seg = [EXIF_HEADER, export_exif(exif, dpi).as_ref()].concat();
    (seg.len() <= MAX_SEGMENT).then_some(seg)
}

/// Build an EXIF APP1 segment, adding missing resolution tags when the source metadata is valid.
fn exif_segment_with_resolution(exif: &[u8], dpi: Option<(f32, f32)>) -> Option<Vec<u8>> {
    let seg = [EXIF_HEADER, crate::resolution::export_exif_adding_missing_resolution(exif, dpi).as_ref()].concat();
    (seg.len() <= MAX_SEGMENT).then_some(seg)
}

/// The XMP APP1 segment. When the packet doesn't fit in one segment, the layered-document
/// properties are left out; `None` when it still doesn't fit.
pub(crate) fn xmp_segment(xmp: &str, dpi: Option<(f32, f32)>) -> Option<Vec<u8>> {
    let xmp = export_xmp(xmp, dpi);
    let seg = |x: &str| [XMP_HEADER, x.as_bytes()].concat();
    if XMP_HEADER.len() + xmp.len() <= MAX_SEGMENT {
        return Some(seg(&xmp));
    }
    let trimmed = LAYERED_ONLY_XMP.iter().fold(xmp.into_owned(), |x, name| remove_element(&x, name));
    (XMP_HEADER.len() + trimmed.len() <= MAX_SEGMENT).then(|| seg(&trimmed))
}

/// `xmp` without its `<prefix:name …>…</prefix:name>` and `<prefix:name …/>` elements. Unbalanced
/// markup is kept as is.
fn remove_element(xmp: &str, name: &str) -> String {
    let local = format!(":{name}");
    let mut out = String::with_capacity(xmp.len());
    let mut rest = xmp;
    while let Some(j) = rest.find(&local) {
        let head = rest.get(..j).unwrap_or_default();
        let prefix_len = head.bytes().rev().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')).count();
        let lt = j - prefix_len;
        let after = rest.get(j + local.len()..).unwrap_or_default();
        // An opening tag of exactly this name (not a closing tag, an attribute or a longer name).
        let opening = prefix_len > 0 && lt > 0 && rest.get(lt - 1..lt) == Some("<");
        if !opening || !after.starts_with(['>', '/', ' ', '\t', '\r', '\n']) {
            out.push_str(rest.get(..j + local.len()).unwrap_or_default());
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let end = if after.get(..gt).is_some_and(|s| s.ends_with('/')) {
            gt + 1
        } else {
            let close = format!("</{}{local}>", rest.get(lt..j).unwrap_or_default());
            let Some(c) = after.find(&close) else { break };
            c + close.len()
        };
        out.push_str(rest.get(..lt - 1).unwrap_or_default());
        rest = after.get(end..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, Metadata};

    /// Build a little-endian TIFF with an empty IFD0 for metadata tests.
    fn empty_tiff() -> Vec<u8> {
        // Little-endian TIFF with an empty IFD0 and no next IFD.
        b"II*\0\x08\0\0\0\0\0\0\0\0\0\0\0".to_vec()
    }

    /// Return whether the JPEG begins with a JFIF APP0 segment.
    fn has_jfif_app0(bytes: &[u8]) -> bool {
        bytes.get(2..11) == Some(b"\xff\xe0\0\x10JFIF\0")
    }

    /// `info` reads the frame from the markers, and which decoder the file goes to.
    #[test]
    fn ff_search_matches_plain_search() {
        let mut b: Vec<u8> = (0..300u32).map(|i| (i * 7 % 255) as u8).collect();
        for at in [None, Some(0), Some(7), Some(8), Some(15), Some(203), Some(296), Some(299)] {
            if let Some(i) = at {
                b[i] = 0xff;
            }
            for start in [0, 1, 5, 9, 290, 299, 300] {
                let s = &b[start..];
                assert_eq!(find_ff(s), s.iter().position(|&v| v == 0xff));
            }
        }
    }

    #[test]
    fn info_reads_the_frame_and_the_decoder() {
        let img = Image::from_raw(37, 21, ChannelLayout::Rgb, SampleType::U8, vec![128; 37 * 21 * 3]).unwrap();
        let rgb = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        let rgb_info = crate::JpegInfo { width: 37, height: 21, layout: ChannelLayout::Rgb, whole_coefficients: false, held_coefficients: 0 };
        assert_eq!(info(&rgb), Some(rgb_info));
        // A 12-bit extended frame (SOF1) declaring 40,000 × 30,000 gray, markers only.
        let mut twelve = vec![0xFF, 0xD8, 0xFF, 0xC1, 0, 11, 12];
        twelve.extend_from_slice(&30_000u16.to_be_bytes());
        twelve.extend_from_slice(&40_000u16.to_be_bytes());
        twelve.extend_from_slice(&[1, 1, 0x11, 0, 0xFF, 0xD9]);
        let twelve_info =
            crate::JpegInfo { width: 40_000, height: 30_000, layout: ChannelLayout::Gray, whole_coefficients: true, held_coefficients: 0 };
        assert_eq!(info(&twelve), Some(twelve_info));
        assert_eq!(info(b"not a jpeg"), None);
        // Markers only, 200 × 100 with `n` components: four are CMYK, two no layout `decode` returns.
        let sof0 = |n: u8| {
            let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 8 + 3 * n, 8, 0, 100, 0, 200, n];
            for i in 1..=n {
                b.extend_from_slice(&[i, 0x11, 0]);
            }
            b.extend_from_slice(&[0xFF, 0xD9]);
            b
        };
        assert_eq!(info(&sof0(4)).map(|i| (i.width, i.height, i.layout)), Some((200, 100, ChannelLayout::Cmyk)));
        assert_eq!(info(&sof0(2)), None);
    }

    /// A frame the decoder can't turn into rows as it reads reports every coefficient it holds:
    /// a block of 64 `i16`s per component block of every MCU. That is an 8-bit progressive frame
    /// (SOF2), or an 8-bit sequential one whose first scan lacks some of its components.
    #[test]
    fn info_counts_the_coefficients_a_decode_holds() {
        // Markers only: SOI, a frame of `marker` declaring 48,000 × 34,200 with `factors`, a first
        // scan of the first `scan` components when given, EOI.
        let frame = |marker: u8, precision: u8, factors: &[u8], scan: Option<u8>| {
            let mut b = vec![0xFF, 0xD8, 0xFF, marker, 0, 8 + 3 * factors.len() as u8, precision];
            b.extend_from_slice(&34_200u16.to_be_bytes());
            b.extend_from_slice(&48_000u16.to_be_bytes());
            b.push(factors.len() as u8);
            for (i, &f) in factors.iter().enumerate() {
                b.extend_from_slice(&[i as u8 + 1, f, 0]);
            }
            if let Some(n) = scan {
                b.extend_from_slice(&[0xFF, 0xDA, 0, 6 + 2 * n, n]);
                for c in 1..=n {
                    b.extend_from_slice(&[c, 0]);
                }
                b.extend_from_slice(&[0, 63, 0]);
            }
            b.extend_from_slice(&[0xFF, 0xD9]);
            b
        };
        let held = |b: &[u8]| info(b).map(|i| i.held_coefficients);
        let all_444 = 6_000 * 4_275 * 3 * 128;
        // 4:4:4: 6,000 × 4,275 MCUs of one block per component.
        assert_eq!(held(&frame(0xC2, 8, &[0x11, 0x11, 0x11], Some(1))), Some(all_444));
        // 4:2:0: 3,000 × 2,138 MCUs of 4 + 1 + 1 blocks.
        assert_eq!(held(&frame(0xC2, 8, &[0x22, 0x11, 0x11], None)), Some(3_000 * 2_138 * 6 * 128));
        // Sequential frames whose first scan lacks components hold them all too.
        assert_eq!(held(&frame(0xC0, 8, &[0x11, 0x11, 0x11], Some(1))), Some(all_444));
        assert_eq!(held(&frame(0xC1, 8, &[0x11, 0x11, 0x11], Some(2))), Some(all_444));
        // A sequential first scan with every component is read a line of MCUs at a time, and a
        // 12-bit progressive frame goes to craft-jpeg: they hold none.
        assert_eq!(held(&frame(0xC0, 8, &[0x11, 0x11, 0x11], Some(3))), Some(0));
        assert_eq!(held(&frame(0xC0, 8, &[0x11, 0x11, 0x11], None)), Some(0));
        assert_eq!(held(&frame(0xC2, 12, &[0x11, 0x11, 0x11], None)), Some(0));
    }

    /// The consumer's own estimate (WordCraft's `jpeg_coefficients`), ported as the oracle that
    /// `worst_case_coefficients` must never undercut.
    fn oracle(bytes: &[u8]) -> u64 {
        fn worst(bytes: &[u8]) -> u64 {
            let Some((w, h)) = info(bytes).map(|i| (i.width, i.height)) else { return 0 };
            let mcus = u64::from(w).div_ceil(32) * u64::from(h).div_ceil(32);
            mcus.saturating_mul(2 * 64 * 16 * 4)
        }
        fn frame(sof: &[u8]) -> Option<u64> {
            let height = u64::from(u16::from_be_bytes([*sof.get(1)?, *sof.get(2)?]));
            let width = u64::from(u16::from_be_bytes([*sof.get(3)?, *sof.get(4)?]));
            let n = usize::from(*sof.get(5)?);
            let s: Vec<(u64, u64)> = sof
                .get(6..6 + 3 * n)?
                .as_chunks::<3>()
                .0
                .iter()
                .map(|&[_, s, _]| (u64::from(s >> 4).clamp(1, 4), u64::from(s & 15).clamp(1, 4)))
                .collect();
            let (hm, vm) = s.iter().fold((1, 1), |(h, v), &(a, b)| (h.max(a), v.max(b)));
            let mcus = width.div_ceil(8 * hm).checked_mul(height.div_ceil(8 * vm))?;
            s.iter().try_fold(0u64, |sum, &(h, v)| sum.checked_add(mcus.checked_mul(2 * 64 * h * v)?))
        }
        let mut at = 2;
        let mut most = 0u64;
        loop {
            if bytes.get(at) != Some(&0xFF) {
                return worst(bytes).max(most);
            }
            while bytes.get(at) == Some(&0xFF) {
                at += 1;
            }
            let Some(&code) = bytes.get(at) else { return worst(bytes).max(most) };
            at += 1;
            match code {
                0xD8 | 0x01 | 0xD0..=0xD7 => continue,
                0xD9 | 0xDA => return most,
                _ => {}
            }
            let len = match (bytes.get(at), bytes.get(at + 1)) {
                (Some(&hi), Some(&lo)) => usize::from(u16::from_be_bytes([hi, lo])),
                _ => 0,
            };
            let Some(seg) = bytes.get(at + 2..at + len).filter(|_| len >= 2) else { return worst(bytes).max(most) };
            if matches!(code, 0xC2 | 0xC6 | 0xCA | 0xCE) {
                let Some(h) = frame(seg) else { return worst(bytes).max(most) };
                most = most.max(h);
            }
            at += len;
        }
    }

    /// A JPEG header: each frame is (marker, precision, width, height, sampling bytes), then an
    /// optional scan of `scan` components, then EOI.
    fn header(frames: &[(u8, u8, u16, u16, &[u8])], scan: Option<u8>) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        for &(marker, precision, w, h, f) in frames {
            b.extend_from_slice(&[0xFF, marker]);
            b.extend_from_slice(&(8 + 3 * f.len() as u16).to_be_bytes());
            b.push(precision);
            b.extend_from_slice(&h.to_be_bytes());
            b.extend_from_slice(&w.to_be_bytes());
            b.push(f.len() as u8);
            for (i, &s) in f.iter().enumerate() {
                b.extend_from_slice(&[i as u8 + 1, s, 0]);
            }
        }
        if let Some(n) = scan {
            b.extend_from_slice(&[0xFF, 0xDA, 0, 6 + 2 * n, n]);
            (1..=n).for_each(|c| b.extend_from_slice(&[c, 0]));
            b.extend_from_slice(&[0, 63, 0]);
        }
        b.extend_from_slice(&[0xFF, 0xD9]);
        b
    }

    /// Never less than the oracle or `held_coefficients`; equal to the larger on valid headers.
    #[test]
    fn worst_case_coefficients_covers_both_rule_sets() {
        const C3: &[u8] = &[0x22, 0x11, 0x11];
        const G: &[u8] = &[0x11];
        let cases: Vec<Vec<u8>> = vec![
            header(&[(0xC0, 8, 640, 480, C3)], Some(3)),
            header(&[(0xC0, 8, 640, 480, G)], Some(1)),
            header(&[(0xC0, 8, 640, 480, C3)], Some(1)),
            header(&[(0xC1, 8, 640, 480, C3)], None),
            header(&[(0xC2, 8, 640, 480, C3)], Some(1)),
            header(&[(0xC2, 12, 640, 480, C3)], Some(1)),
            header(&[(0xC6, 8, 640, 480, C3)], None),
            header(&[(0xCA, 8, 640, 480, C3)], None),
            header(&[(0xCE, 8, 640, 480, C3)], None),
            header(&[(0xC0, 8, 64, 64, G), (0xC2, 8, 6000, 4000, C3), (0xC2, 8, 100, 100, C3)], Some(1)),
            header(&[(0xC2, 8, 640, 480, &[0x00, 0x00, 0x00])], None),
            header(&[(0xC2, 8, 640, 480, &[0xFF, 0xFF, 0xFF, 0xFF])], None),
            header(&[(0xC2, 8, 65535, 65535, &[0x44; 4])], None),
            header(&[(0xC0, 8, 65535, 65535, &[0xFF; 4])], Some(1)),
            header(&[], None),
            vec![0xFF, 0xD8],
            vec![],
            b"not a jpeg at all".to_vec(),
        ];
        for b in &cases {
            let held = scan_metadata(b).held_coefficients;
            let got = worst_case_coefficients(b);
            assert!(got >= oracle(b) && got >= held, "{b:02x?}");
            assert!(got >= info(b).map_or(0, |i| i.held_coefficients));
        }
        let at = |i: usize| worst_case_coefficients(&cases[i]);
        // Equal to the oracle where the shared rule charges nothing extra.
        for i in [0, 1, 6, 7, 8, 9, 10, 12, 14] {
            assert_eq!(at(i), oracle(&cases[i]), "case {i}");
        }
        // Where it does, the shared figure wins (the oracle misses a short first scan).
        assert_eq!(oracle(&cases[2]), 0);
        assert_eq!(at(2), scan_metadata(&cases[2]).held_coefficients);
        // Unclamped factors (15 × 15) charge the shared rule more than the clamped oracle.
        assert_eq!(at(11), scan_metadata(&cases[11]).held_coefficients);
        assert!(at(11) > oracle(&cases[11]));
        assert!(at(2) > 0 && at(3) == 0 && at(0) == 0);
        // 12-bit and arithmetic progressive frames are charged though craft-jpeg limits itself.
        assert_eq!(at(5), at(4));
        // Sampling factors clamp: 0 acts as 1, 15 as 4.
        assert_eq!(at(10), 80 * 60 * 3 * 128);
        assert_eq!(at(12), 2048 * 2048 * 4 * 2 * 64 * 16);
        // The largest of several frames wins.
        assert_eq!(at(9), oracle(&header(&[(0xC2, 8, 6000, 4000, C3)], None)));
    }

    /// A cut header charges the costliest frame at its size; every prefix and random damage stays
    /// bounded, never panics and never undercuts the oracle.
    #[test]
    fn worst_case_coefficients_hostile_input() {
        let full = header(&[(0xC2, 8, 6000, 4000, &[0x22, 0x11, 0x11])], Some(1));
        let cut = &full[..full.len() - 8];
        // Cut inside the component list: a frame the scan can't read, at its own size.
        let sof_cut = &full[..14];
        assert_eq!(worst_case_coefficients(sof_cut), oracle(sof_cut));
        assert_eq!(worst_case_coefficients(cut), oracle(cut));
        let mut bases = vec![full.clone(), header(&[(0xC0, 8, 6000, 4000, &[0x11; 3])], Some(1))];
        bases.push(header(&[(0xC2, 8, 65535, 65535, &[0xFF; 4])], None));
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        for base in &bases {
            for n in 0..=base.len() {
                let b = &base[..n];
                let got = worst_case_coefficients(b);
                assert!(got >= oracle(b) && got < 1 << 50);
            }
            for _ in 0..2000 {
                let mut b = base.clone();
                for _ in 0..=next() % 4 {
                    let i = next() % b.len();
                    b[i] = next() as u8;
                }
                b.truncate(next() % (b.len() + 1));
                let got = worst_case_coefficients(&b);
                assert!(got >= oracle(&b) && got >= scan_metadata(&b).held_coefficients && got < 1 << 50);
            }
        }
    }

    /// An APP1 Extended XMP fragment (GUID, total size, offset, data).
    fn extended_xmp(offset: u32) -> Vec<u8> {
        let mut seg = EXTENDED_XMP_HEADER.to_vec();
        seg.extend_from_slice(&[b'A'; 32]);
        seg.extend_from_slice(&1_000_000u32.to_be_bytes());
        seg.extend_from_slice(&offset.to_be_bytes());
        seg.push(b'x');
        let mut out = vec![0xFF, 0xE1];
        out.extend_from_slice(&u16::try_from(seg.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(&seg);
        out
    }

    #[test]
    fn extended_xmp_fragments_are_left_out_of_the_backend_parse() {
        let img = Image::from_raw(16, 16, ChannelLayout::Rgb, SampleType::U8, vec![120; 16 * 16 * 3]).unwrap();
        let plain = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        assert!(without_extended_xmp(&plain).is_none());
        // 20000 consistent but incomplete fragments right after SOI: zune-jpeg would re-sort and
        // re-check the growing pending list after each one.
        let mut hostile = plain[..2].to_vec();
        for k in 0..20_000u32 {
            hostile.extend_from_slice(&extended_xmp(k * 2));
        }
        hostile.extend_from_slice(&plain[2..]);
        assert_eq!(without_extended_xmp(&hostile).as_deref(), Some(&plain[..]));
        let back = decode(&hostile, &Limits::default()).unwrap();
        assert_eq!(back.dimensions(), (16, 16));
    }

    /// A tiny truncated scan under a frame patched to 20000×20000 is refused before the backend
    /// fills 400 million pixels from it; the same file with its real size still recovers.
    #[test]
    fn a_short_truncated_scan_cannot_declare_a_huge_frame() {
        let img = Image::from_raw(16, 16, ChannelLayout::Rgb, SampleType::U8, vec![128; 16 * 16 * 3]).unwrap();
        let full = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        let cut = &full[..full.len() - 4];
        assert_eq!(data_end(cut).0, DataEnd::Truncated);
        assert!(crate::decode_as(Format::Jpeg, cut).is_ok());
        let sof = cut.windows(2).position(|w| w == [0xFF, 0xC0] || w == [0xFF, 0xC2]).unwrap();
        let mut huge = cut.to_vec();
        huge[sof + 5..sof + 9].copy_from_slice(&[0x4E, 0x20, 0x4E, 0x20]);
        let e = crate::decode_as(Format::Jpeg, &huge).unwrap_err();
        assert!(e.to_string().contains("too short"), "{e}");
    }

    /// The truncated-scan budget counts scan data, not the file length: APP padding before the
    /// window cannot admit an oversized output. The same padded file with its real size still
    /// decodes (with the truncation warning).
    #[test]
    fn app_padding_cannot_admit_an_oversized_truncated_frame() {
        let img = Image::from_raw(16, 16, ChannelLayout::Rgb, SampleType::U8, vec![128; 16 * 16 * 3]).unwrap();
        let full = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        let cut = &full[..full.len() - 4];
        // Four full-length COMMENT segments (~256 KiB) stand where APP padding would; they are
        // complete, so the file otherwise parses normally.
        let fill = [b'p'; 65533];
        let mut padded = cut[..2].to_vec();
        for _ in 0..4 {
            padded.extend_from_slice(&[0xFF, 0xFE]);
            padded.extend_from_slice(&0xFFFFu16.to_be_bytes());
            padded.extend_from_slice(&fill);
        }
        padded.extend_from_slice(&cut[2..]);
        assert_eq!(data_end(&padded).0, DataEnd::Truncated);
        // Well above the frame's pixels / budget, yet far above the scan data: the old
        // file-length bound admitted this frame.
        assert!(padded.len() > (u64::from(20_000u32) * u64::from(20_000u32) / MAX_PIXELS_PER_BYTE) as usize, "padding too small");
        assert!(crate::decode_as(Format::Jpeg, &padded).is_ok());
        let sof = padded.windows(2).position(|w| w == [0xFF, 0xC0] || w == [0xFF, 0xC2]).unwrap();
        let mut huge = padded.clone();
        huge[sof + 5..sof + 9].copy_from_slice(&[0x4E, 0x20, 0x4E, 0x20]);
        let e = crate::decode_as(Format::Jpeg, &huge).unwrap_err();
        assert!(e.to_string().contains("too short"), "{e}");
    }

    /// A complete file is measured on its scan data too: padding can't admit an oversized frame.
    /// The same padded file at its real size decodes, and so does an odd layout whose unread tail
    /// counts as scan data.
    #[test]
    fn app_padding_cannot_admit_an_oversized_complete_frame() {
        let img = Image::from_raw(16, 16, ChannelLayout::Rgb, SampleType::U8, vec![128; 16 * 16 * 3]).unwrap();
        let full = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        let fill = [b'p'; 65533];
        let mut padded = full[..2].to_vec();
        for _ in 0..4 {
            padded.extend_from_slice(&[0xFF, 0xFE]);
            padded.extend_from_slice(&0xFFFFu16.to_be_bytes());
            padded.extend_from_slice(&fill);
        }
        padded.extend_from_slice(&full[2..]);
        let (end, scanned) = data_end(&padded);
        assert_eq!(end, DataEnd::Complete);
        assert!(scanned < 1024, "the padding isn't scan data: {scanned}");
        assert!(padded.len() as u64 > u64::from(20_000u32) * 20_000 / MAX_PIXELS_PER_BYTE, "padding too small");
        assert!(crate::decode_as(Format::Jpeg, &padded).is_ok());
        let sof = padded.windows(2).position(|w| w == [0xFF, 0xC0] || w == [0xFF, 0xC2]).unwrap();
        let mut huge = padded.clone();
        huge[sof + 5..sof + 9].copy_from_slice(&[0x4E, 0x20, 0x4E, 0x20]);
        let e = crate::decode_as(Format::Jpeg, &huge).unwrap_err();
        assert!(e.to_string().contains("too short"), "{e}");
        // A stray byte where a marker should be, before the padding: the walk skips it and still
        // recognises the padding, so an oversized frame is refused there too.
        let mut odd = huge.clone();
        odd.insert(2, 0x00);
        let (end, scanned) = data_end(&odd);
        assert_eq!(end, DataEnd::Complete);
        assert!(scanned < 1024, "the padding after a stray byte isn't scan data: {scanned}");
        let e = crate::decode_as(Format::Jpeg, &odd).unwrap_err();
        assert!(e.to_string().contains("too short"), "{e}");
        // Stray bytes never count: 256 KiB of junk where a marker should be (the decoder skips
        // it), then the oversized frame's segments, is refused. (Bytes after a scan starts are
        // entropy-coded data, any value, and count as such.)
        let mut junk = huge[..2].to_vec();
        junk.extend(std::iter::repeat_n(0x00u8, 4 * 65536));
        junk.extend_from_slice(&huge[2..]);
        let (end, scanned) = data_end(&junk);
        assert_eq!(end, DataEnd::Complete);
        assert!(scanned < 1024, "junk isn't scan data: {scanned}");
        let e = crate::decode_as(Format::Jpeg, &junk).unwrap_err();
        assert!(e.to_string().contains("too short"), "{e}");
        // Junk with no marker after it: no image data at all.
        let mut tail = vec![0xFF, 0xD8];
        tail.extend(std::iter::repeat_n(0x00u8, 4096));
        assert_eq!(data_end(&tail), (DataEnd::Empty, 0));
    }

    /// CMYK JPEG output must omit JFIF while retaining Adobe's CMYK marker.
    #[test]
    fn cmyk_jpeg_omits_jfif_but_keeps_adobe_cmyk_marker() {
        let img = Image::from_raw(2, 1, ChannelLayout::Cmyk, SampleType::U8, vec![0, 32, 64, 96, 255, 224, 192, 160]).unwrap();
        let bytes = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();

        let meta = scan_metadata(&bytes);
        assert!(!has_jfif_app0(&bytes));
        assert_eq!(meta.frame, Some((2, 1, 4)));
        assert_eq!(meta.adobe_transform, Some(0));
        assert_eq!(crate::decode_as(Format::Jpeg, &bytes).unwrap().layout(), ChannelLayout::Cmyk);
    }

    /// CMYK JPEG output stores image resolution in EXIF when the source has no EXIF.
    #[test]
    fn cmyk_jpeg_preserves_dpi_without_source_exif() {
        let mut img = Image::from_raw(1, 1, ChannelLayout::Cmyk, SampleType::U8, vec![0, 0, 0, 0]).unwrap();
        img.meta.dpi = Some((300.0, 150.0));
        let bytes = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();

        assert!(!has_jfif_app0(&bytes));
        assert_eq!(crate::decode_as(Format::Jpeg, &bytes).unwrap().meta.dpi, Some((300.0, 150.0)));
    }

    /// CMYK JPEG output adds missing EXIF resolution tags to existing metadata.
    #[test]
    fn cmyk_jpeg_preserves_dpi_with_source_exif_without_resolution_tags() {
        let mut img = Image::from_raw(1, 1, ChannelLayout::Cmyk, SampleType::U8, vec![0, 0, 0, 0]).unwrap();
        img.meta = Metadata { exif: Some(empty_tiff()), dpi: Some((240.0, 120.0)), ..Default::default() };
        let bytes = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();

        assert_eq!(crate::decode_as(Format::Jpeg, &bytes).unwrap().meta.dpi, Some((240.0, 120.0)));
    }

    /// Adding missing resolution tags makes them readable from the rewritten IFD0.
    #[test]
    fn exif_with_resolution_adds_missing_ifd0_entries() {
        let exif = empty_tiff();
        let rewritten = crate::resolution::export_exif_adding_missing_resolution(&exif, Some((240.0, 120.0)));

        assert_eq!(crate::resolution::exif_resolution(&rewritten), Some((240.0, 120.0)));
    }

    /// RGB JPEG output continues to store image resolution in JFIF density fields.
    #[test]
    fn rgb_jpeg_keeps_jfif_density() {
        let mut img = Image::from_raw(1, 1, ChannelLayout::Rgb, SampleType::U8, vec![12, 34, 56]).unwrap();
        img.meta.dpi = Some((96.0, 96.0));
        let bytes = crate::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();

        assert!(has_jfif_app0(&bytes));
        assert_eq!(scan_metadata(&bytes).dpi, Some((96.0, 96.0)));
    }

    // --- Arithmetic-coded and 12-bit files, decoded by craft-jpeg ---

    fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
        out.extend_from_slice(&[0xFF, marker]);
        out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(body);
    }

    /// One 8×8 gray block, quantizer 1, with entropy-coded scan data `data`.
    fn gray_block(sof: u8, precision: u8, dht: Option<&[u8]>, data: &[u8]) -> Vec<u8> {
        let mut s = vec![0xFF, 0xD8];
        let mut dqt = vec![0u8];
        dqt.extend([1u8; 64]);
        segment(&mut s, 0xDB, &dqt);
        segment(&mut s, sof, &[precision, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        if let Some(dht) = dht {
            segment(&mut s, 0xC4, dht);
        }
        segment(&mut s, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
        s.extend_from_slice(data);
        s.extend([0xFF, 0xD9]);
        s
    }

    /// SOF9 with only a DC coefficient (from a T.81 arithmetic encoder): 8-bit DC 200 → sample
    /// 200 / 8 + 128 = 153; 12-bit DC 1000 → 1000 / 8 + 2048 = 2173.
    fn arithmetic_block(precision: u8) -> Vec<u8> {
        let data: &[u8] = if precision == 8 { &[210, 217, 68, 0] } else { &[210, 234, 134, 0] };
        gray_block(0xC9, precision, None, data)
    }

    /// A 12-bit SOF1 stream: one DC coefficient. The DC table has one code (`0`, category 13),
    /// the AC table one (`0`, end of block). DC = 7616 → sample 7616 / 8 + 2048 = 3000.
    fn twelve_bit_huffman_block() -> Vec<u8> {
        let mut dht = vec![0x00, 1];
        dht.extend([0u8; 15]);
        dht.push(13);
        dht.extend([0x10, 1]);
        dht.extend([0u8; 15]);
        dht.push(0x00);
        // Bits: `0`, 7616 in 13 bits, `0`, one pad `1`.
        let word: u16 = (7616 << 2) | 1;
        gray_block(0xC1, 12, Some(&dht), &word.to_be_bytes())
    }

    /// A valid 12-bit progressive DC-first scan with a one-bit zero-DC Huffman code.
    fn twelve_bit_progressive_dc_block() -> Vec<u8> {
        let mut s = vec![0xFF, 0xD8];
        let mut dqt = vec![0u8];
        dqt.extend([1u8; 64]);
        segment(&mut s, 0xDB, &dqt);
        segment(&mut s, 0xC2, &[12, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        let mut dht = vec![0x00, 1];
        dht.extend([0u8; 15]);
        dht.push(0);
        segment(&mut s, 0xC4, &dht);
        // One component, DC table 0, DC coefficients only, first approximation.
        segment(&mut s, 0xDA, &[1, 1, 0, 0, 0, 0]);
        s.push(0x7F); // DC category 0 followed by one padding bits.
        s.extend([0xFF, 0xD9]);
        s
    }

    fn twelve_bit(v: f32) -> u16 {
        (v * 65535.0 / 4095.0).round() as u16
    }

    fn progressive_frame_header(comps: &[(u8, u8)]) -> Vec<u8> {
        let mut s = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC2, 0, (8 + 3 * comps.len()) as u8, 8, 0, 8, 0, 8, comps.len() as u8];
        for (i, &(h, v)) in comps.iter().enumerate() {
            s.extend([i as u8 + 1, (h << 4) | v, 0]);
        }
        s
    }

    #[test]
    fn first_component_subsampling() {
        assert!(first_component_subsampled(&progressive_frame_header(&[(1, 1), (1, 1), (1, 1), (2, 2)])));
        assert!(first_component_subsampled(&progressive_frame_header(&[(1, 2), (1, 2), (2, 1)])));
        assert!(!first_component_subsampled(&progressive_frame_header(&[(2, 2), (1, 1), (1, 1)])));
        assert!(!first_component_subsampled(&progressive_frame_header(&[(1, 1); 4])));
        assert!(!first_component_subsampled(&progressive_frame_header(&[])));
        assert!(!first_component_subsampled(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]));
        assert!(!first_component_subsampled(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 20, 8]));
        assert!(!first_component_subsampled(b"not a jpeg"));
    }

    #[test]
    fn renderer_validation_refuses_unsafe_sampling_under_abort_policy() {
        // The frame header alone exercises refusal before any pixel decode.
        let header = progressive_frame_header(&[(1, 1), (1, 1), (1, 1), (2, 2)]);
        let refuses_sampling = |result| {
            matches!(
                result,
                Err(CodecError::Malformed { format: Format::Jpeg, message })
                if message == "JPEG first component is subsampled below the frame maximum"
            )
        };
        assert!(refuses_sampling(validate_jpeg_rgba8_with_panic_policy(&header, 64, true)));
        assert!(!refuses_sampling(validate_jpeg_rgba8_with_panic_policy(&header, 64, false)));

        let mut ordinary = Vec::new();
        jpeg_encoder::Encoder::new(&mut ordinary, 90).encode(&[128], 1, 1, jpeg_encoder::ColorType::Luma).unwrap();
        validate_jpeg_rgba8_with_panic_policy(&ordinary, 1, true).unwrap();
    }

    #[test]
    fn jpeg_sampling_guard_finds_frame_after_marker_padding() {
        let header = progressive_frame_header(&[(1, 1), (1, 1), (1, 1), (2, 2)]);
        // Exercise the shared header guard, without a pixel payload.
        for split in [2, 8] {
            let (before, after) = header.split_at(split);
            for padding in [&[0][..], &[0x7F, 0][..], &[0xFF, 0][..], &[0xFF][..]] {
                let padded = [before, padding, after].concat();
                assert!(first_component_subsampled(&padded), "offset {split}, padding {padding:?}");
                let got = validate_jpeg_rgba8_with_panic_policy(&padded, 64, true);
                assert!(matches!(got, Err(CodecError::Malformed { message, .. }) if message.contains("frame maximum")));
            }
        }
        assert!(!first_component_subsampled(&[0xFF, 0xD8, 0, 0xFF]));
    }

    #[test]
    fn routing() {
        assert!(!uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(!uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC1, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC9, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC1, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(!uses_craft_jpeg(&[0xFF, 0xD8]));
        assert!(!uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC2, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC2, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        assert!(uses_craft_jpeg(&[0xFF, 0xD8, 0xFF, 0xC3, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
    }

    #[test]
    fn lossless_is_unsupported_not_damaged() {
        let bytes = gray_block(0xC3, 12, None, &[0x12, 0x34]);
        let e = crate::decode(&bytes).unwrap_err();
        assert!(matches!(e, CodecError::Unsupported { .. }), "{e:?}");
    }

    #[test]
    fn twelve_bit_progressive_dc_scan_decodes() {
        let img = crate::decode(&twelve_bit_progressive_dc_block()).unwrap();
        assert_eq!((img.dimensions(), img.sample_type(), img.layout()), ((8, 8), SampleType::U16, ChannelLayout::Gray));
        assert!(img.warnings.is_empty());
    }

    #[test]
    fn malformed_twelve_bit_progressive_scan_is_malformed_not_damaged() {
        // A DC-first progressive scan must have Se=0, not the sequential scan's Se=63.
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC2, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0];
        bytes.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0, 0x12, 0x34, 0xFF, 0xD9]);
        let e = crate::decode(&bytes).unwrap_err();
        assert!(matches!(e, CodecError::Malformed { .. }), "{e:?}");
    }

    #[test]
    fn a_twelve_bit_frame_without_scan_data_is_refused_before_decoding() {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC1, 0, 11, 12, 0x40, 0, 0x40, 0, 1, 1, 0x11, 0];
        bytes.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0, 0xFF, 0xD9]);
        let e = crate::decode(&bytes).unwrap_err();
        assert!(e.to_string().contains("no scan data"), "{e}");
    }

    #[test]
    fn arithmetic_eight_bit_block_is_eight_bit() {
        let img = crate::decode(&arithmetic_block(8)).unwrap();
        assert_eq!((img.dimensions(), img.sample_type(), img.layout()), ((8, 8), SampleType::U8, ChannelLayout::Gray));
        assert!(img.data().iter().all(|&v| v.abs_diff(153) <= 1), "{:?}", &img.data()[..4]);
        assert!(img.warnings.is_empty());
    }

    #[test]
    fn arithmetic_twelve_bit_block_is_sixteen_bit() {
        let img = crate::decode(&arithmetic_block(12)).unwrap();
        assert_eq!((img.dimensions(), img.sample_type(), img.layout()), ((8, 8), SampleType::U16, ChannelLayout::Gray));
        let want = twelve_bit(2173.0);
        assert!(img.to_u16_samples().unwrap().iter().all(|&v| v.abs_diff(want) <= 1));
    }

    #[test]
    fn twelve_bit_huffman_block_decodes_to_sixteen_bit() {
        let img = crate::decode(&twelve_bit_huffman_block()).unwrap();
        assert_eq!((img.dimensions(), img.sample_type()), ((8, 8), SampleType::U16));
        let want = twelve_bit(3000.0);
        let px = img.to_u16_samples().unwrap();
        assert!(px.iter().all(|&v| v.abs_diff(want) <= 1), "{:?} vs {want}", &px[..4]);
        assert!(img.warnings.is_empty());
    }

    #[test]
    fn unmarked_cmyk_restores_direct_ink_polarity_on_craft_backend() {
        // craft-jpeg always returns non-YCC four-component samples with Adobe inversion undone.
        // The adapter restores direct inks only when no APP14 marker identifies Adobe storage.
        let mut u8_samples = [0, 32, 200, 255];
        undo_craft_cmyk_inversion_u8(&mut u8_samples, true);
        assert_eq!(u8_samples, [255, 223, 55, 0]);
        undo_craft_cmyk_inversion_u8(&mut u8_samples, false);
        assert_eq!(u8_samples, [255, 223, 55, 0], "marked samples keep craft-jpeg polarity");

        let mut u16_samples = [0, 8192, 32768, 65535];
        undo_craft_cmyk_inversion_u16(&mut u16_samples, true);
        assert_eq!(u16_samples, [65535, 57343, 32767, 0]);
        undo_craft_cmyk_inversion_u16(&mut u16_samples, false);
        assert_eq!(u16_samples, [65535, 57343, 32767, 0], "marked samples keep craft-jpeg polarity");
    }

    #[test]
    fn twelve_bit_cmyk_without_app14_keeps_direct_ink_polarity_on_craft_backend() {
        let image = Image::from_u8(16, 16, ChannelLayout::Cmyk, [10u8, 100, 200, 50].repeat(256)).unwrap();
        let mut marked = crate::encode(&image, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap();
        let sof = marked.windows(2).position(|pair| pair == [0xFF, 0xC0]).expect("baseline SOF0");
        marked[sof + 1] = 0xC1;
        marked[sof + 4] = 12;
        assert!(uses_craft_jpeg(&marked), "12-bit SOF1 routes through craft-jpeg");

        let app14 = b"\xFF\xEE\x00\x0EAdobe";
        let start = marked.windows(app14.len()).position(|part| part == app14).expect("Adobe APP14 marker");
        let mut unmarked = Vec::with_capacity(marked.len() - 16);
        unmarked.extend_from_slice(&marked[..start]);
        unmarked.extend_from_slice(&marked[start + 16..]);
        assert!(uses_craft_jpeg(&unmarked));

        let adobe = crate::decode(&marked).unwrap().to_u16_samples().unwrap();
        let direct_inks = crate::decode(&unmarked).unwrap().to_u16_samples().unwrap();
        assert_eq!(adobe.len(), direct_inks.len());
        for (&stored_adobe, &ink) in adobe.iter().zip(&direct_inks) {
            assert_eq!(ink, 65535 - stored_adobe, "unmarked samples remain direct inks");
        }
    }

    #[test]
    fn twelve_bit_jpeg_keeps_its_photoshop_resources() {
        // An APP13 `8BIM` block (paths, Photoshop settings) right after SOI, as on the zune-jpeg path.
        let resources = b"8BIM\x04\x04\0\0\0\0\0\0".to_vec();
        let mut app13 = PHOTOSHOP_HEADER.to_vec();
        app13.extend_from_slice(&resources);
        let stream = twelve_bit_huffman_block();
        let mut with = stream[..2].to_vec();
        segment(&mut with, 0xED, &app13);
        with.extend_from_slice(&stream[2..]);
        assert!(uses_craft_jpeg(&with));
        let img = crate::decode(&with).unwrap();
        assert_eq!(img.meta.photoshop_resources.as_deref(), Some(resources.as_slice()));
    }

    /// Where the entropy-coded data of the last scan starts.
    fn last_scan_data(s: &[u8]) -> usize {
        let sos = s.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
        sos + 2 + usize::from(u16::from_be_bytes([s[sos + 2], s[sos + 3]]))
    }

    /// A 12-bit scan whose entropy data runs out, EOI kept, is never a clean image.
    #[test]
    fn twelve_bit_scan_out_of_data_is_not_clean() {
        let full = twelve_bit_huffman_block();
        let data = last_scan_data(&full);
        for keep in 0..2 {
            let mut s = full[..data + keep].to_vec();
            s.extend([0xFF, 0xD9]);
            if let Ok(img) = crate::decode(&s) {
                assert!(!img.warnings.is_empty(), "keep {keep}: clean decode of missing data");
            }
        }
    }

    /// A multi-MCU SOF1 12-bit scan cut in half, EOI kept, decodes with the Truncated warning; a
    /// truncated scan too short for its frame is refused, as on the zune-jpeg path.
    #[test]
    fn damaged_or_truncated_twelve_bit_scan() {
        let (w, h) = (64usize, 64usize);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 4) as u8, (i / w * 4) as u8, (i * 7) as u8]).collect();
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 90).encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
        // Relabel the baseline frame as 12-bit SOF1: the Huffman data still parses, as different values.
        let sof = out.windows(2).position(|p| p == [0xFF, 0xC0]).unwrap();
        out[sof + 1] = 0xC1;
        out[sof + 4] = 12;
        let data = last_scan_data(&out);
        let end = out.len() - 2;
        let mut cut = out[..data + (end - data) / 2].to_vec();
        if cut.last() == Some(&0xFF) {
            cut.pop();
        }
        cut.extend([0xFF, 0xD9]);
        let img = crate::decode(&out).unwrap();
        assert_eq!(img.sample_type(), SampleType::U16);
        assert!(img.warnings.is_empty(), "{:?}", img.warnings);
        let img = crate::decode(&cut).unwrap();
        assert_eq!(img.warnings, vec![DecodeWarning::Truncated { format: Format::Jpeg }]);
        // Without EOI the same cut is a truncated file; under a 20000×20000 frame it is refused.
        cut.truncate(cut.len() - 2);
        assert_eq!(data_end(&cut).0, DataEnd::Truncated);
        assert!(crate::decode(&cut).is_ok());
        cut[sof + 5..sof + 9].copy_from_slice(&[0x4E, 0x20, 0x4E, 0x20]);
        let e = crate::decode(&cut).unwrap_err();
        assert!(e.to_string().contains("too short for the frame it declares"), "{e}");
    }
}
