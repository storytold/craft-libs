//! The tab strip's keys, drags and drops, and a tab's close button, which the tab declares as a part while it is a
//! member of the strip's run.

use craft_behavior::{SelectionChange, keyed};
use craft_ui::contract::{CollectionEvent, Role, ShowCollection as _};
use craft_ui::runtime::egui::access::AccessTree;
use craft_ui::tabs::{DocumentTabLook, Tab, TabKind, TabRole, TabStrip};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TITLES: [&str; 3] = ["One", "Two", "Three"];

/// Frames a rest of the tooltip delay takes: every frame adds 1/60 s.
const REST_FRAMES: usize = (craft_ui::contract::tooltip::DELAY * 60.0) as usize + 2;

#[trial]
fn overflowing_document_tabs_remain_readable_and_list_every_document() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let mut reference = Fixture::new(|ui| {
        let tabs = [DocumentTab::new(0, "Document num")];
        let _ = DocumentTabs::new("Reference", &tabs).selected(0).show(ui);
    });
    reference.root_value(DocumentTabLook::Shrinking);
    reference.room(egui::vec2(600.0, 100.0)).settle();
    let minimum = reference.by_label("Document num").expect("reference tab").rect.width();
    let mut fixture = Fixture::new(|ui| {
        let tabs: Vec<_> = (0..12).map(|i| DocumentTab::new(i, format!("Document number {i}, scanned.pdf"))).collect();
        let _ = DocumentTabs::new("Documents", &tabs).selected(0).show(ui);
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.room(egui::vec2(600.0, 100.0)).settle();
    let shown: Vec<_> = (0..12).filter_map(|i| fixture.by_label(&format!("Document number {i}, scanned.pdf")).map(|node| node.rect)).collect();
    assert!(shown.len() > 1, "more than one tab remains visible: {shown:?}");
    assert!(
        shown.iter().all(|tab| tab.width() + 0.5 >= minimum),
        "tabs fit their first 12 characters and close button: {shown:?}, minimum {minimum}"
    );
    let more = fixture.by_label("More tabs").expect("overflow button").center();
    fixture.click_at(more).settle();
    for i in 0..12 {
        let name = format!("Document number {i}, scanned.pdf");
        let tree = fixture.tree();
        assert!(
            [Role::MENU_ITEM, Role::MENU_ITEM_CHECK_BOX, Role::MENU_ITEM_RADIO]
                .into_iter()
                .flat_map(|role| tree.with_role(role))
                .any(|key| tree.name_of(key) == Some(name.as_str())),
            "overflow menu lists {name}"
        );
    }
}

#[trial]
fn document_tabs_leave_space_between_neighbours() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let mut fixture = Fixture::new(|ui| {
        let tabs = [DocumentTab::new(1, "One"), DocumentTab::new(2, "Two")];
        let _ = DocumentTabs::new("Documents", &tabs).selected(1).show(ui);
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.settle();
    let first = fixture.by_label("One").expect("first tab").rect;
    let second = fixture.by_label("Two").expect("second tab").rect;
    assert!(first.right() < second.left(), "neighbouring document tabs have a visible gap: {first:?}, {second:?}");
}

#[trial]
fn document_tabs_shrink_before_overflow() {
    let fit = TabKind::Document.fit_mode_in(DocumentTabLook::Shrinking).fit(&[100.0, 100.0, 100.0], 0, &[], 240.0, 50.0, 20.0);
    assert!(fit.overflow.is_empty(), "tabs that can shrink into the strip remain visible");
    assert!(fit.shown.iter().all(|(_, width)| *width < 100.0));
}

#[trial]
fn document_tabs_with_trailing_control_fill_their_parent_row() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing};
    use craft_ui::layout_components::Row;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let mut fixture = Fixture::new(|ui| {
        let tabs: Vec<_> = (0..5).map(|i| DocumentTab::new(i, format!("Document number {i}, scanned.pdf"))).collect();
        Row::new().sizing(Sizing::FillWidth).show(ui, |ui, _| {
            Button::new("Home").show(ui);
            let _ = DocumentTabs::new("Documents", &tabs)
                .trailing(|ui| {
                    Button::new("Open").show(ui);
                })
                .show(ui);
            Button::new("Settings").show(ui);
        });
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.room(egui::vec2(1000.0, 100.0)).settle();
    let visible = (0..5).filter(|i| fixture.by_label(&format!("Document number {i}, scanned.pdf")).is_some()).count();
    assert!(visible > 1, "the strip uses the available width: {visible} tab visible");
}

#[trial]
fn document_tabs_reserve_space_for_the_trailing_control() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let trailing_rect = Rc::new(Cell::new(Rect::NOTHING));
    let seen = trailing_rect.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs: Vec<_> = (0..5).map(|i| DocumentTab::new(i, format!("Long document number {i}, scanned.pdf")).closable(true)).collect();
        let _ = DocumentTabs::new("Documents", &tabs)
            .selected(4)
            .trailing(|ui| {
                seen.set(Button::new("Open").show(ui).rect);
            })
            .show(ui);
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.room(egui::vec2(1000.0, 100.0)).settle();
    let open = fixture.by_label("Open").unwrap_or_else(|| panic!("trailing button: {:?}", trailing_rect.get())).rect;
    for i in 0..5 {
        let tab = fixture.by_label(&format!("Long document number {i}, scanned.pdf")).expect("each tab is visible").rect;
        assert!(tab.right() <= open.left() + 0.5, "tab {i} overlaps the trailing button: {tab:?}, {open:?}");
    }
}

/// Production's Open button follows the last tab; it does not wait at the far end of the row. With tabs that overflow
/// it is still the strip's last item (the reserve trials above).
#[trial]
fn the_trailing_control_follows_the_last_tab_when_they_fit() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let mut fixture = Fixture::new(|ui| {
        let tabs: Vec<_> = (0..2).map(|i| DocumentTab::new(i, format!("Doc {i}.pdf")).closable(true)).collect();
        let _ = DocumentTabs::new("Documents", &tabs)
            .selected(1)
            .trailing(|ui| {
                Button::new("Open").show(ui);
            })
            .show(ui);
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.room(egui::vec2(1000.0, 100.0)).settle();
    let open = fixture.by_label("Open").expect("trailing button").rect;
    let last = fixture.by_label("Doc 1.pdf").expect("last tab").rect;
    assert!(open.left() >= last.right() - 0.5, "the button is not over the tab: {open:?}, {last:?}");
    assert!(open.left() - last.right() < 80.0, "the button sits by the last tab, not at the row's end: {open:?}, {last:?}");
}

#[trial]
fn nested_document_tabs_reserve_space_for_the_trailing_control() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing};
    use craft_ui::layout_components::Row;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let mut fixture = Fixture::new(|ui| {
        let tabs: Vec<_> = (0..5).map(|i| DocumentTab::new(i, format!("Long document number {i}, scanned.pdf")).closable(true)).collect();
        Row::new().sizing(Sizing::FillWidth).show(ui, |ui, _| {
            Button::new("Home").show(ui);
            let _ = DocumentTabs::new("Documents", &tabs)
                .selected(4)
                .trailing(|ui| {
                    Button::new("Open").show(ui);
                })
                .show(ui);
            Button::new("Settings").show(ui);
        });
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.room(egui::vec2(1000.0, 100.0)).settle();
    let open = fixture.by_label("Open").expect("trailing button").rect;
    for i in 0..5 {
        let tab = fixture.by_label(&format!("Long document number {i}, scanned.pdf")).expect("each tab is visible").rect;
        assert!(tab.right() <= open.left() + 0.5, "tab {i} overlaps the trailing button: {tab:?}, {open:?}");
    }
}

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
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
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
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
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

/// A notch of wheel scroll that arrives in four quarter-notch passes shows one tab step, as a whole notch in one pass
/// does: a trackpad's small deltas add up instead of being dropped.
#[trial]
fn small_wheel_deltas_add_up_to_one_tab_step() {
    use craft_ui::testing::pointer;
    for parts in [4_u32, 1] {
        let (mut fixture, shown) = wheel_rig(true);
        let over = fixture.by_label("Two").expect("the tab's node").center();
        fixture.frame(vec![pointer::moved(over)]).idle(1);
        for _ in 0..parts {
            fixture.frame(vec![pointer::wheel_lines(-1.0 / parts as f32)]).idle(2);
        }
        assert_eq!(shown.get(), 1, "a notch in {parts} passes shows exactly the next tab");
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

/// A tab that shows the name alone gives its whole title as its tooltip, also when the strip cuts its name.
#[trial]
fn a_document_tab_shows_its_own_tooltip() {
    use craft_ui::contract::tooltip::DELAY;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    const WHOLE: &str = "Poster.vc @ 50% (CMYK/Preview)";
    for width in [640.0, 60.0] {
        let mut fixture = Fixture::new(|ui| {
            let tabs = [DocumentTab::new(0_usize, "Poster.vc").tooltip(WHOLE)];
            let _ = DocumentTabs::new("Documents", &tabs).show(ui);
        });
        fixture.room(egui::Vec2::new(width, 120.0)).settle();
        let tab = tab_rect(fixture.tree(), "Poster.vc");
        let cut = fixture.texts().iter().any(|run| run.elided);
        assert_eq!(cut, width < 100.0, "the strip cuts the name at {width} pt only");
        fixture.frame(vec![moved(tab.center())]).advance(DELAY).settle();
        let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
        assert!(texts.iter().any(|text| text == WHOLE), "the whole title shows over the tab at {width} pt: {texts:?}");
        assert!(!fixture.texts().iter().any(|run| run.text == "Poster.vc" && !run.elided && cut), "and not the name alone: {texts:?}");
    }
}

/// A modified document tab with a name part shows the `*` right after the name, inside its text, and takes the mark's
/// room once, as the same tab whose `*` follows the whole label does.
#[trial]
fn a_modified_tab_shows_its_mark_after_the_name_part() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    const NAME: &str = "Untitled-1";
    const LABEL: &str = "Untitled-1 @ 93.5 % (RGB/Preview)";
    let shown = |name_len: Option<usize>| {
        let mut fixture = Fixture::new(move |ui| {
            let mut tab = DocumentTab::new(0_usize, LABEL).modified(true);
            if let Some(len) = name_len {
                tab = tab.name_len(len);
            }
            let _ = DocumentTabs::new("Documents", &[tab]).show(ui);
        });
        fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
        let texts: Vec<(String, Rect)> = fixture.texts().iter().map(|run| (run.text.clone(), run.rect)).collect();
        (texts, tab_rects(fixture.tree(), &format!("{LABEL}, modified")).first().copied())
    };
    let (named, named_tab) = shown(Some(NAME.len()));
    let (whole, whole_tab) = shown(None);
    assert!(named.iter().any(|(text, _)| text == "Untitled-1* @ 93.5 % (RGB/Preview)"), "the mark follows the name: {named:?}");
    assert!(!named.iter().any(|(text, _)| text == "*"), "and is not drawn again after the label: {named:?}");
    let star = whole.iter().find(|(text, _)| text == "*").map(|(_, rect)| *rect).expect("without a name part the mark follows the label");
    assert!(whole.iter().any(|(text, _)| text == LABEL), "and the label is drawn whole: {whole:?}");
    let (named_tab, whole_tab) = (named_tab.expect("the named tab's node"), whole_tab.expect("the whole tab's node"));
    let apart = (named_tab.width() - whole_tab.width()).abs();
    assert!(apart < star.width() / 2.0, "the mark takes its room once: {named_tab:?} against {whole_tab:?}, the mark {star:?}");
}

/// A middle click on a document tab asks to close it, unless the tab cannot close; it does not show the tab.
#[trial]
fn a_middle_click_closes_a_closable_document_tab() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    use craft_ui::testing::pointer::middle;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two"), DocumentTab::new(2, "Combine").closable(false)];
        let out = DocumentTabs::new("Documents", &tabs).selected(0).show(ui);
        held.borrow_mut().extend(out.events);
    });
    fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
    let middle_click = |fixture: &mut Fixture, name: &str| {
        let at = fixture.by_label(name).expect("the tab's node").rect.center();
        fixture.frame(vec![moved(at), middle(at, true)]).frame(vec![middle(at, false)]).settle();
    };
    middle_click(&mut fixture, "Two");
    assert_eq!(*seen.borrow(), [DocumentTabsEvent::CloseRequested(1)], "the middle-clicked tab closes and is not shown");
    seen.borrow_mut().clear();
    middle_click(&mut fixture, "Combine");
    assert!(!seen.borrow().iter().any(|event| matches!(event, DocumentTabsEvent::CloseRequested(_))), "{:?}", seen.borrow());
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

/// The bounds of the close button of the tab named `name`.
fn close_rect(tree: &AccessTree, name: &str) -> Rect {
    let label = format!("Close {name}");
    tree.with_role(Role::BUTTON)
        .into_iter()
        .find(|key| tree.name_of(*key) == Some(label.as_str()))
        .and_then(|key| tree.bounds_of(key))
        .expect("the close button's node")
}

/// Files from the operating system, a drag that did not start in the strip, rest over `at` for the tooltip delay and
/// then leave the window.
fn rest_files_over(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(at)], &mut *show);
    frames.idle(REST_FRAMES, &mut *show);
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

