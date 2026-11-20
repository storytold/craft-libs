//! The tab strip's keys, drags and drops, and a tab's close button, which the tab declares as a part while it is a
//! member of the strip's run.

use craft_behavior::{SelectionChange, keyed};
use craft_ui::contract::{CollectionEvent, Role, ShowCollection as _};
use craft_ui::runtime::egui::access::AccessTree;
use craft_ui::tabs::{Tab, TabKind, TabRole, TabStrip};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TITLES: [&str; 3] = ["One", "Two", "Three"];

/// An arrow moves the focus to the next tab without showing it; Enter shows it.
#[trial]
fn an_arrow_moves_the_focus_and_enter_shows_the_tab() {
    let shown = Rc::new(Cell::new(0_usize));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let (now, seen) = (shown.clone(), picked.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Documents").shown(now.get()).show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.last()
            {
                seen.borrow_mut().push(*key);
                now.set(*key);
            }
        }
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!((shown.get(), picked.borrow().clone()), (0, Vec::new()), "the arrow shows no other tab");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(shown.get(), 1, "Enter shows the focused tab");
}

/// A click on a tab's close button closes the tab, and the close button is a node under its tab's node that an
/// AccessKit Click closes.
#[trial]
fn the_close_button_and_its_node_close_a_tab() {
    let closed = Rc::new(RefCell::new(Vec::new()));
    let seen = closed.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Documents").shown(0).show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        seen.borrow_mut().extend(out.closed);
    });
    fixture.settle();
    let tab = fixture.by_label("Two").expect("the tab's node");
    let close = fixture.by_label("Close Two").expect("the close button's node");
    assert_eq!(fixture.tree().parent_of_key(close.node), Some(tab.node), "the close button's node is under its tab's");
    fixture.click(&close);
    assert_eq!(closed.borrow().clone(), vec![1], "the close button closes its tab");
    let first = fixture.by_label("Close One").expect("the close button's node");
    fixture.access(AccessActionKind::Click, &first).idle(1);
    assert_eq!(closed.borrow().clone(), vec![1, 0], "an AccessKit Click on the close node closes its tab");
}

/// A strip of the three titles showing tab 0 that applies its selection changes, with or without `wheel_selects`;
/// returns the fixture and the tab it shows.
fn wheel_rig(wheel_selects: bool) -> (Fixture, Rc<Cell<usize>>) {
    let shown = Rc::new(Cell::new(0_usize));
    let now = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let strip = TabStrip::new("Documents").shown(now.get());
        let strip = if wheel_selects { strip.wheel_selects() } else { strip };
        let out = strip.show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.last()
            {
                now.set(*key);
            }
        }
    });
    fixture.settle();
    (fixture, shown)
}

/// With `wheel_selects`, a wheel notch down over the strip shows the next tab and a notch up the previous one; without
/// it the wheel shows no other tab.
#[trial]
fn the_wheel_steps_the_shown_tab_only_where_the_strip_asks_for_it() {
    use craft_ui::testing::pointer;
    for wheel_selects in [true, false] {
        let (mut fixture, shown) = wheel_rig(wheel_selects);
        let over = fixture.by_label("Two").expect("the tab's node").center();
        fixture.frame(vec![pointer::moved(over)]).idle(1);
        fixture.frame(vec![pointer::wheel_lines(-1.0)]).idle(2);
        assert_eq!(shown.get(), usize::from(wheel_selects), "a notch down, wheel_selects {wheel_selects}");
        if wheel_selects {
            fixture.frame(vec![pointer::wheel_lines(1.0)]).idle(2);
            assert_eq!(shown.get(), 0, "a notch up shows the previous tab");
        }
    }
}

/// A double click on the empty room after the last document tab asks for a new document; a double click on a tab
/// activates it and asks for none.
#[trial]
fn a_double_click_on_the_empty_strip_asks_for_a_new_document() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two")];
        let out = DocumentTabs::new("Documents", &tabs).selected(0).show(ui);
        held.borrow_mut().extend(out.events);
    });
    fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
    let last = fixture.by_label("Two").expect("the tab's node").rect;
    let empty = egui::Pos2::new(last.right() + 60.0, last.center().y);
    fixture.click_at(empty).click_at(empty).settle();
    assert!(seen.borrow().contains(&DocumentTabsEvent::EmptyDoubleClick), "{:?}", seen.borrow());
    seen.borrow_mut().clear();
    fixture.advance(1.0).idle(1);
    fixture.click_at(last.center()).click_at(last.center()).settle();
    let seen = seen.borrow();
    assert!(seen.contains(&DocumentTabsEvent::Activated(1)), "{seen:?}");
    assert!(!seen.contains(&DocumentTabsEvent::EmptyDoubleClick), "{seen:?}");
}

