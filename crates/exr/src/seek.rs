//! Selective reading of flat OpenEXR parts from a seekable source (feature `decode`): the headers
//! are indexed once, then any channels, any display rows and a reduced resolution are read from
//! the file without loading the rest of it.
//!
//! - [`Index::build`] reads the headers and the offset tables only, through `Read + Seek`, after
//!   checking what they declare against [`Limits`] and the source's length. It allocates nothing
//!   sized by pixels, and the index holds no pixel data, so it can be kept per file.
//! - [`read_bands`] reads what a [`Request`] asks for and delivers it incrementally: each
//!   compression block becomes [`Band`]s (one per requested channel: a rectangle of rows) handed
//!   to a callback on the calling thread, in file order. Blocks are decoded on worker threads; the
//!   calling thread hands over what has been decoded each time it has read a piece of the file
//!   and while it waits, so the first bands arrive long before the request has been read. A
//!   caller that writes into tiles or shows lines while the rest is still being read never holds
//!   a whole plane.
//! - [`read_planes`] is a collector over [`read_bands`]: the same request delivered as one
//!   display-window plane per channel once everything is read.
//!
//! A request names channels as `(part, channel)` pairs (positions in [`Index::parts`]), a range of
//! display-window rows, and a level: level `n` keeps every `2^n`-th row and column of the display
//! window, so a reduced picture is made without allocating the full-resolution one. Rows and band
//! positions count from the display window's top-left corner (0, 0), not in the file's absolute
//! coordinates (which [`PartInfo`]'s windows use). Only the chunks that hold a requested channel
//! and a requested row are fetched, chunks that follow each other in the file in one read, and
//! they are decompressed on several threads while the next ones are being read. Samples are
//! converted to `f32` (integer samples above 2^24 lose precision); outside a part's data window
//! (and in rows no chunk holds) a plane has the channel's fill value.
//!
//! A tile writer (an image editor storing a picture in tiles at its own depth) uses it like this:
//! [`Index::build`], then [`Index::estimate`] to reserve the working memory and size a progress
//! bar (one step per chunk), then [`read_bands_try`] with the colour channels as one group
//! ([`Request::groups`]), so each block arrives as one band of interleaved pixels to write into
//! the tiles it covers. Its sink checks the tile budget or scratch disk before each band and
//! returns its own error to stop the read; the read winds down and hands that error back.
//!
//! Deep parts are not read here (see [`crate::deep`]); a flat part beside them is. Tiled parts
//! are read from their largest resolution level. Every size a header declares is checked before
//! it is used and every allocation is bounded by a limit or reserved fallibly.
//!
//! Panics: the `exr` crate's header parser and decompressors can panic on some damaged input.
//! Where panics unwind (native targets), those calls are wrapped in `catch_unwind` and a panic
//! becomes [`Error::Malformed`]. Where panics abort (`wasm32-unknown-unknown`), `catch_unwind`
//! does nothing, so this module checks beforehand what it knows the crate would panic on: the
//! chunk layout is computed here with checked arithmetic instead of the crate's (which panics on
//! some sizes), the crate's block-size formula is checked for overflow, and a chunk's part,
//! coordinates, level and declared size are checked before the crate reads it. A panic inside
//! the crate's header
//! parsing or a decompressor on damaged data is not ruled out there.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use exr::block::UncompressedBlock;
use exr::block::chunk::Chunk;
use exr::math::Vec2;
use exr::meta::MetaData;
use exr::meta::attribute::SampleType as ExrSample;
use exr::meta::header::Header;

use crate::decode::{Limits, MAX_HEADER_ATTRIBUTES, PartInfo, Window, check_previews, part_info};

/// Why a read failed. More kinds may be added: a `match` needs a catch-all arm.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The file is damaged, or a chunk can't be decompressed.
    #[error("OpenEXR: {0}")]
    Malformed(String),
    /// A valid file this reader does not cover (subsampled channels, channels of deep parts).
    #[error("unsupported OpenEXR: {0}")]
    Unsupported(String),
    /// A valid file whose compression this reader does not decode (HTJ2K): the name of the
    /// compression. An app may hand such a file to another decoder.
    #[error("unsupported OpenEXR compression: {0}")]
    UnsupportedCompression(String),
    /// The file declares more than [`Limits`] (or this module's own bounds) allow, or the request
    /// needs more than [`Request::max_bytes`]: a lasting refusal of this file or request.
    #[error("OpenEXR exceeds a limit: {0}")]
    LimitExceeded(String),
    /// The system could not give memory the read needed (within its limits) just now: another
    /// attempt, with more memory free, may succeed.
    #[error("OpenEXR: out of memory: {0}")]
    OutOfMemory(String),
    /// The source failed just now (a seek or read error other than the file ending early):
    /// another attempt may succeed.
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

/// A failed `read_exact` of the headers or tables: the file ending early is a damaged file, any
/// other error is the source failing.
fn read_failed(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::UnexpectedEof { malformed(e) } else { io(e) }
}

fn exr_error(e: exr::error::Error) -> Error {
    match e {
        exr::error::Error::NotSupported(message) => Error::Unsupported(message.to_string()),
        exr::error::Error::Io(e) => io(e),
        e => malformed(e),
    }
}

const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];
/// Headers of all parts together: at most this many bytes and parts. Attributes per header are
/// bounded by `MAX_HEADER_ATTRIBUTES`.
const META_BYTES: usize = 8 << 20;
const MAX_PARTS: usize = 1024;
/// The most chunks all parts of a file may have together. The offset tables and the index hold
/// a few bytes per chunk, so a header declaring billions of tiny tiles (whose tables a short file
/// can't hold anyway) is refused before anything is sized by it. 16 million chunks is a 64K ×
/// 64K picture in 16 × 16 tiles.
const MAX_CHUNKS: usize = 1 << 24;
/// The most bytes one decompressed block may take in a read that sets no
/// [`Request::max_block_bytes`] and whose [`Request::max_bytes`] is smaller: the decompressor
/// allocates a block's bytes before it looks at the data, so a hostile header (a data window a
/// billion pixels wide over one tiny chunk) would otherwise ask for gigabytes. Such a read allows
/// blocks up to `max(this, max_bytes)`: a caller that budgets more memory can read files with
/// larger blocks (wide DWAB parts with many channels hold 512 MiB a block).
const MAX_BLOCK_BYTES: usize = 256 << 20;
/// Chunks are read in pieces: a run of chunks that follow each other in the file, read with one
/// request (one round trip to a network share, not one per chunk). The first piece is small so
/// the first bands arrive early; each later one may be twice as large, up to [`PIECE_BYTES`], so
/// a long read makes few requests. A piece holds at most [`PIECE_CHUNKS`] chunks, and a single
/// chunk larger than the limit is read alone.
const FIRST_PIECE_BYTES: u64 = 256 << 10;
const PIECE_BYTES: u64 = 4 << 20;
const PIECE_CHUNKS: usize = 256;
/// The most decoded bytes a read keeps in flight: the decompressed blocks and the bands of the
/// chunks read but not yet handed to the sink. Compressed bytes say little about this (a flat
/// colour compresses a thousandfold), so the reader estimates each chunk's decoded size from its
/// header and waits for the sink to catch up before reading more.
const IN_FLIGHT_BYTES: u64 = 64 << 20;
/// How often a wait for the workers polls `cancel`.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// One chunk of a part's full-resolution level, as [`Index::chunks`] lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkInfo {
    /// Where the chunk starts in the file.
    pub offset: u64,
    /// The bytes read for it: up to the next chunk in the file (or the file's end), at most a
    /// block's bytes plus its header; 0 when its offset lies outside the chunk area (past the
    /// file's end, or in the headers or offset tables): a read that needs it fails.
    pub len: u64,
    /// The column of its top-left pixel in the part's data window, counted from the data
    /// window's left edge (0).
    pub x: usize,
    /// The row of its top-left pixel, counted from the data window's top edge (0).
    pub y: usize,
    /// Its width in pixels (a tile at the right edge, or a scan-line block, may be narrower than
    /// the tile size; a scan-line block is as wide as the data window).
    pub width: usize,
    /// Its height in pixels (the last scan-line block or the bottom tiles may hold fewer rows).
    pub height: usize,
}

