use super::*;
use CropHandle::*;

const W: f64 = 600.0;
const H: f64 = 400.0;
const EPS: f64 = 1e-6;

fn geo(x0: f64, y0: f64, x1: f64, y1: f64) -> CropGeometry {
    CropGeometry { rect: Rect::new(x0, y0, x1, y1), angle: 0.0 }
}
fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}
fn drag(g: CropGeometry, hd: CropHandle, from: Point, to: Point, aspect: Option<f64>) -> CropGeometry {
    drag_crop(g, hd, from, to, W, H, aspect)
}
fn px_aspect(g: &CropGeometry) -> f64 {
    g.rect.width() * W / (g.rect.height() * H)
}
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

// Feature: dragging a crop handle
//   Scenario: a free corner drag inside the image
#[test]
fn given_free_crop_when_corner_dragged_inside_then_corner_follows_and_opposite_corner_stays() {
    let g = drag(geo(0.2, 0.2, 0.8, 0.8), BottomRight, p(0.8, 0.8), p(0.9, 0.7), None);
    assert!(near(g.rect.x0, 0.2) && near(g.rect.y0, 0.2));
    assert!(near(g.rect.x1, 0.9) && near(g.rect.y1, 0.7));
}

//   Scenario: dragging past the right edge (the weird behaviour of issue 295)
#[test]
fn given_corner_dragged_past_image_edge_then_it_stops_at_the_edge_and_anchor_stays() {
    let g = drag(geo(0.2, 0.2, 0.8, 0.8), BottomRight, p(0.8, 0.8), p(1.3, 0.7), None);
    assert!(near(g.rect.x0, 0.2) && near(g.rect.y0, 0.2), "anchor moved: {:?}", g.rect);
    assert!(near(g.rect.x1, 1.0), "stops at the edge: {:?}", g.rect);
    assert!(near(g.rect.y1, 0.7), "other axis still follows the pointer: {:?}", g.rect);
}

#[test]
fn given_locked_aspect_when_corner_dragged_past_edge_then_ratio_and_anchor_hold() {
    let a = 1.5;
    let g = drag(geo(0.2, 0.2, 0.8, 0.2 + 0.6 * W / H / a), BottomRight, p(0.8, 0.6), p(1.4, 0.9), Some(a));
    assert!(near(g.rect.x0, 0.2) && near(g.rect.y0, 0.2), "{:?}", g.rect);
    assert!(near(px_aspect(&g), a), "{}", px_aspect(&g));
    assert!(g.rect.x1 <= 1.0 + EPS && g.rect.y1 <= 1.0 + EPS);
    assert!(g.rect.x1 > 0.95 || g.rect.y1 > 0.95, "grows until it touches an edge: {:?}", g.rect);
}

#[test]
fn given_locked_aspect_when_edge_dragged_then_ratio_holds_and_frame_grows_about_its_midline() {
    let a = 2.0;
    let start = geo(0.3, 0.3, 0.7, 0.3 + 0.4 * W / H / a);
    let g = drag(start, Bottom, p(0.5, start.rect.y1), p(0.5, start.rect.y1 + 0.1), Some(a));
    assert!(near(g.rect.y0, start.rect.y0), "opposite edge stays");
    assert!(near(g.rect.center().x, start.rect.center().x), "grows symmetrically sideways");
    assert!(near(px_aspect(&g), a));
    assert!(g.rect.height() > start.rect.height());
}

#[test]
fn given_free_crop_when_edge_dragged_then_only_that_edge_moves() {
    let g = drag(geo(0.2, 0.2, 0.8, 0.8), Left, p(0.2, 0.5), p(0.1, 0.9), None);
    assert!(near(g.rect.x0, 0.1) && near(g.rect.y0, 0.2) && near(g.rect.x1, 0.8) && near(g.rect.y1, 0.8));
}

