use egui::accesskit::Role;
use egui::{DragValue, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

#[test]
fn formats_and_reads_the_regions_numbers() {
    let de = LocaleNumbers::from_tag("de");
    assert_eq!(de.format(1.5, 0..=6), "1,5");
    assert_eq!(de.format(1234567.0, 0..=2), "1.234.567");
    assert_eq!(de.format(2.0, 2..=4), "2,00");
    assert_eq!(de.format(-0.001, 0..=2), "0");
    assert_eq!(de.parse("1,5"), Ok(1.5));
    assert_eq!(de.parse("1.234.567"), Ok(1234567.0));
    assert!(de.parse("1.234.").is_err_and(|e| incomplete("1.234.", e)), "a grouping mark still to be followed by digits");
    assert_eq!(de.parse("1.5"), Ok(1.5), "a lone point that can't be grouping");

    let en = LocaleNumbers::from_tag("en-US");
    assert_eq!(en.format(1234567.25, 0..=2), "1,234,567.25");
    assert_eq!(en.parse("1,234,567.25"), Ok(1234567.25));
    assert_eq!(en.parse("１２，３４５．５"), Ok(12345.5), "full-width digits and marks");
    assert_eq!(en.parse("1,5,5"), Err(NumberError::Separator));

    let ar = LocaleNumbers::from_tag("ar-EG");
    assert_eq!(ar.format(1234.5, 1..=1), "١٬٢٣٤٫٥");
    assert_eq!(ar.parse("١٬٢٣٤٫٥"), Ok(1234.5));
    assert_eq!(ar.parse(&ar.format(-12.5, 0..=2)), Ok(-12.5), "the sign and bidi marks read back");
    assert_eq!(ar.parse("12.5"), Ok(12.5));

    let fa = LocaleNumbers::from_tag("fa-IR");
    assert_eq!(fa.format(1.5, 0..=2), "۱٫۵");
    assert_eq!(fa.parse("۱٫۵"), Ok(1.5));
    assert_eq!(fa.parse("1۵"), Err(NumberError::MixedDigits));
}

#[test]
fn a_region_reads_back_its_own_decimal_mark() {
    // English in the US with a decimal comma and no grouping from the Windows settings.
    let mut region = craft_i18n::Region::from_tag("en-US");
    region.decimal = Some(',');
    let numbers = LocaleNumbers::new(Arc::new(Format::new(&region, craft_i18n::Digits::Locale)));
    let written = numbers.format(1234.5, 1..=1);
    assert_eq!((written.as_str(), numbers.parse(&written)), ("1.234,5", Ok(1234.5)));
    assert_eq!(numbers.parse("1.234"), Ok(1234.0), "the grouping moved to the point");
    region.grouping = Some(None);
    let numbers = LocaleNumbers::new(Arc::new(Format::new(&region, craft_i18n::Digits::Locale)));
    assert_eq!(numbers.format(1234.5, 1..=1), "1234,5");
}

#[test]
fn units_wrap_and_strip() {
    let unit = Unit::new("%{n}");
    assert_eq!((unit.prefix(), unit.suffix()), ("%", ""));
    let px = Unit::new("{n} px");
    assert_eq!(px.strip(" 12,5 px "), "12,5");
    assert_eq!(px.strip("12,5"), "12,5");
    let (i18n, _) = I18n::new(&[("fr", "unit\t{n} px\t{n} pt-écran\n", "")], &["fr"]);
    let fr = i18n.from_code("fr").unwrap_or(Lang::EN);
    assert_eq!(Unit::translated(&i18n, fr, "{n} px").suffix(), " pt-écran");
    assert_eq!(Unit::translated(&i18n, Lang::EN, "{n} px").suffix(), " px");
}

fn drag_harness(tag: &str, value: f64) -> Harness<'static, f64> {
    let numbers = LocaleNumbers::from_tag(tag);
    drag_harness_with(value, move |ui, value| LocalizedDragValue::new(DragValue::new(value).speed(0.1), &numbers).ui(ui))
}

