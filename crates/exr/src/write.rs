//! Incremental flat OpenEXR output (feature `decode`).
//!
//! [`write_scanlines`] asks for one typed channel row at a time. [`write_tiles`] asks for one
//! tile at a time. Both buffer a single compression block, with sample and codec scratch space,
//! metadata and one offset per chunk.

use std::collections::HashSet;
use std::io::{Seek, SeekFrom, Write};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};

use exr::block::writer::ChunksWriter;
use exr::block::{BlockIndex, UncompressedBlock, enumerate_ordered_header_block_indices};
use exr::math::{RoundingMode, Vec2};
use exr::meta::attribute::{
    AttributeValue, ChannelDescription, Chromaticities, IntegerBounds, LevelMode, LineOrder, SampleType as ExrSample, Text, TileDescription,
};
use exr::meta::{BlockDescription, Headers, header::Header};

use crate::decode::{Compression, Error, MAX_HEADER_ATTRIBUTES, Result, SampleType, Window, f16, to_exr_compression};

const MAX_PARTS: usize = 1024;
const MAX_CHANNELS: usize = 16_384;
const MAX_CHUNKS: usize = 1 << 20;
const MAX_BLOCK_BYTES: usize = 64 << 20;
const MAX_NAME_BYTES: usize = 255;
const SHORT_NAME_BYTES: usize = 31;
const VERSION_WORD_OFFSET: u64 = 4;
const MAX_HEADER_BYTES: usize = 8 << 20;
const MAX_TEXT_ATTRIBUTES: usize = 16_384;

/// Name and stored sample type of a full-resolution channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel<'a> {
    pub name: &'a str,
    pub sample: SampleType,
}

/// The window, channels and encoding of one flat part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part<'a> {
    /// Required and unique in a multipart file. Optional in a single-part file.
    pub name: Option<&'a str>,
    pub data_window: Window,
    pub channels: &'a [Channel<'a>],
    /// Per-part compression.
    pub compression: Compression,
}

/// One flat tiled part with a single resolution level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePart<'a> {
    pub part: Part<'a>,
    pub tile_width: u32,
    pub tile_height: u32,
}

/// Names and values of the `string` attributes of one part.
pub type TextAttributes<'a> = &'a [(&'a str, &'a str)];

/// Optional file and part attributes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Metadata<'a> {
    /// CIE xy for red, green, blue and white, shared by every part.
    pub chromaticities: Option<[[f32; 2]; 4]>,
    /// Text attributes by input part index. An empty slice supplies no attributes. The file
    /// accepts at most 16,384 text attributes and 8 MiB of estimated headers. A part's header
    /// holds at most 4,096 attributes, so its text attributes leave room for the standard
    /// attributes the writer adds (channels, windows, compression and the like, plus the part
    /// name and type in a multipart file).
    pub text_attributes: &'a [TextAttributes<'a>],
}

