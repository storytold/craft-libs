//! Network-printer destination: the address a person typed, what the printer said it supports, a
//! per-job confirmation and the state of one job. It sits beside the spooler-queue controls in the
//! parent module and shares nothing with them, so existing users are unaffected.
//!
//! The types here are deliberately light and backend-free: the host maps its real client's answers
//! into [`PrinterSummary`], [`Report`] and [`Failure`], supplies [`QueryPrinter`] and [`SendJob`]
//! workers, and words everything through [`Labels`]. No network work happens while drawing.
//!
//! Rules the model keeps:
//! - every option is three-state ([`Capability`]): **supported** (the printer listed it: the control is
//!   enabled and offers only what is listed), **unsupported** (the printer answered and the option is
//!   not part of it: the control is shown disabled with a reason from [`Labels`], and the request never
//!   carries a value for it) or **unknown** (the printer did not say, or the query failed: the control
//!   is enabled with a quiet "not confirmed" note and the value is sent as a hint). Capability is
//!   static; a printer's status (out of paper, cover open) only warns and never disables a control;
//! - the confirmation is never on by default and never remembered: it clears when the address
//!   changes, when the printer is queried again, when a job starts and when the dialog opens;
//! - a job can start only for an address that was queried, with the confirmation ticked.
//!
//! A job has two phases and the model shows both. The host's [`SendJob`] worker sends the document
//! (`JobView::Running`), reports **accepted** (`JobUpdate::Accepted`, `JobView::Accepted`: the printer
//! has the document; nothing says it has printed), may report what the printer says
//! (`JobUpdate::Status`), and finally answers with the result. A printer that cannot report ends with
//! [`Outcome::AcceptedUnknown`]: sent, outcome unknown. Sheet totals are optional: a printer that
//! reports sheets done and no total gets "N sheets" until the app's page count ([`Draft::expected_pages`])
//! supplies an estimated total, when the bar becomes determinate; a total the printer reports wins. All of it arrives over `mpsc` receivers that
//! [`State::poll`] drains once per frame, so nothing blocks the UI thread.

use std::sync::mpsc::{Receiver, TryRecvError};

use crate::capability::{Advertised, Capability, Choices};

/// Duplex setting. [`Sides::token`] is the standard attribute value a printer lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sides {
    OneSided,
    LongEdge,
    ShortEdge,
}

impl Sides {
    pub const ALL: [Sides; 3] = [Sides::OneSided, Sides::LongEdge, Sides::ShortEdge];

    /// The WS-Print keyword for this value (`OneSided`, `TwoSidedLongEdge`, ...).
    pub const fn token(self) -> &'static str {
        match self {
            Self::OneSided => "OneSided",
            Self::LongEdge => "TwoSidedLongEdge",
            Self::ShortEdge => "TwoSidedShortEdge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Orientation {
    Portrait,
    Landscape,
    ReversePortrait,
    ReverseLandscape,
}

impl Orientation {
    pub const ALL: [Orientation; 4] = [Self::Portrait, Self::Landscape, Self::ReversePortrait, Self::ReverseLandscape];

    /// The WS-Print keyword for this value (`Portrait`, `Landscape`, ...).
    pub const fn token(self) -> &'static str {
        match self {
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
            Self::ReversePortrait => "ReversePortrait",
            Self::ReverseLandscape => "ReverseLandscape",
        }
    }
}

/// Print quality, the choices of the dialog. `Photo` is offered only for a printer that lists it
/// ([`Draft::offered_qualities`]); a host maps each to its client's quality setting (`craft-print`
/// has the same four).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    Draft,
    Normal,
    High,
    Photo,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Self::Draft, Self::Normal, Self::High, Self::Photo];

    /// The quality a printer's token names (`"Photo"`; ASCII case does not matter).
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.token().eq_ignore_ascii_case(token.trim()))
    }

    /// The WS-Print keyword for this value (`Draft`, `Normal`, ...).
    pub const fn token(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Normal => "Normal",
            Self::High => "High",
            Self::Photo => "Photo",
        }
    }
}

/// Colour or black and white. [`ColorMode::token`] is the value a printer lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorMode {
    Color,
    Monochrome,
}

impl ColorMode {
    pub const ALL: [ColorMode; 2] = [Self::Color, Self::Monochrome];

    /// The WS-Print keyword for this value (`Color` or `Monochrome`).
    pub const fn token(self) -> &'static str {
        match self {
            Self::Color => "Color",
            Self::Monochrome => "Monochrome",
        }
    }
}

/// A print resolution in dots per inch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Resolution {
    pub x_dpi: u32,
    pub y_dpi: u32,
}

impl Resolution {
    /// A resolution of `x_dpi` across and `y_dpi` along the feed, in dots per inch.
    pub const fn new(x_dpi: u32, y_dpi: u32) -> Self {
        Resolution { x_dpi, y_dpi }
    }

    /// The same resolution in both directions.
    pub const fn square(dpi: u32) -> Self {
        Resolution { x_dpi: dpi, y_dpi: dpi }
    }
}

/// Resolutions offered as hints when a printer listed none ([`Capability::Unknown`]).
pub const COMMON_RESOLUTIONS: [Resolution; 4] = [Resolution::square(150), Resolution::square(300), Resolution::square(600), Resolution::square(1200)];

/// The most copies offered as a hint when a printer did not say ([`Capability::Unknown`]).
pub const UNKNOWN_MAX_COPIES: u32 = 99;

/// What a printer says about each option, as [`Capability`] values.
///
/// The host fills it from its backend's details. Per option: [`Capability::Unknown`] when the
/// printer did not publish it (or asking failed); [`Capability::Supported`] with the list or range it
/// published; [`Capability::Unsupported`] when it published its answer and the option is not in it
/// (an explicit `false`, or a published empty list). The model also treats a published list that
/// offers nothing to choose as unsupported: sides without a two-sided value, colour without
/// [`ColorMode::Color`], copies with a maximum of 1, a list with none of the values the dialog
/// knows. See the README table for the mapping from `craft-print`'s `PrinterDetails`.
///
/// Reshaped from the earlier `Vec<String>` / `Option<(u32, u32)>` fields, where an empty list meant
/// both "not said" and "nothing"; use [`Capability::from_option_list`] and
/// [`Capability::from_option_range`] to convert.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// Media types the printer takes. Empty means it did not say and the printer is left to answer
    /// for itself (see [`PrinterSummary::takes_any`]); not an option of the dialog.
    pub formats: Vec<String>,
    /// Duplex tokens ([`Sides::token`]).
    pub sides: Capability<String>,
    /// Orientation tokens ([`Orientation::token`]).
    pub orientations: Capability<String>,
    /// Quality tokens ([`Quality::token`]); a token that is not one of the four is no choice.
    pub qualities: Capability<String>,
    /// Colour-mode tokens ([`ColorMode::token`]).
    pub color_modes: Capability<String>,
    /// Media-size tokens. There is no control for it: the app names the size it wants in
    /// [`Draft::request`].
    pub media_sizes: Capability<String>,
    pub resolutions: Capability<Resolution>,
    /// Allowed copies, as a range.
    pub copies: Capability<u32>,
}

