//! Printer selection and asynchronous job feedback, independent of any spooler or document.
//! Hosts provide workers; applications own PDF generation, commands and translated wording.

pub(crate) mod dialog;
pub(crate) mod panel;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use dialog::{
    HostedPrint, PrintAnswer, PrintDialog, PrintDialogOutput, PrintDialogWords, PrintFinish, PrintLayout, PrintMarks, PrintMode, PrintPresets,
    PrintSection, PrintSheet, SheetContent, SheetPage,
};
pub use panel::{Destinations, DriverControls, PrintPanel, PrintPanelLayout, PrintPanelOutput, PrintWords};

use craft_ui::contract::Bind;
use craft_ui::runtime::frame::request_repaint_after;
use egui::{Context, Ui};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

/// How often a running worker asks for the next pass, in seconds.
const POLL_SECONDS: f64 = 0.1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
/// A printer the spooler offers.
pub struct Printer {
    /// The spooler's destination: what [`Settings::destination`] and jobs name.
    pub name: String,
    /// What the dialog shows; empty shows `name`. CUPS, for one, describes the queue
    /// `_192_0_2_15` as `192.0.2.15`, the name the user gave the printer.
    pub label: String,
    /// Whether it is the system's default printer.
    pub default: bool,
    /// The note shown when the printer's own dialog decides copies, collate, two-sided and colour.
    pub own_dialog: Option<String>,
}

impl Printer {
    /// A printer queue named `name`, with no label, not the default.
    pub fn new(name: impl Into<String>) -> Self {
        Printer { name: name.into(), label: String::new(), default: false, own_dialog: None }
    }

    /// The text the dialog shows for the printer.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Whether the printer is the system's default.
    #[must_use]
    pub fn is_default(mut self, default: bool) -> Self {
        self.default = default;
        self
    }

    /// Marks the printer's own dialog as deciding copies, collate, two-sided and colour; the panel disables those
    /// controls and shows `note` under them while the printer is chosen.
    #[must_use]
    pub fn own_dialog(mut self, note: impl Into<String>) -> Self {
        self.own_dialog = Some(note.into());
        self
    }
}

impl From<crate::Printer> for Printer {
    /// The dialog's printer for a discovered one: `name`, the `description` as the label, and `default`.
    fn from(printer: crate::Printer) -> Self {
        Printer { name: printer.name, label: printer.description, default: printer.default, own_dialog: None }
    }
}

