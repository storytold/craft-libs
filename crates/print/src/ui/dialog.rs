//! `PrintDialog`: Photoshop's Print Settings (the preview pane, the printer panel and the app's
//! sections), and `HostedPrint`, the same dialog owned by a `DialogHost`.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use craft_behavior::{CursorIcon, EditEvent, EditEvents, IconId, Metrics, Mixed, PartKey, PointerButton, PressPhase, Sense, Value, keyed};
use craft_theme::variant::Tone;
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, LabelScheme, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, Scroll, Show as _, ShowContainer as _, Validity, WordSetCx as _, clicked,
};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::ListRow;
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::patterns::form_dialogs::{
    DialogFields, FieldValue, FormDialog, HostRequests, PagerStep, PanePager, Param, ParamKind, ParamRows, SectionList, set_color_field,
    set_param_field,
};
use craft_ui::patterns::presets::{
    NameCheck, NameRefuses, NamedPresetEvent, NamedPresetRow, Naming, PickerField, PresetError, PresetPicker, PresetRow, PresetSource, PresetState,
    PresetWords,
};
use craft_ui::primitives::{ContentPainterCx, TextRun, Wrap, fill_polygon};
use craft_ui::runtime::env::{Env, Label, Message};
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::requests::HostAnswer;
use craft_ui::sections::{DialogRole, FooterButton, HeaderStrength, Section};
use craft_ui::style::{ComponentStyle, Corners, RuleKind, StyleColor, StyleCx, SurfaceKind, TypeRole};
use craft_ui::text::TextField;
use egui::load::SizedTexture;
use egui::{Color32, Mesh, Pos2, Rect, Response, Shape, TextureId, Ui, Vec2, pos2, vec2};

use super::panel::{Destinations, DriverControls, MOST_COPIES, PrintPanel, PrintPanelLayout, PrintWords};
use super::{Destination, Duplex, Printer, Settings, State};

craft_ui::words! {
    /// The words of the print dialog.
    pub struct PrintDialogWords {
        title: "printDialog.title" = "Print Settings";
        title_document: "printDialog.titleDocument" = "Print";
        print: "printDialog.print" = "Print";
        done: "printDialog.done" = "Done";
        preview: "printDialog.preview" = "Print preview image";
        printer_setup: "printDialog.printerSetup" = "Printer Setup";
        image_box: "printDialog.imageBox" = "Image box";
        sections: "printDialog.sections" = "Sections";
        preset: "printDialog.preset" = "Print Preset";
        custom: "printDialog.custom" = "[Custom]";
        preset_name: "printDialog.presetName" = "Preset name";
        save_preset: "printDialog.savePreset" = "Save Preset";
        previous: "printDialog.previous" = "Previous";
        next: "printDialog.next" = "Next";
        sheet_of: template "printDialog.sheetOf" = "{n} of {count}";
    }
}

/// The index `FormOutput::picked` gives the Print button: Print is 0, Cancel 1, and the extra buttons start at 2.
const PRINT_BUTTON: usize = 0;

/// The index `FormOutput::picked` gives the Done button of the `Image` layout, its one extra button.
const DONE_BUTTON: usize = 2;

/// The image box of the preview.
const IMAGE: PartKey = PartKey::new("image");

/// The message a preview shows instead of a sheet.
const MESSAGE: PartKey = PartKey::new("message");

/// How far a crop or registration mark stands from the image box, and how long it is, in screen points.
const MARK_GAP: f32 = 4.0;
const MARK_LEN: f32 = 10.0;

/// Which marks the preview draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrintMarks {
    /// Corner crop marks.
    pub corner_crop: bool,
    /// Center crop marks.
    pub center_crop: bool,
    /// Registration marks.
    pub registration: bool,
    /// The description.
    pub description: bool,
    /// The labels.
    pub labels: bool,
}

/// How the print dialog is arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrintLayout {
    /// The preview at the start edge, the printer panel and the sections at the end edge.
    #[default]
    Image,
    /// Printer Setup and the app's sections in a scrolling column, then the sheet preview, its pager, the readout and
    /// the notes at the end edge; no Done button.
    Document,
    /// The preset row, the printer row and the header rows above a list of sections beside the shown one, with the
    /// preview under the list.
    Sections,
}

/// What the sheet preview shows on the paper.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SheetContent {
    /// One image box holding a picture; the box can be dragged.
    #[non_exhaustive]
    Image {
        /// The image box on the paper.
        image: egui::Rect,
        /// The picture of the document.
        picture: SizedTexture,
    },
    /// The pages placed on the sheet, clipped to the paper.
    Pages(Vec<SheetPage>),
    /// A message centred in the preview, with no paper.
    Message(Label),
}

/// One page placed on a sheet.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SheetPage {
    /// The page's corners on the paper, in paper points.
    pub corners: [Pos2; 4],
    /// The page's picture; none draws a muted stand-in with `number` centred.
    pub picture: Option<TextureId>,
    /// The part of the picture to draw.
    pub uv: egui::Rect,
    /// The page's number, shown on the stand-in.
    pub number: String,
}

impl SheetPage {
    /// A page at `corners` with no picture, numbered `number`.
    pub fn new(corners: [Pos2; 4], number: impl Into<String>) -> Self {
        SheetPage { corners, picture: None, uv: Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), number: number.into() }
    }

    /// The page's picture `id` and the part `uv` of it to draw.
    #[must_use]
    pub fn picture(mut self, id: TextureId, uv: egui::Rect) -> Self {
        self.picture = Some(id);
        self.uv = uv;
        self
    }
}

/// What the app's layout says for this pass, in points.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PrintSheet {
    /// The paper's size.
    pub paper: egui::Vec2,
    /// The printable area of the paper, when the printer says.
    pub printable: Option<egui::Rect>,
    /// The marks to draw, around the image box or the pages.
    pub marks: PrintMarks,
    /// The readout under the preview.
    pub readout: Label,
    /// How many sheets the document prints on; 1 for a document that fits one.
    pub count: usize,
    /// The outline of each sheet on the paper, in paper points, for a document that prints on several.
    pub outlines: Vec<egui::Rect>,
    /// What the sheet shows on the paper.
    pub content: SheetContent,
    /// The borders drawn around the pages, in paper points.
    pub borders: Vec<egui::Rect>,
    /// The cut lines, in paper points.
    pub cuts: Vec<[Pos2; 2]>,
    /// The notes under the readout.
    pub notes: Vec<Label>,
}

impl PrintSheet {
    /// A sheet of `paper` with the image box `image` holding `picture`, `readout` under the preview, no printable area
    /// and no marks.
    pub fn new(paper: egui::Vec2, image: egui::Rect, picture: SizedTexture, readout: Label) -> Self {
        Self::of(paper, SheetContent::Image { image, picture }, readout)
    }

    /// A sheet of `paper` holding `pages`, with `readout` under the preview.
    pub fn pages(paper: Vec2, pages: Vec<SheetPage>, readout: Label) -> Self {
        Self::of(paper, SheetContent::Pages(pages), readout)
    }

    /// A preview showing `message` instead of a sheet, with `readout` under it. `paper` keeps the preview the size the
    /// sheets around it take.
    pub fn message(paper: Vec2, message: Label, readout: Label) -> Self {
        Self::of(paper, SheetContent::Message(message), readout)
    }

    /// A sheet of `paper` showing `content`, with no printable area, no marks, borders, cuts or notes.
    fn of(paper: Vec2, content: SheetContent, readout: Label) -> Self {
        PrintSheet {
            paper,
            printable: None,
            marks: PrintMarks::default(),
            readout,
            count: 1,
            outlines: Vec::new(),
            content,
            borders: Vec::new(),
            cuts: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// The image box of an `Image` sheet; none for pages or a message.
    pub(super) fn image_box(&self) -> Option<Rect> {
        match &self.content {
            SheetContent::Image { image, .. } => Some(*image),
            SheetContent::Pages(_) | SheetContent::Message(_) => None,
        }
    }

    /// The borders drawn around the pages.
    #[must_use]
    pub fn borders(mut self, borders: Vec<egui::Rect>) -> Self {
        self.borders = borders;
        self
    }

    /// The cut lines.
    #[must_use]
    pub fn cuts(mut self, cuts: Vec<[Pos2; 2]>) -> Self {
        self.cuts = cuts;
        self
    }

    /// The notes under the readout.
    #[must_use]
    pub fn notes(mut self, notes: Vec<Label>) -> Self {
        self.notes = notes;
        self
    }

    /// The printable area of the paper.
    #[must_use]
    pub fn printable(mut self, area: egui::Rect) -> Self {
        self.printable = Some(area);
        self
    }

    /// The marks to draw.
    #[must_use]
    pub fn marks(mut self, marks: PrintMarks) -> Self {
        self.marks = marks;
        self
    }

    /// How many sheets the document prints on.
    #[must_use]
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }

    /// The outline of each sheet on the paper, in paper points.
    #[must_use]
    pub fn outlines(mut self, outlines: Vec<egui::Rect>) -> Self {
        self.outlines = outlines;
        self
    }
}

/// What the print dialog is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrintMode {
    /// Print the document.
    #[default]
    Print,
    /// Edit a print preset: a name field with its refusal in place of the printer and the job.
    PresetEditor,
}