fn drag_harness_with(value: f64, add: impl FnMut(&mut Ui, &mut f64) -> Response + 'static) -> Harness<'static, f64> {
    let mut add = add;
    // The default fonts have no Arabic or Persian digits; the fonts module adds them in apps.
    let mut h = Harness::builder().allow_missing_glyphs().build_ui_state(
        move |ui, value: &mut f64| {
            let label = ui.label("Width");
            add(ui, value).labelled_by(label.id);
        },
        value,
    );
    h.run();
    h
}

/// Clicks the drag value into text editing, types `text` over its selected text and presses Enter.
fn type_into_drag(h: &mut Harness<'_, f64>, text: &str) {
    h.get_by_role(Role::SpinButton).click();
    h.run();
    h.get_by_role(Role::SpinButton).type_text(text);
    h.run();
    h.key_press(Key::Enter);
    h.run();
}

#[test]
fn drag_value_shows_and_takes_german_decimals() {
    let mut h = drag_harness("de", 1.5);
    assert_eq!(h.get_by_role(Role::SpinButton).value().as_deref(), Some("1,5"));
    type_into_drag(&mut h, "2,25");
    assert_eq!(*h.state(), 2.25);
    assert_eq!(h.get_by_role(Role::SpinButton).value().as_deref(), Some("2,25"));
}

#[test]
fn drag_value_in_arabic_digits() {
    let mut h = drag_harness("ar-EG", 1.5);
    assert_eq!(h.get_by_role(Role::SpinButton).value().as_deref(), Some("١٫٥"));
    type_into_drag(&mut h, "٢٬٥٠٠٫٥");
    assert_eq!(*h.state(), 2500.5);
}

#[test]
fn drag_value_keeps_the_apps_text_around_the_number() {
    let numbers = LocaleNumbers::from_tag("de");
    let mut h = drag_harness_with(0.0, move |ui, value| {
        LocalizedDragValue::new(DragValue::new(value).prefix("x: ").suffix(" °"), &numbers)
            .or_format(|v, _| (v == 0.0).then(|| "Auto".to_owned()))
            .or_parse(|t| (t.trim() == "Auto").then_some(0.0))
            .ui(ui)
    });
    assert_eq!(h.get_by_role(Role::SpinButton).value().as_deref(), Some("x: Auto °"));
    type_into_drag(&mut h, "2,5");
    assert_eq!(*h.state(), 2.5);
    assert_eq!(h.get_by_role(Role::SpinButton).value().as_deref(), Some("x: 2,5 °"));

    let numbers = LocaleNumbers::from_tag("de");
    let h = drag_harness_with(1.5, move |ui, value| {
        LocalizedDragValue::new(DragValue::new(value).prefix("x: ").suffix(" °"), &numbers).unit(Unit::new("{n} px")).ui(ui)
    });
    let shown = h.get_by_role(Role::SpinButton).value().unwrap_or_default();
    assert!(shown.contains("x: 1,5 °") && shown.trim_end().ends_with("px"), "a unit goes outside them: {shown:?}");
}

#[test]
fn drag_value_takes_range_speed_and_no_grouping_after_wrapping() {
    let numbers = LocaleNumbers::from_tag("en-US");
    let mut h = drag_harness_with(1920.0, move |ui, value| {
        LocalizedDragValue::new(DragValue::new(value), &numbers).range(1..=4096).speed(1.0).ungrouped().unit(Unit::new("{n} px")).ui(ui)
    });
    let shown = h.get_by_role(Role::SpinButton).value().unwrap_or_default();
    assert!(shown.starts_with("1920"), "{shown:?}");
    type_into_drag(&mut h, "10,000");
    assert_eq!(*h.state(), 4096.0, "grouped text reads back and is clamped to the range");
    assert_eq!(LocaleNumbers::from_tag("de").ungrouped().format(1234567.5, 0..=1), "1234567,5");
}

#[test]
fn drag_value_marks_text_it_cannot_read() {
    let mut h = drag_harness("de", 1.5);
    h.get_by_role(Role::SpinButton).click();
    h.run();
    h.get_by_role(Role::SpinButton).type_text("1x");
    h.run();
    assert!(h.query_by_label("Not a number").is_some(), "marked while editing");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(*h.state(), 1.5, "the value stays");
    assert!(h.query_by_label("Not a number").is_some(), "still marked after reverting");
    assert_eq!(h.get_by_role(Role::SpinButton).value().as_deref(), Some("1,5"));
}

#[test]
fn drag_value_does_not_mark_a_number_being_typed() {
    let mut h = drag_harness("de", 1.5);
    h.get_by_role(Role::SpinButton).click();
    h.run();
    h.get_by_role(Role::SpinButton).type_text("1.");
    h.run();
    assert!(h.query_by_label("Check the decimal and grouping marks").is_none());
}

fn field_harness(tag: &str, value: f64) -> Harness<'static, f64> {
    let numbers = LocaleNumbers::from_tag(tag);
    let mut h = Harness::builder().allow_missing_glyphs().build_ui_state(
        move |ui, value: &mut f64| {
            let label = ui.label("Gap");
            ui.add(NumberField::new(value, &numbers).range(0.0..=10000.0).step(0.5).unit(Unit::new("{n} mm"))).labelled_by(label.id);
        },
        value,
    );
    h.run();
    h
}

