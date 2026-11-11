//! Annotation geometry.

use craft_ui::testing::trial;
use kurbo::{Point, Rect};

use crate::annotate::{Annotation, AnnotationHandle, AnnotationHit, AnnotationId, AnnotationKind, hit};

/// The icon side, in page units, the hit tests are given.
const ICON: f64 = 16.0;

fn annotation(id: u64, kind: AnnotationKind, rect: Rect) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        page: 0,
        kind,
        rect,
        points: Vec::new(),
        strokes: Vec::new(),
        color: [255, 0, 0],
        opacity: 1.0,
        stroke_width: 2.0,
        text: String::new(),
        locked: false,
    }
}

fn square(id: u64, kind: AnnotationKind) -> Annotation {
    annotation(id, kind, Rect::new(10.0, 10.0, 50.0, 30.0))
}

// handles by kind; none for markups and locked ones.

// hit order: selected handles, then last-drawn bodies.
#[trial]
fn hit_prefers_handles_then_the_topmost_body() {
    let (below, above) = (square(1, AnnotationKind::Rectangle), square(2, AnnotationKind::Rectangle));
    let all = [below, above];
    let inside = Point::new(30.0, 20.0);
    assert_eq!(hit(&all, &[], inside, 2.0, ICON), Some(AnnotationHit::Body(AnnotationId(2))));
    let corner = Point::new(11.0, 11.0);
    assert_eq!(hit(&all, &[AnnotationId(1)], corner, 2.0, ICON), Some(AnnotationHit::Handle(AnnotationId(1), AnnotationHandle::Box(-1, -1))));
    assert_eq!(hit(&all, &[], Point::new(200.0, 200.0), 2.0, ICON), None);

    let mut line = square(3, AnnotationKind::Line);
    line.points = vec![Point::new(100.0, 100.0), Point::new(200.0, 100.0)];
    assert_eq!(hit(&[line.clone()], &[], Point::new(150.0, 101.0), 2.0, ICON), Some(AnnotationHit::Body(AnnotationId(3))));
    assert_eq!(hit(&[line], &[], Point::new(150.0, 110.0), 2.0, ICON), None);
}

// reshaped with shift snaps the segment.

// AnnotationGroup::tools order and Eraser in Draw.
