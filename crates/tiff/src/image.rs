//! Image layout of an IFD: dimensions, sample format and the strip / tile chunks holding the pixels.

use crate::{Ifd, Result, TiffError, tags};
use serde::{Deserialize, Serialize};

/// How pixel data is split into chunks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    Strips { rows_per_strip: u32 },
    Tiles { tile_width: u32, tile_height: u32 },
}

/// Pixel layout description of an IFD (TIFF 6.0 §3–§8, §15).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: Vec<u16>,
    pub samples_per_pixel: u16,
    pub compression: u16,
    pub photometric: u16,
    /// 1 = chunky (interleaved), 2 = planar.
    pub planar: u16,
    pub predictor: u16,
    /// 1 = unsigned int, 2 = signed int, 3 = IEEE float.
    pub sample_format: u16,
    pub new_subfile_type: u32,
    pub layout: Layout,
    pub offsets: Vec<u64>,
    pub byte_counts: Vec<u64>,
}

/// One strip or tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub index: usize,
    /// Top-left pixel of the chunk.
    pub x: u32,
    pub y: u32,
    /// Nominal chunk size (tiles may extend past the image; strips are clipped to the image height).
    pub width: u32,
    pub height: u32,
    /// Plane index for planar configuration 2 (else 0).
    pub plane: u16,
    pub offset: u64,
    pub len: u64,
}

impl ImageInfo {
    pub fn from_ifd(ifd: &Ifd) -> Result<ImageInfo> {
        let width = ifd.u32(tags::IMAGE_WIDTH).ok_or(TiffError::MissingTag(tags::IMAGE_WIDTH))?;
        let height = ifd.u32(tags::IMAGE_LENGTH).ok_or(TiffError::MissingTag(tags::IMAGE_LENGTH))?;
        if width == 0 || height == 0 {
            return Err(TiffError::Invalid("zero image dimension".into()));
        }
        let samples_per_pixel = ifd.u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1).max(1);
        let bits_per_sample: Vec<u16> =
            ifd.u64s(tags::BITS_PER_SAMPLE).map(|v| v.into_iter().map(|b| b.min(u16::MAX as u64) as u16).collect()).unwrap_or_else(|| vec![1]);
        let bits_per_sample = if bits_per_sample.is_empty() { vec![1] } else { bits_per_sample };
        let planar = ifd.u16(tags::PLANAR_CONFIGURATION).unwrap_or(1);
        let (layout, offsets, byte_counts) = if let Some(tw) = ifd.u32(tags::TILE_WIDTH) {
            let th = ifd.u32(tags::TILE_LENGTH).ok_or(TiffError::MissingTag(tags::TILE_LENGTH))?;
            if tw == 0 || th == 0 {
                return Err(TiffError::Invalid("zero tile size".into()));
            }
            let offs = ifd.u64s(tags::TILE_OFFSETS).or_else(|| ifd.u64s(tags::STRIP_OFFSETS)).ok_or(TiffError::MissingTag(tags::TILE_OFFSETS))?;
            let cnts = ifd
                .u64s(tags::TILE_BYTE_COUNTS)
                .or_else(|| ifd.u64s(tags::STRIP_BYTE_COUNTS))
                .ok_or(TiffError::MissingTag(tags::TILE_BYTE_COUNTS))?;
            (Layout::Tiles { tile_width: tw, tile_height: th }, offs, cnts)
        } else {
            let offs = ifd.u64s(tags::STRIP_OFFSETS).ok_or(TiffError::MissingTag(tags::STRIP_OFFSETS))?;
            let cnts = ifd.u64s(tags::STRIP_BYTE_COUNTS).unwrap_or_default();
            let rps = ifd.u32(tags::ROWS_PER_STRIP).unwrap_or(height).clamp(1, height);
            (Layout::Strips { rows_per_strip: rps }, offs, cnts)
        };
        Ok(ImageInfo {
            width,
            height,
            bits_per_sample,
            samples_per_pixel,
            compression: ifd.u16(tags::COMPRESSION).unwrap_or(1),
            photometric: ifd.u16(tags::PHOTOMETRIC).unwrap_or(1),
            planar,
            predictor: ifd.u16(tags::PREDICTOR).unwrap_or(1),
            sample_format: ifd.u16(tags::SAMPLE_FORMAT).unwrap_or(1),
            new_subfile_type: ifd.u32(tags::NEW_SUBFILE_TYPE).unwrap_or(0),
            layout,
            offsets,
            byte_counts,
        })
    }

    /// Bits of the first sample.
    pub fn bits(&self) -> u16 {
        self.bits_per_sample[0]
    }

    /// Whether this IFD is a reduced-resolution image (NewSubfileType bit 0).
    pub fn is_reduced(&self) -> bool {
        self.new_subfile_type & 1 != 0
    }

    /// Number of chunks across and down per plane.
    pub fn grid(&self) -> (u32, u32) {
        match self.layout {
            Layout::Strips { rows_per_strip } => (1, self.height.div_ceil(rows_per_strip)),
            Layout::Tiles { tile_width, tile_height } => (self.width.div_ceil(tile_width), self.height.div_ceil(tile_height)),
        }
    }

    /// Nominal chunk dimensions.
    pub fn chunk_size(&self) -> (u32, u32) {
        match self.layout {
            Layout::Strips { rows_per_strip } => (self.width, rows_per_strip),
            Layout::Tiles { tile_width, tile_height } => (tile_width, tile_height),
        }
    }

    /// The chunk count the image declares (offsets present, cut to the layout's plane capacity) with the
    /// geometry `chunks` needs to place each one. No descriptors are built, so a pathological strip array
    /// costs it a length check.
    fn chunk_grid(&self) -> (usize, u32, usize) {
        let (across, down) = self.grid();
        let per_plane = across as usize * down as usize;
        let planes = if self.planar == 2 { self.samples_per_pixel as usize } else { 1 };
        (self.offsets.len().min(per_plane.saturating_mul(planes)), across, per_plane)
    }

    fn declared_chunk_count(&self) -> usize {
        self.chunk_grid().0
    }

    /// The one chunk when the image has exactly one, else `None` — and no descriptor vector beyond it. Preview
    /// extraction only serves single-chunk images, so it asks here before serving a hostile multi-strip layout.
    pub fn single_chunk(&self, data_len: u64) -> Option<Chunk> {
        if self.declared_chunk_count() != 1 {
            return None;
        }
        self.chunks(data_len).into_iter().next()
    }

    /// All chunks that have an offset. Missing byte counts are inferred as "to the next offset / end of data"
    /// using `data_len`.
    pub fn chunks(&self, data_len: u64) -> Vec<Chunk> {
        let (n, across, per_plane) = self.chunk_grid();
        let (cw, ch) = self.chunk_size();
        let mut out = Vec::with_capacity(n);
        // the distinct offsets in order, for chunks without a byte count (sorted once, not scanned per chunk)
        let mut sorted: Vec<u64> = Vec::new();
        if (0..n).any(|i| self.byte_counts.get(i).is_none_or(|&l| l == 0)) {
            sorted = self.offsets.clone();
            sorted.sort_unstable();
            sorted.dedup();
        }
        for i in 0..n {
            let plane = i / per_plane.max(1);
            let j = i % per_plane.max(1);
            let (cx, cy) = (j as u32 % across, j as u32 / across);
            let x = cx * cw;
            let y = cy * ch;
            let height = match self.layout {
                Layout::Strips { .. } => ch.min(self.height - y),
                Layout::Tiles { .. } => ch,
            };
            let offset = self.offsets[i];
            let len = match self.byte_counts.get(i) {
                Some(&l) if l > 0 => l,
                _ => {
                    // infer: up to the next larger offset, else end of data
                    let next = sorted.get(sorted.partition_point(|&o| o <= offset)).copied().unwrap_or(data_len);
                    next.saturating_sub(offset)
                }
            };
            out.push(Chunk { index: i, x, y, width: cw, height, plane: plane as u16, offset, len });
        }
        out
    }
}

