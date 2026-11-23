//! Integration tests: built-in profiles, known colorimetric values, round trips, CMYK
//! behaviour, intents, BPC, interpolation accuracy, real-world profiles and an independent
//! oracle (moxcms, dev-dependency only).

use craft_cms::{Builtin, Intent, Profile, Transform, TransformOptions};

fn lab() -> &'static Profile {
    Builtin::LabD50.profile()
}
fn srgb() -> &'static Profile {
    Builtin::Srgb.profile()
}
fn cmyk() -> &'static Profile {
    Builtin::CoatedCmyk.profile()
}

fn enc(l: [f64; 3]) -> [f32; 3] {
    [(l[0] / 100.0) as f32, ((l[1] + 128.0) / 255.0) as f32, ((l[2] + 128.0) / 255.0) as f32]
}

#[test]
fn builtins_encode_and_reparse() {
    for b in Builtin::ALL {
        let p = b.profile();
        let bytes = p.to_bytes();
        let q = Profile::parse(&bytes).unwrap_or_else(|e| panic!("{b:?}: {e}"));
        assert_eq!(q.description, b.description(), "{b:?}");
        assert_eq!(q.color_space, p.color_space);
        assert_eq!(&*q.to_bytes(), &*bytes, "{b:?} bytes are kept verbatim");
        assert_eq!(Builtin::from_id(b.id()), Some(b));
        // Every builtin converts to Lab and back.
        Transform::new(p, lab(), Intent::RelativeColorimetric, false).unwrap();
        Transform::new(lab(), p, Intent::RelativeColorimetric, false).unwrap();
    }
    assert_eq!(Builtin::from_id("Adobe RGB (1998)"), Some(Builtin::AdobeRgbCompat));
}

#[test]
fn malformed_profiles_are_rejected_without_panicking() {
    assert!(Profile::parse(&[]).is_err());
    assert!(Profile::parse(&[0u8; 200]).is_err());
    let good = srgb().to_bytes();
    for cut in [100, 131, 140, 200, good.len() - 20] {
        let _ = Profile::parse(&good[..cut]);
    }
    let mut bad = good.to_vec();
    for i in (128..bad.len()).step_by(7) {
        bad[i] ^= 0x5A;
        let _ = Profile::parse(&bad);
    }
    let c = cmyk().to_bytes();
    let mut bad = c.to_vec();
    for i in (128..400).step_by(3) {
        bad[i] = 0xFF;
        if let Ok(p) = Profile::parse(&bad) {
            let _ = Transform::new(&p, srgb(), Intent::Perceptual, true);
        }
    }
}

/// Independent oracle: moxcms (BSD-3/Apache-2.0, dev-dependency only) converting with the same
/// profile bytes.
#[test]
fn oracle_moxcms_rgb() {
    use moxcms::{ColorProfile, Layout, TransformOptions as MoxOpts};
    for dst in [Builtin::DisplayP3, Builtin::AdobeRgbCompat, Builtin::ProPhotoCompat] {
        let s = ColorProfile::new_from_slice(&srgb().to_bytes()).unwrap();
        let d = ColorProfile::new_from_slice(&dst.profile().to_bytes()).unwrap();
        let mt = s.create_transform_8bit(Layout::Rgb, &d, Layout::Rgb, MoxOpts::default()).unwrap();
        let src: Vec<u8> = (0..4096u32).flat_map(|i| [(i * 37 % 256) as u8, (i * 91 % 256) as u8, (i * 13 % 256) as u8]).collect();
        let mut theirs = vec![0u8; src.len()];
        mt.transform(&src, &mut theirs).unwrap();
        let ours_t = Transform::new(srgb(), dst.profile(), Intent::RelativeColorimetric, false).unwrap();
        let mut ours = vec![0u8; src.len()];
        ours_t.convert_u8(&src, 3, &mut ours, 3, false).unwrap();
        let worst = ours.iter().zip(&theirs).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
        assert!(worst <= 2, "{dst:?}: max diff {worst}");
    }
}

