//! WebP: decode lossy/lossless (first frame) via `image-webp`; encode lossless via
//! `image-webp`'s VP8L encoder, or lossy with our own VP8 encoder (`super::vp8`) wrapped in the
//! RIFF container written here (a bare `VP8 ` chunk, or `VP8X` with `ALPH` alpha and `ICCP` /
//! `EXIF` / `XMP ` chunks). ICC/EXIF/XMP both ways.

use std::io::Cursor;

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits, alloc_zeroed};

const F: Format = Format::WebP;

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

type Decoder<'a> = image_webp::WebPDecoder<Cursor<&'a [u8]>>;

/// The decoder with its chunks read, the stored size and the output layout, the size checked.
fn open<'a>(bytes: &'a [u8], limits: &Limits) -> Result<(Decoder<'a>, u32, u32, ChannelLayout), CodecError> {
    let mut dec = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(err)?;
    dec.set_memory_limit(limits.alloc_usize());
    let (w, h) = dec.dimensions();
    let layout = if dec.has_alpha() { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    limits.check(w, h, layout, SampleType::U8)?;
    // VP8L back-references and VP8 skipped macroblocks compress flat areas extremely well, so the
    // ratio is generous; it still stops a header-only file from asking for a gigabyte.
    Limits::check_plausible(u64::from(w) * u64::from(h) * layout.channels() as u64, bytes.len(), 1 << 16)?;
    Ok((dec, w, h, layout))
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let (mut dec, w, h, layout) = open(bytes, limits)?;
    let size = dec.output_buffer_size().ok_or_else(|| err("image too large"))?;
    let mut buf = alloc_zeroed::<u8>(size)?;
    dec.read_image(&mut buf).map_err(err)?;
    let icc = dec.icc_profile().ok().flatten();
    let exif = dec.exif_metadata().ok().flatten();
    let xmp = dec.xmp_metadata().ok().flatten().and_then(|b| String::from_utf8(b).ok());
    let mut img = Image::from_u8(w, h, layout, buf)?;
    img.icc = icc;
    img.meta = Metadata { exif, xmp, ..Default::default() };
    if dec.is_animated() && dec.num_frames() > 1 {
        img.warnings.push(DecodeWarning::MoreFrames { total: Some(dec.num_frames()) });
    }
    Ok(img)
}

/// The stored size and EXIF block [`decode`] would find, from the container's chunks alone: the
/// same header reads and size checks, without decoding the bitstream.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<(u32, u32, Option<Vec<u8>>), CodecError> {
    let (mut dec, w, h, _) = open(bytes, limits)?;
    Ok((w, h, dec.exif_metadata().ok().flatten()))
}

/// Whether `bytes` is an animated WebP of more than one frame.
pub(crate) fn is_animated(bytes: &[u8]) -> bool {
    image_webp::WebPDecoder::new(Cursor::new(bytes)).is_ok_and(|d| d.is_animated() && d.num_frames() > 1)
}

