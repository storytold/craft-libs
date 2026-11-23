//! `PrintDialog`: Photoshop's Print Settings (the preview pane, the printer panel and the app's
//! sections), and `HostedPrint`, the same dialog owned by a `DialogHost`.

use std::sync::mpsc::Receiver;

use craft_behavior::{CursorIcon, EditEvent, EditEvents, Metrics, Mixed, PartKey, PointerButton, PressPhase, Sense};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Role, Show as _, ShowContainer as _, WordSetCx as _,
};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, FormDialog, HostRequests, Param, ParamRows, set_color_field, set_param_field};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::requests::HostAnswer;
use craft_ui::sections::{DialogRole, FooterButton, Section};
use craft_ui::style::{ComponentStyle, Corners, RuleKind, StyleCx, SurfaceKind};
use egui::load::SizedTexture;
use egui::{Pos2, Rect, Response, Ui, Vec2, pos2, vec2};

use super::panel::{MOST_COPIES, PrintPanel};
use super::{Duplex, Printer, Settings, State};

craft_ui::words! {
    /// The words of the print dialog.
    pub struct PrintDialogWords {
        title: "printDialog.title" = "Print Settings";
        print: "printDialog.print" = "Print";
        done: "printDialog.done" = "Done";
        preview: "printDialog.preview" = "Print preview image";
        printer_setup: "printDialog.printerSetup" = "Printer Setup";
        image_box: "printDialog.imageBox" = "Image box";
    }
}

/// The index `FormOutput::picked` gives the Done button: Print is 0, Cancel 1, and the extra buttons start at 2.
const DONE_BUTTON: usize = 2;

/// The image box of the preview.
const IMAGE: PartKey = PartKey::new("image");

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

/// What the app's layout says for this pass, in points.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PrintSheet {
    /// The paper's size.
    pub paper: egui::Vec2,
    /// The printable area of the paper, when the printer says.
    pub printable: Option<egui::Rect>,
    /// The image box on the paper.
    pub image: egui::Rect,
    /// The picture of the document.
    pub picture: SizedTexture,
    /// The marks to draw.
    pub marks: PrintMarks,
    /// The readout under the preview.
    pub readout: Label,
}

impl PrintSheet {
    /// A sheet of `paper` with the image box `image` holding `picture`, `readout` under the preview, no printable area
    /// and no marks.
    pub fn new(paper: egui::Vec2, image: egui::Rect, picture: SizedTexture, readout: Label) -> Self {
        PrintSheet { paper, printable: None, image, picture, marks: PrintMarks::default(), readout }
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
}

/// One titled section of the app's rows (Color Management, Position and Size, Printing Marks,
/// Functions, Save as PDF).
#[non_exhaustive]
pub struct PrintSection {
    /// The section's title.
    pub title: Label,
    /// Its rows.
    pub params: Vec<Param>,
}

impl PrintSection {
    /// The section titled `title` with its `params` rows.
    pub fn new(title: impl Into<Label>, params: Vec<Param>) -> Self {
        Self { title: title.into(), params }
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

/// Where the paper stands in the preview's room: the paper's top-left corner and the screen points per paper point.
#[derive(Clone, Copy)]
struct Placement {
    origin: Pos2,
    scale: f32,
}

impl Placement {
    /// The screen rect of `paper`, a rect in paper points.
    fn rect(self, paper: Rect) -> Rect {
        Rect::from_min_size(self.origin + paper.min.to_vec2() * self.scale, paper.size() * self.scale)
    }
}

/// The style of the preview: the room it asks for.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(super) struct PrintPreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
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
}

impl<'a> PrintPreview<'a> {
    pub(super) fn new(label: impl Into<Label>, sheet: &'a PrintSheet, position: Bind<'a, Pos2>) -> Self {
        PrintPreview { common: Common::new(), sheet, position }.label(label)
    }