/// One chunk of the full-resolution level of a part, in offset-table order.
#[derive(Debug, Clone, Copy)]
struct ChunkRef {
    /// Where it starts in the file, and how many bytes to read for it (0: its offset is not valid,
    /// it is never read).
    offset: u64,
    len: u64,
    /// Its top-left pixel in the part's data window (0-based) and its size.
    x: usize,
    y: usize,
    cols: usize,
    rows: usize,
    /// Its tile column and row (scan-line blocks: column 0, row = block number).
    tile: (usize, usize),
}

/// Where one part's samples go in the display window, and its chunks.
#[derive(Debug, Clone)]
struct PartLayout {
    /// The part's data window position in the display window.
    offset: (i64, i64),
    /// The data window's size.
    size: (usize, usize),
    /// The bytes of one pixel of all its channels, uncompressed.
    pixel_bytes: usize,
    /// The chunks of the full-resolution level (smaller levels of tiled parts are never read).
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
    /// [`Limits::max_ratio`] at build: how many bytes one byte of a chunk may decompress to.
    max_ratio: u64,
}

impl Index {
    /// Reads the headers and offset tables of the file in `source` and checks them against
    /// `limits`: the window and pixel limits. `max_bytes` bounds what the whole file may decode
    /// to in [`crate::decode`] and plays no part here (a read is bounded by
    /// [`Request::max_bytes`]); `max_ratio` is kept in the index, and a read refuses a chunk whose
    /// block is more than its bytes can decompress to (see [`Index::check_blocks`]). Nothing is
    /// sized by pixels.
    /// `cancel` is polled while reading.
    /// The display window and every data window are held to `limits.max_pixels`; see
    /// [`Index::build_with_display_limit`] to hold the display window to its own bound.
    pub fn build<R: Read + Seek + ?Sized>(source: &mut R, limits: &Limits, cancel: &dyn Fn() -> bool) -> Result<Index> {
        Index::build_with_display_limit(source, limits, limits.max_pixels, cancel)
    }

    /// [`Index::build`] with the display window held to `max_display_pixels` instead of
    /// `limits.max_pixels`, which then bounds the data windows only: an app may allow parts larger
    /// than the picture it shows (overscan) while keeping the planes it allocates smaller.
    pub fn build_with_display_limit<R: Read + Seek + ?Sized>(
        source: &mut R,
        limits: &Limits,
        max_display_pixels: u64,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Index> {
        let len = source.seek(SeekFrom::End(0)).map_err(io)?;
        source.seek(SeekFrom::Start(0)).map_err(io)?;
        let prefix = read_headers(source, cancel)?;
        check_previews(&prefix).map_err(malformed)?;
        // (the crate's own parsing panics on some malformed headers: reported, never raised,
        // where panics unwind)
        let meta = catch_unwind(AssertUnwindSafe(|| MetaData::read_from_buffered(Cursor::new(&prefix), false)))
            .map_err(|_| malformed("invalid OpenEXR headers"))?
            .map_err(exr_error)?;
        if meta.headers.is_empty() || meta.headers.len() > MAX_PARTS {
            return Err(malformed("part count"));
        }
        for (i, h) in meta.headers.iter().enumerate() {
            if matches!(h.compression, exr::prelude::Compression::HTJ2K32 | exr::prelude::Compression::HTJ2K256) {
                return Err(Error::UnsupportedCompression("HTJ2K".into()));
            }
            check_part(i, h, limits)?;
        }
        let first = meta.headers.first().ok_or_else(|| malformed("missing header"))?;
        let display = first.shared_attributes.display_window;
        let (dw, dh) = (display.size.0, display.size.1);
        let display_pixels = (dw as u64).saturating_mul(dh as u64);
        if dw == 0 || dh == 0 || dw > limits.max_width as usize || dh > limits.max_height as usize || display_pixels > max_display_pixels {
            return Err(Error::LimitExceeded(format!("display window of {dw}x{dh}")));
        }
        let tables = read_tables(source, &meta, prefix.len() as u64, len, cancel)?;
        let chunks_start = prefix.len() as u64 + tables.iter().map(|t| t.len() as u64 * 8).sum::<u64>();
        let layout = layout_of(&meta, &tables, chunks_start, len)?;
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
        Ok(Index { meta, parts, layout, display, len, max_ratio: limits.max_ratio })
    }

    /// Every part's header facts: channels (in the order a request's `(part, channel)` pairs
    /// count them), windows (in the file's absolute coordinates), compression and text attributes.
    pub fn parts(&self) -> &[PartInfo] {
        &self.parts
    }

    /// The display window of the file (the first part's, in absolute coordinates): every plane
    /// covers it, and a request's rows and a band's position count from its top-left corner.
    pub fn display(&self) -> Window {
        self.display
    }

    /// Stored RGB primaries and white point (red, green, blue, white in CIE xy), from the first
    /// part that supplies the shared chromaticities attribute.
    pub fn chromaticities(&self) -> Option<[[f32; 2]; 4]> {
        let c = self.meta.headers.iter().find_map(|header| header.shared_attributes.chromaticities.as_ref())?;
        Some([c.red, c.green, c.blue, c.white].map(|v| [v.0, v.1]))
    }

    /// The chunks of part `part`'s full-resolution level, in offset-table order (none for a deep
    /// part or a part that doesn't exist): where each lies in the file and which pixels of the
    /// part's data window it holds. For diagnostics and tests; [`read_bands`] picks the chunks a
    /// request needs by itself.
    pub fn chunks(&self, part: usize) -> impl Iterator<Item = ChunkInfo> + '_ {
        self.layout.get(part).map_or(&[][..], |p| p.chunks.as_slice()).iter().map(|c| ChunkInfo {
            offset: c.offset,
            len: c.len,
            x: c.x,
            y: c.y,
            width: c.cols,
            height: c.rows,
        })
    }

    /// Checks every chunk of the full-resolution level of every flat part against what its bytes
    /// can decompress to (see [`Index::build`]'s `max_ratio`), from the index alone: an app can
    /// refuse a file before another decoder allocates by its headers. A chunk's bytes are counted
    /// here as the gap to the next chunk in the file (the index holds no payload sizes); a read
    /// makes the same check for the chunks it needs, and again, once a chunk is read, against the
    /// payload size the chunk declares, and also holds each block to its own byte limit.
    pub fn check_blocks(&self) -> Result<()> {
        for (pi, part) in self.layout.iter().enumerate() {
            let Some(header) = self.meta.headers.get(pi).filter(|h| !h.deep) else { continue };
            for c in &part.chunks {
                self.check_ratio(pi, header, part, c, c.len)?;
            }
        }
        Ok(())
    }