static NOTHING_KNOWN: Caps = Caps {
    formats: Vec::new(),
    sides: Capability::Unknown,
    orientations: Capability::Unknown,
    qualities: Capability::Unknown,
    color_modes: Capability::Unknown,
    media_sizes: Capability::Unknown,
    resolutions: Capability::Unknown,
    copies: Capability::Unknown,
};

/// The typed view of a token capability: the values the dialog knows, in its order. Unknown stays
/// unknown. A published list with none of the known values, or none for which `varied` holds (a
/// choice among nothing), is unsupported.
fn typed<T: Copy + PartialEq>(cap: &Capability<String>, all: &[T], token: fn(T) -> &'static str, varied: fn(&[T]) -> bool) -> Capability<T> {
    match cap {
        Capability::Unknown => Capability::Unknown,
        Capability::Unsupported => Capability::Unsupported,
        Capability::Supported(Advertised { choices: Choices::List(tokens), default }) => {
            let values: Vec<T> = all.iter().copied().filter(|v| tokens.iter().any(|t| t == token(*v))).collect();
            if values.is_empty() || !varied(&values) {
                return Capability::Unsupported;
            }
            let default = default.as_deref().and_then(|d| values.iter().copied().find(|v| token(*v) == d));
            Capability::Supported(Advertised { choices: Choices::List(values), default })
        }
        // A range of tokens means nothing.
        Capability::Supported(_) => Capability::Unknown,
    }
}

/// What to offer: every known value (as hints) for an unknown option, the listed values for a
/// supported one, nothing for an unsupported one.
fn offered<T: Copy>(cap: &Capability<T>, hints: &[T]) -> Vec<T> {
    match cap {
        Capability::Unknown => hints.to_vec(),
        Capability::Unsupported => Vec::new(),
        other => other.list().to_vec(),
    }
}

/// What a query learned about a printer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrinterSummary {
    pub name: String,
    pub color: Option<bool>,
    pub state: String,
    pub queued_jobs: Option<u32>,
    pub reasons: Vec<String>,
    pub caps: Caps,
}

impl PrinterSummary {
    fn lists(&self, format: &str) -> bool {
        self.caps.formats.iter().any(|f| f.eq_ignore_ascii_case(format))
    }

    /// Does the printer take any of `formats` (media types the host can produce)? A printer that
    /// lists no formats is left to answer for itself.
    pub fn takes_any(&self, formats: &[&str]) -> bool {
        self.caps.formats.is_empty() || formats.iter().any(|f| self.lists(f))
    }
}

/// Which kind of failure happened; the app words the heading, [`Failure::detail`] is the client's
/// own text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The address was refused by the host's network policy.
    Policy,
    /// The printer could not be reached.
    Transport,
    /// The printer's reply could not be understood.
    Protocol,
    /// The printer refused the request.
    Fault,
    /// The printer accepts none of the formats the document can be sent in.
    NoCommonFormat,
    /// The printer does not support a requested setting.
    Unsupported,
    /// The document could not be prepared.
    Encode,
    /// A setting is not valid.
    Invalid,
    /// The worker ended without an answer.
    Stopped,
    Other,
}

impl FailureKind {
    /// A tolerant mapping from a client's error-kind names; unknown names become `Other`.
    pub fn from_name(name: &str) -> FailureKind {
        match name {
            "policy" => Self::Policy,
            "transport" | "connection_closed" | "http" => Self::Transport,
            "xml" | "protocol" | "too_large" => Self::Protocol,
            "fault" => Self::Fault,
            "no_common_format" => Self::NoCommonFormat,
            "unsupported" => Self::Unsupported,
            "encode" => Self::Encode,
            "invalid" => Self::Invalid,
            "stopped" => Self::Stopped,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub detail: String,
    /// The request may have reached the printer (the connection dropped after the document went
    /// out): sending again could print twice. The UI says so; see [`Labels::outcome_unknown_warning`].
    pub outcome_unknown: bool,
}

impl Failure {
    /// A failure of `kind` with the detail shown under its heading; not marked as possibly delivered.
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Failure { kind, detail: detail.into(), outcome_unknown: false }
    }

    /// Mark the failure as one that may have been delivered.
    #[must_use]
    pub fn with_outcome_unknown(mut self, outcome_unknown: bool) -> Self {
        self.outcome_unknown = outcome_unknown;
        self
    }
}

/// How a job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    CompletedWithWarnings,
    CompletedWithErrors,
    Canceled,
    Aborted,
    Forgotten,
    StillRunning,
    /// The printer has the document and cannot say more: sent, outcome unknown. Not a failure and not
    /// a claim that it printed.
    AcceptedUnknown,
}

impl Outcome {
    /// A tolerant mapping from a client's outcome names; unknown names become `Aborted`.
    pub fn from_name(name: &str) -> Outcome {
        match name {
            "completed" => Outcome::Completed,
            "completed_with_warnings" => Outcome::CompletedWithWarnings,
            "completed_with_errors" => Outcome::CompletedWithErrors,
            "canceled" => Outcome::Canceled,
            "forgotten" => Outcome::Forgotten,
            "still_running" => Outcome::StillRunning,
            "accepted_unknown" | "no_status" => Outcome::AcceptedUnknown,
            _ => Outcome::Aborted,
        }
    }

    /// The job printed, with or without warnings.
    pub fn is_success(self) -> bool {
        matches!(self, Outcome::Completed | Outcome::CompletedWithWarnings)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    pub printer: String,
    pub sheets: u64,
    /// The printer's reasons and the client's warnings, as received.
    pub notes: Vec<String>,
}

/// How far a job the printer accepted has got, as far as the printer says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// Accepted; nothing more is known (yet, or ever).
    #[default]
    Accepted,
    /// Queued behind other work.
    Waiting,
    Held,
    Printing,
    /// Paused and needing attention; the reasons are in [`Progress::notes`].
    Stopped,
}

impl Stage {
    /// A tolerant mapping from a client's job-state names (`pending`, `held`, `processing`,
    /// `stopped`); anything else, `unknown` included, is [`Stage::Accepted`].
    pub fn from_name(name: &str) -> Stage {
        match name {
            "pending" => Stage::Waiting,
            "held" => Stage::Held,
            "processing" => Stage::Printing,
            "stopped" => Stage::Stopped,
            _ => Stage::Accepted,
        }
    }
}

