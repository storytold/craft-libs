//! Stories of `SaveForWebDialog`.

use super::{
    Animation, BrowserPreview, OptimizeAnswer, SaveForWebDialog, SaveForWebOptions, TableMarks, WebDocument, WebFormat, WebPictures, WebPreset,
    WebSlice, WebSlices, WebVariant,
};
use crate::primitives::TextureCache;
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Rect, Ui, Vec2, pos2};
use std::sync::OnceLock;

/// The picture's size in pixels.
const PICTURE: [usize; 2] = [360, 240];

/// The encoded file size the story reports, in bytes.
const ENCODED_BYTES: u64 = 49_350;

fn picture(ui: &Ui, key: &'static str, color: Color32) -> SizedTexture {
    let id = TextureCache::texture(ui.ctx(), key, || ColorImage::filled(PICTURE, color));
    SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32))
}

fn size() -> PixelSize {
    PixelSize::new(PICTURE[0] as u32, PICTURE[1] as u32)
}

fn presets() -> Vec<WebPreset> {
    let png = SaveForWebOptions { format: WebFormat::Png24, ..SaveForWebOptions::default() };
    vec![WebPreset::new("JPEG Medium", SaveForWebOptions::default()), WebPreset::new("PNG-24", png)]
}

/// The animation and preset-list values of a story, read before the story binds its other values.
fn extras(data: &mut StoryData, animation: bool, presets_listed: bool) -> (bool, bool) {
    (*data.value("animation", animation), *data.value("presets", presets_listed))
}

/// The dialog with the animation and the preset list `extras` asked for.
fn with_extras(dialog: SaveForWebDialog<'_>, (animation, presets_listed): (bool, bool)) -> SaveForWebDialog<'_> {
    let dialog = if animation { dialog.animation(Animation::new(12, 24.0)) } else { dialog };
    if presets_listed { dialog.presets(presets()) } else { dialog }
}

/// The pictures of a story whose encode succeeded.
fn ready(ui: &Ui) -> WebPictures {
    let original = picture(ui, "form-dialogs-web-original", Color32::from_rgb(0x8a, 0x6f, 0x4d));
    let encoded = picture(ui, "form-dialogs-web-encoded", Color32::from_rgb(0x84, 0x70, 0x52));
    let mut pictures = WebPictures::new().original(original);
    pictures.set(WebVariant::new(0, 1, encoded, ENCODED_BYTES));
    pictures
}

#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let original = picture(ui, "form-dialogs-web-original", Color32::from_rgb(0x8a, 0x6f, 0x4d));
    let encoded = picture(ui, "form-dialogs-web-encoded", Color32::from_rgb(0x84, 0x70, 0x52));
    let mut pictures = WebPictures::new().original(original);
    pictures.set(WebVariant::new(0, 1, encoded, ENCODED_BYTES));
    let flags = extras(data, false, true);
    let ((options, open), web_at) = data.value_at("web", (SaveForWebOptions::default(), true));
    with_extras(SaveForWebDialog::new("poster.psd", size(), pictures, web_at.bind(options)).opened(web_at.bind(open)), flags).show(ui).into()
}

#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_pending_animation(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let original = picture(ui, "form-dialogs-web-original", Color32::from_rgb(0x8a, 0x6f, 0x4d));
    let flags = extras(data, true, false);
    let ((options, open), options_open_at) = data.value_at("web-animation", (SaveForWebOptions::default(), true));
    let dialog = SaveForWebDialog::new("loop.psd", size(), WebPictures::new().original(original), options_open_at.bind(options))
        .opened(options_open_at.bind(open));
    with_extras(dialog, flags).show(ui).into()
}

/// The palette the GIF story's encode reports.
fn palette() -> Vec<[u8; 3]> {
    vec![[138, 111, 77], [132, 112, 82], [20, 20, 20], [240, 240, 240], [200, 60, 60], [60, 160, 90], [60, 90, 200], [230, 200, 60]]
}

#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_color_table(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let original = picture(ui, "form-dialogs-web-original", Color32::from_rgb(0x8a, 0x6f, 0x4d));
    let encoded = picture(ui, "form-dialogs-web-encoded", Color32::from_rgb(0x84, 0x70, 0x52));
    let mut pictures = WebPictures::new().original(original);
    pictures.set(WebVariant::new(0, 1, encoded, ENCODED_BYTES).colors(8).palette(palette()));
    let start = SaveForWebOptions { format: WebFormat::Gif, colors: 8, ..SaveForWebOptions::default() };
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-table", (start, true));
    let dialog =
        SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options)).opened(options_open_at.bind(open)).marks(story_marks());
    with_extras(dialog, flags).show(ui).into()
}

