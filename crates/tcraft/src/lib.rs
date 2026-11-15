#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Streaming `TCTX` tile and baked-map payloads in TextureCraft `.tcraft` archives.
//!
//! This codec handles the neutral binary payload, not the application's project model or ZIP
//! envelope. Reads use a caller-owned budget shared with other archive entries. No reader opens
//! a file or reserves memory based on a claimed payload before checking it.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Write};

use craft_zip::Budget;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TILE_SIDE: u32 = 64;
pub const TILE_AREA: usize = 64 * 64;
pub const HEADER_BYTES: u64 = 24;
pub const VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"TCTX";
const CHUNK: usize = 1 << 16;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid TCTX payload: {0}")]
    Invalid(String),
    #[error("TCTX version {0} is newer than supported")]
    NewerVersion(u32),
    #[error("TCTX budget: {0}")]
    Budget(#[from] craft_zip::Exceeded),
    #[error("TCTX limit: {0}")]
    Limit(String),
    #[error("TCTX I/O: {0}")]
    Io(#[from] io::Error),
}

/// Neutral top-level project schema. Nested app-domain values and unknown fields remain JSON
/// so they round-trip without requiring TextureCraft's in-memory model.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ProjectDocument {
    pub format_version: u32,
    pub name: String,
    pub mesh: String,
    pub normal_format: Value,
    pub texture_sets: Vec<Value>,
    pub layers: BTreeMap<String, Value>,
    pub next_id: u64,
    pub display: Value,
    #[serde(default)]
    pub cameras: Vec<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Serialize for ProjectDocument {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error as _, SerializeMap as _};
        const RESERVED: &[&str] = &["format_version", "name", "mesh", "normal_format", "texture_sets", "layers", "next_id", "display", "cameras"];
        if self.extra.keys().any(|key| RESERVED.contains(&key.as_str())) {
            return Err(S::Error::custom("an extra project field duplicates a known field"));
        }
        let mut map = serializer.serialize_map(Some(9 + self.extra.len()))?;
        map.serialize_entry("format_version", &self.format_version)?;
        map.serialize_entry("name", &self.name)?;
        map.serialize_entry("mesh", &self.mesh)?;
        map.serialize_entry("normal_format", &self.normal_format)?;
        map.serialize_entry("texture_sets", &self.texture_sets)?;
        map.serialize_entry("layers", &self.layers)?;
        map.serialize_entry("next_id", &self.next_id)?;
        map.serialize_entry("display", &self.display)?;
        map.serialize_entry("cameras", &self.cameras)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Read only the version before an application applies its own recovery policy. Serde's
/// structured parse rejects a duplicate or invalid version field, like the project reader.
pub fn project_version(bytes: &[u8]) -> Result<u32, serde_json::Error> {
    #[derive(Deserialize)]
    struct Version {
        format_version: u32,
    }
    Ok(serde_json::from_slice::<Version>(bytes)?.format_version)
}

/// Width, height and number of components expected by the enclosing project schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub components: usize,
}

#[derive(Clone, Copy, Debug)]
struct Header {
    layout: Layout,
    count: u32,
}

fn charge(budget: &mut Budget, n: u64) -> Result<(), Error> {
    budget.charge(n).map_err(Error::from)
}

fn read_header(reader: &mut (impl Read + ?Sized), budget: &mut Budget) -> Result<Header, Error> {
    charge(budget, HEADER_BYTES)?;
    let mut bytes = [0u8; HEADER_BYTES as usize];
    reader.read_exact(&mut bytes)?;
    if bytes.get(..4) != Some(MAGIC.as_slice()) {
        return Err(Error::Invalid("bad magic".into()));
    }
    let word = |i: usize| bytes.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
    let version = word(1);
    if version > VERSION {
        return Err(Error::NewerVersion(version));
    }
    Ok(Header { layout: Layout { width: word(2), height: word(3), components: word(4) as usize }, count: word(5) })
}