    /// Refuses chunk `c` of part `pi` when its decompressed block is more than `len` bytes can
    /// hold: at most `max_ratio` bytes per byte (four times that for DWA, whose RLE stage, up to
    /// 64 : 1, and zlib, up to about 1032 : 1, together reach about 66 000 : 1), plus 1 MiB. A
    /// hostile header can declare a block of gigabytes over a chunk of a few bytes, and the
    /// decompressors allocate the whole block first.
    fn check_ratio(&self, pi: usize, header: &Header, part: &PartLayout, c: &ChunkRef, len: u64) -> Result<()> {
        let block = (c.cols as u64).saturating_mul(c.rows as u64).saturating_mul(part.pixel_bytes as u64);
        let ratio = if crate::dwa_guard::is_dwa(header.compression) { self.max_ratio.saturating_mul(4) } else { self.max_ratio };
        let most = len.checked_mul(ratio).and_then(|b| b.checked_add(1 << 20)).unwrap_or(u64::MAX);
        if block > most {
            return Err(Error::LimitExceeded(format!(
                "a block of part {pi} decompresses to {block} bytes, more than its {len}-byte chunk can hold ({most})"
            )));
        }
        Ok(())
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
    r.read_exact(&mut start).map_err(read_failed)?;
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
            if attributes > MAX_HEADER_ATTRIBUTES {
                return Err(Error::LimitExceeded(format!("more than {MAX_HEADER_ATTRIBUTES} attributes in a header")));
            }
            read_name(r, &mut prefix, None)?;
            let mut size = [0u8; 4];
            r.read_exact(&mut size).map_err(read_failed)?;
            prefix.extend_from_slice(&size);
            let n = usize::try_from(i32::from_le_bytes(size)).map_err(malformed)?;
            let end =
                prefix.len().checked_add(n).filter(|end| *end <= META_BYTES).ok_or_else(|| Error::LimitExceeded("headers over 8 MiB".into()))?;
            let begin = prefix.len();
            prefix.try_reserve_exact(n).map_err(|_| Error::OutOfMemory("header allocation".into()))?;
            prefix.resize(end, 0);
            for bytes in prefix.get_mut(begin..).ok_or_else(|| malformed("attribute size"))?.chunks_mut(64 << 10) {
                if cancel() {
                    return Err(Error::Cancelled);
                }
                r.read_exact(bytes).map_err(read_failed)?;
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
        r.read_exact(&mut next).map_err(read_failed)?;
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
                r.read_exact(&mut one).map_err(read_failed)?;
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
    if w == 0 || ht == 0 {
        return Err(malformed(format!("empty data window of part {index}")));
    }
    if w > limits.max_width as usize || ht > limits.max_height as usize || (w as u64).saturating_mul(ht as u64) > limits.max_pixels {
        return Err(Error::LimitExceeded(format!("part {index} of {w}x{ht}")));
    }
    if h.channels.list.is_empty() || h.channels.list.len() > 4096 {
        return Err(malformed(format!("channel count of part {index}")));
    }
    // (the decompressor's actual block is no larger than the crate's bound, so it fits too; how
    // large a block may be depends on the read: see `read_bands`)
    if !crate_bound_fits(h) {
        return Err(malformed(format!("the block size of part {index} overflows")));
    }
    Ok(())
}

/// The bytes of all channels of one pixel of `h`, checked.
fn pixel_bytes(h: &Header) -> Option<usize> {
    h.channels.list.iter().map(|c| c.sample_type.bytes_per_sample()).try_fold(0usize, usize::checked_add)
}

/// The bytes of the largest block the decompressor allocates for a chunk of `h`: a pixel's bytes
/// times a tile's area, or times the width and the rows of a scan-line block (no more than the
/// part has). `None` when it overflows.
fn block_bytes(h: &Header) -> Option<usize> {
    let pixels = match h.blocks {
        exr::meta::BlockDescription::Tiles(t) => t.tile_size.0.checked_mul(t.tile_size.1)?,
        exr::meta::BlockDescription::ScanLines => h.compression.scan_lines_per_block().min(h.layer_size.1).checked_mul(h.layer_size.0)?,
    };
    pixel_bytes(h)?.checked_mul(pixels)
}

/// Whether the `exr` crate's own bound on a chunk of `h` (`Header::max_block_byte_size`: like
/// [`block_bytes`] but with every scan line a block may hold, even past the part's last row) can
/// be computed: the crate computes it without overflow checks. It is not a limit here: before
/// the crate reads a chunk, [`check_chunk_header`] has made sure the size the chunk declares is
/// no more than the bytes it holds, so the crate never allocates by that bound.
fn crate_bound_fits(h: &Header) -> bool {
    let pixels = match h.blocks {
        exr::meta::BlockDescription::Tiles(t) => t.tile_size.0.checked_mul(t.tile_size.1),
        exr::meta::BlockDescription::ScanLines => h.compression.scan_lines_per_block().checked_mul(h.layer_size.0),
    };
    pixels.zip(pixel_bytes(h)).and_then(|(pixels, bytes)| pixels.checked_mul(bytes)).is_some()
}

/// The offset tables, one per part, read from where the headers end.
fn read_tables<R: Read + Seek + ?Sized>(r: &mut R, meta: &MetaData, headers_end: u64, len: u64, cancel: &dyn Fn() -> bool) -> Result<Vec<Vec<u64>>> {
    let chunks: usize = meta.headers.iter().map(|h| h.chunk_count).fold(0, usize::saturating_add);
    if chunks > MAX_CHUNKS {
        return Err(Error::LimitExceeded(format!("{chunks} chunks, more than {MAX_CHUNKS}")));
    }
    // Eight bytes of table per chunk: a header can't claim more chunks than the file holds.
    if (chunks as u64).saturating_mul(8) > len.saturating_sub(headers_end) {
        return Err(malformed("offset tables longer than the file"));
    }
    r.seek(SeekFrom::Start(headers_end)).map_err(io)?;
    let mut tables = Vec::new();
    tables.try_reserve_exact(meta.headers.len()).map_err(|_| Error::OutOfMemory("offset table allocation".into()))?;
    let mut buf = vec![0u8; 8 * 8192];
    for h in &meta.headers {
        let mut table: Vec<u64> = Vec::new();
        table.try_reserve_exact(h.chunk_count).map_err(|_| Error::OutOfMemory("offset table allocation".into()))?;
        let mut left = h.chunk_count;
        while left > 0 {
            if cancel() {
                return Err(Error::Cancelled);
            }
            let take = left.min(8192);
            let bytes = buf.get_mut(..take * 8).ok_or_else(|| malformed("table buffer"))?;
            r.read_exact(bytes).map_err(read_failed)?;
            table.extend(bytes.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)));
            left -= take;
        }
        tables.push(table);
    }
    Ok(tables)
}

/// Each part's chunks of the full-resolution level: where they are and which pixels they hold. A
/// chunk ends where the next one in the file starts (or at the end of the file); an offset
/// outside the chunk area is kept as a chunk that is never read.
///
/// The layout is computed here rather than with the `exr` crate's block iterator, which allocates
/// for every level and panics on some sizes. The full-resolution level comes first in the offset
/// table of every level mode (scan lines, single-level tiles, mip and rip maps), its blocks row by
/// row and left to right.
fn layout_of(meta: &MetaData, tables: &[Vec<u64>], chunks_start: u64, len: u64) -> Result<Vec<PartLayout>> {
    let allocation = |_| Error::OutOfMemory("chunk layout allocation".into());
    let first = meta.headers.first().ok_or_else(|| malformed("missing header"))?;
    let display = first.shared_attributes.display_window;
    let mut starts: Vec<u64> = Vec::new();
    starts.try_reserve_exact(tables.iter().map(Vec::len).fold(0, usize::saturating_add)).map_err(allocation)?;
    starts.extend(tables.iter().flatten().copied().filter(|o| *o >= chunks_start && *o < len));
    starts.sort_unstable();
    starts.dedup();
    let end_of = |offset: u64| match starts.binary_search(&offset) {
        Ok(i) => starts.get(i + 1).copied().unwrap_or(len),
        Err(_) => offset,
    };
    let mut out = Vec::new();
    out.try_reserve_exact(meta.headers.len()).map_err(allocation)?;
    for (pi, h) in meta.headers.iter().enumerate() {
        let pos = h.own_attributes.layer_position;
        let offset = (i64::from(pos.0) - i64::from(display.position.0), i64::from(pos.1) - i64::from(display.position.1));
        if h.deep {
            out.push(PartLayout { offset, size: (0, 0), pixel_bytes: 0, chunks: Vec::new() });
            continue;
        }
        let table = tables.get(pi).ok_or_else(|| malformed("offset table"))?;
        // A chunk is at most its block's bytes plus its own small header (part number, position
        // and sizes): never read more for one.
        let most = (block_bytes(h).ok_or_else(|| malformed("block size"))? as u64).saturating_add(64);
        let (w, ht) = (h.layer_size.0, h.layer_size.1);
        let (bw, bh) = match h.blocks {
            exr::meta::BlockDescription::Tiles(t) => (t.tile_size.0, t.tile_size.1),
            exr::meta::BlockDescription::ScanLines => (w, h.compression.scan_lines_per_block()),
        };
        if bw == 0 || bh == 0 {
            return Err(malformed(format!("block size of part {pi}")));
        }
        let across = w.div_ceil(bw);
        let count =
            across.checked_mul(ht.div_ceil(bh)).filter(|n| *n <= table.len()).ok_or_else(|| malformed(format!("chunk count of part {pi}")))?;
        let mut chunks = Vec::new();
        chunks.try_reserve_exact(count).map_err(allocation)?;
        for (i, at) in table.iter().copied().enumerate().take(count) {
            let tile = (i % across, i / across);
            // (inside the data window: a tile index is below the count of blocks across or down)
            let (x, y) = (tile.0.saturating_mul(bw), tile.1.saturating_mul(bh));
            let valid = at >= chunks_start && at < len;
            let bytes = if valid { end_of(at).saturating_sub(at).min(most) } else { 0 };
            chunks.push(ChunkRef { offset: at, len: bytes, x, y, cols: bw.min(w.saturating_sub(x)), rows: bh.min(ht.saturating_sub(y)), tile });
        }
        let pixel_bytes = pixel_bytes(h).ok_or_else(|| malformed("pixel size"))?;
        out.push(PartLayout { offset, size: (w, ht), pixel_bytes, chunks });
    }
    Ok(out)
}

