//! The stroke trail's coverage mask: capsule stamps, restamps over a kept base, and thin strokes on a large
//! document; and the colour its quad carries, the tone's themed one or the host's own.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_ui::canvas::Canvas;
use craft_ui::canvas_controls::brush::{CoverageMask, StrokeTrail, TrailTone};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::primitives::Serials;
use craft_ui::style::DataColor;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, Shape};
use std::cell::RefCell;
use std::rc::Rc;

fn mask() -> CoverageMask {
    CoverageMask::new(&mut Serials::default(), Rect::new(0.0, 0.0, 100.0, 100.0), 100)
}

#[trial]
fn a_capsule_stamp_is_the_union_of_round_footprints() {
    let mut level = mask();
    level.stamp_capsule(Point::new(20.0, 50.0), Point::new(80.0, 50.0), 10.0);
    assert_eq!(level.coverage_at(Point::new(50.0, 50.0)), 255, "the middle");
    assert_eq!(level.coverage_at(Point::new(88.0, 50.0)), 255, "inside the end cap");
    assert_eq!(level.coverage_at(Point::new(50.0, 62.0)), 0, "beside the capsule");
    assert_eq!(level.coverage_at(Point::new(5.0, 50.0)), 0, "behind the start");
    assert_eq!(level.coverage_at(Point::new(95.0, 50.0)), 0, "past the end");
    assert_eq!(level.coverage_at(Point::new(11.0, 41.0)), 0, "no wedge at the corner of the bounding rect");

    let mut diagonal = mask();
    diagonal.stamp_capsule(Point::new(20.0, 20.0), Point::new(60.0, 60.0), 8.0);
    assert_eq!(diagonal.coverage_at(Point::new(40.0, 40.0)), 255, "along the diagonal");
    assert_eq!(diagonal.coverage_at(Point::new(64.0, 64.0)), 255, "inside the end cap");
    assert_eq!(diagonal.coverage_at(Point::new(68.0, 52.0)), 0, "no wedge past the end");
}

#[trial]
fn a_restamp_keeps_earlier_strokes_and_stays_inside_its_rect() {
    let mut mask = mask();
    mask.stamp_capsule(Point::new(10.0, 10.0), Point::new(30.0, 10.0), 3.0);
    mask.begin_stroke();
    mask.restamp(Rect::new(0.0, 0.0, 50.0, 50.0), |_| 0.5);
    assert_eq!(mask.coverage_at(Point::new(20.5, 10.5)), 255, "the earlier stroke stays above the new coverage");
    assert_eq!(mask.coverage_at(Point::new(40.5, 40.5)), 128, "the new coverage where nothing was");

    mask.restamp(Rect::new(60.0, 60.0, 70.0, 70.0), |_| 1.0);
    assert_eq!(mask.coverage_at(Point::new(65.5, 65.5)), 255, "inside the rect");
    assert_eq!(mask.coverage_at(Point::new(59.5, 65.5)), 0, "left of the rect");
    assert_eq!(mask.coverage_at(Point::new(70.5, 65.5)), 0, "right of the rect");
    assert_eq!(mask.coverage_at(Point::new(65.5, 70.5)), 0, "below the rect");
}

#[trial]
fn a_thin_stamp_on_a_large_document_still_shows() {
    let mut mask = CoverageMask::new(&mut Serials::default(), Rect::new(0.0, 0.0, 20_000.0, 20_000.0), 1000);
    assert_eq!((mask.scale(), mask.size()), (20.0, (1000, 1000)));
    // A one-unit line on a texel's edge, away from every texel centre.
    mask.restamp(Rect::new(5000.0, 5000.0, 5200.0, 5020.0), |at| if (at.y - 5000.5).abs() <= 0.5 { 1.0 } else { 0.0 });
    assert!(mask.coverage_at(Point::new(5100.0, 5005.0)) > 0, "the line shows");
    assert_eq!(mask.coverage_at(Point::new(5100.0, 5025.0)), 0, "nothing past the restamped rect");
}

/// A mask over the whole 100 by 100 document, covered everywhere.
fn covered() -> CoverageMask {
    let mut mask = mask();
    mask.restamp(Rect::new(0.0, 0.0, 100.0, 100.0), |_| 1.0);
    mask
}

