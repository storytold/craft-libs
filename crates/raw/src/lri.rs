//! Light LRI (Light L16): a chain of `LELR` blocks, each a protocol-buffers message and the packed sensor data of
//! some of the camera's 16 modules.
//!
//! Sources: the protocol-buffers wire format (Google's public encoding documentation: varints, the four wire types
//! used here, length-delimited sub-messages) and our own black-box analysis of the one CC0 LRI sample on
//! raw.pixls.us (Light L16); every field number and value below was found on it.
//!
//! - Block: `LELR`, the block's length (u64, header included), the message's offset from the block start (u64) and
//!   length (u32), the message kind (u8: 0 = capture and calibration, 1 = the camera's view settings), zero padding
//!   to 32 bytes. Blocks follow each other to the end of the file; the sample has nine.
//! - Kind 0, field 12 (repeated): one module's capture. 2 = module id (0..=4 the 28 mm A1..A5, 5..=9 the 70 mm
//!   B1..B5, 10..=15 the 150 mm C1..C6), 4 = mirror position (only on the 70 mm modules of
//!   the sample, the ones that aim their view with a mirror; signed), 8 = exposure time in ns, 13 = the red site of the
//!   2x2 Bayer cell {1: column, 2: row}: the site whose mean is lowest under the overcast daylight of the sample and
//!   the one that renders its sky neutral (-1 on the monochrome module A2, whose four sites read the same),
//!   9 = the surface {2: {1: width, 2: height}, 3: format, 4: row stride in bytes, 5: data offset from the block
//!   start}. The sample stores five 4160 x 3120 modules a block, 16 MiB apart, all upright (the 70 mm modules' mirrors
//!   don't flip what is stored). Format 7 packs 10-bit samples least significant bits first: read that way,
//!   same-colour neighbours differ by 4.6 on average, against 330 read most significant bits first and 350 as MIPI
//!   RAW10.
//! - Kind 0, field 3: the capture time {1: year .. 6: second}, local (it becomes [`Camera::capture_unix`], the file
//!   having no Exif block); field 8: the model (`L16`).
//! - Kind 0, field 13 (repeated): a module's factory calibration, {1: module id} and one of these (the sample spreads
//!   them over three blocks, one record per module in each)
//!   - 2 (repeated): colour under one illuminant {1: illuminant, 2: white-balanced camera RGB → XYZ D50 (its rows sum
//!     to D50's white), 3: XYZ → camera RGB, 4: R/G and 5: B/G of the illuminant's white in camera RGB (3 maps that
//!     white to them), 6: 24 chart patches}. Matrix 3 and fields 4/5 put illuminant 0's white at xy (0.448, 0.409)
//!     (standard A), 2's at (0.314, 0.322) (D65) and 6's at (0.379, 0.374) (TL84); A and D65 are used. Each matrix
//!     is fields 1..=9 (f32), row-major.
//!   - 4: vignetting {2 (repeated): {1: mirror position, 2: {1: 17 columns, 2: 13 rows, 3: the gains, f32 row-major}}}:
//!     1.0 at the centre, up to 3.4 in a 28 mm module's corners, the inverse of the sample's sky falloff. The 70 mm
//!     modules have one grid per mirror position; the nearest to the capture's is used. The grid's outer points sit on
//!     the image's edges (it is symmetric about the centre point), so it is a DNG `GainMap` over the frame.
//! - Kind 0, field 16: the sensor {2: {1: black level, 2: white level}}: 42 and 1023 on the sample, whose darkest
//!   samples sit at 34..43.
//! - Kind 1, field 15: the as-shot white-balance gains {1: R, 2: Gr, 3: Gb, 4: B} (2.02, 1, 1, 1.48 on the sample:
//!   the D65 white of module A1 is R/G 0.49, B/G 0.70).
//!
//! One module is developed: the lowest-numbered colour module (A1 on the sample, a 28 mm view). Fusing the modules
//! into one picture, the per-site 4x4 crosstalk matrices of the vignetting record (field 13.4.1) and the geometric
//! calibration (field 13.3) are not read.
use crate::opcodes::{Area, Opcode};
use crate::{
    BlackLevel, Camera, Cfa, ColorData, LevelSource, LevelSources, Limits, Matrix3, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat,
    RawImage, Rect, Result,
};

const MAGIC: &[u8; 4] = b"LELR";
const HEADER: usize = 32;
/// More blocks than any capture writes (the sample has nine).
const MAX_BLOCKS: usize = 4096;
/// Surface format of 10-bit samples packed least significant bits first.
const FORMAT_PACKED10: u64 = 7;
/// Calibration illuminants as the file numbers them, and their Exif `LightSource` codes.
const ILLUMINANT_A: u64 = 0;
const ILLUMINANT_D65: u64 = 2;
const EXIF_STANDARD_A: u16 = 17;
const EXIF_D65: u16 = 21;
/// Sensor levels of the sample, for a file without the sensor record.
const DEFAULT_BLACK: f32 = 42.0;
const DEFAULT_WHITE: f32 = 1023.0;
/// Vignetting grid size (17 x 13 on the sample); larger grids are refused.
const MAX_GRID_POINTS: usize = 4096;
/// The smallest vignetting gain: unity, less f32 rounding.
const MIN_GAIN: f32 = 1.0 - 1e-4;

/// A protocol-buffers field value (wire types 0, 1, 2 and 5; groups are not used by LRI and are refused).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Val<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

