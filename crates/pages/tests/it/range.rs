//! The page range field's menu of the app's page choices, and the page-range row of a parameter dialog.

use craft_pages::ui::{PageRangeChoices, PageRangeField, PageRangeOptions};
use craft_pages::{PageRange, PageScope, Subset};
use craft_ui::contract::{Bind, Entry};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamRows, set_param_field};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// The menu button lists the app's entries and a pick reports their pages; a field given no entries has no button.
#[trial]
fn the_menu_lists_the_apps_entries_and_a_field_without_entries_has_no_button() {
    let picked: Rc<RefCell<Option<Vec<usize>>>> = Rc::default();
    let seen = picked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let menu = [Entry::new(vec![0, 2, 4], "Odd pages only"), Entry::new(vec![1, 3], "Even pages only")];
        if let Some(pages) = PageRangeField::new("Pages", &[], 5, 0).menu(&menu).show(ui).picked {
            *seen.borrow_mut() = Some(pages);
        }
    });
    fixture.settle();
    let button = fixture.by_label("Select pages").expect("the field has a menu button");
    fixture.click(&button).settle();
    let odd = fixture.by_label("Odd pages only").expect("the menu lists the app's entries");
    assert!(fixture.by_label("Even pages only").is_some(), "every entry is listed");
    fixture.click(&odd).settle();
    assert_eq!(*picked.borrow(), Some(vec![0, 2, 4]), "the pick reports the entry's pages");

    let mut bare = Fixture::new(|ui| {
        let _ = PageRangeField::new("Pages", &[], 5, 0).show(ui);
    });
    bare.settle();
    assert!(bare.by_label("Select pages").is_none(), "no entries, no button");
}

/// The page-range row of a parameter dialog, over a document of 12 pages.
fn page_range_row() -> Vec<Param> {
    vec![PageRangeOptions::param("pages", "Page range", 12, 2, PageRangeChoices::FromTo)]
}

/// The page-range row keeps the range the user picks in the dialog's fields as JSON and reads back what automation
/// sets there; an absent or unreadable value is no range. A slip in the stored form loses the user's range silently.
#[trial]
fn a_page_range_row_keeps_its_range_as_json_and_reads_it_back() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let rows = page_range_row();
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Form", &rows, Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();

    let range = fixture.by_label("Pages from").expect("the from-to radio");
    fixture.click(&range).settle();
    let to = fixture.by_label("to").expect("the To field");
    fixture.click(&to).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("5").key(Key::Enter, Modifiers::NONE).settle();
    assert!(matches!(values.borrow().get("pages"), Some(FieldValue::Json(_))), "the row keeps its range as JSON");
    let typed = PageRangeOptions::value(&values.borrow(), "pages").expect("the row's value reads back as a range");
    assert_eq!((typed.scope, typed.from, typed.to), (PageScope::Range, 1, Some(5)), "the typed To lands in the fields");
    assert_eq!(typed.pages(12, 2, &[]), vec![0, 1, 2, 3, 4]);

    let set = PageRange { scope: PageScope::Range, from: 3, to: Some(7), subset: Subset::Odd };
    let automation = json!({"scope": "range", "from": 3, "to": 7, "subset": "odd"});
    set_param_field(&page_range_row(), &mut values.borrow_mut(), "pages", automation).expect("the row takes a JSON value");
    fixture.settle();
    assert_eq!(PageRangeOptions::value(&values.borrow(), "pages"), Some(set), "a range set by automation reads back");
    let stored = DialogFields::from_pairs([("pages", PageRangeOptions::field_value(&set))]);
    assert_eq!(PageRangeOptions::value(&stored, "pages"), Some(set), "the row's stored form reads back as the same range");

    assert_eq!(PageRangeOptions::value(&DialogFields::new(), "pages"), None, "no value is no range");
    set_param_field(&page_range_row(), &mut values.borrow_mut(), "pages", json!("all pages")).expect("a custom row keeps any JSON");
    fixture.settle();
    assert_eq!(PageRangeOptions::value(&values.borrow(), "pages"), None, "a value that is not a range reads as none");
}

