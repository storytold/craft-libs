//! `Pasteboard`: the Menu key or Shift+F10 on the focused canvas asks for the pasteboard menu at the view's centre, as
//! a secondary click on the pasteboard does where it lands; the pasteboard is the canvas minus the pages and the
//! uncovered rects in any view; and a press on it keeps a typed zoom unapplied, as a press on the image does not.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_ui::buttons::Button;
use craft_ui::canvas::{Canvas, NavigatorZoomRow, PageFrame, Pasteboard};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;

#[trial]
fn the_menu_key_on_the_focused_canvas_asks_for_the_pasteboard_menu_at_the_views_centre() {
    let centre = Point::new(200.0, 150.0);
    let asked: Rc<Cell<Option<Point>>> = Rc::new(Cell::new(None));
    let seen = Rc::clone(&asked);
    let pages = [PageFrame::new(Rect::new(100.0, 80.0, 300.0, 220.0)).name("Artboard 1")];
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), centre, 1.0);
        let shown = Canvas::new("Document").view(view).show(ui, |ui, body| Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect));
        if let Some(at) = shown.inner.and_then(|out| out.context_at) {
            seen.set(Some(at));
        }
    });
    fixture.settle().key(Key::Tab, Modifiers::NONE).settle();
    fixture.key(Key::F10, Modifiers::SHIFT).settle();
    // The centre lies on the page: a keyed request has no point of its own to test against the pages.
    assert_eq!(asked.get(), Some(centre));
}

/// Whether the convex quad `quad` holds `p`: on the inner side of every edge by more than `margin` (a negative margin
/// takes in points just outside).
fn holds(quad: &[Pos2; 4], p: Pos2, margin: f32) -> bool {
    let sides: Vec<f32> = (0..4)
        .map(|i| {
            let (a, b) = (quad[i], quad[(i + 1) % 4]);
            let edge = b - a;
            (edge.x * (p.y - a.y) - edge.y * (p.x - a.x)) / edge.length().max(f32::EPSILON)
        })
        .collect();
    sides.iter().all(|side| *side > margin) || sides.iter().all(|side| *side < -margin)
}

#[trial]
fn uncovered_rects_stay_open_in_a_turned_view() {
    let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let pages = [PageFrame::new(Rect::new(120.0, 90.0, 260.0, 200.0))];
    // One loose layer overlaps the page; the other runs past the visible area.
    let uncovered = [Rect::new(230.0, 150.0, 320.0, 240.0), Rect::new(340.0, 20.0, 520.0, 110.0)];
    let holes: Vec<Rect> = pages.iter().map(|page| page.rect).chain(uncovered).collect();
    // Doc points this close to a hole's edge are skipped: either answer is right on the line.
    let near = 0.5;
    for rotation in [0.0, 30.0] {
        let view = CanvasView::new(screen, Point::new(200.0, 150.0), 1.0).rotation(rotation);
        let quads = Pasteboard::quads(&view, &pages, &uncovered);
        assert!(!quads.is_empty(), "{rotation}°: the pasteboard shows");
        let (mut open, mut covered) = (0, 0);
        let samples = (0..55).flat_map(|i| (0..41).map(move |j| pos2(3.7 + 7.3 * i as f32, 2.9 + 7.3 * j as f32)));
        for p in samples.filter(|p| screen.contains(*p)) {
            let doc = view.to_doc(p);
            if holes.iter().any(|hole| hole.inflate(near, near).contains(doc) && !hole.inflate(-near, -near).contains(doc)) {
                continue;
            }
            if holes.iter().any(|hole| hole.contains(doc)) {
                open += 1;
                assert!(!quads.iter().any(|quad| holds(quad, p, 1e-3)), "{rotation}°: {p:?} (document {doc:?}) lies in a hole and in the pasteboard");
            } else {
                covered += 1;
                assert!(
                    quads.iter().any(|quad| holds(quad, p, -1e-3)),
                    "{rotation}°: {p:?} (document {doc:?}) lies outside every hole and every quad"
                );
            }
        }
        assert!(open > 0 && covered > 0, "{rotation}°: the samples reach both the holes and the pasteboard");
    }
}

