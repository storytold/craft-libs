//! Bounded OpenEXR reading and writing on top of the `exr` crate (feature `decode`).
//!
//! - [`parts`] lists every part from the headers alone (size, windows, channels, compression,
//!   text attributes), after checking the declared sizes against [`Limits`] and the file's size.
//! - [`pick_part`] chooses the part a plain picture decode shows: the best colour channel group
//!   ([`crate::channels::color_channels`]), the earliest part on a tie.
//! - [`decode_part`] decodes the channels of one flat part that a filter keeps, in their stored
//!   sample type, over the part's data window. Every compression the `exr` crate reads is read,
//!   scan lines and tiles alike (the largest resolution level). Other parts' chunks are skipped
//!   unread, and channels the filter drops are never allocated.
//! - [`write`] writes one scan-line part.
//!
//! The `exr` crate sizes buffers from headers before it reads pixels, so everything that reaches
//! it is checked first: preview attributes against their own size, each part's samples against
//! [`Limits`], all parts together against the budget, and the samples against what a file of
//! this size can decompress to.

use std::io::Cursor;

use exr::block::UncompressedBlock;
use exr::block::chunk::TileCoordinates;
use exr::image::read::image::ReadImage;
use exr::image::read::layers::{ChannelsReader, ReadAllLayers, ReadChannels};
use exr::meta::MetaData;
use exr::meta::attribute::{AttributeValue, SampleType as ExrSample};
use exr::meta::header::Header;
use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression as ExrCompression, Encoding, FlatSamples, Layer, LayerAttributes, LineOrder, WritableImage,
};
pub use half::f16;

use crate::channels::color_channels;

/// Why a file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is damaged or not OpenEXR.
    #[error("OpenEXR: {0}")]
    Malformed(String),
    /// A valid file this reader does not cover (deep data in a flat read, HTJ2K, subsampled
    /// channels kept by the filter).
    #[error("unsupported OpenEXR: {0}")]
    Unsupported(String),
    /// The file declares more than [`Limits`] allow.
    #[error("OpenEXR exceeds a limit: {0}")]
    LimitExceeded(String),
    /// The picture handed to [`write`] cannot be stored.
    #[error("OpenEXR: cannot write: {0}")]
    Encode(String),
}

/// Result of this module's functions.
pub type Result<T> = std::result::Result<T, Error>;

fn malformed(e: impl std::fmt::Display) -> Error {
    Error::Malformed(e.to_string())
}

/// An `exr`-crate error, its `NotSupported` kind kept apart from damaged data.
fn map_exr(e: exr::error::Error) -> Error {
    match e {
        exr::error::Error::NotSupported(message) => Error::Unsupported(message.to_string()),
        e => malformed(e),
    }
}

/// Caps on what a file may ask the reader to allocate. `#[non_exhaustive]`: start from
/// [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Largest data window width and height of a part.
    pub max_width: u32,
    pub max_height: u32,
    /// Largest data window of a part, in pixels.
    pub max_pixels: u64,
    /// Most bytes of samples all parts together may declare, counting 4 bytes a sample. A decode
    /// allocates at most this for the channels it keeps.
    pub max_bytes: u64,
    /// The most a compressed byte is taken to expand to (DWA parts get 4 times as much). A file
    /// declaring more samples than its size times this, plus 1 MiB, is refused before reading.
    pub max_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        let max_bytes = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 1 << 30 };
        Limits { max_width: 1 << 18, max_height: 1 << 18, max_pixels: 1 << 30, max_bytes, max_ratio: 1 << 16 }
    }
}

impl Limits {
    /// Effectively unlimited (still bounded by overflow checks).
    pub fn none() -> Self {
        Limits { max_width: u32::MAX, max_height: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX, max_ratio: u64::MAX }
    }

    /// Checks a `width × height` buffer of `bytes_per_pixel` against the size limits.
    pub(crate) fn check_bytes(&self, width: u32, height: u32, bytes_per_pixel: u64) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(Error::Malformed(format!("zero-sized picture {width}x{height}")));
        }
        if width > self.max_width || height > self.max_height {
            return Err(Error::LimitExceeded(format!("{width}x{height} exceeds {}x{}", self.max_width, self.max_height)));
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > self.max_pixels {
            return Err(Error::LimitExceeded(format!("{pixels} pixels exceed {}", self.max_pixels)));
        }
        let bytes = pixels.saturating_mul(bytes_per_pixel);
        if bytes > self.max_bytes || usize::try_from(bytes).is_err() {
            return Err(Error::LimitExceeded(format!("{bytes} bytes exceed {}", self.max_bytes)));
        }
        Ok(())
    }
}

