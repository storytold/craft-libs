//! Stories of `PathRow`.

use super::PathRow;
use crate::contract::{Bind, ShowContainer as _};
use crate::form::Field;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The row in its labelled field, over `path`.
fn row(ui: &mut Ui, path: &mut String) -> StoryOutput {
    let mut shown = StoryOutput::default();
    Field::new("Image").show(ui, |ui, _| {
        shown = PathRow::new("Image", Bind::bound(&mut *path)).placeholder("No file chosen").show(ui).into();
    });
    shown
}

#[story(PathRow<'static>, covers = [Default], drive = Drive::Never)]
fn path_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    row(ui, data.value("path", "/photos/trip/sunrise.png".to_owned()))
}

#[story(PathRow<'static>, covers = [], drive = Drive::Never)]
fn path_row_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    row(ui, data.value("empty", String::new()))
}