/// A drag that did not start in the strip, resting over an unselected document tab, selects it after the rest delay and
/// not before; a drag of the strip's own tab (a reorder) selects nothing it crosses, and a panel strip selects nothing
/// under a drag.
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
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(two.center())], &mut show);
    assert!(!seen.borrow().contains(&DocumentTabsEvent::Selected(1)), "files select no tab in the pass they arrive in: {:?}", seen.borrow());
    frames.idle(REST_FRAMES / 2, &mut show);
    assert!(!seen.borrow().contains(&DocumentTabsEvent::Selected(1)), "files select no tab before the rest delay: {:?}", seen.borrow());
    frames.idle(REST_FRAMES, &mut show);
    assert!(seen.borrow().contains(&DocumentTabsEvent::Selected(1)), "files resting over a tab select it: {:?}", seen.borrow());
    frames.leave_files();
    frames.frame(vec![moved(two.center())], &mut show);

    seen.borrow_mut().clear();
    frames.frame(vec![moved(one.center())], &mut show);
    frames.frame(vec![press(one.center())], &mut show);
    for step in 1..=4 {
        frames.frame(vec![moved(one.center().lerp(three.center(), step as f32 / 4.0))], &mut show);
    }
    frames.idle(REST_FRAMES, &mut show);
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
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event {
                picked.borrow_mut().extend(keys.iter().copied());
            }
        }
    };
    frames.idle(2, &mut panel);
    let two = tab_rect(&frames.tree(), "Two");
    rest_files_over(&mut frames, two.center(), &mut panel);
    assert!(!picked.borrow().contains(&1), "a panel strip selects nothing under a drag: {:?}", picked.borrow());
}

