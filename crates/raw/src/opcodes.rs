//! DNG opcode lists (`OpcodeList1/2/3`, DNG 1.7 chapter 7). Opcode lists are always big-endian.
//!
//! Every opcode is parsed and can be written back; unknown or malformed ones are kept as [`Opcode::Unknown`].
//! Applying them is the develop pipeline's work. Two rules of the specification that any reader needs are
//! here as data helpers: the DNG 1.6 skip rule ([`applied_ops`]: a `WarpRectilinear` or `WarpFisheye` that
//! immediately follows an *optional* `WarpRectilinear2` is skipped, the older opcode being the fallback for
//! readers that don't know opcode 14) and the `WarpRectilinear2` radial factor ([`wr2_factor`]).

use serde::{Deserialize, Serialize};

/// Rectangular area + plane selection + pitch shared by the "area" opcodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    pub plane: u32,
    pub planes: u32,
    pub row_pitch: u32,
    pub col_pitch: u32,
}

/// Values per coefficient set of `WarpRectilinear2`: 15 radial (`kr0..kr14`), 2 tangential (`kt0`, `kt1`),
/// `min_valid_radius`, `max_valid_radius`.
pub const WR2_LEN: usize = 19;
/// Most coefficient sets (planes) a `WarpRectilinear2` may carry.
const WR2_MAX_PLANES: usize = 4;
/// Largest magnitude accepted for a `WarpRectilinear2` coefficient (a real lens fit is far below this; it keeps the
/// polynomial finite for any radius up to 1).
const WR2_MAX_COEF: f64 = 1e9;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Opcode {
    /// Per-plane radial (kr0..kr3) + tangential (kt0, kt1) coefficients and the relative optical centre.
    WarpRectilinear {
        planes: Vec<[f64; 6]>,
        center: [f64; 2],
    },
    WarpFisheye {
        planes: Vec<[f64; 4]>,
        center: [f64; 2],
    },
    /// DNG 1.6 `WarpRectilinear2` (opcode 14; DNG specification 1.6.0.0, chapter 7, "WarpRectilinear2"). Per plane
    /// `[kr0 .. kr14, kt0, kt1, min_valid_radius, max_valid_radius]` ([`WR2_LEN`] values, see [`wr2_factor`]),
    /// the relative optical centre, and `reciprocalRadial`. `optional` is opcode flag bit 0, which turns on the skip
    /// rule for the next warp opcode.
    WarpRectilinear2 {
        planes: Vec<[f64; WR2_LEN]>,
        center: [f64; 2],
        reciprocal: bool,
        optional: bool,
    },
    FixVignetteRadial {
        k: [f64; 5],
        center: [f64; 2],
    },
    FixBadPixelsConstant {
        constant: u32,
        bayer_phase: u32,
    },
    FixBadPixelsList {
        bayer_phase: u32,
        points: Vec<(u32, u32)>,
        rects: Vec<[u32; 4]>,
    },
    TrimBounds {
        top: u32,
        left: u32,
        bottom: u32,
        right: u32,
    },
    MapTable {
        area: Area,
        table: Vec<u16>,
    },
    MapPolynomial {
        area: Area,
        coefficients: Vec<f64>,
    },
    GainMap {
        area: Area,
        points_v: u32,
        points_h: u32,
        spacing: [f64; 2],
        origin: [f64; 2],
        map_planes: u32,
        gains: Vec<f32>,
    },
    DeltaPerRow {
        area: Area,
        deltas: Vec<f32>,
    },
    DeltaPerColumn {
        area: Area,
        deltas: Vec<f32>,
    },
    ScalePerRow {
        area: Area,
        scales: Vec<f32>,
    },
    ScalePerColumn {
        area: Area,
        scales: Vec<f32>,
    },
    Unknown {
        id: u32,
        flags: u32,
        params: Vec<u8>,
    },
}

impl Opcode {
    /// Whether this is a lens correction (`WarpRectilinear`, `WarpRectilinear2`, `FixVignetteRadial`): from a DNG, or
    /// the equivalent of a camera's own correction recorded in its raw (Panasonic RW2). Developers that apply these
    /// through the "profile corrections" of the develop settings remove them before demosaicing.
    pub fn is_lens_correction(&self) -> bool {
        matches!(self, Opcode::WarpRectilinear { .. } | Opcode::WarpRectilinear2 { .. } | Opcode::FixVignetteRadial { .. })
    }

