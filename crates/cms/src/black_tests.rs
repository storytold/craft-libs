use super::*;

fn coated() -> &'static Profile {
    Builtin::CoatedCmyk.profile()
}
fn fogra() -> &'static Profile {
    Builtin::CoatedFogra39.profile()
}
fn opts() -> TransformOptions {
    TransformOptions::default()
}

/// L* of a CMYK colour, measured by the prebuilt transform.
fn lightness(t: &Transform, cmyk: [f32; 4]) -> f32 {
    let mut lab = [0.0f32; 3];
    t.eval(&cmyk, &mut lab).unwrap();
    lab[0] * 100.0
}

#[test]
fn the_plain_transform_turns_black_into_four_inks() {
    // What the policy is for: without it, black alone gains chromatic ink across presses.
    let t = Transform::with_options(coated(), fogra(), opts()).unwrap();
    let mut out = [0.0f32; 4];
    t.eval(&[0.0, 0.0, 0.0, 0.5], &mut out).unwrap();
    assert!(out[..3].iter().any(|v| *v > 0.01), "{out:?}");
}

#[test]
fn black_only_stays_black_only_on_every_path() {
    for (src, dst) in [(coated(), fogra()), (fogra(), coated()), (fogra(), fogra())] {
        let bp = BlackPreserving::new(src, dst, opts()).unwrap();
        for k in [0.0f32, 0.1, 0.5, 0.9, 1.0] {
            for fast in [false, true] {
                let mut out = [1.0f32; 4];
                if fast { bp.eval_fast(&[0.0, 0.0, 0.0, k], &mut out) } else { bp.eval(&[0.0, 0.0, 0.0, k], &mut out) }.unwrap();
                assert_eq!(out[..3], [0.0; 3], "k {k} fast {fast}");
                assert_eq!(out[3], bp.map_k(k));
            }
        }
        let src8: Vec<u8> = (0..=255u8).flat_map(|k| [0, 0, 0, k, 7]).collect();
        let mut dst8 = vec![0u8; src8.len()];
        bp.convert_u8(&src8, 5, &mut dst8, 5, true).unwrap();
        for (k, px) in dst8.as_chunks::<5>().0.iter().enumerate() {
            assert_eq!(px[..3], [0; 3], "u8 k {k}");
            assert!((f32::from(px[3]) - bp.map_k(k as f32 / 255.0) * 255.0).abs() <= 0.5, "u8 k {k}: {}", px[3]);
            assert_eq!(px[4], 7, "extra channel copied");
        }
        let src16: Vec<u16> = (0..=64u16).flat_map(|k| [0, 0, 0, k * 1023]).collect();
        let mut dst16 = vec![0u16; src16.len()];
        bp.convert_u16(&src16, 4, &mut dst16, 4, false).unwrap();
        for (px, k) in dst16.as_chunks::<4>().0.iter().zip(src16.as_chunks::<4>().0) {
            assert_eq!(px[..3], [0; 3], "u16 {k:?}");
            assert!((f32::from(px[3]) - bp.map_k(f32::from(k[3]) / 65535.0) * 65535.0).abs() <= 0.5, "u16 {k:?}: {}", px[3]);
        }
        let srcf: Vec<f32> = (0..=10).flat_map(|k| [0.0, 0.0, 0.0, k as f32 / 10.0]).collect();
        let mut dstf = vec![0.0f32; srcf.len()];
        bp.convert_f32(&srcf, 4, &mut dstf, 4, false).unwrap();
        for (px, k) in dstf.as_chunks::<4>().0.iter().zip(srcf.as_chunks::<4>().0) {
            assert_eq!(*px, [0.0, 0.0, 0.0, bp.map_k(k[3])], "f32 {k:?}");
        }
    }
}

#[test]
fn the_destination_black_matches_the_source_lightness() {
    // Measured worst error over 201 K steps, both directions, both intents, BPC on and off:
    // 0.007 L*.
    for intent in [Intent::RelativeColorimetric, Intent::Perceptual] {
        for bpc in [true, false] {
            let o = TransformOptions { intent, bpc, ..opts() };
            let coated_lab = Transform::with_options(coated(), Builtin::LabD50.profile(), o).unwrap();
            let fogra_lab = Transform::with_options(fogra(), Builtin::LabD50.profile(), o).unwrap();
            // Both directions for one setting; the other settings in the forward one.
            let directions = if intent == Intent::RelativeColorimetric && bpc { 2 } else { 1 };
            for (src, dst, src_lab, dst_lab) in
                [(coated(), fogra(), &coated_lab, &fogra_lab), (fogra(), coated(), &fogra_lab, &coated_lab)].into_iter().take(directions)
            {
                let bp = BlackPreserving::new(src, dst, o).unwrap();
                let dst_black = lightness(dst_lab, [0.0, 0.0, 0.0, 1.0]);
                for k in [0.1f32, 0.5, 0.9, 1.0] {
                    let want = lightness(src_lab, [0.0, 0.0, 0.0, k]);
                    if want <= dst_black {
                        assert_eq!(bp.map_k(k), 1.0, "darker than the destination's black clamps to K = 1");
                        continue;
                    }
                    let got = lightness(dst_lab, [0.0, 0.0, 0.0, bp.map_k(k)]);
                    assert!((got - want).abs() < 0.02, "{intent:?} bpc {bpc} k {k}: L* {got} for {want}");
                }
                assert_eq!(bp.map_k(0.0), 0.0, "paper stays paper");
                assert!(bp.map_k(1.0) > 0.95, "full black stays near full black: {}", bp.map_k(1.0));
                assert!(bp.map_k(0.25) < bp.map_k(0.75));
            }
        }
    }
    let same = BlackPreserving::new(fogra(), fogra(), opts()).unwrap();
    for k in [0.0f32, 0.2, 0.6, 1.0] {
        assert!((same.map_k(k) - k).abs() < 1e-3, "{k} -> {}", same.map_k(k));
    }
}