/// Leaving a tab restarts the rest wait, and a rest over a tab's close button selects nothing. The strip scrolled under
/// a still pointer is not covered here.
#[trial]
fn a_drag_leaving_a_tab_restarts_the_wait_and_a_rest_on_the_close_button_selects_nothing() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    let seen = RefCell::new(Vec::new());
    let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two"), DocumentTab::new(2, "Three")];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let out = DocumentTabs::new("Documents", &tabs).selected(0).show(ui);
        seen.borrow_mut().extend(out.events);
    };
    frames.idle(2, &mut show);
    let tree = frames.tree();
    let (one, two, three) = (tab_rect(&tree, "One"), tab_rect(&tree, "Two"), tab_rect(&tree, "Three"));
    let close = close_rect(&tree, "Two");
    let selected = |key: usize| seen.borrow().contains(&DocumentTabsEvent::Selected(key));

    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(two.center())], &mut show);
    frames.idle(REST_FRAMES / 2, &mut show);
    frames.frame(vec![moved(three.center())], &mut show);
    frames.idle(REST_FRAMES / 2, &mut show);
    assert!(!selected(1) && !selected(2), "half a delay on each of two tabs selects neither: {:?}", seen.borrow());
    frames.idle(REST_FRAMES, &mut show);
    assert!(selected(2) && !selected(1), "the tab the pointer rests on selects, the one it left does not: {:?}", seen.borrow());

    frames.leave_files();
    frames.frame(vec![moved(one.center())], &mut show);
    seen.borrow_mut().clear();
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(close.center())], &mut show);
    frames.idle(REST_FRAMES * 2, &mut show);
    assert!(!selected(1), "a rest over the close button selects nothing: {:?}", seen.borrow());
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

    drop_files_at(&mut frames, Pos2::new((one.right() + two.left()) / 2.0, two.center().y), &mut show);
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