/// The print presets a [`PrintDialog`] lists in its preset row.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PrintPresets<'a> {
    /// The presets, as the preset picker's rows.
    pub rows: &'a [PresetRow<String>],
    /// The preset the settings started from or were last filled from.
    pub held: Option<String>,
    /// The names of the built-in presets, which a typed name may not take.
    pub built_in: &'a [String],
}

impl<'a> PrintPresets<'a> {
    /// The presets `rows`, the `held` one and the `built_in` names.
    pub fn new(rows: &'a [PresetRow<String>], held: Option<String>, built_in: &'a [String]) -> Self {
        PrintPresets { rows, held, built_in }
    }
}

/// One titled section of the app's rows (Color Management, Position and Size, Printing Marks,
/// Functions, Save as PDF).
#[non_exhaustive]
pub struct PrintSection {
    /// The section's title.
    pub title: Label,
    /// Its rows.
    pub params: Vec<Param>,
    /// Whether the section can fold, and whether it starts open; `None` is a fixed section.
    folded: Option<bool>,
    /// The title-row buttons: the id a press files, the label and whether the button is enabled.
    buttons: Vec<(String, Label, bool)>,
    /// The rows stand without the section's heading in a document dialog (`PrintLayout::Document`).
    untitled: bool,
}

impl PrintSection {
    /// The section titled `title` with its `params` rows.
    pub fn new(title: impl Into<Label>, params: Vec<Param>) -> Self {
        Self { title: title.into(), params, folded: None, buttons: Vec::new(), untitled: false }
    }

    /// Collapses the section when `open` is false, through `Section::initially_open`.
    #[must_use]
    pub fn folded(mut self, open: bool) -> Self {
        self.folded = Some(open);
        self
    }

    /// In a document dialog (`PrintLayout::Document`) the section's rows follow the ones above with no heading of their
    /// own, as production's Paper rows follow Two-sided.
    #[must_use]
    pub fn untitled(mut self) -> Self {
        self.untitled = true;
        self
    }

    /// A title-row button; a press is filed as `HostRequests::request_action(id)`.
    #[must_use]
    pub fn button(mut self, id: impl Into<String>, label: impl Into<Label>, enabled: bool) -> Self {
        self.buttons.push((id.into(), label.into(), enabled));
        self
    }

    /// Shows the section with its rows over `values`. Its rows' requests and a title button's press go into `requests`.
    fn show(&self, ui: &mut Ui, at: usize, inline: bool, values: &mut DialogFields, requests: &mut HostRequests) {
        // Declared before `section`, whose controls closure borrows it.
        let mut pressed = None;
        // A document dialog's rows: the captions at the row start, the heading a small caption over them.
        let rows = |ui: &mut Ui, values: &mut DialogFields, requests: &mut HostRequests| {
            let rows = ParamRows::new(self.title.clone(), &self.params, Bind::bound(&mut *values)).heading(self.title.clone());
            let mut rows = if inline { rows.labels(LabelScheme::Inline) } else { rows }.show(ui);
            requests.absorb(&mut rows);
        };
        if inline && self.untitled && self.buttons.is_empty() {
            rows(ui, values, requests);
            return;
        }
        let mut section = match self.folded {
            Some(open) => Section::new(self.title.clone()).initially_open(open),
            None => Section::fixed(self.title.clone()),
        };
        if !self.buttons.is_empty() {
            let buttons = &self.buttons;
            let pressed = &mut pressed;
            section = section.controls(move |ui, _| {
                for (id, label, enabled) in buttons {
                    let button = Button::new(label.clone()).enabled(*enabled).id_salt(id.as_str()).show(ui);
                    if clicked(&button) {
                        *pressed = Some(id.clone());
                    }
                }
            });
        }
        if inline {
            section = section.strength(HeaderStrength::Overline);
        }
        section.id_salt(("section", at)).show(ui, |ui, _| rows(ui, values, requests));
        if let Some(id) = pressed {
            requests.request_action(id);
        }
    }
}

/// How a print dialog finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrintFinish {
    /// Print the document.
    Print,
    /// Close, keeping the settings.
    Done,
}

/// The margin round the paper in the preview's well, as a share of the well's width (production's is about 4%).
const PAPER_INSET: f32 = 0.04;

/// Where the paper stands in the preview's room: the paper's top-left corner and the screen points per paper point.
#[derive(Clone, Copy)]
struct Placement {
    origin: Pos2,
    scale: f32,
}

impl Placement {
    /// The screen rect of `paper`, a rect in paper points.
    fn rect(self, paper: Rect) -> Rect {
        Rect::from_min_size(self.point(paper.min), paper.size() * self.scale)
    }

    /// The screen point of `at`, a point in paper points.
    fn point(self, at: Pos2) -> Pos2 {
        self.origin + at.to_vec2() * self.scale
    }
}

/// The style of the preview: the room it asks for, the stand-in of a page with no picture and the text it shows.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(super) struct PrintPreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The fill of a page with no picture.
    stand_in: StyleColor,
    /// A stand-in's number and the message shown instead of a sheet.
    ink: StyleColor,
}

/// What a grayscale job's pictures are multiplied by: the light grey of production's preview. The vertex colour only
/// scales the picture's channels; the picture itself is rendered grey by the app.
const GRAYSCALE_TINT: Color32 = Color32::from_gray(235);

/// The textured quad of a page: `corners` (screen points) are the picture's top-left, top-right, bottom-right and
/// bottom-left, paired in that order with `uv`'s corners, so a turned placement turns the picture with it.
fn page_quad(texture: TextureId, corners: [Pos2; 4], uv: Rect, tint: Color32) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let uvs = [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        // White leaves the picture's own colours: the vertex colour multiplies the texture.
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

/// Paints `run` centred on `centre`.
fn centred_text(painter: &ContentPainterCx<'_>, run: &TextRun<'_>, centre: Pos2, width: f32) {
    painter.with_draw(|draw| {
        let layout = draw.layout(run, width);
        draw.galley(centre - layout.size() / 2.0, &layout);
    });
}

/// Paints `page` at `place`: its picture as a textured quad, or a stand-in with its number centred.
fn paint_page(painter: &ContentPainterCx<'_>, style: &PrintPreviewStyle, place: Placement, page: &SheetPage, tint: Color32) {
    let corners = page.corners.map(|corner| place.point(corner));
    match page.picture {
        Some(texture) => painter.data_shapes([Shape::mesh(page_quad(texture, corners, page.uv, tint))]),
        None => {
            painter.data_shapes([Shape::mesh(fill_polygon(&corners, style.stand_in.egui(), 0.0))]);
            let centre = Pos2::ZERO + corners.iter().fold(Vec2::ZERO, |sum, corner| sum + corner.to_vec2()) / 4.0;
            let run = TextRun::new(page.number.as_str()).role(TypeRole::Caption).color(style.ink);
            centred_text(painter, &run, centre, f32::INFINITY);
        }
    }
}

/// The paper, its printable area, the picture in its box and the marks. The value is where the image box's top-left
/// corner stands on the paper, in paper points; a drag of the image box reports the positions it moves the box to as
/// one gesture, and a host check can refuse a position.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub(super) struct PrintPreview<'a> {
    #[common]
    common: Common,
    sheet: &'a PrintSheet,
    /// The image box's top-left corner on the paper, in paper points.
    position: Bind<'a, Pos2>,
    /// The preview of a job that prints in grayscale (`on`): the pictures stand tinted grey, as production's do.
    #[prop(vis = "pub(super)")]
    grayscale: bool,
}

impl<'a> PrintPreview<'a> {
    pub(super) fn new(label: impl Into<Label>, sheet: &'a PrintSheet, position: Bind<'a, Pos2>) -> Self {
        PrintPreview { common: Common::new(), sheet, position, grayscale: false }.label(label)
    }

    /// The colour the pictures are multiplied by.
    fn tint(&self) -> Color32 {
        // design: allow[raw-colour/const](a white tint leaves the page picture's own colours unchanged)
        if self.grayscale { GRAYSCALE_TINT } else { Color32::WHITE }
    }

    fn place(&self, room: Rect) -> Option<Placement> {
        let paper = self.sheet.paper;
        if !(paper.x > 0.0 && paper.y > 0.0) {
            return None;
        }
        // The paper stands inset in its well, as production's does: a margin of the well's own colour round the sheet.
        let room = room.shrink(room.width() * PAPER_INSET);
        let scale = (room.width() / paper.x).min(room.height() / paper.y);
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        Some(Placement { origin: room.center() - paper * scale / 2.0, scale })
    }
}