    /// This lens correction re-expressed for the part `crop` of the `width × height` image it was written for: applied
    /// to the cropped image it gives the crop of the corrected whole. The centre moves into the crop and the
    /// coefficients are rescaled for the crop's farthest corner, the radius every coordinate is normalised by. Centres
    /// use the warps' convention (0 and 1 at the first and last pixel), also for `FixVignetteRadial`, as the develop
    /// pipeline reads them. `None` for other opcodes, an image or crop under 2 × 2, or a crop outside the image.
    pub fn cropped(&self, width: usize, height: usize, crop: crate::Rect) -> Option<Opcode> {
        let fits = crop.x.checked_add(crop.width).is_some_and(|r| r <= width) && crop.y.checked_add(crop.height).is_some_and(|b| b <= height);
        if width < 2 || height < 2 || crop.width < 2 || crop.height < 2 || !fits {
            return None;
        }
        let (aw, ah) = (width as f64 - 1.0, height as f64 - 1.0);
        let (cw, ch) = (crop.width as f64 - 1.0, crop.height as f64 - 1.0);
        // the centre in the crop, and the ratio of the crop's normalising radius to the image's
        let reframe = |c: [f64; 2]| -> Option<([f64; 2], f64)> {
            let (px, py) = (c[0] * aw, c[1] * ah);
            let (qx, qy) = (px - crop.x as f64, py - crop.y as f64);
            let s = max_radius(qx, qy, cw, ch) / max_radius(px, py, aw, ah);
            (s.is_finite() && s > 0.0 && qx.is_finite() && qy.is_finite()).then_some(([qx / cw, qy / ch], s))
        };
        match self {
            // source radius r·(kr0 + kr1 r² + kr2 r⁴ + kr3 r⁶) and tangential terms quadratic in r: with r = r'·s,
            // kr_i scales by s^2i and kt by s
            Opcode::WarpRectilinear { planes, center } => {
                let (center, s) = reframe(*center)?;
                let planes = planes
                    .iter()
                    .map(|k| {
                        let mut k = *k;
                        for (i, v) in k[..4].iter_mut().enumerate() {
                            *v *= s.powi(2 * i as i32);
                        }
                        k[4] *= s;
                        k[5] *= s;
                        k
                    })
                    .collect();
                Some(Opcode::WarpRectilinear { planes, center })
            }
            // radial factor Σ kr_i r^i between the valid radii: kr_i scales by s^i, the radii by 1/s
            Opcode::WarpRectilinear2 { planes, center, reciprocal, optional } => {
                let (center, s) = reframe(*center)?;
                let planes = planes
                    .iter()
                    .map(|k| {
                        let mut k = *k;
                        for (i, v) in k[..15].iter_mut().enumerate() {
                            *v *= s.powi(i as i32);
                        }
                        k[15] *= s;
                        k[16] *= s;
                        k[17] /= s;
                        k[18] /= s;
                        k
                    })
                    .collect();
                Some(Opcode::WarpRectilinear2 { planes, center, reciprocal: *reciprocal, optional: *optional })
            }
            // gain 1 + Σ k_i r^(2i+2)
            Opcode::FixVignetteRadial { k, center } => {
                let (center, s) = reframe(*center)?;
                let mut k = *k;
                for (i, v) in k.iter_mut().enumerate() {
                    *v *= s.powi(2 * i as i32 + 2);
                }
                Some(Opcode::FixVignetteRadial { k, center })
            }
            _ => None,
        }
    }
}

/// The opcodes of a list that a reader applies, in order: all of them, minus the DNG 1.6 skip rule (specification
/// 1.6.0.0, chapter 7) — a `WarpRectilinear` or `WarpFisheye` immediately after a `WarpRectilinear2` that is marked
/// optional is not applied. A `WarpRectilinear2` that could not be parsed ([`Opcode::Unknown`]) triggers no skip.
pub fn applied_ops(list: &[Opcode]) -> impl Iterator<Item = &Opcode> {
    let mut skip = false;
    list.iter().filter(move |op| {
        let skipped = skip && matches!(op, Opcode::WarpRectilinear { .. } | Opcode::WarpFisheye { .. });
        skip = matches!(op, Opcode::WarpRectilinear2 { optional: true, .. });
        !skipped
    })
}

