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

/// A hidden label given content shows the content alone in place of the label (a picture for an option), and the
/// label's text still names the control.
#[trial]
fn a_hidden_label_with_content_shows_only_the_content() {
    use craft_ui::choice::{Radio, RadioGroup};
    use craft_ui::contract::FieldLayout;
    use craft_ui::display::Text;
    const SIZE: &str = "Small";
    let mut fixture = Fixture::new(|ui| {
        let mut picked = 0u8;
        RadioGroup::new("Size", Bind::bound(&mut picked)).show(ui, |ui, cx| {
            let side = SideLabel::new().layout(FieldLayout::After).kind(LabelKind::Hidden);
            cx.labelled_with(ui, 1u8, SIZE.into(), side, Radio::new(1u8), Text::new("sample"));
        });
    });
    fixture.settle();
    let texts = fixture.texts();
    let sample = texts.iter().find(|run| run.text == "sample").expect("the content is shown");
    assert!(texts.iter().all(|run| run.text != SIZE), "the label's text is not shown");
    let radio = fixture.by_label(SIZE).expect("the label's text names the control");
    assert!(radio.rect.max.x <= sample.rect.min.x, "the content follows its radio, as a label after it does");
}
