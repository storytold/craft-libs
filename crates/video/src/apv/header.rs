//! RFC 9924 §5.3 and §8 syntax structures: Access Unit, PBU, Frame Header, Tile Info, and Metadata.

use craft_media::bitstream::{BitReader, BitWriter};

use crate::apv::tables::DEFAULT_Q_MATRIX_VAL;
use crate::apv::{ChromaFormat, ColorInfo, ContentLightLevel, Error, MasteringDisplay, Result};

/// Four-character signature `'aPv1'` (`0x61507631`) at the start of every APV Access Unit (RFC 9924 §5.3.1).
pub const AU_SIGNATURE: [u8; 4] = *b"aPv1";

/// Primitive Bitstream Unit (PBU) types (RFC 9924 §5.3.3 Table 3).
pub const PBU_PRIMARY_FRAME: u8 = 1;
pub const PBU_NON_PRIMARY_FRAME: u8 = 2;
pub const PBU_PREVIEW_FRAME: u8 = 25;
pub const PBU_DEPTH_FRAME: u8 = 26;
pub const PBU_ALPHA_FRAME: u8 = 27;
pub const PBU_AU_INFO: u8 = 65;
pub const PBU_METADATA: u8 = 66;
pub const PBU_FILLER: u8 = 67;

/// Maximum supported frame dimension in luma samples (guards against hostile allocations).
pub const MAX_DIMENSION: u32 = 16384;

/// Maximum tiles per axis (RFC 9924 §9.4.1 specifies 20; we allow up to 64 for robustness).
pub const MAX_TILE_DIM: u32 = 64;

/// Parsed `frame_header()` and derived tile geometry (RFC 9924 §5.3.5–5.3.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub band_idc: u8,
    pub width: u32,
    pub height: u32,
    pub chroma: ChromaFormat,
    pub bit_depth: u8,
    pub capture_time_distance: u8,
    pub color_description_present: bool,
    pub color: ColorInfo,
    pub use_q_matrix: bool,
    /// Per-component 8x8 quantization matrix in raster order (`y * 8 + x`).
    pub q_matrix: [[u8; 64]; 4],
    pub tile_width_in_mbs: u32,
    pub tile_height_in_mbs: u32,
    pub tile_cols: u32,
    pub tile_rows: u32,
    /// Tile column start positions in luma samples (`len == tile_cols + 1`).
    pub col_starts: Vec<u32>,
    /// Tile row start positions in luma samples (`len == tile_rows + 1`).
    pub row_starts: Vec<u32>,
    pub tile_sizes_in_fh: Option<Vec<u32>>,
    /// Byte offset within the PBU payload where `frame_header()` ends (start of the tile loop).
    pub header_bytes: usize,
}

impl FrameHeader {
    pub fn frame_width_in_mbs(&self) -> u32 {
        self.width.div_ceil(16)
    }

    pub fn frame_height_in_mbs(&self) -> u32 {
        self.height.div_ceil(16)
    }

    pub fn num_tiles(&self) -> usize {
        (self.tile_cols as usize) * (self.tile_rows as usize)
    }

    pub fn qp_bd_offset(&self) -> i32 {
        (self.bit_depth.saturating_sub(8) as i32) * 6
    }
}

/// A parsed PBU slice inside an Access Unit.
#[derive(Debug, Clone)]
pub struct PbuSlice<'a> {
    pub pbu_type: u8,
    pub group_id: u16,
    /// Payload after the 4-byte `pbu_header()`.
    pub payload: &'a [u8],
}

/// Unwrap an Access Unit byte slice: accepts either a direct `access_unit` starting with `'aPv1'`
/// or a `raw_bitstream_access_unit` prefixed with a 32-bit big-endian `au_size` (RFC 9924 Appendix A).
pub fn unwrap_au(data: &[u8]) -> Result<&[u8]> {
    if data.len() < 4 {
        return Err(Error::Truncated);
    }
    if data[..4] == AU_SIGNATURE {
        return Ok(data);
    }
    if data.len() >= 8 && data[4..8] == AU_SIGNATURE {
        let au_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if au_size < 4 || au_size == 0xFFFF_FFFF {
            return Err(Error::Invalid("invalid au_size in raw bitstream"));
        }
        let end = 4usize.checked_add(au_size).ok_or(Error::Truncated)?;
        return data.get(4..end).ok_or(Error::Truncated);
    }
    Err(Error::Invalid("missing aPv1 access unit signature"))
}