/// One frame of a `tone` trail over the covered mask, with the host's `color` and `share` when it sets them.
fn trail_frame(tone: TrailTone, color: Option<DataColor>, share: Option<f32>) -> Vec<Shape> {
    let mask = Rc::new(RefCell::new(covered()));
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), Point::new(50.0, 50.0), 1.0);
        let _ = Canvas::new("Document").document(Size::new(100.0, 100.0)).view(view).show(ui, |ui, body| {
            let mut mask = mask.borrow_mut();
            let mut trail = StrokeTrail::new("Trail", &mut mask).tone(tone);
            if let Some(color) = color {
                trail = trail.color(color);
            }
            if let Some(share) = share {
                trail = trail.share(share);
            }
            let _ = trail.show_at(ui, body.rect);
        });
    });
    fixture.settle();
    fixture.frame_shapes()
}

/// The colour the trail's quad carries.
fn trail_ink(tone: TrailTone, color: Option<DataColor>, share: Option<f32>) -> Color32 {
    let quads: Vec<Color32> = trail_frame(tone, color, share)
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.vertices.len() == 4 => Some(mesh.vertices[0].color),
            _ => None,
        })
        .collect();
    assert_eq!(quads.len(), 1, "the trail paints exactly one quad");
    quads[0]
}

/// Whether `a` and `b` are the same colour within one level per channel.
fn same(a: Color32, b: Color32) -> bool {
    let (x, y) = (a.to_array(), b.to_array());
    x.iter().zip(y.iter()).all(|(p, q)| (i32::from(*p) - i32::from(*q)).abs() <= 1)
}

#[trial]
fn a_trail_with_no_host_tint_keeps_its_tone_and_that_tones_share() {
    // Without a share the trail shows its tone's own share of the same themed colour.
    let tone = trail_ink(TrailTone::Danger, None, None);
    let solid = trail_ink(TrailTone::Danger, None, Some(1.0));
    assert!(same(tone, solid.gamma_multiply(TrailTone::Danger.share())), "the tone keeps its own share: {tone:?} against {solid:?}");
    assert_ne!(trail_ink(TrailTone::Light, None, None), trail_ink(TrailTone::Shade, None, None), "each tone shows its own themed colour");
    assert_ne!(trail_ink(TrailTone::Shade, None, None), trail_ink(TrailTone::Danger, None, None), "…and not one shared with another tone");
}

#[trial]
fn a_host_tint_replaces_the_tone_colour_and_the_share_it_paints() {
    let chosen = Color32::from_rgb(0x20, 0x80, 0x60);
    let full = trail_ink(TrailTone::Danger, Some(DataColor::new(chosen)), Some(1.0));
    assert!(same(full, chosen), "a share of 1 shows the host's colour unchanged: {full:?}");
    let faded = trail_ink(TrailTone::Danger, Some(DataColor::new(chosen)), Some(0.5));
    assert!(same(faded, chosen.gamma_multiply(0.5)), "the share fades the host's colour: {faded:?}");
    assert_ne!(faded, full, "the same colour at another share paints differently");
    // A colour with no share falls back to the tone's own share.
    assert!(
        same(trail_ink(TrailTone::Danger, Some(DataColor::new(chosen)), None), chosen.gamma_multiply(TrailTone::Danger.share())),
        "a colour with no share keeps the tone's share"
    );
    // A share with no colour still colours the footprint with the tone's themed colour.
    assert_ne!(
        trail_ink(TrailTone::Danger, None, Some(1.0)),
        trail_ink(TrailTone::Light, None, Some(1.0)),
        "only the share set: the tone's colour still paints"
    );
}

#[trial]
fn a_share_that_is_not_a_number_fails_closed_to_the_tones_share() {
    let chosen = Color32::from_rgb(0x20, 0x80, 0x60);
    for share in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        // With the tone's colour: a non-finite share paints exactly like the tone's own share.
        assert_eq!(
            trail_ink(TrailTone::Danger, None, Some(share)),
            trail_ink(TrailTone::Danger, None, None),
            "a share of {share} fails closed to the tone default"
        );
        // With a host colour: the colour stays, only the share falls back to the tone's.
        assert_eq!(
            trail_ink(TrailTone::Danger, Some(DataColor::new(chosen)), Some(share)),
            trail_ink(TrailTone::Danger, Some(DataColor::new(chosen)), None),
            "a share of {share} keeps the host colour at the tone's share"
        );
    }
}

#[trial]
fn the_share_clamps_at_both_ends_of_its_range() {
    assert_eq!(trail_ink(TrailTone::Danger, None, Some(0.0)), Color32::TRANSPARENT, "a share of 0 shows no footprint");
    assert_eq!(trail_ink(TrailTone::Danger, None, Some(-0.25)), trail_ink(TrailTone::Danger, None, Some(0.0)), "a share below 0 acts as 0");
    assert_eq!(trail_ink(TrailTone::Danger, None, Some(1.5)), trail_ink(TrailTone::Danger, None, Some(1.0)), "a share above 1 acts as full");
}