#[test]
fn given_move_past_edge_then_size_is_kept_and_frame_slides_along_the_edge() {
    let start = geo(0.5, 0.25, 0.9, 0.65);
    let g = drag(start, Move, p(0.7, 0.45), p(1.3, 0.55), None);
    assert!(near(g.rect.width(), 0.4) && near(g.rect.height(), 0.4));
    assert!(near(g.rect.x1, 1.0), "flush with the right edge: {:?}", g.rect);
    assert!(near(g.rect.y0, 0.35), "still follows the pointer vertically: {:?}", g.rect);
}

#[test]
fn given_edge_dragged_across_the_opposite_edge_then_crop_keeps_a_minimum_size() {
    let g = drag(geo(0.2, 0.2, 0.8, 0.8), Right, p(0.8, 0.5), p(-0.5, 0.5), None);
    assert!(g.rect.width() * W >= MIN_CROP_FRACTION * H - EPS && g.rect.x1 > g.rect.x0);
    assert!(near(g.rect.x0, 0.2));
}

#[test]
fn given_straightened_image_when_dragged_outside_then_crop_stays_inside_the_rotated_image() {
    let start = crate::crop_fit_angle(W, H, 12.0, Some(1.5));
    let c = start.rect.center();
    for hd in [TopLeft, TopRight, BottomRight, BottomLeft, Top, Right, Bottom, Left, Move] {
        for aspect in [None, Some(1.5)] {
            let g = drag(start, hd, c, p(3.0, -3.0), aspect);
            assert!(g.is_within_image(W, H), "{hd:?} {aspect:?}: {:?}", g.rect);
            assert_eq!(g.angle, 12.0);
        }
    }
}

#[test]
fn given_hostile_numbers_then_the_start_crop_is_returned_not_a_panic() {
    let s = geo(0.2, 0.2, 0.8, 0.8);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(drag(s, BottomRight, p(0.8, 0.8), p(bad, 0.5), None), s);
    }
    assert_eq!(drag_crop(s, BottomRight, p(0.8, 0.8), p(0.9, 0.9), 0.0, 400.0, None), s);
    assert_eq!(drag(s, BottomRight, p(0.8, 0.8), p(0.9, 0.9), Some(f64::NAN)), s);
    assert_eq!(drag(s, BottomRight, p(0.8, 0.8), p(0.9, 0.9), Some(-1.0)), s);
}

//   Scenario: a locked edge drag when the frame already touches the image edge (review finding)
#[test]
fn given_locked_frame_flush_with_the_top_when_right_edge_dragged_out_then_it_grows_sliding_down() {
    let a = 1.5;
    let start = geo(0.2, 0.0, 0.5, 0.3 * W / H * 0.0 + 0.3 * W / (a * H));
    let g = drag(start, Right, p(0.5, 0.2), p(0.9, 0.2), Some(a));
    assert!(near(g.rect.x0, 0.2), "anchor edge stays: {:?}", g.rect);
    assert!(g.rect.width() > start.rect.width() + 0.05, "it must grow: {:?}", g.rect);
    assert!(near(px_aspect(&g), a));
    assert!(g.is_within_image(W, H));
}

#[test]
fn given_full_fit_straightened_crop_when_locked_edge_dragged_then_it_still_respects_the_image() {
    let start = crate::crop_fit_angle(W, H, 20.0, Some(1.5));
    let g = drag(start, Right, p(0.9, 0.5), p(1.5, 0.5), Some(1.5));
    assert!(g.is_within_image(W, H) && near(px_aspect(&g), 1.5));
}

//   Scenario: extreme or hostile magnitudes
#[test]
fn given_an_extreme_locked_ratio_then_the_frame_can_still_be_moved() {
    let start = geo(0.1, 0.4, 0.9, 0.4 + 0.8 * W / (60.0 * H));
    let g = drag(start, Move, p(0.5, 0.4), p(0.5, 0.7), Some(60.0));
    assert!(g.rect.y0 > 0.5, "moved: {:?}", g.rect);
}

