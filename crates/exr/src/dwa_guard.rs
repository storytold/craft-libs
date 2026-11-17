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

use exr::block::chunk::{CompressedBlock, CompressedScanLineBlock, CompressedTileBlock, TileCoordinates};
use exr::math::Vec2;
use exr::meta::header::Header;
use exr::prelude::Compression;

/// The counters' bytes at the start of a DWA payload.
const COUNTERS: usize = 11 * 8;

/// Whether chunks of `compression` go through this check.
pub(crate) fn is_dwa(compression: Compression) -> bool {
    matches!(compression, Compression::DWAA(_) | Compression::DWAB(_))
}

/// What the check needs of a DWA part: the bytes of one pixel of all its channels, and how many
/// channels it has. `None` when the pixel's bytes overflow.
fn shape(header: &Header) -> Option<(usize, usize)> {
    let pixel_bytes = header.channels.list.iter().map(|c| c.sample_type.bytes_per_sample()).try_fold(0usize, usize::checked_add)?;
    Some((pixel_bytes, header.channels.list.len()))
}

/// Checks the DWA payload `payload` of a block of `width × height` pixels of the part `header`
/// describes: an error message when a counter declares more than a valid chunk of that block
/// can hold. A payload as long as the block's raw bytes is stored uncompressed (the crate doesn't
/// read counters then); one shorter than the counters is left to the crate, which refuses it.
pub(crate) fn check_payload(header: &Header, size: (usize, usize), payload: &[u8]) -> Result<(), String> {
    if !is_dwa(header.compression) {
        return Ok(());
    }
    check_shaped(shape(header).ok_or("a DWA pixel size overflows")?, size, payload)
}

/// [`check_payload`] with the part's [`shape`] worked out already.
fn check_shaped((pixel_bytes, channels): (usize, usize), (width, height): (usize, usize), payload: &[u8]) -> Result<(), String> {
    let raw = pixel_bytes.checked_mul(width).and_then(|b| b.checked_mul(height)).ok_or("a DWA block size overflows")?;
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

/// The pixel size of the block the `exr` crate decompresses a chunk of the part `header`
/// describes into, from the chunk's coordinates: the crate's own placement
/// (`Header::get_block_data_indices`, then `get_absolute_block_pixel_coordinates`), so a scan-line
/// `y` the crate rounds into a block (it divides toward zero: rows just above the data window land
/// in block 0) is sized as that block. An error where the crate can't place the chunk.
fn block_size(header: &Header, block: &CompressedBlock) -> Result<(usize, usize), String> {
    if let CompressedBlock::Tile(tile) = block {
        // (the crate's level size asserts on levels of 64 and more; no valid file has 32)
        if tile.coordinates.level_index.0 >= 32 || tile.coordinates.level_index.1 >= 32 {
            return Err("a DWA tile of an impossible level".into());
        }
    }
    let tile = header.get_block_data_indices(block).map_err(|e| format!("a DWA chunk the decoder can't place: {e}"))?;
    let bounds = header.get_absolute_block_pixel_coordinates(tile).map_err(|e| format!("a DWA chunk the decoder can't place: {e}"))?;
    Ok((bounds.size.0, bounds.size.1))
}

/// Checks every chunk the offset tables point at that is, or names, a flat DWA part (`tables` is
/// where the offset tables start), before the `exr` crate's whole-image reader, which offers no
/// hook between reading a chunk and decompressing it, gets the bytes. It fails closed: a chunk of
/// a DWA part whose part number, coordinates, size or payload can't be read, or whose block can't
/// be placed, is an error, not left to the crate.
pub(crate) fn check_file(bytes: &[u8], meta: &exr::meta::MetaData, tables: usize) -> Result<(), String> {
    if !meta.headers.iter().any(|h| !h.deep && is_dwa(h.compression)) {
        return Ok(());
    }
    let multipart = meta.requirements.is_multilayer();
    let word = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(i32::from_le_bytes);
    let cut = || "a DWA chunk cut off by the end of the file".to_string();
    // Per part: its shape when it is a flat DWA part (worked out once, not per chunk).
    let shapes: Vec<Option<(usize, usize)>> = meta
        .headers
        .iter()
        .map(|h| if !h.deep && is_dwa(h.compression) { shape(h).ok_or("a DWA pixel size overflows").map(Some) } else { Ok(None) })
        .collect::<Result<_, _>>()?;
    let mut table = tables;
    for (table_part, header) in meta.headers.iter().enumerate() {
        let entries =
            bytes.get(table..table.saturating_add(header.chunk_count.saturating_mul(8))).ok_or("the offset tables run past the end of the file")?;
        table = table.saturating_add(header.chunk_count.saturating_mul(8));
        for entry in entries.as_chunks::<8>().0 {
            let offset = usize::try_from(u64::from_le_bytes(*entry)).ok();
            // The part the crate decodes the chunk as: the one it names (multi-part files).
            let (part, mut at) = match (multipart, offset) {
                (false, Some(at)) => (Some(table_part), at),
                (true, Some(at)) => (word(at).and_then(|p| usize::try_from(p).ok()), at.saturating_add(4)),
                (_, None) => (None, 0),
            };
            let named = part.and_then(|p| shapes.get(p).copied().flatten().map(|s| (p, s)));
            let Some((part, shape)) = named else {
                // (not a DWA chunk, unless it sits in a DWA part's table and can't be read)
                if shapes.get(table_part).copied().flatten().is_some() && part.is_none_or(|p| p >= meta.headers.len()) {
                    return Err(cut());
                }
                continue;
            };
            let header = meta.headers.get(part).ok_or_else(cut)?;
            let block = match header.blocks {
                exr::meta::BlockDescription::ScanLines => {
                    let y = word(at).ok_or_else(cut)?;
                    at = at.saturating_add(4);
                    CompressedBlock::ScanLine(CompressedScanLineBlock { y_coordinate: y, compressed_pixels_le: Vec::new() })
                }
                exr::meta::BlockDescription::Tiles(_) => {
                    let tile = [word(at), word(at.saturating_add(4)), word(at.saturating_add(8)), word(at.saturating_add(12))];
                    at = at.saturating_add(16);
                    let [Some(x), Some(y), Some(lx), Some(ly)] = tile else { return Err(cut()) };
                    let index = |v: i32| usize::try_from(v).map_err(|_| "a DWA tile at a negative position".to_string());
                    let coordinates = TileCoordinates { tile_index: Vec2(index(x)?, index(y)?), level_index: Vec2(index(lx)?, index(ly)?) };
                    CompressedBlock::Tile(CompressedTileBlock { coordinates, compressed_pixels_le: Vec::new() })
                }
            };
            let size = block_size(header, &block)?;
            let len = word(at).ok_or_else(cut)?;
            let len = usize::try_from(len).map_err(|_| "a DWA chunk declaring a negative size".to_string())?;
            let start = at.saturating_add(4);
            let payload = bytes.get(start..start.saturating_add(len)).ok_or_else(cut)?;
            check_shaped(shape, size, payload)?;
        }
    }
    Ok(())
}
