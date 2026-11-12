//! V1 and V2 render as they did before process V3, on every platform: 16-bit samples of small
//! synthetic renders, captured on main at 76b5377 (before process V3 existed), within
//! [`LSB`]. `tests_light::v1_and_v2_render_as_before_v3` checks the same byte for byte, but only
//! on macOS aarch64, where its hashes were captured.

use crate::raster::Rgb32f;
use craft_develop_model::{DevelopSettings, ProcessVersion, controls};

use crate::{DeepSamples, OutputDepth, RenderRequest, SourceInfo, render};

/// 480 × 320, arithmetic only (no platform maths): luminance rising over 12 stops left to right,
/// four colour bands top to bottom, thin bright and dark lines every 16 px (so V2's
/// Highlights / Shadows median has something to keep), and a few clipped highlights.
fn scene() -> Rgb32f {
    Rgb32f::from_fn(480, 320, |x, y| {
        let v = 0.0002 * 1.0175f32.powi(x as i32);
        let c = match y / 80 {
            0 => [v, v, v],
            1 => [v * 1.3, v * 0.8, v * 0.4],
            2 => [v * 0.35, v * 0.9, v * 1.4],
            _ => [v * 0.9, v * 0.3, v * 1.1],
        };
        if x % 16 == 7 {
            c.map(|k| k * 2.5)
        } else if y % 16 == 11 {
            c.map(|k| k * 0.35)
        } else if x > 440 && y % 40 < 3 {
            [4.0, 4.0, 4.0]
        } else {
            c
        }
    })
}

fn camera_look() -> SourceInfo {
    let knots = std::array::from_fn(|i| {
        let x = 0.004 * 1.18f32.powi(i as i32);
        [x, 1.0 - (-2.0 * x).exp()]
    });
    let curve = crate::tone::CameraTone::new(knots).unwrap().with_chroma([1.3, 1.2, 1.1, 1.0, 0.9, 0.7, 0.5, 0.4]).unwrap();
    SourceInfo { raw: true, as_shot_temp: 5200.0, as_shot_tint: 4.0, camera_tone: Some(curve), ..Default::default() }
}

/// Where the samples are taken (output pixels of the 480 × 320 render).
const AT: [(usize, usize); 12] =
    [(20, 20), (120, 30), (232, 40), (300, 27), (400, 60), (470, 1), (60, 100), (200, 125), (350, 140), (90, 200), (263, 210), (420, 290)];

/// 16-bit samples of V1 and V2 renders (the same order as `LEGACY`): three sources (rendered,
/// raw, raw with a camera look) × two edits (plain, a typical edit with Contrast, Highlights,
/// Shadows, Whites, Blacks and Clarity) × two processes.
fn legacy_samples() -> Vec<[u16; 3]> {
    let infos = [SourceInfo::default(), SourceInfo { raw: true, as_shot_temp: 5200.0, as_shot_tint: 4.0, ..Default::default() }, camera_look()];
    let mut edited = DevelopSettings::for_raw(5200.0, 4.0);
    for (id, v) in [
        ("light.exposure", 0.4),
        ("light.contrast", 30.0),
        ("light.highlights", -60.0),
        ("light.shadows", 40.0),
        ("light.whites", 15.0),
        ("light.blacks", -20.0),
        ("effects.clarity", 20.0),
    ] {
        controls::set(&mut edited, id, v);
    }
    let req = RenderRequest { depth: OutputDepth::U16, ..RenderRequest::fit(480, 480) };
    let src = scene();
    let mut out = Vec::new();
    for info in &infos {
        for s in [DevelopSettings::for_raw(5200.0, 4.0), edited.clone()] {
            for process in [ProcessVersion(1), ProcessVersion(2)] {
                let s = DevelopSettings { process, ..s.clone() };
                let deep = render(&src, info, &s, &req).deep.unwrap();
                let DeepSamples::U16(v) = deep.samples else { panic!("16-bit samples") };
                for (x, y) in AT {
                    let i = (y * deep.width + x) * 3;
                    out.push([v[i], v[i + 1], v[i + 2]]);
                }
            }
        }
    }
    out
}

/// Largest difference allowed from the captured samples, in 16-bit steps. The renders call the
/// platform's `powf` / `exp2` / `log2`, whose last bit can differ between platforms: a relative
/// change of ~1e-7, far below one step (1.5e-5), so a sample moves by at most one step where it
/// sits at a rounding boundary. A change of how V1 or V2 render moves samples by far more.
const LSB: u16 = 2;

