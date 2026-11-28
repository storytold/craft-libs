//! The crop frame's geometry: resizing, fitting into the canvas, keeping a turned frame inside, the guides and the
//! turn grid.

use craft_behavior::geometry::{AspectHold, BoxPart, CropBox, CropGuide, turn_grid};
use kurbo::{Point, Rect, Size, Vec2};

#[test]
fn handles_resize_with_shift_alt_and_a_ratio() {
    let d = Vec2::new;
    let resize = |r: Rect, part: BoxPart, by: Vec2, ratio: Option<f64>, alt: bool| {
        CropBox::upright(r).resized(part, by, ratio.map_or(AspectHold::Free, AspectHold::Ratio), alt).rect
    };
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

#[test]
fn guides_stay_in_the_frame_and_orient_by_corner() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    for guide in CropGuide::ALL {
        for orientation in 0..guide.orientations() {
            for (w, h) in [(300.0, 200.0), (200.0, 300.0), (1.0, 4000.0)] {
                for line in guide.guides(orientation, w, h, 50.0, 96) {
                    assert!(line.len() >= 2, "{guide:?} {orientation}: a line has two ends");
                    for p in line {
                        let inside = (-1e-9..=1.0 + 1e-9).contains(&p.x) && (-1e-9..=1.0 + 1e-9).contains(&p.y);
                        assert!(inside, "{guide:?} {orientation} at {w} x {h}: {p:?} leaves the frame");
                    }
                }
            }
        }
        assert!(guide.guides(0, 0.5, 200.0, 50.0, 96).is_empty(), "{guide:?}: a frame under one point has no guide");
        assert!(guide.guides(0, f64::NAN, 200.0, 50.0, 96).is_empty(), "{guide:?}: a size that is not finite has no guide");
    }
    // Where a straight guide crosses each axis.
    let positions = |guide: CropGuide| {
        let lines = guide.guides(0, 300.0, 200.0, 50.0, 96);
        let mut xs: Vec<f64> = lines.iter().filter(|l| near(l[0].x, l[1].x)).map(|l| l[0].x).collect();
        let mut ys: Vec<f64> = lines.iter().filter(|l| near(l[0].y, l[1].y)).map(|l| l[0].y).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        (xs, ys)
    };
    let (xs, ys) = positions(CropGuide::Thirds);
    assert!(xs.len() == 2 && near(xs[0], 1.0 / 3.0) && near(xs[1], 2.0 / 3.0), "thirds across: {xs:?}");
    assert!(ys.len() == 2 && near(ys[0], 1.0 / 3.0) && near(ys[1], 2.0 / 3.0), "thirds down: {ys:?}");
    let golden = 0.618_033_988_749_894_9;
    let (xs, ys) = positions(CropGuide::GoldenRatio);
    assert!(xs.len() == 2 && near(xs[0], 1.0 - golden) && near(xs[1], golden), "golden section across: {xs:?}");
    assert!(ys.len() == 2 && near(ys[0], 1.0 - golden) && near(ys[1], golden), "golden section down: {ys:?}");
    // The Triangle's perpendiculars meet its diagonal at right angles on screen, whichever way the frame lies.
    for (w, h) in [(300.0, 200.0), (200.0, 300.0)] {
        let lines = CropGuide::Triangle.guides(0, w, h, 50.0, 96);
        assert_eq!(lines.len(), 3, "a diagonal and two perpendiculars");
        let screen = |p: Point| Vec2::new(p.x * w, p.y * h);
        let diagonal = screen(lines[0][1]) - screen(lines[0][0]);
        for perpendicular in &lines[1..] {
            let foot = perpendicular[1];
            assert!(near(foot.x, foot.y), "the foot lies on the diagonal: {foot:?}");
            let across = screen(foot) - screen(perpendicular[0]);
            assert!(across.dot(diagonal).abs() < 1e-6, "{w} x {h}: a right angle, {across:?} on {diagonal:?}");
        }
        // The other orientation runs the diagonal from the top right.
        let mirrored = CropGuide::Triangle.guides(1, w, h, 50.0, 96);
        assert!(mirrored[0][0].distance(Point::new(1.0, 0.0)) < 1e-9 && mirrored[0][1].distance(Point::new(0.0, 1.0)) < 1e-9, "{mirrored:?}");
    }
    // The Golden Spiral starts in a different corner in each of its four orientations.
    let mut starts: Vec<(i64, i64)> = (0..4)
        .map(|o| {
            let spiral = CropGuide::GoldenSpiral.guides(o, 300.0, 200.0, 50.0, 96);
            let start = spiral[0][0];
            assert!(near(start.x, start.x.round()) && near(start.y, start.y.round()), "orientation {o} starts in a corner: {start:?}");
            (start.x.round() as i64, start.y.round() as i64)
        })
        .collect();
    starts.sort_unstable();
    assert_eq!(starts, [(0, 0), (0, 1), (1, 0), (1, 1)]);
}

