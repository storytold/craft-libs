//! Patterns: the global `Patt` / `Pat2` / `Pat3` tagged blocks (8 / 16 / 32-bit documents) and
//! standalone `.pat` files.
//!
//! Layout per the public Photoshop File Formats Specification ("Patterns" and "Virtual Memory
//! Array List"):
//!
//! ```text
//! pattern  := version u32 (1), image mode u32, height i16, width i16,
//!             name (Unicode string), unique id (Pascal string, unpadded),
//!             [indexed: 256 × RGB palette], virtual memory array list
//! VMA list := version u32 (3), length u32, rect (top, left, bottom, right i32),
//!             channel count u32, then count + 2 arrays (user mask, sheet mask)
//! array    := written u32 (0 = absent), length u32 (0 = absent), depth u32, rect,
//!             depth u16, compression u8 (0 raw, 1 PackBits RLE), data
//! ```
//!
//! In the tagged blocks each pattern is prefixed by its u32 length and padded to 4 bytes; a
//! `.pat` file is `8BPT`, version u16 (1), count u32, then the patterns without length prefixes.
//! Photoshop writes the channel count as 24 and only marks the real channels as written; like
//! ag-psd (MIT) we take the first written arrays as the colour channels and the next one as
//! transparency. Photoshop stores a pattern's transparency in the last (sheet mask) array and
//! leaves the user mask unwritten (psd-tools multichannel-pattern-fill.psd, Photoshop CC 2019); a
//! file with transparency in the user mask array is refused by Photoshop (photocraft#8054).

use crate::compression::{Compression, MAX_DECODED_BYTES, PlaneLayout, decode_planes, encode_planes};
use crate::error::{PsdError, Result};
use crate::header::Version;
use crate::io::WriteExt;
use std::borrow::Cow;

/// Largest pattern edge accepted (Photoshop's own limit is far lower).
const MAX_EDGE: u32 = 30_000;
/// Budget for zero-padded channel planes summed over every pattern of one parse (a block, a `.pat`
/// file, an `.abr`). Photoshop writes every channel at the pattern rectangle, so a smaller channel
/// rectangle is only padding; without this cap a few hundred bytes (1x1 channels, a huge pattern
/// rectangle) would allocate up to `MAX_DECODED_BYTES` of zeros.
const MAX_PADDED_PATTERN_BYTES: u64 = 512 << 20;
// A PackBits run packet emits at most 128 bytes from a 2-byte packet.
const MAX_RLE_EXPANSION: u64 = 64;

/// Decoded bytes left for the patterns of one parse. Shared by every pattern read in one call, so
/// many patterns can't each claim the whole per-pattern allowance.
#[derive(Debug, Clone)]
pub struct PatternBudget {
    decoded_left: u64,
    padded_left: u64,
}

impl Default for PatternBudget {
    fn default() -> Self {
        PatternBudget { decoded_left: MAX_DECODED_BYTES, padded_left: MAX_PADDED_PATTERN_BYTES }
    }
}

impl PatternBudget {
    /// A budget of at most `decoded` decoded bytes (zero padding included), and never more than
    /// the default limits.
    pub fn with_decoded_limit(decoded: u64) -> Self {
        let d = Self::default();
        PatternBudget { decoded_left: d.decoded_left.min(decoded), padded_left: d.padded_left.min(decoded) }
    }

    /// Cap both decoded and padded bytes to the amount still available to a containing file.
    pub fn cap_decoded_to(&mut self, decoded: u64) {
        self.decoded_left = self.decoded_left.min(decoded);
        self.padded_left = self.padded_left.min(decoded);
    }

    /// Decoded bytes still available.
    pub fn decoded_left(&self) -> u64 {
        self.decoded_left
    }
}

