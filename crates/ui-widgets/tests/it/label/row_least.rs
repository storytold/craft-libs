//! A field whose control is a row of several controls (a path field and its Browse button) puts the row under its
//! label when the label beside it would leave the row less than its least width.

use craft_theme::Size;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Rect, Vec2};

/// How far two edges may differ and still line up, in points.
const SLACK: f32 = 0.5;
const BROWSE: &str = "Browse…";

/// A field labelled `label` over a fill-width path field and a Browse button, in a room `width` wide.
fn path_field(label: &'static str, width: f32) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        Field::new(label).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                TextField::new(label, Bind::given(String::from("/photos/agents"))).limit(None).sizing(Sizing::FillWidth).show(ui);
                Button::new(BROWSE).show(ui);
            });
        });
    });
    fixture.room(Vec2::new(width, 300.0)).density(Size::Compact);
    fixture.settle();
    fixture
}

fn caption(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text:?} is painted"))
}

/// In a room 240 pt wide, the long label beside the row would leave it less than the path field's and the button's
/// least widths together. The row shows under the label, and the button's text shows whole.
#[trial]
fn a_long_label_puts_a_row_of_controls_under_it_when_the_row_would_fall_below_its_least() {
    const LABEL: &str = "Folder agents can open from";
    let fixture = path_field(LABEL, 240.0);
    let (label, field) = (caption(&fixture, LABEL), fixture.by_label(LABEL).expect("the label names the path field").rect);
    assert!(label.max.y <= field.min.y + SLACK, "the path field shows under its label: {label:?} {field:?}");
    let texts = fixture.texts();
    let browse = texts.iter().find(|run| run.text == BROWSE).unwrap_or_else(|| panic!("{BROWSE:?} shows whole: {texts:?}"));
    assert!(browse.clip.min.x <= browse.rect.min.x + SLACK && browse.clip.max.x >= browse.rect.max.x - SLACK, "Browse shows whole: {browse:?}");
}

/// A short label leaves the row its least width beside it, so the label stays beside the row.
#[trial]
fn a_short_label_stays_beside_a_row_of_controls_that_fits() {
    const LABEL: &str = "Folder";
    let fixture = path_field(LABEL, 240.0);
    let (label, field) = (caption(&fixture, LABEL), fixture.by_label(LABEL).expect("the label names the path field").rect);
    assert!(label.max.x <= field.min.x + SLACK && label.max.y > field.min.y, "the label stays beside the path field: {label:?} {field:?}");
}
