//! `PrintDialog`: Photoshop's Print Settings (the preview pane, the printer panel and the app's
//! sections), and `HostedPrint`, the same dialog owned by a `DialogHost`.
#![allow(clippy::todo)] // P3(dialogs-w16): dl.K44 bodies.

use std::sync::mpsc::Receiver;

use craft_behavior::{EditEvent, EditEvents};
use craft_ui::contract::{Bind, NoValue, Outcome};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::patterns::form_dialogs::{DialogFields, HostRequests, Param};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::requests::HostAnswer;
use egui::load::SizedTexture;
use egui::{Response, Ui};

use super::{Printer, Settings, State};

craft_ui::words! {
    /// The words of the print dialog.
    pub struct PrintDialogWords {
        title: "printDialog.title" = "Print Settings";
        print: "printDialog.print" = "Print";
        done: "printDialog.done" = "Done";
        preview: "printDialog.preview" = "Print preview";
    }
}

/// Which marks the preview draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    // P3(dialogs-w16): dl.K44 `PrintSheet::new`
    pub fn new(paper: egui::Vec2, image: egui::Rect, picture: SizedTexture, readout: Label) -> Self {
        let _ = (paper, image, picture, readout);
        todo!("dl.K44")
    }

    /// The printable area of the paper.
    // P3(dialogs-w16): dl.K44 builder
    #[must_use]
    pub fn printable(self, area: egui::Rect) -> Self {
        let _ = (self, area);
        todo!("dl.K44")
    }

    /// The marks to draw.
    // P3(dialogs-w16): dl.K44 builder
    #[must_use]
    pub fn marks(self, marks: PrintMarks) -> Self {
        let _ = (self, marks);
        todo!("dl.K44")
    }
}

/// One titled section of the app's rows (Color Management, Position and Size, Printing Marks,
/// Functions, Save as PDF).
pub struct PrintSection {
    /// The section's title.
    pub title: Label,
    /// Its rows.
    pub params: Vec<Param>,
}

/// How a print dialog finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintFinish {
    /// Print the document.
    Print,
    /// Close, keeping the settings.
    Done,
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
    /// The image box drag as total travel in paper points since the press.
    // P3(dialogs-w16): dl.K44 `EditEvents`
    pub moved: EditEvents<egui::Vec2>,
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
        PrintDialog { settings, printers, fields, sheet, sections: &[], opened: None, busy: false, discovering: false, answers: Vec::new() }
    }

    /// Draws the dialog.
    pub fn show(self, ui: &mut Ui) -> PrintDialogOutput {
        let PrintDialog { settings, printers, fields, sheet, sections, opened, busy, discovering, answers } = self;
        let _ = (ui, settings, printers, fields, sheet, sections, opened, busy, discovering, answers);
        todo!("dl.K44")
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
    moves: Vec<EditEvent<egui::Vec2>>,
    refresh: bool,
}

impl HostedPrint {
    /// The dialog over `state`, the sections' `fields` and the `sections`.
    pub fn new(state: State, fields: DialogFields, sections: Vec<PrintSection>) -> Self {
        HostedPrint { id: dialog_id("print"), state, fields, sections, sheet: None, moves: Vec::new(), refresh: true }
    }

    /// Whether the app should send a printer list: once on the first pass and on each Refresh.
    pub fn take_refresh(&mut self) -> bool {
        todo!("dl.K44")
    }

    /// Takes the printer list the app sends; `false` while a discovery or a job runs.
    pub fn listen(&mut self, printers: Receiver<Vec<Printer>>) -> bool {
        let _ = printers;
        todo!("dl.K44")
    }

    /// This pass's layout, from the app.
    pub fn set_sheet(&mut self, sheet: PrintSheet) {
        self.sheet = Some(sheet);
    }

    /// The image box drags since the last call.
    // P3(dialogs-w16): dl.K44 `EditEvents`
    pub fn take_moves(&mut self) -> EditEvents<egui::Vec2> {
        std::mem::take(&mut self.moves).into_iter().collect()
    }

    /// The sections' values, for the app to read and write.
    pub fn fields_mut(&mut self) -> &mut DialogFields {
        &mut self.fields
    }
}

impl StackDialog for HostedPrint {
    type Result = PrintAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PrintAnswer>> {
        let _ = (ui, &self.state, &self.sections, self.refresh);
        todo!("dl.K44")
    }

    fn confirm(&mut self, ctx: &egui::Context) -> Option<PrintAnswer> {
        let _ = ctx;
        todo!("dl.K44")
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        todo!("dl.K44")
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let _ = (ctx, field, value);
        todo!("dl.K44")
    }
}
