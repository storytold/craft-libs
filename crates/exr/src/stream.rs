//! Bounded block-by-block reading of one flat scan-line part (feature `decode`): a reader that
//! never sizes a buffer by the whole picture. [`Decoder::new`] reads the headers (at most 8 MiB
//! and 4096 attributes) and the offset table through `Read + Seek`; [`Decoder::next_band`] then
//! returns one compression block at a time as interleaved f32 samples. A caller that writes bands
//! into tiled storage can open pictures larger than memory.
//!
//! The colour channels are the ones [`crate::channels::color_channels`] picks, so this reader and
//! [`crate::decode::decode_part`] show the same picture. Missing alpha is 1.
//!
//! Only single-part, flat, scan-line files are taken ([`Error::Unsupported`] otherwise): the
//! caller falls back to the whole-part decoder for the rest.
use std::io::{Cursor, Read, Seek, SeekFrom};

use exr::{
    block::{
        UncompressedBlock,
        chunk::{Chunk, CompressedBlock, CompressedScanLineBlock},
        lines::LineIndex,
    },
    meta::{BlockDescription, MetaData, attribute::SampleType},
};

use crate::channels::{ColorLayout, color_channels};
use crate::decode::{Error, Result};

const META_BYTES: usize = 8 << 20;
const BLOCK_BYTES: usize = 64 << 20;
const MAX_CHUNKS: usize = 1 << 20;

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Malformed(e.to_string())
}
fn limit(s: &str) -> Error {
    Error::LimitExceeded(s.into())
}
fn unsupported(s: &str) -> Error {
    Error::Unsupported(s.into())
}

fn read<const N: usize>(r: &mut impl Read) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    r.read_exact(&mut bytes).map_err(invalid)?;
    Ok(bytes)
}

fn string(r: &mut impl Read, prefix: &mut Vec<u8>) -> Result<bool> {
    for i in 0..256 {
        let b = read::<1>(r)?;
        prefix.extend(b);
        if b == [0] {
            return Ok(i != 0);
        }
    }
    Err(limit("EXR attribute name exceeds 255 bytes"))
}

