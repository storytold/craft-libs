#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[macro_use]
mod typed_vec;

#[cfg(any(test, feature = "arbitrary"))]
pub mod arbitrary;
pub mod curve;
mod geom;
mod num;
pub mod order;
mod position;
mod segments;
pub mod sweep;
pub mod topology;

#[cfg(feature = "generators")]
pub mod generators;

pub use geom::{Point, Segment};
use kurbo::Shape;
pub use segments::{SegIdx, Segments};

// pub so that we can use it in fuzz tests, but it's really private
#[doc(hidden)]
pub mod treevec;

use topology::{BinaryWindingNumber, Topology};

use crate::segments::NonClosedPath;

#[cfg(test)]
pub mod perturbation;

/// A fill rule tells us how to decide whether a point is "inside" a polyline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FillRule {
    /// The point is "inside" if its winding number is odd.
    EvenOdd,
    /// The point is "inside" if its winding number is non-zero.
    NonZero,
}

impl FillRule {
    /// According to this fill rule, is the winding number inside the shape?
    pub fn is_inside(self, winding: i32) -> bool {
        match self {
            FillRule::EvenOdd => winding % 2 != 0,
            FillRule::NonZero => winding != 0,
        }
    }
}

/// Binary operations between sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum BinaryOp {
    /// A point is in the union of two sets if it is in either one.
    Union,
    /// A point is in the intersection of two sets if it is in both.
    Intersection,
    /// A point is in the difference of two sets if it is in the first but not the second.
    Difference,
    /// A point is in the exclusive-or of two sets if it is in one or the other, but not both.
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// The input points were faulty.
pub enum Error {
    /// At least one of the inputs was infinite.
    Infinity,
    /// At least one of the inputs was not a number.
    NaN,
    /// One of the inputs had a non-closed path.
    NonClosedPath(NonClosedPath),
    /// The accuracy parameter was NaN, infinite, zero or negative.
    InvalidEps,
}