/// `WarpRectilinear2` radial factor `r_real / r_ideal` for one coefficient set `k` at ideal radius `r`
/// (specification 1.6.0.0, "WarpRectilinear2"): `f(r) = Σ kr_p · r^p` (p = 0..14) evaluated at `r` clamped to
/// `[min_valid_radius, max_valid_radius]`, so `r_real = r · f` — or `r / f` when `reciprocalRadial` is non-zero.
/// `None` when the factor is not finite (a degenerate set, or `f = 0` with `reciprocalRadial`).
pub fn wr2_factor(k: &[f64; WR2_LEN], reciprocal: bool, r: f64) -> Option<f64> {
    let rc = r.clamp(k[17].min(k[18]), k[17].max(k[18]));
    let f = k[..15].iter().rev().fold(0.0, |acc, &c| acc * rc + c);
    let f = if reciprocal { 1.0 / f } else { f };
    f.is_finite().then_some(f)
}

/// The three opcode lists of a DNG raw IFD.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OpcodeLists {
    pub list1: Vec<Opcode>,
    pub list2: Vec<Opcode>,
    pub list3: Vec<Opcode>,
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_be_bytes(self.d.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_be_bytes(self.d.get(self.p..self.p + 2)?.try_into().ok()?);
        self.p += 2;
        Some(v)
    }
    fn f64(&mut self) -> Option<f64> {
        let v = f64::from_be_bytes(self.d.get(self.p..self.p + 8)?.try_into().ok()?);
        self.p += 8;
        Some(v)
    }
    fn finite_f64(&mut self) -> Option<f64> {
        self.f64().filter(|v| v.is_finite())
    }
    fn f32(&mut self) -> Option<f32> {
        let v = f32::from_be_bytes(self.d.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    fn area(&mut self) -> Option<Area> {
        Some(Area {
            top: self.u32()?,
            left: self.u32()?,
            bottom: self.u32()?,
            right: self.u32()?,
            plane: self.u32()?,
            planes: self.u32()?,
            row_pitch: self.u32()?.max(1),
            col_pitch: self.u32()?.max(1),
        })
    }
    fn remaining(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }
}

fn parse_one(id: u32, flags: u32, params: &[u8]) -> Option<Opcode> {
    let mut reader = Rd { d: params, p: 0 };
    let parsed = match id {
        1 | 2 | 3 | 14 => parse_warp(id, flags, &mut reader)?,
        4 => Opcode::FixBadPixelsConstant { constant: reader.u32()?, bayer_phase: reader.u32()? },
        5 => parse_bad_pixel_list(&mut reader)?,
        6 => Opcode::TrimBounds { top: reader.u32()?, left: reader.u32()?, bottom: reader.u32()?, right: reader.u32()? },
        7..=13 => parse_area_opcode(id, &mut reader)?,
        _ => Opcode::Unknown { id, flags, params: params.to_vec() },
    };
    Some(parsed)
}

fn parse_warp(id: u32, flags: u32, reader: &mut Rd<'_>) -> Option<Opcode> {
    match id {
        1 => {
            let planes = parse_planes(reader, 4, |r| {
                Some([r.finite_f64()?, r.finite_f64()?, r.finite_f64()?, r.finite_f64()?, r.finite_f64()?, r.finite_f64()?])
            })?;
            Some(Opcode::WarpRectilinear { planes, center: [reader.finite_f64()?, reader.finite_f64()?] })
        }
        2 => {
            let planes = parse_planes(reader, 4, |r| Some([r.finite_f64()?, r.finite_f64()?, r.finite_f64()?, r.finite_f64()?]))?;
            Some(Opcode::WarpFisheye { planes, center: [reader.finite_f64()?, reader.finite_f64()?] })
        }
        3 => Some(Opcode::FixVignetteRadial {
            k: [reader.finite_f64()?, reader.finite_f64()?, reader.finite_f64()?, reader.finite_f64()?, reader.finite_f64()?],
            center: [reader.finite_f64()?, reader.finite_f64()?],
        }),
        14 => parse_warp_rectilinear2(flags, reader),
        _ => None,
    }
}

fn parse_planes<const N: usize>(reader: &mut Rd<'_>, max: usize, mut parse: impl FnMut(&mut Rd<'_>) -> Option<[f64; N]>) -> Option<Vec<[f64; N]>> {
    let count = reader.u32()? as usize;
    if count == 0 || count > max {
        return None;
    }
    (0..count).map(|_| parse(reader)).collect()
}

