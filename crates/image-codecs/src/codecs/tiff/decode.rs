use std::borrow::Cow;

use super::bands::{Tally, convert_ab, decode_bands, extra_sample_indices, refill_zeroed, split_extra_samples, zeroed};
use super::tags::{F, TAG_IMAGE_SOURCE_DATA, TAG_PHOTOSHOP, TAG_XMP, TEXT_TAGS, photoshop_tags};
use super::ycbcr::YCbCr;
use super::{
    ChannelLayout, ChunkType, CodecError, DecodeWarning, Decoder, ExifDirs, File, Ifd, Image, Limits, Metadata, Read, SampleType, Seek, SeekFrom, Tag,
    camera_raw, err, ifd, io, map_err, tiff_exif,
};

/// Decodes the page Photoshop would open: the first full-resolution directory.
pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    decode_page(bytes, None, limits)
}

/// Decodes page `page` of [`ifd::tiff_info`]'s list (`None`: the default page). The image
/// carries the page's metadata, a [`DecodeWarning::MorePages`] when the file holds more
/// full-resolution pages, and a [`DecodeWarning::Truncated`] when image data is cut off.
pub(crate) fn decode_page(bytes: &[u8], page: Option<usize>, limits: &Limits) -> Result<Image, CodecError> {
    let (file, dir, more) = open_page(bytes, page)?;
    let mut img = decode_ifd(&file, &dir, limits)?;
    let extra_samples = std::mem::take(&mut img.meta.extra_samples);
    img.meta = metadata(&file, &dir, &mut img.icc);
    img.meta.extra_samples = extra_samples;
    img.warnings.extend(more);
    Ok(img)
}

/// Inspect the selected directory's sample classification without decoding any pixels.
/// Mirror document import: inspect bands first, then the default whole-image route when
/// auxiliary channels are present. Preparation reads metadata, not strips or tiles.
pub(crate) fn has_unretained_samples(bytes: &[u8], offset: u64) -> Result<bool, CodecError> {
    let file = File::parse(bytes).ok_or_else(|| err("not a TIFF or BigTIFF header"))?;
    let dir = file.ifd(offset).ok_or_else(|| err("image directory cut off"))?;
    let bands = prepare(&file, &dir, &Limits::banded(), cfg!(target_pointer_width = "32"))?;
    if bands.extras_over_limit > 0 || bands.extras_dropped > 0 {
        return Ok(true);
    }
    if bands.lay.extra_at.is_empty() {
        return Ok(false);
    }
    Ok(prepare(&file, &dir, &Limits::default(), true)?.extras_dropped > 0)
}

/// The file, page `page`'s directory (`None`: the default page) and the
/// [`DecodeWarning::MorePages`] the page opens with when the file holds more.
fn open_page(bytes: &[u8], page: Option<usize>) -> Result<(File<'_>, Ifd, Option<DecodeWarning>), CodecError> {
    if camera_raw(bytes) {
        return Err(CodecError::unsupported(F, "camera raw files (such as CR2, NEF, ARW or DNG) are not flat images; decode them with photocraft-raw"));
    }
    let file = File::parse(bytes).ok_or_else(|| err("not a TIFF or BigTIFF header"))?;
    let info = ifd::info_of(&file)?;
    let index = match page {
        Some(i) => i,
        None => info.default_page().ok_or_else(|| err("no image directory"))?,
    };
    let p = info
        .pages
        .get(index)
        .ok_or_else(|| CodecError::InvalidImage(format!("TIFF page {index} does not exist (the file has {} directories)", info.pages.len())))?;
    let dir = file.ifd(p.ifd_offset).ok_or_else(|| err("image directory cut off"))?;
    let pages = info.page_count();
    let more = (pages > 1).then(|| DecodeWarning::MorePages { total: info.complete.then_some(u32::try_from(pages).unwrap_or(u32::MAX)) });
    Ok((file, dir, more))
}

/// Output bytes [`TiffBands::decode`] aims to decode at once (at least one band per worker): small
/// next to the document the rows are written into, and to the copy a caller may convert them to.
const GROUP_BYTES: usize = 64 << 20;

/// A TIFF page opened for decoding a group of bands at a time, for images too large to hold
/// twice (#8169): [`TiffBands::decode`] hands each group to its caller, who writes it into the
/// document, so only one group exists beside the document. The pixels, layout and metadata are
/// those [`decode_page`] returns; [`TiffBands::upright`] applies the orientation as
/// [`Image::oriented`] does.
pub struct TiffBands<'a> {
    b: &'a [u8],
    p: Prepared,
    /// The page's Orientation tag (1–8).
    stored: u16,
    /// The orientation the bands are turned by (1: as stored).
    turn: u16,
    /// `Limits::max_alloc`: the most one group of bands may take.
    cap: u64,
    pub icc: Option<Vec<u8>>,
    pub meta: Metadata,
    /// What the decoder noticed: more pages, and after [`TiffBands::decode`] data cut off.
    pub warnings: Vec<DecodeWarning>,
}

impl<'a> TiffBands<'a> {
    /// Opens page `page` (`None`: the page [`decode_page`] opens by default) and checks it
    /// against `limits`; on 64-bit targets its `max_alloc` bounds each group of bands, not the
    /// whole image.
    pub fn open(bytes: &'a [u8], page: Option<usize>, limits: &Limits) -> Result<Self, CodecError> {
        let (file, dir, more) = open_page(bytes, page)?;
        // On 32-bit targets the whole image must still fit `max_alloc`, as it did before: the
        // document holding it shares the same small address space (wasm32: 4 GiB).
        let p = prepare(&file, &dir, limits, cfg!(target_pointer_width = "32"))?;
        let band = p.band_bytes()?;
        if band as u64 > limits.max_alloc {
            return Err(CodecError::LimitExceeded(format!(
                "one strip or row of tiles of this {}x{} image decodes to {band} bytes, over the {} byte limit",
                p.w, p.h, limits.max_alloc
            )));
        }
        let mut icc = None;
        let meta = metadata(&file, &dir, &mut icc);
        let stored = ifd::orientation_of(&file, &dir);
        // Extra samples are dropped here only on 32-bit targets, where the whole image counts.
        let mut warnings = prepared_warnings(&p);
        warnings.extend(more);
        Ok(TiffBands { b: bytes, p, stored, turn: 1, cap: limits.max_alloc, icc, meta, warnings })
    }