fn write_header(writer: &mut (impl Write + ?Sized), layout: Layout, count: u32) -> Result<(), Error> {
    let components = u32::try_from(layout.components).map_err(|_| Error::Limit("too many components".into()))?;
    writer.write_all(MAGIC)?;
    for value in [VERSION, layout.width, layout.height, components, count] {
        writer.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

fn expect_end(reader: &mut (impl Read + ?Sized)) -> Result<(), Error> {
    let mut byte = [0u8; 1];
    if reader.read(&mut byte)? != 0 {
        return Err(Error::Invalid("trailing data".into()));
    }
    Ok(())
}

fn tile_dimensions(layout: Layout) -> Result<(u32, u32, usize), Error> {
    if layout.width == 0 || layout.height == 0 || layout.components == 0 {
        return Err(Error::Invalid("empty tile layout".into()));
    }
    let x = layout.width.div_ceil(TILE_SIDE);
    let y = layout.height.div_ceil(TILE_SIDE);
    let bytes = TILE_AREA.checked_mul(layout.components).and_then(|n| n.checked_mul(2)).ok_or_else(|| Error::Limit("tile is too large".into()))?;
    Ok((x, y, bytes))
}

/// Decode tiles one at a time, transferring each tile to the callback.
/// Positions and duplicate tiles are checked before delivery; the entry must end exactly.
pub fn read_tiles(
    reader: &mut (impl Read + ?Sized),
    expected: Layout,
    max_texels: u64,
    budget: &mut Budget,
    mut tile: impl FnMut(u32, u32, Vec<u16>) -> Result<(), Error>,
) -> Result<(), Error> {
    let header = read_header(reader, budget)?;
    let texels = u64::from(expected.width) * u64::from(expected.height);
    if texels == 0 || texels > max_texels {
        return Err(Error::Limit("tile image exceeds its texel limit".into()));
    }
    if header.layout != expected {
        return Err(Error::Invalid("tile layout differs from the project".into()));
    }
    let (tiles_x, tiles_y, bytes_per_tile) = tile_dimensions(expected)?;
    if u64::from(header.count) > u64::from(tiles_x) * u64::from(tiles_y) {
        return Err(Error::Invalid("more tiles than the image holds".into()));
    }
    let mut bytes = Vec::new();
    let mut seen = HashSet::new();
    for _ in 0..header.count {
        let payload = u64::try_from(bytes_per_tile).map_err(|_| Error::Limit("tile is too large".into()))?.saturating_add(8);
        charge(budget, payload)?;
        if bytes.is_empty() {
            bytes.try_reserve_exact(bytes_per_tile).map_err(|_| Error::Limit("tile allocation failed".into()))?;
            bytes.resize(bytes_per_tile, 0);
        }
        let mut pos = [0u8; 8];
        reader.read_exact(&mut pos)?;
        let x = u32::from_le_bytes([pos[0], pos[1], pos[2], pos[3]]);
        let y = u32::from_le_bytes([pos[4], pos[5], pos[6], pos[7]]);
        if x >= tiles_x || y >= tiles_y || seen.contains(&(x, y)) {
            return Err(Error::Invalid(format!("bad or repeated tile ({x}, {y})")));
        }
        seen.try_reserve(1).map_err(|_| Error::Limit("tile index allocation failed".into()))?;
        seen.insert((x, y));
        reader.read_exact(&mut bytes)?;
        let mut values = Vec::new();
        values.try_reserve_exact(bytes_per_tile / 2).map_err(|_| Error::Limit("tile allocation failed".into()))?;
        values.extend(bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)));
        tile(x, y, values)?;
    }
    expect_end(reader)
}

