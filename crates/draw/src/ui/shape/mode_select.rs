//! [`VectorModeSelect`]: the Shape, Path and Pixels mode choice of the vector tools.

use super::{ShapeWords, VectorMode};
use craft_ui::contract::{Bind, EditResponse, Entry, Show as _};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use egui::Ui;

/// The mode select of the vector tools' bars; a mode in `offered` with `false` shows disabled.
#[must_use = "show the select with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct VectorModeSelect<'a> {
    mode: Bind<'a, VectorMode>,
    offered: &'a [(VectorMode, bool)],
}

impl<'a> VectorModeSelect<'a> {
    /// The select of `mode` over the `offered` modes, each with whether it is enabled.
    pub fn new(mode: Bind<'a, VectorMode>, offered: &'a [(VectorMode, bool)]) -> Self {
        VectorModeSelect { mode, offered }
    }

    /// Shows the select and reports the picked mode.
    pub fn show(self, ui: &mut Ui) -> EditResponse<VectorMode> {
        let VectorModeSelect { mode, offered } = self;
        let words = Env::of(ui).words::<ShapeWords>();
        let entries: Vec<Entry<VectorMode>> = offered
            .iter()
            .map(|&(offered_mode, enabled)| {
                let label = match offered_mode {
                    VectorMode::Shape => &words.shape_mode,
                    VectorMode::Path => &words.path_mode,
                    VectorMode::Pixels => &words.pixels_mode,
                };
                Entry::new(offered_mode, label.clone()).enabled(enabled)
            })
            .collect();
        Select::new(words.mode.clone(), mode, &entries).show(ui)
    }
}