/// One pattern tile with planar, big-endian samples.
#[derive(Debug, Clone, PartialEq)]
pub struct PsdPattern {
    /// PSD colour mode number (1 gray, 2 indexed, 3 RGB, 4 CMYK, 7 multichannel, 8 duotone, 9 Lab).
    pub mode: u32,
    /// Tile width in pixels.
    pub width: u32,
    /// Tile height in pixels.
    pub height: u32,
    /// Name without the trailing NUL (built-ins look like `$$$/Patterns/…=Water`).
    pub name: String,
    /// Unique id (usually a UUID).
    pub id: String,
    /// Indexed-colour palette (768 bytes, RGB triplets).
    pub palette: Option<Vec<u8>>,
    /// Bits per sample: 1, 8, 16 or 32.
    pub depth: u16,
    /// Decoded colour planes (`width × height` samples each).
    pub channels: Vec<Vec<u8>>,
    /// Decoded transparency plane, if any.
    pub alpha: Option<Vec<u8>>,
}

/// Colour channels implied by a PSD colour mode.
pub fn mode_channels(mode: u32) -> usize {
    match mode {
        3 | 9 => 3,
        4 => 4,
        _ => 1,
    }
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

struct PatternChannel<'a> {
    data: &'a [u8],
    compression: Compression,
    source: PlaneLayout,
    destination: PlaneLayout,
    x: usize,
    y: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or(PsdError::LimitExceeded("pattern length overflow"))?;
        if end > self.b.len() {
            return Err(PsdError::UnexpectedEof { offset: self.p, needed: end - self.b.len() });
        }
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn rect(&mut self) -> Result<[i32; 4]> {
        Ok([self.i32()?, self.i32()?, self.i32()?, self.i32()?])
    }
}

/// A pattern's mode, name, id and palette, up to its virtual memory array list.
struct PatternHeader {
    mode: u32,
    name: String,
    id: String,
    palette: Option<Vec<u8>>,
}

fn read_header(r: &mut Rd) -> Result<PatternHeader> {
    let version = r.u32()?;
    if version != 1 {
        return Err(PsdError::invalid(format!("pattern version {version}")));
    }
    let mode = r.u32()?;
    let _h = r.u16()?;
    let _w = r.u16()?;
    let n = r.u32()? as usize;
    if n > 65_536 {
        return Err(PsdError::LimitExceeded("pattern name length"));
    }
    let units: Vec<u16> = r.take(n * 2)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    let name = String::from_utf16_lossy(&units).trim_end_matches('\0').to_string();
    let idl = r.u8()? as usize;
    let id = String::from_utf8_lossy(r.take(idl)?).to_string();
    let palette = if mode == 2 {
        let p = r.take(768)?.to_vec();
        // Some writers follow the palette with 4 extra bytes (ag-psd skips them).
        if r.b.get(r.p..r.p + 4).is_some_and(|v| v != [0, 0, 0, 3]) {
            r.take(4)?;
        }
        Some(p)
    } else {
        None
    };
    Ok(PatternHeader { mode, name, id, palette })
}

/// The body of a pattern's virtual memory array list (after its version and length).
fn read_vma_body<'a>(r: &mut Rd<'a>) -> Result<&'a [u8]> {
    let vver = r.u32()?;
    if vver != 3 {
        return Err(PsdError::invalid(format!("virtual memory array list version {vver}")));
    }
    let len = r.u32()? as usize;
    r.take(len)
}

/// Pixel storage needed by a pattern, inspected without decoding channel data.
#[derive(Debug, Clone)]
pub struct PatternMemory {
    /// Pattern width in pixels.
    pub width: u32,
    /// Pattern height in pixels.
    pub height: u32,
    /// Bits per sample, including packed bitmap samples.
    pub depth: u16,
    /// PSD colour mode number.
    pub mode: u32,
    /// All written planes, including any ignored extra arrays.
    pub planes: usize,
    /// Sum of full padded channel plane sizes.
    pub decoded_bytes: u64,
}

fn read_pattern(r: &mut Rd, budget: &mut PatternBudget) -> Result<PsdPattern> {
    read_pattern_with(r, budget, true, &mut |_| {})
}

