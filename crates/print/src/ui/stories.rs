//! Stories of `PrintPanel` and `PrintDialog`.

use super::dialog::PrintPreview;
use super::{
    Destination, DriverControls, Duplex, PrintDialog, PrintLayout, PrintMarks, PrintMode, PrintPanel, PrintPanelLayout, PrintPresets, PrintSection,
    PrintSheet, Printer, Settings, SheetPage,
};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::patterns::form_dialogs::{DialogFields, PagerUnit, PanePager, Param, ParamKind};
use craft_ui::patterns::presets::{PresetError, PresetRow};
use craft_ui::primitives::TextureCache;
use craft_ui::runtime::env::Message;
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Pos2, Rect, TextureId, Ui, Vec2, pos2};

fn printers() -> Vec<Printer> {
    vec![
        Printer::new("_192_0_2_15").label("192.0.2.15").is_default(true),
        Printer::new("Office_1").label("Office"),
        Printer::new("Office_2").label("Office"),
    ]
}

/// The settings the stories start from: the Office printer.
fn office() -> Settings {
    Settings { destination: Destination::Printer("Office_1".to_owned()), ..Settings::default() }
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { duplex: Some(Duplex::LongEdge), ..office() };
    let printers = printers();
    PrintPanel::new(data.bind("settings", start), &printers).show(ui).into()
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PrintPanel::new(data.bind("settings", Settings::default()), &[]).discovering(true).show(ui).into()
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_printer_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    PrintPanel::new(data.bind("settings", office()), &printers).layout(PrintPanelLayout::PrinterOnly).driver(DriverControls::Page).show(ui).into()
}

/// Collate unchecked beside Copies, and Properties… after Refresh.
#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_collate_properties(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { copies: 3, collate: false, ..office() };
    let printers = printers();
    PrintPanel::new(data.bind("settings", start), &printers).driver(DriverControls::Properties).show(ui).into()
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

/// The image box of an `Image` sheet; the whole paper for any other.
fn image_box(sheet: &PrintSheet) -> Rect {
    sheet.image_box().unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, sheet.paper))
}

#[story(PrintPreview<'static>, covers = [Default])]
fn print_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = preview_sheet(ui);
    let (position, position_at) = data.value_at("position", image_box(&sheet).min);
    PrintPreview::new("Print preview", &sheet, position_at.bind(position)).show(ui).into()
}

/// Four pages on a sheet with borders and cut lines, one still rendering as a stand-in.
#[story(PrintPreview<'static>, covers = [])]
fn print_preview_pages(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = four_up_sheet(ui);
    let (position, position_at) = data.value_at("position", image_box(&sheet).min);
    PrintPreview::new("Print preview", &sheet, position_at.bind(position)).show(ui).into()
}

/// A message in place of the sheet.
#[story(PrintPreview<'static>, covers = [])]
fn print_preview_message(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = PrintSheet::message(LETTER, "The page range selects no pages.".into(), "No pages".into());
    let (position, position_at) = data.value_at("position", image_box(&sheet).min);
    PrintPreview::new("Print preview", &sheet, position_at.bind(position)).show(ui).into()
}

/// The image box hangs over the paper's right edge, a position the host refuses.
#[story(PrintPreview<'static>, covers = [Invalid], drive = Drive::Never)]
fn print_preview_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = preview_sheet(ui);
    let paper = Rect::from_min_size(pos2(0.0, 0.0), sheet.paper);
    let size = image_box(&sheet).size();
    let position = data.bind("position", pos2(400.0, 156.0)).check_with((paper, size), |(paper, size), at| {
        if paper.contains_rect(Rect::from_min_size(*at, *size)) { Validity::Ok } else { Validity::Refuse(Message::new("Off the paper")) }
    });
    PrintPreview::new("Print preview", &sheet, position).show(ui).into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { duplex: Some(Duplex::LongEdge), ..office() };
    let printers = printers();
    let sheet = sheet(ui, true);
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default())).sheet(&sheet).ready(true).show(ui).into()
}

/// The artwork outlines of the sections story's document, in document points: the page inset half an inch and the
/// image box of [`sheet`].
const SECTIONS_OUTLINES: [Rect; 2] =
    [Rect::from_min_max(pos2(36.0, 36.0), pos2(576.0, 756.0)), Rect::from_min_max(pos2(72.0, 72.0), pos2(540.0, 384.0))];

