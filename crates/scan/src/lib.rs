//! Image acquisition from a scanner.
//!
//! A platform scanner backend (for example Windows WIA, which needs `unsafe` COM calls) is not in
//! this crate: it lives in the app and is plugged in through [`Backend`] and [`register`]. This crate
//! holds the shared types and rules and has no unsafe code. The backend's `scanners` connects and reads
//! properties; a device that fails to open is left out, and the others are still returned. It does
//! not feed paper. Its `acquire` does: it pulls sheets from the feeder, or reads the glass, and
//! returns each page as in-memory [`Page`] bytes (and optionally streams them through
//! [`ProgressToken`] while the transfer runs).
//!
//! **Buffers first.** The public surface never requires a path. Language bindings and the app
//! keep pages in memory, build a PDF from those bytes, or hand them to another consumer. Temp
//! files under the process temp directory are only an internal WIA scratch while a page is still
//! arriving; they are deleted before [`Page`] is published. Optional path helpers
//! ([`write_pages`], [`page_path`], [`listed_page_files`]) are sinks/sources for a CLI or a
//! binding that *chooses* to spill to disk — they are never called by `acquire`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

mod backend;
pub mod bmp;
pub mod bw;
mod contract;
pub mod density;
pub mod job;
pub mod length;
pub mod trace;

pub use backend::{
    AMBIGUOUS_DEVICE, Backend, BackendError, Listing, NO_BACKEND_PREFIX, REGISTRY_DOMAIN, RegisterError, Registry, UNKNOWN_BACKEND, acquire,
    backend_names, diagnostic_dump, global, page_length_support, register, scanner_details, scanner_list, scanners, status, supported, tag_support,
};
pub use contract::{
    Compression, ErrorContext, ImageFormat, ListDetails, NativeCodeExt, ParseWireError, Phase, Recovery, ScanErrorInfo, contract_json, recovery,
};
/// The vocabulary shared with the print side lives in craft-job; these are the same names and wire formats this
/// crate has always exported (`"<backend>:<native id>"`, the native error code).
pub use craft_job::{HRESULT_DOMAIN, NativeCode, namespaced_id, split_id};
pub use length::{
    InferOptions, Inference, Inferred, LengthRecord, LengthSource, PageLength, PageLengthSupport, Refusal, assess_sheet_end, infer_and_crop,
    infer_sheet_end, mils_of, rows_for_mils,
};

/// Where the page sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Source {
    /// Feeder when [`Scanner::feeder_ready`] is set, otherwise the bed. Resolved at acquire time
    /// from a fresh status read. Some drivers leave the ready bit on after the tray is empty.
    #[default]
    Auto,
    /// The automatic document feeder. Hardware duplex is a separate flag on
    /// [`ScanSettings::duplex`] (`FEEDER|DUPLEX`); leave that off for one side.
    Feeder,
    /// The flatbed glass. One page.
    Glass,
}

/// The pixels the driver is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    /// 8-bit grey. The usual input for a text page.
    #[default]
    Gray,
    /// 24-bit color.
    Rgb,
    /// 1-bit black and white.
    BlackAndWhite,
}

impl Source {
    /// The wire name: `auto`, `feeder` or `glass`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Feeder => "feeder",
            Self::Glass => "glass",
        }
    }

    /// Reads a source name; `glass` also accepts `flatbed` and `bed`. Case-sensitive; unknown text is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "feeder" => Some(Self::Feeder),
            "glass" | "flatbed" | "bed" => Some(Self::Glass),
            _ => None,
        }
    }

    /// Turn [`Self::Auto`] into feeder or glass. Never returns [`Self::Auto`].
    ///
    /// `feeder_ready` is `None` when the backend cannot say whether paper is loaded: the feeder is
    /// then tried first, and an empty feeder is learned from the failure ([`auto_fallback_to_glass`]).
    pub fn resolve(self, feeder_ready: Option<bool>, has_feeder: bool, has_glass: bool) -> Result<Self, ScanError> {
        match self {
            Self::Auto => {
                if feeder_ready != Some(false) && has_feeder {
                    Ok(Self::Feeder)
                } else if has_glass {
                    Ok(Self::Glass)
                } else if has_feeder {
                    Ok(Self::Feeder)
                } else {
                    Err(ScanError::Failed("no scanner source is available".into()))
                }
            }
            Self::Feeder | Self::Glass => Ok(self),
        }
    }

    /// Whether this is the document feeder (false for `Auto`).
    pub fn is_feeder(self) -> bool {
        matches!(self, Self::Feeder)
    }

    /// Whether this is the glass (false for `Auto`).
    pub fn is_glass(self) -> bool {
        matches!(self, Self::Glass)
    }
}

impl Color {
    /// The wire name: `gray`, `rgb` or `bw`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gray => "gray",
            Self::Rgb => "rgb",
            Self::BlackAndWhite => "bw",
        }
    }

    /// WIA data-type code. Threshold is `0`, grey is `2`, color is `3`.
    pub fn data_type(self) -> i32 {
        match self {
            Self::BlackAndWhite => 0,
            Self::Gray => 2,
            Self::Rgb => 3,
        }
    }

    /// Bits per pixel of the delivered image: 1, 8 or 24.
    pub fn depth(self) -> u32 {
        match self {
            Self::BlackAndWhite => 1,
            Self::Gray => 8,
            Self::Rgb => 24,
        }
    }

    /// The colour for a driver `WIA_IPA_DATATYPE` value (0, 2 or 3); `None` for any other value.
    pub fn from_data_type(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::BlackAndWhite),
            2 => Some(Self::Gray),
            3 => Some(Self::Rgb),
            _ => None,
        }
    }
}

/// How the page is turned. The values are the driver's rotation codes, not degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    Portrait,
    Landscape,
    UpsideDown,
    Left,
}

impl Rotation {
    /// The wire name: `portrait`, `landscape`, `180` or `270`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
            Self::UpsideDown => "180",
            Self::Left => "270",
        }
    }

    /// Reads a rotation name or angle (`portrait`/`0`, `landscape`/`90`, `180`/`upside-down`, `270`/`left`); otherwise `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "portrait" | "0" => Some(Self::Portrait),
            "landscape" | "90" => Some(Self::Landscape),
            "180" | "upside-down" => Some(Self::UpsideDown),
            "270" | "left" => Some(Self::Left),
            _ => None,
        }
    }

    /// The driver's rotation code (0 to 3), not an angle in degrees.
    pub fn driver(self) -> i32 {
        match self {
            Self::Portrait => 0,
            Self::Landscape => 1,
            Self::UpsideDown => 2,
            Self::Left => 3,
        }
    }

    /// The rotation for a driver code (0 to 3); `None` for any other value.
    pub fn from_driver(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Portrait),
            1 => Some(Self::Landscape),
            2 => Some(Self::UpsideDown),
            3 => Some(Self::Left),
            _ => None,
        }
    }
}

/// A WIA range: `min`, `max`, and `step`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueSpan {
    pub min: i32,
    pub max: i32,
    pub step: i32,
}

impl ValueSpan {
    /// Whether `value` lies in `min..=max` and is a whole number of steps above `min` (a `step` of 0 counts as 1).
    pub fn contains(self, value: i32) -> bool {
        if value < self.min || value > self.max {
            return false;
        }
        // In i64: a driver can publish any bounds, and `value - min` can overflow i32.
        let step = if self.step == 0 { 1 } else { i64::from(self.step) };
        (i64::from(value) - i64::from(self.min)) % step == 0
    }
}

/// Driver intent presets (`WIA_IPS_CUR_INTENT`). Fine controls still override after intent is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    None,
    Color,
    Gray,
    Text,
    BestPreview,
    MaximizeQuality,
    MinimizeSize,
}

impl Intent {
    /// The wire name, e.g. `preview` for [`Self::BestPreview`].
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Color => "color",
            Self::Gray => "gray",
            Self::Text => "text",
            Self::BestPreview => "preview",
            Self::MaximizeQuality => "quality",
            Self::MinimizeSize => "size",
        }
    }

    /// Reads an intent name, accepting the aliases `rgb`, `grey`, `grayscale`, `ocr`, `best-preview`, `max-quality` and
    /// `min-size`. Unknown text is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "color" | "rgb" => Some(Self::Color),
            "gray" | "grey" | "grayscale" => Some(Self::Gray),
            "text" | "ocr" => Some(Self::Text),
            "preview" | "best-preview" => Some(Self::BestPreview),
            "quality" | "max-quality" => Some(Self::MaximizeQuality),
            "size" | "min-size" => Some(Self::MinimizeSize),
            _ => None,
        }
    }

    /// The driver's `WIA_IPS_CUR_INTENT` bit value for this intent.
    pub fn driver(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Color => 1,
            Self::Gray => 2,
            Self::Text => 4,
            Self::BestPreview => 262_144,
            Self::MaximizeQuality => 131_072,
            Self::MinimizeSize => 65_536,
        }
    }

    /// The intent for a driver bit value; `None` for a combination or any value that is not a single intent.
    pub fn from_driver(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Color),
            2 => Some(Self::Gray),
            4 => Some(Self::Text),
            65_536 => Some(Self::MinimizeSize),
            131_072 => Some(Self::MaximizeQuality),
            262_144 => Some(Self::BestPreview),
            _ => None,
        }
    }
}

/// Named page size (`WIA_IPS_PAGE_SIZE`). A region still overrides extent after this is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    A4,
    Letter,
    Legal,
    Auto,
    Custom,
    BusinessCard,
}

impl PageSize {
    /// The wire name, e.g. `business-card`.
    pub fn name(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::Letter => "letter",
            Self::Legal => "legal",
            Self::Auto => "auto",
            Self::Custom => "custom",
            Self::BusinessCard => "business-card",
        }
    }

    /// Reads a page size name; `card` is accepted for `business-card`. Unknown text is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "a4" => Some(Self::A4),
            "letter" => Some(Self::Letter),
            "legal" => Some(Self::Legal),
            "auto" => Some(Self::Auto),
            "custom" => Some(Self::Custom),
            "business-card" | "card" => Some(Self::BusinessCard),
            _ => None,
        }
    }

    /// The driver's `WIA_IPS_PAGE_SIZE` code, which is not in the order of the variants.
    pub fn driver(self) -> i32 {
        match self {
            Self::A4 => 0,
            Self::Letter => 1,
            Self::Custom => 2,
            Self::Legal => 3,
            Self::BusinessCard => 6,
            Self::Auto => 100,
        }
    }

    /// The page size for a driver code; `None` for a code this crate does not name.
    pub fn from_driver(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::A4),
            1 => Some(Self::Letter),
            2 => Some(Self::Custom),
            3 => Some(Self::Legal),
            6 => Some(Self::BusinessCard),
            100 => Some(Self::Auto),
            _ => None,
        }
    }
}

/// How a scanner is attached. Derived from the driver's port string; the same hardware can appear
/// once per transport (USB minidriver vs inbox WSD) with different published capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Connection {
    Usb,
    /// Web Services for Devices / WS-Scan (network).
    Wsd,
    /// Some other attachment the backend knows: a network scanner found over HTTP (eSCL), a
    /// platform bridge, a virtual device. It says "not USB, not WSD" without guessing.
    Other,
    #[default]
    Unknown,
}

impl Connection {
    /// The wire name: `usb`, `wsd`, `other` or `unknown`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Wsd => "wsd",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }

    /// Classify a WIA port string (`WIA_DIP_PORT_NAME`) or PnP location.
    pub fn from_port(port: &str) -> Self {
        let lower = port.to_ascii_lowercase();
        if lower.contains("usbscan") || lower.starts_with(r"\\.\") {
            Self::Usb
        } else if lower.contains("webservices")
            || lower.contains("wsd")
            || lower.contains("scannerservice")
            || lower.starts_with("urn:uuid:")
            || lower.starts_with("http://")
            || lower.starts_with("https://")
        {
            Self::Wsd
        } else if lower.contains("usb") {
            // Broad USB match after WSD checks (a URL must not become USB).
            Self::Usb
        } else {
            Self::Unknown
        }
    }
}

