//! `PrintPanel`: the printer, copies, two-sided and grayscale choices of one print job.

use super::{Duplex, Printer, Settings, shown_names};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Held, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

/// The most copies one job may ask for.
const MOST_COPIES: u32 = 999;

craft_ui::words! {
    /// The words of the print panel.
    pub struct PrintWords {
        title: "printPanel.title" = "Print settings";
        printer: "printPanel.printer" = "Printer";
        select_printer: "printPanel.selectPrinter" = "Select a printer";
        refresh: "printPanel.refresh" = "Refresh";
        copies: "printPanel.copies" = "Copies";
        printer_default: "printPanel.printerDefault" = "Printer default";
        duplex: "printPanel.duplex" = "Two-sided";
        one_sided: "printPanel.oneSided" = "One-sided";
        long_edge: "printPanel.longEdge" = "Long edge";
        short_edge: "printPanel.shortEdge" = "Short edge";
        grayscale: "printPanel.grayscale" = "Grayscale";
    }
}

/// What the print panel reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PrintPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The Refresh button was pressed this pass: the caller starts printer discovery again.
    pub refresh: bool,
}

impl Outcome for PrintPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The printer choice with a Refresh button, the number of copies, the two-sided mode and the grayscale option of one
/// print job, over the job's [`Settings`].
///
/// - The printer list shows each printer's label, or its name, and edits the printer's name.
/// - Refresh is disabled while printers are being found, and the whole panel while a job runs.
/// - Copies step with the arrow keys, by ten with Shift.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct PrintPanel<'a> {
    settings: Bind<'a, Settings>,
    printers: &'a [Printer],
    /// Printers are being found: Refresh is disabled.
    #[prop]
    discovering: bool,
    /// A job is running: the panel is disabled.
    #[prop]
    busy: bool,
}

impl<'a> PrintPanel<'a> {
    /// The panel over `settings`, offering `printers`.
    pub fn new(settings: Bind<'a, Settings>, printers: &'a [Printer]) -> Self {
        PrintPanel { settings, printers, discovering: false, busy: false }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> PrintPanelOutput {
        let PrintPanel { settings, printers, discovering, busy } = self;
        let env = Env::of(ui);
        let words = env.words::<PrintWords>();
        let queues: Vec<Entry<String>> =
            printers.iter().zip(shown_names(printers)).map(|(printer, shown)| Entry::new(printer.name.clone(), shown)).collect();
        let duplexes: [Entry<Option<Duplex>>; 4] = [
            Entry::new(None, words.printer_default.clone()),
            Entry::new(Some(Duplex::Off), words.one_sided.clone()),
            Entry::new(Some(Duplex::LongEdge), words.long_edge.clone()),
            Entry::new(Some(Duplex::ShortEdge), words.short_edge.clone()),
        ];
        let mut held = Held::new(settings);
        let mut refresh = false;
        let mut shown = FieldGroup::new(words.title.clone()).enabled(!busy).show(ui, |ui, cx| {
            let before = held.read(cx)?;
            let mut now = before.clone();
            Field::new(words.printer.clone()).id_salt("printer").show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    let picked = Select::new(words.printer.clone(), Bind::given(now.printer.clone()), &queues)
                        .placeholder(words.select_printer.clone())
                        .id_salt("queue")
                        .show(ui);
                    if let Some(name) = picked.value() {
                        now.printer.clone_from(name);
                    }
                    let button = Button::new(words.refresh.clone()).enabled(!discovering).id_salt("refresh").show(ui);
                    refresh |= clicked(&button);
                });
            });
            Field::new(words.copies.clone()).id_salt("copies").show(ui, |ui, _| {
                if let Some(copies) = copies_field(&words.copies, now.copies).show(ui).value().copied().flatten() {
                    now.copies = (copies.round() as u32).clamp(1, MOST_COPIES);
                }
            });
            Field::new(words.duplex.clone()).id_salt("duplex").show(ui, |ui, _| {
                if let Some(duplex) =
                    Select::new(words.duplex.clone(), Bind::given(now.duplex), &duplexes).id_salt("duplex").show(ui).value().copied()
                {
                    now.duplex = duplex;
                }
            });
            Field::new(words.grayscale.clone()).id_salt("grayscale").show(ui, |ui, _| {
                if let Some(on) =
                    Checkbox::new(Bind::given(now.grayscale)).label(words.grayscale.clone()).id_salt("grayscale").show(ui).value().copied()
                {
                    now.grayscale = on;
                }
            });
            (now != before).then_some((before, now))
        });
        // Once per pass, after the real run: the edit the body decided is written.
        shown.settle(|cx, edit| {
            if let Some((before, now)) = edit.take() {
                held.write(cx, &before, now, words.title.as_str());
            }
        });
        PrintPanelOutput { response: shown.output, refresh }
    }
}

/// The whole-number copies field.
fn copies_field(name: &Label, copies: u32) -> NumberField<'static> {
    NumberField::new(name.clone(), Bind::given(Some(f64::from(copies)))).step(1.0).range(1.0..=f64::from(MOST_COPIES)).commit_decimals(0)
}
