//! photocraft#16790: Shift squares a Normal-style marquee from the shorter side of the drag, whether it is pressed after
//! the drag started or, with nothing selected, held from the press; Shift+Alt centres that square on the start point.

use craft_behavior::geometry::{Constraint, SelectionOperation};
use craft_behavior::{CancelReason, PixelSize, PointerButton};
use craft_imaging::ui::select::{MarqueeKind, MarqueeTool, SelectEvent, SelectToolPass, SelectionProbe, SelectionShape};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolContext, ToolHost, ToolInput, ToolKey, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Rect as ScreenRect, pos2};
use kurbo::{Point, Rect};
use std::cell::RefCell;

const NONE: Mods = Mods::NONE;
const SHIFT: Mods = Mods { shift: true, ..NONE };
const SHIFT_ALT: Mods = Mods { shift: true, alt: true, ..NONE };

/// A 1000 × 800 document, with an optional selection.
struct Probe(Option<Rect>);

impl SelectionProbe for Probe {
    fn document(&self) -> u64 {
        1
    }

    fn size(&self) -> PixelSize {
        PixelSize::new(1000, 800)
    }

    fn has_selection(&self) -> bool {
        self.0.is_some()
    }

    fn inside_selection(&self, at: Point) -> bool {
        self.0.is_some_and(|rect| rect.contains(at))
    }

    fn bounds(&self) -> Option<Rect> {
        self.0
    }

    fn floating(&self) -> bool {
        false
    }

    fn on_floating(&self, _: Point) -> bool {
        false
    }
}

/// The marquee with its probe, building the pass the app builds on every call.
struct Marquee {
    tool: RefCell<MarqueeTool>,
    probe: Probe,
}

impl Marquee {
    fn pass(&self) -> std::cell::RefMut<'_, MarqueeTool> {
        self.tool.borrow_mut()
    }
}

impl CanvasTool for Marquee {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).each_pass(cx);
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> craft_ui::canvas::ToolAction {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).press(pointer, cx)
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).drag(pointer, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).release(pointer, cx);
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).hover(pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).cancel(reason, cx);
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).key(key, cx)
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        SelectToolPass::new(&mut *self.pass(), &self.probe).paint(draw, cx);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> craft_behavior::Cursor {
        SelectToolPass::new(&mut *self.pass(), &self.probe).cursor(cx)
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        SelectToolPass::new(&mut *self.pass(), &self.probe).takes(button, mods)
    }

    fn claims(&self) -> craft_ui::canvas::ToolClaims {
        SelectToolPass::new(&mut *self.pass(), &self.probe).claims()
    }

    fn takes_every_move(&self) -> bool {
        SelectToolPass::new(&mut *self.pass(), &self.probe).takes_every_move()
    }

    fn command_form(&self) -> craft_ui::canvas::ToolPath {
        SelectToolPass::new(&mut *self.pass(), &self.probe).command_form()
    }
}

/// A canvas at 100 % whose document point and screen point coincide.
fn rig(selection: Option<Rect>) -> ToolFixture<Marquee> {
    let same_points = |room: ScreenRect| {
        let centre = room.center();
        craft_behavior::CanvasView::new(room, Point::new(f64::from(centre.x), f64::from(centre.y)), 1.0)
    };
    let tool = Marquee { tool: RefCell::new(MarqueeTool::new(MarqueeKind::Rectangle)), probe: Probe(selection) };
    ToolFixture::with_view("Marquee", tool, same_points, |_, _| {})
}

fn inject(rig: &mut ToolFixture<Marquee>, input: ToolInput) {
    let (ctx, id) = (rig.fixture().ctx().clone(), rig.canvas_id());
    ToolHost::inject(&ctx, id, input);
    rig.fixture().run_injected();
}

/// A press with `at_press` held, a drag and a release with `later` held: the rectangle committed and its operation.
fn drag(rig: &mut ToolFixture<Marquee>, from: (f64, f64), to: (f64, f64), at_press: Mods, later: Mods) -> (Rect, SelectionOperation) {
    let at = |p: (f64, f64)| pos2(p.0 as f32, p.1 as f32);
    inject(rig, ToolInput::down(PointerButton::Primary, at(from)).mods(at_press));
    inject(rig, ToolInput::moved(at(to)).mods(later));
    inject(rig, ToolInput::up(at(to)).mods(later));
    let events = rig.tool_mut().tool.get_mut().take_events();
    match events.as_slice() {
        [SelectEvent::Commit { shape: SelectionShape::Rectangle(rect), operation, .. }] => (*rect, *operation),
        _ => panic!("one rectangle commit: {events:?}"),
    }
}