/// Editing the end of a range must not silently rewrite its start, even when the range is temporarily reversed.
#[trial]
fn editing_to_preserves_from() {
    let values = Rc::new(RefCell::new(DialogFields::from_pairs([(
        "pages",
        PageRangeOptions::field_value(&PageRange { scope: PageScope::Range, from: 8, to: Some(12), subset: Subset::All }),
    )])));
    let shown = values.clone();
    let rows = page_range_row();
    let mut fixture = Fixture::new(move |ui| {
        let _ = ParamRows::new("Form", &rows, Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    let to = fixture.by_label("to").expect("the To field");
    fixture.click(&to).key(Key::A, Modifiers::COMMAND).type_text("5").key(Key::Enter, Modifiers::NONE).settle();
    let range = PageRangeOptions::value(&values.borrow(), "pages").expect("the range");
    assert_eq!(range.from, 8, "changing To must preserve From");
    assert_eq!(range.to, Some(5));
    assert_eq!(range.pages(12, 2, &[]), vec![7], "a committed end below From still selects the starting page");
}

/// In a document dialog the page range options read as production's: the heading over a column of radio buttons, the
/// page fields ending in "of N", and no Subset where the dialog keeps it in a row of its own.
#[trial]
fn the_page_range_options_of_a_document_dialog_stack_under_their_heading_and_count_the_pages() {
    use craft_ui::patterns::form_dialogs::{ParamDialog, ParamLayout};
    let mut fixture = Fixture::new(|ui| {
        let mut fields = DialogFields::new();
        ParamDialog::new("range", "Rotate", vec![PageRangeOptions::param("pages", "Pages", 12, 2, PageRangeChoices::SelectedFromTo)])
            .layout(ParamLayout::Document)
            .values(Bind::bound(&mut fields))
            .show(ui);
    });
    fixture.room(egui::vec2(1100.0, 700.0));
    fixture.settle();
    let at = |name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} is shown")).rect;
    let (all, selected, from) = (at("All pages"), at("Selected pages"), at("Pages from"));
    assert!(selected.min.y > all.max.y - 2.0 && from.min.y > selected.max.y - 2.0, "the radios stack: {all:?} {selected:?} {from:?}");
    assert!(fixture.texts().iter().any(|run| run.text == "of 12"), "the range ends in the page count");
    assert!(fixture.by_label("Subset").is_none(), "this arrangement has no Subset");
}

/// Production's Background and Watermark dialogs put the page range options on one line: All pages, Pages from and
/// to, then the Subset, with no page count after the range.
#[trial]
fn line_page_range_options_stand_on_one_line_with_no_page_count() {
    use craft_ui::patterns::form_dialogs::{ParamDialog, ParamLayout};
    let mut fixture = Fixture::new(|ui| {
        let mut fields = DialogFields::new();
        ParamDialog::new("range", "Add Background", vec![PageRangeOptions::param("range", "Page Range Options", 3, 0, PageRangeChoices::Line)])
            .layout(ParamLayout::Document)
            .values(Bind::bound(&mut fields))
            .show(ui);
    });
    fixture.room(egui::vec2(1100.0, 700.0));
    fixture.settle();
    let at = |name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} is shown")).rect;
    let (all, from, subset) = (at("All pages"), at("Pages from"), at("Subset"));
    for (name, rect) in [("Pages from", from), ("Subset", subset)] {
        assert!((rect.center().y - all.center().y).abs() < 4.0, "{name} {rect:?} stands on the line of All pages {all:?}");
    }
    assert!(all.max.x <= from.min.x && from.max.x <= subset.min.x, "in reading order: {all:?} {from:?} {subset:?}");
    assert!(!fixture.texts().iter().any(|run| run.text == "of 3"), "the range shows no page count");
}

/// Production's Rotate Pages lines its captions up in one column with the controls after it: the Pages: caption
/// stands beside the page range group, not over it, and the group's options start where the Direction: list does.
/// Without the column the group's caption heads it.
#[trial]
fn a_caption_column_puts_a_group_caption_beside_its_group_and_lines_the_controls_up() {
    use craft_ui::patterns::form_dialogs::{ParamDialog, ParamKind, ParamLayout};
    let shown = |column: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let mut fields = DialogFields::new();
            let direction =
                ParamKind::choice(vec![("90".into(), "Clockwise 90 degrees".into()), ("180".into(), "180 degrees".into())], Some("90".into()));
            let rows = vec![
                Param::new("direction", "Direction:", direction),
                PageRangeOptions::param("pages", "Pages:", 12, 2, PageRangeChoices::SelectedFromTo),
            ];
            ParamDialog::new("range", "Rotate", rows).layout(ParamLayout::Document).caption_column(column).values(Bind::bound(&mut fields)).show(ui);
        });
        fixture.room(egui::vec2(1100.0, 700.0));
        fixture.settle();
        let caption = fixture.texts().iter().find(|run| run.text.starts_with("Pages") && !run.text.contains("from")).map(|run| run.rect);
        let caption =
            caption.unwrap_or_else(|| panic!("the Pages caption shows: {:?}", fixture.texts().iter().map(|run| &run.text).collect::<Vec<_>>()));
        let all = fixture.by_label("All pages").expect("All pages shows").rect;
        let direction = fixture.by_label("Direction:").or_else(|| fixture.by_label("Direction")).expect("the Direction list shows").rect;
        (caption, all, direction)
    };
    let (caption, all, direction) = shown(true);
    assert!(caption.max.x <= all.min.x && caption.min.y >= all.min.y - 4.0, "the caption {caption:?} stands beside the group {all:?}");
    assert!((all.min.x - direction.min.x).abs() <= 2.0, "the group {all:?} starts where the Direction list {direction:?} does");
    let (caption, all, _) = shown(false);
    assert!(caption.max.y <= all.min.y + 1.0, "without the column the caption {caption:?} heads the group {all:?}");
}