    /// The paper fitted in `room`, centred; none while the paper has no area.
    fn place(&self, room: Rect) -> Option<Placement> {
        let paper = self.sheet.paper;
        if !(paper.x > 0.0 && paper.y > 0.0) {
            return None;
        }
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

    fn style(_: &StyleCx<'_>, _: PrintPreviewStyleKey) -> PrintPreviewStyle {
        PrintPreviewStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = cx.tokens.size.plot.width;
        let paper = self.sheet.paper;
        let aspect = if paper.x > 0.0 && paper.y > 0.0 { paper.y / paper.x } else { 1.0 };
        Vec2::new(width, width * aspect)
    }

    fn bind(&mut self) -> Option<BindMut<'_, Pos2>> {
        Some(self.position.reborrow())
    }

    /// The image box stays where the sheet puts it while it is dragged: the picture follows the pointer, the part does
    /// not, so the press keeps its hit.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(place) = self.place(rect) else { return };
        let part = cx.part(IMAGE, place.rect(self.sheet.image)).sense(Sense::DRAG).pointer_only("the Position and Size rows place the image");
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

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PrintDialogWords>();
        let node = access.node(IMAGE, Role::BUTTON);
        node.name = Name::Text(words.image_box.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(place) = self.place(cx.rect) else { return };
        let sheet = self.sheet;
        let position = cx.value().copied().unwrap_or(sheet.image.min);
        let painter = ui.content_painter(cx);
        let rule = |kind: RuleKind, from: Pos2, to: Pos2| painter.with_draw(|draw| draw.rule(kind, from, to));
        let outline = |kind: RuleKind, rect: Rect| {
            rule(kind, rect.left_top(), rect.right_top());
            rule(kind, rect.right_top(), rect.right_bottom());
            rule(kind, rect.right_bottom(), rect.left_bottom());
            rule(kind, rect.left_bottom(), rect.left_top());
        };
        let paper = place.rect(Rect::from_min_size(Pos2::ZERO, sheet.paper));
        painter.surface(cx.rect, SurfaceKind::Well, Corners::None);
        painter.surface(paper, SurfaceKind::Canvas, Corners::None);
        let image = place.rect(Rect::from_min_size(position, sheet.image.size()));
        let visible = image.intersect(paper);
        if image.width() > 0.0 && image.height() > 0.0 && visible.width() > 0.0 && visible.height() > 0.0 {
            let uv = Rect::from_min_max(
                pos2((visible.min.x - image.min.x) / image.width(), (visible.min.y - image.min.y) / image.height()),
                pos2((visible.max.x - image.min.x) / image.width(), (visible.max.y - image.min.y) / image.height()),
            );
            painter.picture(sheet.picture.id, visible, uv, 0.0);
        }
        if let Some(area) = sheet.printable {
            outline(RuleKind::Grid, place.rect(area));
        }
        outline(RuleKind::Edge, paper);
        outline(RuleKind::DropMark, image);

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
        PrintPreviewOutput { edit: cx.edit_response() }
    }
}

/// What the preview reports.
#[derive(Debug)]
pub(super) struct PrintPreviewOutput {
    /// The image box drag: `Begin` at the first move past the click slop, `Update` with the image box's position on the
    /// paper in paper points, `Commit` at the release.
    edit: EditResponse<Pos2>,
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

/// Photoshop's Print Settings: the preview at the start edge (paper, printable area, image box,
/// marks, readout), the printer panel and the sections at the end edge, and Print, Cancel and Done
/// in the footer.
#[must_use = "show the dialog with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct PrintDialog<'a> {
    settings: Bind<'a, Settings>,
    printers: &'a [Printer],
    fields: Bind<'a, DialogFields>,
    sheet: &'a PrintSheet,
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
    /// The dialog's requests of the host.
    pub requests: HostRequests,
}

impl Outcome for PrintDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> PrintDialog<'a> {
    /// The dialog over `settings`, the `printers` found, the sections' `fields` and this pass's
    /// `sheet`.
    pub fn new(settings: Bind<'a, Settings>, printers: &'a [Printer], fields: Bind<'a, DialogFields>, sheet: &'a PrintSheet) -> Self {
        PrintDialog {
            settings,
            printers,
            fields,
            sheet,
            sections: &[],
            opened: None,
            busy: false,
            discovering: false,
            ready: false,
            answers: Vec::new(),
        }
    }

