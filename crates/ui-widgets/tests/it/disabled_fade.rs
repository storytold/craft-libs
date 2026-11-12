//! How a disabled component dims what it shows. `DisabledLook::Faded`: a component that shows data paints its enabled
//! look at `Opacity::DISABLED`, its faces and the faded children it shows each faded once; a settled region holding one
//! replays the faded paint. `DisabledLook::States`: the data colours its `Draw` paints dim and keep their hues.

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_theme::{Opacity, Srgba};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Lazy, Props as _, Show as _};
use craft_ui::list::ThumbAspect;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use craft_ui::style::DataColor;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use craft_ui::tile_grid::{BadgeKind, BadgeSlot, Tile, TileBadge};
use egui::epaint::ColorMode;
use egui::{Color32, Rect, Shape, Ui, pos2};
use std::cell::Cell;

const REGION: Rect = Rect { min: pos2(10.0, 10.0), max: pos2(210.0, 310.0) };

/// A tile with a picture (its `RowThumb` child), a text badge (a face), the stars (a face), a colour dot and a caption.
fn tile(ui: &mut Ui, enabled: bool) {
    in_region(ui, REGION, |ui| {
        Tile::new("IMG_0002.jpg")
            .thumb(ThumbAspect::SQUARE, |cx, rect| {
                cx.painter.rect_filled(rect, 0.0, Color32::from_rgb(0x30, 0x90, 0xd0));
            })
            .badge(TileBadge::new(BadgeSlot::EndTop, BadgeKind::Text(Label::from("1/3")), Tone::Neutral, None))
            .rating(3)
            .label_color(Srgba::rgb(0.86, 0.31, 0.24))
            .enabled(enabled)
            .show(ui);
    });
}

/// The strongest alpha among the colours `shape` paints with, or `None` for a shape that paints none.
fn alpha(shape: &Shape) -> Option<u8> {
    let solid = |mode: &ColorMode| match mode {
        ColorMode::Solid(color) => color.a(),
        ColorMode::UV(_) => 0,
    };
    let alphas: Vec<u8> = match shape {
        Shape::Rect(rect) => vec![rect.fill.a(), rect.stroke.color.a()],
        Shape::Circle(circle) => vec![circle.fill.a(), circle.stroke.color.a()],
        Shape::Ellipse(ellipse) => vec![ellipse.fill.a(), ellipse.stroke.color.a()],
        Shape::Path(path) => vec![path.fill.a(), solid(&path.stroke.color)],
        Shape::LineSegment { stroke, .. } => vec![stroke.color.a()],
        Shape::Mesh(mesh) => mesh.vertices.iter().map(|vertex| vertex.color.a()).collect(),
        Shape::Text(text) => text.galley.rows.iter().flat_map(|row| row.row.visuals.mesh.vertices.iter().map(|vertex| vertex.color.a())).collect(),
        _ => Vec::new(),
    };
    alphas.into_iter().max().filter(|a| *a > 0)
}

fn alphas(shapes: &[Shape]) -> Vec<Option<u8>> {
    shapes.iter().map(alpha).collect()
}

/// The shapes of a frame once the scene shows the same thing two frames running.
fn settled_shapes(frames: &mut Frames, mut scene: impl FnMut(&mut Ui)) -> Vec<Shape> {
    frames.idle(4, &mut scene);
    frames.frame_shapes(&mut scene)
}

/// Every shape a disabled tile paints is `Opacity::DISABLED` as opaque as the same shape enabled: its own paint, its
/// faces and its picture, which the `RowThumb` child fades itself. Nothing is faded twice, and the text keeps its
/// enabled colours. A disabled button keeps its token look and paints nothing faded.
#[trial]
fn a_disabled_faded_component_paints_its_enabled_look_faded_once() {
    let enabled = settled_shapes(&mut Frames::new(), |ui| tile(ui, true));
    let disabled = settled_shapes(&mut Frames::new(), |ui| tile(ui, false));
    assert_eq!(enabled.len(), disabled.len(), "a disabled tile paints the shapes it paints enabled");
    let mut faded = 0;
    for (at, (on, off)) in alphas(&enabled).into_iter().zip(alphas(&disabled)).enumerate() {
        let Some(on) = on else { continue };
        let off = f32::from(off.unwrap_or(0));
        let expected = f32::from(on) * Opacity::DISABLED;
        assert!((off - expected).abs() <= 1.5, "shape {at}: alpha {off} disabled, {on} enabled: {:?}", disabled[at]);
        faded += 1;
    }
    assert!(faded > 5, "the picture, the badge, the stars, the dot and the caption each paint: {faded}");
    let fallbacks = |shapes: &[Shape]| -> Vec<Color32> {
        shapes.iter().filter_map(|shape| if let Shape::Text(text) = shape { Some(text.fallback_color.to_opaque()) } else { None }).collect()
    };
    assert_eq!(fallbacks(&enabled), fallbacks(&disabled), "the disabled tile's text keeps its enabled ink");
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

/// A settled region holding a disabled faded tile runs nothing, and replays the faded paint of the frame that ran.
#[trial]
fn a_settled_region_replays_a_faded_component_without_running_it() {
    let runs = Cell::new(0);
    let scene = |ui: &mut Ui| {
        Lazy::new("faded", 0.0).settles(true).show(ui, |ui| {
            runs.set(runs.get() + 1);
            tile(ui, false);
        });
    };
    let ran = settled_shapes(&mut Frames::new(), |ui| tile(ui, false));
    let mut frames = Frames::new();
    frames.frame(vec![], scene);
    assert!(runs.get() > 0, "the first frame runs the region");
    frames.idle(30, scene);
    for frame in 0..4 {
        runs.set(0);
        let shapes = frames.frame_shapes(scene);
        assert_eq!(runs.get(), 0, "settled frame {frame} ran the region");
        assert_eq!(alphas(&shapes), alphas(&ran), "settled frame {frame} paints the faded shapes a run paints");
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