fn parse_warp_rectilinear2(flags: u32, reader: &mut Rd<'_>) -> Option<Opcode> {
    let count = reader.u32()? as usize;
    // N sets of 19 doubles, then the centre (2 doubles) and reciprocalRadial (LONG); nothing else.
    if count == 0 || count > WR2_MAX_PLANES || reader.remaining() != count * WR2_LEN * 8 + 16 + 4 {
        return None;
    }
    let mut planes = Vec::with_capacity(count);
    for _ in 0..count {
        let mut coefficients = [0.0; WR2_LEN];
        for value in &mut coefficients {
            *value = reader.finite_f64().filter(|value| value.abs() <= WR2_MAX_COEF)?;
        }
        if !(coefficients[17] >= 0.0 && coefficients[17] < coefficients[18] && coefficients[18] <= 1.0) {
            return None;
        }
        planes.push(coefficients);
    }
    let center = [reader.finite_f64().filter(|value| value.abs() <= 4.0)?, reader.finite_f64().filter(|value| value.abs() <= 4.0)?];
    Some(Opcode::WarpRectilinear2 { planes, center, reciprocal: reader.u32()? != 0, optional: flags & 1 != 0 })
}

fn parse_bad_pixel_list(reader: &mut Rd<'_>) -> Option<Opcode> {
    let bayer_phase = reader.u32()?;
    let (point_count, rect_count) = (reader.u32()? as usize, reader.u32()? as usize);
    if point_count.saturating_mul(8).saturating_add(rect_count.saturating_mul(16)) > reader.remaining() {
        return None;
    }
    let points = (0..point_count).map(|_| Some((reader.u32()?, reader.u32()?))).collect::<Option<_>>()?;
    let rects = (0..rect_count).map(|_| Some([reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?])).collect::<Option<_>>()?;
    Some(Opcode::FixBadPixelsList { bayer_phase, points, rects })
}

fn parse_area_opcode(id: u32, reader: &mut Rd<'_>) -> Option<Opcode> {
    let area = reader.area()?;
    match id {
        7 => {
            let count = reader.u32()? as usize;
            if count == 0 || count > 65536 || count * 2 > reader.remaining() {
                return None;
            }
            Some(Opcode::MapTable { area, table: (0..count).map(|_| reader.u16()).collect::<Option<_>>()? })
        }
        8 => {
            let degree = reader.u32()? as usize;
            if degree > 8 {
                return None;
            }
            Some(Opcode::MapPolynomial { area, coefficients: (0..=degree).map(|_| reader.finite_f64()).collect::<Option<_>>()? })
        }
        9 => parse_gain_map(area, reader),
        10..=13 => parse_delta_scale(id, area, reader),
        _ => None,
    }
}

fn parse_gain_map(area: Area, reader: &mut Rd<'_>) -> Option<Opcode> {
    let (points_v, points_h) = (reader.u32()?, reader.u32()?);
    let spacing = [reader.finite_f64()?, reader.finite_f64()?];
    let origin = [reader.finite_f64()?, reader.finite_f64()?];
    let map_planes = reader.u32()?;
    let count = (points_v as usize).checked_mul(points_h as usize)?.checked_mul(map_planes as usize)?;
    if count == 0 || count > reader.remaining() / 4 {
        return None;
    }
    let gains = (0..count).map(|_| reader.f32()).collect::<Option<_>>()?;
    Some(Opcode::GainMap { area, points_v, points_h, spacing, origin, map_planes, gains })
}

fn parse_delta_scale(id: u32, area: Area, reader: &mut Rd<'_>) -> Option<Opcode> {
    let count = reader.u32()? as usize;
    if count > reader.remaining() / 4 {
        return None;
    }
    let values: Vec<f32> = (0..count).map(|_| reader.f32()).collect::<Option<_>>()?;
    Some(match id {
        10 => Opcode::DeltaPerRow { area, deltas: values },
        11 => Opcode::DeltaPerColumn { area, deltas: values },
        12 => Opcode::ScalePerRow { area, scales: values },
        _ => Opcode::ScalePerColumn { area, scales: values },
    })
}

fn put_area(o: &mut Vec<u8>, a: &Area) {
    for v in [a.top, a.left, a.bottom, a.right, a.plane, a.planes, a.row_pitch, a.col_pitch] {
        o.extend_from_slice(&v.to_be_bytes());
    }
}

