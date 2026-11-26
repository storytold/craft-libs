//! The Shape tools and the Create Shape dialog. The view maps one document unit to one screen point.

use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::geometry::shape::ShapeGeometry;
use craft_draw::ui::shape::VectorMode;
use craft_draw::ui::shape_drag::{
    BoxConstraint, BoxConstraints, CreateShapeDialog, CreateShapeOptions, CreatedShape, ShapeDragEvent, ShapeDragOptions, ShapeDragTool, ShapeKind,
};
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogHost, DialogId, DialogResult};
use craft_ui::testing::{Fixture, ToolFixture, trial};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// The view of a canvas filling `room`, with its middle at the document's middle.
fn room_view(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

fn tool(kind: ShapeKind) -> ShapeDragTool {
    let mut tool = ShapeDragTool::new();
    tool.options(ShapeDragOptions::new(kind));
    tool
}

/// A Shape tool on a canvas, with the events it reported so far.
struct Shapes {
    fixture: ToolFixture<ShapeDragTool>,
    log: Vec<ShapeDragEvent>,
}

impl Shapes {
    fn new(options: ShapeDragOptions) -> Self {
        let mut tool = ShapeDragTool::new();
        tool.options(options);
        Shapes { fixture: ToolFixture::with_view("Shape", tool, room_view, |_, _| {}), log: Vec::new() }
    }

    fn sync(&mut self) {
        let events = self.fixture.tool_mut().take_events();
        self.log.extend(events);
    }

    /// The screen point of the document point `p`.
    fn at(&self, p: Point) -> Pos2 {
        room_view(self.fixture.canvas().rect).to_screen(p)
    }

    fn button(&mut self, p: Point, pressed: bool, modifiers: Modifiers) {
        let pos = self.at(p);
        self.fixture.fixture().frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }]);
        self.sync();
    }

    fn press(&mut self, p: Point, modifiers: Modifiers) {
        self.fixture.fixture().hold(modifiers);
        let pos = self.at(p);
        self.fixture.fixture().frame(vec![Event::PointerMoved(pos)]);
        self.button(p, true, modifiers);
    }

    fn move_to(&mut self, p: Point, modifiers: Modifiers) {
        self.fixture.fixture().hold(modifiers);
        let pos = self.at(p);
        self.fixture.fixture().frame(vec![Event::PointerMoved(pos)]);
        self.sync();
    }

    fn release(&mut self, p: Point, modifiers: Modifiers) {
        self.button(p, false, modifiers);
        self.fixture.fixture().hold(Modifiers::NONE);
        self.fixture.fixture().frame(Vec::new());
        self.sync();
    }

    /// Space goes down or up.
    fn space(&mut self, down: bool) {
        let key = Event::Key { key: egui::Key::Space, physical_key: None, pressed: down, repeat: false, modifiers: Modifiers::NONE };
        self.fixture.fixture().frame(vec![key]);
        self.sync();
    }

    /// Press at `from`, move past the press slop, then to `to`, and release there, all with `modifiers`.
    fn drag(&mut self, from: Point, to: Point, modifiers: Modifiers) {
        self.press(from, modifiers);
        self.move_to(from + Vec2::new(40.0, 30.0), modifiers);
        self.move_to(to, modifiers);
        self.release(to, modifiers);
    }

    /// The shape the last drag committed.
    fn committed(&self) -> Option<ShapeGeometry> {
        self.log.iter().rev().find_map(|event| match event {
            ShapeDragEvent::Draw(EditEvent::Commit { after, .. }) => Some(after.clone()),
            _ => None,
        })
    }

    /// The shape of the last `Update`.
    fn proposed(&self) -> Option<ShapeGeometry> {
        self.log.iter().rev().find_map(|event| match event {
            ShapeDragEvent::Draw(EditEvent::Update { value, .. }) => Some(value.clone()),
            _ => None,
        })
    }

    /// The box the last drag committed.
    fn committed_box(&self) -> Option<Rect> {
        self.committed().map(|shape| shape.bounds())
    }

    /// Whether the last gesture ended as a click.
    fn clicked(&self) -> bool {
        matches!(self.log.last(), Some(ShapeDragEvent::Click { .. }))
    }

    fn forget(&mut self) {
        self.log.clear();
    }
}

const PRESS: Point = Point::new(100.0, 100.0);