/// What a backend can say about the paper path before a scan. Every flag is "set only when the
/// backend reported it": a `false` is not a promise that nothing is wrong, and a backend that cannot
/// preflight at all returns `None` from [`Backend::status`] instead of a status of `false`s.
///
/// The feeder is the one three-state flag, because it drives decisions: `Some(true)` paper is
/// loaded, `Some(false)` the backend knows the feeder is empty (an eSCL `AdfState` of empty), and
/// `None` it cannot tell. A backend whose conditions are only known after the fact (SANE) reports
/// them as errors instead ([`ScanError::Native`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HandlingStatus {
    /// Paper is loaded in the feeder; `None` when not reported.
    pub feeder_ready: Option<bool>,
    pub flat_ready: bool,
    pub duplex_ready: bool,
    pub cover_open: bool,
    pub path_cover_open: bool,
    pub jam: bool,
    /// Several sheets were fed together.
    pub multiple_feed: bool,
    /// The device asks for attention without saying why.
    pub attention: bool,
    /// The lamp or another device fault.
    pub lamp_error: bool,
}

impl HandlingStatus {
    /// The cause that should stop `acquire` before it starts, if any. The first match wins: a jam
    /// is reported ahead of a cover, a double feed, or a lamp fault.
    ///
    /// `attention` does not block: devices also raise it for a paused scanner or a lamp warning. It
    /// only explains a failure afterwards.
    pub fn acquire_block_kind(self) -> Option<InterruptKind> {
        if self.jam {
            Some(InterruptKind::Jam)
        } else if self.cover_open || self.path_cover_open {
            Some(InterruptKind::CoverOpen)
        } else if self.multiple_feed {
            Some(InterruptKind::MultiFeed)
        } else if self.lamp_error {
            Some(InterruptKind::DeviceError)
        } else {
            None
        }
    }

    /// The neutral sentence for [`Self::acquire_block_kind`].
    pub fn acquire_block_reason(self) -> Option<&'static str> {
        self.acquire_block_kind().map(InterruptKind::sentence)
    }
}

/// Whether a scan from `source` can start, given what the backend reported before it. `None` (the
/// backend cannot preflight) never blocks. A jam, cover, double feed or lamp fault blocks every
/// source; a feeder that is known to be empty blocks [`Source::Feeder`] only. The error carries
/// the kind, so [`auto_fallback_to_glass`] and the app's advice work from it.
pub fn preflight(source: Source, status: Option<HandlingStatus>) -> Result<(), ScanError> {
    let Some(status) = status else { return Ok(()) };
    if let Some(kind) = status.acquire_block_kind() {
        return Err(ScanError::Native { kind, native: None, message: kind.sentence().to_string() });
    }
    if source == Source::Feeder && status.feeder_ready == Some(false) {
        let kind = InterruptKind::FeederEmpty;
        return Err(ScanError::Native { kind, native: None, message: kind.sentence().to_string() });
    }
    Ok(())
}

/// Sentence for a feeder transfer that answered busy while nothing names a cause. An empty feeder
/// looks like this on the Windows WSD/WIA driver; other backends never produce
/// [`InterruptKind::BusyOrFeederEmpty`] and report [`InterruptKind::FeederEmpty`] instead.
pub const BUSY_OR_FEEDER_EMPTY: &str = "The scanner is busy, or the feeder has no paper.";

/// The code [`ProgressToken`] stores for a source: 0 unknown or auto, 1 feeder, 2 glass.
fn source_code(source: Source) -> u8 {
    match source {
        Source::Auto => 0,
        Source::Feeder => 1,
        Source::Glass => 2,
    }
}

/// The source for a stored code; `None` for 0 and for any value that is not a source.
fn source_from_code(code: u8) -> Option<Source> {
    match code {
        1 => Some(Source::Feeder),
        2 => Some(Source::Glass),
        _ => None,
    }
}

/// Pages a failed acquire still delivered (`Incomplete` carries them; every other error has none).
pub fn pages_in_error(error: &ScanError) -> usize {
    match error {
        ScanError::Incomplete { acquired, .. } => acquired.pages.len(),
        _ => 0,
    }
}

/// Whether a failed acquire should be retried once on the glass.
///
/// Only for Auto that resolved to the feeder, when the glass exists and no page arrived, and the
/// failure is the empty-feeder classification (`BusyOrFeederEmpty` or `FeederEmpty`). Drivers keep
/// the WIA ready bit set for an empty tray, so Auto cannot tell before it tries. A jam, cover,
/// double feed, lamp or attention error, a plain busy, and any shortfall after pages stay errors.
pub fn auto_fallback_to_glass(requested: Source, resolved: Source, has_glass: bool, pages_received: usize, error: &ScanError) -> bool {
    requested == Source::Auto
        && resolved == Source::Feeder
        && has_glass
        && pages_received == 0
        && matches!(error.interrupt_kind(), InterruptKind::BusyOrFeederEmpty | InterruptKind::FeederEmpty)
}

/// After a backend's transfer returns.
///
/// The progress token already keeps pages streamed through [`ProgressToken`] when this is `Err`.
/// A paper jam or cover fault is therefore still an error after a partial stack. The feeder
/// running out ([`InterruptKind::FeederEmpty`]) with pages in hand is a normal end. Cancel with no
/// pages is cancel.
pub fn after_transfer(page_count: usize, cancelled: bool, download_error: Option<ScanError>) -> Result<(), ScanError> {
    if cancelled && page_count == 0 {
        return Err(ScanError::Failed("the scan was cancelled".into()));
    }
    if let Some(error) = download_error {
        if page_count == 0 || !feeder_ended(&error) {
            return Err(clarify_mid_transfer(error, page_count));
        }
    } else if page_count == 0 {
        return Err(ScanError::Failed("the scanner transferred no pages".into()));
    }
    Ok(())
}

/// Sentence for a feeder that delivered fewer pages than the caller asked for and then said it
/// had no more paper.
pub fn feeder_shortfall_message(pages_requested: usize, pages_transferred: usize) -> String {
    format!("Only {pages_transferred} of {pages_requested} pages were scanned: the feeder reported no more paper.")
}

/// [`after_transfer`] plus the page-count check for a fixed-count feeder scan.
///
/// `pages_requested` is `0` for "the whole feeder" (a short stack is a normal end). When it is
/// `N > 0` on the feeder and `0 < pages_transferred < N`, a transfer that ended without a hard
/// error, or with the scanner saying "no more paper / empty", is reported as an error so the
/// caller (via [`ScanError::Incomplete`]) keeps the pages that did arrive but does not call the
/// scan a success. Jams and other faults keep their own sentence. A user cancel with pages in
/// hand stays a normal end. Pass `pages_requested = 0` for hardware duplex (sides are not sheets)
/// and `feeder = false` for the glass.
pub fn after_transfer_checked(
    pages_requested: usize,
    feeder: bool,
    pages_transferred: usize,
    cancelled: bool,
    download_error: Option<ScanError>,
) -> Result<(), ScanError> {
    let ended_empty = download_error.as_ref().is_none_or(feeder_ended);
    after_transfer(pages_transferred, cancelled, download_error)?;
    if feeder && !cancelled && ended_empty && pages_requested > 0 && pages_transferred > 0 && pages_transferred < pages_requested {
        return Err(ScanError::Native {
            kind: InterruptKind::FeederEmpty,
            native: None,
            message: feeder_shortfall_message(pages_requested, pages_transferred),
        });
    }
    Ok(())
}

/// A transfer that stopped with pages in hand because the device asked for attention reads like an
/// interrupted stack, not a preflight block. Everything else keeps its own sentence.
fn clarify_mid_transfer(error: ScanError, page_count: usize) -> ScanError {
    if page_count > 0 && error.interrupt_kind() == InterruptKind::Attention {
        return error
            .restated(InterruptKind::Other, "The scan was interrupted. Check for a paper jam, an open cover, or that the scanner stayed online.");
    }
    error
}

/// True when the transfer stopped because the feeder had no more sheets, not because of a jam.
fn feeder_ended(error: &ScanError) -> bool {
    matches!(error.interrupt_kind(), InterruptKind::FeederEmpty | InterruptKind::BusyOrFeederEmpty)
}

/// What a backend registers so [`CancelToken::request`] can stop a transfer from another thread.
pub type Canceller = Arc<dyn Fn() + Send + Sync>;

/// Cancel an in-flight `acquire`. The backend registers a [`Canceller`] while its transfer runs;
/// [`Self::request`] sets the flag and calls it.
#[derive(Clone, Default)]
pub struct CancelToken {
    requested: Arc<AtomicBool>,
    /// The running transfer's canceller. `None` when idle.
    canceller: Arc<Mutex<Option<Canceller>>>,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken").field("requested", &self.is_requested()).finish()
    }
}

impl PartialEq for CancelToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.requested, &other.requested)
    }
}

impl Eq for CancelToken {}

impl CancelToken {
    /// A token that has not been cancelled and has no canceller registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the token cancelled and, when a transfer has registered a canceller, calls it on this thread. Safe to call
    /// repeatedly and from any thread.
    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        // Clone out of the lock so a slow cancel never blocks the worker that clears it.
        let canceller = self.canceller.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if let Some(cancel) = canceller {
            cancel();
        }
    }

    /// Whether [`Self::request`] has been called on this token or a clone of it.
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// Register the canceller for the transfer that is about to run (backends only). When the token was already
    /// requested (the cancel came before the worker got here) the canceller is also called at once, on this thread
    /// and outside the lock, so an early cancel is never lost and no backend has to re-check the flag itself.
    pub fn set_canceller(&self, canceller: Canceller) {
        *self.canceller.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&canceller));
        if self.is_requested() {
            canceller();
        }
    }

    /// Forget the canceller when the transfer is over (backends only).
    pub fn clear_canceller(&self) {
        *self.canceller.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// Live page arrivals while `acquire` runs. Each finished page is a full in-memory [`Page`]
/// (no path). The dialog — or a binding — drains them with [`Self::take_pages`] as each sheet
/// finishes, so a jam or Stop still leaves what already transferred.
///
/// Banded and WIA 2.0 transfers also publish how far the **current** page has filled
/// ([`Self::page_percent`], [`Self::bytes_received`]) so a long single-page scan is not stuck at
/// "Scanning…" with a frozen bar.
#[derive(Clone, Default)]
pub struct ProgressToken {
    done: Arc<AtomicUsize>,
    pending: Arc<Mutex<Vec<Page>>>,
    /// 0 = unknown; 1..=101 means 0..=100% of the page in flight.
    page_percent: Arc<AtomicU32>,
    bytes_received: Arc<AtomicU64>,
    /// 0 = unknown expected size.
    bytes_expected: Arc<AtomicU64>,
    /// The source the engine is transferring from: 0 = not known yet, 1 = feeder, 2 = glass.
    source_used: Arc<AtomicU8>,
}

impl std::fmt::Debug for ProgressToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressToken")
            .field("pages_done", &self.pages_done())
            .field("page_percent", &self.page_percent())
            .field("bytes_received", &self.bytes_received())
            .field("bytes_expected", &self.bytes_expected())
            .finish()
    }
}

impl PartialEq for ProgressToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.done, &other.done)
    }
}

impl Eq for ProgressToken {}

impl ProgressToken {
    /// A token with no pages, no progress and no source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pages pushed so far, including any already taken with [`Self::take_pages`].
    pub fn pages_done(&self) -> usize {
        self.done.load(Ordering::SeqCst)
    }

    /// Driver-reported or byte-derived fill of the page currently transferring.
    pub fn page_percent(&self) -> Option<u8> {
        match self.page_percent.load(Ordering::SeqCst) {
            0 => None,
            n => Some(n.saturating_sub(1).min(100) as u8),
        }
    }