fn size(rect: Rect) -> (f64, f64) {
    (rect.width(), rect.height())
}

#[trial]
fn shift_pressed_after_the_start_squares_from_the_shorter_side() {
    let mut marquee = rig(None);
    let (rect, _) = drag(&mut marquee, (100.5, 100.5), (300.5, 150.5), NONE, SHIFT);
    assert_eq!(rect, Rect::new(100.0, 100.0, 151.0, 151.0), "a 200 x 50 drag selects 51 x 51 from the start point");
    // Up and to the left, the square still grows from the start point towards the pointer.
    let (rect, _) = drag(&mut marquee, (300.5, 250.5), (100.5, 200.5), NONE, SHIFT);
    assert_eq!(rect, Rect::new(250.0, 200.0, 301.0, 251.0));
    // A taller drag squares from its width.
    let (rect, _) = drag(&mut marquee, (100.5, 100.5), (150.5, 300.5), NONE, SHIFT);
    assert_eq!(size(rect), (51.0, 51.0));
}

#[trial]
fn shift_alt_pressed_after_the_start_centres_a_square_of_the_shorter_half_extent() {
    let mut marquee = rig(None);
    let (rect, _) = drag(&mut marquee, (200.5, 150.5), (320.5, 180.5), NONE, SHIFT_ALT);
    // Half extents 120 and 30: the 30 wins, so the square runs 30 each way from the start point.
    assert_eq!(rect, Rect::new(170.0, 120.0, 231.0, 181.0));
    assert_eq!(size(rect), (61.0, 61.0));
}

#[trial]
fn shift_held_from_the_press_with_nothing_selected_squares_from_the_shorter_side() {
    let mut marquee = rig(None);
    let (rect, operation) = drag(&mut marquee, (100.0, 100.0), (300.0, 200.0), SHIFT, SHIFT);
    assert_eq!(size(rect), (100.0, 100.0), "a 200 x 100 drag selects 100 x 100");
    assert_eq!(rect.origin(), Point::new(100.0, 100.0));
    assert_eq!(operation, SelectionOperation::Add);
}

#[trial]
fn with_a_selection_shift_or_alt_at_the_press_pick_the_operation_without_squaring() {
    let mut marquee = rig(Some(Rect::new(600.0, 600.0, 700.0, 700.0)));
    let (rect, operation) = drag(&mut marquee, (100.0, 100.0), (300.0, 200.0), SHIFT, SHIFT);
    assert_eq!((size(rect), operation), ((200.0, 100.0), SelectionOperation::Add));
    let alt = Mods { alt: true, ..NONE };
    let (rect, operation) = drag(&mut marquee, (100.0, 100.0), (300.0, 200.0), alt, alt);
    assert_eq!((size(rect), operation), ((200.0, 100.0), SelectionOperation::Subtract));
}

#[trial]
fn fixed_ratio_fixed_size_a_released_shift_and_a_square_between_pixels_keep_their_behaviour() {
    let mut marquee = rig(None);
    // Shift released before the mouse-up: the plain rectangle.
    let (rect, _) = drag(&mut marquee, (100.5, 100.5), (300.5, 150.5), NONE, NONE);
    assert_eq!(size(rect), (201.0, 51.0));
    // #5472: a square pressed between pixels has equal sides.
    let (rect, _) = drag(&mut marquee, (10.3, 10.7), (50.6, 30.2), NONE, SHIFT);
    assert!(rect.width() > 0.0 && rect.width() == rect.height(), "{rect:?}");
    // Fixed Ratio keeps the larger drag extent.
    marquee.tool_mut().tool.get_mut().constraint(Constraint::Ratio(1.0, 1.0));
    let (rect, _) = drag(&mut marquee, (100.0, 100.0), (300.0, 150.0), NONE, NONE);
    assert_eq!(size(rect), (200.0, 200.0));
    // Fixed Size ignores Shift.
    marquee.tool_mut().tool.get_mut().constraint(Constraint::Size(64.0, 32.0));
    let (rect, _) = drag(&mut marquee, (100.0, 100.0), (300.0, 150.0), NONE, SHIFT);
    assert_eq!(size(rect), (64.0, 32.0));
}
