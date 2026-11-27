//! List rows: the rename draft, the row menu and the trailing parts a row reveals.

use craft_behavior::{EditEvent, Mixed, keyed};
use craft_ui::contract::{CollectionEvent, Props as _, RowHeight, ShowCollection as _, request_rename};
use craft_ui::list::{List, ListRow, RowEvent};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The events a list rig records.
type Events = Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>;

const NAMES: [&str; 3] = ["Artboard 1", "Artboard 2", "Artboard 3"];

/// The list's id, which a rename request names.
fn list_id() -> Id {
    Id::NULL.with("rows")
}

/// A list of the three rows whose rename draft is `draft`, and what it reported.
fn rig(draft: fn(&usize) -> Option<String>) -> (Fixture, Events) {
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

#[trial]
fn list_row_declares_and_accepts_access_click() {
    use craft_ui::testing::access::Declared;
    use craft_ui::testing::focus::AccessActionKind;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = List::options("Artboards").show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]).option());
        held.borrow_mut().extend(out.events);
    });
    fixture.settle();
    let row = fixture.by_label(NAMES[1]).expect("the row");
    assert!(fixture.tree().declares_key(row.node, Declared::Click), "the row offers Click to automation");
    seen.borrow_mut().clear();
    fixture.access(AccessActionKind::Click, &row).settle();
    assert!(seen.borrow().iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "Click selects the row");
}

#[trial]
fn access_click_activates_a_single_click_list_row() {
    use craft_ui::contract::ActivateOn;
    use craft_ui::testing::focus::AccessActionKind;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key);
        let out = List::new("Rows").activate(ActivateOn::Click).show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.settle();
    let row = fixture.by_label(NAMES[1]).expect("the row");
    seen.borrow_mut().clear();
    fixture.access(AccessActionKind::Click, &row).settle();
    assert!(seen.borrow().iter().any(|event| matches!(event, CollectionEvent::Activate(1))));
}

/// The rects of the three rows of a list shown with `rows`.
fn row_rects(rows: RowHeight) -> Vec<egui::Rect> {
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        List::new("Artboards").row_height(rows).show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    NAMES.iter().map(|name| fixture.by_label(name).expect("the row's node").rect).collect()
}

/// Text rows are shorter than control-high rows, in the layout and in each row's own height.
#[trial]
fn text_rows_are_shorter_than_control_rows() {
    let (control, text) = (row_rects(RowHeight::Control), row_rects(RowHeight::Text));
    for (control, text) in control.iter().zip(&text) {
        assert!(text.height() < control.height(), "a text row is shorter: {text:?} against {control:?}");
    }
    assert!(text[2].top() - text[0].top() < control[2].top() - control[0].top(), "text rows follow each other closer");
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

/// The rename field caps no name length: a name typed past the text field's usual 1,024 characters is kept whole, and
/// Enter commits all of it.
#[trial]
fn the_rename_field_accepts_a_name_longer_than_a_thousand_characters() {
    const LONG: usize = 1_500;
    let (mut fixture, seen) = rig(|_| None);
    request_rename(fixture.ctx(), list_id(), &0_usize);
    fixture.settle();
    fixture.type_text(&"x".repeat(LONG));
    fixture.settle();
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE);
    fixture.settle();
    let committed: Vec<usize> = seen
        .borrow()
        .iter()
        .filter_map(|event| match event {
            CollectionEvent::Rename(EditEvent::Commit { after, .. }) => Some(after.name.chars().count()),
            _ => None,
        })
        .collect();
    assert_eq!(committed, [LONG], "the whole name was committed, none of it cut");
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
    // A button is exposed only while its row shows it (L-252): the hidden ones are found from the shown one, a row apart.
    let button = |fixture: &Fixture, row: usize| fixture.by_label(&format!("More ({})", NAMES[row])).map(|node| node.center());
    let row = |fixture: &Fixture, row: usize| fixture.by_label(NAMES[row]).expect("each row shows").rect.center();
    let second = button(&fixture, 1).expect("the selected row exposes its menu button");
    let (first, third) = (second + (row(&fixture, 0) - row(&fixture, 1)), second + (row(&fixture, 2) - row(&fixture, 1)));
    assert!(button(&fixture, 0).is_none(), "a row neither hovered nor selected exposes no button");
    let rest = fixture.frame_shapes();
    assert!(drawn_at(&rest, second) > 0, "the selected row shows its button");
    assert_eq!(drawn_at(&rest, first), 0, "a row neither hovered nor selected shows none");
    fixture.frame(vec![craft_ui::testing::pointer::moved(third)]);
    assert!(button(&fixture, 2).is_some(), "the hovered row exposes its button");
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
    assert!(!seen.borrow().iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "the selection is left alone");
}

/// A thumbnails-only row is its thumbnails: no label band, so it is narrower than a plain row, it paints no label, and
/// the label is still its accessible name.
#[trial]
fn a_thumbnails_only_row_is_as_wide_as_its_thumbnails_and_named_by_its_label() {
    use craft_ui::list::ThumbSize;
    use egui::Rect;
    use std::cell::Cell;
    let (only, plain) = (Rc::new(Cell::new(Rect::NOTHING)), Rc::new(Cell::new(Rect::NOTHING)));
    let (only_in, plain_in) = (only.clone(), plain.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Masks").show_items(ui, &source, &mut |key, _| {
            let at = *key;
            let (only, plain) = (only_in.clone(), plain_in.clone());
            let row = ListRow::new(NAMES[at]).thumb(ThumbSize::Medium, move |_, rect| if at == 0 { only.set(rect) } else { plain.set(rect) });
            if at == 0 { row.thumbnails_only() } else { row }
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let (row, other) =
        (fixture.by_label(NAMES[0]).expect("the row is named by its label").rect, fixture.by_label(NAMES[1]).expect("the plain row").rect);
    let picture = only.get();
    assert!(row.contains_rect(picture), "the row holds its thumbnail: {row:?} {picture:?}");
    assert!(row.width() < other.width(), "the row is no wider than its thumbnails: {} vs {}", row.width(), other.width());
    assert!(
        row.width() < 2.0 * picture.width() && row.height() < 2.0 * picture.height(),
        "the row is as large as its thumbnail: {row:?} {picture:?}"
    );
    let texts = fixture.texts();
    assert!(!texts.iter().any(|text| text.text == NAMES[0]), "the row paints no label");
    assert!(texts.iter().any(|text| text.text == NAMES[1]), "the plain row paints its label");
}

/// A thumbnails-only row keeps its width in the frames that place it without measuring it, and a press past its
/// thumbnails is a press on no row: only a press on the row picks it.
#[trial]
fn a_press_past_a_thumbnails_only_row_picks_nothing() {
    use craft_ui::list::ThumbSize;
    let seen: Events = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key);
        let out = List::new("Masks").show_items(ui, &source, &mut |key, _| {
            let row = ListRow::new(NAMES[*key]).thumb(ThumbSize::Medium, |_, _| {});
            if *key == 0 { row.thumbnails_only() } else { row }
        });
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0)).settle();
    fixture.idle(3);
    let (row, other) = (fixture.by_label(NAMES[0]).expect("the row").rect, fixture.by_label(NAMES[1]).expect("the plain row").rect);
    assert!(row.width() < other.width(), "the row stays as wide as its thumbnails: {row:?} {other:?}");
    let picks = |seen: &Events| seen.borrow().iter().filter(|event| matches!(event, CollectionEvent::Selection { .. })).count();
    seen.borrow_mut().clear();
    fixture.click_at(egui::pos2(row.right() + 20.0, row.center().y)).settle();
    assert_eq!(picks(&seen), 0, "a press past the thumbnails picks no row: {:?}", seen.borrow());
    fixture.click_at(row.center()).settle();
    assert!(picks(&seen) > 0, "a press on the row picks it: {:?}", seen.borrow());
}

/// A drag on a thumbnail that carries a part drag starts with the primary button only: a secondary and a middle drag
/// start no session and report no `PartDrag`.
#[trial]
fn a_part_drag_starts_only_with_the_primary_button() {
    use craft_ui::list::{RowDragSpot, ThumbSize, ThumbTarget};
    use craft_ui::testing::pointer;
    use egui::{Event, Pos2, Rect};
    use std::cell::Cell;
    let seen: Events = Rc::new(RefCell::new(Vec::new()));
    let picture = Rc::new(Cell::new(Rect::NOTHING));
    let (held, painted) = (seen.clone(), picture.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let out = List::new("Layers").show_items(ui, &source, &mut |key, _| {
            let painted = painted.clone();
            ListRow::new(NAMES[*key])
                .thumb(ThumbSize::Medium, move |_, rect| painted.set(rect))
                .part_drag(RowDragSpot::Thumb(ThumbTarget::First), 7_u32)
        });
        held.borrow_mut().extend(out.events);
    });
    fixture.settle();
    let from = picture.get().center();
    let primary: fn(Pos2, bool) -> Event = |at, down| if down { pointer::press(at) } else { pointer::release(at) };
    let buttons: [fn(Pos2, bool) -> Event; 3] = [pointer::secondary, pointer::middle, primary];
    let mut started = Vec::new();
    for button in buttons {
        seen.borrow_mut().clear();
        fixture.frame(vec![pointer::moved(from)]);
        fixture.frame(vec![button(from, true)]);
        for step in 1..=4 {
            fixture.frame(vec![pointer::moved(from + Vec2::new(10.0 * step as f32, 0.0))]);
        }
        fixture.frame(vec![button(from + Vec2::new(40.0, 0.0), false)]);
        fixture.settle();
        started.push(seen.borrow().iter().filter(|event| matches!(event, CollectionEvent::Item(_, RowEvent::PartDrag(_)))).count());
    }
    assert_eq!(started[..2], [0, 0], "a secondary and a middle drag start no part drag");
    assert!(started[2] > 0, "the primary drag starts it");
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
    let changes: Vec<SelectionChange<u64>> = seen
        .borrow()
        .iter()
        .filter_map(|event| if let CollectionEvent::Selection { change, .. } = event { Some(change.clone()) } else { None })
        .collect();
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