/// The CMYK oracle with the built-in Coated FOGRA39 bytes, so it runs on every platform: the
/// same comparison as [`oracle_moxcms_system_cmyk`] (CMYK → sRGB, 8-bit, relative colorimetric
/// without BPC).
#[test]
fn oracle_moxcms_builtin_cmyk() {
    use moxcms::{ColorProfile, Layout, RenderingIntent, TransformOptions as MoxOpts};
    let cmyk = Builtin::CoatedFogra39.profile();
    let theirs_p = ColorProfile::new_from_slice(&cmyk.to_bytes()).unwrap();
    let s = ColorProfile::new_from_slice(&srgb().to_bytes()).unwrap();
    let opts = MoxOpts { rendering_intent: RenderingIntent::RelativeColorimetric, ..Default::default() };
    let mt = theirs_p.create_transform_8bit(Layout::Rgba, &s, Layout::Rgb, opts).unwrap();
    let src: Vec<u8> = (0..2048u32).flat_map(|i| [(i * 37 % 256) as u8, (i * 91 % 256) as u8, (i * 13 % 256) as u8, (i * 57 % 256) as u8]).collect();
    let mut theirs = vec![0u8; 2048 * 3];
    mt.transform(&src, &mut theirs).unwrap();
    let t = Transform::new(cmyk, srgb(), Intent::RelativeColorimetric, false).unwrap();
    let mut ours = vec![0u8; 2048 * 3];
    t.convert_u8(&src, 4, &mut ours, 3, false).unwrap();
    let mut diffs: Vec<i32> = ours.iter().zip(&theirs).map(|(a, b)| (*a as i32 - *b as i32).abs()).collect();
    let mean = diffs.iter().sum::<i32>() as f64 / diffs.len() as f64;
    let wi = diffs.iter().enumerate().max_by_key(|(_, d)| **d).map_or(0, |(i, _)| i / 3);
    diffs.sort_unstable();
    let (p99, worst) = (diffs[diffs.len() * 99 / 100], diffs[diffs.len() - 1]);
    eprintln!("Coated FOGRA39 → sRGB vs moxcms: max {worst}, p99 {p99}, mean {mean:.3}");
    eprintln!("worst px cmyk {:?}: ours {:?} theirs {:?}", &src[wi * 4..wi * 4 + 4], &ours[wi * 3..wi * 3 + 3], &theirs[wi * 3..wi * 3 + 3]);
    // Measured: max 10, p99 5, mean 0.89. The largest differences are dark, steep sRGB values;
    // the worst (CMYK 237/147/21/33, Lab 37.95/−4.24/−38.55) is red 25.1 by an independent
    // float computation, as ours (25), where moxcms gives 15.
    assert!(mean < 1.2 && p99 <= 6 && worst <= 12, "max {worst} p99 {p99} mean {mean}");
}

