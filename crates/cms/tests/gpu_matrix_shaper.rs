//! Independent ICC matrix/TRC reference for display shadows.
//!
//! Fixtures are generated here from the published sRGB equation and a synthetic monitor,
//! with no photograph or device profile. The expected values implement ICC.1:2010 Annex F.3
//! directly in f64, without Transform, Curve evaluation, or craft_cms::math:
//! https://www.color.org/specification/ICC1v43_2010-12.pdf

use craft_cms::pipeline::{Pipeline, Stage};
use craft_cms::{Builtin, Clut, Curve, GpuMatrixShaper, Intent, Lut3d, Profile, Transform, TransformOptions};

const SOURCE_MATRIX: [[f64; 3]; 3] = [[0.4360747, 0.3850649, 0.1430804], [0.2225045, 0.7168786, 0.0606169], [0.0139322, 0.0971045, 0.7141733]];
// Deliberately synthetic, D50-referenced wider-gamut monitor; not a user's measured profile.
const MONITOR_MATRIX: [[f64; 3]; 3] = [[0.55, 0.27, 0.1442], [0.25, 0.69, 0.06], [0.01, 0.06, 0.7549]];
const GAMMA: f64 = 2.2;
const SOURCE_ENTRIES: usize = 1024;
const DISPLAY_TOLERANCE: f64 = 1.0 / 255.0;

fn fixed16(x: f64) -> f64 {
    (x * 65536.0).round() / 65536.0
}

fn srgb_sample(i: usize) -> f64 {
    let x = i as f64 / (SOURCE_ENTRIES - 1) as f64;
    let linear = if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    (linear * 65535.0).round() / 65535.0
}

fn synthetic_profiles() -> (Profile, Profile) {
    let mut source = Builtin::Srgb.profile().clone();
    source.description = "Synthetic sampled sRGB for display regression".into();
    source.matrix = Some(SOURCE_MATRIX);
    source.trc = Some(std::array::from_fn(|_| Curve::Table((0..SOURCE_ENTRIES).map(|i| srgb_sample(i) as f32).collect())));
    let source = source.with_encoded_bytes();
    let mut monitor = Builtin::Srgb.profile().clone();
    monitor.description = "Synthetic gamma 2.2 display for regression".into();
    monitor.matrix = Some(MONITOR_MATRIX);
    monitor.trc = Some(std::array::from_fn(|_| Curve::Parametric { kind: 0, p: [GAMMA, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }));
    let monitor = monitor.with_encoded_bytes();
    // Exercise actual ICC serialization/parsing, including fixed-point matrix/TRC values.
    (Profile::parse(&source.to_bytes()).unwrap(), Profile::parse(&monitor.to_bytes()).unwrap())
}

fn synthetic_transform(bpc: bool) -> Transform {
    let (source, monitor) = synthetic_profiles();
    Transform::new(&source, &monitor, Intent::RelativeColorimetric, bpc).unwrap()
}

fn reference(rgb: [f32; 3]) -> [f64; 3] {
    let linear = rgb.map(|x| {
        let p = f64::from(x.clamp(0.0, 1.0)) * (SOURCE_ENTRIES - 1) as f64;
        let i = (p as usize).min(SOURCE_ENTRIES - 2);
        srgb_sample(i) + (srgb_sample(i + 1) - srgb_sample(i)) * (p - i as f64)
    });
    let xyz = SOURCE_MATRIX.map(|row| row.iter().zip(linear).map(|(m, x)| fixed16(*m) * x).sum::<f64>());
    // Solve M_monitor * linear_monitor = XYZ using elimination, rather than the production
    // matrix inverse/multiplication implementation.
    let mut a: [[f64; 4]; 3] =
        std::array::from_fn(|r| [fixed16(MONITOR_MATRIX[r][0]), fixed16(MONITOR_MATRIX[r][1]), fixed16(MONITOR_MATRIX[r][2]), xyz[r]]);
    for pivot in 0..3 {
        let divisor = a[pivot][pivot];
        for value in &mut a[pivot][pivot..] {
            *value /= divisor;
        }
        let normalized = a[pivot];
        for (r, row) in a.iter_mut().enumerate() {
            if r != pivot {
                let factor = row[pivot];
                for (value, coefficient) in row.iter_mut().zip(normalized).skip(pivot) {
                    *value -= factor * coefficient;
                }
            }
        }
    }
    std::array::from_fn(|c| a[c][3].clamp(0.0, 1.0).powf(1.0 / fixed16(GAMMA)))
}

/// CPU interpretation of the public GPU data layout; expected colours come only from
/// `reference`, so this tests the sampled representation, not Transform against itself.
fn sample_gpu(gpu: &GpuMatrixShaper, rgb: [f32; 3]) -> [f32; 3] {
    fn interpolate(table: &[[f32; 4]], channel: usize, coordinate: f32) -> f32 {
        let i = (coordinate as usize).min(table.len() - 2);
        let f = coordinate - i as f32;
        table[i][channel] + (table[i + 1][channel] - table[i][channel]) * f
    }
    let linear: [f32; 3] =
        std::array::from_fn(|c| interpolate(gpu.input_table(), c, rgb[c].clamp(0.0, 1.0) * GpuMatrixShaper::INPUT_INTERVALS as f32));
    let rows = gpu.matrix_rows();
    std::array::from_fn(|c| {
        let row = rows[c];
        let value = row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2] + row[3];
        let coordinate = value.clamp(0.0, 1.0).sqrt().sqrt() * (GpuMatrixShaper::OUTPUT_ENTRIES - 1) as f32;
        interpolate(gpu.output_table(), c, coordinate).clamp(0.0, 1.0)
    })
}

