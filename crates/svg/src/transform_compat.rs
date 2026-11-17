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
