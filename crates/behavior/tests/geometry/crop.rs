//! The crop frame's geometry: resizing and fitting into the canvas.

use craft_behavior::geometry::{BoxPart, CropBox};
use kurbo::{Point, Rect, Size, Vec2};

#[test]
fn handles_resize_with_shift_alt_and_a_ratio() {
    let d = Vec2::new;
    let resize = |r: Rect, part: BoxPart, by: Vec2, ratio: Option<f64>, alt: bool| CropBox::upright(r).resized(part, by, ratio, alt).rect;
    let r = Rect::new(20.0, 20.0, 100.0, 60.0);
    assert_eq!(resize(r, BoxPart::Edge(1), d(20.0, 10.0), None, false), Rect::new(20.0, 20.0, 120.0, 60.0));
    assert_eq!(resize(r, BoxPart::Corner(2), d(40.0, 10.0), None, false), Rect::new(20.0, 20.0, 140.0, 70.0));
    // A ratio: the larger relative extent wins at a corner, the opposite corner stays.
    let k = 120.0 / 50.0;
    let held = resize(Rect::new(20.0, 20.0, 140.0, 70.0), BoxPart::Corner(0), d(-10.0, -20.0), Some(k), false);
    assert_eq!((held.x1, held.y1), (140.0, 70.0));
    assert!((held.width() / held.height() - k).abs() < 1e-9, "{held:?}");
    assert!((held.height() - 70.0).abs() < 1e-9, "the larger relative extent: {held:?}");
    // Alt about the centre.
    assert_eq!(resize(Rect::new(40.0, 20.0, 80.0, 60.0), BoxPart::Edge(1), d(10.0, 0.0), None, true), Rect::new(30.0, 20.0, 90.0, 60.0));
    // A ratio on an edge: centred on the other axis.
    assert_eq!(resize(Rect::new(40.0, 20.0, 80.0, 60.0), BoxPart::Edge(1), d(20.0, 0.0), Some(1.0), false), Rect::new(40.0, 10.0, 100.0, 70.0));
    // Past the other side the frame flips.
    assert_eq!(resize(Rect::new(20.0, 20.0, 60.0, 60.0), BoxPart::Edge(1), d(-50.0, 0.0), None, false), Rect::new(10.0, 20.0, 20.0, 60.0));
}

#[test]
fn fitted_is_the_largest_frame_of_the_same_proportions_inside_the_canvas() {
    let canvas = Size::new(200.0, 100.0);
    let frame = CropBox::upright(Rect::new(0.0, 0.0, 200.0, 100.0));
    assert_eq!(frame.fitted(canvas, 0.0), Some(frame));
    for deg in [-45.0, -10.0, 3.0, 10.0, 30.0, 45.0, 90.0] {
        let f = frame.fitted(canvas, deg).unwrap();
        let inside = |b: &CropBox| b.corners().iter().all(|p| p.x >= -1e-6 && p.y >= -1e-6 && p.x <= 200.0 + 1e-6 && p.y <= 100.0 + 1e-6);
        assert!(inside(&f), "{deg}: {f:?}");
        assert!((f.rect.width() / f.rect.height() - 2.0).abs() < 1e-9, "keeps 2:1 at {deg}");
        assert!(f.rect.center().distance(Point::new(100.0, 50.0)) < 1e-9, "centred at {deg}");
        assert_eq!(f.angle, deg);
        let c = f.rect.center();
        let bigger = CropBox::new(
            Rect::new(c.x - f.rect.width() * 0.505, c.y - f.rect.height() * 0.505, c.x + f.rect.width() * 0.505, c.y + f.rect.height() * 0.505),
            deg,
        );
        assert!(!inside(&bigger), "{deg}: 1% larger still fits");
    }
    // A 100x100 frame in a 200x100 canvas at 0 degrees is as large as the canvas allows.
    let square = CropBox::upright(Rect::new(10.0, 10.0, 110.0, 110.0)).fitted(canvas, 0.0).unwrap();
    assert_eq!(square.rect, Rect::new(50.0, 0.0, 150.0, 100.0));
    // A flipped rect keeps its proportions.
    assert_eq!(CropBox::upright(Rect::new(200.0, 100.0, 0.0, 0.0)).fitted(canvas, 0.0).unwrap().rect, Rect::new(0.0, 0.0, 200.0, 100.0));
    for bad in [Size::new(0.0, 100.0), Size::new(200.0, -1.0), Size::new(f64::NAN, 100.0)] {
        assert_eq!(frame.fitted(bad, 0.0), None);
    }
    assert_eq!(frame.fitted(canvas, f64::NAN), None);
    assert_eq!(CropBox::upright(Rect::new(1.0, 1.0, 1.0, 5.0)).fitted(canvas, 0.0), None);
}