fn check_reference(gpu: &GpuMatrixShaper, rgb: [f32; 3]) {
    let actual = sample_gpu(gpu, rgb);
    let expected = reference(rgb);
    for c in 0..3 {
        assert!((f64::from(actual[c]) - expected[c]).abs() <= DISPLAY_TOLERANCE, "input {rgb:?}, channel {c}: GPU {actual:?}, ICC {expected:?}");
    }
}

#[test]
fn sampled_srgb_to_gamma_display_preserves_neutral_and_colored_shadows() {
    for bpc in [false, true] {
        let transform = synthetic_transform(bpc);
        let gpu = transform.gpu_matrix_shaper().unwrap();
        assert_eq!(gpu.input_table().len(), GpuMatrixShaper::INPUT_INTERVALS + 1);
        assert_eq!(gpu.output_table().len(), GpuMatrixShaper::OUTPUT_ENTRIES);
        assert!(gpu.input_table().iter().chain(gpu.output_table()).all(|v| v[3] == 1.0));
        for v in 0..=255 {
            check_reference(&gpu, [v as f32 / 255.0; 3]);
        }
        // Includes coloured shadows within the old LUT's very first 0..8/255 cell.
        for r in 0..=16 {
            for g in 0..=16 {
                for b in 0..=16 {
                    check_reference(&gpu, [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]);
                }
            }
        }
        for rgb in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.9, 0.6, 0.3]] {
            check_reference(&gpu, rgb);
        }
    }
}

#[test]
fn coarse_final_rgb_lut_fails_the_same_icc_shadow_reference() {
    let transform = synthetic_transform(false);
    let gpu = transform.gpu_matrix_shaper().unwrap();
    let coarse = Lut3d::from_transform(&transform, 33).unwrap();
    let rgb = [2.0 / 255.0; 3];
    let expected = reference(rgb);
    let old = coarse.sample(rgb);
    // Even float LUT nodes fail, before the additional rounding of RGBA8 upload. This
    // guards the cause of the original bug rather than a particular observed screenshot.
    assert!(old.iter().zip(expected).all(|(a, e)| e - f64::from(*a) > 2.0 / 255.0), "old {old:?}, reference {expected:?}");
    check_reference(&gpu, rgb);
}

