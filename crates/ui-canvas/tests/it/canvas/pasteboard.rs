//! `Pasteboard`: the Menu key or Shift+F10 on the focused canvas asks for the pasteboard menu at the view's centre, as
//! a secondary click on the pasteboard does where it lands; the pasteboard is the canvas minus the pages and the
//! uncovered rects in any view; and a press on it keeps a typed zoom unapplied, as a press on the image does not.

use craft_behavior::kurbo::{Point, Rect, Size};
use craft_behavior::{CanvasView, EditEvent, PixelSize};
use craft_ui::buttons::Button;
use craft_ui::canvas::{AppPaint, Canvas, NavigatorZoomRow, PageFrame, Pasteboard, PasteboardColor, RasterDisplay};
use craft_ui::contract::{Bind, Rename, Show as _, ShowContainer as _};
use craft_ui::primitives::Checker;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, Key, Mesh, Modifiers, Pos2, Shape, TextureId, pos2, vec2};
use std::cell::{Cell, RefCell};
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

const LONG_NAME: &str = "A fairly long artboard name";

/// The painted name `LONG_NAME` of a page 400 document units wide at `zoom`, with the page's on-screen width.
fn painted_name_at(zoom: f64) -> (f32, Option<egui::Rect>, bool) {
    let pages = [PageFrame::new(Rect::new(0.0, 0.0, 400.0, 100.0)).name(LONG_NAME)];
    let width = Rc::new(Cell::new(0.0_f32));
    let seen = Rc::clone(&width);
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), Point::new(200.0, 50.0), zoom);
        seen.set(view.to_screen(Point::new(400.0, 0.0)).x - view.to_screen(Point::ZERO).x);
        let _ = Canvas::new("Document").view(view).show(ui, |ui, body| Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect));
    });
    fixture.room(vec2(900.0, 300.0)).settle();
    let name = fixture.texts().iter().find(|text| text.text == LONG_NAME);
    (width.get(), name.map(|text| text.rect), name.is_some_and(|text| text.elided))
}

#[trial]
fn a_long_page_name_is_cut_inside_its_pages_width_and_is_gone_below_the_shortest_cut() {
    let (wide, name, whole_elided) = painted_name_at(1.0);
    let name = name.expect("a wide page shows its name");
    assert!(!whole_elided, "a wide page shows its name whole");
    let (narrow, cut, cut_elided) = painted_name_at(0.25);
    let cut = cut.expect("a narrower page still shows a cut name");
    assert!(narrow < wide && cut_elided, "narrowing the page cuts the name");
    assert!(cut.width() <= narrow, "the cut name stays inside its page's width: {} in {narrow}", cut.width());
    assert!(cut.width() < name.width(), "the cut name is shorter than the whole one");
    let (_, gone, _) = painted_name_at(0.03);
    assert!(gone.is_none(), "below the shortest readable cut the name is left out");
}

/// The pages of the rename trial: the first renames, the second does not.
fn rename_pages() -> [PageFrame; 2] {
    [
        PageFrame::new(Rect::new(100.0, 80.0, 300.0, 220.0)).name("Artboard 1").renames(true),
        PageFrame::new(Rect::new(340.0, 80.0, 500.0, 220.0)).name("Artboard 2"),
    ]
}

/// Double presses the name `label`, then runs `then`; returns every rename event the pasteboard reported, in order, and
/// how many painted texts read `label` once the rename is open.
fn rename_events(label: &str, then: impl FnOnce(&mut Fixture)) -> (Vec<EditEvent<Rename<usize>>>, usize) {
    let pages = rename_pages();
    let log: Rc<RefCell<Vec<EditEvent<Rename<usize>>>>> = Rc::default();
    let seen = Rc::clone(&log);
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), Point::new(300.0, 150.0), 1.0);
        let shown = Canvas::new("Document").view(view).show(ui, |ui, body| Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect));
        if let Some(event) = shown.inner.and_then(|out| out.rename) {
            seen.borrow_mut().push(event);
        }
    });
    fixture.room(vec2(700.0, 400.0)).settle();
    let name = fixture.by_label(label).expect("the page's name");
    fixture.click(&name).click(&name).settle();
    let painted = fixture.texts().iter().filter(|text| text.text == label).count();
    then(&mut fixture);
    fixture.settle();
    let events = log.borrow().clone();
    (events, painted)
}

/// The rename a `Commit` carries.
fn commit_of(event: &EditEvent<Rename<usize>>) -> Option<&Rename<usize>> {
    match event {
        EditEvent::Commit { after, .. } => Some(after),
        _ => None,
    }
}

/// Types `text` over the open rename field.
fn type_over(fixture: &mut Fixture, text: &str) {
    fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
}

