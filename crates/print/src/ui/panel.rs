//! `PrintPanel`: the destination, copies, collate, grayscale and two-sided choices of one print job.

use super::{Destination, Duplex, Printer, Settings, shown_names};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, Entry, FieldLayout, Held, LabelScheme, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

/// The most copies one job may ask for.
pub(super) const MOST_COPIES: u32 = 999;

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
        print_in_grayscale: "printPanel.printInGrayscale" = "Print in grayscale";
        print_settings: "printPanel.printSettings" = "Print Settings…";
        orientation: "printPanel.orientation" = "Orientation";
        portrait: "printPanel.portrait" = "Portrait";
        landscape: "printPanel.landscape" = "Landscape";
        page_in_driver: "printPanel.pageInDriver" = "set in Print Settings…";
        setup: "printPanel.setup" = "Setup…";
        collate: "printPanel.collate" = "Collate";
        properties: "printPanel.properties" = "Properties…";
        save_as_pdf: "printPanel.saveAsPdf" = "Save as PDF";
        default_mark: "printPanel.defaultMark" = "(default)";
        finding: "printPanel.finding" = "Finding printers…";
        search_printers: "printPanel.searchPrinters" = "Search printers";
        no_printers_match: "printPanel.noPrintersMatch" = "No printers match.";
    }
}

/// Which printer-driver controls the panel offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DriverControls {
    /// None.
    #[default]
    None,
    /// Properties… after Refresh; its press is `settings_clicked`.
    Properties,
    /// Print Settings…, Portrait and Landscape and the page line.
    Page,
}

/// Which destinations the printer list offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Destinations {
    /// The printers only.
    #[default]
    Printers,
    /// The printers, then Save as PDF at the end of the list.
    PrintersAndFile,
}

/// What the print panel reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PrintPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The Refresh button was pressed this pass: the caller starts printer discovery again.
    pub refresh: bool,
    /// A destination was chosen this pass: the printer, or Save as PDF. The caller takes the paper from the new
    /// printer's driver.
    pub printer_chosen: Option<Destination>,
    /// Print Settings… or Properties… was pressed this pass: the caller opens the driver's own dialog.
    pub settings_clicked: bool,
    /// Portrait (`false`) or Landscape (`true`) was pressed this pass: the caller turns the driver settings.
    pub turned: Option<bool>,
}

impl Outcome for PrintPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Which controls the print panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrintPanelLayout {
    /// Printer, copies, two-sided mode and grayscale.
    #[default]
    Full,
    /// The printer choice and its Refresh and Print Settings… buttons only.
    PrinterOnly,
}

/// The destination choice with a Refresh button, the number of copies, collation, the grayscale option and the
/// two-sided mode of one print job, over the job's [`Settings`].
///
/// - The list shows each printer's label, or its name, the default marked, and edits [`Settings::destination`]; with
///   `Destinations::PrintersAndFile`, Save as PDF follows the printers.
/// - Refresh is disabled while printers are being found, and the whole panel while a job runs.
/// - Copies, Collate and Grayscale share one row. Copies step with the arrow keys, by ten with Shift.
/// - While the chosen printer has its own dialog ([`Printer::own_dialog`]), Copies, Collate, Grayscale and Two-sided
///   are disabled and its note shows under them.
/// - `DriverControls::Properties` puts Properties… after Refresh; `DriverControls::Page` shows Print Settings…,
///   Portrait/Landscape and the page line with `page` and `page_error`, which the caller keeps. Their presses go into
///   the output.
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
    /// Which printer-driver controls are shown: Properties…, or Print Settings… with Portrait and Landscape and the page
    /// line, for a caller that can ask a printer driver for its page.
    #[prop]
    driver: DriverControls,
    /// Which destinations the list offers.
    #[prop]
    destinations: Destinations,
    /// The layout sets two-sided printing: the Two-sided row is left out.
    #[prop]
    duplex_by_layout: bool,
    /// A document dialog's compact rows, as production's: each caption stands at the row start with its control right
    /// after it, Refresh is left out (the printers are found as the dialog opens) and Grayscale reads "Print in
    /// grayscale".
    #[prop]
    inline: bool,
    /// The driver's paper in points, as the caller last took it.
    #[prop(quantity = "paper size the printer driver reports, in points")]
    page: Option<egui::Vec2>,
    /// Why the driver gave no page.
    #[prop]
    page_error: Option<String>,
    /// Which controls the panel shows.
    #[prop]
    layout: PrintPanelLayout,
    /// The printer list is for a long fleet: a "Search printers" field heads it, focused as it opens, and narrows the
    /// printers by their names (case and accents ignored) while Save as PDF stays at the end; typing changes no choice,
    /// only a pick does, and the open list is wide enough for long names (PRINTER_LIST_WIDTH, within the screen).
    #[prop]
    printer_search: bool,
}