#[test]
fn given_a_huge_finite_pointer_then_a_locked_corner_still_grows_to_the_edge() {
    let a = 1.5;
    let start = geo(0.2, 0.2, 0.5, 0.2 + 0.3 * W / (a * H));
    let g = drag(start, BottomRight, p(0.5, 0.4), p(1e300, 1e300), Some(a));
    assert!(g.rect.x1 > 0.95 || g.rect.y1 > 0.95, "{:?}", g.rect);
    let g = drag(start, BottomRight, p(0.5, 0.4), p(1e300, 1e300), None);
    assert!(near(g.rect.x1, 1.0) && near(g.rect.y1, 1.0), "{:?}", g.rect);
}

#[test]
fn given_an_unrotated_image_then_results_never_leave_the_unit_square() {
    let g = drag(geo(0.2, 0.2, 0.8, 0.8), BottomRight, p(0.8, 0.8), p(5.0, 5.0), None);
    assert!(g.rect.x1 <= 1.0 && g.rect.y1 <= 1.0 && g.rect.x0 >= 0.0 && g.rect.y0 >= 0.0, "{:?}", g.rect);
}

#[test]
fn handle_numbering_matches_the_overlay() {
    assert_eq!(CropHandle::from_index(0), Some(TopLeft));
    assert_eq!(CropHandle::from_index(6), Some(Bottom));
    assert_eq!(CropHandle::from_index(8), Some(Move));
    assert_eq!(CropHandle::from_index(9), None);
    assert_eq!(CropHandle::from_name("bottomLeft"), Some(BottomLeft));
    assert_eq!(CropHandle::from_name("nope"), None);
}

mod props {
    use super::*;
    use proptest::prelude::*;

    fn any_handle() -> impl Strategy<Value = CropHandle> {
        (0u8..9).prop_map(|i| CropHandle::from_index(i).unwrap_or(Move))
    }

    proptest! {
        /// Whatever the pointer does, the crop stays valid; the anchor and the ratio hold.
        #[test]
        fn drag_is_always_inside_anchored_and_shaped(
            angle in -45.0f64..45.0,
            ratio in prop::option::of(0.3f64..4.0),
            hd in any_handle(),
            tx in -4.0f64..5.0, ty in -4.0f64..5.0,
            shrink in 0.2f64..1.0,
        ) {
            let base = crate::crop_fit_angle(W, H, angle, ratio);
            // a smaller start crop so there is room to move
            let c = base.rect.center();
            let start = CropGeometry { rect: Rect::from_center(c, base.rect.width() * shrink, base.rect.height() * shrink), angle };
            prop_assume!(start.is_within_image(W, H));
            let g = drag_crop(start, hd, c, p(tx, ty), W, H, ratio);
            prop_assert!(g.is_within_image(W, H), "{:?} -> {:?}", start.rect, g.rect);
            prop_assert!(g.rect.width() > 0.0 && g.rect.height() > 0.0);
            let min = MIN_CROP_FRACTION * W.min(H) - 1e-6;
            prop_assert!(g.rect.width() * W >= min.min(start.rect.width() * W) && g.rect.height() * H >= min.min(start.rect.height() * H), "min size {:?}", g.rect);
            if let Some(a) = ratio {
                prop_assert!((px_aspect(&g) - a).abs() < 1e-6 * a.max(1.0), "ratio {a}: {}", px_aspect(&g));
            }
            // anchors: edges opposite the handle do not move
            let (s, r) = (start.rect, g.rect);
            let keep_l = matches!(hd, TopRight | BottomRight | Right);
            let keep_r = matches!(hd, TopLeft | BottomLeft | Left);
            let keep_t = matches!(hd, BottomLeft | BottomRight | Bottom);
            let keep_b = matches!(hd, TopLeft | TopRight | Top);
            {
                if keep_l { prop_assert!((r.x0 - s.x0).abs() < 1e-6, "x0"); }
                if keep_r { prop_assert!((r.x1 - s.x1).abs() < 1e-6, "x1"); }
                if keep_t { prop_assert!((r.y0 - s.y0).abs() < 1e-6, "y0"); }
                if keep_b { prop_assert!((r.y1 - s.y1).abs() < 1e-6, "y1"); }
            }
            if hd == Move {
                prop_assert!((r.width() - s.width()).abs() < 1e-9 && (r.height() - s.height()).abs() < 1e-9);
            }
        }

        #[test]
        fn drag_never_panics_on_arbitrary_numbers(
            x0 in -2.0f64..3.0, y0 in -2.0f64..3.0, x1 in -2.0f64..3.0, y1 in -2.0f64..3.0,
            angle in -90.0f64..90.0, tx in -1e9f64..1e9, ty in -1e9f64..1e9,
            ratio in prop::option::of(-1.0f64..10.0), hd in any_handle(),
        ) {
            let g = drag_crop(CropGeometry { rect: Rect::new(x0, y0, x1, y1), angle }, hd, p(0.5, 0.5), p(tx, ty), W, H, ratio);
            prop_assert!(g.rect.x0.is_finite() && g.rect.y1.is_finite());
        }
    }
}