/// The text the dialog shows for each of `printers`, in order: its label, or its name when it has
/// none. Text two printers would share is followed by the name in brackets, so neither hides the
/// other; when even that text is shared (another printer's label is `Office (Office_1)`), those
/// printers show their names, which the spooler keeps distinct.
pub fn shown_names(printers: &[Printer]) -> Vec<String> {
    let base: Vec<&str> = printers.iter().map(|p| if p.label.trim().is_empty() { p.name.as_str() } else { p.label.as_str() }).collect();
    let mut base_count: HashMap<&str, usize> = HashMap::with_capacity(base.len());
    for text in &base {
        *base_count.entry(text).or_default() += 1;
    }
    let mut shown: Vec<String> = printers
        .iter()
        .zip(&base)
        .map(|(printer, text)| {
            let shared = base_count.get(text).is_some_and(|n| *n > 1);
            if shared && *text != printer.name { format!("{text} ({})", printer.name) } else { (*text).to_owned() }
        })
        .collect();
    // Which printers show each text. A shared text turns every printer showing it into its name;
    // a printer that shows its name never changes again, so each moves at most once and the
    // whole cascade stays linear in the number of printers.
    let mut by_text: HashMap<String, Vec<usize>> = HashMap::with_capacity(shown.len());
    for (i, text) in shown.iter().enumerate() {
        by_text.entry(text.clone()).or_default().push(i);
    }
    let mut pending: Vec<String> = by_text.iter().filter(|(_, ids)| ids.len() > 1).map(|(text, _)| text.clone()).collect();
    while let Some(text) = pending.pop() {
        let Some(ids) = by_text.get_mut(&text) else { continue };
        if ids.len() < 2 {
            continue;
        }
        let (stay, moving): (Vec<usize>, Vec<usize>) = ids.iter().partition(|&&i| printers.get(i).is_some_and(|p| p.name == text));
        *ids = stay;
        for i in moving {
            let (Some(printer), Some(slot)) = (printers.get(i), shown.get_mut(i)) else { continue };
            slot.clone_from(&printer.name);
            let ids = by_text.entry(printer.name.clone()).or_default();
            ids.push(i);
            if ids.len() == 2 {
                pending.push(printer.name.clone());
            }
        }
    }
    shown
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Two-sided printing mode.
pub enum Duplex {
    /// One-sided printing.
    Off,
    /// Two-sided, flipping on the long edge.
    LongEdge,
    /// Two-sided, flipping on the short edge.
    ShortEdge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Where a print job goes.
pub enum Destination {
    /// The printer with this name.
    Printer(String),
    /// Save as PDF.
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// What the user chose for one print job.
pub struct Settings {
    /// Number of copies.
    pub copies: u32,
    /// None preserves the printer's default instead of forcing simplex.
    pub duplex: Option<Duplex>,
    /// Print in grayscale.
    pub grayscale: bool,
    /// Where the job goes: a printer by its [`Printer::name`], or Save as PDF.
    pub destination: Destination,
    /// Collate the copies.
    pub collate: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { copies: 1, duplex: None, grayscale: false, destination: Destination::Printer(String::new()), collate: true }
    }
}

/// Starts discovery of printers; the returned receiver delivers the list once.
pub type Discover = Box<dyn Fn() -> Receiver<Vec<Printer>>>;
/// The receiver of one print job's outcome: the backend's receipt on success, its error text on failure.
pub type Submission = Receiver<Result<String, String>>;
/// Starts a print job from the document's PDF bytes with the chosen [`Settings`].
pub type Submit = Box<dyn Fn(Vec<u8>, Settings) -> Submission>;

/// Apps format these states in their own language. Receipts and errors are backend text.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Feedback {
    /// Discovery is running.
    FindingPrinters,
    /// Discovery finished and found no printers.
    NoPrinters,
    /// Discovery's worker stopped without a list.
    DiscoveryFailed,
    /// A job was handed to the worker.
    Submitting,
    /// The job succeeded; carries the backend's receipt text.
    Submitted(String),
    /// The job failed; carries the backend's error text.
    Failed(String),
    /// The job's worker stopped without an outcome.
    WorkerStopped,
}

#[derive(Default)]
#[non_exhaustive]
/// The state of the printer controls: settings, the printer list, the feedback and the running workers.
pub struct State {
    /// The current choices.
    pub settings: Settings,
    /// The printers found by the last discovery; empty while none is known.
    pub printers: Vec<Printer>,
    /// The latest feedback for the app to show, if any.
    pub message: Option<Feedback>,
    discovery: Option<Receiver<Vec<Printer>>>,
    submission: Option<Submission>,
    loaded: bool,
}

impl State {
    /// Whether a print job is running.
    pub fn busy(&self) -> bool {
        self.submission.is_some()
    }

    /// Whether printer discovery is running.
    pub fn discovering(&self) -> bool {
        self.discovery.is_some()
    }

    /// A refresh invalidates the old list until discovery completes: a removed queue must
    /// not remain selectable while the host is checking it. Save as PDF needs no queue, so it
    /// waits only for a running job.
    pub fn ready(&self) -> bool {
        match &self.settings.destination {
            Destination::File => !self.busy(),
            Destination::Printer(name) => !self.busy() && !self.discovering() && self.listed(name),
        }
    }

    /// Whether the last discovery found a printer named `name`.
    fn listed(&self, name: &str) -> bool {
        self.printers.iter().any(|p| p.name == name)
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
        self.listen(discover())
    }

    /// Takes the receiver of a printer discovery the caller started. False means another discovery
    /// or job is active; the receiver is dropped.
    pub fn listen(&mut self, printers: Receiver<Vec<Printer>>) -> bool {
        if self.discovering() || self.busy() {
            return false;
        }
        self.discovery = Some(printers);
        self.loaded = true;
        self.message = Some(Feedback::FindingPrinters);
        true
    }

    /// Reads finished workers' results into [`State::printers`] and [`State::message`], and asks for the next pass in a short, fixed interval while a worker runs. Call it once per frame.
    pub fn poll(&mut self, ctx: &Context) {
        if let Some(receiver) = &self.discovery {
            match receiver.try_recv() {
                Ok(printers) => {
                    self.printers = printers;
                    // Save as PDF stays whatever the spooler lists; only a printer that is gone is replaced.
                    let gone = matches!(&self.settings.destination, Destination::Printer(name) if !self.listed(name));
                    if gone {
                        let fallback = self.printers.iter().find(|p| p.default).or(self.printers.first());
                        self.settings.destination = Destination::Printer(fallback.map(|p| p.name.clone()).unwrap_or_default());
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
            request_repaint_after(ctx, POLL_SECONDS);
        }
    }

    /// Polls the workers and shows the printer controls over [`State::settings`], starting discovery
    /// on the first pass. Apps render feedback, their preview and the submit/export buttons around
    /// the panel, using their own command and document state.
    pub fn run(&mut self, ui: &mut Ui, discover: &Discover) -> PrintPanelOutput {
        if !self.loaded {
            self.refresh(discover);
        }
        self.poll(ui.ctx());
        let output = PrintPanel::new(Bind::bound(&mut self.settings), &self.printers)
            .discovering(self.discovery.is_some())
            .busy(self.submission.is_some())
            .show(ui);
        if output.refresh {
            self.refresh(discover);
        }
        output
    }
}
