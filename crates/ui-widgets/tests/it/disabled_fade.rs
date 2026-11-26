//! How a disabled component dims what it shows. `DisabledLook::Faded`: a component that shows data paints its enabled
//! look under one veil of the surface it sits on, which covers its faces and the faded children it shows too; a
//! settled region holding one replays the veiled paint. `DisabledLook::States`: the data colours its `Draw` paints dim
//! and keep their hues.

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_theme::{Opacity, Srgba};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{MemoProps, MemoView, Props as _, Show as _};
use craft_ui::list::ThumbAspect;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use craft_ui::style::DataColor;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, MemoRun, last_frame_memo_runs, trial};
use craft_ui::tile_grid::{BadgeKind, BadgeSlot, Tile, TileBadge};
use egui::epaint::ColorMode;
use egui::{Color32, Rect, Shape, Ui, pos2};
use std::cell::Cell;

const REGION: Rect = Rect { min: pos2(10.0, 10.0), max: pos2(210.0, 310.0) };
/// The tile's picture fill.
const PICTURE: Color32 = Color32::from_rgb(0x30, 0x90, 0xd0);

/// A tile with a picture (its `RowThumb` child, faded itself), a text badge (a face), the stars (a face), a colour
/// dot and a caption. Returns the tile's rect.
fn tile(ui: &mut Ui, enabled: bool) -> Rect {
    let rect = Cell::new(Rect::NOTHING);
    in_region(ui, REGION, |ui| {
        let shown = Tile::new("IMG_0002.jpg")
            .thumb(ThumbAspect::SQUARE, |cx, rect| {
                cx.painter.rect_filled(rect, 0.0, PICTURE);
            })
            .badge(TileBadge::new(BadgeSlot::EndTop, BadgeKind::Text(Label::from("1/3")), Tone::Neutral, None))
            .rating(3)
            .label_color(Srgba::rgb(0.86, 0.31, 0.24))
            .enabled(enabled)
            .show(ui);
        rect.set(shown.response.rect);
    });
    rect.get()
}

/// The solid colours `shape` paints with, in a fixed order; a `Shape::Vec` gives its members'.
fn colors_of(shape: &Shape, out: &mut Vec<Color32>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| colors_of(shape, out)),
        Shape::Rect(rect) => out.extend([rect.fill, rect.stroke.color]),
        Shape::Circle(circle) => out.extend([circle.fill, circle.stroke.color]),
        Shape::Ellipse(ellipse) => out.extend([ellipse.fill, ellipse.stroke.color]),
        Shape::Path(path) => {
            out.push(path.fill);
            if let ColorMode::Solid(color) = &path.stroke.color {
                out.push(*color);
            }
        }
        Shape::LineSegment { stroke, .. } => out.push(stroke.color),
        Shape::Mesh(mesh) => out.extend(mesh.vertices.iter().map(|vertex| vertex.color)),
        Shape::Text(text) => out.extend(text.galley.rows.iter().flat_map(|row| row.row.visuals.mesh.vertices.iter().map(|vertex| vertex.color))),
        _ => {}
    }
}

/// The solid colours of each shape.
fn colors(shapes: &[Shape]) -> Vec<Vec<Color32>> {
    shapes
        .iter()
        .map(|shape| {
            let mut out = Vec::new();
            colors_of(shape, &mut out);
            out
        })
        .collect()
}

/// Whether `shape` has the look of a veil: a plain filled rect at the veil's alpha.
fn veil_like(shape: &Shape) -> bool {
    let alpha = Color32::WHITE.gamma_multiply(1.0 - Opacity::DISABLED).a();
    matches!(shape, Shape::Rect(rect) if rect.brush.is_none() && rect.fill.a() == alpha)
}

/// The shapes of a frame once the scene shows the same thing two frames running.
fn settled_shapes(frames: &mut Frames, mut scene: impl FnMut(&mut Ui)) -> Vec<Shape> {
    frames.idle(4, &mut scene);
    frames.frame_shapes(&mut scene)
}