/// A findings list given `max_rows` is as tall as that many rows, not the room it is offered, and scrolls the rest.
#[trial]
fn a_findings_list_with_max_rows_is_as_tall_as_those_rows() {
    use craft_ui::contract::WordSetCx as _;
    use craft_ui::list::{Finding, Findings, FindingsList, FindingsWords, Severity};
    let mut fixture = Fixture::new(move |ui| {
        let findings: Vec<Finding> = (0..20).map(|at| Finding::new(at, Severity::Warning, format!("Finding {at}"))).collect();
        let source = Findings::new(&findings);
        FindingsList::new("Findings").max_rows(3).show_items(ui, &source, &mut |key, icx| findings[*key as usize].row(&icx.words::<FindingsWords>()));
    });
    let room = Vec2::new(400.0, 500.0);
    fixture.room(room).settle();
    let list = fixture.by_label("Findings").expect("the list").rect;
    let row = fixture.by_label("Finding 0, Warning").expect("the first finding").rect;
    assert!(list.height() >= 3.0 * row.height() - 0.5, "the list shows three rows: {list:?}, a row {row:?}");
    assert!(list.height() < 5.0 * row.height(), "the list is not as tall as the room: {list:?}, a row {row:?}");
}

/// A list of the three rows built by `row`, in `room`, and what it reported.
fn rows_rig(room: Vec2, row: fn(usize) -> ListRow<'static>) -> (Fixture, Events) {
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

/// A leaf that measures far taller than any row, to show the height a row gives its hosted fields.
#[derive(craft_ui::contract::Component)]
struct Tall {
    #[common]
    common: craft_ui::contract::Common,
}

impl craft_ui::contract::Component for Tall {
    const ROLE: craft_ui::contract::Role = craft_ui::contract::Role::IMAGE;
    const SHOWS_LABEL: bool = false;
    type Style = super::label::kit::PlainStyle;
    type Output = egui::Response;

    fn style(_: &craft_ui::style::StyleCx<'_>, _: TallStyleKey) -> super::label::kit::PlainStyle {
        super::label::kit::PlainStyle::default()
    }

    fn measure(&mut self, _: &craft_ui::contract::MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(40.0, 1000.0)
    }

    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::empty()
    }

    fn finish(self, cx: craft_ui::contract::Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// A hosted field, required or optional, takes its own height up to its row's, as a cell field does: in a row taller
/// than the base row (a large thumbnail) it gets the whole row, centred, not one base row.
#[trial]
fn hosted_fields_take_their_own_height_up_to_the_row() {
    use craft_ui::list::ThumbSize;
    let tall = |key: usize| {
        ListRow::new(NAMES[key])
            .thumb(ThumbSize::Large, |_, _| {})
            .trailing_field(Tall { common: craft_ui::contract::Common::new() }.label(format!("Field {key}")), |_| {})
            .optional_field(Tall { common: craft_ui::contract::Common::new() }.label(format!("Optional {key}")), |_| {})
    };
    let (fixture, _) = rows_rig(Vec2::new(800.0, 600.0), tall);
    let (plain, _) = rows_rig(Vec2::new(800.0, 600.0), |key| ListRow::new(NAMES[key]));
    let base = plain.by_label(NAMES[0]).expect("the plain row").rect.height();
    let row = fixture.by_label(NAMES[0]).expect("the tall row").rect;
    assert!(row.height() > base, "the thumbnail row is taller than the base row: {row:?}, base {base}");
    for name in ["Field 0", "Optional 0"] {
        let field = fixture.by_label(name).unwrap_or_else(|| panic!("{name} shows")).rect;
        assert!((field.height() - row.height()).abs() < 0.5, "{name} takes the row's height: {field:?} in {row:?}");
        assert!(field.top() >= row.top() - 0.5 && field.bottom() <= row.bottom() + 0.5, "{name} stays in its row");
        assert!((field.center().y - row.center().y).abs() < 0.5, "{name} is centred on its row");
    }
}

/// A list sized to its content makes room for each row's optional item and shows its label whole, whether the label
/// is shorter or longer than its least width.
#[trial]
fn a_content_sized_list_makes_room_for_its_rows_optional_items() {
    use craft_ui::contract::Sizing;
    use std::cell::Cell;
    const ROWS: [(&str, &str); 2] = [("Levels", "Image > Adjustments"), ("Gaussian Blur and Sharpen More", "Filter > Blur")];
    let shown = Rc::new(Cell::new(egui::Rect::NOTHING));
    let held = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(ROWS[*key].0));
        let out = List::new("Commands")
            .sizing(Sizing::Content)
            .show_items(ui, &source, &mut |key, _| ListRow::new(ROWS[*key].0).trailing("Ctrl+L").optional(ROWS[*key].1));
        held.set(out.response.rect);
    });
    fixture.room(Vec2::new(800.0, 300.0));
    fixture.settle();
    let list = shown.get();
    assert!(list.width() < 800.0, "the list takes its width from its rows: {list:?}");
    let whole = |text: &str| fixture.texts().iter().find(|run| run.text == text && list.contains_rect(run.rect)).map(|run| run.elided);
    for (label, path) in ROWS {
        assert_eq!(whole(label), Some(false), "{label} shows whole");
        assert_eq!(whole(path), Some(false), "the path of {label} shows whole");
    }
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

/// A click on a row's "More" button opens the host's context menu at the button, and the list reports the row's
/// request in every pass the menu stays open, with no event in those passes: the button stays shown with the pointer
/// away from the row, and the menu's Rename, picked passes later, opens that row's rename field. Once the menu closes
/// the list reports no request.
#[trial]
fn the_row_menu_reads_its_row_from_the_list_while_open_and_renames_it() {
    use craft_ui::contract::{Entry, Role};
    use craft_ui::menu::Menu;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let reported = Rc::new(RefCell::new(None::<usize>));
    let (held, menu_of) = (seen.clone(), reported.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let out = List::new("Artboards").rename(true).row_menu().show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        *menu_of.borrow_mut() = out.menu.as_ref().map(|request| request.key);
        if let Some(row) = out.menu.as_ref().map(|request| request.key) {
            let entries = [Entry::new(String::from("rename"), "Rename"), Entry::new(String::from("delete"), "Delete")];
            let menu = Menu::context("Artboard", &out.response).pick(ui, &entries);
            if menu.picked.as_deref() == Some("rename") {
                request_rename(ui.ctx(), out.response.id, &row);
            }
        }
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    // The button shows, and is exposed, on the hovered row.
    let over = fixture.by_label(NAMES[0]).expect("the first row shows").rect.center();
    fixture.frame(vec![craft_ui::testing::pointer::moved(over)]).settle();
    let button = fixture.by_label(&format!("More ({})", NAMES[0])).expect("the hovered row has its menu button").center();
    fixture.click_at(button).settle();
    assert!(!fixture.by_role(Role::MENU).is_empty(), "the host's menu opens from the button");
    seen.borrow_mut().clear();
    let away = fixture.by_label(NAMES[2]).expect("the last row shows").rect.center_bottom() + Vec2::new(0.0, 40.0);
    fixture.frame(vec![craft_ui::testing::pointer::moved(away)]);
    let shapes = fixture.frame_shapes();
    assert!(drawn_at(&shapes, button) > 0, "the button stays shown while its menu is open");
    assert!(!seen.borrow().iter().any(|event| matches!(event, CollectionEvent::ContextMenu(_))), "no new request in the later pass");
    assert_eq!(*reported.borrow(), Some(0), "the list still reports the row's request");
    let rename = fixture.by_label("Rename").expect("the menu has Rename");
    fixture.click(&rename).settle();
    assert!(
        seen.borrow()
            .iter()
            .any(|event| matches!(event, CollectionEvent::Rename(EditEvent::Begin { before: Mixed::Same(rename), .. }) if rename.key == 0)),
        "Rename opens the row's rename field: {:?}",
        seen.borrow()
    );
    assert!(fixture.by_role(Role::MENU).is_empty(), "the menu closed");
    assert_eq!(*reported.borrow(), None, "a closed menu has no request");
}

/// A cell field shows in its column's cell in place of the cell's text, and in the label's column in the label's place
/// with the label still the row's name; a click on it takes the focus and its edit reaches the output.
#[trial]
fn a_cell_field_stands_in_its_cell_takes_the_click_and_reports_its_edit() {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::contract::{Bind, ColumnSpec};
    use craft_ui::table::Table;
    use craft_ui::text::{NumberField, TextField};
    use egui::{Key, Modifiers, Rect};
    const NAME: ColumnKey = ColumnKey::new("name");
    const SIZE: ColumnKey = ColumnKey::new("size");
    let typed = Rc::new(RefCell::new(None::<f64>));
    let size_cell = Rc::new(RefCell::new(Rect::NOTHING));
    let (held, cell) = (typed.clone(), size_cell.clone());
    let mut fixture = Fixture::new(move |ui| {
        let columns = vec![ColumnSpec::new(NAME, "Name", ColumnWidth::fill(1.0)), ColumnSpec::new(SIZE, "Size", ColumnWidth::fill(1.0))];
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let held = held.clone();
        let cell = cell.clone();
        Table::<usize>::new("Stops", columns).show_items(ui, &source, &mut |_, icx| {
            *cell.borrow_mut() = icx.cell(&SIZE).unwrap_or(Rect::NOTHING);
            let held = held.clone();
            ListRow::new("Row label")
                .cells(icx.cells())
                .cell(SIZE, "Size text")
                .cell_field(NAME, TextField::new("Name field", Bind::given("draft".to_owned())), |_| {})
                .cell_field(SIZE, NumberField::new("Size field", Bind::given(Some(4.0))), move |out| {
                    if let Some(value) = out.edit.events.landed() {
                        *held.borrow_mut() = *value;
                    }
                })
        });
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    let painted = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);
    assert!(!painted(&fixture, "Size text"), "the cell paints no text of its own");
    assert!(!painted(&fixture, "Row label"), "the field in the label's column stands in the label's place");
    assert!(fixture.by_label("Row label").is_some(), "the label stays the row's name");
    let field = fixture.by_label("Size field").expect("the size field shows");
    let cell = *size_cell.borrow();
    assert!(cell.expand(0.5).contains_rect(field.rect), "the field lies in its cell: {:?} in {cell:?}", field.rect);
    fixture.click(&field).settle();
    assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), field.node, "a click on the cell reaches the field");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("12").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*typed.borrow(), Some(12.0), "the field's edit reaches the output");
}

