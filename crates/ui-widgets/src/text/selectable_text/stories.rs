//! Stories of `SelectableText`.

use super::SelectableText;
use crate::contract::Show as _;
use crate::primitives::Span;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const LONG: &str = "A name that runs on well past the end of the field, so the field scrolls its text under the caret";

#[story(SelectableText<'static>, covers = [Default], drive = Drive::Never)]
fn selectable_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SelectableText::new("Path", "/projects/poster.indd").show(ui).into()
}

#[story(SelectableText<'static>, covers = [LongText], drive = Drive::Never)]
fn selectable_text_long(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SelectableText::new("Path", LONG).show(ui).into()
}

#[story(SelectableText<'static>, covers = [], drive = Drive::Never)]
fn selectable_text_styled(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let spans = vec![
        Span::new(".title").strong(),
        Span::new(" {\n  "),
        Span::new("font-size").muted(),
        Span::new(": 12px;\n  "),
        Span::new("fill").muted(),
        Span::new(": #336699;\n}\n"),
        Span::new("#frame > .caption").strong(),
        Span::new(" {\n  "),
        Span::new("opacity").muted(),
        Span::new(": 0.5;\n}"),
    ];
    SelectableText::spans("Rules", spans).mono().multiline().show(ui).into()
}
