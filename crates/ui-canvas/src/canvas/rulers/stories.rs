//! Stories of the rulers.

use super::{CORNER, Rulers};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_with};
use crate::canvas::{Canvas, RulerText};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use crate::testing::{pointer, stage};
use craft_behavior::kurbo::Point;
use craft_behavior::{Mixed, RulerUnits};
use egui::{Rect, Ui, Vec2};

/// How far the corner is dragged, in drag thresholds: out of the corner and onto the document.
const DRAG_THRESHOLDS: f32 = 12.0;

/// The least x the invalid story accepts for the zero point.
const LEAST_X: f64 = 0.0;

/// The units the menu story offers.
const UNITS: [UnitKind; 2] = [UnitKind::Pixels, UnitKind::Points];

fn pixels() -> RulerText {
    RulerText::new(RulerUnits::pixels(), UnitKind::Pixels)
}

fn drag_corner(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(corner) = fixture.part(id, CORNER) else { return };
    let travel = DRAG_THRESHOLDS * crate::testing::input::max_click_dist(fixture.ctx());
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

/// A canvas whose view lies inside the strips and whose rulers count from `zero` in pixels.
fn rulers_canvas(room: Rect, pixels_per_point: f32, zero: Point, settings: Settings) -> Canvas<'static> {
    let text = pixels();
    let view =
        StoryDocument::STANDARD.view(room, pixels_per_point, settings.zoom).rotation(settings.rotation).flip_x(settings.flip).aspect(settings.aspect);
    Canvas::new("Document").view(view).show_rulers(true).rulers(text, text, zero)
}

#[story(Rulers<'static>, covers = [Default], drive = Drive::Custom(drag_corner))]
fn zero_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let zero = data.value("zero", Point::ZERO);
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), *zero, settings);
    on_canvas_with(ui, room, canvas, Some(&StoryDocument::STANDARD), |ui, at| {
        configured(Rulers::new("Rulers", Bind::bound(&mut *zero)), settings).show_at(ui, at)
    })
}

#[story(Rulers<'static>, covers = [Open], drive = Drive::Custom(open_units_menu))]
fn units_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let zero = data.value("zero", Point::ZERO);
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), *zero, settings);
    on_canvas_with(ui, room, canvas, Some(&StoryDocument::STANDARD), |ui, at| {
        configured(Rulers::new("Rulers", Bind::bound(&mut *zero)), settings).units(&UNITS).show_at(ui, at)
    })
}

#[story(Rulers<'static>, covers = [Mixed], drive = Drive::Custom(drag_corner))]
fn zero_points_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let zero = data.value("zero", Mixed::<Point>::Mixed);
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), Point::ZERO, settings);
    on_canvas_with(ui, room, canvas, Some(&StoryDocument::STANDARD), |ui, at| {
        configured(Rulers::new("Rulers", Bind::bound_mixed(&mut *zero)), settings).show_at(ui, at)
    })
}

#[story(Rulers<'static>, covers = [Invalid], drive = Drive::Custom(drag_corner))]
fn zero_point_left_of_the_document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = settings(data);
    let zero = data.value("zero", Point::new(LEAST_X - 1.0, 0.0));
    let room = stage(ui);
    let canvas = rulers_canvas(room, ui.pixels_per_point(), *zero, settings);
    on_canvas_with(ui, room, canvas, Some(&StoryDocument::STANDARD), |ui, at| {
        let inside =
            |zero: &Point| if zero.x < LEAST_X { Validity::Refuse(Message::new("The zero point is left of the document")) } else { Validity::Ok };
        configured(Rulers::new("Rulers", Bind::bound(&mut *zero).check(inside)), settings).show_at(ui, at)
    })
}