/// sRGB white separates to no ink on every evaluation path, for every intent but absolute
/// colorimetric, and CMYK paper comes back as sRGB white; other colours keep the profile's values.
#[test]
fn white_is_exact_on_every_path() {
    let none = [0.0f32; 4];
    for cmyk in [Builtin::CoatedCmyk, Builtin::CoatedFogra39] {
        for intent in [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation] {
            for bpc in [false, true] {
                for precise_float in [false, true] {
                    let opts = TransformOptions { intent, bpc, precise_float };
                    let at = format!("{cmyk:?} {opts:?}");
                    let t = Transform::with_options(srgb(), cmyk.profile(), opts).unwrap();
                    let (mut e, mut f) = ([1.0f32; 4], [1.0f32; 4]);
                    t.eval(&[1.0; 3], &mut e).unwrap();
                    t.eval_fast(&[1.0; 3], &mut f).unwrap();
                    assert_eq!((e, f), (none, none), "{at}: eval / eval_fast");
                    let mut f32s = [1.0f32; 8];
                    t.convert_f32(&[1.0; 6], 3, &mut f32s, 4, false).unwrap();
                    assert_eq!(f32s, [0.0; 8], "{at}: convert_f32");
                    let mut inplace = [1.0f32; 4];
                    t.apply(&mut inplace, 4).unwrap();
                    assert_eq!(inplace, none, "{at}: apply");
                    let (mut u8s, mut u16s) = ([9u8; 4], [9u16; 4]);
                    t.convert_u8(&[255; 3], 3, &mut u8s, 4, false).unwrap();
                    t.convert_u16(&[65535; 3], 3, &mut u16s, 4, false).unwrap();
                    assert_eq!((u8s, u16s), ([0; 4], [0; 4]), "{at}: convert_u8 / convert_u16");

                    let back = Transform::with_options(cmyk.profile(), srgb(), opts).unwrap();
                    let (mut e, mut f) = ([0.0f32; 3], [0.0f32; 3]);
                    back.eval(&none, &mut e).unwrap();
                    back.eval_fast(&none, &mut f).unwrap();
                    assert_eq!((e, f), ([1.0; 3], [1.0; 3]), "{at}: paper to sRGB");

                    // Near white: the last step to white exceeds the step before it by the ink
                    // the profile itself puts at white: 2·10⁻⁵ for Coated FOGRA39, whose white
                    // is a grid node, and up to 0.8 % (C + M + Y) for the Photocraft profile,
                    // whose white falls between nodes (as with lcms's white-node fix).
                    let ink = |g: f32| {
                        let mut o = [0.0f32; 4];
                        t.eval(&[g / 255.0; 3], &mut o).unwrap();
                        o.iter().sum::<f32>()
                    };
                    let (before, last) = (ink(253.0) - ink(254.0), ink(254.0) - ink(255.0));
                    let allowed = if cmyk == Builtin::CoatedFogra39 { 1e-4 } else { 0.01 };
                    assert!(before > 0.0 && (last - before).abs() < allowed, "{at}: ink steps {before} then {last}");
                }
            }
            let proof = Transform::proof(srgb(), cmyk.profile(), srgb(), intent, true, false).unwrap();
            let (mut e, mut f) = ([0.0f32; 3], [0.0f32; 3]);
            proof.eval(&[1.0; 3], &mut e).unwrap();
            proof.eval_fast(&[1.0; 3], &mut f).unwrap();
            assert_eq!((e, f), ([1.0; 3], [1.0; 3]), "{cmyk:?} {intent:?}: proof");
        }
        // Lab white is no node of the device-link grid; it is still exact.
        let from_lab = Transform::new(lab(), cmyk.profile(), Intent::RelativeColorimetric, false).unwrap();
        let (mut e, mut f, mut u) = ([1.0f32; 4], [1.0f32; 4], [9u8; 4]);
        from_lab.eval(&enc([100.0, 0.0, 0.0]), &mut e).unwrap();
        from_lab.eval_fast(&enc([100.0, 0.0, 0.0]), &mut f).unwrap();
        from_lab.convert_u8(&[255, 128, 128], 3, &mut u, 4, false).unwrap();
        assert_eq!((e, f, u), (none, none, [0; 4]), "{cmyk:?}: Lab white");
        // Absolute colorimetric keeps the paper colour.
        let abs = Transform::new(cmyk.profile(), srgb(), Intent::AbsoluteColorimetric, false).unwrap();
        let mut o = [0.0f32; 3];
        abs.eval(&none, &mut o).unwrap();
        assert!(o[2] < 0.99, "{cmyk:?}: absolute paper {o:?}");
    }
}