/// A message, checked to be well formed: its bytes. Fields are read from them on demand, so a message costs no memory
/// beyond the file, however many fields it packs.
#[derive(Clone, Copy, Debug, Default)]
struct Msg<'a>(&'a [u8]);

/// The fields of a message in file order; ends after the first malformed field.
struct Fields<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = Result<(u64, Val<'a>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.b.len() {
            return None;
        }
        let field = field(self.b, &mut self.pos);
        if field.is_err() {
            self.pos = self.b.len();
        }
        Some(field)
    }
}

fn field<'a>(b: &'a [u8], pos: &mut usize) -> Result<(u64, Val<'a>)> {
    let key = varint(b, pos)?;
    let (num, wire) = (key >> 3, key & 7);
    let take = |pos: &mut usize, n: usize| -> Result<&'a [u8]> {
        let end = pos.checked_add(n).ok_or_else(|| corrupt("field length"))?;
        let s = b.get(*pos..end).ok_or_else(|| corrupt("field past the end of its message"))?;
        *pos = end;
        Ok(s)
    };
    let val = match wire {
        0 => Val::Varint(varint(b, pos)?),
        1 => Val::Fixed64(u64::from_le_bytes(take(pos, 8)?.try_into().map_err(|_| corrupt("fixed64"))?)),
        2 => {
            let n = usize::try_from(varint(b, pos)?).map_err(|_| corrupt("field length"))?;
            Val::Bytes(take(pos, n)?)
        }
        5 => Val::Fixed32(u32::from_le_bytes(take(pos, 4)?.try_into().map_err(|_| corrupt("fixed32"))?)),
        w => return Err(corrupt(&format!("wire type {w}"))),
    };
    Ok((num, val))
}

fn varint(b: &[u8], pos: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let c = *b.get(*pos).ok_or_else(|| corrupt("truncated varint"))?;
        let payload = u64::from(c & 0x7f);
        if shift == 63 && payload > 1 {
            return Err(corrupt("varint overflow"));
        }
        *pos += 1;
        v |= payload << shift;
        if c < 0x80 {
            return Ok(v);
        }
    }
    Err(corrupt("varint longer than 10 bytes"))
}

fn corrupt(what: &str) -> RawError {
    RawError::Corrupt(format!("LRI: {what}"))
}

impl<'a> Msg<'a> {
    fn parse(b: &'a [u8]) -> Result<Msg<'a>> {
        for field in (Fields { b, pos: 0 }) {
            field?;
        }
        Ok(Msg(b))
    }

    fn all(&self, num: u64) -> impl Iterator<Item = Val<'a>> + '_ {
        // `parse` checked every field, so none of them is an error here
        Fields { b: self.0, pos: 0 }.map_while(Result::ok).filter(move |(n, _)| *n == num).map(|(_, v)| v)
    }

    // A singular scalar field written more than once takes its last value, as protocol buffers read it.
    fn varint(&self, num: u64) -> Option<u64> {
        self.all(num).filter_map(|v| if let Val::Varint(x) = v { Some(x) } else { None }).last()
    }

    /// A varint field read as a signed (two's complement) number: -1 is written as 2^64 - 1.
    fn int(&self, num: u64) -> Option<i64> {
        self.varint(num).map(|v| v as i64)
    }

    fn f32(&self, num: u64) -> Option<f32> {
        self.all(num).filter_map(|v| if let Val::Fixed32(x) = v { Some(f32::from_bits(x)) } else { None }).last().filter(|v| v.is_finite())
    }

    fn bytes(&self, num: u64) -> Option<&'a [u8]> {
        self.all(num).filter_map(|v| if let Val::Bytes(b) = v { Some(b) } else { None }).last()
    }

    /// Repeated float field `num` in wire order: packed records (a length-delimited run of little-endian f32s) and
    /// single values alike. `None` past `max` values or for a packed record that isn't whole floats.
    fn f32s(&self, num: u64, max: usize) -> Option<Vec<f32>> {
        let mut out = Vec::new();
        for v in self.all(num) {
            // checked before copying, so an oversized packed record is refused without being copied
            let count = match v {
                Val::Fixed32(_) => 1,
                Val::Bytes(b) if b.len() % 4 == 0 => b.len() / 4,
                _ => return None,
            };
            if count > max.saturating_sub(out.len()) {
                return None;
            }
            match v {
                Val::Fixed32(x) => out.push(f32::from_bits(x)),
                Val::Bytes(b) => out.extend(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c))),
                _ => return None,
            }
        }
        Some(out)
    }

    /// All sub-messages `num`; malformed nested messages are corrupt input.
    fn subs(&self, num: u64) -> impl Iterator<Item = Result<Msg<'a>>> + '_ {
        self.all(num).filter_map(|v| if let Val::Bytes(b) = v { Some(b) } else { None }).map(Msg::parse)
    }

    /// The first sub-message `num`, once every one of them is checked.
    fn sub(&self, num: u64) -> Result<Option<Msg<'a>>> {
        let mut first = None;
        for m in self.subs(num) {
            first = first.or(Some(m?));
        }
        Ok(first)
    }
}

/// One `LELR` block: its bytes (header included), message and message kind.
struct Block<'a> {
    bytes: &'a [u8],
    msg: Msg<'a>,
    msg_at: usize,
    kind: u8,
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

