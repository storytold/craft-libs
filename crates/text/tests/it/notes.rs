//! The notes text model: typing keeps the formatting around it.

use craft_text::ui::notes::{NotesParagraph, NotesRun, NotesText};
use craft_ui::testing::trial;

fn run(text: &str, bold: bool) -> NotesRun {
    NotesRun::new(text).bold(bold)
}

fn paragraph(runs: Vec<NotesRun>) -> NotesParagraph {
    NotesParagraph::new(runs)
}

#[trial]
fn typed_replaces_the_span_between_the_common_prefix_and_suffix() {
    let notes = NotesText::new(vec![paragraph(vec![run("bold", true), run(" plain", false)])]);
    let replaced = notes.typed("bXYZ plain").expect("a prefix and a suffix are shared");
    assert_eq!(replaced.paragraphs[0].runs, vec![run("bXYZ", true), run(" plain", false)]);
    let split = notes.typed("bold\r\n plain").expect("shares both ends");
    assert_eq!(split.plain(), "bold\n plain");
    assert_eq!(split.paragraphs.len(), 2);
}

/// A bullet wider than its hanging indent (none here) pushes the first line's text past it, so the text starts after
/// the marker instead of under it.
#[trial]
fn the_first_line_starts_after_a_marker_wider_than_its_hanging_indent() {
    use craft_text::lists::ListMarker;
    use craft_text::ui::notes::NotesEditor;
    use craft_ui::contract::Show as _;
    use craft_ui::testing::Fixture;
    use egui::{Key, Modifiers};

    let bulleted = paragraph(vec![run("Open", false)]).list(0, Some(ListMarker::Bullet("•".into())));
    let mut notes = NotesText::new(vec![bulleted]);
    let mut fx = Fixture::new(move |ui| {
        NotesEditor::new("Notes", &mut notes).show(ui);
    });
    fx.settle();
    let editor = fx.by_label("Notes").expect("the editor is named");
    fx.click(&editor).key(Key::Home, Modifiers::NONE).settle();
    let caret = fx.ime().expect("the focused editor places the caret").caret;
    let marker = fx.texts().iter().find(|painted| painted.text.starts_with('•')).expect("the marker is painted").rect;
    assert!(caret.center().x >= marker.right() - 0.5, "the caret before the first char ({caret:?}) sits after the marker ({marker:?})");
}