    /// Draws the dialog. The body reports the drag events and the Refresh press of its last run only, so a measuring run
    /// adds none; a second pass of a frame has no new input and reports none.
    pub fn show(self, ui: &mut Ui) -> PrintDialogOutput {
        let PrintDialog { mut settings, printers, fields, sheet, sections, opened, busy, discovering, ready, answers } = self;
        let env = Env::of(ui);
        let words = env.words::<PrintDialogWords>();
        let mut form = FormDialog::new(words.title.clone(), fields)
            .ok_label(words.print.clone())
            .ok_enabled(ready && !busy)
            .extra_buttons([FooterButton::new(words.done.clone(), DialogRole::Other)])
            .answers(answers);
        if let Some(opened) = opened {
            form = form.opened(opened);
        }
        let mut moved = EditEvents::new();
        let mut refresh = false;
        let shown = form.show(ui, |ui, form, values| {
            moved = EditEvents::new();
            refresh = false;
            for answer in form.take_answers() {
                match answer {
                    HostAnswer::Color { key, color } | HostAnswer::Eyedropper { key, color } => {
                        for section in sections {
                            set_color_field(&section.params, values, &key, color);
                        }
                    }
                    HostAnswer::Path { key, path } => values.set(&key, FieldValue::Text(path.to_string_lossy().into_owned())),
                    _ => {}
                }
            }
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    let preview = PrintPreview::new(words.preview.clone(), sheet, Bind::given(sheet.image.min)).show(ui);
                    moved.extend(preview.edit.events);
                    Text::new(sheet.readout.clone()).show(ui);
                });
                Stack::new().show(ui, |ui, _| {
                    Section::fixed(words.printer_setup.clone()).id_salt("printer").show(ui, |ui, _| {
                        let panel = PrintPanel::new(settings.reborrowed(), printers).discovering(discovering).busy(busy).show(ui);
                        refresh |= panel.refresh;
                    });
                    for (at, section) in sections.iter().enumerate() {
                        Section::fixed(section.title.clone()).id_salt(("section", at)).show(ui, |ui, _| {
                            let mut rows = ParamRows::new(section.title.clone(), &section.params, Bind::bound(&mut *values)).show(ui);
                            form.requests().absorb(&mut rows);
                        });
                    }
                });
            });
            ready && !busy
        });
        let finish = if shown.accepted.is_some() {
            Some(PrintFinish::Print)
        } else if shown.picked == Some(DONE_BUTTON) {
            Some(PrintFinish::Done)
        } else {
            None
        };
        PrintDialogOutput { response: shown.response, closed: shown.closed, finish, moved, refresh, requests: shown.requests }
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