/// The marks of the colour table story: one cell locked, one transparent and one shifted from another colour.
fn story_marks() -> &'static TableMarks {
    static MARKS: OnceLock<TableMarks> = OnceLock::new();
    MARKS.get_or_init(|| TableMarks {
        locked: vec![[20, 20, 20]],
        transparent: vec![[240, 240, 240]],
        shifted: vec![([138, 111, 77], [132, 112, 82])],
    })
}

/// The slices of the slices story: three side by side.
fn story_slices() -> &'static [WebSlice] {
    static SLICES: OnceLock<Vec<WebSlice>> = OnceLock::new();
    SLICES.get_or_init(|| {
        (0..3u32)
            .map(|at| {
                let left = at as f32 * 120.0;
                WebSlice { number: at + 1, rect: Rect::from_min_max(pos2(left, 0.0), pos2(left + 120.0, PICTURE[1] as f32)) }
            })
            .collect()
    })
}

#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_slices(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let original = picture(ui, "form-dialogs-web-original", Color32::from_rgb(0x8a, 0x6f, 0x4d));
    let encoded = picture(ui, "form-dialogs-web-encoded", Color32::from_rgb(0x84, 0x70, 0x52));
    let mut pictures = WebPictures::new().original(original);
    pictures.set(WebVariant::new(0, 1, encoded, ENCODED_BYTES));
    let start = SaveForWebOptions { slices: WebSlices::Selected(vec![2]), ..SaveForWebOptions::default() };
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-slices", (start, true));
    let dialog = SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options))
        .opened(options_open_at.bind(open))
        .slices(story_slices());
    with_extras(dialog, flags).show(ui).into()
}

#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_failed_encode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let original = picture(ui, "form-dialogs-web-original", Color32::from_rgb(0x8a, 0x6f, 0x4d));
    let mut pictures = WebPictures::new().original(original);
    pictures.set(WebVariant::failed(0, 1, "The encoder ran out of memory"));
    let start = SaveForWebOptions { format: WebFormat::Gif, ..SaveForWebOptions::default() };
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-failed", (start, true));
    with_extras(SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options)).opened(options_open_at.bind(open)), flags)
        .show(ui)
        .into()
}

/// A vector document of two artboards, clipped to the second.
#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_vector(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pictures = ready(ui);
    let start = SaveForWebOptions { format: WebFormat::Png24, clip_to_artboard: true, artboard: Some(1), ..SaveForWebOptions::default() };
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-vector", (start, true));
    let document = WebDocument::vector(vec!["Artboard 1".to_owned(), "Artboard 2".to_owned()]);
    let dialog =
        SaveForWebDialog::new("logo.vcr", size(), pictures, options_open_at.bind(options)).opened(options_open_at.bind(open)).document(document);
    with_extras(dialog, flags).show(ui).into()
}

/// A GIF with the Lossy slider after Web Snap.
#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_gif_lossy(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pictures = ready(ui);
    let start = SaveForWebOptions { format: WebFormat::Gif, lossy: 30, ..SaveForWebOptions::default() };
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-lossy", (start, true));
    with_extras(SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options)).opened(options_open_at.bind(open)), flags)
        .show(ui)
        .into()
}

/// The Done button beside Save…, and Preview in Browser disabled with its reason.
#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_done_and_unavailable_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pictures = ready(ui);
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-done", (SaveForWebOptions::default(), true));
    let reason = Label::from("No browser is installed");
    let dialog = SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options))
        .opened(options_open_at.bind(open))
        .browser_preview(BrowserPreview::Unavailable(reason));
    with_extras(dialog, flags).show(ui).into()
}

/// The Optimize to File Size… rows, open at 48 K.
#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_optimize(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pictures = ready(ui);
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-optimize", (SaveForWebOptions::default(), true));
    let dialog = SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options))
        .opened(options_open_at.bind(open))
        .optimize()
        .optimize_start(48 * 1024);
    with_extras(dialog, flags).show(ui).into()
}

/// The result line after a search that reached its target.
#[story(SaveForWebDialog<'static>, covers = [], drive = Drive::Never)]
fn save_for_web_optimize_result(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pictures = ready(ui);
    let flags = extras(data, false, false);
    let ((options, open), options_open_at) = data.value_at("web-optimize-result", (SaveForWebOptions::default(), true));
    let answer = OptimizeAnswer::new(None, 31_800, 32 * 1024, true);
    let dialog = SaveForWebDialog::new("poster.psd", size(), pictures, options_open_at.bind(options))
        .opened(options_open_at.bind(open))
        .optimize()
        .optimize_answer(Ok(answer));
    with_extras(dialog, flags).show(ui).into()
}