/// A screen reader's click on an item of a list that activates on a click picks and activates it, as a pointer click
/// does; one on a list that activates on a double click only picks it.
#[trial]
fn an_access_click_activates_an_item_where_a_click_does() {
    use craft_ui::contract::ActivateOn;
    use craft_ui::testing::focus::AccessActionKind;
    for (activate, activates) in [(ActivateOn::Click, true), (ActivateOn::DoubleClick, false)] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let held = events.clone();
        let mut fixture = Fixture::new(move |ui| {
            let keys = [0_usize, 1, 2];
            let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
            let out = List::new("Swatches").activate(activate).show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
            held.borrow_mut().extend(out.events);
        });
        fixture.room(Vec2::new(300.0, 200.0)).settle();
        events.borrow_mut().clear();
        let row = fixture.by_label(NAMES[1]).expect("the second row");
        fixture.access(AccessActionKind::Click, &row).settle();
        let events = events.borrow();
        assert!(events.iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "{activate:?}: the click picks: {events:?}");
        let activated = events.iter().any(|event| matches!(event, CollectionEvent::Activate(1)));
        assert_eq!(activated, activates, "{activate:?}: {events:?}");
    }
}

/// A link in the label's column is only as wide as its text, as a label is: a press on it follows the link, and a
/// press past it acts on the row (selects it, and would start its drag), as production's rows named by a link did.
#[trial]
fn a_link_in_a_cell_is_as_wide_as_its_text_and_a_press_past_it_acts_on_the_row() {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::buttons::Link;
    use craft_ui::contract::ColumnSpec;
    use craft_ui::table::Table;
    use egui::Rect;
    use std::cell::Cell;
    const NAME: ColumnKey = ColumnKey::new("name");
    let followed = Rc::new(Cell::new(0));
    let selections = Rc::new(Cell::new(0));
    let name_cell = Rc::new(RefCell::new(Rect::NOTHING));
    let (clicks, selected, cell) = (followed.clone(), selections.clone(), name_cell.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let (clicks, cell) = (clicks.clone(), cell.clone());
        let out =
            Table::<usize>::new("Appearance", vec![ColumnSpec::new(NAME, "Name", ColumnWidth::fill(1.0))]).show_items(ui, &source, &mut |_, icx| {
                *cell.borrow_mut() = icx.cell(&NAME).unwrap_or(Rect::NOTHING);
                let clicks = clicks.clone();
                ListRow::new("Fill").cells(icx.cells()).cell_field(NAME, Link::new("Fill:"), move |out| {
                    if out.clicked() {
                        clicks.set(clicks.get() + 1);
                    }
                })
            });
        let changes = out.events.iter().filter(|event| matches!(event, CollectionEvent::Selection { .. })).count();
        selected.set(selected.get() + changes);
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    let link = fixture.by_label("Fill:").expect("the link shows");
    let cell = *name_cell.borrow();
    assert!(link.rect.width() < cell.width() / 2.0, "the link is as wide as its text: {:?} in {cell:?}", link.rect);
    let past = egui::Pos2::new((link.rect.right() + cell.right()) / 2.0, link.rect.center().y);
    fixture.click_at(past).settle();
    assert_eq!(followed.get(), 0, "a press past the text is not the link's");
    assert!(selections.get() > 0, "a press past the text acts on the row");
    fixture.click(&link).settle();
    assert_eq!(followed.get(), 1, "a press on the text follows the link");
}

/// A row's label band takes the toggle columns it leaves empty next to it, up to the nearest toggle it fills: a row
/// without toggles starts its label at the row's start, a row with the first toggle only starts it inside the second
/// column, and a row with both starts it past them. A row without toggles that keeps its label in the label column
/// starts it past them too.
#[trial]
fn a_rows_label_takes_the_empty_toggle_columns_next_to_it() {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::contract::{ColumnSpec, ToggleColumn};
    use craft_ui::list::{ToggleCell, ToggleState};
    use std::cell::Cell;
    const INCLUDE: ColumnKey = ColumnKey::new("include");
    const DIALOG: ColumnKey = ColumnKey::new("dialog");
    const NAME: ColumnKey = ColumnKey::new("name");
    const LABELS: [&str; 4] = ["Set", "Action", "Step", "Tip"];
    let ends = Rc::new(Cell::new((0.0_f32, 0.0_f32)));
    let held = ends.clone();
    let mut fixture = Fixture::new(move |ui| {
        let toggle = |key| ColumnSpec::toggle(key, "", ToggleColumn::default());
        let columns = vec![toggle(INCLUDE), toggle(DIALOG), ColumnSpec::new(NAME, "", ColumnWidth::fill(1.0))];
        let keys = [0_usize, 1, 2, 3];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Actions").columns(columns).show_items(ui, &source, &mut |key, icx| {
            let end = |column| icx.cell(&column).map_or(0.0, |cell| cell.right());
            held.set((end(INCLUDE), end(DIALOG)));
            let check = |column| ToggleCell::check(column, ToggleState::On, "Include {row}");
            let row = ListRow::new(LABELS[*key]).cells(icx.cells());
            match key {
                0 => row,
                1 => row.toggle(check(INCLUDE)),
                2 => row.toggle(check(INCLUDE)).toggle(check(DIALOG)),
                _ => row.in_label_column(),
            }
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let start = |label: &str| {
        shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos.x),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label} paints"))
    };
    let (include, dialog) = ends.get();
    assert!(start("Set") < include, "a row without toggles starts its label in the first toggle column");
    assert!(start("Action") > include && start("Action") < dialog, "a row with the first toggle starts its label in the second column");
    assert!(start("Step") > dialog, "a row with both toggles starts its label past them");
    assert!(start("Tip") > dialog, "a row that keeps its label in the label column leaves the toggle columns empty");
}

/// In a list of toggle columns only, the label takes the room past the last column: it shows whole, as measure counts
/// it, and starts past its toggle.
#[trial]
fn a_label_in_a_list_of_toggle_columns_only_shows_whole_past_them() {
    use craft_behavior::ColumnKey;
    use craft_ui::contract::{ColumnSpec, ToggleColumn};
    use craft_ui::list::{ToggleCell, ToggleState};
    use std::cell::Cell;
    const CHECK: ColumnKey = ColumnKey::new("check");
    const LABEL: &str = "Order date and shipping region";
    let end = Rc::new(Cell::new(0.0_f32));
    let held = end.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let columns = vec![ColumnSpec::toggle(CHECK, "", ToggleColumn::default())];
        let _ = List::new("Fields").columns(columns).show_items(ui, &source, &mut |_, icx| {
            held.set(icx.cell(&CHECK).map_or(0.0, |cell| cell.right()));
            ListRow::new(LABEL).cells(icx.cells()).toggle(ToggleCell::check(CHECK, ToggleState::On, "{row}"))
        });
    });
    fixture.room(Vec2::new(400.0, 200.0));
    fixture.settle();
    let label = fixture
        .frame_shapes()
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.job.text == LABEL => Some(text.clone()),
            _ => None,
        })
        .expect("the label paints");
    assert!(!label.galley.elided, "the label shows whole");
    assert!(label.pos.x > end.get(), "the label starts past the toggle column");
}

/// A toggle column holds one control-high toggle with room on each side: wider than its control-high row, narrower
/// than two toggles.
#[trial]
fn a_toggle_column_holds_one_toggle_with_room_on_each_side() {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::contract::{ColumnSpec, ToggleColumn};
    use craft_ui::list::ToggleCell;
    use std::cell::Cell;
    const EYE: ColumnKey = ColumnKey::new("eye");
    const NAME: ColumnKey = ColumnKey::new("name");
    let eye = Rc::new(Cell::new(egui::Rect::NOTHING));
    let held = eye.clone();
    let mut fixture = Fixture::new(move |ui| {
        let columns = vec![ColumnSpec::toggle(EYE, "", ToggleColumn::sweepable()), ColumnSpec::new(NAME, "", ColumnWidth::fill(1.0))];
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Layers").columns(columns).show_items(ui, &source, &mut |key, icx| {
            held.set(icx.cell(&EYE).unwrap_or(egui::Rect::NOTHING));
            ListRow::new(NAMES[*key]).cells(icx.cells()).toggle(ToggleCell::eye(EYE, true))
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let cell = eye.get();
    assert!(cell.width() > cell.height(), "the toggle column is wider than its control-high row: {cell:?}");
    assert!(cell.width() < 2.0 * cell.height(), "the toggle column holds one toggle: {cell:?}");
}

/// A toggle cell's glyph rests in `text.muted` and reads in `text.normal` under the pointer.
#[trial]
fn a_toggle_glyph_rests_muted_and_reads_normal_under_the_pointer() {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::contract::{ColumnSpec, ToggleColumn};
    use craft_ui::list::ToggleCell;
    use craft_ui::runtime::env::Env;
    use craft_ui::testing::pointer;
    use egui::Color32;
    use std::cell::Cell;
    const EYE: ColumnKey = ColumnKey::new("eye");
    const NAME: ColumnKey = ColumnKey::new("name");
    let seen = Rc::new(Cell::new((egui::Rect::NOTHING, Color32::TRANSPARENT, Color32::TRANSPARENT)));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let text = Env::of(ui).tokens.tokens().color.text;
        let color = |[r, g, b, a]: [u8; 4]| Color32::from_rgba_unmultiplied(r, g, b, a);
        let columns = vec![ColumnSpec::toggle(EYE, "", ToggleColumn::sweepable()), ColumnSpec::new(NAME, "", ColumnWidth::fill(1.0))];
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Layers").columns(columns).show_items(ui, &source, &mut |key, icx| {
            held.set((icx.cell(&EYE).unwrap_or(egui::Rect::NOTHING), color(text.muted.0), color(text.normal.0)));
            ListRow::new(NAMES[*key]).cells(icx.cells()).toggle(ToggleCell::eye(EYE, true))
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let tints = |shapes: &[egui::Shape], cell: egui::Rect| -> Vec<Color32> {
        let mut tints: Vec<Color32> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Mesh(mesh) if cell.contains(mesh.calc_bounds().center()) => Some(mesh.vertices.iter().map(|vertex| vertex.color)),
                _ => None,
            })
            .flatten()
            .filter(|color| color.a() > 0)
            .collect();
        tints.dedup();
        tints
    };
    let (cell, muted, normal) = seen.get();
    let rest = tints(&fixture.frame_shapes(), cell);
    fixture.frame(vec![pointer::moved(cell.center())]);
    fixture.frame(Vec::new());
    let hovered = tints(&fixture.frame_shapes(), cell);
    assert!(!rest.is_empty() && rest.iter().all(|tint| *tint == muted), "the eye rests in text.muted {muted:?}: {rest:?}");
    assert!(
        !hovered.is_empty() && hovered.iter().all(|tint| *tint == normal),
        "the eye reads in text.normal {normal:?} under the pointer: {hovered:?}"
    );
}