    /// Bytes of the page in flight that have arrived; 0 between pages.
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received.load(Ordering::SeqCst)
    }

    /// The size the page in flight is expected to reach, or `None` when unknown.
    pub fn bytes_expected(&self) -> Option<u64> {
        match self.bytes_expected.load(Ordering::SeqCst) {
            0 => None,
            n => Some(n),
        }
    }

    /// The source the engine is transferring from now (never [`Source::Auto`]). It changes from the
    /// feeder to the glass when Auto falls back, before the first page of the glass scan arrives.
    pub fn source(&self) -> Option<Source> {
        source_from_code(self.source_used.load(Ordering::SeqCst))
    }

    /// Records the source the transfer is using; [`Source::Auto`] is stored as unknown.
    pub fn set_source(&self, source: Source) {
        self.source_used.store(source_code(source), Ordering::SeqCst);
    }

    /// Pages that finished since the last take. Order is transfer order.
    pub fn take_pages(&self) -> Vec<Page> {
        self.pending.lock().map(|mut guard| std::mem::take(&mut *guard)).unwrap_or_default()
    }

    /// Begin (or refresh) the expected uncompressed size for the page about to transfer.
    pub fn set_expected_bytes(&self, expected: Option<u64>) {
        self.bytes_expected.store(expected.unwrap_or(0), Ordering::SeqCst);
        self.bytes_received.store(0, Ordering::SeqCst);
        self.page_percent.store(0, Ordering::SeqCst);
    }

    /// Update fill for the page in flight. `percent` is 0..=100 from the driver when known.
    ///
    /// When the driver percent is omitted (inbox WSD JFIF: `lPercentComplete` peaks near 40
    /// for the full stream, then snaps to 100) and no expected size is set, percent is derived
    /// from `received` with a soft ceiling so the bar climbs with the wire bytes and only
    /// claims 100 when the caller passes `Some(100)` (or a matching `expected`).
    pub fn set_transfer(&self, percent: Option<u8>, received: u64, expected: Option<u64>) {
        if let Some(exp) = expected {
            self.bytes_expected.store(exp, Ordering::SeqCst);
        } else if percent == Some(100) && received > 0 && self.bytes_expected.load(Ordering::SeqCst) == 0 {
            // Stream finished: lock expected to what actually arrived (compressed size).
            self.bytes_expected.store(received, Ordering::SeqCst);
        }
        self.bytes_received.store(received, Ordering::SeqCst);
        let pct = percent.or_else(|| {
            let exp = self.bytes_expected.load(Ordering::SeqCst);
            if let Some(done) = received.min(exp).saturating_mul(100).checked_div(exp) {
                Some(done as u8)
            } else if received > 0 {
                // Unknown total: asymptotic toward 99 (never claim done from bytes alone).
                let soft = received.saturating_add(64 * 1024).max(1);
                Some((received.saturating_mul(100) / soft).min(99) as u8)
            } else {
                None
            }
        });
        if let Some(p) = pct {
            self.page_percent.store(u32::from(p.min(100)) + 1, Ordering::SeqCst);
        }
    }

    /// Queues a finished page for [`Self::take_pages`] and resets the in-flight percentage and byte count for the next
    /// page.
    pub fn push_page(&self, page: Page) {
        self.done.fetch_add(1, Ordering::SeqCst);
        self.page_percent.store(0, Ordering::SeqCst);
        self.bytes_received.store(0, Ordering::SeqCst);
        // Keep bytes_expected: the next page is usually the same size.
        if let Ok(mut guard) = self.pending.lock() {
            guard.push(page);
        }
    }
}

/// What one source (the bed or the feeder) offers. Empty means the driver did not publish a list.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ImageChoices {
    pub dpi: Vec<u32>,
    /// Y resolutions when the driver lists them separately from [`Self::dpi`].
    pub dpi_y: Vec<u32>,
    pub colors: Vec<Color>,
    pub brightness: Option<ValueSpan>,
    pub contrast: Option<ValueSpan>,
    pub threshold: Option<ValueSpan>,
    pub rotation: Vec<Rotation>,
    /// Transfer formats the item lists (`bmp`, `jpeg`, `png`, `tiff`, …).
    pub formats: Vec<String>,
    pub compression: Vec<String>,
    pub intents: Vec<Intent>,
    pub page_sizes: Vec<PageSize>,
    /// True when `WIA_IPS_AUTO_DESKEW` is published.
    pub auto_deskew: bool,
    /// Orientation codes when distinct from rotation.
    pub orientation: Vec<i32>,
    pub photometric: Vec<i32>,
    pub invert: bool,
    pub mirror: bool,
    pub lamp: bool,
}

/// A WIA list attribute is `[count, nominal, values...]`.
pub fn listed_values(raw: &[i32]) -> Vec<i32> {
    let Some(count) = raw.first().copied().and_then(|count| usize::try_from(count).ok()) else {
        return Vec::new();
    };
    let end = count.saturating_add(2);
    if count == 0 || raw.len() < end {
        return Vec::new();
    }
    raw[2..end].to_vec()
}

/// A WIA range attribute is `[min, nominal, max, step]`.
pub fn spanned_values(raw: &[i32]) -> Option<ValueSpan> {
    if raw.len() < 4 {
        return None;
    }
    let step = if raw[3] == 0 { 1 } else { raw[3] };
    Some(ValueSpan { min: raw[0], max: raw[2], step })
}

/// What the driver says it can do. Flags are the WIA document-handling bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scanner {
    pub id: String,
    pub name: String,
    pub feeder: bool,
    pub flatbed: bool,
    pub duplex: bool,
    /// A sheet is waiting in the feeder. Independent of [`Self::feeder`]. Same as
    /// [`HandlingStatus::feeder_ready`].
    pub feeder_ready: Option<bool>,
    pub handling_status: HandlingStatus,
    /// Sheets the feeder holds, when the driver publishes capacity.
    pub feeder_capacity: Option<u32>,
    /// How the device is attached, from the driver's port string when present.
    pub connection: Connection,
    /// Driver port string (`\\\\.\\Usbscan0`, a WSD URL, …), when published.
    pub port: Option<String>,
    /// Optical resolution when the driver publishes a positive value. Missing on many WSD class drivers.
    pub optical_x: Option<u32>,
    pub optical_y: Option<u32>,
    /// How many sheets the feeder will accept. `0` in the range means the whole feeder. `None`
    /// when the driver does not publish a range.
    pub pages: Option<ValueSpan>,
    /// Bed width and height in thousandths of an inch, when the driver publishes them.
    pub bed: Option<(u32, u32)>,
    /// Sheet-feeder width and height in thousandths of an inch.
    pub sheet: Option<(u32, u32)>,
    pub glass: Option<ImageChoices>,
    pub feeder_choices: Option<ImageChoices>,
    /// What the driver actually answered. `Reported::default()` (nothing answered) makes every plain
    /// `bool` and empty list above read as "unknown" through the `*_presence` methods.
    pub reported: Reported,
}

/// Whether a device has something. A plain `false` or empty list cannot say which of the last two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Presence {
    /// The driver did not say (or the read failed).
    #[default]
    Unknown,
    /// The driver answered and the device does not have it.
    Absent,
    /// The device has it.
    Present,
}

impl Presence {
    /// Every state, in the order `unknown`, `absent`, `present`.
    pub const ALL: &'static [Presence] = &[Presence::Unknown, Presence::Absent, Presence::Present];

    /// Wire string: `unknown`, `absent`, `present`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Absent => "absent",
            Self::Present => "present",
        }
    }
}

/// One list or range option of an item ([`ImageChoices`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChoiceField {
    Dpi,
    Colors,
    Brightness,
    Contrast,
    Threshold,
    Rotation,
    Formats,
    Compression,
    Intents,
    PageSizes,
}

/// What a backend could actually read, so a consumer can tell "the driver published nothing for
/// this" from "the driver was not asked or the read failed". All defaults mean "unknown".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Reported {
    /// The document-handling capability bits were read, so `feeder`, `flatbed` and `duplex` being
    /// `false` mean "absent" and not "not read". A `true` is always a positive answer.
    pub units: bool,
    /// Options of the glass item whose property storage was read and had no such property.
    pub glass_absent: Vec<ChoiceField>,
    /// The same for the feeder item.
    pub feeder_absent: Vec<ChoiceField>,
    /// Sheet-length detection and longest page of the glass item, as far as the backend read them.
    pub glass_length: PageLengthSupport,
    /// The same for the feeder item.
    pub feeder_length: PageLengthSupport,
}

impl ImageChoices {
    /// Whether this item lists any value for `field`.
    fn has(&self, field: ChoiceField) -> bool {
        match field {
            ChoiceField::Dpi => !self.dpi.is_empty(),
            ChoiceField::Colors => !self.colors.is_empty(),
            ChoiceField::Brightness => self.brightness.is_some(),
            ChoiceField::Contrast => self.contrast.is_some(),
            ChoiceField::Threshold => self.threshold.is_some(),
            ChoiceField::Rotation => !self.rotation.is_empty(),
            ChoiceField::Formats => !self.formats.is_empty(),
            ChoiceField::Compression => !self.compression.is_empty(),
            ChoiceField::Intents => !self.intents.is_empty(),
            ChoiceField::PageSizes => !self.page_sizes.is_empty(),
        }
    }
}

impl Scanner {
    /// `Present` when the unit exists, `Absent` when the capability bits were read without it, otherwise `Unknown`.
    fn unit_presence(&self, present: bool) -> Presence {
        match (present, self.reported.units) {
            (true, _) => Presence::Present,
            (false, true) => Presence::Absent,
            (false, false) => Presence::Unknown,
        }
    }

    /// Automatic document feeder: present, absent (the capability bits were read without it) or unknown.
    pub fn feeder_presence(&self) -> Presence {
        self.unit_presence(self.feeder)
    }

    /// Flatbed (glass): present, absent (the capability bits were read without it) or unknown.
    pub fn flatbed_presence(&self) -> Presence {
        self.unit_presence(self.flatbed)
    }

    /// Hardware duplex unit.
    pub fn duplex_presence(&self) -> Presence {
        self.unit_presence(self.duplex)
    }

    /// What the item for `source` publishes about page length. All unknown when the backend did not read
    /// it. `Source::Auto` asks the glass.
    pub fn length_support(&self, source: Source) -> PageLengthSupport {
        match source {
            Source::Feeder => self.reported.feeder_length,
            Source::Glass | Source::Auto => self.reported.glass_length,
        }
    }

    /// Whether the glass or feeder item lists `field`. `Present` when it has values; `Absent` when
    /// the backend recorded that the item had no such property (or the unit itself is absent);
    /// otherwise `Unknown`, including when the item was never read (`None`). `Source::Auto` asks
    /// the glass.
    pub fn choice_presence(&self, source: Source, field: ChoiceField) -> Presence {
        let (item, absent, unit) = match source {
            Source::Feeder => (&self.feeder_choices, &self.reported.feeder_absent, self.feeder_presence()),
            Source::Glass | Source::Auto => (&self.glass, &self.reported.glass_absent, self.flatbed_presence()),
        };
        if item.as_ref().is_some_and(|choices| choices.has(field)) {
            Presence::Present
        } else if unit == Presence::Absent || absent.contains(&field) {
            Presence::Absent
        } else {
            Presence::Unknown
        }
    }
}

/// A scanner as the device list shows it. Read without opening the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannerRef {
    pub id: String,
    pub name: String,
    /// Driver port string, when the device-info storage publishes it.
    pub port: Option<String>,
    pub connection: Connection,
}

impl ScannerRef {
    /// A reference whose `connection` is classified from `port` ([`Connection::Unknown`] without one).
    pub fn new(id: String, name: String, port: Option<String>) -> Self {
        let connection = port.as_deref().map(Connection::from_port).unwrap_or(Connection::Unknown);
        Self { id, name, port, connection }
    }

