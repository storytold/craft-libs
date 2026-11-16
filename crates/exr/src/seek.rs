//! Selective reading of flat OpenEXR parts from a seekable source (feature `decode`): the headers
//! are indexed once, then any channels, any display rows and a reduced resolution are read from
//! the file without loading the rest of it.
//!
//! - [`Index::build`] reads the headers and the offset tables only, through `Read + Seek`, after
//!   checking what they declare against [`Limits`] and the source's length. It allocates nothing
//!   sized by pixels, and the index holds no pixel data, so it can be kept per file.
//! - [`read_bands`] reads what a [`Request`] asks for and delivers it incrementally: each
//!   compression block that has been decoded becomes [`Band`]s (one per requested channel: a
//!   rectangle of rows) handed to a callback, in file order, as soon as the block is decoded. A
//!   caller that writes into tiles or shows lines while the rest is still being read never holds
//!   a whole plane.
//! - [`read_planes`] is a collector over [`read_bands`]: the same request delivered as one
//!   display-window plane per channel once everything is read.
//!
//! A request names channels as `(part, channel)` pairs (positions in [`Index::parts`]), a range of
//! display-window rows, and a level: level `n` keeps every `2^n`-th row and column of the display
//! window, so a reduced picture is made without allocating the full-resolution one. Only the
//! chunks that hold a requested channel and a requested row are fetched, chunks that follow each
//! other in the file in one read, and they are decompressed on several threads while the next
//! ones are being read. Samples are converted to `f32`; outside a part's data window (and in rows
//! no chunk holds) a plane has the channel's fill value.
//!
//! Deep parts are not read here (see [`crate::deep`]); a flat part beside them is. Tiled parts
//! are read from their largest resolution level. Every size a header declares is checked before
//! it is used, every allocation is bounded by the caller's [`Request::max_bytes`] or reserved
//! fallibly, and nothing panics on hostile input.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use exr::block::UncompressedBlock;
use exr::block::chunk::Chunk;
use exr::meta::MetaData;
use exr::meta::attribute::SampleType as ExrSample;
use exr::meta::header::Header;

use crate::decode::{Limits, PartInfo, Window, check_previews, part_info};

/// Why a read failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is damaged, or a chunk can't be decompressed.
    #[error("OpenEXR: {0}")]
    Malformed(String),
    /// A valid file this reader does not cover (HTJ2K compression, subsampled channels).
    #[error("unsupported OpenEXR: {0}")]
    Unsupported(String),
    /// The file declares more than [`Limits`] allow, or the request needs more than
    /// [`Request::max_bytes`] (or than the system can give).
    #[error("OpenEXR exceeds a limit: {0}")]
    LimitExceeded(String),
    /// The source failed just now (a seek or read error): another attempt may succeed.
    #[error("OpenEXR: cannot read: {0}")]
    Io(String),
    /// The `cancel` callback asked to stop.
    #[error("OpenEXR: cancelled")]
    Cancelled,
}

/// Result of this module's functions.
pub type Result<T> = std::result::Result<T, Error>;

fn malformed(e: impl std::fmt::Display) -> Error {
    Error::Malformed(e.to_string())
}

fn io(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}

fn exr_error(e: exr::error::Error) -> Error {
    match e {
        exr::error::Error::NotSupported(message) => Error::Unsupported(message.to_string()),
        exr::error::Error::Io(e) => io(e),
        e => malformed(e),
    }
}

const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];
/// Headers of all parts together: at most this many bytes, parts, and attributes per part.
const META_BYTES: usize = 8 << 20;
const MAX_PARTS: usize = 1024;
const MAX_ATTRIBUTES: usize = 4096;
/// The most bytes one uncompressed block of a part may take: the decompressor allocates that much
/// for a block before it looks at the data, so a hostile header (a data window a billion pixels
/// wide over one tiny chunk) would otherwise ask for gigabytes.
const MAX_BLOCK_BYTES: usize = 256 << 20;
/// Chunks are read in pieces: a run of chunks that follow each other in the file, read with one
/// request (one round trip to a network share, not one per chunk). The first piece is small so
/// the first bands arrive early; each later one may be twice as large, up to [`PIECE_BYTES`], so
/// a long read makes few requests. A piece holds at most [`PIECE_CHUNKS`] chunks, and a single
/// chunk larger than the limit is read alone.
const FIRST_PIECE_BYTES: u64 = 256 << 10;
const PIECE_BYTES: u64 = 4 << 20;
const PIECE_CHUNKS: usize = 256;

/// One chunk of a part, in offset-table order.
#[derive(Debug, Clone, Copy)]
struct ChunkRef {
    /// Where it starts in the file, and how many bytes to read for it (0: its offset is not valid,
    /// it is never read).
    offset: u64,
    len: u64,
    /// Its first row in the part's data window and how many rows it holds.
    y: i64,
    rows: usize,
    /// Of the full-resolution level (tiled files also store reduced levels).
    largest: bool,
}

/// Where one part's samples go in the display window, and its chunks.
#[derive(Debug, Clone)]
struct PartLayout {
    /// The part's data window position in the display window.
    offset: (i64, i64),
    /// The data window's size.
    size: (usize, usize),
    chunks: Vec<ChunkRef>,
}

