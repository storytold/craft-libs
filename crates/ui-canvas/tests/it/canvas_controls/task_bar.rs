//! The contextual task bar (N6): hung centred under its anchor's screen bounds, following the anchor with the offset
//! the grip gave it, reset and pinned from its More menu, and kept inside the canvas area whatever place it is given.
//! A fixed bar neither moves nor has a menu, and an edge bar sits at the canvas view's top edge.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::overlay::{Align, Side};
use craft_ui::buttons::Button;
use craft_ui::canvas_controls::{TaskBar, TaskBarDismiss, TaskBarGrip, TaskBarHang, TaskBarPlace};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::menu::PanelRow;
use craft_ui::runtime::commands::{CommandRef, EditCommands};
use craft_ui::testing::canvas_story::{StoryDocument, on_canvas, on_canvas_view};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::{Cell, RefCell};
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

/// A scene with the frame at its start and no place.
fn scene() -> Rc<RefCell<Scene>> {
    Rc::new(RefCell::new(Scene {
        anchor: frame(DocVec::ZERO),
        place: TaskBarPlace::default(),
        bounds: Rect::NOTHING,
        area: Rect::NOTHING,
        rect: Rect::NOTHING,
    }))
}

/// A frame of the story document, moved by `by` document points. It lies high on the story canvas (360 points tall),
/// so the bar hangs below it after every move and drop the trial makes.
fn frame(by: DocVec) -> Vec<Point> {
    [(120.0, 40.0), (520.0, 40.0), (520.0, 200.0), (120.0, 200.0)].into_iter().map(|(x, y)| Point::new(x, y) + by).collect()
}

/// A frame low on the story canvas: below it the canvas has no room for a bar.
fn low_frame() -> Vec<Point> {
    [(120.0, 290.0), (520.0, 290.0), (520.0, 390.0), (120.0, 390.0)].into_iter().map(|(x, y)| Point::new(x, y)).collect()
}

/// A frame far larger than the story canvas, as a crop frame on a document zoomed past the view: it leaves no room for
/// a bar above or below it.
fn filling_frame() -> Vec<Point> {
    [(-2000.0, -2000.0), (3000.0, -2000.0), (3000.0, 3000.0), (-2000.0, 3000.0)].into_iter().map(|(x, y)| Point::new(x, y)).collect()
}

/// A canvas that shows the bar for the scene's anchor and place, with the More menu before a Done button: a movable
/// bar, or a `fixed` one, which takes no place.
fn fixture(scene: &Rc<RefCell<Scene>>, fixed: bool) -> Fixture {
    fixture_inner(scene, fixed, |bar| bar)
}

/// A canvas that shows the bar for the scene's anchor and place, set up by `configure`, with the More menu before a
/// Done button.
fn fixture_with(scene: &Rc<RefCell<Scene>>, configure: impl for<'a> Fn(TaskBar<'a>) -> TaskBar<'a> + 'static) -> Fixture {
    fixture_inner(scene, false, configure)
}

