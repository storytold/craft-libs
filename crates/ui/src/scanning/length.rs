//! The page-length choice of the scan settings: what the options are, which of them the scanner and the
//! app's other choices allow, and why not. Backend independent: the app maps [`PageLength`] to its own
//! request (for `craft-scan`: driver, size, auto, infer).
//!
//! The four options, in the order a list shows them:
//!
//! * **Driver default**: the scanner's own length. Always available.
//! * **Match page size**: the length of the page size the app has chosen. Available when a page size
//!   that names a length is chosen ([`Settings::page_size_chosen`]); otherwise disabled with a reason.
//! * **Detect (scanner)**: the scanner measures the sheet itself. Three-state like every other option:
//!   [`Choices::detect_length`] supported means the device published it (enabled); unsupported means
//!   the device answered without it (disabled with a reason); unknown means nothing was published
//!   (enabled, with the quiet "not confirmed" note, and only a hint).
//! * **Detect from image (approximate)**: a guess made from the scanned image. Always available, and
//!   always shown with a note that it is approximate.

use super::*;

/// How the page length is decided. See the notes at the top of the `length` module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PageLength {
    /// The scanner's own length.
    #[default]
    Driver,
    /// The length of the chosen page size.
    MatchSize,
    /// The scanner detects it.
    Detect,
    /// Guessed from the scanned image (approximate).
    Infer,
}

impl PageLength {
    /// Every option, in list order.
    pub const ALL: [PageLength; 4] = [PageLength::Driver, PageLength::MatchSize, PageLength::Detect, PageLength::Infer];
}

/// Why an option cannot be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthReason {
    /// No page size that names a length is chosen.
    NoPageSize,
    /// The scanner answered that it does not detect the page length.
    NoDetection,
}

/// Whether one option can be chosen now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthAvailability {
    /// Enabled, and the scanner confirmed it (or it needs no confirmation).
    Available,
    /// Enabled, but the scanner did not say either way: only a hint.
    Unconfirmed,
    /// Disabled, with a reason.
    Unavailable(LengthReason),
}

impl LengthAvailability {
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Unavailable(_))
    }
}

/// One row of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LengthChoice {
    pub value: PageLength,
    pub availability: LengthAvailability,
}

impl Choices {
    /// Whether `value` can be chosen, given whether the app has a page size that names a length.
    pub fn length_availability(&self, value: PageLength, page_size_chosen: bool) -> LengthAvailability {
        match value {
            PageLength::Driver | PageLength::Infer => LengthAvailability::Available,
            PageLength::MatchSize if page_size_chosen => LengthAvailability::Available,
            PageLength::MatchSize => LengthAvailability::Unavailable(LengthReason::NoPageSize),
            PageLength::Detect => match &self.detect_length {
                Capability::Unsupported => LengthAvailability::Unavailable(LengthReason::NoDetection),
                Capability::Unknown => LengthAvailability::Unconfirmed,
                _ => LengthAvailability::Available,
            },
        }
    }

    /// All four options with their availability, in list order.
    pub fn length_choices(&self, page_size_chosen: bool) -> [LengthChoice; 4] {
        PageLength::ALL.map(|value| LengthChoice { value, availability: self.length_availability(value, page_size_chosen) })
    }

    /// `value` if it can be chosen, else the driver default.
    pub fn length_or_default(&self, value: PageLength, page_size_chosen: bool) -> PageLength {
        if self.length_availability(value, page_size_chosen).is_enabled() { value } else { PageLength::Driver }
    }
}

/// The wording of the page-length row, supplied by the app.
#[derive(Clone, Copy, Debug)]
pub struct LengthLabels<'a> {
    /// The row caption, e.g. "Page length".
    pub heading: &'a str,
    /// "Driver default".
    pub driver: &'a str,
    /// "Match page size".
    pub match_size: &'a str,
    /// "Detect (scanner)".
    pub detect: &'a str,
    /// "Detect from image (approximate)".
    pub infer: &'a str,
    /// Why "Match page size" is disabled, e.g. "Choose a page size first."
    pub no_page_size: &'a str,
    /// Why "Detect (scanner)" is disabled, e.g. "This scanner does not detect the page length."
    pub no_detection: &'a str,
    /// The note shown while "Detect from image" is chosen, e.g. "Estimated from the scanned image, so it can be wrong."
    pub infer_note: &'a str,
}

impl<'a> LengthLabels<'a> {
    pub fn option(&self, value: PageLength) -> &'a str {
        match value {
            PageLength::Driver => self.driver,
            PageLength::MatchSize => self.match_size,
            PageLength::Detect => self.detect,
            PageLength::Infer => self.infer,
        }
    }

    pub fn reason(&self, reason: LengthReason) -> &'a str {
        match reason {
            LengthReason::NoPageSize => self.no_page_size,
            LengthReason::NoDetection => self.no_detection,
        }
    }

    /// The quiet line under the row for the current choice: the unconfirmed note for an unconfirmed
    /// detection, the approximation note for an image guess, otherwise nothing.
    pub fn note_for(&self, value: PageLength, availability: LengthAvailability, unconfirmed: &'a str) -> &'a str {
        match (value, availability) {
            (PageLength::Infer, _) => self.infer_note,
            (_, LengthAvailability::Unconfirmed) => unconfirmed,
            _ => "",
        }
    }
}
