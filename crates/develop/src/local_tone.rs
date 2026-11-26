//! The camera's local tone mapping: a raw's DNG `ProfileGainTableMap` (Apple ProRAW), rendered
//! when the photo's Profile option "Camera local tone mapping" is on. Off by default: Lightroom
//! Classic renders ProRAW without it (see `lightcraft_raw::gaintable`).
//!
//! The map is a per-pixel gain that depends on the pixel's position in the raw's active area and
//! on its colour after the baseline exposure (linear ProPhoto). The source the pipeline gets is
//! exactly that colour (scene-linear Rec.2020 after the baseline exposure), so the gain is
//! evaluated here, on the resampled source at output resolution: a preview colours only the pixels
//! it shows. A gain multiplies R, G and B alike, so it commutes with the white balance (a matrix)
//! that follows. Files that also carry a `ProfileLookTable` get the gain after the table instead of
//! before it (DNG 1.7.1); Apple ProRAW has none.

use craft_color_transform::{D50, D65, PROPHOTO, REC2020, bradford};

use crate::raster::{Rgb32f, par_rows};
use crate::sensor::gaintable::{GainTableMap, GainTableMapApply, SourcePlacement};

/// A raw's gain table map and where its developed source sits in the active area.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalTone {
    pub map: GainTableMap,
    pub placement: SourcePlacement,
}

/// Multiply the resampled, not yet white-balanced source `img` by the local tone gain at each
/// pixel. For an output pixel `(x, y)`, `active_uv(x, y)` must return normalised (0..=1)
/// coordinates in the EXIF-oriented developed source, not in the raw active area.
/// [`SourcePlacement::active`] undoes the source orientation and maps through the crop to
/// normalised active-area coordinates. Return `None` to leave a blank-canvas pixel unchanged.
pub fn apply(img: &mut Rgb32f, lt: &LocalTone, active_uv: impl Fn(usize, usize) -> Option<(f64, f64)> + Sync) {
    let Some(eval) = lt.map.evaluator() else { return };
    let w = img.width;
    let to_prophoto = PROPHOTO.from_xyz().mul(&bradford(D65, D50)).mul(&REC2020.to_xyz()).to_f32();
    par_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let Some((u, v)) = active_uv(x, y) else { continue };
            let (ax, ay) = lt.placement.active(u, v);
            let m = &to_prophoto;
            let q: [f32; 3] = std::array::from_fn(|i| m[i][0] * px[0] + m[i][1] * px[1] + m[i][2] * px[2]);
            let g = eval.gain(q, ax, ay);
            *px = px.map(|c| c * g);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_tone_maps_oriented_source_coordinates_in_full_images_and_tiles() {
        // An original synthetic gain grid: the one-sample tables give g = 1 + 2x + 4y.
        let lt = LocalTone {
            map: GainTableMap {
                points_v: 2,
                points_h: 2,
                points_n: 1,
                spacing_v: 1.0,
                spacing_h: 1.0,
                origin_v: 0.0,
                origin_h: 0.0,
                weights: [0.0; 5],
                gamma: 1.0,
                gains: vec![1.0, 3.0, 5.0, 7.0],
            },
            placement: SourcePlacement { rect: [0.25, 0.25, 0.5, 0.5], orientation: craft_geom::Orientation::Rotate90 },
        };
        let source_uv = |x, y| match (x, y) {
            (2, 1) => Some((0.0, 0.0)),
            (3, 1) => Some((1.0, 1.0)),
            _ => None,
        };
        let pixel = [0.125, 0.25, 0.5];
        let mut full = Rgb32f { width: 4, height: 3, data: vec![pixel; 12] };
        apply(&mut full, &lt, source_uv);

        let mut expected = vec![pixel; 12];
        // The oriented corners land on opposite crop corners in the active area.
        expected[6] = [0.5625, 1.125, 2.25]; // (0.25, 0.75), gain 4.5
        expected[7] = [0.4375, 0.875, 1.75]; // (0.75, 0.25), gain 3.5
        assert_eq!(full.data, expected);

        let mut tile = Rgb32f { width: 2, height: 2, data: vec![pixel; 4] };
        apply(&mut tile, &lt, |x, y| source_uv(x + 2, y + 1));
        assert_eq!(tile.data, [full.data[6], full.data[7], full.data[10], full.data[11]]);
    }
}