/// What an OpenEXR file holds, from its headers and offset tables: the facts [`read_bands`] needs
/// to fetch only the chunks a request touches. Holds no pixel data.
#[derive(Debug, Clone)]
pub struct Index {
    meta: MetaData,
    parts: Vec<PartInfo>,
    layout: Vec<PartLayout>,
    display: Window,
    len: u64,
}

impl Index {
    /// Reads the headers and offset tables of the file in `source` and checks them against
    /// `limits` (the window and pixel limits; `max_bytes` and `max_ratio` bound what the whole
    /// file may decode to in [`crate::decode`] and play no part here, a read is bounded by
    /// [`Request::max_bytes`]). Nothing is sized by pixels. `cancel` is polled while reading.
    pub fn build<R: Read + Seek + ?Sized>(source: &mut R, limits: &Limits, cancel: &dyn Fn() -> bool) -> Result<Index> {
        let len = source.seek(SeekFrom::End(0)).map_err(io)?;
        source.seek(SeekFrom::Start(0)).map_err(io)?;
        let prefix = read_headers(source, cancel)?;
        check_previews(&prefix).map_err(malformed)?;
        // (the crate's own parsing panics on some malformed headers: reported, never raised)
        let meta = catch_unwind(AssertUnwindSafe(|| MetaData::read_from_buffered(Cursor::new(&prefix), false)))
            .map_err(|_| malformed("invalid OpenEXR headers"))?
            .map_err(exr_error)?;
        if meta.headers.is_empty() || meta.headers.len() > MAX_PARTS {
            return Err(malformed("part count"));
        }
        for (i, h) in meta.headers.iter().enumerate() {
            if matches!(h.compression, exr::prelude::Compression::HTJ2K32 | exr::prelude::Compression::HTJ2K256) {
                return Err(Error::Unsupported("HTJ2K compression".into()));
            }
            check_part(i, h, limits)?;
        }
        let first = meta.headers.first().ok_or_else(|| malformed("missing header"))?;
        let display = first.shared_attributes.display_window;
        let (dw, dh) = (display.size.0, display.size.1);
        let display_pixels = (dw as u64).saturating_mul(dh as u64);
        if dw == 0 || dh == 0 || dw > limits.max_width as usize || dh > limits.max_height as usize || display_pixels > limits.max_pixels {
            return Err(Error::LimitExceeded(format!("display window of {dw}x{dh}")));
        }
        let tables = read_tables(source, &meta, prefix.len() as u64, len, cancel)?;
        let chunks_start = prefix.len() as u64 + tables.iter().map(|t| t.len() as u64 * 8).sum::<u64>();
        let layout = catch_unwind(AssertUnwindSafe(|| layout_of(&meta, &tables, chunks_start, len)))
            .map_err(|_| malformed("invalid OpenEXR chunk layout"))??;
        let mut parts: Vec<PartInfo> = meta.headers.iter().enumerate().map(|(i, h)| part_info(i, h)).collect();
        // (an index stays small however much a renderer writes into the headers: the text
        // attributes are kept in the part infos, the crate's own copies are dropped)
        let mut meta = meta;
        for h in &mut meta.headers {
            h.shared_attributes.other.clear();
            h.own_attributes.other.clear();
            h.own_attributes.preview = None;
        }
        for (info, h) in parts.iter_mut().zip(&meta.headers) {
            info.deep = h.deep;
        }
        let display = parts.first().map_or(Window { x: 0, y: 0, width: 0, height: 0 }, |p| p.display_window);
        Ok(Index { meta, parts, layout, display, len })
    }

    /// Every part's header facts: channels (in the order a request's `(part, channel)` pairs
    /// count them), windows, compression and text attributes.
    pub fn parts(&self) -> &[PartInfo] {
        &self.parts
    }

    /// The display window of the file (the first part's): every plane covers it.
    pub fn display(&self) -> Window {
        self.display
    }

    /// The file's length when it was indexed.
    pub fn file_len(&self) -> u64 {
        self.len
    }

    /// About the bytes the index holds in memory.
    pub fn bytes(&self) -> usize {
        let chunks: usize = self.layout.iter().map(|p| p.chunks.len()).sum();
        let infos: usize = self
            .parts
            .iter()
            .map(|p| {
                let channels: usize = p.channels.iter().map(|c| c.name.len() + 48).sum();
                let text: usize = p.text_attributes.iter().map(|(k, v)| k.len() + v.len() + 48).sum();
                channels + text + p.compression.len() + 256
            })
            .sum();
        chunks.saturating_mul(std::mem::size_of::<ChunkRef>()).saturating_add(infos).saturating_add(self.meta.headers.len().saturating_mul(1024))
    }
}

