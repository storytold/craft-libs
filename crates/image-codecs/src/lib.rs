//! `craft-image-codecs`: symmetric, depth- and metadata-preserving codecs for
//! flat raster image formats.
//!
//! * Every format we **write** we can also **read** (see
//!   [`ASYMMETRIC_EXCEPTIONS`] for the documented, feature-gated exception).
//! * Bit depth (8/16-bit integer, 16/32-bit float), ICC profiles and basic
//!   metadata (EXIF, XMP, DPI, text) are preserved where the format allows.
//! * [`caps`] declares per-format capabilities and [`fidelity_warnings`]
//!   tells the UI exactly what an export will lose.
//! * APIs use bytes or caller-provided readers, with no ambient filesystem access,
//!   and build for `wasm32-unknown-unknown`. Byte decoders enforce [`Limits`];
//!   [`exr_stream`] instead bounds metadata and one decoded compression block.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod allocation;
pub mod cicp;
mod codecs;
mod error;
pub mod exif_color;
pub mod exr_stream;
mod fidelity;
mod format;
mod image;
mod inspection;
mod lab;
mod options;
pub mod orientation;
pub mod resolution;
pub mod source_settings;
pub mod web;
pub use crate::allocation::probe_allocation_size;
pub use crate::cicp::Cicp;
pub use crate::cicp::ContentLightLevel;
pub use crate::codecs::exr_cryptomatte::{cryptomatte_id, cryptomatte_key, cryptomatte_preview_color};
pub use crate::codecs::gif_frames::{AnimationFrame, MAX_GIF_FRAMES};
pub use crate::codecs::png::{
    PngDecodeRows, PngExtras, PngHeader, PngPalette, PngRowSink, PngRows, SharedBytes, encode_indexed as encode_png_indexed,
    encode_pq as encode_png_pq, palette as png_palette, stereo as png_stereo,
};
pub use crate::codecs::tiff::{
    PhotoshopTags as TiffPhotoshopTags, TiffBands, photoshop_tags as tiff_photoshop_tags, writes_little_endian as tiff_writes_little_endian,
};
pub use crate::codecs::tiff_ifd::{ExtraSampleKind, InkSet, TiffInfo, TiffPage, TiffPageKind};
pub use crate::error::CodecError;
pub use crate::exif_color::{EXIF_COLOR_SPACE_SRGB, exif_color_space};
pub use crate::fidelity::{FidelityWarning, fidelity_warnings, fidelity_warnings_with};
pub use crate::format::{ASYMMETRIC_EXCEPTIONS, Format, FormatCaps, caps, detect, from_extension};
pub use crate::image::{
    ChannelLayout, CryptomatteBuffer, CryptomatteLayer, DecodeWarning, DeepChannel, DeepImage, ExrChannelInfo, ExrPartInfo, ExtraSample, Image,
    MAX_EXTRA_SAMPLES, Metadata, SampleType, convert_samples,
};
pub use crate::inspection::{RasterColorType, RasterInspection, ToneClass, classify_tone, embedded_icc, inspect_raster};
pub use crate::options::{
    DdsCompression, DdsOptions, DecodeOptions, DpxBits, EncodeOptions, ExrCompression, JpegFormat, Limits, PngCompression, PngStereo,
    TIFF_PYRAMID_MIN, TiffCompression,
};
pub use crate::orientation::{OrientSteps, exif_orientation, upright_exif, upright_xmp};
pub use crate::resolution::{exif_resolution, export_exif, export_xmp, photoshop_resolution, xmp_resolution};
pub use crate::source_settings::{WebpEncoding, jpeg_quality, webp_encoding};
pub use half::f16;

use crate::codecs::{dds, dicom, dpx, exr, fits, heif, iff, jpeg, jxl, pcx, png, pnm, tiff, via_image, webp, xpm};

/// Detect the format and decode with default [`Limits`].
pub fn decode(bytes: &[u8]) -> Result<Image, CodecError> {
    decode_with(bytes, &DecodeOptions::default())
}

/// Detect the format and decode.
pub fn decode_with(bytes: &[u8], opts: &DecodeOptions) -> Result<Image, CodecError> {
    let format = detect(bytes).ok_or(CodecError::UnknownFormat)?;
    decode_as_with(format, bytes, opts)
}

