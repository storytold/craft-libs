//! Configuration and VOP headers (ISO/IEC 14496-2 §6.2.2 to §6.2.5).

use craft_media::bitstream::BitReader;

use super::super::tables::ZIGZAG;
use super::vlc::{self, eof, marker};
use super::{DecodeError, Result};

/// The default intra quantiser matrix of the MPEG quantisation method (§6.3.3), raster order.
pub(super) const DEFAULT_INTRA_MATRIX: [u8; 64] = [
    8, 17, 18, 19, 21, 23, 25, 27, 17, 18, 19, 21, 23, 25, 27, 28, 20, 21, 22, 23, 24, 26, 28, 30, 21, 22, 23, 24, 26, 28, 30, 32, 22, 23, 24, 26,
    28, 30, 32, 35, 23, 24, 26, 28, 30, 32, 35, 38, 25, 26, 28, 30, 32, 35, 38, 41, 27, 28, 30, 32, 35, 38, 41, 45,
];

/// The default non-intra quantiser matrix, raster order.
pub(super) const DEFAULT_INTER_MATRIX: [u8; 64] = [
    16, 17, 18, 19, 20, 21, 22, 23, 17, 18, 19, 20, 21, 22, 23, 24, 18, 19, 20, 21, 22, 23, 24, 25, 19, 20, 21, 22, 23, 24, 26, 27, 20, 21, 22, 23,
    25, 26, 27, 28, 21, 22, 23, 24, 26, 27, 28, 30, 22, 23, 24, 26, 27, 28, 30, 31, 23, 24, 25, 27, 28, 30, 31, 33,
];

/// `video_signal_type` of the visual object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VideoSignal {
    pub video_format: Option<u8>,
    pub full_range: bool,
    /// `(colour_primaries, transfer_characteristics, matrix_coefficients)`.
    pub colour: Option<(u8, u8, u8)>,
}

/// Visual object header (`00 00 01 B5`).
pub(super) fn parse_visual_object(data: &[u8]) -> Result<VideoSignal> {
    let mut r = BitReader::new(data);
    let rd = |r: &mut BitReader, n| r.read_bits(n).map_err(eof);
    if rd(&mut r, 1)? == 1 {
        rd(&mut r, 7)?; // visual_object_verid, visual_object_priority
    }
    let kind = rd(&mut r, 4)?;
    let mut s = VideoSignal::default();
    if (kind == 1 || kind == 2) && rd(&mut r, 1)? == 1 {
        s.video_format = Some(rd(&mut r, 3)? as u8);
        s.full_range = rd(&mut r, 1)? == 1;
        if rd(&mut r, 1)? == 1 {
            s.colour = Some((rd(&mut r, 8)? as u8, rd(&mut r, 8)? as u8, rd(&mut r, 8)? as u8));
        }
    }
    Ok(s)
}

/// `sprite_enable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sprite {
    None,
    Static,
    Gmc,
}

/// The fields of a video object layer header (`00 00 01 2x`) the decoder uses.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Vol {
    pub verid: u8,
    pub object_type: u8,
    pub sar: (u16, u16),
    pub low_delay: bool,
    pub time_resolution: u32,
    /// Bits of `vop_time_increment`.
    pub time_bits: u32,
    pub fixed_increment: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub interlaced: bool,
    pub sprite: Sprite,
    pub warp_points: u8,
    pub warp_accuracy: u8,
    pub mpeg_quant: bool,
    /// Raster order.
    pub intra_matrix: [u8; 64],
    pub inter_matrix: [u8; 64],
    pub quarter_sample: bool,
    pub resync_markers: bool,
    pub data_partitioned: bool,
    pub reversible_vlc: bool,
}

/// `vop_time_increment` bits for a resolution: enough for `resolution - 1`, at least one.
pub(super) fn time_bits(resolution: u32) -> u32 {
    (32 - resolution.saturating_sub(1).leading_zeros()).max(1)
}