/// The headers of the file in `r` (up to and including the empty name that ends them), as raw
/// bytes: bounded in size, parts and attributes before anything parses them.
fn read_headers<R: Read + ?Sized>(r: &mut R, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let mut start = [0u8; 8];
    r.read_exact(&mut start).map_err(malformed)?;
    if start.get(..4) != Some(&MAGIC[..]) {
        return Err(malformed("OpenEXR signature"));
    }
    let version = start.get(4..8).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_le_bytes).ok_or_else(|| malformed("version"))?;
    let multipart = version & 0x1000 != 0;
    let mut prefix = start.to_vec();
    let mut headers = 0usize;
    let mut pending: Option<u8> = None;
    loop {
        let mut attributes = 0usize;
        // One header: attributes (name, type, size, value) up to an empty name.
        loop {
            if cancel() {
                return Err(Error::Cancelled);
            }
            if !read_name(r, &mut prefix, pending.take())? {
                break;
            }
            attributes += 1;
            if attributes > MAX_ATTRIBUTES {
                return Err(Error::LimitExceeded("more than 4096 attributes in a header".into()));
            }
            read_name(r, &mut prefix, None)?;
            let mut size = [0u8; 4];
            r.read_exact(&mut size).map_err(malformed)?;
            prefix.extend_from_slice(&size);
            let n = usize::try_from(i32::from_le_bytes(size)).map_err(malformed)?;
            let end =
                prefix.len().checked_add(n).filter(|end| *end <= META_BYTES).ok_or_else(|| Error::LimitExceeded("headers over 8 MiB".into()))?;
            let begin = prefix.len();
            prefix.try_reserve_exact(n).map_err(|_| Error::LimitExceeded("header allocation".into()))?;
            prefix.resize(end, 0);
            for bytes in prefix.get_mut(begin..).ok_or_else(|| malformed("attribute size"))?.chunks_mut(64 << 10) {
                if cancel() {
                    return Err(Error::Cancelled);
                }
                r.read_exact(bytes).map_err(malformed)?;
            }
        }
        headers += 1;
        if headers > MAX_PARTS {
            return Err(Error::LimitExceeded("more than 1024 parts".into()));
        }
        if !multipart {
            return Ok(prefix);
        }
        // A multi-part header list ends with one more empty name.
        let mut next = [0u8; 1];
        r.read_exact(&mut next).map_err(malformed)?;
        match next.first().copied() {
            Some(0) | None => {
                prefix.push(0);
                return Ok(prefix);
            }
            Some(b) => pending = Some(b),
        }
    }
}

/// One NUL-terminated name (at most 255 bytes) appended to `prefix`, `first` already read; `false`
/// for the empty name that ends a header.
fn read_name<R: Read + ?Sized>(r: &mut R, prefix: &mut Vec<u8>, first: Option<u8>) -> Result<bool> {
    let mut i = 0usize;
    let mut next = first;
    loop {
        let b = match next.take() {
            Some(b) => b,
            None => {
                let mut one = [0u8; 1];
                r.read_exact(&mut one).map_err(malformed)?;
                one.first().copied().unwrap_or(0)
            }
        };
        prefix.push(b);
        if b == 0 {
            return Ok(i != 0);
        }
        i += 1;
        if i > 255 {
            return Err(Error::LimitExceeded("an attribute name over 255 bytes".into()));
        }
    }
}

/// What a header may declare before any chunk of the part is sized by it.
fn check_part(index: usize, h: &Header, limits: &Limits) -> Result<()> {
    if h.deep {
        // (flat parts beside it are read; a request for a channel of this one is refused)
        return Ok(());
    }
    let (w, ht) = (h.layer_size.0, h.layer_size.1);
    if w > limits.max_width as usize || ht > limits.max_height as usize || (w as u64).saturating_mul(ht as u64) > limits.max_pixels {
        return Err(Error::LimitExceeded(format!("part {index} of {w}x{ht}")));
    }
    if h.channels.list.is_empty() || h.channels.list.len() > 4096 {
        return Err(malformed(format!("channel count of part {index}")));
    }
    block_bytes(h).ok_or_else(|| Error::LimitExceeded(format!("a block of part {index} is over {MAX_BLOCK_BYTES} bytes")))?;
    Ok(())
}

/// The bytes of one uncompressed block of `h` (its pixel bytes times a block's pixels, checked):
/// `None` past [`MAX_BLOCK_BYTES`] or when it doesn't fit in a `usize`.
fn block_bytes(h: &Header) -> Option<usize> {
    let (w, rows) = match h.blocks {
        exr::meta::BlockDescription::Tiles(t) => (t.tile_size.0, t.tile_size.1),
        exr::meta::BlockDescription::ScanLines => (h.layer_size.0, h.compression.scan_lines_per_block().min(h.layer_size.1.max(1))),
    };
    let per_pixel = h.channels.list.iter().map(|c| c.sample_type.bytes_per_sample()).try_fold(0usize, usize::checked_add)?;
    per_pixel.checked_mul(w)?.checked_mul(rows).filter(|b| *b <= MAX_BLOCK_BYTES)
}

