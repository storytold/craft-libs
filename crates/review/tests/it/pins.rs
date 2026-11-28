//! Point and area comments on the same canvas.

use std::cell::Cell;
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_review::CommentAnchor;
use craft_review::comments::ThreadId;
use craft_review::ui::{CommentPin, CommentPins};
use craft_ui::contract::Role;
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Shape, vec2};
use kurbo::{Point, Rect as DocumentRect};

const POINT: ThreadId = ThreadId(1);
const AREA: ThreadId = ThreadId(2);
const BOUNDS: DocumentRect = DocumentRect::new(140.0, 85.0, 220.0, 145.0);

fn pins(view: &CanvasView) -> [CommentPin<'static>; 2] {
    let point = CommentAnchor::Canvas { at: [50.0, 60.0], node: None };
    let area = CommentAnchor::CanvasArea { bounds: BOUNDS };
    let color = [200, 80, 40];
    [
        CommentPin::from_canvas_anchor(POINT, &point, view, "Ana", "Point", color).expect("point pin"),
        CommentPin::from_canvas_anchor(AREA, &area, view, "Ana", "Region", color).expect("area pin"),
    ]
}

#[trial]
fn point_and_area_pins_open_their_threads() {
    let selected = Rc::new(Cell::new(None));
    let open = selected.clone();
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.max_rect(), Point::new(200.0, 150.0), 1.0);
        let shown = CommentPins::new("Comments", &pins(&view), "Me").open(open.get()).show(ui);
        if let Some(id) = shown.opened {
            open.set(Some(id));
        }
    });
    fixture.settle();
    let point = fixture.by_label("Comment by Ana: Point").expect("point button");
    fixture.click(&point).settle();
    assert_eq!(selected.get(), Some(POINT));

    let area = fixture.by_label("Area comment by Ana: Region").expect("area button");
    assert_eq!(fixture.tree().role_of(area.node), Some(Role::BUTTON));
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    if fixture.tree().focused_key() == point.node {
        fixture.key(Key::Tab, Modifiers::NONE).settle();
    }
    let focused = fixture.tree().focused_key();
    assert_eq!(focused, area.node, "Tab reached {:?}", fixture.tree().name_of(focused));
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(selected.get(), Some(AREA));
    assert_eq!(fixture.tree().expanded_of(area.node), Some(true));
}

#[trial]
fn the_selected_area_is_stroked_at_its_document_bounds_after_zoom_and_pan() {
    let room = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let initial = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
    let zoomed_and_panned = CanvasView::new(room, Point::new(180.0, 130.0), 1.4);
    let mut frames = Frames::new();
    for (view, open) in [(initial, AREA), (zoomed_and_panned, AREA), (zoomed_and_panned, POINT)] {
        let shapes = frames.frame_shapes(|ui| {
            let _ = CommentPins::new("Comments", &pins(&view), "Me").open(Some(open)).show(ui);
        });
        let corners = view.quad(BOUNDS);
        let outlines: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Path(path) if path.points.as_slice() == corners => Some(path),
                _ => None,
            })
            .collect();
        if open == AREA {
            assert!(!outlines.is_empty(), "the stroke follows all four projected corners");
            assert!(outlines.iter().all(|path| path.closed && !path.stroke.is_empty() && path.fill == Color32::TRANSPARENT));
        } else {
            assert!(outlines.is_empty(), "the area is outlined only while selected");
        }
    }
}
