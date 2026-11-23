//! A document canvas owns its view in a pass: this pass's rect inside the rulers, the first fit, the wheel, a
//! middle-button pan, the Overscroll clamp and the zoom ease. The app only copies the view it reports.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::{Canvas, HandTool, NavigationOptions, Rulers, ToolHost, canvas_space, tool_pass};
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
            let mut hosted = None;
            let mut zero = Point::ZERO;
            let shown = Canvas::new("Document")
                .view(given)
                .document(DOC)
                .navigation(navigation)
                .with_rulers(ruled.get().then(|| Rulers::new("Rulers", Bind::bound(&mut zero))))
                .show_at(ui, at.get(), |ui, body| {
                    if let Some(space) = canvas_space(&body.env) {
                        saw.borrow_mut().push((body.rect, space.view));
                    }
                    if options.hand
                        && let Some(pass) = tool_pass(&body.env)
                    {
                        hosted = Some(ToolHost::new().run(ui, &pass, &mut hand));
                    }
                });
            let next = hosted.and_then(|hosted| hosted.view).or(shown.output.view);
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

/// The body is the view, which lies in `canvas` past the rulers along the top and left edges, out to the canvas's far
/// corner.
fn inside_the_rulers(canvas: Rect, body: Rect, view: CanvasView) -> bool {
    body == view.rect && canvas.contains_rect(body) && body.min.x > canvas.min.x && body.min.y > canvas.min.y && body.max == canvas.max
}

#[trial]
fn the_body_reads_this_passes_rect_inside_the_rulers_while_they_show() {
    let start = CanvasView::new(Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)), Point::new(1000.0, 750.0), 0.5);
    let mut document = Document::new(start, CANVAS, Options::default());
    document.rulers.set(true);
    document.fixture.frame(Vec::new());
    let (body, first) = *document.seen.borrow().first().expect("the body ran in the first pass");
    assert!(inside_the_rulers(CANVAS, body, first), "the first pass's view lies inside the rulers: {:?} in {body:?}", first.rect);

    let wider = Rect::from_min_max(CANVAS.min, CANVAS.max + Vec2::new(200.0, 100.0));
    document.rect.set(wider);
    document.fixture.frame(Vec::new());
    let (body, after) = document.last_seen();
    assert_eq!(body.max, wider.max, "the canvas took its new rect");
    assert!(inside_the_rulers(wider, body, after), "the pass after a resize carries the new rect: {:?} in {body:?}", after.rect);

    document.rulers.set(false);
    document.fixture.frame(Vec::new());
    let (body, plain) = document.last_seen();
    assert_eq!((plain.rect, body), (wider, wider), "with the rulers off the view takes the canvas's whole rect");
}