/// What the printer says about an accepted job.
///
/// The total is optional because many printers report sheets completed and never a total. While it
/// is unknown the UI shows "N sheets" (an indeterminate figure, no bar); when it is known, from the
/// printer or from the page count the app expects ([`Draft::expected_pages`]), it shows a
/// determinate bar. A total the printer reports always wins over an expected one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    pub sheets_done: Option<u32>,
    pub sheets_total: Option<u32>,
    /// The printer's reasons, as received.
    pub notes: Vec<String>,
}

impl Progress {
    /// `done / total` in `0.0..=1.0` when both counters are known and the total is not zero;
    /// `None` is "indeterminate".
    pub fn fraction(&self) -> Option<f32> {
        match (self.sheets_done, self.sheets_total) {
            (Some(done), Some(total)) if total > 0 => Some(done.min(total) as f32 / total as f32),
            _ => None,
        }
    }

    /// Fill in a total the printer did not report (or reported as zero) with `expected`; a total
    /// that is known stays as it is.
    #[must_use]
    pub fn with_expected_total(mut self, expected: Option<u32>) -> Self {
        if !self.sheets_total.is_some_and(|total| total > 0) {
            self.sheets_total = expected.filter(|total| *total > 0);
        }
        self
    }
}

/// What a [`SendJob`] worker reports before its result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobUpdate {
    /// The printer has accepted the document. Sending is over; nothing says it has printed.
    Accepted(Progress),
    /// A newer status of the accepted job.
    Status(Progress),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum JobView {
    #[default]
    Idle,
    /// The document is being sent; the printer has not accepted it yet.
    Running {
        canceling: bool,
    },
    /// The printer has the document (see [`Stage`]); the worker is still watching it.
    Accepted {
        canceling: bool,
        progress: Progress,
    },
    Finished(Report),
    Failed(Failure),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Query {
    #[default]
    Idle,
    Running,
    Done(Box<PrinterSummary>),
    Failed(Failure),
}

/// What the person asked for; the host turns it into its client's print options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub copies: Option<u32>,
    pub sides: Option<Sides>,
    pub orientation: Option<Orientation>,
    pub quality: Option<Quality>,
    pub color: Option<ColorMode>,
    pub resolution: Option<Resolution>,
    pub media_size: Option<String>,
    /// The page count the app set in [`Draft::expected_pages`], for the host to pass on (a
    /// `craft-print` worker puts it in `JobOptions::expected_pages`). `None` when the app did not say.
    pub expected_pages: Option<u32>,
}

/// Why Print is not available yet (nothing is worded for a missing tick: the box is in plain view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Address,
    NotQueried,
    Format,
    NoPages,
    Unconfirmed,
    Busy,
}

/// `192.0.2.50:5357/service` is read as `http://192.0.2.50:5357/service`; anything with another
/// scheme is left alone for the host's validator to accept or refuse.
pub fn normalize_url(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() || t.contains("://") { t.to_owned() } else { format!("http://{t}") }
}

/// The pure model of the destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// The address as typed.
    pub url: String,
    pub query: Query,
    /// The (normalised) address `query` was made for.
    queried: String,
    /// Bumped by every query so an answer that arrives late is dropped.
    generation: u64,
    pub copies: u32,
    pub sides: Option<Sides>,
    pub orientation: Option<Orientation>,
    pub quality: Option<Quality>,
    pub color: Option<ColorMode>,
    pub resolution: Option<Resolution>,
    /// How many pages the document has, when the app knows. Set it before [`State::start_job`]: it
    /// goes to the host in [`Request::expected_pages`] and, as an estimate of the sheets
    /// ([`Draft::expected_sheets`]), becomes the total of the progress bar when the printer reports
    /// none. The model never changes it.
    pub expected_pages: Option<u32>,
    /// The person's yes to this one print. Never on by default, never kept.
    pub confirmed: bool,
    pub job: JobView,
    /// The sheet total estimated when the current job started (settings cannot change under it).
    job_expected_sheets: Option<u32>,
}

impl Default for Draft {
    fn default() -> Self {
        Draft {
            url: String::new(),
            query: Query::Idle,
            queried: String::new(),
            generation: 0,
            copies: 1,
            sides: None,
            orientation: None,
            quality: None,
            color: None,
            resolution: None,
            expected_pages: None,
            confirmed: false,
            job: JobView::Idle,
            job_expected_sheets: None,
        }
    }
}

impl Draft {
    /// A draft for the printer at `url` (nothing queried yet).
    pub fn for_address(url: &str) -> Self {
        Draft { url: url.to_owned(), ..Default::default() }
    }

    /// The typed address, trimmed and with `http://` added when it has no scheme (see [`normalize_url`]).
    pub fn normalized_url(&self) -> String {
        normalize_url(&self.url)
    }

    /// A job is in flight: being sent, or accepted and still watched.
    pub fn running(&self) -> bool {
        matches!(self.job, JobView::Running { .. } | JobView::Accepted { .. })
    }

    /// A query of the printer is running.
    pub fn querying(&self) -> bool {
        self.query == Query::Running
    }

    /// The printer's answer, when it is for the address now typed.
    pub fn printer(&self) -> Option<&PrinterSummary> {
        match &self.query {
            Query::Done(p) if self.queried == self.normalized_url() => Some(p),
            _ => None,
        }
    }

    /// Call after the address text changed: what was learned about another printer goes, and so
    /// does the confirmation.
    pub fn address_edited(&mut self) {
        self.confirmed = false;
        if self.queried != self.normalized_url() {
            if self.query != Query::Running {
                self.query = Query::Idle;
            }
            self.clear_choices();
        }
    }

    fn clear_choices(&mut self) {
        self.copies = 1;
        self.sides = None;
        self.orientation = None;
        self.quality = None;
        self.color = None;
        self.resolution = None;
    }

    /// The dialog opens: a confirmation from before is void, and so is a finished job's report.
    pub fn dialog_opened(&mut self) {
        self.confirmed = false;
        if !self.running() {
            self.job = JobView::Idle;
        }
    }

    /// What the printer said, or nothing (all unknown) when no printer was queried for this address
    /// or the query failed.
    pub fn caps(&self) -> &Caps {
        self.printer().map_or(&NOTHING_KNOWN, |p| &p.caps)
    }

    /// The duplex option. A list without a two-sided value is [`Capability::Unsupported`]
    /// (nothing to choose). Together with the sides control, the "flip on long/short edge" choices
    /// are unavailable then.
    pub fn sides_state(&self) -> Capability<Sides> {
        typed(&self.caps().sides, &Sides::ALL, Sides::token, |v| v.iter().any(|s| *s != Sides::OneSided))
    }

    /// What the queried printer says about orientation.
    pub fn orientation_state(&self) -> Capability<Orientation> {
        typed(&self.caps().orientations, &Orientation::ALL, Orientation::token, |_| true)
    }