fn blocks(bytes: &[u8]) -> Result<Vec<Block<'_>>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        if out.len() >= MAX_BLOCKS {
            return Err(RawError::Limit("LRI block count"));
        }
        let rest = bytes.get(at..).unwrap_or_default();
        if !rest.starts_with(MAGIC) || rest.len() < HEADER {
            return Err(corrupt(&format!("no block header at byte {at}")));
        }
        let len = u64_at(rest, 4).and_then(|v| usize::try_from(v).ok()).filter(|&l| (HEADER..=rest.len()).contains(&l));
        let len = len.ok_or_else(|| corrupt(&format!("block at byte {at} runs past the end of the file")))?;
        let block = rest.get(..len).unwrap_or_default();
        let msg_at = u64_at(block, 12).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| corrupt("message offset"))?;
        let msg_len = block.get(20..24).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or_else(|| corrupt("message length"))?;
        let msg = msg_at
            .checked_add(msg_len as usize)
            .and_then(|end| block.get(msg_at..end))
            .ok_or_else(|| corrupt(&format!("message of the block at byte {at} outside the block")))?;
        let kind = block.get(24).copied().unwrap_or_default();
        out.push(Block { bytes: block, msg: Msg::parse(msg)?, msg_at, kind });
        at += len;
    }
    Ok(out)
}

/// One module's capture, with its samples' place in the file.
struct Module<'a> {
    id: u64,
    width: usize,
    height: usize,
    stride: usize,
    /// From the first sample on (rows `stride` bytes apart).
    data: &'a [u8],
    /// The red site of the 2x2 cell (column, row); `None` on a monochrome module.
    red: Option<(usize, usize)>,
    /// Field 4, signed; 0 when absent, as on the 28 mm modules (their single grid is keyed 0).
    mirror_position: i64,
}

fn module<'a>(block: &'a [u8], msg_at: usize, m: &Msg<'a>) -> Result<Option<Module<'a>>> {
    let Some(id) = m.varint(2) else { return Ok(None) };
    if id > 15 {
        return Ok(None);
    }
    let Some(surface) = m.sub(9)? else { return Ok(None) };
    let Some(size) = surface.sub(2)? else { return Ok(None) };
    let (Some(width), Some(height)) = (size.varint(1).and_then(|v| usize::try_from(v).ok()), size.varint(2).and_then(|v| usize::try_from(v).ok()))
    else {
        return Ok(None);
    };
    let (Some(stride), Some(offset)) =
        (surface.varint(4).and_then(|v| usize::try_from(v).ok()), surface.varint(5).and_then(|v| usize::try_from(v).ok()))
    else {
        return Ok(None);
    };
    let Some(row_bytes) = width.checked_mul(10).map(|v| v.div_ceil(8)) else { return Ok(None) };
    // even dimensions: whole 2x2 cells
    if surface.varint(3) != Some(FORMAT_PACKED10) || width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 || stride < row_bytes {
        return Ok(None);
    }
    let Some(end) = stride.checked_mul(height - 1).and_then(|v| v.checked_add(row_bytes)).and_then(|v| v.checked_add(offset)) else {
        return Ok(None);
    };
    if offset < HEADER || end > msg_at {
        return Err(corrupt("module sample span outside the data area"));
    }
    let data = block.get(offset..end).ok_or_else(|| corrupt("module sample span outside the block"))?;
    let red = m.sub(13)?.and_then(|b| match (b.int(1)?, b.int(2)?) {
        (x @ 0..=1, y @ 0..=1) => Some((x as usize, y as usize)),
        _ => None,
    });
    Ok(Some(Module { id, width, height, stride, data, red, mirror_position: m.int(4).unwrap_or(0) }))
}

/// A Bayer cell with red at (`x`, `y`): blue on the opposite corner, green on the other two.
fn bayer(x: usize, y: usize) -> Cfa {
    let mut pattern = vec![1u8; 4];
    pattern[y * 2 + x] = 0;
    pattern[(1 - y) * 2 + (1 - x)] = 2;
    Cfa { width: 2, height: 2, pattern, origin_x: 0, origin_y: 0 }
}

fn mat3(m: &Msg) -> Option<Matrix3> {
    let mut v = [0f64; 9];
    for (i, x) in v.iter_mut().enumerate() {
        *x = f64::from(m.f32(i as u64 + 1)?);
    }
    Some([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]])
}

/// Whether `m` has an inverse: a determinant of magnitude at least 1e-300 (an all-zero matrix has none).
fn invertible(m: &Matrix3) -> bool {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    det.is_finite() && det.abs() >= 1e-300
}

/// Calls `f` with each of module `id`'s calibration records (kind-0 field 13), in file order.
fn calibration<'a>(headers: &[&Block<'a>], id: u64, mut f: impl FnMut(Msg<'a>) -> Result<()>) -> Result<()> {
    for b in headers {
        for c in b.msg.subs(13) {
            let c = c?;
            if c.varint(1) == Some(id) {
                f(c)?;
            }
        }
    }
    Ok(())
}