const SHAPE_RECTANGULAR: u32 = 0;

/// Read a quantiser matrix sent in zigzag order: up to 64 values, a 0 ends it early and the last
/// value repeats.
fn quant_matrix(r: &mut BitReader) -> Result<[u8; 64]> {
    let mut zz = [0u8; 64];
    let mut n = 0;
    while n < 64 {
        let v = r.read_bits(8).map_err(eof)? as u8;
        if v == 0 {
            break;
        }
        zz[n] = v;
        n += 1;
    }
    if n == 0 {
        return Err(DecodeError::Invalid("empty quantiser matrix"));
    }
    let last = zz[n - 1];
    zz[n..].fill(last);
    let mut m = [0u8; 64];
    for (i, &p) in ZIGZAG.iter().enumerate() {
        m[p] = zz[i];
    }
    Ok(m)
}

fn unsupported(what: &'static str) -> DecodeError {
    DecodeError::Unsupported(what)
}

impl Vol {
    pub(super) fn parse(data: &[u8]) -> Result<Vol> {
        let mut r = BitReader::new(data);
        let r = &mut r;
        let bits = |r: &mut BitReader, n| r.read_bits(n).map_err(eof);
        let flag = |r: &mut BitReader| r.read_bit().map_err(eof);
        flag(r)?; // random_accessible_vol
        let object_type = bits(r, 8)? as u8;
        let mut verid = 1;
        if flag(r)? {
            verid = bits(r, 4)? as u8;
            bits(r, 3)?; // video_object_layer_priority
        }
        let sar = match bits(r, 4)? {
            1 => (1, 1),
            2 => (12, 11),
            3 => (10, 11),
            4 => (16, 11),
            5 => (40, 33),
            15 => {
                let w = bits(r, 8)? as u16;
                let h = bits(r, 8)? as u16;
                if w == 0 || h == 0 { (1, 1) } else { (w, h) }
            }
            _ => (1, 1),
        };
        // Without vol_control_parameters, only the Simple object types are low delay.
        let mut low_delay = matches!(object_type, 0 | 1);
        if flag(r)? {
            if bits(r, 2)? != 1 {
                return Err(unsupported("chroma format other than 4:2:0"));
            }
            low_delay = flag(r)?;
            if flag(r)? {
                // vbv_parameters: 15+1+15+1+15+1+3+11+1+15+1 bits
                r.skip(79).map_err(eof)?;
            }
        }
        let shape = bits(r, 2)?;
        if shape != SHAPE_RECTANGULAR {
            return Err(unsupported("non-rectangular video object layer shape"));
        }
        marker(r)?;
        let time_resolution = bits(r, 16)?;
        if time_resolution == 0 {
            return Err(DecodeError::Invalid("zero vop_time_increment_resolution"));
        }
        marker(r)?;
        let tb = time_bits(time_resolution);
        let fixed_increment = if flag(r)? { Some(bits(r, tb)?) } else { None };
        marker(r)?;
        let width = bits(r, 13)?;
        marker(r)?;
        let height = bits(r, 13)?;
        marker(r)?;
        if width == 0 || height == 0 {
            return Err(DecodeError::Invalid("zero picture size"));
        }
        let interlaced = flag(r)?;
        if !flag(r)? {
            return Err(unsupported("overlapped block motion compensation"));
        }
        let sprite = match bits(r, if verid == 1 { 1 } else { 2 })? {
            0 => Sprite::None,
            1 => Sprite::Static,
            2 => Sprite::Gmc,
            _ => return Err(DecodeError::Invalid("reserved sprite_enable")),
        };
        if sprite == Sprite::Static {
            return Err(unsupported("static sprites"));
        }
        let (mut warp_points, mut warp_accuracy) = (0, 0);
        if sprite == Sprite::Gmc {
            warp_points = bits(r, 6)? as u8;
            if warp_points > 3 {
                return Err(unsupported("more than three GMC warping points"));
            }
            warp_accuracy = bits(r, 2)? as u8;
            if flag(r)? {
                return Err(unsupported("sprite brightness change"));
            }
        }
        if flag(r)? {
            // not_8_bit
            let quant_precision = bits(r, 4)?;
            let bits_per_pixel = bits(r, 4)?;
            if quant_precision != 5 || bits_per_pixel != 8 {
                return Err(unsupported("samples other than 8 bits"));
            }
        }
        let mpeg_quant = flag(r)?;
        let (mut intra_matrix, mut inter_matrix) = (DEFAULT_INTRA_MATRIX, DEFAULT_INTER_MATRIX);
        if mpeg_quant {
            if flag(r)? {
                intra_matrix = quant_matrix(r)?;
            }
            if flag(r)? {
                inter_matrix = quant_matrix(r)?;
            }
        }
        let quarter_sample = verid != 1 && flag(r)?;
        if !flag(r)? {
            return Err(unsupported("complexity estimation headers"));
        }
        let resync_markers = !flag(r)?;
        let data_partitioned = flag(r)?;
        let reversible_vlc = data_partitioned && flag(r)?;
        if verid != 1 {
            if flag(r)? {
                return Err(unsupported("NEWPRED"));
            }
            if flag(r)? {
                return Err(unsupported("reduced resolution VOPs"));
            }
        }
        if flag(r)? {
            return Err(unsupported("scalability"));
        }
        Ok(Vol {
            verid,
            object_type,
            sar,
            low_delay,
            time_resolution,
            time_bits: tb,
            fixed_increment,
            width,
            height,
            interlaced,
            sprite,
            warp_points,
            warp_accuracy,
            mpeg_quant,
            intra_matrix,
            inter_matrix,
            quarter_sample,
            resync_markers,
            data_partitioned,
            reversible_vlc,
        })
    }
}