/// A disabled tile paints the shapes and colours it paints enabled, its picture's included, and one veil over them,
/// after its last shape; the faded picture it shows adds no veil of its own. A disabled button keeps its token look
/// and paints nothing faded.
#[trial]
fn a_disabled_faded_component_paints_its_enabled_look_faded_once() {
    let enabled = settled_shapes(&mut Frames::new(), |ui| {
        tile(ui, true);
    });
    let at = Cell::new(Rect::NOTHING);
    let disabled = settled_shapes(&mut Frames::new(), |ui| at.set(tile(ui, false)));
    let shown = at.get().intersect(REGION);
    let veils: Vec<usize> = disabled.iter().enumerate().filter(|(_, shape)| veil_like(shape)).map(|(index, _)| index).collect();
    let before = enabled.iter().filter(|shape| veil_like(shape)).count();
    assert_eq!(veils.len(), before + 1, "the disabled tile adds one veil, and its faded picture none: {veils:?}");
    let veil = *veils.last().unwrap_or_else(|| panic!("the disabled tile paints a veil"));
    let Shape::Rect(rect) = &disabled[veil] else { panic!("the veil is a rect") };
    assert!(rect.rect.contains_rect(shown), "the veil {:?} covers the tile {shown:?}", rect.rect);
    assert!(
        disabled[veil + 1..].iter().all(|shape| !shape.visual_bounding_rect().intersects(shown)),
        "the veil is painted after the tile's last shape"
    );
    let mut own = disabled.clone();
    let _veil = own.remove(veil);
    let (own, enabled) = (colors(&own), colors(&enabled));
    assert_eq!(own, enabled, "under the veil the disabled tile paints its enabled colours");
    let painting = own.iter().filter(|shape| shape.iter().any(|color| color.a() > 0)).count();
    assert!(painting > 5, "the picture, the badge, the stars, the dot and the caption each paint: {painting}");
    assert!(own.iter().flatten().any(|color| *color == PICTURE), "the picture paints its own colour under the veil");
    let button = settled_shapes(&mut Frames::new(), |ui| {
        in_region(ui, REGION, |ui| {
            Button::new("Apply").enabled(false).show(ui);
        });
    });
    assert!(
        button.iter().filter_map(|shape| if let Shape::Text(text) = shape { Some(text) } else { None }).all(|text| text.opacity_factor == 1.0),
        "a disabled button is not faded"
    );
}

/// A view holding a disabled tile.
#[derive(MemoProps)]
struct Faded {}

fn faded(_: &mut Faded, ui: &mut Ui) {
    tile(ui, false);
}

/// The runs of every view in the last frame.
fn view_runs(frames: &Frames) -> usize {
    last_frame_memo_runs(&frames.ctx).iter().filter(|(_, _, run)| *run == MemoRun::Ran).count()
}

/// A settled memoized view holding a disabled faded tile runs nothing, and replays the veiled paint of the frame that
/// ran.
#[trial]
fn a_settled_view_replays_a_faded_component_without_running_it() {
    let scene = |ui: &mut Ui| {
        MemoView::new("faded", Faded {}, faded).show(ui);
    };
    let ran = colors(&settled_shapes(&mut Frames::new(), |ui| {
        tile(ui, false);
    }));
    let mut frames = Frames::new();
    frames.frame(vec![], scene);
    assert!(view_runs(&frames) > 0, "the first frame runs the view");
    frames.idle(30, scene);
    for frame in 0..4 {
        let shapes = frames.frame_shapes(scene);
        assert_eq!(view_runs(&frames), 0, "settled frame {frame} ran the view");
        assert_eq!(colors(&shapes), ran, "settled frame {frame} paints the veiled colours a run paints");
    }
}

/// The fill of the badge an icon button paints on its icon in `color`.
fn badge_fill(enabled: bool, color: Color32) -> Color32 {
    let shapes = settled_shapes(&mut Frames::new(), |ui| {
        in_region(ui, REGION, |ui| {
            IconButton::new(IconSpec::svg(IconId::new("eyedropper")).badge(DataColor::new(color))).label("Pick").enabled(enabled).show(ui);
        });
    });
    shapes.iter().find_map(|shape| if let Shape::Circle(circle) = shape { Some(circle.fill) } else { None }).expect("the badge paints")
}

/// A disabled button paints its icon's badge, a data colour, dimmer than an enabled one, and two badge colours stay
/// two colours (not the one disabled fill).
#[trial]
fn a_disabled_button_dims_its_badge_and_keeps_its_hue() {
    let (black, white) = (Color32::from_gray(0), Color32::from_gray(255));
    let enabled = badge_fill(true, white);
    let disabled = badge_fill(false, white);
    assert!(disabled.a() < enabled.a(), "the disabled badge {disabled:?} is dimmer than the enabled {enabled:?}");
    assert_ne!(badge_fill(false, black), disabled, "two disabled badges keep their own colours");
}
