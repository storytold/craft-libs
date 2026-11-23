//! Lasso containment: the 60 % rule, the even–odd loop and the input bounds.

use craft_ink::select::{LASSO_SAMPLES, Lasso, MAX_LASSO_COORD, MAX_LASSO_POINTS};

fn unit_square() -> Lasso {
    Lasso::new(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]).expect("a square")
}

#[test]
fn sixty_percent_selects_and_fifty_nine_does_not() {
    let lasso = unit_square();
    // 64 samples is the cap; with a 100-point stroke every other sample is taken.
    let mut inside = Vec::new();
    let mut outside = Vec::new();
    for i in 0..1_000 {
        if i < 600 {
            inside.push([i as f64 % 10.0, 5.0]);
        } else {
            outside.push([20.0 + i as f64, 5.0]);
        }
    }
    let mut stroke = Vec::new();
    stroke.extend(inside.iter().copied());
    stroke.extend(outside.iter().copied());
    // Interleaved so the sample hits a representative share.
    let mut mixed = Vec::new();
    for i in 0..100 {
        mixed.push(if i % 10 < 6 { [i as f64 % 5.0, 5.0] } else { [50.0, 50.0] });
    }
    assert!(lasso.contains_stroke(&mixed, [0.0, 0.0]), "6 of every 10 samples");
    let mut fewer = mixed.clone();
    for point in fewer.iter_mut().take(50) {
        if point[0] < 5.0 {
            *point = [50.0, 50.0];
        }
    }
    assert!(!lasso.contains_stroke(&fewer, [0.0, 0.0]), "below 60 %");
    // An exact 6/10 selection of a ten-point stroke.
    let exact: Vec<[f64; 2]> = (0..6).map(|i| [i as f64, 5.0]).chain((0..4).map(|i| [20.0 + i as f64, 5.0])).collect();
    assert!(lasso.contains_stroke(&exact, [0.0, 0.0]));
    let one_less: Vec<[f64; 2]> = (0..5).map(|i| [i as f64, 5.0]).chain((0..5).map(|i| [20.0 + i as f64, 5.0])).collect();
    assert!(!lasso.contains_stroke(&one_less, [0.0, 0.0]));
}

#[test]
fn the_origin_places_a_stroke_relative_to_the_loop() {
    let lasso = unit_square();
    let stroke = [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0]];
    assert!(lasso.contains_stroke(&stroke, [1.0, 1.0]));
    assert!(!lasso.contains_stroke(&stroke, [-100.0, 0.0]));
}

#[test]
fn a_crossing_stroke_is_not_selected() {
    let lasso = unit_square();
    let crossing = [[5.0, 5.0], [15.0, 5.0], [25.0, 5.0], [35.0, 5.0]];
    assert!(!lasso.contains_stroke(&crossing, [0.0, 0.0]));
}

#[test]
fn the_loop_needs_three_usable_points() {
    assert!(Lasso::new(&[]).is_none());
    assert!(Lasso::new(&[[0.0, 0.0], [1.0, 1.0]]).is_none());
    assert!(Lasso::new(&[[0.0, 0.0], [1.0, 1.0], [f64::NAN, 2.0]]).is_none());
    assert!(Lasso::new(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]).is_some());
}

#[test]
fn a_long_or_hostile_loop_is_thinned_and_clamped() {
    let many: Vec<[f64; 2]> = (0..50_000).map(|i| [f64::from(i % 400) * 1e12, f64::from(i / 400) * 1e12]).collect();
    let lasso = Lasso::new(&many).expect("a loop");
    assert!(lasso.polygon().len() <= MAX_LASSO_POINTS);
    assert!(lasso.polygon().iter().flatten().all(|v| v.abs() <= MAX_LASSO_COORD));
    assert!(lasso.contains([0.0, 0.0]) || !lasso.contains([0.0, 0.0]), "never panics");
}

#[test]
fn sampling_spreads_at_most_sixty_four_points_over_the_stroke() {
    // A stroke much longer than LASSO_SAMPLES is sampled at an even stride.
    let lasso = unit_square();
    let mut points = vec![[50.0, 50.0]; 6_399];
    points.push([5.0, 5.0]);
    assert!(!lasso.contains_stroke(&points, [0.0, 0.0]), "1 sample of 64");
    let inside = vec![[5.0, 5.0]; LASSO_SAMPLES * 9];
    assert!(lasso.contains_stroke(&inside, [0.0, 0.0]));
    // Non-finite points are never inside and never count as samples.
    let hostile = vec![[f64::NAN, 5.0], [f64::INFINITY, 5.0], [5.0, 5.0], [5.0, 5.0], [5.0, 5.0]];
    assert!(lasso.contains_stroke(&hostile, [0.0, 0.0]));
}