/// A ⇧ Line drag snaps onto the axis within 30° of it, keeping the offset along it and at least a pixel long, and
/// onto the diagonal at the shorter offset past that; the drag's last proposal is the line it commits.
#[trial]
fn a_shift_line_snaps_to_the_axis_or_the_diagonal_and_commits_what_it_proposed() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Line));
    let line_end = |shape: Option<ShapeGeometry>| match shape {
        Some(ShapeGeometry::Line { from, to, .. }) => Some(to - from),
        _ => None,
    };
    shapes.drag(PRESS, PRESS + Vec2::new(0.95, 0.4), Modifiers::SHIFT);
    assert_eq!(line_end(shapes.committed()), Some(Vec2::new(1.0, 0.0)), "a short drag still draws a 1 px horizontal line");
    assert_eq!(shapes.proposed(), shapes.committed(), "the last proposal is the committed line");

    shapes.forget();
    shapes.drag(PRESS, PRESS + Vec2::new(-60.0, 50.0), Modifiers::SHIFT);
    assert_eq!(line_end(shapes.committed()), Some(Vec2::new(-50.0, 50.0)), "past 30° the line takes the diagonal");
    assert_eq!(shapes.proposed(), shapes.committed());
}

/// ⇧ squares the box from the drag's shorter side, ⇧⌥ from the press as the centre; an Ellipse becomes a circle.
#[trial]
fn shift_squares_the_box_from_the_shorter_side() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle));
    shapes.drag(PRESS, PRESS + Vec2::new(60.0, 20.0), Modifiers::SHIFT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 120.0, 120.0)), "the square takes the shorter side at the press");

    shapes.forget();
    shapes.drag(PRESS, PRESS + Vec2::new(-60.0, 20.0), Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(80.0, 80.0, 120.0, 120.0)), "⇧⌥ centres the square on the press");

    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Ellipse));
    shapes.drag(PRESS, PRESS + Vec2::new(20.0, 60.0), Modifiers::SHIFT);
    assert_eq!(shapes.committed(), Some(ShapeGeometry::Ellipse { rect: Rect::new(100.0, 100.0, 120.0, 120.0) }), "a circle of the shorter side");
}

/// A Rectangle drag under the gear `constraint` with `change` applied to the gear's values.
fn gear(constraint: BoxConstraint, change: impl FnOnce(&mut BoxConstraints)) -> Shapes {
    let mut constraints = BoxConstraints { constraint, ..Default::default() };
    change(&mut constraints);
    Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle).constraints(constraints))
}

/// The gear's Square and From Center hold without ⇧ or ⌥, which turn them off; Fixed Size puts its size at the
/// release; Proportional draws the largest box of its ratio inside the drag; an empty Fixed Size field draws the
/// dragged box.
#[trial]
fn the_rectangle_gear_constrains_the_drag_and_shift_and_alt_turn_it_off() {
    let to = PRESS + Vec2::new(60.0, 20.0);
    let mut shapes = gear(BoxConstraint::Square, |_| {});
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 120.0, 120.0)), "Square holds without ⇧");
    shapes.drag(PRESS, to, Modifiers::SHIFT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 120.0)), "⇧ turns Square off");

    let mut shapes = gear(BoxConstraint::Unconstrained, |c| c.from_center = true);
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(40.0, 80.0, 160.0, 120.0)), "From Center draws about the press");
    shapes.drag(PRESS, to, Modifiers::ALT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 120.0)), "⌥ turns From Center off");

    let mut shapes = gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(30.0), Some(10.0)));
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(160.0, 120.0, 190.0, 130.0)), "Fixed Size puts its top left at the release");

    let mut shapes = gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(30.0), None));
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 120.0)), "an empty field draws the dragged box");

    let mut shapes = gear(BoxConstraint::Proportional, |c| c.ratio = (Some(2.0), Some(1.0)));
    shapes.drag(PRESS, PRESS + Vec2::new(60.0, 50.0), Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 130.0)), "the largest 2 : 1 box inside the drag");
}

/// The corners of the triangle `shape` draws, from the apex clockwise.
fn corners(shape: Option<ShapeGeometry>) -> Vec<Point> {
    let Some(shape @ ShapeGeometry::Triangle { .. }) = shape else { return Vec::new() };
    shape.to_path(0.1).elements().iter().filter_map(|el| el.end_point()).collect()
}

