//! The Word Paragraph dialog's pages: a check box whose paragraphs differ stays mixed across page switches until clicked;
//! the contextual spacing box.

use craft_behavior::Mixed;
use craft_text::ui::character::{
    AsianTypography, IndentKind, LineBreakOptions, ParagraphDialog, ParagraphFormat, ParagraphPreview, ParagraphTab, SpecialIndent,
};
use craft_ui::contract::Show as _;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn toggled(fixture: &Fixture, label: &str) -> Option<Mixed<bool>> {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.tree().toggled_of(target.node)
}

#[trial]
fn word_paragraph_tabs_leave_mixed_boxes_unset_until_clicked() {
    let format = ParagraphFormat { breaks: Some(LineBreakOptions::default()), asian: Some(AsianTypography::default()), ..ParagraphFormat::default() };
    let dialog = Rc::new(RefCell::new(ParagraphDialog::new(format).word(true).held_ok().tab(ParagraphTab::LineAndPageBreaks)));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);
    assert!(matches!(toggled(&fixture, "Keep with next"), Some(Mixed::Mixed)), "a box whose paragraphs differ is shown mixed");

    let asian = fixture.by_label("Asian Typography").expect("the Asian Typography tab is shown");
    fixture.click(&asian).idle(4);
    assert!(matches!(toggled(&fixture, "Apply Asian line-breaking rules (kinsoku)"), Some(Mixed::Mixed)), "the other page's box is mixed too");
    let breaks = fixture.by_label("Line and Page Breaks").expect("the Line and Page Breaks tab is shown");
    fixture.click(&breaks).idle(4);
    assert!(matches!(toggled(&fixture, "Keep with next"), Some(Mixed::Mixed)), "switching pages does not decide a mixed box");

    let keep = fixture.by_label("Keep with next").expect("the box is shown");
    fixture.click(&keep).idle(4);
    assert!(matches!(toggled(&fixture, "Keep with next"), Some(Mixed::Same(true))), "a click checks the box");

    let ok = fixture.by_label("OK").expect("OK is shown");
    fixture.click(&ok).idle(4);
    let answer = dialog.borrow_mut().take_ok().expect("OK filed its answer");
    let breaks = answer.format.breaks.expect("the page is answered");
    assert_eq!(breaks.keep_with_next, Some(true));
    assert_eq!(breaks.widow_control, None, "a box left alone stays unset");
    assert_eq!(answer.format.asian.expect("the page is answered").kinsoku, None);
}

/// "Don't add space between paragraphs of the same style" under the spacing rows: shown mixed for paragraphs that
/// differ, a click turns it on, and the answer carries it.
#[trial]
fn a_mixed_contextual_spacing_box_clicked_is_on_in_the_answer() {
    const LABEL: &str = "Don't add space between paragraphs of the same style";
    let format = ParagraphFormat { contextual_spacing: Some(None), ..ParagraphFormat::default() };
    let dialog = Rc::new(RefCell::new(ParagraphDialog::new(format).word(true).held_ok()));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);
    assert!(matches!(toggled(&fixture, LABEL), Some(Mixed::Mixed)), "paragraphs that differ show the box mixed");
    let boxed = fixture.by_label(LABEL).expect("the box is shown");
    fixture.click(&boxed).idle(4);
    assert!(matches!(toggled(&fixture, LABEL), Some(Mixed::Same(true))), "a click turns the mixed box on");

    let ok = fixture.by_label("OK").expect("OK is shown");
    fixture.click(&ok).idle(4);
    let answer = dialog.borrow_mut().take_ok().expect("OK filed its answer");
    assert_eq!(answer.format.contextual_spacing, Some(Some(true)), "the answer carries the box");
}

fn retype_field(fixture: &mut Fixture, label: &str, text: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.click(&target).idle(2);
    fixture.key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Tab, Modifiers::NONE).idle(4);
}

