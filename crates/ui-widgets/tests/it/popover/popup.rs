//! Popups: the dismissal rules, and the wheel and keys of an open popup list.

use crate::overlay::kit::{Leaf, Opened, Opener, Seen};
use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, CollectionEvent, Overlay, Props as _, Role, Scroll, Show as _, ShowCollection as _, ShowContainer as _, Trigger, clicked,
};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow};
use craft_ui::popover::{Popover, PopupList, PopupListKind};
use craft_ui::testing::pointer::{self, click, moved, press, release};
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const OPENER: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(180.0, 120.0) };
const BELOW: Rect = Rect { min: Pos2::new(300.0, 300.0), max: Pos2::new(360.0, 330.0) };

/// An opener of the preset `make` and a leaf below it at a fixed place; returns whether the popup body ran.
fn scene(frames: &mut Frames, events: Vec<egui::Event>, trigger: Trigger, make: fn() -> Overlay, open: bool, below: &RefCell<Seen>) -> bool {
    let opened = Opened::default();
    let content = RefCell::new(Seen::default());
    frames.frame(events, |ui: &mut Ui| {
        Opener::new(&opened, &content, trigger, make).open(open).id_salt("opener").show_at(ui, OPENER);
        Leaf::new(below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
    });
    opened.shown.get()
}

/// Opens the popup, then presses outside it on the leaf below; returns whether the popup still shows after the press
/// and what the leaf below saw.
fn press_outside(make: fn() -> Overlay) -> (bool, u32) {
    let mut frames = Frames::new();
    let below = RefCell::new(Seen::default());
    let frame = |frames: &mut Frames, events, open| scene(frames, events, Trigger::Manual, make, open, &below);
    for _ in 0..3 {
        frame(&mut frames, vec![], false);
    }
    frame(&mut frames, vec![], true);
    for _ in 0..30 {
        frame(&mut frames, vec![], false);
    }
    assert!(frame(&mut frames, vec![], false), "the popup is open before the press");
    let at = BELOW.center();
    frame(&mut frames, vec![moved(at)], false);
    frame(&mut frames, vec![press(at)], false);
    frame(&mut frames, vec![release(at)], false);
    let still = [frame(&mut frames, vec![], false), frame(&mut frames, vec![], false)];
    let presses = below.borrow().presses;
    (still[1], presses)
}

#[trial]
fn an_outside_press_closes_a_dropdown_and_reaches_the_widget_below() {
    let (open, presses) = press_outside(Overlay::dropdown);
    assert!(!open, "an outside press closes the popup");
    assert!(presses > 0, "the press reaches the widget below");
}

#[trial]
fn a_click_on_the_opener_closes_its_open_menu() {
    let mut frames = Frames::new();
    let below = RefCell::new(Seen::default());
    let opened = Opened::default();
    let content = RefCell::new(Seen::default());
    let mut show = |ui: &mut Ui| {
        opened.shown.set(false);
        Opener::new(&opened, &content, Trigger::Click, Overlay::menu).id_salt("opener").show_at(ui, OPENER);
        Leaf::new(&below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
    };
    frames.idle(3, &mut show);
    click(&mut frames, OPENER.center(), &mut show);
    frames.idle(30, &mut show);
    assert!(opened.shown.get(), "a click opens the menu");
    click(&mut frames, OPENER.center(), &mut show);
    frames.idle(30, &mut show);
    assert!(!opened.shown.get(), "the press on the opener reaches it, and its toggle closes the menu");
}

#[trial]
fn a_number_draft_in_a_popover_commits_when_an_outside_press_closes_it() {
    let (open, value) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(Some(10.0f64))));
    let (shown_open, shown_value) = (open.clone(), value.clone());
    let mut fixture = Fixture::new(move |ui| {
        let opener = craft_ui::buttons::Button::new("Stroke").show(ui);
        let mut is_open = shown_open.get();
        Popover::new("Stroke options", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, _| {
            let mut number = shown_value.get();
            NumberField::new("Weight", Bind::bound(&mut number)).show(ui);
            shown_value.set(number);
        });
        shown_open.set(is_open);
    });
    fixture.settle();
    let field = fixture.by_label("Weight").expect("the field shows in the popover");
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    // An expression applies on commit only, so it stays a draft until the popover closes.
    fixture.type_text("3+4");
    fixture.idle(2);
    assert_eq!(value.get(), Some(10.0), "the expression is a draft before the press");
    let dialog = fixture.by_role(Role::DIALOG).first().cloned().expect("the popover shows");
    fixture.click_at(dialog.rect.right_bottom() + Vec2::splat(40.0));
    fixture.idle(30);
    assert_eq!(value.get(), Some(7.0), "the draft commits when the outside press closes the popover");
    assert!(!open.get(), "the outside press closes the popover");
}

