//! The scan contract: every enum and error identity a binding may expose, defined once.
//!
//! Language bindings (CLI, MCP tools, Python, the harness) must not invent their own enums or
//! parse prose. They list a type's `ALL`, spell a value with `as_str`, read one back with
//! `FromStr`/`parse`, and take an error's identity from [`ScanErrorInfo`]. The wire strings are
//! stable lowercase snake_case (`business-card` keeps its hyphen: it is a page-size name the CLI
//! has always taken).
//!
//! Adding a variant means: add it to the type, to its `ALL`, and to `as_str`; the exhaustive
//! `match` in `as_str` fails to compile until it is named, and `contract_json` (read by the Python
//! drift test) carries the new value to every binding.

use std::fmt;
use std::str::FromStr;

use crate::{
    Color, Connection, HRESULT_DOMAIN, HandlingStatus, Intent, InterruptKind, LengthSource, NativeCode, PageLength, PageSize, ProgressToken,
    ScanError, ScanSettings, Source, pages_in_error,
};

/// A wire string that names no value of the enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseWireError {
    /// The enum that was being read (`"source"`, `"color"`, ...).
    pub what: &'static str,
    /// The text that matched nothing.
    pub got: String,
}

impl fmt::Display for ParseWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {} {:?}", self.what, self.got)
    }
}

impl std::error::Error for ParseWireError {}

macro_rules! wire_text {
    ($ty:ident, $what:literal $(, unknown_reads_as $fallback:expr)?) => {
        impl FromStr for $ty {
            type Err = ParseWireError;

            fn from_str(text: &str) -> Result<Self, ParseWireError> {
                Self::parse(text).ok_or_else(|| ParseWireError { what: $what, got: text.to_string() })
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        /// With the `serde` feature: the existing wire string.
        #[cfg(feature = "serde")]
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <String as serde::Deserialize>::deserialize(deserializer)?;
                match text.parse::<$ty>() {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        let _ = &error;
                        $(return Ok($fallback);)?
                        #[allow(unreachable_code)]
                        Err(serde::de::Error::custom(error))
                    }
                }
            }
        }
    };
}

// ---- enums defined elsewhere in the crate: list and spell them here ------------------------------

impl Source {
    /// Every source, in the order a UI lists them.
    pub const ALL: &'static [Source] = &[Source::Auto, Source::Feeder, Source::Glass];

    /// Wire string: `auto`, `feeder`, `glass` (the same as [`Self::name`]).
    pub fn as_str(self) -> &'static str {
        self.name()
    }
}
wire_text!(Source, "source");

impl Color {
    /// Every color mode, in the order a UI lists them.
    pub const ALL: &'static [Color] = &[Color::Gray, Color::Rgb, Color::BlackAndWhite];

    /// Wire string: `rgb`, `gray`, `bw` (the same as [`Self::name`]).
    pub fn as_str(self) -> &'static str {
        self.name()
    }

    /// Read a wire string; `grey` and `color` are accepted as the CLI always has.
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "gray" | "grey" => Some(Self::Gray),
            "rgb" | "color" => Some(Self::Rgb),
            "bw" => Some(Self::BlackAndWhite),
            _ => None,
        }
    }
}
wire_text!(Color, "color");

impl Intent {
    /// Every intent preset.
    pub const ALL: &'static [Intent] =
        &[Intent::None, Intent::Color, Intent::Gray, Intent::Text, Intent::BestPreview, Intent::MaximizeQuality, Intent::MinimizeSize];

    /// Wire string: `none`, `color`, `gray`, `text`, `preview`, `quality`, `size`.
    pub fn as_str(self) -> &'static str {
        self.name()
    }
}
wire_text!(Intent, "intent");

impl PageSize {
    /// Every named page size.
    pub const ALL: &'static [PageSize] = &[PageSize::A4, PageSize::Letter, PageSize::Legal, PageSize::Auto, PageSize::Custom, PageSize::BusinessCard];

    /// Wire string: `a4`, `letter`, `legal`, `auto`, `custom`, `business-card`.
    pub fn as_str(self) -> &'static str {
        self.name()
    }
}
wire_text!(PageSize, "page size");

wire_text!(PageLength, "page length");
wire_text!(LengthSource, "length source");

impl Connection {
    /// Every connection class.
    pub const ALL: &'static [Connection] = &[Connection::Wsd, Connection::Usb, Connection::Other, Connection::Unknown];

