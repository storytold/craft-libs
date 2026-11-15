//! The canvas view's mapping between document and screen.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use emath::{Rect as ScreenRect, pos2, vec2};
use kurbo::{Point, Rect, Size};

fn view() -> CanvasView {
    CanvasView::new(ScreenRect::from_min_max(pos2(40.0, 30.0), pos2(840.0, 630.0)), Point::new(500.0, 400.0), 0.75)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_document_point_survives_the_trip_to_the_screen_and_back() {
    let views = [
        view(),
        view().rotation(33.0),
        view().rotation(-120.0).flip_x(true),
        view().aspect(2.0).flip_x(true),
        view().rotation(90.0).aspect(0.5),
        view().rotation(217.5).flip_x(true).aspect(1.5),
    ];
    for view in views {
        for p in [Point::new(0.0, 0.0), Point::new(512.0, 300.0), Point::new(-40.0, 1200.0)] {
            let back = view.to_doc(view.to_screen(p));
            assert!(near(back.x, p.x) && near(back.y, p.y), "{p:?} came back as {back:?} through {view:?}");
        }
    }
}

#[test]
fn quarter_turns_map_a_rect_to_an_axis_aligned_quad() {
    let doc = Rect::new(10.0, 20.0, 310.0, 220.0);
    for turns in 0..8 {
        for flip in [false, true] {
            let quad = view().rotation(90.0 * f64::from(turns)).flip_x(flip).aspect(1.25).quad(doc);
            for i in 0..4 {
                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                assert!(a.x == b.x || a.y == b.y, "edge {i} of turn {turns} (flip {flip}) is slanted: {a:?} to {b:?}");
            }
        }
    }
    let turned = view().rotation(30.0).quad(doc);
    assert!(turned[0].x != turned[1].x && turned[0].y != turned[1].y, "a 30 degree turn slants the edges");
}

#[test]
fn fit_and_fill_measure_the_stretched_document() {
    let size = Size::new(1000.0, 500.0);
    let rect = |w: f32, h: f32| ScreenRect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
    let stage = |aspect: f64, rect: ScreenRect| CanvasView::new(rect, Point::new(0.0, 0.0), 1.0).aspect(aspect);

    // Fit keeps 20 points of margin per side: 1000 points across for a 1000 px wide document, 2000 shown at aspect 2.
    let (mut square, mut wide) = (stage(1.0, rect(1040.0, 800.0)), stage(2.0, rect(1040.0, 800.0)));
    zoom::fit(&mut square, size, 1.0, 20.0);
    zoom::fit(&mut wide, size, 1.0, 20.0);
    assert!(near(square.zoom, 1.0), "{}", square.zoom);
    assert!(near(wide.zoom, 0.5), "{}", wide.zoom);
    assert!(near(wide.center.x, 500.0) && near(wide.center.y, 250.0), "{:?}", wide.center);

    // On a square rect the height limits fill at both aspects; the 1000 by 400 rect below is where the stretched width decides.
    let (mut square, mut wide) = (stage(1.0, rect(1000.0, 1000.0)), stage(2.0, rect(1000.0, 1000.0)));
    zoom::fill(&mut square, size, 1.0);
    zoom::fill(&mut wide, size, 1.0);
    assert!(near(square.zoom, 2.0) && near(wide.zoom, 2.0), "{} {}", square.zoom, wide.zoom);
    // A wide rect: the unstretched document is limited by its width, the stretched one by its height.
    let (mut square, mut wide) = (stage(1.0, rect(1000.0, 400.0)), stage(2.0, rect(1000.0, 400.0)));
    zoom::fill(&mut square, size, 1.0);
    zoom::fill(&mut wide, size, 1.0);
    assert!(near(square.zoom, 1.0), "{}", square.zoom);
    assert!(near(wide.zoom, 0.8), "{}", wide.zoom);
    assert!(near(wide.center.x, 500.0) && near(wide.center.y, 250.0), "{:?}", wide.center);
}