//   Scenario: moving a frame that fills a straightened photo's height (issue storytold/lightcraft#2299)
#[test]
fn given_largest_locked_frame_on_a_straightened_photo_when_moved_sideways_then_it_slides_along_the_tilt() {
    for angle in [3.0, -10.0] {
        let a = 2.0 / 3.0;
        let start = crate::crop_fit_angle(W, H, angle, Some(a));
        let c = start.rect.center();
        let g = drag(start, Move, c, p(c.x - 0.5, c.y), Some(a));
        assert!(g.is_within_image(W, H), "{angle}: {:?}", g.rect);
        assert!(near(g.rect.width(), start.rect.width()) && near(g.rect.height(), start.rect.height()), "a move keeps the size");
        assert!(g.rect.x0 < start.rect.x0 - 0.2, "{angle}: stuck at {:?} (from {:?})", g.rect, start.rect);
        let gc = g.rect.center();
        let further = drag(g, Move, gc, p(gc.x - 0.5, gc.y), Some(a));
        assert!((further.rect.x0 - g.rect.x0).abs() < 1e-4, "{angle}: not at the left side: {:?} → {:?}", g.rect, further.rect);
        let back = drag(g, Move, c, p(c.x + 0.5, c.y), Some(a));
        assert!(back.rect.x1 > start.rect.x1 + 0.2, "{angle}: {:?}", back.rect);
    }
}

//   Scenario: a straightened move that fits goes exactly where the pointer says
#[test]
fn given_straightened_photo_when_a_small_frame_is_moved_inside_then_it_follows_the_pointer() {
    let start = CropGeometry { rect: Rect::new(0.4, 0.4, 0.5, 0.5), angle: 8.0 };
    let g = drag(start, Move, p(0.45, 0.45), p(0.5, 0.42), None);
    assert!(near(g.rect.x0, 0.45) && near(g.rect.y0, 0.37), "{:?}", g.rect);
}

//   Scenario: a frame a handle drag left within the tolerance past a tilted edge (review of storytold/lightcraft#2299)
#[test]
fn given_a_frame_just_past_a_tilted_edge_when_moved_then_it_still_slides_and_a_click_changes_nothing() {
    let a = 2.0 / 3.0;
    let fit = crate::crop_fit_angle(W, H, 3.0, Some(a));
    let k = 1.0 + 5e-7 / fit.rect.height();
    let start = CropGeometry { rect: Rect::from_center(fit.rect.center(), fit.rect.width() * k, fit.rect.height() * k), angle: 3.0 };
    assert!(start.is_within_image(W, H));
    let c = start.rect.center();
    let z = drag(start, Move, c, c, Some(a)).rect;
    let s = start.rect;
    assert!(near(z.x0, s.x0) && near(z.y0, s.y0) && near(z.x1, s.x1) && near(z.y1, s.y1), "a click inside the frame moves nothing: {z:?}");
    let g = drag(start, Move, c, p(c.x - 0.5, c.y), Some(a));
    assert!(g.rect.x0 < start.rect.x0 - 0.2, "stuck at {:?}", g.rect);
    assert!(g.is_within_image(W, H));
}