/// The offset tables, one per part, read from where the headers end.
fn read_tables<R: Read + Seek + ?Sized>(r: &mut R, meta: &MetaData, headers_end: u64, len: u64, cancel: &dyn Fn() -> bool) -> Result<Vec<Vec<u64>>> {
    // Eight bytes of table per chunk: a header can't claim more chunks than the file holds.
    let chunks: u64 = meta.headers.iter().map(|h| h.chunk_count as u64).fold(0, u64::saturating_add);
    if chunks.saturating_mul(8) > len.saturating_sub(headers_end) {
        return Err(malformed("offset tables longer than the file"));
    }
    r.seek(SeekFrom::Start(headers_end)).map_err(io)?;
    let mut tables = Vec::with_capacity(meta.headers.len());
    for h in &meta.headers {
        let mut table: Vec<u64> = Vec::new();
        table.try_reserve_exact(h.chunk_count).map_err(|_| Error::LimitExceeded("offset table allocation".into()))?;
        let mut buf = vec![0u8; 8 * 8192];
        let mut left = h.chunk_count;
        while left > 0 {
            if cancel() {
                return Err(Error::Cancelled);
            }
            let take = left.min(8192);
            let bytes = buf.get_mut(..take * 8).ok_or_else(|| malformed("table buffer"))?;
            r.read_exact(bytes).map_err(malformed)?;
            table.extend(bytes.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)));
            left -= take;
        }
        tables.push(table);
    }
    Ok(tables)
}

/// Each part's chunks: where they are and which rows they hold. A chunk ends where the next one
/// in the file starts (or at the end of the file); an offset outside the chunk area is kept
/// as a chunk that is never read.
fn layout_of(meta: &MetaData, tables: &[Vec<u64>], chunks_start: u64, len: u64) -> Result<Vec<PartLayout>> {
    let first = meta.headers.first().ok_or_else(|| malformed("missing header"))?;
    let display = first.shared_attributes.display_window;
    let mut starts: Vec<u64> = tables.iter().flatten().copied().filter(|o| *o >= chunks_start && *o < len).collect();
    starts.sort_unstable();
    starts.dedup();
    let end_of = |offset: u64| match starts.binary_search(&offset) {
        Ok(i) => starts.get(i + 1).copied().unwrap_or(len),
        Err(_) => offset,
    };
    let mut out = Vec::with_capacity(meta.headers.len());
    for (pi, h) in meta.headers.iter().enumerate() {
        let pos = h.own_attributes.layer_position;
        let offset = (i64::from(pos.0) - i64::from(display.position.0), i64::from(pos.1) - i64::from(display.position.1));
        if h.deep {
            out.push(PartLayout { offset, size: (0, 0), chunks: Vec::new() });
            continue;
        }
        let table = tables.get(pi).ok_or_else(|| malformed("offset table"))?;
        // A chunk is at most its block's bytes plus its own small header (part number, position
        // and sizes): never read more for one.
        let most = (block_bytes(h).ok_or_else(|| malformed("block size"))? as u64).saturating_add(64);
        let blocks = h.blocks_increasing_y_order();
        if blocks.len() != h.chunk_count {
            return Err(malformed(format!("chunk count of part {pi}")));
        }
        let mut chunks = Vec::with_capacity(h.chunk_count);
        for (i, tile) in blocks.enumerate() {
            let bounds = h.get_absolute_block_pixel_coordinates(tile.location).map_err(exr_error)?;
            let at = table.get(i).copied().unwrap_or(0);
            let valid = at >= chunks_start && at < len;
            let bytes = if valid { end_of(at).saturating_sub(at).min(most) } else { 0 };
            chunks.push(ChunkRef {
                offset: at,
                len: bytes,
                y: i64::from(bounds.position.1),
                rows: bounds.size.1,
                largest: tile.location.is_largest_resolution_level(),
            });
        }
        out.push(PartLayout { offset, size: (h.layer_size.0, h.layer_size.1), chunks });
    }
    Ok(out)
}

/// What to read.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// The channels wanted as `(part, channel)` (positions in [`Index::parts`] and in the part's
    /// [`PartInfo::channels`]), in the order [`Band::slot`] and the planes of [`read_planes`] count
    /// them. A channel of a deep part is refused.
    pub channels: &'a [(usize, usize)],
    /// The value of each requested channel (same order) where the file has no samples; missing
    /// entries count as 0.
    pub fill: &'a [f32],
    /// The rows of the display window (full resolution) to read; `None`: all.
    pub rows: Option<Range<i64>>,
    /// Every `2^level`-th row and column of the display window (0: full resolution, at most 16).
    pub level: u32,
    /// The most bytes of planes [`read_planes`] may allocate (and the check [`read_bands`] makes
    /// against what the request would take as planes).
    pub max_bytes: usize,
}

impl<'a> Request<'a> {
    /// Every row of `channels` at full resolution, fill 0, up to `max_bytes` of planes.
    pub fn new(channels: &'a [(usize, usize)], max_bytes: usize) -> Request<'a> {
        Request { channels, fill: &[], rows: None, level: 0, max_bytes }
    }
}

/// The size of the planes of a read at `level` of a `width` × `height` display window.
pub fn level_size((width, height): (usize, usize), level: u32) -> (usize, usize) {
    let stride = 1usize << level.min(16);
    (width.div_ceil(stride), height.div_ceil(stride))
}

/// A rectangle of rows of one requested channel, decoded: `samples` holds `height` rows of
/// `width` values, row by row. `x` and `y` are in the output grid: display-window pixels at level
/// 0, every `2^level`-th pixel above.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    /// The position of the channel in [`Request::channels`].
    pub slot: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub samples: Vec<f32>,
}