/// Decode as a specific format with default [`Limits`].
pub fn decode_as(format: Format, bytes: &[u8]) -> Result<Image, CodecError> {
    decode_as_with(format, bytes, &DecodeOptions::default())
}

/// Decode as a specific format.
pub fn decode_as_with(format: Format, bytes: &[u8], opts: &DecodeOptions) -> Result<Image, CodecError> {
    // HEIF without the `heif` feature: its decoder stub says what is missing from this build.
    if !caps(format).read && !matches!(format, Format::Heif | Format::Jxl) {
        return Err(CodecError::unsupported(format, "decoding is not available for this format"));
    }
    let l = &opts.limits;
    let img = match format {
        Format::Png => png::decode(bytes, l),
        Format::Jpeg => jpeg::decode(bytes, l),
        Format::Tiff => tiff::decode(bytes, l),
        Format::WebP => webp::decode(bytes, l),
        Format::Pnm => pnm::decode(bytes, l),
        Format::Pcx => pcx::decode(bytes, l),
        Format::Xpm => xpm::decode(bytes, l),
        Format::Dds => dds::decode(bytes, l),
        Format::Fits => fits::decode(bytes, l),
        Format::Dpx => dpx::decode(bytes, l),
        Format::Cineon => dpx::decode_cineon(bytes, l),
        Format::Dicom => dicom::decode(bytes, l),
        Format::Iff => iff::decode(bytes, l),
        Format::OpenExr => exr::decode(bytes, l),
        Format::Heif => heif::decode(bytes, l, opts.keep_orientation),
        Format::Jxl => Err(CodecError::unsupported(format, "JPEG XL reading is not yet available; export is supported")),
        Format::Gif | Format::Bmp | Format::Tga | Format::Ico | Format::Qoi | Format::Hdr | Format::Avif => via_image::decode(format, bytes, l),
    }?;
    finish_format(format, bytes, img, opts)
}

/// What [`decode_as_with`] does with a PNG decoded in steps by [`PngDecodeRows`] (`img` is its
/// output): the resolution from the metadata, the upright orientation and the final limit check,
/// so the result equals [`decode_as_with`]'s on the same file.
pub fn finish_png_decode(img: Image, opts: &DecodeOptions) -> Result<Image, CodecError> {
    match begin_png_finish(img, opts)? {
        PngFinish::Done(img) => Ok(*img),
        PngFinish::Orient(mut steps) => loop {
            if let Some(img) = steps.step(&mut || true)? {
                return Ok(img);
            }
        },
    }
}

/// Where [`begin_png_finish`] leaves a decoded PNG.
pub enum PngFinish {
    /// Finished: it was already upright (or the options keep the orientation).
    Done(Box<Image>),
    /// To be turned upright, a band of rows at a time, by driving these steps.
    Orient(Box<OrientSteps>),
}

/// [`finish_png_decode`] with the turning upright (orientations 2-8) left to the caller, in
/// bands it can pace: the resolution from the metadata and the limit checks are made here, and the
/// result of the steps equals what [`finish_png_decode`] returns.
pub fn begin_png_finish(mut img: Image, opts: &DecodeOptions) -> Result<PngFinish, CodecError> {
    if img.meta.dpi.is_none() {
        img.meta.dpi = resolution::from_metadata(img.meta.exif.as_deref(), img.meta.xmp.as_deref());
    }
    let o = img.meta.exif.as_deref().map_or(1, exif_orientation);
    let l = &opts.limits;
    // As `finish_decode` does: the final guard, then (turning 5-8 swaps the sides) the upright size.
    l.check(img.width(), img.height(), img.layout(), img.sample_type())?;
    if opts.keep_orientation || !(2..=8).contains(&o) {
        return Ok(PngFinish::Done(Box::new(img)));
    }
    if (5..=8).contains(&o) {
        l.check(img.height(), img.width(), img.layout(), img.sample_type())?;
    }
    Ok(PngFinish::Orient(Box::new(OrientSteps::new(img, o)?)))
}