/// The sections beside the shown one, with a preset row, a one-line header and a document on three sheets.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_sections(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = office();
    let printers = printers();
    let sheet = sheet(ui, true).count(3).outlines(SECTIONS_OUTLINES.to_vec());
    let rows = [PresetRow::item("Default".to_owned(), "Default", 0), PresetRow::item("Proof".to_owned(), "Proof", 0)];
    let built_in = ["Default".to_owned()];
    let header = [Param::number("pagesPerSheet", "Pages per sheet", 1.0, 1.0, 0)];
    let sections = [
        PrintSection::new("Position and Size", vec![Param::number("scale", "Scale", 100.0, 1.0, 0)]),
        PrintSection::new("Printing Marks", vec![Param::number("markWeight", "Mark weight", 0.25, 0.25, 2)]),
    ];
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default()))
        .layout(PrintLayout::Sections)
        .sheet(&sheet)
        .presets(PrintPresets::new(&rows, Some("Proof".to_owned()), &built_in))
        .header(&header)
        .sections(&sections)
        .driver(DriverControls::Page)
        .ready(true)
        .show(ui)
        .into()
}

/// The sections layout while the host has no sheet to show: its message stands where the preview would.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_sections_no_sheet(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sections = [PrintSection::new("Position and Size", vec![Param::number("scale", "Scale", 100.0, 1.0, 0)])];
    PrintDialog::new(data.bind("settings", Settings::default()), &printers, Bind::given(DialogFields::default()))
        .layout(PrintLayout::Sections)
        .message("The document has no printable artwork")
        .sections(&sections)
        .show(ui)
        .into()
}

/// The preset editor: a name field whose refusal shows under it.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_preset_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sections = [PrintSection::new("Position and Size", vec![Param::number("scale", "Scale", 100.0, 1.0, 0)])];
    let ((settings, settings_at), (name, name_at)) = data.pair_at(("settings", Settings::default()), ("name", "Default".to_owned()));
    PrintDialog::new(settings_at.bind(settings), &[], Bind::given(DialogFields::default()))
        .layout(PrintLayout::Sections)
        .mode(PrintMode::PresetEditor)
        .preset_name(name_at.bind(name))
        .check_name(|name| name.eq_ignore_ascii_case("Default").then_some(PresetError::NameTaken))
        .sections(&sections)
        .show(ui)
        .into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let sheet = sheet(ui, false);
    PrintDialog::new(data.bind("settings", Settings::default()), &[], Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .discovering(true)
        .show(ui)
        .into()
}

#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_busy(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = sheet(ui, false);
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default())).sheet(&sheet).busy(true).show(ui).into()
}

/// Letter paper in points.
const LETTER: Vec2 = Vec2::new(612.0, 792.0);
/// Landscape Letter paper in points.
const LETTER_LANDSCAPE: Vec2 = Vec2::new(792.0, 612.0);
/// A Letter page inset half an inch, in points.
const LETTER_INSET: Vec2 = Vec2::new(540.0, 720.0);
/// The printable area of Letter paper, a quarter inch in, in points.
const LETTER_PRINTABLE: Vec2 = Vec2::new(576.0, 756.0);
/// One cell of four pages per Letter sheet, in points.
const FOUR_UP_CELL: Vec2 = Vec2::new(270.0, 360.0);
/// One page of a booklet on landscape Letter, in points.
const BOOKLET_PAGE: Vec2 = Vec2::new(342.0, 540.0);