/// How a channel's samples are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    F16,
    F32,
    U32,
}

impl SampleType {
    pub(crate) fn of(t: ExrSample) -> Self {
        match t {
            ExrSample::F16 => SampleType::F16,
            ExrSample::F32 => SampleType::F32,
            ExrSample::U32 => SampleType::U32,
        }
    }
}

/// A channel as the header declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    /// The name as stored (`diffuse.R`); the part name is not prefixed.
    pub name: String,
    pub sample: SampleType,
    /// Horizontal and vertical sampling (1, 1 for a full-resolution channel).
    pub sampling: (usize, usize),
}

/// A window: position and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Header facts of one part.
#[derive(Debug, Clone, PartialEq)]
pub struct PartInfo {
    /// Position in the file.
    pub index: usize,
    /// The part's name (`name` attribute), the prefix renderers give its channels.
    pub name: Option<String>,
    /// The `view` attribute of a stereo part.
    pub view: Option<String>,
    pub data_window: Window,
    pub display_window: Window,
    pub deep: bool,
    pub tiled: bool,
    /// The compression's name as the `exr` crate spells it (`ZIP16`, `PIZ`, `DWAA(None)`, …).
    pub compression: String,
    pub channels: Vec<ChannelInfo>,
    /// Every text attribute of the part and of the shared header (`cryptomatte/…/manifest`,
    /// `owner`, …), name and value, sorted by name.
    pub text_attributes: Vec<(String, String)>,
}

impl PartInfo {
    /// The value of the text attribute called `name`.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.text_attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// How much the part looks like a colour picture: 4 RGBA, 3 RGB, 2 YA, 1 Y (or a lone
    /// channel), 0 anything else.
    pub fn color_rank(&self) -> usize {
        color_channels(self.channels.iter().map(|c| c.name.as_str())).map_or(0, |(layout, _)| layout.channels())
    }
}

/// Refuses headers whose `preview` attributes declare more pixels than their value holds.
///
/// The `exr` crate sizes a preview's buffer from its declared dimensions (up to 4 MiB, zero
/// filled) before it finds the attribute too short, and in its non-pedantic mode then skips it
/// and goes on: thousands of tiny malformed previews would each cost that allocation. This walks
/// the raw headers and checks each preview against its own size. Anything else odd is left to
/// the crate to report.
pub(crate) fn check_previews(b: &[u8]) -> Result<()> {
    if b.get(..4) != Some(&[0x76, 0x2f, 0x31, 0x01][..]) {
        return Ok(());
    }
    let Some(flags) = b.get(4..8).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_le_bytes) else { return Ok(()) };
    let multipart = flags & 0x1000 != 0;
    let cstr = |at: usize| -> Option<(&[u8], usize)> {
        let rest = b.get(at..)?;
        let n = rest.iter().position(|&c| c == 0)?;
        Some((rest.get(..n)?, at + n + 1))
    };
    let u32_at = |at: usize| b.get(at..at.checked_add(4)?).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_le_bytes);
    let mut at = 8;
    loop {
        // One header's attributes: name, type, size, value; an empty name ends the header.
        loop {
            let Some((name, next)) = cstr(at) else { return Ok(()) };
            at = next;
            if name.is_empty() {
                break;
            }
            let Some((kind, next)) = cstr(at) else { return Ok(()) };
            let Some(size) = u32_at(next).and_then(|n| i32::try_from(n).ok()).and_then(|n| usize::try_from(n).ok()) else { return Ok(()) };
            let value = next + 4;
            let end = match value.checked_add(size) {
                Some(end) if end <= b.len() => end,
                _ => return Ok(()),
            };
            if kind == b"preview" {
                let declared = u32_at(value)
                    .zip(u32_at(value + 4))
                    .and_then(|(w, h)| u64::from(w).checked_mul(u64::from(h))?.checked_mul(4))
                    .filter(|_| size >= 8);
                if declared.is_none_or(|n| n > (size - 8) as u64) {
                    return Err(malformed("a preview attribute declares more pixels than it holds"));
                }
            }
            at = end;
        }
        // A multi-part header list ends with an empty header (a lone NUL).
        if !multipart || b.get(at) == Some(&0) {
            return Ok(());
        }
    }
}

fn window_size(size: exr::math::Vec2<usize>) -> (u32, u32) {
    (u32::try_from(size.0).unwrap_or(u32::MAX), u32::try_from(size.1).unwrap_or(u32::MAX))
}