impl Component for PrintPreview<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    /// One image box: its position has no mixed state.
    const SINGLE_VALUE: bool = true;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = PrintPreviewStyle;
    type Output = PrintPreviewOutput;

    fn style(cx: &StyleCx<'_>, _: PrintPreviewStyleKey) -> PrintPreviewStyle {
        let tokens = cx.tokens;
        PrintPreviewStyle { metrics: Metrics::new(), stand_in: cx.color(tokens.color.surface.sunken), ink: cx.color(tokens.color.text.muted) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = cx.tokens.size.plot.width;
        let paper = self.sheet.paper;
        let aspect = if paper.x > 0.0 && paper.y > 0.0 { paper.y / paper.x } else { 1.0 };
        // A message takes the lines it wraps to at this width, so a larger text size grows the part instead of
        // overflowing it.
        let message = match &self.sheet.content {
            SheetContent::Message(message) => cx.layout_at(&TextRun::new(cx.env.resolve(message).as_str()).wrap(Wrap::Wrap), width).size().y,
            _ => 0.0,
        };
        let inner = width * (1.0 - 2.0 * PAPER_INSET);
        Vec2::new(width, (inner * aspect + width * 2.0 * PAPER_INSET).max(message))
    }

    fn bind(&mut self) -> Option<BindMut<'_, Pos2>> {
        Some(self.position.reborrow())
    }

    /// The image box stays where the sheet puts it while it is dragged: the picture follows the pointer, the part does
    /// not, so the press keeps its hit. Only an `Image` sheet has the part: pages and a message are not dragged.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(image) = self.sheet.image_box() else { return };
        let Some(place) = self.place(rect) else { return };
        let part = cx.part(IMAGE, place.rect(image)).sense(Sense::DRAG).pointer_only("the Position and Size rows place the image");
        part.cursor(CursorIcon::Move);
    }

    /// Every phase of a press carries the press point and the gesture keeps the position it began from, so the drag
    /// reads both, keeping nothing across passes. Only a running drag takes the release: a part that senses no click
    /// reports `click: false` within the slop too.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(place) = self.place(cx.rect) else { return };
        let Some(result) = cx.part_if_declared(IMAGE) else { return };
        let phase = result.press.phase;
        let moved = |cx: &Cx<'_, Self>, start: craft_behavior::PressStart, pos: Pos2| match cx.gesture_before()? {
            Mixed::Same(from) => Some(from + (pos - start.origin) / place.scale),
            Mixed::Mixed => None,
        };
        match phase {
            PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                cx.begin();
                if let Some(at) = moved(cx, start, pos) {
                    cx.propose(at);
                }
            }
            PressPhase::Drag { start, pos, .. } | PressPhase::Release { start, pos, .. } if start.button == PointerButton::Primary => {
                if let Some(at) = moved(cx, start, pos) {
                    cx.propose(at);
                    if matches!(phase, PressPhase::Release { .. }) {
                        cx.commit();
                    }
                }
            }
            _ => {}
        }
    }

    /// The image box is a button; a message standing in for the sheet is a text node with the message.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let SheetContent::Message(message) = &self.sheet.content {
            let node = access.node(MESSAGE, Role::LABEL);
            node.name = Name::Text(cx.env.resolve(message).as_str().to_owned());
            node.bounds(cx.rect);
            return;
        }
        if self.sheet.image_box().is_none() {
            return;
        }
        let words = cx.words::<PrintDialogWords>();
        let node = access.node(IMAGE, Role::BUTTON);
        node.name = Name::Text(words.image_box.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let sheet = self.sheet;
        let style = &cx.style;
        // A message stands in for the whole sheet: no paper, the text centred in the room the sheets take.
        if let SheetContent::Message(message) = &sheet.content {
            let painter = ui.content_painter(cx);
            painter.surface(cx.rect, SurfaceKind::Well, Corners::None);
            let shown = cx.env.resolve(message);
            let run = TextRun::new(shown.as_str()).wrap(Wrap::Wrap).color(style.ink);
            centred_text(&painter, &run, cx.rect.center(), cx.rect.width());
            return;
        }
        let Some(place) = self.place(cx.rect) else { return };
        let tint = self.tint();
        let paper = place.rect(Rect::from_min_size(Pos2::ZERO, sheet.paper));
        {
            let painter = ui.content_painter(cx);
            painter.surface(cx.rect, SurfaceKind::Well, Corners::None);
            // Paper is white in every theme: the pages' own pictures are white pages, and the gaps between them are paper.
            // design: allow[raw-colour/const](the colour of printed paper, not of the interface)
            painter.data_shapes([Shape::rect_filled(paper, 0.0, Color32::WHITE)]);
        }
        // The box the marks stand around: the image box, or the pages' extent.
        let marked = match &sheet.content {
            SheetContent::Image { image, picture } => {
                let position = cx.value().copied().unwrap_or(image.min);
                let image = place.rect(Rect::from_min_size(position, image.size()));
                let visible = image.intersect(paper);
                if image.width() > 0.0 && image.height() > 0.0 && visible.width() > 0.0 && visible.height() > 0.0 {
                    let uv = Rect::from_min_max(
                        pos2((visible.min.x - image.min.x) / image.width(), (visible.min.y - image.min.y) / image.height()),
                        pos2((visible.max.x - image.min.x) / image.width(), (visible.max.y - image.min.y) / image.height()),
                    );
                    ui.content_painter(cx).data_shapes([Shape::mesh(page_quad(
                        picture.id,
                        [visible.left_top(), visible.right_top(), visible.right_bottom(), visible.left_bottom()],
                        uv,
                        tint,
                    ))]);
                }
                Some(image)
            }
            SheetContent::Pages(pages) => {
                // A page placed past the paper's edge prints cut there, so the preview cuts it too.
                ui.clipped(paper, |ui| {
                    let painter = ui.content_painter(cx);
                    for page in pages {
                        paint_page(&painter, style, place, page, tint);
                    }
                });
                let corners = pages.iter().flat_map(|page| page.corners.map(|corner| place.point(corner)));
                Some(Rect::from_points(&corners.collect::<Vec<_>>()).intersect(paper)).filter(|extent| extent.is_positive())
            }
            SheetContent::Message(_) => None,
        };
        let painter = ui.content_painter(cx);
        let rule = |kind: RuleKind, from: Pos2, to: Pos2| painter.with_draw(|draw| draw.rule(kind, from, to));
        let outline = |kind: RuleKind, rect: Rect| {
            rule(kind, rect.left_top(), rect.right_top());
            rule(kind, rect.right_top(), rect.right_bottom());
            rule(kind, rect.right_bottom(), rect.left_bottom());
            rule(kind, rect.left_bottom(), rect.left_top());
        };
        for border in &sheet.borders {
            outline(RuleKind::Edge, place.rect(*border));
        }
        for [from, to] in &sheet.cuts {
            rule(RuleKind::Grid, place.point(*from), place.point(*to));
        }
        if let Some(area) = sheet.printable {
            outline(RuleKind::Grid, place.rect(area));
        }
        for sheet_outline in &sheet.outlines {
            outline(RuleKind::Grid, place.rect(*sheet_outline));
        }
        outline(RuleKind::Edge, paper);
        let Some(image) = marked else { return };
        if sheet.image_box().is_some() {
            outline(RuleKind::DropMark, image);
        }

        let marks = sheet.marks;
        let sides = [
            (image.center_top(), vec2(0.0, -1.0)),
            (image.center_bottom(), vec2(0.0, 1.0)),
            (image.left_center(), vec2(-1.0, 0.0)),
            (image.right_center(), vec2(1.0, 0.0)),
        ];
        if marks.corner_crop {
            let corners =
                [(image.left_top(), -1.0, -1.0), (image.right_top(), 1.0, -1.0), (image.left_bottom(), -1.0, 1.0), (image.right_bottom(), 1.0, 1.0)];
            for (corner, sx, sy) in corners {
                rule(RuleKind::Edge, corner + vec2(sx * MARK_GAP, 0.0), corner + vec2(sx * (MARK_GAP + MARK_LEN), 0.0));
                rule(RuleKind::Edge, corner + vec2(0.0, sy * MARK_GAP), corner + vec2(0.0, sy * (MARK_GAP + MARK_LEN)));
            }
        }
        if marks.center_crop {
            for (middle, out) in sides {
                rule(RuleKind::Edge, middle + out * MARK_GAP, middle + out * (MARK_GAP + MARK_LEN));
            }
        }
        if marks.registration {
            let half = MARK_LEN / 2.0;
            for (middle, out) in sides {
                let at = middle + out * (MARK_GAP * 2.0 + MARK_LEN * 1.5);
                rule(RuleKind::Edge, at - vec2(half, 0.0), at + vec2(half, 0.0));
                rule(RuleKind::Edge, at - vec2(0.0, half), at + vec2(0.0, half));
            }
        }
        // The description and the labels are text of the printed page; the preview shows where each stands.
        if marks.description {
            let from = paper.left_bottom() + vec2(MARK_GAP, -MARK_GAP);
            // design: allow[literal-size/vec2](printer's marks are page geometry in points, not UI sizes)
            rule(RuleKind::Edge, from, from + vec2(MARK_LEN * 3.0, 0.0));
        }
        if marks.labels {
            let from = paper.left_top() + vec2(MARK_GAP, MARK_GAP);
            // design: allow[literal-size/vec2](printer's marks are page geometry in points, not UI sizes)
            rule(RuleKind::Edge, from, from + vec2(MARK_LEN * 3.0, 0.0));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PrintPreviewOutput {
        let scale = cx.env.scale().0;
        let paper_px = match &self.sheet.content {
            SheetContent::Message(_) => None,
            _ => self.place(cx.rect).map(|place| {
                let size = place.rect(Rect::from_min_size(Pos2::ZERO, self.sheet.paper)).size();
                // A float to integer cast saturates and maps NaN to zero, so an odd size cannot wrap.
                [(size.x * scale).round() as u32, (size.y * scale).round() as u32]
            }),
        };
        PrintPreviewOutput { edit: cx.edit_response(), paper_px }
    }
}

/// What the preview reports.
#[derive(Debug)]
pub(super) struct PrintPreviewOutput {
    /// The image box drag: `Begin` at the first move past the click slop, `Update` with the image box's position on the
    /// paper in paper points, `Commit` at the release.
    edit: EditResponse<Pos2>,
    /// The page box as drawn this pass, in physical pixels (its rect times `pixels_per_point`, rounded, saturating).
    /// `None` before the preview is drawn, while the paper has no area, and for a message in place of a sheet.
    paper_px: Option<[u32; 2]>,
}

impl Outcome for PrintPreviewOutput {
    type Value = Pos2;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// What the dialog keeps between passes for the values the host does not bind.
#[derive(Clone, Debug, Default)]
struct Own {
    section: String,
    sheet: usize,
    name: String,
    picker: PresetState<String>,
}

/// The value `bind` holds this pass; `None` while it holds none (a kept or mixed binding).
fn read<T: Value>(bind: &mut Bind<'_, T>) -> Option<T> {
    match bind.reborrow().get() {
        Some(Mixed::Same(value)) => Some(value),
        _ => None,
    }
}

/// Writes `value` through `bind` as one step. A given binding keeps its value: the host takes the dialog's reports.
// PLAN: form_dialogs' `read_bind` and `write_bind` (form.rs) do the same and are `pub(crate)`; the form_dialogs owner makes them public so this copy goes.
fn write<T: Value>(bind: &mut Bind<'_, T>, value: T) {
    let before = bind.reborrow().get().unwrap_or(Mixed::Mixed);
    let _ = bind.reborrow().apply(&EditEvent::set(before, value, Arc::from(""), None));
}

/// The host's binding when it gave one, else the dialog's own memory.
fn bound_or<'b, T: Value>(host: &'b mut Option<Bind<'_, T>>, own: &'b mut T) -> Bind<'b, T> {
    match host {
        Some(bind) => bind.reborrowed(),
        None => Bind::bound(own),
    }
}

