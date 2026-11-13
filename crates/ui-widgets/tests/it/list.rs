//! List rows: the rename draft, the row menu and the trailing parts a row reveals.

use craft_behavior::{EditEvent, Mixed, keyed};
use craft_ui::contract::{CollectionEvent, Props as _, RowHeight, ShowCollection as _, request_rename};
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

/// A row's label band takes the toggle columns it leaves empty next to it, up to the nearest toggle it fills: a row
/// without toggles starts its label at the row's start, a row with the first toggle only starts it inside the second
/// column, and a row with both starts it past them.
#[trial]
fn a_rows_label_takes_the_empty_toggle_columns_next_to_it() {
    use craft_behavior::{ColumnKey, ColumnWidth};
    use craft_ui::contract::{ColumnSpec, ToggleColumn};
    use craft_ui::list::{ToggleCell, ToggleState};
    use std::cell::Cell;
    const INCLUDE: ColumnKey = ColumnKey::new("include");
    const DIALOG: ColumnKey = ColumnKey::new("dialog");
    const NAME: ColumnKey = ColumnKey::new("name");
    const LABELS: [&str; 3] = ["Set", "Action", "Step"];
    let ends = Rc::new(Cell::new((0.0_f32, 0.0_f32)));
    let held = ends.clone();
    let mut fixture = Fixture::new(move |ui| {
        let toggle = |key| ColumnSpec::toggle(key, "", ToggleColumn::default());
        let columns = vec![toggle(INCLUDE), toggle(DIALOG), ColumnSpec::new(NAME, "", ColumnWidth::fill(1.0))];
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let _ = List::new("Actions").columns(columns).show_items(ui, &source, &mut |key, icx| {
            let end = |column| icx.cell(&column).map_or(0.0, |cell| cell.right());
            held.set((end(INCLUDE), end(DIALOG)));
            let check = |column| ToggleCell::check(column, ToggleState::On, "Include {row}");
            let row = ListRow::new(LABELS[*key]).cells(icx.cells());
            match key {
                0 => row,
                1 => row.toggle(check(INCLUDE)),
                _ => row.toggle(check(INCLUDE)).toggle(check(DIALOG)),
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
    assert!(!events.iter().any(|event| matches!(event, CollectionEvent::ContextMenu(_) | CollectionEvent::Selection(_))), "{events:?}");
}

/// A list with the edge press `edge` (none without), and what it reported.
fn edge_rig(edge: bool) -> (Fixture, Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>) {
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
        assert!(!events.iter().any(|event| matches!(event, CollectionEvent::Selection(_))), "no row is picked: {events:?}");
    }
    seen.borrow_mut().clear();
    let second = line(&fixture, NAMES[1]);
    fixture.click_at(second).settle();
    fixture.hold(Modifiers::NONE);
    let events = seen.borrow();
    assert!(!events.iter().any(|event| matches!(event, CollectionEvent::EdgePressed { .. })), "a refused line: {events:?}");
    assert!(events.iter().any(|event| matches!(event, CollectionEvent::Selection(_))), "the row takes the click: {events:?}");
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
    assert!(events.iter().any(|event| matches!(event, CollectionEvent::Selection(_))), "{events:?}");
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
            if let CollectionEvent::Selection(change) = event {
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
        let last_change = events.iter().rposition(|event| matches!(event, CollectionEvent::Selection(_)));
        assert!(last_change.is_some_and(|at| at < activations[0]), "{select:?}: selected before the activation, not after: {events:?}");
    }
}