/// The steps after a decoder of `format` has produced `img` from `bytes`.
fn finish_format(format: Format, bytes: &[u8], mut img: Image, opts: &DecodeOptions) -> Result<Image, CodecError> {
    // Without a resolution of the format's own (PNG pHYs, TIFF tags, JPEG's metadata), the
    // XMP or EXIF one counts, like Photoshop: a WebP, HEIF or PNG from a camera or phone
    // opens at its recorded resolution, not at the 72 ppi default.
    if img.meta.dpi.is_none() && format != Format::Tiff {
        img.meta.dpi = resolution::from_metadata(img.meta.exif.as_deref(), img.meta.xmp.as_deref());
    }
    // Turn the pixels upright, like Photoshop: a TIFF records it in the decoded page's own
    // directory, the others in their EXIF block. The metadata is rewritten to Orientation = 1
    // on the way. HEIF keeps it in its container, and its decoder has already applied it.
    let o = match format {
        Format::Tiff => tiff::orientation(bytes, None),
        Format::Heif => 1,
        Format::Dpx => dpx::orientation(bytes),
        Format::Cineon => dpx::cineon_orientation(bytes),
        _ => img.meta.exif.as_deref().map_or(1, exif_orientation),
    };
    finish_decode(img, o, opts)
}

/// Decodes a PNG a group of rows at a time, for a caller that builds its own pixel storage and
/// never needs the whole decoded image. `begin` gets the [`PngHeader`] once and returns whether to go on, then `rows(y0, data)`
/// gets whole rows `y0..` (`group` of them, fewer in the last call; native-endian) in final order, holding exactly the
/// samples [`decode`] would return. The result is the metadata, complete once the file is read.
///
/// `Ok(None)`, with no row delivered, means the rows can't be had in final order (an
/// interlaced or animated PNG) or `begin` returned false: use [`decode_as_with`]. Damaged files fail as [`decode_as_with`]
/// fails them, possibly after some rows. The limits are checked before the first row; the
/// orientation is not applied, so [`PngExtras::orientation`] other than 1 is for the caller to
/// handle (or to fall back to [`decode_as_with`], which turns the image upright).
pub fn decode_png_rows<E: From<CodecError>>(
    bytes: &[u8],
    opts: &DecodeOptions,
    group: u32,
    begin: &mut dyn FnMut(&PngHeader) -> Result<bool, E>,
    rows: &mut PngRowSink<'_, E>,
) -> Result<Option<PngExtras>, E> {
    png::decode_rows(bytes, &opts.limits, group, begin, rows)
}

/// The upright pixel size [`decode`] would return, read from the file's header without decoding
/// pixels, for WebP, OpenEXR (flat) and the GIF/BMP/TGA/ICO/QOI/HDR family. It runs the same
/// header reads and size checks as the decoder (default [`Limits`]) and swaps the size for an
/// EXIF orientation of 5–8, so wherever [`decode`] succeeds the two agree. Other formats (and
/// deep EXR or AVIF) return [`CodecError::Unsupported`]: decode those to learn their size.
pub fn probe_size(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let format = detect(bytes).ok_or(CodecError::UnknownFormat)?;
    if !caps(format).read {
        return Err(CodecError::unsupported(format, "decoding is not available for this format"));
    }
    let l = &Limits::default();
    let (w, h, exif) = match format {
        Format::WebP => webp::probe(bytes, l)?,
        Format::OpenExr => exr::probe(bytes, l).map(|(w, h)| (w, h, None))?,
        Format::Gif | Format::Bmp | Format::Tga | Format::Ico | Format::Qoi | Format::Hdr | Format::Avif => via_image::probe(format, bytes, l)?,
        _ => return Err(CodecError::unsupported(format, "no header-only size probe")),
    };
    let o = exif.as_deref().map_or(1, exif_orientation);
    Ok(if (5..=8).contains(&o) { (h, w) } else { (w, h) })
}

/// A JPEG's frame as its header declares it, read without decoding pixels: the stored size, the
/// layout [`decode`] returns, whether [`decode`] hands the file to its whole-coefficient
/// decoder (arithmetic-coded, lossless or 12-bit frames), whose working set is several times the
/// decoded image, and the coefficients it holds beside its output when it can't decode row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub layout: ChannelLayout,
    pub whole_coefficients: bool,
    /// Bytes of DCT coefficients [`decode`] holds at once beside its output when it can't turn
    /// the frame into rows as it reads: an 8-bit progressive frame (SOF2), whose scans refine
    /// every block in turn, or an 8-bit Huffman one (SOF0, SOF1) whose first scan lacks some of
    /// its components. Every block of every component at its own sampling, 64 16-bit values
    /// each; 0 for the frames it decodes a line of MCUs at a time.
    pub held_coefficients: u64,
}

