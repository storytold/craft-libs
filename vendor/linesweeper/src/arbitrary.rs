//! Utilities for fuzz and/or property testing using `arbitrary`.

use arbitrary::Unstructured;
use kurbo::{CubicBez, Point};
use polycool::Quadratic;

use crate::geom;

/// Generate an arbitrary float in some range.
pub fn float_in_range(
    start: f64,
    end: f64,
    u: &mut Unstructured<'_>,
) -> Result<f64, arbitrary::Error> {
    if start >= end {
        return Err(arbitrary::Error::EmptyChoose);
    }
    let num: u32 = u.arbitrary()?;
    let t = num as f64 / u32::MAX as f64;
    // Craft patch (wide float range): `end - start` overflows for a range wider
    // than `f64::MAX`, so the value is interpolated from each end.
    Ok((start * (1.0 - t) + end * t).clamp(start, end))
}

fn float(u: &mut Unstructured<'_>) -> Result<f64, arbitrary::Error> {
    float_in_range(-1e6, 1e6, u)
}

fn float_at_least(min: f64, u: &mut Unstructured<'_>) -> Result<f64, arbitrary::Error> {
    float_in_range(min, 1e6, u)
}

fn float_at_most(max: f64, u: &mut Unstructured<'_>) -> Result<f64, arbitrary::Error> {
    float_in_range(-1e6, max, u)
}

/// Generate a float in some range, but give it a chance to be close to another float.
fn another_float_in_range(
    orig: f64,
    start: f64,
    end: f64,
    u: &mut Unstructured<'_>,
) -> Result<f64, arbitrary::Error> {
    let close: bool = u.arbitrary()?;
    if close {
        let ulps: i32 = u.int_in_range(-32..=32)?;
        let scale = 1.0f64 + ulps as f64 * f64::EPSILON;
        Ok((orig * scale).clamp(start, end))
    } else {
        float_in_range(start, end, u)
    }
}

fn point(u: &mut Unstructured<'_>) -> Result<Point, arbitrary::Error> {
    Ok(Point::new(float(u)?, float(u)?))
}

fn point_at_least(min: f64, u: &mut Unstructured<'_>) -> Result<Point, arbitrary::Error> {
    Ok(Point::new(float(u)?, float_at_least(min, u)?))
}

fn point_at_most(max: f64, u: &mut Unstructured<'_>) -> Result<Point, arbitrary::Error> {
    Ok(Point::new(float(u)?, float_at_most(max, u)?))
}

/// Generate an arbitrary cubic Bezier, guaranteed to be monotonically increasing in y.
pub fn monotonic_bezier(u: &mut Unstructured<'_>) -> Result<CubicBez, arbitrary::Error> {
    let p0 = point(u)?;
    let p3 = point(u)?;
    let (p0, mut p3) = if p0.y < p3.y { (p0, p3) } else { (p3, p0) };
    if p3.y == p0.y {
        p3.y += 1.0;
    }

    let p1 = point_at_least(p0.y, u)?;
    let p2 = point_at_most(p3.y, u)?;

    let ret = CubicBez::new(p0, p1, p2, p3);
    Ok(geom::monotonic_cubic_pieces(ret)
        .into_iter()
        .find(|x| x.piece.end().y > x.piece.start().y)
        .unwrap()
        .piece
        .to_kurbo_cubic())
}

/// Generate an arbitrary cubic Bezier, guaranteed to be monotonically increasing in y.
///
/// This generated Bezier has a chance to be "close" to `first`, for example by starting
/// at the same point or with the same tangent.
///
/// Returns [`arbitrary::Error::IncorrectFormat`] when the generated control points
/// have no piece that increases in y.
pub fn another_monotonic_bezier(
    u: &mut Unstructured<'_>,
    first: &CubicBez,
) -> Result<CubicBez, arbitrary::Error> {
    let same_start: bool = u.arbitrary()?;
    let same_start_tangent: bool = same_start && u.arbitrary()?;

    let p0 = if same_start { first.p0 } else { point(u)? };

    let p1 = if same_start_tangent {
        first.p1
    } else {
        point_at_least(p0.y, u)?
    };

    let p3 = point_at_least(p0.y.max(first.p0.y), u)?;
    let p2 = point_at_most(p3.y, u)?;

    let ret = CubicBez::new(p0, p1, p2, p3);
    // Craft patch (flat cubic): when every control point has the same y there is
    // no increasing piece, and returning `ret` gave a horizontal cubic.
    geom::monotonic_cubic_pieces(ret)
        .into_iter()
        .find(|x| x.piece.end().y > x.piece.start().y)
        .map(|c| c.piece.to_kurbo_cubic())
        .ok_or(arbitrary::Error::IncorrectFormat)
}

/// Generate an arbitrary quadratic function, with coefficients of roughly the scale `size`.
pub fn quadratic(size: f64, u: &mut Unstructured<'_>) -> Result<Quadratic, arbitrary::Error> {
    let use_coeffs: bool = u.arbitrary()?;
    if use_coeffs {
        let c2 = float_in_range(-size, size, u)?;
        let c1 = another_float_in_range(c2, -size, size, u)?;
        let c0 = another_float_in_range(c1, -size, size, u)?;

        Ok(Quadratic::new([c0, c1, c2]))
    } else {
        // Generate the roots, with a bias towards an almost-repeated root.
        let size = size.sqrt();

        let r1 = float_in_range(-size, size, u)?;
        let r2 = another_float_in_range(r1, -size, size, u)?;
        let scale = float_in_range(-size, size, u)?;

        Ok(Quadratic::new([scale * r1 * r2, -scale * (r1 + r2), scale]))
    }
}

// Craft patch (wide float range).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_in_range_handles_a_range_wider_than_f64_max() {
        let (start, end) = (-1e308, 1e308);
        for bytes in [[0u8; 4], [0x80, 0, 0, 0], [0, 0, 0, 0x80], [0xff; 4]] {
            let x = float_in_range(start, end, &mut Unstructured::new(&bytes)).unwrap();
            assert!(
                x.is_finite() && (start..=end).contains(&x),
                "{x} for {bytes:?}"
            );
        }
        assert_eq!(
            float_in_range(start, end, &mut Unstructured::new(&[0u8; 4])).unwrap(),
            start
        );
        assert_eq!(
            float_in_range(start, end, &mut Unstructured::new(&[0xff; 4])).unwrap(),
            end
        );
    }
}

// Craft patch (flat cubic).
#[cfg(test)]
mod flat_cubic_tests {
    use super::*;

    #[test]
    fn another_monotonic_bezier_increases_in_y_or_fails() {
        // `first` starts with a horizontal tangent.
        let first = CubicBez::new((0.0, 0.0), (1.0, 0.0), (2.0, 1.0), (3.0, 1.0));
        // The same start and tangent as `first`, `p3.y` at its minimum of 0 and
        // `p2.y` at its maximum of 0: every control point has y = 0.
        let flat = [
            1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
        ];
        assert_eq!(
            another_monotonic_bezier(&mut Unstructured::new(&flat), &first),
            Err(arbitrary::Error::IncorrectFormat)
        );
        let bytes: Vec<u8> = (0..=255u8)
            .cycle()
            .take(4096)
            .map(|b| b.wrapping_mul(37))
            .collect();
        let mut u = Unstructured::new(&bytes);
        while !u.is_empty() {
            if let Ok(c) = another_monotonic_bezier(&mut u, &first) {
                assert!(c.p3.y > c.p0.y, "{c:?}");
            }
        }
    }
}
