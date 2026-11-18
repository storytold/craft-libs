//! The Word Paragraph dialog's pages: a check box whose paragraphs differ stays mixed across page switches until clicked.

use craft_behavior::Mixed;
use craft_text::ui::character::{AsianTypography, LineBreakOptions, ParagraphDialog, ParagraphFormat, ParagraphTab};
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

fn toggled(fixture: &Fixture, label: &str) -> Option<Mixed<bool>> {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.tree().toggled_of(target.node)
}

#[trial]
fn word_paragraph_tabs_leave_mixed_boxes_unset_until_clicked() {
    let mut format = ParagraphFormat::default();
    format.breaks = Some(LineBreakOptions::default());
    format.asian = Some(AsianTypography::default());
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
