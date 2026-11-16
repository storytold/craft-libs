//! Context-aware parsing of DNG opcode parameters. Image processing remains the caller's job.
//!
//! The typed [`crate::opcodes`] model is the file's own view (owned, serialisable, lenient: a
//! malformed opcode becomes `Unknown`, a zero pitch reads as 1, coordinates stay as written). This
//! module answers a different question for a caller about to apply an opcode to a given image:
//! parameters borrowed from the record (bad-pixel lists are not copied), checked strictly (a zero
//! pitch, a count larger than its bytes or a warp on a mosaic is an error) and areas clamped to the
//! image. The record envelope comes from [`crate::opcode_raw`]; the rectangle on the wire is
//! [`crate::opcodes::Area`], which [`Area::clamp`] turns into image coordinates.

/// Image properties used to validate and clamp opcode parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub width: usize,
    pub height: usize,
    pub planes: usize,
    pub mosaic: bool,
    pub allow_trim: bool,
}
/// A parameter failure, retaining the diagnostic used by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Malformed(&'static str),
    Unsupported(&'static str),
}
/// A clamped image area and its sample pitches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub plane: usize,
    pub planes: usize,
    pub row_pitch: usize,
    pub col_pitch: usize,
}
impl Area {
    /// Iterate sampled rows, rejecting a zero pitch even for caller-constructed areas.
    pub fn rows(&self) -> Result<impl Iterator<Item = usize> + use<>, ParseError> {
        if self.row_pitch == 0 {
            return Err(ParseError::Malformed("opcode area with a zero pitch"));
        }
        Ok((self.top..self.bottom.max(self.top)).step_by(self.row_pitch))
    }
    /// Iterate sampled columns, rejecting a zero pitch even for caller-constructed areas.
    pub fn cols(&self) -> Result<impl Iterator<Item = usize> + use<>, ParseError> {
        if self.col_pitch == 0 {
            return Err(ParseError::Malformed("opcode area with a zero pitch"));
        }
        Ok((self.left..self.right.max(self.left)).step_by(self.col_pitch))
    }
}
/// Borrowed bad-pixel coordinates, decoded lazily without copying the payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointRecords<'a>(&'a [u8]);
impl PointRecords<'_> {
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.0.as_chunks::<8>().0.iter().map(|b| (u32::from_be_bytes([b[0], b[1], b[2], b[3]]), u32::from_be_bytes([b[4], b[5], b[6], b[7]])))
    }
}
/// Borrowed bad-pixel rectangles, decoded lazily without copying the payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleRecords<'a>(&'a [u8]);
impl RectangleRecords<'_> {
    pub fn iter(&self) -> impl Iterator<Item = [u32; 4]> + '_ {
        self.0.as_chunks::<16>().0.iter().map(|b| {
            [
                u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
                u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
                u32::from_be_bytes([b[12], b[13], b[14], b[15]]),
            ]
        })
    }
}
/// Parameters for the fourteen defined opcodes.
#[derive(Clone, Debug, PartialEq)]
pub enum Params<'a> {
    WarpRectilinear { planes: Vec<[f64; 6]>, center: [f64; 2] },
    WarpFisheye { planes: Vec<[f64; 4]>, center: [f64; 2] },
    FixVignetteRadial { k: [f64; 5], center: [f64; 2] },
    FixBadPixelsConstant { constant: u32, bayer_phase: u32 },
    FixBadPixelsList { bayer_phase: u32, points: PointRecords<'a>, rects: RectangleRecords<'a> },
    TrimBounds { top: usize, left: usize, bottom: usize, right: usize },
    MapTable { area: Area, table: Vec<u16> },
    MapPolynomial { area: Area, coefficients: Vec<f64> },
    GainMap { area: Area, points_v: usize, points_h: usize, spacing: [f64; 2], origin: [f64; 2], map_planes: usize, gains: Vec<f32> },
    DeltaPerRow { area: Area, deltas: Vec<f32> },
    DeltaPerColumn { area: Area, deltas: Vec<f32> },
    ScalePerRow { area: Area, scales: Vec<f32> },
    ScalePerColumn { area: Area, scales: Vec<f32> },
    WarpRectilinear2 { planes: Vec<[f64; 19]>, center: [f64; 2], reciprocal: bool },
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        let end = self.pos.checked_add(n).ok_or(ParseError::Malformed("opcode parameters"))?;
        let b = self.bytes.get(self.pos..end).ok_or(ParseError::Malformed("opcode parameters too short"))?;
        self.pos = end;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, ParseError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Result<f32, ParseError> {
        let v = f32::from_bits(self.u32()?);
        if v.is_finite() { Ok(v) } else { Err(ParseError::Malformed("non-finite opcode parameter")) }
    }
    fn f64(&mut self) -> Result<f64, ParseError> {
        let b = self.take(8)?;
        let v = f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        if v.is_finite() { Ok(v) } else { Err(ParseError::Malformed("non-finite opcode parameter")) }
    }
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }
    fn count(&mut self, size: usize) -> Result<usize, ParseError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(size) > self.remaining() { Err(ParseError::Malformed("opcode count larger than its parameters")) } else { Ok(n) }
    }
    fn array<const N: usize>(&mut self) -> Result<[f64; N], ParseError> {
        let mut a = [0.0; N];
        for x in &mut a {
            *x = self.f64()?;
        }
        Ok(a)
    }
    fn area(&mut self, c: Context) -> Result<Area, ParseError> {
        let wire = crate::opcodes::Area {
            top: self.u32()?,
            left: self.u32()?,
            bottom: self.u32()?,
            right: self.u32()?,
            plane: self.u32()?,
            planes: self.u32()?,
            row_pitch: self.u32()?,
            col_pitch: self.u32()?,
        };
        Area::clamp(wire, c)
    }
}
impl Area {
    /// The area of an opcode as written ([`crate::opcodes::Area`]) clamped to the image of `c`:
    /// coordinates to its size, the plane range to its planes. A zero pitch is an error here (the
    /// typed reader reads it as 1).
    pub fn clamp(wire: crate::opcodes::Area, c: Context) -> Result<Area, ParseError> {
        if wire.row_pitch == 0 || wire.col_pitch == 0 {
            return Err(ParseError::Malformed("opcode area with a zero pitch"));
        }
        let px = |v: u32| v as usize;
        let plane = px(wire.plane).min(c.planes);
        Ok(Area {
            top: px(wire.top).min(c.height),
            left: px(wire.left).min(c.width),
            bottom: px(wire.bottom).min(c.height),
            right: px(wire.right).min(c.width),
            plane,
            planes: px(wire.planes).min(c.planes - plane),
            row_pitch: px(wire.row_pitch),
            col_pitch: px(wire.col_pitch),
        })
    }
}
/// Parse a known opcode's parameters. Unknown identifiers return an unsupported error.
pub fn parse(id: u32, bytes: &[u8], context: Context) -> Result<Params<'_>, ParseError> {
    let mut r = Reader { bytes, pos: 0 };
    match id {
        1 | 2 | 14 => {
            if context.mosaic {
                return Err(ParseError::Unsupported("a warp opcode on a CFA mosaic"));
            }
            let per = match id {
                1 => 6,
                2 => 4,
                _ => 19,
            };
            let n = r.count(per * 8)?;
            if n != 1 && n != context.planes {
                return Err(ParseError::Malformed("warp opcode coefficient sets"));
            }
            match id {
                1 => {
                    let planes = (0..n).map(|_| r.array()).collect::<Result<_, _>>()?;
                    let center = r.array()?;
                    Ok(Params::WarpRectilinear { planes, center })
                }
                2 => {
                    let planes = (0..n).map(|_| r.array()).collect::<Result<_, _>>()?;
                    let center = r.array()?;
                    Ok(Params::WarpFisheye { planes, center })
                }
                _ => {
                    let mut planes = Vec::with_capacity(n);
                    for _ in 0..n {
                        let mut p: [f64; 19] = r.array()?;
                        p[17] = p[17].clamp(0.0, 1.0);
                        p[18] = p[18].clamp(0.0, 1.0);
                        if p[17] >= p[18] {
                            return Err(ParseError::Malformed("WarpRectilinear2 valid radius range"));
                        }
                        planes.push(p);
                    }
                    let center = r.array()?;
                    let reciprocal = r.u32()? != 0;
                    Ok(Params::WarpRectilinear2 { planes, center, reciprocal })
                }
            }
        }
        3 => Ok(Params::FixVignetteRadial { k: r.array()?, center: r.array()? }),
        4 => Ok(Params::FixBadPixelsConstant { constant: r.u32()?, bayer_phase: r.u32()? }),
        5 => {
            let bayer_phase = r.u32()?;
            let points = r.u32()? as usize;
            let rects = r.u32()? as usize;
            if points.saturating_mul(8).saturating_add(rects.saturating_mul(16)) > r.remaining() {
                return Err(ParseError::Malformed("FixBadPixelsList counts larger than its parameters"));
            }
            let points =
                PointRecords(r.take(points.checked_mul(8).ok_or(ParseError::Malformed("FixBadPixelsList counts larger than its parameters"))?)?);
            let rects =
                RectangleRecords(r.take(rects.checked_mul(16).ok_or(ParseError::Malformed("FixBadPixelsList counts larger than its parameters"))?)?);
            Ok(Params::FixBadPixelsList { bayer_phase, points, rects })
        }
        6 => {
            let top = r.u32()? as usize;
            let left = r.u32()? as usize;
            let bottom = r.u32()? as usize;
            let right = r.u32()? as usize;
            if !context.allow_trim {
                return Err(ParseError::Unsupported("TrimBounds before the demosaic"));
            }
            let bottom = bottom.min(context.height);
            let right = right.min(context.width);
            if top >= bottom || left >= right {
                return Err(ParseError::Malformed("TrimBounds outside the picture"));
            }
            Ok(Params::TrimBounds { top, left, bottom, right })
        }
        7..=13 => {
            let area = r.area(context)?;
            match id {
                7 => {
                    let n = r.count(2)?;
                    if n == 0 {
                        return Err(ParseError::Malformed("MapTable without entries"));
                    }
                    let table = (0..n)
                        .map(|_| {
                            let b = r.take(2)?;
                            Ok(u16::from_be_bytes([b[0], b[1]]))
                        })
                        .collect::<Result<_, ParseError>>()?;
                    Ok(Params::MapTable { area, table })
                }
                8 => {
                    let degree = r.u32()? as usize;
                    if degree > 8 {
                        return Err(ParseError::Malformed("MapPolynomial degree above 8"));
                    }
                    let coefficients = (0..=degree).map(|_| r.f64()).collect::<Result<_, _>>()?;
                    Ok(Params::MapPolynomial { area, coefficients })
                }
                9 => {
                    let points_v = r.u32()? as usize;
                    let points_h = r.u32()? as usize;
                    let spacing = r.array()?;
                    let origin = r.array()?;
                    let map_planes = r.u32()? as usize;
                    let n = points_v.saturating_mul(points_h).saturating_mul(map_planes);
                    if points_v == 0 || points_h == 0 || map_planes == 0 || n.saturating_mul(4) > r.remaining() {
                        return Err(ParseError::Malformed("GainMap size"));
                    }
                    if (points_v > 1 && spacing[0] <= 0.0) || (points_h > 1 && spacing[1] <= 0.0) {
                        return Err(ParseError::Malformed("GainMap spacing"));
                    }
                    let gains = (0..n).map(|_| r.f32()).collect::<Result<_, _>>()?;
                    Ok(Params::GainMap { area, points_v, points_h, spacing, origin, map_planes, gains })
                }
                _ => {
                    let n = r.count(4)?;
                    let vals = (0..n).map(|_| r.f32()).collect::<Result<Vec<_>, _>>()?;
                    let need = if matches!(id, 10 | 12) {
                        area.bottom.saturating_sub(area.top).div_ceil(area.row_pitch)
                    } else {
                        area.right.saturating_sub(area.left).div_ceil(area.col_pitch)
                    };
                    if vals.len() < need {
                        return Err(ParseError::Malformed("opcode with fewer values than rows or columns"));
                    }
                    Ok(match id {
                        10 => Params::DeltaPerRow { area, deltas: vals },
                        11 => Params::DeltaPerColumn { area, deltas: vals },
                        12 => Params::ScalePerRow { area, scales: vals },
                        _ => Params::ScalePerColumn { area, scales: vals },
                    })
                }
            }
        }
        _ => Err(ParseError::Unsupported("unknown DNG opcode")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }
    fn d(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }
    fn f(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }
    fn ctx() -> Context {
        Context { width: 4, height: 3, planes: 2, mosaic: false, allow_trim: true }
    }
    #[test]
    fn caller_constructed_areas_reject_zero_pitch() -> Result<(), ParseError> {
        let mut area = Area { top: 1, left: 2, bottom: 7, right: 9, plane: 0, planes: 1, row_pitch: 2, col_pitch: 3 };
        assert_eq!(area.rows()?.collect::<Vec<_>>(), vec![1, 3, 5]);
        assert_eq!(area.cols()?.collect::<Vec<_>>(), vec![2, 5, 8]);
        area.row_pitch = 0;
        area.col_pitch = 0;
        assert!(area.rows().is_err());
        assert!(area.cols().is_err());
        // Empty/reversed bounds must not allow an invalid pitch through either.
        area.bottom = 0;
        area.right = 0;
        assert!(area.rows().is_err());
        assert!(area.cols().is_err());
        Ok(())
    }

    #[test]
    fn parses_all_variants_and_clamps_areas() -> Result<(), ParseError> {
        let area = u(&[0, 1, 8, 9, 1, 5, 1, 2]);
        let cases = vec![
            (1, [u(&[1]), d(&[1., 0., 0., 0., 0., 0., 0.5, 0.5])].concat()),
            (2, [u(&[1]), d(&[1., 0., 0., 0., 0.5, 0.5])].concat()),
            (3, d(&[0., 0., 0., 0., 0., 0.5, 0.5])),
            (4, u(&[0, 0])),
            (5, u(&[0, 1, 1, 1, 2, 0, 0, 1, 1])),
            (6, u(&[0, 1, 8, 9])),
            (7, [area.clone(), u(&[1]), vec![0, 1]].concat()),
            (8, [area.clone(), u(&[0]), d(&[1.])].concat()),
            (9, [area.clone(), u(&[1, 1]), d(&[0., 0., 0., 0.]), u(&[1]), f(&[1.])].concat()),
            (10, [area.clone(), u(&[3]), f(&[1., 2., 3.])].concat()),
            (11, [area.clone(), u(&[2]), f(&[1., 2.])].concat()),
            (12, [area.clone(), u(&[3]), f(&[1., 2., 3.])].concat()),
            (13, [area.clone(), u(&[2]), f(&[1., 2.])].concat()),
            (14, [u(&[1]), d(&[0.; 17]), d(&[-1., 2., 0.5, 0.5]), u(&[1])].concat()),
        ];
        for (id, bytes) in cases {
            let p = parse(id, &bytes, ctx())?;
            assert!(matches!(
                (id, p),
                (1, Params::WarpRectilinear { .. })
                    | (2, Params::WarpFisheye { .. })
                    | (3, Params::FixVignetteRadial { .. })
                    | (4, Params::FixBadPixelsConstant { .. })
                    | (5, Params::FixBadPixelsList { .. })
                    | (6, Params::TrimBounds { .. })
                    | (7, Params::MapTable { .. })
                    | (8, Params::MapPolynomial { .. })
                    | (9, Params::GainMap { .. })
                    | (10, Params::DeltaPerRow { .. })
                    | (11, Params::DeltaPerColumn { .. })
                    | (12, Params::ScalePerRow { .. })
                    | (13, Params::ScalePerColumn { .. })
                    | (14, Params::WarpRectilinear2 { .. })
            ));
        }
        assert!(matches!(
            parse(7, &[area, u(&[1]), vec![0, 1]].concat(), ctx()),
            Ok(Params::MapTable { area: Area { bottom: 3, right: 4, plane: 1, planes: 1, .. }, .. })
        ));
        Ok(())
    }
    #[test]
    fn validates_in_application_order() {
        assert_eq!(parse(1, &[], Context { mosaic: true, ..ctx() }), Err(ParseError::Unsupported("a warp opcode on a CFA mosaic")));
        assert_eq!(parse(6, &[], Context { allow_trim: false, ..ctx() }), Err(ParseError::Malformed("opcode parameters too short")));
        assert_eq!(
            parse(6, &u(&[0, 0, 1, 1]), Context { allow_trim: false, ..ctx() }),
            Err(ParseError::Unsupported("TrimBounds before the demosaic"))
        );
        assert_eq!(parse(5, &u(&[0, u32::MAX, u32::MAX]), ctx()), Err(ParseError::Malformed("FixBadPixelsList counts larger than its parameters")));
        let a = u(&[0, 0, 3, 4, 0, 1, 1, 1]);
        assert_eq!(parse(10, &[a, u(&[1]), f(&[f32::NAN])].concat(), ctx()), Err(ParseError::Malformed("non-finite opcode parameter")));
    }

    #[test]
    fn typed_areas_clamp_to_the_image() {
        let c = ctx();
        let wire = crate::opcodes::Area { top: 1, left: 2, bottom: 9_999, right: 9_999, plane: 9, planes: 9, row_pitch: 2, col_pitch: 1 };
        let a = Area::clamp(wire, c).unwrap();
        assert_eq!((a.bottom, a.right), (c.height, c.width));
        assert_eq!((a.plane, a.planes), (c.planes, 0));
        assert!(Area::clamp(crate::opcodes::Area { row_pitch: 0, ..wire }, c).is_err());
        // the typed reader and this one agree on a well-formed area
        let typed = crate::opcodes::Opcode::MapTable { area: crate::opcodes::Area { col_pitch: 1, ..wire }, table: vec![1, 2] };
        let bytes = crate::opcodes::write_list(&[typed]);
        let record = &crate::opcode_raw::parse(&bytes, 1).unwrap()[0];
        match parse(record.id, &record.params, c).unwrap() {
            Params::MapTable { area, table } => {
                assert_eq!(area, a);
                assert_eq!(table, [1, 2]);
            }
            other => panic!("MapTable expected, got {other:?}"),
        }
    }
}