    /// The bands turned upright for the page's Orientation tag, as [`Image::oriented`] turns a
    /// whole image: the size and resolution swap for 5–8, and the EXIF and XMP orientation are
    /// rewritten to 1.
    pub fn upright(mut self) -> Self {
        let o = self.stored;
        if !(2..=8).contains(&o) || self.turn == o {
            return self;
        }
        self.turn = o;
        if let Some(e) = &mut self.meta.exif
            && let Cow::Owned(fixed) = crate::orientation::upright_exif(e)
        {
            *e = fixed;
        }
        if let Some(x) = &mut self.meta.xmp
            && let Cow::Owned(fixed) = crate::orientation::upright_xmp(x)
        {
            *x = fixed;
        }
        if o >= 5 {
            self.meta.dpi = self.meta.dpi.map(|(x, y)| (y, x));
        }
        self
    }

    /// The page's Orientation tag (1–8; 1 when absent or invalid).
    pub fn orientation(&self) -> u16 {
        self.stored
    }

    /// The size of the image the bands make up (upright when [`TiffBands::upright`] was applied).
    pub fn dimensions(&self) -> (u32, u32) {
        if self.turn >= 5 { (self.p.h, self.p.w) } else { (self.p.w, self.p.h) }
    }

    pub fn layout(&self) -> ChannelLayout {
        self.p.lay.out_layout
    }

    /// How many samples of a pixel are neither colour nor the alpha channel: alpha and spot
    /// channels that [`TiffBands::decode`] does not read (a whole-image decode does).
    pub fn extra_samples(&self) -> usize {
        self.p.lay.extra_at.len()
    }

    pub fn sample_type(&self) -> SampleType {
        self.p.lay.out
    }

    /// Bands decoded at once: about [`GROUP_BYTES`], at least one per worker, at most `max_alloc`.
    fn bands_per_group(&self) -> Result<usize, CodecError> {
        let band = self.p.band_bytes()?.max(1);
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads();
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        let cap = usize::try_from(self.cap).unwrap_or(usize::MAX);
        Ok((GROUP_BYTES / band).max(workers).min(cap / band).max(1))
    }

    /// Bytes of the largest group [`TiffBands::decode`] hands over: what decoding needs beside
    /// the document, for a caller checking that it fits in memory. A file stored as one strip
    /// is one group, the whole image.
    pub fn group_bytes(&self) -> Result<usize, CodecError> {
        let rows = self.bands_per_group()?.saturating_mul(self.p.band_rows).min(self.p.h as usize);
        self.p.lay.out_row().checked_mul(rows).ok_or_else(|| CodecError::LimitExceeded("band size".into()))
    }

    /// A conservative bound on the decoder's own scratch memory in a whole-image decode of this
    /// page (as `decode_page` does it): one strip or tile per worker that decodes a band at
    /// once. Zero when strips are decoded in place. Add the decoded image itself and any
    /// extra-sample planes.
    pub fn decode_scratch_bytes(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads() as u64;
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        scratch_bound(self.p.lay.direct(), self.p.scratch_bytes as u64, self.p.band_rows, self.p.h, workers)
    }

    /// Decodes the image a group of bands at a time, in order, handing each group to `sink`
    /// with the position of its top-left pixel in the image [`TiffBands::dimensions`] gives. A
    /// group is about 64 MiB (at least one strip or row of tiles per worker, at most `max_alloc`;
    /// see [`TiffBands::group_bytes`]). Data cut off by the end of the file adds a
    /// [`DecodeWarning::Truncated`] to `warnings`.
    pub fn decode<E: From<CodecError>>(&mut self, mut sink: impl FnMut(u32, u32, &Image) -> Result<(), E>) -> Result<(), E> {
        let per_group = self.bands_per_group()?;
        let p = &self.p;
        let band = p.band_bytes()?.max(1);
        let bands = (p.h as usize).div_ceil(p.band_rows.max(1));
        let mut buf: Vec<u8> = Vec::new();
        let mut tally = Tally::default();
        let mut first = 0;
        while first < bands {
            let n = per_group.min(bands - first);
            let y0 = first * p.band_rows;
            let rows = (n * p.band_rows).min(p.h as usize - y0);
            let len = p.lay.out_row().checked_mul(rows).ok_or_else(|| CodecError::LimitExceeded("band size".into()))?;
            // Missing chunks leave their rows as they are: they must start empty in every group.
            buf = refill_zeroed(buf, len)?;
            tally = tally + decode_bands(p, self.b, &mut buf, band, first, &mut [], 0)?;
            if let Some(c) = p.ycbcr {
                c.convert(&mut buf, p.lay.out_channels(), p.lay.out);
            }
            if let Some(signed) = p.lab {
                convert_ab(&mut buf, p.lay.out_channels(), p.lay.out, signed, false);
            }
            let rows = u32::try_from(rows).map_err(|_| err("band too tall"))?;
            let y0 = u32::try_from(y0).map_err(|_| err("band out of range"))?;
            let group = Image::from_raw(p.w, rows, p.lay.out_layout, p.lay.out, std::mem::take(&mut buf))?;
            let (x, y) = match self.turn {
                3 | 4 => (0, p.h.saturating_sub(y0 + rows)),
                5 | 8 => (y0, 0),
                6 | 7 => (p.h.saturating_sub(y0 + rows), 0),
                _ => (0, y0),
            };
            let group = group.oriented(self.turn)?;
            sink(x, y, &group)?;
            buf = group.into_data();
            first += n;
        }
        if tally.decoded == 0 && tally.missing > 0 {
            return Err(err("the image data is missing (cut off or past the end of the file)").into());
        }
        if tally.missing > 0 {
            // After the extra-sample warnings and before `MorePages`, as the whole-image decode
            // orders them.
            let at = self.warnings.iter().position(|w| matches!(w, DecodeWarning::MorePages { .. })).unwrap_or(self.warnings.len());
            self.warnings.insert(at, DecodeWarning::Truncated { format: F });
        }
        Ok(())
    }
}

