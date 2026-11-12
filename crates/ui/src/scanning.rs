//! Scanner selection and capture settings, independent of any scanning backend or document.
//! Hosts provide workers; applications own the Scan action, interruptions, duplex flows, document
//! handling, commands, theme and translated wording.
//!
//! Listing scanners opens nothing. Details (what a scanner supports) are requested only for the
//! one selected scanner, and a scanner whose details cannot be read stays listed and selected with
//! all-unknown [`Choices`] so scanning remains possible.
//!
//! Every option is three-state ([`Capability`]): supported (the device published a list or range:
//! enabled, restricted to it, device default first), unsupported (the device published its answer
//! without it, or reports the unit absent: shown disabled with a reason, nothing sent) or unknown
//! (not published, or the details read failed: enabled with a quiet note, sent only as a hint).
//! Capability is static; the device's dynamic status (tray empty, cover open) is a separate list of
//! [`Warning`]s that only warns and never disables anything.

use crate::capability::{Capability, Choices as Offered};
use std::sync::mpsc::{Receiver, TryRecvError};

/// How a scanner is reached. Backends map their own transport to one of these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    Usb,
    Network,
    #[default]
    Unknown,
}

/// One listed scanner. `id` is the backend's stable identity; `name` is display text only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scanner {
    pub id: String,
    pub name: String,
    pub connection: Connection,
}

/// Where pages come from. Apps map their own enum to this one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The backend picks: the feeder when a sheet is waiting, otherwise the glass.
    #[default]
    Auto,
    Feeder,
    Glass,
}

/// Colour modes. Apps map their own enum to this one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Color {
    #[default]
    Gray,
    Color,
    BlackAndWhite,
}

/// How many pages to take from the feeder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pages {
    /// Until the feeder is empty.
    #[default]
    All,
    Count(u32),
}

/// Quick starting points. Applying one sets colour and resolution from what the scanner offers;
/// the user can still change either afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Grey at a document resolution: text and OCR.
    DocumentText,
    ColorDocument,
    BlackAndWhite,
    /// Colour at the highest photo resolution up to 600 dpi.
    Photo,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::DocumentText, Preset::ColorDocument, Preset::BlackAndWhite, Preset::Photo];

    /// The colour mode this preset selects.
    fn color(self) -> Color {
        match self {
            Preset::DocumentText => Color::Gray,
            Preset::ColorDocument | Preset::Photo => Color::Color,
            Preset::BlackAndWhite => Color::BlackAndWhite,
        }
    }

    /// The resolution this preset selects from the `listed` ones, or `None` when none are listed.
    fn dpi(self, listed: &[u32]) -> Option<u32> {
        match self {
            Preset::Photo => photo_dpi(listed),
            _ => preferred_dpi(listed),
        }
    }

    /// Whether this preset makes sense for `choices`: unknown colour modes offer every preset, an
    /// unsupported colour option none, a published list the presets whose colour it lists.
    pub fn offered(self, choices: &Choices) -> bool {
        choices.colors.allows(&self.color())
    }
}

/// What the selected scanner supports, one [`Capability`] per option. `Choices::default()` is all
/// unknown: it is what applies when the details cannot be read, and nothing in it is hidden or
/// disabled.
///
/// Build one by starting from `Choices::default()` and setting the fields the device answered.
/// [`presence`] maps a yes/no/unknown answer for a unit; `Capability::from_option_list`,
/// `from_option_range` and `list_default_first` map a published list (the last puts the device
/// default first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Choices {
    /// The automatic document feeder. Unsupported means no feeder: the feeder source, the page
    /// count and duplex are disabled and `Auto` uses the glass.
    pub feeder: Capability<()>,
    /// The flatbed glass.
    pub glass: Capability<()>,
    pub colors: Capability<Color>,
    /// Resolutions in dots per inch, in the order to list them (a range is offered as the common
    /// values inside it).
    pub dpi: Capability<u32>,
    /// Largest feeder page count the scanner accepts, as a range `1..=max`. Unknown is up to
    /// [`MAX_PAGES`]; unsupported means the count cannot be chosen.
    pub max_pages: Capability<u32>,
    /// Whether "until the feeder is empty" is accepted.
    pub all_pages: Capability<()>,
    /// Hardware duplex (both sides in one pass). Unsupported means no duplex unit.
    pub duplex: Capability<()>,
    /// Whether the scanner detects the length of a sheet itself. Supported: it published the feature.
    /// Unsupported: it answered without it, so "Detect (scanner)" is disabled with a reason. Unknown (the
    /// default): not published, so the option stays enabled with the quiet note and is only a hint.
    pub detect_length: Capability<()>,
}