/// A pointer drag from `from` to `to` in a few steps, released at `to`.
fn drag(frames: &mut Frames, from: Pos2, to: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![moved(from)], &mut *show);
    frames.frame(vec![press(from)], &mut *show);
    for step in 1..=4 {
        frames.frame(vec![moved(from.lerp(to, step as f32 / 4.0))], &mut *show);
    }
    frames.frame(vec![release(to)], &mut *show);
    frames.idle(1, &mut *show);
}

/// A document tab that does not move keeps its place: a drag of it reorders nothing, a drag of another tab past it
/// reorders nothing, and a drag of another tab that leaves it where it is still reorders.
#[trial]
fn a_tab_that_does_not_move_keeps_its_place() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    let seen = RefCell::new(Vec::new());
    let tabs =
        [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two").movable(false), DocumentTab::new(2, "Three"), DocumentTab::new(3, "Four")];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let out = DocumentTabs::new("Documents", &tabs).selected(0).reorderable(true).show(ui);
        seen.borrow_mut().extend(out.events);
    };
    frames.idle(2, &mut show);
    let tree = frames.tree();
    let (one, two, three, four) = (tab_rect(&tree, "One"), tab_rect(&tree, "Two"), tab_rect(&tree, "Three"), tab_rect(&tree, "Four"));
    let reordered = |seen: &RefCell<Vec<DocumentTabsEvent<usize>>>| -> Vec<(Vec<usize>, Option<usize>)> {
        seen.borrow()
            .iter()
            .filter_map(|event| match event {
                DocumentTabsEvent::Reordered { documents, before, .. } => Some((documents.clone(), *before)),
                _ => None,
            })
            .collect()
    };

    drag(&mut frames, two.center(), Pos2::new(four.right() - 2.0, four.center().y), &mut show);
    assert!(reordered(&seen).is_empty(), "the fixed tab does not move: {:?}", seen.borrow());

    seen.borrow_mut().clear();
    drag(&mut frames, three.center(), Pos2::new(one.left() + 2.0, one.center().y), &mut show);
    assert!(reordered(&seen).is_empty(), "no tab passes the fixed one: {:?}", seen.borrow());

    seen.borrow_mut().clear();
    drag(&mut frames, four.center(), Pos2::new(three.left() + 2.0, three.center().y), &mut show);
    assert_eq!(reordered(&seen), vec![(vec![3_usize], Some(2_usize))], "a move that leaves the fixed tab in place reorders: {:?}", seen.borrow());
}