    /// Wire string: `wsd`, `usb`, `other`, `unknown`.
    pub fn as_str(self) -> &'static str {
        self.name()
    }

    /// Read a wire string (not a port: see [`Self::from_port`]).
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|value| value.as_str() == text)
    }
}
wire_text!(Connection, "connection");

impl InterruptKind {
    /// Every kind, in declaration order.
    pub const ALL: &'static [InterruptKind] = &[
        InterruptKind::CoverOpen,
        InterruptKind::Jam,
        InterruptKind::PaperProblem,
        InterruptKind::FeederEmpty,
        InterruptKind::MultiFeed,
        InterruptKind::Offline,
        InterruptKind::Attention,
        InterruptKind::Busy,
        InterruptKind::BusyOrFeederEmpty,
        InterruptKind::DeviceError,
        InterruptKind::Cancelled,
        InterruptKind::Other,
    ];

    /// Wire string (`cover_open`, `jam`, `paper_problem`, `feeder_empty`, `multi_feed`, `offline`,
    /// `attention`, `busy`, `busy_or_feeder_empty`, `device_error`, `cancelled`, `other`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CoverOpen => "cover_open",
            Self::Jam => "jam",
            Self::PaperProblem => "paper_problem",
            Self::FeederEmpty => "feeder_empty",
            Self::MultiFeed => "multi_feed",
            Self::Offline => "offline",
            Self::Attention => "attention",
            Self::Busy => "busy",
            Self::BusyOrFeederEmpty => "busy_or_feeder_empty",
            Self::DeviceError => "device_error",
            Self::Cancelled => "cancelled",
            Self::Other => "other",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|value| value.as_str() == text)
    }
}
wire_text!(InterruptKind, "interrupt kind", unknown_reads_as InterruptKind::Other);

// ---- enums that only exist here -------------------------------------------------------------------

/// An image format a caller may ask the driver for (`ScanSettings::format`). Pages themselves are
/// labelled by [`crate::image_format`], which can also answer `bin` for bytes it does not know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Bmp,
    Jpeg,
    Png,
    Tiff,
    Jpeg2k,
}

impl ImageFormat {
    pub const ALL: &'static [ImageFormat] = &[ImageFormat::Bmp, ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff, ImageFormat::Jpeg2k];

    /// Wire string: `bmp`, `jpeg`, `png`, `tiff`, `jpeg2k`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bmp => "bmp",
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Jpeg2k => "jpeg2k",
        }
    }

    /// Read a name or extension: `jpg`, `tif`, `jp2` and a leading dot are accepted.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "bmp" => Some(Self::Bmp),
            "jpeg" | "jpg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tiff" | "tif" => Some(Self::Tiff),
            "jpeg2k" | "jp2" => Some(Self::Jpeg2k),
            _ => None,
        }
    }
}
wire_text!(ImageFormat, "image format");

/// A compression the driver may offer (`ScanSettings::compression`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    None,
    Rle4,
    Rle8,
    G3,
    G4,
    Jpeg,
    Jbig,
    Jpeg2k,
    Png,
}

impl Compression {
    pub const ALL: &'static [Compression] = &[
        Compression::None,
        Compression::Rle4,
        Compression::Rle8,
        Compression::G3,
        Compression::G4,
        Compression::Jpeg,
        Compression::Jbig,
        Compression::Jpeg2k,
        Compression::Png,
    ];

    /// Wire string: `none`, `rle4`, `rle8`, `g3`, `g4`, `jpeg`, `jbig`, `jpeg2k`, `png`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Rle4 => "rle4",
            Self::Rle8 => "rle8",
            Self::G3 => "g3",
            Self::G4 => "g4",
            Self::Jpeg => "jpeg",
            Self::Jbig => "jbig",
            Self::Jpeg2k => "jpeg2k",
            Self::Png => "png",
        }
    }

    /// Read a name; `jpg` and `jp2` are accepted.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "rle4" => Some(Self::Rle4),
            "rle8" => Some(Self::Rle8),
            "g3" => Some(Self::G3),
            "g4" => Some(Self::G4),
            "jpeg" | "jpg" => Some(Self::Jpeg),
            "jbig" => Some(Self::Jbig),
            "jpeg2k" | "jp2" => Some(Self::Jpeg2k),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
}
wire_text!(Compression, "compression");

