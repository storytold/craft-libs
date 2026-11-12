//! List rows: the rename draft, the row menu and the trailing parts a row reveals.

use craft_behavior::{EditEvent, Mixed, keyed};
use craft_ui::contract::{CollectionEvent, Props as _, ShowCollection as _, request_rename};
use craft_ui::list::{List, ListRow, RowEvent};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const NAMES: [&str; 3] = ["Artboard 1", "Artboard 2", "Artboard 3"];

/// The list's id, which a rename request names.
fn list_id() -> Id {
    Id::NULL.with("rows")
}

/// A list of the three rows whose rename draft is `draft`, and what it reported.
fn rig(draft: fn(&usize) -> Option<String>) -> (Fixture, Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let list = List::new("Artboards").id(list_id()).rename(true).rename_draft(draft);
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    (fixture, seen)
}

/// The text the rename field of `row` opens with.
fn opened_with(draft: fn(&usize) -> Option<String>, row: usize) -> Option<String> {
    let (mut fixture, seen) = rig(draft);
    request_rename(fixture.ctx(), list_id(), &row);
    fixture.settle();
    seen.borrow().iter().find_map(|event| match event {
        CollectionEvent::Rename(EditEvent::Begin { before: Mixed::Same(rename), .. }) => Some(rename.name.clone()),
        _ => None,
    })
}

/// A rename draft replaces the row's text in the field it opens; without one the field holds the row's name.
#[trial]
fn a_rename_draft_opens_the_field_with_it_and_without_one_the_field_holds_the_name() {
    let empty_for_first: fn(&usize) -> Option<String> = |row| (*row == 0).then(String::new);
    assert_eq!(opened_with(empty_for_first, 0).as_deref(), Some(""), "the draft opens the field empty");
    assert_eq!(opened_with(empty_for_first, 1).as_deref(), Some(NAMES[1]), "a row without a draft opens with its name");
}

/// The shapes drawn wholly inside a small square around `at`: an icon's, not a row fill's.
fn drawn_at(shapes: &[egui::Shape], at: egui::Pos2) -> usize {
    let square = egui::Rect::from_center_size(at, Vec2::splat(24.0));
    shapes.iter().filter(|shape| square.contains_rect(shape.visual_bounding_rect())).count()
}

/// A list's row menu button shows on the selected row and on the hovered one, not on another; a click on it asks for
/// that row's menu.
#[trial]
fn the_row_menu_button_shows_on_the_hovered_and_selected_rows_and_asks_for_the_row_menu() {
    use craft_behavior::Selection;
    use craft_ui::contract::Bind;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let list = List::new("Artboards").selection(Bind::given(Selection::single(1))).row_menu();
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let button = |fixture: &Fixture, row: usize| fixture.by_label(&format!("More ({})", NAMES[row])).expect("each row has its menu button").center();
    let (first, second, third) = (button(&fixture, 0), button(&fixture, 1), button(&fixture, 2));
    let rest = fixture.frame_shapes();
    assert!(drawn_at(&rest, second) > 0, "the selected row shows its button");
    assert_eq!(drawn_at(&rest, first), 0, "a row neither hovered nor selected shows none");
    fixture.frame(vec![craft_ui::testing::pointer::moved(third)]);
    let hovered = fixture.frame_shapes();
    assert!(drawn_at(&hovered, third) > 0, "the hovered row shows its button");
    assert_eq!(drawn_at(&hovered, first), 0, "another row still shows none");
    seen.borrow_mut().clear();
    fixture.click_at(first).settle();
    assert!(
        seen.borrow().iter().any(|event| matches!(event, CollectionEvent::ContextMenu(request) if request.key == 0)),
        "a click on the button asks for its row's menu"
    );
}

/// A right-click on a row's thumbnail asks for the row's menu and names the thumbnail. With command clicks on, a
/// Cmd/Ctrl click on a row's label or on its thumbnail is a command on the row and leaves the selection alone.
#[trial]
fn pointer_commands_name_the_thumbnail_and_leave_the_selection() {
    use craft_ui::list::{ThumbSize, ThumbTarget};
    use craft_ui::testing::pointer;
    use egui::{Modifiers, Rect};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let thumbs = Rc::new(RefCell::new([Rect::NOTHING; 3]));
    let (held, painted) = (seen.clone(), thumbs.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = List::new("Channels").command_click(true).show_items(ui, &source, &mut |key, _| {
            let (at, painted) = (*key, painted.clone());
            ListRow::new(NAMES[*key]).thumb(ThumbSize::Medium, move |_, rect| painted.borrow_mut()[at] = rect)
        });
        held.borrow_mut().extend(out.events);
    });
    fixture.settle();
    let thumb = |at: usize| thumbs.borrow()[at].center();
    fixture.frame(vec![pointer::moved(thumb(1)), pointer::secondary(thumb(1), true)]);
    fixture.frame(vec![pointer::secondary(thumb(1), false)]).settle();
    let menu = seen.borrow().iter().find_map(|event| match event {
        CollectionEvent::ContextMenu(request) => Some((request.key, request.part.and_then(ThumbTarget::of_part))),
        _ => None,
    });
    assert_eq!(menu, Some((1, Some(ThumbTarget::First))), "the right-click on the thumbnail asks for the row's menu and names it");

    seen.borrow_mut().clear();
    let label = fixture.by_label(NAMES[0]).expect("the first row").rect.right_center() - Vec2::new(20.0, 0.0);
    fixture.hold(Modifiers::COMMAND).click_at(label).settle();
    fixture.click_at(thumb(2)).settle();
    fixture.hold(Modifiers::NONE);
    let commands: Vec<usize> =
        seen.borrow().iter().filter_map(|event| if let CollectionEvent::CommandClick { key, .. } = event { Some(*key) } else { None }).collect();
    assert_eq!(commands, [0, 2], "a Cmd/Ctrl click on the label and one on the thumbnail are commands on their rows");
    assert!(!seen.borrow().iter().any(|event| matches!(event, CollectionEvent::Selection(_))), "the selection is left alone");
}

