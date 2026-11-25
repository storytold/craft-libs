//! The cursor service's images (8.5): a painted pointer reaches the window as its cursor image where the window takes
//! one at its size, and is painted at the pointer otherwise.

use craft_behavior::{CountMark, Cursor, CursorBadge, CursorIcon, CursorPriority, IconId, PaintedCursor, PointerGlyph};
use craft_host::target::{Backend, Caps, Os, Runtime, ShellInfo};
use craft_ui::runtime::cursor;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, install_spec, trial};
use egui::{CustomCursorImage, Pos2, Rect, Vec2, ViewportId};
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
    frame_in(frames, ViewportId::ROOT, requests)
}

/// [`frame`] in the window `viewport`.
fn frame_in(frames: &mut Frames, viewport: ViewportId, requests: &[(Cursor, CursorPriority)]) -> Seen {
    frames.frame_in_viewport(viewport, vec![moved(AT)], |ui| {
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

    // Only the root window takes an image: the framework's other windows are egui's immediate viewports, which drop
    // it, so a child window paints its pointer.
    let mut frames_child = frames(linux(), 1.0);
    painted(&mut frames_child, PaintedCursor::circle(20.0));
    let child = frame_in(
        &mut frames_child,
        ViewportId::from_hash_of("child window"),
        &[(Cursor::Painted(PaintedCursor::circle(20.0)), CursorPriority::Tool)],
    );
    assert!(child.image.is_none() && child.shapes > 0 && child.icon == egui::CursorIcon::None, "a child window paints its pointer");

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
        PaintedCursor::type_on(false),
        PaintedCursor::type_on(true),
        PaintedCursor::Locator,
        PaintedCursor::arrow(false),
        PaintedCursor::arrow(true),
        PaintedCursor::path_text(true, false),
        PaintedCursor::pointer_tool(PointerGlyph::Crosshair, IconId::new("crop")),
        PaintedCursor::pointer_tool(PointerGlyph::Arrow, IconId::new("paint_bucket")),
    ] {
        let seen = painted(&mut frames, cursor.clone());
        assert!(seen.image.is_some() && seen.shapes == 0, "{cursor:?}");
    }
    let badged = painted(&mut frames, PaintedCursor::badged(CursorIcon::Default, CursorBadge::Add));
    assert!(badged.image.is_none() && badged.shapes > 0 && badged.icon == egui::CursorIcon::Default, "the arrow stays the system's");
    // The enums are non-exhaustive in another crate, so a badge without art would compile and show nothing.
    for badge in [
        CursorBadge::Move,
        CursorBadge::Arrow,
        CursorBadge::Target,
        CursorBadge::Continue,
        CursorBadge::Count(CountMark::Add),
        CursorBadge::Count(CountMark::Move),
        CursorBadge::Count(CountMark::Delete),
        CursorBadge::Corner,
    ] {
        let seen = painted(&mut frames, PaintedCursor::badged(CursorIcon::Default, badge));
        assert!(seen.shapes > 0, "{badge:?} has marks");
    }
    let hidden = painted(&mut frames, PaintedCursor::Hidden);
    assert!(hidden.image.is_none() && hidden.shapes == 0 && hidden.icon == egui::CursorIcon::None);
}

/// Sa-9 (4): a shell that cannot set the system cursor (Wayland with no cursor theme, where a hidden pointer never
/// comes back) keeps the system pointer under every painted cursor and paints none, in the main window and another.
#[trial]
fn a_shell_that_cannot_set_the_system_cursor_keeps_the_system_pointer() {
    let wayland = linux().backend(Backend::Wayland);
    let without = wayland.clone().caps(wayland.caps.difference(Caps::SYSTEM_CURSOR));
    for cursor in [PaintedCursor::circle(20.0), PaintedCursor::circle(300.0), PaintedCursor::Hand, PaintedCursor::Hidden] {
        let mut with = frames(wayland.clone(), 1.25);
        let seen = painted(&mut with, cursor.clone());
        assert_eq!(seen.icon, egui::CursorIcon::None, "{cursor:?}: with the system cursor the painted one stands in for it");

        let mut kept = frames(without.clone(), 1.0);
        for viewport in [ViewportId::ROOT, ViewportId::from_hash_of("child window")] {
            let seen = frame_in(&mut kept, viewport, &[(Cursor::Painted(cursor.clone()), CursorPriority::Tool)]);
            assert_ne!(seen.icon, egui::CursorIcon::None, "{cursor:?} in {viewport:?}: the system pointer stays");
            assert!(seen.image.is_none() && seen.shapes == 0, "{cursor:?} in {viewport:?}: no tool cursor is drawn");
        }
    }
}

/// A tool icon rides below and right of its pointer: beside the plain pointer, the image's marks reach as far left and up
/// from the hotspot (the arrow's tip, the crosshair's centre) and farther right and down; an icon the catalog lacks
/// leaves the plain pointer.
#[trial]
fn a_tool_icon_rides_below_right_of_its_pointer() {
    // How far the opaque pixels reach from the hotspot: left, up, right, down.
    fn reach(image: &CustomCursorImage) -> [i32; 4] {
        let (side, [hx, hy]) = (i32::from(image.size[0]), image.hotspot.map(i32::from));
        let mut reach = [0; 4];
        for (i, px) in image.rgba.chunks(4).enumerate() {
            if px[3] > 0 {
                let (dx, dy) = (i as i32 % side - hx, i as i32 / side - hy);
                reach = [reach[0].max(-dx), reach[1].max(-dy), reach[2].max(dx), reach[3].max(dy)];
            }
        }
        reach
    }
    let mut frames = frames(linux(), 1.0);
    for (plain, pointer) in [(PaintedCursor::arrow(false), PointerGlyph::Arrow), (PaintedCursor::Crosshair, PointerGlyph::Crosshair)] {
        let plain = reach(&painted(&mut frames, plain).image.unwrap());
        for icon in ["paint_bucket", "ruler", "protractor"] {
            let with = reach(&painted(&mut frames, PaintedCursor::pointer_tool(pointer, IconId::new(icon))).image.unwrap());
            assert_eq!(with[..2], plain[..2], "{pointer:?} {icon}: nothing reaches farther left or up");
            assert!(with[2] > plain[2] && with[3] > plain[3], "{pointer:?} {icon}: the icon lies below right");
        }
        let missing = reach(&painted(&mut frames, PaintedCursor::pointer_tool(pointer, IconId::new("no_such_icon"))).image.unwrap());
        assert_eq!(missing, plain, "{pointer:?}: only the pointer");
    }
}

/// An arrow with a tool icon keeps ink on its hotspot (the arrow's tip), and a badge lands below right of the icon: the
/// marks reach no farther left or up than without it, and farther right (photocraft#16671).
#[trial]
fn a_badged_tool_pointer_keeps_its_tip_on_the_hotspot_and_the_badge_below_right() {
    fn reach(image: &CustomCursorImage) -> [i32; 4] {
        let (side, [hx, hy]) = (i32::from(image.size[0]), image.hotspot.map(i32::from));
        let mut reach = [0; 4];
        for (i, px) in image.rgba.chunks(4).enumerate() {
            if px[3] > 0 {
                let (dx, dy) = (i as i32 % side - hx, i as i32 / side - hy);
                reach = [reach[0].max(-dx), reach[1].max(-dy), reach[2].max(dx), reach[3].max(dy)];
            }
        }
        reach
    }
    for scale in [1.0, 1.25, 2.0] {
        let mut frames = frames(linux(), scale);
        let plain = painted(&mut frames, PaintedCursor::pointer_tool(PointerGlyph::Arrow, IconId::new("lasso"))).image.unwrap();
        let [hx, hy] = plain.hotspot.map(i32::from);
        assert!(pixel(&plain, hx, hy)[3] > 0, "{scale}: the arrow's tip is on the hotspot");
        for badge in [CursorBadge::Add, CursorBadge::Subtract, CursorBadge::Intersect] {
            let badged = PaintedCursor::pointer_tool_badged(PointerGlyph::Arrow, IconId::new("lasso"), Some(badge));
            let image = painted(&mut frames, badged).image.unwrap();
            let [hx, hy] = image.hotspot.map(i32::from);
            assert!(pixel(&image, hx, hy)[3] > 0, "{scale} {badge:?}: the arrow's tip is on the hotspot");
            let (with, without) = (reach(&image), reach(&plain));
            assert_eq!(with[..2], without[..2], "{scale} {badge:?}: nothing reaches farther left or up");
            // A minus lies on the icon's bottom edge, so only the plus and the cross reach lower.
            assert!(with[2] > without[2] && with[3] >= without[3], "{scale} {badge:?}: the badge lies at the icon's lower right");
        }
        let none = PaintedCursor::pointer_tool_badged(PointerGlyph::Arrow, IconId::new("lasso"), None);
        assert_eq!(reach(&painted(&mut frames, none).image.unwrap()), reach(&plain), "{scale}: no badge is the plain pointer");
    }
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

/// The straight RGBA of the pixel at (`x`, `y`).
fn pixel(image: &CustomCursorImage, x: i32, y: i32) -> [u8; 4] {
    let at = (y as usize * usize::from(image.size[0]) + x as usize) * 4;
    [image.rgba[at], image.rgba[at + 1], image.rgba[at + 2], image.rgba[at + 3]]
}

/// The I-beam is two colours, so it shows on light and dark grounds, and its stem and serifs lie on whole device pixels
/// at every scale: a stroke is never split over two pixels, which would show as grey fringes of its own.
#[trial]
fn the_painted_i_beam_shows_on_light_and_dark_and_keeps_whole_pixel_strokes() {
    for scale in [1.0, 1.25, 1.5] {
        let mut frames = frames(linux(), scale);
        let beam = painted(&mut frames, PaintedCursor::IBeam).image.expect("the I-beam is a window image");
        let [hx, hy] = beam.hotspot.map(i32::from);
        let side = i32::from(beam.size[0]);
        // The stem, across the middle row: one run of pixels with the stroke between halo on both sides.
        let row: Vec<[u8; 4]> = (0..side).map(|x| pixel(&beam, x, hy)).collect();
        let first = row.iter().position(|p| p[3] > 0).expect("the stem is drawn") as i32;
        let last = row.iter().rposition(|p| p[3] > 0).expect("the stem is drawn") as i32;
        assert!(first < hx && hx < last, "{scale}: halo on both sides of the stem: {first}..{last} about {hx}");
        assert!(row[first as usize..=last as usize].iter().all(|p| p[3] > 0), "{scale}: the stem is one run");
        assert_eq!(hx - first, last - hx, "{scale}: the halo is as wide on the left as on the right");
        assert_ne!(row[hx as usize][..3], row[first as usize][..3], "{scale}: the stroke and its halo differ in colour");
        assert_eq!(row[first as usize], row[last as usize], "{scale}: the halo is the same on both sides");
        let mut alphas: Vec<u8> = row.iter().map(|p| p[3]).filter(|a| *a > 0).collect();
        alphas.sort_unstable();
        alphas.dedup();
        assert!(alphas.len() <= 2, "{scale}: no partly covered column across the stem: {alphas:?}");
        // The serifs, down a column beside the stem: two runs, each of whole pixels.
        let x = hx + (2.0 * scale).round() as i32;
        let column: Vec<[u8; 4]> = (0..side).map(|y| pixel(&beam, x, y)).collect();
        let runs = column.windows(2).filter(|w| w[0][3] == 0 && w[1][3] > 0).count() + usize::from(column[0][3] > 0);
        assert_eq!(runs, 2, "{scale}: a serif at each end");
        let mut alphas: Vec<u8> = column.iter().map(|p| p[3]).filter(|a| *a > 0).collect();
        alphas.sort_unstable();
        alphas.dedup();
        assert!(alphas.len() <= 2, "{scale}: no partly covered row across a serif: {alphas:?}");
        // The path-text pointer with no grabbable end is the beam alone, and the type pointer draws the same stem.
        let bare = painted(&mut frames, PaintedCursor::path_text(false, false)).image.unwrap();
        assert_eq!((bare.size, &bare.rgba), (beam.size, &beam.rgba), "{scale}: path text shares the beam");
        let typed = painted(&mut frames, PaintedCursor::type_on(false)).image.unwrap();
        let [tx, ty] = typed.hotspot.map(i32::from);
        for dx in -2..=2 {
            assert_eq!(pixel(&typed, tx + dx, ty), pixel(&beam, hx + dx, hy), "{scale}: the type pointer's stem at {dx}");
        }
    }
}

/// An arrow turned toward a direction keeps its tip on the hotspot and its body behind the tip, along the opposite
/// direction, whichever way it points.
#[trial]
fn a_turned_arrow_points_along_its_direction_about_the_hotspot() {
    let mut frames = frames(linux(), 1.0);
    let mut seen: Vec<CustomCursorImage> = Vec::new();
    for dir in [Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(-1.0, 0.0), Vec2::new(-1.0, -1.0)] {
        let image = painted(&mut frames, PaintedCursor::arrow_toward(dir)).image.expect("the arrow is a window image");
        let [hx, hy] = image.hotspot.map(i32::from);
        let side = i32::from(image.size[0]);
        assert!(pixel(&image, hx, hy)[3] > 0, "{dir:?}: the tip is on the hotspot");
        // The centroid of what is drawn, from the hotspot.
        let (mut sum, mut count) = (Vec2::ZERO, 0.0f32);
        for y in 0..side {
            for x in 0..side {
                if pixel(&image, x, y)[3] > 0 {
                    sum += Vec2::new((x - hx) as f32, (y - hy) as f32);
                    count += 1.0;
                }
            }
        }
        let behind = (sum / count).normalized().dot(-dir.normalized());
        assert!(behind > 0.9, "{dir:?}: the body lies behind the tip, cosine {behind}");
        assert!(seen.iter().all(|other| other.rgba != image.rgba), "{dir:?}: the art differs from the other directions");
        seen.push(image);
    }
}

/// A brush tip outline requested within a region (a canvas's view on the canvas's layer) is a mark on that canvas:
/// the window takes it whole only while it lies inside the region with no layer over it. At the region's edge, or
/// under a layer above the region's, its ring paints on the region's layer clipped to the region, under that layer,
/// and the window takes only its marks.
#[trial]
fn a_tip_outline_shows_only_inside_its_region() {
    const RADIUS: f32 = 40.0;
    let canvas = egui::LayerId::background();
    let cover = egui::Id::unique("cover");
    // One frame with the pointer at `AT`, the tip requested within `clip`, and an area over `over` when given: the
    // cursor image, and each painted shape's clip with whether it is the ring or the area's fill, in paint order.
    let run = |frames: &mut Frames, clip: Rect, over: Option<Rect>| {
        frames.frame_in_viewport(ViewportId::ROOT, vec![moved(AT)], |ui| {
            let region = cursor::CursorRegion::new(canvas, clip);
            cursor::request_within(ui.ctx(), Cursor::Painted(PaintedCursor::circle(RADIUS).marked(true, None)), CursorPriority::Tool, region);
            if let Some(over) = over {
                egui::Area::new(cover).fixed_pos(over.min).show(ui.ctx(), |ui| {
                    ui.allocate_space(over.size());
                    ui.painter().rect_filled(over, 0.0, egui::Color32::RED);
                });
            }
        });
        let mut paint = frames.take_paint().expect("the frames record their paint");
        paint.textures_delta.clear();
        let shapes: Vec<(Rect, Option<bool>)> = paint
            .shapes
            .iter()
            .map(|clipped| {
                let kind = match &clipped.shape {
                    egui::Shape::Circle(circle) if circle.radius > RADIUS / 2.0 => Some(true),
                    // The area fades in, so its red fill is scaled down.
                    egui::Shape::Rect(rect) if rect.fill.r() > 0 && rect.fill.g() == 0 && rect.fill.b() == 0 => Some(false),
                    _ => None,
                };
                (clipped.clip_rect, kind)
            })
            .collect();
        (frames.cursor_image().cloned(), shapes)
    };
    let rings = |shapes: &[(Rect, Option<bool>)]| shapes.iter().filter(|(_, kind)| *kind == Some(true)).map(|(clip, _)| *clip).collect::<Vec<_>>();
    let mut frames = frames(linux(), 1.0);

    // Inside the region with nothing over it: the whole outline moves with the hardware pointer.
    let room = Rect::from_center_size(AT, Vec2::splat(400.0));
    let (image, shapes) = run(&mut frames, room, None);
    let whole = image.expect("the whole outline is the window's image").size[0];
    assert!(rings(&shapes).is_empty(), "the image is not painted as well");

    // The region's edge runs under the ring: the ring paints clipped to the region; the window takes the marks.
    let edge = Rect::from_min_max(Pos2::new(AT.x - RADIUS / 2.0, AT.y - 200.0), Pos2::new(AT.x + 200.0, AT.y + 200.0));
    let (image, shapes) = run(&mut frames, edge, None);
    assert!(image.expect("the marks still move with the hardware pointer").size[0] < whole, "the image holds only the marks");
    let clips = rings(&shapes);
    assert!(!clips.is_empty() && clips.iter().all(|clip| edge.contains_rect(*clip)), "the ring is cut off at the region's edge: {clips:?}");

    // A layer over part of the ring: the ring paints under it.
    let over = Rect::from_min_size(Pos2::new(AT.x + RADIUS / 2.0, AT.y - 100.0), Vec2::new(100.0, 200.0));
    // egui shows a new area from its second frame.
    run(&mut frames, room, Some(over));
    let (image, shapes) = run(&mut frames, room, Some(over));
    assert!(image.expect("the marks").size[0] < whole, "a covered outline is not the window's image");
    let ring_at = shapes.iter().position(|(_, kind)| *kind == Some(true)).expect("the ring is painted");
    let cover_at = shapes.iter().position(|(_, kind)| *kind == Some(false)).expect("the area is painted");
    assert!(ring_at < cover_at, "the ring lies under the layer above the canvas");

    // A request with no region keeps the pointer's own path at any position.
    let seen = painted(&mut frames, PaintedCursor::circle(RADIUS));
    assert!(seen.image.is_some() && seen.shapes == 0, "an outline with no region is the window's image");
}