/// Reads the headers and checks every part's declared size, alone and together, before any pixel
/// allocation. Headers are read without the crate's validation, which refuses deep parts.
fn read_meta(bytes: &[u8], limits: &Limits) -> Result<MetaData> {
    check_previews(bytes)?;
    let meta = MetaData::read_from_buffered(Cursor::new(bytes), false).map_err(map_exr)?;
    let (mut total, mut dwa) = (0u64, 0u64);
    for header in meta.headers.iter() {
        let (w, h) = window_size(header.layer_size);
        if w > limits.max_width || h > limits.max_height {
            return Err(Error::LimitExceeded(format!("a {w}x{h} part exceeds {}x{}", limits.max_width, limits.max_height)));
        }
        let pixels = u64::from(w) * u64::from(h);
        if pixels > limits.max_pixels {
            return Err(Error::LimitExceeded(format!("a part of {pixels} pixels exceeds {}", limits.max_pixels)));
        }
        let bytes = pixels.saturating_mul(header.channels.list.len().max(1) as u64 * 4);
        total = total.saturating_add(bytes);
        if matches!(header.compression, ExrCompression::DWAA(_) | ExrCompression::DWAB(_)) {
            dwa = dwa.saturating_add(bytes);
        }
    }
    if total > limits.max_bytes {
        return Err(Error::LimitExceeded(format!("{total} bytes across {} parts exceed {}", meta.headers.len(), limits.max_bytes)));
    }
    // The crate allocates a flat part's samples before reading its chunks, so the file must be
    // able to hold them. DWA's run-length channels are zlib-packed on top and reach about
    // 132000:1, so DWA parts are counted at a quarter. Deep parts size their samples from the
    // file's own counts.
    if !meta.requirements.has_deep_data {
        let declared = total.saturating_sub(dwa).saturating_add(dwa / 4);
        let capacity = (bytes.len() as u64).saturating_mul(limits.max_ratio).saturating_add(1 << 20);
        if declared > capacity {
            return Err(Error::LimitExceeded(format!("a {} byte file cannot hold {declared} bytes of samples", bytes.len())));
        }
    }
    Ok(meta)
}