/// The bounds of every tab node named `name`, from the reading start.
fn tab_rects(tree: &AccessTree, name: &str) -> Vec<Rect> {
    let mut rects: Vec<Rect> =
        tree.with_role(Role::TAB).into_iter().filter(|key| tree.name_of(*key) == Some(name)).filter_map(|key| tree.bounds_of(key)).collect();
    rects.sort_by(|a, b| a.left().total_cmp(&b.left()));
    rects
}

/// The bounds of the tab node named `name`.
fn tab_rect(tree: &AccessTree, name: &str) -> Rect {
    tab_rects(tree, name).first().copied().expect("the tab's node")
}

/// Files from the operating system, a drag that did not start in the strip, rest over `at` for a few passes and then
/// leave the window.
fn rest_files_over(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(at)], &mut *show);
    frames.idle(3, &mut *show);
    frames.leave_files();
    frames.frame(vec![moved(at)], &mut *show);
}

/// Files from the operating system hover over `at` and drop there.
fn drop_files_at(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(at)], &mut *show);
    frames.idle(1, &mut *show);
    frames.drop_files(&["a.png"]);
    frames.frame(vec![moved(at)], &mut *show);
    frames.idle(1, &mut *show);
}

/// Where the last `FilesDropped` of `events` puts the files: before a tab, or last.
fn files_before(events: &[craft_ui::tabs::DocumentTabsEvent<usize>]) -> Option<Option<usize>> {
    use craft_ui::tabs::DocumentTabsEvent;
    events.iter().rev().find_map(|event| match event {
        DocumentTabsEvent::FilesDropped { before, .. } => Some(*before),
        _ => None,
    })
}

/// A drag that did not start in the strip, resting over an unselected document tab, selects it in that pass; a drag of
/// the strip's own tab (a reorder) selects nothing it crosses, and a panel strip selects nothing under a drag.
/// `Fixture` cannot hover files from the OS, so these trials drive `Frames`.
#[trial]
fn a_drag_resting_over_a_tab_selects_it_unless_it_is_the_strips_own() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    let seen = RefCell::new(Vec::new());
    let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two"), DocumentTab::new(2, "Three")];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let out = DocumentTabs::new("Documents", &tabs).selected(0).reorderable(true).show(ui);
        seen.borrow_mut().extend(out.events);
    };
    frames.idle(2, &mut show);
    let tree = frames.tree();
    let (one, two, three) = (tab_rect(&tree, "One"), tab_rect(&tree, "Two"), tab_rect(&tree, "Three"));
    rest_files_over(&mut frames, two.center(), &mut show);
    assert!(seen.borrow().contains(&DocumentTabsEvent::Selected(1)), "files resting over a tab select it: {:?}", seen.borrow());

    seen.borrow_mut().clear();
    frames.frame(vec![moved(one.center())], &mut show);
    frames.frame(vec![press(one.center())], &mut show);
    for step in 1..=4 {
        frames.frame(vec![moved(one.center().lerp(three.center(), step as f32 / 4.0))], &mut show);
    }
    frames.idle(3, &mut show);
    let crossed = seen.borrow().iter().any(|event| matches!(event, DocumentTabsEvent::Selected(1 | 2)));
    assert!(!crossed, "the strip's own drag selects no tab it rests over: {:?}", seen.borrow());
    frames.frame(vec![moved(one.center())], &mut show);
    frames.frame(vec![release(one.center())], &mut show);
    frames.idle(1, &mut show);

    let picked = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut panel = |ui: &mut Ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Panels").kind(TabKind::Panel).shown(0).show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event {
                picked.borrow_mut().extend(keys.iter().copied());
            }
        }
    };
    frames.idle(2, &mut panel);
    let two = tab_rect(&frames.tree(), "Two");
    rest_files_over(&mut frames, two.center(), &mut panel);
    assert!(!picked.borrow().contains(&1), "a panel strip selects nothing under a drag: {:?}", picked.borrow());
}