/// [`TiffBands::decode_scratch_bytes`]'s bound: none when chunks are decoded in place,
/// otherwise one chunk per worker that decodes a band, and never more chunks than the image has.
fn scratch_bound(direct: bool, chunk_bytes: u64, band_rows: usize, h: u32, workers: u64) -> u64 {
    if direct {
        return 0;
    }
    let bands = u64::from(h).div_ceil(band_rows.max(1) as u64).max(1);
    chunk_bytes.saturating_mul(workers.min(bands))
}

/// The Orientation (1–8) of page `page` (`None`: the default page); 1 when absent, malformed
/// or out of range. Classic TIFF and BigTIFF.
pub(crate) fn orientation(bytes: &[u8], page: Option<usize>) -> u16 {
    let Some(file) = File::parse(bytes) else { return 1 };
    let Ok(info) = ifd::info_of(&file) else { return 1 };
    page.or_else(|| info.default_page()).and_then(|i| info.pages.get(i)).and_then(|p| file.ifd(p.ifd_offset)).map_or(1, |d| ifd::orientation_of(&file, &d))
}

/// The page's metadata; the ICC profile goes to `icc`.
fn metadata(f: &File<'_>, dir: &Ifd, icc: &mut Option<Vec<u8>>) -> Metadata {
    *icc = f.tag_bytes(dir, Tag::IccProfile.to_u16()).filter(|b| !b.is_empty()).map(<[u8]>::to_vec);
    let text = |tag: u16| {
        let b = f.tag_bytes(dir, tag)?;
        let s = std::str::from_utf8(b).ok()?.trim_end_matches('\0');
        (!s.is_empty()).then(|| s.to_owned())
    };
    let mut meta = Metadata { xmp: text(TAG_XMP), ..Default::default() };
    let unit = f.tag_uint(dir, Tag::ResolutionUnit.to_u16()).unwrap_or(2);
    if let (Some(x), Some(y)) = (f.tag_f64(dir, Tag::XResolution.to_u16()), f.tag_f64(dir, Tag::YResolution.to_u16()))
        && x > 0.0
        && y > 0.0
        && x.is_finite()
        && y.is_finite()
    {
        meta.dpi = match unit {
            2 => Some((x as f32, y as f32)),
            3 => Some(((x * 2.54) as f32, (y * 2.54) as f32)),
            _ => None,
        };
    }
    for (tag, key) in TEXT_TAGS {
        if let Some(s) = text(tag.to_u16()) {
            meta.text.push(((*key).to_owned(), s));
        }
    }
    // Photoshop keeps a layered TIFF's layers and resources in IFD0, where `photoshop_tags`
    // reads them; any other page reads its own directory's copies.
    let (resources, layers) =
        if dir.at == f.first_ifd { photoshop_tags(f.b) } else { (f.tag_bytes(dir, TAG_PHOTOSHOP), f.tag_bytes(dir, TAG_IMAGE_SOURCE_DATA)) };
    meta.photoshop_resources = resources.map(<[u8]>::to_vec);
    meta.photoshop_layers = layers.map(<[u8]>::to_vec);
    // EXIF: the Exif and GPS directories IFD0 points to, with IFD0's descriptive tags, as an
    // EXIF block (the other pages are reduced copies or layers of the same image).
    if dir.at == f.first_ifd
        && let Some(e) = tiff_exif::read(f.b).filter(ExifDirs::has_directories)
    {
        meta.exif = Some(tiff_exif::block(&e));
    }
    meta
}

// ---------------------------------------------------------------------------
// Banded pixel decoding
// ---------------------------------------------------------------------------

/// The file seen with a few byte ranges replaced: the header's first-IFD offset (so the `tiff`
/// decoder opens the chosen directory as its IFD0) and, for WhiteIsZero and palette images,
/// the PhotometricInterpretation value (so the decoder hands over the stored samples and this
/// module applies the interpretation itself). Reads past the end return no bytes.
#[derive(Clone, Copy)]
pub(super) struct Patched<'a> {
    b: &'a [u8],
    patches: &'a [(u64, Vec<u8>)],
    pos: u64,
}

impl Read for Patched<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(src) = usize::try_from(self.pos).ok().and_then(|p| self.b.get(p..)) else { return Ok(0) };
        let n = out.len().min(src.len());
        let (Some(o), Some(s)) = (out.get_mut(..n), src.get(..n)) else { return Ok(0) };
        o.copy_from_slice(s);
        let end = self.pos.saturating_add(n as u64);
        for (at, bytes) in self.patches {
            let lo = (*at).max(self.pos);
            let hi = at.saturating_add(bytes.len() as u64).min(end);
            for i in lo..hi {
                let (Ok(oi), Ok(pi)) = (usize::try_from(i - self.pos), usize::try_from(i - at)) else { continue };
                if let (Some(d), Some(&v)) = (o.get_mut(oi), bytes.get(pi)) {
                    *d = v;
                }
            }
        }
        self.pos = end;
        Ok(n)
    }
}

impl Seek for Patched<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => (self.b.len() as u64).checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let new = new.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        self.pos = new;
        Ok(new)
    }
}

/// How a stored sample is laid out in the `tiff` crate's chunk buffer (native endian).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Src {
    /// 1, 2 or 4-bit unsigned, packed MSB first, rows padded to a byte.
    Bits(u8),
    U8,
    U16,
    U32,
    U64,
    F16,
    F32,
    F64,
}

impl Src {
    pub(super) fn bytes(self) -> usize {
        match self {
            Src::Bits(_) | Src::U8 => 1,
            Src::U16 | Src::F16 => 2,
            Src::U32 | Src::F32 => 4,
            Src::U64 | Src::F64 => 8,
        }
    }