/// Upper bound for an unknown page limit and for the page count control.
pub const MAX_PAGES: u32 = 999;

/// Values offered for a resolution the device did not list (or listed as a range).
pub const COMMON_DPI: [u32; 6] = [75, 100, 150, 200, 300, 600];

/// A yes/no/unknown answer about a unit (feeder, glass, duplex unit) as a [`Capability`].
pub fn presence(present: Option<bool>) -> Capability<()> {
    match present {
        None => Capability::Unknown,
        Some(true) => Capability::list_default_first(&[()], None),
        Some(false) => Capability::Unsupported,
    }
}

impl Choices {
    /// Whether `source` can be chosen. `Auto` can be unless both units are unsupported.
    pub fn offers(&self, source: Source) -> bool {
        match source {
            Source::Feeder => self.feeder.is_enabled(),
            Source::Glass => self.glass.is_enabled(),
            Source::Auto => self.feeder.is_enabled() || self.glass.is_enabled(),
        }
    }

    /// What `source` really means here: `Auto` becomes the glass when the feeder is unsupported and
    /// the feeder when the glass is; everything else is unchanged.
    pub fn resolve(&self, source: Source) -> Source {
        match source {
            Source::Auto if self.feeder.is_unsupported() && self.glass.is_enabled() => Source::Glass,
            Source::Auto if self.glass.is_unsupported() && self.feeder.is_enabled() => Source::Feeder,
            other => other,
        }
    }

    /// Whether the page count and duplex controls apply to `source`.
    pub fn feeder_in_play(&self, source: Source) -> bool {
        self.resolve(source) != Source::Glass && self.feeder.is_enabled()
    }

    /// The most pages one request may ask for: the largest value the device offers, capped at `MAX_PAGES`.
    fn page_limit(&self) -> u32 {
        match self.max_pages.choices() {
            Some(Offered::Range { max, .. }) => (*max).clamp(1, MAX_PAGES),
            Some(Offered::List(values)) => values.iter().copied().max().unwrap_or(MAX_PAGES).clamp(1, MAX_PAGES),
            _ => MAX_PAGES,
        }
    }

    /// Resolutions to offer: the list as published (device default first when the app built it
    /// with `list_default_first`), the common values inside a published range, the common values
    /// when unknown, nothing when unsupported.
    pub fn dpi_options(&self) -> Vec<u32> {
        match self.dpi.choices() {
            Some(Offered::List(values)) => values.clone(),
            Some(Offered::Range { min, max }) => {
                let inside: Vec<u32> = COMMON_DPI.iter().copied().filter(|d| min <= d && d <= max).collect();
                if inside.is_empty() { vec![*min, *max] } else { inside }
            }
            _ if self.dpi.is_unsupported() => Vec::new(),
            _ => COMMON_DPI.to_vec(),
        }
    }

    /// Colour modes to offer: the published list, every mode when unknown, none when unsupported.
    pub fn color_options(&self) -> Vec<Color> {
        if self.colors.is_unsupported() {
            Vec::new()
        } else if self.colors.is_supported() {
            self.colors.list().to_vec()
        } else {
            vec![Color::Gray, Color::Color, Color::BlackAndWhite]
        }
    }