fn read_pattern_with(r: &mut Rd, budget: &mut PatternBudget, decode: bool, visit: &mut dyn FnMut(PatternMemory)) -> Result<PsdPattern> {
    let PatternHeader { mode, name, id, palette } = read_header(r)?;
    let body = read_vma_body(r)?;
    let mut v = Rd { b: body, p: 0 };
    let [top, left, bottom, right] = v.rect()?;
    let w = right.checked_sub(left).ok_or(PsdError::LimitExceeded("pattern width overflow"))?;
    let h = bottom.checked_sub(top).ok_or(PsdError::LimitExceeded("pattern height overflow"))?;
    if w <= 0 || h <= 0 || w as u32 > MAX_EDGE || h as u32 > MAX_EDGE {
        return Err(PsdError::LimitExceeded("pattern size"));
    }
    let (w, h) = (w as u32, h as u32);
    let count = v.u32()?;
    if count > 64 {
        return Err(PsdError::LimitExceeded("pattern channel count"));
    }
    // Validate every channel and the total padded size before decoding any plane.
    // Otherwise a later channel can exceed the existing pattern budget only after
    // earlier channels have already allocated several gigabytes of zero padding.
    // `budget` is shared by every pattern of the parse, so many patterns can't each pad up to the
    // per-parse cap.
    let mut channels = Vec::new();
    let mut first_depth: Option<u16> = None;
    for _ in 0..count + 2 {
        if v.p >= body.len() {
            break;
        }
        if v.u32()? == 0 {
            continue;
        }
        let alen = v.u32()? as usize;
        if alen == 0 {
            continue;
        }
        if alen < 23 {
            return Err(PsdError::invalid("short virtual memory array"));
        }
        let d32 = v.u32()?;
        let [ct, cl, cb, cr] = v.rect()?;
        let d16 = v.u16()?;
        let comp = v.u8()?;
        let data = v.take(alen - 23)?;
        let d = if matches!(d16, 1 | 8 | 16 | 32) { d16 } else { d32 as u16 };
        if !matches!(d, 1 | 8 | 16 | 32) {
            return Err(PsdError::invalid(format!("pattern depth {d}")));
        }
        // Every plane is sized by one depth; a channel at another depth can't be written back.
        if *first_depth.get_or_insert(d) != d {
            return Err(PsdError::invalid("pattern channels have different depths"));
        }
        if ct < top || cl < left || cb > bottom || cr > right || cb < ct || cr < cl {
            return Err(PsdError::invalid("pattern channel rectangle falls outside pattern rectangle"));
        }
        let cw = usize::try_from(cr.checked_sub(cl).ok_or(PsdError::LimitExceeded("pattern channel width overflow"))?)
            .map_err(|_| PsdError::invalid("negative pattern channel width"))?;
        let ch = usize::try_from(cb.checked_sub(ct).ok_or(PsdError::LimitExceeded("pattern channel height overflow"))?)
            .map_err(|_| PsdError::invalid("negative pattern channel height"))?;
        let channel_layout = PlaneLayout { planes: 1, width: cw, height: ch, depth: d, version: Version::Psd };
        let channel_bytes = channel_layout.total_bytes()?;
        let layout = PlaneLayout { planes: 1, width: w as usize, height: h as usize, depth: d, version: Version::Psd };
        let plane_bytes = layout.total_bytes()?;
        budget.decoded_left =
            budget.decoded_left.checked_sub(plane_bytes).ok_or(PsdError::LimitExceeded("pattern data exceeds MAX_DECODED_BYTES"))?;
        if cw != w as usize || ch != h as usize {
            budget.padded_left = budget
                .padded_left
                .checked_sub(plane_bytes)
                .ok_or(PsdError::LimitExceeded("padded pattern channels exceed MAX_PADDED_PATTERN_BYTES"))?;
        }
        let _ = layout.decoded_len()?;
        let compression = if comp == 1 {
            Compression::Rle
        } else if comp == 0 {
            Compression::Raw
        } else {
            Compression::Unknown(u16::from(comp))
        };
        let max_expansion = match compression {
            Compression::Raw => Some(1),
            Compression::Rle => Some(MAX_RLE_EXPANSION),
            Compression::Zip | Compression::ZipPrediction | Compression::Unknown(_) => None,
        };
        if let Some(max_expansion) = max_expansion {
            let expansion_limit = u64::try_from(data.len())
                .map_err(|_| PsdError::LimitExceeded("pattern channel input exceeds address space"))?
                .checked_mul(max_expansion)
                .ok_or(PsdError::LimitExceeded("pattern channel input size overflow"))?;
            if channel_bytes > expansion_limit {
                return Err(PsdError::LimitExceeded("pattern channel exceeds its input-derived size limit"));
            }
        }
        let x = usize::try_from(cl.checked_sub(left).ok_or(PsdError::LimitExceeded("pattern channel x offset overflow"))?)
            .map_err(|_| PsdError::invalid("negative pattern channel x offset"))?;
        let y = usize::try_from(ct.checked_sub(top).ok_or(PsdError::LimitExceeded("pattern channel y offset overflow"))?)
            .map_err(|_| PsdError::invalid("negative pattern channel y offset"))?;
        channels.push(PatternChannel { data, compression, source: channel_layout, destination: layout, x, y });
    }
    let nc = mode_channels(mode);
    if channels.len() < nc {
        return Err(PsdError::invalid("pattern has fewer channels than its colour mode"));
    }
    visit(PatternMemory {
        width: w,
        height: h,
        depth: first_depth.unwrap_or(8),
        mode,
        planes: channels.len(),
        decoded_bytes: channels
            .iter()
            .try_fold(0u64, |sum, c| sum.checked_add(c.destination.total_bytes()?).ok_or(PsdError::LimitExceeded("pattern memory size overflow")))?,
    });
    let mut planes = Vec::new();
    for channel in channels.into_iter().filter(|_| decode) {
        let plane = decode_planes(channel.compression, channel.data, &channel.source)?;
        planes.push(place_channel_plane(plane, &channel.source, channel.x, channel.y, &channel.destination)?);
    }
    let alpha = (planes.len() > nc).then(|| planes.remove(nc));
    planes.truncate(nc);
    let depth = first_depth.unwrap_or(8);
    Ok(PsdPattern { mode, width: w, height: h, name, id, palette, depth, channels: planes, alpha })
}