/// Whether a preset name can be saved: it says something and the host's check lets it through. The one rule for the
/// dialog's OK and the hosted dialog's `confirm`.
fn usable_name(name: &str, refused: Option<PresetError>) -> bool {
    !name.trim().is_empty() && refused.is_none()
}

/// The preview with its readout and, for a document of several sheets, the step row; with no sheet, the host's message.
fn preview_room(
    ui: &mut Ui,
    words: &PrintDialogWords,
    sheet: Option<&PrintSheet>,
    message: Option<&Label>,
    index: &mut Bind<'_, usize>,
    moved: &mut EditEvents<Pos2>,
    stepped: &mut Option<usize>,
) {
    let Some(sheet) = sheet else {
        if let Some(message) = message {
            Text::new(message.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
        return;
    };
    let position = sheet.image_box().map_or(Pos2::ZERO, |image| image.min);
    let preview = PrintPreview::new(words.preview.clone(), sheet, Bind::given(position)).show(ui);
    moved.extend(preview.edit.events);
    Text::new(sheet.readout.clone()).show(ui);
    if sheet.count > 1 {
        step_row(ui, words, sheet.count, index, stepped);
    }
}

/// Previous, "{n} of {count}" and Next. A step writes the index and reports it for this pass.
fn step_row(ui: &mut Ui, words: &PrintDialogWords, count: usize, index: &mut Bind<'_, usize>, stepped: &mut Option<usize>) {
    let last = count.saturating_sub(1);
    let shown = read(index).unwrap_or(0).min(last);
    let mut to = None;
    Row::new().show(ui, |ui, _| {
        if clicked(&IconButton::new(IconId::new("chevron_left")).label(words.previous.clone()).enabled(shown > 0).show(ui)) {
            to = Some(shown - 1);
        }
        let place = words.sheet_of.fill(&[("n", (shown + 1).to_string().as_str()), ("count", count.to_string().as_str())]);
        Text::new(Label::from(place)).show(ui);
        if clicked(&IconButton::new(IconId::new("chevron_right")).label(words.next.clone()).enabled(shown < last).show(ui)) {
            to = Some(shown + 1);
        }
    });
    if let Some(to) = to {
        write(index, to);
        *stepped = Some(to);
    }
}

/// The preset row: the picker over the host's presets, `[Custom]` while the settings match none, and the row's events.
fn preset_row(
    ui: &mut Ui,
    words: &PrintDialogWords,
    presets: &PrintPresets<'_>,
    check: Option<&NameRefuses<'_>>,
    picker: &mut PresetState<String>,
    events: &mut Vec<NamedPresetEvent<String>>,
) {
    let source = PresetSource::listed(presets.rows);
    let mut pick = PresetPicker::new(words.preset.clone(), Bind::given(presets.held.clone()), &source, Bind::bound(picker)).field(PickerField::Name);
    if presets.held.is_none() {
        pick = pick.placeholder(words.custom.clone());
    }
    let mut row = NamedPresetRow::new(words.preset.clone(), pick).built_in(|key: &String| presets.built_in.contains(key));
    if let Some(check) = check {
        row = row.check_name(NameCheck::of_host(check));
    }
    events.extend(row.show(ui).events);
}

/// The preset editor's name field with the host's refusal under it. Returns whether the name can be saved.
fn name_row(ui: &mut Ui, words: &PrintDialogWords, check: Option<&NameRefuses<'_>>, name: &mut Bind<'_, String>) -> bool {
    let preset_words = Env::of(ui).words::<PresetWords>();
    let current = read(name).unwrap_or_default();
    let host = check.map(NameCheck::of_host);
    let naming = Naming::new(host.as_ref(), &preset_words.name_taken, &preset_words.name_invalid);
    let mut typed = None;
    Field::new(words.preset_name.clone()).id_salt("name").show(ui, |ui, _| {
        let checked = Bind::given(current.clone()).check_with(&naming, |naming, text| match naming.problem(text) {
            Some(message) => Validity::Refuse(Message::new(message)),
            None => Validity::Ok,
        });
        typed = TextField::new(words.preset_name.clone(), checked).id_salt("name-text").select_on_focus().show(ui).value().cloned();
    });
    let shown = match typed {
        Some(text) => {
            write(name, text.clone());
            text
        }
        None => current,
    };
    usable_name(&shown, check.and_then(|check| check(&shown)))
}

/// The rows that show `Settings` key `key` (`copies`, `duplex`, `grayscale`) in place of the printer panel: in the
/// sections layout the panel shows only the printer, and an app's section or header row with the key is the one the user
/// sees, so automation sets and lists that row.
fn row_of_setting<'p>(layout: PrintLayout, sections: &'p [PrintSection], header: &'p [Param], key: &str) -> Option<&'p [Param]> {
    if layout != PrintLayout::Sections || !matches!(key, "copies" | "duplex" | "grayscale") {
        return None;
    }
    let mut rows = sections.iter().map(|section| section.params.as_slice()).chain([header]);
    rows.find(|params| params.iter().any(|param| param.key == key))
}

/// `params` as rows over `values`; the colour, path and eyedropper requests they file go to `requests`.
fn param_rows(ui: &mut Ui, requests: &mut HostRequests, title: &Label, params: &[Param], values: &mut DialogFields) {
    let mut rows = ParamRows::new(title.clone(), params, Bind::bound(values)).show(ui);
    requests.absorb(&mut rows);
}

/// Photoshop's Print Settings: the preview at the start edge (paper, printable area, image box,
/// marks, readout), the printer panel and the sections at the end edge, and Print, Cancel and Done
/// in the footer. With [`PrintLayout::Sections`] the preset row, the printer row and the header rows
/// stand above a list of sections beside the shown one, and the preview sits under the list.
///
/// - [`PrintLayout::Document`] puts Printer Setup and the sections in a scrolling column, then the sheet preview with
///   the pager centred under it, the readout and the notes at the end edge. It has no Done button, the printer list
///   ends with Save as PDF, and Print reads Save as PDF while that is chosen. A pager press is filed with
///   `HostRequests::request_preview_step`.
/// - A section's title buttons and its custom rows' buttons are filed with `HostRequests::request_action`.
#[must_use = "show the dialog with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct PrintDialog<'a> {
    settings: Bind<'a, Settings>,
    printers: &'a [Printer],
    fields: Bind<'a, DialogFields>,
    /// This pass's layout, from the app; without one the dialog shows no preview, and `message` in its place.
    #[prop]
    sheet: Option<&'a PrintSheet>,
    /// The app's sections under the printer panel.
    #[prop]
    sections: &'a [PrintSection],
    /// Binds the open state.
    #[prop]
    opened: Option<Bind<'a, bool>>,
    /// A job is running.
    #[prop]
    busy: bool,
    /// Printer discovery is running.
    #[prop]
    discovering: bool,
    /// A job can start: the caller's `State::ready`, the one rule for it. Print is disabled while it is false.
    #[prop]
    ready: bool,
    /// The host's answers to the dialog's requests.
    #[prop]
    answers: Vec<HostAnswer>,
    /// Binds the number of the sheet the preview shows, for a document that prints on several.
    #[prop]
    sheet_index: Option<Bind<'a, usize>>,
    /// A line of the host's under the preview.
    #[prop(into)]
    message: Option<Label>,
    /// How the dialog is arranged.
    #[prop]
    layout: PrintLayout,
    /// Binds the shown section, with [`PrintLayout::Sections`].
    #[prop]
    section: Option<Bind<'a, String>>,
    /// The print presets the preset row lists, with [`PrintLayout::Sections`].
    #[prop]
    presets: Option<PrintPresets<'a>>,
    /// Rows above the sections, with [`PrintLayout::Sections`].
    #[prop]
    header: &'a [Param],
    /// What the dialog is for.
    #[prop]
    mode: PrintMode,
    /// Binds the preset editor's name.
    #[prop]
    preset_name: Option<Bind<'a, String>>,
    /// Whether a typed preset name may be used, in the preset editor.
    #[prop(boxed = "Fn(&str) -> Option<PresetError> + 'a")]
    check_name: Option<Box<NameRefuses<'a>>>,
    /// The printer-driver controls the panel offers.
    #[prop]
    driver: DriverControls,
    /// The layout sets two-sided printing: the Two-sided row is left out.
    #[prop]
    duplex_by_layout: bool,
    /// The pager under the sheet preview in the `Document` layout.
    #[prop]
    preview_pager: Option<PanePager>,
    /// The printer list searches a long fleet ([`PrintPanel::printer_search`]).
    #[prop]
    printer_search: bool,
    /// Makes Print keep the dialog open and report its press in `PrintDialogOutput::held`.
    #[prop(vis = "pub(crate)")]
    hold_ok: bool,
    /// Closes the dialog as Print in this pass: the host accepted the held answer.
    #[prop(vis = "pub(crate)")]
    finish_held: bool,
    /// Shows the reason under the rows while the settings and the values are the refused ones.
    #[prop(vis = "pub(crate)")]
    refusal: Option<(Label, Settings, DialogFields)>,
}

/// What [`PrintDialog::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies what the dialog reports"]
pub struct PrintDialogOutput {
    /// The dialog.
    pub response: Response,
    /// How the dialog closed this pass.
    pub closed: Option<CloseReason>,
    /// Print or Done was chosen.
    pub finish: Option<PrintFinish>,
    /// The image box drag as the box's top-left corner on the paper, in paper points.
    pub moved: EditEvents<egui::Pos2>,
    /// The panel's Refresh was pressed.
    pub refresh: bool,
    /// The panel's Properties… was pressed.
    pub(crate) settings_clicked: bool,
    /// The dialog's requests of the host.
    pub requests: HostRequests,
    /// What the user did to the print presets this pass (Create, Rename, Delete).
    pub presets: Vec<NamedPresetEvent<String>>,
    /// The sheet the user stepped to this pass.
    pub sheet: Option<usize>,
    /// Print Settings… was pressed this pass: the host opens the driver's own dialog.
    pub setup: bool,
    /// The preview's page box as drawn this pass, in physical pixels: the size to render previews at. `None` before it
    /// is drawn.
    pub paper_px: Option<[u32; 2]>,
    /// The pager press made this pass.
    pub preview_step: Option<PagerStep>,
    /// The destination the user picked in the panel this pass.
    pub(crate) printer_chosen: Option<Destination>,
    /// Print was pressed on a dialog that holds its OK: the host files the answer.
    pub(crate) held: bool,
}

impl Outcome for PrintDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> PrintDialog<'a> {
    /// The dialog over `settings`, the `printers` found and the sections' `fields`.
    pub fn new(settings: Bind<'a, Settings>, printers: &'a [Printer], fields: Bind<'a, DialogFields>) -> Self {
        PrintDialog {
            settings,
            printers,
            fields,
            sheet: None,
            sections: &[],
            opened: None,
            busy: false,
            discovering: false,
            ready: false,
            answers: Vec::new(),
            sheet_index: None,
            message: None,
            layout: PrintLayout::default(),
            section: None,
            presets: None,
            header: &[],
            mode: PrintMode::default(),
            preset_name: None,
            check_name: None,
            driver: DriverControls::None,
            duplex_by_layout: false,
            preview_pager: None,
            printer_search: false,
            hold_ok: false,
            finish_held: false,
            refusal: None,
        }
    }

    /// Draws the dialog. The body reports the drag events, the Refresh, Setup, pager and section button presses, the
    /// preset events, the sheet step and the destination picked of its last run only, so a measuring run adds none; a
    /// second pass of a frame has no new input and reports none.
    pub fn show(self, ui: &mut Ui) -> PrintDialogOutput {
        let PrintDialog {
            mut settings,
            printers,
            fields,
            sheet,
            sections,
            opened,
            busy,
            discovering,
            ready,
            answers,
            mut sheet_index,
            message,
            layout,
            mut section,
            presets,
            header,
            mode,
            mut preset_name,
            check_name,
            driver,
            duplex_by_layout,
            preview_pager,
            printer_search,
            hold_ok,
            finish_held,
            refusal,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<PrintDialogWords>();
        let editor = mode == PrintMode::PresetEditor;
        let document = layout == PrintLayout::Document;
        let grayscale = matches!(settings.reborrow().get(), Some(Mixed::Same(Settings { grayscale: true, .. })));
        let to_file = matches!(settings.reborrow().get(), Some(Mixed::Same(Settings { destination: Destination::File, .. })));
        let title = if document { words.title_document.clone() } else { words.title.clone() };
        let ok = if editor {
            words.save_preset.clone()
        } else if document && to_file {
            env.words::<PrintWords>().save_as_pdf.clone()
        } else {
            words.print.clone()
        };
        let destinations = if document { Destinations::PrintersAndFile } else { Destinations::Printers };
        let titles: Vec<String> = sections.iter().map(|section| section.title.as_str().to_owned()).collect();
        let mut form = FormDialog::new(title, fields).ok_label(ok).ok_enabled(editor || (ready && !busy)).accept_stays_open(hold_ok).answers(answers);
        if !editor && !document {
            form = form.extra_buttons([FooterButton::new(words.done.clone(), DialogRole::Other)]);
        }
        if let Some(opened) = opened {
            form = form.opened(opened);
        }
        let mut moved = EditEvents::new();
        let mut refresh = false;
        let mut setup = false;
        let mut preset_events: Vec<NamedPresetEvent<String>> = Vec::new();
        let mut stepped = None;
        let mut settings_clicked = false;
        let mut paper_px = None;
        let mut preview_step = None;
        let mut printer_chosen = None;
        let shown = form.show(ui, |ui, form, values| {
            moved = EditEvents::new();
            refresh = false;
            setup = false;
            preset_events.clear();
            stepped = None;
            settings_clicked = false;
            paper_px = None;
            preview_step = None;
            printer_chosen = None;
            // A measuring run closes nothing: the real run of the same pass makes the close.
            if finish_held && !craft_ui::runtime::measure::active(ui.ctx()) {
                form.request_close_as(CloseReason::Activated);
            }
            for answer in form.take_answers() {
                match answer {
                    HostAnswer::Color { key, color } | HostAnswer::Eyedropper { key, color } => {
                        for params in sections.iter().map(|section| section.params.as_slice()).chain([header]) {
                            set_color_field(params, values, &key, color);
                        }
                    }
                    HostAnswer::Path { key, path } => values.set(&key, FieldValue::Text(path.to_string_lossy().into_owned())),
                    _ => {}
                }
            }
            let mut own = form.memory::<Own>().clone();
            let accepted = {
                let Own { section: own_section, sheet: own_sheet, name: own_name, picker } = &mut own;
                let mut section_bind = bound_or(&mut section, own_section);
                let mut index_bind = bound_or(&mut sheet_index, own_sheet);
                let mut name_bind = bound_or(&mut preset_name, own_name);
                if editor {
                    name_row(ui, &words, check_name.as_deref(), &mut name_bind)
                } else {
                    // Printer Setup and the app's sections.
                    let mut printer_setup = |ui: &mut Ui, values: &mut DialogFields, requests: &mut HostRequests| {
                        let mut printer = |ui: &mut Ui| {
                            let panel = PrintPanel::new(settings.reborrowed(), printers)
                                .discovering(discovering)
                                .busy(busy)
                                .driver(driver)
                                .destinations(destinations)
                                .duplex_by_layout(duplex_by_layout)
                                .inline(document)
                                .printer_search(printer_search)
                                .show(ui);
                            refresh |= panel.refresh;
                            settings_clicked |= panel.settings_clicked;
                            if panel.printer_chosen.is_some() {
                                printer_chosen = panel.printer_chosen;
                            }
                        };
                        // A document dialog's printer rows head the dialog with no heading of their own, as production's.
                        if document {
                            printer(ui);
                        } else {
                            Section::fixed(words.printer_setup.clone()).id_salt("printer").show(ui, |ui, _| printer(ui));
                        }
                        for (at, section) in sections.iter().enumerate() {
                            section.show(ui, at, document, values, requests);
                        }
                    };
                    // The sheet preview, the pager under it in the `Document` layout, the readout and the notes.
                    let mut sheet_column = |ui: &mut Ui, moved: &mut EditEvents<Pos2>| {
                        let Some(sheet) = sheet else {
                            if let Some(message) = message.as_ref() {
                                Text::new(message.clone()).show(ui);
                            }
                            return None;
                        };
                        let position = sheet.image_box().map_or(Pos2::ZERO, |image| image.min);
                        let preview = PrintPreview::new(words.preview.clone(), sheet, Bind::given(position)).grayscale(grayscale).show(ui);
                        moved.extend(preview.edit.events);
                        paper_px = preview.paper_px;
                        let mut step = None;
                        if let Some(pager) = preview_pager.as_ref().filter(|_| document) {
                            Row::new().align(RowAlign::Center).fill_width().show(ui, |ui, _| step = pager.clone().show(ui));
                        }
                        Text::new(sheet.readout.clone()).show(ui);
                        for note in &sheet.notes {
                            Text::new(note.clone()).shade(Shade::Muted).show(ui);
                        }
                        if let Some(message) = message.as_ref() {
                            Text::new(message.clone()).show(ui);
                        }
                        step
                    };
                    match layout {
                        PrintLayout::Image => {
                            Row::new().show(ui, |ui, _| {
                                Stack::new().show(ui, |ui, _| {
                                    if preview_pager.is_none() && sheet.is_some_and(|s| s.count > 1) {
                                        preview_room(ui, &words, sheet, message.as_ref(), &mut index_bind, &mut moved, &mut stepped);
                                    } else {
                                        preview_step = sheet_column(ui, &mut moved);
                                    }
                                });
                                Stack::new().show(ui, |ui, _| printer_setup(ui, values, form.requests()));
                            });
                        }
                        PrintLayout::Document => {
                            Row::new().show(ui, |ui, _| {
                                Stack::new().scroll(Scroll::vertical()).show(ui, |ui, _| printer_setup(ui, values, form.requests()));
                                Stack::new().show(ui, |ui, _| preview_step = sheet_column(ui, &mut moved));
                            });
                        }
                        PrintLayout::Sections => {
                            let mut preview =
                                |ui: &mut Ui| preview_room(ui, &words, sheet, message.as_ref(), &mut index_bind, &mut moved, &mut stepped);
                            Stack::new().show(ui, |ui, _| {
                                if let Some(presets) = &presets {
                                    preset_row(ui, &words, presets, check_name.as_deref(), picker, &mut preset_events);
                                }
                                let panel = PrintPanel::new(settings.reborrowed(), printers)
                                    .layout(PrintPanelLayout::PrinterOnly)
                                    .driver(driver)
                                    .discovering(discovering)
                                    .busy(busy)
                                    .show(ui);
                                refresh |= panel.refresh;
                                setup |= panel.settings_clicked;
                                if !header.is_empty() {
                                    param_rows(ui, form.requests(), &words.title, header, values);
                                }
                                let current = read(&mut section_bind).filter(|title| titles.contains(title)).or_else(|| titles.first().cloned());
                                let Some(current) = current else {
                                    preview(ui);
                                    return;
                                };
                                let source = keyed(&titles, |title: &String| title.clone());
                                // PLAN: assumes `SectionList::foot(self, impl FnOnce(&mut Ui) + 'a) -> Self` (type-w6 request on ty.X70).
                                let picked = SectionList::new(words.sections.clone(), &source, current.clone()).foot(|ui| preview(ui)).show(
                                    ui,
                                    |title| ListRow::new(Label::from(title.clone())),
                                    |ui| {
                                        if let Some(section) = sections.iter().find(|section| section.title.as_str() == current) {
                                            param_rows(ui, form.requests(), &section.title, &section.params, values);
                                        }
                                    },
                                );
                                if let Some(title) = picked {
                                    write(&mut section_bind, title);
                                }
                            });
                        }
                    }
                    if let Some(step) = preview_step {
                        form.requests().request_preview_step(step);
                    }
                    // The host's refusal of the held OK stays until a setting or a value changes.
                    let now = settings.reborrow().get();
                    let refused = refusal
                        .as_ref()
                        .filter(|(_, refused, fields)| *fields == *values && matches!(&now, Some(Mixed::Same(now)) if now == refused));
                    if let Some((reason, ..)) = refused {
                        Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
                    }
                    ready && !busy
                }
            };
            *form.memory::<Own>() = own;
            accepted
        });
        let finish = if shown.accepted.is_some() {
            Some(PrintFinish::Print)
        } else if !document && shown.picked == Some(DONE_BUTTON) {
            Some(PrintFinish::Done)
        } else {
            None
        };
        // `picked` is this pass's press: a second pass of the frame has none, so a held Print is reported once.
        let held = hold_ok && shown.picked == Some(PRINT_BUTTON);
        PrintDialogOutput {
            response: shown.response,
            closed: shown.closed,
            finish,
            moved,
            refresh,
            settings_clicked,
            requests: shown.requests,
            presets: preset_events,
            sheet: stepped,
            setup,
            paper_px,
            preview_step,
            printer_chosen,
            held,
        }
    }
}

/// What a finished [`HostedPrint`] gives the app.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PrintAnswer {
    /// Print or Done.
    pub finish: PrintFinish,
    /// The job settings.
    pub settings: Settings,
    /// The sections' values.
    pub fields: DialogFields,
}

