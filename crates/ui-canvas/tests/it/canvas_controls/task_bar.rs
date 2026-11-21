//! The contextual task bar (N6): hung centred under its anchor's screen bounds, following the anchor until the grip
//! moves it, reset and pinned from its More menu, and kept inside the canvas area whatever place it is given.

use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_ui::buttons::Button;
use craft_ui::canvas_controls::{TaskBar, TaskBarDismiss, TaskBarPlace};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::canvas_story::on_canvas;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const SLACK: f32 = 1.5;

/// What the trial's canvas shows and what the bar reported in the last pass.
#[derive(Clone)]
struct Scene {
    /// The anchor, in document points: a crop frame's corners.
    anchor: Vec<Point>,
    place: TaskBarPlace,
    /// The anchor's bounds on screen.
    bounds: Rect,
    /// The canvas rect the bar is kept in.
    area: Rect,
    /// The bar's rect.
    rect: Rect,
}

/// A frame of the story document, moved by `by` document points. It lies high on the story canvas (360 points tall),
/// so the bar hangs below it after every move and drop the trial makes.
fn frame(by: DocVec) -> Vec<Point> {
    [(120.0, 40.0), (520.0, 40.0), (520.0, 200.0), (120.0, 200.0)].into_iter().map(|(x, y)| Point::new(x, y) + by).collect()
}

/// A canvas that shows the bar for the scene's anchor and place, with the More menu before a Done button.
fn fixture(scene: &Rc<RefCell<Scene>>) -> Fixture {
    let shared = Rc::clone(scene);
    let mut fixture = Fixture::new(move |ui| {
        let _ = on_canvas(ui, |ui, area, view| {
            let mut held = shared.borrow_mut();
            let scene = &mut *held;
            scene.bounds = Rect::from_points(&scene.anchor.iter().map(|point| view.to_screen(*point)).collect::<Vec<_>>());
            scene.area = area;
            let shown = TaskBar::new("Crop", &scene.anchor, Bind::bound(&mut scene.place)).show(ui, area, |ui, cx| {
                cx.more_menu(ui);
                let _ = Button::new("Done").show(ui);
            });
            scene.rect = shown.output.rect;
            shown.output
        });
    });
    fixture.room(Vec2::new(1120.0, 720.0));
    fixture
}

fn seen(scene: &Rc<RefCell<Scene>>) -> Scene {
    scene.borrow().clone()
}

/// What the selection bar trials' canvas shows and what the bar reported.
#[derive(Clone)]
struct Pick {
    /// The anchor in document points; empty once the bar closed, so the caller stops showing it.
    anchor: Vec<Point>,
    /// The anchor sits at the top edge of the canvas area instead of in its middle.
    at_top: bool,
    dismiss: TaskBarDismiss,
    bounds: Rect,
    area: Rect,
    rect: Rect,
    /// The bar reported a close in some pass.
    closed: bool,
}

/// A canvas that shows a selection bar over the pick's anchor, with one button.
fn pick_fixture(pick: &Rc<RefCell<Pick>>) -> Fixture {
    let shared = Rc::clone(pick);
    let mut fixture = Fixture::new(move |ui| {
        let _ = on_canvas(ui, |ui, area, view| {
            let mut held = shared.borrow_mut();
            let pick = &mut *held;
            if pick.at_top && !pick.closed {
                let origin = view.to_screen(Point::ZERO);
                let scale = f64::from(view.to_screen(Point::new(1.0, 0.0)).x - origin.x);
                let y = f64::from(area.min.y + 2.0 - origin.y) / scale;
                pick.anchor = [(120.0, y), (520.0, y), (520.0, y + 30.0), (120.0, y + 30.0)].map(|(x, y)| Point::new(x, y)).to_vec();
            }
            pick.bounds = Rect::from_points(&pick.anchor.iter().map(|point| view.to_screen(*point)).collect::<Vec<_>>());
            pick.area = area;
            let shown = TaskBar::selection("Selection", &pick.anchor).dismiss(pick.dismiss).show(ui, area, |ui, _| {
                let _ = Button::new("Bold").show(ui);
            });
            pick.rect = shown.output.rect;
            if shown.output.closed {
                pick.closed = true;
                pick.anchor.clear();
            }
            shown.output
        });
    });
    fixture.room(Vec2::new(1120.0, 720.0));
    fixture
}

fn picked(pick: &Rc<RefCell<Pick>>) -> Pick {
    pick.borrow().clone()
}

/// A pick over a frame lowered to leave room above it for the bar, so the bar hangs above until `at_top` moves it.
fn new_pick(dismiss: TaskBarDismiss, at_top: bool) -> Rc<RefCell<Pick>> {
    Rc::new(RefCell::new(Pick {
        anchor: frame(DocVec::new(0.0, 80.0)),
        at_top,
        dismiss,
        bounds: Rect::NOTHING,
        area: Rect::NOTHING,
        rect: Rect::NOTHING,
        closed: false,
    }))
}