#[test]
fn colours_with_any_chromatic_ink_take_the_ordinary_path() {
    let bp = BlackPreserving::new(coated(), fogra(), opts()).unwrap();
    for input in [[1e-6f32, 0.0, 0.0, 0.5], [0.0, 0.2, 0.0, 0.5], [0.3, 0.3, 0.3, 0.0], [0.0, 0.0, 1.0, 1.0]] {
        let (mut a, mut b) = ([0.0f32; 4], [0.0f32; 4]);
        bp.eval(&input, &mut a).unwrap();
        bp.transform().eval(&input, &mut b).unwrap();
        assert_eq!(a, b, "{input:?}");
    }
    let src = [1u8, 0, 0, 128, 0, 1, 0, 128];
    let (mut a, mut b) = ([0u8; 8], [0u8; 8]);
    bp.convert_u8(&src, 4, &mut a, 4, false).unwrap();
    bp.transform().convert_u8(&src, 4, &mut b, 4, false).unwrap();
    assert_eq!(a, b);
}

#[test]
fn refusals_and_hostile_values() {
    let srgb = Builtin::Srgb.profile();
    assert!(matches!(BlackPreserving::new(srgb, fogra(), opts()), Err(CmsError::InvalidArgument(_))));
    assert!(matches!(BlackPreserving::new(fogra(), srgb, opts()), Err(CmsError::InvalidArgument(_))));
    let abs = TransformOptions { intent: Intent::AbsoluteColorimetric, ..opts() };
    assert!(matches!(BlackPreserving::new(fogra(), coated(), abs), Err(CmsError::InvalidArgument(_))));
    let bp = BlackPreserving::new(coated(), fogra(), opts()).unwrap();
    for k in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 2.0] {
        assert!((0.0..=1.0).contains(&bp.map_k(k)), "{k}");
    }
    let mut out = [0.0f32; 4];
    assert!(bp.eval(&[0.0; 3], &mut out).is_err(), "short input");
    assert!(bp.convert_u8(&[0, 0, 0, 9, 0], 4, &mut [0u8; 4], 4, false).is_err(), "partial pixel");
    assert_eq!(invert(&[], 50.0), 0.0);
    assert_eq!(invert(&[100.0], 50.0), 0.0);
    assert_eq!(invert(&[100.0, 0.0], 150.0), 0.0);
    assert_eq!(invert(&[100.0, 0.0], -5.0), 1.0);
    assert!((invert(&[100.0, 50.0, 0.0], 25.0) - 0.75).abs() < 1e-12);
    assert_eq!(map(&[], f32::INFINITY, 0.5), 0.0);
}

#[test]
fn black_beyond_the_destination_maximum_maps_to_full_black_between_samples() {
    // Source L* = 100 (1 - K), destination L* = 100 - 50 K: from K = 0.5 the source is darker
    // than any destination black, and 0.5 falls between the 1024 samples.
    let ramp = |f: fn(f64) -> f64| (0..K_SAMPLES).map(|i| f(i as f64 / (K_SAMPLES - 1) as f64)).collect::<Vec<_>>();
    let (src, dst) = (ramp(|k| 100.0 * (1.0 - k)), ramp(|k| 100.0 - 50.0 * k));
    let (curve, saturate) = k_curve(&src, &dst).unwrap();
    assert!((saturate - 0.5).abs() < 1e-6, "{saturate}");
    for k in [0.5001f32, 0.6, 1.0] {
        assert_eq!(map(&curve, saturate, k), 1.0, "{k}");
    }
    assert!((map(&curve, saturate, 0.25) - 0.5).abs() < 1e-4);
    let (_, never) = k_curve(&dst, &src).unwrap();
    assert_eq!(never, f32::INFINITY, "a lighter source never saturates");
}
