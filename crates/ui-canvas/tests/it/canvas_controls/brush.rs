//! The stroke trail's coverage mask: capsule stamps, restamps over a kept base, and thin strokes on a large document.

use craft_behavior::kurbo::{Point, Rect};
use craft_ui::canvas_controls::brush::CoverageMask;
use craft_ui::primitives::Serials;
use craft_ui::testing::trial;

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

/// A small brush tip's window image (photocraft#13602): the tip's ring at the hotspot, a fully clear gap of
/// `SMALL_TIP_GAP` points, then opaque crosshair arms on all four axes, at every display scale.
#[trial]
fn a_small_tips_image_leaves_a_clear_gap_between_the_tip_and_its_arms() {
    use craft_behavior::PaintedCursor;
    use craft_ui::primitives::painted_image;
    use craft_ui::runtime::env::Env;
    use craft_ui::testing::Fixture;
    use std::cell::RefCell;
    use std::rc::Rc;

    let env: Rc<RefCell<Option<Env>>> = Rc::default();
    let seen = env.clone();
    let mut fixture = Fixture::new(move |ui| *seen.borrow_mut() = Some(Env::of(ui)));
    fixture.settle();
    let env = env.borrow().clone().expect("the fixture ran");
    let extent = 1.0;
    let ring: Vec<egui::Pos2> = (0..16)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            egui::pos2(a.cos() * extent, a.sin() * extent)
        })
        .collect();
    let cursor = PaintedCursor::small_tip(vec![ring], extent);
    let (gap, arm) = (PaintedCursor::SMALL_TIP_GAP, PaintedCursor::SMALL_TIP_ARM);
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let image = painted_image(&env, &cursor, scale).expect("a small tip fits a cursor image");
        let [w, _] = image.size.map(i32::from);
        let [hx, hy] = image.hotspot.map(i32::from);
        let alpha = |x: i32, y: i32| image.rgba[((y * w + x) * 4 + 3) as usize];
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let at = |points: f32| {
                let d = (points * scale).round() as i32;
                alpha(hx + dx * d, hy + dy * d)
            };
            assert!(at(extent) > 0, "scale {scale} ({dx}, {dy}): the ring is drawn");
            // Clear of the ring's and the arm's halo by two points on each side.
            let mut p = extent + 2.0;
            while p <= extent + gap - 2.0 {
                assert_eq!(at(p), 0, "scale {scale} ({dx}, {dy}): the gap is clear at {p} points");
                p += 0.25;
            }
            let mut p = extent + gap + 1.0;
            while p <= extent + gap + arm - 1.0 {
                assert_eq!(at(p), 255, "scale {scale} ({dx}, {dy}): the arm is opaque at {p} points");
                p += 0.25;
            }
        }
    }
}