/// Files from the operating system dropped in the gap before the second of three document tabs go before it, and past
/// the last tab go last; a reorder drag in the same strip still reorders.
#[trial]
fn files_dropped_between_tabs_land_before_the_tab_after_the_gap() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    let seen = RefCell::new(Vec::new());
    let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two"), DocumentTab::new(2, "Three")];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let out = DocumentTabs::new("Documents", &tabs).selected(0).reorderable(true).takes_files(true).show(ui);
        seen.borrow_mut().extend(out.events);
    };
    frames.idle(2, &mut show);
    let tree = frames.tree();
    let (one, two, three) = (tab_rect(&tree, "One"), tab_rect(&tree, "Two"), tab_rect(&tree, "Three"));

    drop_files_at(&mut frames, Pos2::new(two.left() + 1.0, two.center().y), &mut show);
    assert_eq!(files_before(&seen.borrow()), Some(Some(1)), "files in the gap go before the tab after it: {:?}", seen.borrow());

    seen.borrow_mut().clear();
    drop_files_at(&mut frames, Pos2::new(three.right() + 40.0, three.center().y), &mut show);
    assert_eq!(files_before(&seen.borrow()), Some(None), "files past the last tab go last: {:?}", seen.borrow());

    seen.borrow_mut().clear();
    let end = Pos2::new(three.right() - 2.0, three.center().y);
    frames.frame(vec![moved(one.center())], &mut show);
    frames.frame(vec![press(one.center())], &mut show);
    for step in 1..=4 {
        frames.frame(vec![moved(one.center().lerp(end, step as f32 / 4.0))], &mut show);
    }
    frames.frame(vec![release(end)], &mut show);
    frames.idle(1, &mut show);
    let seen = seen.borrow();
    assert!(
        seen.iter().any(|event| matches!(event, DocumentTabsEvent::Reordered { documents, .. } if documents.as_slice() == [0])),
        "the strip's own drag still reorders: {seen:?}"
    );
    assert_eq!(files_before(&seen), None, "a reorder drops no files: {seen:?}");
}

/// Two text tabs both labelled "Options" have a node each, and a click on the second shows the second: the keys, not
/// the labels, give each tab its identity.
#[trial]
fn tabs_with_the_same_label_keep_their_own_selection_and_click() {
    let shown = Rc::new(Cell::new(0_usize));
    let now = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Settings").kind(TabKind::Text).shown(now.get()).show_items(ui, &source, &mut |_, _| Tab::new("Options"));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.last()
            {
                now.set(*key);
            }
        }
    });
    fixture.settle();
    let rects = tab_rects(fixture.tree(), "Options");
    assert_eq!(rects.len(), 2, "each tab has its own node: {rects:?}");
    fixture.click_at(rects[1].center()).settle();
    assert_eq!(shown.get(), 1, "a click on the second tab shows the second");
    assert_eq!(tab_rects(fixture.tree(), "Options").len(), 2, "both tabs keep their nodes after the click");
    fixture.click_at(rects[0].center()).settle();
    assert_eq!(shown.get(), 0, "a click on the first tab shows the first");
}

/// The strip reports the tab under the pointer, also while a press held on another component is dragged over it, and
/// none over the empty room.
#[trial]
fn the_strip_reports_the_tab_under_the_pointer_while_another_root_holds_the_press() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let seen = Rc::new(RefCell::new(None));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two")];
        let out = DocumentTabs::new("Documents", &tabs).selected(0).show(ui);
        *held.borrow_mut() = out.hovered;
        Button::new("Canvas").show(ui);
    });
    fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
    let second = fixture.by_label("Two").expect("the tab's node").rect;
    let canvas = fixture.by_label("Canvas").expect("the button's node").center();
    let empty = egui::Pos2::new(second.right() + 60.0, second.center().y);
    fixture.frame(vec![craft_ui::testing::pointer::moved(canvas)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(canvas)]);
    fixture.frame(vec![craft_ui::testing::pointer::moved(second.center())]);
    fixture.frame(vec![]);
    assert_eq!(*seen.borrow(), Some(1), "the second tab is under the pointer");
    fixture.frame(vec![craft_ui::testing::pointer::moved(empty)]);
    fixture.frame(vec![]);
    assert_eq!(*seen.borrow(), None, "the empty room has no tab");
}

const PANELS: [&str; 6] = ["Layers", "Channels", "Properties", "Adjustments", "History", "Navigator"];

