//! The canvas view's mapping between document and screen.

use craft_behavior::CanvasView;
use emath::{Rect as ScreenRect, pos2};
use kurbo::{Point, Rect};

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