fn type_into_field(h: &mut Harness<'_, f64>, text: &str) {
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role(Role::TextInput).type_text(text);
    h.run();
    h.key_press(Key::Enter);
    h.run();
}

#[test]
fn number_field_reads_on_commit() {
    let mut h = field_harness("de", 1.5);
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("1,5"));
    type_into_field(&mut h, "1.234,5 mm");
    assert_eq!(*h.state(), 1234.5);
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("1.234,5"));
    type_into_field(&mut h, "20000");
    assert_eq!(*h.state(), 10000.0, "clamped to the range");
}

#[test]
fn number_field_takes_full_width_and_persian_digits() {
    let mut h = field_harness("en", 1.0);
    type_into_field(&mut h, "１２．５");
    assert_eq!(*h.state(), 12.5);
    let mut h = field_harness("fa", 1.0);
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("۱"));
    type_into_field(&mut h, "۱۲٫۵");
    assert_eq!(*h.state(), 12.5);
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("۱۲٫۵"));
}

#[test]
fn number_field_keeps_text_it_cannot_read() {
    let mut h = field_harness("de", 1.5);
    type_into_field(&mut h, "1,5,5");
    assert_eq!(*h.state(), 1.5, "the value stays");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("1,5,5"), "the text stays to be fixed");
    type_into_field(&mut h, "2,5");
    assert_eq!(*h.state(), 2.5);
}

#[test]
fn number_field_rounds_before_clamping() {
    let mut h = field_harness("de", 1.5);
    type_into_field(&mut h, "9999,99999999");
    assert_eq!(*h.state(), 10000.0, "rounded to six decimals, then clamped");
    let numbers = LocaleNumbers::from_tag("de");
    let mut h = Harness::builder().build_ui_state(
        move |ui, value: &mut i32| {
            let label = ui.label("Count");
            ui.add(NumberField::new(value, &numbers).range(-10.0..=10.0)).labelled_by(label.id);
        },
        0,
    );
    h.run();
    for (text, want) in [("2,7", 3), ("2,5", 3), ("-0,5", -1), ("-0,4", 0), ("9,6", 10), ("10,5", 10)] {
        h.get_by_role(Role::TextInput).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run();
        h.get_by_role(Role::TextInput).type_text(text);
        h.run();
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(*h.state(), want, "{text}");
    }
}

