//! Stories of the full-window parts.

use super::{FullWindowCanvas, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, FullWindowZoomBar, RasterOverlay};
use crate::canvas::{PixelRect, RasterSource};
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::primitives::{Checker, Revision};
use crate::runtime::pending::Pending;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::{Color32, ColorImage, Ui};

/// The story image's size in pixels.
const IMAGE: PixelSize = PixelSize::new(320, 200);

/// A solid image with a transparent left half, so the checkerboard shows.
struct Flat {
    colour: Color32,
    half: bool,
}

impl RasterSource for Flat {
    fn size(&self) -> PixelSize {
        IMAGE
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [width, height] = out.size;
        for row in 0..height {
            for column in 0..width {
                let x = region.x0 + ((column as u32 + 1) * factor) as i32 - 1;
                let transparent = self.half && x < (IMAGE.width / 2) as i32;
                out.pixels[row * width + column] = if transparent { Color32::TRANSPARENT } else { self.colour };
            }
        }
    }
}

#[story(FullWindowSide, covers = [Default], drive = Drive::Never)]
fn side(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FullWindowSide::new("Properties")
        .show(ui, |ui, _| {
            Text::new("Radius").show(ui);
        })
        .output
        .into()
}

#[story(FullWindowSide, covers = [], drive = Drive::Never)]
fn side_headerless(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FullWindowSide::new("Settings")
        .headerless()
        .show(ui, |ui, _| {
            Text::new("Effects").show(ui);
        })
        .output
        .into()
}

/// The preview states the zoom bar shows: nothing, a render of unknown length, a render partly done.
const PREVIEWS: [(&str, Option<Option<f32>>); 3] = [("Idle", None), ("Pending", Some(None)), ("Half done", Some(Some(0.4)))];

#[story(FullWindowZoomBar, covers = [], drive = Drive::Never)]
fn zoom_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let note = *data.value("note", true);
    let view = data.value("view", FullWindowView::new());
    let bar = FullWindowZoomBar::new(Bind::bound(view));
    let bar = if note { bar.note("Preview 120 ms") } else { bar };
    bar.show(ui).into()
}

#[story(FullWindowZoomBar, covers = [], drive = Drive::Never)]
fn zoom_bar_with_progress(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let progress = *data.choice("preview", Some(Some(0.4)), &PREVIEWS);
    let view = data.value("view", FullWindowView::new());
    FullWindowZoomBar::new(Bind::bound(view)).progress(progress).show(ui).into()
}

#[story(FullWindowTitle, covers = [], drive = Drive::Never)]
fn title(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FullWindowTitle::new("Liquify…").subject("Layer 1").zoom(66.7).show(ui).into()
}

#[story(FullWindowStatus, covers = [], drive = Drive::Never)]
fn status_loading(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let pending: Pending<(), String> = Pending::default();
    FullWindowStatus::of(&pending).show(ui).into()
}

#[story(FullWindowStatus, covers = [], drive = Drive::Never)]
fn status_failed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let pending: Pending<(), String> = Pending::Failed("The filter ran out of memory.".to_owned());
    FullWindowStatus::of(&pending).show(ui).into()
}

#[story(FullWindowCanvas<'static>, covers = [], drive = Drive::Never)]
fn canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    static IMAGE_SOURCE: Flat = Flat { colour: Color32::from_rgb(0x4a, 0x7f, 0xc1), half: true };
    static MASK: Flat = Flat { colour: Color32::from_rgba_premultiplied(0x40, 0x00, 0x00, 0x40), half: false };
    let checkerboard = *data.value("checkerboard", true);
    let overlay = *data.value("overlay", true);
    let tint = *data.value("tint", true);
    let view = data.value("view", FullWindowView::new());
    let mut canvas = FullWindowCanvas::new(Bind::bound(view), &IMAGE_SOURCE);
    if checkerboard {
        canvas = canvas.checkerboard(Checker::new());
    }
    if overlay {
        let layer = RasterOverlay::new(&MASK);
        canvas = canvas.overlay(if tint { layer.tint(Color32::from_rgb(0xc0, 0x40, 0x40)) } else { layer });
    }
    canvas.show(ui).into()
}