    /// Resolutions a preset may pick from: only what the device published.
    fn published_dpi(&self) -> Vec<u32> {
        if self.dpi.is_supported() { self.dpi_options() } else { Vec::new() }
    }

    /// The settings as a request: unsupported options are omitted (nothing is sent), unknown ones
    /// are marked as hints, supported ones carry a value the device listed (a range value is moved
    /// into the range). Call after [`Settings::clamp`].
    pub fn request(&self, settings: &Settings) -> Request {
        let resolved = self.resolve(settings.source);
        let source = match resolved {
            Source::Feeder => unit_sent(&self.feeder, resolved),
            Source::Glass => unit_sent(&self.glass, resolved),
            Source::Auto if self.feeder.is_supported() || self.glass.is_supported() => Sent::Value(Source::Auto),
            Source::Auto => Sent::Hint(Source::Auto),
        };
        let in_play = self.feeder_in_play(settings.source);
        let pages = if !in_play {
            Sent::Omitted
        } else {
            match settings.pages {
                Pages::All => unit_sent(&self.all_pages, Pages::All),
                Pages::Count(n) => match self.max_pages.clamp_to_supported(&n) {
                    None => Sent::Omitted,
                    Some(n) if self.max_pages.is_supported() => Sent::Value(Pages::Count(n.clamp(1, MAX_PAGES))),
                    Some(n) => Sent::Hint(Pages::Count(n.clamp(1, MAX_PAGES))),
                },
            }
        };
        let duplex = match (in_play, settings.duplex) {
            (true, true) => unit_sent(&self.duplex, true),
            (true, false) if self.duplex.is_supported() => Sent::Value(false),
            _ => Sent::Omitted,
        };
        let page_length = if !in_play {
            Sent::Omitted
        } else {
            match self.length_availability(settings.page_length, settings.page_size_chosen) {
                LengthAvailability::Available => Sent::Value(settings.page_length),
                LengthAvailability::Unconfirmed => Sent::Hint(settings.page_length),
                LengthAvailability::Unavailable(_) => Sent::Omitted,
            }
        };
        Request { source, color: value_sent(&self.colors, &settings.color), dpi: value_sent(&self.dpi, &settings.dpi), pages, duplex, page_length }
    }
}

/// What to send for a value that depends on a unit capability (`Capability<()>`).
fn unit_sent<T>(unit: &Capability<()>, value: T) -> Sent<T> {
    match unit {
        Capability::Unsupported => Sent::Omitted,
        Capability::Unknown => Sent::Hint(value),
        _ => Sent::Value(value),
    }
}

/// What to send for `value` given what `cap` offers: omitted when nothing fits, a hint when the capability is
/// unconfirmed, else the value clamped into what the device listed.
fn value_sent<T: PartialOrd + Clone>(cap: &Capability<T>, value: &T) -> Sent<T> {
    match (cap.clamp_to_supported(value), cap.is_supported()) {
        (None, _) => Sent::Omitted,
        (Some(v), true) => Sent::Value(v),
        (Some(v), false) => Sent::Hint(v),
    }
}

/// One option as it goes to the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Sent<T> {
    /// Not sent: the device said it does not have the option (or it does not apply).
    Omitted,
    /// The device did not say whether it supports this. Send it as a hint the backend may ignore.
    Hint(T),
    /// The device listed this value (or a range containing it).
    Value(T),
}

impl<T> Sent<T> {
    /// The value to put on the wire, hint or not; `None` when omitted.
    pub fn value(&self) -> Option<&T> {
        match self {
            Sent::Omitted => None,
            Sent::Hint(v) | Sent::Value(v) => Some(v),
        }
    }

    /// Whether the value is only a hint the backend may ignore.
    pub fn is_hint(&self) -> bool {
        matches!(self, Sent::Hint(_))
    }
}

