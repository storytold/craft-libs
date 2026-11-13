//! The cursor service's images (8.5): a painted pointer reaches the window as its cursor image where the window takes
//! one at its size, and is painted at the pointer otherwise.

use craft_behavior::{Cursor, CursorBadge, CursorIcon, CursorPriority, IconId, PaintedCursor};
use craft_host::target::{Backend, Os, Runtime, ShellInfo};
use craft_ui::runtime::cursor;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, install_spec, trial};
use egui::{CustomCursorImage, Pos2, Vec2};
use std::sync::Arc;

const AT: Pos2 = Pos2::new(300.0, 300.0);

/// Frames on `shell` in a window of `scale` device pixels per point, keeping what they paint.
fn frames(shell: ShellInfo, scale: f32) -> Frames {
    let mut frames = Frames::uninstalled();
    let mut spec = install_spec();
    spec.shell_info = shell;
    craft_ui::install(&frames.ctx, spec).expect("the runtime installs once");
    frames.pixels_per_point(scale);
    frames.record_paint();
    frames
}

/// What one frame with the pointer at `AT` and `requests` sent out: the cursor image, the system cursor and how many
/// shapes it painted (the ghost layer's, as nothing else paints).
struct Seen {
    image: Option<CustomCursorImage>,
    icon: egui::CursorIcon,
    shapes: usize,
}

fn frame(frames: &mut Frames, requests: &[(Cursor, CursorPriority)]) -> Seen {
    frames.frame(vec![moved(AT)], |ui| {
        for (cursor, priority) in requests {
            cursor::request(ui.ctx(), cursor.clone(), *priority);
        }
    });
    let mut paint = frames.take_paint().expect("the frames record their paint");
    paint.textures_delta.clear();
    let shapes = paint.shapes.iter().filter(|s| !matches!(s.shape, egui::Shape::Noop)).count();
    Seen { image: frames.cursor_image().cloned(), icon: frames.cursor_icon(), shapes }
}

fn painted(frames: &mut Frames, cursor: PaintedCursor) -> Seen {
    frame(frames, &[(Cursor::Painted(cursor), CursorPriority::Tool)])
}