/// The rows of the popup list: more than its share of the window shows.
const ROWS: usize = 40;
/// The buttons below the opener: enough to make the page taller than the window.
const PAGE: usize = 40;

fn row(at: usize) -> String {
    format!("Row {at}")
}

struct Rows;

impl DisplayOrder<usize> for Rows {
    fn len(&self) -> usize {
        ROWS
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Owned(row(at)))
    }
}

impl ItemSource<usize> for Rows {}

/// A short options list fills its opener's width without padding making the popup wider than the opener.
#[trial]
fn a_popup_list_with_short_rows_fits_its_wide_opener() {
    let mut open = true;
    let anchor = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(240.0, 30.0));
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Open").show_at(ui, anchor).response;
        PopupList::new("Popup", Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
            List::<usize>::options("Choices").show_items(ui, &Rows, &mut |at, _| ListRow::new(row(*at)).option());
        });
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    let opener = fixture.by_label("Open").expect("the opener").rect;
    let popup = fixture.by_label("Popup").expect("the popup").rect;
    let row = fixture.by_label(&row(0)).expect("a row").rect;
    assert!(popup.width() >= opener.width() - 1.0, "the popup is at least as wide as its opener: {popup:?} {opener:?}");
    assert!(popup.width() <= opener.width() + 1.0, "padding does not widen the popup: {popup:?} {opener:?}");
    assert!(row.left() >= popup.left() && row.right() <= popup.right(), "the row stays inside the popup: {row:?} {popup:?}");
}

/// A document field's options start at its edge, so the selection fills the field's width.
#[trial]
fn a_document_popup_list_aligns_its_rows_with_the_opener() {
    let mut open = true;
    let anchor = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(240.0, 30.0));
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Open").show_at(ui, anchor).response;
        PopupList::new("Popup", Bind::bound(&mut open)).kind(PopupListKind::Document).opened_by(&opener).show(ui, |ui, _| {
            List::<usize>::options("Choices").show_items(ui, &Rows, &mut |at, _| ListRow::new(row(*at)).option());
        });
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    let opener = fixture.by_label("Open").expect("the opener").rect;
    let row = fixture.by_label(&row(0)).expect("a row").rect;
    assert!((row.left() - opener.left()).abs() <= 1.0 && (row.right() - opener.right()).abs() <= 1.0, "row {row:?}, opener {opener:?}");
}

/// A scrolling page taller than the window: an "Open" button that opens a popup list of `ROWS` rows while `open`, then
/// `PAGE` buttons. The rows the list activates go to `activated`.
fn page(open: Rc<Cell<bool>>, activated: Rc<RefCell<Vec<usize>>>) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            let opener = Button::new("Open").show(ui);
            if clicked(&opener) {
                open.set(true);
            }
            for at in 0..PAGE {
                Button::new(format!("Page {at}")).show(ui);
            }
            let mut shown = open.get();
            // The popup names its list, which takes the opener's list keys (`PopupList::list`).
            let list = opener.id.with("rows");
            PopupList::new("Rows", Bind::bound(&mut shown)).opened_by(&opener).list(list).show(ui, |ui, _| {
                let out = List::<usize>::options("Rows").id(list).show_items(ui, &Rows, &mut |at, _| ListRow::new(row(*at)).option());
                activated.borrow_mut().extend(out.events.iter().filter_map(|event| match event {
                    CollectionEvent::Activate(at) => Some(*at),
                    _ => None,
                }));
            });
            open.set(shown);
        });
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    fixture
}