/// The preset rows the host listed, the one the settings started from and the built-in names.
struct HeldPresets {
    rows: Vec<PresetRow<String>>,
    held: Option<String>,
    built_in: Vec<String>,
}

/// The check on a typed preset name, as the host gave it.
type NameFn = dyn Fn(&str) -> Option<PresetError> + Send;

/// The print dialog owned by a `DialogHost` (not registered). It polls its [`State`] each pass,
/// asks the app for a printer list with [`take_refresh`](Self::take_refresh) and takes the list
/// with [`listen`](Self::listen).
///
/// With [`held_ok`](Self::held_ok), Print files its answer for the app ([`take_ok`](Self::take_ok)) and waits disabled
/// until [`answer_ok`](Self::answer_ok): `Ok` closes the dialog with the held answer at the next pass, `Err` shows the
/// reason under the rows until a setting or a value changes, and [`release_ok`](Self::release_ok) takes the held answer
/// back without a reason.
pub struct HostedPrint {
    id: DialogId,
    state: State,
    fields: DialogFields,
    sections: Vec<PrintSection>,
    sheet: Option<PrintSheet>,
    moves: Vec<EditEvent<egui::Pos2>>,
    refresh: bool,
    /// The panel's Properties… press, not taken yet.
    settings_clicked: bool,
    /// The page box the preview drew, in physical pixels, not taken yet.
    paper_px: Option<[u32; 2]>,
    requests: HostRequests,
    json_form: Option<Box<JsonForm>>,
    layout: PrintLayout,
    mode: PrintMode,
    header: Vec<Param>,
    check_name: Option<Box<NameFn>>,
    presets: Option<HeldPresets>,
    preset_events: Vec<NamedPresetEvent<String>>,
    message: Option<Label>,
    /// The title of the shown section; one the list does not have shows the first.
    section: String,
    sheet_index: usize,
    preset_name: String,
    driver: DriverControls,
    setup: bool,
    duplex_by_layout: bool,
    preview_pager: Option<PanePager>,
    printer_search: bool,
    /// The destination the user picked, or automation set, not taken yet.
    printer_chosen: Option<Destination>,
    /// `choice: "done"` came in: the next `confirm` answers Done.
    done_chosen: bool,
    /// Whether Print files its answer for the app instead of closing.
    hold: bool,
    /// The Print the app holds back until it answers, and its refusal.
    held: HeldAnswer<PrintAnswer>,
    /// The app's say on whether the settings can print (a page range that names no page cannot): Print is disabled while
    /// it is false, on top of the state's own readiness.
    settings_valid: bool,
}