/// The widths of the panel tabs a strip shows in a room `room` wide, by title, with the second panel shown.
fn panel_widths(room: f32) -> Vec<(&'static str, f32)> {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        ui.set_max_width(room);
        let keys = [0_usize, 1, 2, 3, 4, 5];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = TabStrip::new("Panels").kind(TabKind::Panel).shown(1).show_items(ui, &source, &mut |key, _| Tab::new(PANELS[*key]));
    };
    frames.idle(3, &mut show);
    let tree = frames.tree();
    PANELS.iter().filter_map(|name| tab_rects(&tree, name).first().map(|rect| (*name, rect.width()))).collect()
}

/// Panel tabs shrink around the shown tab: it keeps its whole title while the others keep their first letters, and
/// tabs overflow from the end only when even those do not fit.
#[trial]
fn panel_tabs_keep_the_shown_tab_whole_and_overflow_from_the_end() {
    let wide = panel_widths(2000.0);
    assert_eq!(wide.len(), PANELS.len(), "every tab fits in a wide room");
    let natural_shown = wide[1].1;
    let mut fewer = wide.len();
    for room in [700.0, 500.0, 400.0, 300.0, 220.0] {
        let shown = panel_widths(room);
        assert!(!shown.is_empty() && shown.len() <= fewer, "rooms below shrink tabs, never add them: {room} {shown:?}");
        fewer = shown.len();
        let active = shown.iter().find(|(name, _)| *name == PANELS[1]).expect("the shown tab stays");
        assert!((active.1 - natural_shown).abs() < 1.0, "the shown tab keeps its width in {room}: {shown:?}");
        let others: Vec<f32> = shown.iter().filter(|(name, _)| *name != PANELS[1]).map(|&(_, w)| w).collect();
        let least = others.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(least > 0.0, "every shown tab keeps room for letters in {room}: {shown:?}");
        let hidden: Vec<&str> = PANELS.iter().copied().filter(|name| shown.iter().all(|(shown, _)| shown != name)).collect();
        let first = PANELS.iter().position(|name| hidden.contains(name));
        if let Some(first) = first {
            let after = PANELS[first..].iter().filter(|name| **name != PANELS[1]);
            assert!(after.into_iter().all(|name| hidden.contains(name)), "overflow takes the last tabs: {hidden:?}");
        }
    }
    assert!(fewer < PANELS.len(), "the narrowest room overflows");
}

/// A strip places its tabs by their measure in its first frame: a panel strip too narrow for its tabs fits them by
/// their measured widths and takes their measured height, in a frame that runs once and measures each tab's height
/// at most twice (at its ideal width, and at the width the fit shrinks it to), and the strip and every tab have the
/// rects they have once the strip settles.
#[trial]
fn a_tab_strip_takes_its_measured_extents_in_its_first_frame() {
    let mut fixture = Fixture::new(|ui| {
        ui.set_max_width(300.0);
        let keys = [0_usize, 1, 2, 3, 4, 5];
        let source = keyed(&keys, |key: &usize| *key);
        let strip = TabStrip::new("Panels").kind(TabKind::Panel).natural_height(true).shown(1);
        let _ = strip.show_items(ui, &source, &mut |key, _| Tab::new(PANELS[*key]));
    });
    let rects = |fixture: &Fixture| {
        let strip = fixture.by_label("Panels").expect("the strip's node").rect;
        (strip, PANELS.map(|name| tab_rects(fixture.tree(), name).first().copied()))
    };
    fixture.idle(1);
    assert_eq!(fixture.passes(), 1, "the first frame runs once");
    let measures = craft_ui::testing::row_measures(fixture.ctx());
    assert!(measures.len() == PANELS.len() && measures.iter().all(|(_, count)| *count <= 2), "each tab is measured at most twice: {measures:?}");
    let first = rects(&fixture);
    fixture.idle(3);
    assert_eq!(first, rects(&fixture), "the first frame places the strip and its tabs as the settled strip does");
}

/// A strip reports the rects of the tabs it places (`tab_rects`, where a dock registers its insertion spots) as it
/// places them, in the frame a tab's title changes.
#[trial]
fn a_strip_reports_its_tab_rects_in_the_frame_a_title_changes() {
    let titles = Rc::new(RefCell::new(TITLES.map(str::to_owned)));
    let reported = Rc::new(RefCell::new(Vec::new()));
    let (now, seen) = (titles.clone(), reported.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let names = now.borrow().clone();
        let out = TabStrip::new("Documents").shown(0).show_items(ui, &source, &mut |key, _| Tab::new(names[*key].clone()));
        *seen.borrow_mut() = out.tab_rects;
    });
    fixture.settle();
    titles.borrow_mut()[0] = "A much longer first title".to_owned();
    fixture.idle(1);
    let placed: Vec<(usize, Rect)> = titles.borrow().iter().enumerate().map(|(key, name)| (key, tab_rect(fixture.tree(), name))).collect();
    assert_eq!(*reported.borrow(), placed, "the strip reports the tabs where it places them");
}