/// [`JpegInfo`] of `bytes`; `None` without a readable frame of 1, 3 or 4 components.
pub fn jpeg_info(bytes: &[u8]) -> Option<JpegInfo> {
    jpeg::info(bytes)
}

/// The colour table of a GIF's first frame (its Local Color Table, else the Global one) and the
/// entry its Graphic Control Extension makes transparent, read without decoding pixels.
pub fn gif_palette(bytes: &[u8]) -> Option<(Vec<[u8; 3]>, Option<u8>)> {
    crate::codecs::gif_palette::palette(bytes)
}

/// Returns the embedded colour table for an indexed BMP, if it has one.
pub fn bmp_palette(bytes: &[u8]) -> Result<Option<Vec<[u8; 3]>>, CodecError> {
    via_image::bmp_palette(bytes)
}

/// Encodes `indices` (row-major, one byte per pixel) as an 8-bit palette BMP with the colour
/// table `palette` (1 to 256 entries). Used for Indexed Color documents; BMP has no transparency.
pub fn encode_bmp_indexed(width: u32, height: u32, indices: &[u8], palette: &[[u8; 3]]) -> Result<Vec<u8>, CodecError> {
    via_image::encode_bmp_indexed(width, height, indices, palette)
}

/// Lists the pages (image directories) of a TIFF or BigTIFF file without decoding pixels: the
/// main IFD chain and its SubIFDs, each with its size, kind (page, reduced-resolution copy or
/// mask) and storage. [`TiffInfo::default_page`] is the page [`decode`] opens.
pub fn tiff_info(bytes: &[u8]) -> Result<TiffInfo, CodecError> {
    crate::codecs::tiff_ifd::tiff_info(bytes)
}

/// Decodes page `page` (an index into [`tiff_info`]'s `pages`) of a TIFF or BigTIFF file,
/// applying that page's orientation unless `opts.keep_orientation`.
pub fn decode_tiff_page(bytes: &[u8], page: usize, opts: &DecodeOptions) -> Result<Image, CodecError> {
    let img = tiff::decode_page(bytes, Some(page), &opts.limits)?;
    finish_decode(img, tiff::orientation(bytes, Some(page)), opts)
}

