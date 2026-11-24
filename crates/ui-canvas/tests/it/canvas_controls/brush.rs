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

// The brush cursor's rules for a tip too small to read (photocraft#13602): under 3 points across the tip shows inside
// four crosshair arms that leave it clear; 3 points or more keep the outline.
mod small_tip_rules {
    use craft_behavior::kurbo::{Point, Rect as DocRect};
    use craft_behavior::{CanvasView, Cursor, CursorBadge, PaintedCursor, PointerButton};
    use craft_ui::canvas::{ToolContext, ToolPointer};
    use craft_ui::canvas_controls::brush::{BrushCursor, BrushCursorOptions, BrushTip, BrushTipOutline, PaintingCursor, TipRings};
    use craft_ui::testing::trial;
    use egui::{Pos2, Rect, Vec2};

    /// The pointer `cursor` gives through `BrushCursor::cursor`, with Caps Lock `caps_lock` and the pass's pointer at `at`.
    fn paint(cursor: BrushCursor<'_>, options: &BrushCursorOptions, view: &CanvasView, caps_lock: bool, at: Option<Pos2>) -> PaintedCursor {
        let pointer = at.map(|at| ToolPointer::new(at, PointerButton::Primary, 0.0));
        let cx = ToolContext::new(view.rect, None).pointer(pointer).caps_lock(caps_lock);
        match cursor.cursor(&cx, options, view) {
            Cursor::Painted(painted) => painted,
            other => panic!("a painted cursor, not {other:?}"),
        }
    }

    /// A view at `zoom` points per document unit, with document (100, 100) at screen (100, 100).
    fn view(zoom: f64) -> CanvasView {
        CanvasView::new(Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)), Point::new(100.0, 100.0), zoom)
    }

    fn normal() -> BrushCursorOptions {
        BrushCursorOptions::new(PaintingCursor::NormalBrushTip)
    }

    /// The cursor of a hard round tip `diameter` units across (both outlines at its edge), at `zoom`.
    fn round(diameter: f64, zoom: f64) -> PaintedCursor {
        paint(BrushCursor::new(&BrushTip::round(diameter, diameter / 2.0)), &normal(), &view(zoom), false, None)
    }

    /// The farthest point of `rings` from the pointer.
    fn reach_of(rings: &[Vec<Pos2>]) -> f32 {
        rings.iter().flatten().fold(0.0, |far, p| far.max(p.to_vec2().length()))
    }

    #[track_caller]
    fn small_tip(cursor: &PaintedCursor) -> (f32, f32) {
        match cursor {
            PaintedCursor::SmallTip { rings, extent, .. } => {
                assert!(rings.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite()), "finite rings");
                (*extent, reach_of(rings))
            }
            other => panic!("a small tip, not {other:?}"),
        }
    }

    #[trial]
    fn a_round_tip_under_three_points_across_shows_inside_arms_that_leave_it_clear() {
        for (diameter, zoom, extent) in [(2.0, 1.0, 1.0), (1.0, 1.0, 0.5), (1.0, 2.0, 1.0), (4.0, 0.5, 1.0)] {
            let (at, ring) = small_tip(&round(diameter, zoom));
            assert!((at - extent).abs() < 0.01 && (ring - extent).abs() < 0.01, "{diameter} at {zoom}: the tip's own ring, {at} {ring}");
        }
        // Smaller tips, a zero one and sizes that are no number show the least dot.
        for diameter in [0.4, 0.0, -3.0, f64::NAN, f64::INFINITY] {
            let (at, ring) = small_tip(&round(diameter, 1.0));
            assert!(
                (at - PaintedCursor::SMALL_TIP_DOT).abs() < 0.01 && (ring - PaintedCursor::SMALL_TIP_DOT).abs() < 0.01,
                "{diameter}: {at} {ring}"
            );
        }
    }

    #[trial]
    fn a_tip_three_points_across_or_more_keeps_its_outline() {
        for (diameter, zoom, radius) in [(3.0, 1.0, 1.5), (6.0, 1.0, 3.0), (2.0, 2.0, 2.0), (12.0, 1.0, 6.0)] {
            match round(diameter, zoom) {
                PaintedCursor::Circle { radius: r, .. } => assert!((r - radius).abs() < 0.01, "{diameter} at {zoom}: {r}"),
                other => panic!("{diameter} at {zoom}: the circle, not {other:?}"),
            }
        }
    }

    #[trial]
    fn a_small_shaped_tip_shows_inside_the_arms_and_a_large_one_keeps_its_outline() {
        let square = TipRings::new(vec![vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]]);
        let tip = |diameter: f64| BrushTip::new(diameter, BrushTipOutline::traced(square.clone(), square.clone()));
        let (extent, ring) = small_tip(&paint(BrushCursor::new(&tip(2.0)), &normal(), &view(1.0), false, None));
        assert!((extent - 2f32.sqrt()).abs() < 0.01 && (ring - extent).abs() < 0.01, "{extent} {ring}");
        assert!(matches!(paint(BrushCursor::new(&tip(8.0)), &normal(), &view(1.0), false, None), PaintedCursor::Outline { .. }));
    }

    #[trial]
    fn a_small_pixel_cell_shows_inside_arms_clear_of_its_farthest_corner() {
        let cell = |side: f64| BrushTip::new(side, BrushTipOutline::pixels(DocRect::new(100.0, 100.0, 100.0 + side, 100.0 + side), false));
        let pointer = Some(Pos2::new(100.3, 100.3));
        let one = cell(1.0);
        let (extent, ring) = small_tip(&paint(BrushCursor::new(&one), &normal(), &view(1.0), false, pointer));
        let farthest = Vec2::splat(0.7).length();
        assert!((extent - farthest).abs() < 0.01 && (ring - farthest).abs() < 0.01, "the arms clear the far corner: {extent} {ring}");
        assert_eq!(paint(BrushCursor::new(&one), &normal(), &view(1.0), false, None), PaintedCursor::Crosshair, "no pointer this pass");
        let eight = cell(8.0);
        assert_eq!(
            paint(BrushCursor::new(&eight), &normal(), &view(1.0), false, pointer),
            PaintedCursor::Hidden,
            "a readable cell hides the pointer"
        );
        assert_eq!(paint(BrushCursor::new(&eight).centre(true), &normal(), &view(1.0), false, pointer), PaintedCursor::Crosshair);
        assert!(matches!(paint(BrushCursor::new(&cell(2.0)), &normal(), &view(2.0), false, pointer), PaintedCursor::Hidden), "4 points on screen");
    }

    #[trial]
    fn the_precise_cursor_and_caps_lock_keep_the_crosshair_and_a_badge_rides_on_a_small_tip() {
        let tip = BrushTip::round(1.0, 0.5);
        assert_eq!(paint(BrushCursor::new(&tip), &normal(), &view(1.0), true, None), PaintedCursor::Crosshair);
        assert_eq!(
            paint(BrushCursor::new(&tip), &BrushCursorOptions::new(PaintingCursor::Precise), &view(1.0), false, None),
            PaintedCursor::Crosshair
        );
        let badged = paint(BrushCursor::new(&tip).badge(Some(CursorBadge::Add)), &normal(), &view(1.0), false, None);
        assert!(matches!(badged, PaintedCursor::SmallTip { badge: Some(CursorBadge::Add), .. }), "{badged:?}");
    }
}
