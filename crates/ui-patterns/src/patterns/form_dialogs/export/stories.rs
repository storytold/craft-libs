//! Stories of the export dialogs.

use super::{
    ExportAsDialog, ExportOptions, ExportOptionsKind, ExportPreview, FormatOptions, FormatOptionsDialog, JpegOptions, OutputPathRow, SizeEstimate,
    TargaOptions,
};
use crate::contract::Bind;
use crate::patterns::form_dialogs::PreviewState;
use crate::primitives::TextureCache;
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Ui, Vec2};

/// The document's size in pixels.
const DOCUMENT: PixelSize = PixelSize::new(1200, 800);
/// The preview picture's size in points at zoom 1.
const PICTURE: [usize; 2] = [360, 240];
/// The size estimate of the preview, in bytes.
const ESTIMATE: u64 = 1_258_291;

fn picture(ui: &Ui) -> SizedTexture {
    let id = TextureCache::texture(ui.ctx(), "form-dialogs-export-story", || ColorImage::filled(PICTURE, Color32::from_rgb(0x6f, 0x8a, 0x4d)));
    SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32))
}

const FORMATS: [(&str, &str); 6] = [("Png", "png"), ("Jpg", "jpg"), ("Webp", "webp"), ("Tiff", "tiff"), ("Tga", "tga"), ("Ico", "ico")];

/// The dialog over the options the story keeps; the `format` value replaces the options' format when it changes, so a
/// pick inside the dialog stays until the value changes.
fn export_dialog(ui: &mut Ui, data: &mut StoryData, preview_ready: bool) -> StoryOutput {
    let format = (*data.choice("format", "png", &FORMATS)).to_owned();
    let ready = *data.value("preview_ready", preview_ready);
    let exact = *data.value("exact_size", false);
    let (options, applied) = data.value("export_options", (ExportOptions::default(), "png".to_owned()));
    if *applied != format {
        options.format.clone_from(&format);
        *applied = format;
    }
    let mut options = options.clone();
    let mut dialog = ExportAsDialog::new("Poster", DOCUMENT, Bind::bound(&mut options)).opened(Bind::bound(data.value("open", true)));
    if ready {
        let bytes = if exact { SizeEstimate::Exact(ESTIMATE) } else { SizeEstimate::Proxy(ESTIMATE) };
        dialog = dialog.preview(ExportPreview::new(1, PreviewState::Ready(picture(ui))).estimate(bytes));
    }
    let output: StoryOutput = dialog.show(ui).into();
    data.value("export_options", (ExportOptions::default(), "png".to_owned())).0 = options;
    output
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
    FormatOptionsDialog::new(ExportOptionsKind::Jpeg)
        .opened(Bind::bound(data.value("open", true)))
        .initial(FormatOptions::Jpeg(JpegOptions::new(quality)))
        .show(ui)
        .into()
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn tiff_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    FormatOptionsDialog::new(ExportOptionsKind::Tiff).opened(Bind::bound(data.value("open", true))).show(ui).into()
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn targa_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let alpha = *data.value("alpha", true);
    FormatOptionsDialog::new(ExportOptionsKind::Targa)
        .opened(Bind::bound(data.value("open", true)))
        .initial(FormatOptions::Targa(TargaOptions::new(if alpha { 32 } else { 24 })))
        .show(ui)
        .into()
}

#[story(FormatOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn dds_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    FormatOptionsDialog::new(ExportOptionsKind::Dds).opened(Bind::bound(data.value("open", true))).show(ui).into()
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