/// Every filled rect among `shapes`, nested lists included.
fn fills(shapes: &[egui::Shape], out: &mut Vec<(egui::Rect, egui::Color32)>) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => fills(inner, out),
            egui::Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
            _ => {}
        }
    }
}

/// A list whose props name a surface paints it under its rows, over the list's rect.
#[trial]
fn a_list_paints_the_surface_its_props_name() {
    use craft_ui::runtime::env::Env;
    use craft_ui::style::SurfaceKind;
    use egui::{Color32, Rect};
    use std::cell::Cell;
    let shown = Rc::new(Cell::new((Rect::NOTHING, Color32::TRANSPARENT)));
    let held = shown.clone();
    let mut fixture = Fixture::new(move |ui| {
        let [r, g, b, a] = Env::of(ui).tokens.tokens().color.surface.sunken.0;
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let out =
            List::new("Artboards").surface(SurfaceKind::Sunken).natural_height(true).show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.set((out.response.rect, Color32::from_rgba_unmultiplied(r, g, b, a)));
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let (list, sunken) = shown.get();
    let mut seen = Vec::new();
    fills(&fixture.frame_shapes(), &mut seen);
    assert!(
        seen.iter().any(|(rect, fill)| *fill == sunken && rect.contains_rect(list.shrink(1.0)) && list.expand(1.0).contains_rect(*rect)),
        "a sunken fill covers the list {list:?}: {seen:?}"
    );
}

/// What a table of the three names shows with `pad`: its rect, its header cell's, its first row's and its empty
/// text's (the text of an empty table of the same props), and the pad in points.
fn padded(pad: craft_ui::contract::Pad) -> (egui::Rect, egui::Rect, egui::Rect, egui::Rect, f32) {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::contract::{ColumnSpec, pad_of};
    use craft_ui::runtime::env::Env;
    use craft_ui::table::Table;
    use std::cell::Cell;
    const EMPTY: &str = "No artboards";
    let shown = Rc::new(Cell::new((egui::Rect::NOTHING, 0.0)));
    let rig = |keys: &'static [usize], shown: Rc<Cell<(egui::Rect, f32)>>| {
        let mut fixture = Fixture::new(move |ui| {
            let source = keyed(keys, |key: &usize| *key);
            let columns = vec![ColumnSpec::new(ColumnKey::new("name"), "Name", ColumnWidth::Controls(4.0))];
            let table = Table::new("Artboards", columns).pad(pad).empty(EMPTY).natural_height(true);
            let out = table.show_items(ui, &source, &mut |key, icx| ListRow::new(NAMES[*key]).cells(icx.cells()));
            shown.set((out.response.rect, pad_of(&Env::of(ui).tokens, pad)));
        });
        fixture.room(Vec2::new(300.0, 200.0));
        fixture.settle();
        fixture
    };
    let fixture = rig(&[0, 1, 2], shown.clone());
    let (table, points) = shown.get();
    let header = fixture.by_label("Name").expect("the header cell shows").rect;
    let row = fixture.by_label(NAMES[0]).expect("the first row shows").rect;
    let mut empty = rig(&[], shown);
    fn text_at(shapes: &[egui::Shape], text: &str) -> Option<egui::Rect> {
        shapes.iter().find_map(|shape| match shape {
            egui::Shape::Vec(inner) => text_at(inner, text),
            egui::Shape::Text(shown) if shown.galley.text() == text => Some(shown.visual_bounding_rect()),
            _ => None,
        })
    }
    let text = text_at(&empty.frame_shapes(), EMPTY).expect("the empty table says so");
    (table, header, row, text, points)
}

/// A table's padding lies between its edge and everything it draws: the header, the rows and the empty text start the
/// pad further in than in an unpadded table.
#[trial]
fn a_padded_table_draws_its_header_rows_and_empty_text_inside_the_pad() {
    use craft_ui::contract::Pad;
    let (table, header, row, empty, pad) = padded(Pad::Control);
    let (bare, bare_header, bare_row, bare_empty, _) = padded(Pad::None);
    assert!(pad > 0.0, "the control pad is a width");
    let inside = table.shrink(pad - 0.5);
    for (name, rect) in [("header", header), ("row", row), ("empty text", empty)] {
        assert!(inside.contains_rect(rect), "the {name} {rect:?} sits inside the pad of {table:?}");
    }
    let start = |rect: egui::Rect, of: egui::Rect| rect.left() - of.left();
    assert!((start(header, table) - start(bare_header, bare) - pad).abs() <= 1.0, "the header starts the pad further in");
    assert!((start(row, table) - start(bare_row, bare) - pad).abs() <= 1.0, "the rows start the pad further in");
    assert!((start(empty, table) - start(bare_empty, bare) - pad).abs() <= 1.0, "the empty text starts the pad further in");
    assert!((header.top() - table.top() - (bare_header.top() - bare.top()) - pad).abs() <= 1.0, "the header starts the pad lower");
}

/// The ink of the text run `label` paints in `shapes`.
fn ink_of(shapes: &[egui::Shape], label: &str) -> egui::Color32 {
    shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let vertex = text.galley.rows.iter().flat_map(|row| row.row.visuals.mesh.vertices.iter()).map(|vertex| vertex.color).next();
                Some(text.override_text_color.or(vertex.filter(|color| *color != egui::Color32::PLACEHOLDER)).unwrap_or(text.fallback_color))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{label} paints no text"))
}

/// A disabled row's text takes the disabled ink, in a list row and an option row alike.
#[trial]
fn a_disabled_row_paints_its_text_in_the_disabled_ink() {
    let scene = |ui: &mut egui::Ui| {
        let rows = [ListRow::new("Alpha"), ListRow::new("Beta").enabled(false), ListRow::new("Gamma").option().enabled(false)];
        for (at, row) in rows.into_iter().enumerate() {
            ui.push_id(at, |ui| craft_ui::testing::show_as_only_item(ui, row));
        }
    };
    let mut frames = craft_ui::testing::Frames::new();
    frames.idle(4, scene);
    let shapes = frames.frame_shapes(scene);
    let (enabled, disabled, option) = (ink_of(&shapes, "Alpha"), ink_of(&shapes, "Beta"), ink_of(&shapes, "Gamma"));
    assert_ne!(disabled, enabled, "a disabled row's text is not in the enabled ink");
    assert_eq!(option, disabled, "an option row and a list row share the disabled ink");
}

/// A right-click on the empty space below the rows asks for the list's own menu, not a row's, and leaves the selection
/// alone.
#[trial]
fn a_right_click_below_the_rows_asks_for_the_lists_own_menu() {
    use craft_ui::testing::pointer;
    let (mut fixture, seen) = rig(|_| None);
    let (list, last) = (fixture.by_label("Artboards").expect("the list").rect, fixture.by_label(NAMES[2]).expect("the last row").rect);
    assert!(list.bottom() > last.bottom() + 8.0, "the list is taller than its rows: {list:?} {last:?}");
    let below = egui::Pos2::new(last.center().x, (last.bottom() + list.bottom()) / 2.0);
    fixture.frame(vec![pointer::moved(below), pointer::secondary(below, true)]);
    fixture.frame(vec![pointer::secondary(below, false)]).settle();
    let events = seen.borrow();
    assert!(events.iter().any(|event| matches!(event, CollectionEvent::EmptyContextMenu { .. })), "{events:?}");
    assert!(!events.iter().any(|event| matches!(event, CollectionEvent::ContextMenu(_) | CollectionEvent::Selection { .. })), "{events:?}");
}

/// A list with the edge press `edge` (none without), and what it reported.
fn edge_rig(edge: bool) -> (Fixture, Events) {
    use craft_behavior::{Cursor, CursorIcon};
    use craft_keymap::Mods;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let mut list = List::new("Artboards");
        if edge {
            // Only the line under the first row takes the press.
            list = list.edge_press(Mods::ALT, |above: &usize, _: &usize| (*above == 0).then_some(Cursor::System(CursorIcon::Crosshair)));
        }
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    (fixture, seen)
}

/// Alt-click on the line between two rows: with an edge press the line takes it (its cursor, `EdgePressed`, no row
/// picked) where the test takes the rows, and the rows keep it on a line the test refuses.
#[trial]
fn an_edge_press_takes_the_alt_click_on_the_line_it_accepts() {
    use craft_behavior::{Cursor, CursorIcon};
    use egui::{Modifiers, Pos2};
    let (mut fixture, seen) = edge_rig(true);
    let line = |fixture: &Fixture, row: &str| {
        let rect = fixture.by_label(row).expect("the row").rect;
        Pos2::new(rect.center().x, rect.bottom() - 1.0)
    };
    let first = line(&fixture, NAMES[0]);
    fixture.hold(Modifiers::ALT);
    fixture.frame(vec![craft_ui::testing::pointer::moved(first)]).idle(1);
    assert_eq!(craft_ui::runtime::cursor::current(fixture.ctx()), Some(Cursor::System(CursorIcon::Crosshair)), "the line's cursor");
    fixture.click_at(first).settle();
    {
        let events = seen.borrow();
        assert!(events.iter().any(|event| matches!(event, CollectionEvent::EdgePressed { above: 0, below: 1, .. })), "{events:?}");
        assert!(!events.iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "no row is picked: {events:?}");
    }
    seen.borrow_mut().clear();
    let second = line(&fixture, NAMES[1]);
    fixture.click_at(second).settle();
    fixture.hold(Modifiers::NONE);
    let events = seen.borrow();
    assert!(!events.iter().any(|event| matches!(event, CollectionEvent::EdgePressed { .. })), "a refused line: {events:?}");
    assert!(events.iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "the row takes the click: {events:?}");
}

/// A list without an edge press is unchanged: an Alt-click on the line between two rows picks a row.
#[trial]
fn a_list_without_an_edge_press_gives_the_line_to_the_rows() {
    use egui::{Modifiers, Pos2};
    let (mut fixture, seen) = edge_rig(false);
    let rect = fixture.by_label(NAMES[0]).expect("the row").rect;
    fixture.hold(Modifiers::ALT).click_at(Pos2::new(rect.center().x, rect.bottom() - 1.0)).settle();
    fixture.hold(Modifiers::NONE);
    let events = seen.borrow();
    assert!(!events.iter().any(|event| matches!(event, CollectionEvent::EdgePressed { .. })), "{events:?}");
    assert!(events.iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "{events:?}");
}

/// A revealed row far below the view, past rows no pass built yet and taller than the list's estimate, shows wholly
/// in view in the frame that reveals it: the list measures the rows it scrolls past before it places them, runs the
/// frame once, and measures no row twice.
#[trial]
fn a_revealed_row_past_unbuilt_rows_taller_than_the_estimate_shows_wholly_in_one_pass() {
    use craft_ui::list::ThumbSize;
    use std::cell::Cell;
    const ROWS: usize = 40;
    let names: Vec<String> = (0..ROWS).map(|at| format!("Photo {at:02}")).collect();
    let reveal = Rc::new(Cell::new(None::<usize>));
    let asked = reveal.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..ROWS).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let mut list = List::new("Photos");
        if let Some(key) = asked.take() {
            list = list.reveal(key);
        }
        list.show_items(ui, &source, &mut |key, _| ListRow::new(names[*key].clone()).thumb(ThumbSize::Large, |_, _| {}));
    });
    fixture.room(Vec2::new(300.0, 300.0));
    fixture.settle();
    let view = |fixture: &Fixture| fixture.by_label("Photos").expect("the list's node").rect;
    let in_view = |fixture: &Fixture, name: &str| fixture.by_label(name).is_some_and(|row| view(fixture).contains_rect(row.rect));
    assert!(in_view(&fixture, "Photo 00") && !in_view(&fixture, "Photo 39"), "the list starts at the top");

    reveal.set(Some(ROWS - 1));
    fixture.idle(1);
    assert!(in_view(&fixture, "Photo 39"), "the revealed row shows wholly in the frame that reveals it");
    assert_eq!(fixture.passes(), 1, "the reveal runs the frame once");
    let measures = craft_ui::runtime::measure::row_measures(fixture.ctx());
    assert!(!measures.is_empty() && measures.iter().all(|(_, count)| *count == 1), "no row is measured twice in the pass: {measures:?}");
    fixture.idle(2);
    assert!(in_view(&fixture, "Photo 39"), "the row stays in view");

    reveal.set(Some(0));
    fixture.idle(1);
    assert!(in_view(&fixture, "Photo 00"), "the first row scrolls back wholly into view");
}