//   Scenario: a frame a handle drag enlarged to a tilted edge (review of storytold/lightcraft#2299)
#[test]
fn given_a_frame_enlarged_to_a_tilted_edge_by_a_handle_when_moved_then_it_still_slides() {
    let a = 2.0 / 3.0;
    for angle in [3.0, 6.0, -10.0, 12.0] {
        let fit = crate::crop_fit_angle(W, H, angle, Some(a));
        let small = CropGeometry { rect: Rect::from_center(fit.rect.center(), fit.rect.width() * 0.6, fit.rect.height() * 0.6), angle };
        // the second fills the photo's height, at the tolerance's edge top and bottom
        let shrunk = drag(fit, BottomRight, p(0.5, 0.5), p(0.35, 0.35), Some(a));
        for start in [small, shrunk] {
            let big = drag(start, BottomRight, p(0.5, 0.5), p(1.5, 1.5), Some(a));
            let c = big.rect.center();
            let g = drag(big, Move, c, p(c.x - 0.5, c.y), Some(a));
            assert!(g.rect.x0 < big.rect.x0 - 0.2, "{angle}: stuck at {:?}", g.rect);
            assert!(
                g.is_within_image(W, H)
                    && (g.rect.width() / big.rect.width() - 1.0).abs() < 1e-5
                    && (g.rect.height() / big.rect.height() - 1.0).abs() < 1e-5,
                "{angle}: {:?}",
                g.rect
            );
        }
    }
}

//   Scenario: a smaller frame pushed sideways past a tilted edge slides along it (storytold/lightcraft#2299)
#[test]
fn given_a_small_frame_pushed_past_a_tilted_edge_then_it_stops_on_the_edge_and_slides_along_it() {
    let start = CropGeometry { rect: Rect::new(0.35, 0.35, 0.55, 0.65), angle: 6.0 };
    let c = start.rect.center();
    let g = drag(start, Move, c, p(c.x - 0.2, c.y), None);
    assert!(near(g.rect.x0, 0.15) && near(g.rect.y0, 0.35), "{:?}", g.rect);
    let (a, b) = (drag(start, Move, c, p(c.x - 0.6, c.y), None), drag(start, Move, c, p(c.x - 1.0, c.y), None));
    for g in [a, b] {
        assert!(g.is_within_image(W, H) && g.rect.x0 < 0.01, "{:?}", g.rect);
    }
    assert!(b.rect.y0 > a.rect.y0 + 0.01 && a.rect.y0 > start.rect.y0, "{:?} then {:?}", a.rect, b.rect);
}

//   Scenario: a free corner dragged diagonally out on a straightened photo (issue storytold/lightcraft#2308)
#[test]
fn given_straightened_photo_when_a_free_corner_is_dragged_out_diagonally_then_both_sides_grow() {
    for angle in [6.0, 12.0] {
        let start = CropGeometry { rect: Rect::new(0.3, 0.3, 0.6, 0.6), angle };
        let g = drag(start, BottomRight, p(0.6, 0.6), p(1.2, 1.2), None);
        assert!(g.is_within_image(W, H), "{angle}: {:?}", g.rect);
        assert!(near(g.rect.x0, 0.3) && near(g.rect.y0, 0.3), "{angle}: the anchor corner stays: {:?}", g.rect);
        // both sides grow (main stops at y1 0.600)
        assert!(g.rect.x1 > 0.9 && g.rect.y1 > 0.9, "{angle}: one side only: {:?}", g.rect);
        let further = drag(g, BottomRight, p(g.rect.x1, g.rect.y1), p(1.2, 1.2), None);
        assert!((further.rect.x1 - g.rect.x1).abs() < 1e-4 && (further.rect.y1 - g.rect.y1).abs() < 1e-4, "{angle}: {:?}", further.rect);
    }
    let start = CropGeometry { rect: Rect::new(0.3, 0.3, 0.6, 0.6), angle: 8.0 };
    for (hd, from, to) in [(TopLeft, p(0.3, 0.3), p(-0.5, -0.5)), (TopRight, p(0.6, 0.3), p(1.5, -0.5)), (BottomLeft, p(0.3, 0.6), p(-0.5, 1.5))] {
        let g = drag(start, hd, from, to, None);
        assert!(g.is_within_image(W, H), "{hd:?}: {:?}", g.rect);
        assert!(g.rect.width() > 0.5 && g.rect.height() > 0.45, "{hd:?}: one side only: {:?}", g.rect);
        let (ax, ay) = (if from.x < 0.5 { g.rect.x1 } else { g.rect.x0 }, if from.y < 0.5 { g.rect.y1 } else { g.rect.y0 });
        assert!(near(ax, if from.x < 0.5 { 0.6 } else { 0.3 }) && near(ay, if from.y < 0.5 { 0.6 } else { 0.3 }), "{hd:?}: {:?}", g.rect);
    }
}