/// Split a raw `.apv` bitstream (`raw_bitstream_access_unit()` sequence per RFC 9924 Appendix A)
/// into individual Access Unit slices (each starting with `'aPv1'`).
pub fn split_raw_bitstream(mut data: &[u8]) -> Result<Vec<&[u8]>> {
    if data.is_empty() {
        return Err(Error::Truncated);
    }
    // If the buffer starts directly with 'aPv1', treat the entire buffer as a single AU.
    if data.len() >= 4 && data[..4] == AU_SIGNATURE {
        return Ok(vec![data]);
    }
    let mut aus = Vec::new();
    while !data.is_empty() {
        if data.len() < 8 {
            return Err(Error::Truncated);
        }
        let au_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if au_size < 4 || au_size == 0xFFFF_FFFF {
            return Err(Error::Invalid("invalid au_size in raw bitstream"));
        }
        let end = 4usize.checked_add(au_size).ok_or(Error::Truncated)?;
        let au = data.get(4..end).ok_or(Error::Truncated)?;
        if au[..4] != AU_SIGNATURE {
            return Err(Error::Invalid("missing aPv1 signature in raw bitstream AU"));
        }
        aus.push(au);
        data = &data[end..];
    }
    if aus.is_empty() {
        return Err(Error::Truncated);
    }
    Ok(aus)
}

/// Parse the PBU list of a single Access Unit (RFC 9924 §5.3.1–5.3.3).
/// PBUs with `reserved_zero_8bits > 0` are ignored as required by RFC 9924 §5.3.3.
pub fn parse_au_pbus(data: &[u8]) -> Result<Vec<PbuSlice<'_>>> {
    let au = unwrap_au(data)?;
    if au.len() < 8 {
        return Err(Error::Truncated);
    }
    let mut pos = 4usize;
    let mut pbus = Vec::new();
    while pos < au.len() {
        let sz_bytes = au.get(pos..pos + 4).ok_or(Error::Truncated)?;
        let pbu_size = u32::from_be_bytes([sz_bytes[0], sz_bytes[1], sz_bytes[2], sz_bytes[3]]) as usize;
        if pbu_size == 0 || pbu_size == 0xFFFF_FFFF {
            return Err(Error::Invalid("invalid pbu_size"));
        }
        if pbu_size < 4 {
            return Err(Error::Truncated);
        }
        pos += 4;
        let pbu_end = pos.checked_add(pbu_size).ok_or(Error::Truncated)?;
        let pbu = au.get(pos..pbu_end).ok_or(Error::Truncated)?;
        pos = pbu_end;

        let pbu_type = pbu[0];
        let group_id = u16::from_be_bytes([pbu[1], pbu[2]]);
        let reserved_zero_8bits = pbu[3];
        if reserved_zero_8bits > 0 {
            // RFC 9924 §5.3.3: Decoders MUST ignore PBU with reserved_zero_8bits > 0.
            continue;
        }
        if group_id == 0xFFFF || (group_id == 0 && pbu_type <= 64) {
            return Err(Error::Invalid("invalid PBU group_id"));
        }
        pbus.push(PbuSlice { pbu_type, group_id, payload: &pbu[4..] });
    }
    Ok(pbus)
}

/// Read `byte_alignment()` and verify all alignment bits are zero (RFC 9924 §5.3.17).
pub fn read_byte_alignment(r: &mut BitReader) -> Result<()> {
    while !r.is_byte_aligned() {
        let bit = r.read_bit().map_err(|_| Error::Truncated)?;
        if bit {
            return Err(Error::Invalid("non-zero alignment bit"));
        }
    }
    Ok(())
}