/// The `details` field of a scanner listing row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListDetails {
    /// The device was listed and not opened.
    Skipped,
    /// The device was opened and its capabilities read.
    Loaded,
    /// The device would not open; the row carries the reason.
    Error,
}

impl ListDetails {
    pub const ALL: &'static [ListDetails] = &[ListDetails::Skipped, ListDetails::Loaded, ListDetails::Error];

    /// Wire string: `skipped`, `loaded`, `error`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Loaded => "loaded",
            Self::Error => "error",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|value| value.as_str() == text)
    }
}
wire_text!(ListDetails, "list details");

/// Where in a scan a failure happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// No page had arrived, and none was in flight.
    BeforeFirstPage,
    /// At least one page had arrived and no page was part-way across.
    BetweenPages,
    /// A page was part-way across (bytes received, page not finished).
    MidPage,
}

impl Phase {
    pub const ALL: &'static [Phase] = &[Phase::BeforeFirstPage, Phase::BetweenPages, Phase::MidPage];

    /// Wire string: `before_first_page`, `between_pages`, `mid_page`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BeforeFirstPage => "before_first_page",
            Self::BetweenPages => "between_pages",
            Self::MidPage => "mid_page",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|value| value.as_str() == text)
    }

    /// The phase for what is known at the moment of failure.
    ///
    /// `mid_page` is true when bytes of a page had arrived and the page had not finished; it is
    /// only knowable when a [`ProgressToken`] watched the transfer ([`ProgressToken::in_page`]).
    /// Without one, a failure after pages arrived reads as [`Self::BetweenPages`]: a part-way page
    /// is not told apart from a clean gap.
    pub fn classify(pages_received: usize, mid_page: bool) -> Self {
        if mid_page {
            Self::MidPage
        } else if pages_received > 0 {
            Self::BetweenPages
        } else {
            Self::BeforeFirstPage
        }
    }
}
wire_text!(Phase, "phase");

// ---- recovery --------------------------------------------------------------------------------------

/// What a caller can do about a stopped scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Recovery {
    /// Nothing was kept and every wanted page is still wanted: start again with the same settings.
    pub retryable: bool,
    /// Pages were kept and some are still wanted: scan on from where it stopped.
    pub resumable: bool,
}

/// The one rule behind "Continue" versus "Try again" (the desktop dialog and every binding use it).
///
/// `pages_expected` is the wanted page count, `None` when the whole feeder was asked for. With
/// nothing left to scan neither flag is set. Otherwise: pages kept means resumable, none kept
/// means retryable. The failure's kind and phase choose the *wording* of the advice, not the
/// action, so they are not inputs.
pub fn recovery(pages_received: usize, pages_expected: Option<u32>) -> Recovery {
    let received = u32::try_from(pages_received).unwrap_or(u32::MAX);
    let wanted_more = pages_expected.is_none_or(|expected| expected > received);
    Recovery { retryable: wanted_more && pages_received == 0, resumable: wanted_more && pages_received > 0 }
}

// ---- HandlingStatus ---------------------------------------------------------------------------------