/// The ribbon's tab names; the last is the contextual tab.
const RIBBON: [&str; 6] = ["File", "Home", "Insert", "Layout", "Review", "Table Design"];

/// The key of the contextual tab.
const TABLE: usize = 5;

/// The role of a ribbon trial's tab: the last one is contextual.
fn ribbon_role(key: &usize) -> TabRole {
    if *key == TABLE { TabRole::Contextual } else { TabRole::Standard }
}

/// The sliding underline a frame painted in the tab row `row`: the horizontal two-point line in its band, as its left
/// and right x. Ribbon tabs paint no line of their own.
fn underline(shapes: &[egui::Shape], row: Rect) -> Option<(f32, f32)> {
    shapes.iter().find_map(|shape| match shape {
        egui::Shape::Path(path) if !path.closed && path.points.len() == 2 => {
            let (a, b) = (path.points[0], path.points[1]);
            let flat = (a.y - b.y).abs() < 0.01;
            (flat && a.y >= row.top() && a.y <= row.bottom()).then(|| (a.x.min(b.x), a.x.max(b.x)))
        }
        _ => None,
    })
}

/// A ribbon strip's one underline runs to a newly shown tab, and a tab shown while it runs turns it where it is: at the
/// reversal it lies between the two tabs, and it comes back to rest under the first tab without a jump. The wheel
/// shows the tabs, one frame per change.
#[trial]
fn the_ribbon_underline_retargets_from_its_shown_position_mid_motion() {
    use craft_ui::testing::pointer;
    let shown = Rc::new(Cell::new(0_usize));
    let now = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let strip = TabStrip::new("Ribbon").kind(TabKind::Ribbon).wheel_selects().shown(now.get());
        let out = strip.show_items(ui, &source, &mut |key, _| Tab::new(RIBBON[*key]));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.last()
            {
                now.set(*key);
            }
        }
    });
    fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
    let (first, third) = (tab_rect(fixture.tree(), RIBBON[0]), tab_rect(fixture.tree(), RIBBON[2]));
    let row = first.union(third);
    let rest = underline(&fixture.frame_shapes(), row).expect("the shown tab's underline");
    // It runs under the first tab label, inset from both sides of the tab by the same amount (the tab padding less
    // half the stroke), as the old ribbon drew it.
    let inset = rest.0 - first.left();
    assert!(inset > 0.5 && (first.right() - rest.1 - inset).abs() < 0.5, "at rest it lies under the first tab label: {rest:?} {first:?}");

    fixture.frame(vec![pointer::moved(row.center())]).idle(1);
    fixture.frame(vec![pointer::wheel_lines(-2.0)]);
    assert_eq!(shown.get(), 2, "two notches down show the third tab");
    let going: Vec<f32> = (0..2).filter_map(|_| underline(&fixture.frame_shapes(), row)).map(|line| line.0).collect();
    assert!(
        going.len() == 2 && going[1] > first.left() + 0.5 && going[1] < third.left() - 0.5,
        "the underline is on its way to the third tab: {going:?} {first:?} {third:?}"
    );

    fixture.frame(vec![pointer::wheel_lines(2.0)]);
    assert_eq!(shown.get(), 0, "two notches up show the first tab again");
    let distance = third.left() - first.left();
    let turned = underline(&fixture.frame_shapes(), row).expect("the underline at the reversal").0;
    assert!(
        turned > first.left() + 0.5 && turned < third.left() - 0.5,
        "at the reversal the underline lies between the two tabs, not at either target: {turned} {first:?} {third:?}"
    );
    assert!((turned - going[1]).abs() < distance / 2.0, "the reversal starts where the underline was: {going:?} then {turned}");
    let mut last = turned;
    for _ in 0..30 {
        let at = underline(&fixture.frame_shapes(), row).expect("the underline while it returns").0;
        assert!(at <= last + 0.5, "the underline only moves back toward the first tab: {last} then {at}");
        assert!((last - at).abs() < distance / 2.0, "the underline does not jump: {last} then {at}");
        last = at;
    }
    assert!((last - rest.0).abs() < 0.5, "the underline comes to rest under the first tab: {last} {rest:?}");
}

