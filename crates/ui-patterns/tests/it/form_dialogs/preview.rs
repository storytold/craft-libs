//! The preview's zoom row and the picture point a click reports.

use craft_behavior::CanvasTransform;
use craft_ui::contract::Show as _;
use craft_ui::patterns::form_dialogs::{PreviewPane, PreviewView, PreviewZoom};
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Key, Modifiers, Pos2, TextureId, Vec2, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Zooms closer than this are the same zoom.
const SAME: f32 = 1e-4;

fn type_zoom(fixture: &mut Fixture, text: &str) {
    let field = fixture.by_label("Zoom").expect("the row has a zoom field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE).settle();
}

// A typed zoom sets the view's zoom, inside the view's zoom range.
#[trial]
fn a_typed_zoom_sets_the_preview_zoom_inside_its_range() {
    let view = Rc::new(RefCell::new(PreviewView::new(CanvasTransform::new(1.0, Vec2::ZERO))));
    let shown = view.clone();
    let mut fixture = Fixture::new(move |ui| {
        let current = shown.borrow().clone();
        if let Some(next) = PreviewZoom::new(&current).show(ui).view {
            *shown.borrow_mut() = next;
        }
    });
    fixture.settle();

    type_zoom(&mut fixture, "200");
    assert!((view.borrow().transform.scale - 2.0).abs() < SAME, "{:?}", view.borrow().transform);
    assert!(!view.borrow().fit);

    type_zoom(&mut fixture, "5000");
    let end = *view.borrow().range.end();
    assert!((view.borrow().transform.scale - end).abs() < SAME, "a zoom past the range stops at its end: {:?}", view.borrow().transform);
}

/// The picture the click trials show.
const PICTURE: Vec2 = vec2(64.0, 48.0);

/// What a pane at `scale`, centring on a click or not, reports for a click at `from_centre` points from the well's
/// centre: the picture point and whether the view moved.
fn click(scale: f32, centres: bool, from_centre: Vec2) -> (Option<Pos2>, bool) {
    let seen = Rc::new(Cell::new((None, false)));
    let out = seen.clone();
    let view = PreviewView::new(CanvasTransform::new(scale, Vec2::ZERO));
    let mut fixture = Fixture::new(move |ui| {
        let shown = PreviewPane::new("Preview", SizedTexture::new(TextureId::default(), PICTURE), &view).click_centres(centres).show(ui);
        if shown.clicked.is_some() || shown.view.is_some() {
            out.set((shown.clicked, shown.view.is_some()));
        }
    });
    fixture.settle();
    let well = fixture.by_label("Preview").expect("the pane").rect;
    fixture.click_at(well.center() + from_centre).settle();
    seen.get()
}

// A click reports the picture point under it, centring the view or not; a click beside the picture reports none.
#[trial]
fn a_click_reports_the_picture_point_under_it() {
    let near = |a: Option<Pos2>, b: Pos2| a.is_some_and(|a| (a - b).length() < 0.5);
    // At half size the picture sits in the middle of the well: a point 8 to the right is 16 picture pixels.
    let (point, _) = click(0.5, true, vec2(8.0, -4.0));
    assert!(near(point, pos2(PICTURE.x / 2.0 + 16.0, PICTURE.y / 2.0 - 8.0)), "{point:?}");
    // The half-size picture's edge is 16 points right of the centre.
    let (beside, _) = click(0.5, true, vec2(PICTURE.x / 4.0 + 4.0, 0.0));
    assert_eq!(beside, None, "a click in the well beside the picture");

    // Zoomed in, the picture overflows the well: a click centres the view unless the pane is told not to.
    let (centred, moved) = click(8.0, true, vec2(16.0, 8.0));
    assert!(near(centred, pos2(PICTURE.x / 2.0 + 2.0, PICTURE.y / 2.0 + 1.0)) && moved, "{centred:?} {moved}");
    let (kept, moved) = click(8.0, false, vec2(16.0, 8.0));
    assert!(near(kept, pos2(PICTURE.x / 2.0 + 2.0, PICTURE.y / 2.0 + 1.0)) && !moved, "{kept:?} {moved}");
}
