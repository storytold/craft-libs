//! The Shape tools and the Create Shape dialog. The view maps one document unit to one screen point.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect};
use craft_draw::geometry::shape::ShapeGeometry;
use craft_draw::ui::shape_drag::{CreateShapeDialog, CreateShapeOptions, CreatedShape, ShapeDragOptions, ShapeDragTool, ShapeKind};
use craft_ui::dialog::{DialogHost, DialogId, DialogResult};
use craft_ui::testing::{ToolFixture, trial};
use serde_json::{Value, json};

/// The view of a canvas filling `room`, with its middle at the document's middle.
fn room_view(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

fn tool(kind: ShapeKind) -> ShapeDragTool {
    let mut tool = ShapeDragTool::new();
    tool.options(ShapeDragOptions::new(kind));
    tool
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
