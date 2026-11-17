//! Prefix-preserving transform-list compatibility for existing SVG importers.
//! At most six numbers are retained per operation; input scanning is not byte-capped.
//! Unknown/invalid-arity operations are ignored, unterminated suffixes stop parsing.
//! Matrix overflow behavior is unchanged; callers retain their geometry validation.

use crate::prefix_path as pathdata;
use kurbo::Affine;

/// Parse transforms using the established EffectCraft recovery and composition order.
pub fn parse_transform(s: &str) -> Affine {
    let mut m = Affine::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let (a, too_many) = pathdata::numbers_bounded(&rest[open + 1..open + close], 6);
        rest = &rest[open + close + 1..];
        let t = if too_many {
            Affine::IDENTITY
        } else {
            match (name, a.as_slice()) {
                ("matrix", [a0, a1, a2, a3, a4, a5]) => Affine::new([*a0, *a1, *a2, *a3, *a4, *a5]),
                ("translate", [x]) => Affine::translate((*x, 0.0)),
                ("translate", [x, y]) => Affine::translate((*x, *y)),
                ("scale", [scale]) => Affine::scale(*scale),
                ("scale", [x, y]) => Affine::scale_non_uniform(*x, *y),
                ("rotate", [angle]) => Affine::rotate(angle.to_radians()),
                ("rotate", [angle, x, y]) => Affine::translate((*x, *y)) * Affine::rotate(angle.to_radians()) * Affine::translate((-*x, -*y)),
                ("skewX", [angle]) => Affine::new([1.0, 0.0, angle.to_radians().tan(), 1.0, 0.0, 0.0]),
                ("skewY", [angle]) => Affine::new([1.0, angle.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
                _ => Affine::IDENTITY,
            }
        };
        m *= t;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_keep_composition_order() {
        let cases = [
            ("matrix(1 2 3 4 5 6)", Affine::new([1., 2., 3., 4., 5., 6.])),
            ("translate(2)", Affine::translate((2., 0.))),
            ("translate(2,3)", Affine::translate((2., 3.))),
            ("scale(2)", Affine::scale(2.)),
            ("scale(2 3)", Affine::scale_non_uniform(2., 3.)),
            ("rotate(30)", Affine::rotate(30f64.to_radians())),
            ("rotate(30 2 3)", Affine::translate((2., 3.)) * Affine::rotate(30f64.to_radians()) * Affine::translate((-2., -3.))),
            ("skewX(30)", Affine::new([1., 0., 30f64.to_radians().tan(), 1., 0., 0.])),
            ("skewY(30)", Affine::new([1., 30f64.to_radians().tan(), 0., 1., 0., 0.])),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_transform(text).as_coeffs(), expected.as_coeffs());
            assert_eq!(parse_transform(&format!("translate(7,9), {text}")).as_coeffs(), (Affine::translate((7., 9.)) * expected).as_coeffs());
        }
    }

    #[test]
    fn malformed_and_oversized_operations_keep_accepted_prefix() {
        for text in ["", "unknown(1)", "translate()", "scale(1 2 3)", "matrix(1 2 3 4 5 6 7)", "rotate(2 3)"] {
            assert_eq!(parse_transform(text), Affine::IDENTITY);
        }
        for suffix in ["translate(", "unknown(1)", "matrix(1 2 3 4 5 6 7)"] {
            assert_eq!(parse_transform(&format!("translate(3,4){suffix}")), Affine::translate((3., 4.)));
        }
        assert_eq!(parse_transform("unknown(1) scale(2)"), Affine::scale(2.));
        assert_eq!(parse_transform("translate(3 4oops)"), Affine::translate((3., 4.)));
        let huge = format!("scale({}) translate(2)", "1 ".repeat(10_000));
        assert_eq!(parse_transform(&huge), Affine::translate((2., 0.)));
    }
}