    /// The output sample type: integers above 16 bits keep their top 16, doubles become f32.
    fn out(self) -> SampleType {
        match self {
            Src::Bits(_) | Src::U8 => SampleType::U8,
            Src::U16 | Src::U32 | Src::U64 => SampleType::U16,
            Src::F16 => SampleType::F16,
            Src::F32 | Src::F64 => SampleType::F32,
        }
    }

    /// Stored bytes and output bytes are the same: a plain copy.
    pub(super) fn is_identity(self) -> bool {
        matches!(self, Src::U8 | Src::U16 | Src::F16 | Src::F32)
    }
}

/// Everything needed to decode and place the chunks of one directory.
pub(super) struct Layout {
    pub(super) w: usize,
    pub(super) src: Src,
    pub(super) out: SampleType,
    pub(super) out_layout: ChannelLayout,
    /// Samples per pixel in a decoded chunk (1 for planar data).
    pub(super) chunk_spp: usize,
    pub(super) planar: bool,
    /// Planes decoded for planar data: one per output channel (extra planes are skipped).
    pub(super) planes: usize,
    /// Tile width (a strip is `w` wide).
    pub(super) cw: usize,
    pub(super) across: usize,
    pub(super) chunks_per_plane: usize,
    /// WhiteIsZero: invert the gray channel.
    pub(super) invert: bool,
    /// The decoded sample (chunky) or file plane (planar) of the alpha channel (when there is one).
    alpha_sample: usize,
    /// Associated (premultiplied) alpha to turn straight.
    pub(super) premultiplied: bool,
    /// Palette images: the colour map (16-bit RGB per index).
    pub(super) palette: Option<Vec<[u16; 3]>>,
    /// Upper bound for one chunk's scratch buffer.
    pub(super) scratch_cap: usize,
    /// File sample indices that are neither colour nor the chosen alpha (spot / extra alpha).
    pub(super) extra_at: Vec<usize>,
}

impl Layout {
    pub(super) fn out_channels(&self) -> usize {
        self.out_layout.channels()
    }
    /// The decoded sample (chunky) or file plane (planar) of output channel `k`.
    pub(super) fn source_sample(&self, k: usize) -> usize {
        if self.out_layout.has_alpha() && k + 1 == self.out_channels() { self.alpha_sample } else { k }
    }
    pub(super) fn out_row(&self) -> usize {
        self.w * self.out_channels() * self.out.bytes()
    }
    pub(super) fn extra_row(&self) -> usize {
        self.w.saturating_mul(self.extra_at.len()).saturating_mul(self.out.bytes())
    }
    /// A chunky strip already in the output's layout: decoded in place.
    pub(super) fn direct(&self) -> bool {
        self.copyable() && self.across == 1
    }
    /// Chunk rows are output rows: copied without conversion.
    pub(super) fn copyable(&self) -> bool {
        !self.planar && self.palette.is_none() && self.src.is_identity() && self.chunk_spp == self.out_channels()
    }
}

/// A chunk read that hit the end of the file (the data is missing, not malformed).
pub(super) fn is_eof(e: &tiff::TiffError) -> bool {
    matches!(e, tiff::TiffError::IoError(io) if io.kind() == io::ErrorKind::UnexpectedEof)
}

/// The sample type of a (SampleFormat, BitsPerSample) pair.
fn sample_src(format: u64, bits: u64) -> Result<Src, CodecError> {
    Ok(match (format, bits) {
        (1, 1 | 2 | 4) => Src::Bits(bits as u8),
        (1, 8) => Src::U8,
        (1, 16) => Src::U16,
        (1, 32) => Src::U32,
        (1, 64) => Src::U64,
        (3, 16) => Src::F16,
        (3, 32) => Src::F32,
        (3, 64) => Src::F64,
        (2, _) => return Err(CodecError::unsupported(F, "signed integer samples")),
        _ => return Err(CodecError::unsupported(F, format!("{bits}-bit samples (sample format {format})"))),
    })
}

/// The ColorMap of a palette image as RGB entries.
fn read_palette(f: &File<'_>, dir: &Ifd, spp: u64, bits: u64, src: Src) -> Result<Vec<[u16; 3]>, CodecError> {
    if spp != 1 || !matches!(src, Src::Bits(_) | Src::U8 | Src::U16) {
        return Err(CodecError::unsupported(F, format!("palette image with {spp} samples of {bits} bits")));
    }
    let n = 1u64 << bits;
    let map = dir.get(ifd::TAG_COLOR_MAP).filter(|e| e.ty == 3 && e.count == 3 * n).ok_or_else(|| err("palette image without a valid ColorMap"))?;
    let at = |i: u64| f.uint(map, i).and_then(|v| u16::try_from(v).ok());
    let table: Option<Vec<[u16; 3]>> = (0..n).map(|i| Some([at(i)?, at(n + i)?, at(2 * n + i)?])).collect();
    table.ok_or_else(|| err("palette ColorMap cut off"))
}

/// The layout of a multiband read that was asked for every sample.
fn multiband_layout(photometric: u64, has_alpha: bool, gray_alpha: bool) -> ChannelLayout {
    match (photometric, has_alpha) {
        (2 | 8 | 9, true) => ChannelLayout::Rgba,
        (2 | 8 | 9, false) => ChannelLayout::Rgb,
        (5, true) => ChannelLayout::CmykA,
        (5, false) => ChannelLayout::Cmyk,
        _ if gray_alpha => ChannelLayout::GrayA,
        _ => ChannelLayout::Gray,
    }
}