fn params(op: &Opcode) -> (u32, u32, Vec<u8>) {
    let mut o = Vec::new();
    let f64s = |o: &mut Vec<u8>, v: &[f64]| v.iter().for_each(|x| o.extend_from_slice(&x.to_be_bytes()));
    let u32s = |o: &mut Vec<u8>, v: &[u32]| v.iter().for_each(|x| o.extend_from_slice(&x.to_be_bytes()));
    let f32s = |o: &mut Vec<u8>, v: &[f32]| v.iter().for_each(|x| o.extend_from_slice(&x.to_be_bytes()));
    let id = match op {
        Opcode::WarpRectilinear { planes, center } => {
            u32s(&mut o, &[planes.len() as u32]);
            planes.iter().for_each(|p| f64s(&mut o, p));
            f64s(&mut o, center);
            1
        }
        Opcode::WarpFisheye { planes, center } => {
            u32s(&mut o, &[planes.len() as u32]);
            planes.iter().for_each(|p| f64s(&mut o, p));
            f64s(&mut o, center);
            2
        }
        Opcode::WarpRectilinear2 { planes, center, reciprocal, optional } => {
            u32s(&mut o, &[planes.len() as u32]);
            planes.iter().for_each(|p| f64s(&mut o, p));
            f64s(&mut o, center);
            u32s(&mut o, &[*reciprocal as u32]);
            return (14, *optional as u32, o);
        }
        Opcode::FixVignetteRadial { k, center } => {
            f64s(&mut o, k);
            f64s(&mut o, center);
            3
        }
        Opcode::FixBadPixelsConstant { constant, bayer_phase } => {
            u32s(&mut o, &[*constant, *bayer_phase]);
            4
        }
        Opcode::FixBadPixelsList { bayer_phase, points, rects } => {
            u32s(&mut o, &[*bayer_phase, points.len() as u32, rects.len() as u32]);
            points.iter().for_each(|&(r, c)| u32s(&mut o, &[r, c]));
            rects.iter().for_each(|r| u32s(&mut o, r));
            5
        }
        Opcode::TrimBounds { top, left, bottom, right } => {
            u32s(&mut o, &[*top, *left, *bottom, *right]);
            6
        }
        Opcode::MapTable { area, table } => {
            put_area(&mut o, area);
            u32s(&mut o, &[table.len() as u32]);
            table.iter().for_each(|v| o.extend_from_slice(&v.to_be_bytes()));
            7
        }
        Opcode::MapPolynomial { area, coefficients } => {
            put_area(&mut o, area);
            u32s(&mut o, &[coefficients.len().saturating_sub(1) as u32]);
            f64s(&mut o, coefficients);
            8
        }
        Opcode::GainMap { area, points_v, points_h, spacing, origin, map_planes, gains } => {
            put_area(&mut o, area);
            u32s(&mut o, &[*points_v, *points_h]);
            f64s(&mut o, spacing);
            f64s(&mut o, origin);
            u32s(&mut o, &[*map_planes]);
            f32s(&mut o, gains);
            9
        }
        Opcode::DeltaPerRow { area, deltas: v }
        | Opcode::DeltaPerColumn { area, deltas: v }
        | Opcode::ScalePerRow { area, scales: v }
        | Opcode::ScalePerColumn { area, scales: v } => {
            put_area(&mut o, area);
            u32s(&mut o, &[v.len() as u32]);
            f32s(&mut o, v);
            match op {
                Opcode::DeltaPerRow { .. } => 10,
                Opcode::DeltaPerColumn { .. } => 11,
                Opcode::ScalePerRow { .. } => 12,
                _ => 13,
            }
        }
        Opcode::Unknown { id, flags, params } => return (*id, *flags, params.clone()),
    };
    (id, 0, o)
}

/// Serialise an opcode list (inverse of [`parse_list`]).
pub fn write_list(list: &[Opcode]) -> Vec<u8> {
    let mut out = (list.len() as u32).to_be_bytes().to_vec();
    for op in list {
        let (id, flags, p) = params(op);
        for v in [id, if id == 14 { 0x0106_0000 } else { 0x0103_0000 }, flags, p.len() as u32] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&p);
    }
    out
}

/// Parse an opcode list blob. Malformed opcodes become [`Opcode::Unknown`]; a truncated list stops early.
pub fn parse_list(d: &[u8]) -> Vec<Opcode> {
    let mut r = Rd { d, p: 0 };
    let Some(count) = r.u32() else { return vec![] };
    let mut out = Vec::new();
    for _ in 0..count.min(4096) {
        let (Some(id), Some(_ver), Some(flags), Some(size)) = (r.u32(), r.u32(), r.u32(), r.u32()) else { break };
        let Some(params) = d.get(r.p..r.p.saturating_add(size as usize)) else { break };
        r.p += size as usize;
        out.push(parse_one(id, flags, params).unwrap_or(Opcode::Unknown { id, flags, params: params.to_vec() }));
    }
    out
}

/// Radius normaliser: distance from the centre to the farthest image corner.
pub(crate) fn max_radius(cx: f64, cy: f64, w: f64, h: f64) -> f64 {
    [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().map(|&(x, y)| (x - cx).hypot(y - cy)).fold(0.0, f64::max).max(1e-9)
}

#[cfg(test)]
mod tests;