/// A buffer to fill in the channel's stored type. Tiles use a contiguous, row-major channel plane.
#[derive(Debug)]
pub enum RowSamples<'a> {
    F16(&'a mut [f16]),
    F32(&'a mut [f32]),
    U32(&'a mut [u32]),
}

impl RowSamples<'_> {
    pub fn len(&self) -> usize {
        match self {
            Self::F16(row) => row.len(),
            Self::F32(row) => row.len(),
            Self::U32(row) => row.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn reborrow(&mut self) -> RowSamples<'_> {
        match self {
            Self::F16(row) => RowSamples::F16(row),
            Self::F32(row) => RowSamples::F32(row),
            Self::U32(row) => RowSamples::U32(row),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::F16(row) => row.fill(f16::ZERO),
            Self::F32(row) => row.fill(0.0),
            Self::U32(row) => row.fill(0),
        }
    }

    fn append_to(&self, data: &mut Vec<u8>) {
        match self {
            Self::F16(row) => row.iter().for_each(|sample| data.extend_from_slice(&sample.to_bits().to_ne_bytes())),
            Self::F32(row) => row.iter().for_each(|sample| data.extend_from_slice(&sample.to_ne_bytes())),
            Self::U32(row) => row.iter().for_each(|sample| data.extend_from_slice(&sample.to_ne_bytes())),
        }
    }

    fn copy_range_to(&self, range: Range<usize>, data: &mut [u8]) -> Result<()> {
        fn copy<T, const N: usize>(samples: &[T], range: Range<usize>, data: &mut [u8], bytes: impl Fn(&T) -> [u8; N]) -> Result<()> {
            let samples = samples.get(range).ok_or_else(|| encode("invalid tile sample range"))?;
            let (destinations, remainder) = data.as_chunks_mut::<N>();
            if !remainder.is_empty() || destinations.len() != samples.len() {
                return Err(encode("invalid tile channel byte length"));
            }
            for (sample, dest) in samples.iter().zip(destinations) {
                *dest = bytes(sample);
            }
            Ok(())
        }
        match self {
            Self::F16(samples) => copy(samples, range, data, |sample| sample.to_bits().to_ne_bytes()),
            Self::F32(samples) => copy(samples, range, data, |sample| sample.to_ne_bytes()),
            Self::U32(samples) => copy(samples, range, data, |sample| sample.to_ne_bytes()),
        }
    }
}

/// Writes one or more flat scan-line parts with a common `display_window`.
///
/// `fill_row(part, row, channel, samples)` fills the given buffer and returns the number of
/// samples supplied. `part` and `channel` are indices into the supplied descriptors. Parts
/// follow input order and rows increase from `0` at each part's data-window top. Within a row,
/// channels are requested in file name order. A callback error stops writing and is returned.
///
/// Pass a fresh seekable output positioned at byte zero. The caller owns buffering, truncation
/// and cleanup of partial output after an I/O or callback error. Invalid descriptors are rejected
/// before output begins. Maximums are 1,024 parts, 16,384 channels across all parts, 1,048,576
/// chunks across all parts and 64 MiB of uncompressed samples per chunk.
pub fn write_scanlines<W: Write + Seek + ?Sized>(
    out: &mut W,
    display_window: Window,
    parts: &[Part<'_>],
    fill_row: impl FnMut(usize, usize, usize, RowSamples<'_>) -> Result<usize>,
) -> Result<()> {
    write_scanlines_with_metadata(out, display_window, parts, &Metadata::default(), fill_row)
}

/// [`write_scanlines`] with optional file and part attributes.
pub fn write_scanlines_with_metadata<W: Write + Seek + ?Sized>(
    out: &mut W,
    display_window: Window,
    parts: &[Part<'_>],
    metadata: &Metadata<'_>,
    mut fill_row: impl FnMut(usize, usize, usize, RowSamples<'_>) -> Result<usize>,
) -> Result<()> {
    let (headers, channel_orders) = prepare_headers(display_window, parts.iter().map(|&part| (part, Storage::ScanLines)), metadata)?;
    let mut rows = SampleBuffers::new(&headers, |header| header.layer_size.0)?;
    write_blocks(out, headers, |index, header| {
        let order = channel_orders.get(index.layer).ok_or_else(|| encode("invalid scan-line block part"))?;
        fill_block(index, header, order, &mut rows, &mut fill_row)
    })
}

/// Writes one or more flat tiled parts with a common display window and one resolution level.
///
/// `fill_tile(part, window, writer)` is called once per tile and supplies each channel once
/// through [`TileWriter::fill_channel`]. `part` is the input part index. `window` gives the
/// tile's absolute coordinates and its actual size, clipped to its part's data window at edges.
/// Parts follow input order and tiles are requested left to right, then top to bottom.
/// The output and block limits are as for [`write_scanlines`], with the uncompressed block
/// limit applied to the nominal tile dimensions.
pub fn write_tiles<W: Write + Seek + ?Sized>(
    out: &mut W,
    display_window: Window,
    parts: &[TilePart<'_>],
    metadata: &Metadata<'_>,
    mut fill_tile: impl FnMut(usize, Window, &mut TileWriter<'_>) -> Result<()>,
) -> Result<()> {
    let layouts = parts.iter().map(|part| (part.part, Storage::Tiles(part.tile_width, part.tile_height)));
    let (headers, channel_orders) = prepare_headers(display_window, layouts, metadata)?;
    let mut planes = SampleBuffers::new(&headers, |header| {
        let Vec2(w, h) = header.max_block_pixel_size();
        w.min(header.layer_size.0).saturating_mul(h.min(header.layer_size.1))
    })?;
    let layouts: Vec<_> = headers.iter().zip(&channel_orders).map(|(header, order)| tile_layout(header, order)).collect::<Result<_>>()?;
    let mut supplied = vec![false; layouts.iter().map(Vec::len).max().unwrap_or_default()];
    write_blocks(out, headers, |index, header| {
        let layout = layouts.get(index.layer).ok_or_else(|| encode("invalid tile part"))?;
        let supplied = supplied.get_mut(..layout.len()).ok_or_else(|| encode("invalid tile channel count"))?;
        supplied.fill(false);
        let window = tile_window(index, header)?;
        let bytes = block_bytes(index, header)?;
        let mut data = block_buffer(bytes)?;
        data.resize(bytes, 0);
        let Vec2(width, height) = index.pixel_size;
        let mut writer =
            TileWriter { width, height, bytes_per_pixel: header.channels.bytes_per_pixel, layout, supplied, data: &mut data, planes: &mut planes };
        fill_tile(index.layer, window, &mut writer)?;
        if let Some(channel) = writer.supplied.iter().position(|&supplied| !supplied) {
            return Err(encode(format_args!("part {}, tile at ({}, {}): missing channel {channel}", index.layer, window.x, window.y)));
        }
        Ok(data)
    })
}

/// Receives the channel planes of one tile in the caller's channel order.
pub struct TileWriter<'a> {
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
    layout: &'a [TileChannel],
    supplied: &'a mut [bool],
    data: &'a mut [u8],
    planes: &'a mut SampleBuffers,
}

impl TileWriter<'_> {
    /// Fills a typed, row-major channel plane containing one sample per pixel. The callback
    /// returns the number of samples it supplied.
    pub fn fill_channel(&mut self, channel: usize, fill: impl FnOnce(RowSamples<'_>) -> Result<usize>) -> Result<()> {
        let layout = self.layout.get(channel).ok_or_else(|| encode(format_args!("unknown tile channel {channel}")))?;
        let supplied = self.supplied.get_mut(channel).ok_or_else(|| encode("invalid tile channel bookkeeping"))?;
        if *supplied {
            return Err(encode(format_args!("duplicate tile channel {channel}")));
        }
        let len = self.width.checked_mul(self.height).ok_or_else(|| encode("tile sample count overflow"))?;
        let mut plane = self.planes.get(layout.sample, len)?;
        plane.clear();
        let count = fill(plane.reborrow())?;
        if count != len {
            return Err(encode(format_args!("tile channel {channel}: expected {len} samples, got {count}")));
        }
        let stride = self.width * self.bytes_per_pixel;
        let start = self.width * layout.offset;
        let end = start + self.width * layout.sample.bytes_per_sample();
        for (y, row) in self.data.chunks_exact_mut(stride).enumerate() {
            let dest = row.get_mut(start..end).ok_or_else(|| encode("invalid tile channel byte range"))?;
            plane.copy_range_to(y * self.width..(y + 1) * self.width, dest)?;
        }
        *supplied = true;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct TileChannel {
    sample: ExrSample,
    // Width-independent byte count of the channels preceding this one in file order.
    offset: usize,
}

fn tile_layout(header: &Header, order: &[usize]) -> Result<Vec<TileChannel>> {
    let mut channels = vec![TileChannel { sample: ExrSample::F16, offset: 0 }; order.len()];
    let mut offset = 0;
    for (&input, channel) in order.iter().zip(&header.channels.list) {
        let entry = channels.get_mut(input).ok_or_else(|| encode("invalid tile channel order"))?;
        *entry = TileChannel { sample: channel.sample_type, offset };
        offset += channel.sample_type.bytes_per_sample();
    }
    Ok(channels)
}

fn tile_window(index: BlockIndex, header: &Header) -> Result<Window> {
    let position = |start: i32, offset: usize| {
        i32::try_from(offset).ok().and_then(|offset| start.checked_add(offset)).ok_or_else(|| encode("tile coordinate overflow"))
    };
    let origin = header.own_attributes.layer_position;
    Ok(Window {
        x: position(origin.0, index.pixel_position.0)?,
        y: position(origin.1, index.pixel_position.1)?,
        width: u32::try_from(index.pixel_size.0).map_err(encode)?,
        height: u32::try_from(index.pixel_size.1).map_err(encode)?,
    })
}

fn write_blocks<W: Write + Seek + ?Sized>(
    mut out: &mut W,
    headers: Headers,
    mut read_block: impl FnMut(BlockIndex, &Header) -> Result<Vec<u8>>,
) -> Result<()> {
    if out.stream_position().map_err(encode)? != 0 {
        return Err(encode("output must start at byte zero"));
    }
    let has_long_names = headers.iter().any(|header| {
        header.own_attributes.layer_name.as_ref().is_some_and(|name| name.bytes().len() > SHORT_NAME_BYTES)
            || header.channels.list.iter().any(|channel| channel.name.bytes().len() > SHORT_NAME_BYTES)
    });
    let mut version_update = None;
    let mut block_error = None;
    let written = catch_unwind(AssertUnwindSafe(|| {
        exr::block::write(&mut *out, headers, true, |meta, writer| {
            if has_long_names && !meta.requirements.has_long_names {
                version_update = Some(exr::meta::Requirements { has_long_names: true, ..meta.requirements });
            }
            for (chunk_index, index) in enumerate_ordered_header_block_indices(&meta.headers) {
                let data =
                    meta.headers.get(index.layer).ok_or_else(|| encode("invalid block part")).and_then(|header| read_block(index, header)).map_err(
                        |error| {
                            block_error = Some(error);
                            exr::error::Error::Aborted
                        },
                    )?;
                let chunk = UncompressedBlock { index, data }.compress_to_chunk(&meta.headers)?;
                writer.write_chunk(chunk_index, chunk)?;
            }
            Ok(())
        })
    }));
    if let Some(error) = block_error {
        return Err(error);
    }
    written.map_err(|_| encode("block writer panicked"))?.map_err(encode)?;
    // The codec can omit bit 10 when inferring requirements from channel names. Update the
    // OpenEXR v2 version word in place, preserving the codec's other flags and all chunk offsets.
    if let Some(requirements) = version_update {
        let position = out.stream_position().map_err(encode)?;
        out.seek(SeekFrom::Start(VERSION_WORD_OFFSET)).map_err(encode)?;
        requirements.write(&mut out).map_err(encode)?;
        out.seek(SeekFrom::Start(position)).map_err(encode)?;
    }
    out.flush().map_err(encode)
}

#[derive(Clone, Copy)]
enum Storage {
    ScanLines,
    Tiles(u32, u32),
}

impl Storage {
    fn describe(self, size: Vec2<usize>, compression: exr::compression::Compression) -> Result<(BlockDescription, Vec2<usize>, usize)> {
        match self {
            Self::ScanLines => {
                let rows = compression.scan_lines_per_block();
                Ok((BlockDescription::ScanLines, Vec2(size.0, rows), size.1.div_ceil(rows)))
            }
            Self::Tiles(width, height) => {
                let dimension = |n| usize::try_from(n).ok().filter(|&n| n > 0 && n <= MAX_BLOCK_BYTES);
                let (w, h) = dimension(width).zip(dimension(height)).ok_or_else(|| encode("invalid tile dimensions"))?;
                let chunks = size.0.div_ceil(w).checked_mul(size.1.div_ceil(h)).ok_or_else(|| encode("tile count overflow"))?;
                let tile_size = Vec2(w, h);
                let tiles = TileDescription { tile_size, level_mode: LevelMode::Singular, rounding_mode: RoundingMode::Down };
                Ok((BlockDescription::Tiles(tiles), tile_size, chunks))
            }
        }
    }
}

fn prepare_headers<'a>(
    display_window: Window,
    parts: impl ExactSizeIterator<Item = (Part<'a>, Storage)>,
    metadata: &Metadata<'_>,
) -> Result<(Headers, Vec<Vec<usize>>)> {
    let part_count = parts.len();
    if part_count == 0 || part_count > MAX_PARTS {
        return Err(encode("expected 1 to 1,024 parts"));
    }
    if !metadata.text_attributes.is_empty() && metadata.text_attributes.len() != part_count {
        return Err(encode("text attributes must have one entry per part"));
    }
    if metadata.text_attributes.iter().map(|attributes| attributes.len()).fold(0usize, usize::saturating_add) > MAX_TEXT_ATTRIBUTES {
        return Err(encode(format_args!("more than {MAX_TEXT_ATTRIBUTES} text attributes")));
    }
    let chromaticities = metadata.chromaticities.map(checked_chromaticities).transpose()?;
    let display = window_bounds(display_window)?;
    let mut headers = Headers::new();
    let mut channel_orders = Vec::with_capacity(part_count);
    let mut part_names = HashSet::new();
    let (mut channel_count, mut chunk_count) = (0usize, 0usize);
    let mut max_file_bytes = 16usize; // Magic, version and header-list terminator.
    let mut header_bytes = 16usize;
    for (index, (part, storage)) in parts.enumerate() {
        let name = part.name.map(|name| checked_name(name, "part")).transpose()?;
        if let Some(name) = &name {
            if !part_names.insert(name.clone()) {
                return Err(encode(format_args!("duplicate part name {name}")));
            }
        } else if part_count > 1 {
            return Err(encode(format_args!("multipart part {index} needs a name")));
        }

        let data = window_bounds(part.data_window)?;
        let (width, height) = (data.size.0, data.size.1);
        channel_count = channel_count.saturating_add(part.channels.len());
        if part.channels.is_empty() || channel_count > MAX_CHANNELS {
            return Err(encode(format_args!("part {index} needs channels and the file supports up to {MAX_CHANNELS} channels")));
        }

        let compression = to_exr_compression(part.compression);
        let (blocks, Vec2(block_width, block_height), chunks) = storage.describe(data.size, compression)?;
        chunk_count = chunk_count.saturating_add(chunks);
        if chunk_count > MAX_CHUNKS {
            return Err(encode(format_args!("more than {MAX_CHUNKS} blocks")));
        }

        let mut channels = Vec::with_capacity(part.channels.len());
        for (channel_index, channel) in part.channels.iter().enumerate() {
            let name = checked_name(channel.name, "channel")?;
            let sample = match channel.sample {
                SampleType::F16 => ExrSample::F16,
                SampleType::F32 => ExrSample::F32,
                SampleType::U32 => ExrSample::U32,
            };
            channels.push((channel_index, ChannelDescription::named(name, sample)));
        }
        channels.sort_unstable_by(|(_, a), (_, b)| a.name.cmp(&b.name));
        if let Some([(_, a), _]) = channels.windows(2).find(|pair| matches!(pair, [(_, a), (_, b)] if a.name == b.name)) {
            return Err(encode(format_args!("duplicate channel name {} in part {index}", a.name)));
        }

        let bytes_per_pixel: usize = channels.iter().map(|(_, channel)| channel.sample_type.bytes_per_sample()).sum();
        let bounded_height = if matches!(storage, Storage::ScanLines) { block_height.min(height) } else { block_height };
        if block_width.saturating_mul(bounded_height).saturating_mul(bytes_per_pixel) > MAX_BLOCK_BYTES {
            return Err(encode(format_args!("a block in part {index} exceeds {MAX_BLOCK_BYTES} uncompressed bytes")));
        }
        // The codec uses usize for the nominal block size and output offsets. Allow space
        // beyond raw samples for standard attributes, channel entries, and chunk prefixes/offsets.
        if block_width.checked_mul(block_height).and_then(|n| n.checked_mul(bytes_per_pixel)).is_none() {
            return Err(encode(format_args!("a block in part {index} exceeds the codec's addressable size")));
        }
        let header_estimate = 1024 + channels.iter().map(|(_, channel)| channel.name.bytes().len() + 32).sum::<usize>();
        header_bytes = header_bytes.saturating_add(header_estimate);
        let attributes = metadata.text_attributes.get(index).copied().unwrap_or_default();
        let extra_bytes = text_bytes(attributes, &mut header_bytes)?;
        let overhead = header_estimate + extra_bytes + chunks * 32;
        max_file_bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
            .and_then(|samples| samples.checked_add(overhead))
            .and_then(|part_bytes| max_file_bytes.checked_add(part_bytes))
            .ok_or_else(|| encode("file may exceed the codec's addressable size"))?;

        let (order, descriptions) = channels.into_iter().unzip();
        let mut header = Header::new(Text::from_slice_unchecked(b""), data.size, descriptions)
            .with_display_window(display)
            .with_position(data.position)
            .with_encoding(compression, blocks, LineOrder::Increasing);
        header.own_attributes.layer_name = name;
        header.shared_attributes.chromaticities = chromaticities;
        set_text_attributes(&mut header, attributes)?;
        let header_attributes = header.all_named_attributes().count();
        if header_attributes > MAX_HEADER_ATTRIBUTES {
            return Err(encode(format_args!(
                "part {index} has {header_attributes} attributes, standard ones included, and a header holds at most {MAX_HEADER_ATTRIBUTES}"
            )));
        }
        headers.push(header);
        channel_orders.push(order);
    }
    Ok((headers, channel_orders))
}

fn checked_name(name: &str, kind: &str) -> Result<Text> {
    // exr::Text stores one byte per character. Check the character count before allocating it.
    if name.is_empty() || name.chars().take(MAX_NAME_BYTES + 1).count() > MAX_NAME_BYTES || name.bytes().any(|byte| byte == 0) {
        return Err(encode(format_args!("invalid {kind} name: expected 1 to {MAX_NAME_BYTES} OpenEXR bytes without NUL")));
    }
    Text::new_or_none(name).ok_or_else(|| encode(format_args!("invalid {kind} name: unsupported characters")))
}

fn checked_chromaticities(points: [[f32; 2]; 4]) -> Result<Chromaticities> {
    if points.iter().flatten().any(|value| !value.is_finite()) {
        return Err(encode("chromaticities must be finite"));
    }
    let [red, green, blue, white] = points.map(|[x, y]| Vec2(x, y));
    Ok(Chromaticities { red, green, blue, white })
}

fn text_bytes(attributes: TextAttributes<'_>, header_bytes: &mut usize) -> Result<usize> {
    let start = *header_bytes;
    if start > MAX_HEADER_BYTES {
        return Err(encode("OpenEXR headers exceed 8 MiB"));
    }
    for &(key, value) in attributes {
        *header_bytes = header_bytes.saturating_add(key.len()).saturating_add(value.len()).saturating_add(16);
        if *header_bytes > MAX_HEADER_BYTES {
            return Err(encode("OpenEXR headers exceed 8 MiB"));
        }
    }
    Ok(*header_bytes - start)
}

fn set_text_attributes(header: &mut Header, attributes: TextAttributes<'_>) -> Result<()> {
    let mut seen = HashSet::new();
    for &(name, value) in attributes {
        let key = checked_name(name, "attribute")?;
        if !seen.insert(name) {
            return Err(encode(format_args!("duplicate attribute {name}")));
        }
        let value = Text::new_or_none(value).ok_or_else(|| encode(format_args!("invalid string attribute {name}: unsupported characters")))?;
        if value.bytes().contains(&0) {
            return Err(encode(format_args!("invalid string attribute {name}: contains NUL")));
        }
        let attrs = &mut header.own_attributes;
        let field = match name {
            "owner" => &mut attrs.owner,
            "comments" => &mut attrs.comments,
            "capDate" => &mut attrs.capture_date,
            "wrapmodes" => &mut attrs.wrap_mode_name,
            "view" => &mut attrs.view_name,
            "software" => &mut attrs.software_name,
            "renderingTransform" => &mut attrs.rendering_transform_name,
            "lookModTransform" => &mut attrs.look_modification_transform_name,
            "channels"
            | "compression"
            | "dataWindow"
            | "displayWindow"
            | "lineOrder"
            | "pixelAspectRatio"
            | "screenWindowCenter"
            | "screenWindowWidth"
            | "name"
            | "type"
            | "version"
            | "chunkCount"
            | "tiles"
            | "maxSamplesPerPixel"
            | "chromaticities"
            | "timeCode"
            | "preview"
            | "whiteLuminance"
            | "adoptedNeutral"
            | "xDensity"
            | "utcOffset"
            | "longitude"
            | "latitude"
            | "altitude"
            | "focus"
            | "expTime"
            | "aperture"
            | "isoSpeed"
            | "envmap"
            | "keyCode"
            | "framesPerSecond"
            | "multiView"
            | "worldToCamera"
            | "worldToNDC"
            | "deepImageState"
            | "originalDataWindow"
            | "nearClipPlane"
            | "farClipPlane"
            | "horizontalFieldOfView"
            | "verticalFieldOfView" => {
                return Err(encode(format_args!("reserved attribute {name}")));
            }
            _ => {
                attrs.other.insert(key, AttributeValue::Text(value));
                continue;
            }
        };
        *field = Some(value);
    }
    Ok(())
}

fn window_bounds(window: Window) -> Result<IntegerBounds> {
    let axis = |position: i32, size: u32| {
        // The codec works with inclusive and exclusive ends and negates the origin in i32.
        let size = i32::try_from(size).ok().filter(|&size| size > 0)?;
        position.checked_neg()?;
        position.checked_add(size)?;
        usize::try_from(size).ok()
    };
    let size = axis(window.x, window.width).zip(axis(window.y, window.height)).ok_or_else(|| encode("invalid OpenEXR window dimensions"))?;
    Ok(IntegerBounds::new((window.x, window.y), size))
}

struct SampleBuffers {
    half: Vec<f16>,
    float: Vec<f32>,
    uint: Vec<u32>,
}

impl SampleBuffers {
    fn new(headers: &[Header], samples: impl Fn(&Header) -> usize) -> Result<Self> {
        let mut lengths = (0, 0, 0);
        for header in headers {
            for channel in &header.channels.list {
                let len = match channel.sample_type {
                    ExrSample::F16 => &mut lengths.0,
                    ExrSample::F32 => &mut lengths.1,
                    ExrSample::U32 => &mut lengths.2,
                };
                *len = (*len).max(samples(header));
            }
        }
        fn zeroed<T: Default + Clone>(width: usize) -> Result<Vec<T>> {
            let mut samples = Vec::new();
            samples.try_reserve_exact(width).map_err(|_| encode("could not allocate sample buffer"))?;
            samples.resize(width, T::default());
            Ok(samples)
        }
        Ok(Self { half: zeroed(lengths.0)?, float: zeroed(lengths.1)?, uint: zeroed(lengths.2)? })
    }

    fn get(&mut self, sample: ExrSample, width: usize) -> Result<RowSamples<'_>> {
        match sample {
            ExrSample::F16 => self.half.get_mut(..width).map(RowSamples::F16),
            ExrSample::F32 => self.float.get_mut(..width).map(RowSamples::F32),
            ExrSample::U32 => self.uint.get_mut(..width).map(RowSamples::U32),
        }
        .ok_or_else(|| encode("sample buffer has the wrong size"))
    }
}

fn fill_block(
    index: BlockIndex,
    header: &Header,
    order: &[usize],
    rows: &mut SampleBuffers,
    fill_row: &mut impl FnMut(usize, usize, usize, RowSamples<'_>) -> Result<usize>,
) -> Result<Vec<u8>> {
    let part = index.layer;
    let Vec2(width, height) = index.pixel_size;
    let mut data = block_buffer(block_bytes(index, header)?)?;
    let first = index.pixel_position.1;
    let end = first.checked_add(height).ok_or_else(|| encode("scan-line row index overflow"))?;
    for row in first..end {
        for (&channel, description) in order.iter().zip(&header.channels.list) {
            let mut samples = rows.get(description.sample_type, width)?;
            samples.clear();
            let count = fill_row(part, row, channel, samples.reborrow())?;
            if count != width {
                return Err(encode(format_args!("part {part}, channel {channel}, row {row}: expected {width} samples, got {count}")));
            }
            samples.append_to(&mut data);
        }
    }
    Ok(data)
}

fn block_bytes(index: BlockIndex, header: &Header) -> Result<usize> {
    let Vec2(width, height) = index.pixel_size;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(header.channels.bytes_per_pixel))
        .filter(|&bytes| (1..=MAX_BLOCK_BYTES).contains(&bytes))
        .ok_or_else(|| encode("invalid block size"))
}

fn block_buffer(bytes: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    data.try_reserve_exact(bytes).map_err(|_| encode("could not allocate block buffer"))?;
    Ok(data)
}

fn encode(message: impl std::fmt::Display) -> Error {
    Error::Encode(message.to_string())
}
