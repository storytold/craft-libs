//! `Surfaces`: one swatch per surface kind, each with the ink that reads on it, in the theme the story is drawn in.

use craft_ui::contract::{Gap, Pad, Pattern, Props, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::style::{SurfaceKind, TypeRole};
use egui::Ui;

/// The surface kinds as one swatch each, named by the kind, with text in the kind's own ink.
#[derive(Pattern, Props)]
pub struct Surfaces<'a> {
    kinds: &'a [SurfaceKind],
}

impl<'a> Surfaces<'a> {
    /// The swatches of every [`SurfaceKind`], in the order of `SurfaceKind::ALL`.
    pub fn new() -> Surfaces<'static> {
        Surfaces { kinds: &SurfaceKind::ALL }
    }

    /// Shows the swatches one under the other.
    pub fn show(self, ui: &mut Ui) {
        Stack::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| {
            for &kind in self.kinds {
                Stack::new().fill_width().surface(kind).pad(Pad::Regular).show(ui, |ui, _| {
                    Text::new(format!("{kind:?}")).role(TypeRole::Label).show(ui);
                });
            }
        });
    }
}