/// The chunk tables must match the geometry. Checked in 64 bits, before the `tiff` crate does
/// the same arithmetic in narrower types.
fn check_chunk_tables(f: &File<'_>, dir: &Ifd, (w, h): (u32, u32), spp: u64, planar: bool, tiled: bool) -> Result<(u64, u64, u64), CodecError> {
    let need = |tag: u16, name: &str| f.tag_uint(dir, tag).ok_or_else(|| err(format!("missing or unreadable {name}")));
    let (cw, ch) = if tiled {
        let tw = need(ifd::TAG_TILE_WIDTH, "TileWidth")?;
        let tl = need(ifd::TAG_TILE_LENGTH, "TileLength")?;
        (tw, tl)
    } else {
        (u64::from(w), f.tag_uint(dir, ifd::TAG_ROWS_PER_STRIP).unwrap_or(u64::from(h)).min(u64::from(h)))
    };
    if cw == 0 || ch == 0 {
        return Err(err("zero tile or strip size"));
    }
    // The chunk tables must match the geometry. Checked here, in 64 bits, before the `tiff`
    // crate does the same arithmetic in narrower types.
    let planes_in_file = if planar { spp } else { 1 };
    let expected = u64::from(w)
        .div_ceil(cw)
        .checked_mul(u64::from(h).div_ceil(ch))
        .and_then(|n| n.checked_mul(planes_in_file))
        .filter(|&n| n <= u64::from(u32::MAX))
        .ok_or_else(|| err("too many strips or tiles"))?;
    let (offsets_tag, counts_tag) = if tiled { (324, 325) } else { (273, 279) };
    let counts = (dir.get(offsets_tag).map(|e| e.count), dir.get(counts_tag).map(|e| e.count));
    if counts != (Some(expected), Some(expected)) {
        return Err(err(format!("{expected} strips or tiles expected, the tables list {counts:?}")));
    }
    Ok((cw, ch, expected))
}

/// Byte patches the `tiff` crate reads through: the directory is seen as the last of its chain,
/// and PhotometricInterpretation is handed over as black-is-zero (or RGB for Lab) when asked to.
fn header_patches(
    f: &File<'_>,
    dir: &Ifd,
    photometric_entry: Option<ifd::Entry>,
    patch_photometric: bool,
    lab_only: bool,
) -> Result<Vec<(u64, Vec<u8>)>, CodecError> {
    let mut patches: Vec<(u64, Vec<u8>)> = Vec::new();
    let encode_u = |v: u64, len: usize| -> Vec<u8> {
        let b = if f.le { v.to_le_bytes() } else { v.to_be_bytes() };
        if f.le { b.get(..len).map(<[u8]>::to_vec).unwrap_or_default() } else { b.get(8 - len..).map(<[u8]>::to_vec).unwrap_or_default() }
    };
    if f.big {
        patches.push((8, encode_u(dir.at, 8)));
    } else {
        let at = u32::try_from(dir.at).map_err(|_| err("directory offset out of range"))?;
        patches.push((4, encode_u(u64::from(at), 4)));
    }
    // The directory is seen as the last of its chain: the chain was walked already, and a
    // cycle through it must not fail this page.
    patches.push((dir.next_at, vec![0; if f.big { 8 } else { 4 }]));
    if patch_photometric {
        let e = photometric_entry.ok_or_else(|| err("missing PhotometricInterpretation"))?;
        let len = match e.ty {
            3 if e.count == 1 => 2,
            4 if e.count == 1 => 4,
            _ => return Err(err("malformed PhotometricInterpretation")),
        };
        patches.push((e.value_at, encode_u(if lab_only { 2 } else { 1 }, len)));
    }
    Ok(patches)
}

/// Lab and YCbCr are converted after decoding, from 8 or 16-bit samples only: Lab needs at least
/// its three colour samples, YCbCr exactly three.
fn check_converted_model(photometric: u64, spp: u64, bits: u64, src: Src) -> Result<(), CodecError> {
    let (name, samples_ok) = match photometric {
        8 | 9 => ("Lab", spp >= 3),
        6 => ("YCbCr", spp == 3),
        _ => return Ok(()),
    };
    if samples_ok && matches!(src, Src::U8 | Src::U16) {
        return Ok(());
    }
    Err(CodecError::unsupported(F, format!("{name} image with {spp} samples of {bits} bits")))
}

/// A directory checked and laid out for decoding, before any pixel is read.
pub(super) struct Prepared {
    w: u32,
    h: u32,
    pub(super) lay: Layout,
    /// Rows per band: one strip, or one row of tiles.
    band_rows: usize,
    patches: Vec<(u64, Vec<u8>)>,
    tl: tiff::decoder::Limits,
    /// Lab samples: `Some(true)` for CIELab (a* and b* stored signed), `Some(false)` for ICCLab.
    lab: Option<bool>,
    /// YCbCr samples: the conversion to RGB (#10316).
    ycbcr: Option<YCbCr>,
    /// The colour samples of a pixel in the file's colour model.
    color: usize,
    /// The ExtraSamples tag (0 unspecified, 1 associated alpha, 2 unassociated alpha).
    extra_kinds: Vec<u64>,
    /// Extra samples past `MAX_EXTRA_SAMPLES`, left out.
    extras_over_limit: usize,
    /// Extra samples left out because they did not fit the allocation budget beside the image.
    extras_dropped: u32,
    /// Bytes one strip or tile decodes to, before clipping to the image: a worker's scratch
    /// buffer. A strip is the full width; a tile keeps its declared size even at an edge.
    scratch_bytes: usize,
}

impl Prepared {
    /// A decoder over `b` (the file the directory was read from) that opens the directory.
    pub(super) fn open<'a>(&'a self, b: &'a [u8]) -> Result<Decoder<Patched<'a>>, CodecError> {
        Decoder::new(Patched { b, patches: &self.patches, pos: 0 }).map(|d| d.with_limits(self.tl.clone())).map_err(map_err)
    }

    /// Output bytes of one full band.
    fn band_bytes(&self) -> Result<usize, CodecError> {
        self.lay.out_row().checked_mul(self.band_rows).ok_or_else(|| CodecError::LimitExceeded("band size".into()))
    }
}

