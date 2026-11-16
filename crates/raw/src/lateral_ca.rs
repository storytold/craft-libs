//! Lateral chromatic aberration of a Bayer mosaic as a radial map, to be corrected before demosaic.
//!
//! A lens with lateral CA images red and blue at slightly different magnifications than green: where green's image
//! of a point sits at sensor radius ρ about the optical centre, red's sits at ρ·(1 + red(ρ)) and blue's at
//! ρ·(1 + blue(ρ)). [`LateralCa`] holds that map (a knot table per colour). It is data: the develop pipeline
//! corrects it where it lives, on the mosaic (each red and blue site resampled from its own colour's sites, so red
//! and blue line up with green before any demosaic). Corrected after demosaic it overshoots, because the
//! colour-difference demosaics borrow green's edges for red and blue, so the picture shows only part of the
//! mosaic's CA.
//!
//! The map rides in [`crate::OpcodeLists::lateral_ca`] next to the DNG opcode lists. It is not a DNG opcode and the
//! DNG writer never writes it. The Sony decoder fills it from the ILCE-7RM6's CA table (`arw_lens_rm6`).

use serde::{Deserialize, Serialize};

/// Largest |relative radial scale| a map may hold: 15 px at a 7500 px radius. (The ILCE-7RM6's tables reach
/// 1724 / 3.36·10⁶ ≈ 5.1·10⁻⁴ over the 54 measured photos.)
pub const MAX_SCALE: f64 = 2e-3;
/// Most knots per colour.
pub const MAX_KNOTS: usize = 64;

/// Lateral chromatic aberration of a Bayer mosaic: where green's image of a point sits at sensor radius ρ about
/// `center`, red's sits at ρ·(1 + red(ρ)) and blue's at ρ·(1 + blue(ρ)). Corrected on the mosaic before demosaic
/// (after `OpcodeList2`); not a DNG opcode, never written to a DNG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LateralCa {
    /// Active-area pixel-index coordinates (the convention of `WarpRectilinear`'s centre × (size − 1)).
    pub center: [f64; 2],
    /// Knot i sits at radius i · spacing px; linear in between, the last segment extended.
    pub spacing: f64,
    /// Relative radial scale of red / blue at each knot (Sony: table value / 3.36·10⁶).
    pub red: Vec<f64>,
    pub blue: Vec<f64>,
}

impl LateralCa {
    /// `None` unless the map is valid ([`Self::is_valid`]).
    pub fn new(center: [f64; 2], spacing: f64, red: Vec<f64>, blue: Vec<f64>) -> Option<LateralCa> {
        let ca = LateralCa { center, spacing, red, blue };
        ca.is_valid().then_some(ca)
    }

    /// The centre is finite, `spacing` finite and ≥ 1, each colour has 2 to [`MAX_KNOTS`] knots all within
    /// ±[`MAX_SCALE`] (NaN fails), and not every knot is zero. A map that bypassed [`Self::new`] (deserialised,
    /// or built field by field) has to be checked again before it is applied.
    pub fn is_valid(&self) -> bool {
        // a NaN fails the comparison
        let knots = |k: &[f64]| (2..=MAX_KNOTS).contains(&k.len()) && k.iter().all(|v| v.abs() <= MAX_SCALE);
        self.center.iter().all(|c| c.is_finite())
            && self.spacing.is_finite()
            && self.spacing >= 1.0
            && knots(&self.red)
            && knots(&self.blue)
            && self.red.iter().chain(&self.blue).any(|&v| v != 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a case is, then the centre, spacing, red and blue of [`LateralCa::new`].
    type Case = (&'static str, [f64; 2], f64, Vec<f64>, Vec<f64>);

    #[test]
    fn hostile_tables_are_refused() {
        let (ok, c) = (vec![1e-4, 2e-4], [10.0, 10.0]);
        let cases: Vec<Case> = vec![
            ("NaN knot", c, 400.0, vec![f64::NAN, 1e-4], ok.clone()),
            ("inf knot", c, 400.0, ok.clone(), vec![f64::INFINITY, 1e-4]),
            ("-inf knot", c, 400.0, vec![f64::NEG_INFINITY, 1e-4], ok.clone()),
            ("1e300", c, 400.0, vec![1e300, 1e-4], ok.clone()),
            ("beyond the bound", c, 400.0, vec![MAX_SCALE * 1.001, 0.0], ok.clone()),
            ("no knots", c, 400.0, vec![], ok.clone()),
            ("one knot", c, 400.0, vec![1e-4], ok.clone()),
            ("65 knots", c, 400.0, vec![1e-4; 65], ok.clone()),
            ("10 000 knots", c, 400.0, ok.clone(), vec![1e-4; 10_000]),
            ("spacing 0", c, 0.0, ok.clone(), ok.clone()),
            ("spacing 0.5", c, 0.5, ok.clone(), ok.clone()),
            ("negative spacing", c, -400.0, ok.clone(), ok.clone()),
            ("NaN spacing", c, f64::NAN, ok.clone(), ok.clone()),
            ("inf spacing", c, f64::INFINITY, ok.clone(), ok.clone()),
            ("NaN centre", [f64::NAN, 0.0], 400.0, ok.clone(), ok.clone()),
            ("inf centre", [0.0, f64::INFINITY], 400.0, ok.clone(), ok.clone()),
            ("all zero", c, 400.0, vec![0.0; 2], vec![0.0; 2]),
        ];
        for (what, c, s, r, b) in cases {
            assert!(LateralCa::new(c, s, r, b).is_none(), "{what}");
        }
        assert!(LateralCa::new(c, 1.0, vec![MAX_SCALE, -MAX_SCALE], vec![0.0, 0.0]).is_some(), "at the bound");
        assert!(LateralCa::new(c, 1.0, vec![1e-4; MAX_KNOTS], vec![1e-4; 2]).is_some(), "the most knots");
        // a map built field by field (or deserialised) never went through `new`: `is_valid` is the same check
        let valid = LateralCa::new([47.5, 31.5], 8.0, vec![MAX_SCALE; 2], vec![-MAX_SCALE; 2]).unwrap();
        assert!(valid.is_valid());
        for bad in [
            LateralCa { red: vec![f64::NAN; 2], ..valid.clone() },
            LateralCa { red: vec![], ..valid.clone() },
            LateralCa { spacing: 0.0, ..valid.clone() },
            LateralCa { center: [f64::NAN, 0.0], ..valid.clone() },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }

    /// A serialised map reads back unchanged, and lists written before the field existed read as no map.
    #[test]
    fn opcode_lists_without_the_field_read_as_no_map() {
        let ca = LateralCa::new([47.5, 31.5], 8.0, vec![1e-4, 2e-4], vec![-1e-4, 0.0]).unwrap();
        let lists = crate::OpcodeLists { lateral_ca: Some(ca), ..Default::default() };
        let json = serde_json::to_string(&lists).unwrap();
        assert_eq!(serde_json::from_str::<crate::OpcodeLists>(&json).unwrap(), lists);
        let old: crate::OpcodeLists = serde_json::from_str(r#"{"list1":[],"list2":[],"list3":[]}"#).unwrap();
        assert_eq!(old.lateral_ca, None);
    }
}
