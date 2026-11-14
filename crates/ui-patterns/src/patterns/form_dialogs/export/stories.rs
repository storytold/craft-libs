//! Stories of the export dialogs.

use super::{
    ExportAsDialog, ExportOptions, ExportOptionsKind, ExportPreview, FormatOptions, FormatOptionsDialog, HostedExportAs, HostedFormatOptions,
    JpegOptions, OutputPathRow, SizeEstimate, TargaOptions,
};
use crate::contract::Bind;
use crate::dialog::stories::{hosted, hosted_with};
use crate::patterns::form_dialogs::PreviewState;
use crate::primitives::TextureCache;
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Ui, Vec2};

/// The document's size in pixels.
const DOCUMENT: PixelSize = PixelSize::new(4000, 3000);
/// The preview picture's size in points at zoom 1: the document fitted to a 512 px proxy.
const PICTURE: [usize; 2] = [512, 384];
/// The size estimate of the preview, in bytes.
const ESTIMATE: u64 = 3_565_158;

/// A sky, a sun and hills.
fn landscape() -> ColorImage {
    let [width, height] = PICTURE;
    let (sun_x, sun_y, sun_r) = (width as f32 * 0.7, height as f32 * 0.38, height as f32 * 0.14);
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        let t = y as f32 / height as f32;
        for x in 0..width {
            let (mut r, mut g, mut b) = (40.0 + 210.0 * t, 90.0 + 80.0 * t, 160.0 - 50.0 * t);
            let (dx, dy) = (x as f32 - sun_x, y as f32 - sun_y);
            if dx * dx + dy * dy < sun_r * sun_r {
                (r, g, b) = (255.0, 232.0, 150.0);
            }
            let ridge = height as f32 * (0.68 + 0.08 * (x as f32 / width as f32 * 9.0).sin());
            if y as f32 > ridge {
                let shade = 0.55 + 0.25 * ((y as f32 - ridge) / height as f32);
                (r, g, b) = (40.0 * shade, 90.0 * shade, 60.0 * shade);
            }
            pixels.push(Color32::from_rgb(r as u8, g as u8, b as u8));
        }
    }
    ColorImage::new(PICTURE, pixels)
}

fn picture(ui: &Ui) -> SizedTexture {
    let id = TextureCache::texture(ui.ctx(), "form-dialogs-export-story", landscape);
    SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32))
}

const FORMATS: [(&str, &str); 6] = [("Png", "png"), ("Jpg", "jpg"), ("Webp", "webp"), ("Tiff", "tif"), ("Tga", "tga"), ("Ico", "ico")];

/// Export As on the story's dialog host, opening with the `format` value's format. With `preview_ready` the story
/// answers each preview the dialog asks for at once.
fn export_dialog(ui: &mut Ui, data: &mut StoryData, preview_ready: bool) -> StoryOutput {
    let format = (*data.choice("format", "jpg", &FORMATS)).to_owned();
    let ready = *data.value("preview_ready", preview_ready);
    let exact = *data.value("exact_size", false);
    let texture = picture(ui);
    let knobs = (format.clone(), exact);
    let open = || {
        let options = ExportOptions { format, ..ExportOptions::default() };
        HostedExportAs::new("Poster", DOCUMENT, options)
    };
    hosted_with(ui, data, knobs, open, |host| {
        let Some(dialog) = host.get_mut::<HostedExportAs>().filter(|_| ready) else { return };
        if let Some(wanted) = dialog.wanted_preview() {
            let bytes = if exact { SizeEstimate::Exact(ESTIMATE) } else { SizeEstimate::Proxy(ESTIMATE) };
            dialog.set_preview(ExportPreview::new(wanted.revision, PreviewState::Ready(texture)).estimate(bytes));
        }
    })
}

#[story(ExportAsDialog<'static>, covers = [], drive = Drive::Never)]
fn export_as(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    export_dialog(ui, data, true)
}

#[story(ExportAsDialog<'static>, covers = [], drive = Drive::Never)]
fn export_as_rendering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    export_dialog(ui, data, false)
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn jpeg_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quality = *data.ranged("quality", 6_u8, 0.0..=12.0);
    hosted(ui, data, quality, || HostedFormatOptions::new(ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(quality))))
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn tiff_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedFormatOptions::new(ExportOptionsKind::Tiff))
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn targa_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let alpha = *data.value("alpha", true);
    hosted(ui, data, alpha, || {
        HostedFormatOptions::new(ExportOptionsKind::Targa).initial(FormatOptions::Targa(TargaOptions::new(if alpha { 32 } else { 24 })))
    })
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn dds_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedFormatOptions::new(ExportOptionsKind::Dds))
}

const TEMPLATE_PATH: &str = "[Comp Name].[fileExtension]";
const CHOSEN_PATH: &str = "/renders/poster_final.mov";

#[story(OutputPathRow<'static>, covers = [], drive = Drive::Never)]
fn output_path_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let with_templates = *data.value("templates", true);
    let chosen = *data.value("file_chosen", false);
    let path = data.value("path", TEMPLATE_PATH.to_owned());
    if chosen != (*path == CHOSEN_PATH) {
        *path = if chosen { CHOSEN_PATH } else { TEMPLATE_PATH }.to_owned();
    }
    let mut row = OutputPathRow::new(Bind::bound(&mut *path));
    if with_templates {
        row = row.templates(vec![
            (TEMPLATE_PATH.to_owned(), Label::from("Comp name")),
            ("[Comp Name]_[#####].[fileExtension]".to_owned(), Label::from("Comp name and frame")),
        ]);
    }
    let output: StoryOutput = row.show(ui).into();
    let chosen = *path == CHOSEN_PATH;
    *data.value("file_chosen", chosen) = chosen;
    output
}