/// Group of VOP header: the time code in seconds.
pub(super) fn parse_gov(data: &[u8]) -> Result<u64> {
    let mut r = BitReader::new(data);
    let h = r.read_bits(5).map_err(eof)?;
    let m = r.read_bits(6).map_err(eof)?;
    marker(&mut r)?;
    let s = r.read_bits(6).map_err(eof)?;
    Ok(u64::from(h) * 3600 + u64::from(m) * 60 + u64::from(s))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VopKind {
    I,
    P,
    B,
    S,
}

/// A VOP header (`00 00 01 B6`).
#[derive(Clone, Debug)]
pub(super) struct Vop {
    pub kind: VopKind,
    pub modulo: u32,
    pub increment: u32,
    pub coded: bool,
    pub rounding: bool,
    pub dc_thr: u8,
    pub top_field_first: bool,
    pub alternate_scan: bool,
    pub quant: u8,
    pub fcode_f: u8,
    pub fcode_b: u8,
    /// Sprite warping vector differences `(du, dv)` of each point.
    pub warp: [(i32, i32); 3],
    /// Bit position of the first macroblock in the VOP payload.
    pub data_start: usize,
}

/// `warping_mv_code`: a `dmv_length`, the value, a marker.
fn warping_mv(r: &mut BitReader) -> Result<i32> {
    let len = vlc::dmv_length().read(r, "invalid sprite warping vector")?;
    let v = if len == 0 {
        0
    } else {
        let raw = r.read_bits(len).map_err(eof)? as i32;
        if raw >> (len - 1) == 0 { raw - ((1 << len) - 1) } else { raw }
    };
    marker(r)?;
    Ok(v)
}

/// Saturating count of the `1` bits of `modulo_time_base`.
fn modulo_time_base(r: &mut BitReader) -> Result<u32> {
    let mut n = 0u32;
    while r.read_bit().map_err(eof)? {
        n = n.saturating_add(1);
    }
    Ok(n)
}

impl Vop {
    pub(super) fn parse(data: &[u8], vol: &Vol) -> Result<Vop> {
        let mut r = BitReader::new(data);
        let r = &mut r;
        let bits = |r: &mut BitReader, n| r.read_bits(n).map_err(eof);
        let flag = |r: &mut BitReader| r.read_bit().map_err(eof);
        let kind = match bits(r, 2)? {
            0 => VopKind::I,
            1 => VopKind::P,
            2 => VopKind::B,
            _ => VopKind::S,
        };
        if kind == VopKind::S && vol.sprite != Sprite::Gmc {
            return Err(DecodeError::Invalid("S-VOP without GMC"));
        }
        let modulo = modulo_time_base(r)?;
        marker(r)?;
        let increment = bits(r, vol.time_bits)?;
        marker(r)?;
        let mut vop = Vop {
            kind,
            modulo,
            increment,
            coded: flag(r)?,
            rounding: false,
            dc_thr: 0,
            top_field_first: true,
            alternate_scan: false,
            quant: 1,
            fcode_f: 1,
            fcode_b: 1,
            warp: [(0, 0); 3],
            data_start: 0,
        };
        if !vop.coded {
            return Ok(vop);
        }
        if matches!(kind, VopKind::P | VopKind::S) {
            vop.rounding = flag(r)?;
        }
        vop.dc_thr = bits(r, 3)? as u8;
        if vol.interlaced {
            vop.top_field_first = flag(r)?;
            vop.alternate_scan = flag(r)?;
        }
        if kind == VopKind::S {
            for p in vop.warp.iter_mut().take(usize::from(vol.warp_points)) {
                *p = (warping_mv(r)?, warping_mv(r)?);
            }
        }
        vop.quant = bits(r, 5)? as u8;
        if vop.quant == 0 {
            return Err(DecodeError::Invalid("zero vop_quant"));
        }
        if kind != VopKind::I {
            vop.fcode_f = bits(r, 3)? as u8;
            if vop.fcode_f == 0 {
                return Err(DecodeError::Invalid("zero vop_fcode_forward"));
            }
        }
        if kind == VopKind::B {
            vop.fcode_b = bits(r, 3)? as u8;
            if vop.fcode_b == 0 {
                return Err(DecodeError::Invalid("zero vop_fcode_backward"));
            }
        }
        vop.data_start = r.position();
        Ok(vop)
    }
}

/// The fields of a video packet header after its resync marker (§6.2.5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PacketHeader {
    pub mb_index: usize,
    pub quant: u8,
}

