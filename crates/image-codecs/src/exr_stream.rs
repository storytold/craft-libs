//! Bounded, seekable single-part scanline OpenEXR decoding. Total pixel count does not
//! size a decoded image buffer: only one compression block is returned at a time.
use std::io::{Cursor, Read, Seek, SeekFrom};

use crate::{CodecError, Format};
use exr::{
    block::{
        UncompressedBlock,
        chunk::{Chunk, CompressedBlock, CompressedScanLineBlock},
        lines::LineIndex,
    },
    meta::{BlockDescription, MetaData, attribute::SampleType},
};

const META_BYTES: usize = 8 << 20;
const BLOCK_BYTES: usize = 64 << 20;
const MAX_CHUNKS: usize = 1 << 20;

fn invalid(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(Format::OpenExr, e)
}
fn limit(s: &str) -> CodecError {
    CodecError::LimitExceeded(s.into())
}
fn unsupported(s: &str) -> CodecError {
    CodecError::unsupported(Format::OpenExr, s)
}

fn read<const N: usize>(r: &mut impl Read) -> Result<[u8; N], CodecError> {
    let mut bytes = [0; N];
    r.read_exact(&mut bytes).map_err(invalid)?;
    Ok(bytes)
}

fn string(r: &mut impl Read, prefix: &mut Vec<u8>) -> Result<bool, CodecError> {
    for i in 0..256 {
        let b = read::<1>(r)?;
        prefix.extend(b);
        if b == [0] {
            return Ok(i != 0);
        }
    }
    Err(limit("EXR attribute name exceeds 255 bytes"))
}

fn metadata(r: &mut impl Read, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, CodecError> {
    let start = read::<8>(r)?;
    if start.get(..4) != Some(&[0x76, 0x2f, 0x31, 0x01]) {
        return Err(invalid("EXR signature"));
    }
    let version = u32::from_le_bytes(start.get(4..).ok_or_else(|| invalid("version"))?.try_into().map_err(invalid)?);
    if version & (0x200 | 0x800 | 0x1000) != 0 {
        return Err(unsupported("streaming requires a single flat scanline part"));
    }
    let mut prefix = start.to_vec();
    let mut attributes = 0;
    loop {
        if cancel() {
            return Err(invalid("EXR import cancelled"));
        }
        if !string(r, &mut prefix)? {
            break;
        }
        attributes += 1;
        if attributes > 4096 {
            return Err(limit("EXR metadata exceeds 4096 attributes"));
        }
        string(r, &mut prefix)?;
        let length = read::<4>(r)?;
        prefix.extend(length);
        let n = usize::try_from(i32::from_le_bytes(length)).map_err(invalid)?;
        if n > META_BYTES || prefix.len().checked_add(n).is_none_or(|v| v > META_BYTES) {
            return Err(limit("EXR metadata exceeds 8 MiB"));
        }
        let begin = prefix.len();
        prefix.try_reserve_exact(n).map_err(|_| limit("EXR metadata allocation"))?;
        prefix.resize(begin + n, 0);
        for bytes in prefix.get_mut(begin..).ok_or_else(|| invalid("attribute size"))?.chunks_mut(64 << 10) {
            if cancel() {
                return Err(invalid("EXR import cancelled"));
            }
            r.read_exact(bytes).map_err(invalid)?;
        }
    }
    Ok(prefix)
}

/// Header facts used by the document owner to admit tile and working memory.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub gray: bool,
    pub has_alpha: bool,
    pub all_half: bool,
    pub working_bytes: u64,
    pub blocks: usize,
}