#[test]
fn gpu_display_representation_accepts_u8_u16_and_float_samples() {
    let transform = synthetic_transform(false);
    let gpu = transform.gpu_matrix_shaper().unwrap();
    for v in [0_u8, 1, 2, 7, 8, 16, 32, 128, 255] {
        check_reference(&gpu, [v as f32 / 255.0; 3]);
    }
    for v in [0_u16, 1, 2, 15, 63, 257, 514, 1028, 4096, 32768, 65535] {
        check_reference(&gpu, [v as f32 / 65535.0; 3]);
    }
    for rgb in [[0.000003, 0.00001, 0.00002], [0.00123, 0.00456, 0.00789], [0.2, 0.45, 0.8], [-0.5, 0.0, 2.0]] {
        check_reference(&gpu, rgb);
    }
    assert_eq!(sample_gpu(&gpu, [0.0; 3]), [0.0; 3]);
    // White differs very slightly from 1 because the synthetic matrices use ICC s15Fixed16.
    check_reference(&gpu, [1.0; 3]);
}

#[test]
fn absent_curves_and_matrix_use_identity_without_losing_offsets() {
    let identity = Transform::from_pipeline(Pipeline::new(3, Vec::new()).unwrap(), TransformOptions::default()).unwrap().gpu_matrix_shaper().unwrap();
    for rgb in [[0.0; 3], [1.0; 3], [0.001, 0.2, 0.8], [-2.0, 0.5, 2.0]] {
        for (actual, input) in sample_gpu(&identity, rgb).into_iter().zip(rgb) {
            assert!((actual - input.clamp(0.0, 1.0)).abs() < 1e-6);
        }
    }
    let matrix = Stage::Matrix { rows: 3, cols: 3, m: vec![0.8, 0.0, 0.0, 0.0, 0.7, 0.0, 0.0, 0.0, 0.6], offset: vec![0.01, 0.02, 0.03] };
    let transform =
        Transform::from_pipeline(Pipeline::new(3, vec![matrix]).unwrap(), TransformOptions::default()).unwrap().gpu_matrix_shaper().unwrap();
    let input = [0.1, 0.2, 0.3];
    let output = sample_gpu(&transform, input);
    for (actual, expected) in output.into_iter().zip([0.09, 0.16, 0.21]) {
        assert!((actual - expected).abs() < 1e-6, "{output:?}");
    }
}

#[test]
fn unsupported_clut_proof_and_non_rgb_transforms_retain_fallback() {
    let identity_clut = Clut::sample(vec![2; 3], 3, |input, output| output.copy_from_slice(input)).unwrap();
    let clut = Transform::from_pipeline(Pipeline::new(3, vec![Stage::Clut(identity_clut)]).unwrap(), TransformOptions::default()).unwrap();
    assert!(clut.gpu_matrix_shaper().is_none());
    let srgb = Builtin::Srgb.profile();
    let lab = Transform::new(Builtin::LabD50.profile(), srgb, Intent::RelativeColorimetric, false).unwrap();
    assert!(lab.gpu_matrix_shaper().is_none());
    let gray = Transform::new(Builtin::GrayGamma22.profile(), srgb, Intent::RelativeColorimetric, false).unwrap();
    assert!(gray.gpu_matrix_shaper().is_none());
    let proof = Transform::proof(srgb, Builtin::DisplayP3.profile(), srgb, Intent::RelativeColorimetric, false, false).unwrap();
    assert!(proof.gpu_matrix_shaper().is_none());
}

#[test]
fn malformed_matrix_storage_is_not_exposed_to_the_gpu() {
    for (coefficients, offsets) in [(vec![2.0; 8], vec![0.0; 3]), (vec![2.0; 9], vec![0.0; 2]), (vec![f64::INFINITY; 9], vec![0.0; 3])] {
        // A matrix whose storage does not match its shape is refused when the pipeline is built.
        let Ok(pipeline) = Pipeline::new(3, vec![Stage::Matrix { rows: 3, cols: 3, m: coefficients, offset: offsets }]) else { continue };
        let transform = Transform::from_pipeline(pipeline, TransformOptions::default()).unwrap();
        assert!(transform.gpu_matrix_shaper().is_none());
    }
}