fn fixture_inner(scene: &Rc<RefCell<Scene>>, fixed: bool, configure: impl for<'a> Fn(TaskBar<'a>) -> TaskBar<'a> + 'static) -> Fixture {
    let shared = Rc::clone(scene);
    let mut fixture = Fixture::new(move |ui| {
        let _ = on_canvas(ui, |ui, area, view| {
            let mut held = shared.borrow_mut();
            let scene = &mut *held;
            scene.bounds = Rect::from_points(&scene.anchor.iter().map(|point| view.to_screen(*point)).collect::<Vec<_>>());
            scene.area = area;
            let bar = if fixed {
                TaskBar::fixed("Crop", &scene.anchor)
            } else {
                configure(TaskBar::new("Crop", &scene.anchor, Bind::bound(&mut scene.place)))
            };
            let shown = bar.show(ui, area, |ui, cx| {
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
    let scene = scene();
    let mut fixture = fixture(&scene, false);
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
    assert!(dropped.place.offset.is_some() && dropped.place.at.is_none(), "the drag gave the bar an offset: {:?}", dropped.place);
    assert!((dropped.rect.min - (followed.rect.min + by)).length() < 2.0 * SLACK, "the bar sits where it was dropped: {:?}", dropped.rect);

    scene.borrow_mut().anchor = frame(DocVec::new(90.0, 10.0));
    fixture.settle();
    let kept = seen(&scene);
    let shift = kept.bounds.min - dropped.bounds.min;
    assert!(shift.length() > SLACK, "the anchor moved on screen");
    assert!(
        (kept.rect.min - dropped.rect.min - shift).length() < SLACK,
        "a moved bar follows its anchor with its offset: anchor shifted {shift:?}, bar {:?} to {:?}",
        dropped.rect,
        kept.rect
    );

    pick(&mut fixture, "Reset bar position");
    let reset = seen(&scene);
    assert_eq!(reset.place, TaskBarPlace::default(), "Reset clears the place");
    assert!(centred_under_anchor(&reset), "the bar follows its anchor again {:?}: {:?}", reset.bounds, reset.rect);

    pick(&mut fixture, "Pin bar position");
    let pinned = seen(&scene);
    assert!(pinned.place.pinned && pinned.place.at.is_some() && pinned.place.offset.is_none(), "Pin keeps the place: {:?}", pinned.place);
    scene.borrow_mut().anchor = frame(DocVec::new(-60.0, -40.0));
    fixture.settle();
    let held = seen(&scene);
    assert!((held.rect.min - pinned.rect.min).length() < SLACK, "a pinned bar stays while the anchor moves");

    pick(&mut fixture, "Pin bar position");
    let unpinned = seen(&scene);
    assert!(!unpinned.place.pinned && unpinned.place.at.is_none() && unpinned.place.offset.is_some(), "Pin again unpins: {:?}", unpinned.place);
    assert!((unpinned.rect.min - held.rect.min).length() < SLACK, "the bar jumped when it was unpinned: {:?}", unpinned.rect);
    scene.borrow_mut().anchor = frame(DocVec::new(-20.0, -40.0));
    fixture.settle();
    let after = seen(&scene);
    let shift = after.bounds.min - unpinned.bounds.min;
    assert!((after.rect.min - unpinned.rect.min - shift).length() < SLACK, "an unpinned bar follows its anchor again");

    scene.borrow_mut().place = TaskBarPlace::new(false, Some([f32::NAN, 0.0]));
    fixture.settle();
    let unset = seen(&scene);
    assert!(centred_under_anchor(&unset), "a non-finite place counts as unset: {:?}", unset.rect);

    scene.borrow_mut().place = TaskBarPlace::new(false, Some([1.0e30, -1.0e30]));
    fixture.settle();
    let far = seen(&scene);
    assert!(inside_area(&far), "a place far outside comes back inside {:?}: {:?}", far.area, far.rect);
}

/// The More menu lists Hide, Pin and Reset, then the host's rows in the order given; a submenu opens.
#[trial]
fn the_more_menu_lists_the_hosts_rows_after_its_own_and_runs_them() {
    let scene = scene();
    let mut fixture = fixture_with(&scene, |bar| {
        bar.more_rows(vec![
            PanelRow::Command(CommandRef::from(EditCommands::Undo)),
            PanelRow::Submenu("Arrange".into(), vec![PanelRow::Command(CommandRef::from(EditCommands::Cut))]),
        ])
    });
    fixture.settle();
    let more = fixture.by_label("More options").expect("the bar has its More button");
    fixture.click(&more).settle();
    let tops: Vec<f32> = ["Hide bar", "Pin bar position", "Reset bar position", "Undo", "Arrange"]
        .into_iter()
        .map(|label| fixture.by_label(label).unwrap_or_else(|| panic!("the More menu lists {label}")).rect.top())
        .collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "the rows are not in order, the bar's own first: {tops:?}");
    assert!(fixture.by_label("Cut").is_none(), "the submenu is closed");
    let arrange = fixture.by_label("Arrange").expect("the submenu row");
    fixture.click(&arrange).settle();
    assert!(fixture.by_label("Cut").is_some(), "the submenu opens");
}

/// Beside the anchor near the canvas's right edge the bar flips to the anchor's other side, and a moved bar whose
/// offset place would cover the kept-clear point shows at its unmoved place with its offset kept.
#[trial]
fn a_bar_beside_a_point_flips_and_keeps_clear_of_it() {
    let scene = scene();
    let clear = Rc::new(Cell::new(None));
    let kept = Rc::clone(&clear);
    let mut fixture = fixture_with(&scene, move |bar| {
        let bar = bar.side(Side::End, Align::Start);
        match kept.get() {
            Some(point) => bar.keep_clear(point),
            None => bar,
        }
    });
    fixture.settle();
    let origin = seen(&scene);
    // How far a document point moves on screen, to put the anchor at the canvas's right edge.
    scene.borrow_mut().anchor = frame(DocVec::new(100.0, 0.0));
    fixture.settle();
    let probed = seen(&scene);
    let scale = f64::from(probed.bounds.min.x - origin.bounds.min.x) / 100.0;
    let far = 100.0 + f64::from(probed.area.max.x - probed.bounds.max.x - 8.0) / scale;
    scene.borrow_mut().anchor = frame(DocVec::new(far, 0.0));
    fixture.settle();
    let flipped = seen(&scene);
    assert!(
        flipped.rect.right() <= flipped.bounds.left() + SLACK,
        "the bar flipped to the left of the anchor {:?}: {:?}",
        flipped.bounds,
        flipped.rect
    );

    // A point a little to the bar's right: a bar moved by 60 points would cover it.
    let at = egui::pos2(flipped.rect.max.x + 20.0, flipped.rect.center().y);
    let corner = frame(DocVec::new(far, 0.0))[0];
    clear.set(Some(corner + DocVec::new(f64::from(at.x - flipped.bounds.min.x) / scale, f64::from(at.y - flipped.bounds.min.y) / scale)));
    fixture.settle();
    assert!((seen(&scene).rect.min - flipped.rect.min).length() < SLACK, "the bar stays at its unmoved place until moved");

    let grip = fixture.by_label("Move bar").expect("the grip is named");
    fixture.drag(&grip, Vec2::new(60.0, 0.0)).settle();
    let dropped = seen(&scene);
    let offset = dropped.place.offset.expect("the drag gave the bar an offset");
    assert!((offset[0] - 60.0).abs() < 2.0 * SLACK && offset[1].abs() < 2.0 * SLACK, "the offset is the drag: {offset:?}");
    assert!((dropped.rect.min - flipped.rect.min).length() < SLACK, "the bar covers the point at its offset place: {:?}", dropped.rect);
    fixture.settle();
    assert_eq!(seen(&scene).place.offset, dropped.place.offset, "the offset is kept on the next pass");
}

#[trial]
fn a_fixed_task_bar_ignores_a_drag_shows_no_more_menu_and_opens_above_with_no_room_below() {
    let scene = Rc::new(RefCell::new(Scene {
        anchor: low_frame(),
        place: TaskBarPlace::default(),
        bounds: Rect::NOTHING,
        area: Rect::NOTHING,
        rect: Rect::NOTHING,
    }));
    let mut fixture = fixture(&scene, true);
    fixture.settle();
    let shown = seen(&scene);
    assert!(shown.rect.is_positive(), "the bar shows: {:?}", shown.rect);
    assert!(fixture.by_label("Move bar").is_none(), "a fixed bar has no grip");
    assert!(fixture.by_label("More options").is_none(), "a fixed bar has no More menu");
    assert!(shown.rect.bottom() < shown.bounds.top(), "with no room below, the bar hangs above {:?}: {:?}", shown.bounds, shown.rect);
    assert!(inside_area(&shown), "inside the canvas {:?}: {:?}", shown.area, shown.rect);

    let free = shown.rect.left_center() + Vec2::new(2.0, 0.0);
    fixture.drag_at(free, Vec2::new(-120.0, -60.0)).settle();
    let dragged = seen(&scene);
    assert!((dragged.rect.min - shown.rect.min).length() < SLACK, "a drag moves nothing: {:?} then {:?}", shown.rect, dragged.rect);
    assert_eq!(dragged.place, TaskBarPlace::default(), "a fixed bar keeps no place");

    // An edge bar takes no anchor: it sits at the view's top edge, at its end, wherever the view scrolls.
    let centre = Rc::new(Cell::new(StoryDocument::STANDARD.centre()));
    let reported = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let (view_centre, shown_at) = (Rc::clone(&centre), Rc::clone(&reported));
    let mut fixture = Fixture::new(move |ui| {
        let at = view_centre.get();
        let _ = on_canvas_view(
            ui,
            |room, ppp| CanvasView::new(room, at, 1.0 / f64::from(ppp)),
            Some(&StoryDocument::STANDARD),
            |ui, area, _| {
                let bar = TaskBar::at_edge("Find", Align::End).show(ui, area, |ui, _| {
                    let _ = Button::new("Next").show(ui);
                });
                shown_at.set((area, bar.output.rect));
                bar.output
            },
        );
    });
    fixture.room(Vec2::new(1120.0, 720.0));
    fixture.settle();
    let (area, edge) = reported.get();
    assert!(edge.is_positive(), "the edge bar shows with no anchor: {edge:?}");
    assert!(area.expand(SLACK).contains_rect(edge), "inside the canvas {area:?}: {edge:?}");
    assert!(edge.top() - area.top() < edge.height(), "at the top edge of {area:?}: {edge:?}");
    assert!(area.right() - edge.right() < edge.left() - area.left(), "nearer the end edge of {area:?}: {edge:?}");
    centre.set(centre.get() + DocVec::new(80.0, 60.0));
    fixture.settle();
    let (_, scrolled) = reported.get();
    assert!(
        (scrolled.min - edge.min).length() < SLACK && (scrolled.size() - edge.size()).length() < SLACK,
        "the edge bar keeps its rect while the view scrolls: {edge:?} then {scrolled:?}"
    );
}

/// An anchor that fills the canvas leaves no room on either side: the task bar and a fixed bar still show all their
/// members, over the anchor at the area's bottom edge, instead of an empty surface at its top (photocraft#16657).
#[trial]
fn a_bar_whose_anchor_fills_the_area_keeps_its_members_at_the_bottom_edge() {
    for fixed in [false, true] {
        let scene = Rc::new(RefCell::new(Scene {
            anchor: filling_frame(),
            place: TaskBarPlace::default(),
            bounds: Rect::NOTHING,
            area: Rect::NOTHING,
            rect: Rect::NOTHING,
        }));
        let mut fixture = fixture(&scene, fixed);
        fixture.settle();
        let shown = seen(&scene);
        assert!(shown.bounds.contains_rect(shown.area), "fixed {fixed}: the anchor {:?} covers the canvas {:?}", shown.bounds, shown.area);
        let done = fixture.by_label("Done").unwrap_or_else(|| panic!("fixed {fixed}: the bar shows its members"));
        assert!(done.rect.is_positive(), "fixed {fixed}: Done has a size: {:?}", done.rect);
        assert!(shown.rect.contains_rect(done.rect.shrink(SLACK)), "fixed {fixed}: Done {:?} lies in the bar {:?}", done.rect, shown.rect);
        assert!(inside_area(&shown), "fixed {fixed}: inside the canvas {:?}: {:?}", shown.area, shown.rect);
        assert!(
            (shown.area.bottom() - shown.rect.bottom()).abs() < SLACK,
            "fixed {fixed}: on the canvas's bottom edge {:?}: {:?}",
            shown.area,
            shown.rect
        );
        assert_eq!(fixture.by_label("More options").is_some(), !fixed, "fixed {fixed}: only the task bar has its More menu");
    }
}

/// The grip is one of the bar's members: it sits on the members' line, centred with them, inside the bar's padding,
/// and a drag on it still moves the bar.
#[trial]
fn the_grip_sits_on_the_members_line() {
    let scene = scene();
    let mut fixture = fixture(&scene, false);
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
    let mut fixture = fixture(&scene, false);
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
    assert!(dropped.place.offset.is_some(), "the drop writes the place, as an offset from the anchor while the bar follows it: {:?}", dropped.place);
    fixture.settle();
    assert!((seen(&scene).rect.min - dropped_at).length() < SLACK, "the written place keeps the bar there: {:?}", seen(&scene).rect);
}

/// A fixed bar of two icon buttons (production's text box Done and Discard), compact or not: its rect, and each
/// button's rect by name.
fn two_buttons(compact: bool) -> (Rect, Vec<Rect>) {
    use craft_behavior::IconId;
    use craft_ui::buttons::IconButton;
    use craft_ui::contract::Props as _;
    let scene = Rc::new(RefCell::new(Rect::NOTHING));
    let shared = Rc::clone(&scene);
    let mut fixture = Fixture::new(move |ui| {
        let _ = on_canvas(ui, |ui, area, _| {
            let anchor = frame(DocVec::ZERO);
            let shown = TaskBar::fixed("Text box actions", &anchor).compact(compact).show(ui, area, |ui, _| {
                let _ = IconButton::new(IconId::new("check")).label("Done adding text").show(ui);
                let _ = IconButton::new(IconId::new("close")).label("Discard this text").show(ui);
            });
            *shared.borrow_mut() = shown.output.rect;
            shown.output
        });
    });
    fixture.room(Vec2::new(1120.0, 720.0));
    fixture.settle();
    let buttons = ["Done adding text", "Discard this text"].map(|name| fixture.by_label(name).map_or(Rect::NOTHING, |button| button.rect));
    let bar = *scene.borrow();
    (bar, buttons.to_vec())
}

/// A compact bar holds its members at the small control size in a tight frame: smaller than the standard bar, with
/// every member whole inside it.
#[trial]
fn a_compact_task_bar_is_smaller_and_shows_every_member_whole() {
    let (standard, _) = two_buttons(false);
    let (compact, buttons) = two_buttons(true);
    assert!(compact.is_positive() && standard.is_positive(), "both bars show: {standard:?} {compact:?}");
    assert!(compact.height() + 4.0 < standard.height(), "the compact bar is lower: {compact:?} against {standard:?}");
    assert!(compact.width() + 4.0 < standard.width(), "the compact bar is narrower: {compact:?} against {standard:?}");
    for button in buttons {
        assert!(button.is_positive() && compact.expand(SLACK).contains_rect(button), "each member shows whole in {compact:?}: {button:?}");
    }
}

/// The fill of the bar grip painted inside `grip` among `shapes`.
fn grip_ink(shapes: &[egui::Shape], grip: Rect) -> Option<egui::Color32> {
    shapes.iter().find_map(|shape| match shape {
        egui::Shape::Rect(rect) if grip.contains_rect(rect.rect) => Some(rect.fill),
        _ => None,
    })
}

/// The bar grip lights under the pointer and while the bar moves, and goes back to its rest ink after the drop.
#[trial]
fn the_bar_grip_lights_under_the_pointer_and_while_the_bar_moves() {
    let scene = scene();
    let mut fixture = fixture_with(&scene, |bar| bar.grip(TaskBarGrip::Bar));
    fixture.settle();
    let grip = fixture.by_label("Move bar").expect("the grip is named");
    let rest = grip_ink(&fixture.frame_shapes(), grip.rect).expect("the grip paints its bar");

    fixture.frame(vec![moved(grip.center())]).idle(8);
    let hovered = grip_ink(&fixture.frame_shapes(), grip.rect);
    assert!(hovered.is_some_and(|ink| ink != rest), "the grip lights under the pointer: {hovered:?} against {rest:?}");

    let by = Vec2::new(-60.0, 30.0);
    fixture.drag_hold_at(grip.center(), by);
    let moving = grip_ink(&fixture.frame_shapes(), grip.rect.translate(by));
    assert!(moving.is_some_and(|ink| ink != rest), "the grip is lit while the bar moves: {moving:?} against {rest:?}");

    fixture.release_at(grip.center() + by).frame(vec![moved(grip.center() + by + Vec2::new(0.0, 200.0))]).idle(8);
    let dropped = fixture.by_label("Move bar").expect("the grip is named");
    assert_eq!(grip_ink(&fixture.frame_shapes(), dropped.rect), Some(rest), "the grip is back to rest once dropped and left");
}

/// A bar hung clear of the handles hangs farther below its anchor than the default, by the handle ring at least.
#[trial]
fn a_bar_clear_of_the_handles_hangs_farther_below_its_anchor() {
    let below = |configure: fn(TaskBar<'_>) -> TaskBar<'_>| {
        let scene = scene();
        let mut fixture = fixture_with(&scene, configure);
        fixture.settle();
        let seen = seen(&scene);
        seen.rect.top() - seen.bounds.bottom()
    };
    let (default, clear) = (below(|bar| bar), below(|bar| bar.hang_gap(TaskBarHang::ClearOfHandles)));
    assert!(default > 0.0, "the bar hangs below its anchor: {default}");
    assert!(clear > default + 2.0 * SLACK, "the bar clear of the handles hangs farther below: {clear} against {default}");
}
