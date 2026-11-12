//! The camera's local tone mapping: a raw's DNG `ProfileGainTableMap` (Apple ProRAW), rendered
//! when the photo's Profile option "Camera local tone mapping" is on. Off by default: Lightroom
//! Classic renders ProRAW without it (see `crate::sensor::gaintable`).
//!
//! The map is a per-pixel gain that depends on the pixel's position in the raw's active area and
//! on its colour after the baseline exposure (linear ProPhoto). The source the pipeline gets is
//! exactly that colour (scene-linear Rec.2020 after the baseline exposure), so the gain is
//! evaluated here, on the resampled source at output resolution: a preview colours only the pixels
//! it shows. A gain multiplies R, G and B alike, so it commutes with the white balance (a matrix)
//! that follows. Files that also carry a `ProfileLookTable` get the gain after the table instead of
//! before it (DNG 1.7.1); Apple ProRAW has none.

use crate::color::{D50, D65, PROPHOTO, REC2020, bradford};
use crate::raster::Rgb32f;
use crate::sensor::gaintable::{GainTableMap, GainTableMapExt, SourcePlacement};
use craft_geom::Point;

use crate::{Plan, SourceInfo};
use craft_develop_model::DevelopSettings;

/// A raw's gain table map and where its developed source sits in the active area.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalTone {
    pub map: GainTableMap,
    pub placement: SourcePlacement,
}

/// Whether the render applies the source's local tone mapping.
pub fn enabled(info: &SourceInfo, s: &DevelopSettings) -> bool {
    s.profile.camera_local_tone && info.raw && info.local_tone.is_some()
}

/// Multiply the resampled, not yet white-balanced source `img` (the output frame of `p`) by the
/// local tone gain at each pixel. Blank canvas (outside a lens / perspective warp) is left alone.
pub fn apply(img: &mut Rgb32f, info: &SourceInfo, p: &Plan<'_>) {
    let Some(lt) = info.local_tone.as_deref().filter(|_| enabled(info, &p.settings)) else { return };
    let Some(eval) = lt.map.evaluator() else { return };
    let (w, h) = (img.width, img.height);
    let frame = &p.frame;
    let (ow, oh) = (frame.ow.max(1e-9), frame.oh.max(1e-9));
    let o2t = frame.out_to_oriented(w, h);
    let user = frame.orient.inverse();
    let to_prophoto = PROPHOTO.from_xyz().mul(&bradford(D65, D50)).mul(&REC2020.to_xyz()).to_f32();
    crate::for_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            // output pixel → user-oriented source px → EXIF-oriented source (0..1) → active area
            let s = match &frame.warp {
                Some(wp) => match wp.frame(&o2t, x, y) {
                    Some((_, s)) => s,
                    None => continue,
                },
                None => o2t.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5)),
            };
            let (u, v) = user.map(s.x / ow, s.y / oh, 1.0, 1.0);
            let (ax, ay) = lt.placement.active(u, v);
            let m = &to_prophoto;
            let q: [f32; 3] = std::array::from_fn(|i| m[i][0] * px[0] + m[i][1] * px[1] + m[i][2] * px[2]);
            let g = eval.gain(q, ax, ay);
            *px = px.map(|c| c * g);
        }
    });
}
