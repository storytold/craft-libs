//! The Format Text panel: a control the host cannot apply takes no input and says why in its tooltip.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_text::ui::character::{CharacterChange, CharacterValues, FormatControl, FormatText, FormatTextChange, LineSpacing, ParagraphValues};
use craft_ui::contract::ShowContainer as _;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::runtime::env::Label;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

const BOLD_REASON: &str = "The font has no bold face";
const LINE_REASON: &str = "Added text keeps its own line spacing";
const SCALE_REASON: &str = "Added text cannot be scaled";

/// The changes the edits carry, in order.
fn changes(edits: &RefCell<Vec<EditEvent<FormatTextChange>>>) -> Vec<FormatTextChange> {
    edits.borrow().iter().filter_map(|edit| edit.value().cloned()).collect()
}

/// The texts painted once the pointer has rested on the control named `label` past the tooltip delay.
fn texts_on_hover(fixture: &mut Fixture, label: &str) -> Vec<String> {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.frame(vec![moved(target.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    fixture.texts().iter().map(|run| run.text.clone()).collect()
}

#[trial]
fn an_unsupported_control_takes_no_input_and_shows_its_reason() {
    let character = CharacterValues {
        family: Some("Helvetica".to_owned()),
        size: Some(12.0),
        faux_bold: Some(false),
        faux_italic: Some(false),
        horizontal_scale: Some(100.0),
        ..CharacterValues::default()
    };
    let paragraph = ParagraphValues::default();
    let families = vec!["Helvetica".to_owned(), "Times".to_owned(), "Courier".to_owned()];
    let unsupported = vec![
        (FormatControl::Bold, Label::from(BOLD_REASON)),
        (FormatControl::LineSpacing, Label::from(LINE_REASON)),
        (FormatControl::HorizontalScale, Label::from(SCALE_REASON)),
    ];
    let edits = Rc::new(RefCell::new(Vec::new()));
    let sink = edits.clone();
    let mut fixture = Fixture::new(move |ui| {
        let out = FormatText::new("Format text", &character, &paragraph, &families)
            .line_spacing(LineSpacing::Multiple(1.0))
            .unsupported(&unsupported)
            .show(ui);
        sink.borrow_mut().extend(out.edits);
    });
    fixture.idle(10);

    // An enabled select opens on the click and picks the next spacing with Down and Enter.
    let line = fixture.by_label("Line spacing").expect("the line spacing select is shown");
    fixture.click(&line).idle(2);
    fixture.key(Key::ArrowDown, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).idle(2);
    assert!(edits.borrow().is_empty(), "an unsupported line spacing takes no pick: {:?}", changes(&edits));

    let scale = fixture.by_label("Horizontal scale").expect("the horizontal scale field is shown");
    fixture.click(&scale).idle(2);
    fixture.type_text("150");
    fixture.key(Key::Enter, Modifiers::NONE).idle(2);
    assert!(edits.borrow().is_empty(), "an unsupported scale takes no typing: {:?}", changes(&edits));

    let bold = fixture.by_label("Bold").expect("the Bold toggle is shown");
    fixture.click(&bold).idle(2);
    assert!(edits.borrow().is_empty(), "an unsupported toggle takes no click: {:?}", changes(&edits));

    // The same click reaches a supported toggle: the clicks above were refused, not lost.
    let italic = fixture.by_label("Italic").expect("the Italic toggle is shown");
    fixture.click(&italic).idle(2);
    let italic_on = FormatTextChange::Character(CharacterChange::FauxItalic(true));
    assert!(changes(&edits).contains(&italic_on), "a supported toggle reports its click: {:?}", changes(&edits));

    for (label, reason) in [("Bold", BOLD_REASON), ("Line spacing", LINE_REASON), ("Horizontal scale", SCALE_REASON)] {
        let texts = texts_on_hover(&mut fixture, label);
        assert!(texts.iter().any(|text| text.contains(reason)), "{label}'s tooltip says why it is disabled: {texts:?}");
    }
}

/// The size field keeps room to show its value in a narrow panel: the colour well, the list buttons and the
/// alignment menu beside it take what they need and the field the rest, never less than a few digits.
#[trial]
fn the_size_field_keeps_room_for_its_value_in_a_narrow_panel() {
    let character = CharacterValues { family: Some("Helvetica".to_owned()), size: None, ..CharacterValues::default() };
    let paragraph = ParagraphValues::default();
    let families = vec!["Helvetica".to_owned()];
    let mut fixture = Fixture::new(move |ui| {
        ui.set_max_width(215.0);
        FormatText::new("Format text", &character, &paragraph, &families).show(ui);
    });
    fixture.idle(10);
    let size = fixture.by_label("Font size").expect("the size field is shown");
    assert!(size.rect.width() >= 64.0, "the size field is {} wide: {:?}", size.rect.width(), size.rect);
    assert!(
        fixture.texts().iter().any(|run| run.text == "Mixed"),
        "a mixed size reads Mixed: {:?}",
        fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()
    );
}

/// In the Tools pane's width (272 points less its padding, Comfortable density) the size field still reads its value:
/// "14" and "pt", not just the "tT" mark and the arrow (a fill-width field has a least width of a few digits).
#[trial]
fn the_size_field_reads_its_value_in_a_272_point_pane() {
    use craft_theme::Size;
    let character = CharacterValues { family: Some("Helvetica".to_owned()), size: Some(14.0), ..CharacterValues::default() };
    let paragraph = ParagraphValues::default();
    let families = vec!["Helvetica".to_owned()];
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::layout_components::Stack::new().fill().show_at(ui, ui.available_rect_before_wrap(), |ui, _| {
            FormatText::new("Format text", &character, &paragraph, &families).show(ui);
        });
    });
    fixture.room(egui::vec2(218.0, 600.0)).density(Size::Comfortable);
    fixture.idle(10);
    let size = fixture.by_label("Font size").expect("the size field is shown");
    let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    assert!(texts.iter().any(|text| text == "14") && texts.iter().any(|text| text == "pt"), "the size reads 14pt: {texts:?} in {:?}", size.rect);
    assert!(size.rect.width() >= 64.0, "the size field is {} wide: {:?}", size.rect.width(), size.rect);
    let align = fixture.by_label("Alignment").expect("the alignment menu is shown");
    assert!(align.rect.right() <= 218.5, "the alignment menu stays in the panel: {:?}", align.rect);
}

/// The mixed size reads "Mixed" in the same width.
#[trial]
fn the_size_field_reads_mixed_in_a_272_point_pane() {
    use craft_theme::Size;
    let character = CharacterValues { family: Some("Helvetica".to_owned()), size: None, ..CharacterValues::default() };
    let paragraph = ParagraphValues::default();
    let families = vec!["Helvetica".to_owned()];
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::layout_components::Stack::new().fill().show_at(ui, ui.available_rect_before_wrap(), |ui, _| {
            FormatText::new("Format text", &character, &paragraph, &families).show(ui);
        });
    });
    fixture.room(egui::vec2(218.0, 600.0)).density(Size::Comfortable);
    fixture.idle(10);
    let size = fixture.by_label("Font size").expect("the size field is shown");
    let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    assert!(texts.iter().filter(|text| *text == "Mixed").count() >= 2, "the size reads Mixed beside the line spacing: {texts:?} in {:?}", size.rect);
    assert!(size.rect.width() >= 64.0, "the size field is {} wide: {:?}", size.rect.width(), size.rect);
    let align = fixture.by_label("Alignment").expect("the alignment menu is shown");
    assert!(align.rect.right() <= 218.5, "the alignment menu stays in the panel: {:?}", align.rect);
}

