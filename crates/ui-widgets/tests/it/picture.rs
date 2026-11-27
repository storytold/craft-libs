//! `Picture`: its SVG source on hostile input, and sizing in device pixels.

use craft_ui::contract::{Props as _, Show as _};
use craft_ui::display::{Picture, PictureSize};
use craft_ui::list::ThumbAspect;
use craft_ui::runtime::env::Scale;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Rect, Shape, Vec2, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A small valid drawing.
const MARK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="#2a7"/></svg>"##;

/// A 4:1 drawing.
const WIDE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 20"><rect width="80" height="20" fill="#36c"/></svg>"##;

/// An SVG picture with no aspect is as wide as its view box says; an aspect set explicitly wins.
#[trial]
fn an_svg_picture_without_an_aspect_takes_its_view_box_aspect() {
    use craft_ui::list::ThumbAspect;
    let mut fixture = Fixture::new(|ui| {
        Picture::svg("Wide", WIDE).show(ui);
        Picture::svg("Square", WIDE).aspect(ThumbAspect::SQUARE).show(ui);
    });
    fixture.room(Vec2::new(600.0, 300.0)).settle();
    let wide = fixture.by_label("Wide").expect("the wide picture's node").rect;
    let square = fixture.by_label("Square").expect("the square picture's node").rect;
    assert!((wide.width() / wide.height() - 4.0).abs() < 0.05, "a 4:1 drawing is four times wider than tall: {wide:?}");
    assert!((square.width() - square.height()).abs() < 0.05, "an explicit aspect wins: {square:?}");
}

/// The rects textured shapes cover in one painted frame; a picture with square corners paints a textured mesh.
fn pictures(shapes: &[Shape]) -> Vec<Rect> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => Some(mesh.calc_bounds()),
            _ => None,
        })
        .collect()
}

/// A malformed SVG, a gzip stream, a document with a DTD entity, one past the node limit and a picture past the side
/// cap each draw nothing, frame after frame, and never panic, while a valid mark beside them draws.
#[trial]
fn a_malformed_or_oversized_svg_draws_nothing_and_never_panics() {
    let mut nodes = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">"#);
    for _ in 0..12_000 {
        nodes.push_str("<g/>");
    }
    nodes.push_str("</svg>");
    let too_many_nodes: &'static [u8] = Box::leak(nodes.into_bytes().into_boxed_slice());
    let refused: [(&str, &'static [u8], Rect); 5] = [
        ("Malformed", b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=", Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 40.0))),
        ("Gzip", b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03\x03\x00\x00\x00\x00\x00\x00\x00\x00\x00", Rect::from_min_size(pos2(60.0, 10.0), vec2(40.0, 40.0))),
        (
            "Entity",
            br#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa">]><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><text>&a;</text></svg>"#,
            Rect::from_min_size(pos2(110.0, 10.0), vec2(40.0, 40.0)),
        ),
        ("Nodes", too_many_nodes, Rect::from_min_size(pos2(160.0, 10.0), vec2(40.0, 40.0))),
        // Wider than the side cap of 4096 physical pixels at one pixel per point.
        ("Huge", MARK, Rect::from_min_size(pos2(10.0, 120.0), vec2(4200.0, 40.0))),
    ];
    let valid = Rect::from_min_size(pos2(210.0, 10.0), vec2(40.0, 40.0));
    let mut fixture = Fixture::new(move |ui| {
        for (label, svg, rect) in refused {
            Picture::svg(label, svg).show_at(ui, rect);
        }
        Picture::svg("Valid", MARK).show_at(ui, valid);
    });
    fixture.room(Vec2::new(4400.0, 300.0));
    for frame in 0..3 {
        let drawn = pictures(&fixture.frame_shapes());
        for (label, _, rect) in refused {
            assert!(!drawn.iter().any(|picture| rect.expand(0.5).contains_rect(*picture)), "frame {frame}: {label} draws a picture: {drawn:?}");
        }
        let marks = drawn.iter().filter(|picture| valid.expand(0.5).contains_rect(**picture)).count();
        assert_eq!(marks, 1, "frame {frame}: the valid mark draws one picture: {drawn:?}");
    }
}

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

/// How many shapes a rendered picture of a transparent image paints: as built, or opted out of its transparency grid.
fn shapes_of_rendered(opt_out: bool) -> usize {
    use craft_ui::list::ThumbSize;
    use craft_ui::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
    let mut fixture = Fixture::new(move |ui| {
        let options = ThumbnailOptions::new().jobs(std::sync::Arc::new(craft_host::InlineJobs));
        let thumbnails = Thumbnails::<u64>::in_context(ui.ctx(), "picture_checker_test", options);
        let source = FakeThumbSource::<u64>::new();
        let picture = Picture::rendered("Mask", 1, &thumbnails, &source).extent(PictureSize::Thumb(ThumbSize::Large));
        // The default case sets nothing, so a default that turned the grid off fails here.
        if opt_out { picture.checker(false) } else { picture }.show(ui);
    });
    fixture.settle();
    fixture.frame_shapes().len()
}

#[trial]
fn a_rendered_picture_paints_a_checker_unless_it_opts_out() {
    let with = shapes_of_rendered(false);
    let without = shapes_of_rendered(true);
    assert!(with > without, "the default paints {with} shapes, the opt-out {without}: the grid is the difference");
}
