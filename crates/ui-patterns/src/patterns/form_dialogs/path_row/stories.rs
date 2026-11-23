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

/// A row whose Browse is off, with the reason as the button's tooltip.
#[story(PathRow<'static>, covers = [], drive = Drive::Never)]
fn path_row_browse_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (path, path_at) = data.value_at("path", "/photos/trip/sunrise.png".to_owned());
    let mut shown = StoryOutput::default();
    Field::new("Image").show(ui, |ui, _| {
        shown = PathRow::new("Image", path_at.bind(&mut *path)).browse_reason("Browsing needs the file service.").show(ui).into();
    });
    shown
}

#[story(PathRow<'static>, covers = [], drive = Drive::Never)]
fn path_row_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    row(ui, data.value("empty", String::new()))
}
