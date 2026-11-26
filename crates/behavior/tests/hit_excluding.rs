//! A hit shape with discs cut out: a point in a hole misses where the shape alone hits.

use craft_behavior::HitShape;
use emath::{Pos2, Rect, pos2};

#[test]
fn a_point_inside_a_hole_misses_and_one_beside_it_hits() {
    let shape = HitShape::Rect(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)));
    let hole = (pos2(10.0, 10.0), 5.0);
    let cut = HitShape::excluding(shape.clone(), vec![hole]);
    let (inside, beside): (Pos2, Pos2) = (pos2(12.0, 10.0), pos2(50.0, 50.0));
    assert!(shape.hit(inside), "the shape alone takes the point the hole covers");
    assert!(!cut.hit(inside), "a point in a hole misses");
    assert!(cut.hit(beside), "a point outside every hole still hits");
    assert_eq!(cut.bounds(), shape.bounds(), "the holes leave the bounds as they are");
}