    /// What the queried printer says about print quality.
    pub fn quality_state(&self) -> Capability<Quality> {
        typed(&self.caps().qualities, &Quality::ALL, Quality::token, |_| true)
    }

    /// A list without [`ColorMode::Color`] is a black-and-white printer: [`Capability::Unsupported`].
    pub fn color_state(&self) -> Capability<ColorMode> {
        typed(&self.caps().color_modes, &ColorMode::ALL, ColorMode::token, |v| v.contains(&ColorMode::Color))
    }

    /// The resolutions the queried printer lists, if it lists any.
    pub fn resolution_state(&self) -> Capability<Resolution> {
        self.caps().resolutions.clone()
    }

    /// Copies as a range; a maximum of 1 is [`Capability::Unsupported`] (nothing to choose).
    pub fn copies_state(&self) -> Capability<u32> {
        match &self.caps().copies {
            Capability::Supported(Advertised { choices: Choices::Range { max, .. }, .. }) if *max <= 1 => Capability::Unsupported,
            other => other.clone(),
        }
    }

    /// The media size names the queried printer lists, if it lists any.
    pub fn media_state(&self) -> Capability<String> {
        self.caps().media_sizes.clone()
    }

    /// The choices to offer: the listed ones, all known values (as hints) when the printer did not
    /// say, none when it does not support the option.
    pub fn offered_sides(&self) -> Vec<Sides> {
        offered(&self.sides_state(), &Sides::ALL)
    }

    /// The orientations to offer in the menu (see [`Self::offered_sides`]).
    pub fn offered_orientations(&self) -> Vec<Orientation> {
        offered(&self.orientation_state(), &Orientation::ALL)
    }

    /// The qualities to offer in the menu (see [`Self::offered_sides`]).
    pub fn offered_qualities(&self) -> Vec<Quality> {
        offered(&self.quality_state(), &Quality::ALL)
    }

    /// The color modes to offer in the menu (see [`Self::offered_sides`]).
    pub fn offered_colors(&self) -> Vec<ColorMode> {
        offered(&self.color_state(), &ColorMode::ALL)
    }

    /// The resolutions to offer: the listed ones, else the common ones as hints.
    pub fn offered_resolutions(&self) -> Vec<Resolution> {
        offered(&self.resolution_state(), &COMMON_RESOLUTIONS)
    }

    /// The most copies to offer: what the printer lists, [`UNKNOWN_MAX_COPIES`] when it did not
    /// say, 1 when it takes no more than one.
    pub fn max_copies(&self) -> u32 {
        match self.copies_state() {
            Capability::Unknown => UNKNOWN_MAX_COPIES,
            Capability::Supported(Advertised { choices: Choices::Range { max, .. }, .. }) => max.clamp(1, 9999),
            _ => 1,
        }
    }

    /// The fewest copies the printer takes: the bottom of the range it lists (never above
    /// [`Self::max_copies`]), 1 when it lists none.
    pub fn min_copies(&self) -> u32 {
        match self.copies_state() {
            Capability::Supported(Advertised { choices: Choices::Range { min, .. }, .. }) => min.clamp(1, self.max_copies()),
            _ => 1,
        }
    }

    /// The copies within what the printer takes ([`Self::min_copies`] to [`Self::max_copies`]).
    fn copies_in_range(&self) -> u32 {
        self.copies.clamp(self.min_copies(), self.max_copies())
    }

    /// The sheets the job should use, estimated from [`Self::expected_pages`]: halved (rounded up)
    /// for a two-sided job, times the copies. An estimate: the printer's own counting can differ,
    /// and what it reports wins. `None` without a page count (or with zero pages).
    pub fn expected_sheets(&self) -> Option<u32> {
        let pages = self.expected_pages.filter(|pages| *pages > 0)?;
        let sides = self.sides.filter(|s| self.sides_state().allows(s));
        let per_sheet = if matches!(sides, Some(Sides::LongEdge | Sides::ShortEdge)) { 2 } else { 1 };
        Some(pages.div_ceil(per_sheet).saturating_mul(self.copies_in_range()))
    }

    /// Start a query: returns the generation its answer must carry. Clears the confirmation.
    pub fn begin_query(&mut self) -> u64 {
        self.generation += 1;
        self.confirmed = false;
        self.query = Query::Running;
        self.generation
    }

    /// File a query's answer; one for an older query, or for an address no longer typed, is dropped.
    ///
    /// The latest query's answer for an address no longer typed ends that query too: nothing is
    /// learned for the address now typed, and leaving the query "running" would keep the Query
    /// button disabled and printing blocked until the dialog was closed and opened again.
    pub fn finish_query(&mut self, generation: u64, url: &str, result: Result<PrinterSummary, Failure>) {
        if generation != self.generation {
            return;
        }
        if url != self.normalized_url() {
            self.query = Query::Idle;
            return;
        }
        match result {
            Ok(summary) => {
                self.queried = url.to_owned();
                self.query = Query::Done(Box::new(summary));
                self.keep_only_offered();
            }
            Err(f) => {
                self.queried.clear();
                self.query = Query::Failed(f);
                self.clear_choices();
            }
        }
    }

    /// Drop choices the (new) printer does not allow (an unsupported option is cleared, a listed one
    /// kept only when listed, an unknown one kept as a hint), and bring copies within its limit.
    pub fn keep_only_offered(&mut self) {
        self.sides = self.sides.filter(|s| self.sides_state().allows(s));
        self.orientation = self.orientation.filter(|o| self.orientation_state().allows(o));
        self.quality = self.quality.filter(|q| self.quality_state().allows(q));
        self.color = self.color.filter(|c| self.color_state().allows(c));
        self.resolution = self.resolution.filter(|r| self.resolution_state().allows(r));
        self.copies = self.copies_in_range();
    }

    /// What stops a print now. `address_ok`: the address passes the host's policy. `sheets`: the
    /// app's own settings give something to print. `formats`: media types the host can produce.
    pub fn block(&self, address_ok: bool, sheets: bool, formats: &[&str]) -> Option<Block> {
        if self.running() || self.querying() {
            return Some(Block::Busy);
        }
        if !address_ok {
            return Some(Block::Address);
        }
        let Some(printer) = self.printer() else { return Some(Block::NotQueried) };
        if !printer.takes_any(formats) {
            return Some(Block::Format);
        }
        if !sheets {
            return Some(Block::NoPages);
        }
        if !self.confirmed {
            return Some(Block::Unconfirmed);
        }
        None
    }