/// What a read did, for measurements and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Chunks fetched, per part.
    pub chunks: Vec<usize>,
    /// Bytes of chunks read from the source.
    pub bytes: u64,
    /// Scan lines (or tile rows) decompressed.
    pub lines: usize,
}

/// The planes of [`read_planes`]: one per requested channel, `width` × `height` (the display
/// window at the request's level).
#[derive(Debug, Clone, PartialEq)]
pub struct Planes {
    pub width: usize,
    pub height: usize,
    pub planes: Vec<Vec<f32>>,
    pub stats: Stats,
}

/// The planes' geometry for [`chunk_bands`].
struct Geometry<'a> {
    /// The display window's size and the output's (at the level).
    size: (usize, usize),
    out: (usize, usize),
    stride: i64,
    /// The display rows read.
    rows: Range<i64>,
    /// Each part's channel → request slot.
    slots: &'a [Vec<Option<usize>>],
}

impl Geometry<'_> {
    /// A display row is read: in the rows, on the level's grid.
    fn wanted(&self, row: i64) -> bool {
        row >= self.rows.start && row < self.rows.end && row.rem_euclid(self.stride) == 0
    }

    /// A chunk holding display rows `a..b` has a row read.
    fn any_wanted(&self, a: i64, b: i64) -> bool {
        let first = a.max(self.rows.start);
        let first = first + (self.stride - first.rem_euclid(self.stride)) % self.stride;
        first < b.min(self.rows.end)
    }
}

/// The bands of what `request` asks for, handed to `sink` one decoded block at a time, in file
/// order.
///
/// The chunks are read on the calling thread, in pieces of chunks that follow each other in the
/// file, and decompressed on worker threads while the next piece is read. Between pieces the
/// calling thread hands every band decoded so far to `sink`, so the first bands arrive after the
/// first few hundred kilobytes have been read, not after the whole request, and `sink` needs to be
/// neither `Send` nor `Sync`. At most one piece is read ahead of what has been delivered, which
/// keeps the memory a read holds at about two pieces whatever the file's size. Where threads
/// can't be started (WebAssembly), the chunks are decoded on the calling thread instead. `cancel`
/// is polled before each piece and while waiting for the workers.
///
/// A band lies inside the planes the request describes ([`level_size`] of the display window);
/// rows or columns no chunk or data window covers are not delivered (they keep the channel's
/// fill value in [`read_planes`]).
pub fn read_bands<R: Read + Seek + ?Sized>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &(dyn Fn() -> bool + Sync),
    sink: &mut dyn FnMut(Band),
) -> Result<Stats> {
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    read_bands_on(source, index, request, cancel, sink, workers)
}