/// The print dialog owned by a `DialogHost` (not registered). It polls its [`State`] each pass,
/// asks the app for a printer list with [`take_refresh`](Self::take_refresh) and takes the list
/// with [`listen`](Self::listen).
pub struct HostedPrint {
    id: DialogId,
    state: State,
    fields: DialogFields,
    sections: Vec<PrintSection>,
    sheet: Option<PrintSheet>,
    moves: Vec<EditEvent<egui::Pos2>>,
    refresh: bool,
    requests: HostRequests,
    json_form: Option<Box<JsonForm>>,
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
            requests: HostRequests::new(),
            json_form: None,
        }
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

    /// Asks for a printer list again on the next pass, as Refresh does: the list the app sent was not the system's
    /// (its spooler was busy).
    pub fn request_refresh(&mut self) {
        self.refresh = true;
    }

    /// Takes the printer list the app sends; `false` while a discovery or a job runs.
    pub fn listen(&mut self, printers: Receiver<Vec<Printer>>) -> bool {
        self.state.listen(printers)
    }

    /// This pass's layout, from the app.
    pub fn set_sheet(&mut self, sheet: PrintSheet) {
        self.sheet = Some(sheet);
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

    /// The answer of the dialog as it stands.
    fn answer(&self, finish: PrintFinish) -> PrintAnswer {
        PrintAnswer { finish, settings: self.state.settings.clone(), fields: self.fields.clone() }
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

    /// Polls the state, shows the dialog over the sheet the app set, and keeps the drags, the Refresh press and the
    /// sections' requests for the app. A pass before the first `set_sheet` shows nothing.
    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PrintAnswer>> {
        self.state.poll(ui.ctx());
        let sheet = self.sheet.as_ref()?;
        let answers = if craft_ui::runtime::measure::active(ui.ctx()) { Vec::new() } else { self.requests.take_answers() };
        let (busy, discovering, ready) = (self.state.busy(), self.state.discovering(), self.state.ready());
        let shown = PrintDialog::new(Bind::bound(&mut self.state.settings), &self.state.printers, Bind::bound(&mut self.fields), sheet)
            .sections(&self.sections)
            .busy(busy)
            .discovering(discovering)
            .ready(ready)
            .answers(answers)
            .show(ui);
        let PrintDialogOutput { closed, finish, moved, refresh, mut requests, .. } = shown;
        self.moves.extend(moved);
        self.refresh |= refresh;
        if let Some(request) = requests.take_color() {
            self.requests.request_color(request);
        }
        if let Some(request) = requests.take_path() {
            self.requests.request_path(request);
        }
        if let Some(request) = requests.take_eyedropper() {
            self.requests.request_eyedropper(request);
        }
        if let Some(point) = requests.take_preview_click() {
            self.requests.request_preview_click(point);
        }
        match (finish, closed) {
            (Some(finish), _) => Some(DialogResult::Ok(self.answer(finish))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    /// Answers Print as the button does: none while a job cannot start.
    fn confirm(&mut self, _: &egui::Context) -> Option<PrintAnswer> {
        self.state.ready().then(|| self.answer(PrintFinish::Print))
    }

    /// The `Settings` keys (`printer`, `copies`, `duplex`, `grayscale`) and each section parameter's value as
    /// [`DialogFields`] serializes it.
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut map = match serde_json::to_value(&self.fields).ok()? {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        let settings = &self.state.settings;
        map.insert("printer".to_owned(), settings.printer.clone().into());
        map.insert("copies".to_owned(), settings.copies.into());
        map.insert("duplex".to_owned(), duplex_json(settings.duplex));
        map.insert("grayscale".to_owned(), settings.grayscale.into());
        if let Some(form) = &self.json_form {
            form(&self.fields, &mut map);
        }
        Some(serde_json::Value::Object(map))
    }

    /// Sets a `Settings` key, or a section parameter through the shared parameter-field rule.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        match field {
            "printer" => {
                let name = value.as_str().filter(|name| self.state.printers.iter().any(|printer| printer.name == *name));
                let name = name.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.state.settings.printer = name.to_owned();
            }
            "copies" => {
                let copies = value.as_u64().and_then(|copies| u32::try_from(copies).ok()).filter(|copies| (1..=MOST_COPIES).contains(copies));
                self.state.settings.copies = copies.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            }
            "duplex" => self.state.settings.duplex = duplex_from_json(&value).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?,
            "grayscale" => self.state.settings.grayscale = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?,
            _ => {
                let section =
                    self.sections.iter().find(|section| section.params.iter().any(|param| param.key == field || param.unit_key() == Some(field)));
                let section = section.ok_or_else(|| FieldRefusal::Unknown.message(field))?;
                return set_param_field(&section.params, &mut self.fields, field, value);
            }
        }
        Ok(())
    }
}