/// A Triangle drag fills its box, apex on top; ⇧ draws the equilateral triangle in the circle of the shorter
/// side's square; ⇧⌥ centres it on the press.
#[trial]
fn a_triangle_fills_its_box_and_shift_makes_it_equilateral() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Triangle));
    let to = PRESS + Vec2::new(60.0, 40.0);
    shapes.drag(PRESS, to, Modifiers::NONE);
    let plain = corners(shapes.committed());
    assert_eq!(plain.first(), Some(&Point::new(130.0, 100.0)), "the apex is the box's top centre: {plain:?}");
    assert!(plain.len() >= 3 && plain[1..].iter().all(|p| p.y == 140.0), "the base lies on the bottom edge: {plain:?}");

    shapes.drag(PRESS, to, Modifiers::SHIFT);
    let equal = corners(shapes.committed());
    assert!(equal.len() >= 3, "a triangle: {equal:?}");
    let sides = [(equal[0] - equal[1]).hypot(), (equal[1] - equal[2]).hypot(), (equal[2] - equal[0]).hypot()];
    assert!(sides.iter().all(|side| (side - 20.0 * 3f64.sqrt()).abs() < 1e-9), "three equal sides in the 40 px square's circle: {sides:?}");
    assert!((equal[0] - Point::new(120.0, 100.0)).hypot() < 1e-9, "the apex is the top of the circle, got {:?}", equal[0]);

    shapes.drag(PRESS, to, Modifiers::SHIFT | Modifiers::ALT);
    let centred = corners(shapes.committed());
    assert!(centred.len() >= 3, "a triangle: {centred:?}");
    let centre = (centred[0].to_vec2() + centred[1].to_vec2() + centred[2].to_vec2()) / 3.0;
    assert!((centre.to_point() - PRESS).hypot() < 1e-9, "⇧⌥ puts the centre on the press, got {centre:?}");
}

/// A press moved 2 × 1 points and released: a Rectangle in Shape mode draws it, in Path mode it is a click, and an
/// Ellipse in Shape mode keeps the press slop.
#[trial]
fn a_shape_mode_rectangle_ends_a_click_within_one_point() {
    let nudge = |kind: ShapeKind, mode: VectorMode| {
        let mut shapes = Shapes::new(ShapeDragOptions::new(kind).mode(mode));
        let to = PRESS + Vec2::new(2.0, 1.0);
        shapes.press(PRESS, Modifiers::NONE);
        shapes.move_to(to, Modifiers::NONE);
        shapes.release(to, Modifiers::NONE);
        shapes
    };
    let drawn = nudge(ShapeKind::Rectangle, VectorMode::Shape);
    assert!(!drawn.clicked(), "the Rectangle in Shape mode drew: {:?}", drawn.log);
    assert_eq!(drawn.committed_box(), Some(Rect::new(100.0, 100.0, 102.0, 101.0)));
    assert!(nudge(ShapeKind::Rectangle, VectorMode::Path).clicked(), "in Path mode the press slop holds");
    assert!(nudge(ShapeKind::Ellipse, VectorMode::Shape).clicked(), "other kinds keep the press slop");
}

/// Space held moves the box by the pointer's whole motion, the first move after Space went down included; released,
/// the drag resizes again.
#[trial]
fn space_moves_the_box_by_the_full_motion() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle));
    let corner = PRESS + Vec2::new(40.0, 30.0);
    shapes.press(PRESS, Modifiers::NONE);
    shapes.move_to(corner, Modifiers::NONE);
    assert_eq!(shapes.proposed().map(|shape| shape.bounds()), Some(Rect::new(100.0, 100.0, 140.0, 130.0)));

    shapes.space(true);
    shapes.move_to(corner + Vec2::new(10.0, 5.0), Modifiers::NONE);
    assert_eq!(shapes.proposed().map(|shape| shape.bounds()), Some(Rect::new(110.0, 105.0, 150.0, 135.0)), "the first move moves the box");

    shapes.space(false);
    let end = corner + Vec2::new(30.0, 25.0);
    shapes.move_to(end, Modifiers::NONE);
    shapes.release(end, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(110.0, 105.0, 170.0, 155.0)), "without Space the drag resizes from the moved press");
}

#[trial]
fn the_shape_tool_owes_the_canvas_what_every_tool_owes() {
    ToolFixture::with_view("Shape", tool(ShapeKind::Rectangle), room_view, |_, _| {}).check();
}

/// Opens Create Shape for a click of `kind` at (10, 20) and returns its id.
fn open_create(host: &mut DialogHost, kind: ShapeKind) -> DialogId {
    host.open(CreateShapeDialog::new(CreateShapeOptions::initial(kind, Point::new(10.0, 20.0))))
}

/// The fields automation lists for the open dialog `id`.
fn fields(host: &DialogHost, id: DialogId) -> Value {
    host.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).unwrap_or(Value::Null)
}

/// OK through the host; the shape it answered.
fn confirm(host: &mut DialogHost, ctx: &egui::Context) -> Option<CreatedShape> {
    assert!(host.confirm::<CreateShapeDialog>(ctx), "OK is enabled");
    host.take::<CreateShapeDialog>().and_then(DialogResult::ok)
}

/// The bounds and star ratio of a polygon or star.
fn star_of(shape: &CreatedShape) -> Option<(Rect, f64)> {
    match shape.geometry {
        ShapeGeometry::Polygon { rect, star_ratio, .. } => Some((rect, star_ratio)),
        _ => None,
    }
}