/// [`read_bands`] with at most `workers` decoding threads (0: decode on the calling thread, as
/// where threads can't be started; the tests use it to cover that path natively).
pub(crate) fn read_bands_on<R: Read + Seek + ?Sized>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &(dyn Fn() -> bool + Sync),
    sink: &mut dyn FnMut(Band),
    workers: usize,
) -> Result<Stats> {
    let mut stats = Stats { chunks: vec![0; index.layout.len()], ..Default::default() };
    let level = request.level.min(16);
    let stride = 1i64 << level;
    let (w, h) = (index.display.width as usize, index.display.height as usize);
    let out = level_size((w, h), level);
    let need = plane_bytes(out, request.channels.len());
    if need > request.max_bytes {
        return Err(Error::LimitExceeded(format!("{w}×{h} channels need {} bytes, more than the {} the read may use", need, request.max_bytes)));
    }
    // Per part: the request slot of each of its channels (`None`: not requested).
    let mut slots: Vec<Vec<Option<usize>>> = index.parts.iter().map(|p| vec![None; p.channels.len()]).collect();
    for (slot, (part, channel)) in request.channels.iter().enumerate() {
        let info = index.parts.get(*part).ok_or_else(|| malformed("no such part"))?;
        if info.deep {
            return Err(Error::Unsupported("a channel of a deep part".into()));
        }
        let c = info.channels.get(*channel).ok_or_else(|| malformed("no such channel"))?;
        if c.sampling != (1, 1) {
            return Err(Error::Unsupported("a subsampled channel".into()));
        }
        if let Some(s) = slots.get_mut(*part).and_then(|p| p.get_mut(*channel)) {
            *s = Some(slot);
        }
    }
    let rows = request.rows.clone().unwrap_or(0..h as i64);
    let geometry = Geometry { size: (w, h), out, stride, rows: rows.start.max(0)..rows.end.min(h as i64), slots: &slots };
    // The chunks to read, in file order.
    let mut jobs: Vec<(usize, u64, u64)> = Vec::new();
    for (pi, part) in index.layout.iter().enumerate() {
        if !slots.get(pi).is_some_and(|s| s.iter().any(Option::is_some)) {
            continue;
        }
        for c in &part.chunks {
            let top = c.y.saturating_add(part.offset.1);
            if !c.largest || !geometry.any_wanted(top, top.saturating_add(c.rows as i64)) {
                continue;
            }
            if c.len == 0 {
                return Err(malformed("a chunk offset outside the file"));
            }
            jobs.push((pi, c.offset, c.len));
        }
    }
    jobs.sort_by_key(|(_, offset, _)| *offset);
    if jobs.is_empty() {
        return Ok(stats);
    }
    let workers = workers.min(jobs.len());
    // (declared outside the scope because the workers borrow the receiving end)
    let (to_workers, queue) = mpsc::channel::<Job>();
    let queue = Mutex::new(queue);
    std::thread::scope(|scope| {
        // Owned by this closure so that every way out of it (an error, a cancel) drops the
        // sending end, which ends the workers' loops before the scope waits for them.
        let to_workers = to_workers;
        let (done, results) = mpsc::channel();
        let mut started = 0usize;
        for _ in 0..workers {
            let (queue, done, geometry) = (&queue, done.clone(), &geometry);
            let worker =
                std::thread::Builder::new().name("exr-decode".into()).spawn_scoped(scope, move || decode_worker(index, geometry, queue, &done));
            started += usize::from(worker.is_ok());
        }
        // (only the workers hold senders now: if they all stop, waiting for a result fails
        // instead of blocking forever)
        drop(done);
        let mut order = InOrder::default();
        let mut pos: Option<u64> = None;
        let mut rest = jobs.as_slice();
        let mut most = FIRST_PIECE_BYTES;
        let mut submitted = 0usize;
        // The sequence number of the first chunk of the piece read last.
        let mut last_piece = 0usize;
        while !rest.is_empty() {
            if cancel() {
                return Err(Error::Cancelled);
            }
            // Read ahead by one piece at most: everything before the piece read last must have
            // reached the sink first. (`order.next` is a chunk not received yet whenever this
            // waits, so a result is on its way.)
            while order.next < last_piece {
                let (seq, result) = results.recv().map_err(|_| malformed("a decoding thread stopped"))?;
                order.accept(seq, result);
                order.deliver(sink, &mut stats)?;
                if cancel() {
                    return Err(Error::Cancelled);
                }
            }
            let (piece, more) = rest.split_at(piece_len(rest, most));
            rest = more;
            most = most.saturating_mul(2).min(PIECE_BYTES);
            let Piece { data, chunks } = read_piece(source, piece, &mut pos, &mut stats)?;
            let data = Arc::new(data);
            last_piece = submitted;
            for (part, range) in chunks {
                let job = Job { seq: submitted, part, data: Arc::clone(&data), range };
                submitted += 1;
                if started == 0 {
                    order.accept(job.seq, decode_job(index, &geometry, &job));
                } else if let Err(mpsc::SendError(job)) = to_workers.send(job) {
                    // (the queue outlives this loop, so a send doesn't fail; if it ever did, the
                    // chunk is decoded here rather than lost)
                    order.accept(job.seq, decode_job(index, &geometry, &job));
                }
            }
            while let Ok((seq, result)) = results.try_recv() {
                order.accept(seq, result);
            }
            order.deliver(sink, &mut stats)?;
        }
        // Everything is read: the rest as it is decoded.
        while order.next < submitted {
            if cancel() {
                return Err(Error::Cancelled);
            }
            let (seq, result) = results.recv().map_err(|_| malformed("a decoding thread stopped"))?;
            order.accept(seq, result);
            order.deliver(sink, &mut stats)?;
        }
        Ok(stats)
    })
}