    /// The settings to send. An unsupported option never carries a value; a supported one carries
    /// only a listed value; an unknown one carries the person's choice as a hint. `media` is the
    /// media-size token the app wants, handled the same way (there is no control: the app names it).
    pub fn request(&self, media: Option<&str>) -> Request {
        let media_size = media.and_then(|m| self.media_state().clamp_to_supported(&m.to_owned()));
        Request {
            copies: Some(self.copies_in_range()).filter(|n| *n > 1 && self.copies_state().is_enabled()),
            sides: self.sides.filter(|s| self.sides_state().allows(s)),
            orientation: self.orientation.filter(|o| self.orientation_state().allows(o)),
            quality: self.quality.filter(|q| self.quality_state().allows(q)),
            color: self.color.filter(|c| self.color_state().allows(c)),
            resolution: self.resolution.filter(|r| self.resolution_state().allows(r)),
            media_size,
            expected_pages: self.expected_pages.filter(|pages| *pages > 0),
        }
    }

    /// A job starts: the confirmation is spent.
    pub fn job_started(&mut self) {
        self.confirmed = false;
        self.job_expected_sheets = self.expected_sheets();
        self.job = JobView::Running { canceling: false };
    }

    /// Cancel was pressed; returns `true` when a running job should be told.
    pub fn job_canceling(&mut self) -> bool {
        match &mut self.job {
            JobView::Running { canceling } | JobView::Accepted { canceling, .. } if !*canceling => {
                *canceling = true;
                true
            }
            _ => false,
        }
    }

    /// The printer accepted the document. Ignored unless a job is being sent; a cancel already
    /// pressed is kept.
    pub fn job_accepted(&mut self, progress: Progress) {
        let progress = progress.with_expected_total(self.job_expected_sheets);
        if let JobView::Running { canceling } = self.job {
            self.job = JobView::Accepted { canceling, progress };
        }
    }

    /// A newer status of the accepted job. A status that arrives before the acceptance is taken as
    /// the acceptance; one that arrives after the job ended is dropped.
    pub fn job_progress(&mut self, progress: Progress) {
        let progress = progress.with_expected_total(self.job_expected_sheets);
        match &mut self.job {
            JobView::Accepted { progress: shown, .. } => *shown = progress,
            JobView::Running { canceling } => self.job = JobView::Accepted { canceling: *canceling, progress },
            _ => {}
        }
    }

    /// Set the job's view: how it ended, or [`JobView::Idle`] to clear it.
    pub fn job_finished(&mut self, view: JobView) {
        self.job = view;
    }
}

pub type QueryReceiver = Receiver<Result<PrinterSummary, Failure>>;

/// Ask the printer at the (normalised) address what it supports.
pub type QueryPrinter = Box<dyn Fn(&str) -> QueryReceiver>;

/// A running job: its result, optionally what happens on the way, and a way to ask the host to stop
/// it. Build it with [`JobHandle::new`] (and [`JobHandle::with_updates`]).
///
/// The worker sends [`JobUpdate::Accepted`] when the printer has the document, [`JobUpdate::Status`]
/// as it learns more, and last the result on `result`; for a printer that cannot report it sends
/// `Accepted` and then `Ok(Report { outcome: Outcome::AcceptedUnknown, .. })`. A worker that sends no
/// updates behaves as before. A worker that stops after `Accepted` without a result is shown as
/// sent, outcome unknown, not as a failure.
#[non_exhaustive]
pub struct JobHandle {
    pub result: Receiver<Result<Report, Failure>>,
    pub updates: Option<Receiver<JobUpdate>>,
    pub cancel: Box<dyn Fn()>,
}

impl JobHandle {
    /// A handle with a result and a cancel, and no updates.
    pub fn new(result: Receiver<Result<Report, Failure>>, cancel: Box<dyn Fn()>) -> Self {
        JobHandle { result, updates: None, cancel }
    }

    /// Also deliver what happens between acceptance and the result.
    #[must_use]
    pub fn with_updates(mut self, updates: Receiver<JobUpdate>) -> Self {
        self.updates = Some(updates);
        self
    }
}

/// Send the app's document to the (normalised) address with the person's settings.
pub type SendJob = Box<dyn Fn(&str, Request) -> JobHandle>;

/// Validate a normalised address against the host's network policy. `Err` is shown inline.
pub type Validate = dyn Fn(&str) -> Result<(), Failure>;

/// App-localised wording. Strings documented with `{state}`, `{reasons}`, `{printer}` or `{n}`
/// are templates; identity never depends on the text.
pub struct Labels<'a> {
    pub address: &'a str,
    /// Example text shown in an empty address field.
    pub address_hint: &'a str,
    /// A note under the field saying which addresses are accepted; empty to omit.
    pub address_note: &'a str,
    pub query_printer: &'a str,
    pub querying: &'a str,
    pub query_first: &'a str,
    /// Template with `{state}`.
    pub printer_status: &'a str,
    /// Template with `{reasons}`.
    pub reasons: &'a str,
    pub no_common_format: &'a str,
    pub copies: &'a str,
    pub sides: &'a str,
    pub sides_off: &'a str,
    pub sides_long_edge: &'a str,
    pub sides_short_edge: &'a str,
    pub orientation: &'a str,
    pub portrait: &'a str,
    pub landscape: &'a str,
    pub reverse_portrait: &'a str,
    pub reverse_landscape: &'a str,
    pub quality: &'a str,
    pub draft: &'a str,
    pub normal: &'a str,
    pub high: &'a str,
    pub photo: &'a str,
    pub color: &'a str,
    pub color_color: &'a str,
    pub color_monochrome: &'a str,
    pub resolution: &'a str,
    /// Template with `{dpi}`: a square resolution.
    pub resolution_dpi: &'a str,
    /// Template with `{x}` and `{y}`: a resolution that differs by axis.
    pub resolution_dpi_xy: &'a str,
    /// The printer's own default.
    pub auto: &'a str,
    /// Shown, with the disabled control, when the printer answered and does not support the option
    /// (for example "This printer only prints one-sided."). The control stays visible so people
    /// understand why; the reason is also its accessible description.
    pub unsupported_copies: &'a str,
    pub unsupported_sides: &'a str,
    pub unsupported_orientation: &'a str,
    pub unsupported_quality: &'a str,
    pub unsupported_color: &'a str,
    pub unsupported_resolution: &'a str,
    /// A quiet note for an enabled option the printer did not publish ("not confirmed by the
    /// printer"); the value is sent as a hint. Empty to omit.
    pub unconfirmed_note: &'a str,
    /// The wording of the per-job confirmation belongs to the app.
    pub confirm: &'a str,
    /// Template with `{printer}`.
    pub sending: &'a str,
    pub canceling: &'a str,
    pub cancel: &'a str,
    /// Template with `{printer}`.
    pub sent: &'a str,
    /// Template with `{printer}`: the printer has the document and nothing more is known yet.
    pub accepted: &'a str,
    pub stage_waiting: &'a str,
    pub stage_held: &'a str,
    pub stage_printing: &'a str,
    pub stage_stopped: &'a str,
    /// Template with `{n}`.
    pub sheets: &'a str,
    /// Template with `{done}` and `{total}`.
    pub sheets_progress: &'a str,
    pub outcome_warnings: &'a str,
    pub outcome_errors: &'a str,
    pub outcome_canceled: &'a str,
    pub outcome_aborted: &'a str,
    pub outcome_forgotten: &'a str,
    pub outcome_still_running: &'a str,
    /// Sent, and the printer cannot say what became of it.
    pub outcome_accepted_unknown: &'a str,
    /// Shown under a failure whose request may have reached the printer: check the printer before
    /// sending again.
    pub outcome_unknown_warning: &'a str,
    pub failure_policy: &'a str,
    pub failure_transport: &'a str,
    pub failure_protocol: &'a str,
    pub failure_fault: &'a str,
    pub failure_no_common_format: &'a str,
    pub failure_unsupported: &'a str,
    pub failure_encode: &'a str,
    pub failure_invalid: &'a str,
    pub failure_stopped: &'a str,
    pub failure_other: &'a str,
}