/// The blocked matrix/TRC path (64 pixels at a time) must match the per-pixel evaluation for
/// every pixel count around the block size, with and without extra channels.
#[test]
fn matrix_trc_blocks_match_per_pixel_eval() {
    let pairs = [(Builtin::Srgb, Builtin::DisplayP3), (Builtin::ProPhotoCompat, Builtin::Srgb), (Builtin::AdobeRgbCompat, Builtin::Rec2020)];
    for (a, b) in pairs {
        let t = Transform::new(a.profile(), b.profile(), Intent::RelativeColorimetric, true).unwrap();
        for &(ss, ds, copy) in &[(3usize, 3usize, false), (4, 4, true), (5, 4, true)] {
            let max = 20011;
            let seed = |i: usize| (i.wrapping_mul(2654435761) >> 7) as u32;
            let src16: Vec<u16> = (0..max * ss).map(|i| (seed(i) >> 3) as u16).collect();
            let src8: Vec<u8> = (0..max * ss).map(|i| (seed(i) >> 11) as u8).collect();
            let mut full8 = vec![0u8; max * ds];
            t.convert_u8(&src8, ss, &mut full8, ds, copy).unwrap();
            for px in [1usize, 2, 63, 64, 65, 127, 128, 129, 1000] {
                let mut o16 = vec![0u16; px * ds];
                t.convert_u16(&src16[..px * ss], ss, &mut o16, ds, copy).unwrap();
                let mut o8 = vec![0u8; px * ds];
                t.convert_u8(&src8[..px * ss], ss, &mut o8, ds, copy).unwrap();
                assert_eq!(o8[..], full8[..px * ds], "u8 prefix of a longer buffer, {px} px");
                for p in 0..px {
                    let inp: Vec<f32> = (0..3).map(|k| src16[p * ss + k] as f32 / 65535.0).collect();
                    let mut want = [0.0f32; 3];
                    t.eval_fast(&inp, &mut want).unwrap();
                    for k in 0..3 {
                        assert_eq!(o16[p * ds + k], (want[k].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16, "u16 px {p} ch {k} of {px}");
                    }
                    if copy && ds > 3 {
                        assert_eq!(o16[p * ds + 3], src16[p * ss + 3]);
                        assert_eq!(o8[p * ds + 3], src8[p * ss + 3]);
                    }
                }
            }
        }
    }
}

/// 8-bit and 16-bit conversion through an analytic pipeline (no lookup table, here RGB to Lab)
/// must equal the exact per-pixel evaluation. The 8-bit path tabulates the first stage's
/// curves per code; `curves_are_tables` selects whether those curves are parametric (their
/// `eval` is analytic) or `curv` tables (their `eval` interpolates inside the same loop).
fn check_exact_core(src_profile: &Profile, curves_are_tables: bool) {
    let trc = src_profile.trc.as_ref().expect("matrix/TRC source");
    assert!(trc.iter().all(|c| matches!(c, craft_cms::Curve::Table(_)) == curves_are_tables), "TRC kind");
    let t = Transform::new(src_profile, lab(), Intent::RelativeColorimetric, true).unwrap();
    let px = 256usize * 256;
    let src: Vec<u8> = (0..px as u32).flat_map(|i| [(i & 255) as u8, (i >> 8) as u8, (i.wrapping_mul(2654435761) >> 13) as u8, 200]).collect();
    let mut out = vec![0u8; px * 4];
    t.convert_u8(&src, 4, &mut out, 4, true).unwrap();
    for (sp, dp) in src.as_chunks::<4>().0.iter().zip(out.as_chunks::<4>().0) {
        let inp = [sp[0] as f32 / 255.0, sp[1] as f32 / 255.0, sp[2] as f32 / 255.0];
        let mut want = [0.0f32; 3];
        t.eval(&inp, &mut want).unwrap();
        for k in 0..3 {
            assert_eq!(dp[k], (want[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, "u8 {sp:?} ch {k}");
        }
        assert_eq!(dp[3], 200);
    }
    // 16-bit input stays off the 8-bit table and must agree with the untabulated evaluation.
    let src16: Vec<u16> = (0..px * 4).map(|i| if i % 4 == 3 { 51400 } else { (i.wrapping_mul(2654435761) >> 9) as u16 }).collect();
    let mut out16 = vec![0u16; px * 4];
    t.convert_u16(&src16, 4, &mut out16, 4, true).unwrap();
    for (sp, dp) in src16.as_chunks::<4>().0.iter().zip(out16.as_chunks::<4>().0) {
        let inp = [sp[0] as f32 / 65535.0, sp[1] as f32 / 65535.0, sp[2] as f32 / 65535.0];
        let mut want = [0.0f32; 3];
        t.eval(&inp, &mut want).unwrap();
        for k in 0..3 {
            assert_eq!(dp[k], (want[k].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16, "u16 {sp:?} ch {k}");
        }
        assert_eq!(dp[3], 51400);
    }
}

#[test]
fn exact_core_u8_matches_exact_eval() {
    check_exact_core(srgb(), false);
}

/// sRGB the way Photoshop embeds it: a v2 profile whose TRCs are 1024-entry 16-bit tables.
fn srgb_as_v2_tables() -> Profile {
    let mut p = srgb().clone();
    let table = |c: &craft_cms::Curve| {
        craft_cms::Curve::Table((0..1024).map(|i| ((c.eval64(i as f64 / 1023.0) * 65535.0).round() / 65535.0) as f32).collect())
    };
    let trc = p.trc.clone().unwrap();
    p.trc = Some([table(&trc[0]), table(&trc[1]), table(&trc[2])]);
    p.version = (2, 0x10);
    p.with_encoded_bytes()
}

/// Same, with `curv` table TRCs: the first-stage table is built from `Curve::eval`'s table
/// branch and must still be applied to its own channel.
#[test]
fn exact_core_u8_matches_exact_eval_with_table_trc() {
    // Distinct tables per channel, so a first stage applied to the wrong channel shows up.
    let mut p = srgb_as_v2_tables();
    let table = |g: f64| craft_cms::Curve::Table((0..1024).map(|i| ((i as f64 / 1023.0).powf(g) * 65535.0).round() as f32 / 65535.0).collect());
    p.trc = Some([table(1.8), table(2.2), table(2.6)]);
    let table_trc = Profile::parse(&p.with_encoded_bytes().to_bytes()).unwrap();
    check_exact_core(&table_trc, true);
}