/// What to ask the backend for. See [`Choices::request`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Request {
    pub source: Sent<Source>,
    pub color: Sent<Color>,
    pub dpi: Sent<u32>,
    pub pages: Sent<Pages>,
    pub duplex: Sent<bool>,
    /// Omitted off the feeder and for an option the scanner or the page size rules out; a hint for an
    /// unconfirmed detection.
    pub page_length: Sent<PageLength>,
}

/// The device's dynamic status. A warning is shown beside the controls and never disables one:
/// capability (what the device can do) and status (what it is doing now) are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// No paper in the feeder.
    FeederEmpty,
    /// A cover or door is open.
    CoverOpen,
    /// The feeder reported ready, yet no sheet is seen: the report may be wrong.
    FeederReadyButEmpty,
}

/// The user's choices. `scanner` is the backend id and is what apps remember between sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub scanner: String,
    pub source: Source,
    pub color: Color,
    pub dpi: u32,
    pub pages: Pages,
    /// Hardware duplex. Only sent where [`Choices::duplex`] is not unsupported (see [`Choices::request`]).
    pub duplex: bool,
    /// The preset last applied, cleared when colour or resolution is changed by hand.
    pub preset: Option<Preset>,
    /// How the page length is decided (feeder only). See [`PageLength`].
    pub page_length: PageLength,
    /// Set by the app each frame: whether it has a page size that names a length. The component cannot see the
    /// app's page size; this is what enables "Match page size".
    pub page_size_chosen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scanner: String::new(),
            source: Source::Auto,
            color: Color::Gray,
            dpi: 300,
            pages: Pages::All,
            duplex: false,
            preset: None,
            page_length: PageLength::Driver,
            page_size_chosen: false,
        }
    }
}

impl Settings {
    /// Apply `preset` against `choices`. A colour mode the scanner lacks falls back to the first one
    /// it lists; an unpublished resolution list leaves the resolution as it is; an unsupported
    /// option is left alone.
    pub fn apply_preset(&mut self, preset: Preset, choices: &Choices) {
        if let Some(dpi) = preset.dpi(&choices.published_dpi()) {
            self.dpi = dpi;
        }
        let wanted = preset.color();
        if !choices.colors.is_unsupported() {
            self.color = if choices.colors.allows(&wanted) { wanted } else { choices.colors.list().first().copied().unwrap_or(wanted) };
        }
        self.preset = Some(preset);
    }

    /// Bring every field into what `choices` allows. Unknown and unsupported options are left
    /// alone (an unsupported one is not sent, see [`Choices::request`]). Never changes `scanner`.
    pub fn clamp(&mut self, choices: &Choices) {
        if !choices.offers(self.source) {
            self.source = [Source::Auto, Source::Glass, Source::Feeder].into_iter().find(|s| choices.offers(*s)).unwrap_or(Source::Auto);
        }
        if choices.dpi.is_supported() && !choices.dpi.allows(&self.dpi) {
            let listed = choices.dpi_options();
            self.dpi = match choices.dpi.choices() {
                Some(Offered::Range { .. }) => choices.dpi.clamp_to_supported(&self.dpi).unwrap_or(self.dpi),
                _ => choices.dpi.default_value().copied().or_else(|| preferred_dpi(&listed)).unwrap_or(self.dpi),
            };
        }
        if choices.colors.is_supported() && !choices.colors.allows(&self.color) {
            let listed = choices.colors.list();
            self.color = choices
                .colors
                .default_value()
                .copied()
                .or_else(|| listed.contains(&Color::Gray).then_some(Color::Gray))
                .or_else(|| listed.first().copied())
                .unwrap_or(self.color);
        }
        self.pages = match self.pages {
            Pages::All if choices.all_pages.is_unsupported() => Pages::Count(1),
            Pages::All => Pages::All,
            Pages::Count(n) => Pages::Count(n.clamp(1, choices.page_limit())),
        };
        if choices.duplex.is_unsupported() || !choices.feeder.is_enabled() {
            self.duplex = false;
        }
        self.page_length = choices.length_or_default(self.page_length, self.page_size_chosen);
    }
}