fn metadata(r: &mut impl Read, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>> {
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

/// Header facts a caller needs to admit its tile and working memory before decoding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    /// Luminance (bands are Y, A) rather than colour (R, G, B, A).
    pub gray: bool,
    pub has_alpha: bool,
    /// Every picked channel is half float.
    pub all_half: bool,
    /// The working memory one [`Decoder::next_band`] needs at most, with headroom.
    pub working_bytes: u64,
    /// Compression blocks in the part (the number of bands).
    pub blocks: usize,
    /// The `chromaticities` attribute (red, green, blue and white x, y), when the file names one.
    pub chromaticities: Option<[f32; 8]>,
}

/// One block of rows: interleaved little-endian f32, Y A or R G B A, at source resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// A part being read band by band.
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
    /// Reads only bounded metadata and the offset table; allocates no picture or channel buffer.
    /// `cancel` is checked while reading.
    pub fn new(mut source: R, cancel: &dyn Fn() -> bool) -> Result<Self> {
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
        let (layout, picked) = color_channels(names.iter().map(String::as_str)).ok_or_else(|| unsupported("no R/G/B or Y channels"))?;
        let gray = matches!(layout, ColorLayout::Y | ColorLayout::YA);
        let mut selected: Vec<Option<usize>> = picked.into_iter().map(Some).collect();
        if !matches!(layout, ColorLayout::YA | ColorLayout::Rgba) {
            selected.push(None);
        }
        if selected.len() != if gray { 2 } else { 4 } {
            return Err(unsupported("EXR channel selection"));
        }
        // Reported, not refused: whether custom primaries can be shown is the caller's colour policy.
        let chromaticities =
            header.shared_attributes.chromaticities.map(|c| [c.red.0, c.red.1, c.green.0, c.green.1, c.blue.0, c.blue.1, c.white.0, c.white.1]);
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
            chromaticities,
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

    /// What the headers say.
    pub fn info(&self) -> Info {
        self.info
    }

    /// Decodes the next block, `None` after the last. `cancel` is checked while reading and
    /// between rows.
    pub fn next_band(&mut self, cancel: &dyn Fn() -> bool) -> Result<Option<Band>> {
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
        let chunk = Chunk {
            layer_index: 0,
            compressed_block: CompressedBlock::ScanLine(CompressedScanLineBlock { y_coordinate: y, compressed_pixels_le: packed }),
        };
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

#[cfg(test)]
mod tests {
    //! PhotoCraft's tests; its encoder and whole-file decoder are replaced by
    //! [`crate::decode::write`] and [`crate::decode::decode_part`].
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::io::Cursor;

    use super::*;
    use crate::decode::{Channel, Compression, Limits, Samples, decode_part, f16, write};

    const W: usize = 31;
    const H: usize = 35;

    fn value(i: usize) -> f32 {
        [-0.25, 3.5, 0.0, 0.0001, 0.5][i % 5]
    }

    /// A 31 × 35 file of these channels, half or float, with this compression.
    fn fixture(names: &[&str], half: bool, compression: Compression) -> Vec<u8> {
        let channels: Vec<Channel> = names
            .iter()
            .enumerate()
            .map(|(c, n)| {
                let v: Vec<f32> = (0..W * H).map(|p| value(p * names.len() + c)).collect();
                let samples = if half { Samples::F16(v.iter().map(|&x| f16::from_f32(x)).collect()) } else { Samples::F32(v) };
                Channel { name: (*n).into(), samples }
            })
            .collect();
        write(W as u32, H as u32, &channels, compression).unwrap()
    }

    /// What the whole-part decoder gives, interleaved as bands are: Y A or R G B A, alpha 1 when absent.
    fn reference(bytes: &[u8], names: &[&str]) -> Vec<u8> {
        let part = decode_part(bytes, 0, &|_| true, &Limits::default()).unwrap();
        let gray = names.contains(&"Y");
        let order: &[&str] = if gray { &["Y", "A"] } else { &["R", "G", "B", "A"] };
        let mut out = Vec::new();
        for p in 0..W * H {
            for n in order {
                let v = part.channel(n).map_or(1.0, |c| c.samples.get_f32(p));
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn scanline_blocks_match_whole_decoder_at_each_layout_depth_and_compression() {
        for names in [&["Y"][..], &["Y", "A"], &["R", "G", "B"], &["R", "G", "B", "A"]] {
            for half in [true, false] {
                for compression in [Compression::None, Compression::Rle, Compression::Zip1, Compression::Zip16, Compression::Piz] {
                    let bytes = fixture(names, half, compression);
                    let want = reference(&bytes, names);
                    let mut stream = Decoder::new(Cursor::new(&bytes), &|| false).unwrap();
                    let info = stream.info();
                    assert_eq!(
                        (info.all_half, info.gray, info.has_alpha, info.chromaticities),
                        (half, names.contains(&"Y"), names.contains(&"A"), None)
                    );
                    let nc = if info.gray { 2 } else { 4 };
                    let row = W * nc * 4;
                    let mut actual = vec![0; want.len()];
                    let mut blocks = 0;
                    while let Some(band) = stream.next_band(&|| false).unwrap() {
                        actual[band.y as usize * row..(band.y + band.height) as usize * row].copy_from_slice(&band.data);
                        blocks += 1;
                    }
                    assert_eq!(blocks, info.blocks);
                    assert_eq!(actual, want, "{names:?}/half {half}/{compression:?}");
                }
            }
        }
    }

    fn change_window(bytes: &mut [u8], width: i32, height: i32) {
        let needle = b"dataWindow\0box2i\0\x10\0\0\0";
        let at = bytes.windows(needle.len()).position(|v| v == needle).unwrap() + needle.len();
        bytes[at + 8..at + 12].copy_from_slice(&(width - 1).to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&(height - 1).to_le_bytes());
    }

    #[test]
    fn enormous_header_is_not_a_whole_image_allocation() {
        let mut bytes = fixture(&["R", "G", "B"], true, Compression::Zip16);
        change_window(&mut bytes, 65536, 32768);
        assert!(matches!(decode_part(&bytes, 0, &|_| true, &Limits::default()), Err(Error::LimitExceeded(_))));
        // Streaming gets as far as the now-truncated offset table, without a pixel allocation.
        assert!(matches!(Decoder::new(Cursor::new(bytes), &|| false), Err(Error::Malformed(_))));
    }

    #[test]
    fn cancellation_truncation_duplicate_offsets_and_forged_lengths_fail() {
        let bytes = fixture(&["R", "G", "B"], true, Compression::Zip16);
        assert!(Decoder::new(Cursor::new(&bytes), &|| true).is_err());
        let mut decoder = Decoder::new(Cursor::new(&bytes), &|| false).unwrap();
        assert!(decoder.next_band(&|| true).is_err());
        for end in [0, 8, bytes.len() / 2] {
            assert!(Decoder::new(Cursor::new(&bytes[..end]), &|| false).and_then(|mut r| r.next_band(&|| false)).is_err());
        }
        // Locate the offset table by finding the header end.
        let mut input = Cursor::new(&bytes);
        let _ = MetaData::read_from_buffered(&mut input, false).unwrap();
        let at = input.position() as usize;
        let offset = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        let mut corrupt = bytes.clone();
        corrupt[offset + 4..offset + 8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(Decoder::new(Cursor::new(corrupt), &|| false).unwrap().next_band(&|| false).is_err());
        let mut duplicate = bytes.clone();
        duplicate[at + 8..at + 16].copy_from_slice(&bytes[at..at + 8]);
        let mut decoder = Decoder::new(Cursor::new(duplicate), &|| false).unwrap();
        assert!(decoder.next_band(&|| false).unwrap().is_some());
        assert!(decoder.next_band(&|| false).is_err());
    }

    #[test]
    fn hostile_metadata_is_bounded_and_mutated_inputs_never_panic() {
        let bytes = fixture(&["R", "G", "B"], true, Compression::Zip16);
        let mut large = bytes.clone();
        let needle = b"channels\0chlist\0";
        let length = bytes.windows(needle.len()).position(|v| v == needle).unwrap() + needle.len();
        large[length..length + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(matches!(Decoder::new(Cursor::new(large), &|| false), Err(Error::LimitExceeded(_))));
        let mut many = bytes[..8].to_vec();
        for _ in 0..4097 {
            many.extend_from_slice(b"z\0string\0\0\0\0\0");
        }
        many.push(0);
        assert!(matches!(Decoder::new(Cursor::new(many), &|| false), Err(Error::LimitExceeded(_))));
        // Small deterministic mutations cover headers, offsets and compressed payloads.
        for i in 0..256 {
            let mut corrupt = bytes.clone();
            let at = i * (bytes.len() - 1) / 255;
            corrupt[at] ^= 0xff;
            if let Ok(mut decoder) = Decoder::new(Cursor::new(corrupt), &|| false) {
                while let Ok(Some(_)) = decoder.next_band(&|| false) {}
            }
        }
    }

    #[test]
    fn mixed_samples_and_offset_data_window_keep_coordinates_and_values() {
        use exr::prelude::{AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Layer, LayerAttributes, LineOrder, WritableImage};
        let channels = AnyChannels::sort(
            vec![
                AnyChannel::new("R", FlatSamples::F16(vec![f16::from_f32(-0.25); W * H])),
                AnyChannel::new("G", FlatSamples::F32(vec![3.5; W * H])),
                AnyChannel::new("B", FlatSamples::U32(vec![42; W * H])),
            ]
            .into_iter()
            .collect(),
        );
        let attributes = LayerAttributes { layer_position: exr::math::Vec2(-15, 40), ..Default::default() };
        let layer = Layer::new(
            (W, H),
            attributes,
            Encoding { compression: Compression::ZIP16, blocks: Blocks::ScanLines, line_order: LineOrder::Decreasing },
            channels,
        );
        let mut bytes = Cursor::new(Vec::new());
        exr::image::Image::from_layer(layer).write().to_buffered(&mut bytes).unwrap();
        let mut decoder = Decoder::new(bytes, &|| false).unwrap();
        assert!(!decoder.info().all_half);
        let mut rows = 0;
        while let Some(band) = decoder.next_band(&|| false).unwrap() {
            assert_eq!(band.y, rows);
            rows += band.height;
            for p in band.data.as_chunks::<16>().0 {
                let v: Vec<f32> = p.as_chunks::<4>().0.iter().map(|s| f32::from_le_bytes(*s)).collect();
                assert_eq!(v, [-0.25, 3.5, 42.0, 1.0]);
            }
        }
        assert_eq!(rows, H as u32);
    }

    #[test]
    fn unsupported_variants_and_reported_chromaticities() {
        let bytes = fixture(&["R", "G", "B"], true, Compression::Zip16);
        for flag in [0x200_u32, 0x800, 0x1000] {
            let mut changed = bytes.clone();
            changed[4..8].copy_from_slice(&(2 | flag).to_le_bytes());
            assert!(matches!(Decoder::new(Cursor::new(changed), &|| false), Err(Error::Unsupported(_))));
        }
        // Custom primaries are reported for the caller's colour policy, not refused.
        use exr::meta::attribute::Chromaticities;
        use exr::prelude::{
            AnyChannel, AnyChannels, Blocks, Encoding, FlatSamples, ImageAttributes, IntegerBounds, Layer, LayerAttributes, LineOrder, WritableImage,
        };
        let p3 = [0.68_f32, 0.32, 0.265, 0.69, 0.15, 0.06, 0.3127, 0.329];
        let v = exr::math::Vec2;
        let mut attrs = ImageAttributes::new(IntegerBounds::new((0, 0), (2, 2)));
        attrs.chromaticities = Some(Chromaticities { red: v(p3[0], p3[1]), green: v(p3[2], p3[3]), blue: v(p3[4], p3[5]), white: v(p3[6], p3[7]) });
        let list = ["R", "G", "B"].into_iter().map(|n| AnyChannel::new(n, FlatSamples::F32(vec![0.5; 4]))).collect();
        let layer = Layer::new(
            (2, 2),
            LayerAttributes::default(),
            Encoding { compression: exr::prelude::Compression::ZIP1, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
            AnyChannels::sort(list),
        );
        let mut custom = Cursor::new(Vec::new());
        exr::image::Image::from_layers(attrs, vec![layer]).write().to_buffered(&mut custom).unwrap();
        assert_eq!(Decoder::new(Cursor::new(custom.into_inner()), &|| false).unwrap().info().chromaticities, Some(p3));
        let none = Decoder::new(Cursor::new(&bytes), &|| false).unwrap().info();
        assert_eq!(none.chromaticities, None);
    }
}
