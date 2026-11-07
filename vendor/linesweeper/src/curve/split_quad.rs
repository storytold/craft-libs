#![allow(missing_docs)]
#![allow(dead_code)]

use arrayvec::ArrayVec;

use super::Quadratic;

pub struct SplitQuadPart {
    quad: Quadratic,
    until: f64,
}

pub struct SplitQuad {
    quads: ArrayVec<SplitQuadPart, 3>,
}

impl SplitQuad {
    pub fn single(q: Quadratic) -> Self {
        Self {
            quads: [SplitQuadPart {
                quad: q,
                until: f64::INFINITY,
            }]
            .into_iter()
            .collect(),
        }
    }
}

pub struct QuadBound {
    upper: SplitQuad,
    lower: SplitQuad,
}

impl QuadBound {
    pub fn new(mut q: Quadratic, upper: f64, lower: f64, shift: f64) -> Self {
        debug_assert!(lower <= 0.0 && 0.0 <= upper);
        debug_assert!(shift >= 0.0);
        let center = -q.c1 / (2.0 * q.c0);
        let extremum = q.c0 + q.c1 * center;
        if center.is_infinite() || extremum.is_infinite() {
            // Our quad is close enough to linear.
            q.c2 = 0.0;
            let upper = q + upper;
            let lower = q + lower;

            let signed_shift = if q.c1 > 0.0 { shift } else { -shift };

            Self {
                upper: SplitQuad::single(upper.shift(-signed_shift)),
                lower: SplitQuad::single(lower.shift(signed_shift)),
            }
        } else {
            todo!()
        }
    }
}