#[test]
fn number_field_keeps_a_value_it_shows_rounded_until_it_is_edited() {
    let mut h = field_harness("en", 1.23456789);
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("1.234568"));
    type_into_field(&mut h, "1.234568");
    assert_eq!(*h.state(), 1.23456789, "the text it shows, typed back");
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("9");
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(*h.state(), 1.23456789, "Escape");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("1.234568"));
    type_into_field(&mut h, "1.2345678");
    assert_eq!(*h.state(), 1.234568, "an edit is rounded to the decimals shown");
}

#[test]
fn steps_go_to_multiples_of_the_step() {
    let (none, shift, cmd) = (Modifiers::NONE, Modifiers::SHIFT, Modifiers::COMMAND);
    assert_eq!(stepped(5.1, 1.0, 6, none, 1.0), 6.0);
    assert_eq!(stepped(5.1, 1.0, 6, none, -1.0), 5.0);
    assert_eq!(stepped(5.0, 1.0, 6, none, -2.0), 3.0);
    assert_eq!(stepped(5.4, 1.0, 6, shift, 1.0), 15.0, "Shift rounds to the step, then adds ten");
    assert_eq!(round_half_away(stepped(1.23, 1.0, 2, cmd, 1.0), 2), 1.3, "a tenth with Ctrl/Cmd");
    assert_eq!(stepped(5.1, 1.0, 0, cmd, 1.0), 6.0, "a count has no tenths");
    assert_eq!(stepped(4.9999999, 1.0, 6, none, 1.0), 6.0, "within a millionth of a multiple is on it");
}

fn field_with(value: f64, add: impl FnMut(&mut Ui, &mut f64) -> Response + 'static) -> Harness<'static, f64> {
    let mut add = add;
    let mut h = Harness::builder().allow_missing_glyphs().build_ui_state(
        move |ui, value: &mut f64| {
            let label = ui.label("Gap");
            add(ui, value).labelled_by(label.id);
        },
        value,
    );
    h.run();
    h
}

#[test]
fn number_field_steps_with_arrow_keys_and_the_wheel() {
    let mut h = field_harness("de", 1.3);
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(*h.state(), 1.5, "to the next multiple of 0,5");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("1,5"));
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(*h.state(), 2.0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run();
    assert_eq!(*h.state(), 0.0, "ten steps down, clamped");
    h.get_by_role(Role::TextInput).type_text("4");
    h.run();
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("4"), "the stepped text is selected, so typing replaces it");
    let at = h.get_by_role(Role::TextInput).rect().center();
    h.hover_at(at);
    h.run();
    let wheel = |y| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, y),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    };
    h.event(wheel(2.0));
    h.run();
    assert_eq!(*h.state(), 5.0, "two notches of 0,5 up from the typed 4");
}

#[test]
fn number_field_draws_a_recessed_box() {
    let numbers = LocaleNumbers::from_tag("en");
    let mut h = field_with(1.0, move |ui, value| {
        let frame = Recessed { height: 30.0, ..Recessed::from_style(ui.style()) };
        ui.add(NumberField::new(value, &numbers).recessed(frame).desired_width(80.0))
    });
    let rect = h.get_by_role(Role::TextInput).rect();
    assert!((rect.height() - 30.0).abs() < 0.5, "{rect:?}");
    type_into_field(&mut h, "2.5");
    assert_eq!(*h.state(), 2.5);
}

fn scrub_harness(value: f64) -> Harness<'static, f64> {
    let numbers = LocaleNumbers::from_tag("en");
    let mut h = Harness::builder().build_ui_state(
        move |ui, value: &mut f64| {
            ui.horizontal(|ui| {
                let label = ui.label("Gap");
                ui.add(NumberField::new(value, &numbers).range(0.0..=100.0).scrub_label(&label));
            });
        },
        value,
    );
    h.run();
    h
}