/// What a ribbon strip of the six tabs with two end items shows in a room `room` wide: the tabs on the strip with their
/// widths, whether each end item paints its label, each end item's width, and whether the overflow button is there.
struct EndView {
    tabs: Vec<(&'static str, f32)>,
    share_label: bool,
    comments_label: bool,
    share: f32,
    comments: f32,
    overflow: bool,
}

impl EndView {
    fn shows(&self, name: &str) -> bool {
        self.tabs.iter().any(|(shown, _)| *shown == name)
    }
}

fn end_view(room: f32) -> EndView {
    use craft_behavior::IconId;
    use craft_ui::contract::Entry;
    use craft_ui::runtime::commands::{CommandRef, EditCommands};
    use craft_ui::tabs::StripEnd;
    let rows: [Entry<CommandRef>; 1] = [Entry::new(EditCommands::Paste.into(), "Show comments")];
    let items =
        [StripEnd::command("Share", IconId::new("link").into(), EditCommands::Copy), StripEnd::menu("Comments", IconId::new("list").into(), &rows)];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        ui.set_max_width(room);
        let keys: Vec<usize> = (0..RIBBON.len()).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let strip = TabStrip::new("Ribbon").kind(TabKind::Ribbon).shown(1).roles(ribbon_role).trailing(&items);
        let _ = strip.show_items(ui, &source, &mut |key, _| Tab::new(RIBBON[*key]).role(ribbon_role(key)));
    };
    frames.idle(3, &mut show);
    let tree = frames.tree();
    let painted = |name: &str| frames.texts().iter().any(|text| text.text == name);
    let buttons = tree.with_role(Role::BUTTON);
    let width = |name: &str| {
        buttons.iter().find(|key| tree.name_of(**key) == Some(name)).and_then(|key| tree.bounds_of(*key)).map_or(0.0, |rect| rect.width())
    };
    EndView {
        tabs: RIBBON.iter().filter_map(|name| tab_rects(&tree, name).first().map(|rect| (*name, rect.width()))).collect(),
        share_label: painted("Share"),
        comments_label: painted("Comments"),
        share: width("Share"),
        comments: width("Comments"),
        overflow: buttons.iter().any(|key| !matches!(tree.name_of(*key), Some("Share" | "Comments"))),
    }
}

/// As the room narrows, the end items drop their labels, the last one first, before any tab overflows; then the tabs
/// overflow while the items keep their icons. The shown tab and the contextual tab stay on the strip while a plain tab
/// overflows, and the contextual tab gives way only to plain tabs narrower than itself.
#[trial]
fn end_items_drop_their_labels_before_a_tab_overflows() {
    let wide = end_view(2000.0);
    assert_eq!(wide.tabs.len(), RIBBON.len(), "every tab fits a wide room");
    assert!(wide.share_label && wide.comments_label && !wide.overflow, "a wide room shows both labels and no overflow");
    let table = wide.tabs.iter().find(|(name, _)| *name == RIBBON[TABLE]).map_or(0.0, |&(_, width)| width);
    let (mut comments_only, mut all_short, mut kept) = (false, false, false);
    // A coarse sweep from the width that holds every tab and both labels down to a room that overflows most tabs;
    // each stage of the order lasts for several steps.
    const STEP: f32 = 16.0;
    const FLOOR: f32 = 200.0;
    let top = wide.tabs.iter().map(|&(_, width)| width).sum::<f32>() + wide.share + wide.comments;
    for step in 0..=((top + STEP - FLOOR) / STEP) as usize {
        let room = top + STEP - step as f32 * STEP;
        let view = end_view(room);
        assert!(view.share_label || !view.comments_label, "Comments drops its label before Share in {room}");
        assert!(view.share > 0.0 && view.comments > 0.0, "the end items stay in {room}");
        if !view.comments_label {
            assert!(view.comments < wide.comments - 0.5, "an item without its label is narrower in {room}");
        }
        let all_tabs = view.tabs.len() == RIBBON.len();
        assert_eq!(view.overflow, !all_tabs, "the overflow button is there only while a tab overflows in {room}");
        if all_tabs {
            comments_only |= view.share_label && !view.comments_label;
            all_short |= !view.share_label && !view.comments_label;
            continue;
        }
        assert!(!view.share_label && !view.comments_label, "both items are icon only before a tab overflows in {room}");
        assert!(view.shows(RIBBON[1]), "the shown tab stays in {room}");
        if view.shows(RIBBON[TABLE]) {
            kept = true;
        } else {
            let wider = view.tabs.iter().filter(|(name, _)| *name != RIBBON[1]).find(|&&(_, width)| width >= table);
            assert!(wider.is_none(), "the contextual tab gives way only to narrower plain tabs in {room}: {:?}", view.tabs);
        }
    }
    assert!(comments_only, "a room shows Share whole and Comments icon only with every tab");
    assert!(all_short, "a room shows both items icon only with every tab");
    assert!(kept, "a room keeps the contextual tab while a plain tab overflows");
}