/// Parse `frame_header()` from the payload of a frame PBU (after `pbu_header()`), per RFC 9924 §5.3.5–5.3.8.
/// Returns `Ok(None)` if any `reserved_zero_*` field is non-zero (RFC 9924 §5.3.5 requires ignoring the PBU).
pub fn parse_frame_header(payload: &[u8]) -> Result<Option<FrameHeader>> {
    let mut r = BitReader::new(payload);

    // frame_info() (RFC 9924 §5.3.6)
    let profile_idc = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
    let level_idc = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
    let band_idc = r.read_bits(3).map_err(|_| Error::Truncated)? as u8;
    let reserved_zero_5bits = r.read_bits(5).map_err(|_| Error::Truncated)? as u8;
    let width = r.read_bits(24).map_err(|_| Error::Truncated)?;
    let height = r.read_bits(24).map_err(|_| Error::Truncated)?;
    let chroma_format_idc = r.read_bits(4).map_err(|_| Error::Truncated)? as u8;
    let bit_depth_minus8 = r.read_bits(4).map_err(|_| Error::Truncated)? as u8;
    let capture_time_distance = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
    let fi_reserved_8 = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;

    // frame_header() (RFC 9924 §5.3.5)
    let fh_reserved_8_a = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
    if reserved_zero_5bits > 0 || fi_reserved_8 > 0 || fh_reserved_8_a > 0 {
        return Ok(None);
    }

    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::Invalid("unsupported frame dimensions"));
    }
    let chroma = ChromaFormat::from_idc(chroma_format_idc).ok_or(Error::Invalid("invalid chroma_format_idc"))?;
    if chroma == ChromaFormat::Yuv422 && !width.is_multiple_of(2) {
        return Err(Error::Invalid("4:2:2 frame_width must be even"));
    }
    if !(2..=8).contains(&bit_depth_minus8) {
        return Err(Error::Invalid("bit_depth_minus8 out of range 2..=8"));
    }
    let bit_depth = bit_depth_minus8 + 8;

    let color_description_present = r.read_bit().map_err(|_| Error::Truncated)?;
    let color = if color_description_present {
        let primaries = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
        let transfer = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
        let matrix = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
        let full_range = r.read_bit().map_err(|_| Error::Truncated)?;
        ColorInfo { primaries, transfer, matrix, full_range }
    } else {
        ColorInfo::default()
    };

    let use_q_matrix = r.read_bit().map_err(|_| Error::Truncated)?;
    let mut q_matrix = [[DEFAULT_Q_MATRIX_VAL; 64]; 4];
    if use_q_matrix {
        for c in 0..chroma.num_comps() {
            for y in 0..8 {
                for x in 0..8 {
                    let q = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
                    if q == 0 {
                        return Err(Error::Invalid("q_matrix entry 0 is reserved"));
                    }
                    q_matrix[c][y * 8 + x] = q;
                }
            }
        }
    }

    // tile_info() (RFC 9924 §5.3.8)
    let tile_width_in_mbs = r.read_bits(20).map_err(|_| Error::Truncated)?;
    let tile_height_in_mbs = r.read_bits(20).map_err(|_| Error::Truncated)?;
    if tile_width_in_mbs == 0 || tile_height_in_mbs == 0 {
        return Err(Error::Invalid("tile dimensions in MBs must be non-zero"));
    }

    let w_mbs = width.div_ceil(16);
    let h_mbs = height.div_ceil(16);
    let tile_cols = w_mbs.div_ceil(tile_width_in_mbs);
    let tile_rows = h_mbs.div_ceil(tile_height_in_mbs);
    if tile_cols == 0 || tile_rows == 0 || tile_cols > MAX_TILE_DIM || tile_rows > MAX_TILE_DIM {
        return Err(Error::Invalid("too many tiles in frame"));
    }

    let mut col_starts = Vec::with_capacity(tile_cols as usize + 1);
    let mut start_mb = 0u32;
    while start_mb < w_mbs {
        col_starts.push(start_mb * 16);
        start_mb = start_mb.saturating_add(tile_width_in_mbs);
    }
    col_starts.push(w_mbs * 16);

    let mut row_starts = Vec::with_capacity(tile_rows as usize + 1);
    let mut start_mb = 0u32;
    while start_mb < h_mbs {
        row_starts.push(start_mb * 16);
        start_mb = start_mb.saturating_add(tile_height_in_mbs);
    }
    row_starts.push(h_mbs * 16);

    let num_tiles = (tile_cols as usize) * (tile_rows as usize);
    let tile_size_present_in_fh = r.read_bit().map_err(|_| Error::Truncated)?;
    let tile_sizes_in_fh = if tile_size_present_in_fh {
        let mut sizes = Vec::with_capacity(num_tiles);
        for _ in 0..num_tiles {
            let sz = r.read_bits(32).map_err(|_| Error::Truncated)?;
            if sz == 0 {
                return Err(Error::Invalid("tile_size_in_fh cannot be 0"));
            }
            sizes.push(sz);
        }
        Some(sizes)
    } else {
        None
    };

    let fh_reserved_8_b = r.read_bits(8).map_err(|_| Error::Truncated)? as u8;
    if fh_reserved_8_b > 0 {
        return Ok(None);
    }
    read_byte_alignment(&mut r)?;

    Ok(Some(FrameHeader {
        profile_idc,
        level_idc,
        band_idc,
        width,
        height,
        chroma,
        bit_depth,
        capture_time_distance,
        color_description_present,
        color,
        use_q_matrix,
        q_matrix,
        tile_width_in_mbs,
        tile_height_in_mbs,
        tile_cols,
        tile_rows,
        col_starts,
        row_starts,
        tile_sizes_in_fh,
        header_bytes: r.byte_pos(),
    }))
}

