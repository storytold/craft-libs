//! Stories of `PrintPanel` and `PrintDialog`.

use super::dialog::PrintPreview;
use super::{Duplex, PrintDialog, PrintMarks, PrintPanel, PrintSheet, Printer, Settings};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::patterns::form_dialogs::DialogFields;
use craft_ui::primitives::TextureCache;
use craft_ui::runtime::env::Message;
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Rect, Ui, Vec2, pos2};

fn printers() -> Vec<Printer> {
    vec![
        Printer::new("_192_0_2_15").label("192.0.2.15").is_default(true),
        Printer::new("Office_1").label("Office"),
        Printer::new("Office_2").label("Office"),
    ]
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), duplex: Some(Duplex::LongEdge), ..Settings::default() };
    let printers = printers();
    PrintPanel::new(data.bind("settings", start), &printers).show(ui).into()
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PrintPanel::new(data.bind("settings", Settings::default()), &[]).discovering(true).show(ui).into()
}

const PICTURE: [usize; 2] = [240, 160];

/// A letter sheet with the image box inset one inch, the printable area and, with `marks`, corner crop and registration
/// marks.
fn sheet(ui: &Ui, marks: bool) -> PrintSheet {
    let id = TextureCache::texture(ui.ctx(), "print-dialog-story", || ColorImage::filled(PICTURE, Color32::from_rgb(0x4a, 0x7f, 0xc1)));
    let picture = SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    let image = Rect::from_min_size(pos2(72.0, 72.0), Vec2::new(468.0, 312.0));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    let sheet = PrintSheet::new(Vec2::new(612.0, 792.0), image, picture, "468 × 312 pt".into())
        .printable(Rect::from_min_size(pos2(18.0, 18.0), Vec2::new(576.0, 756.0)));
    if marks { sheet.marks(PrintMarks { corner_crop: true, registration: true, ..PrintMarks::default() }) } else { sheet }
}

/// The sheet of the preview story: Letter paper, the image box centred over a sample photo.
fn preview_sheet(ui: &Ui) -> PrintSheet {
    let id = TextureCache::texture(ui.ctx(), "print-preview-story", || sample_picture(1, [240, 320]));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    let image = Rect::from_center_size(pos2(306.0, 396.0), Vec2::new(360.0, 480.0));
    // design: allow[literal-size/vec2](page and paper sizes are document points, story data)
    PrintSheet::new(Vec2::new(612.0, 792.0), image, SizedTexture::new(id, Vec2::new(240.0, 320.0)), "360 × 480 pt".into())
}

#[story(PrintPreview<'static>, covers = [Default])]
fn print_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = preview_sheet(ui);
    let (position, position_at) = data.value_at("position", sheet.image.min);
    PrintPreview::new("Print preview", &sheet, position_at.bind(position)).show(ui).into()
}

/// The image box hangs over the paper's right edge, a position the host refuses.
#[story(PrintPreview<'static>, covers = [Invalid], drive = Drive::Never)]
fn print_preview_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = preview_sheet(ui);
    let paper = Rect::from_min_size(pos2(0.0, 0.0), sheet.paper);
    let size = sheet.image.size();
    let position = data.bind("position", pos2(400.0, 156.0)).check_with((paper, size), |(paper, size), at| {
        if paper.contains_rect(Rect::from_min_size(*at, *size)) { Validity::Ok } else { Validity::Refuse(Message::new("Off the paper")) }
    });
    PrintPreview::new("Print preview", &sheet, position).show(ui).into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), duplex: Some(Duplex::LongEdge), ..Settings::default() };
    let printers = printers();
    let sheet = sheet(ui, true);
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default()), &sheet).ready(true).show(ui).into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = sheet(ui, false);
    PrintDialog::new(data.bind("settings", Settings::default()), &[], Bind::given(DialogFields::default()), &sheet).discovering(true).show(ui).into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_busy(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), ..Settings::default() };
    let printers = printers();
    let sheet = sheet(ui, false);
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default()), &sheet).busy(true).show(ui).into()
}
