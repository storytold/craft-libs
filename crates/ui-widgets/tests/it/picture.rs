//! `Picture` sized in device pixels.

use craft_ui::contract::Show as _;
use craft_ui::display::{Picture, PictureSize};
use craft_ui::runtime::env::Scale;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// The side, in texels, of the picture's image.
const SIDE: usize = 32;

fn image() -> ColorImage {
    ColorImage::new([SIDE, SIDE], vec![Color32::from_rgb(40, 120, 200); SIDE * SIDE])
}

/// A `DevicePixels` picture keeps one texel to a device pixel at every display scale: its size in points times the
/// scale is its texel size.
#[trial]
fn a_device_pixel_picture_keeps_one_texel_to_a_pixel_at_every_scale() {
    for scale in [1.0_f32, 2.0] {
        let size = Rc::new(Cell::new(Vec2::ZERO));
        let seen = size.clone();
        let mut fixture = Fixture::new(move |ui| {
            seen.set(Picture::new("Gradient", 1, &image).extent(PictureSize::DevicePixels).show(ui).rect.size());
        });
        fixture.scale(Scale::new(scale));
        fixture.settle();
        let device = size.get() * scale;
        assert!(
            (device.x - SIDE as f32).abs() < 0.01 && (device.y - SIDE as f32).abs() < 0.01,
            "at scale {scale} the picture is {device:?} device pixels, not {SIDE} square"
        );
    }
}