/// A stacking key moves a tab as a drag does: it does not pass a tab that does not move, and a move that leaves that
/// tab in place still reorders.
#[trial]
fn a_stacking_key_does_not_move_a_tab_past_a_fixed_one() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs =
            [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two").movable(false), DocumentTab::new(2, "Three"), DocumentTab::new(3, "Four")];
        let out = DocumentTabs::new("Documents", &tabs).selected(0).reorderable(true).show(ui);
        held.borrow_mut().extend(out.events);
    });
    fixture.room(egui::Vec2::new(640.0, 120.0)).settle();
    let reordered = |seen: &RefCell<Vec<DocumentTabsEvent<usize>>>| -> Vec<(Vec<usize>, Option<usize>)> {
        seen.borrow()
            .iter()
            .filter_map(|event| match event {
                DocumentTabsEvent::Reordered { documents, before, .. } => Some((documents.clone(), *before)),
                _ => None,
            })
            .collect()
    };

    let three = fixture.by_label("Three").expect("the tab's node").rect;
    fixture.click_at(three.center()).settle();
    seen.borrow_mut().clear();
    fixture.key(Key::OpenBracket, Modifiers::COMMAND).settle();
    assert!(reordered(&seen).is_empty(), "no tab passes the fixed one by key: {:?}", seen.borrow());

    let four = fixture.by_label("Four").expect("the tab's node").rect;
    fixture.click_at(four.center()).settle();
    seen.borrow_mut().clear();
    fixture.key(Key::OpenBracket, Modifiers::COMMAND).settle();
    assert_eq!(reordered(&seen), vec![(vec![3_usize], Some(2_usize))], "a key move that leaves the fixed tab in place reorders: {:?}", seen.borrow());
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
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
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