/// The bytes of a chunk, clipped to the available data (truncated files yield a shorter slice).
/// Returns `None` only when the chunk starts past the end of `data`.
pub fn chunk_bytes<'a>(data: &'a [u8], c: &Chunk) -> Option<&'a [u8]> {
    let start = usize::try_from(c.offset).ok()?;
    if start >= data.len() {
        return None;
    }
    let end = (c.offset.saturating_add(c.len)).min(data.len() as u64) as usize;
    Some(&data[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strips(offsets: Vec<u64>, byte_counts: Vec<u64>) -> ImageInfo {
        ImageInfo {
            width: 1,
            height: offsets.len() as u32,
            bits_per_sample: vec![8],
            samples_per_pixel: 1,
            compression: 1,
            photometric: 1,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Strips { rows_per_strip: 1 },
            offsets,
            byte_counts,
        }
    }

    /// One offset is one chunk, and `single_chunk` refuses to build descriptors when the image declares more.
    #[test]
    fn single_chunk_is_the_only_chunk() {
        let one = strips(vec![50], vec![10]);
        assert_eq!(one.single_chunk(100).map(|c| (c.index, c.offset, c.len)), one.chunks(100).first().map(|c| (c.index, c.offset, c.len)));
        // the inferred length matches too, and never falls back past the data
        let inferred = strips(vec![70], vec![]);
        assert_eq!(inferred.single_chunk(100).map(|c| c.len), Some(30));
        assert!(strips(vec![50, 10], vec![10, 5]).single_chunk(100).is_none(), "a multi-chunk image has no single chunk");
        assert!(strips(Vec::new(), Vec::new()).single_chunk(100).is_none());
    }

    #[test]
    fn missing_byte_counts_run_to_the_next_larger_offset() {
        let info = strips(vec![50, 10, 30, 10, 70], vec![0, 0, 5]);
        let lens: Vec<u64> = info.chunks(100).iter().map(|c| c.len).collect();
        assert_eq!(lens, vec![20, 20, 5, 20, 30]);
    }

    /// A million strips without byte counts (offsets in descending order) are measured in one sort, not one
    /// scan of all offsets per strip.
    #[test]
    fn many_strips_without_byte_counts_are_fast() {
        let n = 1_000_000u64;
        let info = strips((0..n).rev().map(|i| 1000 + i * 2).collect(), Vec::new());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let chunks = info.chunks(1000 + n * 2);
            let _ = tx.send(chunks.iter().map(|c| c.len).collect::<Vec<u64>>());
        });
        let lens = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("chunks() took longer than 20 s");
        assert!(lens.iter().all(|&l| l == 2));
    }
}