/// What to read.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// The channels wanted as `(part, channel)` (positions in [`Index::parts`] and in the part's
    /// [`PartInfo::channels`]), in the order [`Band::slot`] and the planes of [`read_planes`] count
    /// them. A channel may be named more than once (each slot gets its own bands). A channel of a
    /// deep part is refused.
    pub channels: &'a [(usize, usize)],
    /// Use this part's data window as the output grid, keeping samples beyond the display
    /// window. Every requested channel must belong to this part. Default: the display window.
    pub data_window: Option<usize>,
    /// The value of each requested channel (same order) where the file has no samples; missing
    /// entries count as 0. Only [`read_planes`] uses it ([`read_bands`] delivers samples only).
    pub fill: &'a [f32],
    /// The rows to read, counted from the output window's top row, at full resolution.
    /// `None`: all rows.
    pub rows: Option<Range<i64>>,
    /// Every `2^level`-th row and column of the display window (0: full resolution, at most 16).
    pub level: u32,
    /// The most bytes of samples the read may produce: [`read_bands`] checks the requested rows
    /// at the level, [`read_planes`] the whole display-window planes it allocates. Without
    /// [`Request::max_block_bytes`], it also raises the most one decompressed block may take
    /// above the 256 MiB default.
    pub max_bytes: usize,
    /// The most threads that decompress (`None`: as many as the machine runs at once; `Some(0)`:
    /// none, everything is decoded on the calling thread). Never more than the chunks need.
    pub workers: Option<usize>,
    /// The most bytes one decompressed block may take (`None`: `max(256 MiB, max_bytes)`). A
    /// needed chunk whose block is larger is refused before anything is read.
    ///
    /// It is a limit of its own because the `exr` crate's decompressors allocate a whole block at
    /// once and can't report that the memory isn't there: the process aborts. A block must stay
    /// below what the caller is sure the system can give, even when the read's total budget
    /// (`max_bytes`, which allocations this module makes fallibly count against) is far larger. A
    /// block is one strip of the file (at most 256 scan lines, for DWAB): 8 GiB is a 16K-wide
    /// strip of 256 lines of 512 f32 channels.
    pub max_block_bytes: Option<usize>,
    /// Runs of request slots delivered together, as one band per block whose pixels hold the
    /// slots' values in slot order (`0..4` over R, G, B, A of a part: one RGBA band): for a caller
    /// that stores pixels interleaved. Every slot of a group must name a channel of the same part
    /// (they then share every block), and no slot may be in two groups; slots in no group come one
    /// band each, as without groups. Empty: no groups. [`read_planes`] ignores it.
    pub groups: &'a [Range<usize>],
}

impl<'a> Request<'a> {
    /// Every row of `channels` at full resolution, fill 0, up to `max_bytes` of samples, with as
    /// many decoding threads as the machine runs at once.
    pub fn new(channels: &'a [(usize, usize)], max_bytes: usize) -> Request<'a> {
        Request { channels, data_window: None, fill: &[], rows: None, level: 0, max_bytes, workers: None, max_block_bytes: None, groups: &[] }
    }
}

/// The size of the planes of a read at `level` of a `width` × `height` display window.
pub fn level_size((width, height): (usize, usize), level: u32) -> (usize, usize) {
    let stride = 1usize << level.min(16);
    (width.div_ceil(stride), height.div_ceil(stride))
}

/// A rectangle of rows of one requested channel, or of a group of them ([`Request::groups`]),
/// decoded: `samples` holds `height` rows of `width` pixels, row by row, each pixel
/// `values_per_pixel` values in slot order. `x` and `y` are in the output grid, counted from the
/// selected window's top-left corner (0, 0, whatever its absolute position): window pixels at
/// level 0, every `2^level`-th pixel above.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    /// The position of the channel in [`Request::channels`]; for a group, its first slot.
    pub slot: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// The values per pixel: 1, or the size of the group the band is of.
    pub values_per_pixel: usize,
    /// The chunk the band was decoded from: its number in the read, from 0 in file order, counting
    /// the chunks [`Index::estimate`] counts. A chunk may give no band (none of its rows on the
    /// level's grid, say) or several; the bands of a chunk come together, so progress is the
    /// highest `chunk` seen plus one, out of [`Estimate::chunks`].
    pub chunk: usize,
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
    /// The most decoded bytes the read had in flight at once, as estimated from the headers: the
    /// decompressed blocks and the bands of the chunks read but not yet handed to the sink.
    pub in_flight: u64,
}

/// What a read will take, from [`Index::estimate`], before anything is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Estimate {
    /// The chunks it fetches: one unit of progress each (a read hands every chunk's bands over
    /// together).
    pub chunks: usize,
    /// The bytes of those chunks it reads from the source.
    pub compressed_bytes: u64,
    /// An upper estimate of the most memory the read holds at once besides what the sink keeps:
    /// the decoded bytes in flight (as [`Stats::in_flight`] counts them: each chunk's
    /// decompressed block and its bands, exactly as large as they are made, grouped or not), as
    /// much again for the decompressors' scratch, two pieces of compressed bytes, and each
    /// worker's copy of the chunk it decodes.
    pub working_bytes: u64,
    /// The bytes of the planes [`read_planes`] would allocate for the request (whole display
    /// window planes, one per slot: it ignores [`Request::groups`]). Not part of `working_bytes`,
    /// and not checked here: [`read_planes`] checks it against [`Request::max_bytes`] when it
    /// runs, a caller that collects bands itself checks what it allocates.
    pub planes_bytes: u64,
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

/// How a read with a fallible sink ([`read_bands_try`]) ended early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop<E> {
    /// The read failed.
    Read(Error),
    /// The sink returned this error; nothing was handed to it after.
    Sink(E),
}

impl<E> From<Error> for Stop<E> {
    fn from(e: Error) -> Self {
        Stop::Read(e)
    }
}

impl<E: std::fmt::Display> std::fmt::Display for Stop<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::Read(e) => e.fmt(f),
            Stop::Sink(e) => e.fmt(f),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for Stop<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Stop::Read(e) => Some(e),
            Stop::Sink(_) => None,
        }
    }
}

/// The planes' geometry for [`chunk_bands`].
struct Geometry<'a> {
    /// The display window's size and the output's (at the level).
    size: (usize, usize),
    out: (usize, usize),
    stride: i64,
    in_data_window: bool,
    /// The display rows read.
    rows: Range<i64>,
    /// Each part's channel → the request slots that name it.
    slots: &'a [Vec<Vec<usize>>],
    /// Per request slot: the band it goes into (the first slot of its group, or itself), that
    /// band's values per pixel, and the slot's place among them.
    units: &'a [(usize, usize, usize)],
}