/// The wheel over an open popup list scrolls the list and leaves the page below where it was; over the page, with the
/// list closed, the same wheel scrolls the page.
#[trial]
fn the_wheel_over_an_open_popup_list_scrolls_the_list_and_not_the_surface_below() {
    let open = Rc::new(Cell::new(true));
    let mut fixture = page(Rc::clone(&open), Rc::default());
    let opener = fixture.by_label("Open").expect("the opener shows").rect;
    let first = fixture.by_label(&row(0)).expect("the list shows its first row").rect;
    let over = fixture.by_label(&row(2)).expect("the list shows its third row").rect.center();
    fixture.frame(vec![pointer::moved(over)]);
    for _ in 0..3 {
        fixture.frame(vec![pointer::wheel(-60.0)]);
    }
    fixture.settle();
    assert!(open.get(), "the wheel leaves the list open");
    assert!(fixture.by_label(&row(0)).is_none_or(|shown| shown.rect.min.y < first.min.y - 1.0), "the wheel over the list scrolls it");
    assert_eq!(fixture.by_label("Open").map(|shown| shown.rect), Some(opener), "the page below the list stays where it was");

    open.set(false);
    fixture.settle();
    let on_page = fixture.by_label("Page 4").expect("the page shows its buttons").rect.center();
    eprintln!(
        "DIAG popover::popup::the_wheel_over_an_open_popup_list_scrolls_the_list_and_not_the_surface_below: after the close still animating {}",
        fixture.repaint_requested()
    );
    fixture.frame(vec![pointer::moved(on_page)]);
    fixture.frame(vec![pointer::wheel(-60.0)]);
    eprintln!(
        "DIAG popover::popup::the_wheel_over_an_open_popup_list_scrolls_the_list_and_not_the_surface_below: after the wheel Open {:?} (was {:?}) Page 4 {:?} (was {on_page:?})",
        fixture.by_label("Open").map(|shown| shown.rect.min.y),
        opener.min.y,
        fixture.by_label("Page 4").map(|shown| shown.rect.center())
    );
    fixture.settle();
    eprintln!(
        "DIAG popover::popup::the_wheel_over_an_open_popup_list_scrolls_the_list_and_not_the_surface_below: settled Open {:?} list open {}",
        fixture.by_label("Open").map(|shown| shown.rect.min.y),
        open.get()
    );
    assert!(fixture.by_label("Open").is_none_or(|shown| shown.rect.min.y < opener.min.y - 1.0), "the wheel over the page scrolls it");
}

/// A pointer resting on the list keeps no hold on the lead: End scrolls another row under it, and Enter picks the row
/// End went to. The list keys reach the list through its opener's key focus; a click gives a button none, so the
/// opener is reached by Tab.
#[trial]
fn a_resting_pointer_leaves_the_lead_where_the_keys_moved_it() {
    let open = Rc::new(Cell::new(false));
    let activated = Rc::new(RefCell::new(Vec::new()));
    let mut fixture = page(Rc::clone(&open), Rc::clone(&activated));
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    fixture.key(Key::Space, Modifiers::NONE).settle();
    assert!(open.get(), "Space on the focused opener opens the list");
    eprintln!(
        "DIAG popover::popup::a_resting_pointer_leaves_the_lead_where_the_keys_moved_it: after Space focused {:?} stop {:?} opener {:?}",
        fixture.focused(),
        fixture.focused_stop(),
        fixture.by_label("Open").map(|shown| shown.node)
    );
    let rest = fixture.by_label(&row(3)).expect("the list shows its fourth row").rect.center();
    fixture.frame(vec![pointer::moved(rest)]).settle();
    fixture.key(Key::End, Modifiers::NONE).settle();
    eprintln!(
        "DIAG popover::popup::a_resting_pointer_leaves_the_lead_where_the_keys_moved_it: after End focused {:?} stop {:?} last row shown {} first row shown {}",
        fixture.focused(),
        fixture.focused_stop(),
        fixture.by_label(&row(ROWS - 1)).is_some(),
        fixture.by_label(&row(0)).is_some()
    );
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    eprintln!(
        "DIAG popover::popup::a_resting_pointer_leaves_the_lead_where_the_keys_moved_it: after Enter open {} activated {:?}",
        open.get(),
        activated.borrow()
    );
    assert_eq!(*activated.borrow(), [ROWS - 1], "Enter picks the row End went to, not the one under the resting pointer");
}