/// Decodes the pixels of one directory.
fn decode_ifd(f: &File<'_>, dir: &Ifd, limits: &Limits) -> Result<Image, CodecError> {
    let p = prepare(f, dir, limits, true)?;
    // The output, allocated once and fallibly.
    let len = p.lay.out_row().checked_mul(p.h as usize).ok_or_else(|| CodecError::LimitExceeded("image size".into()))?;
    let mut data = zeroed(len)?;
    let extra_row = p.lay.extra_row();
    let extra_len = extra_row.checked_mul(p.h as usize).ok_or_else(|| CodecError::LimitExceeded("extra samples".into()))?;
    let mut extra_data = if p.lay.extra_at.is_empty() { Vec::new() } else { zeroed(extra_len)? };
    let extra_band = extra_row.saturating_mul(p.band_rows);
    let tally = decode_bands(&p, f.b, &mut data, p.band_bytes()?, 0, extra_data.as_mut_slice(), extra_band)?;
    if tally.decoded == 0 && tally.missing > 0 {
        return Err(err("the image data is missing (cut off or past the end of the file)"));
    }
    if let Some(c) = p.ycbcr {
        c.convert(&mut data, p.lay.out_channels(), p.lay.out);
    }
    if let Some(signed) = p.lab {
        convert_ab(&mut data, p.lay.out_channels(), p.lay.out, signed, false);
    }
    let mut img = Image::from_raw(p.w, p.h, p.lay.out_layout, p.lay.out, data)?;
    if !p.lay.extra_at.is_empty() {
        // TIFF 6.0 defines ExtraSamples 0, 1 and 2 only; a reserved value reads as unspecified,
        // so a save never writes it back.
        let kind_of = |si: usize| match si.checked_sub(p.color).and_then(|k| p.extra_kinds.get(k)).copied() {
            Some(v @ 0..=2) => v as u16,
            _ => 0,
        };
        img.meta.extra_samples = split_extra_samples(&extra_data, p.w as usize, p.h as usize, &p.lay.extra_at, p.lay.out, kind_of)?;
    }
    img.warnings.extend(prepared_warnings(&p));
    if tally.missing > 0 {
        img.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(img)
}

/// What [`prepare`] left out of the image: extra samples past the supported count, then extra
/// samples over the allocation budget. Both decoders report them, in this order.
fn prepared_warnings(p: &Prepared) -> Vec<DecodeWarning> {
    let mut warnings = Vec::new();
    if p.extras_over_limit > 0 {
        let count = u32::try_from(p.extras_over_limit).unwrap_or(u32::MAX);
        warnings.push(DecodeWarning::ExtraSamplesOverLimit { format: F, count });
    }
    if p.extras_dropped > 0 {
        warnings.push(DecodeWarning::ExtraSamplesDropped { format: F, count: p.extras_dropped });
    }
    warnings
}

/// Checks one directory and lays it out for decoding. `whole`: the image is decoded into one
/// buffer, which `limits.max_alloc` bounds; otherwise only its size is checked here.
fn prepare(f: &File<'_>, dir: &Ifd, limits: &Limits, whole: bool) -> Result<Prepared, CodecError> {
    // The values the `tiff` crate reads must exist: a count past the end of the file is
    // neither allocated nor read.
    if let Some(e) = dir.entries.iter().find(|e| ifd::STRUCTURAL_TAGS.contains(&e.tag) && f.data(e).is_none()) {
        return Err(err(format!("tag {} points past the end of the file", e.tag)));
    }
    let need = |tag: u16, name: &str| f.tag_uint(dir, tag).ok_or_else(|| err(format!("missing or unreadable {name}")));
    let w = u32::try_from(need(ifd::TAG_WIDTH, "ImageWidth")?).map_err(|_| err("ImageWidth out of range"))?;
    let h = u32::try_from(need(ifd::TAG_HEIGHT, "ImageLength")?).map_err(|_| err("ImageLength out of range"))?;
    if w == 0 || h == 0 {
        return Err(CodecError::InvalidImage(format!("zero-sized image {w}x{h}")));
    }
    let spp = f.tag_uint(dir, ifd::TAG_SAMPLES).unwrap_or(1);
    if spp == 0 || spp > 64 {
        return Err(CodecError::unsupported(F, format!("{spp} samples per pixel")));
    }
    let bits_all = f.tag_uints(dir, ifd::TAG_BITS, spp).unwrap_or_else(|| vec![1]);
    let bits = bits_all.first().copied().unwrap_or(1);
    if bits_all.iter().any(|&b| b != bits) {
        return Err(CodecError::unsupported(F, format!("mixed bits per sample {bits_all:?}")));
    }
    let format = f.tag_uint(dir, ifd::TAG_SAMPLE_FORMAT).unwrap_or(1);
    let photometric_entry = dir.get(ifd::TAG_PHOTOMETRIC).copied();
    let photometric = f.tag_uint(dir, ifd::TAG_PHOTOMETRIC).ok_or_else(|| CodecError::unsupported(F, "missing PhotometricInterpretation"))?;
    let planar = f.tag_uint(dir, ifd::TAG_PLANAR) == Some(2);
    let extra = f.tag_uints(dir, ifd::TAG_EXTRA_SAMPLES, 64).unwrap_or_default();
    let tiled = dir.get(ifd::TAG_TILE_OFFSETS).is_some();
    let (cw, ch, expected) = check_chunk_tables(f, dir, (w, h), spp, planar, tiled)?;
    let (offsets_tag, counts_tag) = if tiled { (324, 325) } else { (273, 279) };
    let src = sample_src(format, bits)?;
    // WhiteIsZero and palette images are handed to the `tiff` crate as BlackIsZero: it then
    // returns the stored samples, and the interpretation is applied here (for every depth and
    // with alpha, which the crate's own inversion does not cover).
    let invert = photometric == 0;
    let palette = if photometric == 3 { Some(read_palette(f, dir, spp, bits, src)?) } else { None };
    // CIE L*a*b*: CIELab (8; a* and b* signed) and ICCLab (9; a* and b* offset) are handed to the
    // `tiff` crate as RGB, then labelled Lab with a* and b* in PhotoCraft's encoding
    // (`convert_ab`).
    let lab = matches!(photometric, 8 | 9);
    // YCbCr (6), as scanners and cameras write it (usually JPEG-compressed with subsampled
    // chroma): the `tiff` crate decodes and upsamples the samples, and they are turned into RGB
    // after decoding (#10316).
    let ycbcr = photometric == 6;
    check_converted_model(photometric, spp, bits, src)?;
    // TIFF 6.0 lets any extra sample be the alpha (#1349). For RGB and CMYK the `tiff` crate
    // returns the alpha only when it is the first extra sample: otherwise it is asked for every
    // sample (as multiband gray) and the colour and alpha samples are picked here. An alpha
    // position past the samples the file has is ignored.
    let color: usize = match photometric {
        2 | 6 | 8 | 9 => 3,
        5 => 4,
        _ => 1,
    };
    let alpha_at = extra.iter().position(|&e| e == 1 || e == 2).filter(|&a| (color as u64).saturating_add(a as u64) < spp);
    // Samples the colour model does not keep. RGB, CMYK and Lab only hand back every sample when
    // asked (photometric patched to black-is-zero below); gray already comes back as multiband.
    let unused = !extra.is_empty() && (spp as usize) > color.saturating_add(usize::from(alpha_at.is_some()));
    let all_samples = palette.is_none() && matches!(photometric, 2 | 5 | 8 | 9) && (alpha_at.is_some_and(|a| a > 0) || (!planar && unused));
    let patches = header_patches(f, dir, photometric_entry, invert || palette.is_some() || all_samples || lab, lab && !all_samples)?;

    let tl = {
        let mut l = tiff::decoder::Limits::default();
        l.decoding_buffer_size = limits.alloc_usize();
        // No compressed chunk can be larger than the file.
        l.intermediate_buffer_size = f.b.len().max(1);
        l
    };
    let mut dec = Decoder::new(Patched { b: f.b, patches: &patches, pos: 0 }).map(|d| d.with_limits(tl.clone())).map_err(map_err)?;
    if dec.dimensions().map_err(map_err)? != (w, h) || (dec.get_chunk_type() == ChunkType::Tile) != tiled {
        return Err(err("inconsistent image directory"));
    }
    let ct = dec.colortype().map_err(map_err)?;
    let gray_alpha = alpha_at.is_some() || (extra.is_empty() && spp == 2);
    let (out_layout, chunk_spp) = match ct {
        _ if palette.is_some() => (ChannelLayout::Rgb, 1),
        tiff::ColorType::Gray(_) => (ChannelLayout::Gray, 1),
        tiff::ColorType::Multiband { num_samples, .. } if all_samples => {
            // Lab reads as RGB here and is relabelled below.
            let layout = multiband_layout(photometric, alpha_at.is_some(), gray_alpha);
            (layout, usize::from(num_samples))
        }
        tiff::ColorType::Multiband { num_samples, .. } if photometric <= 1 => {
            (if gray_alpha { ChannelLayout::GrayA } else { ChannelLayout::Gray }, usize::from(num_samples))
        }
        tiff::ColorType::RGB(_) => (ChannelLayout::Rgb, 3),
        tiff::ColorType::YCbCr(_) if ycbcr => (ChannelLayout::Rgb, 3),
        tiff::ColorType::RGBA(_) => (ChannelLayout::Rgba, 4),
        tiff::ColorType::CMYK(_) => (ChannelLayout::Cmyk, 4),
        tiff::ColorType::CMYKA(_) => (ChannelLayout::CmykA, 5),
        other => return Err(CodecError::unsupported(F, format!("colour type {other:?}"))),
    };
    let out_layout = match out_layout {
        ChannelLayout::Rgb if lab => ChannelLayout::Lab,
        ChannelLayout::Rgba if lab => ChannelLayout::LabA,
        _ if lab => return Err(err("Lab image that does not read as three colour samples")),
        l => l,
    };
    let chunk_spp = if planar { 1 } else { chunk_spp };
    // The sample (chunky) or plane (planar) that holds the alpha: the colour samples come first.
    let alpha_sample = out_layout.color_channels() + alpha_at.unwrap_or(0);
    if out_layout.has_alpha() && (alpha_sample as u64 >= spp || (!planar && alpha_sample >= chunk_spp)) {
        return Err(err("the alpha sample is past the samples of a pixel"));
    }
    if matches!(src, Src::Bits(_)) && (out_layout.channels() > 1 || spp != 1) && palette.is_none() {
        return Err(CodecError::unsupported(F, format!("{bits}-bit samples with {spp} samples per pixel")));
    }
    if (!planar && palette.is_none() && chunk_spp < out_layout.channels()) || (planar && out_layout.channels() as u64 > spp) {
        return Err(err("fewer samples than the colour type needs"));
    }
    let out = match (&palette, src) {
        (Some(_), Src::U16) => SampleType::U16,
        (Some(_), _) => SampleType::U8,
        _ => src.out(),
    };
    if whole {
        limits.check(w, h, out_layout, out)?;
    } else {
        limits.check_size(w, h)?;
    }
    // Decompression-bomb guard: no compression method expands data by more than a few thousand
    // times (Deflate at most ~1032:1, LZW ~2730:1, PackBits 128:1), so a file far too small for
    // its declared size is refused before the image is allocated. CCITT Group 4 codes a blank row
    // of a strip or tile in one bit (a V0 code), so its bound is the chunk width (the decoder
    // reads at most 65535): a blank landscape A4 page at 600 dpi, 7016 px wide, compresses past
    // 4096:1 (Groups 3 and 2 are not decoded). Uncompressed data may be up
    // to three quarters missing (that decodes with a `Truncated` warning). Only the bytes the
    // strip or tile tables point at inside the file count (and never more than the file), so
    // padding the file while pointing the chunks past its end does not raise the bound: the
    // banded decoder has no whole-image `max_alloc` to fall back on.
    let compression = f.tag_uint(dir, ifd::TAG_COMPRESSION).unwrap_or(1);
    let stored = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(spp).saturating_mul(bits).div_ceil(8);
    let ratio: u64 = match compression {
        1 => 4,
        32773 => 130,
        4 => cw.clamp(4096, 65_535),
        _ => 4096,
    };
    let len = f.b.len() as u64;
    let (offsets, sizes) = (
        dir.get(offsets_tag).ok_or_else(|| err("missing strip or tile offsets"))?,
        dir.get(counts_tag).ok_or_else(|| err("missing strip or tile byte counts"))?,
    );
    let mut held = 0u64;
    for i in 0..expected {
        if held >= len {
            break;
        }
        if let (Some(at), Some(n)) = (f.uint(offsets, i), f.uint(sizes, i)) {
            held = held.saturating_add(n.min(len.saturating_sub(at)));
        }
    }
    let held = held.min(len);
    if stored > held.saturating_mul(ratio).saturating_add(1 << 20) {
        return Err(err(format!("the strips or tiles in this file hold {held} bytes, too few for a {w}x{h} image")));
    }

    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (usize::try_from(cw).map_err(|_| err("tile too wide"))?, usize::try_from(ch).map_err(|_| err("tile too tall"))?);
    let across = wu.div_ceil(cw);
    let down = hu.div_ceil(ch);
    let chunks_per_plane = across.checked_mul(down).ok_or_else(|| err("too many tiles"))?;
    let planes_in_file = if planar { spp as usize } else { 1 };
    let total = if tiled { dec.tile_count() } else { dec.strip_count() }.map_err(map_err)?;
    drop(dec);
    if Some(total as usize) != chunks_per_plane.checked_mul(planes_in_file) {
        return Err(err("strip or tile count does not match the image size"));
    }
    // One chunk's decoded bytes, clipped to the image: the scratch buffer may hold a little
    // more (the `tiff` crate reads whole tiles in some planar cases) but never an amount that
    // only a hostile tile size could ask for.
    let chunk_bytes = cw
        .min(wu)
        .checked_mul(ch.min(hu))
        .and_then(|px| px.checked_mul(chunk_spp))
        .and_then(|s| s.checked_mul(src.bytes()))
        .ok_or_else(|| CodecError::LimitExceeded("tile size".into()))?;
    let scratch_cap = chunk_bytes.saturating_mul(2).max(64 << 20).min(limits.alloc_usize());
    // One chunk as the decoder actually reads it: a tile keeps its declared size at an edge,
    // which can be more than the clipped image bytes above.
    let scratch_bytes = cw.saturating_mul(ch).saturating_mul(chunk_spp).saturating_mul(src.bytes());
    let (extra_at, extras_over_limit) = extra_sample_indices(spp as usize, color, alpha_at, out_layout.has_alpha());
    let mut lay = Layout {
        w: wu,
        src,
        out,
        out_layout,
        chunk_spp,
        planar,
        planes: if planar { out_layout.channels() } else { 1 },
        cw,
        across,
        chunks_per_plane,
        invert: invert && palette.is_none(),
        alpha_sample,
        premultiplied: out_layout.has_alpha() && alpha_at.and_then(|a| extra.get(a)) == Some(&1),
        palette,
        scratch_cap,
        extra_at,
    };
    // Extra samples share the allocation budget with the image: their interleaved buffer and the
    // planes split from it are held next to the image, so both count against `max_alloc`. When
    // they don't fit, the image still opens (it passed its own check above) without them.
    let mut extras_dropped = 0;
    if whole && !lay.extra_at.is_empty() {
        let per_pixel = out_layout.channels().saturating_add(lay.extra_at.len().saturating_mul(2)).saturating_mul(out.bytes());
        if limits.check_bytes(w, h, per_pixel as u64).is_err() {
            extras_dropped = u32::try_from(lay.extra_at.len()).unwrap_or(u32::MAX);
            lay.extra_at.clear();
        }
    }
    Ok(Prepared {
        w,
        h,
        lay,
        band_rows: ch.min(hu),
        patches,
        tl,
        lab: lab.then_some(photometric == 8),
        ycbcr: ycbcr.then(|| YCbCr::of(f, dir, compression, if out == SampleType::U16 { 65535.0 } else { 255.0 })),
        color,
        extra_kinds: extra,
        extras_over_limit,
        extras_dropped,
        scratch_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncodeOptions, ExtraSample, Format};

    /// `TiffBands::open` drops extra samples over the budget only on 32-bit targets (wasm32),
    /// so it reports them through the same function as the whole-image decode.
    #[test]
    fn extra_samples_over_the_budget_are_reported_by_both_decoders() {
        // Several strips (about 1 MB each), so one strip of all four samples fits the image's budget.
        let (w, h) = (1024u32, 1024u32);
        let n = (w * h) as usize;
        let mut img = Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, vec![90; 3 * n]).unwrap();
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: vec![7; n] }];
        let bytes = crate::encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
        let (file, dir, _) = open_page(&bytes, None).unwrap();
        let tight = Limits { max_alloc: 3 * n as u64, ..Limits::default() };
        let p = prepare(&file, &dir, &tight, true).unwrap();
        assert_eq!(prepared_warnings(&p), vec![DecodeWarning::ExtraSamplesDropped { format: F, count: 1 }]);
        assert!(prepared_warnings(&prepare(&file, &dir, &tight, false).unwrap()).is_empty(), "banded: only the band counts");
        assert!(prepared_warnings(&prepare(&file, &dir, &Limits::default(), true).unwrap()).is_empty());
        let whole = decode(&bytes, &tight).unwrap();
        assert_eq!(whole.warnings, prepared_warnings(&p));
    }

    /// The scratch bound a whole-image decode reserves: none for chunks decoded in place, one
    /// chunk per worker otherwise, never more chunks than the image has bands.
    #[test]
    fn whole_image_scratch_is_one_chunk_per_worker() {
        assert_eq!(scratch_bound(true, 1 << 20, 16, 1024, 8), 0, "strips decoded in place need no scratch");
        assert_eq!(scratch_bound(false, 1 << 20, 16, 1024, 8), 8 << 20, "one strip per worker");
        assert_eq!(scratch_bound(false, 1 << 20, 16, 32, 8), 2 << 20, "never more strips than the image has");
        assert_eq!(scratch_bound(false, 1 << 20, 0, 2, 8), 2 << 20, "a zero band height counts as one row");
        assert_eq!(scratch_bound(false, u64::MAX, 1, 1, 8), u64::MAX, "the bound saturates");
    }
}