#[test]
fn v1_and_v2_samples_are_as_before_v3_on_every_platform() {
    let got = legacy_samples();
    assert_eq!(got.len(), LEGACY.len());
    let worst = got.iter().zip(&LEGACY).flat_map(|(g, w)| (0..3).map(move |k| g[k].abs_diff(w[k]))).max().unwrap_or(0);
    for (i, (g, w)) in got.iter().zip(&LEGACY).enumerate() {
        assert!((0..3).all(|k| g[k].abs_diff(w[k]) <= LSB), "sample {i}: {g:?}, captured {w:?} (worst {worst})");
    }
}

/// [`legacy_samples`] on main at 76b5377.
#[rustfmt::skip]
const LEGACY: [[u16; 3]; 144] = [
    [240, 240, 240], [1222, 1222, 1222], [6573, 6573, 6573], [6760, 6760, 6760], [34082, 34082, 34082], [65535, 65535, 65535],
    [796, 355, 165], [6369, 3524, 1681], [28819, 19569, 13215], [0, 770, 1153], [0, 17104, 20902], [41649, 18367, 40817],
    [240, 240, 240], [1222, 1222, 1222], [6573, 6573, 6573], [6760, 6760, 6760], [34082, 34082, 34082], [65535, 65535, 65535],
    [796, 355, 165], [6369, 3524, 1681], [28819, 19569, 13215], [0, 770, 1153], [0, 17104, 20902], [41649, 18367, 40817],
    [314, 314, 314], [1699, 1699, 1699], [8278, 8278, 8278], [6991, 6991, 6991], [34698, 34698, 34698], [65535, 65535, 65535],
    [1062, 474, 220], [8221, 4847, 2529], [32250, 22021, 14995], [0, 1040, 1558], [0, 21972, 26663], [44088, 19551, 43211],
    [314, 314, 314], [1699, 1699, 1699], [8271, 8271, 8271], [6646, 6646, 6646], [34667, 34667, 34667], [65535, 65535, 65535],
    [1062, 474, 220], [8219, 4846, 2528], [32239, 22013, 14989], [0, 1040, 1558], [0, 22622, 27432], [44072, 19543, 43195],
    [272, 272, 272], [1384, 1384, 1384], [7098, 7098, 7098], [7293, 7293, 7293], [34521, 34521, 34521], [65535, 65535, 65535],
    [902, 403, 187], [6894, 3899, 1901], [29974, 20395, 13815], [0, 872, 1307], [0, 18017, 21983], [42788, 18920, 41935],
    [272, 272, 272], [1384, 1384, 1384], [7098, 7098, 7098], [7293, 7293, 7293], [34521, 34521, 34521], [65535, 65535, 65535],
    [902, 403, 187], [6894, 3899, 1901], [29974, 20395, 13815], [0, 872, 1307], [0, 18017, 21983], [42788, 18920, 41935],
    [16, 16, 16], [127, 127, 127], [5866, 5866, 5866], [3892, 3892, 3892], [36157, 36157, 36157], [65535, 65535, 65535],
    [59, 27, 12], [3177, 1437, 666], [32984, 22546, 15376], [0, 63, 95], [0, 22149, 26872], [45537, 20255, 44634],
    [16, 16, 16], [127, 127, 127], [5856, 5856, 5856], [3340, 3340, 3340], [36123, 36123, 36123], [65535, 65535, 65535],
    [59, 27, 12], [3174, 1436, 665], [32972, 22537, 15370], [0, 63, 95], [0, 22867, 27722], [45520, 20246, 44616],
    [477, 477, 477], [2433, 2433, 2433], [9924, 9924, 9924], [10170, 10170, 10170], [42194, 42194, 42194], [65533, 65533, 65533],
    [1800, 660, 167], [10387, 5624, 1736], [39549, 25689, 14987], [0, 1533, 2297], [0, 23519, 28494], [54941, 22090, 53814],
    [477, 477, 477], [2433, 2433, 2433], [9924, 9924, 9924], [10170, 10170, 10170], [42194, 42194, 42194], [65533, 65533, 65533],
    [1800, 660, 167], [10387, 5624, 1736], [39549, 25689, 14987], [0, 1533, 2297], [0, 23519, 28494], [54941, 22090, 53814],
    [639, 639, 639], [3436, 3436, 3436], [12637, 12637, 12637], [10872, 10872, 10872], [43751, 43751, 43751], [65393, 65393, 65393],
    [2464, 904, 228], [13380, 7612, 2802], [44578, 29433, 18028], [0, 2136, 3161], [0, 30532, 36793], [58572, 24437, 57388],
    [639, 639, 639], [3436, 3436, 3436], [12627, 12627, 12627], [10397, 10397, 10397], [43721, 43721, 43721], [65393, 65393, 65393],
    [2464, 904, 228], [13377, 7610, 2801], [44566, 29424, 18020], [0, 2136, 3161], [0, 31308, 37712], [58556, 24426, 57373],
];
