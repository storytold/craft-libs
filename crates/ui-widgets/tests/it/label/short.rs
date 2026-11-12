//! A short caption: the field shows the short text with its colon, and the long label names the control.

use craft_ui::contract::{Bind, LabelKind, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;

const NAME: &str = "Horizontal distortion";

#[trial]
fn a_short_caption_shows_in_place_of_the_label_and_the_label_names_the_control() {
    let mut fixture = Fixture::new(|ui| {
        Field::new(NAME).side(SideLabel::new().kind(LabelKind::Short("H".into()))).show(ui, |ui, _| {
            NumberField::new(NAME, Bind::given(Some(10.0))).show(ui);
        });
    });
    fixture.settle();
    let texts = fixture.texts();
    let caption = texts.iter().find(|run| run.text == "H").expect("the short caption is shown");
    assert!(
        texts.iter().any(|run| run.text.trim() == ":" && run.rect.min.x >= caption.rect.min.x),
        "the short caption takes the colon of a beside text label"
    );
    assert!(texts.iter().all(|run| run.text != NAME), "the long label is not shown");
    let control = fixture.by_label(NAME).expect("the long label names the control");
    assert!(control.rect.min.x >= caption.rect.max.x, "the control follows its caption");
}