fn place_channel_plane(mut plane: Vec<u8>, source: &PlaneLayout, x: usize, y: usize, destination: &PlaneLayout) -> Result<Vec<u8>> {
    if source.planes != 1 || destination.planes != 1 || source.depth != destination.depth {
        return Err(PsdError::invalid("incompatible pattern channel layouts"));
    }
    let x_end = x.checked_add(source.width).ok_or(PsdError::LimitExceeded("pattern channel x offset overflow"))?;
    let y_end = y.checked_add(source.height).ok_or(PsdError::LimitExceeded("pattern channel y offset overflow"))?;
    if x_end > destination.width || y_end > destination.height {
        return Err(PsdError::invalid("pattern channel rectangle falls outside pattern rectangle"));
    }
    let source_len = source.decoded_len()?;
    if plane.len() != source_len {
        return Err(PsdError::invalid("pattern channel payload does not match its rectangle"));
    }
    let destination_len = destination.decoded_len()?;
    if x == 0 && y == 0 && source.width == destination.width && source.height == destination.height {
        // Preserve the old placement's zeroed unused bits without copying a full
        // bitmap plane. Only the last byte of each non-byte-aligned row changes.
        if source.depth == 1 && !source.width.is_multiple_of(8) {
            let mask = u8::MAX << (8 - source.width % 8);
            for row in plane.chunks_exact_mut(source.row_bytes()) {
                if let Some(last) = row.last_mut() {
                    *last &= mask;
                }
            }
        }
        return Ok(plane);
    }
    let mut out = zeroed_pattern_plane(destination_len)?;
    if source.depth == 1 {
        let source_row_bytes = source.row_bytes();
        let destination_row_bytes = destination.row_bytes();
        for row in 0..source.height {
            for col in 0..source.width {
                let source_pos = row
                    .checked_mul(source_row_bytes)
                    .and_then(|v| v.checked_add(col / 8))
                    .ok_or(PsdError::LimitExceeded("pattern channel source offset overflow"))?;
                let destination_col = x.checked_add(col).ok_or(PsdError::LimitExceeded("pattern channel destination offset overflow"))?;
                let destination_pos = y
                    .checked_add(row)
                    .and_then(|v| v.checked_mul(destination_row_bytes))
                    .and_then(|v| v.checked_add(destination_col / 8))
                    .ok_or(PsdError::LimitExceeded("pattern channel destination offset overflow"))?;
                let source_bit = 7 - col % 8;
                let destination_bit = 7 - destination_col % 8;
                let source_byte = plane.get(source_pos).ok_or(PsdError::invalid("pattern channel payload does not match its rectangle"))?;
                let value = (*source_byte >> source_bit) & 1;
                let dst = out.get_mut(destination_pos).ok_or(PsdError::invalid("pattern channel placement exceeds its output plane"))?;
                *dst |= value << destination_bit;
            }
        }
    } else {
        let bytes_per_sample = usize::from(source.depth / 8);
        let source_row_bytes = source.row_bytes();
        let destination_row_bytes = destination.row_bytes();
        let row_bytes = source.width.checked_mul(bytes_per_sample).ok_or(PsdError::LimitExceeded("pattern channel row size overflow"))?;
        let x_bytes = x.checked_mul(bytes_per_sample).ok_or(PsdError::LimitExceeded("pattern channel x offset overflow"))?;
        for row in 0..source.height {
            let source_start = row.checked_mul(source_row_bytes).ok_or(PsdError::LimitExceeded("pattern channel source offset overflow"))?;
            let destination_start = y
                .checked_add(row)
                .and_then(|v| v.checked_mul(destination_row_bytes))
                .and_then(|v| v.checked_add(x_bytes))
                .ok_or(PsdError::LimitExceeded("pattern channel destination offset overflow"))?;
            let source_end = source_start.checked_add(row_bytes).ok_or(PsdError::LimitExceeded("pattern channel source offset overflow"))?;
            let destination_end =
                destination_start.checked_add(row_bytes).ok_or(PsdError::LimitExceeded("pattern channel destination offset overflow"))?;
            let source_row = plane.get(source_start..source_end).ok_or(PsdError::invalid("pattern channel payload does not match its rectangle"))?;
            let destination_row =
                out.get_mut(destination_start..destination_end).ok_or(PsdError::invalid("pattern channel placement exceeds its output plane"))?;
            destination_row.copy_from_slice(source_row);
        }
    }
    Ok(out)
}

