//! Stories of `Spinner`.

use super::{Spinner, SpinnerSize};
use crate::contract::{Props as _, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout_components::Grid;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::ColumnWidth;
use egui::Ui;

/// The name and the spinner of each size, in two columns that fit their content.
static COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];

#[story(Spinner, covers = [Default], drive = Drive::Never)]
fn spinner(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Grid::new(&COLUMNS).show(ui, |ui, _| {
        for (name, size) in [("Sm", SpinnerSize::Sm), ("Md", SpinnerSize::Md), ("Lg", SpinnerSize::Lg), ("Control", SpinnerSize::Control)] {
            Text::new(name).id_salt(("name", name)).show(ui);
            Spinner::new().extent(size).label(format!("Loading {name}")).id_salt(name).show(ui);
        }
    });
    shown.into()
}