#[test]
fn dragging_the_label_scrubs_the_field() {
    let mut h = scrub_harness(5.4);
    let at = h.get_by_role(Role::Label).rect().center();
    let drag = |h: &mut Harness<'_, f64>, by: &[f32]| {
        h.hover_at(at);
        h.run();
        h.drag_at(at);
        h.run();
        for dx in by {
            h.hover_at(at + egui::vec2(*dx, 0.0));
            h.run();
        }
    };
    drag(&mut h, &[20.0, 40.0]);
    h.drop_at(at + egui::vec2(40.0, 0.0));
    h.run();
    assert_eq!(*h.state(), 25.0, "20 steps of 2 points from 5,4: the first to 6");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("25"));

    drag(&mut h, &[20.0, -20.0]);
    assert_eq!(*h.state(), 15.0);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(*h.state(), 25.0, "Escape puts the value back");
    h.drop_at(at);
    h.run();

    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
    h.get_by_role(Role::TextInput).type_text("7");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(*h.state(), 7.0, "a click on the label focuses the field with its text selected");
}

#[test]
fn a_click_on_the_label_keeps_a_value_it_shows_rounded() {
    let mut h = scrub_harness(1.23456789);
    let at = h.get_by_role(Role::Label).rect().center();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(*h.state(), 1.23456789);
}

#[derive(Default)]
struct Mixed {
    value: f64,
    mixed: bool,
    changes: usize,
}

#[test]
fn a_mixed_field_is_blank_until_a_number_is_entered() {
    let numbers = LocaleNumbers::from_tag("en");
    let mut h = Harness::builder().build_ui_state(
        move |ui, m: &mut Mixed| {
            let label = ui.label("Gap");
            let r = ui.add(NumberField::new(&mut m.value, &numbers).mixed(m.mixed).placeholder("Mixed")).labelled_by(label.id);
            if r.changed() {
                (m.changes, m.mixed) = (m.changes + 1, false);
            }
        },
        Mixed { value: 3.0, mixed: true, changes: 0 },
    );
    h.run();
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some(""));
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.key_press(Key::ArrowUp);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!((h.state().value, h.state().changes), (3.0, 0), "blank commits and arrow keys change nothing");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some(""));
    type_into_mixed(&mut h, "3");
    assert_eq!((h.state().value, h.state().changes), (3.0, 1), "the value it had is still a change: it goes to every object");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("3"));
}

fn type_into_mixed(h: &mut Harness<'_, Mixed>, text: &str) {
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text(text);
    h.run();
    h.key_press(Key::Enter);
    h.run();
}