/// Cheap listing: opens no device. The receiver yields once; a dropped sender means the worker died.
pub type ListReceiver = Receiver<Result<Vec<Scanner>, String>>;
pub type Discover = Box<dyn Fn() -> ListReceiver>;
/// Details for ONE scanner id. Called only for the selected scanner.
pub type ChoicesReceiver = Receiver<Result<Choices, String>>;
pub type LoadChoices = Box<dyn Fn(&str) -> ChoicesReceiver>;

/// Apps format these in their own language. Backend sentences are carried unmodified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Feedback {
    FindingScanners,
    NoScanners,
    /// The listing failed with the backend's sentence.
    DiscoveryFailed(String),
    /// A worker ended without answering.
    WorkerStopped,
    /// The selected scanner's capabilities are being read.
    Checking,
    /// They could not be read. The text is the backend's sentence, empty when the worker stopped
    /// without one. The scanner stays selected with [`Choices::default`] so scanning stays possible.
    DetailsFailed(String),
}

/// What is known about the selected scanner's capabilities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // `Ready` is the common case and boxing it would change the public variant
pub enum DetailsState {
    #[default]
    Idle,
    Loading,
    Ready(Choices),
    Failed(String),
}

/// Note shown for a scanner that is also listed over the other connection. Apps localize it
/// (see [`Labels::also_usb`] and [`Labels::also_network`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamePrinter {
    AlsoUsb,
    AlsoNetwork,
}

/// `name` without a trailing ` [..]` suffix (network entries often carry a device id there).
fn base_name(name: &str) -> &str {
    let trimmed = name.trim_end();
    if trimmed.ends_with(']')
        && let Some(open) = trimmed.rfind(" [")
        && let Some(head) = trimmed.get(..open)
    {
        return head.trim_end();
    }
    trimmed
}

/// Whether two device names are the same once the trailing connection tag is dropped, ignoring case.
fn same_base_name(a: &str, b: &str) -> bool {
    let (a, b) = (base_name(a), base_name(b));
    !a.is_empty() && a.to_lowercase() == b.to_lowercase()
}

/// The note for `item` when another listed scanner has the same base name over a different known
/// connection (one device reached both by USB and over the network). Unknown connections never match.
pub fn same_printer_note(scanners: &[Scanner], item: &Scanner) -> Option<SamePrinter> {
    if item.connection == Connection::Unknown {
        return None;
    }
    let other = scanners
        .iter()
        .find(|o| o.id != item.id && o.connection != Connection::Unknown && o.connection != item.connection && same_base_name(&o.name, &item.name))?;
    match other.connection {
        Connection::Usb => Some(SamePrinter::AlsoUsb),
        Connection::Network => Some(SamePrinter::AlsoNetwork),
        Connection::Unknown => None,
    }
}

/// The scanner to select when none is remembered: the USB entry of a device that is also listed
/// over the network (the vendor's own driver is usually the fuller one), else the first.
pub fn default_scanner_id(scanners: &[Scanner]) -> Option<String> {
    let both = scanners.iter().find(|item| {
        item.connection == Connection::Usb && scanners.iter().any(|o| o.connection == Connection::Network && same_base_name(&o.name, &item.name))
    });
    both.or_else(|| scanners.first()).map(|item| item.id.clone())
}

/// The resolution for document presets: 300 dpi, else 200, else the first one listed.
fn preferred_dpi(listed: &[u32]) -> Option<u32> {
    [300, 200].into_iter().find(|dpi| listed.contains(dpi)).or_else(|| listed.first().copied())
}

/// Prefer 600 dpi for photographs, then 400, then 300, then the highest listed up to 600.
fn photo_dpi(listed: &[u32]) -> Option<u32> {
    [600, 400, 300]
        .into_iter()
        .find(|dpi| listed.contains(dpi))
        .or_else(|| listed.iter().copied().filter(|dpi| *dpi <= 600).max())
        .or_else(|| listed.first().copied())
}