impl Labels<'_> {
    /// The application's heading for a failure of `kind`.
    pub fn failure_heading(&self, kind: FailureKind) -> &str {
        match kind {
            FailureKind::Policy => self.failure_policy,
            FailureKind::Transport => self.failure_transport,
            FailureKind::Protocol => self.failure_protocol,
            FailureKind::Fault => self.failure_fault,
            FailureKind::NoCommonFormat => self.failure_no_common_format,
            FailureKind::Unsupported => self.failure_unsupported,
            FailureKind::Encode => self.failure_encode,
            FailureKind::Invalid => self.failure_invalid,
            FailureKind::Stopped => self.failure_stopped,
            FailureKind::Other => self.failure_other,
        }
    }

    /// The sentence for an ending; `None` for a plain success (the "sent" notice says it).
    pub fn outcome_text(&self, outcome: Outcome) -> Option<&str> {
        match outcome {
            Outcome::Completed => None,
            Outcome::CompletedWithWarnings => Some(self.outcome_warnings),
            Outcome::CompletedWithErrors => Some(self.outcome_errors),
            Outcome::Canceled => Some(self.outcome_canceled),
            Outcome::Aborted => Some(self.outcome_aborted),
            Outcome::Forgotten => Some(self.outcome_forgotten),
            Outcome::StillRunning => Some(self.outcome_still_running),
            Outcome::AcceptedUnknown => Some(self.outcome_accepted_unknown),
        }
    }

    /// The sentence for a stage of an accepted job; `{printer}` is filled for [`Stage::Accepted`].
    pub fn stage_text(&self, stage: Stage, printer: &str) -> String {
        match stage {
            Stage::Accepted => fill(self.accepted, "printer", printer),
            Stage::Waiting => self.stage_waiting.to_owned(),
            Stage::Held => self.stage_held.to_owned(),
            Stage::Printing => self.stage_printing.to_owned(),
            Stage::Stopped => self.stage_stopped.to_owned(),
        }
    }

    fn sides_label(&self, sides: Sides) -> &str {
        match sides {
            Sides::OneSided => self.sides_off,
            Sides::LongEdge => self.sides_long_edge,
            Sides::ShortEdge => self.sides_short_edge,
        }
    }

    fn orientation_label(&self, orientation: Orientation) -> &str {
        match orientation {
            Orientation::Portrait => self.portrait,
            Orientation::Landscape => self.landscape,
            Orientation::ReversePortrait => self.reverse_portrait,
            Orientation::ReverseLandscape => self.reverse_landscape,
        }
    }

    fn color_label(&self, color: ColorMode) -> &str {
        match color {
            ColorMode::Color => self.color_color,
            ColorMode::Monochrome => self.color_monochrome,
        }
    }

    fn resolution_label(&self, r: Resolution) -> String {
        if r.x_dpi == r.y_dpi {
            fill(self.resolution_dpi, "dpi", &r.x_dpi.to_string())
        } else {
            fill(&fill(self.resolution_dpi_xy, "x", &r.x_dpi.to_string()), "y", &r.y_dpi.to_string())
        }
    }

    /// The note under an option's control: the reason when it is unsupported, the quiet note when
    /// the printer did not say, nothing when it listed it.
    fn note<'s, T>(&'s self, state: &Capability<T>, reason: &'s str) -> Option<&'s str> {
        let text = match state {
            Capability::Unsupported => reason,
            Capability::Unknown => self.unconfirmed_note,
            _ => "",
        };
        (!text.is_empty()).then_some(text)
    }

    fn quality_label(&self, quality: Quality) -> &str {
        match quality {
            Quality::Draft => self.draft,
            Quality::Normal => self.normal,
            Quality::High => self.high,
            Quality::Photo => self.photo,
        }
    }
}

fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(&format!("{{{key}}}"), value)
}

/// What a frame of [`State::show`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Why Print is unavailable now (`None` means it may start), for the app's Print button.
    pub block: Option<Block>,
    /// An option, the address or the confirmation changed.
    pub changed: bool,
    /// Query printer was clicked and a query was started.
    pub query_started: bool,
    /// Cancel was clicked and the host was told.
    pub cancel_requested: bool,
}

/// The destination's state: the model plus the workers it is waiting for.
#[derive(Default)]
pub struct State {
    pub draft: Draft,
    query_rx: Option<(u64, String, QueryReceiver)>,
    job: Option<JobHandle>,
}

impl State {
    /// The panel state for `draft`, with nothing else started.
    pub fn new(draft: Draft) -> Self {
        State { draft, ..Default::default() }
    }

    /// Start a query. False when the address is not acceptable, a query or job is active; no
    /// worker is started then.
    pub fn start_query(&mut self, query: &QueryPrinter, address_ok: bool) -> bool {
        if !address_ok || self.draft.querying() || self.draft.running() {
            return false;
        }
        let url = self.draft.normalized_url();
        let generation = self.draft.begin_query();
        self.query_rx = Some((generation, url.clone(), query(&url)));
        true
    }

    /// Start a job when nothing blocks it ([`Draft::block`]); the confirmation is spent. `media`
    /// is the media-size token the app wants (used only when the printer lists it).
    pub fn start_job(&mut self, send: &SendJob, address_ok: bool, sheets: bool, formats: &[&str], media: Option<&str>) -> bool {
        if self.draft.block(address_ok, sheets, formats).is_some() {
            return false;
        }
        let request = self.draft.request(media);
        let handle = send(&self.draft.normalized_url(), request);
        self.draft.job_started();
        self.job = Some(handle);
        true
    }

    /// Ask the host to stop the running job, once. True when it was told.
    pub fn cancel_job(&mut self) -> bool {
        if !self.draft.job_canceling() {
            return false;
        }
        if let Some(job) = &self.job {
            (job.cancel)();
        }
        true
    }