/// Adds the app's own keys to the automation listing of the sections' values.
type JsonForm = dyn Fn(&DialogFields, &mut serde_json::Map<String, serde_json::Value>) + Send;

impl HostedPrint {
    /// The dialog over `state`, the sections' `fields` and the `sections`.
    pub fn new(state: State, fields: DialogFields, sections: Vec<PrintSection>) -> Self {
        HostedPrint {
            id: dialog_id("print"),
            state,
            fields,
            sections,
            sheet: None,
            moves: Vec::new(),
            refresh: true,
            settings_clicked: false,
            paper_px: None,
            requests: HostRequests::new(),
            json_form: None,
            layout: PrintLayout::default(),
            mode: PrintMode::default(),
            header: Vec::new(),
            check_name: None,
            presets: None,
            preset_events: Vec::new(),
            message: None,
            section: String::new(),
            sheet_index: 0,
            preset_name: String::new(),
            driver: DriverControls::None,
            setup: false,
            duplex_by_layout: false,
            preview_pager: None,
            printer_search: false,
            printer_chosen: None,
            done_chosen: false,
            hold: false,
            held: HeldAnswer::new(),
            settings_valid: true,
        }
    }

    /// What the dialog is for.
    #[must_use]
    pub fn mode(mut self, mode: PrintMode) -> Self {
        self.mode = mode;
        self
    }