/// What happened in a frame, for the app's own handling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// A setting changed (including the selected scanner). Drop any preview that depended on them.
    pub changed: bool,
    /// Refresh was clicked and a listing was started.
    pub refresh: bool,
    /// Try again was clicked and a details load was started.
    pub retry: bool,
}

enum ListFailure {
    Backend(String),
    Stopped,
}

#[derive(Default)]
pub struct State {
    pub settings: Settings,
    scanners: Vec<Scanner>,
    /// A listing has finished (successfully or not), or one is running.
    requested: bool,
    loading: bool,
    list_failure: Option<ListFailure>,
    generation: u64,
    list_rx: Option<(u64, ListReceiver)>,
    details_id: String,
    details: DetailsState,
    details_generation: u64,
    details_rx: Option<(u64, String, ChoicesReceiver)>,
    warnings: Vec<Warning>,
}

impl State {
    /// The listed scanners, in the order the backend returned them.
    pub fn scanners(&self) -> &[Scanner] {
        &self.scanners
    }

    /// The selected scanner when it is listed.
    pub fn selected(&self) -> Option<&Scanner> {
        self.scanners.iter().find(|s| s.id == self.settings.scanner)
    }

    /// Whether a scanner listing is in progress.
    pub fn loading(&self) -> bool {
        self.loading
    }

    /// Whether the selected scanner's choices are being read.
    pub fn checking(&self) -> bool {
        self.details == DetailsState::Loading
    }

    /// The device's current status warnings, drawn beside the controls. They never disable one.
    pub fn set_warnings(&mut self, warnings: Vec<Warning>) {
        self.warnings = warnings;
    }

    /// The device's current status warnings.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// How reading the selected scanner's choices has gone.
    pub fn details(&self) -> &DetailsState {
        &self.details
    }