/// Interleaved little-endian F32, GrayA or RGBA, at source resolution.
pub struct Band {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub struct Decoder<R> {
    source: R,
    meta: MetaData,
    info: Info,
    offsets: Vec<u64>,
    seen: Vec<bool>,
    channels: Vec<Option<usize>>,
    next: usize,
    file_len: u64,
    chunks_start: u64,
    raw_limit: usize,
    rows: usize,
}

impl<R: Read + Seek> Decoder<R> {
    /// Read only bounded metadata and offsets; allocate no full-image or channel buffer.
    pub fn new(mut source: R, cancel: &dyn Fn() -> bool) -> Result<Self, CodecError> {
        let file_len = source.seek(SeekFrom::End(0)).map_err(invalid)?;
        source.seek(SeekFrom::Start(0)).map_err(invalid)?;
        let prefix = metadata(&mut source, cancel)?;
        let meta = std::panic::catch_unwind(|| MetaData::read_from_buffered(Cursor::new(&prefix), false))
            .map_err(|_| invalid("invalid EXR metadata"))?
            .map_err(invalid)?;
        let header = meta.headers.first().ok_or_else(|| invalid("missing header"))?;
        if meta.headers.len() != 1 || header.deep || !matches!(header.blocks, BlockDescription::ScanLines) {
            return Err(unsupported("streaming requires a single flat scanline part"));
        }
        let (w, h) = (header.layer_size.0, header.layer_size.1);
        if w == 0 || h == 0 || w > 1 << 18 || h > 1 << 18 {
            return Err(limit("EXR dimensions exceed the streaming limit of 262144"));
        }
        let list = &header.channels.list;
        if list.is_empty() || list.len() > 64 || list.iter().any(|c| c.sampling.0 != 1 || c.sampling.1 != 1) {
            return Err(unsupported("EXR channel count or subsampling"));
        }
        // The same channels the whole-file decoder shows, so both routes open the same image.
        let names: Vec<String> = list.iter().map(|c| c.name.to_string()).collect();
        let (layout, picked) = crate::image::exr_color_channels(names.iter().map(String::as_str)).ok_or_else(|| unsupported("no R/G/B or Y channels"))?;
        let gray = matches!(layout, crate::ChannelLayout::Gray | crate::ChannelLayout::GrayA);
        let mut selected: Vec<Option<usize>> = picked.into_iter().map(Some).collect();
        if !matches!(layout, crate::ChannelLayout::GrayA | crate::ChannelLayout::Rgba) {
            selected.push(None);
        }
        if selected.len() != if gray { 2 } else { 4 } {
            return Err(unsupported("EXR channel selection"));
        }
        // The document path currently tags Rec. 709 linear samples. Keep custom
        // primaries on the existing decoder until their profiles can be mapped.
        if !gray && header.shared_attributes.chromaticities.is_some() {
            return Err(unsupported("streaming custom EXR chromaticities requires a color profile"));
        }
        let all_half = selected.iter().flatten().all(|i| list.get(*i).is_some_and(|c| c.sample_type == SampleType::F16));
        let has_alpha = selected.last().copied().flatten().is_some();
        let channels = (0..list.len()).map(|i| selected.iter().position(|s| *s == Some(i))).collect();
        let rows = header.max_block_pixel_size().1;
        let expected = h.div_ceil(rows.max(1));
        if rows == 0 || header.chunk_count != expected || expected > MAX_CHUNKS {
            return Err(limit("EXR block count exceeds the streaming budget"));
        }
        let raw_limit = w
            .checked_mul(rows)
            .and_then(|n| n.checked_mul(header.channels.bytes_per_pixel))
            .filter(|n| *n <= BLOCK_BYTES)
            .ok_or_else(|| limit("EXR compression block exceeds 64 MiB"))?;
        let output = (w as u64).saturating_mul(rows as u64).saturating_mul(selected.len() as u64 * 4);
        let info = Info {
            width: w as u32,
            height: h as u32,
            gray,
            has_alpha,
            all_half,
            blocks: expected,
            working_bytes: (raw_limit as u64).saturating_mul(8).saturating_add(output.saturating_mul(2)).saturating_add(1 << 20),
        };
        let chunks_start = source
            .stream_position()
            .map_err(invalid)?
            .checked_add(expected as u64 * 8)
            .filter(|v| *v <= file_len)
            .ok_or_else(|| invalid("truncated offset table"))?;
        let mut offsets = Vec::new();
        offsets.try_reserve_exact(expected).map_err(|_| limit("EXR offset allocation"))?;
        for _ in 0..expected {
            if cancel() {
                return Err(invalid("EXR import cancelled"));
            }
            let offset = u64::from_le_bytes(read(&mut source)?);
            if offset < chunks_start || offset.checked_add(8).is_none_or(|v| v > file_len) {
                return Err(invalid("EXR chunk offset"));
            }
            offsets.push(offset);
        }
        Ok(Self { source, meta, info, offsets, seen: vec![false; expected], channels, next: 0, file_len, chunks_start, raw_limit, rows })
    }

    pub fn info(&self) -> Info {
        self.info
    }

