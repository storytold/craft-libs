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

/// Convert a CSS `transform` declaration value into SVG `transform` attribute syntax.
///
/// Rewrites CSS units (`px`, `deg`, `rad`, `grad`, `turn`) and CSS-only functions (`translateX`,
/// `translateY`, `scaleX`, `scaleY`, `skew`, `none`) into SVG syntax. Returns `None` when the
/// value already uses SVG syntax or cannot be expressed (e.g. percentages, `em`, 3D functions).
pub fn css_transform_to_svg(value: &str) -> Option<String> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some("matrix(1 0 0 1 0 0)".into());
    }
    let mut out = Vec::new();
    let mut rest = v;
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        let name = rest.get(..open)?.trim().to_ascii_lowercase();
        let args: Vec<&str> = rest.get(open + 1..close)?.split(|c: char| c == ',' || c.is_whitespace()).filter(|a| !a.is_empty()).collect();
        let len = |i: usize| args.get(i).and_then(|a| css_length(a));
        let ang = |i: usize| args.get(i).and_then(|a| css_angle(a));
        let num = |i: usize| args.get(i).and_then(|a| a.parse::<f64>().ok()).filter(|n| n.is_finite());
        let f = match (name.as_str(), args.len()) {
            ("matrix", 6) => format!("matrix({} {} {} {} {} {})", num(0)?, num(1)?, num(2)?, num(3)?, len(4)?, len(5)?),
            ("translate", 1) => format!("translate({} 0)", len(0)?),
            ("translate", 2) => format!("translate({} {})", len(0)?, len(1)?),
            ("translatex", 1) => format!("translate({} 0)", len(0)?),
            ("translatey", 1) => format!("translate(0 {})", len(0)?),
            ("scale", 1) => format!("scale({})", num(0)?),
            ("scale", 2) => format!("scale({} {})", num(0)?, num(1)?),
            ("scalex", 1) => format!("scale({} 1)", num(0)?),
            ("scaley", 1) => format!("scale(1 {})", num(0)?),
            ("rotate", 1) => format!("rotate({})", ang(0)?),
            ("skewx", 1) => format!("skewX({})", ang(0)?),
            ("skewy", 1) => format!("skewY({})", ang(0)?),
            ("skew", 1) => format!("skewX({})", ang(0)?),
            ("skew", 2) => format!("matrix(1 {} {} 1 0 0)", ang(1)?.to_radians().tan(), ang(0)?.to_radians().tan()),
            _ => return None,
        };
        out.push(f);
        rest = rest.get(close + 1..)?;
    }
    let svg = out.join(" ");
    (!out.is_empty() && svg != v).then_some(svg)
}

/// A CSS transform length in user units: `px` or unitless.
fn css_length(a: &str) -> Option<f64> {
    let a = a.to_ascii_lowercase();
    let n = a.strip_suffix("px").unwrap_or(&a).parse::<f64>().ok()?;
    n.is_finite().then_some(n)
}

/// A CSS angle in degrees.
fn css_angle(a: &str) -> Option<f64> {
    let a = a.to_ascii_lowercase();
    let (n, k) = if let Some(n) = a.strip_suffix("deg") {
        (n, 1.0)
    } else if let Some(n) = a.strip_suffix("grad") {
        (n, 0.9)
    } else if let Some(n) = a.strip_suffix("rad") {
        (n, 180.0 / std::f64::consts::PI)
    } else if let Some(n) = a.strip_suffix("turn") {
        (n, 360.0)
    } else {
        (a.as_str(), 1.0)
    };
    let d = n.parse::<f64>().ok()? * k;
    d.is_finite().then_some(d)
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

    #[test]
    fn css_transforms_convert_to_svg_syntax() {
        assert_eq!(css_transform_to_svg("translate(100px, 100px)").as_deref(), Some("translate(100 100)"));
        assert_eq!(css_transform_to_svg(" translateX(5px) rotate(0.25turn) ").as_deref(), Some("translate(5 0) rotate(90)"));
        assert_eq!(css_transform_to_svg("rotate(1.5707963267948966rad) scaleY(2)").as_deref(), Some("rotate(90) scale(1 2)"));
        assert_eq!(css_transform_to_svg("skew(45deg)").as_deref(), Some("skewX(45)"));
        assert_eq!(css_transform_to_svg("matrix(1, 0, 0, 1, 10px, 20px)").as_deref(), Some("matrix(1 0 0 1 10 20)"));
        assert_eq!(css_transform_to_svg("none").as_deref(), Some("matrix(1 0 0 1 0 0)"));
        assert_eq!(css_transform_to_svg("translate(10 20)"), None);
        assert_eq!(css_transform_to_svg("translate(50%, 0)"), None);
        assert_eq!(css_transform_to_svg("rotate3d(1, 0, 0, 45deg)"), None);
        assert_eq!(css_transform_to_svg("translate(1px"), None);
        // CSS Values3 section4.4: unit identifiers are ASCII case-insensitive.
        // https://www.w3.org/TR/css-values-3/#dimensions
        for unit in ["px", "PX", "Px", "pX"] {
            let input = format!("translateX(5{unit}) scale(2)");
            assert_eq!(
                css_transform_to_svg(&input).map(|svg| parse_transform(&svg) * kurbo::Point::new(1.0, 2.0)),
                Some(kurbo::Point::new(7.0, 4.0)),
                "{input}",
            );
        }
    }
}