/// [`read_bands`] collected: one plane per requested channel, the display window at the request's
/// level, filled with [`Request::fill`] where nothing was read. The planes are reserved up front
/// (an allocation the system refuses is [`Error::LimitExceeded`], not an abort), so the cost is
/// known before the first chunk is read.
pub fn read_planes<R: Read + Seek + ?Sized>(source: &mut R, index: &Index, request: &Request, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Planes> {
    let (w, h) = (index.display.width as usize, index.display.height as usize);
    let (ow, oh) = level_size((w, h), request.level.min(16));
    let n = ow.checked_mul(oh).ok_or_else(|| Error::LimitExceeded("display window".into()))?;
    let need = plane_bytes((ow, oh), request.channels.len());
    if need > request.max_bytes {
        return Err(Error::LimitExceeded(format!("{w}×{h} channels need {need} bytes, more than the {} the read may use", request.max_bytes)));
    }
    let mut planes: Vec<Vec<f32>> = Vec::with_capacity(request.channels.len());
    for slot in 0..request.channels.len() {
        let mut plane: Vec<f32> = Vec::new();
        plane.try_reserve_exact(n).map_err(|_| Error::LimitExceeded(format!("{need} bytes of planes: the system can't give that memory")))?;
        plane.resize(n, request.fill.get(slot).copied().unwrap_or(0.0));
        planes.push(plane);
    }
    let stats = read_bands(source, index, request, cancel, &mut |band: Band| {
        let Some(plane) = planes.get_mut(band.slot) else { return };
        for r in 0..band.height {
            let start = (band.y + r).checked_mul(ow).and_then(|row| row.checked_add(band.x));
            let (Some(start), Some(from)) = (start, r.checked_mul(band.width)) else { continue };
            if let (Some(dst), Some(src)) =
                (plane.get_mut(start..start.saturating_add(band.width)), band.samples.get(from..from.saturating_add(band.width)))
            {
                dst.copy_from_slice(src);
            }
        }
    })?;
    Ok(Planes { width: ow, height: oh, planes, stats })
}

/// The bytes of `count` planes of `size`, saturating.
fn plane_bytes((w, h): (usize, usize), count: usize) -> usize {
    w.saturating_mul(h).saturating_mul(count).saturating_mul(std::mem::size_of::<f32>())
}

/// One chunk to decode: its sequence number in file order, its part, and where its bytes lie in
/// the piece it was read with (shared by all the chunks of that piece).
struct Job {
    seq: usize,
    part: usize,
    data: Arc<Vec<u8>>,
    range: Range<usize>,
}

/// What decoding one chunk gives: its bands and the scan lines decompressed.
type Decoded = Result<(Vec<Band>, usize)>;

/// A decoding thread: takes chunks from `queue` until it is closed, and sends each one's bands
/// back through `done` (until nobody listens any more: the read has ended early).
fn decode_worker(index: &Index, g: &Geometry, queue: &Mutex<Receiver<Job>>, done: &Sender<(usize, Decoded)>) {
    loop {
        // (the lock is held while waiting for the next chunk, so the workers take turns waiting;
        // a poisoned lock can't happen, no worker panics while holding it, and would end this one)
        let next = match queue.lock() {
            Ok(queue) => queue.recv(),
            Err(_) => return,
        };
        let Ok(job) = next else { return };
        if done.send((job.seq, decode_job(index, g, &job))).is_err() {
            return;
        }
    }
}

/// The bands of one chunk. A panic in the `exr` crate's decompressors is reported as a damaged
/// chunk, never raised.
fn decode_job(index: &Index, g: &Geometry, job: &Job) -> Decoded {
    let buf = job.data.get(job.range.clone()).unwrap_or_default();
    catch_unwind(AssertUnwindSafe(|| chunk_bands(index, job.part, buf, g))).unwrap_or_else(|_| Err(malformed("a chunk could not be decompressed")))
}

/// Puts decoded chunks back into file order: the workers finish them in any order, the sink gets
/// them by sequence number.
#[derive(Default)]
struct InOrder {
    /// The sequence number of the next chunk to deliver.
    next: usize,
    /// Chunks decoded ahead of it.
    waiting: std::collections::BTreeMap<usize, Decoded>,
}

impl InOrder {
    fn accept(&mut self, seq: usize, decoded: Decoded) {
        self.waiting.insert(seq, decoded);
    }

    /// Hands every chunk that is next in order to `sink`; the first chunk that failed to decode
    /// ends the read with its error (after the chunks before it were delivered).
    fn deliver(&mut self, sink: &mut dyn FnMut(Band), stats: &mut Stats) -> Result<()> {
        while let Some(decoded) = self.waiting.remove(&self.next) {
            self.next += 1;
            let (bands, lines) = decoded?;
            stats.lines = stats.lines.saturating_add(lines);
            for band in bands {
                sink(band);
            }
        }
        Ok(())
    }
}

/// The bytes of a piece, and each of its chunks' part and range in them.
struct Piece {
    data: Vec<u8>,
    chunks: Vec<(usize, Range<usize>)>,
}

/// How many of `jobs` (chunks in file order) the next piece takes: chunks that each start where
/// the one before ends, up to [`PIECE_CHUNKS`] of them or `most` bytes, at least one.
fn piece_len(jobs: &[(usize, u64, u64)], most: u64) -> usize {
    let mut take = 0;
    let mut bytes = 0u64;
    let mut end = None;
    while let Some((_, offset, len)) = jobs.get(take) {
        if take > 0 && (take >= PIECE_CHUNKS || end != Some(*offset) || bytes.saturating_add(*len) > most) {
            break;
        }
        bytes = bytes.saturating_add(*len);
        end = Some(offset.saturating_add(*len));
        take += 1;
    }
    take
}

/// The raw bytes of the chunks `piece` (part, offset, length; in file order, each starting where
/// the one before ends), read with one request after a seek when the source stands elsewhere
/// (`pos`), and each chunk's part and range in them. A chunk the file ends in keeps the bytes
/// there are.
fn read_piece<R: Read + Seek + ?Sized>(source: &mut R, piece: &[(usize, u64, u64)], pos: &mut Option<u64>, stats: &mut Stats) -> Result<Piece> {
    let Some((_, start, _)) = piece.first() else { return Ok(Piece { data: Vec::new(), chunks: Vec::new() }) };
    let start = *start;
    let total = piece.iter().map(|(_, _, len)| *len).fold(0u64, u64::saturating_add);
    let mut data = Vec::new();
    data.try_reserve_exact(usize::try_from(total).map_err(|_| malformed("chunk size"))?)
        .map_err(|_| Error::LimitExceeded("chunk allocation".into()))?;
    if *pos != Some(start) {
        source.seek(SeekFrom::Start(start)).map_err(io)?;
    }
    (&mut *source).take(total).read_to_end(&mut data).map_err(io)?;
    *pos = Some(start.saturating_add(data.len() as u64));
    stats.bytes = stats.bytes.saturating_add(data.len() as u64);
    let mut chunks = Vec::with_capacity(piece.len());
    for (pi, offset, len) in piece {
        let a = usize::try_from(offset.saturating_sub(start)).unwrap_or(usize::MAX).min(data.len());
        let b = usize::try_from(*len).map_or(data.len(), |l| a.saturating_add(l)).min(data.len());
        chunks.push((*pi, a..b));
        if let Some(c) = stats.chunks.get_mut(*pi) {
            *c += 1;
        }
    }
    Ok(Piece { data, chunks })
}

/// Decompress the chunk `buf` of part `pi` and convert the lines of the requested channels to
/// bands (`g.slots`: each channel's request slot): the bands, and the scan lines decompressed. A
/// chunk that names another part than its offset-table slot's is not placed.
fn chunk_bands(index: &Index, pi: usize, buf: &[u8], g: &Geometry) -> Result<(Vec<Band>, usize)> {
    let meta = &index.meta;
    let mut read = buf;
    let chunk = Chunk::read(&mut read, meta).map_err(exr_error)?;
    if chunk.layer_index != pi {
        return Ok((Vec::new(), 0));
    }
    let block = UncompressedBlock::decompress_chunk(chunk, meta, false).map_err(exr_error)?;
    let (Some(header), Some(part)) = (meta.headers.get(pi), index.layout.get(pi)) else { return Ok((Vec::new(), 0)) };
    let slots = g.slots.get(pi).map_or(&[][..], Vec::as_slice);
    let lines = block.index.pixel_size.1;
    let ((lw, lh), (w, h)) = (part.size, g.size);
    let (ow, _) = g.out;
    // Per request slot: the rows converted, as (output row, output column, values).
    let mut rows: std::collections::BTreeMap<usize, Vec<(usize, usize, Vec<f32>)>> = std::collections::BTreeMap::new();
    // (typed: the line reader is generic over the sample type, and an untyped `0` would make the
    // integer buffer `i32`, turning ids from 2^31 up negative)
    let (mut f16s, mut f32s, mut u32s): (Vec<exr::prelude::f16>, Vec<f32>, Vec<u32>) = (Vec::new(), Vec::new(), Vec::new());
    for line in block.lines(&header.channels) {
        let Some(Some(slot)) = slots.get(line.location.channel).copied() else { continue };
        let Some(kind) = header.channels.list.get(line.location.channel).map(|c| c.sample_type) else { continue };
        let (x0, y, count) = (line.location.position.0, line.location.position.1, line.location.sample_count);
        // The samples of data-window row `y`, columns `x0..x0 + count`, at display row `ty` from
        // display column `tx0`; clipped to the display window.
        if y >= lh || x0.checked_add(count).is_none_or(|end| end > lw) {
            continue;
        }
        let Some(ty) = usize::try_from(y as i64 + part.offset.1).ok().filter(|ty| *ty < h) else { continue };
        if !g.wanted(ty as i64) {
            continue;
        }
        let tx0 = x0 as i64 + part.offset.0;
        let skip = usize::try_from(-tx0).unwrap_or(0).min(count);
        let tx = usize::try_from(tx0.max(0)).unwrap_or(usize::MAX);
        let take = (count - skip).min(w.saturating_sub(tx));
        if take == 0 {
            continue;
        }
        let samples: Vec<f32> = match kind {
            ExrSample::F16 => {
                f16s.resize(count, exr::prelude::f16::ZERO);
                line.read_samples_into_slice(&mut f16s).map_err(exr_error)?;
                let mut out = vec![0.0; take];
                if let Some(src) = f16s.get(skip..skip + take) {
                    half::slice::HalfFloatSliceExt::convert_to_f32_slice(src, &mut out);
                }
                out
            }
            ExrSample::F32 => {
                f32s.resize(count, 0.0);
                line.read_samples_into_slice(&mut f32s).map_err(exr_error)?;
                f32s.iter().skip(skip).take(take).copied().collect()
            }
            ExrSample::U32 => {
                u32s.resize(count, 0);
                line.read_samples_into_slice(&mut u32s).map_err(exr_error)?;
                u32s.iter().skip(skip).take(take).map(|s| *s as f32).collect()
            }
        };
        let (row, col, values) = if g.stride == 1 {
            (ty, tx, samples)
        } else {
            // A reduced level: every stride-th column of the row, on the level's grid.
            let stride = g.stride as usize;
            let first = tx.div_ceil(stride) * stride;
            let values: Vec<f32> = samples.iter().enumerate().filter(|(j, _)| (tx + j) % stride == 0).map(|(_, v)| *v).collect();
            (ty / stride, first / stride, values)
        };
        if values.is_empty() || col.saturating_add(values.len()) > ow {
            continue;
        }
        rows.entry(slot).or_default().push((row, col, values));
    }
    // One band per run of consecutive rows of a channel that start in the same column and are as
    // wide as each other (all the rows of a scan-line block or of a tile).
    let mut bands = Vec::new();
    for (slot, mut list) in rows {
        list.sort_by_key(|(row, _, _)| *row);
        let mut current: Option<Band> = None;
        for (row, col, values) in list {
            if let Some(b) = current.as_mut()
                && b.x == col
                && b.width == values.len()
                && b.y + b.height == row
            {
                b.samples.extend_from_slice(&values);
                b.height += 1;
                continue;
            }
            bands.extend(current.take());
            current = Some(Band { slot, x: col, y: row, width: values.len(), height: 1, samples: values });
        }
        bands.extend(current);
    }
    Ok((bands, lines))
}