impl Geometry<'_> {
    fn offset(&self, part: &PartLayout) -> (i64, i64) {
        if self.in_data_window { (0, 0) } else { part.offset }
    }

    /// A display row is read: in the rows, on the level's grid.
    fn wanted(&self, row: i64) -> bool {
        row >= self.rows.start && row < self.rows.end && row.rem_euclid(self.stride) == 0
    }

    /// A chunk holding display rows `a..b` has a row read.
    fn any_wanted(&self, a: i64, b: i64) -> bool {
        let first = a.max(self.rows.start);
        // (saturating: the rows are clamped to the display window, but the chunk's top may not be)
        let first = first.saturating_add((self.stride - first.rem_euclid(self.stride)) % self.stride);
        first < b.min(self.rows.end)
    }

    /// How many rows are read (in the rows, on the level's grid).
    fn rows_read(&self) -> usize {
        let (a, b) = (self.rows.start.max(0), self.rows.end.max(0));
        let count = |n: i64| n.saturating_add(self.stride - 1) / self.stride;
        usize::try_from(count(b).saturating_sub(count(a))).unwrap_or(0)
    }
}

/// A chunk the request needs, in the order the jobs are numbered (file order).
struct Planned {
    part: usize,
    offset: u64,
    len: u64,
    chunk: ChunkRef,
    /// Its estimated decoded bytes: the decompressed block and the bands of the requested slots.
    cost: u64,
}

/// What a request needs, worked out from the index alone: the shared first step of a read and of
/// [`Index::estimate`].
struct Plan {
    /// Per part and channel: the request slots that name it.
    slots: Vec<Vec<Vec<usize>>>,
    size: (usize, usize),
    out: (usize, usize),
    stride: i64,
    in_data_window: bool,
    rows: Range<i64>,
    /// The chunks to read, in file order.
    jobs: Vec<Planned>,
    /// The decoding threads to start.
    workers: usize,
    /// Per request slot: see [`Geometry::units`].
    units: Vec<(usize, usize, usize)>,
}

impl Plan {
    fn geometry(&self) -> Geometry<'_> {
        Geometry {
            size: self.size,
            out: self.out,
            stride: self.stride,
            in_data_window: self.in_data_window,
            rows: self.rows.clone(),
            slots: &self.slots,
            units: &self.units,
        }
    }
}

fn request_window(index: &Index, request: &Request) -> Result<Window> {
    if let Some(part) = request.data_window {
        if request.channels.iter().any(|(p, _)| *p != part) {
            return Err(malformed("every channel in a data-window request must belong to that part"));
        }
        index.parts.get(part).map(|p| p.data_window).ok_or_else(|| malformed("no such data window part"))
    } else {
        Ok(index.display)
    }
}

/// Checks `request` against `index` and lists the chunks it needs (see [`read_bands`] for the
/// checks): nothing is read.
fn plan(index: &Index, request: &Request) -> Result<Plan> {
    let level = request.level.min(16);
    let stride = 1i64 << level;
    let window = request_window(index, request)?;
    let (w, h) = (window.width as usize, window.height as usize);
    let in_data_window = request.data_window.is_some();
    let out = level_size((w, h), level);
    // Per part and channel: the request slots that name it.
    let mut slots: Vec<Vec<Vec<usize>>> = index.parts.iter().map(|p| vec![Vec::new(); p.channels.len()]).collect();
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
            s.push(slot);
        }
    }
    // Groups: slots of one part (so they share every block), none in two groups.
    let mut units: Vec<(usize, usize, usize)> = (0..request.channels.len()).map(|slot| (slot, 1, 0)).collect();
    let mut grouped = vec![false; request.channels.len()];
    for group in request.groups {
        let members = request
            .channels
            .get(group.clone())
            .filter(|m| !m.is_empty())
            .ok_or_else(|| malformed(format!("channel group {group:?} is empty or past the requested channels")))?;
        if members.iter().any(|(part, _)| Some(*part) != members.first().map(|m| m.0)) {
            return Err(Error::Unsupported(format!("channel group {group:?} spans parts")));
        }
        for taken in grouped.get_mut(group.clone()).unwrap_or_default() {
            if std::mem::replace(taken, true) {
                return Err(malformed(format!("channel group {group:?} overlaps another")));
            }
        }
        for (k, unit) in units.get_mut(group.clone()).unwrap_or_default().iter_mut().enumerate() {
            *unit = (group.start, group.len(), k);
        }
    }
    let rows = request.rows.clone().unwrap_or(0..h as i64);
    // (both ends inside the display window, so an empty or reversed range at either end of i64
    // stays empty instead of overflowing when aligned to the level's grid)
    let rows = rows.start.clamp(0, h as i64)..rows.end.clamp(0, h as i64);
    let geometry = Geometry { size: (w, h), out, stride, in_data_window, rows: rows.clone(), slots: &slots, units: &units };
    // What the request delivers: the rows read, at the level, of every requested channel.
    let need = plane_bytes((out.0, geometry.rows_read()), request.channels.len());
    if need > request.max_bytes {
        return Err(Error::LimitExceeded(format!("the requested rows need {need} bytes, more than the {} the read may use", request.max_bytes)));
    }
    // The chunks to read, in file order. The decompressor allocates each one's whole block, so a
    // block may take no more than the read allows (see `Request::max_block_bytes`).
    let most_block = request.max_block_bytes.unwrap_or(MAX_BLOCK_BYTES.max(request.max_bytes)) as u64;
    let mut jobs: Vec<Planned> = Vec::new();
    for (pi, part) in index.layout.iter().enumerate() {
        let requested = slots.get(pi).map_or(0, |s| s.iter().map(Vec::len).sum::<usize>());
        if requested == 0 {
            continue;
        }
        for c in &part.chunks {
            let top = (c.y as i64).saturating_add(geometry.offset(part).1);
            if !geometry.any_wanted(top, top.saturating_add(c.rows as i64)) {
                continue;
            }
            if c.len == 0 {
                return Err(malformed("a chunk offset outside the chunk area of the file"));
            }
            if let Some(header) = index.meta.headers.get(pi) {
                index.check_ratio(pi, header, part, c, c.len)?;
            }
            let pixels = (c.cols as u64).saturating_mul(c.rows as u64);
            let block = pixels.saturating_mul(part.pixel_bytes as u64);
            if block > most_block {
                return Err(Error::LimitExceeded(format!(
                    "a block of part {pi} decompresses to {block} bytes, more than the {most_block} one block of this read may use"
                )));
            }
            let cost = block.saturating_add(pixels.saturating_mul(requested as u64).saturating_mul(4));
            jobs.try_reserve(1).map_err(|_| Error::OutOfMemory("chunk list allocation".into()))?;
            jobs.push(Planned { part: pi, offset: c.offset, len: c.len, chunk: *c, cost });
        }
    }
    jobs.sort_by_key(|j| j.offset);
    // (no more threads than chunks can be in flight: two pieces)
    let workers =
        request.workers.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from)).min(jobs.len()).min(2 * PIECE_CHUNKS);
    Ok(Plan { slots, size: (w, h), out, stride, in_data_window, rows, jobs, workers, units })
}

impl Index {
    /// What reading `request` will take, before anything is read: the chunks (one unit of
    /// progress each), their bytes, the most memory the read holds at once, and the planes
    /// [`read_planes`] allocates. Fails as the read would on the same request (limits, unknown or
    /// deep channels), so an app can reserve memory and size a progress bar first.
    pub fn estimate(&self, request: &Request) -> Result<Estimate> {
        estimate_within(self, request, IN_FLIGHT_BYTES)
    }
}

/// [`Index::estimate`] for a read keeping at most `budget` decoded bytes in flight.
pub(crate) fn estimate_within(index: &Index, request: &Request, budget: u64) -> Result<Estimate> {
    let plan = plan(index, request)?;
    let sum = |f: fn(&Planned) -> u64| plan.jobs.iter().map(f).fold(0u64, u64::saturating_add);
    let most = |f: fn(&Planned) -> u64| plan.jobs.iter().map(f).max().unwrap_or(0);
    let (compressed, longest) = (sum(|j| j.len), most(|j| j.len));
    // As the read bounds them: decoded bytes within the budget or one chunk, two pieces.
    let decoded = sum(|j| j.cost).min(budget.max(most(|j| j.cost)));
    let pieces = compressed.min(PIECE_BYTES.max(longest).saturating_mul(2));
    let copies = (plan.workers.max(1) as u64).saturating_mul(longest).min(compressed);
    let (w, h) = level_size(plan.size, request.level.min(16));
    Ok(Estimate {
        chunks: plan.jobs.len(),
        compressed_bytes: compressed,
        working_bytes: decoded.saturating_mul(2).saturating_add(pieces).saturating_add(copies),
        planes_bytes: plane_bytes((w, h), request.channels.len()) as u64,
    })
}