impl From<NonClosedPath> for Error {
    fn from(ncp: NonClosedPath) -> Self {
        Error::NonClosedPath(ncp)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Infinity => write!(f, "one of the inputs was infinite"),
            Error::NaN => write!(f, "one of the inputs had a NaN"),
            Error::NonClosedPath(_) => write!(f, "one of the inputs had a non-closed path"),
            Error::InvalidEps => {
                write!(f, "the accuracy parameter was not a positive finite number")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Computes a boolean operation between two sets, each of which is described as
/// a collection of closed curves.
///
/// This function calls [`binary_op_with_eps`] with an automatically-chosen
/// accuracy parameter. The automatically-chosen accuracy is fairly
/// conservative, so you may get better performance by choosing your own.
pub fn binary_op(
    set_a: &kurbo::BezPath,
    set_b: &kurbo::BezPath,
    fill_rule: FillRule,
    op: BinaryOp,
) -> Result<topology::Contours, Error> {
    // Find the extremal values, to figure out how much precision we can support.
    let bbox = set_a.bounding_box().union(set_b.bounding_box());
    let min = bbox.min_x().min(bbox.min_y());
    let max = bbox.max_x().max(bbox.max_y());
    if min.is_infinite() || max.is_infinite() {
        return Err(Error::Infinity);
    }
    // If there was any NaN in the input, it should have propagated to the min and max.
    if min.is_nan() || max.is_nan() {
        return Err(Error::NaN);
    }

    // TODO: we did some analysis for error bounds in the case of polylines.
    // Think more about what makes sense for curves.
    let m_2 = min.abs().max(max.abs());
    let eps = m_2 * (f64::EPSILON * 64.0);
    let eps = eps.max(1e-6);

    debug_assert!(eps.is_finite());
    binary_op_with_eps(set_a, set_b, fill_rule, op, eps)
}

/// Computes a boolean operation between two sets, each of which is described as
/// a collection of closed curves.
///
/// The output paths will not exactly agree with the input paths in general.
/// The accuracy parameter `eps` bounds how much they can differ.
///
/// Returns [`Error::InvalidEps`] when `eps` is NaN, infinite, zero or
/// negative. An `eps` below `64 * f64::EPSILON` times the largest coordinate
/// magnitude is raised to that value.
pub fn binary_op_with_eps(
    set_a: &kurbo::BezPath,
    set_b: &kurbo::BezPath,
    fill_rule: FillRule,
    op: BinaryOp,
    eps: f64,
) -> Result<topology::Contours, Error> {
    if !(eps.is_finite() && eps > 0.0) {
        return Err(Error::InvalidEps);
    }
    let top = Topology::from_paths_binary(set_a, set_b, eps)?;
    #[cfg(feature = "debug-svg")]
    {
        svg::save(
            "out.svg",
            &top.dump_svg(|tag| {
                if tag == topology::OneOfTwo::A {
                    "red".to_owned()
                } else {
                    "blue".to_owned()
                }
            }),
        )
        .unwrap();
    }

    let inside = |windings: &BinaryWindingNumber| {
        let inside_one = |winding| match fill_rule {
            FillRule::EvenOdd => winding % 2 != 0,
            FillRule::NonZero => winding != 0,
        };

        match op {
            BinaryOp::Union => inside_one(windings.shape_a) || inside_one(windings.shape_b),
            BinaryOp::Intersection => inside_one(windings.shape_a) && inside_one(windings.shape_b),
            BinaryOp::Xor => inside_one(windings.shape_a) != inside_one(windings.shape_b),
            BinaryOp::Difference => inside_one(windings.shape_a) && !inside_one(windings.shape_b),
        }
    };

    Ok(top.contours(inside))
}

#[cfg(test)]
mod tests {
    use insta::assert_ron_snapshot;
    use kurbo::BezPath;

    use super::*;

    #[test]
    fn two_squares() {
        fn to_bez(mut points: impl Iterator<Item = (f64, f64)>) -> BezPath {
            let p = points.next().unwrap();
            let mut ret = BezPath::default();
            ret.move_to(p);
            for q in points {
                ret.line_to(q);
            }
            ret.line_to(p);
            ret
        }
        let a = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let b = vec![(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
        let output = binary_op(
            &to_bez(a.into_iter()),
            &to_bez(b.into_iter()),
            FillRule::EvenOdd,
            BinaryOp::Intersection,
        )
        .unwrap();

        insta::assert_ron_snapshot!(output);
    }

    // This example has a union of two not-quite-axis-aligned crosses. It
    // suffers from some extra quadratic segments near the intersection points,
    // but it used to be worse.
    #[test]
    fn path_blowup() {
        let path1 = "M-90.03662872314453,-212 L-90.03662872314453,212 L90.03565216064453,212 L90.03565216064453,-212 L-90.03662872314453,-212 Z";
        let path2 = "M211.99964904785156,-90.03582000732422 L-212.00035095214844,-90.03646087646484 L-212.00062561035156,90.03582000732422 L211.99937438964844,90.03646087646484 L211.99964904785156,-90.03582000732422 Z";
        if let Ok(output) = binary_op(
            &BezPath::from_svg(path1).unwrap(),
            &BezPath::from_svg(path2).unwrap(),
            FillRule::NonZero,
            BinaryOp::Union,
        ) {
            let output = output.contours().next().unwrap();
            let path_length = output.path.elements().len();
            // This should ideally be more like 12, but there
            // are some small horizontal segments in the output.
            // It used to be 77, though...
            assert!(path_length <= 16);
        }
    }

    #[test]
    fn test_empty_horizontals() {
        let square = kurbo::Rect::from_origin_size((0.0, 0.0), (10.0, 10.0)).to_path(0.0);
        let line_and_back = {
            let mut path = kurbo::Line::new((-1.0, 0.0), (1.0, 0.0)).to_path(0.0);
            path.close_path();
            path
        };
        let line_and_back_2 = {
            let mut path = kurbo::Line::new((2.0, 1.0), (12.0, 1.0)).to_path(0.0);
            path.close_path();
            path
        };

        Topology::from_path(&line_and_back, 1e-3).unwrap();
        Topology::from_path(&line_and_back_2, 1e-3).unwrap();
        Topology::<i32>::from_paths([(&line_and_back, ()), (&line_and_back_2, ())], 1e-3).unwrap();
        Topology::<i32>::from_paths(
            [(&line_and_back, ()), (&line_and_back_2, ()), (&square, ())],
            1e-3,
        )
        .unwrap();

        let output = binary_op(
            &square,
            &line_and_back_2,
            FillRule::EvenOdd,
            BinaryOp::Difference,
        )
        .unwrap();
        assert_ron_snapshot!(output);
    }

    #[test]
    fn monotonicity_violation() {
        let path = "M-16.252847470703124,8.977497100830078 C-8.126453262439954,8.977497100830078 -3.724613933559983,8.977497100830078 4.401851380682135,8.977497100830078 C4.401851380682135,8.977497100830078 16.252847872314454,8.977497100830075 16.252847872314454,-0.5280563794708262 C16.252847872314454,-8.977497100830078 4.401851380682135,-8.977497100830075 4.401851380682135,-8.977497100830075 L4.401851380682135,-8.977497100830075 C-5.078945812623724,-8.977497100830075 -1.5236468651340243,-8.977497100830075 -11.004562568404802,-8.977497100830075 Z";
        let path = BezPath::from_svg(path).unwrap();
        Topology::from_path(&path, 0.001).unwrap();
    }

    #[test]
    fn empty_paths() {
        let contours = binary_op(
            &BezPath::new(),
            &BezPath::new(),
            FillRule::EvenOdd,
            BinaryOp::Xor,
        )
        .unwrap();
        assert!(contours.contours().next().is_none());
    }

    #[test]
    fn repeated_points() {
        Topology::from_polylines_binary(
            [[
                Point::new(0.0, 0.0),
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(1.0, 1.0),
            ]],
            [[
                Point::new(0.0, 0.0),
                Point::new(0.0, 1.0),
                Point::new(1.0, 1.0),
            ]],
            1e-6,
        );
    }

    #[test]
    fn quadratics_in_quadratics_out() {
        let mut quad_path = BezPath::new();
        quad_path.move_to((0.0, 0.0));
        quad_path.quad_to((1.0, -5.0), (2.0, 0.0));
        quad_path.quad_to((1.0, 5.0), (0.0, 0.0));

        let mut rectangle = BezPath::new();
        rectangle.move_to((-1.0, -1.0));
        rectangle.line_to((-1.0, 1.0));
        rectangle.line_to((3.0, 1.0));
        rectangle.line_to((3.0, -1.0));
        rectangle.line_to((-1.0, -1.0));

        let mut segs = Segments::default();
        segs.add_bez_path(&quad_path).unwrap();

        let contours = binary_op(
            &quad_path,
            &rectangle,
            FillRule::EvenOdd,
            BinaryOp::Intersection,
        )
        .unwrap();
        assert_eq!(contours.contours().count(), 1);
        let contour = contours.contours().next().unwrap();
        assert_eq!(
            contour
                .path
                .elements()
                .iter()
                .filter(|el| matches!(el, &&kurbo::PathEl::QuadTo(..)))
                .count(),
            4
        );
    }

    /// Two unit circles whose centers are one unit apart, at `(offset, offset)`
    /// and `(offset + 1, offset)`.
    fn crossing_circles(offset: f64) -> (BezPath, BezPath) {
        (
            kurbo::Circle::new((offset, offset), 1.0).to_path(1e-9),
            kurbo::Circle::new((offset + 1.0, offset), 1.0).to_path(1e-9),
        )
    }

    /// The single contour's area, measured after moving it back to the origin.
    fn single_contour_area(contours: &topology::Contours, offset: f64) -> f64 {
        let contours: Vec<_> = contours.contours().collect();
        assert_eq!(contours.len(), 1);
        (kurbo::Affine::translate((-offset, -offset)) * contours[0].path.clone())
            .area()
            .abs()
    }

    const UNION_AREA: f64 = 5.054816;

    #[test]
    fn binary_op_with_eps_rejects_invalid_eps() {
        let (a, b) = crossing_circles(0.0);
        // -inf goes last: before the check it made the sweep loop forever.
        for eps in [
            f64::NAN,
            -1.0,
            -1e-6,
            0.0,
            -0.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let result = binary_op_with_eps(&a, &b, FillRule::NonZero, BinaryOp::Union, eps);
            assert_eq!(result.err(), Some(Error::InvalidEps), "eps = {eps}");
        }
    }

    #[test]
    fn topology_raises_unusable_eps() {
        let inside = |w: &BinaryWindingNumber| w.shape_a != 0 || w.shape_b != 0;
        // At 1.5e8 one ulp is about 3e-8, so 1e-10 used to recurse in
        // intersect_cubics_rec until the stack overflowed.
        for (offset, eps) in [
            (0.0, f64::NAN),
            (0.0, -1.0),
            (0.0, 0.0),
            (1.5e8, 1e-10),
            (1.5e8, 0.0),
            (0.0, f64::NEG_INFINITY),
            (0.0, f64::INFINITY),
        ] {
            let (a, b) = crossing_circles(offset);
            for correctness in [
                sweep::Correctness::GoodEnough,
                sweep::Correctness::ReallyCorrect,
            ] {
                let top = Topology::from_paths_with_correctness(
                    [(&a, topology::OneOfTwo::A), (&b, topology::OneOfTwo::B)],
                    eps,
                    correctness,
                )
                .unwrap();
                let area = single_contour_area(&top.contours(inside), offset);
                assert!(
                    (area - UNION_AREA).abs() < 1e-3,
                    "offset {offset}, eps {eps}, {correctness:?}: area {area}"
                );
            }
        }
    }

    #[test]
    fn binary_op_with_eps_raises_eps_below_coordinate_resolution() {
        let (a, b) = crossing_circles(1.5e8);
        let contours =
            binary_op_with_eps(&a, &b, FillRule::NonZero, BinaryOp::Union, 1e-10).unwrap();
        let area = single_contour_area(&contours, 1.5e8);
        assert!((area - UNION_AREA).abs() < 1e-3, "area {area}");
    }
}