/// The corners of `rect` with the picture upright: top-left, top-right, bottom-right, bottom-left.
fn upright(rect: Rect) -> [Pos2; 4] {
    [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
}

/// The whole of a page picture.
fn whole() -> Rect {
    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
}

/// The rendered picture of a document page.
fn page_picture(ui: &Ui) -> TextureId {
    TextureCache::texture(ui.ctx(), "print-page-story", || sample_picture(2, [170, 220]))
}

/// One Letter page with its picture, inset half an inch, and a note under the readout.
fn document_sheet(ui: &Ui) -> PrintSheet {
    let page = SheetPage::new(upright(Rect::from_min_size(pos2(36.0, 36.0), LETTER_INSET)), "1").picture(page_picture(ui), whole());
    PrintSheet::pages(LETTER, vec![page], "Letter, 1 page".into())
        .printable(Rect::from_min_size(pos2(18.0, 18.0), LETTER_PRINTABLE))
        .notes(vec!["Pages are scaled to fit the printable area.".into()])
}

/// Four pages on a Letter sheet with borders and cut lines; the first has its picture, the others are still rendering
/// and show stand-ins.
fn four_up_sheet(ui: &Ui) -> PrintSheet {
    let cells = [pos2(18.0, 18.0), pos2(324.0, 18.0), pos2(18.0, 414.0), pos2(324.0, 414.0)];
    let boxes: Vec<Rect> = cells.iter().map(|at| Rect::from_min_size(*at, FOUR_UP_CELL)).collect();
    let pages = boxes
        .iter()
        .enumerate()
        .map(|(at, rect)| {
            let page = SheetPage::new(upright(*rect), (at + 1).to_string());
            if at == 0 { page.picture(page_picture(ui), whole()) } else { page }
        })
        .collect();
    let cuts = vec![[pos2(306.0, 0.0), pos2(306.0, 792.0)], [pos2(0.0, 396.0), pos2(612.0, 396.0)]];
    PrintSheet::pages(LETTER, pages, "Letter, 4 pages per sheet".into()).borders(boxes).cuts(cuts)
}

/// A booklet sheet: two pages side by side on landscape Letter, the last page beside the first.
fn booklet_sheet(ui: &Ui) -> PrintSheet {
    let last = SheetPage::new(upright(Rect::from_min_size(pos2(36.0, 36.0), BOOKLET_PAGE)), "4");
    let first = SheetPage::new(upright(Rect::from_min_size(pos2(414.0, 36.0), BOOKLET_PAGE)), "1").picture(page_picture(ui), whole());
    PrintSheet::pages(LETTER_LANDSCAPE, vec![last, first], "Booklet, sheet 1 of 2, front".into()).cuts(vec![[pos2(396.0, 0.0), pos2(396.0, 612.0)]])
}

/// The page-handling section of a document print dialog.
fn page_handling() -> PrintSection {
    PrintSection::new(
        "Page Handling",
        vec![Param::new("autoRotate", "Auto-rotate pages", ParamKind::bool(true)), Param::new("reverse", "Reverse pages", ParamKind::bool(false))],
    )
}

/// The document layout over one Letter page, printing to the Office printer.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = document_sheet(ui);
    let sections = [page_handling()];
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .sections(&sections)
        .ready(true)
        .show(ui)
        .into()
}

/// Save as PDF chosen: the list ends with it and the accepting button reads Save as PDF.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_to_file(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = document_sheet(ui);
    let start = Settings { destination: Destination::File, ..Settings::default() };
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .ready(true)
        .show(ui)
        .into()
}

/// A browser printer whose own dialog decides the job: Copies, Collate, Grayscale and Two-sided are disabled under its
/// note.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_browser_printer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut printers = printers();
    printers.push(Printer::new("Browser").own_dialog("The browser's print dialog sets copies, collate, two-sided and colour."));
    let sheet = document_sheet(ui);
    let start = Settings { destination: Destination::Printer("Browser".to_owned()), ..Settings::default() };
    PrintDialog::new(data.bind("settings", start), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .ready(true)
        .show(ui)
        .into()
}

/// A booklet sets two-sided printing itself: the panel has no Two-sided row.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_booklet(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = booklet_sheet(ui);
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .duplex_by_layout(true)
        .ready(true)
        .show(ui)
        .into()
}

/// Four pages per sheet, three still rendering as stand-ins, with the pager on the first sheet.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_four_up(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = four_up_sheet(ui);
    let pager = PanePager { shown: 1, count: 3, back: false, next: true, unit: PagerUnit::Sheet, ..PanePager::default() };
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .preview_pager(pager)
        .ready(true)
        .show(ui)
        .into()
}

/// A message instead of the sheet: the page range selects no pages.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_message(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = PrintSheet::message(LETTER, "The page range selects no pages.".into(), "No pages".into());
    let pager = PanePager { shown: 0, count: 0, back: false, next: false, unit: PagerUnit::Sheet, ..PanePager::default() };
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .preview_pager(pager)
        .show(ui)
        .into()
}

/// The last sheet: the pager's next button is disabled.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_last_sheet(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = four_up_sheet(ui);
    let pager = PanePager { shown: 3, count: 3, back: true, next: false, unit: PagerUnit::Sheet, ..PanePager::default() };
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .preview_pager(pager)
        .ready(true)
        .show(ui)
        .into()
}

/// A section that starts folded under one that is open.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_folded_section(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = document_sheet(ui);
    let comments = PrintSection::new("Comments and Forms", vec![Param::new("comments", "Print comments", ParamKind::bool(false))]).folded(false);
    let sections = [page_handling().folded(true), comments];
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .sections(&sections)
        .ready(true)
        .show(ui)
        .into()
}

/// A section with buttons in its title row, one disabled.
#[story(PrintDialog<'static>, covers = [], drive = Drive::Never)]
fn print_dialog_section_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let printers = printers();
    let sheet = document_sheet(ui);
    let sections = [page_handling().button("pageSetup", "Page Setup…", true).button("resetPaper", "Reset", false)];
    PrintDialog::new(data.bind("settings", office()), &printers, Bind::given(DialogFields::default()))
        .sheet(&sheet)
        .layout(PrintLayout::Document)
        .sections(&sections)
        .ready(true)
        .show(ui)
        .into()
}
