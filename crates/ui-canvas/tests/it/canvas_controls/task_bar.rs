//! The contextual task bar (N6): hung centred under its anchor's screen bounds, following the anchor until the grip
//! moves it, reset and pinned from its More menu, and kept inside the canvas area whatever place it is given.

use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_ui::buttons::Button;
use craft_ui::canvas_controls::{TaskBar, TaskBarPlace};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::canvas_story::on_canvas;
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