fn centred_under_anchor(scene: &Scene) -> bool {
    (scene.rect.center().x - scene.bounds.center().x).abs() < SLACK && scene.rect.top() > scene.bounds.bottom()
}

fn inside_area(scene: &Scene) -> bool {
    scene.area.expand(SLACK).contains_rect(scene.rect)
}

/// Opens the More menu and runs its row `row`.
fn pick(fixture: &mut Fixture, row: &str) {
    let more = fixture.by_label("More options").expect("the bar has its More button");
    fixture.click(&more).settle();
    let row = fixture.by_label(row).expect("the More menu lists the row");
    fixture.click(&row).settle();
}

#[trial]
fn a_selection_bar_hangs_above_its_anchor_flips_below_without_room_and_cannot_be_moved() {
    let pick = new_pick(TaskBarDismiss::Close, false);
    let mut fixture = pick_fixture(&pick);
    fixture.settle();
    let above = picked(&pick);
    assert!(above.rect.is_positive(), "the bar shows: {:?}", above.rect);
    assert!(above.rect.bottom() <= above.bounds.top() + SLACK, "above the anchor {:?}: {:?}", above.bounds, above.rect);
    assert!((above.rect.center().x - above.bounds.center().x).abs() < SLACK, "centred on the anchor");

    // A press and drag on the bar's own edge, away from its button, moves nothing: it has no move band.
    fixture.drag_at(above.rect.min + Vec2::new(1.0, 1.0), Vec2::new(-100.0, 60.0)).settle();
    let dragged = picked(&pick);
    assert!((dragged.rect.min - above.rect.min).length() < SLACK, "a drag on the bar leaves it: {:?} then {:?}", above.rect, dragged.rect);

    pick.borrow_mut().at_top = true;
    fixture.settle();
    let flipped = picked(&pick);
    assert!(flipped.rect.top() >= flipped.bounds.bottom() - SLACK, "below the anchor {:?} with no room above: {:?}", flipped.bounds, flipped.rect);
    assert!(flipped.area.expand(SLACK).contains_rect(flipped.rect), "inside the canvas {:?}: {:?}", flipped.area, flipped.rect);
}

#[trial]
fn an_outside_press_closes_a_selection_bar_and_reports_it_unless_it_keeps() {
    for dismiss in [TaskBarDismiss::Close, TaskBarDismiss::Keep] {
        let pick = new_pick(dismiss, false);
        let mut fixture = pick_fixture(&pick);
        fixture.settle();
        assert!(fixture.by_label("Bold").is_some(), "{dismiss:?}: the bar shows");
        let area = picked(&pick).area;
        fixture.click_at(egui::pos2(area.min.x + 10.0, area.max.y - 10.0)).settle();
        let after = picked(&pick);
        match dismiss {
            TaskBarDismiss::Keep => {
                assert!(!after.closed, "Keep reports no close");
                assert!(fixture.by_label("Bold").is_some(), "Keep stays after an outside press");
            }
            _ => {
                assert!(after.closed, "Close reports the close");
                assert!(fixture.by_label("Bold").is_none(), "Close stops showing the bar");
            }
        }
    }
}

#[trial]
fn the_task_bar_follows_its_anchor_until_moved_and_stays_inside_the_area() {
    let scene = Rc::new(RefCell::new(Scene {
        anchor: frame(DocVec::ZERO),
        place: TaskBarPlace::default(),
        bounds: Rect::NOTHING,
        area: Rect::NOTHING,
        rect: Rect::NOTHING,
    }));
    let mut fixture = fixture(&scene);
    fixture.settle();
    let first = seen(&scene);
    assert!(first.rect.is_positive(), "the bar shows: {:?}", first.rect);
    assert!(centred_under_anchor(&first), "centred below the anchor {:?}: {:?}", first.bounds, first.rect);
    assert!(inside_area(&first), "inside the canvas {:?}: {:?}", first.area, first.rect);

    scene.borrow_mut().anchor = frame(DocVec::new(40.0, 30.0));
    fixture.settle();
    let followed = seen(&scene);
    assert!(centred_under_anchor(&followed), "the bar follows the anchor {:?}: {:?}", followed.bounds, followed.rect);
    assert!(followed.rect.center().x > first.rect.center().x, "it moved with the anchor");

    // A press on the grip that does not drag leaves the bar on its anchor.
    let grip = fixture.by_label("Move bar").expect("the grip is named");
    fixture.click(&grip).settle();
    assert_eq!(seen(&scene).place, TaskBarPlace::default(), "a click moves nothing");

    let by = Vec2::new(-120.0, 60.0);
    fixture.drag(&grip, by).settle();
    let dropped = seen(&scene);
    assert!(dropped.place.at.is_some(), "the drag gave the bar a place");
    assert!((dropped.rect.min - (followed.rect.min + by)).length() < 2.0 * SLACK, "the bar stays where it was dropped: {:?}", dropped.rect);

    scene.borrow_mut().anchor = frame(DocVec::new(90.0, 10.0));
    fixture.settle();
    let kept = seen(&scene);
    assert!((kept.rect.min - dropped.rect.min).length() < SLACK, "a moved bar keeps its left edge while the anchor moves: {:?}", kept.rect);

    pick(&mut fixture, "Reset bar position");
    let reset = seen(&scene);
    assert_eq!(reset.place, TaskBarPlace::default(), "Reset clears the place");
    assert!(centred_under_anchor(&reset), "the bar follows its anchor again {:?}: {:?}", reset.bounds, reset.rect);

    pick(&mut fixture, "Pin bar position");
    let pinned = seen(&scene);
    assert!(pinned.place.pinned && pinned.place.at.is_some(), "Pin keeps the place: {:?}", pinned.place);
    scene.borrow_mut().anchor = frame(DocVec::new(-60.0, -40.0));
    fixture.settle();
    assert!((seen(&scene).rect.min - pinned.rect.min).length() < SLACK, "a pinned bar stays while the anchor moves");

    scene.borrow_mut().place = TaskBarPlace::new(false, Some([f32::NAN, 0.0]));
    fixture.settle();
    let unset = seen(&scene);
    assert!(centred_under_anchor(&unset), "a non-finite place counts as unset: {:?}", unset.rect);

    scene.borrow_mut().place = TaskBarPlace::new(false, Some([1.0e30, -1.0e30]));
    fixture.settle();
    let far = seen(&scene);
    assert!(inside_area(&far), "a place far outside comes back inside {:?}: {:?}", far.area, far.rect);
}