/// One disclosure glyph: a section's disclosure and a row's trailing disclosure each paint one filled triangle, the
/// same size, and no stroked chevron.
#[trial]
fn a_sections_disclosure_and_a_rows_trailing_disclosure_paint_the_same_triangle() {
    use craft_ui::contract::{Bind, Show as _};
    use craft_ui::controls::disclosure::Disclosure;
    use craft_ui::list::{TrailingFace, TrailingPart};
    let triangles = |fixture: &mut Fixture| -> Vec<Vec2> {
        let shapes = fixture.frame_shapes();
        assert!(!shapes.iter().any(|shape| matches!(shape, egui::Shape::Path(path) if !path.closed)), "no stroked chevron paints");
        shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds().size()),
                _ => None,
            })
            .collect()
    };
    let mut section = Fixture::new(|ui| {
        let _ = Disclosure::new(Bind::given(true)).label("Details").show(ui);
    });
    section.room(Vec2::new(300.0, 200.0));
    section.settle();
    let mut row = Fixture::new(|ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Layers").show_items(ui, &source, &mut |key, _| {
            ListRow::new(NAMES[*key]).trailing_part(TrailingPart::new(1), TrailingFace::disclosure(true), "Effects")
        });
    });
    row.room(Vec2::new(300.0, 200.0));
    row.settle();
    let (section, row) = (triangles(&mut section), triangles(&mut row));
    assert_eq!((section.len(), row.len()), (1, 1), "each paints one filled triangle: {section:?} {row:?}");
    assert!((section[0] - row[0]).length() < 0.01, "the two triangles are the same size: {section:?} {row:?}");
}