/// Write tiles in caller order, without buffering the image. A failure may leave a partial
/// entry; callers writing a project should discard the incomplete output.
pub fn write_tiles<'a>(
    writer: &mut (impl Write + ?Sized),
    layout: Layout,
    count: u32,
    tiles: impl IntoIterator<Item = (u32, u32, &'a [u16])>,
) -> Result<(), Error> {
    let (tiles_x, tiles_y, bytes_per_tile) = tile_dimensions(layout)?;
    if u64::from(count) > u64::from(tiles_x) * u64::from(tiles_y) {
        return Err(Error::Invalid("more tiles than the image holds".into()));
    }
    write_header(writer, layout, count)?;
    let mut seen = HashSet::new();
    let mut bytes = Vec::new();
    let mut written = 0u32;
    for (x, y, values) in tiles {
        if written >= count || x >= tiles_x || y >= tiles_y || seen.contains(&(x, y)) || values.len() != bytes_per_tile / 2 {
            return Err(Error::Invalid("bad, repeated or incorrectly sized tile".into()));
        }
        seen.try_reserve(1).map_err(|_| Error::Limit("tile index allocation failed".into()))?;
        seen.insert((x, y));
        if bytes.is_empty() {
            bytes.try_reserve_exact(bytes_per_tile).map_err(|_| Error::Limit("tile allocation failed".into()))?;
        }
        bytes.clear();
        bytes.extend(values.iter().flat_map(|v| v.to_le_bytes()));
        writer.write_all(&x.to_le_bytes())?;
        writer.write_all(&y.to_le_bytes())?;
        writer.write_all(&bytes)?;
        written += 1;
    }
    if written != count {
        return Err(Error::Invalid("tile count differs from its data".into()));
    }
    Ok(())
}