/// Write `frame_header()` to a byte vector (RFC 9924 §5.3.5–5.3.8).
pub fn write_frame_header(h: &FrameHeader) -> Vec<u8> {
    let mut w = BitWriter::new();
    // frame_info()
    w.write_bits(h.profile_idc as u32, 8);
    w.write_bits(h.level_idc as u32, 8);
    w.write_bits((h.band_idc & 7) as u32, 3);
    w.write_bits(0, 5); // reserved_zero_5bits
    w.write_bits(h.width, 24);
    w.write_bits(h.height, 24);
    w.write_bits(h.chroma.idc() as u32, 4);
    w.write_bits(h.bit_depth.saturating_sub(8) as u32, 4);
    w.write_bits(h.capture_time_distance as u32, 8);
    w.write_bits(0, 8); // reserved_zero_8bits

    // frame_header()
    w.write_bits(0, 8); // reserved_zero_8bits
    w.write_bit(h.color_description_present);
    if h.color_description_present {
        w.write_bits(h.color.primaries as u32, 8);
        w.write_bits(h.color.transfer as u32, 8);
        w.write_bits(h.color.matrix as u32, 8);
        w.write_bit(h.color.full_range);
    }
    w.write_bit(h.use_q_matrix);
    if h.use_q_matrix {
        for c in 0..h.chroma.num_comps() {
            for y in 0..8 {
                for x in 0..8 {
                    w.write_bits(h.q_matrix[c][y * 8 + x].max(1) as u32, 8);
                }
            }
        }
    }
    // tile_info()
    w.write_bits(h.tile_width_in_mbs, 20);
    w.write_bits(h.tile_height_in_mbs, 20);
    if let Some(sizes) = &h.tile_sizes_in_fh {
        w.write_bit(true);
        for &sz in sizes {
            w.write_bits(sz, 32);
        }
    } else {
        w.write_bit(false);
    }
    w.write_bits(0, 8); // reserved_zero_8bits
    w.finish()
}