    /// Rows above the sections.
    #[must_use]
    pub fn header(mut self, header: Vec<Param>) -> Self {
        self.header = header;
        self
    }

    /// Whether a typed preset name may be used, in the preset editor.
    #[must_use]
    pub fn check_name(mut self, check: impl Fn(&str) -> Option<PresetError> + Send + 'static) -> Self {
        self.check_name = Some(Box::new(check));
        self
    }

    /// Shows the driver's controls in the printer row.
    #[must_use]
    pub fn driver(mut self, driver: DriverControls) -> Self {
        self.driver = driver;
        self
    }

    /// The print presets the preset row lists, the one the settings started from, and the built-in names.
    pub fn set_presets(&mut self, rows: Vec<PresetRow<String>>, held: Option<String>, built_in: Vec<String>) {
        self.presets = Some(HeldPresets { rows, held, built_in });
    }

    /// What the user did to the print presets since the last call.
    pub fn take_preset_events(&mut self) -> Vec<NamedPresetEvent<String>> {
        std::mem::take(&mut self.preset_events)
    }

    /// A line of the host's under the preview.
    pub fn set_message(&mut self, message: Option<Label>) {
        self.message = message;
    }

    /// The number of the sheet the preview shows.
    pub fn sheet_index(&self) -> usize {
        self.sheet_index
    }

    /// The preset editor's name.
    pub fn preset_name(&self) -> &str {
        &self.preset_name
    }

    /// The sections' values.
    pub fn fields(&self) -> &DialogFields {
        &self.fields
    }

    /// Sets the section value `key`, unchecked: the app writes a value its own custom row holds.
    pub fn set_field(&mut self, key: impl Into<String>, value: FieldValue) {
        let key: String = key.into();
        self.fields.set(&key, value);
    }

    /// Print Settings… was pressed since the last call: the host opens the driver's own dialog.
    pub fn take_setup(&mut self) -> bool {
        std::mem::take(&mut self.setup)
    }

    /// Tells the dialog whether the settings the app owns are valid (they are until told otherwise). While they are not,
    /// Print is disabled and Enter prints nothing.
    pub fn set_settings_valid(&mut self, valid: bool) {
        self.settings_valid = valid;
    }

    /// Lists keys the sections' values do not show as they are: `form` gets the values and the listing, and may replace or
    /// add entries (a custom paper as `[width, height]`). It runs after the `Settings` keys are added.
    #[must_use]
    pub fn json_form(mut self, form: impl Fn(&DialogFields, &mut serde_json::Map<String, serde_json::Value>) + Send + 'static) -> Self {
        self.json_form = Some(Box::new(form));
        self
    }

    /// Whether the app should send a printer list: once on the first pass and on each Refresh. It answers once: a
    /// second pass of the same frame gets `false`, the app having answered in the first.
    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    /// Whether the panel's Properties… was pressed since the last call: the app opens the driver's options. The press is
    /// filed in the pass that receives it and taken once.
    pub fn take_settings_clicked(&mut self) -> bool {
        std::mem::take(&mut self.settings_clicked)
    }

    /// The page box the preview drew, in physical pixels (width, height), for the app to render the preview at. It is
    /// filed in the pass that drew the preview and taken once: `None` until the next pass that draws it, and while the
    /// preview shows a message or a paper with no area.
    pub fn take_paper_px(&mut self) -> Option<[u32; 2]> {
        self.paper_px.take()
    }

    /// Asks for a printer list again on the next pass, as Refresh does: the list the app sent was not the system's
    /// (its spooler was busy).
    pub fn request_refresh(&mut self) {
        self.refresh = true;
    }

    /// Takes the printer list the app sends; `false` while a discovery or a job runs.
    pub fn listen(&mut self, printers: Receiver<Vec<Printer>>) -> bool {
        self.state.listen(printers)
    }

    /// This pass's layout, from the app; `None` shows the dialog without a preview, and the message in its place.
    pub fn set_sheet(&mut self, sheet: impl Into<Option<PrintSheet>>) {
        self.sheet = sheet.into();
    }

    /// The image box drags since the last call.
    pub fn take_moves(&mut self) -> EditEvents<egui::Pos2> {
        std::mem::take(&mut self.moves).into_iter().collect()
    }

    /// The sections' values, for the app to read and write.
    pub fn fields_mut(&mut self) -> &mut DialogFields {
        &mut self.fields
    }

    /// The colour, Browse and eyedropper requests the sections filed, and where the app answers them.
    pub fn requests_mut(&mut self) -> &mut HostRequests {
        &mut self.requests
    }

    /// The dialog's arrangement.
    #[must_use]
    pub fn layout(mut self, layout: PrintLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Replaces the app's sections.
    pub fn set_sections(&mut self, sections: Vec<PrintSection>) {
        self.sections = sections;
    }

    /// The printer state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The printer state, for the app to change.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// The printer-driver controls the panel offers.
    pub fn set_driver(&mut self, driver: DriverControls) {
        self.driver = driver;
    }

    /// Whether the layout sets two-sided printing, leaving out the Two-sided row.
    pub fn set_duplex_by_layout(&mut self, on: bool) {
        self.duplex_by_layout = on;
    }

    /// The pager under the sheet preview in the `Document` layout.
    pub fn set_preview_pager(&mut self, pager: Option<PanePager>) {
        self.preview_pager = pager;
    }

    /// Whether the printer list searches a long fleet ([`PrintPanel::printer_search`]).
    pub fn set_printer_search(&mut self, on: bool) {
        self.printer_search = on;
    }

    /// The pager press since the last call.
    pub fn take_preview_step(&mut self) -> Option<PagerStep> {
        self.requests.take_preview_step()
    }

    /// The id of the section button pressed since the last call.
    pub fn take_action(&mut self) -> Option<String> {
        self.requests.take_action()
    }

    /// The destination the user picked in the panel, or a `set_json_field("printer", ..)` write, once; `State::poll`'s
    /// replacement of an unlisted name is not reported.
    pub fn take_printer_chosen(&mut self) -> Option<Destination> {
        self.printer_chosen.take()
    }

    /// Withdraws a held OK: OK is enabled again, the values are kept and no refusal line shows.
    pub fn release_ok(&mut self) {
        self.held.withdraw();
    }

    /// The dialog holds OK until the app answers it with `answer_ok`.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// Holds OK until the app answers it with `answer_ok`.
    pub fn hold_ok(&mut self) {
        self.hold = true;
    }

    /// The held OK's answer when OK was pressed, once.
    pub fn take_ok(&mut self) -> Option<PrintAnswer> {
        self.held.take()
    }

    /// Answers the held OK: `Ok` closes the dialog, `Err` keeps it open and shows the refusal. Without a held OK it does
    /// nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// The answer of the dialog as it stands.
    fn answer(&self, finish: PrintFinish) -> PrintAnswer {
        PrintAnswer { finish, settings: self.state.settings.clone(), fields: self.fields.clone() }
    }

    /// Whether the dialog's OK can answer now: a job can start, or in the editor the name can be saved.
    fn can_answer(&self) -> bool {
        match self.mode {
            PrintMode::Print => self.state.ready(),
            PrintMode::PresetEditor => usable_name(&self.preset_name, self.check_name.as_ref().and_then(|check| check(&self.preset_name))),
        }
    }

    /// Answers with `finish`: the answer itself, or none while the host holds OK and takes it from `take_ok`.
    fn conclude(&mut self, finish: PrintFinish) -> Option<PrintAnswer> {
        let answer = self.answer(finish);
        if self.hold {
            self.held.file(answer);
            None
        } else {
            Some(answer)
        }
    }

    /// The title of the shown section: the one named, else the first.
    fn shown_section(&self) -> Option<&str> {
        let named = self.sections.iter().map(|section| section.title.as_str()).find(|title| *title == self.section);
        named.or_else(|| self.sections.first().map(|section| section.title.as_str()))
    }
}

impl HoldsOk for HostedPrint {
    type Answer = PrintAnswer;

    fn held_ok(self) -> Self {
        HostedPrint::held_ok(self)
    }