/// A secondary click on a document tab asks for the tab's menu without showing the tab.
#[trial]
fn a_secondary_click_on_a_document_tab_asks_for_its_menu_without_showing_it() {
    let shown = Rc::new(Cell::new(0_usize));
    let menus = Rc::new(Cell::new(0_usize));
    let (now, asked) = (shown.clone(), menus.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Documents").shown(now.get()).show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        for event in &out.response.events {
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                && let Some(key) = keys.last()
            {
                now.set(*key);
            }
        }
        if out.menu.is_some() {
            asked.set(asked.get() + 1);
        }
    });
    fixture.settle();
    let two = tab_rect(fixture.tree(), "Two").center();
    fixture.frame(vec![moved(two)]);
    fixture.frame(vec![secondary(two, true)]);
    fixture.frame(vec![secondary(two, false)]);
    fixture.settle();
    assert!(menus.get() > 0, "the secondary click asks for the tab's menu");
    assert_eq!(shown.get(), 0, "the secondary click does not show the tab");
}

/// A modified document tab marks itself with the suite's dot, not an asterisk after its name: no `*` is painted and the
/// tab is as wide as an unmodified one plus the dot's room.
#[trial]
fn a_modified_document_tab_shows_a_dot_and_no_asterisk() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let look = DocumentTabLook::Shrinking;
    let tab = |modified: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let tabs = [DocumentTab::new(0, "Report.pdf").modified(modified)];
            let _ = DocumentTabs::new("Docs", &tabs).selected(0).show(ui);
        });
        fixture.root_value(look);
        fixture.room(egui::vec2(600.0, 100.0)).settle();
        let stars = fixture.texts().iter().filter(|run| run.text.contains('*')).count();
        let width = fixture.by_role(Role::TAB).first().map_or(0.0, |tab| tab.rect.width());
        (stars, width)
    };
    let (plain, modified) = (tab(false), tab(true));
    assert_eq!(modified.0, 0, "no asterisk is painted");
    assert!(modified.1 > plain.1, "the dot takes room beside the name: {} vs {}", modified.1, plain.1);
}

/// Upstream's document tabs by default (no `DocumentTabLook` set): whole titles fitted with no shrink, neighbours
/// touching, and a `*` after a modified tab's name with no dot.
#[trial]
fn document_tabs_keep_whole_touching_titles_and_an_asterisk_by_default() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, FitMode};
    assert_eq!(TabKind::Document.fit_mode(), FitMode::Whole);
    assert_eq!(TabKind::Document.fit_mode_in(DocumentTabLook::default()), FitMode::Whole);
    let mut fixture = Fixture::new(|ui| {
        let tabs = [DocumentTab::new(1, "One"), DocumentTab::new(2, "Two").modified(true)];
        let _ = DocumentTabs::new("Documents", &tabs).selected(1).show(ui);
    });
    fixture.room(egui::vec2(600.0, 100.0)).settle();
    let first = fixture.by_label("One").expect("first tab").rect;
    let second = fixture.by_role(Role::TAB).get(1).map(|tab| tab.rect).expect("second tab");
    assert!((second.left() - first.right()).abs() < 0.5, "neighbouring document tabs touch: {first:?}, {second:?}");
    let stars = fixture.texts().iter().filter(|run| run.text.contains('*')).count();
    assert_eq!(stars, 1, "the modified tab shows its asterisk");
}

