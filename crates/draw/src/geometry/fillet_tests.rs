//! Tests of rounded corners.

use craft_ui::testing::trial;
use kurbo::{Point, Vec2};

use super::fillet::fillet;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// PhotoCraft's values: a right angle of a 100 × 60 rectangle caps at 30 and steps (1, 1); an
// acute triangle vertex; a small radius is kept.
#[trial]
fn fillet_matches_photocraft() {
    let (a, b, c) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 60.0));
    let Some(f) = fillet(b, a, c, 1e12) else { panic!("a right angle rounds") };
    assert!(near(f.radius, 30.0));
    assert!(near(f.half_angle, std::f64::consts::FRAC_PI_4));
    assert!((f.centre - Point::new(30.0, 30.0)).hypot() < 1e-9);
    assert!((f.from - Point::new(30.0, 0.0)).hypot() < 1e-9);
    assert!((f.to - Point::new(0.0, 30.0)).hypot() < 1e-9);
    assert!((f.step() - Vec2::new(1.0, 1.0)).hypot() < 1e-9);

    let Some(small) = fillet(b, a, c, 5.0) else { panic!("a small radius rounds") };
    assert!(near(small.radius, 5.0));
    assert!((small.centre - Point::new(5.0, 5.0)).hypot() < 1e-9);

    // An acute vertex: 60° at the origin of an equilateral triangle with side 10.
    let (p, q) = (Point::new(10.0, 0.0), Point::new(5.0, 75f64.sqrt()));
    let Some(t) = fillet(p, a, q, 1e12) else { panic!("an acute angle rounds") };
    assert!(near(t.half_angle, std::f64::consts::FRAC_PI_6));
    assert!(near(t.radius, 5.0 * std::f64::consts::FRAC_PI_6.tan()));
    assert!(near((t.centre - a).hypot(), t.radius / t.half_angle.sin()));
}

// `None` for a zero radius, NaN, a zero-length edge, a straight and a folded vertex.
