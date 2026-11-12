//! How a row too short for its children shares the shortfall: blank room in fields and selects goes before any
//! content, content made to be cut gives way before labels, and a wrapping text keeps its words whole.

use craft_ui::buttons::{Button, Link};
use craft_ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::primitives::Wrap;
use craft_ui::testing::{Fixture, last_pass_measuring_runs, trial};
use craft_ui::text::TextField;
use egui::{Rect, Vec2};

const ROOM: Vec2 = Vec2::new(800.0, 600.0);
const SLACK: f32 = 0.5;

/// The rect of the widget labelled `label` in a row of `children`, in a room `width` wide.
fn rect_in_row(width: f32, label: &'static str, children: impl Fn(&mut egui::Ui) + 'static) -> Rect {
    let mut fixture = Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| children(ui));
    });
    fixture.room(Vec2::new(width, ROOM.y)).settle();
    fixture.by_label(label).map_or(Rect::NOTHING, |target| target.rect)
}

/// A wrapping text after a wide sibling, which leaves it less room than its widest word, breaks between words only:
/// it is at least as wide as that word, and the row narrows the sibling to make the room.
#[trial]
fn a_wrapping_text_in_a_short_row_is_never_narrower_than_its_widest_word() {
    const WORD: &str = "following";
    const TEXT: &str = "of the following rules:";
    const BUTTON: &str = "A button with a long label";
    let word = rect_in_row(ROOM.x, WORD, |ui| {
        Text::new(WORD).show(ui);
    });
    let button = rect_in_row(ROOM.x, BUTTON, |ui| {
        Button::new(BUTTON).show(ui);
    });
    assert!(word.width() > 0.0 && button.width() > 0.0, "both measure: {word:?}, {button:?}");
    // The button takes all but half the word's width.
    let width = button.width() + word.width() / 2.0;
    let text = rect_in_row(width, TEXT, |ui| {
        Button::new(BUTTON).show(ui);
        Text::new(TEXT).show(ui);
    });
    assert!(text.width() >= word.width() - SLACK, "the text {text:?} is narrower than its widest word {word:?}");
}

/// The rects of a select and a path link that elides, side by side in a labelled field, in a room `width` wide, and
/// whether the select's label was cut.
fn select_and_path(width: f32) -> (Rect, Rect, bool) {
    const PATH: &str = "[Comp Name].[fileExtension]";
    let mut fixture = Fixture::new(|ui| {
        Field::new("Output To").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let entries = [Entry::new(0, "Comp name"), Entry::new(1, "Comp name and frame")];
                Select::new("Templates", Bind::given(0), &entries).show(ui);
                Link::new(PATH).wrap(Wrap::ElidePath).show(ui);
            });
        });
    });
    fixture.room(Vec2::new(width, ROOM.y)).settle();
    let rect = |label: &str| fixture.by_label(label).map_or(Rect::NOTHING, |target| target.rect);
    let label_cut = fixture.texts().iter().any(|text| text.text == "Comp name" && text.elided);
    (rect("Templates"), rect(PATH), label_cut)
}

/// A select beside a path link that elides: in a row too short for both, the select gives only its blank length,
/// then the path gives way and the select's label stays whole.
#[trial]
fn a_path_that_elides_gives_way_before_a_select_beside_it() {
    let (select, path, _) = select_and_path(ROOM.x);
    assert!(select.width() > 0.0 && path.width() > 0.0, "both measure: {select:?}, {path:?}");
    // The select's whole width is missing: more than its blank length.
    let width = path.right() - select.width();
    let (short, cut, label_cut) = select_and_path(width);
    assert!(short.width() < select.width() - SLACK, "the select keeps its blank length {short:?}");
    assert!(!label_cut, "the select's label is cut");
    assert!(cut.right() <= width + SLACK && cut.width() < path.width(), "the path elides inside the room {width}: {cut:?}");
}

/// The rects of a search field and of the count after it, in a row `width` wide, and whether the settled frame ran a
/// measuring run.
fn field_and_count(width: f32) -> (Rect, Rect, bool) {
    const COUNT: &str = "1 of 3";
    let mut fixture = Fixture::new(|ui| {
        Row::new().show(ui, |ui, _| {
            TextField::search("Find", Bind::given(String::from("craft"))).show(ui);
            Text::new(COUNT).show(ui);
        });
    });
    fixture.room(Vec2::new(width, ROOM.y)).settle();
    let count = fixture.texts().iter().find(|text| text.text == COUNT).map_or(Rect::NOTHING, |text| text.rect);
    let measured = !last_pass_measuring_runs(fixture.ctx()).is_empty();
    (fixture.by_label("Find").map_or(Rect::NOTHING, |target| target.rect), count, measured)
}

/// A search field at its field length beside a count (FindBar): a row short by less than the field's blank length
/// narrows the field, and the count stays on one line. A settled frame measures nothing.
#[trial]
fn a_field_gives_its_blank_length_before_a_readout_beside_it_wraps() {
    let (field, count, _) = field_and_count(ROOM.x);
    assert!(field.width() > 0.0 && count.width() > 0.0, "both measure: {field:?}, {count:?}");
    let width = count.right() - count.width() / 2.0;
    let (short, line, measured) = field_and_count(width);
    assert!(short.width() < field.width() - SLACK, "the field keeps its length {short:?}");
    assert!((line.height() - count.height()).abs() <= SLACK && line.width() >= count.width() - SLACK, "the count {line:?} wraps");
    assert!(line.right() <= width + SLACK, "the count ends at {} past the room {width}", line.right());
    assert!(!measured, "a settled frame measures");
}