#[test]
fn the_turn_grid_has_centred_cells_and_only_visible_lines() {
    // Where the vertical and the horizontal lines lie, in screen points of a `w` by `h` frame.
    let positions = |w: f64, h: f64, step: f64, visible: Rect| {
        let lines = turn_grid(w, h, step, visible);
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for line in &lines {
            assert_eq!(line.len(), 2);
            if line[0].x == line[1].x {
                assert!(line[0].y == 0.0 && line[1].y == 1.0, "a line crosses the whole frame: {line:?}");
                xs.push((line[0].x * w * 1e6).round() / 1e6);
            } else {
                assert!(line[0].x == 0.0 && line[1].x == 1.0 && line[0].y == line[1].y, "{line:?}");
                ys.push((line[0].y * h * 1e6).round() / 1e6);
            }
        }
        (xs, ys)
    };
    let whole = |w: f64, h: f64| Rect::new(0.0, 0.0, w, h);
    // Counted from the centre, 48 points apart.
    let (xs, ys) = positions(200.0, 100.0, 48.0, whole(200.0, 100.0));
    assert_eq!(xs, [4.0, 52.0, 100.0, 148.0, 196.0]);
    assert_eq!(ys, [2.0, 50.0, 98.0]);
    // A line that would lie on the outline is left out.
    let (xs, ys) = positions(96.0, 96.0, 48.0, whole(96.0, 96.0));
    assert_eq!((xs, ys), (vec![48.0], vec![48.0]));
    // Only the lines inside the part of the frame that shows.
    let (xs, ys) = positions(200.0, 100.0, 48.0, Rect::new(60.0, 0.0, 120.0, 100.0));
    assert_eq!(xs, [100.0]);
    assert_eq!(ys, [2.0, 50.0, 98.0]);
    // A frame zoomed far past the screen still gives only the few lines in view.
    let (xs, _) = positions(1e7, 1e7, 48.0, Rect::new(5e6, 5e6, 5e6 + 100.0, 5e6 + 100.0));
    assert_eq!(xs, [5e6, 5e6 + 48.0, 5e6 + 96.0]);
    assert!(turn_grid(200.0, 100.0, 48.0, Rect::new(300.0, 0.0, 400.0, 100.0)).is_empty(), "nothing of the frame shows");
    assert!(turn_grid(0.5, 100.0, 48.0, whole(0.5, 100.0)).is_empty(), "a frame under one point");
    assert!(turn_grid(f64::NAN, 100.0, 48.0, whole(200.0, 100.0)).is_empty());
    assert!(turn_grid(200.0, 100.0, 0.0, whole(200.0, 100.0)).is_empty(), "no step");
    assert!(turn_grid(200.0, 100.0, 48.0, Rect::new(f64::NAN, 0.0, 200.0, 100.0)).is_empty());
}

#[test]
fn a_turned_frame_is_kept_inside_the_bounds_moved_then_shrunk() {
    let bounds = Rect::new(0.0, 0.0, 800.0, 500.0);
    let inside = |b: &CropBox| {
        let t = b.turned_bounds();
        t.x0 >= -1e-6 && t.y0 >= -1e-6 && t.x1 <= 800.0 + 1e-6 && t.y1 <= 500.0 + 1e-6
    };
    // A frame that fits stays bit for bit, also one that a rounding error puts a hair past the edge.
    let small = CropBox::new(Rect::new(300.0, 200.0, 500.0, 300.0), 10.0);
    assert_eq!(small.kept_inside(bounds), Some(small));
    let touching = CropBox::upright(bounds).fitted(Size::new(800.0, 500.0), 7.0).unwrap();
    assert_eq!(touching.kept_inside(bounds), Some(touching));
    // Fitted at 7°, its turned bounds span the whole height: 1e-9 down is a rounding error past the edge.
    let hair = touching.moved(Vec2::new(0.0, 1e-9));
    assert_eq!(hair.kept_inside(bounds), Some(hair));
    // Too far right but small enough: moved in the shortest way on that axis only, its size and angle kept.
    let past = CropBox::new(Rect::new(650.0, 200.0, 790.0, 300.0), 30.0);
    let moved = past.kept_inside(bounds).unwrap();
    assert!(inside(&moved), "{moved:?}");
    assert!((moved.rect.width() - past.rect.width()).abs() < 1e-9 && (moved.rect.height() - past.rect.height()).abs() < 1e-9, "{moved:?}");
    assert_eq!(moved.angle, past.angle);
    assert!((moved.turned_bounds().x1 - 800.0).abs() < 1e-9, "moved the least: {moved:?}");
    assert!((moved.center().y - past.center().y).abs() < 1e-12, "the axis that fits is kept: {moved:?}");
    // Too large at its angle: shrunk about its centre, proportions kept, to the largest that fits.
    let whole = CropBox::new(bounds, 30.0);
    let shrunk = whole.kept_inside(bounds).unwrap();
    assert!(inside(&shrunk), "{shrunk:?}");
    assert!((shrunk.rect.width() / shrunk.rect.height() - 1.6).abs() < 1e-9, "8 : 5 kept: {shrunk:?}");
    assert!(shrunk.rect.width() < 800.0, "{shrunk:?}");
    assert!(shrunk.center().distance(Point::new(400.0, 250.0)) < 1e-9, "{shrunk:?}");
    let t = shrunk.turned_bounds();
    assert!((t.height() - 500.0).abs() < 1e-6 || (t.width() - 800.0).abs() < 1e-6, "the largest that fits touches the bounds: {t:?}");
    // A large frame off the centre is shrunk, then moved in.
    let off = CropBox::new(Rect::new(300.0, 100.0, 1100.0, 600.0), 30.0);
    let kept = off.kept_inside(bounds).unwrap();
    assert!(inside(&kept) && (kept.rect.width() - shrunk.rect.width()).abs() < 1e-9, "{kept:?}");
    // A quarter turn swaps which side binds; a half turn fits as it is.
    let quarter = CropBox::new(bounds, 90.0).kept_inside(bounds).unwrap();
    assert!((quarter.rect.width() - 500.0).abs() < 1e-9 && (quarter.rect.height() - 312.5).abs() < 1e-9, "{quarter:?}");
    assert!(quarter.center().distance(Point::new(400.0, 250.0)) < 1e-9);
    let half = CropBox::new(bounds, 180.0);
    assert_eq!(half.kept_inside(bounds), Some(half));
    // Nothing to keep.
    assert_eq!(CropBox::upright(Rect::new(10.0, 10.0, 10.0, 50.0)).kept_inside(bounds), None);
    assert_eq!(CropBox::new(Rect::new(10.0, 10.0, 50.0, 50.0), f64::NAN).kept_inside(bounds), None);
    assert_eq!(small.kept_inside(Rect::new(0.0, 0.0, 0.0, 500.0)), None);
}