/// OK remembers width, height, From Center and the star ratio for the kind, and the next open of that kind starts
/// from them (before any pass, as automation meets it); Cancel remembers nothing; the unit picked for a field is not
/// remembered; another kind keeps its own memory.
#[trial]
fn create_shape_ok_remembers_the_size_and_cancel_and_a_picked_unit_do_not() {
    let ctx = egui::Context::default();
    let mut host = DialogHost::new();

    let id = open_create(&mut host, ShapeKind::Star);
    assert_eq!(fields(&host, id)["width"], json!(100.0), "a first open starts at 100 × 100 px");
    for (field, value) in [("width", json!(250)), ("height", json!(80)), ("starRatio", json!(40)), ("fromCenter", json!(true))] {
        assert_eq!(host.set_json_field(&ctx, id, field, value), Ok(()), "{field} takes its value");
    }
    assert!(host.set_json_field(&ctx, id, "depth", json!(3)).is_err(), "an unknown field is refused");
    assert_eq!(host.set_json_field(&ctx, id, "widthUnit", json!("inches")), Ok(()));
    let listed = fields(&host, id);
    assert_eq!((listed["widthUnit"].clone(), listed["height"].clone()), (json!("inches"), json!(80.0)), "the set values are listed");
    let shape = confirm(&mut host, &ctx).expect("OK answers the shape");
    let centred = Rect::new(10.0 - 125.0, 20.0 - 40.0, 10.0 + 125.0, 20.0 + 40.0);
    assert_eq!(star_of(&shape), Some((centred, 0.4)), "the answer carries the set values, centred on the click");
    assert!(shape.from_center);

    let id = open_create(&mut host, ShapeKind::Star);
    assert_eq!(host.set_json_field(&ctx, id, "width", json!(300)), Ok(()));
    let listed = fields(&host, id);
    assert_eq!(listed["widthUnit"], json!("pixels"), "the picked unit is not remembered: the width is in pixels again");
    assert_eq!((listed["height"].clone(), listed["starRatio"].clone(), listed["fromCenter"].clone()), (json!(80.0), json!(40.0), json!(true)));
    assert_eq!(host.set_json_field(&ctx, id, "fromCenter", json!(false)), Ok(()));
    assert!(host.cancel::<CreateShapeDialog>(&ctx));
    assert_eq!(host.take::<CreateShapeDialog>(), Some(DialogResult::Cancel));

    open_create(&mut host, ShapeKind::Star);
    let shape = confirm(&mut host, &ctx).expect("OK before any pass answers the remembered values");
    assert_eq!(star_of(&shape), Some((centred, 0.4)), "Cancel remembered nothing: the OK'd size and ratio come back");
    assert!(shape.from_center);

    open_create(&mut host, ShapeKind::Ellipse);
    let ellipse = confirm(&mut host, &ctx).expect("an ellipse answers");
    assert_eq!(ellipse.geometry.bounds(), Rect::new(10.0, 20.0, 110.0, 120.0), "an ellipse keeps its own memory: the defaults");
}

/// A width typed past 1–300,000 px keeps 300,000 and raises the range alert once, for every kind; a width in range
/// raises none.
#[trial]
fn create_shape_alerts_once_for_a_size_typed_past_its_range() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Triangle] {
        let host = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.room(egui::Vec2::new(1200.0, 900.0));
        let options = CreateShapeOptions::initial(kind, Point::new(10.0, 20.0)).recalled();
        host.borrow_mut().open(CreateShapeDialog::new(options).alert_title("PhotoCraft"));
        fixture.settle();
        let width = fixture
            .by_role(Role::SPIN_BUTTON)
            .into_iter()
            .find(|field| fixture.tree().name_of(field.node) == Some("Width"))
            .expect("the Width field");
        let type_width = |fixture: &mut Fixture, text: &str| {
            fixture.click(&width).settle();
            fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
            fixture.key(Key::Tab, Modifiers::NONE).settle();
        };
        let width_now = |host: &Rc<RefCell<DialogHost>>| host.borrow().get::<CreateShapeDialog>().map(|dialog| dialog.options().size.0);
        let take = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().get_mut::<CreateShapeDialog>().and_then(CreateShapeDialog::take_range_alert);

        type_width(&mut fixture, "400000");
        assert_eq!(width_now(&host), Some(300_000.0), "{kind:?}: the field keeps the end of the range");
        assert!(take(&host).is_some(), "{kind:?}: the typed width raises the range alert");
        assert!(take(&host).is_none(), "{kind:?}: the alert is taken once");

        type_width(&mut fixture, "500");
        assert_eq!(width_now(&host), Some(500.0));
        assert!(take(&host).is_none(), "{kind:?}: a width in range raises no alert");
    }
}
