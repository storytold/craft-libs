//! Printer selection and asynchronous job feedback, independent of any spooler or document.
//! Hosts provide workers; applications own PDF generation, commands and translated wording.

use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplex {
    Off,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub printer: String,
    pub copies: u32,
    /// None preserves the printer's default instead of forcing simplex.
    pub duplex: Option<Duplex>,
    pub grayscale: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { printer: String::new(), copies: 1, duplex: None, grayscale: false }
    }
}

pub type Discover = Box<dyn Fn() -> Receiver<Vec<Printer>>>;
pub type Submission = Receiver<Result<String, String>>;
pub type Submit = Box<dyn Fn(Vec<u8>, Settings) -> Submission>;

/// Apps format these states in their own language. Receipts and errors are backend text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Feedback {
    FindingPrinters,
    NoPrinters,
    DiscoveryFailed,
    Submitting,
    Submitted(String),
    Failed(String),
    WorkerStopped,
}

/// Translated labels supplied by the app; identity never depends on their text.
pub struct Labels<'a> {
    pub printer: &'a str,
    pub select_printer: &'a str,
    pub refresh: &'a str,
    pub copies: &'a str,
    pub printer_default: &'a str,
    pub duplex: &'a str,
    pub one_sided: &'a str,
    pub long_edge: &'a str,
    pub short_edge: &'a str,
    pub grayscale: &'a str,
}

#[derive(Default)]
pub struct State {
    pub settings: Settings,
    pub printers: Vec<Printer>,
    pub message: Option<Feedback>,
    discovery: Option<Receiver<Vec<Printer>>>,
    submission: Option<Submission>,
    loaded: bool,
}

impl State {
    pub fn busy(&self) -> bool {
        self.submission.is_some()
    }

    pub fn discovering(&self) -> bool {
        self.discovery.is_some()
    }

    /// A refresh invalidates the old list until discovery completes: a removed queue must
    /// not remain selectable while the host is checking it.
    pub fn ready(&self) -> bool {
        !self.busy() && !self.discovering() && self.printers.iter().any(|p| p.name == self.settings.printer)
    }

    /// Refuse replacing an active job's receiver. The caller still owns the refused receiver;
    /// dropping a receiver cannot cancel a job already handed to a native worker.
    pub fn begin(&mut self, receiver: Submission) -> Result<(), Submission> {
        if self.busy() {
            return Err(receiver);
        }
        self.submission = Some(receiver);
        self.message = Some(Feedback::Submitting);
        Ok(())
    }

    /// False means another discovery or job is active; no worker was started.
    pub fn refresh(&mut self, discover: &Discover) -> bool {
        if self.discovering() || self.busy() {
            return false;
        }
        self.discovery = Some(discover());
        self.loaded = true;
        self.message = Some(Feedback::FindingPrinters);
        true
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.discovery {
            match receiver.try_recv() {
                Ok(printers) => {
                    self.printers = printers;
                    if !self.printers.iter().any(|p| p.name == self.settings.printer) {
                        self.settings.printer =
                            self.printers.iter().find(|p| p.default).or(self.printers.first()).map(|p| p.name.clone()).unwrap_or_default();
                    }
                    self.message = self.printers.is_empty().then_some(Feedback::NoPrinters);
                    self.discovery = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.printers.clear();
                    self.message = Some(Feedback::DiscoveryFailed);
                    self.discovery = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &self.submission {
            match receiver.try_recv() {
                Ok(result) => {
                    self.message = Some(match result {
                        Ok(receipt) => Feedback::Submitted(receipt),
                        Err(error) => Feedback::Failed(error),
                    });
                    self.submission = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.message = Some(Feedback::WorkerStopped);
                    self.submission = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.discovering() || self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Draw just the controls. Apps render feedback, their preview and the submit/export
    /// buttons around this component, using their own command and document state.
    pub fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, discover: &Discover, labels: &Labels<'_>) {
        if !self.loaded {
            self.refresh(discover);
        }
        self.poll(ui.ctx());
        ui.push_id(id, |ui| {
            ui.add_enabled_ui(!self.busy(), |ui| {
                let width = ui.available_width().clamp(32.0, 230.0);
                let printer_label = ui.label(labels.printer);
                egui::ComboBox::from_id_salt("printer")
                    .width(width)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .selected_text(if self.settings.printer.is_empty() { labels.select_printer } else { &self.settings.printer })
                    .show_ui(ui, |ui| {
                        for printer in &self.printers {
                            ui.selectable_value(&mut self.settings.printer, printer.name.clone(), &printer.name);
                        }
                    })
                    .response
                    .labelled_by(printer_label.id);
                if ui.add_enabled(!self.discovering(), egui::Button::new(labels.refresh)).clicked() {
                    self.refresh(discover);
                }
                ui.horizontal(|ui| {
                    let label = ui.label(labels.copies);
                    ui.add(egui::DragValue::new(&mut self.settings.copies).range(1..=999)).labelled_by(label.id);
                });
                let choices = [
                    (None, labels.printer_default),
                    (Some(Duplex::Off), labels.one_sided),
                    (Some(Duplex::LongEdge), labels.long_edge),
                    (Some(Duplex::ShortEdge), labels.short_edge),
                ];
                let duplex_label = ui.label(labels.duplex);
                egui::ComboBox::from_id_salt("duplex")
                    .width(width)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .selected_text(
                        choices.iter().find(|(value, _)| *value == self.settings.duplex).map(|(_, text)| *text).unwrap_or(labels.printer_default),
                    )
                    .show_ui(ui, |ui| {
                        for (value, text) in choices {
                            ui.selectable_value(&mut self.settings.duplex, value, text);
                        }
                    })
                    .response
                    .labelled_by(duplex_label.id);
                ui.checkbox(&mut self.settings.grayscale, labels.grayscale);
            });
        });
    }
}

#[cfg(test)]
mod tests;