/// Read a dense f32 map, validating declared size and charging its shared budget first.
pub fn read_baked(
    reader: &mut (impl Read + ?Sized),
    declared_bytes: u64,
    components: usize,
    max_texels: u64,
    budget: &mut Budget,
) -> Result<(Layout, Vec<f32>), Error> {
    let header = read_header(reader, budget)?;
    let layout = header.layout;
    let texels = u64::from(layout.width) * u64::from(layout.height);
    if texels == 0 || texels > max_texels || layout.components != components || components == 0 {
        return Err(Error::Invalid("bad baked-map dimensions or components".into()));
    }
    let count = texels.checked_mul(components as u64).ok_or_else(|| Error::Limit("too many map values".into()))?;
    let payload = count.checked_mul(4).ok_or_else(|| Error::Limit("map is too large".into()))?;
    if declared_bytes != HEADER_BYTES.checked_add(payload).ok_or_else(|| Error::Limit("map is too large".into()))? {
        return Err(Error::Invalid(format!("{declared_bytes} bytes for a {}×{}×{} map", layout.width, layout.height, layout.components)));
    }
    charge(budget, payload)?;
    let count = usize::try_from(count).map_err(|_| Error::Limit("too many map values".into()))?;
    let mut values = Vec::new();
    let chunk_bytes = CHUNK.min(count) * 4;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(chunk_bytes).map_err(|_| Error::Limit("map chunk allocation failed".into()))?;
    bytes.resize(chunk_bytes, 0);
    while values.len() < count {
        let n = CHUNK.min(count - values.len());
        let chunk = bytes.get_mut(..n * 4).ok_or_else(|| Error::Invalid("bad map chunk".into()))?;
        reader.read_exact(chunk)?;
        values.try_reserve(n).map_err(|_| Error::Limit("map allocation failed".into()))?;
        values.extend(chunk.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
    }
    expect_end(reader)?;
    Ok((layout, values))
}

/// Write a dense f32 map in bounded chunks without constructing a second copy of its payload.
pub fn write_baked(writer: &mut (impl Write + ?Sized), layout: Layout, values: &[f32]) -> Result<(), Error> {
    let count = u64::from(layout.width).checked_mul(u64::from(layout.height)).and_then(|n| n.checked_mul(layout.components as u64));
    if layout.width == 0 || layout.height == 0 || layout.components == 0 || count != Some(values.len() as u64) {
        return Err(Error::Invalid("map dimensions differ from its data".into()));
    }
    write_header(writer, layout, 0)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(CHUNK * 4).map_err(|_| Error::Limit("map chunk allocation failed".into()))?;
    for chunk in values.chunks(CHUNK) {
        bytes.clear();
        bytes.extend(chunk.iter().flat_map(|v| v.to_le_bytes()));
        writer.write_all(&bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header(layout: Layout, count: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_header(&mut bytes, layout, count).unwrap();
        bytes
    }

    #[test]
    fn project_schema_keeps_nested_and_future_fields() {
        let json = serde_json::json!({
            "format_version": 2, "name": "Stone", "mesh": "mesh.obj", "normal_format": "openGl",
            "texture_sets": [{"name": "main", "futureSet": {"keep": true}}],
            "layers": {"1": {"kind": "paint", "futureLayer": [1, 2]}}, "next_id": 2,
            "display": {"futureDisplay": 4}, "mesh_file": "source.obj", "futureProject": [3]
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        assert_eq!(project_version(&bytes).unwrap(), 2);
        let document: ProjectDocument = serde_json::from_slice(&bytes).unwrap();
        let mut expected = json;
        expected["cameras"] = serde_json::json!([]);
        assert_eq!(serde_json::to_value(&document).unwrap(), expected);
        let mut duplicate = document;
        duplicate.extra.insert("mesh".into(), serde_json::json!("other.obj"));
        assert!(serde_json::to_vec(&duplicate).is_err());
        assert!(project_version(br#"{"format_version":1,"format_version":2}"#).is_err());
        assert!(project_version(br#"{"format_version":-1}"#).is_err());
    }

    #[test]
    fn baked_map_matches_the_native_little_endian_wire_layout() {
        let layout = Layout { width: 2, height: 1, components: 1 };
        let mut expected = b"TCTX".to_vec();
        for value in [1u32, 2, 1, 1, 0] {
            expected.extend_from_slice(&value.to_le_bytes());
        }
        expected.extend_from_slice(&1.0f32.to_le_bytes());
        expected.extend_from_slice(&(-0.5f32).to_le_bytes());
        let mut actual = Vec::new();
        write_baked(&mut actual, layout, &[1.0, -0.5]).unwrap();
        assert_eq!(actual, expected);
        let (decoded, values) = read_baked(&mut Cursor::new(&expected), expected.len() as u64, 1, 2, &mut Budget::new("project", 32)).unwrap();
        assert_eq!(decoded, layout);
        assert_eq!(values, [1.0, -0.5]);
        expected[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(read_baked(&mut Cursor::new(&expected), 32, 1, 2, &mut Budget::new("project", 32)), Err(Error::NewerVersion(2))));
    }

    #[test]
    fn dense_maps_round_trip_bit_exactly_in_bounded_chunks() {
        let layout = Layout { width: 300, height: 300, components: 1 };
        let mut values = vec![0.25; 90_000];
        values[123] = f32::from_bits(0x7fc0_1234);
        let mut bytes = Vec::new();
        write_baked(&mut bytes, layout, &values).unwrap();
        let mut budget = Budget::new("project", bytes.len() as u64);
        let (decoded_layout, decoded) = read_baked(&mut Cursor::new(&bytes), bytes.len() as u64, 1, 100_000, &mut budget).unwrap();
        assert_eq!(decoded_layout, layout);
        assert!(decoded.iter().zip(values.iter()).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn dense_maps_refuse_false_lengths_limits_and_trailing_data() {
        let layout = Layout { width: 2, height: 2, components: 1 };
        let mut bytes = Vec::new();
        write_baked(&mut bytes, layout, &[1.0; 4]).unwrap();
        let mut budget = Budget::new("project", 100);
        assert!(read_baked(&mut Cursor::new(&bytes), 999, 1, 4, &mut budget).is_err());
        let mut budget = Budget::new("project", 39);
        assert!(matches!(read_baked(&mut Cursor::new(&bytes), 40, 1, 4, &mut budget), Err(Error::Budget(_))));
        let mut extra = bytes.clone();
        extra.push(0);
        let mut budget = Budget::new("project", 100);
        assert!(read_baked(&mut Cursor::new(extra), 40, 1, 4, &mut budget).is_err());
    }

    #[test]
    fn hostile_headers_and_tiles_are_rejected_without_trusting_counts() {
        let layout = Layout { width: 64, height: 64, components: 1 };
        assert!(matches!(
            read_tiles(&mut Cursor::new(header(layout, 0)), layout, 4095, &mut Budget::new("project", 100), |_, _, _| Ok(())),
            Err(Error::Limit(_))
        ));
        let mut bad_magic = header(layout, 0);
        bad_magic[0] = b'X';
        assert!(read_tiles(&mut Cursor::new(bad_magic), layout, u64::MAX, &mut Budget::new("project", 100), |_, _, _| Ok(())).is_err());
        assert!(read_tiles(&mut Cursor::new(header(layout, 2)), layout, u64::MAX, &mut Budget::new("project", 100), |_, _, _| Ok(())).is_err());
        let other = Layout { width: 65, ..layout };
        assert!(read_tiles(&mut Cursor::new(header(layout, 0)), other, u64::MAX, &mut Budget::new("project", 100), |_, _, _| Ok(())).is_err());
        let mut outside = header(layout, 1);
        outside.extend_from_slice(&1u32.to_le_bytes());
        outside.extend_from_slice(&0u32.to_le_bytes());
        assert!(read_tiles(&mut Cursor::new(outside), layout, u64::MAX, &mut Budget::new("project", 10_000), |_, _, _| Ok(())).is_err());
        let mut truncated = header(layout, 1);
        truncated.extend_from_slice(&[0; 8]);
        assert!(read_tiles(&mut Cursor::new(truncated), layout, u64::MAX, &mut Budget::new("project", 10_000), |_, _, _| Ok(())).is_err());
        let mut extra = header(layout, 0);
        extra.push(1);
        assert!(read_tiles(&mut Cursor::new(extra), layout, u64::MAX, &mut Budget::new("project", 100), |_, _, _| Ok(())).is_err());
    }

    #[test]
    fn sparse_tiles_stream_and_reject_duplicates_and_short_budget() {
        let layout = Layout { width: 128, height: 64, components: 1 };
        let mut bytes = header(layout, 2);
        for x in [0u32, 1] {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            for _ in 0..TILE_AREA {
                bytes.extend_from_slice(&(x as u16 + 1).to_le_bytes());
            }
        }
        let first = vec![1u16; TILE_AREA];
        let second = vec![2u16; TILE_AREA];
        let mut written = Vec::new();
        write_tiles(&mut written, layout, 2, [(0, 0, first.as_slice()), (1, 0, second.as_slice())]).unwrap();
        assert_eq!(written, bytes, "the streamed writer keeps the native wire layout");
        assert!(write_tiles(&mut Vec::new(), layout, 2, [(0, 0, first.as_slice())]).is_err());
        let mut seen = Vec::new();
        let mut budget = Budget::new("project", bytes.len() as u64);
        read_tiles(&mut Cursor::new(&bytes), layout, u64::MAX, &mut budget, |x, y, tile| {
            seen.push((x, y, tile.first().copied()));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, [(0, 0, Some(1)), (1, 0, Some(2))]);
        let mut duplicate = bytes.clone();
        let second = HEADER_BYTES as usize + 8 + TILE_AREA * 2;
        duplicate[second..second + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(read_tiles(&mut Cursor::new(duplicate), layout, u64::MAX, &mut Budget::new("project", u64::MAX), |_, _, _| Ok(())).is_err());
        assert!(matches!(
            read_tiles(&mut Cursor::new(bytes), layout, u64::MAX, &mut Budget::new("project", HEADER_BYTES), |_, _, _| Ok(())),
            Err(Error::Budget(_))
        ));
    }
}