    /// Case-insensitive substring match against id, name or port. Empty text matches nothing.
    pub fn matches(&self, filter: &str) -> bool {
        let needle = filter.trim().to_lowercase();
        if needle.is_empty() {
            return false;
        }
        self.id.to_lowercase().contains(&needle)
            || self.name.to_lowercase().contains(&needle)
            || self.port.as_deref().is_some_and(|port| port.to_lowercase().contains(&needle))
    }
}

/// A request that pulls paper when `acquire` runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanSettings {
    /// Device id or a unique substring of the name. Empty uses the only scanner, or the first.
    pub device: String,
    pub source: Source,
    pub dpi: u32,
    /// Y resolution when it differs from [`Self::dpi`]. `None` writes the same value on both axes.
    pub dpi_y: Option<u32>,
    pub color: Color,
    /// How many sheets to pull. `0` means until the feeder is empty, and never more than
    /// [`MAX_PAGES`]. The bed is one page.
    pub pages: u32,
    /// Left alone when `None`, so the driver's current value stands.
    pub brightness: Option<i32>,
    pub contrast: Option<i32>,
    pub threshold: Option<i32>,
    pub rotation: Option<Rotation>,
    /// A window on the page, in thousandths of an inch. `None` keeps the driver's full area.
    pub region: Option<Region>,
    /// Transfer format name (`bmp`, `jpeg`, …). `None` leaves the driver's preferred format.
    pub format: Option<String>,
    /// Compression name (`none`, `jpeg`, `g4`, …). `None` leaves the driver alone.
    pub compression: Option<String>,
    /// Intent written before fine controls when the driver lists it.
    pub intent: Option<Intent>,
    /// Page-size preset. A [`Self::region`] still overrides the extent afterward.
    pub page_size: Option<PageSize>,
    /// How the page length is decided: [`PageLength::Driver`] (the default) leaves it to the driver, as
    /// before this field existed. See [`PageLength`] for `Auto` and `AutoInfer`.
    pub page_length: PageLength,
    /// Automatic deskew when the driver publishes the control.
    pub auto_deskew: Option<bool>,
    /// Orientation code from the driver's list (`WIA_IPS_ORIENTATION`), when distinct from rotation.
    pub orientation: Option<i32>,
    /// Photometric interpretation code when the driver lists values.
    pub photometric: Option<i32>,
    pub invert: Option<bool>,
    pub mirror: Option<bool>,
    /// Lamp on/off when `WIA_IPS_LAMP` is published.
    pub lamp: Option<bool>,
    /// Request `FEEDER|DUPLEX` when the device reports duplex. Off by default.
    pub duplex: bool,
    /// Optional cancel token. The dialog and tests call [`CancelToken::request`].
    pub cancel: Option<CancelToken>,
    /// Optional progress token. Each finished page is pushed as the driver hands it over.
    pub progress: Option<ProgressToken>,
}

/// Origin and size on the bed or the sheet, in thousandths of an inch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

/// Where that window falls in pixels at one resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionPixels {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The rectangle `acquire` will ask for. A full page leaves the driver's origin alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlannedFrame {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Write [`Self::x`] and [`Self::y`] as the origin. A full page does not.
    pub move_origin: bool,
}

/// One transferred page, held entirely in memory.
///
/// Bindings and the app consume [`Self::bytes`] directly (create a PDF, OCR, write elsewhere).
/// There is no associated path on disk unless a caller separately uses [`write_pages`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub bytes: Vec<u8>,
    /// `bmp`, `png`, `jpeg`, or `tiff`, from the file header.
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    /// Resolution the DELIVERED image has, horizontally, in dots per inch: read from the file (BMP pels per metre,
    /// PNG `pHYs`, JFIF/EXIF density, TIFF `XResolution`), see [`density`]. Falls back to [`Self::requested_dpi`]
    /// only when the file carries nothing usable (no density, unit "none", zero or absurd values). The PDF page
    /// size of a scan is `width / dpi * 72`, so this is the number to size a page with. It differs from the request
    /// when the device caps the image size (a 300 dpi request answered at 150) or snaps to a supported step.
    pub dpi: u32,
    /// The delivered resolution vertically (equal to [`Self::dpi`] unless the device sends non-square pixels).
    pub dpi_y: u32,
    /// The dpi the job asked for (`ScanSettings::dpi`), kept so callers can tell when the device delivered less.
    pub requested_dpi: u32,
    /// A black-and-white request got deeper data from the driver, so this page was thresholded in
    /// software to a real 1-bit BMP (see [`bw`]). `false` for pages delivered as they came.
    pub thresholded: bool,
    /// How this page's length was obtained and what it is: [`LengthSource::Driver`] unless the backend sized
    /// the page (see [`PageLength`]). An [`LengthSource::Inferred`] page is cropped from the delivered image
    /// and its length is approximate.
    pub length: LengthRecord,
}

/// Pages from `acquire`, with the source that was actually used (never [`Source::Auto`]).
///
/// Always in-memory. Spilling to files is the caller's choice ([`write_pages`]), not part of acquire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acquired {
    pub source: Source,
    pub pages: Vec<Page>,
}

pub const MAX_PAGES: u32 = 200;
pub const MIN_DPI: u32 = 50;
pub const MAX_DPI: u32 = 1200;

impl Default for ScanSettings {
    /// 200 dpi grey, one page from `Auto`, no device name, no region or other optional setting, and the driver's page length.
    fn default() -> Self {
        Self {
            device: String::new(),
            source: Source::Auto,
            dpi: 200,
            dpi_y: None,
            color: Color::Gray,
            pages: 1,
            brightness: None,
            contrast: None,
            threshold: None,
            rotation: None,
            region: None,
            format: None,
            compression: None,
            intent: None,
            page_size: None,
            page_length: PageLength::Driver,
            auto_deskew: None,
            orientation: None,
            photometric: None,
            invert: None,
            mirror: None,
            lamp: None,
            duplex: false,
            cancel: None,
            progress: None,
        }
    }
}

/// Why a scan stopped, as the backend decided it, kept on an [`ScanError::Incomplete`] beside the
/// pages that did arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cause {
    pub kind: InterruptKind,
    pub native: Option<NativeCode>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// No backend can do this. The text is kept for compatibility with the Windows-only releases.
    #[error("scanning is only supported on Windows (WIA) in this version")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
    /// Pages were received before the transfer ended with an error (jam, offline, incomplete stack).
    /// `cause` is what the backend decided stopped it, when it said.
    #[error("{message}")]
    Incomplete { message: String, acquired: Acquired, cause: Option<Cause> },
    /// A failure the backend classified. It reads like [`Self::Failed`]; it also carries the cause
    /// the backend settled on and, when it has one, its own error code, so a caller need not parse
    /// the sentence. This is how a backend builds an error without the core knowing its codes.
    #[error("{message}")]
    Native { kind: InterruptKind, native: Option<NativeCode>, message: String },
}

impl PartialEq for ScanError {
    /// Errors are equal when they say the same thing: `Failed` and `Native` compare by message alone (a native code
    /// does not change the sentence), `Incomplete` by its message, and `Unsupported` only to itself.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unsupported, Self::Unsupported) => true,
            (Self::Incomplete { message: a, .. }, Self::Incomplete { message: b, .. }) => a == b,
            // A code does not change what an error says; `Failed` and `Native` with one sentence are equal.
            (a, b) => match (a.plain_message(), b.plain_message()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }
}

impl Eq for ScanError {}

impl ScanError {
    /// The message of a `Failed` or `Native` error; `None` for the other variants.
    fn plain_message(&self) -> Option<&str> {
        match self {
            Self::Failed(message) | Self::Native { message, .. } => Some(message),
            Self::Unsupported | Self::Incomplete { .. } => None,
        }
    }

    /// An error a backend classified, with its own code.
    pub fn native_error(kind: InterruptKind, native: NativeCode, message: impl Into<String>) -> Self {
        Self::Native { kind, native: Some(native), message: message.into() }
    }

    /// The cause and native code behind this error, if the backend gave them.
    pub fn cause(&self) -> Option<Cause> {
        match self {
            Self::Native { kind, native, .. } => Some(Cause { kind: *kind, native: *native }),
            Self::Incomplete { cause, .. } => *cause,
            Self::Unsupported | Self::Failed(_) => None,
        }
    }

    /// The backend's own error code, when it gave one.
    pub fn native(&self) -> Option<NativeCode> {
        self.cause().and_then(|cause| cause.native)
    }

    /// The same failure restated as `message` by a rule of the app or backend (a status read named
    /// the fault). The native code stays; the kind is the one given.
    pub fn restated(&self, kind: InterruptKind, message: &str) -> Self {
        Self::Native { kind, native: self.native(), message: message.to_string() }
    }
}

/// What stopped a scan, for the instruction a caller shows ("close the cover", "clear the jam", ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InterruptKind {
    /// A scanner or ADF cover is open.
    CoverOpen,
    /// The device said "paper jam" outright.
    Jam,
    /// A catch-all paper fault: a jam or several sheets fed together. The cause is not known,
    /// so the instruction covers both.
    PaperProblem,
    /// The feeder ran out of sheets, including a fixed-count shortfall. A backend that can tell an
    /// empty feeder (SANE `NO_DOCS`, eSCL `AdfEmpty`) reports this.
    FeederEmpty,
    /// The feeder took more than one sheet.
    MultiFeed,
    /// Offline or unreachable.
    Offline,
    /// The scanner asks for attention without saying why (a door, a full store, a panel message).
    Attention,
    /// Busy, locked, or warming up: waiting is the cure.
    Busy,
    /// A feeder transfer answered busy and nothing names a cause, so an empty feeder cannot be
    /// ruled out. This is a quirk of the Windows WSD/WIA driver; other backends never produce it.
    BusyOrFeederEmpty,
    /// A lamp or device fault that a power cycle may clear.
    DeviceError,
    /// The device ended the scan itself (the Cancel key on its panel, the job dropped on its side). A
    /// backend that can tell reports this; `ScanJob` ends `CancelledByDevice`. Not a fault.
    Cancelled,
    /// Anything the backend did not name.
    Other,
}

impl InterruptKind {
    /// The neutral sentence for this kind, in the words the app shows. A backend may use it as the
    /// message of an error it classified, or word its own.
    pub fn sentence(self) -> &'static str {
        match self {
            Self::CoverOpen => "A cover on the scanner is open.",
            Self::Jam => "The scanner has a paper jam.",
            Self::PaperProblem => "The scanner has a paper problem.",
            Self::FeederEmpty => "The feeder has no paper.",
            Self::MultiFeed => "The feeder took more than one sheet.",
            Self::Offline => "The scanner is offline. Check that it is powered on and connected.",
            Self::Attention => "The scanner needs attention. Check it, then try again.",
            Self::Busy => "The scanner is busy. Try again in a moment.",
            Self::BusyOrFeederEmpty => BUSY_OR_FEEDER_EMPTY,
            Self::DeviceError => "The scanner reported a device fault. Turning it off and on may clear it.",
            Self::Cancelled => "The scan was cancelled at the scanner.",
            Self::Other => "The scan failed.",
        }
    }

    /// The opening of [`Self::sentence`] that [`interrupt_kind`] recognises, so a sentence with
    /// different advice after it still matches. `None` for [`Self::Other`].
    fn lead(self) -> Option<&'static str> {
        match self {
            Self::Offline => Some("The scanner is offline."),
            Self::Attention => Some("The scanner needs attention."),
            Self::Busy => Some("The scanner is busy."),
            Self::DeviceError => Some("The scanner reported a device fault."),
            Self::Other => None,
            other => Some(other.sentence()),
        }
    }
}