/// The bands of what `request` asks for, handed to `sink` one decoded block at a time, in file
/// order: by offset in the file, so a part written bottom-up (`LineOrder::Decreasing`) arrives
/// bottom-up, and the chunks of several parts arrive interleaved as the file stores them. Within
/// a block, the bands come by request slot (a group at its first slot).
///
/// The chunks are read on the calling thread, in pieces of chunks that follow each other in the
/// file, and decompressed on worker threads ([`Request::workers`]) while the next piece is read.
/// Each time a piece has been read, and while waiting, the calling thread hands every band that
/// is next in file order to `sink`, so the first bands arrive after the first few hundred
/// kilobytes have been read, not after the whole request, and `sink` needs to be neither `Send`
/// nor `Sync`.
///
/// Memory is bounded. At most one piece is read ahead of what has been delivered, so at most two
/// pieces of compressed bytes (8 MiB, or one larger chunk) are held. Each chunk's decoded bytes
/// are estimated from the headers as its decompressed block plus 4 bytes per pixel for each
/// requested slot of its part (see [`Stats::in_flight`]); the chunks in flight stay within
/// 64 MiB of that, and a chunk whose estimate alone is over 64 MiB is decoded alone, once
/// everything before it has been delivered, so however many workers run, at most one such chunk
/// is held at a time. A block may take no more than [`Request::max_block_bytes`] (by default
/// 256 MiB, or [`Request::max_bytes`] if that is more); a needed chunk over it is refused
/// ([`Error::LimitExceeded`]) before anything is read.
/// For one big chunk the real peak is several times its block: its compressed bytes in the
/// piece, the `exr` crate's copy of its payload (a chunk's declared size is checked against the
/// bytes it holds first, so that copy is no larger than the chunk), the decompressed block, the
/// decompressor's scratch buffers, and the `f32` bands; roughly 3 to 4 times
/// `max(256 MiB, max_bytes)` at worst ([`Index::estimate`] gives the bound for a request). Where
/// threads can't be started (WebAssembly) the chunks are decoded on the calling thread instead.
/// `cancel` is polled before each piece, before each chunk is handed to `sink`, and at least
/// every 50 ms while waiting for the workers.
///
/// A band lies inside the planes the request describes ([`level_size`] of the display window);
/// rows or columns no chunk or data window covers are not delivered. A chunk the request needs
/// whose offset lies outside the file fails the read before anything is read or delivered
/// ([`Error::Malformed`]). A chunk that can't be decoded or isn't the block its offset-table
/// entry stands for fails the read when its turn comes: the bands before it have been delivered
/// by then.
pub fn read_bands<R: Read + Seek + ?Sized>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Band),
) -> Result<Stats> {
    read_bands_within(source, index, request, cancel, sink, IN_FLIGHT_BYTES)
}

/// [`read_bands`] that keeps what it did in `stats` as it goes, so the counts are there also when
/// the read fails or is cancelled (they then say how far it got). `stats` is reset first.
pub fn read_bands_with_stats<R: Read + Seek + ?Sized>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Band),
    stats: &mut Stats,
) -> Result<()> {
    read_infallible(source, index, request, cancel, sink, IN_FLIGHT_BYTES, stats)
}

/// [`read_bands_with_stats`] with a sink that can stop the read with an error of its own (a tile
/// store out of room, a full scratch disk): nothing more is read or handed to the sink after it
/// fails, the decoding threads wind down as on a cancel, and the sink's error comes back as
/// [`Stop::Sink`], unchanged. `stats` says how far the read got either way; after a sink error
/// its `lines` include the chunk whose band the sink refused (the chunk was decoded whole).
///
/// This is also the way to pull: a caller that wants to decide band by band whether to go on (and
/// check a budget before each one) does that in the sink. A pulling iterator would have to run the
/// read on a thread of its own, because the read hands bands over from the middle of its loop.
pub fn read_bands_try<R: Read + Seek + ?Sized, E>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Band) -> std::result::Result<(), E>,
    stats: &mut Stats,
) -> std::result::Result<(), Stop<E>> {
    read_into(source, index, request, cancel, sink, IN_FLIGHT_BYTES, stats)
}

/// [`read_bands`] keeping at most `budget` decoded bytes in flight (the tests use a small one).
pub(crate) fn read_bands_within<R: Read + Seek + ?Sized>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Band),
    budget: u64,
) -> Result<Stats> {
    let mut stats = Stats::default();
    read_infallible(source, index, request, cancel, sink, budget, &mut stats)?;
    Ok(stats)
}

/// [`read_into`] with a sink that can't fail.
fn read_infallible<R: Read + Seek + ?Sized>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Band),
    budget: u64,
    stats: &mut Stats,
) -> Result<()> {
    let mut sink = |band: Band| -> std::result::Result<(), std::convert::Infallible> {
        sink(band);
        Ok(())
    };
    read_into(source, index, request, cancel, &mut sink, budget, stats).map_err(|stop| match stop {
        Stop::Read(e) => e,
        Stop::Sink(never) => match never {},
    })
}

