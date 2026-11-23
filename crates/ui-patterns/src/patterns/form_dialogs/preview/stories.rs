//! Stories of `PreviewPane` and `PreviewZoom`.

use super::{PreviewPane, PreviewState, PreviewView, PreviewZoom, WELL};
use crate::contract::Show as _;
use crate::primitives::TextureCache;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Ui, Vec2};

/// The picture's side in points at zoom 1, wider than the well so the pane pans.
const PICTURE: [usize; 2] = [480, 320];

fn picture(ui: &Ui) -> SizedTexture {
    let id = TextureCache::texture(ui.ctx(), "form-dialogs-preview-story", || ColorImage::filled(PICTURE, Color32::from_rgb(0x4a, 0x7f, 0xc1)));
    SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32))
}

#[story(PreviewPane<'static>, covers = [Default], drive = Drive::Drag(Grip::new(WELL)))]
fn preview_pane(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", PreviewView::default());
    let shown = PreviewPane::new("Preview", picture(ui), view).show(ui);
    if let Some(next) = shown.view.clone() {
        *view = next;
    }
    shown.into()
}

#[story(PreviewZoom, covers = [], drive = Drive::Never)]
fn preview_zoom(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", PreviewView::default());
    let shown = PreviewZoom::new(view).show(ui);
    if let Some(next) = shown.view.clone() {
        *view = next;
    }
    shown.into()
}

#[story(PreviewPane<'static>, covers = [], drive = Drive::Never)]
fn preview_pane_pending(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", PreviewView::default());
    PreviewPane::state("Preview", PreviewState::Pending(Some(0.4)), view).show(ui).into()
}

#[story(PreviewPane<'static>, covers = [], drive = Drive::Never)]
fn preview_pane_pending_unknown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", PreviewView::default());
    PreviewPane::state("Preview", PreviewState::Pending(None), view).show(ui).into()
}

#[story(PreviewPane<'static>, covers = [], drive = Drive::Never)]
fn preview_pane_failed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", PreviewView::default());
    PreviewPane::state("Preview", PreviewState::Failed("The preview could not be rendered.".into()), view).show(ui).into()
}

#[story(PreviewPane<'static>, covers = [], drive = Drive::Drag(Grip::new(WELL)))]
fn preview_pane_fit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", PreviewView::fitting().zoom_range(0.01..=32.0));
    let shown = PreviewPane::state("Preview", PreviewState::Ready(picture(ui)), view).show(ui);
    if let Some(next) = shown.view.clone() {
        *view = next;
    }
    shown.into()
}