/// A thumbnail is square and framed around its picture: the frame is a square-cornered rect whose inside the picture
/// fills, so nothing of the picture shows past the frame, and the row holds the frame.
#[trial]
fn a_thumbnails_frame_is_square_and_lies_outside_its_picture() {
    use craft_ui::list::ThumbSize;
    use std::cell::Cell;
    let picture = Rc::new(Cell::new(egui::Rect::NOTHING));
    let painted = picture.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Layers").show_items(ui, &source, &mut |key, _| {
            let painted = painted.clone();
            ListRow::new(NAMES[*key]).thumb(ThumbSize::Medium, move |_, rect| painted.set(rect))
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let picture = picture.get();
    let row = fixture.by_label(NAMES[0]).expect("the row").rect;
    let frame = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Rect(rect) if !rect.stroke.is_empty() && rect.rect.contains_rect(picture) && rect.rect != picture => Some(rect.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a frame paints around the picture {picture:?}"));
    assert_eq!(frame.corner_radius, egui::CornerRadius::ZERO, "the frame is square");
    let inner = frame.rect.shrink(frame.stroke.width);
    assert!(inner.contains_rect(picture) && picture.contains_rect(inner.shrink(0.01)), "the picture fills the frame's inside: {frame:?} {picture:?}");
    assert!(row.contains_rect(frame.rect), "the row holds the frame: {row:?} {frame:?}");
}

/// A row whose label comes from styled spans draws its match, and nothing else of the label, on the highlight, and the
/// joined text names the row. A row that dropped the spans would draw the label as one plain span.
#[trial]
fn a_row_labelled_from_spans_highlights_only_its_match_and_is_named_by_the_joined_text() {
    use craft_ui::primitives::Span;
    const JOINED: &str = "over the harbour at dusk";
    let mut fixture = Fixture::new(|ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key).with_text(|_| std::borrow::Cow::Borrowed(JOINED));
        let _ = List::new("Results").show_items(ui, &source, &mut |_, _| {
            let spans = vec![Span::new("over the "), Span::new("harbour").highlight(), Span::new(" at dusk")];
            ListRow::new(JOINED).label_spans(spans)
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    assert!(fixture.by_label(JOINED).is_some(), "the joined text names the row");
    let shapes = fixture.frame_shapes();
    let galley = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(shown) if shown.galley.text() == JOINED => Some(shown.galley.clone()),
            _ => None,
        })
        .expect("the row draws its label whole");
    let marked: Vec<&str> = galley
        .job
        .sections
        .iter()
        .filter(|section| section.format.background != egui::Color32::TRANSPARENT)
        .filter_map(|section| galley.job.text.get(section.byte_range.start.0..section.byte_range.end.0))
        .collect();
    assert_eq!(marked, ["harbour"], "only the match is drawn on the highlight");
}

/// What a double click on the second row of a list with `select` and `allow_empty`, nothing selected, reports, and the
/// selection it leaves with a host that keeps every selection the list reports.
fn double_clicked(
    select: craft_ui::contract::SelectMode,
    allow_empty: bool,
) -> (Vec<CollectionEvent<usize, RowEvent>>, craft_behavior::Selection<usize>) {
    use craft_behavior::{Selection, SelectionModel as _};
    use craft_ui::contract::Bind;
    let state = Rc::new(RefCell::new((Vec::new(), Selection::default())));
    let held = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let given = held.borrow().1.clone();
        let list = List::new("Artboards").select(select).allow_empty(allow_empty).selection(Bind::given(given));
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        let (events, selection) = &mut *held.borrow_mut();
        for event in &out.events {
            if let CollectionEvent::Selection { change, .. } = event {
                selection.apply(change.clone());
            }
        }
        events.extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    state.borrow_mut().0.clear();
    let row = fixture.by_label(NAMES[1]).expect("the second row");
    fixture.click(&row).settle();
    fixture.click(&row).settle();
    state.take()
}

/// A double click on an unselected row selects it and activates it once, with no selection change after the
/// activation: the second click keeps what the first selected. In a single selection that may be empty the row stays
/// selected; the second click does not deselect it.
#[trial]
fn a_double_click_selects_the_row_activates_it_once_and_changes_nothing_after() {
    use craft_ui::contract::SelectMode;
    for (select, allow_empty) in [(SelectMode::Multi, false), (SelectMode::Single, true)] {
        let (events, selection) = double_clicked(select, allow_empty);
        let selected: Vec<usize> = (0..NAMES.len()).filter(|key| selection.contains(key)).collect();
        assert_eq!(selected, [1], "{select:?}: the row is selected: {events:?}");
        let activations: Vec<usize> =
            events.iter().enumerate().filter_map(|(at, event)| matches!(event, CollectionEvent::Activate(1)).then_some(at)).collect();
        assert_eq!(activations.len(), 1, "{select:?}: one activation: {events:?}");
        let last_change = events.iter().rposition(|event| matches!(event, CollectionEvent::Selection { .. }));
        assert!(last_change.is_some_and(|at| at < activations[0]), "{select:?}: selected before the activation, not after: {events:?}");
    }
}

/// A collection's group header reads in the body weight; a table's section heading in the strong weight.
#[trial]
fn a_group_header_is_regular_and_a_section_heading_strong() {
    use craft_theme::{Contrast, ControlSize, Mode, TypeRole, Variant};
    use craft_ui::contract::{Component as _, Meta as _};
    use craft_ui::style::ComponentStyle as _;
    use craft_ui::testing::style::{tokens, with_cx};
    with_cx(&egui::Context::default(), &tokens(Mode::Light, Contrast::Standard), Variant::SECONDARY, ControlSize::Md, |cx| {
        let role = |row: ListRow<'_>| ListRow::style(cx, row.style_key()).texts()[0].role;
        assert_eq!(role(ListRow::new("Folder").heading()), TypeRole::Body, "a group header");
        assert_eq!(role(ListRow::new("Camera").section_heading()), TypeRole::BodyStrong, "a section heading");
    });
}

/// A reorder drag held in the bottom edge band scrolls the rows on every frame, and a frame egui runs twice scrolls
/// them once; held below the list it still scrolls; moved back over the middle rows, the rows stay and no frame is
/// asked for.
#[trial]
fn a_row_dragged_to_the_edge_scrolls_the_list_and_rests_without_frames() {
    use craft_ui::contract::{ReorderRule, Sizing};
    use craft_ui::testing::pointer::moved;
    use egui::Pos2;
    use std::borrow::Cow;
    use std::cell::Cell;

    const ROWS: usize = 40;
    let name = |at: usize| format!("Layer {at}");
    let list_id = Id::NULL.with("long-rows");
    let discard = Rc::new(Cell::new(false));
    let asks = discard.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..ROWS).collect();
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| Cow::Owned(format!("Layer {key}")));
        let list = List::new("Layers").id(list_id).sizing(Sizing::Content).max_rows(6).reorder(ReorderRule::new());
        list.show_items(ui, &source, &mut |key, _| ListRow::new(format!("Layer {key}")));
        if asks.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs the frame twice");
        }
    });
    fixture.room(Vec2::new(300.0, 400.0));
    fixture.settle();
    let list = fixture.node_of(list_id).expect("the list's node").rect;
    let first = fixture.by_label(&name(0)).expect("the first row").rect;
    let row = fixture.by_label(&name(1)).expect("the second row").rect.min.y - first.min.y;
    // How far the rows have scrolled, from the first row whose top is in view: its place in the rows, less how far
    // below the rows' top it sits. Rows above the view are not built, or sit above the top.
    let scrolled = |fixture: &Fixture| {
        (0..ROWS)
            .find_map(|at| {
                let shown = fixture.by_label(&name(at)).filter(|shown| shown.rect.min.y >= first.min.y - 0.5 && shown.rect.height() >= row - 0.5)?;
                Some(at as f32 * row - (shown.rect.min.y - first.min.y))
            })
            .unwrap_or(0.0)
    };

    let grab = fixture.by_label(&name(4)).expect("a row near the bottom").rect.center();
    let band = Pos2::new(grab.x, list.max.y - row / 4.0);
    fixture.drag_hold_at(grab, band - grab);
    let mut last = scrolled(&fixture);
    assert!(last > 0.0, "the drag into the bottom band scrolls the rows");
    for _ in 0..3 {
        fixture.frame(Vec::new());
        let now = scrolled(&fixture);
        assert!(now > last, "every frame in the band scrolls on: {now} after {last}");
        last = now;
    }

    // Two frames of one pass each, then two frames of which the first runs twice: the second pair scrolls as far as
    // the first (two steps), not a step further. Each pair is measured after a one-pass frame.
    let before = scrolled(&fixture);
    fixture.frame(Vec::new()).frame(Vec::new());
    assert_eq!(fixture.passes(), 1, "a frame of the held drag runs one pass");
    let between = scrolled(&fixture);
    discard.set(true);
    fixture.frame(Vec::new());
    discard.set(false);
    assert_eq!(fixture.passes(), 2, "the trial's frame ran twice");
    fixture.frame(Vec::new());
    let (plain, rerun) = (between - before, scrolled(&fixture) - between);
    assert!(plain > 0.0 && rerun < plain * 1.25, "a frame run twice scrolls one frame's step: {rerun} against {plain}");

    let below = Pos2::new(grab.x, list.max.y + 2.0 * row);
    fixture.frame(vec![moved(below)]).frame(Vec::new());
    let past = scrolled(&fixture);
    fixture.idle(2);
    assert!(scrolled(&fixture) > past, "held below the list, the rows still scroll");

    fixture.frame(vec![moved(Pos2::new(grab.x, list.center().y))]).idle(2);
    let rest = scrolled(&fixture);
    fixture.frame(Vec::new());
    assert_eq!(scrolled(&fixture), rest, "over the middle rows nothing scrolls");
    assert!(!fixture.repaint_requested(), "a drag held over the middle rows asks for no frame");
    fixture.release_at(Pos2::new(grab.x, list.center().y));
}

/// A right click on a row's button asks for the row's menu, as a right click elsewhere on the row does, and does not
/// press the button: an attachment's Save button opens the attachment's menu.
#[trial]
fn a_secondary_click_on_a_trailing_part_opens_the_row_menu() {
    use craft_ui::list::{TrailingFace, TrailingPart};
    use craft_ui::primitives::IconSpec;
    use craft_ui::testing::pointer;
    const SAVES: [&str; 3] = ["Save 1", "Save 2", "Save 3"];
    let row = |key: usize| {
        ListRow::new(NAMES[key]).trailing_part(
            TrailingPart::new(0),
            TrailingFace::Icon(IconSpec::svg(craft_behavior::IconId::new("settings"))),
            SAVES[key],
        )
    };
    let (mut fixture, seen) = rows_rig(Vec2::new(320.0, 200.0), row);
    let button = fixture.by_label(SAVES[1]).expect("the part has a node").center();
    fixture.frame(vec![pointer::moved(button), pointer::secondary(button, true)]);
    fixture.frame(vec![pointer::secondary(button, false)]).settle();
    let events = seen.borrow();
    assert!(events.iter().any(|event| matches!(event, CollectionEvent::ContextMenu(request) if request.key == 1)), "the row's menu: {events:?}");
    assert!(
        !events.iter().any(|event| matches!(event, CollectionEvent::Item(_, RowEvent::Trailing { .. }))),
        "the button is not pressed: {events:?}"
    );
}

/// A status cell's tip is the cell's description, and its action is reached from the keyboard: into the list, onto
/// the row's parts, and Enter reports the action with the cell's column.
#[trial]
fn a_status_cell_action_is_reached_by_keys_and_reports_its_column() {
    use craft_behavior::{ColumnKey, ColumnWidth, Selection};
    use craft_theme::variant::Tone;
    use craft_ui::contract::{Bind, ColumnSpec};
    use craft_ui::list::CellStatus;
    use egui::{Key, Modifiers};
    const NAME: ColumnKey = ColumnKey::new("name");
    const WARNINGS: ColumnKey = ColumnKey::new("warnings");
    const TIP: &str = "Its form fields are flattened.\nTwo bookmarks point to pages that are not kept.";
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let columns = vec![ColumnSpec::new(NAME, "Name", ColumnWidth::fill(1.0)), ColumnSpec::new(WARNINGS, "Warnings", ColumnWidth::fill(1.0))];
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let list = List::new("Files").columns(columns).selection(Bind::given(Selection::single(0)));
        let out = list.show_items(ui, &source, &mut |_, icx| {
            let status = CellStatus::new(Tone::Warn, "2 warnings").tip(TIP).action("Details");
            ListRow::new("report.pdf").cells(icx.cells()).cell_status(WARNINGS, status)
        });
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    let cell = fixture.by_label("2 warnings").expect("the status cell has a node");
    assert_eq!(fixture.tree().description_of(cell.node), Some(TIP), "the tip is the cell's description");
    let details = fixture.by_label("Details").expect("the action has a node");

    fixture.key(Key::Tab, Modifiers::NONE).key(Key::ArrowRight, Modifiers::NONE).settle();
    assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), details.node, "the keys reach the action");
    seen.borrow_mut().clear();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    let actions: Vec<ColumnKey> = seen
        .borrow()
        .iter()
        .filter_map(|event| match event {
            CollectionEvent::Item(0, RowEvent::CellAction { column, .. }) => Some(column.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(actions, [WARNINGS], "Enter on the action reports it once, with its column");
}

/// The row a collection reports under the pointer: the hovered row, none over the empty space below the rows, and
/// none while a drag is held over a row.
#[trial]
fn a_collection_reports_the_row_under_the_pointer() {
    use craft_ui::contract::ReorderRule;
    use craft_ui::testing::pointer;
    use egui::Pos2;
    use std::cell::Cell;
    let hovered = Rc::new(Cell::new(None::<usize>));
    let held = hovered.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = List::new("Artboards").reorder(ReorderRule::new()).show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.set(out.hovered);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let row = |fixture: &Fixture, at: usize| fixture.by_label(NAMES[at]).expect("the row shows").rect;
    let (first, second, last) = (row(&fixture, 0), row(&fixture, 1), row(&fixture, 2));
    fixture.frame(vec![pointer::moved(second.center())]).settle();
    assert_eq!(hovered.get(), Some(1), "the row under the pointer");
    let list = fixture.by_label("Artboards").expect("the list").rect;
    assert!(list.bottom() > last.bottom() + 8.0, "the list is taller than its rows: {list:?} {last:?}");
    fixture.frame(vec![pointer::moved(Pos2::new(last.center().x, (last.bottom() + list.bottom()) / 2.0))]).settle();
    assert_eq!(hovered.get(), None, "no row under the pointer below the rows");
    fixture.drag_hold_at(first.center(), last.center() - first.center());
    assert_eq!(hovered.get(), None, "no row while a drag is held over one");
    fixture.release_at(last.center());
    fixture.frame(vec![pointer::moved(second.center())]).settle();
    assert_eq!(hovered.get(), Some(1), "after the drag the row under the pointer again");
}

/// Where the text shape that paints `text` lies among `shapes`, nested lists included.
fn painted_at(shapes: &[egui::Shape], text: &str) -> Option<egui::Rect> {
    shapes.iter().find_map(|shape| match shape {
        egui::Shape::Vec(inner) => painted_at(inner, text),
        egui::Shape::Text(shown) if shown.galley.text() == text => Some(shown.galley.rect.translate(shown.pos.to_vec2())),
        _ => None,
    })
}

/// A list's row of a document's field name, with its labels as document text aligned `align` (`None` for UI text):
/// the row's rect and its label's.
fn document_row(align: Option<craft_ui::primitives::HAlign>) -> (egui::Rect, egui::Rect) {
    use craft_ui::fonts::FontRef;
    use craft_ui::list::DocumentRows;
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize];
        let source = keyed(&keys, |key: &usize| *key);
        let mut list = List::new("Fields");
        if let Some(align) = align {
            // A document size well above the interface's body size.
            list = list.document_rows(DocumentRows::new(FontRef::proportional(), 32.0, align));
        }
        let _ = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
    });
    fixture.room(Vec2::new(400.0, 300.0));
    fixture.settle();
    let row = fixture.by_label(NAMES[0]).expect("the row shows").rect;
    let label = painted_at(&fixture.frame_shapes(), NAMES[0]).expect("the label paints");
    (row, label)
}

