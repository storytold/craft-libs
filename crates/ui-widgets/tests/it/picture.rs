//! `Picture` sized in device pixels.

use craft_ui::contract::Show as _;
use craft_ui::display::{Picture, PictureSize};
use craft_ui::list::ThumbAspect;
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

/// A painter picture takes the size the caller gives, not the icon side: it is as wide and high as asked, and as large
/// as a larger ask, so it does not collapse to the control's icon.
#[trial]
fn a_painter_picture_takes_the_size_in_points_it_is_given() {
    let seen = Rc::new(Cell::new((Vec2::ZERO, Vec2::ZERO, Vec2::ZERO)));
    let out = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut draw = |size| Picture::painter("Preview", size, ThumbAspect::new(300, 100), |_, _| ()).show(ui).rect.size();
        out.set((draw(PictureSize::Icon), draw(PictureSize::Points(100)), draw(PictureSize::Points(50))));
    });
    fixture.settle();
    let (icon, tall, short) = seen.get();
    assert!((tall - Vec2::new(300.0, 100.0)).abs().max_elem() < 0.01, "a 300x100 point painter is {tall:?}");
    assert!((short.x * 2.0 - tall.x).abs() < 0.01 && short.y < tall.y, "halving the height halves the width: {short:?} vs {tall:?}");
    assert!(tall.y > icon.y, "an explicit size is not the icon side: {tall:?} vs {icon:?}");
}