#[trial]
fn indents_show_and_switch_between_characters_and_lengths() {
    let format = ParagraphFormat {
        before_text: Some(1.5),
        before_text_kind: IndentKind::Characters,
        after_text: Some(Some(36.0)),
        after_text_kind: IndentKind::Length,
        special: Some(SpecialIndent::FirstLine),
        by: Some(2.0),
        by_kind: IndentKind::Characters,
        char_size: Some(10.5),
        ..ParagraphFormat::default()
    };
    let decimal = Rc::new(Cell::new('.'));
    let seen_decimal = decimal.clone();
    let dialog = Rc::new(RefCell::new(ParagraphDialog::new(format).word(true).held_ok()));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        seen_decimal.set(Env::of(ui).format().separators().0);
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);
    let dec = decimal.get();

    // Character indents show as `ch` without losing their value; lengths show in inches.
    let before = fixture.by_label("Before text").expect("Before text is shown");
    let expected_before = format!("1{dec}5");
    assert!(fixture.tree().value_of(before.node).is_some_and(|v| v.contains(&expected_before)), "Before text shows 1.5 ch");
    let by = fixture.by_label("By").expect("By is shown");
    assert!(fixture.tree().value_of(by.node).is_some_and(|v| v.contains('2')), "By shows 2 ch");

    // Bare number in a characters field stays in characters.
    retype_field(&mut fixture, "By", "3");
    assert_eq!(dialog.borrow().format().by, Some(3.0), "bare number stays in characters");
    assert_eq!(dialog.borrow().format().by_kind, IndentKind::Characters);

    // Length typed in a characters field switches to Length (points), even past 327.67.
    retype_field(&mut fixture, "Before text", "1000 pt");
    assert_eq!(dialog.borrow().format().before_text, Some(1000.0));
    assert_eq!(dialog.borrow().format().before_text_kind, IndentKind::Length);

    // Characters typed in a length field switches to Characters (even past 22 inches), and clamps at 327.67.
    retype_field(&mut fixture, "After text", "100 ch");
    assert_eq!(dialog.borrow().format().after_text, Some(Some(100.0)));
    assert_eq!(dialog.borrow().format().after_text_kind, IndentKind::Characters);

    retype_field(&mut fixture, "After text", "400 ch");
    assert_eq!(dialog.borrow().format().after_text, Some(Some(327.67)));
    assert_eq!(dialog.borrow().format().after_text_kind, IndentKind::Characters);

    // `0 ch` switches to Characters with value 0.0, and `2 ic` parses as characters too.
    retype_field(&mut fixture, "Before text", "0 ch");
    assert_eq!(dialog.borrow().format().before_text, Some(0.0));
    assert_eq!(dialog.borrow().format().before_text_kind, IndentKind::Characters);

    retype_field(&mut fixture, "Before text", &format!("0{dec}5 in"));
    assert_eq!(dialog.borrow().format().before_text, Some(36.0));
    assert_eq!(dialog.borrow().format().before_text_kind, IndentKind::Length);

    retype_field(&mut fixture, "By", "2 ic");
    assert_eq!(dialog.borrow().format().by, Some(2.0));
    assert_eq!(dialog.borrow().format().by_kind, IndentKind::Characters);

    let ok = fixture.by_label("OK").expect("OK is shown");
    fixture.click(&ok).idle(4);
    let answer = dialog.borrow_mut().take_ok().expect("OK filed its answer");
    assert_eq!(answer.format.before_text, Some(36.0));
    assert_eq!(answer.format.before_text_kind, IndentKind::Length);
    assert_eq!(answer.format.after_text, Some(Some(327.67)));
    assert_eq!(answer.format.after_text_kind, IndentKind::Characters);
    assert_eq!(answer.format.by, Some(2.0));
    assert_eq!(answer.format.by_kind, IndentKind::Characters);
}

#[trial]
fn escape_in_character_indent_restores_value_and_keeps_characters_kind() {
    let format =
        ParagraphFormat { special: Some(SpecialIndent::FirstLine), by: Some(2.0), by_kind: IndentKind::Characters, ..ParagraphFormat::default() };
    let dialog = Rc::new(RefCell::new(ParagraphDialog::new(format).word(true)));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);

    let target = fixture.by_label("By").expect("By is shown");
    fixture.click(&target).idle(2);
    fixture.key(Key::A, Modifiers::COMMAND).type_text("5").key(Key::Escape, Modifiers::NONE).idle(4);
    assert_eq!(dialog.borrow().format().by, Some(2.0), "Escape restores previous characters indent");
    assert_eq!(dialog.borrow().format().by_kind, IndentKind::Characters);
}

#[trial]
fn out_of_range_character_indent_shows_ch_bounds_and_preview_scales_chars() {
    let format = ParagraphFormat {
        before_text: Some(500.0),
        before_text_kind: IndentKind::Characters,
        special: Some(SpecialIndent::FirstLine),
        by: Some(2.0),
        by_kind: IndentKind::Characters,
        char_size: Some(10.5),
        ..ParagraphFormat::default()
    };
    let decimal = Rc::new(Cell::new('.'));
    let seen_decimal = decimal.clone();
    let mut dialog = ParagraphDialog::new(format.clone()).word(true);
    let mut fixture = Fixture::new(move |ui| {
        seen_decimal.set(Env::of(ui).format().separators().0);
        let _ = dialog.show(ui, &DialogStack::default());
        ParagraphPreview::new("Preview", &format, "Sample Text").show(ui);
    });
    fixture.idle(10);
    let bound = format!("327{}67 ch", decimal.get());
    assert!(
        fixture.texts().iter().any(|run| run.text.contains(&bound)),
        "range message shows bounds in characters ({bound}): {:?}",
        fixture.texts().iter().map(|run| &run.text).collect::<Vec<_>>()
    );
}