    fn take_ok(&mut self) -> Option<PrintAnswer> {
        HostedPrint::take_ok(self)
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        HostedPrint::answer_ok(self, answer);
    }
}

/// The automation name of a duplex mode.
fn duplex_json(duplex: Option<Duplex>) -> serde_json::Value {
    match duplex {
        None => serde_json::Value::Null,
        Some(Duplex::Off) => "off".into(),
        Some(Duplex::LongEdge) => "long_edge".into(),
        Some(Duplex::ShortEdge) => "short_edge".into(),
    }
}

/// The duplex mode an automation value names; `None` when it names none.
fn duplex_from_json(value: &serde_json::Value) -> Option<Option<Duplex>> {
    match value {
        serde_json::Value::Null => Some(None),
        serde_json::Value::String(name) => match name.as_str() {
            "off" => Some(Some(Duplex::Off)),
            "long_edge" => Some(Some(Duplex::LongEdge)),
            "short_edge" => Some(Some(Duplex::ShortEdge)),
            _ => None,
        },
        _ => None,
    }
}

impl StackDialog for HostedPrint {
    type Result = PrintAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    /// Polls the state, shows the dialog over the sheet the app set (the message without one), and keeps the drags, the
    /// Refresh, pager, Setup and button presses, the preset events, the destination picked and the sections' requests
    /// for the app.
    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PrintAnswer>> {
        self.state.poll(ui.ctx());
        let answers = if craft_ui::runtime::measure::active(ui.ctx()) { Vec::new() } else { self.requests.take_answers() };
        let finish_held = self.held.is_accepted();
        // Print waits for the app's answer: a second press cannot file over the held one.
        let ready = self.state.ready() && self.settings_valid && !self.held.is_waiting();
        let (busy, discovering) = (self.state.busy(), self.state.discovering());
        let refusal = self.held.refusal().map(|(reason, refused)| (reason.clone(), refused.settings.clone(), refused.fields.clone()));
        let mut dialog = PrintDialog::new(Bind::bound(&mut self.state.settings), &self.state.printers, Bind::bound(&mut self.fields))
            .sections(&self.sections)
            .busy(busy)
            .discovering(discovering)
            .ready(ready)
            .answers(answers)
            .layout(self.layout)
            .mode(self.mode)
            .header(&self.header)
            .driver(self.driver)
            .duplex_by_layout(self.duplex_by_layout)
            .printer_search(self.printer_search)
            .hold_ok(self.hold)
            .finish_held(finish_held)
            .sheet_index(Bind::bound(&mut self.sheet_index))
            .section(Bind::bound(&mut self.section))
            .preset_name(Bind::bound(&mut self.preset_name));
        if let Some(sheet) = &self.sheet {
            dialog = dialog.sheet(sheet);
        }
        if let Some(message) = &self.message {
            dialog = dialog.message(message.clone());
        }
        if let Some(pager) = self.preview_pager.clone() {
            dialog = dialog.preview_pager(pager);
        }
        if let Some(refusal) = refusal {
            dialog = dialog.refusal(refusal);
        }
        if let Some(held) = &self.presets {
            dialog = dialog.presets(PrintPresets::new(&held.rows, held.held.clone(), &held.built_in));
        }
        if let Some(check) = &self.check_name {
            dialog = dialog.check_name(move |name| check(name));
        }
        let shown = dialog.show(ui);
        let PrintDialogOutput { closed, finish, moved, refresh, settings_clicked, paper_px, requests, presets, setup, printer_chosen, held, .. } =
            shown;
        self.moves.extend(moved);
        if paper_px.is_some() {
            self.paper_px = paper_px;
        }
        self.refresh |= refresh;
        self.setup |= setup;
        self.preset_events.extend(presets);
        self.settings_clicked |= settings_clicked;
        if printer_chosen.is_some() {
            self.printer_chosen = printer_chosen;
        }
        if self.held.refusal().is_some_and(|(_, refused)| refused.settings != self.state.settings || refused.fields != self.fields) {
            self.held.clear_refusal();
        }
        if held {
            self.held.file(self.answer(PrintFinish::Print));
        }
        self.requests.extend(requests);
        match (finish, closed) {
            (Some(finish), _) => Some(DialogResult::Ok(self.answer(finish))),
            (None, Some(CloseReason::Activated)) if finish_held => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// Answers Done after `choice: "done"`, else Print as the button does: none while a job cannot start (in the preset
    /// editor, while the name cannot be saved). A held dialog files the answer for the app, as its button does, and
    /// answers none; once the app accepted the held answer, it answers that.
    fn confirm(&mut self, _: &egui::Context) -> Option<PrintAnswer> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() {
            return None;
        }
        let finish = if std::mem::take(&mut self.done_chosen) {
            PrintFinish::Done
        } else if self.settings_valid && self.can_answer() {
            PrintFinish::Print
        } else {
            return None;
        };
        self.conclude(finish)
    }

    /// The `Settings` keys (`printer`, `copies`, `duplex`, `grayscale`, `collate`), each section parameter's value as
    /// [`DialogFields`] serializes it, and the dialog's own: `section` (the shown section), `sheet`, `sheets`, `notes`,
    /// `preset` (the held preset) and, in the preset editor, `name`. A `Settings` key an app's row shows in the
    /// sections layout lists the row's value.
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut map = match serde_json::to_value(&self.fields).ok()? {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        let settings = &self.state.settings;
        let printer = match &settings.destination {
            Destination::Printer(name) => name.clone().into(),
            Destination::File => serde_json::Value::Null,
        };
        map.insert("printer".to_owned(), printer);
        for (key, value) in [
            ("copies", settings.copies.into()),
            ("duplex", duplex_json(settings.duplex)),
            ("grayscale", settings.grayscale.into()),
            ("collate", settings.collate.into()),
        ] {
            if row_of_setting(self.layout, &self.sections, &self.header, key).is_none() {
                map.insert(key.to_owned(), value);
            }
        }
        map.insert("section".to_owned(), self.shown_section().map_or(serde_json::Value::Null, Into::into));
        if let Some(pager) = &self.preview_pager {
            map.insert("sheet".to_owned(), pager.shown.into());
            map.insert("sheets".to_owned(), pager.count.into());
        } else {
            map.insert("sheet".to_owned(), self.sheet_index.into());
        }
        let held = self.presets.as_ref().and_then(|presets| presets.held.clone());
        map.insert("preset".to_owned(), held.map_or(serde_json::Value::Null, Into::into));
        if self.mode == PrintMode::PresetEditor {
            map.insert("name".to_owned(), self.preset_name.clone().into());
        }
        let notes = self.sections.iter().flat_map(|section| &section.params);
        let notes = notes.filter(|param| matches!(param.kind, ParamKind::ReadOnlyText { .. }) && param.visible(&self.fields));
        map.insert("notes".to_owned(), notes.map(|param| serde_json::Value::from(param.key.clone())).collect());
        if let Some(form) = &self.json_form {
            form(&self.fields, &mut map);
        }
        Some(serde_json::Value::Object(map))
    }

    /// Sets a `Settings` key, the dialog's own (`section`, `sheet`, `name`, `preset`, and `choice` = `"done"`), or a
    /// section parameter through the shared parameter-field rule. A `Settings` key an app's row shows in the sections
    /// layout sets the row. `printer` takes a listed printer's name, or `null` for Save as PDF where the layout offers
    /// it; the write is a choice the app takes with [`take_printer_chosen`](HostedPrint::take_printer_chosen). A
    /// section parameter's set is recorded as an edit of its row, as the user's is ([`HostRequests::set_field`]).
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        if let Some(params) = row_of_setting(self.layout, &self.sections, &self.header, field) {
            return set_param_field(params, &mut self.fields, field, value);
        }
        match field {
            "printer" => {
                let destination = match &value {
                    serde_json::Value::Null if self.layout == PrintLayout::Document => Some(Destination::File),
                    serde_json::Value::String(name) if self.state.printers.iter().any(|printer| printer.name == *name) => {
                        Some(Destination::Printer(name.clone()))
                    }
                    _ => None,
                };
                let destination = destination.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.state.settings.destination = destination.clone();
                self.printer_chosen = Some(destination);
            }
            "copies" => {
                let copies = value.as_u64().and_then(|copies| u32::try_from(copies).ok()).filter(|copies| (1..=MOST_COPIES).contains(copies));
                self.state.settings.copies = copies.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            }
            "duplex" => self.state.settings.duplex = duplex_from_json(&value).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?,
            "grayscale" => self.state.settings.grayscale = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?,
            "collate" => self.state.settings.collate = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?,
            "choice" => {
                if self.mode != PrintMode::Print {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                if value.as_str() != Some("done") {
                    return Err(FieldRefusal::Rejects(&value).message(field));
                }
                self.done_chosen = true;
            }
            "section" => {
                if self.layout != PrintLayout::Sections {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                let title = value.as_str().filter(|title| self.sections.iter().any(|section| section.title.as_str() == *title));
                self.section = title.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?.to_owned();
            }
            "sheet" => {
                let count = self.sheet.as_ref().map(|sheet| sheet.count).filter(|count| *count > 0);
                let count = count.ok_or_else(|| FieldRefusal::Inactive.message(field))?;
                let index = value.as_u64().and_then(|index| usize::try_from(index).ok());
                self.sheet_index = index.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?.min(count - 1);
            }
            "name" => {
                if self.mode != PrintMode::PresetEditor {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                self.preset_name = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?.to_owned();
            }
            "preset" => {
                // A row's key, else its name in any case ("proofs" applies the saved "Proofs").
                let rows =
                    self.presets.as_ref().map(|presets| presets.rows.iter().filter(|row| !row.is_group()).collect::<Vec<_>>()).unwrap_or_default();
                let key = value.as_str().and_then(|name| {
                    rows.iter()
                        .find(|row| row.key == name)
                        .or_else(|| rows.iter().find(|row| row.label.eq_ignore_ascii_case(name)))
                        .map(|row| row.key.clone())
                });
                let key = key.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.preset_events.push(NamedPresetEvent::Apply(key));
            }
            _ => {
                let section =
                    self.sections.iter().find(|section| section.params.iter().any(|param| param.key == field || param.unit_key() == Some(field)));
                let section = section.ok_or_else(|| FieldRefusal::Unknown.message(field))?;
                return self.requests.set_field(&section.params, &mut self.fields, field, value);
            }
        }
        Ok(())
    }
}