/// Production shows the held tab in the strip and no filename badge by the pointer: while a document tab is dragged, its
/// name is painted once.
#[trial]
fn a_dragged_document_tab_shows_no_badge() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let tabs = [DocumentTab::new(0_usize, "Quarterly"), DocumentTab::new(1, "Other")];
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let _ = DocumentTabs::new("Documents", &tabs).selected(0).reorderable(true).show(ui);
    };
    frames.idle(2, &mut show);
    let tree = frames.tree();
    let one = tab_rect(&tree, "Quarterly");
    let to = Pos2::new(one.center().x, one.center().y + 60.0);
    frames.frame(vec![moved(one.center())], &mut show);
    frames.frame(vec![press(one.center())], &mut show);
    for step in 1..=4 {
        frames.frame(vec![moved(one.center().lerp(to, step as f32 / 4.0))], &mut show);
    }
    frames.idle(2, &mut show);
    let shown = frames.texts().iter().filter(|text| text.text == "Quarterly").count();
    assert_eq!(shown, 1, "the name is painted once, by the held tab, with no badge beside the pointer");
}

/// Under `DocumentTabLook::Shrinking` (PdfCraft's tabs) a very long document name does not stretch its tab across the
/// strip: the tab stays near 22 characters wide and the next tab keeps its room.
#[trial]
fn a_very_long_document_name_is_cut_to_a_compact_tab() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let long = "a-very-long-synthetic-document-file-name-for-truncation-test-1234567890.pdf";
    let tabs = [DocumentTab::new(0_usize, long), DocumentTab::new(1, "Short")];
    let mut fixture = Fixture::new(move |ui| {
        let _ = DocumentTabs::new("Documents", &tabs).selected(0).show(ui);
    });
    fixture.root_value(DocumentTabLook::Shrinking);
    fixture.idle(6);
    let tab = fixture.by_label(long).expect("the long tab").rect;
    let short = fixture.by_label("Short").expect("the next tab").rect;
    assert!(tab.width() < 320.0, "the long name is cut short, got {}", tab.width());
    assert!(short.left() >= tab.right(), "the next tab follows it");
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
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
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
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
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

/// A document tab dragged from one strip onto another strip's tab, or onto a strip with no tabs at all (every document
/// in windows of its own), is dropped into that strip: the drop says which tab it landed on, none on an empty strip.
#[trial]
fn a_tab_dropped_on_another_strip_lands_there_even_with_no_tabs() {
    use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
    for others in [vec![DocumentTab::new(2_usize, "Three")], Vec::new()] {
        let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two")];
        let (seen, strip) = (RefCell::new(Vec::new()), Cell::new(Rect::NOTHING));
        let mut frames = Frames::new();
        let mut show = |ui: &mut Ui| {
            let _ = DocumentTabs::new("Main", &tabs).selected(0).reorderable(true).show(ui);
            ui.add_space(40.0);
            let out = DocumentTabs::new("Window", &others).reorderable(true).show(ui);
            strip.set(out.response.rect);
            seen.borrow_mut().extend(out.events);
        };
        frames.idle(2, &mut show);
        let tree = frames.tree();
        let from = tab_rect(&tree, "One").center();
        let to = if others.is_empty() { strip.get().center() } else { tab_rect(&tree, "Three").center() };
        frames.frame(vec![moved(from)], &mut show);
        frames.frame(vec![press(from)], &mut show);
        for step in 1..=6 {
            frames.frame(vec![moved(from.lerp(to, step as f32 / 6.0))], &mut show);
        }
        frames.frame(vec![release(to)], &mut show);
        frames.idle(1, &mut show);
        let over = seen.borrow().iter().find_map(|event| match event {
            DocumentTabsEvent::DroppedInto { documents, over, .. } if documents.as_slice() == [0] => Some(*over),
            _ => None,
        });
        let wanted = others.first().map(|tab| tab.id);
        assert_eq!(over, Some(wanted), "the tab dropped on {wanted:?} lands in the other strip: {:?}", seen.borrow());
    }
}
