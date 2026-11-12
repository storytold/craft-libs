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

/// An empty event log says so with the collection's own empty text: one label, not a notice.
#[trial]
fn an_empty_event_log_says_so_in_the_collections_empty_text() {
    use craft_ui::contract::Role;
    use craft_ui::list::EventList;
    let mut fixture = Fixture::new(move |ui| {
        let keys: [u64; 0] = [];
        let source = keyed(&keys, |key: &u64| *key);
        let _ = EventList::new("Events").show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key as usize]));
    });
    fixture.settle();
    let empty = fixture.by_label("No events.").expect("the empty log says so");
    let list = fixture.by_role(Role::LIST).into_iter().next().expect("the list");
    assert!(empty.rect.left() < list.rect.center().x, "the text starts at the reading start");
    assert!(fixture.by_role(Role::STATUS).is_empty(), "no notice");
}

/// A single click on a finding activates it, so the app goes to what it is about.
#[trial]
fn a_click_on_a_finding_activates_it() {
    use craft_ui::contract::WordSetCx as _;
    use craft_ui::list::{Finding, Findings, FindingsList, FindingsWords, Severity};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let findings: Vec<Finding> = NAMES.iter().enumerate().map(|(at, name)| Finding::new(at as u64, Severity::Warning, *name)).collect();
        let source = Findings::new(&findings);
        let out = FindingsList::new("Findings").show_items(ui, &source, &mut |key, icx| findings[*key as usize].row(&icx.words::<FindingsWords>()));
        held.borrow_mut().extend(out.events);
    });
    fixture.settle();
    let row = fixture.by_label("Artboard 2, Warning").expect("the second finding");
    fixture.click(&row).settle();
    assert!(seen.borrow().iter().any(|event| matches!(event, CollectionEvent::Activate(1))), "one click activates the finding");
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

/// The components a row hosts under its label stand in one line inside the row, which grows to hold them, and a click on
/// one reaches its output.
#[trial]
fn components_below_the_label_stand_in_a_line_inside_a_taller_row_and_report_their_clicks() {
    use craft_ui::buttons::Button;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DELETES: AtomicUsize = AtomicUsize::new(0);
    let row = |key: usize| {
        let row = ListRow::new(NAMES[key]);
        if key != 1 {
            return row;
        }
        row.below(Button::new("Delete"), |out| {
            if craft_ui::contract::clicked(&out) {
                DELETES.fetch_add(1, Ordering::SeqCst);
            }
        })
        .below(Button::new("Cancel"), |_| {})
    };
    let (mut fixture, _) = rows_rig(Vec2::new(400.0, 300.0), row);
    let plain = fixture.by_label(NAMES[0]).expect("a plain row shows").rect;
    let asking = fixture.by_label(NAMES[1]).expect("the asking row shows").rect;
    let delete = fixture.by_label("Delete").expect("Delete shows");
    let cancel = fixture.by_label("Cancel").expect("Cancel shows").rect;
    assert!(asking.height() > plain.height(), "the row grows for the line under its text: {asking:?} beside {plain:?}");
    for button in [delete.rect, cancel] {
        assert!(asking.contains_rect(button), "the components stay inside their row: {button:?} in {asking:?}");
    }
    assert!(!delete.rect.intersects(cancel), "the components stand side by side: {:?} {cancel:?}", delete.rect);
    assert!((delete.rect.center().y - cancel.center().y).abs() < 0.5, "in one line");
    fixture.click(&delete).settle();
    assert_eq!(DELETES.load(Ordering::SeqCst), 1, "the click reaches the component's output");
}

/// A tree row narrows its hosted field and its indent together, so a deep row whose field fits at its least narrows
/// it instead of keeping it whole; a row too narrow for even its field's least keeps the field at its own width.
#[trial]
fn a_deep_rows_field_and_indent_narrow_together_and_a_row_too_narrow_for_its_least_keeps_the_field_whole() {
    let row = |key: usize| {
        ListRow::new(NAMES[key])
            .depth(8)
            .trailing_field(craft_ui::text::HexField::hex("Color", craft_ui::contract::Bind::given(craft_theme::Rgba::rgb(0, 0, 0))), |_| {})
    };
    let field_at = |width: f32| {
        let (fixture, _) = rows_rig(Vec2::new(width, 200.0), row);
        (fixture.by_label("Color").expect("the field shows").rect, fixture.by_label(NAMES[0]).expect("the row shows").rect)
    };
    let (wide, _) = field_at(800.0);
    // The full indent and the field's least do not fit 160 pt; the field's least does, once the indent gives way.
    let (deep, row_rect) = field_at(160.0);
    assert!(deep.width() < wide.width(), "the field narrows: {deep:?} from {wide:?}");
    assert!(deep.right() <= row_rect.right() + 0.5, "the field stays inside its row: {deep:?} in {row_rect:?}");
    let (short, _) = field_at(100.0);
    assert!((short.width() - wide.width()).abs() < 0.5, "a row too narrow for the field's least keeps it whole: {short:?} from {wide:?}");
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

/// A click on a row's "More" button opens the host's context menu at the button; the button stays shown while the
/// menu is open with the pointer away from the row; the menu's Rename opens that row's rename field.
#[trial]
fn the_row_menu_opens_the_host_menu_keeps_its_button_and_renames_the_row() {
    use craft_ui::contract::{Entry, Role};
    use craft_ui::menu::Menu;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let menu_row = Rc::new(RefCell::new(None::<usize>));
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let out = List::new("Artboards").rename(true).row_menu().show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        for event in &out.events {
            if let CollectionEvent::ContextMenu(request) = event {
                *menu_row.borrow_mut() = Some(request.key);
            }
        }
        let row = *menu_row.borrow();
        if let Some(row) = row {
            let entries = [Entry::new(String::from("rename"), "Rename"), Entry::new(String::from("delete"), "Delete")];
            let menu = Menu::context("Artboard", &out.response).pick(ui, &entries);
            if menu.picked.as_deref() == Some("rename") {
                request_rename(ui.ctx(), out.response.id, &row);
            }
            if menu.closed.is_some() {
                *menu_row.borrow_mut() = None;
            }
        }
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let button = fixture.by_label(&format!("More ({})", NAMES[0])).expect("the row has its menu button").center();
    fixture.click_at(button).settle();
    assert!(!fixture.by_role(Role::MENU).is_empty(), "the host's menu opens from the button");
    let away = fixture.by_label(NAMES[2]).expect("the last row shows").rect.center_bottom() + Vec2::new(0.0, 40.0);
    fixture.frame(vec![craft_ui::testing::pointer::moved(away)]);
    let shapes = fixture.frame_shapes();
    assert!(drawn_at(&shapes, button) > 0, "the button stays shown while its menu is open");
    let rename = fixture.by_label("Rename").expect("the menu has Rename");
    fixture.click(&rename).settle();
    assert!(
        seen.borrow()
            .iter()
            .any(|event| matches!(event, CollectionEvent::Rename(EditEvent::Begin { before: Mixed::Same(rename), .. }) if rename.key == 0)),
        "Rename opens the row's rename field: {:?}",
        seen.borrow()
    );
}