fn zeroed_pattern_plane(len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| PsdError::OutOfMemory("not enough memory for the padded pattern channel"))?;
    out.resize(len, 0);
    Ok(out)
}

/// Append one validated pattern, backpatching lengths in its final output buffer.
fn write_pattern(p: &PsdPattern, out: &mut Vec<u8>) -> Result<()> {
    if !matches!(p.depth, 1 | 8 | 16 | 32) {
        return Err(PsdError::Unsupported(format!("pattern depth {}", p.depth)));
    }
    if p.width > MAX_EDGE || p.height > MAX_EDGE || p.width > i16::MAX as u32 || p.height > i16::MAX as u32 {
        return Err(PsdError::LimitExceeded("pattern size"));
    }
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&p.mode.to_be_bytes());
    out.extend_from_slice(&(p.height as u16).to_be_bytes());
    out.extend_from_slice(&(p.width as u16).to_be_bytes());
    let units: Vec<u16> = p.name.encode_utf16().chain(std::iter::once(0)).collect();
    // The reader refuses longer names (see `read_pattern`).
    if units.len() > 65_536 {
        return Err(PsdError::LimitExceeded("pattern name length"));
    }
    out.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    let id = p.id.as_bytes();
    let idl = id.len().min(255);
    out.push(idl as u8);
    out.extend_from_slice(&id[..idl]);
    if p.mode == 2 {
        let mut pal = p.palette.clone().unwrap_or_default();
        pal.resize(768, 0);
        out.extend_from_slice(&pal);
    }
    let rect = [0i32, 0, p.height as i32, p.width as i32];
    out.extend_from_slice(&3u32.to_be_bytes());
    let at = out.begin_len(false);
    for v in rect {
        out.extend_from_slice(&v.to_be_bytes());
    }
    const SLOTS: u32 = 24;
    out.extend_from_slice(&SLOTS.to_be_bytes());
    let layout = PlaneLayout { planes: 1, width: p.width as usize, height: p.height as usize, depth: p.depth, version: Version::Psd };
    let array = |plane: &[u8], out: &mut Vec<u8>| -> Result<()> {
        let (comp, data) = if p.depth == 8 {
            (1u8, Cow::Owned(encode_planes(Compression::Rle, plane, &layout)?))
        } else {
            let total = layout.decoded_len()?;
            if plane.len() != total {
                return Err(PsdError::invalid(format!("decoded buffer has {} bytes, layout requires {}", plane.len(), total)));
            }
            (0u8, Cow::Borrowed(plane))
        };
        out.extend_from_slice(&1u32.to_be_bytes());
        let len = u32::try_from(data.len()).ok().and_then(|n| n.checked_add(23)).ok_or(PsdError::LimitExceeded("pattern array length"))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&u32::from(p.depth).to_be_bytes());
        for v in rect {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&p.depth.to_be_bytes());
        out.push(comp);
        out.extend_from_slice(&data);
        Ok(())
    };
    let nc = p.channels.len().min(SLOTS as usize);
    for c in &p.channels[..nc] {
        array(c, out)?;
    }
    for _ in nc..SLOTS as usize {
        out.extend_from_slice(&0u32.to_be_bytes());
    }
    // User mask slot: unused; sheet mask slot: transparency, where Photoshop keeps it (photocraft#8054).
    out.extend_from_slice(&0u32.to_be_bytes());
    match &p.alpha {
        Some(a) => array(a, out)?,
        None => out.extend_from_slice(&0u32.to_be_bytes()),
    }
    out.end_len(at, false)?;
    Ok(())
}