impl HandlingStatus {
    /// Every flag, by wire name, in bit order.
    pub const FLAG_NAMES: [&'static str; 9] =
        ["feeder_ready", "flat_ready", "duplex_ready", "cover_open", "path_cover_open", "jam", "multiple_feed", "attention", "lamp_error"];

    /// The flags that mean the scanner is blocked or abnormal (a ready bit is not one).
    pub const CONDITION_NAMES: [&'static str; 6] = ["jam", "cover_open", "path_cover_open", "multiple_feed", "attention", "lamp_error"];

    /// All nine flags as `(name, set)`, in [`Self::FLAG_NAMES`] order. An unreported feeder reads as
    /// not set; use `feeder_ready` itself to tell "empty" from "unknown".
    pub fn flags(&self) -> [(&'static str, bool); 9] {
        [
            ("feeder_ready", self.feeder_ready == Some(true)),
            ("flat_ready", self.flat_ready),
            ("duplex_ready", self.duplex_ready),
            ("cover_open", self.cover_open),
            ("path_cover_open", self.path_cover_open),
            ("jam", self.jam),
            ("multiple_feed", self.multiple_feed),
            ("attention", self.attention),
            ("lamp_error", self.lamp_error),
        ]
    }

    /// Names of the blocking or abnormal flags that are set, in [`Self::CONDITION_NAMES`] order.
    pub fn conditions(&self) -> Vec<&'static str> {
        let set = self.flags();
        Self::CONDITION_NAMES.iter().copied().filter(|name| set.iter().any(|(flag, on)| flag == name && *on)).collect()
    }
}

// ---- structured error -------------------------------------------------------------------------------

/// What is known about a failure besides the error itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ErrorContext {
    /// Pages that had arrived (an `Incomplete` error's own pages count too; the larger wins).
    pub pages_received: usize,
    /// Pages wanted; `None` for the whole feeder (and for hardware duplex, which counts sides).
    pub pages_expected: Option<u32>,
    /// A page was part-way across ([`ProgressToken::in_page`]).
    pub mid_page: bool,
}

impl ErrorContext {
    /// The context of a failed `acquire`: page counts from the settings and, when the
    /// settings carry a progress token, how far the transfer had got.
    pub fn from_settings(settings: &ScanSettings) -> Self {
        let progress = settings.progress.as_ref();
        Self {
            pages_received: progress.map_or(0, ProgressToken::pages_done),
            pages_expected: (settings.pages > 0 && !settings.duplex).then_some(settings.pages),
            mid_page: progress.is_some_and(ProgressToken::in_page),
        }
    }
}

/// A [`ScanError`] as data: the identity a binding reports, without parsing the sentence.
///
/// `ScanError`'s `Display` text is unchanged; this sits beside it. The kind, phase, recovery flags
/// and message are neutral; `native` carries the backend's own code for logs and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScanErrorInfo {
    /// The backend's own error code, when the error carried one. A sentence an app decided on from
    /// a status read (a jam, an empty tray) keeps the code that triggered it.
    #[cfg_attr(feature = "serde", serde(default))]
    pub native: Option<NativeCode>,
    /// The cause. Taken from the structured source (the backend's own decision at the point it
    /// built the error); only an error with none behind it falls back to its sentence.
    pub kind: InterruptKind,
    pub phase: Phase,
    /// Nothing was kept: try again with the same settings.
    pub retryable: bool,
    /// Pages were kept: scan on from where it stopped.
    pub resumable: bool,
    /// The same text as the error's `Display`.
    pub message: String,
}

impl ScanErrorInfo {
    pub fn new(error: &ScanError, context: ErrorContext) -> Self {
        let pages = context.pages_received.max(pages_in_error(error));
        let phase = Phase::classify(pages, context.mid_page);
        // Scanning that cannot run at all is not a thing to retry.
        let recoverable = !matches!(error, ScanError::Unsupported);
        let recover = if recoverable { recovery(pages, context.pages_expected) } else { Recovery::default() };
        Self {
            native: error.native(),
            kind: error.interrupt_kind(),
            phase,
            retryable: recover.retryable,
            resumable: recover.resumable,
            message: error.to_string(),
        }
    }

    /// [`Self::new`] with the context of a failed acquire taken from its settings.
    pub fn from_acquire(error: &ScanError, settings: &ScanSettings) -> Self {
        Self::new(error, ErrorContext::from_settings(settings))
    }

    /// The Windows HRESULT, when `native` is in the `"hresult"` domain. For the apps whose CLI and
    /// Python bindings have always had an `hresult` field; other domains answer `None`.
    pub fn hresult(&self) -> Option<u32> {
        self.native.as_ref().and_then(NativeCodeExt::as_hresult)
    }

    /// The symbolic name of [`Self::hresult`] (`WIA_ERROR_BUSY`), only for the `"hresult"` domain.
    pub fn code_name(&self) -> Option<&'static str> {
        self.native.filter(|native| native.domain == HRESULT_DOMAIN).and_then(|native| native.name)
    }

    /// `hresult` spelled `0x80210006`, the way the CLI and the tools print it.
    pub fn hresult_hex(&self) -> Option<String> {
        self.hresult().map(|code| format!("{code:#010x}"))
    }
}

/// What craft-scan adds to [`NativeCode`] (which lives in craft-job, so it cannot have inherent methods here).
pub trait NativeCodeExt {
    /// The code as an unsigned HRESULT, when the domain is `"hresult"` and it fits.
    fn as_hresult(&self) -> Option<u32>;
}

