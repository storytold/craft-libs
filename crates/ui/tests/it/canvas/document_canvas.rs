//! A document canvas owns its view in a pass: this pass's rect inside the rulers, the first fit, the wheel, a
//! middle-button pan, the Overscroll clamp and the zoom ease. The app only copies the view it reports.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::{Canvas, HandTool, NavigationOptions, ToolHost, canvas_space};
use craft_ui::contract::{Bind, ShowContainer as _};
use craft_ui::patterns::canvas::NavigatorPanel;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const DOC: Size = Size::new(2000.0, 1500.0);

const CANVAS: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));

const ROOM: Vec2 = Vec2::new(1400.0, 800.0);

#[derive(Clone, Copy, Default)]
struct Options {
    animated: bool,
    hand: bool,
    navigator: bool,
}

/// A canvas over `DOC` whose app copies the one view the host reports, and what its body saw in every pass.
struct Document {
    fixture: Fixture,
    rect: Rc<Cell<Rect>>,
    rulers: Rc<Cell<bool>>,
    /// The body's rect and the view in its `CanvasSpace`, one entry per pass.
    seen: Rc<RefCell<Vec<(Rect, CanvasView)>>>,
}

impl Document {
    fn new(start: CanvasView, rect: Rect, options: Options) -> Self {
        let view = Rc::new(RefCell::new(start));
        let rect = Rc::new(Cell::new(rect));
        let rulers = Rc::new(Cell::new(false));
        let seen: Rc<RefCell<Vec<(Rect, CanvasView)>>> = Rc::default();
        let (held, at, ruled, saw) = (view, Rc::clone(&rect), Rc::clone(&rulers), Rc::clone(&seen));
        let mut navigation = NavigationOptions::default();
        navigation.animated_zoom = options.animated;
        let mut hand = HandTool::new(navigation);
        let mut fixture = Fixture::new(move |ui| {
            if options.navigator {
                NavigatorPanel::new("Navigator", Bind::bound(&mut *held.borrow_mut()), DOC).navigation(navigation).show(ui);
            }
            let given = *held.borrow();
            let shown =
                Canvas::new("Document").view(given).document(DOC).navigation(navigation).show_rulers(ruled.get()).show_at(ui, at.get(), |_, body| {
                    if let Some(space) = canvas_space(&body.env) {
                        saw.borrow_mut().push((body.rect, space.view));
                    }
                });
            let next = if options.hand { ToolHost::new().run(ui, &shown.output, &mut hand).view } else { shown.output.view };
            if let Some(next) = next {
                *held.borrow_mut() = next;
            }
        });
        fixture.room(ROOM);
        Document { fixture, rect, rulers, seen }
    }

    fn last_seen(&self) -> (Rect, CanvasView) {
        *self.seen.borrow().last().expect("the body ran")
    }
}

/// `view.rect` lies in `body` past the rulers along the top and left edges, out to the body's far corner.
fn inside_the_rulers(body: Rect, view: CanvasView) -> bool {
    body.contains_rect(view.rect) && view.rect.min.x > body.min.x && view.rect.min.y > body.min.y && view.rect.max == body.max
}

#[trial]
fn the_body_reads_this_passes_rect_inside_the_rulers_while_they_show() {
    let start = CanvasView::new(Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)), Point::new(1000.0, 750.0), 0.5);
    let mut document = Document::new(start, CANVAS, Options::default());
    document.rulers.set(true);
    document.fixture.frame(Vec::new());
    let (body, first) = *document.seen.borrow().first().expect("the body ran in the first pass");
    assert!(inside_the_rulers(body, first), "the first pass's view lies inside the rulers: {:?} in {body:?}", first.rect);

    let wider = Rect::from_min_max(CANVAS.min, CANVAS.max + Vec2::new(200.0, 100.0));
    document.rect.set(wider);
    document.fixture.frame(Vec::new());
    let (body, after) = document.last_seen();
    assert_eq!(body.max, wider.max, "the canvas took its new rect");
    assert!(inside_the_rulers(body, after), "the pass after a resize carries the new rect: {:?} in {body:?}", after.rect);

    document.rulers.set(false);
    document.fixture.frame(Vec::new());
    let (body, plain) = document.last_seen();
    assert_eq!(plain.rect, body, "with the rulers off the view takes the canvas's whole rect");
}
