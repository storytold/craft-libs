//! The scalar sRGB pair against IEC 61966-2-1 and against the copies of it in craft-cms,
//! craft-mesh and craft-psd.
//!
//! The encoding is the one the ICC sRGB registry entry gives for IEC 61966-2-1:1999
//! (<https://registry.color.org/rgb-registry/srgb>); the decoding is the CSS Color 4 sample code
//! (<https://www.w3.org/TR/css-color-4/#color-conversion-code>). Outside `[0, 1]` the pair keeps
//! its own rule: negative values stay on the linear segment and values above 1 follow the power
//! segment.

use craft_texture::{linear_to_srgb, srgb_to_linear};

/// The largest difference allowed between the `f32` pair and the `f64` formulas.
const STANDARD_TOLERANCE: f32 = 3.0e-7;
/// The largest difference allowed between the pair and craft-cms's ICC curve, which is evaluated
/// in `f64` and returned as `f32`.
const CMS_TOLERANCE: f32 = 2.4e-7;
/// The 8-bit and 16-bit code ranges.
const DEPTHS: [u32; 2] = [255, 65_535];

/// One direction of an sRGB pair.
type Scalar = fn(f32) -> f32;

fn encode64(c: f64) -> f64 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn decode64(c: f64) -> f64 {
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Every code of a depth with its normalized value.
fn codes(max: u32) -> impl Iterator<Item = (u32, f32)> {
    (0..=max).map(move |n| (n, n as f32 / max as f32))
}

/// The largest difference between `f` and `g` over every code of a depth, and the code it is at.
/// A difference that is not finite fails at its code.
fn worst(max: u32, f: impl Fn(f32) -> f32, g: impl Fn(f32) -> f32) -> (f32, u32) {
    let mut most = (0.0, 0);
    for (n, v) in codes(max) {
        let d = (f(v) - g(v)).abs();
        assert!(d.is_finite(), "code {n} of {max} gives a difference of {d}");
        if d > most.0 {
            most = (d, n);
        }
    }
    most
}

#[test]
#[should_panic(expected = "code 0 of 255 gives a difference of NaN")]
fn worst_fails_on_a_difference_that_is_not_finite() {
    worst(255, |_| f32::NAN, |v| v);
}

/// Inputs outside `[0, 1]` that the crates' copies must also agree on.
const OUTSIDE: [f32; 10] = [-0.0, -1.0e-6, -0.5, -1.0, -100.0, 1.5, 2.0, 100.0, f32::INFINITY, f32::NEG_INFINITY];

#[test]
fn encode_matches_the_standard_at_every_8_and_16_bit_code() {
    for max in DEPTHS {
        let (d, n) = worst(max, linear_to_srgb, |v| encode64(f64::from(v)) as f32);
        assert!(d <= STANDARD_TOLERANCE, "code {n} of {max} is {d} from the standard");
    }
}

#[test]
fn decode_matches_the_standard_at_every_8_and_16_bit_code() {
    for max in DEPTHS {
        let (d, n) = worst(max, srgb_to_linear, |v| decode64(f64::from(v)) as f32);
        assert!(d <= STANDARD_TOLERANCE, "code {n} of {max} is {d} from the standard");
    }
}

#[test]
fn every_encoded_code_round_trips() {
    for max in DEPTHS {
        for (n, v) in codes(max) {
            let back = (linear_to_srgb(srgb_to_linear(v)) * max as f32).round() as u32;
            assert_eq!(back, n, "code {n} of {max}");
        }
    }
}

#[test]
fn every_linear_code_round_trips_within_rounding() {
    for max in DEPTHS {
        let (d, n) = worst(max, |v| srgb_to_linear(linear_to_srgb(v)), |v| v);
        assert!(d <= STANDARD_TOLERANCE, "code {n} of {max} comes back {d} away");
    }
}

#[test]
fn outputs_do_not_decrease_over_the_code_grids() {
    for max in DEPTHS {
        for f in [linear_to_srgb, srgb_to_linear] {
            let out: Vec<f32> = codes(max).map(|(_, v)| f(v)).collect();
            let fall = out.windows(2).position(|w| w[1] < w[0]);
            assert_eq!(fall, None, "the output falls after code {fall:?} of {max}");
        }
    }
}

#[test]
fn encode_changes_segment_after_0_0031308() {
    let t = 0.003_130_8_f32;
    for v in [t.next_down(), t] {
        assert_eq!(linear_to_srgb(v), v * 12.92, "{v} is on the linear segment");
    }
    let above = t.next_up();
    assert_eq!(linear_to_srgb(above), 1.055 * above.powf(1.0 / 2.4) - 0.055);
    for v in [t.next_down(), t, above] {
        assert!((linear_to_srgb(v) - encode64(f64::from(v)) as f32).abs() <= STANDARD_TOLERANCE, "{v}");
    }
}

#[test]
fn decode_changes_segment_after_0_04045() {
    let t = 0.040_45_f32;
    for v in [t.next_down(), t] {
        assert_eq!(srgb_to_linear(v), v / 12.92, "{v} is on the linear segment");
    }
    let above = t.next_up();
    assert_eq!(srgb_to_linear(above), ((above + 0.055) / 1.055).powf(2.4));
    for v in [t.next_down(), t, above] {
        assert!((srgb_to_linear(v) - decode64(f64::from(v)) as f32).abs() <= STANDARD_TOLERANCE, "{v}");
    }
}

#[test]
fn zero_and_one_map_to_themselves() {
    for f in [linear_to_srgb, srgb_to_linear] {
        assert_eq!(f(0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(f(-0.0).to_bits(), (-0.0_f32).to_bits());
        assert!((f(1.0) - 1.0).abs() <= f32::EPSILON);
    }
}

#[test]
fn values_above_one_follow_the_power_segment() {
    for v in [1.000_001_f32, 1.5, 2.0, 10.0] {
        let (e, d) = (encode64(f64::from(v)), decode64(f64::from(v)));
        assert!((f64::from(linear_to_srgb(v)) - e).abs() <= e * 1.0e-6, "encode {v}");
        assert!((f64::from(srgb_to_linear(v)) - d).abs() <= d * 1.0e-6, "decode {v}");
    }
    assert!((linear_to_srgb(2.0) - 1.353_256).abs() < 1.0e-6);
    assert!((srgb_to_linear(2.0) - 4.953_846).abs() < 1.0e-5);
}

#[test]
fn negative_values_stay_on_the_linear_segment() {
    for v in [-f32::MIN_POSITIVE, -1.0e-6, -0.003_130_8, -0.040_45, -0.5, -1.0, -100.0] {
        assert_eq!(linear_to_srgb(v), v * 12.92, "encode {v}");
        assert_eq!(srgb_to_linear(v), v / 12.92, "decode {v}");
    }
    assert!((linear_to_srgb(-1.0) + 12.92).abs() < 1.0e-5);
    assert!((srgb_to_linear(-1.0) + 0.077_399_38).abs() < 1.0e-7);
}

#[test]
fn infinities_keep_their_sign_and_nan_stays_nan() {
    for f in [linear_to_srgb, srgb_to_linear] {
        assert_eq!(f(f32::INFINITY), f32::INFINITY);
        assert_eq!(f(f32::NEG_INFINITY), f32::NEG_INFINITY);
        assert!(f(f32::NAN).is_nan());
    }
}

#[test]
fn mesh_pair_matches_bit_for_bit() {
    let pairs: [(Scalar, Scalar); 2] = [(linear_to_srgb, craft_mesh::linear_to_srgb), (srgb_to_linear, craft_mesh::srgb_to_linear)];
    for (ours, mesh) in pairs {
        for max in DEPTHS {
            for (n, v) in codes(max) {
                assert_eq!(ours(v).to_bits(), mesh(v).to_bits(), "code {n} of {max}");
            }
        }
        for v in OUTSIDE {
            assert_eq!(ours(v).to_bits(), mesh(v).to_bits(), "{v}");
        }
        assert!(mesh(f32::NAN).is_nan());
    }
}

#[test]
fn psd_encoder_matches_bit_for_bit() {
    for max in DEPTHS {
        for (n, v) in codes(max) {
            assert_eq!(linear_to_srgb(v).to_bits(), craft_psd::float::srgb_encode(v).to_bits(), "code {n} of {max}");
        }
    }
    for v in OUTSIDE {
        assert_eq!(linear_to_srgb(v).to_bits(), craft_psd::float::srgb_encode(v).to_bits(), "{v}");
    }
    assert!(craft_psd::float::srgb_encode(f32::NAN).is_nan());
}

#[test]
fn cms_curve_matches_within_rounding() {
    let trc = craft_cms::curve::srgb_trc();
    for max in DEPTHS {
        let (d, n) = worst(max, srgb_to_linear, |v| trc.eval(v));
        assert!(d <= CMS_TOLERANCE, "decode code {n} of {max} is {d} from craft-cms");
        let (d, n) = worst(max, linear_to_srgb, |v| trc.eval_inverse(v));
        assert!(d <= CMS_TOLERANCE, "encode code {n} of {max} is {d} from craft-cms");
    }
}