/// The read behind [`read_bands`], counting into `stats`.
fn read_into<R: Read + Seek + ?Sized, E>(
    source: &mut R,
    index: &Index,
    request: &Request,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Band) -> std::result::Result<(), E>,
    budget: u64,
    stats: &mut Stats,
) -> std::result::Result<(), Stop<E>> {
    *stats = Stats { chunks: vec![0; index.layout.len()], ..Default::default() };
    let plan = plan(index, request)?;
    let geometry = plan.geometry();
    let jobs = &plan.jobs;
    if jobs.is_empty() {
        return Ok(());
    }
    // (declared outside the scope because the workers borrow the receiving end)
    let (to_workers, queue) = mpsc::channel::<Job>();
    let queue = Mutex::new(queue);
    std::thread::scope(|scope| {
        // Owned by this closure so that every way out of it (an error, a cancel, the sink's
        // error) drops the sending end, which ends the workers' loops before the scope waits.
        let to_workers = to_workers;
        let (done, results) = mpsc::channel();
        let mut started = 0usize;
        for _ in 0..plan.workers {
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
        let mut most = FIRST_PIECE_BYTES;
        let mut submitted = 0usize;
        // The decoded bytes of the chunks submitted and not yet delivered.
        let mut in_flight = 0u64;
        // The sequence number of the first chunk of the piece read last.
        let mut last_piece = 0usize;
        // Hands over what is next in order and takes it off the bytes in flight.
        let mut deliver = |order: &mut InOrder, in_flight: &mut u64, stats: &mut Stats| -> std::result::Result<(), Stop<E>> {
            let before = order.next;
            let delivered = order.deliver(sink, stats, cancel);
            let done: u64 = jobs.get(before..order.next).unwrap_or_default().iter().map(|j| j.cost).fold(0, u64::saturating_add);
            *in_flight = in_flight.saturating_sub(done);
            delivered
        };
        while submitted < jobs.len() {
            if cancel() {
                return Err(Error::Cancelled.into());
            }
            let rest = jobs.get(submitted..).unwrap_or_default();
            let take = piece_len(rest, most, budget);
            let piece = rest.get(..take).unwrap_or_default();
            let cost = piece.iter().map(|j| j.cost).fold(0, u64::saturating_add);
            // Read ahead by one piece at most, and keep the decoded bytes in flight within the
            // budget (a piece over it alone is read once everything before it is delivered).
            // (`order.next` is a chunk not received yet whenever this waits, so a result is on
            // its way: everything decoded on this thread is delivered at once.)
            while order.next < last_piece || (order.next < submitted && in_flight.saturating_add(cost) > budget) {
                let (seq, result) = next_result(&results, cancel)?;
                order.accept(seq, result);
                deliver(&mut order, &mut in_flight, stats)?;
                if cancel() {
                    return Err(Error::Cancelled.into());
                }
            }
            most = most.saturating_mul(2).min(PIECE_BYTES);
            let Piece { data, chunks } = read_piece(source, piece, &mut pos, stats)?;
            let data = Arc::new(data);
            last_piece = submitted;
            in_flight = in_flight.saturating_add(cost);
            stats.in_flight = stats.in_flight.max(in_flight);
            for (planned, range) in piece.iter().zip(chunks) {
                let job = Job { seq: submitted, part: planned.part, chunk: planned.chunk, data: Arc::clone(&data), range };
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
            deliver(&mut order, &mut in_flight, stats)?;
        }
        // Everything is read: the rest as it is decoded.
        while order.next < submitted {
            if cancel() {
                return Err(Error::Cancelled.into());
            }
            let (seq, result) = next_result(&results, cancel)?;
            order.accept(seq, result);
            deliver(&mut order, &mut in_flight, stats)?;
        }
        Ok(())
    })
}

/// The next decoded chunk from the workers, polling `cancel` while waiting. (Only called while a
/// worker holds a chunk: on the calling thread everything is decoded before it would wait.)
fn next_result(results: &Receiver<(usize, Decoded)>, cancel: &dyn Fn() -> bool) -> Result<(usize, Decoded)> {
    loop {
        match results.recv_timeout(CANCEL_POLL) {
            Ok(result) => return Ok(result),
            Err(RecvTimeoutError::Timeout) => {
                if cancel() {
                    return Err(Error::Cancelled);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Err(malformed("a decoding thread stopped")),
        }
    }
}

/// [`read_bands`] collected: one plane per requested channel covering the whole display window
/// at the request's level (also when [`Request::rows`] selects only some rows: the other rows
/// keep the fill value), filled with [`Request::fill`] where nothing was read. The planes are
/// reserved up front and count against [`Request::max_bytes`] (an allocation the system refuses
/// is [`Error::OutOfMemory`], not an abort), so the cost is known before the first chunk is
/// read.
pub fn read_planes<R: Read + Seek + ?Sized>(source: &mut R, index: &Index, request: &Request, cancel: &dyn Fn() -> bool) -> Result<Planes> {
    let window = request_window(index, request)?;
    let (w, h) = (window.width as usize, window.height as usize);
    let (ow, oh) = level_size((w, h), request.level.min(16));
    let n = ow.checked_mul(oh).ok_or_else(|| Error::LimitExceeded("display window".into()))?;
    let need = plane_bytes((ow, oh), request.channels.len());
    if need > request.max_bytes {
        return Err(Error::LimitExceeded(format!("{w}×{h} channels need {need} bytes, more than the {} the read may use", request.max_bytes)));
    }
    let mut planes: Vec<Vec<f32>> = Vec::new();
    planes.try_reserve_exact(request.channels.len()).map_err(|_| Error::OutOfMemory("plane list allocation".into()))?;
    for slot in 0..request.channels.len() {
        let mut plane: Vec<f32> = Vec::new();
        plane.try_reserve_exact(n).map_err(|_| Error::OutOfMemory(format!("{need} bytes of planes: the system can't give that memory now")))?;
        plane.resize(n, request.fill.get(slot).copied().unwrap_or(0.0));
        planes.push(plane);
    }
    // (one plane per slot: groups would interleave)
    let request = Request { groups: &[], ..request.clone() };
    let stats = read_bands(source, index, &request, cancel, &mut |band: Band| {
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

/// One chunk to decode: its sequence number in file order, its part, where it belongs, and where
/// its bytes lie in the piece it was read with (shared by all the chunks of that piece).
struct Job {
    seq: usize,
    part: usize,
    chunk: ChunkRef,
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
        let seq = job.seq;
        let mut report = Report { seq, done, sent: false };
        let decoded = decode_job(index, g, &job);
        // (its share of the piece's bytes goes now, not after the result is taken: the two-pieces
        // bound counts on it)
        drop(job);
        report.sent = true;
        if done.send((seq, decoded)).is_err() {
            return;
        }
    }
}

/// Reports a chunk as failed if the worker decoding it unwinds before sending its result, so the
/// calling thread, which waits for every chunk in order, never waits for one that won't come.
struct Report<'a> {
    seq: usize,
    done: &'a Sender<(usize, Decoded)>,
    sent: bool,
}

impl Drop for Report<'_> {
    fn drop(&mut self) {
        if !self.sent {
            let _ = self.done.send((self.seq, Err(malformed("a decoding thread stopped"))));
        }
    }
}

/// The bands of one chunk. A panic in the `exr` crate's decompressors is reported as a damaged
/// chunk, never raised (where panics unwind).
fn decode_job(index: &Index, g: &Geometry, job: &Job) -> Decoded {
    let buf = job.data.get(job.range.clone()).unwrap_or_default();
    let decoded = catch_unwind(AssertUnwindSafe(|| chunk_bands(index, job.part, &job.chunk, buf, g)))
        .unwrap_or_else(|_| Err(malformed("a chunk could not be decompressed")));
    decoded.map(|(mut bands, lines)| {
        for band in &mut bands {
            band.chunk = job.seq;
        }
        (bands, lines)
    })
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

    /// Hands every chunk that is next in order to `sink`, polling `cancel` before each one; the
    /// first chunk that failed to decode ends the read with its error (after the chunks before it
    /// were delivered).
    /// The sink's error stops the delivery at once (no band after it).
    fn deliver<E>(
        &mut self,
        sink: &mut dyn FnMut(Band) -> std::result::Result<(), E>,
        stats: &mut Stats,
        cancel: &dyn Fn() -> bool,
    ) -> std::result::Result<(), Stop<E>> {
        while self.waiting.contains_key(&self.next) {
            if cancel() {
                return Err(Error::Cancelled.into());
            }
            let Some(decoded) = self.waiting.remove(&self.next) else { break };
            self.next += 1;
            let (bands, lines) = decoded?;
            stats.lines = stats.lines.saturating_add(lines);
            for band in bands {
                sink(band).map_err(Stop::Sink)?;
            }
        }
        Ok(())
    }
}

/// The bytes of a piece, and each of its chunks' range in them (in the piece's order).
struct Piece {
    data: Vec<u8>,
    chunks: Vec<Range<usize>>,
}

/// How many of `jobs` (chunks in file order) the next piece takes: chunks that each start where
/// the one before ends, up to [`PIECE_CHUNKS`] of them, `most` bytes or `budget` decoded bytes,
/// at least one.
fn piece_len(jobs: &[Planned], most: u64, budget: u64) -> usize {
    let mut take = 0;
    let (mut bytes, mut cost) = (0u64, 0u64);
    let mut end = None;
    while let Some(job) = jobs.get(take) {
        if take > 0
            && (take >= PIECE_CHUNKS || end != Some(job.offset) || bytes.saturating_add(job.len) > most || cost.saturating_add(job.cost) > budget)
        {
            break;
        }
        bytes = bytes.saturating_add(job.len);
        cost = cost.saturating_add(job.cost);
        end = Some(job.offset.saturating_add(job.len));
        take += 1;
    }
    take
}

/// The raw bytes of the chunks `piece` (in file order, each starting where the one before ends),
/// read with one request after a seek when the source stands elsewhere (`pos`), and each chunk's
/// range in them. A chunk the file ends in keeps the bytes there are.
fn read_piece<R: Read + Seek + ?Sized>(source: &mut R, piece: &[Planned], pos: &mut Option<u64>, stats: &mut Stats) -> Result<Piece> {
    let Some(start) = piece.first().map(|j| j.offset) else { return Ok(Piece { data: Vec::new(), chunks: Vec::new() }) };
    let total = piece.iter().map(|j| j.len).fold(0u64, u64::saturating_add);
    let mut data = Vec::new();
    data.try_reserve_exact(usize::try_from(total).map_err(|_| malformed("chunk size"))?)
        .map_err(|_| Error::OutOfMemory("chunk allocation".into()))?;
    if *pos != Some(start) {
        source.seek(SeekFrom::Start(start)).map_err(io)?;
    }
    (&mut *source).take(total).read_to_end(&mut data).map_err(io)?;
    *pos = Some(start.saturating_add(data.len() as u64));
    stats.bytes = stats.bytes.saturating_add(data.len() as u64);
    let mut chunks = Vec::new();
    chunks.try_reserve_exact(piece.len()).map_err(|_| Error::OutOfMemory("chunk allocation".into()))?;
    for job in piece {
        let a = usize::try_from(job.offset.saturating_sub(start)).unwrap_or(usize::MAX).min(data.len());
        let b = usize::try_from(job.len).map_or(data.len(), |l| a.saturating_add(l)).min(data.len());
        chunks.push(a..b);
        if let Some(c) = stats.chunks.get_mut(job.part) {
            *c += 1;
        }
    }
    Ok(Piece { data, chunks })
}

/// Checks the header of the chunk `buf` of flat part `pi` (`header`), which its offset table
/// says is `expected`, before the `exr` crate reads it: the crate allocates and zero-fills the
/// payload size a chunk declares (up to its own bound per block, which can be hundreds of
/// megabytes) before reading it, and computes a tile's position from the level it declares
/// (panicking on some). The header is the part number (in multi-part files), then the first
/// scan line's y or the tile's column, row and level (x, y), then the payload size, all `i32`.
fn check_chunk_header(meta: &MetaData, header: &Header, pi: usize, expected: &ChunkRef, buf: &[u8]) -> Result<Range<usize>> {
    let word = |i: usize| buf.get(i * 4..i * 4 + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(i32::from_le_bytes);
    let short = || malformed("a chunk shorter than its header");
    let mut at = 0usize;
    if meta.requirements.is_multilayer() {
        if usize::try_from(word(0).ok_or_else(short)?).ok() != Some(pi) {
            return Err(malformed("a chunk of another part where its offset table points"));
        }
        at = 1;
    }
    let in_place = match header.blocks {
        exr::meta::BlockDescription::ScanLines => {
            let y = word(at).ok_or_else(short)?;
            at += 1;
            i64::from(y) == i64::from(header.own_attributes.layer_position.1).saturating_add(expected.y as i64)
        }
        exr::meta::BlockDescription::Tiles(_) => {
            let tile = [word(at), word(at + 1), word(at + 2), word(at + 3)];
            at += 4;
            let [Some(x), Some(y), Some(lx), Some(ly)] = tile else { return Err(short()) };
            (usize::try_from(x).ok(), usize::try_from(y).ok()) == (Some(expected.tile.0), Some(expected.tile.1)) && (lx, ly) == (0, 0)
        }
    };
    if !in_place {
        return Err(malformed("a chunk is not the block its offset table entry stands for"));
    }
    let size = usize::try_from(word(at).ok_or_else(short)?).map_err(|_| malformed("a chunk declaring a negative size"))?;
    if size > buf.len().saturating_sub((at + 1) * 4) {
        return Err(malformed("a chunk declaring more bytes than it holds"));
    }
    let start = (at + 1) * 4;
    Ok(start..start + size)
}

/// Decompress the chunk `buf` of part `pi`, which its offset table says is `expected`, and convert
/// the lines of the requested channels to bands (`g.slots`: each channel's request slots): the
/// bands, and the scan lines decompressed. A chunk that names another part, level or position
/// than its offset-table entry, or declares more bytes than it holds, is refused before the
/// `exr` crate reads it.
fn chunk_bands(index: &Index, pi: usize, expected: &ChunkRef, buf: &[u8], g: &Geometry) -> Result<(Vec<Band>, usize)> {
    let meta = &index.meta;
    let (Some(header), Some(part)) = (meta.headers.get(pi), index.layout.get(pi)) else { return Err(malformed("no such part")) };
    let payload = check_chunk_header(meta, header, pi, expected, buf)?;
    // (a DWA chunk's counters can make the crate allocate terabytes: see `crate::dwa_guard`)
    let payload = buf.get(payload).unwrap_or_default();
    // (planned against the gap to the next chunk; the payload the chunk declares may be smaller)
    index.check_ratio(pi, header, part, expected, payload.len() as u64)?;
    crate::dwa_guard::check_payload(header, (expected.cols, expected.rows), payload).map_err(malformed)?;
    let mut read = buf;
    let chunk = Chunk::read(&mut read, meta).map_err(exr_error)?;
    let block = UncompressedBlock::decompress_chunk(chunk, meta, false).map_err(exr_error)?;
    if block.index.level != Vec2(0, 0) || block.index.pixel_position != Vec2(expected.x, expected.y) {
        return Err(malformed("a chunk is not the block its offset table entry stands for"));
    }
    let slots = g.slots.get(pi).map_or(&[][..], Vec::as_slice);
    let lines = block.index.pixel_size.1;
    let ((lw, lh), (w, h)) = (part.size, g.size);
    let (offset_x, offset_y) = g.offset(part);
    let (ow, _) = g.out;
    // The output rows of the block: its data-window rows that land in the display window on a
    // wanted row. They are consecutive in the output grid (wanted rows are every stride-th).
    let (by, bh) = (block.index.pixel_position.1, block.index.pixel_size.1);
    let out_rows: Vec<usize> = (by..by.saturating_add(bh))
        .filter_map(|y| usize::try_from(y as i64 + offset_y).ok())
        .filter(|ty| *ty < h && g.wanted(*ty as i64))
        .map(|ty| ty / g.stride as usize)
        .collect();
    let (Some(&top), Some(&bottom)) = (out_rows.first(), out_rows.last()) else { return Ok((Vec::new(), lines)) };
    let height = bottom.saturating_sub(top).saturating_add(1);
    // One band per output unit (a group, or a slot of its own), each row written straight into it
    // at its place: no band is copied or regrown.
    let mut bands: std::collections::BTreeMap<usize, Band> = std::collections::BTreeMap::new();
    // (typed: the line reader is generic over the sample type, and an untyped `0` would make the
    // integer buffer `i32`, turning ids from 2^31 up negative)
    let (mut f16s, mut f32s, mut u32s): (Vec<exr::prelude::f16>, Vec<f32>, Vec<u32>) = (Vec::new(), Vec::new(), Vec::new());
    for line in block.lines(&header.channels) {
        let Some(wanted) = slots.get(line.location.channel).filter(|s| !s.is_empty()) else { continue };
        let Some(kind) = header.channels.list.get(line.location.channel).map(|c| c.sample_type) else { continue };
        let (x0, y, count) = (line.location.position.0, line.location.position.1, line.location.sample_count);
        // The samples of data-window row `y`, columns `x0..x0 + count`, at display row `ty` from
        // display column `tx0`; clipped to the display window.
        if y >= lh || x0.checked_add(count).is_none_or(|end| end > lw) {
            continue;
        }
        let Some(ty) = usize::try_from(y as i64 + offset_y).ok().filter(|ty| *ty < h) else { continue };
        if !g.wanted(ty as i64) {
            continue;
        }
        let tx0 = x0 as i64 + offset_x;
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
        // (a channel requested more than once goes into each of its slots)
        for slot in wanted {
            let Some(&(unit, n, k)) = g.units.get(*slot) else { continue };
            let band = match bands.entry(unit) {
                std::collections::btree_map::Entry::Occupied(band) => band.into_mut(),
                std::collections::btree_map::Entry::Vacant(place) => {
                    let len = height.saturating_mul(values.len()).saturating_mul(n);
                    let mut samples = Vec::new();
                    samples.try_reserve_exact(len).map_err(|_| Error::OutOfMemory("band allocation".into()))?;
                    samples.resize(len, 0.0);
                    place.insert(Band { slot: unit, x: col, y: top, width: values.len(), height, values_per_pixel: n, chunk: 0, samples })
                }
            };
            // (every line of a block covers the same columns; a block whose lines don't is damaged)
            if band.x != col || band.width != values.len() || row < top || row > bottom {
                return Err(malformed("the lines of a block disagree on where they lie"));
            }
            let base = (row - top).saturating_mul(band.width).saturating_mul(n);
            for (i, v) in values.iter().enumerate() {
                if let Some(slot) = band.samples.get_mut(base + i * n + k) {
                    *slot = *v;
                }
            }
        }
    }
    drop(block);
    Ok((bands.into_values().collect(), lines))
}