#[trial]
fn a_double_press_on_a_renaming_name_begins_a_rename_that_commits_once_or_cancels_and_a_locked_frame_does_not_begin() {
    let begins = |events: &[EditEvent<Rename<usize>>]| events.iter().filter(|event| matches!(event, EditEvent::Begin { .. })).count();
    let (committed, painted) = rename_events("Artboard 1", |fixture| {
        type_over(fixture, "Renamed");
        fixture.key(Key::Enter, Modifiers::NONE);
    });
    assert_eq!(begins(&committed), 1, "a double press begins one rename: {committed:?}");
    assert!(painted <= 1, "the name is not painted beside the open field: {painted} runs read it");
    let commits: Vec<_> = committed.iter().filter_map(commit_of).collect();
    assert_eq!(commits.len(), 1, "Enter commits once: {committed:?}");
    assert_eq!((commits[0].key, commits[0].name.as_str()), (0, "Renamed"), "the commit carries the page and the typed name");

    let (escaped, _) = rename_events("Artboard 1", |fixture| {
        type_over(fixture, "Renamed");
        fixture.key(Key::Escape, Modifiers::NONE);
    });
    assert!(escaped.iter().any(|event| matches!(event, EditEvent::Cancel { .. })), "Escape cancels: {escaped:?}");
    assert!(escaped.iter().all(|event| commit_of(event).is_none()), "Escape commits nothing: {escaped:?}");

    let (unchanged, _) = rename_events("Artboard 1", |fixture| {
        fixture.key(Key::Enter, Modifiers::NONE);
    });
    assert!(unchanged.iter().any(|event| matches!(event, EditEvent::Cancel { .. })), "an unchanged name cancels: {unchanged:?}");
    assert!(unchanged.iter().all(|event| commit_of(event).is_none()), "an unchanged name commits nothing: {unchanged:?}");

    let (locked, _) = rename_events("Artboard 2", |_| {});
    assert!(locked.is_empty(), "a frame that does not rename begins nothing: {locked:?}");
}

/// The colour the pasteboard surround takes in the order trial.
const SURROUND: Color32 = Color32::from_gray(40);
/// The picture's own colour in the order trial.
const PICTURE: Color32 = Color32::from_rgb(204, 128, 77);

/// The plain fill of a shape, if it has one.
fn fill_of(shape: &Shape) -> Option<Color32> {
    match shape {
        Shape::Rect(rect) => Some(rect.fill),
        Shape::Path(path) if path.closed => Some(path.fill),
        Shape::Mesh(mesh) => mesh.vertices.first().map(|vertex| vertex.color),
        _ => None,
    }
}

/// The view and the shapes of a frame of a canvas on a dark pasteboard that shows `pages` and then a picture over the
/// whole canvas.
fn pasteboard_shapes(pages: Vec<PageFrame>, shows_outside: bool) -> (CanvasView, Vec<Shape>) {
    let room = vec2(400.0, 300.0);
    let view = CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, room), Point::new(200.0, 150.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).pasteboard(PasteboardColor::DarkGray).show(ui, |ui, body| {
            let pasteboard = Pasteboard::new("Pasteboard", &pages);
            let _ = if shows_outside { pasteboard.shows_outside() } else { pasteboard }.show_at(ui, body.rect);
            let paint = AppPaint::new(|painter, view: CanvasView| {
                painter.painter.add(Shape::rect_filled(view.rect, 0.0, PICTURE));
            });
            RasterDisplay::painted("Picture", PixelSize::new(400, 300), paint).picture().show_at(ui, body.rect);
        });
    });
    fixture.room(room).settle();
    (view, fixture.frame_shapes())
}

#[trial]
fn with_shows_outside_a_picture_shows_between_artboards_and_a_checker_page_paints_its_grid_only_on_the_page() {
    let boards = || {
        vec![
            PageFrame::new(Rect::new(20.0, 20.0, 180.0, 280.0)).name("One").active(true),
            PageFrame::new(Rect::new(220.0, 20.0, 380.0, 280.0)).name("Two").active(false),
        ]
    };
    for shows_outside in [false, true] {
        let (_, shapes) = pasteboard_shapes(boards(), shows_outside);
        let picture = shapes.iter().position(|shape| fill_of(shape) == Some(PICTURE)).expect("the picture paints");
        let surround: Vec<usize> = shapes.iter().enumerate().filter(|(_, shape)| fill_of(shape) == Some(SURROUND)).map(|(at, _)| at).collect();
        assert!(!surround.is_empty(), "shows_outside {shows_outside}: the surround paints");
        assert_eq!(
            surround.iter().all(|&at| at < picture),
            shows_outside,
            "shows_outside {shows_outside}: the surround lies under the picture only when the document shows outside the boards"
        );
    }
    let page = Rect::new(20.0, 20.0, 180.0, 280.0);
    let (view, shapes) =
        pasteboard_shapes(vec![PageFrame::new(page).checker(Checker::default()), PageFrame::new(Rect::new(220.0, 20.0, 380.0, 280.0))], false);
    let quad = egui::Rect::from_points(&view.quad(page)).expand(0.5);
    let grids: Vec<&Mesh> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => Some(&**mesh),
            _ => None,
        })
        .collect();
    assert!(!grids.is_empty(), "the checker page paints a grid");
    assert!(grids.iter().all(|mesh| mesh.vertices.iter().all(|vertex| quad.contains(vertex.pos))), "the grid lies inside the page's quad only");
}
