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
    pub adobe_transform: Option<u8>,
    /// (width, height, components) from the first SOFn.
    pub frame: Option<(u32, u32, u8)>,
}

/// Walk the marker segments up to SOS and collect metadata.
pub(crate) fn scan_metadata(b: &[u8]) -> JpegMeta {
    let mut m = JpegMeta::default();
    let mut icc_parts: Vec<(u8, u8, &[u8])> = Vec::new();
    // Photoshop splits a large resource block over consecutive APP13 segments.
    let mut photoshop: Vec<u8> = Vec::new();
    let mut jfif_dpi = None;
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
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 || i + 2 + len > b.len() {
            break;
        }
        let seg = &b[i + 4..i + 2 + len];
        match marker {
            0xE0 if seg.len() >= 12 && seg.starts_with(b"JFIF\0") => {
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
            }
            0xEE if seg.len() >= 12 && seg.starts_with(b"Adobe") => {
                m.adobe_transform = Some(seg[11]);
            }
            _ => {}
        }
        i += 2 + len;
    }
    m.dpi = photoshop_resolution(&photoshop)
        .or_else(|| m.xmp.as_deref().and_then(xmp_resolution))
        .or_else(|| m.exif.as_deref().and_then(exif_resolution))
        .or(jfif_dpi);
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
            let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&v| v == 0xFF)) else { return cut(scan_data, scanned) };
            i += p;
            continue;
        }
        let Some(&code) = b.get(i + 1) else { return cut(scan_data, scanned) };
        match code {
            0xD9 => return (DataEnd::Complete, scanned),
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            _ => {
                let Some(len) = b.get(i + 2..i + 4).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes) else {
                    return cut(scan_data, scanned);
                };
                i = i.saturating_add(2 + usize::from(len));
                if code == 0xDA {
                    let start = i;
                    loop {
                        let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&v| v == 0xFF)) else {
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

/// Whether `craft-jpeg` (rather than `zune-jpeg`) decodes the file: arithmetic coding
/// (SOF9–SOF11) or a DCT frame whose precision isn't 8 bits (#4934, #5003). Lossless (SOF3) and
/// 12-bit progressive Huffman (SOF2) go there too: craft-jpeg reports them unsupported, where
/// zune-jpeg's failure would read as a damaged file.
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

/// Decode a file [`uses_craft_jpeg`] routes away from `zune-jpeg`; `damaged` says entropy-coded
/// data was cut short or corrupt.
fn decode_craft_jpeg(bytes: &[u8], limits: &Limits, adobe_transform: Option<u8>) -> Result<(Image, bool), CodecError> {
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
    let img = match d.samples {
        craft_jpeg::Samples::U8(px) => Image::from_raw(d.width, d.height, layout, SampleType::U8, px)?,
        craft_jpeg::Samples::U16(px) => Image::from_u16(d.width, d.height, layout, &px)?,
    };
    Ok((img, d.damaged))
}

/// Most pixels one byte of a JPEG may account for. Every 8×8 block of a frame needs at least a
/// one-bit DC code (512 pixels a byte for a flat image under ideal Huffman tables), and real files,
/// truncated or not, are far above this.
const MAX_PIXELS_PER_BYTE: u64 = 2048;

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let (end, scan_bytes) = data_end(bytes);
    if end == DataEnd::Empty {
        return Err(err("the file ends before any image data"));
    }
    let meta = scan_metadata(bytes);
    if let Some((w, h, nc)) = meta.frame {
        limits.check_bytes(w, h, u64::from(nc.max(1)))?;
    }
    if uses_craft_jpeg(bytes) {
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
        let (mut img, damaged) = decode_craft_jpeg(bytes, limits, meta.adobe_transform)?;
        img.icc = meta.icc;
        img.meta = Metadata { exif: meta.exif, xmp: meta.xmp, dpi: meta.dpi, photoshop_resources: meta.photoshop, ..Default::default() };
        if end == DataEnd::Truncated || damaged {
            img.warnings.push(DecodeWarning::Truncated { format: F });
        }
        return Ok(img);
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
    let mut px = dec.decode().map_err(err)?;
    let expected = w as usize * h as usize * layout.channels();
    if px.len() < expected {
        return Err(err("short pixel buffer"));
    }
    px.truncate(expected);
    match input {
        // Adobe-style CMYK is stored inverted (255 = no ink).
        ColorSpace::CMYK => px.iter_mut().for_each(|v| *v = 255 - *v),
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
    let optimized = opts.jpeg_format == JpegFormat::Optimized && !subsampled;
    enc.set_optimized_huffman_tables(progressive || optimized);
    let e = |e: jpeg_encoder::EncodingError| CodecError::encode(F, e);
    if opts.embed_metadata {
        if img.layout() != ChannelLayout::Cmyk
            && let Some((x, y)) = img.meta.dpi
            && x >= 1.0
            && y >= 1.0
        {
            enc.set_density(jpeg_encoder::Density::Inch { x: x.round().min(65535.0) as u16, y: y.round().min(65535.0) as u16 });
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
            enc.add_app_segment(1, &seg).map_err(e)?;
        }
        if let Some(seg) = img.meta.xmp.as_deref().and_then(|x| xmp_segment(x, img.meta.dpi)) {
            enc.add_app_segment(1, &seg).map_err(e)?;
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

    fn twelve_bit(v: f32) -> u16 {
        (v * 65535.0 / 4095.0).round() as u16
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
    fn lossless_and_twelve_bit_progressive_are_unsupported_not_damaged() {
        for marker in [0xC3, 0xC2] {
            let mut bytes = vec![0xFF, 0xD8, 0xFF, marker, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0];
            bytes.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0, 0x12, 0x34, 0xFF, 0xD9]);
            let e = crate::decode(&bytes).unwrap_err();
            assert!(matches!(e, CodecError::Unsupported { .. }), "{marker:#x}: {e:?}");
        }
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