/// The grip is one of the bar's members: it sits on the members' line, centred with them, inside the bar's padding,
/// and a drag on it still moves the bar.
#[trial]
fn the_grip_sits_on_the_members_line() {
    let scene = Rc::new(RefCell::new(Scene {
        anchor: frame(DocVec::ZERO),
        place: TaskBarPlace::default(),
        bounds: Rect::NOTHING,
        area: Rect::NOTHING,
        rect: Rect::NOTHING,
    }));
    let mut fixture = fixture(&scene);
    fixture.settle();
    let bar = seen(&scene).rect;
    let grip = fixture.by_label("Move bar").expect("the grip is named");
    let done = fixture.by_label("Done").expect("the bar shows its member");
    assert!(
        (grip.center().y - done.center().y).abs() < SLACK,
        "the grip is centred on the members' line: grip {:?}, member {:?}",
        grip.rect,
        done.rect
    );
    assert!(grip.rect.top() > bar.top() + SLACK, "the grip keeps the bar's padding above it: grip {:?}, bar {bar:?}", grip.rect);
    assert!(grip.rect.left() > bar.left() + SLACK, "the grip keeps the bar's padding before it: grip {:?}, bar {bar:?}", grip.rect);

    let by = Vec2::new(-80.0, 40.0);
    fixture.drag(&grip, by).settle();
    let dropped = seen(&scene);
    assert!((dropped.rect.min - (bar.min + by)).length() < 2.0 * SLACK, "a drag on the grip moves the bar: {:?}", dropped.rect);
}

/// A drag of the grip moves the bar under the pointer in the frame of each step, in one pass, and writes its place
/// once, on the drop: one edit for the whole move.
#[trial]
fn a_dragged_task_bar_stands_under_the_pointer_in_each_frame_and_writes_its_place_on_the_drop() {
    let scene = Rc::new(RefCell::new(Scene {
        anchor: frame(DocVec::ZERO),
        place: TaskBarPlace::default(),
        bounds: Rect::NOTHING,
        area: Rect::NOTHING,
        rect: Rect::NOTHING,
    }));
    let mut fixture = fixture(&scene);
    fixture.settle();
    let start = seen(&scene).rect;
    let from = fixture.by_label("Move bar").expect("the grip is named").center();
    fixture.frame(vec![moved(from)]);
    fixture.frame(vec![press(from)]);
    let step = Vec2::new(-30.0, 15.0);
    for n in 1..=4 {
        let to = start.min + step * n as f32;
        fixture.frame(vec![moved(from + step * n as f32)]);
        let now = seen(&scene);
        assert!((now.rect.min - to).length() < SLACK, "step {n}: the bar stands under the pointer in its frame: {:?}, wanted {to:?}", now.rect);
        assert_eq!(fixture.passes(), 1, "step {n}: the frame runs one pass");
        assert_eq!(now.place, TaskBarPlace::default(), "step {n}: the place is not written while the drag runs");
    }
    let dropped_at = start.min + step * 4.0;
    fixture.frame(vec![release(from + step * 4.0)]);
    let dropped = seen(&scene);
    assert!((dropped.rect.min - dropped_at).length() < SLACK, "the bar stays where it was dropped: {:?}", dropped.rect);
    let at = dropped.place.at.map(|at| dropped.area.min + Vec2::from(at));
    assert!(at.is_some_and(|at| (at - dropped_at).length() < SLACK), "the drop writes the place: {:?}", dropped.place);
    fixture.settle();
    assert!((seen(&scene).rect.min - dropped_at).length() < SLACK, "the written place keeps the bar there: {:?}", seen(&scene).rect);
}