/// A click on the selected event deselects it, so the details under the log close.
#[trial]
fn a_click_on_the_selected_event_deselects_it() {
    use craft_behavior::SelectionChange;
    use craft_ui::list::EventList;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_u64, 1, 2];
        let source = keyed(&keys, |key: &u64| *key);
        let out = EventList::new("Events").show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key as usize]));
        held.borrow_mut().extend(out.events);
    });
    fixture.settle();
    let row = fixture.by_label(NAMES[1]).expect("the second event");
    fixture.click(&row).settle();
    fixture.advance(1.0).idle(1);
    fixture.click(&row).settle();
    let changes: Vec<SelectionChange<u64>> =
        seen.borrow().iter().filter_map(|event| if let CollectionEvent::Selection(change) = event { Some(change.clone()) } else { None }).collect();
    assert!(
        matches!(changes.as_slice(), [SelectionChange::Set(keys), SelectionChange::Clear] if keys == &[1]),
        "selected, then deselected: {changes:?}"
    );
}

/// A list of the three rows built by `row`, in `room`, and what it reported.
fn rows_rig(room: Vec2, row: fn(usize) -> ListRow<'static>) -> (Fixture, Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let out = List::new("Artboards").show_items(ui, &source, &mut |key, _| row(*key));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(room);
    fixture.settle();
    (fixture, seen)
}

/// A hosted component's slot is as wide as it measures; in a narrow list an optional one drops out, and a field
/// narrows toward its least width so the row stays inside the list.
#[trial]
fn hosted_items_take_their_own_width_and_an_optional_one_drops_first() {
    use craft_ui::buttons::Button;
    use craft_ui::display::Badge;
    let row = |key: usize| {
        ListRow::new(NAMES[key])
            .trailing_field(Button::new("Apply to every page"), |_| {})
            .trailing_field(Button::new("OK"), |_| {})
            .trailing_field(craft_ui::text::HexField::hex("Color", craft_ui::contract::Bind::given(craft_theme::Rgba::rgb(0, 0, 0))), |_| {})
            .optional_field(Badge::new("Primary"), |_| {})
    };
    let (fixture, _) = rows_rig(Vec2::new(800.0, 200.0), row);
    let short = fixture.by_label("OK").expect("the short button shows");
    let long = fixture.by_label("Apply to every page").expect("the long button shows");
    assert!(long.rect.width() > short.rect.width(), "the long label's button is wider: {:?} {:?}", long.rect, short.rect);
    assert!(!long.rect.intersects(short.rect), "the slots do not overlap");
    assert!(fixture.by_label("Primary").is_some(), "the optional badge shows in a wide list");
    let wide_field = fixture.by_label("Color").expect("the field shows").rect;
    let (narrow, _) = rows_rig(Vec2::new(360.0, 200.0), row);
    assert!(narrow.by_label("Primary").is_none(), "the optional badge drops out in a narrow list");
    assert!(narrow.by_label("OK").is_some(), "the required button stays");
    let (field, row_rect) = (narrow.by_label("Color").expect("the field stays").rect, narrow.by_label(NAMES[0]).expect("the row shows").rect);
    assert!(field.width() < wide_field.width(), "the field narrows: {field:?} from {wide_field:?}");
    assert!(field.right() <= row_rect.right() + 0.5, "the row's items stay inside it: {field:?} in {row_rect:?}");
}

/// A double click on a row's leading text reports it with its count and does not activate the row; a click on a
/// trailing part reports whether its row held the lead when the press began.
#[trial]
fn the_leading_text_takes_its_double_click_and_a_part_click_tells_whether_its_row_was_the_lead() {
    use craft_ui::list::{TrailingFace, TrailingPart};
    use craft_ui::primitives::IconSpec;
    const NUMBERS: [&str; 3] = ["Options 1", "Options 2", "Options 3"];
    let row = |key: usize| {
        ListRow::new(NAMES[key]).lead_text((key + 1).to_string(), "000000").trailing_part(
            TrailingPart::new(0),
            TrailingFace::Icon(IconSpec::svg(craft_behavior::IconId::new("settings"))),
            NUMBERS[key],
        )
    };
    let (mut fixture, seen) = rows_rig(Vec2::new(320.0, 200.0), row);
    let first = fixture.by_label(NAMES[0]).expect("the row has a node").rect;
    let number = egui::Pos2::new(first.left() + 20.0, first.center().y);
    fixture.click_at(number).click_at(number).settle();
    {
        let seen = seen.borrow();
        assert!(seen.iter().any(|event| matches!(event, CollectionEvent::Item(0, RowEvent::LeadClicked { clicks: 2, .. }))), "{seen:?}");
        assert!(!seen.iter().any(|event| matches!(event, CollectionEvent::Activate(_))), "no activation: {seen:?}");
    }
    let was_lead = |seen: &Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>| {
        seen.borrow().iter().rev().find_map(|event| match event {
            CollectionEvent::Item(2, RowEvent::Trailing { was_lead, .. }) => Some(*was_lead),
            _ => None,
        })
    };
    fixture.advance(1.0).idle(1);
    let options = fixture.by_label(NUMBERS[2]).expect("the part has a node").center();
    fixture.click_at(options).settle();
    assert_eq!(was_lead(&seen), Some(false), "the first click lands on a row that was not the lead");
    fixture.advance(1.0).idle(1);
    fixture.click_at(options).settle();
    assert_eq!(was_lead(&seen), Some(true), "the press moved the lead there, so the next click finds it the lead");
}
