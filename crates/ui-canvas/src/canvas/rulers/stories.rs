//! Stories of the rulers.

use super::{CORNER, RulerMargins, Rulers};
use crate::canvas::testing::canvas_story::{StoryDocument, on_ruled_canvas};
use crate::canvas::{Canvas, RasterSource as _, RulerText};
use crate::contract::Validity;
use crate::primitives::RulerFace;
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use crate::testing::{pointer, stage};
use craft_behavior::kurbo::Point;
use craft_behavior::{Mixed, RulerUnits, TickLabels, Ticks};
use egui::{Rect, Ui, Vec2};

/// How far the corner is dragged, in drag thresholds: out of the corner and onto the document.
const DRAG_THRESHOLDS: f32 = 12.0;

/// The least x the invalid story accepts for the zero point.
const LEAST_X: f64 = 0.0;

/// The margin the face stories shade on each side of the document, pixels.
const MARGIN: f64 = 48.0;

/// The units the menu story offers.
const UNITS: [UnitKind; 2] = [UnitKind::Pixels, UnitKind::Points];

fn pixels() -> RulerText {
    RulerText::new(RulerUnits::pixels(), UnitKind::Pixels)
}

fn drag_corner(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(corner) = fixture.part(id, CORNER) else { return };
    let travel = DRAG_THRESHOLDS * crate::testing::input::slop(fixture.ctx());
    fixture.drag(&corner, Vec2::splat(travel));
}

/// Opens the units menu with a secondary click on the corner.
fn open_units_menu(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(corner) = fixture.part(id, CORNER) else { return };
    let at = corner.center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.idle(2);
}

/// The values the Properties panel edits: the view (zoom in percent, rotation in degrees, flip, pixel aspect) and
/// what the rulers let the pointer do.
#[derive(Clone, Copy)]
struct Settings {
    zoom: f64,
    rotation: f64,
    flip: bool,
    aspect: f64,
    interactive: bool,
    locked: bool,
}

fn settings(data: &mut StoryData) -> Settings {
    Settings {
        zoom: *data.ranged("zoom", 100.0, 10.0..=800.0),
        rotation: *data.ranged("rotation", 0.0, -180.0..=180.0),
        flip: *data.value("flip", false),
        aspect: *data.ranged("aspect", 1.0, 0.5..=2.0),
        interactive: *data.value("interactive", true),
        locked: *data.value("locked", false),
    }
}

/// `rulers` with the pointer settings applied.
fn configured<'a>(rulers: Rulers<'a>, settings: Settings) -> Rulers<'a> {
    rulers.interactive(settings.interactive).origin_locked(settings.locked)
}

/// A canvas that shows `rulers` and whose view lies inside the strips, counting from their zero in `text`.
fn rulers_canvas<'a>(room: Rect, pixels_per_point: f32, settings: Settings, text: RulerText, rulers: Rulers<'a>) -> Canvas<'a> {
    let view =
        StoryDocument::STANDARD.view(room, pixels_per_point, settings.zoom).rotation(settings.rotation).flip_x(settings.flip).aspect(settings.aspect);
    Canvas::new("Document").view(view).with_rulers(Some(configured(rulers, settings))).rulers(text, text)
}

#[story(Rulers<'static>, covers = [Default], drive = Drive::Custom(drag_corner))]
fn zero_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let (zero, zero_at) = data.value_at("zero", Point::ZERO);
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), settings, pixels(), Rulers::new("Rulers", zero_at.bind(&mut *zero)));
    on_ruled_canvas(ui, room, canvas, Some(&StoryDocument::STANDARD))
}

#[story(Rulers<'static>, covers = [Open], drive = Drive::Custom(open_units_menu))]
fn units_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let (zero, zero_at) = data.value_at("zero", Point::ZERO);
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), settings, pixels(), Rulers::new("Rulers", zero_at.bind(&mut *zero)).units(&UNITS));
    on_ruled_canvas(ui, room, canvas, Some(&StoryDocument::STANDARD))
}

#[story(Rulers<'static>, covers = [Mixed], drive = Drive::Custom(drag_corner))]
fn zero_points_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let (zero, zero_at) = data.value_at("zero", Mixed::<Point>::Mixed);
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), settings, pixels(), Rulers::new("Rulers", zero_at.bind_mixed(&mut *zero)));
    on_ruled_canvas(ui, room, canvas, Some(&StoryDocument::STANDARD))
}

#[story(Rulers<'static>, covers = [Invalid], drive = Drive::Custom(drag_corner))]
fn zero_point_left_of_the_document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let (zero, zero_at) = data.value_at("zero", Point::new(LEAST_X - 1.0, 0.0));
    let room = stage(ui);
    let inside =
        |zero: &Point| if zero.x < LEAST_X { Validity::Refuse(Message::new("The zero point is left of the document")) } else { Validity::Ok };
    let canvas = rulers_canvas(room, ui.pixels_per_point(), settings, pixels(), Rulers::new("Rulers", zero_at.bind(&mut *zero).check(inside)));
    on_ruled_canvas(ui, room, canvas, Some(&StoryDocument::STANDARD))
}

/// The rulers in `face` with the document's margins shaded.
fn in_face(ui: &mut Ui, data: &mut StoryData, face: RulerFace) -> StoryOutput {
    let settings = settings(data);
    let (zero, zero_at) = data.value_at("zero", Point::ZERO);
    let room = stage(ui);
    let size = StoryDocument::STANDARD.size();
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    let margins = RulerMargins::new([0.0..MARGIN, width - MARGIN..width], [0.0..MARGIN, height - MARGIN..height]);
    let rulers = Rulers::new("Rulers", zero_at.bind(&mut *zero)).margins(margins).face(face);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), settings, pixels(), rulers);
    on_ruled_canvas(ui, room, canvas, Some(&StoryDocument::STANDARD))
}

/// The edge face: ticks rise from each strip's inner edge, each label beside its major tick, the margins in the well
/// colour.
#[story(Rulers<'static>, covers = [], drive = Drive::Never)]
fn edge_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    in_face(ui, data, RulerFace::Edge)
}

/// The centred face (the productivity set's): a bar inset in each strip, ticks centred on it, labels in place of the major
/// ticks and none over the shaded margins.
#[story(Rulers<'static>, covers = [], drive = Drive::Never)]
fn centred_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    in_face(ui, data, RulerFace::Centred)
}

/// Signed labels (VectorCraft's) on an inch ruler counting from the document's centre: ticks left of and above the zero
/// point carry a minus sign, and a tick writes only the decimals it needs (`-0.5`, `1`).
#[story(Rulers<'static>, covers = [], drive = Drive::Never)]
fn signed_labels(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let size = StoryDocument::STANDARD.size();
    let centre = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let (zero, zero_at) = data.value_at("zero", centre);
    let room = stage(ui);
    let inches = RulerText::new(RulerUnits::new(1.0 / 72.0, Ticks::decimal(false)).with_labels(TickLabels::Signed), UnitKind::Inches);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), settings, inches, Rulers::new("Rulers", zero_at.bind(&mut *zero)));
    on_ruled_canvas(ui, room, canvas, Some(&StoryDocument::STANDARD))
}