/// A list of document rows lays its labels out at the document's size, the row growing to hold them; a centred label
/// centres on the row's width, a start-aligned one starts at the row's start; a row of UI text keeps its text size.
#[trial]
fn document_rows_take_the_documents_size_and_alignment() {
    use craft_ui::primitives::HAlign;
    let (plain_row, plain) = document_row(None);
    let (start_row, start) = document_row(Some(HAlign::Start));
    let (centred_row, centred) = document_row(Some(HAlign::Center));
    assert!(start.height() > plain.height(), "the label is at the document's size: {start:?} against {plain:?}");
    assert!(start_row.height() > plain_row.height(), "the row grows for it: {start_row:?} against {plain_row:?}");
    assert!(start_row.expand(0.5).contains_rect(start), "the row holds its label: {start:?} in {start_row:?}");
    assert!(start.center().x < start_row.center().x, "a start-aligned label starts at the row's start: {start:?} in {start_row:?}");
    assert!(plain.center().x < plain_row.center().x, "a row of UI text keeps its label at the start: {plain:?} in {plain_row:?}");
    assert!((centred.center().x - centred_row.center().x).abs() < 1.0, "a centred label centres on the row: {centred:?} in {centred_row:?}");
}

/// The texts among `names` the latest frame painted, top to bottom.
fn painted_rows(fixture: &Fixture, names: &[&str]) -> Vec<String> {
    let mut runs: Vec<(f32, String)> =
        fixture.texts().iter().filter(|run| names.contains(&run.text.as_str())).map(|run| (run.rect.top(), run.text.clone())).collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs.dedup_by(|a, b| a.1 == b.1);
    runs.into_iter().map(|(_, text)| text).collect()
}

/// A branch of a bookmarks tree in display order, every group open: each node's title and depth.
const BRANCHES: [(&str, usize); 3] = [("İSTANBUL ŞUBESİ", 0), ("Kış dönemi", 1), ("Ankara", 0)];

/// [`BRANCHES`] as the display order of a tree list, keyed by place.
struct Branches;

impl craft_behavior::DisplayOrder<usize> for Branches {
    fn len(&self) -> usize {
        BRANCHES.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn depth(&self, at: usize) -> usize {
        BRANCHES[at].1
    }

    fn has_children(&self, at: usize) -> bool {
        BRANCHES.get(at + 1).is_some_and(|next| next.1 > BRANCHES[at].1)
    }

    fn open(&self, _: usize) -> bool {
        true
    }
}

impl craft_behavior::ItemSource<usize> for Branches {}

/// A filter keeps a row when any of its texts matches the query, and in a tree it keeps the ancestors of a match
/// above it. Case and the dotted and dotless i fold the way the scope's folder folds them: `istanbul` finds
/// `İSTANBUL`, and `KIŞ` finds `Kış`.
#[trial]
fn a_filter_matches_any_of_a_rows_texts_and_keeps_the_ancestors_of_a_match() {
    use craft_behavior::DisplayOrder as _;
    use craft_ui::runtime::env::Label;
    use std::cell::Cell;
    const NOTES: [&str; 3] = ["Cover", "Back page", "Inside"];
    let query = Rc::new(Cell::new(""));
    let asked = query.clone();
    let mut flat = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let list = List::new("Artboards").filter(asked.get(), |key: &usize| vec![Label::from(NAMES[*key]), Label::from(NOTES[*key])]);
        let _ = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
    });
    flat.room(Vec2::new(300.0, 200.0));
    flat.settle();
    assert_eq!(painted_rows(&flat, &NAMES), NAMES, "an empty query keeps every row");
    query.set("back");
    flat.settle();
    assert_eq!(painted_rows(&flat, &NAMES), [NAMES[1]], "a row matches by a text other than its label");

    let tree_query = Rc::new(Cell::new(""));
    let asked = tree_query.clone();
    let mut tree = Fixture::new(move |ui| {
        let list = List::new("Bookmarks").tree(true).filter(asked.get(), |key: &usize| vec![Label::from(BRANCHES[*key].0)]);
        let _ = list.show_items(ui, &Branches, &mut |key, _| {
            let (title, depth) = BRANCHES[*key];
            let row = ListRow::new(title).depth(depth);
            if Branches.has_children(*key) { row.group(true) } else { row }
        });
    });
    tree.room(Vec2::new(300.0, 200.0));
    tree.settle();
    let titles: Vec<&str> = BRANCHES.iter().map(|(title, _)| *title).collect();
    let cases: [(&str, &[&str]); 3] = [("istanbul", &[BRANCHES[0].0]), ("KIŞ", &[BRANCHES[0].0, BRANCHES[1].0]), ("izmir", &[])];
    for (asked, kept) in cases {
        tree_query.set(asked);
        tree.settle();
        assert_eq!(painted_rows(&tree, &titles), kept, "the rows {asked:?} keeps, parents first");
    }
}

/// A group row's triangle is named after the row as a screen reader hears it: a row drawn "March" whose accessible
/// name is "March 2025" has the triangle "Expand March 2025", so two rows with one caption are told apart. A row
/// without an accessible name keeps naming its triangle after its label.
#[trial]
fn a_triangle_is_named_after_the_rows_accessible_name() {
    const MONTHS: [(&str, Option<&str>); 3] = [("March", Some("March 2025")), ("March", Some("March 2026")), ("April", None)];
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let list = List::new("Months").tree(true);
        let _ = list.show_items(ui, &source, &mut |key, _| {
            let (title, name) = MONTHS[*key];
            let row = ListRow::new(title).depth(0).group(false);
            match name {
                Some(name) => row.accessible_name(name),
                None => row,
            }
        });
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    assert!(fixture.by_label("Expand March 2025").is_some(), "the first March's triangle names its year");
    assert!(fixture.by_label("Expand March 2026").is_some(), "the second March's triangle names its year");
    assert!(fixture.by_label("Expand March").is_none(), "no triangle is named by the drawn caption alone");
    assert!(fixture.by_label("Expand April").is_some(), "a row without an accessible name names its triangle after its label");
}

