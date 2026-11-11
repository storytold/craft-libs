//! Print color: the blending color space of transparency groups
//! (`SerializeSettings::group_color_space`), overprint (`Surface::push_overprint`), knockout
//! groups (`Surface::push_group`), soft-mask backdrop and transfer function, DeviceN colors,
//! images and shadings, and the print output intent with the PDF/X markers.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use backend::color::{cmyk, luma};
use backend::geom::{PathBuilder, Rect, Size, Transform};
use backend::num::NormalizedF32;
use backend::page::PageSettings;
use backend::paint::{Fill, LinearGradient, SpreadMethod, Stop};
use backend::surface::Surface;
use backend::{GroupColorSpace, SerializeSettings};
use craft_pdf::write::{Metadata, Writer, backend};
use hayro_syntax::object::{Array, Dict};

fn cmyk_settings() -> SerializeSettings {
    SerializeSettings { group_color_space: GroupColorSpace::Cmyk, ..Default::default() }
}

/// One page drawn by `draw`, written with CMYK blending.
fn page_with(draw: impl FnOnce(&mut Surface<'_>)) -> Vec<u8> {
    let mut writer = Writer::new(cmyk_settings(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
    let mut surface = page.surface();
    draw(&mut surface);
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

fn stops(colors: [backend::color::Color; 2]) -> Vec<Stop> {
    let [a, b] = colors;
    vec![
        Stop { offset: NormalizedF32::ZERO, color: a, opacity: NormalizedF32::ONE },
        Stop { offset: NormalizedF32::ONE, color: b, opacity: NormalizedF32::ONE },
    ]
}

fn fill_rect(surface: &mut Surface<'_>, x: f32, paint: backend::paint::Paint) {
    let mut path = PathBuilder::new();
    path.push_rect(Rect::from_xywh(x, 10.0, 40.0, 40.0).unwrap());
    surface.set_fill(Some(Fill { paint, ..Default::default() }));
    surface.draw_path(&path.finish().unwrap());
}

fn linear(colors: [backend::color::Color; 2]) -> backend::paint::Paint {
    LinearGradient {
        x1: 0.0,
        y1: 0.0,
        x2: 40.0,
        y2: 0.0,
        transform: Transform::identity(),
        spread_method: SpreadMethod::Pad,
        stops: stops(colors),
        anti_alias: false,
    }
    .into()
}

/// The colors of a shading's function (`/C0` and `/C1` of the first shading's exponential
/// function).
fn shading_ends(bytes: &[u8]) -> (Vec<f32>, Vec<f32>) {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let patterns = &page.resources().patterns;
    let key = patterns.keys().next().unwrap();
    let pattern = patterns.get::<Dict<'_>>(key.as_ref()).unwrap();
    let shading = pattern.get::<Dict<'_>>(b"Shading".as_slice()).unwrap();
    let function = shading.get::<Dict<'_>>(b"Function".as_slice()).unwrap();
    let floats = |key: &[u8]| -> Vec<f32> { function.get::<Array<'_>>(key).unwrap().iter::<f32>().collect() };
    (floats(b"C0"), floats(b"C1"))
}

/// A gray stop in a CMYK gradient becomes the black ink `1 - gray`: gray 0.96 is K 0.04, and the
/// 8-bit gray 128 is K 127/255, as `cmyk::Color::new` writes it.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn gray_stops_keep_their_exact_black_in_cmyk_shadings() {
    let cyan = || -> backend::color::Color { cmyk::Color::new_f32(0.04, 0.0, 0.0, 0.0).into() };
    let bytes = page_with(|surface| fill_rect(surface, 0.0, linear([luma::Color::new_f32(0.96).into(), cyan()])));
    assert_eq!(shading_ends(&bytes), (vec![0.0, 0.0, 0.0, 0.04], vec![0.04, 0.0, 0.0, 0.0]));
    let bytes = page_with(|surface| fill_rect(surface, 0.0, linear([luma::Color::new(128).into(), cyan()])));
    assert_eq!(shading_ends(&bytes).0, vec![0.0, 0.0, 0.0, 127.0 / 255.0]);
}
