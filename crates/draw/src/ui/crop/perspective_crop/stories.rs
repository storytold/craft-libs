//! Stories of the perspective crop quad.

use super::{CORNER, PerspectiveCropQuad, PerspectiveQuad};
use craft_behavior::kurbo::Point;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

/// A skewed page marked from corner to corner: a quad that can be committed.
const PAGE: [Point; 4] = [Point::new(250.0, 110.0), Point::new(430.0, 130.0), Point::new(400.0, 290.0), Point::new(220.0, 250.0)];
/// The page the quad is marked on, as the camera saw it.
const PAPER: craft_ui::testing::canvas_story::FlatPicture = craft_ui::testing::canvas_story::FlatPicture::new(egui::Color32::from_rgb(70, 130, 180));
/// The same corners with the last one pulled in: the quad is not convex.
const BENT: [Point; 4] = [Point::new(250.0, 110.0), Point::new(430.0, 130.0), Point::new(400.0, 290.0), Point::new(380.0, 150.0)];

/// Shows the quad `quad` builds and shows as the only canvas control of a canvas. The canvas body runs more than once
/// per pass, so `quad` builds it on every run.
fn on_canvas<R: Into<StoryOutput>>(ui: &mut Ui, mut quad: impl FnMut(&mut Ui, egui::Rect) -> R) -> StoryOutput {
    craft_ui::testing::canvas_story::on_canvas(ui, |ui, at, _| {
        craft_ui::testing::canvas_story::show_quad(ui, at, "Page", PAGE, &PAPER);
        quad(ui, at)
    })
}

#[story(PerspectiveCropQuad<'static>, covers = [Default], drive = Drive::Drag(Grip::part(CORNER.at(0))))]
fn skewed_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quad = data.value("quad", Some(PAGE));
    on_canvas(ui, |ui, at| PerspectiveCropQuad::new("Perspective crop", Bind::bound(&mut *quad)).show_at(ui, at).edit)
}

#[story(PerspectiveCropQuad<'static>, covers = [Invalid], drive = Drive::Drag(Grip::part(CORNER.at(0))))]
fn bent_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quad = data.value("quad", Some(BENT));
    on_canvas(ui, |ui, at| {
        let bind = Bind::bound(&mut *quad).check(|quad: &PerspectiveQuad| match quad {
            Some(quad) if !super::is_valid(quad) => Validity::Refuse(Message::new("The corners do not make a convex quad")),
            _ => Validity::Ok,
        });
        PerspectiveCropQuad::new("Perspective crop", bind).show_at(ui, at).edit
    })
}