const ZOOM_CENTRE: Point = Point::new(200.0, 150.0);
const ZOOM_PAGE: Rect = Rect::new(100.0, 80.0, 300.0, 220.0);
const ZOOM_DOCUMENT: Size = Size::new(2000.0, 1500.0);

/// Where the zoom trial presses after typing.
#[derive(Clone, Copy, Debug)]
enum PressOn {
    /// The pasteboard above the page.
    Pasteboard,
    /// The middle of the page, the image.
    Image,
    /// A button outside the canvas.
    Button,
}

/// Types a zoom into the Navigator's zoom field above a canvas turned by `rotation` degrees, then clicks `on`. Returns
/// whether the typed zoom applied (the row committed it; a plain typed number already shows as a proposal) and whether
/// focus stayed in the field.
fn press_after_typing(rotation: f64, on: PressOn) -> (bool, bool) {
    let zoom_view =
        Rc::new(Cell::new(CanvasView::new(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)), Point::new(1000.0, 750.0), 1.0)));
    let canvas_view: Rc<Cell<Option<CanvasView>>> = Rc::new(Cell::new(None));
    let committed = Rc::new(Cell::new(false));
    let (zoom_shown, canvas_shown, seen) = (Rc::clone(&zoom_view), Rc::clone(&canvas_view), Rc::clone(&committed));
    let pages = [PageFrame::new(ZOOM_PAGE)];
    let mut fixture = Fixture::new(move |ui| {
        let mut given = zoom_shown.get();
        let row = NavigatorZoomRow::new(Bind::bound(&mut given), ZOOM_DOCUMENT).show(ui);
        zoom_shown.set(given);
        seen.set(seen.get() || row.view.committed().is_some());
        let _ = Button::new("Elsewhere").show(ui);
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, ZOOM_CENTRE, 1.0).rotation(rotation);
        canvas_shown.set(Some(view));
        let _ = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect));
    });
    // Tall enough that the pasteboard 60 points above the page lies on the canvas below the zoom row and the button.
    fixture.room(vec2(400.0, 400.0)).settle();
    let field = fixture.by_label("Zoom").expect("the zoom field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("250").settle();
    let typing = fixture.focused_stop();
    assert!(typing.is_some(), "the zoom field holds focus while a zoom is typed");
    assert!(!committed.get(), "a typed zoom waits for its commit");
    let view = canvas_view.get().expect("the canvas showed");
    let at = match on {
        PressOn::Pasteboard => view.to_screen(Point::new(ZOOM_CENTRE.x, ZOOM_PAGE.y0 - 60.0)),
        PressOn::Image => view.to_screen(ZOOM_CENTRE),
        PressOn::Button => fixture.by_label("Elsewhere").expect("the button").center(),
    };
    assert!(matches!(on, PressOn::Button) || view.rect.contains(at), "{on:?}: the press lands on the canvas");
    fixture.click_at(at).settle();
    (committed.get(), fixture.focused_stop() == typing)
}

#[trial]
fn a_press_on_the_pasteboard_keeps_a_typed_zoom_and_one_on_the_image_applies_it() {
    for rotation in [0.0, 30.0] {
        let (applied, kept) = press_after_typing(rotation, PressOn::Pasteboard);
        assert!(!applied, "{rotation}°: a press on the pasteboard leaves the typed zoom unapplied");
        assert!(kept, "{rotation}°: a press on the pasteboard leaves focus in the zoom field");
    }
    assert!(press_after_typing(0.0, PressOn::Image).0, "a press on the image applies the typed zoom");
    assert!(press_after_typing(0.0, PressOn::Button).0, "a press on a button elsewhere applies the typed zoom");
}