/// Bits of `macroblock_number` for a VOP of `mbs` macroblocks.
pub(super) fn mb_number_bits(mbs: usize) -> u32 {
    (usize::BITS - mbs.saturating_sub(1).leading_zeros()).max(1)
}

/// Length of the resync marker (its zeros and the final 1) of a VOP.
pub(super) fn resync_marker_len(vop: &Vop) -> u32 {
    match vop.kind {
        VopKind::I => 17,
        VopKind::P | VopKind::S => 16 + u32::from(vop.fcode_f),
        VopKind::B => 16 + u32::from(vop.fcode_f.max(vop.fcode_b).max(2)),
    }
}

/// Parse a video packet header, the reader just past the resync marker. The header extension's
/// fields must repeat the VOP header's; they are read and skipped.
pub(super) fn parse_packet_header(r: &mut BitReader, vol: &Vol, vop: &Vop, mbs: usize) -> Result<PacketHeader> {
    let mb_index = r.read_bits(mb_number_bits(mbs)).map_err(eof)? as usize;
    if mb_index >= mbs {
        return Err(DecodeError::Invalid("macroblock_number past the VOP"));
    }
    let quant = r.read_bits(5).map_err(eof)? as u8;
    if quant == 0 {
        return Err(DecodeError::Invalid("zero quant_scale"));
    }
    if r.read_bit().map_err(eof)? {
        // header_extension_code
        modulo_time_base(r)?;
        marker(r)?;
        r.read_bits(vol.time_bits).map_err(eof)?;
        marker(r)?;
        r.read_bits(2).map_err(eof)?; // vop_coding_type
        r.read_bits(3).map_err(eof)?; // intra_dc_vlc_thr
        if vop.kind == VopKind::S {
            for _ in 0..vol.warp_points {
                warping_mv(r)?;
                warping_mv(r)?;
            }
        }
        if vop.kind != VopKind::I {
            r.read_bits(3).map_err(eof)?;
        }
        if vop.kind == VopKind::B {
            r.read_bits(3).map_err(eof)?;
        }
    }
    Ok(PacketHeader { mb_index, quant })
}