/// Parses the data of a `Patt` / `Pat2` / `Pat3` global block.
pub fn parse_pattern_block(data: &[u8]) -> Result<Vec<PsdPattern>> {
    parse_pattern_block_with(data, &mut PatternBudget::default())
}

/// [`parse_pattern_block`] charging `budget`, so several blocks of one document share one limit.
/// Charges are monotonic even when a later record in the block fails.
pub fn parse_pattern_block_with(data: &[u8], budget: &mut PatternBudget) -> Result<Vec<PsdPattern>> {
    let mut r = Rd { b: data, p: 0 };
    let mut out = Vec::new();
    while r.p + 4 <= data.len() {
        let len = r.u32()? as usize;
        if len == 0 {
            break;
        }
        let body = r.take(len)?;
        out.push(read_pattern(&mut Rd { b: body, p: 0 }, budget)?);
        let pad = (4 - len % 4) % 4;
        r.p = (r.p + pad).min(data.len());
    }
    Ok(out)
}

/// Inspect a pattern block's dimensions and decoded sizes without allocating pixel planes.
/// Uses the pixel parser’s structural validation and shared decode limits. Returns
/// earlier records even if a later record is malformed, since those can decode first.
pub fn pattern_block_memory(data: &[u8], budget: &mut PatternBudget) -> Vec<PatternMemory> {
    let mut r = Rd { b: data, p: 0 };
    let mut out = Vec::new();
    while r.p + 4 <= data.len() {
        let Ok(len) = r.u32() else { break };
        let len = len as usize;
        if len == 0 {
            break;
        }
        let Ok(body) = r.take(len) else { break };
        // Earlier patterns may already have decoded when a later record is malformed.
        if read_pattern_with(&mut Rd { b: body, p: 0 }, budget, false, &mut |m| out.push(m)).is_err() {
            break;
        }
        r.p = (r.p + (4 - len % 4) % 4).min(data.len());
    }
    out
}