/// Decodes every frame of a GIF (a still GIF yields one), each a full RGBA8 picture of the
/// logical screen with its delay. [`decode`] keeps the first frame only. See
/// [`MAX_GIF_FRAMES`] and `opts.limits` for what is refused.
pub fn decode_gif_frames(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<AnimationFrame>, CodecError> {
    crate::codecs::gif_frames::decode(bytes, opts)
}

/// Decodes every frame of a WebP (a still one yields one frame with a 0 delay, as [`decode`]
/// reads it), each a full picture of the canvas with its duration and the file's ICC profile.
/// [`decode`] keeps the first frame only. See [`MAX_GIF_FRAMES`] and `opts.limits` for what is
/// refused.
pub fn decode_webp_frames(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<AnimationFrame>, CodecError> {
    match crate::codecs::webp::decode_frames(bytes, &opts.limits)? {
        Some(frames) => Ok(frames),
        None => Ok(vec![AnimationFrame { image: decode_as_with(Format::WebP, bytes, opts)?, delay_ms: 0 }]),
    }
}

/// Whether `bytes` is an animated WebP of more than one frame (which [`decode`] opens as its
/// first frame, with a warning, and [`decode_webp_frames`] reads whole).
pub fn webp_is_animated(bytes: &[u8]) -> bool {
    crate::codecs::webp::is_animated(bytes)
}

/// Inspect declared source data which a flat decode and save cannot retain: additional frames,
/// TIFF pages, masks or unretained channels, ICO entries, DDS mip levels, slices, faces or arrays,
/// DPX elements and FITS HDU payloads.
/// Pixels are not decoded; derived TIFF thumbnails do not require preservation here.
/// Other formats return false; this does not establish preservation of other format-specific data.
pub fn has_unretained_source_data(bytes: &[u8]) -> Result<bool, CodecError> {
    match detect(bytes) {
        Some(Format::Gif) => via_image::gif_has_more_frames(bytes).ok_or_else(|| CodecError::malformed(Format::Gif, "cannot inspect every frame")),
        Some(Format::Png) => png::has_additional_images(bytes),
        Some(Format::WebP) => Ok(webp_is_animated(bytes)),
        Some(Format::Tiff) => {
            let info = tiff_info(bytes)?;
            if !info.complete {
                return Err(CodecError::malformed(Format::Tiff, "cannot inspect every source image directory"));
            }
            let retained = info.default_page();
            if info.pages.iter().enumerate().any(|(index, page)| Some(index) != retained && page.kind != TiffPageKind::ReducedResolution) {
                return Ok(true);
            }
            retained.and_then(|index| info.pages.get(index)).map_or(Ok(false), |page| tiff::has_unretained_samples(bytes, page.ifd_offset))
        }
        Some(Format::Ico) => {
            let count = bytes
                .get(4..6)
                .and_then(|b| <[u8; 2]>::try_from(b).ok())
                .ok_or_else(|| CodecError::malformed(Format::Ico, "truncated image count"))?;
            Ok(u16::from_le_bytes(count) > 1)
        }
        Some(Format::Dds) => dds::has_additional_images(bytes),
        Some(Format::Dpx) => dpx::has_additional_images(bytes),
        Some(Format::Fits) => fits::has_additional_images(bytes),
        _ => Ok(false),
    }
}

/// The Orientation (1–8) of the page [`decode`] opens from a TIFF or BigTIFF file; 1 when it is
/// absent, malformed or out of range.
pub fn tiff_orientation(bytes: &[u8]) -> u16 {
    tiff::orientation(bytes, None)
}

/// The Orientation (1–8) of page `page` (an index into [`tiff_info`]'s `pages`); 1 when it is
/// absent, malformed or out of range, or the page does not exist.
pub fn tiff_page_orientation(bytes: &[u8], page: usize) -> u16 {
    tiff::orientation(bytes, Some(page))
}

/// The EXIF block (`Metadata::exif`) of a camera raw built on TIFF (NEF, ARW, CR2, DNG,
/// Panasonic RW2), for the image developed from it: IFD0's descriptive tags (make, model,
/// date…) and the Exif and GPS directories, as a TIFF opens with. Orientation is not part of
/// it, nor the Exif directory's pixel dimensions (the sensor's or the preview's size). When those
/// were all its Exif directory held, the block is IFD0's tags alone, so the camera's make and
/// model are still kept. `None` when the file is not built on a classic TIFF stream or has none
/// of these tags.
pub fn raw_exif(bytes: &[u8]) -> Option<Vec<u8>> {
    let dirs = crate::codecs::tiff_exif::read_raw(bytes).filter(|d| d.has_directories() || !d.ifd0.is_empty())?;
    Some(crate::codecs::tiff_exif::block(&dirs))
}

/// The limits check and the orientation, shared by every decoder.
fn finish_decode(img: Image, o: u16, opts: &DecodeOptions) -> Result<Image, CodecError> {
    let l = &opts.limits;
    // Final guard for decoders whose header we could not pre-inspect.
    l.check(img.width(), img.height(), img.layout(), img.sample_type())?;
    if opts.keep_orientation {
        return Ok(img);
    }
    // Orientations 5–8 swap width and height: with asymmetric limits a stored landscape
    // that fit can turn into a portrait that does not, so check the upright size too.
    if (5..=8).contains(&o) {
        l.check(img.height(), img.width(), img.layout(), img.sample_type())?;
    }
    img.oriented(o)
}

/// Lists the parts of an OpenEXR file without decoding pixels: header facts per part
/// (name, view, size, channels, deep/tiled), what a multi-part chooser — a Maya/Arnold
/// render writes one part per AOV — offers. The indices are what [`decode_exr_part`]
/// takes; [`decode_as_with`] opens the part with the highest [`ExrPartInfo::color_rank`].
pub fn exr_info(bytes: &[u8], limits: &Limits) -> Result<Vec<ExrPartInfo>, CodecError> {
    exr::info(bytes, limits)
}

/// Decodes part `part` (an index into [`exr_info`]'s list) of an OpenEXR file.
/// Deep parts stay with [`decode_deep_exr`]; a file that has any is refused here.
pub fn decode_exr_part(bytes: &[u8], part: usize, opts: &DecodeOptions) -> Result<Image, CodecError> {
    let img = exr::decode_part(bytes, part, &opts.limits)?;
    finish_decode(img, 1, opts)
}

/// Lists every Cryptomatte layer of an EXR file (specification 1.2: object/material IDs
/// with coverage, as written by Arnold, V-Ray, Redshift, Mantra/Karma and Cycles) from the
/// headers alone: name, metadata key, channels and the parsed embedded manifest. The
/// samples come from [`decode_cryptomatte`]; names hash to IDs with [`cryptomatte_id`].
pub fn cryptomatte_layers(bytes: &[u8], limits: &Limits) -> Result<Vec<CryptomatteLayer>, CodecError> {
    codecs::exr_cryptomatte::layers(bytes, limits)
}

/// Decodes the samples of the Cryptomatte layer called `layer_name` (a
/// [`CryptomatteLayer::name`]): per pixel its (ID, coverage) pairs, coverage above zero,
/// sorted by descending coverage.
pub fn decode_cryptomatte(bytes: &[u8], layer_name: &str, limits: &Limits) -> Result<CryptomatteBuffer, CodecError> {
    codecs::exr_cryptomatte::decode(bytes, layer_name, limits)
}

/// Decodes the named channels of one part (exact names as the file stores them, e.g.
/// `diffuse.R`) into an image: R/G/B (plus A when named) give Rgba, a lone Y gives Gray.
/// This renders a channel group without the part's other channels in the way.
pub fn decode_exr_channels(bytes: &[u8], part: usize, names: &[&str], opts: &DecodeOptions) -> Result<Image, CodecError> {
    let img = exr::decode_channels(bytes, part, names, &opts.limits)?;
    finish_decode(img, 1, opts)
}

/// Deep OpenEXR samples (`deepscanline`/`deeptile`): the structured per-pixel sample lists
/// (colour, alpha and Z per sample), for callers that composite deep data themselves —
/// [`decode_as_with`] instead flattens a deep file into a normal [`Image`] with a
/// [`DecodeWarning::DeepFlattened`]. `Err(Unsupported)` when the file has no deep part.
pub fn decode_deep_exr(bytes: &[u8], limits: &Limits) -> Result<DeepImage, CodecError> {
    codecs::exr::check_previews(bytes)?;
    let mut cur = std::io::Cursor::new(bytes);
    let meta = ::exr::meta::MetaData::read_from_buffered(&mut cur, false).map_err(|e| CodecError::malformed(Format::OpenExr, e))?;
    codecs::deep_exr::decode_deep(&meta, bytes, limits)
}

/// Encode `image` as `format`. Conversions follow the same plan that
/// [`fidelity_warnings_with`] reports; fatal conditions (unsupported
/// format, oversize image) return an error.
pub fn encode(image: &Image, format: Format, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    if !caps(format).write {
        return Err(CodecError::unsupported(format, "encoding is not available in this build"));
    }
    if image.width() == 0 || image.height() == 0 {
        return Err(CodecError::InvalidImage("cannot encode an empty image".into()));
    }
    if let Some((mw, mh)) = format.max_dimensions()
        && (image.width() > mw || image.height() > mh)
    {
        return Err(CodecError::encode(format, format!("dimensions exceed {mw}x{mh}")));
    }
    let plan = fidelity::plan(image, format, opts);
    match format {
        Format::Png => png::encode(image, plan, opts),
        Format::Jpeg => jpeg::encode(image, plan, opts),
        Format::Tiff => tiff::encode(image, plan, opts),
        Format::WebP => webp::encode(image, plan, opts),
        Format::Pnm => pnm::encode(image, plan, opts),
        Format::Pcx => Err(CodecError::unsupported(format, "PCX is import-only; save as PNG instead")),
        Format::Xpm => Err(CodecError::unsupported(format, "XPM is import-only; save as PNG instead")),
        Format::Dds => dds::encode(image, plan, opts),
        Format::Fits => fits::encode(image, plan, opts),
        Format::Dpx => dpx::encode(image, plan, opts),
        Format::Cineon => dpx::encode_cineon(image, plan),
        Format::Dicom => Err(CodecError::unsupported(format, "DICOM is import-only; save as TIFF or PNG instead")),
        Format::Iff => iff::encode(image, plan),
        Format::OpenExr => exr::encode(image, plan, opts),
        Format::Heif => Err(CodecError::unsupported(format, "encoding is not available in this build")),
        Format::Jxl => jxl::encode(image, plan),
        Format::Gif | Format::Bmp | Format::Tga | Format::Ico | Format::Qoi | Format::Hdr | Format::Avif => {
            via_image::encode(format, image, plan, opts)
        }
    }
}
