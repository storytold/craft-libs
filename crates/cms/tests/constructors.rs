use craft_cms::{Builtin, Curve, Intent, Mat3, Profile, Transform};

#[test]
fn test_curve_constructors() {
    // Gamma
    let g22 = Curve::gamma(2.2).expect("valid gamma");
    assert_eq!(g22, Curve::Gamma(2.2));

    let g10 = Curve::gamma(1.0).expect("gamma 1.0 is identity");
    assert_eq!(g10, Curve::Identity);

    let g_near = Curve::gamma(1.0 + 1e-7).expect("near-identity gamma normalized");
    assert_eq!(g_near, Curve::Identity);

    assert!(Curve::gamma(0.0).is_err());
    assert!(Curve::gamma(-2.2).is_err());
    assert!(Curve::gamma(f64::NAN).is_err());
    assert!(Curve::gamma(f64::INFINITY).is_err());

    // Parametric
    let p0 = Curve::parametric(0, [2.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).expect("valid parametric type 0");
    assert_eq!(p0, Curve::Parametric { kind: 0, p: [2.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] });

    assert!(Curve::parametric(5, [0.0; 7]).is_err());
    assert!(Curve::parametric(0, [f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).is_err());

    // Sampled table
    let table = Curve::from_table(vec![0.0, 0.5, 1.0]).expect("valid sampled table");
    assert_eq!(table, Curve::Table(vec![0.0, 0.5, 1.0]));

    assert!(Curve::from_table(vec![0.5]).is_err());
    assert!(Curve::from_table(vec![0.0, f32::NAN]).is_err());
    assert!(Curve::from_table(vec![0.0, -0.1, 1.0]).is_err());
    assert!(Curve::from_table(vec![0.0, 2.0]).is_err());
    assert!(Curve::from_table(vec![0.0, 1.0001]).is_err());
}

#[test]
fn test_profile_constructors() {
    let srgb = Builtin::Srgb.profile();
    let white_d65 = [0.95047, 1.0, 1.08883];

    // 1. Matrix shaper with already PCS D50-adapted matrix (e.g. read from ICC tags)
    let pcs_d50_matrix: Mat3 = [[0.4360747, 0.3850649, 0.1430804], [0.2225045, 0.7168786, 0.0606169], [0.0139322, 0.0971045, 0.7141733]];
    let trc = [craft_cms::curve::srgb_trc(), craft_cms::curve::srgb_trc(), craft_cms::curve::srgb_trc()];
    let custom_from_pcs = Profile::matrix_shaper(white_d65, pcs_d50_matrix, trc.clone()).expect("matrix shaper from PCS D50");
    assert_eq!(custom_from_pcs.color_space, craft_cms::ColorSpace::Rgb);
    assert!(custom_from_pcs.same_colors(srgb), "custom profile from PCS D50 matrix should match reference sRGB");

    // 2. Matrix shaper from unadapted D65 source primaries (undergoes single Bradford adaptation)
    let source_d65_matrix: Mat3 = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];
    let custom_from_source = Profile::from_source_matrix(white_d65, source_d65_matrix, trc).expect("matrix shaper from source");
    assert!(custom_from_source.same_colors(srgb), "custom profile adapted from source primaries should match reference sRGB");

    // Evaluate transform
    let xform = Transform::new(&custom_from_pcs, srgb, Intent::RelativeColorimetric, true).expect("transform from custom RGB to sRGB");
    let mut out = [0.0f32; 3];
    xform.eval(&[0.5, 0.5, 0.5], &mut out).expect("eval");
    assert!((out[0] - 0.5).abs() < 1e-2);
    assert!((out[1] - 0.5).abs() < 1e-2);
    assert!((out[2] - 0.5).abs() < 1e-2);

    // Invalid matrix shaper inputs
    assert!(Profile::matrix_shaper([0.0, 1.0, 1.0], pcs_d50_matrix, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());
    assert!(Profile::matrix_shaper([f64::NAN, 1.0, 1.0], pcs_d50_matrix, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());
    let singular: Mat3 = [[0.0; 3]; 3];
    assert!(Profile::matrix_shaper(white_d65, singular, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());

    // Near-degenerate matrix that is invertible before quantization but becomes singular after
    let d50 = [0.9642, 1.0, 0.8249];
    let near_degenerate: Mat3 = [[d50[0] - 0.000001, 0.000001, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, d50[2]]];
    assert!(Profile::matrix_shaper(d50, near_degenerate, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());

    // Gray profile
    let gray = Profile::gray(white_d65, Curve::gamma(2.2).expect("curve")).expect("gray profile");
    assert_eq!(gray.color_space, craft_cms::ColorSpace::Gray);

    let gray_to_srgb = Transform::new(&gray, srgb, Intent::RelativeColorimetric, true).expect("transform from gray to sRGB");
    let mut srgb_out = [0.0f32; 3];
    gray_to_srgb.eval(&[0.5], &mut srgb_out).expect("eval");
    assert!((srgb_out[0] - srgb_out[1]).abs() < 1e-3);
    assert!((srgb_out[1] - srgb_out[2]).abs() < 1e-3);
}

#[test]
fn test_matrix_constructors_against_independent_moxcms_oracle() {
    use moxcms::{ColorProfile, Layout, TransformOptions as MoxOpts};

    let srgb = Builtin::Srgb.profile();
    let white_d65 = [0.95047, 1.0, 1.08883];

    // 1. PCS D50-adapted matrix profile
    let pcs_d50_matrix: Mat3 = [[0.4360747, 0.3850649, 0.1430804], [0.2225045, 0.7168786, 0.0606169], [0.0139322, 0.0971045, 0.7141733]];
    let trc = [craft_cms::curve::srgb_trc(), craft_cms::curve::srgb_trc(), craft_cms::curve::srgb_trc()];
    let custom_from_pcs = Profile::matrix_shaper(white_d65, pcs_d50_matrix, trc.clone()).expect("matrix shaper from PCS D50");

    // 2. Unadapted source matrix profile
    let source_d65_matrix: Mat3 = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];
    let custom_from_source = Profile::from_source_matrix(white_d65, source_d65_matrix, trc).expect("matrix shaper from source");

    // Convert both to standard ICC bytes and parse with independent oracle moxcms
    let mox_from_pcs = ColorProfile::new_from_slice(&custom_from_pcs.to_bytes()).expect("moxcms parse custom PCS profile");
    let mox_from_source = ColorProfile::new_from_slice(&custom_from_source.to_bytes()).expect("moxcms parse custom source profile");
    let mox_dst_srgb = ColorProfile::new_from_slice(&srgb.to_bytes()).expect("moxcms parse builtin sRGB");

    let test_pixels: Vec<u8> = (0..512u32).flat_map(|i| [(i * 17 % 256) as u8, (i * 53 % 256) as u8, (i * 97 % 256) as u8]).collect();

    // Verify 1: moxcms transforming custom_from_pcs -> builtin sRGB produces identity (diff <= 1)
    let mox_xform_pcs = mox_from_pcs.create_transform_8bit(Layout::Rgb, &mox_dst_srgb, Layout::Rgb, MoxOpts::default()).unwrap();
    let mut mox_out_pcs = vec![0u8; test_pixels.len()];
    mox_xform_pcs.transform(&test_pixels, &mut mox_out_pcs).unwrap();
    let max_diff_pcs = test_pixels.iter().zip(&mox_out_pcs).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(max_diff_pcs <= 1, "moxcms PCS-adapted profile -> sRGB max diff was {max_diff_pcs}");

    // Verify 2: moxcms transforming custom_from_source -> builtin sRGB produces identity (diff <= 1)
    let mox_xform_src = mox_from_source.create_transform_8bit(Layout::Rgb, &mox_dst_srgb, Layout::Rgb, MoxOpts::default()).unwrap();
    let mut mox_out_src = vec![0u8; test_pixels.len()];
    mox_xform_src.transform(&test_pixels, &mut mox_out_src).unwrap();
    let max_diff_src = test_pixels.iter().zip(&mox_out_src).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(max_diff_src <= 1, "moxcms source-adapted profile -> sRGB max diff was {max_diff_src}");

    // Verify 3: craft_cms and moxcms agree when converting from custom_from_source to DisplayP3
    let p3 = Builtin::DisplayP3.profile();
    let mox_dst_p3 = ColorProfile::new_from_slice(&p3.to_bytes()).expect("moxcms parse Display P3");
    let mox_xform_p3 = mox_from_source.create_transform_8bit(Layout::Rgb, &mox_dst_p3, Layout::Rgb, MoxOpts::default()).unwrap();
    let mut mox_p3_out = vec![0u8; test_pixels.len()];
    mox_xform_p3.transform(&test_pixels, &mut mox_p3_out).unwrap();

    let ours_xform_p3 = Transform::new(&custom_from_source, p3, Intent::RelativeColorimetric, false).unwrap();
    let mut ours_p3_out = vec![0u8; test_pixels.len()];
    ours_xform_p3.convert_u8(&test_pixels, 3, &mut ours_p3_out, 3, false).unwrap();

    let max_p3_diff = ours_p3_out.iter().zip(&mox_p3_out).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(max_p3_diff <= 1, "craft_cms vs moxcms custom source profile -> P3 max diff was {max_p3_diff}");
}