#[test]
fn degenerate_frames_give_none_or_finite_results() {
    let rects = [
        Rect::new(10.0, 10.0, 10.0, 10.0),
        Rect::new(10.0, 10.0, 10.0, 50.0),
        Rect::new(-1e300, -1e300, 1e300, 1e300),
        Rect::new(0.0, 0.0, 800.0, 500.0),
    ];
    let angles = [0.0, 30.0, -90.0, 1e300];
    let holds = [AspectHold::Free, AspectHold::Ratio(1.6), AspectHold::Diagonal];
    let parts = [BoxPart::Corner(2), BoxPart::Edge(1), BoxPart::Inside];
    let bounds = Rect::new(0.0, 0.0, 800.0, 500.0);
    for rect in rects {
        for angle in angles {
            let b = CropBox::new(rect, angle);
            for part in parts {
                for hold in holds {
                    for alt in [false, true] {
                        let by = Vec2::new(-5.0, 7.0);
                        let resized = b.resized(part, by, hold, alt);
                        assert!(resized.is_finite(), "{b:?} {part:?} {hold:?}: {resized:?}");
                        if let Some(centred) = b.resized_centred(part, by, hold, alt) {
                            assert!(centred.is_finite(), "{centred:?}");
                        }
                    }
                }
            }
            assert!(b.turned_to(0.3, Point::new(5.0, 9.0), Some(15.0), true).is_finite(), "{b:?}");
            if let Some(fitted) = b.fitted(Size::new(800.0, 500.0), angle) {
                assert!(fitted.is_finite(), "{fitted:?}");
            }
            if let Some(kept) = b.kept_inside(bounds) {
                assert!(kept.is_finite(), "{kept:?}");
            }
        }
    }
    // Input that is not finite gives nothing rather than a panic.
    for b in [CropBox::new(Rect::new(f64::NAN, 0.0, 10.0, 10.0), 0.0), CropBox::new(Rect::new(0.0, 0.0, 10.0, 10.0), f64::INFINITY)] {
        let _ = b.resized(BoxPart::Corner(0), Vec2::new(1.0, 1.0), AspectHold::Diagonal, false);
        assert_eq!(b.resized_centred(BoxPart::Corner(0), Vec2::new(1.0, 1.0), AspectHold::Diagonal, false), None);
        assert_eq!(b.kept_inside(bounds), None);
    }
    let huge = Rect::new(0.0, 0.0, 1e300, 1e300);
    for (w, h, step, visible) in [
        (1e300, 1e300, 48.0, huge),
        (1e300, 1e300, 1e-300, huge),
        (f64::INFINITY, 100.0, 48.0, huge),
        (200.0, 100.0, f64::NAN, huge),
        (200.0, 100.0, -1.0, huge),
    ] {
        let lines = turn_grid(w, h, step, visible);
        assert!(lines.len() <= 2 * 4096, "bounded: {}", lines.len());
        assert!(lines.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite()), "{w} x {h} by {step}");
    }
}