    /// Decode one block. Check cancellation while reading and between channel rows.
    pub fn next_band(&mut self, cancel: &dyn Fn() -> bool) -> Result<Option<Band>, CodecError> {
        let Some(offset) = self.offsets.get(self.next).copied() else {
            return Ok(None);
        };
        if cancel() {
            return Err(invalid("EXR import cancelled"));
        }
        self.source.seek(SeekFrom::Start(offset)).map_err(invalid)?;
        let y = i32::from_le_bytes(read(&mut self.source)?);
        let n = usize::try_from(i32::from_le_bytes(read(&mut self.source)?)).map_err(invalid)?;
        if n > self.raw_limit.saturating_mul(2).saturating_add(64 << 10)
            || offset < self.chunks_start
            || offset.checked_add(8).and_then(|v| v.checked_add(n as u64)).is_none_or(|v| v > self.file_len)
        {
            return Err(invalid("EXR compressed block length"));
        }
        let header = self.meta.headers.first().ok_or_else(|| invalid("missing header"))?;
        let row = i64::from(y) - i64::from(header.own_attributes.layer_position.1);
        if row < 0 || row as u64 >= u64::from(self.info.height) || !(row as usize).is_multiple_of(self.rows) {
            return Err(invalid("EXR block coordinate"));
        }
        let seen = self.seen.get_mut(row as usize / self.rows).ok_or_else(|| invalid("EXR block coordinate"))?;
        if *seen {
            return Err(invalid("duplicate EXR block"));
        }
        *seen = true;
        let mut packed = Vec::new();
        packed.try_reserve_exact(n).map_err(|_| limit("EXR block allocation"))?;
        packed.resize(n, 0);
        for bytes in packed.chunks_mut(64 << 10) {
            if cancel() {
                return Err(invalid("EXR import cancelled"));
            }
            self.source.read_exact(bytes).map_err(invalid)?;
        }
        let chunk =
            Chunk { layer_index: 0, compressed_block: CompressedBlock::ScanLine(CompressedScanLineBlock { y_coordinate: y, compressed_pixels_le: packed }) };
        let block = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| UncompressedBlock::decompress_chunk(chunk, &self.meta, false)))
            .map_err(|_| invalid("invalid EXR compression block"))?
            .map_err(invalid)?;
        let (w, h) = (block.index.pixel_size.0, block.index.pixel_size.1);
        if w != self.info.width as usize
            || block.index.pixel_position.0 != 0
            || block.index.pixel_position.1 != row as usize
            || h != self.rows.min(self.info.height as usize - row as usize)
        {
            return Err(invalid("EXR block shape"));
        }
        let expected = w.checked_mul(h).and_then(|n| n.checked_mul(header.channels.bytes_per_pixel)).ok_or_else(|| invalid("EXR block size"))?;
        if block.data.len() != expected {
            return Err(invalid("EXR block sample count"));
        }
        let nc = if self.info.gray { 2 } else { 4 };
        let count = w.checked_mul(h).and_then(|n| n.checked_mul(nc * 4)).ok_or_else(|| limit("EXR output block size"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(count).map_err(|_| limit("EXR output allocation"))?;
        data.resize(count, 0);
        if !self.info.has_alpha {
            for pixel in data.chunks_exact_mut(nc * 4) {
                pixel.get_mut((nc - 1) * 4..).ok_or_else(|| invalid("alpha sample"))?.copy_from_slice(&1_f32.to_le_bytes());
            }
        }
        for (range, line) in LineIndex::lines_in_block(block.index, &header.channels) {
            if cancel() {
                return Err(invalid("EXR import cancelled"));
            }
            let Some(channel) = self.channels.get(line.channel).copied().flatten() else {
                continue;
            };
            let sample = header.channels.list.get(line.channel).ok_or_else(|| invalid("EXR channel"))?.sample_type;
            let input = block.data.get(range).ok_or_else(|| invalid("EXR row size"))?;
            let y = line.position.1.checked_sub(block.index.pixel_position.1).ok_or_else(|| invalid("EXR row coordinate"))?;
            let output = data.get_mut(y * w * nc * 4..(y + 1) * w * nc * 4).ok_or_else(|| invalid("EXR row coordinate"))?;
            for (bytes, pixel) in input.chunks_exact(sample.bytes_per_sample()).zip(output.chunks_exact_mut(nc * 4)) {
                let value = match sample {
                    SampleType::F16 => half::f16::from_bits(u16::from_ne_bytes(bytes.try_into().map_err(invalid)?)).to_f32(),
                    SampleType::F32 => f32::from_ne_bytes(bytes.try_into().map_err(invalid)?),
                    SampleType::U32 => u32::from_ne_bytes(bytes.try_into().map_err(invalid)?) as f32,
                };
                pixel.get_mut(channel * 4..channel * 4 + 4).ok_or_else(|| invalid("EXR channel"))?.copy_from_slice(&value.to_le_bytes());
            }
        }
        self.next += 1;
        Ok(Some(Band { x: 0, y: row as u32, width: w as u32, height: h as u32, data }))
    }
}