/// The paragraph spacing a host gives: its presets in the field's menu and its range, which a typed value is held to.
/// The field sits on the line spacing row, centred on it, inside the panel (PdfCraft: 0, 6, 12, 18 and 24 pt, at most
/// 72 pt).
#[trial]
fn paragraph_spacing_takes_the_hosts_presets_and_range_on_the_line_spacing_row() {
    use craft_text::ui::character::ParagraphChange;
    let character = CharacterValues { family: Some("Helvetica".to_owned()), size: Some(12.0), ..CharacterValues::default() };
    let paragraph = ParagraphValues { space_after: Some(6.0), ..ParagraphValues::default() };
    let families = vec!["Helvetica".to_owned()];
    let edits = Rc::new(RefCell::new(Vec::new()));
    let panel = Rc::new(RefCell::new(egui::Rect::NOTHING));
    let (sink, panel_out) = (edits.clone(), panel.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = FormatText::new("Format text", &character, &paragraph, &families)
            .line_spacing(LineSpacing::Multiple(1.0))
            .paragraph_spacing_presets(&[0.0, 6.0, 12.0, 18.0, 24.0])
            .paragraph_spacing_range(0.0..=72.0)
            .show(ui);
        *panel_out.borrow_mut() = out.response.rect;
        sink.borrow_mut().extend(out.edits);
    });
    fixture.idle(10);
    let line = fixture.by_label("Line spacing").expect("the line spacing select is shown").rect;
    let spacing = fixture.by_label("Paragraph spacing").expect("the paragraph spacing field is shown").rect;
    let panel = *panel.borrow();
    assert!((spacing.center().y - line.center().y).abs() <= 1.0, "the field {spacing:?} is centred on the line spacing row {line:?}");
    assert!(
        spacing.left() >= line.right() && spacing.right() <= panel.right() + 0.5,
        "the field {spacing:?} is beside it, inside the panel {panel:?}"
    );

    let field = fixture.by_label("Paragraph spacing").expect("the paragraph spacing field is shown");
    fixture.click(&field).idle(2);
    fixture.key(Key::A, Modifiers::COMMAND).type_text("100");
    fixture.key(Key::Enter, Modifiers::NONE).idle(2);
    let last = changes(&edits).into_iter().rev().find_map(|change| match change {
        FormatTextChange::Paragraph(ParagraphChange::SpaceAfter(points)) => Some(points),
        _ => None,
    });
    assert_eq!(last, Some(72.0), "a typed spacing is held to the host's range");

    let texts = texts_after_opening(&mut fixture, "Paragraph spacing");
    for preset in ["0", "12", "18", "24"] {
        assert!(texts.iter().any(|text| text.split_whitespace().next() == Some(preset)), "the menu offers {preset} pt: {texts:?}");
    }
}

/// The texts painted once the menu of the field named `label` is open.
fn texts_after_opening(fixture: &mut Fixture, label: &str) -> Vec<String> {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.click(&field).idle(2);
    // Alt+Down opens a number field's presets list.
    fixture.key(Key::ArrowDown, Modifiers::ALT).idle(3);
    fixture.texts().iter().map(|run| run.text.clone()).collect()
}
