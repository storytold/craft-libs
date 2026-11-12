//! How a row too short for its children shares the shortfall: content made to be cut gives way before labels, and a
//! wrapping text keeps its words whole.

use craft_ui::buttons::{Button, Link};
use craft_ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::primitives::Wrap;
use craft_ui::testing::{Fixture, trial};
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

/// The rects of a select and a path link that elides, side by side in a labelled field, in a room `width` wide.
fn select_and_path(width: f32) -> (Rect, Rect) {
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
    (rect("Templates"), rect(PATH))
}

/// A select beside a path link that elides: in a row too short for both, the path gives way first and the select's
/// label stays whole.
#[trial]
fn a_path_that_elides_gives_way_before_a_select_beside_it() {
    let (select, path) = select_and_path(ROOM.x);
    assert!(select.width() > 0.0 && path.width() > 0.0, "both measure: {select:?}, {path:?}");
    // Half the path's width is missing.
    let width = path.right() - path.width() / 2.0;
    let (short, cut) = select_and_path(width);
    assert!((short.width() - select.width()).abs() <= SLACK, "the select keeps its width {select:?}: {short:?}");
    assert!(cut.right() <= width + SLACK && cut.width() < path.width(), "the path elides inside the room {width}: {cut:?}");
}