fn text_attributes(header: &Header) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = header
        .shared_attributes
        .other
        .iter()
        .chain(header.own_attributes.other.iter())
        .filter_map(|(k, v)| match v {
            AttributeValue::Text(t) => Some((k.to_string(), t.to_string())),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn part_info(index: usize, header: &Header) -> PartInfo {
    let (w, h) = window_size(header.layer_size);
    let display = &header.shared_attributes.display_window;
    let (dw, dh) = window_size(display.size);
    PartInfo {
        index,
        name: header.own_attributes.layer_name.as_ref().map(|n| n.to_string()),
        view: header.own_attributes.view_name.as_ref().map(|v| v.to_string()),
        data_window: Window { x: header.own_attributes.layer_position.0, y: header.own_attributes.layer_position.1, width: w, height: h },
        display_window: Window { x: display.position.0, y: display.position.1, width: dw, height: dh },
        deep: header.deep,
        tiled: matches!(header.blocks, exr::meta::BlockDescription::Tiles(_)),
        compression: format!("{:?}", header.compression),
        channels: header
            .channels
            .list
            .iter()
            .map(|c| ChannelInfo { name: c.name.to_string(), sample: SampleType::of(c.sample_type), sampling: (c.sampling.0, c.sampling.1) })
            .collect(),
        text_attributes: text_attributes(header),
    }
}

/// Lists every part of the file from its headers, after checking the declared sizes.
pub fn parts(bytes: &[u8], limits: &Limits) -> Result<Vec<PartInfo>> {
    let meta = read_meta(bytes, limits)?;
    Ok(meta.headers.iter().enumerate().map(|(i, h)| part_info(i, h)).collect())
}

/// The part a plain picture decode shows: the highest [`PartInfo::color_rank`], the earliest on a
/// tie. `None` when no part has colour channels.
pub fn pick_part(parts: &[PartInfo]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for p in parts {
        let rank = p.color_rank();
        if rank > 0 && best.is_none_or(|(_, r)| rank > r) {
            best = Some((p.index, rank));
        }
    }
    best.map(|(i, _)| i)
}

/// The samples of a decoded channel, in their stored type.
#[derive(Debug, Clone, PartialEq)]
pub enum Samples {
    F16(Vec<f16>),
    F32(Vec<f32>),
    U32(Vec<u32>),
}

impl Samples {
    pub fn len(&self) -> usize {
        match self {
            Samples::F16(v) => v.len(),
            Samples::F32(v) => v.len(),
            Samples::U32(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sample `i` as `f32` (U32 IDs exact up to 2^24). 0 past the end.
    pub fn get_f32(&self, i: usize) -> f32 {
        match self {
            Samples::F16(v) => v.get(i).map_or(0.0, |s| s.to_f32()),
            Samples::F32(v) => v.get(i).copied().unwrap_or(0.0),
            Samples::U32(v) => v.get(i).map_or(0.0, |&s| s as f32),
        }
    }

    /// All samples as `f32`.
    pub fn to_f32(&self) -> Vec<f32> {
        match self {
            Samples::F16(v) => v.iter().map(|s| s.to_f32()).collect(),
            Samples::F32(v) => v.clone(),
            Samples::U32(v) => v.iter().map(|&s| s as f32).collect(),
        }
    }
}

/// A decoded channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// The name as stored.
    pub name: String,
    /// `width × height` samples of the part's data window, top row first.
    pub samples: Samples,
}

/// A decoded flat part: its header facts and the channels the filter kept, in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub info: PartInfo,
    pub channels: Vec<Channel>,
}

impl Part {
    /// The kept channel called `name`.
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

/// Places a data-window plane in a display window: samples outside the display window are cut,
/// the display window's pixels the data window does not cover are `fill`. `None` when the plane
/// is not `data.width × data.height` samples or the display window has more than `max_pixels`
/// pixels.
pub fn to_display_window(plane: &[f32], data: Window, display: Window, fill: f32, max_pixels: u64) -> Option<Vec<f32>> {
    let (w, h) = (data.width as usize, data.height as usize);
    if plane.len() != w.checked_mul(h)? || u64::from(display.width) * u64::from(display.height) > max_pixels {
        return None;
    }
    let (dw, dh) = (display.width as usize, display.height as usize);
    let mut out = vec![fill; dw.checked_mul(dh)?];
    let (ox, oy) = (i64::from(data.x) - i64::from(display.x), i64::from(data.y) - i64::from(display.y));
    for (y, row) in plane.chunks_exact(w.max(1)).enumerate() {
        let Some(ty) = usize::try_from(y as i64 + oy).ok().filter(|ty| *ty < dh) else { continue };
        let skip = usize::try_from(-ox).unwrap_or(0).min(w);
        let tx = usize::try_from(ox.max(0)).unwrap_or(usize::MAX);
        let take = (w - skip).min(dw.saturating_sub(tx));
        if take == 0 {
            continue;
        }
        let at = ty * dw + tx;
        if let (Some(dst), Some(src)) = (out.get_mut(at..at + take), row.get(skip..skip + take)) {
            dst.copy_from_slice(src);
        }
    }
    Some(out)
}

/// Which channels of which part a decode keeps.
struct Select<'k> {
    index: usize,
    keep: &'k dyn Fn(&str) -> bool,
    seen: std::cell::Cell<usize>,
}

/// One part's kept channels while it is decoded: each line goes straight into its channel's plane.
struct SelectReader {
    /// Per channel of the part: its plane, `None` when not kept.
    planes: Vec<Option<Samples>>,
    width: usize,
    height: usize,
}

impl<'s> ReadChannels<'s> for Select<'_> {
    type Reader = SelectReader;

    fn create_channels_reader(&'s self, header: &Header) -> exr::error::Result<SelectReader> {
        let position = self.seen.get();
        self.seen.set(position.saturating_add(1));
        let (width, height) = (header.layer_size.0, header.layer_size.1);
        if position != self.index {
            // Another part: keep nothing, so its chunks are skipped unread.
            return Ok(SelectReader { planes: header.channels.list.iter().map(|_| None).collect(), width, height });
        }
        let n = width.checked_mul(height).ok_or(exr::error::Error::Invalid("part size".into()))?;
        let mut planes = Vec::with_capacity(header.channels.list.len());
        for c in header.channels.list.iter() {
            if !(self.keep)(&c.name.to_string()) {
                planes.push(None);
                continue;
            }
            if c.sampling.0 != 1 || c.sampling.1 != 1 {
                return Err(exr::error::Error::NotSupported("subsampled channels".into()));
            }
            planes.push(Some(match c.sample_type {
                ExrSample::F16 => Samples::F16(vec![f16::ZERO; n]),
                ExrSample::F32 => Samples::F32(vec![0.0; n]),
                ExrSample::U32 => Samples::U32(vec![0; n]),
            }));
        }
        Ok(SelectReader { planes, width, height })
    }
}

impl ChannelsReader for SelectReader {
    type Channels = Vec<Option<Samples>>;

    fn filter_block(&self, tile: TileCoordinates) -> bool {
        tile.is_largest_resolution_level() && self.planes.iter().any(Option::is_some)
    }

    fn read_block(&mut self, header: &Header, block: UncompressedBlock) -> exr::error::UnitResult {
        for line in block.lines(&header.channels) {
            let Some(Some(plane)) = self.planes.get_mut(line.location.channel) else { continue };
            let (x, y, count) = (line.location.position.0, line.location.position.1, line.location.sample_count);
            if y >= self.height || x.checked_add(count).is_none_or(|end| end > self.width) {
                continue;
            }
            let at = y * self.width + x;
            match plane {
                Samples::F16(v) => line.read_samples_into_slice(v.get_mut(at..at + count).unwrap_or_default())?,
                Samples::F32(v) => line.read_samples_into_slice(v.get_mut(at..at + count).unwrap_or_default())?,
                Samples::U32(v) => line.read_samples_into_slice(v.get_mut(at..at + count).unwrap_or_default())?,
            }
        }
        Ok(())
    }

    fn into_channels(self) -> Vec<Option<Samples>> {
        self.planes
    }
}

/// Decodes the channels of flat part `index` that `keep` accepts (by the name as stored), in
/// their stored type. A file with deep parts, or a kept channel that is subsampled, is
/// [`Error::Unsupported`].
pub fn decode_part(bytes: &[u8], index: usize, keep: &dyn Fn(&str) -> bool, limits: &Limits) -> Result<Part> {
    let meta = read_meta(bytes, limits)?;
    if meta.requirements.has_deep_data {
        return Err(Error::Unsupported("deep data (use the deep reader)".into()));
    }
    let header = meta.headers.get(index).ok_or_else(|| malformed(format!("part {index} of {}", meta.headers.len())))?;
    if matches!(header.compression, ExrCompression::HTJ2K32 | ExrCompression::HTJ2K256) {
        return Err(Error::Unsupported("HTJ2K compression".into()));
    }
    let info = part_info(index, header);
    let select = Select { index, keep, seen: std::cell::Cell::new(0) };
    let image = ReadImage::new(ReadAllLayers { read_channels: select }, |_| {}).from_buffered(Cursor::new(bytes)).map_err(map_exr)?;
    let planes = image.layer_data.into_iter().nth(index).map(|l| l.channel_data).ok_or_else(|| malformed("the part was not read"))?;
    let channels = info.channels.iter().zip(planes).filter_map(|(c, p)| Some(Channel { name: c.name.clone(), samples: p? })).collect();
    Ok(Part { info, channels })
}

/// Compression for [`write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    None,
    Rle,
    Zip1,
    #[default]
    Zip16,
    Piz,
}

/// Writes one scan-line part of `width × height` with these channels (each `width × height`
/// samples, top row first; the order given does not matter, the file stores them sorted).
pub fn write(width: u32, height: u32, channels: &[Channel], compression: Compression) -> Result<Vec<u8>> {
    let enc = |m: &str| Error::Encode(m.into());
    if width == 0 || height == 0 || channels.is_empty() {
        return Err(enc("empty picture"));
    }
    let n = (width as usize).checked_mul(height as usize).ok_or_else(|| enc("picture too large"))?;
    if channels.iter().any(|c| c.samples.len() != n) {
        return Err(enc("a channel has the wrong number of samples"));
    }
    let mut list = Vec::with_capacity(channels.len());
    for c in channels {
        let name = exr::meta::attribute::Text::new_or_none(c.name.as_str()).ok_or_else(|| enc("a channel name is not valid OpenEXR text"))?;
        let samples = match &c.samples {
            Samples::F16(v) => FlatSamples::F16(v.clone()),
            Samples::F32(v) => FlatSamples::F32(v.clone()),
            Samples::U32(v) => FlatSamples::U32(v.clone()),
        };
        list.push(AnyChannel::new(name, samples));
    }
    let compression = match compression {
        Compression::None => ExrCompression::Uncompressed,
        Compression::Rle => ExrCompression::RLE,
        Compression::Zip1 => ExrCompression::ZIP1,
        Compression::Zip16 => ExrCompression::ZIP16,
        Compression::Piz => ExrCompression::PIZ,
    };
    let encoding = Encoding { compression, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    let layer = Layer::new((width as usize, height as usize), LayerAttributes::default(), encoding, AnyChannels::sort(list.into_iter().collect()));
    let mut out = Cursor::new(Vec::new());
    exr::image::Image::from_layer(layer).write().to_buffered(&mut out).map_err(|e| Error::Encode(e.to_string()))?;
    Ok(out.into_inner())
}