/// Classify an error sentence. **A documented fallback**: an error that carries its kind
/// ([`ScanError::Native`], [`ScanError::Incomplete`] with a cause) never goes through here. It
/// recognises only the fixed sentences of [`InterruptKind::sentence`] (matching the opening where
/// advice follows), [`feeder_shortfall_message`] and "the feeder is empty", so the answer is
/// deterministic; a sentence may carry a prefix (a device name) and still match.
pub fn interrupt_kind(message: &str) -> InterruptKind {
    if message.contains(BUSY_OR_FEEDER_EMPTY) {
        return InterruptKind::BusyOrFeederEmpty;
    }
    const ORDER: [InterruptKind; 10] = [
        InterruptKind::Jam,
        InterruptKind::PaperProblem,
        InterruptKind::CoverOpen,
        InterruptKind::MultiFeed,
        InterruptKind::FeederEmpty,
        InterruptKind::Offline,
        InterruptKind::Attention,
        InterruptKind::Busy,
        InterruptKind::DeviceError,
        InterruptKind::Cancelled,
    ];
    for kind in ORDER {
        if let Some(lead) = kind.lead()
            && message.contains(lead)
        {
            return kind;
        }
    }
    // Same text as [`feeder_shortfall_message`] around its two numbers, and the preflight sentence.
    if message.contains("the feeder reported no more paper.") || message.contains("the feeder is empty") {
        return InterruptKind::FeederEmpty;
    }
    InterruptKind::Other
}

impl ScanError {
    /// The cause. The backend's own decision when the error carries one; for an error with none
    /// behind it, [`interrupt_kind`] of its sentence.
    pub fn interrupt_kind(&self) -> InterruptKind {
        match self.cause() {
            Some(cause) => cause.kind,
            None => interrupt_kind(&self.to_string()),
        }
    }
}

impl ScanSettings {
    /// `self` when the resolution, page count and region are within the crate's limits (`MIN_DPI`..=`MAX_DPI`,
    /// `MAX_PAGES`, a region with a size); otherwise `Failed` naming the first one that is not. Nothing is sent to a
    /// scanner.
    pub fn checked(&self) -> Result<&Self, ScanError> {
        if !(MIN_DPI..=MAX_DPI).contains(&self.dpi) {
            return Err(ScanError::Failed(format!("dpi is {MIN_DPI} to {MAX_DPI}")));
        }
        if let Some(dpi_y) = self.dpi_y
            && !(MIN_DPI..=MAX_DPI).contains(&dpi_y)
        {
            return Err(ScanError::Failed(format!("dpi-y is {MIN_DPI} to {MAX_DPI}")));
        }
        if self.pages > MAX_PAGES {
            return Err(ScanError::Failed(format!("pages is at most {MAX_PAGES}, or 0 for the whole feeder")));
        }
        if let Some(region) = self.region
            && (region.width == 0 || region.height == 0)
        {
            return Err(ScanError::Failed("the scan region has no size".into()));
        }
        Ok(self)
    }
}

/// Pixel extent for a physical size in thousandths of an inch.
pub fn pixels_for(thousandths_of_an_inch: u32, dpi: u32) -> u32 {
    let px = u64::from(thousandths_of_an_inch) * u64::from(dpi) / 1000;
    u32::try_from(px).unwrap_or(u32::MAX)
}

/// Uncompressed BMP size for a page: each row padded to 4 bytes, plus a 54-byte header.
pub fn estimated_bmp_bytes(width: u32, height: u32, depth: u32) -> Option<u64> {
    if depth == 0 || depth > 32 {
        return None;
    }
    let stride = u64::from(width).checked_mul(u64::from(depth))?.div_ceil(32) * 4;
    stride.checked_mul(u64::from(height))?.checked_add(54)
}

/// Keep the same physical area when the resolution changes.
pub fn rescale_extent(extent: u32, from_dpi: u32, to_dpi: u32) -> u32 {
    if from_dpi == 0 {
        return extent;
    }
    let px = u64::from(extent) * u64::from(to_dpi) / u64::from(from_dpi);
    u32::try_from(px).unwrap_or(u32::MAX).max(1)
}

/// Width and height of a BMP, if `bytes` is one (see [`bmp::parse`] for what is accepted).
pub fn bmp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    bmp::size(bytes)
}

/// Width and height of an encoded page: a BMP is read here, JPEG, PNG and TIFF by their headers.
/// Backends other than WIA deliver the latter, so nothing may assume a BMP.
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if let Some(size) = bmp_size(bytes) {
        return Some(size);
    }
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// The container named by the first bytes of `bytes`: `png`, `jpeg`, `tiff`, `bmp`, or `bin` when unrecognised.
/// Only the signature is checked, not that the rest is valid.
pub fn image_format(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "jpeg"
    } else if bytes.starts_with(b"II") || bytes.starts_with(b"MM") {
        "tiff"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else {
        "bin"
    }
}

/// The file written for page number `index` (1-based) in `folder`: `page-001.<format>`.
pub fn page_path(folder: &Path, index: usize, format: &str) -> PathBuf {
    folder.join(format!("page-{index:03}.{format}"))
}

/// Optional disk sink: write each [`Page`]'s bytes as `page-001.{format}`, …
///
/// Not used by `acquire`. For a CLI `--out` directory or a binding that prefers files; in-memory
/// consumers skip this and keep [`Page::bytes`]. Refuses to overwrite an existing `page-*` file.
pub fn write_pages(folder: &Path, pages: &[Page]) -> Result<Vec<PathBuf>, ScanError> {
    use std::io::Write;
    std::fs::create_dir_all(folder).map_err(|error| ScanError::Failed(format!("{}: {error}", folder.display())))?;
    let mut paths = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let path = page_path(folder, index + 1, page.format);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| ScanError::Failed(format!("{}: {error}", path.display())))?;
        file.write_all(&page.bytes).map_err(|error| ScanError::Failed(format!("{}: {error}", path.display())))?;
        paths.push(path);
    }
    Ok(paths)
}

/// `page-001.bmp` (and png/jpeg/tiff/bin) already in `folder`, sorted by index.
pub fn listed_page_files(folder: &Path) -> Result<Vec<PathBuf>, ScanError> {
    let entries = std::fs::read_dir(folder).map_err(|error| ScanError::Failed(format!("{}: {error}", folder.display())))?;
    let mut pages = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ScanError::Failed(format!("{}: {error}", folder.display())))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(index) = stem.strip_prefix("page-").and_then(|rest| rest.parse::<usize>().ok()) else {
            continue;
        };
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if !matches!(ext, "bmp" | "png" | "jpeg" | "jpg" | "tiff" | "tif" | "bin") {
            continue;
        }
        pages.push((index, path));
    }
    pages.sort_by_key(|(index, _)| *index);
    for (expected, (index, _)) in pages.iter().enumerate() {
        if *index != expected + 1 {
            return Err(ScanError::Failed(format!(
                "page files in {} must be page-001, page-002, … without gaps (found page-{index:03})",
                folder.display()
            )));
        }
    }
    Ok(pages.into_iter().map(|(_, path)| path).collect())
}

/// Manual duplex without a hardware duplex bit: `fronts` from the first pass in order; the stack
/// was flipped so the last sheet is on top; `backs_as_scanned` is the reverse-side pass (first
/// image is the back of the last front). Reverses the backs and returns front0, back0, ….
pub fn interleave_manual_duplex<T>(fronts: Vec<T>, backs_as_scanned: Vec<T>) -> Result<Vec<T>, ScanError> {
    if fronts.is_empty() {
        return Err(ScanError::Failed("manual duplex needs the front pages from the first pass".into()));
    }
    if backs_as_scanned.len() != fronts.len() {
        return Err(ScanError::Failed(format!(
            "manual duplex needs one back for each front ({} front(s), {} back(s))",
            fronts.len(),
            backs_as_scanned.len()
        )));
    }
    let mut backs = backs_as_scanned;
    backs.reverse();
    let mut out = Vec::with_capacity(fronts.len() * 2);
    for (front, back) in fronts.into_iter().zip(backs) {
        out.push(front);
        out.push(back);
    }
    Ok(out)
}

/// The smallest listed resolution, or 100 when the driver publishes none.
pub fn preview_dpi(listed: &[u32]) -> u32 {
    listed.iter().copied().filter(|dpi| *dpi > 0).min().unwrap_or(100)
}

/// Pixel origin and size of `region` at `dpi`. A zero side is an error.
pub fn region_pixels(region: Region, dpi: u32) -> Result<RegionPixels, ScanError> {
    if region.width == 0 || region.height == 0 || dpi == 0 {
        return Err(ScanError::Failed("the scan region has no size".into()));
    }
    let width = pixels_for(region.width, dpi);
    let height = pixels_for(region.height, dpi);
    if width == 0 || height == 0 {
        return Err(ScanError::Failed("the scan region has no size".into()));
    }
    Ok(RegionPixels { x: pixels_for(region.left, dpi), y: pixels_for(region.top, dpi), width, height })
}

/// Keep `region` inside `max` (thousandths of an inch). `None` leaves the request as it is.
pub fn fit_region(region: Region, max: Option<(u32, u32)>) -> Result<Region, ScanError> {
    if region.width == 0 || region.height == 0 {
        return Err(ScanError::Failed("the scan region has no size".into()));
    }
    let Some((max_w, max_h)) = max.filter(|(w, h)| *w > 0 && *h > 0) else {
        return Ok(region);
    };
    if region.left >= max_w || region.top >= max_h {
        return Err(ScanError::Failed("the scan region is outside the page".into()));
    }
    let width = region.width.min(max_w - region.left);
    let height = region.height.min(max_h - region.top);
    if width == 0 || height == 0 {
        return Err(ScanError::Failed("the scan region has no size".into()));
    }
    Ok(Region { left: region.left, top: region.top, width, height })
}

/// Snap `value` onto a published range. Extent stays positive when `must_be_positive` is set.
pub fn align_to_span(value: i32, span: ValueSpan, must_be_positive: bool) -> Result<i32, ScanError> {
    let step = if span.step <= 0 { 1 } else { span.step };
    if span.max < span.min {
        return Err(ScanError::Failed("the scanner published an empty range".into()));
    }
    let mut aligned = value.clamp(span.min, span.max);
    let offset = i64::from(aligned) - i64::from(span.min);
    let rem = offset.rem_euclid(i64::from(step));
    if rem != 0 {
        let down = i64::from(aligned) - rem;
        let next = if down >= i64::from(span.min) { down } else { down + i64::from(step) };
        aligned = i32::try_from(next).map_err(|_| ScanError::Failed("a scanner value does not fit".into()))?;
        if aligned > span.max {
            let back = i64::from(span.max) - i64::from(span.min);
            let snapped = i64::from(span.min) + back - back.rem_euclid(i64::from(step));
            aligned = i32::try_from(snapped).map_err(|_| ScanError::Failed("a scanner value does not fit".into()))?;
        }
    }
    if must_be_positive && aligned <= 0 {
        return Err(ScanError::Failed("the scan region has no size".into()));
    }
    if !span.contains(aligned) {
        return Err(ScanError::Failed(format!("a scanner value {value} is outside {} to {}, step {step}", span.min, span.max)));
    }
    Ok(aligned)
}

/// Full-page rescaling, or a region clamped to `max` and turned into pixels at `to_dpi`.
pub fn planned_frame(
    old_width: u32,
    old_height: u32,
    from_dpi: u32,
    to_dpi: u32,
    region: Option<Region>,
    max: Option<(u32, u32)>,
) -> Result<PlannedFrame, ScanError> {
    if let Some(region) = region {
        let fitted = fit_region(region, max)?;
        let px = region_pixels(fitted, to_dpi)?;
        return Ok(PlannedFrame { x: px.x, y: px.y, width: px.width, height: px.height, move_origin: true });
    }
    if old_width == 0 || old_height == 0 {
        return Err(ScanError::Failed("the scanner did not report a page size".into()));
    }
    Ok(PlannedFrame {
        x: 0,
        y: 0,
        width: rescale_extent(old_width, from_dpi, to_dpi),
        height: rescale_extent(old_height, from_dpi, to_dpi),
        move_origin: false,
    })
}

/// `pixels` at `dpi` in thousandths of an inch, rounded down; saturates at `u32::MAX`. `dpi` must not be 0.
fn thousandths_down(pixels: u32, dpi: u32) -> u32 {
    u32::try_from(u64::from(pixels) * 1000 / u64::from(dpi)).unwrap_or(u32::MAX)
}