/// Parse a `metadata()` PBU payload (after `pbu_header()`), extracting MDCV (`payloadType == 5`)
/// and CLL (`payloadType == 6`) if present (RFC 9924 §5.3.10 & §8).
pub fn parse_metadata_pbu(payload: &[u8], mdcv: &mut Option<MasteringDisplay>, cll: &mut Option<ContentLightLevel>) -> Result<()> {
    if payload.len() < 4 {
        return Err(Error::Truncated);
    }
    let metadata_size = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let body = payload.get(4..4usize.checked_add(metadata_size).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
    let mut pos = 0usize;
    while pos < body.len() {
        let mut payload_type = 0u32;
        while *body.get(pos).ok_or(Error::Truncated)? == 0xFF {
            payload_type = payload_type.checked_add(255).ok_or(Error::Invalid("metadata payloadType overflow"))?;
            pos += 1;
        }
        payload_type =
            payload_type.checked_add(*body.get(pos).ok_or(Error::Truncated)? as u32).ok_or(Error::Invalid("metadata payloadType overflow"))?;
        pos += 1;

        let mut payload_size = 0usize;
        while *body.get(pos).ok_or(Error::Truncated)? == 0xFF {
            payload_size = payload_size.checked_add(255).ok_or(Error::Invalid("metadata payloadSize overflow"))?;
            pos += 1;
        }
        payload_size =
            payload_size.checked_add(*body.get(pos).ok_or(Error::Truncated)? as usize).ok_or(Error::Invalid("metadata payloadSize overflow"))?;
        pos += 1;

        let end = pos.checked_add(payload_size).ok_or(Error::Truncated)?;
        let item = body.get(pos..end).ok_or(Error::Truncated)?;
        pos = end;

        match payload_type {
            5 if item.len() >= 24 => {
                let u16_at = |i: usize| u16::from_be_bytes([item[i], item[i + 1]]);
                let u32_at = |i: usize| u32::from_be_bytes([item[i], item[i + 1], item[i + 2], item[i + 3]]);
                *mdcv = Some(MasteringDisplay {
                    primaries: [(u16_at(0), u16_at(2)), (u16_at(4), u16_at(6)), (u16_at(8), u16_at(10))],
                    white_point: (u16_at(12), u16_at(14)),
                    max_luminance: u32_at(16),
                    min_luminance: u32_at(20),
                });
            }
            6 if item.len() >= 4 => {
                *cll = Some(ContentLightLevel { max_cll: u16::from_be_bytes([item[0], item[1]]), max_fall: u16::from_be_bytes([item[2], item[3]]) });
            }
            // Filler metadata (§8.2.1): must be 0xFF bytes
            10 if item.iter().any(|&b| b != 0xFF) => {
                return Err(Error::Invalid("non-0xFF byte in filler metadata"));
            }
            _ => {
                // RFC 9924 §10: A decoder MUST NOT try to process metadata whose type is not recognized.
            }
        }
    }
    Ok(())
}

/// Build a `metadata()` PBU (including its 4-byte `pbu_header`) carrying MDCV and/or CLL metadata.
pub fn write_metadata_pbu(group_id: u16, mdcv: Option<&MasteringDisplay>, cll: Option<&ContentLightLevel>) -> Option<Vec<u8>> {
    if mdcv.is_none() && cll.is_none() {
        return None;
    }
    let mut body = Vec::new();
    if let Some(m) = mdcv {
        body.push(5); // payloadType = 5 (MDCV)
        body.push(24); // payloadSize = 24
        for &(x, y) in &m.primaries {
            body.extend_from_slice(&x.to_be_bytes());
            body.extend_from_slice(&y.to_be_bytes());
        }
        body.extend_from_slice(&m.white_point.0.to_be_bytes());
        body.extend_from_slice(&m.white_point.1.to_be_bytes());
        body.extend_from_slice(&m.max_luminance.to_be_bytes());
        body.extend_from_slice(&m.min_luminance.to_be_bytes());
    }
    if let Some(c) = cll {
        body.push(6); // payloadType = 6 (CLL)
        body.push(4); // payloadSize = 4
        body.extend_from_slice(&c.max_cll.to_be_bytes());
        body.extend_from_slice(&c.max_fall.to_be_bytes());
    }
    let mut pbu = Vec::with_capacity(8 + body.len());
    pbu.push(PBU_METADATA);
    pbu.extend_from_slice(&group_id.to_be_bytes());
    pbu.push(0); // reserved_zero_8bits
    pbu.extend_from_slice(&(body.len() as u32).to_be_bytes());
    pbu.extend_from_slice(&body);
    Some(pbu)
}