/// A ribbon strip leaves the room after its last end item that it leaves before its first tab, so an end item does not
/// touch the strip's end edge, in a wide room and a narrow one (storytold/wordcraft#7164).
#[trial]
fn the_last_end_item_keeps_an_inset_from_the_strip_end() {
    use craft_behavior::IconId;
    use craft_ui::runtime::commands::EditCommands;
    use craft_ui::tabs::StripEnd;
    for room in [2000.0, 600.0, 300.0] {
        let items = [StripEnd::command("Share", IconId::new("link").into(), EditCommands::Copy)];
        let edges = Cell::new((0.0, 0.0));
        let mut frames = Frames::new();
        let mut show = |ui: &mut Ui| {
            ui.set_max_width(room);
            edges.set((ui.max_rect().left(), ui.max_rect().right()));
            let keys: Vec<usize> = (0..RIBBON.len()).collect();
            let source = keyed(&keys, |key: &usize| *key);
            let strip = TabStrip::new("Ribbon").kind(TabKind::Ribbon).shown(1).roles(ribbon_role).trailing(&items);
            let _ = strip.show_items(ui, &source, &mut |key, _| Tab::new(RIBBON[*key]).role(ribbon_role(key)));
        };
        frames.idle(3, &mut show);
        let tree = frames.tree();
        let (left, right) = edges.get();
        let share = tree
            .with_role(Role::BUTTON)
            .iter()
            .find(|key| tree.name_of(**key) == Some("Share"))
            .and_then(|key| tree.bounds_of(*key))
            .expect("the Share item is on the strip");
        let lead = tab_rects(&tree, RIBBON[0]).first().expect("the first tab is on the strip").left() - left;
        assert!(lead > 0.5, "the strip leads in {room}: {lead}");
        assert!(
            right - share.right() >= lead - 0.5,
            "the end item keeps the lead's inset at the end in {room}: {} against {lead}",
            right - share.right()
        );
    }
}

/// The width of the one end item, named "Comments", of a ribbon strip in a wide room: a menu item when `menu`, else a
/// command item with the same label and icon.
fn comments_width(menu: bool) -> f32 {
    use craft_behavior::IconId;
    use craft_ui::contract::Entry;
    use craft_ui::runtime::commands::{CommandRef, EditCommands};
    use craft_ui::tabs::StripEnd;
    let rows: [Entry<CommandRef>; 1] = [Entry::new(EditCommands::Paste.into(), "Show comments")];
    let item = if menu {
        StripEnd::menu("Comments", IconId::new("list").into(), &rows)
    } else {
        StripEnd::command("Comments", IconId::new("list").into(), EditCommands::Copy)
    };
    let items = [item];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        ui.set_max_width(2000.0);
        let keys: Vec<usize> = (0..RIBBON.len()).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let strip = TabStrip::new("Ribbon").kind(TabKind::Ribbon).shown(1).roles(ribbon_role).trailing(&items);
        let _ = strip.show_items(ui, &source, &mut |key, _| Tab::new(RIBBON[*key]).role(ribbon_role(key)));
    };
    frames.idle(3, &mut show);
    let tree = frames.tree();
    let buttons = tree.with_role(Role::BUTTON);
    buttons.iter().find(|key| tree.name_of(**key) == Some("Comments")).and_then(|key| tree.bounds_of(*key)).map_or(0.0, |rect| rect.width())
}