//   Scenario: a free corner pushed along one axis past a tilted edge on a straightened photo
#[test]
fn given_straightened_photo_when_a_free_corner_is_pushed_sideways_past_an_edge_then_the_other_side_keeps_its_size() {
    for angle in [6.0, 12.0, -8.0] {
        let start = CropGeometry { rect: Rect::new(0.3, 0.3, 0.6, 0.6), angle };
        let g = drag(start, BottomRight, p(0.6, 0.6), p(1.3, 0.6), None);
        assert!(g.is_within_image(W, H), "{angle}: {:?}", g.rect);
        assert!(near(g.rect.y1, 0.6) && near(g.rect.x0, 0.3) && near(g.rect.y0, 0.3), "{angle}: the height changed: {:?}", g.rect);
        assert!(g.rect.x1 > 0.85, "{angle}: it stopped short of the edge: {:?}", g.rect);
        let g = drag(start, BottomRight, p(0.6, 0.6), p(0.6, 1.3), None);
        assert!(near(g.rect.x1, 0.6) && g.rect.y1 > 0.8, "{angle}: the width changed: {:?}", g.rect);
    }
}

//   Scenario: a free corner of a crop under the minimum size on a straightened photo
#[test]
fn given_straightened_photo_when_a_corner_of_a_too_small_crop_is_dragged_then_it_reaches_the_minimum() {
    let start = CropGeometry { rect: Rect::new(0.3, 0.3, 0.3 + 2.0 / W, 0.6), angle: 6.0 };
    let g = drag(start, BottomRight, p(start.rect.x1, 0.6), p(start.rect.x1 + 2.0 / W, 0.62), None);
    assert!(g.rect.width() * W >= MIN_CROP_FRACTION * H - EPS, "{:?}", g.rect);
    assert!(g.is_within_image(W, H) && near(g.rect.x0, 0.3) && near(g.rect.y0, 0.3), "{:?}", g.rect);
}

//   Scenario: a free corner moved inside, or across its anchor, on a straightened photo
#[test]
fn given_straightened_photo_when_a_free_corner_moves_inside_then_it_follows_and_keeps_a_minimum_size() {
    let start = CropGeometry { rect: Rect::new(0.3, 0.3, 0.6, 0.6), angle: 8.0 };
    let g = drag(start, BottomRight, p(0.6, 0.6), p(0.7, 0.55), None);
    assert!(near(g.rect.x1, 0.7) && near(g.rect.y1, 0.55) && near(g.rect.x0, 0.3) && near(g.rect.y0, 0.3), "{:?}", g.rect);
    let z = drag(start, BottomRight, p(0.6, 0.6), p(0.6, 0.6), None).rect;
    assert!(near(z.x1, 0.6) && near(z.y1, 0.6), "a click moves nothing: {z:?}");
    let crossed = drag(start, BottomRight, p(0.6, 0.6), p(0.0, 0.0), None);
    assert!(near(crossed.rect.x0, 0.3) && near(crossed.rect.y0, 0.3), "{:?}", crossed.rect);
    assert!(
        crossed.rect.width() * W >= MIN_CROP_FRACTION * H - EPS && crossed.rect.height() * H >= MIN_CROP_FRACTION * H - EPS,
        "{:?}",
        crossed.rect
    );
}
