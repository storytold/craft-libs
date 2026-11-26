//! Stories of the in-ribbon gallery.

use craft_ui::contract::Show as _;
use craft_ui::list::{ThumbAspect, ThumbSize};
use craft_ui::primitives::textures::Revision;
use craft_ui::primitives::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::{Label, with_env};
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_ui::tile_grid::CaptionPlace;
use egui::Ui;

use super::{Gallery, GalleryFace, GalleryItem, GalleryMore};
use crate::ribbon::GroupLevel;

type Paint = Box<dyn Fn(&craft_ui::primitives::ContentPainterCx, egui::Rect)>;

/// How many tiles every story's gallery holds: more than a row at either level.
const COUNT: usize = 10;

/// The gallery of `items` with the third one current, six tiles in a row at the full level and three at the compact.
fn show(ui: &mut Ui, items: &[GalleryItem<'_, usize>]) -> StoryOutput {
    let current = 2;
    Gallery::new("Styles", items, Some(&current)).tiles(6, 3).show(ui).into()
}

/// The name of tile `at`.
fn name(at: usize) -> Label {
    Label::from(format!("Style {}", at + 1))
}

/// A gallery of pictures from `Picture`'s store, one of the stories' photos each.
fn pictured(ui: &mut Ui) -> StoryOutput {
    let images: Vec<Box<dyn Fn() -> egui::ColorImage>> =
        (0..COUNT).map(|at| Box::new(move || sample_picture(at as u64, [48, 48])) as Box<dyn Fn() -> egui::ColorImage>).collect();
    let items: Vec<GalleryItem<'_, usize>> = images
        .iter()
        .enumerate()
        .map(|(at, image)| GalleryItem {
            key: at,
            label: name(at),
            face: GalleryFace::Image { key: at as u64, revision: Revision::default(), image: &**image },
        })
        .collect();
    let current = 2;
    Gallery::new("Styles", &items, Some(&current)).tiles(6, 3).more(GalleryMore::Grid).show(ui).into()
}

#[story(Gallery<'static, usize>, covers = [Default], drive = Drive::Never)]
fn gallery(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    pictured(ui)
}

/// A style gallery: wide samples, two to one, each named under its picture.
#[story(Gallery<'static, usize>, covers = [], drive = Drive::Never)]
fn gallery_captioned(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let images: Vec<Box<dyn Fn() -> egui::ColorImage>> =
        (0..COUNT).map(|at| Box::new(move || sample_picture(at as u64, [96, 48])) as Box<dyn Fn() -> egui::ColorImage>).collect();
    let items: Vec<GalleryItem<'_, usize>> = images
        .iter()
        .enumerate()
        .map(|(at, image)| GalleryItem {
            key: at,
            label: name(at),
            face: GalleryFace::Image { key: at as u64, revision: Revision::default(), image: &**image },
        })
        .collect();
    let current = 2;
    Gallery::new("Styles", &items, Some(&current)).tiles(6, 3).aspect(ThumbAspect::new(2, 1)).caption(CaptionPlace::Below).show(ui).into()
}

/// A gallery of small pages at the large thumbnail size: each preview (a style set's title, heading and paragraph)
/// reads at the size of a ribbon's three rows.
#[story(Gallery<'static, usize>, covers = [], drive = Drive::Never, title = "Large tiles")]
fn gallery_large(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let images: Vec<Box<dyn Fn() -> egui::ColorImage>> =
        (0..COUNT).map(|at| Box::new(move || sample_picture(at as u64, [52, 48])) as Box<dyn Fn() -> egui::ColorImage>).collect();
    let items: Vec<GalleryItem<'_, usize>> = images
        .iter()
        .enumerate()
        .map(|(at, image)| GalleryItem {
            key: at,
            label: name(at),
            face: GalleryFace::Image { key: at as u64, revision: Revision::default(), image: &**image },
        })
        .collect();
    Gallery::new("Style Sets", &items, None).tiles(7, 4).thumb(ThumbSize::Large).aspect(ThumbAspect::new(12, 11)).show(ui).into()
}

/// A framed gallery that holds every item: one rounded frame around the tiles, and no buttons while nothing is left
/// to step to.
#[story(Gallery<'static, usize>, covers = [], drive = Drive::Never, title = "Framed, every item shown")]
fn gallery_framed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let images: Vec<Box<dyn Fn() -> egui::ColorImage>> =
        (0..4).map(|at| Box::new(move || sample_picture(at as u64, [96, 48])) as Box<dyn Fn() -> egui::ColorImage>).collect();
    let items: Vec<GalleryItem<'_, usize>> = images
        .iter()
        .enumerate()
        .map(|(at, image)| GalleryItem {
            key: at,
            label: name(at),
            face: GalleryFace::Image { key: at as u64, revision: Revision::default(), image: &**image },
        })
        .collect();
    let current = 1;
    Gallery::new("Styles", &items, Some(&current))
        .tiles(6, 3)
        .aspect(ThumbAspect::new(2, 1))
        .caption(CaptionPlace::Below)
        .framed()
        .spare_buttons()
        .show(ui)
        .into()
}

#[story(Gallery<'static, usize>, covers = [], drive = Drive::Never)]
fn gallery_compact(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    with_env(
        ui,
        |set| {
            set.values.set(GroupLevel::Compact);
        },
        pictured,
    )
}

/// Clicks the down chevron once: the strip shows the last run of items.
fn step_down(fixture: &mut Fixture) {
    if let Some(button) = fixture.by_label("Next row") {
        fixture.click(&button);
    }
    fixture.idle(2);
}

#[story(Gallery<'static, usize>, covers = [], drive = Drive::Custom(step_down))]
fn gallery_stepped(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    pictured(ui)
}

/// Clicks More: the popover with the whole list opens.
fn open_more(fixture: &mut Fixture) {
    if let Some(button) = fixture.by_label("More") {
        fixture.click(&button);
    }
    fixture.idle(2);
}

#[story(Gallery<'static, usize>, covers = [Open], drive = Drive::Custom(open_more))]
fn gallery_more_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    pictured(ui)
}

#[story(Gallery<'static, usize>, covers = [], drive = Drive::Never)]
fn gallery_painted(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // Glyph tiles: a disc in the content painter's ink whose size grows with the tile's place.
    let paints: Vec<Paint> = (0..COUNT)
        .map(|at| {
            Box::new(move |cx: &craft_ui::primitives::ContentPainterCx, rect: egui::Rect| {
                let radius = rect.size().min_elem() * (at as f32 + 3.0) / (2.0 * COUNT as f32);
                cx.data_shapes([egui::Shape::circle_filled(rect.center(), radius, cx.ink())]);
            }) as Paint
        })
        .collect();
    let items: Vec<GalleryItem<'_, usize>> =
        paints.iter().enumerate().map(|(at, paint)| GalleryItem { key: at, label: name(at), face: GalleryFace::Paint(&**paint) }).collect();
    show(ui, &items)
}

#[story(Gallery<'static, usize>, covers = [], drive = Drive::Never)]
fn gallery_rendered(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // The source renders as a `Later` job: each tile shows the store's placeholder, then its picture.
    let options = ThumbnailOptions::new();
    let thumbnails = Thumbnails::<u64>::in_context(ui.ctx(), "craft_menu.gallery.story", options);
    let source = FakeThumbSource::<u64>::new().later(true);
    let items: Vec<GalleryItem<'_, usize>> = (0..COUNT)
        .map(|at| GalleryItem { key: at, label: name(at), face: GalleryFace::Rendered { key: at as u64, thumbnails: &thumbnails, source: &source } })
        .collect();
    show(ui, &items)
}