/// Facts read from user data (`00 00 01 B2`): the DivX and Xvid build strings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct UserData {
    /// DivX: the stream packs a B-VOP with the following P-VOP (`p` after the build number).
    pub divx_packed: Option<bool>,
    pub xvid_build: Option<u32>,
    pub divx_build: Option<u32>,
}

fn leading_number(s: &[u8]) -> Option<(u32, usize)> {
    let n = s.iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 || n > 9 {
        return None;
    }
    std::str::from_utf8(&s[..n]).ok()?.parse().ok().map(|v| (v, n))
}

pub(super) fn parse_user_data(data: &[u8], into: &mut UserData) {
    if let Some(rest) = data.strip_prefix(b"XviD") {
        into.xvid_build = leading_number(rest).map(|(v, _)| v);
    } else if let Some(rest) = data.strip_prefix(b"DivX") {
        // "DivX503b1393p": version, "b" or "Build", build number, optional "p".
        let Some((_, n)) = leading_number(rest) else { return };
        let rest = &rest[n..];
        let rest = rest.strip_prefix(b"Build").or_else(|| rest.strip_prefix(b"b")).unwrap_or(rest);
        if let Some((build, m)) = leading_number(rest) {
            into.divx_build = Some(build);
            into.divx_packed = Some(rest.get(m) == Some(&b'p'));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_increment_bits() {
        assert_eq!(time_bits(1), 1);
        assert_eq!(time_bits(2), 1);
        assert_eq!(time_bits(25), 5);
        assert_eq!(time_bits(16), 4);
        assert_eq!(time_bits(17), 5);
        assert_eq!(time_bits(30000), 15);
        assert_eq!(time_bits(65535), 16);
    }

    #[test]
    fn macroblock_number_bits() {
        assert_eq!(mb_number_bits(1), 1);
        assert_eq!(mb_number_bits(2), 1);
        assert_eq!(mb_number_bits(99), 7);
        assert_eq!(mb_number_bits(256), 8);
        assert_eq!(mb_number_bits(396), 9);
    }

    #[test]
    fn default_matrices_are_zigzag_monotone_where_expected() {
        assert_eq!(DEFAULT_INTRA_MATRIX[0], 8);
        assert_eq!(DEFAULT_INTRA_MATRIX[63], 45);
        assert_eq!(DEFAULT_INTER_MATRIX[0], 16);
        assert_eq!(DEFAULT_INTER_MATRIX[63], 33);
        // Both are symmetric.
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(DEFAULT_INTER_MATRIX[y * 8 + x], DEFAULT_INTER_MATRIX[x * 8 + y]);
            }
        }
    }

    #[test]
    fn user_data_strings() {
        let mut u = UserData::default();
        parse_user_data(b"DivX503b1393p", &mut u);
        assert_eq!((u.divx_build, u.divx_packed), (Some(1393), Some(true)));
        parse_user_data(b"DivX501Build413", &mut u);
        assert_eq!((u.divx_build, u.divx_packed), (Some(413), Some(false)));
        parse_user_data(b"XviD0064", &mut u);
        assert_eq!(u.xvid_build, Some(64));
    }
}
