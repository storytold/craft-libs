//! Replay ordering and schedule: stable order, bounded time, progressive reveal.

use craft_ink::replay::{MAX_STROKES, Options, Replay, draw_order, path_length};
use craft_ink::stroke::{Brush, Color, Stroke, StrokePoint};

fn stroke(points: &[[f64; 2]], order: u32) -> Stroke {
    let points = points.iter().map(|p| StrokePoint::new(p[0], p[1])).collect();
    Stroke { order, ..Stroke::new(points, Brush::default(), Color::BLACK) }
}

#[test]
fn equal_orders_keep_document_order_and_recorded_orders_sort() {
    let no_orders = [stroke(&[[0.0, 0.0]], 0), stroke(&[[1.0, 0.0]], 0), stroke(&[[2.0, 0.0]], 0)];
    assert_eq!(draw_order(&no_orders), vec![0, 1, 2]);
    let ties = [stroke(&[[0.0, 0.0]], 2), stroke(&[[1.0, 0.0]], 1), stroke(&[[2.0, 0.0]], 2), stroke(&[[3.0, 0.0]], 1)];
    assert_eq!(draw_order(&ties), vec![1, 3, 0, 2], "a stable sort keeps equal orders in input order");
}

#[test]
fn the_schedule_follows_the_draw_order_and_never_overlaps() {
    let strokes = [stroke(&[[0.0, 0.0], [100.0, 0.0]], 2), stroke(&[[0.0, 0.0], [400.0, 0.0]], 1), stroke(&[[0.0, 0.0], [10.0, 0.0]], 0)];
    let replay = Replay::new(&strokes, Options::default()).expect("a replay");
    let timings = replay.timings();
    assert_eq!(timings.iter().map(|t| t.stroke).collect::<Vec<_>>(), vec![2, 1, 0]);
    assert_eq!(timings[0].start, 0.0);
    for pair in timings.windows(2) {
        assert!(pair[1].start >= pair[0].start + pair[0].duration);
    }
    assert_eq!(replay.total_duration(), timings[2].start + timings[2].duration);
    // The long stroke takes longer than the short one, within the clamps.
    assert!(timings[1].duration > timings[2].duration);
    assert!(timings.iter().all(|t| (0.15..=2.5).contains(&t.duration)));
}

#[test]
fn the_stroke_cap_keeps_the_first_strokes_in_draw_order() {
    let strokes: Vec<Stroke> = (0..10).map(|i| stroke(&[[f64::from(i), 0.0], [f64::from(i), 1.0]], 0)).collect();
    let replay = Replay::new(&strokes, Options { max_strokes: 3, ..Options::default() }).expect("a replay");
    assert_eq!(replay.timings().iter().map(|t| t.stroke).collect::<Vec<_>>(), vec![0, 1, 2]);
}

#[test]
fn the_clock_is_bounded_and_the_replay_stops() {
    let strokes = [stroke(&[[0.0, 0.0], [100.0, 0.0]], 0)];
    let mut replay =
        Replay::new(&strokes, Options { speed: 100.0, gap: 0.0, min_duration: 0.0, max_duration: 1.0, max_strokes: MAX_STROKES }).expect("a replay");
    replay.tick(0.25);
    assert!((replay.clock() - 0.25).abs() < 1e-12);
    replay.tick(f64::NAN);
    assert!((replay.clock() - 0.25).abs() < 1e-12);
    replay.tick(10.0);
    assert_eq!(replay.clock(), replay.total_duration());
    assert!(!replay.is_playing() && replay.is_finished());
    replay.seek(-1.0);
    assert_eq!(replay.clock(), 0.0);
    assert!(replay.is_playing());
}

#[test]
fn a_stroke_is_revealed_along_its_path() {
    let strokes = [stroke(&[[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]], 0)];
    let mut replay =
        Replay::new(&strokes, Options { speed: 100.0, gap: 0.0, min_duration: 0.0, max_duration: 2.0, ..Options::default() }).expect("a replay");
    assert!(replay.visible(0, &strokes[0].points).is_empty());
    replay.seek(replay.total_duration() / 2.0);
    let visible = replay.visible(0, &strokes[0].points);
    assert!(visible.len() >= 2);
    assert!((visible.last().map_or(0.0, |p| p.x) - 50.0).abs() < 1e-9, "half the 100 pt path");
    replay.seek(2.0);
    assert_eq!(replay.visible(0, &strokes[0].points), strokes[0].points);
    assert!(replay.visible(9, &strokes[0].points).is_empty());
    // Hostile points are skipped and a single point has no length.
    let hostile = [StrokePoint::new(f64::NAN, 0.0), StrokePoint::new(1.0, 1.0)];
    assert_eq!(path_length(&hostile), 0.0);
}