/// Every frame of an animated WebP, or `None` when it is a still one (or an animation of one
/// frame), which [`decode`] reads whole. The frames come composited onto the canvas by
/// `image-webp` (offsets, blending and disposal applied), each a full picture with the file's
/// ICC profile and its duration in milliseconds.
///
/// The first frame must decode. If a later frame is damaged the frames before it are returned
/// and the first one carries [`DecodeWarning::Truncated`]. The frame count
/// ([`super::gif_frames::MAX_GIF_FRAMES`]) and the total decoded bytes are checked against
/// `limits` before each frame is kept.
pub(crate) fn decode_frames(bytes: &[u8], limits: &Limits) -> Result<Option<Vec<super::gif_frames::AnimationFrame>>, CodecError> {
    use super::gif_frames::{AnimationFrame, MAX_GIF_FRAMES};
    if !is_animated(bytes) {
        return Ok(None);
    }
    let (mut dec, w, h, layout) = open(bytes, limits)?;
    let size = dec.output_buffer_size().ok_or_else(|| err("image too large"))?;
    let icc = dec.icc_profile().ok().flatten();
    let mut buf = alloc_zeroed::<u8>(size)?;
    let mut out: Vec<AnimationFrame> = Vec::new();
    let mut damaged = false;
    for _ in 0..dec.num_frames() {
        if out.len() >= MAX_GIF_FRAMES {
            return Err(CodecError::LimitExceeded(format!("more than {MAX_GIF_FRAMES} frames")));
        }
        // Every frame keeps its own copy of the file's ICC profile along with its pixels.
        let frame_bytes = (size as u64).saturating_add(icc.as_ref().map_or(0, |p| p.len() as u64));
        if frame_bytes.saturating_mul(out.len() as u64 + 1) > limits.max_alloc {
            return Err(CodecError::LimitExceeded(format!("{} frames of {w}x{h} exceed max_alloc {}", out.len() + 1, limits.max_alloc)));
        }
        let delay_ms = match dec.read_frame(&mut buf) {
            Ok(d) => d,
            Err(e) if out.is_empty() => return Err(err(e)),
            Err(_) => {
                damaged = true;
                break;
            }
        };
        let mut image = Image::from_u8(w, h, layout, buf.clone())?;
        image.icc.clone_from(&icc);
        out.push(AnimationFrame { image, delay_ms });
    }
    if damaged && let Some(first) = out.first_mut() {
        first.image.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(Some(out))
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    if !opts.webp_lossless {
        return encode_lossy(&img, opts);
    }
    let ct = match img.layout() {
        ChannelLayout::Gray => image_webp::ColorType::L8,
        ChannelLayout::GrayA => image_webp::ColorType::La8,
        ChannelLayout::Rgb => image_webp::ColorType::Rgb8,
        ChannelLayout::Rgba => image_webp::ColorType::Rgba8,
        l => return Err(CodecError::encode(F, format!("unsupported layout {l:?}"))),
    };
    let mut out = Vec::new();
    let mut enc = image_webp::WebPEncoder::new(&mut out);
    if opts.embed_icc
        && let Some(icc) = &img.icc
    {
        enc.set_icc_profile(icc.clone());
    }
    if opts.embed_metadata {
        if let Some(exif) = export_exif(&img) {
            enc.set_exif_metadata(exif);
        }
        if let Some(xmp) = &img.meta.xmp {
            enc.set_xmp_metadata(crate::resolution::export_xmp(xmp, img.meta.dpi).as_bytes().to_vec());
        }
    }
    enc.encode(img.data(), img.width(), img.height(), ct).map_err(|e| CodecError::encode(F, e))?;
    Ok(out)
}

/// Lossy: the picture as a VP8 key frame, alpha (when any pixel is translucent) as an
/// uncompressed `ALPH` chunk, metadata as chunks.
fn encode_lossy(img: &Image, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    use super::vp8;
    let (w, h) = img.dimensions();
    if w > vp8::MAX_DIMENSION || h > vp8::MAX_DIMENSION {
        return Err(CodecError::encode(F, format!("lossy WebP holds at most {} pixels per side", vp8::MAX_DIMENSION)));
    }
    // The VP8 path takes 8-bit RGB(A); gray becomes RGB with flat chroma.
    let has_alpha = img.layout().has_alpha();
    let target = if has_alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let rgb = img.converted(target, SampleType::U8);
    let stride = target.channels();
    let yuv = vp8::Yuv::from_rgb(rgb.data(), w, h, stride);
    let frame = vp8::encode_frame(&yuv, vp8::Params { quality: opts.webp_quality.min(100) }).map_err(|e| CodecError::encode(F, e))?;
    let alpha: Option<Vec<u8>> = if has_alpha {
        let a: Vec<u8> = rgb.data().iter().skip(3).step_by(4).copied().collect();
        if a.iter().all(|&v| v == 255) { None } else { Some(a) }
    } else {
        None
    };
    let icc = if opts.embed_icc { img.icc.as_deref() } else { None };
    let exif = if opts.embed_metadata { export_exif(img) } else { None };
    let xmp = if opts.embed_metadata { img.meta.xmp.as_deref().map(|x| crate::resolution::export_xmp(x, img.meta.dpi)) } else { None };
    Ok(riff(w, h, &frame, alpha.as_deref(), icc, exif.as_deref(), xmp.as_deref().map(str::as_bytes)))
}

/// The EXIF to write: the image's, upright (the pixels are written as they are shown, so a viewer
/// must never rotate them again), carrying the resolution. WebP has no resolution field of its own,
/// so the resolution goes into EXIF, added when the image has none (#2399).
pub(crate) fn export_exif(img: &Image) -> Option<Vec<u8>> {
    let exif = img.meta.exif.as_deref().unwrap_or_default();
    // A WebP without EXIF already reads as 72 ppi, so the default needs no block of its own (and
    // the extended container it would bring to every web export).
    if exif.is_empty() && img.meta.dpi.is_none_or(is_default_ppi) {
        return None;
    }
    let out = crate::resolution::export_exif_adding_missing_resolution(exif, img.meta.dpi).into_owned();
    (!out.is_empty()).then_some(out)
}

/// Whether a WebP of `img` keeps its resolution: the EXIF written carries it. Not when the image's
/// own EXIF can't be parsed: that block is written as it is rather than replaced, so the
/// resolution is dropped (and reported) instead of the metadata.
pub(crate) fn keeps_dpi(img: &Image) -> bool {
    let bare_default = img.meta.exif.as_deref().is_none_or(<[u8]>::is_empty) && img.meta.dpi.is_none_or(is_default_ppi);
    // The XMP written beside it carries the resolution too, and readers look there first.
    let in_xmp = img.meta.xmp.as_deref().is_some_and(|x| crate::resolution::xmp_resolution(&crate::resolution::export_xmp(x, img.meta.dpi)).is_some());
    bare_default || in_xmp || export_exif(img).is_some_and(|e| crate::resolution::exif_resolution(&e).is_some())
}

/// 72 ppi each way: what a file without a resolution opens as.
fn is_default_ppi((x, y): (f32, f32)) -> bool {
    (x - 72.0).abs() < 0.01 && (y - 72.0).abs() < 0.01
}

/// A RIFF chunk: FourCC, little-endian size, payload, padded to an even length.
fn chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

/// The WebP container around a VP8 frame: the simple form when there is nothing else, else the
/// extended form (`VP8X`) with the alpha plane and the metadata chunks in the specified order.
fn riff(w: u32, h: u32, frame: &[u8], alpha: Option<&[u8]>, icc: Option<&[u8]>, exif: Option<&[u8]>, xmp: Option<&[u8]>) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    let extended = alpha.is_some() || icc.is_some() || exif.is_some() || xmp.is_some();
    if extended {
        let mut flags = 0u8;
        if icc.is_some() {
            flags |= 0x20;
        }
        if alpha.is_some() {
            flags |= 0x10;
        }
        if exif.is_some() {
            flags |= 0x08;
        }
        if xmp.is_some() {
            flags |= 0x04;
        }
        let mut vp8x = vec![flags, 0, 0, 0];
        vp8x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        vp8x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        chunk(&mut body, b"VP8X", &vp8x);
        if let Some(icc) = icc {
            chunk(&mut body, b"ICCP", icc);
        }
        if let Some(a) = alpha {
            // Header byte: reserved, no pre-processing, no filtering, no compression.
            let mut alph = Vec::with_capacity(a.len() + 1);
            alph.push(0);
            alph.extend_from_slice(a);
            chunk(&mut body, b"ALPH", &alph);
        }
    }
    chunk(&mut body, b"VP8 ", frame);
    if let Some(exif) = exif {
        chunk(&mut body, b"EXIF", exif);
    }
    if let Some(xmp) = xmp {
        chunk(&mut body, b"XMP ", xmp);
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}
