//! Auto tone (a Lightroom-like recipe, see [`auto_tone`]) and auto white balance (grey-world
//! statistics on a proxy).

use craft_color_transform::cct::xy_to_temp_tint;
use craft_color_transform::{REC2020, Xy, bradford};
use craft_develop_model::DevelopSettings;
use serde::Serialize;

use crate::colorops::luminance_2020;
use crate::local::{WbSource, effective_wb};
use crate::raster::Rgb32f;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct AutoTone {
    pub exposure: f64,
    pub contrast: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub whites: f64,
    pub blacks: f64,
    pub vibrance: f64,
    pub saturation: f64,
}

/// Exposure in EV that Auto adds per EV of the scene's log-average luminance (the slope is negative:
/// darker scenes get more). The slope is Lightroom's: regressing the Exposure its Auto chose on 24
/// raw photos against our own [`scene_log_mean`] of each gives -0.41 EV/EV (r = -0.92).
const EXPOSURE_SLOPE: f64 = -0.41;
/// Exposure for a scene whose log-average luminance is middle grey: fitted, with [`CONTRAST`] and
/// [`VIBRANCE`], so our renders match Lightroom's Auto results on those photos (see [`auto_tone`]).
const EXPOSURE_AT_GREY: f64 = -0.58;
/// Highlights and Shadows are Lightroom's own: the average its Auto chose on the 24 photos (our
/// sliders aim at the same as its). Freeing them as well matched no better on held-out photos and
/// left them poorly determined.
const HIGHLIGHTS: f64 = -66.0;
const SHADOWS: f64 = 52.0;
/// Contrast and Vibrance are fitted: our slider scales differ from Lightroom's, and our base tone is
/// flatter, so Contrast is far above its +6. Whites, Blacks and Saturation stay at 0 (Whites and
/// Blacks made no difference to the match; Lightroom leaves Saturation at about 0).
const CONTRAST: f64 = 68.0;
const VIBRANCE: f64 = 30.0;