/// A row that a rename request names stays listed while its rename runs, though the query excludes it, and goes when
/// the rename ends.
#[trial]
fn a_row_in_rename_stays_listed_while_the_filter_excludes_it() {
    use craft_ui::runtime::env::Label;
    use egui::{Key, Modifiers};
    let shown = Rc::new(RefCell::new(Vec::<usize>::new()));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (rows, held) = (shown.clone(), seen.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let list = List::new("Artboards").id(list_id()).rename(true).filter(NAMES[0], |key: &usize| vec![Label::from(NAMES[*key])]);
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        *rows.borrow_mut() = out.shown.iter().map(|row| row.key).collect();
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    assert_eq!(*shown.borrow(), [0], "the query keeps the first row alone");
    request_rename(fixture.ctx(), list_id(), &2_usize);
    fixture.settle();
    assert!(
        seen.borrow()
            .iter()
            .any(|event| matches!(event, CollectionEvent::Rename(EditEvent::Begin { before: Mixed::Same(rename), .. }) if rename.key == 2)),
        "the rename of the excluded row begins: {:?}",
        seen.borrow()
    );
    assert!(shown.borrow().contains(&2), "the row stays listed while its rename runs: {:?}", shown.borrow());
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(*shown.borrow(), [0], "the row goes when its rename ends");
}

/// The rename field selects the whole name once when it opens, counted in characters, so typing replaces a multibyte
/// name; a later frame keeps the caret the user put; Escape ends the rename with no edit.
#[trial]
fn a_rename_editor_selects_the_name_once_keeps_the_caret_and_escape_cancels() {
    use craft_ui::testing::focus::egui_focused;
    use craft_ui::testing::text::caret;
    use egui::{Key, Modifiers};
    const SHEETS: [&str; 2] = ["Çalışma sayfası", "Größe"];
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(SHEETS[*key]));
        let out = List::new("Sheets").id(list_id()).rename(true).show_items(ui, &source, &mut |key, _| ListRow::new(SHEETS[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let name = SHEETS[0];
    assert!(name.len() > name.chars().count(), "the name is multibyte");
    let edits = |seen: &Events| -> Vec<(usize, String)> {
        seen.borrow()
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Rename(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => Some((after.key, after.name.clone())),
                _ => None,
            })
            .collect()
    };

    request_rename(fixture.ctx(), list_id(), &0_usize);
    fixture.settle();
    let field = egui_focused(fixture.ctx()).expect("the rename field takes the focus");
    assert_eq!(caret(fixture.ctx(), field), Some(name.chars().count()), "the field opens with the whole name selected");
    fixture.key(Key::Home, Modifiers::NONE);
    eprintln!("DIAG rename caret: right after Home {:?} focused {:?}", caret(fixture.ctx(), field), egui_focused(fixture.ctx()) == Some(field));
    for frame in 0..3 {
        fixture.idle(1);
        eprintln!("DIAG rename caret: idle {frame} {:?} focused {:?}", caret(fixture.ctx(), field), egui_focused(fixture.ctx()) == Some(field));
    }
    assert_eq!(caret(fixture.ctx(), field), Some(0), "later frames keep the caret the user put");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(edits(&seen).is_empty(), "Escape ends the rename with no edit: {:?}", seen.borrow());
    assert_ne!(egui_focused(fixture.ctx()), Some(field), "the rename field is gone");

    request_rename(fixture.ctx(), list_id(), &0_usize);
    fixture.settle();
    fixture.type_text("Özet").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(edits(&seen), [(0, String::from("Özet"))], "typing replaces the whole multibyte name");
}

/// A part revealed on a hovered or selected row is exposed only while it shows: a screen reader finds the part of
/// the hovered row, and no part of the rows that hide theirs.
#[trial]
fn a_revealed_part_is_exposed_only_while_its_row_shows_it() {
    use craft_ui::list::{TrailingFace, TrailingPart};
    use craft_ui::primitives::IconSpec;
    use craft_ui::testing::pointer;
    const MOVES: [&str; 3] = ["Move 1", "Move 2", "Move 3"];
    let row = |key: usize| {
        ListRow::new(NAMES[key]).trailing_part_revealed(
            TrailingPart::new(0),
            TrailingFace::Icon(IconSpec::svg(craft_behavior::IconId::new("settings"))),
            MOVES[key],
        )
    };
    let (mut fixture, _) = rows_rig(Vec2::new(320.0, 200.0), row);
    assert!(MOVES.iter().all(|name| fixture.by_label(name).is_none()), "no row shows its part, so none is exposed");
    let over = fixture.by_label(NAMES[1]).expect("the row's node").rect.center();
    fixture.frame(vec![pointer::moved(over)]).settle();
    assert!(fixture.by_label(MOVES[1]).is_some(), "the hovered row's part is exposed");
    assert!(fixture.by_label(MOVES[0]).is_none() && fixture.by_label(MOVES[2]).is_none(), "the other rows' parts stay hidden");
}

/// Auto rows place by their measure in the first frame. In a column sized from its content along the height, where
/// each pass runs the list's measuring run and its real run, the first frame runs once, measures each row once, builds each row at most
/// twice, and places the rows and the list where the settled list has them. A click that selects a row and a rename
/// request change a state the row's size reads after the pass measured it: the frame shows the row, and the list
/// around it, at the new size.
#[trial]
fn auto_rows_are_measured_once_per_pass_and_a_restated_row_takes_its_new_size_in_the_same_frame() {
    use craft_ui::contract::{ShowContainer as _, Sizing};
    use craft_ui::layout_components::Stack;
    use craft_ui::list::ThumbSize;
    use craft_ui::testing::pointer::{moved, press, release};
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().sizing(Sizing::FillWidth).show(ui, |ui, _| {
            let keys = [0_usize, 1, 2];
            let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
            List::new("Artboards").id(list_id()).rename(true).show_items(ui, &source, &mut |key, icx| {
                let size = if icx.row.selected || icx.row.renaming { ThumbSize::Large } else { ThumbSize::Medium };
                ListRow::new(NAMES[*key]).thumb(size, |_, _| {})
            });
        });
    });
    fixture.room(Vec2::new(300.0, 400.0));
    let list = |fixture: &Fixture| fixture.by_label("Artboards").expect("the list's node").rect;
    let row = |fixture: &Fixture, at: usize| fixture.by_label(NAMES[at]).expect("the row's node").rect;
    fixture.idle(1);
    assert_eq!(fixture.passes(), 1, "the first frame runs once");
    let runs = craft_ui::testing::last_pass_runs(fixture.ctx());
    assert!(runs.iter().any(|(name, count)| name.contains("::List<") && *count >= 2), "the list has a measuring run and a real run: {runs:?}");
    let measures = craft_ui::testing::row_measures(fixture.ctx());
    assert!(measures.len() == NAMES.len() && measures.iter().all(|(_, count)| *count == 1), "each row is measured once: {measures:?}");
    let builds = craft_ui::testing::row_builds(fixture.ctx());
    assert!(builds.len() == NAMES.len() && builds.iter().all(|(_, count)| *count <= 2), "each row is built at most twice: {builds:?}");
    let first = (list(&fixture), [row(&fixture, 0), row(&fixture, 1), row(&fixture, 2)]);
    fixture.idle(3);
    let settled = (list(&fixture), [row(&fixture, 0), row(&fixture, 1), row(&fixture, 2)]);
    assert_eq!(first, settled, "the first frame places the rows and the list as the settled list does");

    let at = settled.1[1].center();
    fixture.frame(vec![moved(at)]);
    fixture.frame(vec![press(at)]);
    fixture.frame(vec![release(at)]);
    let clicked = (list(&fixture), row(&fixture, 1));
    assert!(clicked.1.height() > settled.1[1].height(), "the selected row takes its taller size in the frame of the click's release");
    fixture.idle(2);
    assert_eq!(clicked, (list(&fixture), row(&fixture, 1)), "the frame of the click shows the list and the row at their settled sizes");

    let below = row(&fixture, 1).top();
    request_rename(fixture.ctx(), list_id(), &0_usize);
    fixture.idle(1);
    let renamed = (list(&fixture), row(&fixture, 1).top());
    assert!(renamed.1 > below, "the renaming row takes its taller size in the frame the rename starts");
    fixture.idle(2);
    assert_eq!(renamed, (list(&fixture), row(&fixture, 1).top()), "the frame the rename starts shows the list and the rows settled");
}

/// PageDown on a segmented control in a row of a capped list, whose own roving group leaves the key untaken, pages the
/// list's rows: the list is the scroll owner of its rows, and the move comes to it.
#[trial]
fn page_down_on_a_control_in_a_row_pages_the_capped_list() {
    use craft_ui::choice::Segmented;
    use craft_ui::contract::{Bind, Entry, Sizing};
    use craft_ui::testing::focus::AccessActionKind;
    use egui::{Key, Modifiers};
    const ROWS: usize = 40;
    let list_id = Id::NULL.with("paged-rows");
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..ROWS).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let sizes: Vec<Vec<Entry<u8>>> =
            keys.iter().map(|key| vec![Entry::new(1, format!("Small {key}")), Entry::new(2, format!("Large {key}"))]).collect();
        let list = List::new("Layers").id(list_id).sizing(Sizing::Content).max_rows(6);
        list.show_items(ui, &source, &mut |key, _| {
            let size = Segmented::new(format!("Size {key}"), Bind::given(1_u8), &sizes[*key]);
            ListRow::new(format!("Layer {key}")).trailing_field(size, |_| {})
        });
    });
    fixture.room(Vec2::new(400.0, 400.0)).settle();
    let in_view = |fixture: &Fixture, name: &str| {
        let list = fixture.node_of(list_id).expect("the list's node").rect;
        fixture.by_label(name).is_some_and(|row| list.expand(0.5).contains_rect(row.rect))
    };
    assert!(in_view(&fixture, "Layer 0") && !in_view(&fixture, "Layer 8"), "the list starts at its first rows");
    let segment = fixture.by_label("Small 0").expect("the first row's segment");
    fixture.access(AccessActionKind::Focus, &segment).settle();
    assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), segment.node, "the first row's segment took the focus");
    fixture.key(Key::PageDown, Modifiers::NONE).settle();
    assert!(!in_view(&fixture, "Layer 0") && in_view(&fixture, "Layer 8"), "PageDown on the row's control paged the rows");
}

/// The height of a thumbnail row and how far a nested row's label starts after a top-level one's, under `rows`.
fn thumb_row_geometry(rows: craft_ui::list::ThumbRows) -> (f32, f32) {
    use craft_ui::list::ThumbSize;
    let mut fixture = Fixture::new(|ui| {
        let keys = [0_usize, 1];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Layers").show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]).depth(*key).thumb(ThumbSize::Medium, |_, _| {}));
    });
    fixture.root_value(rows).room(Vec2::new(300.0, 200.0)).settle();
    let label_x = |name: &str| fixture.texts().iter().find(|text| text.text == name).expect("the row paints its label").rect.left();
    let height = fixture.by_label(NAMES[0]).expect("the top-level row").rect.height();
    (height, label_x(NAMES[1]) - label_x(NAMES[0]))
}

/// A list under `ThumbRows::Tight` (VectorCraft's Layers panel) stands its thumbnail rows lower, the thumbnail with a
/// tight gap and no control-and-padding floor, and indents a level by less than an icon slot.
#[trial]
fn tight_thumbnail_rows_stand_lower_and_indent_less() {
    use craft_ui::list::ThumbRows;
    let ((roomy_height, roomy_indent), (tight_height, tight_indent)) = (thumb_row_geometry(ThumbRows::Roomy), thumb_row_geometry(ThumbRows::Tight));
    assert!(tight_height < roomy_height, "the tight row ({tight_height}) is lower than the roomy one ({roomy_height})");
    assert!(0.0 < tight_indent && tight_indent < roomy_indent, "a level indents less ({tight_indent}) than an icon slot ({roomy_indent})");
}
