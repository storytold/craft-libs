//! The tab strip's keys, drags and drops, and a tab's close button, which the tab declares as a part while it is a
//! member of the strip's run.

use craft_behavior::{SelectionChange, keyed};
use craft_ui::contract::{CollectionEvent, Role, ShowCollection as _};
use craft_ui::runtime::egui::access::AccessTree;
use craft_ui::tabs::{Tab, TabKind, TabStrip};
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
