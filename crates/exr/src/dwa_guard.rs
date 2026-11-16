//! A check of DWAA/DWAB chunks before the `exr` crate decompresses them.
//!
//! A DWA payload starts with eleven little-endian `u64` counters, which the `exr` crate (1.74.3,
//! `src/compression/dwa/chunk_header.rs`, `DwaHeader::parse`) reads in this order: version,
//! unknown uncompressed size, unknown compressed size, AC compressed size, DC compressed size,
//! RLE compressed size, RLE uncompressed size, RLE raw size, AC count, DC count, AC compression.
//! The crate checks them only against `i64::MAX`, then inflates sections with these sizes as the
//! size hint (`src/compression/dwa/section_stream.rs`: `inflate(section, expected)` with the
//! unknown uncompressed size, `ac_count * 2` for deflated AC, `dc_count * 2`, and the RLE
//! uncompressed size), and its inflater allocates the hint up front. A 100-byte chunk declaring
//! 2^44 DC values makes it allocate 32 TiB: the process aborts, which `catch_unwind` can't stop.
//!
//! What a valid chunk declares is bounded by its block (`width × height` pixels of all the
//! part's channels, `raw` bytes uncompressed):
//! - the unknown (losslessly zipped) channels and the RLE channels are some of the block's
//!   channels stored planar, so their uncompressed sizes are at most `raw`;
//! - the RLE token stream adds at most one count byte per 127 literal bytes (runs of three or
//!   more shrink), so it stays within `2 × raw`, a bound that leaves room for any encoder;
//! - the lossy channels are cut into 8 × 8 DCT blocks over the block padded to multiples of 8:
//!   one DC value and at most 63 AC values per DCT block and channel.

use exr::meta::header::Header;
use exr::prelude::Compression;

/// The counters' bytes at the start of a DWA payload.
const COUNTERS: usize = 11 * 8;

/// Whether chunks of `compression` go through this check.
pub(crate) fn is_dwa(compression: Compression) -> bool {
    matches!(compression, Compression::DWAA(_) | Compression::DWAB(_))
}

/// Checks the DWA payload `payload` of a block of `width × height` pixels of the part `header`
/// describes: an error message when a counter declares more than a valid chunk of that block
/// can hold. A payload as long as the block's raw bytes is stored uncompressed (the crate doesn't
/// read counters then); one shorter than the counters is left to the crate, which refuses it.
pub(crate) fn check_payload(header: &Header, (width, height): (usize, usize), payload: &[u8]) -> Result<(), String> {
    if !is_dwa(header.compression) {
        return Ok(());
    }
    let channels = header.channels.list.len();
    let pixel_bytes = header.channels.list.iter().map(|c| c.sample_type.bytes_per_sample()).try_fold(0usize, usize::checked_add);
    let raw = pixel_bytes.and_then(|b| b.checked_mul(width)).and_then(|b| b.checked_mul(height)).ok_or("a DWA block size overflows")?;
    if payload.len() == raw {
        return Ok(());
    }
    let Some(counters) = payload.get(..COUNTERS) else { return Ok(()) };
    let counter = |i: usize| counters.get(i * 8..i * 8 + 8).and_then(|b| <[u8; 8]>::try_from(b).ok()).map_or(u64::MAX, u64::from_le_bytes);
    let dct_blocks = width.div_ceil(8).checked_mul(height.div_ceil(8)).and_then(|n| n.checked_mul(channels)).ok_or("a DWA block size overflows")?;
    let raw = raw as u64;
    let bounds: [(usize, &str, u64); 5] = [
        (1, "unknown uncompressed size", raw),
        (6, "RLE uncompressed size", raw.saturating_mul(2)),
        (7, "RLE raw size", raw),
        (8, "AC count", (dct_blocks as u64).saturating_mul(63)),
        (9, "DC count", dct_blocks as u64),
    ];
    for (i, name, most) in bounds {
        let value = counter(i);
        // (also keeps `count * 2` within a 32-bit usize: the bounds are block sizes, which fit)
        if value > most || usize::try_from(value.saturating_mul(2)).is_err() {
            return Err(format!("a DWA chunk declares a {name} of {value}, more than its {width} × {height} block can hold ({most})"));
        }
    }
    Ok(())
}