    /// The message of a failed choices read, `None` in any other state.
    pub fn details_error(&self) -> Option<&str> {
        match &self.details {
            DetailsState::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// What to draw and scan with: the real choices, or [`Choices::default`] when they could not be
    /// read. `None` while they are still loading (nothing should be scanned with a guess that is
    /// about to be replaced) or when no scanner is selected.
    pub fn choices(&self) -> Option<Choices> {
        if self.details_id != self.settings.scanner || self.selected().is_none() {
            return None;
        }
        match &self.details {
            DetailsState::Ready(choices) => Some(choices.clone()),
            DetailsState::Failed(_) => Some(Choices::default()),
            DetailsState::Idle | DetailsState::Loading => None,
        }
    }

    /// The single most relevant state for the app to word, if any.
    pub fn feedback(&self) -> Option<Feedback> {
        if self.loading {
            return Some(Feedback::FindingScanners);
        }
        match &self.list_failure {
            Some(ListFailure::Backend(error)) => return Some(Feedback::DiscoveryFailed(error.clone())),
            Some(ListFailure::Stopped) => return Some(Feedback::WorkerStopped),
            None => {}
        }
        if self.requested && self.scanners.is_empty() {
            return Some(Feedback::NoScanners);
        }
        match &self.details {
            DetailsState::Loading => Some(Feedback::Checking),
            DetailsState::Failed(error) => Some(Feedback::DetailsFailed(error.clone())),
            DetailsState::Idle | DetailsState::Ready(_) => None,
        }
    }

    /// Start a listing. False while one is already running; no worker is started then.
    pub fn refresh(&mut self, discover: &Discover) -> bool {
        let Some(generation) = self.begin_list() else { return false };
        self.list_rx = Some((generation, discover()));
        true
    }

    /// Read the failed scanner again. False when nothing failed.
    pub fn retry(&mut self, load: &LoadChoices) -> bool {
        let Some(generation) = self.retry_details() else { return false };
        let id = self.details_id.clone();
        self.details_rx = Some((generation, id.clone(), load(&id)));
        true
    }

    /// Start reading the selected scanner when that has not happened yet.
    pub fn ensure_details(&mut self, load: &LoadChoices) {
        if self.selected().is_none() {
            return;
        }
        let id = self.settings.scanner.clone();
        if let Some(generation) = self.want_details(&id) {
            self.details_rx = Some((generation, id.clone(), load(&id)));
        }
    }

    /// Collect finished work. Requests occasional repaints while work is pending.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some((generation, receiver)) = &self.list_rx {
            let generation = *generation;
            match receiver.try_recv() {
                Ok(result) => self.finish_list(generation, result.map_err(ListFailure::Backend)),
                Err(TryRecvError::Disconnected) => self.finish_list(generation, Err(ListFailure::Stopped)),
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some((generation, id, receiver)) = &self.details_rx {
            let (generation, id) = (*generation, id.clone());
            match receiver.try_recv() {
                Ok(result) => self.finish_details(generation, &id, result),
                Err(TryRecvError::Disconnected) => self.finish_details(generation, &id, Err(String::new())),
                Err(TryRecvError::Empty) => {}
            }
        }
        self.repaint_if_pending(ctx);
    }

    /// Schedules another frame shortly while a listing or choices read is still waiting for its result.
    fn repaint_if_pending(&self, ctx: &egui::Context) {
        if self.list_rx.is_some() || self.details_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Starts a listing and returns its generation; `None` when one is already running.
    fn begin_list(&mut self) -> Option<u64> {
        if self.loading {
            return None;
        }
        self.generation = self.generation.saturating_add(1);
        self.loading = true;
        self.requested = true;
        self.list_failure = None;
        Some(self.generation)
    }

    /// A stale generation is ignored. On success the remembered selection is kept while still listed,
    /// otherwise the default rule applies; the old details are forgotten so the selection is read
    /// again. A failed listing keeps the previous list.
    fn finish_list(&mut self, generation: u64, result: Result<Vec<Scanner>, ListFailure>) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        self.list_rx = None;
        match result {
            Ok(list) => {
                self.list_failure = None;
                if !list.iter().any(|s| s.id == self.settings.scanner) {
                    self.settings.scanner = default_scanner_id(&list).unwrap_or_default();
                }
                self.scanners = list;
                self.details = DetailsState::Idle;
                self.details_id.clear();
                self.details_rx = None;
            }
            Err(failure) => self.list_failure = Some(failure),
        }
    }

    /// The generation when a load must start (new selection, or nothing loaded yet); `None` when
    /// `id` is empty or already loading, loaded or failed.
    fn want_details(&mut self, id: &str) -> Option<u64> {
        if id.is_empty() || (self.details_id == id && self.details != DetailsState::Idle) {
            return None;
        }
        self.details_id = id.to_string();
        self.details = DetailsState::Loading;
        self.details_generation = self.details_generation.saturating_add(1);
        self.details_rx = None;
        Some(self.details_generation)
    }

    /// Starts a new choices read for the selection after a failure and returns its generation; `None` when the last
    /// read did not fail.
    fn retry_details(&mut self) -> Option<u64> {
        if !matches!(self.details, DetailsState::Failed(_)) {
            return None;
        }
        self.details = DetailsState::Idle;
        let id = self.details_id.clone();
        self.want_details(&id)
    }

    /// Ignored when the generation or id no longer match the selection.
    fn finish_details(&mut self, generation: u64, id: &str, result: Result<Choices, String>) {
        if generation != self.details_generation || id != self.details_id {
            return;
        }
        self.details_rx = None;
        self.details = match result {
            Ok(choices) => {
                self.settings.clamp(&choices);
                DetailsState::Ready(choices)
            }
            Err(error) => {
                self.settings.clamp(&Choices::default());
                DetailsState::Failed(error)
            }
        };
    }
}

mod labels;
mod length;
mod widgets;

pub use labels::{Labels, Reasons};
pub use length::{LengthAvailability, LengthChoice, LengthLabels, LengthReason, PageLength};

#[cfg(test)]
mod tests;
