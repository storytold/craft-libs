//! [`SampleSizeSelect`]: the Sample Size cell and setting.

use craft_behavior::EditEvents;
use craft_ui::contract::{Bind, EditResponse, Entry, Outcome as _, Pattern, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use egui::Ui;

use super::{EyedropperWords, SampleSize};

/// One Sample Size cell and setting: a `Select` over [`SampleSize::ALL`] named with `EyedropperWords::size`, as
/// `EyedropperBar` draws it.
#[must_use = "show the select with `show`"]
#[derive(Pattern)]
pub struct SampleSizeSelect<'a> {
    size: Bind<'a, SampleSize>,
}

impl<'a> SampleSizeSelect<'a> {
    /// The select over `size`.
    pub fn new(size: Bind<'a, SampleSize>) -> Self {
        SampleSizeSelect { size }
    }

    /// Draws the select and reports the pick.
    pub fn show(mut self, ui: &mut Ui) -> EditResponse<SampleSize> {
        let words = Env::of(ui).words::<EyedropperWords>();
        let entries: Vec<Entry<SampleSize>> = SampleSize::ALL.iter().map(|size| Entry::new(*size, size.label(&words))).collect();
        let field =
            Field::new(words.sample_size.clone()).show(ui, |ui, _| Select::new(words.size.clone(), self.size.reborrowed(), &entries).show(ui));
        // A parent's sizing pass places a field without running its body: that run reports no edit.
        let response = field.response().clone();
        field.inner.unwrap_or_else(|| EditResponse::lifted(response, EditEvents::new(), None))
    }
}
