use egui::{pos2, Rect, Vec2};

use super::{anchor_rect, visible_anchor, BarPlacement};

const BAR: BarPlacement = BarPlacement { size: Vec2::new(300.0, 60.0), gap: 8.0, inset: 4.0, min_visible_h: 1.0 };

fn vp() -> Rect {
    Rect::from_min_size(pos2(100.0, 50.0), Vec2::new(1200.0, 700.0))
}

fn place(top: f32, left: f32) -> Rect {
    anchor_rect(line(left, top), vp(), BAR).unwrap_or(Rect::EVERYTHING)
}

fn line(x: f32, y: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), Vec2::new(200.0, 20.0))
}

#[test]
fn sits_above_the_selection_when_there_is_room() {
    let left = vp().center().x;
    let r = place(400.0, left);
    assert_eq!(r.size(), BAR.size);
    assert!((r.max.y - (400.0 - BAR.gap)).abs() < 1e-3);
    assert!((r.center().x - (left + BAR.inset)).abs() < 1e-3);
}

#[test]
fn flips_below_when_there_is_no_room_above() {
    let r = place(60.0, 300.0);
    assert!((r.min.y - (80.0 + BAR.gap)).abs() < 1e-3);
}

#[test]
fn horizontal_shift_keeps_the_bar_in_the_viewport() {
    for left in [0.0, 130.0, 400.0, 2000.0, -500.0] {
        let r = place(400.0, left);
        assert!(r.min.x >= vp().min.x - 1e-3 && r.max.x <= vp().max.x + 1e-3, "left {left}");
    }
}

#[test]
fn an_offscreen_anchor_keeps_the_bar_inside_vertically() {
    for top in [vp().top() - 500.0, vp().bottom() + 500.0] {
        let r = place(top, vp().center().x);
        assert!(r.min.y >= vp().top() - 1e-3 && r.max.y <= vp().bottom() + 1e-3, "top {top}");
    }
}

#[test]
fn visible_anchor_skips_offscreen_rects_and_clips_partial_ones() {
    let clip = vp();
    let above = line(100.0, clip.top() - 300.0);
    let inside = line(120.0, clip.top() + 50.0);
    assert_eq!(visible_anchor([above, inside], clip, 1.0), Some(inside));
    let a = visible_anchor([line(100.0, clip.top() - 10.0)], clip, 1.0).unwrap();
    assert!((a.min.y - clip.top()).abs() < 1e-3 && (a.height() - 10.0).abs() < 1e-3);
    let below = line(100.0, clip.bottom() + 5.0);
    let touching = line(100.0, clip.top() - 20.0);
    assert_eq!(visible_anchor([above, below, touching], clip, 1.0), None);
    assert_eq!(visible_anchor([], clip, 1.0), None);
    assert_eq!(visible_anchor([line(100.0, clip.top() + 200.0), inside], clip, 1.0), Some(inside));
}

#[test]
fn a_sliver_shorter_than_the_minimum_is_not_an_anchor() {
    let clip = vp();
    assert_eq!(visible_anchor([line(100.0, clip.top() - 19.5)], clip, 1.0), None);
    assert!(visible_anchor([line(100.0, clip.top() - 19.5)], clip, 0.25).is_some());
}

#[test]
fn zero_height_edge_touch_does_not_hide_a_later_visible_anchor() {
    let clip = vp();
    let touching = Rect::from_min_size(pos2(100.0, clip.top() - 10.0), Vec2::new(200.0, 10.0));
    let visible = line(100.0, clip.top() + 10.0);
    assert_eq!(visible_anchor([touching, visible], clip, 0.0), Some(visible));
}

#[test]
fn a_bar_for_a_partly_hidden_selection_clears_the_top_edge() {
    let clip = vp();
    let partial = Rect::from_min_size(pos2(100.0, clip.top() - 40.0), Vec2::new(200.0, 50.0));
    let a = visible_anchor([partial], clip, 1.0).unwrap();
    let bar = anchor_rect(a, clip, BAR).unwrap();
    assert!(bar.min.y >= clip.top() - 1e-3 && bar.max.y <= clip.bottom() + 1e-3);
    assert!((bar.min.y - (a.bottom() + BAR.gap)).abs() < 1e-3);
}

#[test]
fn degenerate_inputs_are_rejected() {
    assert!(anchor_rect(Rect::from_min_max(pos2(f32::NAN, 10.0), pos2(20.0, 30.0)), vp(), BAR).is_none());
    assert!(anchor_rect(line(10.0, 10.0), Rect::EVERYTHING, BAR).is_none());
    assert!(anchor_rect(line(10.0, 10.0), Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(0.0, 100.0)), BAR).is_none());
    assert!(anchor_rect(line(10.0, 10.0), vp(), BarPlacement { size: Vec2::new(f32::NAN, 10.0), ..BAR }).is_none());
    assert!(anchor_rect(line(10.0, 10.0), vp(), BarPlacement { size: Vec2::new(0.0, 10.0), ..BAR }).is_none());
    assert!(anchor_rect(line(10.0, 10.0), vp(), BarPlacement { gap: f32::INFINITY, ..BAR }).is_none());
    assert!(visible_anchor([line(100.0, 100.0)], vp(), f32::NAN).is_none());
    let nan = Rect::from_min_size(pos2(f32::NAN, 0.0), Vec2::new(10.0, 10.0));
    assert_eq!(visible_anchor([nan], vp(), 1.0), None);
}

#[test]
fn a_viewport_smaller_than_the_bar_still_yields_a_rect() {
    let tiny = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(20.0, 20.0));
    let r = anchor_rect(line(5.0, 10.0), tiny, BAR).unwrap_or(Rect::EVERYTHING);
    assert!(r.width() > 0.0 && r.min.x.is_finite() && r.min.y.is_finite());
}
