//! Office Math (OMML) written with `craft_math::omml`.

use craft_wordmodel::math::{Math, MathProps};
use craft_wordmodel::props::Rgb;

use crate::xml::W;

/// Write an equation: `m:oMath`, inside `m:oMathPara` for a display equation. The source OMML
/// is written back unchanged when the equation came from a file. `color` (the equation's run
/// colour) goes on every math run without its own: OMML has no equation-level colour.
pub fn write_equation(w: &mut W, linear: &str, display: bool, math: &Math, color: Option<Rgb>) {
    w.raw(&craft_math::omml::write_str(linear, display, math, color));
}

/// `math` with its `#(n)` equation numbers resolved from `counter`, when it has any.
pub fn resolve_numbers(math: &Math, display: bool, counter: &mut u32) -> Option<Math> {
    craft_math::omml::resolve_numbers(math, display, counter)
}

/// `m:mathPr`.
pub fn math_pr(w: &mut W, m: &MathProps) {
    let mut o = craft_math::omml::W::default();
    craft_math::omml::math_pr(&mut o, m);
    w.raw(&o.s);
}