#[test]
fn the_first_field_of_a_group_takes_the_focus() {
    let numbers = LocaleNumbers::from_tag("en");
    let mut opened = false;
    let mut h = Harness::builder().build_ui_state(
        move |ui, values: &mut [f64; 2]| {
            if !opened {
                focus_first_field(ui.ctx(), true);
                opened = true;
            }
            for (value, name) in values.iter_mut().zip(["Width", "Height"]) {
                let label = ui.label(name);
                ui.add(NumberField::new(value, &numbers)).labelled_by(label.id);
            }
        },
        [12.5, 40.0],
    );
    h.run();
    h.event(egui::Event::Text("7".to_owned()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(*h.state(), [7.0, 40.0], "the first field was focused with its text selected");
}

#[test]
fn error_tooltips_come_from_the_catalog() {
    let (i18n, _) = I18n::new(&[("fr", "\tNot a number\tPas un nombre\n", "")], &["fr"]);
    let fr = i18n.from_code("fr").unwrap_or(Lang::EN);
    let numbers = LocaleNumbers::from_tag("fr").with_catalog(&i18n, fr);
    assert_eq!(numbers.describe(NumberError::Unexpected('x')), "Pas un nombre");
    assert_eq!(numbers.describe(NumberError::TooLarge), "Number too large", "English where the row is missing");
}

const PRECISE: f64 = 1.234_567_89;

/// A harness with the widget `add` makes and a button after it to take the focus.
fn with_other(add: impl Fn(&mut Ui, &mut f64) -> Response + 'static) -> Harness<'static, f64> {
    let mut h = Harness::builder().allow_missing_glyphs().build_ui_state(
        move |ui, value: &mut f64| {
            let label = ui.label("Width");
            add(ui, value).labelled_by(label.id);
            let _ = ui.button("Other");
        },
        PRECISE,
    );
    h.run();
    h
}

/// Focuses the widget of `role` and leaves it without an edit in each way the user can: a click
/// elsewhere, Enter and Tab. The value must stay exactly as it was.
fn leaving_untouched_keeps_the_value(role: Role, add: impl Fn(&mut Ui, &mut f64) -> Response + Clone + 'static) {
    let mut changed = Vec::new();
    for leave in ["click elsewhere", "Enter", "Tab"] {
        let mut h = with_other(add.clone());
        h.get_by_role(role).click();
        h.run();
        assert!(h.get_by_role(role).value().is_some_and(|v| v.contains("1,234568")), "shown rounded");
        match leave {
            "Enter" => h.key_press(Key::Enter),
            "Tab" => h.key_press(Key::Tab),
            _ => h.get_by_label("Other").click(),
        }
        h.run();
        h.run();
        assert!(!h.get_by_role(role).is_focused(), "{leave} leaves the field");
        if h.state().to_bits() != PRECISE.to_bits() {
            changed.push((leave, *h.state()));
        }
    }
    assert!(changed.is_empty(), "{changed:?}");
}

#[test]
fn leaving_an_unedited_number_field_keeps_its_value() {
    let numbers = LocaleNumbers::from_tag("de");
    let add = move |ui: &mut Ui, value: &mut f64| ui.add(NumberField::new(value, &numbers));
    leaving_untouched_keeps_the_value(Role::TextInput, add.clone());

    let mut h = with_other(add.clone());
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role(Role::TextInput).type_text("2,5");
    h.run();
    h.key_press(Key::Tab);
    h.run();
    assert_eq!(*h.state(), 2.5, "an edit commits");

    let mut h = with_other(add);
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(*h.state(), 2.0, "a step commits");
}

#[test]
fn leaving_an_unedited_drag_value_keeps_its_value() {
    let numbers = LocaleNumbers::from_tag("de");
    let add = move |ui: &mut Ui, value: &mut f64| ui.add(LocalizedDragValue::new(DragValue::new(value).speed(0.1).max_decimals(6), &numbers));
    leaving_untouched_keeps_the_value(Role::SpinButton, add.clone());

    let mut h = with_other(add.clone());
    type_into_drag(&mut h, "2,5");
    assert_eq!(*h.state(), 2.5, "an edit commits");

    let mut h = with_other(add);
    h.get_by_role(Role::SpinButton).click();
    h.run();
    h.key_press(Key::ArrowUp);
    h.run();
    assert_ne!(*h.state(), PRECISE, "a step commits");
}

#[test]
fn the_wheel_scrolls_past_a_field_it_cannot_step() {
    let numbers = LocaleNumbers::from_tag("de");
    let mut h = with_other(move |ui, value| ui.add(NumberField::new(value, &numbers)));
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("1x");
    h.run();
    let at = h.get_by_role(Role::TextInput).rect().center();
    h.hover_at(at);
    h.run();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    });
    let mut wheel_left = false;
    h.step();
    h.ctx.input(|i| wheel_left = i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
    assert!(wheel_left, "the wheel event stays for the panel under the field");
}

#[test]
fn a_nan_value_stays_put_and_takes_an_edit() {
    let numbers = LocaleNumbers::from_tag("en");
    let mut h = Harness::builder().allow_missing_glyphs().build_ui_state(
        move |ui, (value, changes): &mut (f64, usize)| {
            let label = ui.label("Gap");
            if ui.add(NumberField::new(value, &numbers)).labelled_by(label.id).changed() {
                *changes += 1;
            }
        },
        (f64::NAN, 0),
    );
    h.run();
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.run();
    assert!(h.state().0.is_nan());
    assert_eq!(h.state().1, 0, "an untouched NaN is not written back");

    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role(Role::TextInput).type_text("2.5");
    h.run();
    h.run();
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("2.5"), "typed text stays");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(*h.state(), (2.5, 1));
}