    /// The dialog opens (see [`Draft::dialog_opened`]).
    pub fn dialog_opened(&mut self) {
        self.draft.dialog_opened();
    }

    /// Collect finished work. Requests occasional repaints while work is pending.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some((generation, url, receiver)) = &self.query_rx {
            let (generation, url) = (*generation, url.clone());
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(Failure::new(FailureKind::Stopped, ""))),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.query_rx = None;
                self.draft.finish_query(generation, &url, result);
            }
        }
        if let Some(job) = &self.job {
            // The result is read first and the updates after it: a worker sends its updates before it
            // sends the result or drops its sender, so everything it said is already queued once its
            // end is seen. Reading them the other way round could see the end of a worker that sent
            // `Accepted` between the two reads, and report a document the printer has as a failure.
            let received = job.result.try_recv();
            // What happened on the way first, in order, then the result; one frame drains them all.
            if let Some(updates) = &job.updates {
                while let Ok(update) = updates.try_recv() {
                    match update {
                        JobUpdate::Accepted(progress) => self.draft.job_accepted(progress),
                        JobUpdate::Status(progress) => self.draft.job_progress(progress),
                    }
                }
            }
            let view = match received {
                Ok(Ok(report)) => Some(JobView::Finished(report)),
                Ok(Err(failure)) => Some(JobView::Failed(failure)),
                // A worker that ended after the printer accepted the document did not fail: it sent.
                Err(TryRecvError::Disconnected) => Some(match &self.draft.job {
                    JobView::Accepted { .. } => JobView::Finished(Report {
                        outcome: Outcome::AcceptedUnknown,
                        printer: self.draft.printer().map(|p| p.name.clone()).unwrap_or_default(),
                        sheets: 0,
                        notes: Vec::new(),
                    }),
                    // Still sending: the document may already be on the printer (the reply was lost, or the
                    // worker died after the send), so the person is told to look before sending again.
                    JobView::Running { .. } => JobView::Failed(Failure::new(FailureKind::Stopped, "").with_outcome_unknown(true)),
                    _ => JobView::Failed(Failure::new(FailureKind::Stopped, "")),
                }),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(view) = view {
                self.job = None;
                self.draft.job_finished(view);
            }
        }
        if self.query_rx.is_some() || self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Draw the section: address, Query button, printer facts, listed options, the confirmation
    /// and the job's progress. The Print button stays the app's: enable it when
    /// [`Response::block`] is `None`, then call [`State::start_job`].
    pub fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, host: &Host<'_>, labels: &Labels<'_>) -> Response {
        self.poll(ui.ctx());
        ui.push_id(id, |ui| self.draw(ui, host, labels)).inner
    }

    fn draw(&mut self, ui: &mut egui::Ui, host: &Host<'_>, labels: &Labels<'_>) -> Response {
        let mut response = Response::default();
        let busy = self.draft.running();
        let checked = (host.validate)(&self.draft.normalized_url());
        let address_ok = checked.is_ok();
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(labels.address);
            let edit = ui
                .add_enabled(!busy, egui::TextEdit::singleline(&mut self.draft.url).hint_text(labels.address_hint).desired_width(240.0))
                .labelled_by(label.id);
            if edit.changed() {
                self.draft.address_edited();
                response.changed = true;
            }
            let can_query = address_ok && !busy && !self.draft.querying();
            if ui.add_enabled(can_query, egui::Button::new(labels.query_printer)).clicked() {
                response.query_started = self.start_query(host.query, address_ok);
            }
        });
        match (&checked, self.draft.url.trim().is_empty()) {
            (Err(failure), false) => failure_lines(ui, labels, failure),
            _ if !labels.address_note.is_empty() => {
                ui.label(egui::RichText::new(labels.address_note).small().weak());
            }
            _ => {}
        }
        if self.draft.querying() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(labels.querying);
            });
        }
        if let Query::Failed(failure) = &self.draft.query {
            failure_lines(ui, labels, failure);
        }
        response.changed |= self.printer_and_options(ui, labels, address_ok, host.formats);
        // The yes to this one print: off every time the dialog opens, the address changes or a job starts.
        let can_confirm = self.draft.printer().is_some() && !busy;
        if ui.add_enabled(can_confirm, egui::Checkbox::new(&mut self.draft.confirmed, labels.confirm)).changed() {
            response.changed = true;
        }
        response.block = self.draft.block(address_ok, host.sheets_ok, host.formats);
        response.cancel_requested = self.job_lines(ui, labels);
        response
    }

    fn printer_and_options(&mut self, ui: &mut egui::Ui, labels: &Labels<'_>, address_ok: bool, formats: &[&str]) -> bool {
        let Some(printer) = self.draft.printer().cloned() else {
            if self.draft.query == Query::Idle && address_ok {
                ui.label(egui::RichText::new(labels.query_first).small().weak());
            }
            return false;
        };
        let mut changed = false;
        ui.add_space(2.0);
        ui.label(egui::RichText::new(&printer.name).strong());
        ui.label(egui::RichText::new(fill(labels.printer_status, "state", &printer.state)).small().weak());
        if !printer.reasons.is_empty() {
            ui.label(egui::RichText::new(fill(labels.reasons, "reasons", &printer.reasons.join(", "))).small().weak());
        }
        if !printer.takes_any(formats) {
            ui.label(labels.no_common_format);
            return false;
        }
        let draft = &mut self.draft;
        let (copies_state, sides_state, orientation_state) = (draft.copies_state(), draft.sides_state(), draft.orientation_state());
        let (quality_state, color_state, resolution_state) = (draft.quality_state(), draft.color_state(), draft.resolution_state());
        let sides = draft.offered_sides();
        let orientations = draft.offered_orientations();
        let qualities = draft.offered_qualities();
        let colors = draft.offered_colors();
        let resolutions = draft.offered_resolutions();
        let (min, max) = (draft.min_copies(), draft.max_copies());
        // An unsupported option holds no value, so the disabled control shows the printer's default.
        if !copies_state.is_enabled() {
            draft.copies = 1;
        }
        if !sides_state.is_enabled() {
            draft.sides = None;
        }
        if !orientation_state.is_enabled() {
            draft.orientation = None;
        }
        if !quality_state.is_enabled() {
            draft.quality = None;
        }
        if !color_state.is_enabled() {
            draft.color = None;
        }
        if !resolution_state.is_enabled() {
            draft.resolution = None;
        }
        ui.add_enabled_ui(!draft.running(), |ui| {
            egui::Grid::new("options").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                let copies = &mut draft.copies;
                changed |=
                    option_row(ui, labels.copies, copies_state.is_enabled(), labels.note(&copies_state, labels.unsupported_copies), |ui, label| {
                        let response = ui.add(egui::DragValue::new(copies).range(min..=max)).labelled_by(label);
                        (response.clone(), response.changed())
                    });
                changed |=
                    option_row(ui, labels.sides, sides_state.is_enabled(), labels.note(&sides_state, labels.unsupported_sides), |ui, label| {
                        choice(ui, "sides", label, labels.auto, &mut draft.sides, &sides, |s| labels.sides_label(s).to_owned())
                    });
                changed |= option_row(
                    ui,
                    labels.orientation,
                    orientation_state.is_enabled(),
                    labels.note(&orientation_state, labels.unsupported_orientation),
                    |ui, label| {
                        choice(ui, "orientation", label, labels.auto, &mut draft.orientation, &orientations, |o| {
                            labels.orientation_label(o).to_owned()
                        })
                    },
                );
                changed |= option_row(
                    ui,
                    labels.quality,
                    quality_state.is_enabled(),
                    labels.note(&quality_state, labels.unsupported_quality),
                    |ui, label| choice(ui, "quality", label, labels.auto, &mut draft.quality, &qualities, |q| labels.quality_label(q).to_owned()),
                );
                changed |=
                    option_row(ui, labels.color, color_state.is_enabled(), labels.note(&color_state, labels.unsupported_color), |ui, label| {
                        choice(ui, "color", label, labels.auto, &mut draft.color, &colors, |c| labels.color_label(c).to_owned())
                    });
                changed |= option_row(
                    ui,
                    labels.resolution,
                    resolution_state.is_enabled(),
                    labels.note(&resolution_state, labels.unsupported_resolution),
                    |ui, label| choice(ui, "resolution", label, labels.auto, &mut draft.resolution, &resolutions, |r| labels.resolution_label(r)),
                );
            });
        });
        changed
    }

    /// Progress with Cancel while a job runs, then how it ended. True when Cancel told the host.
    fn job_lines(&mut self, ui: &mut egui::Ui, labels: &Labels<'_>) -> bool {
        match self.draft.job.clone() {
            JobView::Idle => false,
            JobView::Running { canceling } => {
                let mut cancel = false;
                ui.horizontal(|ui| {
                    ui.spinner();
                    let printer = self.draft.printer().map_or_else(|| self.draft.url.trim().to_owned(), |p| p.name.clone());
                    ui.label(if canceling { labels.canceling.to_owned() } else { fill(labels.sending, "printer", &printer) });
                    if ui.add_enabled(!canceling, egui::Button::new(labels.cancel)).clicked() {
                        cancel = self.cancel_job();
                    }
                });
                cancel
            }
            JobView::Accepted { canceling, progress } => {
                let mut cancel = false;
                let printer = self.draft.printer().map_or_else(|| self.draft.url.trim().to_owned(), |p| p.name.clone());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if canceling { labels.canceling.to_owned() } else { labels.stage_text(progress.stage, &printer) });
                    if ui.add_enabled(!canceling, egui::Button::new(labels.cancel)).clicked() {
                        cancel = self.cancel_job();
                    }
                });
                match (progress.fraction(), progress.sheets_done, progress.sheets_total) {
                    // Known total: a determinate bar.
                    (Some(fraction), Some(done), Some(total)) => {
                        let text = fill(&fill(labels.sheets_progress, "done", &done.to_string()), "total", &total.to_string());
                        ui.add(egui::ProgressBar::new(fraction).text(text));
                    }
                    // Unknown total: the count alone, no bar that could never fill.
                    (_, Some(done), _) if done > 0 => {
                        ui.label(egui::RichText::new(fill(labels.sheets, "n", &done.to_string())).small().weak());
                    }
                    _ => {}
                }
                for note in &progress.notes {
                    ui.label(egui::RichText::new(note).small().weak());
                }
                cancel
            }
            JobView::Finished(report) => {
                let text = labels.outcome_text(report.outcome).map_or_else(|| fill(labels.sent, "printer", &report.printer), str::to_owned);
                ui.label(egui::RichText::new(text).strong());
                if report.sheets > 0 {
                    ui.label(egui::RichText::new(fill(labels.sheets, "n", &report.sheets.to_string())).small().weak());
                }
                for note in &report.notes {
                    ui.label(egui::RichText::new(note).small().weak());
                }
                false
            }
            JobView::Failed(failure) => {
                failure_lines(ui, labels, &failure);
                false
            }
        }
    }
}