/// The pixel size of the block a chunk of the part `header` describes, from the coordinates in
/// its header (the scan-line `y`, or the tile's column, row and level), computed the way the
/// `exr` crate places it; `None` where the crate would refuse the chunk itself.
pub(crate) fn block_size(header: &Header, y: Option<i32>, tile: Option<[i32; 4]>) -> Option<(usize, usize)> {
    let (w, h) = (header.layer_size.0, header.layer_size.1);
    match (header.blocks, y, tile) {
        (exr::meta::BlockDescription::ScanLines, Some(y), _) => {
            let lines = header.compression.scan_lines_per_block();
            let row = usize::try_from(i64::from(y).checked_sub(i64::from(header.own_attributes.layer_position.1))?).ok()?;
            let start = (row / lines).checked_mul(lines)?;
            Some((w, lines.min(h.checked_sub(start).filter(|n| *n > 0)?)))
        }
        (exr::meta::BlockDescription::Tiles(t), _, Some([tx, ty, lx, ly])) => {
            let level = |full: usize, l: i32| -> Option<usize> {
                let l = u32::try_from(l).ok().filter(|l| *l < 32)?;
                let size = match t.rounding_mode {
                    exr::math::RoundingMode::Down => full >> l,
                    exr::math::RoundingMode::Up => full.div_ceil(1usize << l),
                };
                Some(size.max(1))
            };
            let (lw, lh) = (level(w, lx)?, level(h, ly)?);
            let x0 = usize::try_from(tx).ok()?.checked_mul(t.tile_size.0)?;
            let y0 = usize::try_from(ty).ok()?.checked_mul(t.tile_size.1)?;
            Some((t.tile_size.0.min(lw.checked_sub(x0).filter(|n| *n > 0)?), t.tile_size.1.min(lh.checked_sub(y0).filter(|n| *n > 0)?)))
        }
        _ => None,
    }
}

/// Checks every chunk of the file's DWA parts (flat ones; `tables` is where the offset tables
/// start) before the `exr` crate's whole-image reader, which offers no hook between reading a
/// chunk and decompressing it, gets the bytes. Chunks the crate would refuse by themselves
/// (outside the file, wrong coordinates) are left to it.
pub(crate) fn check_file(bytes: &[u8], meta: &exr::meta::MetaData, tables: usize) -> Result<(), String> {
    if !meta.headers.iter().any(|h| !h.deep && is_dwa(h.compression)) {
        return Ok(());
    }
    let multipart = meta.requirements.is_multilayer();
    let word = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(i32::from_le_bytes);
    let chunks = meta.headers.iter().map(|h| h.chunk_count).try_fold(0usize, usize::checked_add).ok_or("too many chunks")?;
    let entries = bytes.get(tables..tables.saturating_add(chunks.saturating_mul(8))).ok_or("the offset tables run past the end of the file")?;
    // (every table's entries: the crate decodes a chunk as the part the chunk names, whichever
    // table points at it)
    for entry in entries.as_chunks::<8>().0 {
        let Ok(mut at) = usize::try_from(u64::from_le_bytes(*entry)) else { continue };
        let part = if multipart {
            let part = word(at).and_then(|p| usize::try_from(p).ok());
            at = at.saturating_add(4);
            part
        } else {
            Some(0)
        };
        let Some(header) = part.and_then(|p| meta.headers.get(p)) else { continue };
        if header.deep || !is_dwa(header.compression) {
            continue;
        }
        let size = match header.blocks {
            exr::meta::BlockDescription::ScanLines => {
                let y = word(at);
                at = at.saturating_add(4);
                block_size(header, y, None)
            }
            exr::meta::BlockDescription::Tiles(_) => {
                let tile = [word(at), word(at.saturating_add(4)), word(at.saturating_add(8)), word(at.saturating_add(12))];
                at = at.saturating_add(16);
                match tile {
                    [Some(a), Some(b), Some(c), Some(d)] => block_size(header, None, Some([a, b, c, d])),
                    _ => None,
                }
            }
        };
        // (a payload past the file's end fails in the crate before it is decompressed)
        let (Some(size), Some(len)) = (size, word(at).and_then(|n| usize::try_from(n).ok())) else { continue };
        let start = at.saturating_add(4);
        let Some(payload) = bytes.get(start..start.saturating_add(len)) else { continue };
        check_payload(header, size, payload)?;
    }
    Ok(())
}
