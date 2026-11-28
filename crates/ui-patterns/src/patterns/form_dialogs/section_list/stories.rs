//! Stories of `SectionList`.

use super::SectionList;
use crate::contract::Show as _;
use crate::display::Text;
use crate::list::{ListRow, RowInk};
use crate::runtime::env::Label;
use crate::style::TypeRole;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{DisplayOrder, ItemSource};
use egui::Ui;
use std::borrow::Cow;

/// The story's sections.
const SECTIONS: [&str; 5] = ["General", "Compression", "Marks", "Output", "Advanced"];

/// The section that shows dimmed in the second story.
const DIMMED: usize = 2;

/// The lines of a section's body: enough to scroll in the story's room.
const LINES: usize = 40;

/// The list's rows, keyed by position.
struct Sections;

impl DisplayOrder<usize> for Sections {
    fn len(&self) -> usize {
        SECTIONS.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        SECTIONS.get(at).map(|name| Cow::Borrowed(*name))
    }
}

impl ItemSource<usize> for Sections {}

/// The shown section's body: a title and lines that scroll.
fn body(ui: &mut Ui, section: usize) {
    let name = SECTIONS.get(section).copied().unwrap_or_default();
    Text::new(name).role(TypeRole::Title).show(ui);
    for line in 1..=LINES {
        Text::new(format!("{name}, line {line}")).show(ui);
    }
}

/// Shows the list, with the rows `dimmed` marks in the dimmed ink, and keeps the picked section.
fn list(ui: &mut Ui, data: &mut StoryData, dimmed: Option<usize>) -> StoryOutput {
    let shown = data.value("section", 0_usize);
    let at = *shown;
    let row = |key: &usize| {
        let row = ListRow::new(Label::from(SECTIONS.get(*key).copied().unwrap_or_default()));
        if dimmed == Some(*key) { row.ink(RowInk::Dimmed) } else { row }
    };
    if let Some(next) = SectionList::new("Sections", &Sections, at).show(ui, row, |ui| body(ui, at)) {
        *shown = next;
    }
    StoryOutput::default()
}

#[story(SectionList<'static, usize>, covers = [], drive = Drive::Never)]
fn sections_beside_a_scrolling_body(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    list(ui, data, None)
}

#[story(SectionList<'static, usize>, covers = [], drive = Drive::Never)]
fn sections_with_a_dimmed_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    list(ui, data, Some(DIMMED))
}
