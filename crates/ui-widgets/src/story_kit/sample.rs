//! The sample content of stories whose subject lays out other components: words to lay out, and a sunken surface
//! that shows the subject's extent, padding and gaps.

use crate::contract::{Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout_components::Stack;
use crate::style::SurfaceKind;
use crate::testing::StoryOutput;
use egui::Ui;
use std::ops::Range;

/// The words a story lays out, in order.
const WORDS: [&str; 4] = ["First", "Second", "Third", "Fourth"];

/// Shows the sample words at `range`, one `Text` child each.
#[doc(hidden)]
pub fn words(ui: &mut Ui, range: Range<usize>) {
    for word in &WORDS[range] {
        Text::new(*word).show(ui);
    }
}

/// Shows the subject that `show` draws on a sunken surface that takes the subject's size, and returns the subject's
/// output.
pub(crate) fn outlined(ui: &mut Ui, mut show: impl FnMut(&mut Ui) -> StoryOutput) -> StoryOutput {
    Stack::new().surface(SurfaceKind::Sunken).show(ui, |ui, _| show(ui)).inner.unwrap_or_default()
}