/// An end item that opens a menu shows the chevron of a menu button after its label, as every button that opens a
/// menu does; an item that runs a command shows none, so the menu item is wider by the chevron.
#[trial]
fn a_menu_end_item_shows_the_menu_chevron() {
    let (menu, command) = (comments_width(true), comments_width(false));
    assert!(command > 0.0, "the command item shows");
    assert!(menu > command + 4.0, "the menu item is wider by its chevron: {menu} against {command}");
}

/// A contextual tab the source drops fades out at its slot: while it fades it has no node, a click on its slot
/// selects nothing and the overflow menu does not list it; once the fade ends it is not painted. A frame of two
/// passes paints the fade as one does. A contextual tab that comes back is hit from its first frame.
#[trial]
fn a_contextual_tab_that_leaves_is_not_hit_or_listed_while_it_fades() {
    let (listed, discard) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(false)));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let shown = Rc::new(Cell::new(1_usize));
    let (lists, asks, seen, now) = (listed.clone(), discard.clone(), picked.clone(), shown.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..RIBBON.len()).filter(|&key| key != TABLE || lists.get()).collect();
        // The strip names its overflow entries and its fading tabs by the source's text.
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(RIBBON[*key]));
        let strip = TabStrip::new("Ribbon").kind(TabKind::Ribbon).shown(now.get()).roles(ribbon_role);
        let out = strip.show_items(ui, &source, &mut |key, _| Tab::new(RIBBON[*key]).role(ribbon_role(key)));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.last()
            {
                seen.borrow_mut().push(*key);
                now.set(*key);
            }
        }
        if asks.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs the frame twice");
        }
    });
    fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
    let home = fixture.by_label(RIBBON[1]).expect("the shown tab's node").rect;
    let table = fixture.by_label(RIBBON[TABLE]).expect("the contextual tab's node").rect;
    // A room for the shown tab, the contextual tab and the overflow button: plain tabs overflow.
    let room = home.width() + table.width() + 2.0 * home.height();
    fixture.room(egui::Vec2::new(room, 120.0)).settle();
    let hidden: Vec<&str> = RIBBON.iter().copied().filter(|name| tab_rects(fixture.tree(), name).is_empty()).collect();
    assert!(!hidden.is_empty(), "a plain tab overflows in {room}");
    let table = tab_rect(fixture.tree(), RIBBON[TABLE]);
    fixture.key(Key::Tab, Modifiers::NONE);

    listed.set(false);
    discard.set(true);
    fixture.frame(Vec::new());
    discard.set(false);
    assert_eq!(fixture.passes(), 2, "the frame the tab leaves in runs two passes");
    let painted = |fixture: &Fixture| fixture.texts().iter().any(|text| text.text == RIBBON[TABLE]);
    // Checked only in frames that paint the fade: the fade is the Open motion, so the clicks below may outlast it.
    let still = |fixture: &Fixture| hidden.iter().all(|name| tab_rects(fixture.tree(), name).is_empty());
    assert!(painted(&fixture), "the departed tab fades at its slot after a frame of two passes");
    assert!(fixture.by_label(RIBBON[TABLE]).is_none(), "the departed tab has no node");
    assert!(still(&fixture), "no plain tab moves into the slot while the departed tab fades: {hidden:?}");

    fixture.key(Key::ArrowDown, Modifiers::ALT);
    assert!(hidden.iter().all(|name| fixture.by_label(name).is_some()), "the overflow menu is open and lists the plain tabs: {hidden:?}");
    assert!(fixture.by_label(RIBBON[TABLE]).is_none(), "the overflow menu does not list the departed tab");
    assert!(painted(&fixture), "the departed tab is still fading");
    fixture.key(Key::Escape, Modifiers::NONE);
    assert!(painted(&fixture) && still(&fixture), "no plain tab moves into the slot while the departed tab fades: {hidden:?}");

    picked.borrow_mut().clear();
    fixture.click_at(table.center());
    assert!(picked.borrow().is_empty(), "a click on the departed tab's slot selects nothing: {:?}", picked.borrow());
    fixture.idle(30);
    assert!(!painted(&fixture), "the departed tab is gone once the fade ends");
    assert!(hidden.iter().any(|name| !tab_rects(fixture.tree(), name).is_empty()), "the strip refits once the exit ends: {hidden:?}");

    listed.set(true);
    fixture.frame(Vec::new());
    let back = fixture.by_label(RIBBON[TABLE]).expect("a contextual tab that enters has a node once listed");
    fixture.click(&back);
    assert_eq!(shown.get(), TABLE, "a click on the entering tab shows it");
}
