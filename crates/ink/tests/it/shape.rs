//! Ink to shape: every recognised form, and what must stay ink.

use craft_ink::shape::{InkShape, MAX_CORNERS, recognise};

/// Points along a closed polygon, `per` to a side, with a little deterministic hand wobble.
fn around(corners: &[[f32; 2]], per: usize) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for (i, a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % corners.len()];
        for k in 0..per {
            let t = k as f32 / per as f32;
            let wobble = (out.len() as f32 * 1.7).sin() * 0.6;
            out.push([a[0] + (b[0] - a[0]) * t + wobble, a[1] + (b[1] - a[1]) * t - wobble]);
        }
    }
    out.push(corners[0]);
    out
}

#[test]
fn a_line_keeps_its_ends() {
    let line: Vec<[f32; 2]> = (0..=40).map(|i| i as f32).map(|t| [10.0 + t * 3.0, 20.0 + t + (t * 2.1).sin() * 0.8]).collect();
    assert_eq!(recognise(&line), Some(InkShape::Line { a: line[0], b: line[40] }));
    assert_eq!(recognise(&line).map(|s| s.name()), Some("line"));
}

#[test]
fn a_rectangle_and_a_square_are_recognised() {
    let rectangle = around(&[[50.0, 10.0], [110.0, 10.0], [110.0, 70.0], [10.0, 70.0], [10.0, 10.0]], 12);
    let Some(InkShape::Rectangle { x, y, w, h }) = recognise(&rectangle) else { panic!("{:?}", recognise(&rectangle)) };
    assert!((x - 10.0).abs() < 2.0 && (y - 10.0).abs() < 2.0 && (w - 100.0).abs() < 3.0 && (h - 60.0).abs() < 3.0);
    assert_eq!(recognise(&rectangle).map(|s| s.name()), Some("rectangle"));
    let square = around(&[[0.0, 0.0], [60.0, 0.0], [60.0, 57.0], [0.0, 57.0]], 10);
    assert_eq!(recognise(&square).map(|s| s.name()), Some("square"));
}

#[test]
fn an_ellipse_and_a_circle_are_recognised() {
    let ring = |rx: f32, ry: f32| {
        (0..=64).map(|i| (i as f32 / 64.0) * std::f32::consts::TAU).map(|a| [100.0 + rx * a.cos(), 80.0 + ry * a.sin()]).collect::<Vec<_>>()
    };
    let Some(InkShape::Ellipse { x, w, h, .. }) = recognise(&ring(40.0, 25.0)) else { panic!("{:?}", recognise(&ring(40.0, 25.0))) };
    assert!((x - 60.0).abs() < 0.5 && (w - 80.0).abs() < 0.5 && (h - 50.0).abs() < 0.5);
    assert_eq!(recognise(&ring(40.0, 25.0)).map(|s| s.name()), Some("ellipse"));
    assert_eq!(recognise(&ring(30.0, 29.0)).map(|s| s.name()), Some("circle"));
}

#[test]
fn a_triangle_and_a_polygon_are_recognised() {
    let triangle = around(&[[0.0, 80.0], [45.0, 0.0], [90.0, 80.0]], 15);
    let Some(InkShape::Polygon(corners)) = recognise(&triangle) else { panic!("{:?}", recognise(&triangle)) };
    assert_eq!(corners.len(), 3);
    assert_eq!(recognise(&triangle).map(|s| s.name()), Some("triangle"));
    let pentagon: Vec<[f32; 2]> = (0..5)
        .map(|i| (i as f32 / 5.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2)
        .map(|a| [60.0 + 50.0 * a.cos(), 60.0 + 50.0 * a.sin()])
        .collect();
    let Some(InkShape::Polygon(corners)) = recognise(&around(&pentagon, 12)) else { panic!("pentagon") };
    assert_eq!(corners.len(), 5);
    assert_eq!(recognise(&around(&pentagon, 12)).map(|s| s.name()), Some("polygon"));
    assert!(corners.len() <= MAX_CORNERS);
}

#[test]
fn an_arrow_drawn_in_one_stroke_is_recognised_both_ways_round() {
    let (tail, tip) = ([20.0f32, 100.0], [180.0f32, 100.0]);
    let barb = |sign: f32| {
        let back = (tail[1] - tip[1]).atan2(tail[0] - tip[0]);
        let a = back + sign * 30.0f32.to_radians();
        [tip[0] + 25.0 * a.cos(), tip[1] + 25.0 * a.sin()]
    };
    let line = |a: [f32; 2], b: [f32; 2], n: usize| -> Vec<[f32; 2]> {
        (0..n).map(|k| k as f32 / n as f32).map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]).collect()
    };
    let mut stroke = line(tail, tip, 24);
    stroke.extend(line(tip, barb(1.0), 5));
    stroke.extend(line(barb(1.0), tip, 5));
    stroke.extend(line(tip, barb(-1.0), 5));
    stroke.push(barb(-1.0));
    let Some(InkShape::Arrow { a, b, head_at_end, head_at_start }) = recognise(&stroke) else { panic!("{:?}", recognise(&stroke)) };
    assert!(head_at_end && !head_at_start);
    assert!((a[0] - tail[0]).abs() < 1.0 && (b[0] - tip[0]).abs() < 1.0);
    assert_eq!(recognise(&stroke).map(|s| s.name()), Some("arrow"));
    stroke.reverse();
    let Some(InkShape::Arrow { a, b, .. }) = recognise(&stroke) else { panic!("head first: {:?}", recognise(&stroke)) };
    assert!((a[0] - tail[0]).abs() < 1.0 && (b[0] - tip[0]).abs() < 1.0);
}

#[test]
fn scribbles_letters_and_hostile_points_stay_ink() {
    let zigzag: Vec<[f32; 2]> = (0..20).map(|i| [i as f32 * 8.0, if i % 2 == 0 { 0.0 } else { 30.0 }]).collect();
    assert_eq!(recognise(&zigzag), None);
    assert_eq!(recognise(&[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]]), None);
    assert_eq!(recognise(&[[5.0, 5.0]]), None);
    assert_eq!(recognise(&[[0.0, 0.0], [2.0, 3.0]]), None);
    assert_eq!(recognise(&[]), None);
    let star: Vec<[f32; 2]> = (0..=24)
        .map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            let r = if i % 2 == 0 { 60.0 } else { 20.0 };
            [80.0 + r * a.cos(), 80.0 + r * a.sin()]
        })
        .collect();
    assert_eq!(recognise(&star), None);
    let y = [[40.0, 0.0], [60.0, 30.0], [80.0, 0.0], [60.0, 30.0], [60.0, 90.0]];
    assert_eq!(recognise(&y), None);
    let mut junk = vec![[f32::NAN, 1.0], [f32::INFINITY, 2.0], [3.0, f32::NEG_INFINITY]];
    junk.extend(std::iter::repeat_n([1.0, 1.0], 20_000));
    assert_eq!(recognise(&junk), None);
}
