use super::*;

fn gray_ramp(width: u32, height: u32) -> Vec<f32> {
    (0..width * height).flat_map(|i| [i as f32 / (width * height) as f32; 3]).collect()
}

#[test]
fn identical_renders_are_within_any_tolerance() {
    let rgb = gray_ramp(4, 3);
    let image = Image::new(4, 3, &rgb).unwrap();
    let report = compare(&image, &image).unwrap();
    assert_eq!((report.pixels, report.max_delta_e, report.max_channel, report.worst), (12, 0.0, 0.0, None));
    assert!(report.within(&Tolerance { max_delta_e: 0.0, mean_delta_e: 0.0, max_channel: 0.0 }));
    assert_eq!(report.to_string(), "12 px: ΔE2000 max 0.0000 mean 0.0000, channel max 0.000000, identical");
}

#[test]
fn white_in_the_matrix_is_neutral() {
    // RGB (1, 1, 1) is the D65 white: L* 100, a* = b* = 0
    let [l, a, b] = lab([1.0; 3]);
    assert!((l - 100.0).abs() < 1e-9 && a.abs() < 1e-9 && b.abs() < 1e-9, "{l} {a} {b}");
}

#[test]
fn the_worst_pixel_and_the_means_are_reported() {
    let expected = gray_ramp(3, 2);
    let mut actual = expected.clone();
    // pixel (1, 1): red up by 0.1, a visible difference
    actual[4 * 3] += 0.1;
    let report = compare(&Image::new(3, 2, &expected).unwrap(), &Image::new(3, 2, &actual).unwrap()).unwrap();
    assert_eq!(report.worst, Some((1, 1)));
    assert!(report.max_delta_e > 1.0, "{report}");
    assert!((report.mean_delta_e - report.max_delta_e / 6.0).abs() < 1e-9);
    assert!((report.max_channel - 0.1).abs() < 1e-6);
    assert!((report.mean_channel[0] - 0.1 / 6.0).abs() < 1e-6 && report.mean_channel[1] == 0.0);
    let over = report.exceeded(&Tolerance::BASELINE);
    assert_eq!(over.len(), 3, "{over:?}");
    assert!(over[0].starts_with("max ΔE2000") && over[1].starts_with("mean ΔE2000") && over[2].starts_with("max channel"));
    assert!(!report.within(&Tolerance::default()));
    assert!(report.to_string().ends_with("worst at (1, 1)"));
}

#[test]
fn a_tenth_of_a_10_bit_code_value_passes_the_baseline() {
    let expected = vec![0.18f32; 2 * 2 * 3];
    let actual: Vec<f32> = expected.iter().map(|v| v + 0.1 / 1024.0).collect();
    let report = compare(&Image::new(2, 2, &expected).unwrap(), &Image::new(2, 2, &actual).unwrap()).unwrap();
    assert!(report.within(&Tolerance::BASELINE), "{report}");
    assert!(report.worst.is_some());
}

#[test]
fn a_channel_difference_too_small_for_cielab_still_names_a_pixel() {
    let expected = vec![0.5f32; 6];
    let mut actual = expected.clone();
    actual[5] = f32::from_bits(0.5f32.to_bits() + 1);
    let report = compare(&Image::new(2, 1, &expected).unwrap(), &Image::new(2, 1, &actual).unwrap()).unwrap();
    assert!(report.max_channel > 0.0);
    assert_eq!(report.worst, Some((1, 0)));
}

#[test]
fn an_empty_render_compares_as_identical() {
    let image = Image::new(0, 5, &[]).unwrap();
    let report = compare(&image, &image).unwrap();
    assert_eq!((report.pixels, report.mean_delta_e, report.worst), (0, 0.0, None));
    assert_eq!((image.width(), image.height()), (0, 5));
}

#[test]
fn bad_inputs_are_errors_not_panics() {
    let e = Image::new(2, 2, &[0.0; 11]).unwrap_err();
    assert_eq!(e, ParityError::Length { width: 2, height: 2, samples: 11 });
    assert_eq!(e.to_string(), "11 samples is not RGB for 2 × 2 pixels");
    assert!(Image::new(u32::MAX, u32::MAX, &[]).is_err());

    let a = vec![0.0f32; 6];
    let b = vec![0.0f32; 6];
    let e = compare(&Image::new(2, 1, &a).unwrap(), &Image::new(1, 2, &b).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "render is 1 × 2, expected 2 × 1");

    let mut nan = a.clone();
    nan[4] = f32::NAN;
    let e = compare(&Image::new(2, 1, &a).unwrap(), &Image::new(2, 1, &nan).unwrap()).unwrap_err();
    assert_eq!(e, ParityError::NotFinite { x: 1, y: 0 });
    assert_eq!(e.to_string(), "pixel (1, 0) is not a finite colour");
    let e: Box<dyn std::error::Error> = Box::new(e);
    assert!(e.source().is_none());
}

#[test]
fn a_nan_tolerance_fails_every_check() {
    let rgb = gray_ramp(2, 2);
    let image = Image::new(2, 2, &rgb).unwrap();
    let report = compare(&image, &image).unwrap();
    let nan = Tolerance { max_delta_e: f64::NAN, mean_delta_e: f64::NAN, max_channel: f32::NAN };
    assert_eq!(report.exceeded(&nan).len(), 3);
    assert!(!report.within(&nan));
    assert!(report.within(&Tolerance { max_delta_e: f64::INFINITY, ..Tolerance::BASELINE }));
}