/// What the host supplies to [`State::show`] besides wording.
pub struct Host<'a> {
    /// Checks the normalised address against the host's network policy; `Err` is shown inline.
    pub validate: &'a Validate,
    /// Asks a printer what it supports.
    pub query: &'a QueryPrinter,
    /// The app's own settings give something to print.
    pub sheets_ok: bool,
    /// Media types the app can produce; a printer listing none of them blocks Print.
    pub formats: &'a [&'a str],
}

fn failure_lines(ui: &mut egui::Ui, labels: &Labels<'_>, failure: &Failure) {
    ui.label(egui::RichText::new(labels.failure_heading(failure.kind)).strong());
    if !failure.detail.is_empty() {
        ui.label(egui::RichText::new(&failure.detail).small().weak());
    }
    if failure.outcome_unknown {
        ui.label(egui::RichText::new(labels.outcome_unknown_warning).small().strong());
    }
}

/// One row of the options grid: the label, the control and, under it, a small note. `enabled` is
/// false for an unsupported option: the control stays, disabled, and the note is its reason. The
/// note is also the control's accessible description. `add` draws the control (given the label's id
/// for `labelled_by`) and returns its response and whether the value changed.
fn option_row(
    ui: &mut egui::Ui,
    text: &str,
    enabled: bool,
    note: Option<&str>,
    add: impl FnOnce(&mut egui::Ui, egui::Id) -> (egui::Response, bool),
) -> bool {
    let label = ui.label(text);
    let mut changed = false;
    ui.vertical(|ui| {
        let (response, did_change) = ui.add_enabled_ui(enabled, |ui| add(ui, label.id)).inner;
        changed = did_change && enabled;
        if let Some(note) = note {
            ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(note));
            ui.label(egui::RichText::new(note).small().weak());
        }
    });
    ui.end_row();
    changed
}

/// A choice among what the printer lists, with the printer's own default first. Returns the
/// control's response and whether the value changed.
fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    label: egui::Id,
    auto: &str,
    value: &mut Option<T>,
    offered: &[T],
    name: impl Fn(T) -> String,
) -> (egui::Response, bool) {
    let before = *value;
    let shown = value.map_or_else(|| auto.to_owned(), &name);
    let response = egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(180.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, auto);
            for &o in offered {
                ui.selectable_value(value, Some(o), name(o));
            }
        })
        .response
        .labelled_by(label);
    (response, *value != before)
}

#[cfg(test)]
mod tests;