/// The module's colour matrices (XYZ → camera) and forward matrices under standard A and D65, from its calibration
/// records.
fn colour(headers: &[&Block], id: u64) -> Result<ColorData> {
    let mut color = ColorData::default();
    calibration(headers, id, |cal| {
        for c in cal.subs(2) {
            let c = c?;
            let slot = match c.varint(1) {
                Some(ILLUMINANT_A) => 0,
                Some(ILLUMINANT_D65) => 1,
                _ => continue,
            };
            // a singular colour matrix (all zeros, say) has no camera neutral, and a singular forward matrix sends
            // every colour to black: the other illuminant's pair, or none, is used
            let forward = c.sub(2)?.as_ref().and_then(mat3).filter(invertible);
            let matrix = c.sub(3)?.as_ref().and_then(mat3).filter(invertible);
            let (Some(forward), Some(matrix)) = (forward, matrix) else { continue };
            color.illuminant[slot] = [EXIF_STANDARD_A, EXIF_D65][slot];
            color.color_matrix[slot] = Some(matrix);
            color.forward_matrix[slot] = Some(forward);
        }
        Ok(())
    })?;
    // one illuminant alone sits in the first slot, as a single-illuminant DNG has it
    if color.color_matrix[0].is_none() {
        color.illuminant = [color.illuminant[1], 0];
        color.color_matrix = [color.color_matrix[1], None];
        color.forward_matrix = [color.forward_matrix[1], None];
    }
    Ok(color)
}

/// The module's vignetting gains, from its calibration records, as an `OpcodeList2` gain map over the
/// `width x height` frame (the grid nearest `mirror_position`).
fn vignetting(headers: &[&Block], id: u64, mirror_position: i64, width: usize, height: usize) -> Result<Option<Opcode>> {
    // the grid nearest the mirror position, the first of equally near ones
    let mut nearest: Option<(u64, Msg)> = None;
    calibration(headers, id, |c| {
        if let Some(v) = c.sub(4)? {
            for g in v.subs(2) {
                let g = g?;
                if let Some(grid) = g.sub(2)? {
                    let off = g.int(1).unwrap_or(0).abs_diff(mirror_position);
                    if nearest.is_none_or(|(best, _)| off < best) {
                        nearest = Some((off, grid));
                    }
                }
            }
        }
        Ok(())
    })?;
    let Some((_, grid)) = nearest else { return Ok(None) };
    let (Some(cols), Some(rows)) = (grid.varint(1).and_then(|v| usize::try_from(v).ok()), grid.varint(2).and_then(|v| usize::try_from(v).ok()))
    else {
        return Ok(None);
    };
    let Some(n) = cols.checked_mul(rows).filter(|&n| (4..=MAX_GRID_POINTS).contains(&n) && cols >= 2 && rows >= 2) else { return Ok(None) };
    let Some(gains) = grid.f32s(3, n).filter(|g| g.len() == n) else { return Ok(None) };
    // a gain map that darkens, or a NaN, is not a vignetting correction: the gains are at least 1 (the sample's
    // centres are 1.0, or 1 - 6e-8 after f32 rounding)
    if gains.iter().any(|g| !(g.is_finite() && (MIN_GAIN..=16.0).contains(g))) {
        return Ok(None);
    }
    let area = Area { top: 0, left: 0, bottom: height as u32, right: width as u32, plane: 0, planes: 1, row_pitch: 1, col_pitch: 1 };
    // the gain-map reader places pixel `y` at `y / height`, so the last row and column sit at `(n - 1) / n`: spacing
    // the grid over that span puts its outer points on the frame's edge pixels
    let span = |n: usize| n.saturating_sub(1) as f64 / n.max(1) as f64;
    let spacing = [span(height) / (rows - 1) as f64, span(width) / (cols - 1) as f64];
    Ok(Some(Opcode::GainMap { area, points_v: rows as u32, points_h: cols as u32, spacing, origin: [0.0, 0.0], map_planes: 1, gains }))
}

/// Camera RGB of the as-shot white from the view settings' gains.
fn as_shot_neutral(blocks: &[Block]) -> Result<Option<[f64; 3]>> {
    let mut g = None;
    for b in blocks.iter().filter(|b| b.kind == 1) {
        if let Some(value) = b.msg.sub(15)? {
            g = Some(value);
            break;
        }
    }
    let Some(g) = g else { return Ok(None) };
    let neutral = (|| {
        let (r, gr, gb, b) = (g.f32(1)?, g.f32(2)?, g.f32(3)?, g.f32(4)?);
        let green = f64::from(gr + gb) / 2.0;
        let n = [green / f64::from(r), 1.0, green / f64::from(b)];
        n.iter().all(|v| v.is_finite() && *v > 0.0).then_some(n)
    })();
    Ok(neutral)
}