/// The scene's log-average luminance under the current white balance, in EV relative to middle grey
/// (the mean of log2(Y / 0.18) over a 512 px proxy, scene-linear, before any tone curve). Pixels that
/// aren't finite (a damaged decode) are left out: they get no weight when the proxy is made, and
/// proxy pixels made only of them are skipped. `None` when no pixel is valid.
pub fn scene_log_mean(src: &Rgb32f, info: &WbSource, s: &DevelopSettings) -> Option<f64> {
    let (mut clean, mut weight) = (src.clone(), Rgb32f::new(src.width, src.height));
    let mut any = false;
    for (p, w) in clean.data.iter_mut().zip(weight.data.iter_mut()) {
        if p.iter().all(|v| v.is_finite()) {
            *w = [1.0; 3];
            any = true;
        } else {
            *p = [0.0; 3];
        }
    }
    if !any {
        return None;
    }
    let fit = |img: &Rgb32f| crate::raster::resample::fit(img, 512, 512, crate::raster::resample::Filter::Box);
    let (img, weight) = (fit(&clean), fit(&weight));
    // the mean of the valid pixels under each proxy pixel
    let kept: Vec<[f32; 3]> = img.data.iter().zip(&weight.data).filter(|(_, w)| w[0] > 1e-6).map(|(p, w)| p.map(|v| v / w[0])).collect();
    let mut kept = Rgb32f { width: kept.len(), height: 1, data: kept };
    let base = DevelopSettings { wb: s.wb, process: s.process, ..DevelopSettings::default() };
    // (white balance is per pixel: the valid pixels can be balanced in a row of their own)
    crate::local::scene_linear_pre(&mut kept, info, &base);
    let (mut sum, mut n) = (0f64, 0usize);
    for c in &kept.data {
        let ev = (luminance_2020(*c).max(1e-6) / 0.18).log2();
        if ev.is_finite() {
            sum += ev as f64;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f64)
}

/// Auto settings for `src` under the current white balance (ignores current tone values), aiming at
/// what Lightroom's Auto does: the Exposure a scene needs follows its log-average luminance
/// ([`scene_log_mean`], scene-linear, so the rule doesn't depend on the base tone curve), and the
/// other sliders are one fixed recipe. Calibrated black-box against Lightroom's observed behaviour
/// (the values its Auto chose and its exported results) on 24 Sony raw photos, of which 16 have a
/// starting look that Sony's in-camera DRO doesn't brighten. Held out one photo at a time, those 16
/// come out at a median dE76 of 6.4 from Lightroom's Auto results (8.3 unedited; 18.8 with the
/// previous percentile rule, which ended about 16 L* too bright); all 24 at 7.7 (7.9 unedited).
/// Rendered sources (JPEGs) are already toned, so their log-average reads brighter and Auto changes
/// their exposure less. A source without a single valid pixel gets Exposure 0 (and the rest of the
/// recipe).
pub fn auto_tone(src: &Rgb32f, info: &WbSource, s: &DevelopSettings) -> AutoTone {
    let exposure = scene_log_mean(src, info, s).map_or(0.0, |m| (EXPOSURE_AT_GREY + EXPOSURE_SLOPE * m).clamp(-2.0, 3.0));
    AutoTone {
        exposure: (exposure * 100.0).round() / 100.0,
        contrast: CONTRAST,
        highlights: HIGHLIGHTS,
        shadows: SHADOWS,
        whites: 0.0,
        blacks: 0.0,
        vibrance: VIBRANCE,
        saturation: 0.0,
    }
}

/// An automatic black & white mix (slider values, red … magenta) for `src` under `s`'s white
/// balance and tone: each hue band's colourful pixels are pushed away from the image's mean
/// lightness — bands brighter than average get brighter, darker ones darker — so areas that
/// differ only in colour stay apart in grey. Bands with almost no colourful pixels stay at 0.
pub fn auto_bw_mix(src: &Rgb32f, info: &WbSource, s: &DevelopSettings) -> [f64; 8] {
    use craft_color_transform::oklab::{OkLab, lab_to_lch};
    let oklab = OkLab::for_space(&REC2020);
    let mut img = crate::raster::resample::fit(src, 512, 512, crate::raster::resample::Filter::Box);
    let base = DevelopSettings { wb: s.wb, light: s.light, process: s.process, ..DevelopSettings::default() };
    crate::local::scene_linear_pre(&mut img, info, &base);
    let gain = 2f32.powf(base.light.exposure as f32);
    let (mut mass, mut sum_l) = ([0f64; 8], [0f64; 8]);
    let (mut all_l, mut n) = (0f64, 0usize);
    for p in &img.data {
        let lch = lab_to_lch(oklab.from_rgb(p.map(|v| (v * gain).max(0.0))));
        all_l += lch[0] as f64;
        n += 1;
        let k = (lch[1] / 0.2).min(1.0) as f64; // as in the B&W conversion
        if k < 0.05 {
            continue;
        }
        let w = crate::colorops::band_weights(lch[2]);
        for i in 0..8 {
            mass[i] += w[i] as f64 * k;
            sum_l[i] += w[i] as f64 * k * lch[0] as f64;
        }
    }
    if n == 0 {
        return [0.0; 8];
    }
    let mean = all_l / n as f64;
    let total: f64 = mass.iter().sum();
    std::array::from_fn(|i| {
        if total <= 0.0 || mass[i] / total < 0.01 {
            return 0.0;
        }
        let sep = sum_l[i] / mass[i] - mean;
        (sep * 400.0).clamp(-60.0, 60.0).round()
    })
}

/// Grey-world white balance weighted towards mid-tone, low-chroma pixels. Returns (temp, tint).
pub fn auto_wb(src: &Rgb32f, info: &WbSource) -> (f64, f64) {
    auto_wb_with(src, info, None)
}

/// [`auto_wb`] with an optional camera-space neutral-white estimator (`neutral_white(avg_rgb)`).
pub fn auto_wb_with(src: &Rgb32f, info: &WbSource, neutral_white: Option<&dyn Fn([f64; 3]) -> Option<Xy>>) -> (f64, f64) {
    let img = crate::raster::resample::fit(src, 256, 256, crate::raster::resample::Filter::Box);
    let (mut acc, mut wsum) = ([0.0f64; 3], 0.0f64);
    for c in &img.data {
        let y = luminance_2020(*c);
        if !(0.01..=2.0).contains(&y) {
            continue;
        }
        let mx = c[0].max(c[1]).max(c[2]);
        let mn = c[0].min(c[1]).min(c[2]);
        let chroma = (mx - mn) / (mx + 1e-6);
        let w = (1.0 - chroma).powi(2) as f64 * (1.0 - ((y.log2() + 2.5) / 4.0).abs().min(1.0)) as f64;
        for i in 0..3 {
            acc[i] += c[i] as f64 * w;
        }
        wsum += w;
    }
    if wsum <= 0.0 {
        return (info.as_shot_temp, info.as_shot_tint);
    }
    let avg = acc.map(|v| v / wsum);
    let seen = match neutral_white.filter(|_| info.raw && !info.relative_wb).and_then(|f| f(avg)) {
        Some(white) => white,
        None => {
            let shot = craft_color_transform::cct::temp_tint_to_xy(info.as_shot_temp, info.as_shot_tint);
            let Some(white) = Xy::from_xyz(bradford(REC2020.white, shot).apply(REC2020.to_xyz().apply(avg))) else {
                return (info.as_shot_temp, info.as_shot_tint);
            };
            white
        }
    };
    let (t, tint) = xy_to_temp_tint(seen);
    (t.clamp(2000.0, 50000.0).round(), tint.clamp(-150.0, 150.0).round())
}

/// Temperature/tint currently in effect (for UI display).
pub fn current_wb(info: &WbSource, s: &DevelopSettings) -> (f64, f64) {
    effective_wb(info, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_follows_the_log_average_luminance() {
        let flat = |y: f32| Rgb32f::filled(64, 64, [y; 3]);
        let auto = |img: &Rgb32f| auto_tone(img, &WbSource::default(), &DevelopSettings::default());
        let log_mean = |img: &Rgb32f| scene_log_mean(img, &WbSource::default(), &DevelopSettings::default()).unwrap();
        // a scene at middle grey gets the exposure at grey; each EV darker adds 0.41 EV
        assert!(log_mean(&flat(0.18)).abs() < 1e-3);
        assert_eq!(auto(&flat(0.18)).exposure, -0.58);
        assert!((auto(&flat(0.18 / 8.0)).exposure - (-0.58 + 3.0 * 0.41)).abs() < 1e-9);
        // darker scenes get more, brighter ones less, within -2..3 EV
        let ramp = |lo: f32, step: f32| Rgb32f::from_fn(64, 64, |x, _| [lo + x as f32 * step; 3]);
        let (dark, bright) = (auto(&ramp(0.01, 0.0003)), auto(&ramp(0.8, 0.01)));
        assert!(dark.exposure > 0.5 && bright.exposure < -1.0, "{dark:?} {bright:?}");
        assert_eq!(auto(&flat(1e-9)).exposure, 3.0);
        assert_eq!(auto(&flat(1e6)).exposure, -2.0);
        // the rest is one recipe
        assert_eq!(
            (dark.contrast, dark.highlights, dark.shadows, dark.whites, dark.blacks, dark.vibrance, dark.saturation),
            (68.0, -66.0, 52.0, 0.0, 0.0, 30.0, 0.0)
        );
        assert_eq!((dark.contrast, dark.shadows), (bright.contrast, bright.shadows));
    }

    #[test]
    fn pixels_that_are_not_finite_are_left_out() {
        let auto = |img: &Rgb32f| auto_tone(img, &WbSource::default(), &DevelopSettings::default());
        // half middle grey, half NaN (or infinite): the grey half alone decides, as if the rest weren't there
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let half = Rgb32f::from_fn(64, 64, |x, _| if x < 32 { [0.18; 3] } else { [bad; 3] });
            assert_eq!(auto(&half).exposure, -0.58, "{bad}");
            // also through the resampling of a source larger than the proxy (proxy pixels on the border mix both halves)
            let big = Rgb32f::from_fn(1030, 700, |x, _| if x < 515 { [0.045; 3] } else { [bad; 3] });
            assert!((auto(&big).exposure - (-0.58 + 2.0 * 0.41)).abs() < 1e-9, "{bad}");
            // a few broken pixels scattered over a grey image change nothing
            let speckled = Rgb32f::from_fn(1030, 700, |x, y| if (x * 7 + y * 13) % 97 == 0 { [bad; 3] } else { [0.18; 3] });
            assert_eq!(auto(&speckled).exposure, -0.58, "{bad}");
        }
        // with no valid pixel at all (or none at all) there is nothing to measure: Exposure 0, the recipe as usual
        for img in [Rgb32f::filled(8, 8, [f32::NAN; 3]), Rgb32f::filled(600, 600, [f32::INFINITY; 3]), Rgb32f::new(0, 0)] {
            let a = auto(&img);
            assert_eq!((a.exposure, a.contrast, a.vibrance), (0.0, 68.0, 30.0));
        }
    }

    #[test]
    fn neutral_image_keeps_as_shot_wb() {
        let img = Rgb32f::from_fn(32, 32, |x, y| [0.05 + (x + y) as f32 * 0.004; 3]);
        let (t, tint) = auto_wb(&img, &WbSource::default());
        assert!((t - 6500.0).abs() < 150.0, "{t}");
        assert!(tint.abs() < 6.0, "{tint}");
    }

    #[test]
    fn blue_cast_is_corrected_by_higher_temp() {
        // A bluish cast should be neutralised by telling the pipeline the light was bluer (higher K).
        let img = Rgb32f::from_fn(32, 32, |_, _| [0.16, 0.18, 0.24]);
        let (t, _) = auto_wb(&img, &WbSource::default());
        assert!(t > 7000.0, "{t}");
        let mut s = DevelopSettings::default();
        s.wb.mode = craft_develop_model::WbMode::Custom;
        s.wb.temp = t;
        let (t2, tint2) = auto_wb(&img, &WbSource::default());
        let _ = (t2, tint2);
        let mut out = img.clone();
        let (tt, ti) = auto_wb(&img, &WbSource::default());
        s.wb.temp = tt;
        s.wb.tint = ti;
        crate::local::scene_linear_pre(&mut out, &WbSource::default(), &s);
        let c = out.get(0, 0);
        assert!((c[0] - c[2]).abs() < 0.02, "{c:?}");
    }
}

#[cfg(test)]
mod bw_tests {
    use super::*;

    #[test]
    fn auto_bw_mix_pushes_light_and_dark_hues_apart() {
        // left: a light yellow, right: a dark blue (scene-linear Rec. 2020)
        let src = Rgb32f::from_fn(64, 32, |x, _| if x < 32 { [0.55, 0.5, 0.05] } else { [0.01, 0.02, 0.12] });
        let m = auto_bw_mix(&src, &WbSource::default(), &DevelopSettings::default());
        let (yellow, blue) = (m[2], m[5]);
        assert!(yellow > 0.0 && blue < 0.0, "{m:?}");
        // an image without colour leaves the mix alone
        let grey = Rgb32f::from_fn(16, 16, |x, _| [x as f32 / 16.0; 3]);
        assert_eq!(auto_bw_mix(&grey, &WbSource::default(), &DevelopSettings::default()), [0.0; 8]);
    }
}

#[cfg(test)]
mod tint_tests {
    use super::*;

    #[test]
    fn auto_wb_and_picker_correct_green_with_positive_tint() {
        // The picker delegates a sampled patch to this same auto_wb implementation.
        for (rgb, sign) in [([0.18, 0.24, 0.18], 1.0), ([0.24, 0.18, 0.24], -1.0)] {
            let img = Rgb32f::filled(16, 16, rgb);
            let info = WbSource { raw: true, relative_wb: true, ..Default::default() };
            let (temp, tint) = auto_wb(&img, &info);
            assert!(tint * sign > 0.0, "{rgb:?}: temp {temp}, tint {tint}");
            let mut s = DevelopSettings::default();
            s.wb.mode = craft_develop_model::WbMode::Custom;
            s.wb.temp = temp;
            s.wb.tint = tint;
            let mut corrected = img;
            crate::local::white_balance(&mut corrected, &info, &s);
            let p = corrected.get(0, 0);
            let spread = p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]);
            assert!(spread < 0.003, "{rgb:?} -> {p:?}");
        }
    }
}
