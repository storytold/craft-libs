//! `Picture` sized in device pixels.

use craft_ui::contract::{Props as _, Show as _};
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

/// A `Fill` painter picture takes the room its parent gives: wider in a wider room, never wider than the room, and
/// as high as its aspect makes it at that width.
#[trial]
fn a_fill_painter_picture_takes_the_room_it_is_given_at_its_aspect() {
    let rooms = [120.0_f32, 300.0];
    let seen = Rc::new(Cell::new([Vec2::ZERO; 2]));
    let out = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut sizes = [Vec2::ZERO; 2];
        for (size, room) in sizes.iter_mut().zip(rooms) {
            ui.allocate_ui(Vec2::new(room, 400.0), |ui| {
                ui.set_max_width(room);
                *size = Picture::painter("Preview", ThumbAspect::new(3, 1), |_, _| ())
                    .extent(PictureSize::Fill)
                    .id_salt(room.to_bits())
                    .show(ui)
                    .rect
                    .size();
            });
        }
        out.set(sizes);
    });
    fixture.settle();
    let [narrow, wide] = seen.get();
    assert!(wide.x > narrow.x + 1.0, "a wider room gives a wider picture: {wide:?} against {narrow:?}");
    for (size, room) in [narrow, wide].into_iter().zip(rooms) {
        assert!(size.x <= room + 0.01, "{size:?} fits a room {room} wide");
        assert!((size.x / size.y - 3.0).abs() < 0.05, "{size:?} keeps the 3:1 aspect");
    }
}