/// `pixels` at `dpi` in thousandths of an inch, rounded up; saturates at `u32::MAX`. `dpi` must not be 0.
fn thousandths_up(pixels: u32, dpi: u32) -> u32 {
    u32::try_from((u64::from(pixels) * 1000).div_ceil(u64::from(dpi))).unwrap_or(u32::MAX)
}

/// Map a drag rectangle in preview pixels to thousandths of an inch.
///
/// The origin rounds down and the far edge rounds up, then one extra thousandth, so a later
/// [`pixels_for`] truncation still covers the selection. A zero-area drag is the full page.
pub fn region_from_preview(x0: u32, y0: u32, x1: u32, y1: u32, dpi: u32) -> Result<Option<Region>, ScanError> {
    if dpi == 0 {
        return Err(ScanError::Failed("the preview has no resolution".into()));
    }
    let (left_px, right_px) = (x0.min(x1), x0.max(x1));
    let (top_px, bottom_px) = (y0.min(y1), y0.max(y1));
    if left_px == right_px || top_px == bottom_px {
        return Ok(None);
    }
    let left = thousandths_down(left_px, dpi);
    let top = thousandths_down(top_px, dpi);
    let right = thousandths_up(right_px, dpi).saturating_add(1);
    let bottom = thousandths_up(bottom_px, dpi).saturating_add(1);
    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);
    if width == 0 || height == 0 {
        return Ok(None);
    }
    Ok(Some(Region { left, top, width, height }))
}

/// Inches as thousandths of an inch, rounded; `Failed` for a negative, non-finite or too large value.
fn thousandths_from_inches(inches: f64) -> Result<u32, ScanError> {
    if !inches.is_finite() || inches < 0.0 {
        return Err(ScanError::Failed("the scan region cannot be negative".into()));
    }
    let n = (inches * 1000.0).round();
    if n > f64::from(u32::MAX) {
        return Err(ScanError::Failed("the scan region is too large".into()));
    }
    Ok(n as u32)
}