/// How wide a searchable printer list opens, in points: room for a long network queue's name.
pub const PRINTER_LIST_WIDTH: f32 = 430.0;

impl<'a> PrintPanel<'a> {
    /// The panel over `settings`, offering `printers`.
    pub fn new(settings: Bind<'a, Settings>, printers: &'a [Printer]) -> Self {
        PrintPanel {
            settings,
            printers,
            discovering: false,
            busy: false,
            driver: DriverControls::None,
            destinations: Destinations::Printers,
            duplex_by_layout: false,
            inline: false,
            page: None,
            page_error: None,
            layout: PrintPanelLayout::default(),
            printer_search: false,
        }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> PrintPanelOutput {
        let PrintPanel {
            settings,
            printers,
            discovering,
            busy,
            driver,
            destinations,
            duplex_by_layout,
            inline,
            page,
            page_error,
            layout,
            printer_search,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<PrintWords>();
        let mut queues: Vec<Entry<Destination>> = printers
            .iter()
            .zip(shown_names(printers))
            .map(|(printer, shown)| {
                let shown = if printer.default { format!("{shown} {}", words.default_mark.as_str()) } else { shown };
                Entry::new(Destination::Printer(printer.name.clone()), shown)
            })
            .collect();
        if destinations == Destinations::PrintersAndFile {
            queues.push(Entry::separator());
            queues.push(Entry::new(Destination::File, words.save_as_pdf.clone()));
        }
        let placeholder = if discovering && printers.is_empty() { &words.finding } else { &words.select_printer };
        let duplexes: [Entry<Option<Duplex>>; 4] = [
            Entry::new(None, words.printer_default.clone()),
            Entry::new(Some(Duplex::Off), words.one_sided.clone()),
            Entry::new(Some(Duplex::LongEdge), words.long_edge.clone()),
            Entry::new(Some(Duplex::ShortEdge), words.short_edge.clone()),
        ];
        let mut held = Held::new(settings);
        let mut refresh = false;
        let mut printer_chosen = None;
        let mut settings_clicked = false;
        let mut turned = None;
        let group = FieldGroup::new(words.title.clone()).enabled(!busy);
        let group = if inline { group.labels(LabelScheme::Inline) } else { group };
        let mut shown = group.show(ui, |ui, cx| {
            let before = held.read(cx)?;
            let mut now = before.clone();
            Field::new(words.printer.clone()).id_salt("printer").show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    let select = Select::new(words.printer.clone(), Bind::given(now.destination.clone()), &queues)
                        .placeholder(placeholder.clone())
                        .id_salt("queue");
                    let select = if printer_search {
                        select
                            .searchable()
                            .search_label(words.search_printers.clone())
                            .no_matches(words.no_printers_match.clone())
                            .search_kept(|destination: &Destination| *destination == Destination::File)
                            .list_width(PRINTER_LIST_WIDTH)
                    } else {
                        select
                    };
                    let picked = select.show(ui);
                    if let Some(destination) = picked.value() {
                        if *destination != now.destination {
                            printer_chosen = Some(destination.clone());
                        }
                        now.destination.clone_from(destination);
                    }
                    if !inline {
                        let button = Button::new(words.refresh.clone()).enabled(!discovering).id_salt("refresh").show(ui);
                        refresh |= clicked(&button);
                    }
                    if driver == DriverControls::Properties {
                        let queue = matches!(&now.destination, Destination::Printer(name) if !name.is_empty());
                        let button = Button::new(words.properties.clone()).enabled(queue).id_salt("properties").show(ui);
                        settings_clicked |= clicked(&button);
                    }
                    if layout == PrintPanelLayout::PrinterOnly && driver != DriverControls::None {
                        let button = Button::new(words.setup.clone()).id_salt("setup").show(ui);
                        settings_clicked |= clicked(&button);
                    }
                });
            });
            if layout == PrintPanelLayout::PrinterOnly {
                return (now != before).then_some((before, now));
            }
            // The chosen printer's own dialog decides the job controls: they show the settings, disabled, with its note.
            let own_dialog = match &now.destination {
                Destination::Printer(name) => printers.iter().find(|printer| printer.name == *name).and_then(|printer| printer.own_dialog.clone()),
                Destination::File => None,
            };
            let free = own_dialog.is_none();
            Field::new(words.copies.clone()).id_salt("copies").show(ui, |ui, _| {
                Row::new().show(ui, |ui, cx| {
                    if let Some(copies) = copies_field(&words.copies, now.copies).enabled(free).show(ui).value().copied().flatten() {
                        now.copies = (copies.round() as u32).clamp(1, MOST_COPIES);
                    }
                    // The Field's label names the copies field, so each check box shows its own text through the label step.
                    let after = SideLabel::new().layout(FieldLayout::After);
                    let collate =
                        cx.labelled(ui, "collate", words.collate.clone(), after.clone(), Checkbox::new(Bind::given(now.collate)).enabled(free));
                    if let Some(on) = collate.value().copied() {
                        now.collate = on;
                    }
                    let grayscale = cx.labelled(
                        ui,
                        "grayscale",
                        if inline { words.print_in_grayscale.clone() } else { words.grayscale.clone() },
                        after,
                        Checkbox::new(Bind::given(now.grayscale)).enabled(free),
                    );
                    if let Some(on) = grayscale.value().copied() {
                        now.grayscale = on;
                    }
                });
            });
            if !duplex_by_layout {
                Field::new(words.duplex.clone()).id_salt("duplex").show(ui, |ui, _| {
                    let picked = Select::new(words.duplex.clone(), Bind::given(now.duplex), &duplexes).enabled(free).id_salt("duplex").show(ui);
                    if let Some(duplex) = picked.value().copied() {
                        now.duplex = duplex;
                    }
                });
            }
            if let Some(note) = own_dialog {
                Text::new(note).shade(Shade::Muted).show(ui);
            }
            if driver == DriverControls::Page {
                Field::new(words.print_settings.clone()).id_salt("driver").show(ui, |ui, _| {
                    let button = Button::new(words.print_settings.clone()).id_salt("print-settings").show(ui);
                    settings_clicked |= clicked(&button);
                });
                Field::new(words.orientation.clone()).id_salt("orientation").show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        let wide = page.is_some_and(|paper| paper.x > paper.y);
                        for (landscape, word) in [(false, &words.portrait), (true, &words.landscape)] {
                            let button = Button::new(word.clone()).enabled(wide != landscape).id_salt(word.as_str()).show(ui);
                            if clicked(&button) {
                                turned = Some(landscape);
                            }
                        }
                    });
                    if let Some(paper) = page {
                        let mm = |points: f32| points / 72.0 * 25.4;
                        let line = format!("{:.0} × {:.0} mm · {}", mm(paper.x), mm(paper.y), words.page_in_driver.as_str());
                        Text::new(line).show(ui);
                    }
                    if let Some(error) = &page_error {
                        Text::new(error.clone()).show(ui);
                    }
                });
            }
            (now != before).then_some((before, now))
        });
        // Once per pass, after the real run: the edit the body decided is written.
        shown.settle(|cx, edit| {
            if let Some((before, now)) = edit.take() {
                held.write(cx, &before, now, words.title.as_str());
            }
        });
        PrintPanelOutput { response: shown.output, refresh, printer_chosen, settings_clicked, turned }
    }
}

/// The whole-number copies field.
fn copies_field(name: &Label, copies: u32) -> NumberField<'static> {
    NumberField::new(name.clone(), Bind::given(Some(f64::from(copies)))).step(1.0).range(1.0..=f64::from(MOST_COPIES)).commit_decimals(0)
}
