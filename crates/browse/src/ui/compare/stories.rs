//! Stories of `CompareTile`: a pair (active, rejected, pending, failed) and a survey.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use craft_theme::Srgba;
use craft_ui::contract::{Bind, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::PreviewView;
use craft_ui::primitives::textures::Revision;
use craft_ui::runtime::env::Env;
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use craft_ui::tile_grid::TileFlag;
use egui::{Color32, ColorImage, Rect, Ui, Vec2};

use super::{ArrangeSpace, CompareCaption, CompareImage, ComparePair, CompareTile, RenderState, survey_cell};

/// The share of the room's width the narrow tile takes: too little for the caption's file name.
const NARROW_SHARE: f32 = 0.4;

/// The picture's size in pixels.
const PICTURE: [u32; 2] = [900, 600];
/// The spacing and width of the picture's grid lines, in pixels.
const GRID: (u32, u32) = (100, 2);

/// The sample photo of `key` with light grid lines, so zoom and pan show. Every fixture uploads it anew, so each
/// key's picture is made once per process.
fn gridded(key: u64) -> ColorImage {
    static MADE: Mutex<Option<HashMap<u64, ColorImage>>> = Mutex::new(None);
    let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
    made.get_or_insert_default().entry(key).or_insert_with(|| make_gridded(key)).clone()
}

fn make_gridded(key: u64) -> ColorImage {
    let mut image = sample_picture(key, PICTURE);
    let width = image.size[0];
    for (at, pixel) in image.pixels.iter_mut().enumerate() {
        let (x, y) = ((at % width) as u32, (at / width) as u32);
        if x % GRID.0 < GRID.1 || y % GRID.0 < GRID.1 {
            let lift = |c: u8| c + (u8::MAX - c) / 4 * 3;
            *pixel = Color32::from_rgb(lift(pixel.r()), lift(pixel.g()), lift(pixel.b()));
        }
    }
    image
}

fn image(key: u64) -> CompareImage<'static> {
    CompareImage::Picture(key, Revision::of(&key), PICTURE, Box::new(move || gridded(key)))
}

fn caption(label: &str, name: &str) -> CompareCaption {
    CompareCaption::new(label, name).rating(Some(4)).flag(Some(TileFlag::Picked)).label_color(Some(Srgba::rgb(0.86, 0.31, 0.24)))
}

/// Runs `content` over the room the story fills. The room is a container's body, so the story allocates what the
/// tiles are placed in.
fn filling(ui: &mut Ui, content: impl FnMut(&mut Ui, Rect) -> StoryOutput) -> StoryOutput {
    let mut content = content;
    let room = craft_ui::testing::stage(ui);
    let shown = Row::new().fill().show_at(ui, room, |ui, body| content(ui, body.rect));
    shown.inner.unwrap_or_default()
}

#[story(CompareTile<'static>, covers = [Default], drive = Drive::Never)]
fn compare_pair(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rtl = Env::of(ui).view().direction().is_rtl();
    filling(ui, |ui, area| {
        let mut active = *data.value("active", 0usize);
        let view = data.value("view", PreviewView::fitting());
        let [left, right] = ComparePair::new(ArrangeSpace::new(18.0, 16.0)).rtl(rtl).rects(area);
        let select =
            CompareTile::new("Select", Bind::bound(view), image(1)).caption(caption("Select", "IMG_0001.jpg")).active(active == 0).place(ui, left);
        if select.clicked {
            active = 0;
        }
        let candidate = CompareTile::new("Candidate", Bind::bound(view), image(2))
            .caption(caption("Candidate", "IMG_0002.jpg"))
            .active(active == 1)
            .rejected(true)
            .place(ui, right);
        if candidate.clicked {
            active = 1;
        }
        *data.value("active", 0usize) = active;
        select.into()
    })
}

#[story(CompareTile<'static>, covers = [LongText], drive = Drive::Never)]
fn compare_tile_narrow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    filling(ui, |ui, area| {
        let view = data.value("view", PreviewView::fitting());
        let rect = Rect::from_min_size(area.min, Vec2::splat(area.width() * NARROW_SHARE));
        CompareTile::new("Select", Bind::bound(view), image(1))
            .caption(caption("Select", "A_file_name_much_longer_than_the_tile.jpeg"))
            .place(ui, rect)
            .into()
    })
}

#[story(CompareTile<'static>, covers = [], drive = Drive::Never)]
fn compare_tile_pending(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    filling(ui, |ui, area| {
        let view = data.value("view", PreviewView::fitting());
        CompareTile::new("Candidate", Bind::bound(view), image(2))
            .caption(caption("Candidate", "IMG_0002.jpg"))
            .state(RenderState::Pending(true))
            .place(ui, area)
            .into()
    })
}

#[story(CompareTile<'static>, covers = [], drive = Drive::Never)]
fn compare_tile_failed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    filling(ui, |ui, area| {
        let view = data.value("view", PreviewView::fitting());
        CompareTile::new("Candidate", Bind::bound(view), CompareImage::None)
            .caption(CompareCaption::new("Candidate", "IMG_0002.jpg"))
            .state(RenderState::Failed("The renderer stopped".into()))
            .place(ui, area)
            .into()
    })
}

/// Twelve tiles at 3:2 in the survey arrangement: fitted, no zoom or pan.
#[story(CompareTile<'static>, covers = [Many], drive = Drive::Never)]
fn compare_survey(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const COUNT: usize = 12;
    const ASPECT: f32 = 1.5;
    let rtl = Env::of(ui).view().direction().is_rtl();
    filling(ui, |ui, area| {
        let view = data.value("view", PreviewView::fitting());
        let mut outs = Vec::new();
        for at in 0..COUNT {
            let rect = survey_cell(COUNT, at, area, ASPECT, rtl);
            let name = format!("IMG_{at:04}.jpg");
            let out = CompareTile::new(name.clone(), Bind::bound(view), image(at as u64))
                .caption(CompareCaption::new("", name))
                .survey(true)
                .place(ui, rect);
            outs.push(out);
        }
        outs.swap_remove(0).into()
    })
}