/// `--left`, `--top`, `--width` and `--height` in inches. Width and height must be positive.
pub fn region_from_inches(left: f64, top: f64, width: f64, height: f64) -> Result<Region, ScanError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(ScanError::Failed("the scan region width and height must be greater than zero".into()));
    }
    let left = thousandths_from_inches(left)?;
    let top = thousandths_from_inches(top)?;
    let width = thousandths_from_inches(width)?;
    let height = thousandths_from_inches(height)?;
    if width == 0 || height == 0 {
        return Err(ScanError::Failed("the scan region width and height must be greater than zero".into()));
    }
    Ok(Region { left, top, width, height })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::trace::{TraceDest, trace_dest_from_str};
    use super::{
        CancelToken, Color, Connection, HandlingStatus, Intent, Page, PageSize, ProgressToken, Region, RegionPixels, Rotation, ScanError,
        ScanSettings, Source, ValueSpan, align_to_span, bmp_size, estimated_bmp_bytes, fit_region, image_format, interleave_manual_duplex,
        listed_values, page_path, pixels_for, planned_frame, preview_dpi, region_from_inches, region_from_preview, region_pixels, rescale_extent,
        spanned_values,
    };

    #[test]
    fn a_span_as_wide_as_i32_checks_its_step_without_overflow() {
        // A driver can publish any bounds; `value - min` no longer overflows i32.
        let wide = ValueSpan { min: i32::MIN, max: i32::MAX, step: 3 };
        // Offsets from i32::MIN: 0 is 2^31 (not a multiple of 3), 1 is 2^31 + 1 (one), i32::MAX is 2^32 - 1 (one).
        assert!(!wide.contains(0) && wide.contains(1));
        assert!(wide.contains(i32::MAX) && !wide.contains(i32::MAX - 1));
        let any = ValueSpan { min: -10, max: i32::MAX, step: 0 };
        assert!(any.contains(i32::MAX) && !any.contains(-11));
        assert_eq!(align_to_span(i32::MAX, wide, false).ok(), Some(i32::MAX));
    }

    #[test]
    fn scan_trace_env_values_match_stderr_or_path() {
        assert_eq!(trace_dest_from_str("1"), TraceDest::Stderr);
        assert_eq!(trace_dest_from_str("off"), TraceDest::Off);
        assert!(matches!(trace_dest_from_str("/tmp/trace.jsonl"), TraceDest::File(_)));
    }

    #[test]
    fn letter_width_at_200_dpi_matches_the_driver_extent() {
        assert_eq!(pixels_for(8500, 200), 1700);
        assert_eq!(rescale_extent(1700, 200, 300), 2550);
    }

    #[test]
    fn a_1200_dpi_color_letter_page_is_about_410_megabytes() {
        let width = pixels_for(8500, 1200);
        let height = pixels_for(11693, 1200);
        assert!(estimated_bmp_bytes(width, height, 24).is_some_and(|n| n > 256 * 1024 * 1024));
        assert!(estimated_bmp_bytes(width, height, 8).is_some_and(|n| n < 256 * 1024 * 1024));
        assert!(estimated_bmp_bytes(10, 10, 0).is_none());
    }

    #[test]
    fn color_rotation_intent_and_page_size_round_trip_through_the_driver_codes() {
        for color in [Color::BlackAndWhite, Color::Gray, Color::Rgb] {
            assert_eq!(Color::from_data_type(color.data_type()), Some(color));
        }
        assert_eq!((Color::Gray.name(), Color::Rgb.name(), Color::BlackAndWhite.name()), ("gray", "rgb", "bw"));
        assert_eq!((Color::Gray.depth(), Color::Rgb.depth(), Color::BlackAndWhite.depth()), (8, 24, 1));
        for turn in [Rotation::Portrait, Rotation::Landscape, Rotation::UpsideDown, Rotation::Left] {
            assert_eq!(Rotation::from_driver(turn.driver()), Some(turn));
            assert_eq!(Rotation::parse(turn.name()), Some(turn));
        }
        assert_eq!(Rotation::parse("90"), Some(Rotation::Landscape));
        assert_eq!(Rotation::parse("upside-down"), Some(Rotation::UpsideDown));
        for intent in [Intent::None, Intent::Color, Intent::Gray, Intent::Text, Intent::BestPreview, Intent::MaximizeQuality, Intent::MinimizeSize] {
            assert_eq!(Intent::from_driver(intent.driver()), Some(intent));
            assert_eq!(Intent::parse(intent.name()), Some(intent));
        }
        assert_eq!(Intent::parse("ocr"), Some(Intent::Text));
        assert_eq!(Intent::parse("min-size"), Some(Intent::MinimizeSize));
        for size in [PageSize::A4, PageSize::Letter, PageSize::Legal, PageSize::Auto, PageSize::Custom, PageSize::BusinessCard] {
            assert_eq!(PageSize::from_driver(size.driver()), Some(size));
            assert_eq!(PageSize::parse(size.name()), Some(size));
        }
        assert_eq!(PageSize::parse("card"), Some(PageSize::BusinessCard));
    }

    #[test]
    fn a_short_fixed_count_feeder_scan_is_incomplete_not_success() {
        let no_paper = || ScanError::Failed("The feeder has no paper.".into());
        let no_images = || ScanError::Failed("the feeder is empty".into());
        let jam = || ScanError::Failed("The scanner has a paper jam.".into());
        // 3 requested, 1 arrived, driver said no more paper / no images: incomplete, with the words.
        for end in [Some(no_paper()), Some(no_images()), None] {
            let text = super::after_transfer_checked(3, true, 1, false, end).unwrap_err().to_string();
            assert_eq!(text, "Only 1 of 3 pages were scanned: the feeder reported no more paper.");
            assert!(text.contains("feeder") && text.contains("paper"));
        }
        // Whole feeder (0 / "all"): a short stack is a normal end.
        assert!(super::after_transfer_checked(0, true, 1, false, Some(no_paper())).is_ok());
        assert!(super::after_transfer_checked(0, true, 5, false, None).is_ok());
        // Everything requested arrived (or more): success.
        assert!(super::after_transfer_checked(3, true, 3, false, Some(no_paper())).is_ok());
        assert!(super::after_transfer_checked(3, true, 3, false, None).is_ok());
        // Nothing arrived keeps today's errors.
        assert!(super::after_transfer_checked(3, true, 0, false, Some(no_paper())).unwrap_err().to_string().contains("no paper"));
        assert!(super::after_transfer_checked(3, true, 0, false, None).unwrap_err().to_string().contains("no pages"));
        // A jam keeps its own sentence; a user cancel with pages stays a normal end.
        assert!(super::after_transfer_checked(3, true, 1, false, Some(jam())).unwrap_err().to_string().contains("paper jam"));
        assert!(super::after_transfer_checked(3, true, 1, true, None).is_ok());
        // Glass (not the feeder) and the duplex opt-out (requested 0) are unaffected.
        assert!(super::after_transfer_checked(1, false, 1, false, None).is_ok());
        assert!(super::after_transfer_checked(3, false, 1, false, None).is_ok());
    }

    #[test]
    fn settings_reject_a_dpi_or_a_page_count_outside_the_range() {
        assert!(ScanSettings { dpi: 49, ..ScanSettings::default() }.checked().is_err());
        assert!(ScanSettings { dpi: 1201, ..ScanSettings::default() }.checked().is_err());
        assert!(ScanSettings { dpi_y: Some(49), ..ScanSettings::default() }.checked().is_err());
        assert!(ScanSettings { dpi_y: Some(300), ..ScanSettings::default() }.checked().is_ok());
        assert!(ScanSettings { pages: 201, ..ScanSettings::default() }.checked().is_err());
        assert!(ScanSettings { pages: 0, ..ScanSettings::default() }.checked().is_ok());
        assert_eq!(ScanSettings::default().source, Source::Auto);
        assert_eq!(ScanSettings::default().dpi, 200);
    }

    #[test]
    fn cancel_token_records_a_request_before_any_transfer() {
        let token = CancelToken::new();
        assert!(!token.is_requested());
        token.request();
        assert!(token.is_requested());
        let clone = token.clone();
        assert!(clone.is_requested());
        assert_eq!(token, clone);
        assert_ne!(token, CancelToken::new());
    }

    #[test]
    fn a_cancel_requested_before_the_canceller_registers_fires_when_it_registers() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use super::Canceller;
        let calls = Arc::new(AtomicUsize::new(0));
        let canceller = |calls: &Arc<AtomicUsize>| -> Canceller {
            let calls = Arc::clone(calls);
            Arc::new(move || {
                calls.fetch_add(1, Ordering::SeqCst);
            })
        };
        // The cancel arrives first: request() finds nothing to call, registering then calls it once.
        let token = CancelToken::new();
        token.request();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        token.set_canceller(canceller(&calls));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "an early cancel must reach the backend");
        // A later request still reaches the registered canceller.
        token.request();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // Registering on a token that was never cancelled calls nothing.
        let quiet = Arc::new(AtomicUsize::new(0));
        let fresh = CancelToken::new();
        fresh.set_canceller(canceller(&quiet));
        assert_eq!(quiet.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn progress_token_queues_finished_pages() {
        let token = ProgressToken::new();
        assert_eq!(token.pages_done(), 0);
        assert_eq!(token.page_percent(), None);
        token.set_expected_bytes(Some(1_000));
        token.set_transfer(None, 250, None);
        assert_eq!(token.page_percent(), Some(25));
        assert_eq!(token.bytes_received(), 250);
        assert_eq!(token.bytes_expected(), Some(1_000));
        token.set_transfer(Some(80), 800, None);
        assert_eq!(token.page_percent(), Some(80));
        token.push_page(Page {
            bytes: vec![1],
            format: "bmp",
            width: 1,
            height: 1,
            dpi: 200,
            dpi_y: 200,
            requested_dpi: 200,
            thresholded: false,
            length: Default::default(),
        });
        token.push_page(Page {
            bytes: vec![2],
            format: "bmp",
            width: 1,
            height: 1,
            dpi: 200,
            dpi_y: 200,
            requested_dpi: 200,
            thresholded: false,
            length: Default::default(),
        });
        assert_eq!(token.pages_done(), 2);
        assert_eq!(token.page_percent(), None, "a finished page clears in-page fill");
        let taken = token.take_pages();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].bytes, vec![1]);
        assert!(token.take_pages().is_empty());
        assert_eq!(token.pages_done(), 2);
        assert_ne!(token, ProgressToken::new());
    }

    #[test]
    fn progress_token_soft_percent_without_expected_size() {
        let token = ProgressToken::new();
        token.set_expected_bytes(None);
        token.set_transfer(None, 32_768, None);
        let mid = token.page_percent().expect("soft fill");
        assert!((1..100).contains(&mid), "mid={mid}");
        token.set_transfer(None, 316_180, None);
        let late = token.page_percent().expect("late fill");
        assert!(late >= mid && late < 100, "late={late} mid={mid}");
        token.set_transfer(Some(100), 316_180, None);
        assert_eq!(token.page_percent(), Some(100));
        assert_eq!(token.bytes_expected(), Some(316_180));
    }

    #[test]
    fn page_paths_and_image_formats_follow_the_file_header() {
        assert_eq!(page_path(Path::new("out"), 1, "bmp"), Path::new("out").join("page-001.bmp"));
        assert_eq!(page_path(Path::new("out"), 12, "jpeg"), Path::new("out").join("page-012.jpeg"));
        assert_eq!(image_format(b"\x89PNG\r\n\x1a\n"), "png");
        assert_eq!(image_format(&[0xff, 0xd8, 0xff]), "jpeg");
        assert_eq!(image_format(b"II*\0"), "tiff");
        assert_eq!(image_format(b"MM\0*"), "tiff");
        assert_eq!(image_format(b"BM"), "bmp");
        assert_eq!(image_format(b"????"), "bin");
    }

    #[test]
    fn write_pages_is_an_optional_sink_and_refuses_overwrite() {
        let dir = std::env::temp_dir().join(format!("craft-scan-write-pages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pages = [
            Page {
                bytes: b"BM-a".to_vec(),
                format: "bmp",
                width: 1,
                height: 1,
                dpi: 200,
                dpi_y: 200,
                requested_dpi: 200,
                thresholded: false,
                length: Default::default(),
            },
            Page {
                bytes: b"BM-b".to_vec(),
                format: "bmp",
                width: 1,
                height: 1,
                dpi: 200,
                dpi_y: 200,
                requested_dpi: 200,
                thresholded: false,
                length: Default::default(),
            },
        ];
        let paths = super::write_pages(&dir, &pages).expect("write");
        assert_eq!(paths.len(), 2);
        assert_eq!(std::fs::read(&paths[0]).expect("read"), b"BM-a");
        assert_eq!(std::fs::read(&paths[1]).expect("read"), b"BM-b");
        assert!(super::write_pages(&dir, &pages[..1]).is_err(), "existing page-001 must not be overwritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_list_attribute_drops_the_count_and_the_nominal_value() {
        assert_eq!(listed_values(&[7, 200, 100, 150, 200, 300, 400, 600, 1200]), vec![100, 150, 200, 300, 400, 600, 1200]);
        assert_eq!(listed_values(&[]), Vec::<i32>::new());
        assert_eq!(listed_values(&[0, 200]), Vec::<i32>::new());
        assert_eq!(listed_values(&[3, 200, 100]), Vec::<i32>::new());
        assert_eq!(spanned_values(&[-50, 0, 50, 1]), Some(ValueSpan { min: -50, max: 50, step: 1 }));
        assert_eq!(spanned_values(&[0, 1, 30, 0]), Some(ValueSpan { min: 0, max: 30, step: 1 }));
        assert_eq!(spanned_values(&[1, 2, 3]), None);
        let pages = ValueSpan { min: 0, max: 30, step: 1 };
        assert!(pages.contains(0) && pages.contains(30) && !pages.contains(31));
        let stepped = ValueSpan { min: 0, max: 10, step: 0 };
        assert!(stepped.contains(3), "a zero step means every integer in the range");
    }

    #[test]
    fn a_two_by_one_inch_region_at_1200_dpi_is_2400_by_1200() {
        let region = Region { left: 0, top: 0, width: 2000, height: 1000 };
        assert_eq!(region_pixels(region, 1200), Ok(RegionPixels { x: 0, y: 0, width: 2400, height: 1200 }));
        assert_eq!(region_from_inches(0.0, 0.5, 2.0, 1.0), Ok(Region { left: 0, top: 500, width: 2000, height: 1000 }));
        assert!(region_pixels(Region { left: 0, top: 500, width: 2000, height: 1000 }, 1200).is_ok_and(|px| px.width == 2400 && px.height == 1200));
    }

    #[test]
    fn a_full_page_leaves_the_rescaled_extent_and_a_zero_region_is_refused() {
        let full = planned_frame(1700, 2200, 200, 300, None, Some((8500, 11693)));
        assert!(full.is_ok_and(|frame| {
            !frame.move_origin && frame.width == rescale_extent(1700, 200, 300) && frame.height == rescale_extent(2200, 200, 300)
        }));
        let window = Region { left: 1000, top: 0, width: 2000, height: 1000 };
        assert!(
            planned_frame(1700, 2200, 200, 1200, Some(window), None)
                .is_ok_and(|frame| { frame.move_origin && frame.x == 1200 && frame.width == 2400 && frame.height == 1200 })
        );
        assert!(region_pixels(Region { left: 0, top: 0, width: 0, height: 1000 }, 1200).is_err());
        assert!(ScanSettings { region: Some(Region { left: 0, top: 0, width: 0, height: 1 }), ..ScanSettings::default() }.checked().is_err());
        assert!(region_from_inches(0.0, 0.0, 0.0, 1.0).is_err());
        assert!(region_from_inches(0.0, 0.0, -1.0, 1.0).is_err());
        assert_eq!(
            fit_region(Region { left: 8000, top: 0, width: 2000, height: 1000 }, Some((8500, 11693))),
            Ok(Region { left: 8000, top: 0, width: 500, height: 1000 })
        );
        assert_eq!(fit_region(window, None), Ok(window));
        assert!(fit_region(Region { left: 9000, top: 0, width: 100, height: 100 }, Some((8500, 11693))).is_err());
    }

    #[test]
    fn a_preview_rectangle_covers_the_same_area_at_1200_dpi() {
        assert!(region_from_preview(0, 0, 200, 100, 100).is_ok_and(|region| {
            region.is_some_and(|region| region_pixels(region, 1200).is_ok_and(|px| px.x == 0 && px.y == 0 && px.width >= 2400 && px.height >= 1200))
        }));
        assert!(region_from_preview(10, 10, 10, 40, 100).is_ok_and(|region| region.is_none()));
        let preview_px = 1u32;
        let start = preview_px * 1200 / 75;
        let end = (2_u32 * 1200).div_ceil(75);
        assert!(region_from_preview(1, 0, 2, 1, 75).is_ok_and(|region| {
            region.is_some_and(|region| region_pixels(region, 1200).is_ok_and(|px| px.x <= start && px.x + px.width >= end))
        }));
        assert_eq!(preview_dpi(&[]), 100);
        assert_eq!(preview_dpi(&[300, 100, 200]), 100);
        assert_eq!(align_to_span(20, ValueSpan { min: 0, max: 64, step: 8 }, true), Ok(16));
        assert!(align_to_span(0, ValueSpan { min: 0, max: 100, step: 1 }, true).is_err());
        assert!(region_from_preview(0, 0, 1, 1, 0).is_err());
    }

    #[test]
    fn a_bmp_header_has_a_size() {
        let mut bytes = vec![0u8; 54];
        bytes[0] = b'B';
        bytes[1] = b'M';
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&1700i32.to_le_bytes());
        bytes[22..26].copy_from_slice(&(-2338i32).to_le_bytes());
        bytes[26] = 1;
        bytes[28] = 24;
        assert_eq!(bmp_size(&bytes), Some((1700, 2338)));
        assert_eq!(bmp_size(b"BM"), None);
        // A cut-off header is not a BMP, and neither is one with no planes or a zero width.
        assert_eq!(bmp_size(&bytes[..53]), None);
        bytes[26] = 0;
        assert_eq!(bmp_size(&bytes), None);
    }

    #[test]
    fn manual_duplex_puts_the_first_flipped_back_with_the_last_front() {
        // Fronts F1,F2,F3 then flip the stack: backs arrive B3,B2,B1.
        let interleaved = interleave_manual_duplex(vec!["F1", "F2", "F3"], vec!["B3", "B2", "B1"]).expect("pair");
        assert_eq!(interleaved, vec!["F1", "B1", "F2", "B2", "F3", "B3"]);
        assert!(interleave_manual_duplex(vec!["F1"], vec!["B1", "B2"]).is_err());
        assert!(interleave_manual_duplex(Vec::<&str>::new(), Vec::new()).is_err());
    }

    #[test]
    fn connection_follows_the_port_string() {
        assert_eq!(Connection::from_port(r"\\.\Usbscan0"), Connection::Usb);
        assert_eq!(Connection::from_port("http://192.0.2.10:80/WebServices/Device"), Connection::Wsd);
        assert_eq!(Connection::from_port("WSD-43c6ea17"), Connection::Wsd);
        assert_eq!(
            Connection::from_port("urn:uuid:00000000-0000-4000-8000-0000000000aa/uri:00000000-0000-4000-8000-0000000000aa/ScannerService"),
            Connection::Wsd
        );
        assert_eq!(Connection::from_port(""), Connection::Unknown);
    }

    #[test]
    fn scanner_ref_derives_connection_from_port() {
        let wsd = super::ScannerRef::new("{a}".into(), "Fake".into(), Some("http://127.0.0.1:3920/x".into()));
        assert_eq!(wsd.connection, Connection::Wsd);
        let usb = super::ScannerRef::new("{b}".into(), "U".into(), Some(r"\.\Usbscan0".into()));
        assert_eq!(usb.connection, Connection::Usb);
        assert_eq!(super::ScannerRef::new("{c}".into(), "N".into(), None).connection, Connection::Unknown);
    }

    #[test]
    fn scanner_ref_filter_matches_id_name_or_port_case_insensitively() {
        let item = super::ScannerRef::new("{ABC-123}".into(), "My Scanner".into(), Some("urn:uuid:0000F2/ScannerService".into()));
        assert!(item.matches("abc-123"));
        assert!(item.matches("scanner"));
        assert!(item.matches("0000f2"));
        assert!(!item.matches("other"));
        assert!(!item.matches(""));
        assert!(!item.matches("   "));
    }

    #[test]
    fn listed_page_files_sort_and_refuse_gaps() {
        let dir = std::env::temp_dir().join(format!("craft-scan-listed-pages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp");
        std::fs::write(dir.join("page-002.bmp"), b"BM").expect("write");
        std::fs::write(dir.join("page-001.bmp"), b"BM").expect("write");
        std::fs::write(dir.join("notes.txt"), b"x").expect("write");
        let listed = super::listed_page_files(&dir).expect("pages");
        assert_eq!(listed, vec![dir.join("page-001.bmp"), dir.join("page-002.bmp")]);
        std::fs::write(dir.join("page-004.bmp"), b"BM").expect("write");
        let gap = super::listed_page_files(&dir).expect_err("gap");
        assert!(gap.to_string().contains("without gaps"), "{gap}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_token_reports_the_source_in_use() {
        let token = ProgressToken::new();
        assert_eq!(token.source(), None);
        token.set_source(Source::Feeder);
        assert_eq!(token.source(), Some(Source::Feeder));
        // A clone shares it, as the dialog's copy does; the fallback flips it to the glass.
        let seen = token.clone();
        token.set_source(Source::Glass);
        assert_eq!(seen.source(), Some(Source::Glass));
        token.set_source(Source::Auto);
        assert_eq!(seen.source(), None);
    }

    #[test]
    fn auto_picks_the_feeder_unless_it_is_known_to_be_empty() {
        assert_eq!(Source::Auto.resolve(Some(true), true, true), Ok(Source::Feeder));
        assert_eq!(Source::Auto.resolve(Some(false), true, true), Ok(Source::Glass));
        // Unknown: the backend cannot preflight, so the feeder is tried and the failure decides.
        assert_eq!(Source::Auto.resolve(None, true, true), Ok(Source::Feeder));
        assert_eq!(Source::Auto.resolve(Some(true), false, true), Ok(Source::Glass));
        assert_eq!(Source::Auto.resolve(Some(false), true, false), Ok(Source::Feeder));
        assert_eq!(Source::Auto.resolve(None, false, false), Err(ScanError::Failed("no scanner source is available".into())));
        assert_eq!(Source::Feeder.resolve(Some(false), false, false), Ok(Source::Feeder));
    }

    #[test]
    fn preflight_blocks_only_on_what_the_backend_reported() {
        use super::{InterruptKind as K, preflight};
        // A backend that cannot preflight never blocks.
        assert!(preflight(Source::Feeder, None).is_ok());
        assert!(preflight(Source::Feeder, Some(HandlingStatus::default())).is_ok());
        let kind = |status: HandlingStatus, source| preflight(source, Some(status)).err().map(|e| e.interrupt_kind());
        let empty = HandlingStatus { feeder_ready: Some(false), ..HandlingStatus::default() };
        assert_eq!(kind(empty, Source::Feeder), Some(K::FeederEmpty));
        assert_eq!(kind(empty, Source::Glass), None);
        assert_eq!(kind(HandlingStatus { jam: true, feeder_ready: Some(true), ..HandlingStatus::default() }, Source::Glass), Some(K::Jam));
        assert_eq!(kind(HandlingStatus { path_cover_open: true, ..HandlingStatus::default() }, Source::Feeder), Some(K::CoverOpen));
        assert_eq!(kind(HandlingStatus { multiple_feed: true, jam: true, ..HandlingStatus::default() }, Source::Feeder), Some(K::Jam));
        assert_eq!(kind(HandlingStatus { lamp_error: true, ..HandlingStatus::default() }, Source::Glass), Some(K::DeviceError));
        // Attention alone never blocks.
        assert_eq!(kind(HandlingStatus { attention: true, ..HandlingStatus::default() }, Source::Feeder), None);
    }

    #[test]
    fn a_feeder_that_ran_out_with_pages_in_hand_is_a_normal_end_and_a_jam_is_not() {
        use super::{InterruptKind as K, NativeCode, after_transfer};
        let empty = || ScanError::native_error(K::FeederEmpty, NativeCode::new("sane", 6, Some("SANE_STATUS_NO_DOCS")), "no more documents");
        let jam = || ScanError::native_error(K::Jam, NativeCode::new("sane", 7, None), "jammed");
        assert!(after_transfer(2, false, Some(empty())).is_ok());
        assert!(after_transfer(3, false, Some(ScanError::Failed("the feeder is empty".into()))).is_ok());
        assert_eq!(after_transfer(0, false, Some(empty())).map_err(|e| e.interrupt_kind()), Err(K::FeederEmpty));
        assert_eq!(after_transfer(2, false, Some(jam())).map_err(|e| e.interrupt_kind()), Err(K::Jam));
        assert!(after_transfer(0, true, None).unwrap_err().to_string().contains("cancelled"));
        assert!(after_transfer(1, true, None).is_ok());
        assert!(after_transfer(0, false, None).unwrap_err().to_string().contains("no pages"));
        // The native code survives a rewrite of the sentence.
        let attention = ScanError::native_error(K::Attention, NativeCode::new("x", 1, None), K::Attention.sentence());
        let mid = after_transfer(1, false, Some(attention)).unwrap_err();
        assert!(mid.to_string().contains("interrupted"));
        assert_eq!(mid.native().map(|n| n.domain), Some("x"));
    }

    #[test]
    fn a_short_fixed_count_feeder_scan_is_incomplete_whatever_the_backend() {
        use super::{InterruptKind as K, NativeCode, after_transfer_checked};
        let no_docs = || ScanError::native_error(K::FeederEmpty, NativeCode::new("sane", 6, None), "no docs");
        for end in [Some(no_docs()), Some(ScanError::Failed("the feeder is empty".into())), None] {
            let error = after_transfer_checked(3, true, 1, false, end).unwrap_err();
            assert_eq!(error.to_string(), "Only 1 of 3 pages were scanned: the feeder reported no more paper.");
            assert_eq!(error.interrupt_kind(), K::FeederEmpty);
        }
        assert!(after_transfer_checked(0, true, 1, false, Some(no_docs())).is_ok());
        assert!(after_transfer_checked(3, true, 3, false, Some(no_docs())).is_ok());
        assert!(after_transfer_checked(3, false, 1, false, None).is_ok());
        let jam = ScanError::native_error(K::Jam, NativeCode::new("sane", 7, None), "jammed");
        assert_eq!(after_transfer_checked(3, true, 1, false, Some(jam)).map_err(|e| e.interrupt_kind()), Err(K::Jam));
        // The windows-only quirk kind also ends a whole-feeder stack normally.
        let busy_empty = ScanError::Failed(super::BUSY_OR_FEEDER_EMPTY.into());
        assert!(after_transfer_checked(0, true, 2, false, Some(busy_empty)).is_ok());
    }

    #[test]
    fn auto_falls_back_to_the_glass_only_for_an_empty_feeder_with_no_pages() {
        use super::{Acquired, BUSY_OR_FEEDER_EMPTY, InterruptKind as K, NativeCode, auto_fallback_to_glass as fallback};
        let (auto, feeder, glass) = (Source::Auto, Source::Feeder, Source::Glass);
        // FeederEmpty alone is enough: no backend needs the Windows-only BusyOrFeederEmpty.
        let empty = ScanError::native_error(K::FeederEmpty, NativeCode::new("sane", 6, Some("SANE_STATUS_NO_DOCS")), "no docs");
        assert!(fallback(auto, feeder, true, 0, &empty));
        assert!(fallback(auto, feeder, true, 0, &ScanError::Failed("the feeder is empty".into())));
        assert!(fallback(auto, feeder, true, 0, &ScanError::Failed(BUSY_OR_FEEDER_EMPTY.into())));
        // The user chose the source, auto already chose the glass, there is no glass, or pages arrived.
        assert!(!fallback(feeder, feeder, true, 0, &empty));
        assert!(!fallback(glass, glass, true, 0, &empty));
        assert!(!fallback(auto, glass, true, 0, &empty));
        assert!(!fallback(auto, feeder, false, 0, &empty));
        assert!(!fallback(auto, feeder, true, 2, &empty));
        let incomplete = ScanError::Incomplete {
            message: "the feeder reported no more paper.".into(),
            acquired: Acquired {
                source: Source::Feeder,
                pages: vec![Page {
                    bytes: Vec::new(),
                    format: "bmp",
                    width: 1,
                    height: 1,
                    dpi: 1,
                    dpi_y: 1,
                    requested_dpi: 1,
                    thresholded: false,
                    length: Default::default(),
                }],
            },
            cause: None,
        };
        assert_eq!(super::pages_in_error(&incomplete), 1);
        assert!(!fallback(auto, feeder, true, super::pages_in_error(&incomplete), &incomplete));
        // Every other cause stays an error.
        for kind in K::ALL.iter().filter(|k| !matches!(k, K::FeederEmpty | K::BusyOrFeederEmpty)) {
            let error = ScanError::native_error(*kind, NativeCode::new("x", 1, None), kind.sentence());
            assert!(!fallback(auto, feeder, true, 0, &error), "{kind:?}");
        }
        assert!(!fallback(auto, feeder, true, 0, &ScanError::Failed("the scan was cancelled".into())));
        assert!(!fallback(auto, feeder, true, 0, &ScanError::Unsupported));
    }

    #[test]
    fn interrupt_kind_reads_shortfall_prefix_and_generic_sentences() {
        use super::{InterruptKind as K, NativeCode, feeder_shortfall_message, interrupt_kind};
        assert_eq!(interrupt_kind(&feeder_shortfall_message(3, 1)), K::FeederEmpty);
        assert_eq!(interrupt_kind("the feeder is empty"), K::FeederEmpty);
        assert_eq!(interrupt_kind("USB scanner (usb-2): The scanner is offline. Check that it is powered on and connected."), K::Offline);
        assert_eq!(interrupt_kind("The scan stopped before all pages arrived. Check for a paper jam."), K::Other);
        assert_eq!(interrupt_kind(""), K::Other);
        assert_eq!(ScanError::Failed("The scanner has a paper jam.".into()).interrupt_kind(), K::Jam);
        assert_eq!(interrupt_kind(super::BUSY_OR_FEEDER_EMPTY), K::BusyOrFeederEmpty);
        assert_eq!(interrupt_kind("Example: The scanner is busy, or the feeder has no paper."), K::BusyOrFeederEmpty);
        // An error that carries its kind never goes through the sentence.
        let native = ScanError::native_error(K::Offline, NativeCode::new("x", 1, None), "the scanner has a paper jam");
        assert_eq!(native.interrupt_kind(), K::Offline);
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    #[test]
    fn a_plain_false_is_unknown_until_the_backend_says_it_read_the_bits() {
        let mut scanner = Scanner {
            id: String::new(),
            name: String::new(),
            feeder: false,
            flatbed: true,
            duplex: false,
            feeder_ready: None,
            handling_status: HandlingStatus::default(),
            feeder_capacity: None,
            connection: Connection::Unknown,
            port: None,
            optical_x: None,
            optical_y: None,
            pages: None,
            bed: None,
            sheet: None,
            glass: None,
            feeder_choices: None,
            reported: Reported::default(),
        };
        assert_eq!((scanner.feeder_presence(), scanner.duplex_presence()), (Presence::Unknown, Presence::Unknown));
        assert_eq!(scanner.flatbed_presence(), Presence::Present, "a true is always an answer");
        scanner.reported.units = true;
        assert_eq!((scanner.feeder_presence(), scanner.duplex_presence()), (Presence::Absent, Presence::Absent));
        scanner.feeder = true;
        assert_eq!(scanner.feeder_presence(), Presence::Present);
    }

    #[test]
    fn a_list_is_present_when_it_has_values_absent_when_recorded_missing_and_unknown_otherwise() {
        let mut scanner = Scanner {
            id: String::new(),
            name: String::new(),
            feeder: true,
            flatbed: true,
            duplex: false,
            feeder_ready: None,
            handling_status: HandlingStatus::default(),
            feeder_capacity: None,
            connection: Connection::Unknown,
            port: None,
            optical_x: None,
            optical_y: None,
            pages: None,
            bed: None,
            sheet: None,
            glass: Some(ImageChoices { dpi: vec![300], ..ImageChoices::default() }),
            feeder_choices: None,
            reported: Reported::default(),
        };
        let glass = |s: &Scanner, f| s.choice_presence(Source::Glass, f);
        assert_eq!(glass(&scanner, ChoiceField::Dpi), Presence::Present);
        assert_eq!(glass(&scanner, ChoiceField::Formats), Presence::Unknown, "an empty list alone is not an answer");
        assert_eq!(scanner.choice_presence(Source::Feeder, ChoiceField::Dpi), Presence::Unknown, "item never read");
        scanner.reported.glass_absent.push(ChoiceField::Formats);
        assert_eq!(glass(&scanner, ChoiceField::Formats), Presence::Absent);
        assert_eq!(glass(&scanner, ChoiceField::Colors), Presence::Unknown);
        scanner.feeder = false;
        scanner.reported.units = true;
        assert_eq!(scanner.choice_presence(Source::Feeder, ChoiceField::Dpi), Presence::Absent, "no feeder, nothing to list");
    }
}
