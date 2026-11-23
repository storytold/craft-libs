//! SliderField forwards an opt-in number codec without changing the default parser or numeric stepping.

use craft_behavior::{MotionMode, NumberCodec};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::form::SliderField;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

fn typed(codec: Option<NumberCodec>, text: &str, step: f64) -> (Fixture, Rc<RefCell<f64>>) {
    let value = Rc::new(RefCell::new(40.0));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut value = held.borrow_mut();
        let field = SliderField::new("Mapped", Bind::bound(&mut *value), 0.0..=100.0).step(step);
        let field = match &codec {
            Some(codec) => field.number_codec(codec.clone()),
            None => field,
        };
        field.show(ui);
    });
    fixture.motion(MotionMode::Off).settle();
    let field = fixture.by_role(Role::SPIN_BUTTON).first().copied().expect("number field");
    fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
    (fixture, value)
}

#[trial]
fn codec_override_slider_field_reads_absolute_plus_without_changing_default_relative_plus() {
    let (_, default) = typed(None, "+12", 1.0);
    assert_eq!(*default.borrow(), 52.0);
    let (mut fixture, value) = typed(Some(NumberCodec::default().parse_with(|text| text.parse().ok())), "+12", 1.0);
    assert_eq!(*value.borrow(), 12.0);
    let slider = fixture.by_role(Role::SLIDER).first().copied().expect("slider");
    assert_eq!(fixture.tree().numeric_of(slider.node).expect("numeric slider").value, Some(12.0));
    fixture.access(AccessActionKind::Focus, &slider).key(Key::ArrowRight, Modifiers::NONE).settle();
    assert_eq!(*value.borrow(), 13.0, "the typed domain value keeps model-unit nudges");
    let (_, stepped) = typed(Some(NumberCodec::default().parse_with(|text| text.parse().ok())), "12", 5.0);
    assert_eq!(*stepped.borrow(), 10.0, "typed values still snap to the slider field's step");
}

#[trial]
fn codec_override_slider_field_keeps_rejected_input_from_the_model() {
    for text in ["1+2", "*2", "Auto", "NaN", "inf", "1.2.3", "1,2.3", ""] {
        let codec = NumberCodec::default().or_parse(|_| Some(77.0)).parse_with(|text| text.parse().ok());
        let (_, value) = typed(Some(codec), text, 1.0);
        assert_eq!(*value.borrow(), 40.0, "rejected whole input {text:?} must not land or fall through");
    }
}

#[trial]
fn codec_override_slider_field_retains_range_limits_with_a_domain_reader() {
    let codec = NumberCodec::default().parse_with(|text| text.strip_suffix(" widgets")?.parse().ok());
    let (_, value) = typed(Some(codec.clone()), "1000 widgets", 1.0);
    assert_eq!(*value.borrow(), 100.0, "typed domain values are still range-limited");
    let (_, value) = typed(Some(codec), "7 widgets", 1.0);
    assert_eq!(*value.borrow(), 7.0, "the complete custom grammar reaches the real field");
}