/// Whether `Patt` / `Pat2` / `Pat3` block data keeps a pattern's transparency in the user mask
/// array, as PhotoCraft wrote it before photocraft#8054. Photoshop refuses such a file, so a save must
/// rewrite the block rather than keep it. Unreadable data counts as `false`.
pub fn block_has_user_mask(data: &[u8]) -> bool {
    let mut r = Rd { b: data, p: 0 };
    while r.p + 4 <= data.len() {
        let Ok(len) = r.u32() else { return false };
        if len == 0 {
            break;
        }
        let Ok(body) = r.take(len as usize) else { return false };
        if pattern_has_user_mask(&mut Rd { b: body, p: 0 }).unwrap_or(false) {
            return true;
        }
        let pad = (4 - len as usize % 4) % 4;
        r.p = (r.p + pad).min(data.len());
    }
    false
}

fn pattern_has_user_mask(r: &mut Rd) -> Result<bool> {
    read_header(r)?;
    let mut v = Rd { b: read_vma_body(r)?, p: 0 };
    v.rect()?;
    let count = v.u32()?;
    // Every array takes at least 4 bytes, so the body bounds this loop whatever `count` says.
    for i in 0..count.saturating_add(2) {
        if v.p >= v.b.len() {
            break;
        }
        if v.u32()? == 0 {
            continue;
        }
        let alen = v.u32()? as usize;
        if i == count && alen != 0 {
            return Ok(true);
        }
        v.take(alen)?;
    }
    Ok(false)
}

/// Serializes patterns as `Patt` / `Pat2` / `Pat3` block data.
pub fn write_pattern_block(patterns: &[PsdPattern]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for p in patterns {
        let at = out.begin_len(false);
        write_pattern(p, &mut out)?;
        out.end_len(at, false)?;
        let pad = (4 - (out.len() - at - 4) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    Ok(out)
}

/// Tagged-block key for patterns of a document at `depth` bits.
pub fn block_key(depth: u16) -> [u8; 4] {
    match depth {
        16 => *b"Pat2",
        32 => *b"Pat3",
        _ => *b"Patt",
    }
}

/// The most patterns a `.pat` file may hold: more are refused on read and on write.
pub const MAX_PAT_PATTERNS: usize = 100_000;

/// Parses a `.pat` pattern file.
pub fn parse_pat_file(data: &[u8]) -> Result<Vec<PsdPattern>> {
    let mut r = Rd { b: data, p: 0 };
    let sig = r.take(4)?;
    if sig != b"8BPT" {
        return Err(PsdError::InvalidSignature { expected: "8BPT", found: [sig[0], sig[1], sig[2], sig[3]] });
    }
    let _version = r.u16()?;
    let count = r.u32()? as usize;
    if count > MAX_PAT_PATTERNS {
        return Err(PsdError::LimitExceeded("pattern count"));
    }
    let mut out = Vec::new();
    let mut budget = PatternBudget::default();
    for _ in 0..count {
        out.push(read_pattern(&mut r, &mut budget)?);
    }
    Ok(out)
}

/// Writes a `.pat` pattern file.
pub fn write_pat_file(patterns: &[PsdPattern]) -> Result<Vec<u8>> {
    if patterns.len() > MAX_PAT_PATTERNS {
        return Err(PsdError::LimitExceeded("pattern count"));
    }
    let mut out = b"8BPT".to_vec();
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(patterns.len() as u32).to_be_bytes());
    for p in patterns {
        write_pattern(p, &mut out)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