fn alpha(image: &CustomCursorImage, x: u16, y: u16) -> u8 {
    image.rgba[(usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4 + 3]
}

fn linux() -> ShellInfo {
    ShellInfo::default().os(Os::Linux).backend(Backend::X11)
}

/// The window takes the image on a native build, Wayland only at scale 1 (compositors that ignore winit's cursor
/// viewport show it too large and off the pointer); elsewhere the pointer is painted and the system cursor hidden.
/// macOS sizes the image in points, so it does not grow with the display's density.
#[trial]
fn a_painted_pointer_is_the_window_cursor_image_where_the_window_takes_one() {
    let wayland = || linux().backend(Backend::Wayland);
    let rows: [(&str, ShellInfo, f32, bool); 8] = [
        ("X11 at 1", linux(), 1.0, true),
        ("X11 at 1.25", linux(), 1.25, true),
        ("Wayland at 1", wayland(), 1.0, true),
        ("Wayland at 1.25", wayland(), 1.25, false),
        ("Wayland at 2", wayland(), 2.0, false),
        ("web", linux().runtime(Runtime::Web), 1.0, false),
        ("macOS at 2", linux().os(Os::Mac), 2.0, true),
        ("Windows at 1.5", linux().os(Os::Windows), 1.5, true),
    ];
    let mut sides = Vec::new();
    for (name, shell, scale, image) in rows {
        let mut frames = frames(shell, scale);
        let seen = painted(&mut frames, PaintedCursor::circle(20.0));
        assert_eq!(seen.image.is_some(), image, "{name}: image");
        if image {
            assert_eq!(seen.shapes, 0, "{name}: an image is not painted as well");
            assert_eq!(seen.icon, egui::CursorIcon::Crosshair, "{name}: a visible system cursor if the integration drops the image");
        } else {
            assert!(seen.shapes > 0, "{name}: the pointer is painted");
            assert_eq!(seen.icon, egui::CursorIcon::None, "{name}: the painted pointer is the only pointer");
        }
        sides.push((name, seen.image.map(|image| image.size[0])));
    }
    let side = |name: &str| sides.iter().find(|(n, _)| *n == name).and_then(|(_, side)| *side).unwrap();
    assert!(side("X11 at 1.25") > side("X11 at 1"), "the image grows with the window's scale");
    assert!(side("macOS at 2") < side("Windows at 1.5"), "macOS takes the image in points");

    // A tip larger than Linux takes hands its centre cross to the window and paints its ring; macOS takes it whole.
    let mut frames_linux = frames(linux(), 1.0);
    let big = painted(&mut frames_linux, PaintedCursor::circle(300.0));
    let marks = big.image.expect("the centre cross still moves with the hardware pointer");
    assert!(marks.size[0] < 100 && big.shapes > 0, "the ring is painted, not shrunk into the image");
    let mut frames_mac = frames(linux().os(Os::Mac), 1.0);
    let whole = painted(&mut frames_mac, PaintedCursor::circle(300.0));
    assert!(whole.image.expect("macOS takes large images").size[0] > 600 && whole.shapes == 0);

    // Every pointer the overlay host draws has an image, apart from the badged system cursor (the OS draws its arrow)
    // and the hidden pointer.
    let mut frames = frames(linux(), 1.0);
    let ring = vec![Pos2::new(-8.0, -4.0), Pos2::new(8.0, -4.0), Pos2::new(0.0, 9.0)];
    for cursor in [
        PaintedCursor::Crosshair,
        PaintedCursor::circle(12.0).marked(true, Some(CursorBadge::Add)),
        PaintedCursor::outline(vec![ring]).marked(true, None),
        PaintedCursor::Hand,
        PaintedCursor::zoom(true),
        PaintedCursor::tool(IconId::new("brush"), Vec2::new(4.0, 20.0), Some(CursorBadge::Copy)),
        PaintedCursor::turn(Vec2::new(1.0, 1.0)),
        PaintedCursor::distort(Vec2::new(-1.0, 0.0)),
        PaintedCursor::skew(Vec2::X),
    ] {
        let seen = painted(&mut frames, cursor.clone());
        assert!(seen.image.is_some() && seen.shapes == 0, "{cursor:?}");
    }
    let badged = painted(&mut frames, PaintedCursor::badged(CursorIcon::Default, CursorBadge::Add));
    assert!(badged.image.is_none() && badged.shapes > 0 && badged.icon == egui::CursorIcon::Default, "the arrow stays the system's");
    let hidden = painted(&mut frames, PaintedCursor::Hidden);
    assert!(hidden.image.is_none() && hidden.shapes == 0 && hidden.icon == egui::CursorIcon::None);
}

/// An unchanged pointer hands the window the same image, so the integration uploads it to the OS once; a new size or
/// scale makes a new one; a frame whose winner is a system cursor, or that asks for nothing, clears it (egui keeps an
/// image until it is replaced).
#[trial]
fn the_cursor_image_is_drawn_once_per_look_and_cleared_when_no_painted_pointer_wins() {
    let mut frames = frames(linux(), 1.0);
    let first = painted(&mut frames, PaintedCursor::circle(20.0)).image.unwrap();
    let moved = painted(&mut frames, PaintedCursor::circle(20.0)).image.unwrap();
    assert!(Arc::ptr_eq(&first.rgba, &moved.rgba), "moving the pointer draws and uploads nothing");
    let larger = painted(&mut frames, PaintedCursor::circle(40.0)).image.unwrap();
    assert!(!Arc::ptr_eq(&first.rgba, &larger.rgba) && larger.size[0] > first.size[0]);

    let hover = frame(
        &mut frames,
        &[
            (Cursor::Painted(PaintedCursor::circle(40.0)), CursorPriority::Tool),
            (Cursor::System(CursorIcon::ResizeHorizontal), CursorPriority::Hover),
        ],
    );
    assert!(hover.image.is_none() && hover.icon == egui::CursorIcon::ResizeHorizontal, "a hovered control's cursor wins over the image");
    let left = frame(&mut frames, &[]);
    assert!(left.image.is_none(), "a pass without a painted pointer keeps no image");

    frames.pixels_per_point(2.0);
    let scaled = painted(&mut frames, PaintedCursor::circle(40.0)).image.unwrap();
    assert!(scaled.size[0] > larger.size[0], "a scale change draws the image again");
}

/// The image is the pointer drawn on the device's pixels: an odd square with the hotspot on its centre pixel, the ring
/// opaque at its radius at every scale, clear inside and in the corners.
#[trial]
fn the_cursor_image_draws_the_pointer_around_its_centre_pixel() {
    for scale in [1.0, 1.5, 2.0] {
        let mut frames = frames(linux(), scale);
        let image = painted(&mut frames, PaintedCursor::circle(20.0)).image.unwrap();
        let [w, h] = image.size;
        let hot = image.hotspot;
        assert!(w == h && w % 2 == 1 && hot == [w / 2, h / 2], "{scale}: {:?} {hot:?}", image.size);
        assert_eq!(image.rgba.len(), usize::from(w) * usize::from(h) * 4);
        let radius = (20.0 * scale).round() as u16;
        for (x, y) in [(hot[0] + radius, hot[1]), (hot[0] - radius, hot[1]), (hot[0], hot[1] + radius)] {
            assert!(alpha(&image, x, y) > 200, "{scale}: the ring at ({x}, {y})");
        }
        assert_eq!(alpha(&image, hot[0], hot[1]), 0, "{scale}: the hotspot shows what is under it");
        assert_eq!(alpha(&image, 0, 0), 0, "{scale}: the corners are clear");
    }
}