impl NativeCodeExt for NativeCode {
    fn as_hresult(&self) -> Option<u32> {
        if self.domain == HRESULT_DOMAIN { u32::try_from(self.code).ok() } else { None }
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl ScanError {
    /// The structured identity of this error. See [`ScanErrorInfo`].
    pub fn info(&self, context: ErrorContext) -> ScanErrorInfo {
        ScanErrorInfo::new(self, context)
    }

    /// The Windows HRESULT behind this error, when its native code is in the `"hresult"` domain.
    pub fn hresult(&self) -> Option<u32> {
        self.native().and_then(|native| native.as_hresult())
    }

    /// The symbolic name of [`Self::hresult`], only for the `"hresult"` domain.
    pub fn code_name(&self) -> Option<&'static str> {
        self.native().filter(|native| native.domain == HRESULT_DOMAIN).and_then(|native| native.name)
    }
}

impl ProgressToken {
    /// A page is part-way across: bytes of it have arrived and it has not finished.
    pub fn in_page(&self) -> bool {
        self.bytes_received() > 0
    }
}

// ---- one JSON dump of every list (read by the binding drift tests) ------------------------------------

fn list_json(values: impl Iterator<Item = &'static str>) -> String {
    let items: Vec<String> = values.map(json_string).collect();
    format!("[{}]", items.join(","))
}

/// Every list a binding exposes, as one JSON object. Keys: `kinds`, `phases`, `sources`, `colors`,
/// `intents`, `page_sizes`, `formats`, `compressions`, `connections`, `list_details`,
/// `handling_flags`, `conditions`, `page_lengths` and `length_sources`. A backend's own code table (the HRESULT names of a Windows
/// backend) is not part of the core contract; an app that exposes one adds it beside this dump.
pub fn contract_json() -> String {
    format!(
        "{{\"kinds\":{},\"phases\":{},\"sources\":{},\"colors\":{},\"intents\":{},\"page_sizes\":{},\"formats\":{},\"compressions\":{},\"connections\":{},\"list_details\":{},\"handling_flags\":{},\"conditions\":{},\"page_lengths\":{},\"length_sources\":{}}}",
        list_json(InterruptKind::ALL.iter().map(|v| v.as_str())),
        list_json(Phase::ALL.iter().map(|v| v.as_str())),
        list_json(Source::ALL.iter().map(|v| v.as_str())),
        list_json(Color::ALL.iter().map(|v| v.as_str())),
        list_json(Intent::ALL.iter().map(|v| v.as_str())),
        list_json(PageSize::ALL.iter().map(|v| v.as_str())),
        list_json(ImageFormat::ALL.iter().map(|v| v.as_str())),
        list_json(Compression::ALL.iter().map(|v| v.as_str())),
        list_json(Connection::ALL.iter().map(|v| v.as_str())),
        list_json(ListDetails::ALL.iter().map(|v| v.as_str())),
        list_json(HandlingStatus::FLAG_NAMES.iter().copied()),
        list_json(HandlingStatus::CONDITION_NAMES.iter().copied()),
        list_json(PageLength::ALL.iter().map(|v| v.as_str())),
        list_json(LengthSource::ALL.iter().map(|v| v.as_str())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt_kind;

    fn round_trips<T: Copy + PartialEq + fmt::Debug + FromStr<Err = ParseWireError> + fmt::Display>(all: &[T], as_str: fn(T) -> &'static str) {
        let mut seen = Vec::new();
        for value in all {
            let text = as_str(*value);
            assert!(!text.is_empty() && !seen.contains(&text), "{value:?}: duplicate or empty wire string {text:?}");
            seen.push(text);
            assert_eq!(text.parse::<T>().ok(), Some(*value), "{text}");
            assert_eq!(value.to_string(), text);
        }
        assert!("definitely-not-a-value".parse::<T>().is_err());
    }

    #[test]
    fn every_enum_round_trips_through_its_wire_string() {
        round_trips(Source::ALL, Source::as_str);
        round_trips(Color::ALL, Color::as_str);
        round_trips(Intent::ALL, Intent::as_str);
        round_trips(PageSize::ALL, PageSize::as_str);
        round_trips(Connection::ALL, Connection::as_str);
        round_trips(InterruptKind::ALL, InterruptKind::as_str);
        round_trips(ImageFormat::ALL, ImageFormat::as_str);
        round_trips(Compression::ALL, Compression::as_str);
        round_trips(ListDetails::ALL, ListDetails::as_str);
        round_trips(Phase::ALL, Phase::as_str);
        round_trips(PageLength::ALL, PageLength::as_str);
        round_trips(LengthSource::ALL, LengthSource::as_str);
    }

    #[test]
    fn the_wire_strings_are_the_ones_the_bindings_have_always_used() {
        let names = |all: Vec<&'static str>| all.join(",");
        assert_eq!(names(Source::ALL.iter().map(|v| v.as_str()).collect()), "auto,feeder,glass");
        assert_eq!(names(Color::ALL.iter().map(|v| v.as_str()).collect()), "gray,rgb,bw");
        assert_eq!(names(Intent::ALL.iter().map(|v| v.as_str()).collect()), "none,color,gray,text,preview,quality,size");
        assert_eq!(names(PageSize::ALL.iter().map(|v| v.as_str()).collect()), "a4,letter,legal,auto,custom,business-card");
        assert_eq!(names(Connection::ALL.iter().map(|v| v.as_str()).collect()), "wsd,usb,other,unknown");
        assert_eq!(
            names(InterruptKind::ALL.iter().map(|v| v.as_str()).collect()),
            "cover_open,jam,paper_problem,feeder_empty,multi_feed,offline,attention,busy,busy_or_feeder_empty,device_error,cancelled,other"
        );
        assert_eq!(names(ListDetails::ALL.iter().map(|v| v.as_str()).collect()), "skipped,loaded,error");
        assert_eq!(names(Phase::ALL.iter().map(|v| v.as_str()).collect()), "before_first_page,between_pages,mid_page");
    }

    #[test]
    fn the_aliases_the_cli_accepts_still_parse() {
        assert_eq!(Source::parse("flatbed"), Some(Source::Glass));
        assert_eq!(Color::parse("grey"), Some(Color::Gray));
        assert_eq!(Color::parse("color"), Some(Color::Rgb));
        assert_eq!(Intent::parse("ocr"), Some(Intent::Text));
        assert_eq!(PageSize::parse("card"), Some(PageSize::BusinessCard));
        assert_eq!(ImageFormat::parse(".JPG"), Some(ImageFormat::Jpeg));
        assert_eq!(Compression::parse("jp2"), Some(Compression::Jpeg2k));
    }

    #[test]
    fn handling_status_names_all_nine_flags_and_lists_only_the_abnormal_ones() {
        let all = HandlingStatus {
            feeder_ready: Some(true),
            flat_ready: true,
            duplex_ready: true,
            cover_open: true,
            path_cover_open: true,
            jam: true,
            multiple_feed: true,
            attention: true,
            lamp_error: true,
        };
        assert!(all.flags().iter().all(|(_, on)| *on));
        assert_eq!(all.flags().map(|(name, _)| name), HandlingStatus::FLAG_NAMES);
        assert_eq!(all.conditions(), ["jam", "cover_open", "path_cover_open", "multiple_feed", "attention", "lamp_error"]);
        let ready = HandlingStatus { feeder_ready: Some(true), flat_ready: true, duplex_ready: true, ..HandlingStatus::default() };
        assert!(ready.conditions().is_empty());
        assert_eq!(HandlingStatus { jam: true, ..HandlingStatus::default() }.conditions(), ["jam"]);
        for name in HandlingStatus::CONDITION_NAMES {
            assert!(HandlingStatus::FLAG_NAMES.contains(&name));
        }
        // Not reported is not the same as empty.
        assert_eq!(HandlingStatus::default().feeder_ready, None);
    }

    #[test]
    fn the_recovery_rule_is_continue_with_pages_and_try_again_without() {
        assert_eq!(recovery(0, Some(3)), Recovery { retryable: true, resumable: false });
        assert_eq!(recovery(0, None), Recovery { retryable: true, resumable: false });
        assert_eq!(recovery(2, Some(3)), Recovery { retryable: false, resumable: true });
        assert_eq!(recovery(2, None), Recovery { retryable: false, resumable: true });
        assert_eq!(recovery(3, Some(3)), Recovery::default());
        assert_eq!(recovery(5, Some(3)), Recovery::default());
    }

    #[test]
    fn the_phase_follows_pages_and_the_page_in_flight() {
        assert_eq!(Phase::classify(0, false), Phase::BeforeFirstPage);
        assert_eq!(Phase::classify(2, false), Phase::BetweenPages);
        assert_eq!(Phase::classify(0, true), Phase::MidPage);
        assert_eq!(Phase::classify(2, true), Phase::MidPage);
    }

    #[test]
    fn a_native_error_carries_its_code_and_kind_without_reading_the_sentence() {
        let native = NativeCode::hresult(0x8021_0006, Some("WIA_ERROR_BUSY"));
        let error = ScanError::native_error(InterruptKind::Busy, native, "some text that names nothing");
        let info = error.info(ErrorContext::default());
        assert_eq!(info.native, Some(native));
        assert_eq!((info.hresult(), info.code_name()), (Some(0x8021_0006), Some("WIA_ERROR_BUSY")));
        assert_eq!(info.kind, InterruptKind::Busy);
        assert_eq!(info.phase, Phase::BeforeFirstPage);
        assert!(info.retryable && !info.resumable);
        assert_eq!(info.message, error.to_string());
        assert_eq!(info.hresult_hex().as_deref(), Some("0x80210006"));
        assert_eq!((error.hresult(), error.code_name()), (Some(0x8021_0006), Some("WIA_ERROR_BUSY")));
    }

    #[test]
    fn hresult_and_code_name_answer_only_for_the_hresult_domain() {
        let sane =
            ScanError::native_error(InterruptKind::FeederEmpty, NativeCode::new("sane", 6, Some("SANE_STATUS_NO_DOCS")), "The feeder has no paper.");
        let info = sane.info(ErrorContext::default());
        assert_eq!(info.native.map(|native| (native.domain, native.code)), Some(("sane", 6)));
        assert_eq!((info.hresult(), info.code_name(), info.hresult_hex()), (None, None, None));
        assert_eq!((sane.hresult(), sane.code_name()), (None, None));
        assert_eq!(info.kind, InterruptKind::FeederEmpty);
    }

    #[test]
    fn an_error_with_no_native_code_falls_back_to_its_sentence() {
        let error = ScanError::Failed("Only 1 of 3 pages were scanned: the feeder reported no more paper.".into());
        let info = error.info(ErrorContext { pages_received: 1, pages_expected: Some(3), mid_page: false });
        assert_eq!(info.native, None);
        assert_eq!(info.kind, InterruptKind::FeederEmpty);
        assert_eq!(info.phase, Phase::BetweenPages);
        assert!(!info.retryable && info.resumable);
        assert!(!ScanError::Unsupported.info(ErrorContext::default()).retryable);
    }

    #[test]
    fn the_phase_of_a_part_page_failure_is_mid_page() {
        let error = ScanError::native_error(InterruptKind::Jam, NativeCode::new("sane", 7, None), "");
        let info = error.info(ErrorContext { pages_received: 1, pages_expected: Some(2), mid_page: true });
        assert_eq!(info.phase, Phase::MidPage);
        assert!(info.resumable);
    }

    #[test]
    fn the_json_dump_lists_every_enum() {
        let dump = contract_json();
        for kind in InterruptKind::ALL {
            assert!(dump.contains(&format!("\"{}\"", kind.as_str())), "{kind:?}");
        }
        for name in HandlingStatus::FLAG_NAMES {
            assert!(dump.contains(name));
        }
        assert!(!dump.contains("hresults"));
        assert!(dump.starts_with("{\"kinds\":[") && dump.ends_with('}'));
    }

    #[test]
    fn json_strings_escape_quotes_and_control_characters() {
        assert_eq!(json_string("a\"b\\c\n\u{1}"), "\"a\\\"b\\\\c\\n\\u0001\"");
    }

    #[test]
    fn interrupt_kind_of_a_neutral_sentence_matches_its_kind() {
        for kind in InterruptKind::ALL {
            if *kind != InterruptKind::Other {
                assert_eq!(interrupt_kind(kind.sentence()), *kind, "{kind:?}");
            }
        }
        // Advice after the opening sentence does not matter; neither does a device-name prefix.
        assert_eq!(interrupt_kind("Desk: The scanner is busy. Close the other program."), InterruptKind::Busy);
        assert_eq!(interrupt_kind("The scanner is offline. Remove it and add it again."), InterruptKind::Offline);
        assert_eq!(interrupt_kind(InterruptKind::Other.sentence()), InterruptKind::Other);
    }
}
