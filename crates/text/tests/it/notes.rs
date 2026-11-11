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