/// The capture time as Unix seconds on the camera's local clock (no UTC offset), as [`Camera::capture_unix`] holds it;
/// `None` for an impossible date or time.
fn capture_time(m: &Msg) -> Result<Option<i64>> {
    let Some(t) = m.sub(3)? else { return Ok(None) };
    let part = |n| t.varint(n).and_then(|v| u32::try_from(v).ok());
    let time = (|| {
        let date = chrono::NaiveDate::from_ymd_opt(i32::try_from(t.varint(1)?).ok()?, part(2)?, part(3)?)?;
        Some(date.and_hms_opt(part(4)?, part(5)?, part(6)?)?.and_utc().timestamp())
    })();
    Ok(time)
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let blocks = blocks(bytes)?;
    let headers: Vec<&Block> = blocks.iter().filter(|b| b.kind == 0).collect();
    // the lowest-numbered colour module, the first of equally numbered ones
    let mut colour_module: Option<Module> = None;
    for b in &headers {
        for msg in b.msg.subs(12) {
            if let Some(m) = module(b.bytes, b.msg_at, &msg?)?.filter(|m| m.red.is_some())
                && colour_module.as_ref().is_none_or(|best| m.id < best.id)
            {
                colour_module = Some(m);
            }
        }
    }
    let m = colour_module.ok_or_else(|| RawError::Unsupported("LRI without a colour module of 10-bit packed samples".into()))?;
    let (w, h) = (m.width, m.height);
    let n = w.checked_mul(h).filter(|n| *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let (rx, ry) = m.red.unwrap_or_default();
    let mut sensor = None;
    for b in &headers {
        if let Some(s) = b.msg.sub(16)? {
            sensor = s.sub(2)?;
            if sensor.is_some() {
                break;
            }
        }
    }
    let stated_black = sensor.as_ref().and_then(|s| s.f32(1)).filter(|v| (0.0..1023.0).contains(v));
    let black = stated_black.unwrap_or(DEFAULT_BLACK);
    let stated_white = sensor.as_ref().and_then(|s| s.f32(2)).filter(|v| *v > black && *v <= 1023.0);
    let white = stated_white.unwrap_or(DEFAULT_WHITE);
    let source = |stated: Option<f32>| if stated.is_some() { LevelSource::Metadata } else { LevelSource::Fallback };
    let level_sources = LevelSources { black: source(stated_black), white: source(stated_white) };
    let mut color = colour(&headers, m.id)?;
    color.as_shot_neutral = as_shot_neutral(&blocks)?;
    let mut opcodes = OpcodeLists::default();
    opcodes.list2.extend(vignetting(&headers, m.id, m.mirror_position, w, h)?);
    let data = if mode == Mode::Full {
        // 2^30 samples are 2 GiB, more than one allocation may take on a 32-bit target: refuse rather than abort
        let mut d = Vec::new();
        d.try_reserve_exact(n).map_err(|_| RawError::Limit("LRI sample allocation"))?;
        d.resize(n, 0u16);
        use rayon::prelude::*;
        d.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let src = m.data.get(y * m.stride..).unwrap_or_default();
            crate::unpack::unpack_lsb(src, 10, row);
        });
        RawData::U16(d)
    } else {
        RawData::U16(Vec::new())
    };
    let mut capture_unix = None;
    for b in &headers {
        if let Some(t) = capture_time(&b.msg)? {
            capture_unix = Some(t);
            break;
        }
    }
    let camera = Camera {
        make: Some("Light".into()),
        model: headers.iter().find_map(|b| b.msg.bytes(8)).map(|s| String::from_utf8_lossy(s).trim().to_string()).filter(|s| !s.is_empty()),
        unique_model: None,
        capture_unix,
    };
    let img = RawImage {
        format: RawFormat::Lri,
        width: w,
        height: h,
        cpp: 1,
        data,
        cfa: Some(bayer(rx, ry)),
        bits: 10,
        black: BlackLevel::uniform(black),
        white: vec![white],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::Normal,
        color,
        wb_multipliers: None,
        linearized: false,
        linearization_table: None,
        pixel_aspect: 1.0,
        opcodes,
        camera,
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources,
        notices: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(num: u64, wire: u64, out: &mut Vec<u8>) {
        put_varint((num << 3) | wire, out);
    }

    fn put_varint(mut v: u64, out: &mut Vec<u8>) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn var(num: u64, v: u64) -> Vec<u8> {
        let mut o = Vec::new();
        key(num, 0, &mut o);
        put_varint(v, &mut o);
        o
    }

    fn f32f(num: u64, v: f32) -> Vec<u8> {
        let mut o = Vec::new();
        key(num, 5, &mut o);
        o.extend(v.to_le_bytes());
        o
    }

    fn bytes(num: u64, b: &[u8]) -> Vec<u8> {
        let mut o = Vec::new();
        key(num, 2, &mut o);
        put_varint(b.len() as u64, &mut o);
        o.extend(b);
        o
    }

    fn matrix(num: u64, m: [f32; 9]) -> Vec<u8> {
        bytes(num, &m.iter().enumerate().flat_map(|(i, v)| f32f(i as u64 + 1, *v)).collect::<Vec<_>>())
    }

    fn block(msg: &[u8], data: &[u8], kind: u8) -> Vec<u8> {
        let msg_at = HEADER + data.len();
        let len = msg_at + msg.len();
        let mut b = Vec::new();
        b.extend(MAGIC);
        b.extend((len as u64).to_le_bytes());
        b.extend((msg_at as u64).to_le_bytes());
        b.extend((msg.len() as u32).to_le_bytes());
        b.push(kind);
        b.resize(HEADER, 0);
        b.extend(data);
        b.extend(msg);
        b
    }

    /// `px` as rows of `w` samples, each packed to whole bytes (the stride `module_msg` writes).
    fn pack10_rows(px: &[u16], w: usize) -> Vec<u8> {
        px.chunks(w).flat_map(pack10).collect()
    }

    fn pack10(px: &[u16]) -> Vec<u8> {
        let (mut acc, mut n, mut out) = (0u64, 0u32, Vec::new());
        for &p in px {
            acc |= u64::from(p) << n;
            n += 10;
            while n >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    /// A module capture message (field 12) with its surface at `offset`.
    fn module_msg(id: u64, w: u64, h: u64, offset: u64, red: (i64, i64)) -> Vec<u8> {
        let size = [var(1, w), var(2, h)].concat();
        let surface = [bytes(2, &size), var(3, FORMAT_PACKED10), var(4, (w * 10).div_ceil(8)), var(5, offset)].concat();
        let bayer = [var(1, red.0 as u64), var(2, red.1 as u64)].concat();
        bytes(12, &[var(2, id), var(8, 2_000_000), bytes(9, &surface), bytes(13, &bayer)].concat())
    }

    /// A two-module file (a monochrome A2 and a colour B1 with red at (1, 0)), calibration, sensor record and the
    /// view settings' white balance.
    fn sample(w: usize, h: usize) -> (Vec<u8>, Vec<u16>) {
        let px: Vec<u16> = (0..w * h).map(|i| 42 + (i * 37 % 900) as u16).collect();
        let higher_id_px: Vec<u16> = (0..w * h).map(|i| 100 + (i * 19 % 800) as u16).collect();
        let mono: Vec<u16> = vec![500; w * h];
        let (a, b, c) = (pack10_rows(&mono, w), pack10_rows(&higher_id_px, w), pack10_rows(&px, w));
        let data = [a.clone(), b.clone(), c].concat();
        let time = [var(1, 2017), var(2, 9), var(3, 28), var(4, 6), var(5, 24), var(6, 20)].concat();
        let capture = [
            bytes(3, &time),
            module_msg(1, w as u64, h as u64, HEADER as u64, (-1, -1)),
            // Keep ID 5 before ID 0 to verify selection is by module ID, not file order.
            module_msg(5, w as u64, h as u64, (HEADER + a.len()) as u64, (1, 0)),
            module_msg(0, w as u64, h as u64, (HEADER + a.len() + b.len()) as u64, (1, 0)),
        ]
        .concat();
        let colour_a = [
            var(1, ILLUMINANT_A),
            matrix(2, [0.52, 0.33, 0.12, 0.05, 1.11, -0.16, -0.39, -0.9, 2.12]),
            matrix(3, [0.97, -0.39, 0.02, -0.2, 0.98, 0.26, 0.06, 0.13, 0.44]),
        ]
        .concat();
        let colour_d65 = [
            var(1, ILLUMINANT_D65),
            matrix(2, [0.81, 0.25, -0.09, 0.28, 1.13, -0.4, -0.07, -0.39, 1.29]),
            matrix(3, [0.76, -0.18, -0.04, -0.32, 1.1, 0.25, -0.01, 0.22, 0.48]),
        ]
        .concat();
        let gains: Vec<u8> = [2.0f32, 1.5, 2.0, 1.5, 1.0, 1.5, 2.0, 1.5, 2.0].iter().flat_map(|g| g.to_le_bytes()).collect();
        let grid = [var(1, 3), var(2, 3), bytes(3, &gains)].concat();
        let vignetting = bytes(4, &bytes(2, &[var(1, 0), bytes(2, &grid)].concat()));
        // Calibration for the monochrome module, then ID 5, then ID 0. Both colour modules are valid;
        // ID 0's records are deliberately later so selection must use the lowest ID, not record order.
        let other = bytes(13, &[var(1, 1), bytes(2, &colour_a)].concat());
        let cal = [
            other,
            bytes(13, &[var(1, 5), bytes(2, &colour_a), bytes(2, &colour_d65)].concat()),
            bytes(13, &[var(1, 0), vignetting].concat()),
            bytes(13, &[var(1, 0), bytes(2, &colour_a), bytes(2, &colour_d65)].concat()),
        ]
        .concat();
        let sensor = bytes(16, &[var(1, 2), bytes(2, &[f32f(1, 42.0), f32f(2, 1023.0)].concat())].concat());
        let header = [bytes(8, b"L16"), cal, sensor].concat();
        let wb = bytes(15, &[f32f(1, 2.0), f32f(2, 1.0), f32f(3, 1.0), f32f(4, 1.5)].concat());
        let file = [block(&capture, &data, 0), block(&header, &[], 0), block(&wb, &[], 1)].concat();
        (file, px)
    }

    /// A zero colour matrix was kept, and at white points beyond its illuminant the colour transform used it alone,
    /// so the camera neutral came out [1, 1, 1] and ignored the scene white; a zero forward matrix sent every colour
    /// to black.
    #[test]
    fn a_singular_colour_matrix_is_not_used() {
        let forward = [0.81, 0.25, -0.09, 0.28, 1.13, -0.4, -0.07, -0.39, 1.29];
        let good = [0.76, -0.18, -0.04, -0.32, 1.1, 0.25, -0.01, 0.22, 0.48];
        let record = |illuminant, fwd, m| [var(1, illuminant), matrix(2, fwd), matrix(3, m)].concat();
        let colour_with = |fwd_a: [f32; 9], a: [f32; 9], d65: [f32; 9]| {
            let (ra, rd) = (record(ILLUMINANT_A, fwd_a, a), record(ILLUMINANT_D65, forward, d65));
            let cal = bytes(13, &[var(1, 0), bytes(2, &ra), bytes(2, &rd)].concat());
            let f = block(&cal, &[], 0);
            let blocks = blocks(&f).unwrap();
            colour(&blocks.iter().collect::<Vec<_>>(), 0).unwrap()
        };
        let colour_of = |a, d65| colour_with(forward, a, d65);
        let as_mat = |m: [f32; 9]| -> Matrix3 { std::array::from_fn(|r| std::array::from_fn(|c| f64::from(m[r * 3 + c]))) };
        // a zero D65 matrix leaves standard A alone
        let c = colour_of(good, [0.0; 9]);
        assert_eq!(c.illuminant, [EXIF_STANDARD_A, 0]);
        assert_eq!(c.color_matrix, [Some(as_mat(good)), None]);
        // a zero A matrix leaves D65 alone, in the first slot as a single-illuminant DNG has it
        let c = colour_of([0.0; 9], good);
        assert_eq!(c.illuminant, [EXIF_D65, 0]);
        assert_eq!(c.color_matrix, [Some(as_mat(good)), None]);
        // both zero: no camera matrix, so the decoder's generic colour model applies
        assert_eq!(colour_of([0.0; 9], [0.0; 9]).color_matrix, [None, None]);
        // a zero forward matrix beside a good colour matrix drops that illuminant too
        let c = colour_with([0.0; 9], good, good);
        assert_eq!(c.illuminant, [EXIF_D65, 0]);
        assert_eq!(c.forward_matrix, [Some(as_mat(forward)), None]);
    }

    /// A singular scalar written twice takes its last value, as protocol buffers read it.
    #[test]
    fn a_repeated_scalar_takes_its_last_value() {
        let m = [var(1, 7), f32f(2, 1.5), var(1, 9), f32f(2, 2.5)].concat();
        let m = Msg::parse(&m).unwrap();
        assert_eq!((m.varint(1), m.f32(2)), (Some(9), Some(2.5)));
    }

    /// A 3x3 grid's gains, stored as `records` of field 3, keyed `key`, for module 0, read at `mirror`.
    fn grid_gains(records: &[Vec<u8>], grids: &[(i64, f32)], mirror: i64) -> Option<Vec<f32>> {
        let grid = |key: i64, centre: f32| {
            let mut gains = [2.0f32; 9];
            gains[4] = centre;
            let body: Vec<u8> =
                if records.is_empty() { bytes(3, &gains.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()) } else { records.concat() };
            bytes(2, &[var(1, key as u64), bytes(2, &[var(1, 3), var(2, 3), body].concat())].concat())
        };
        let all: Vec<u8> = grids.iter().flat_map(|&(k, c)| grid(k, c)).collect();
        let cal = bytes(13, &[var(1, 0), bytes(4, &all)].concat());
        let f = block(&cal, &[], 0);
        let blocks = blocks(&f).unwrap();
        match vignetting(&blocks.iter().collect::<Vec<_>>(), 0, mirror, 16, 6).unwrap()? {
            Opcode::GainMap { gains, .. } => Some(gains),
            _ => None,
        }
    }

    /// A repeated float field may be packed in several records or written value by value: all of them, in order,
    /// are the gains.
    #[test]
    fn gains_split_over_records_are_joined() {
        let floats = |v: &[f32]| v.iter().flat_map(|g| g.to_le_bytes()).collect::<Vec<u8>>();
        let want = vec![2.0, 1.5, 2.0, 1.5, 1.0, 1.5, 2.0, 1.5, 2.0];
        // four packed, then one unpacked, then four packed
        let split = [bytes(3, &floats(&want[..4])), f32f(3, want[4]), bytes(3, &floats(&want[5..]))];
        assert_eq!(grid_gains(&split, &[(0, 1.0)], 0), Some(want.clone()));
        // a tenth gain, or a packed record that isn't whole floats, is not a 3x3 grid
        let long = [bytes(3, &floats(&want)), f32f(3, 2.0)];
        assert_eq!(grid_gains(&long, &[(0, 1.0)], 0), None);
        let ragged = [bytes(3, &floats(&want)[..35])];
        assert_eq!(grid_gains(&ragged, &[(0, 1.0)], 0), None);
        // a record is measured against the room left before it is copied: exactly `max` fits, one more doesn't
        let m = [f32f(1, 1.0), bytes(1, &floats(&[2.0; 8]))].concat();
        let m = Msg::parse(&m).unwrap();
        assert_eq!(m.f32s(1, 9).map(|v| v.len()), Some(9));
        assert_eq!(m.f32s(1, 8), None);
    }

    /// The mirror position and the grids' keys are signed: read unsigned, a grid at -5 was 2^64 - 5, the farthest
    /// from a mirror at 10 instead of the nearest.
    #[test]
    fn the_nearest_grid_is_chosen_by_signed_mirror_position() {
        let centre = |g: Option<Vec<f32>>| g.and_then(|g| g.get(4).copied());
        assert_eq!(centre(grid_gains(&[], &[(100, 1.25), (-5, 1.0)], 10)), Some(1.0));
        assert_eq!(centre(grid_gains(&[], &[(100, 1.25), (-5, 1.0)], 80)), Some(1.25));
    }

    /// Rows are packed to whole bytes: at a width that isn't a multiple of 4 the row stride is rounded up.
    #[test]
    fn a_width_of_whole_bytes_per_row_decodes() {
        let (f, px) = sample(6, 4);
        assert_eq!(crate::decode(&f).unwrap().data, RawData::U16(px));
    }

    /// Gains under 1 darken the frame they are meant to brighten: such a grid is not a vignetting correction.
    #[test]
    fn a_darkening_gain_grid_is_not_used() {
        let map = |g: [f32; 9]| {
            let gains: Vec<u8> = g.iter().flat_map(|v| v.to_le_bytes()).collect();
            let grid = [var(1, 3), var(2, 3), bytes(3, &gains)].concat();
            let cal = bytes(13, &[var(1, 0), bytes(4, &bytes(2, &[var(1, 0), bytes(2, &grid)].concat()))].concat());
            let f = block(&cal, &[], 0);
            let blocks = blocks(&f).unwrap();
            vignetting(&blocks.iter().collect::<Vec<_>>(), 0, 0, 16, 6).unwrap()
        };
        // unity at the centre, or a hair under it after f32 rounding, is a correction
        assert!(map([2.0, 1.5, 2.0, 1.5, 1.0, 1.5, 2.0, 1.5, 2.0]).is_some());
        assert!(map([2.0, 1.5, 2.0, 1.5, 0.99999994, 1.5, 2.0, 1.5, 2.0]).is_some());
        // a centre at 0.9, or corners below unity, is not
        assert!(map([2.0, 1.5, 2.0, 1.5, 0.9, 1.5, 2.0, 1.5, 2.0]).is_none());
        assert!(map([0.6; 9]).is_none());
    }

    #[test]
    fn decodes_the_colour_module() {
        let (f, px) = sample(16, 6);
        assert_eq!(crate::probe(&f), Some(RawFormat::Lri));
        let r = crate::decode(&f).unwrap();
        assert_eq!((r.width, r.height, r.bits), (16, 6, 10));
        assert_eq!(r.data, RawData::U16(px.clone()), "the colour module, not the monochrome one");
        assert_eq!(r.cfa.as_ref().map(|c| c.name()).as_deref(), Some("GRBG"));
        assert_eq!((r.black.mean(), r.white_at(0)), (42.0, 1023.0));
        assert_eq!(r.color.illuminant, [EXIF_STANDARD_A, EXIF_D65]);
        assert!(r.color.color_matrix.iter().chain(&r.color.forward_matrix).all(Option::is_some));
        assert_eq!(r.color.as_shot_neutral, Some([0.5, 1.0, 1.0 / 1.5]));
        assert_eq!(r.camera.make.as_deref(), Some("Light"));
        assert_eq!(r.camera.model.as_deref(), Some("L16"));
        // 2017-09-28 06:24:20 on the camera's clock
        assert_eq!(r.camera.capture_unix, Some(1_506_579_860));
        assert_eq!(r.level_sources, LevelSources { black: LevelSource::Metadata, white: LevelSource::Metadata });
        assert_eq!(crate::probe_info(&f).unwrap(), r.info());
        // the grid doubles the corners and leaves the centre, over the whole frame
        let [Opcode::GainMap { area, points_v: 3, points_h: 3, spacing, origin: [0.0, 0.0], map_planes: 1, gains }] = &r.opcodes.list2[..] else {
            panic!("one 3x3 gain map: {:?}", r.opcodes.list2)
        };
        assert_eq!((area.top, area.left, area.bottom, area.right), (0, 0, 6, 16));
        assert_eq!(gains.as_slice(), [2.0, 1.5, 2.0, 1.5, 1.0, 1.5, 2.0, 1.5, 2.0]);
        // a DNG gain-map reader samples pixel y at y / height, so the grid's last row and column must sit on the last
        // pixel row and column, (n - 1) / n, for the edge pixels to take the edge gains as the first ones do
        assert!((2.0 * spacing[0] - 5.0 / 6.0).abs() < 1e-12 && (2.0 * spacing[1] - 15.0 / 16.0).abs() < 1e-12, "{spacing:?}");
    }

    #[test]
    fn varint_rejects_overflowing_tenth_byte() {
        let mut bytes = vec![0x80; 9];
        bytes.push(0x02);
        let mut pos = 0;
        assert!(matches!(varint(&bytes, &mut pos), Err(RawError::Corrupt(_))));
        assert_eq!(pos, 9, "the overflowing byte is not consumed");
    }

    #[test]
    fn red_site_sets_the_pattern() {
        assert_eq!(bayer(0, 0).name(), "RGGB");
        assert_eq!(bayer(1, 0).name(), "GRBG");
        assert_eq!(bayer(0, 1).name(), "GBRG");
        assert_eq!(bayer(1, 1).name(), "BGGR");
    }

    #[test]
    fn monochrome_only_is_unsupported() {
        let w = 8;
        let data = pack10(&[100; 8 * 2]);
        let f = block(&module_msg(1, w, 2, HEADER as u64, (-1, -1)), &data, 0);
        assert!(matches!(crate::decode(&f), Err(RawError::Unsupported(_))));
    }

    /// A message packed with two-byte fields once grew 16-fold in memory as parsed fields (and its nested records
    /// again as vectors of messages); a message is now its bytes, read in place.
    #[test]
    fn a_message_of_many_fields_costs_no_memory_of_its_own() {
        assert_eq!(std::mem::size_of::<Msg>(), std::mem::size_of::<&[u8]>());
        // 4 Mi fields of field 1 = 0, then nested calibration records of the same kind, and no module
        let flood: Vec<u8> = [0x08, 0x00].repeat(4 << 20);
        let records: Vec<u8> = bytes(13, &[0x08, 0x00]).repeat(1 << 20);
        let f = block(&[flood, records].concat(), &[], 0);
        let m = Msg::parse(&f[HEADER..]).unwrap();
        assert_eq!(m.all(1).count(), 4 << 20);
        assert_eq!(m.subs(13).count(), 1 << 20);
        assert!(matches!(crate::decode(&f), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn malformed_files_never_panic() {
        let (f, _) = sample(8, 4);
        for n in 0..f.len() {
            let _ = crate::decode(&f[..n]);
            let _ = crate::probe_info(&f[..n]);
        }
        for i in 0..f.len() {
            for v in [0u8, 0x7f, 0x80, 0xff] {
                let mut g = f.clone();
                g[i] = v;
                let _ = crate::decode(&g);
                let _ = crate::probe_info(&g);
            }
        }
        // a block length of zero, and one far past the file
        let mut g = f.clone();
        g[4..12].copy_from_slice(&0u64.to_le_bytes());
        assert!(crate::decode(&g).is_err());
        g[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(crate::decode(&g).is_err());
    }
}
