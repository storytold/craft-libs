//! Per-job options a request can carry beside the legacy [`crate::Job`]: media, orientation,
//! quality, resolution and an explicit colour choice, plus the page count the app knows.
//!
//! Everything here is a **hint**. A backend applies what it can and ignores the rest; which
//! settings it applies is published as [`PrinterDetails::honoured`](crate::PrinterDetails) (the
//! `Media`, `Orientation`, `Quality`, `Resolution` and `Colour` entries of [`JobHint`](crate::JobHint)), and
//! a UI hides the controls a backend does not honour. A setting that was dropped on the way shows
//! up in [`Submitted::warnings`](crate::Submitted) when the backend can tell.
//!
//! The options travel in [`SubmitRequest::options`](crate::SubmitRequest), not in
//! [`Job`](crate::Job): `Job` is the spooler's own settings (copies, duplex, driver keywords) and
//! is also the argument of the older single-backend functions, which stay as they are.
//! [`ColourMode`] overlaps `Job::grayscale`: when it is not [`ColourMode::Auto`] it decides.

/// Which way up the page is printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Orientation {
    Portrait,
    Landscape,
    ReversePortrait,
    ReverseLandscape,
}

impl Orientation {
    /// A stable lowercase name for logs and tool results.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
            Self::ReversePortrait => "reverse_portrait",
            Self::ReverseLandscape => "reverse_landscape",
        }
    }
}

/// Print quality, from the fastest draft to the finest output. A hint: a backend maps it to the
/// printer's own quality setting when the printer offers a matching value and drops it (with a
/// warning in [`Submitted::warnings`](crate::Submitted)) otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Quality {
    Draft,
    Normal,
    High,
    /// Photographic output (the slowest, finest mode some printers offer beyond [`Self::High`]).
    Photo,
}

impl Quality {
    /// Every quality, from draft to photo.
    pub const ALL: [Quality; 4] = [Self::Draft, Self::Normal, Self::High, Self::Photo];

    /// The quality named `name` (as [`Self::name`] writes it; ASCII case does not matter).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.name().eq_ignore_ascii_case(name.trim()))
    }
}

impl Quality {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Photo => "photo",
        }
    }
}

/// Colour or monochrome output; [`Self::Auto`] leaves the choice to the printer and
/// [`Job::grayscale`](crate::Job).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColourMode {
    #[default]
    Auto,
    Colour,
    Mono,
}

impl ColourMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Colour => "colour",
            Self::Mono => "mono",
        }
    }

    /// Whether the job is monochrome, given the legacy `Job::grayscale`: an explicit choice
    /// wins over it.
    pub const fn is_mono(self, legacy_grayscale: bool) -> bool {
        match self {
            Self::Auto => legacy_grayscale,
            Self::Colour => false,
            Self::Mono => true,
        }
    }
}

/// A print resolution in dots per inch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Resolution {
    pub x_dpi: u32,
    pub y_dpi: u32,
}

impl Resolution {
    pub const fn new(x_dpi: u32, y_dpi: u32) -> Self {
        Self { x_dpi, y_dpi }
    }

    /// The same resolution both ways.
    pub const fn square(dpi: u32) -> Self {
        Self { x_dpi: dpi, y_dpi: dpi }
    }
}

/// Per-job hints; build one with [`Self::new`] and the `with_*` methods. Every field is optional:
/// `None` (and [`ColourMode::Auto`]) means "the printer's default". The struct is
/// `#[non_exhaustive]`, so new hints can be added without breaking callers; read and assign its
/// fields freely.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct JobOptions {
    /// The paper size as a PWG self-describing media name (`"iso_a4_210x297mm"`,
    /// `"na_letter_8.5x11in"`), the same tokens a printer's capabilities list. A backend that
    /// cannot use the token ignores it.
    pub media: Option<String>,
    pub orientation: Option<Orientation>,
    pub quality: Option<Quality>,
    pub resolution: Option<Resolution>,
    pub colour: ColourMode,
    /// How many pages the document has, when the app knows. Not a hint to the printer: it is the
    /// total that progress and status use when the backend reports none (see
    /// [`ExpectedTotals`](crate::ExpectedTotals)). A total the backend reports wins.
    pub expected_pages: Option<u32>,
}

impl JobOptions {
    /// No hints: everything is the printer's default.
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_media(mut self, media: impl Into<String>) -> Self {
        self.media = Some(media.into());
        self
    }

    #[must_use]
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    #[must_use]
    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = Some(quality);
        self
    }

    #[must_use]
    pub fn with_resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    #[must_use]
    pub fn with_colour(mut self, colour: ColourMode) -> Self {
        self.colour = colour;
        self
    }

    #[must_use]
    pub fn with_expected_pages(mut self, pages: u32) -> Self {
        self.expected_pages = Some(pages);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_the_printers_choice() {
        let options = JobOptions::new();
        assert_eq!(options, JobOptions::default());
        assert_eq!((options.media, options.orientation, options.quality, options.resolution, options.expected_pages), (None, None, None, None, None));
        assert_eq!(options.colour, ColourMode::Auto);
    }

    #[test]
    fn builders_set_each_hint() {
        let options = JobOptions::new()
            .with_media("iso_a4_210x297mm")
            .with_orientation(Orientation::Landscape)
            .with_quality(Quality::High)
            .with_resolution(Resolution::new(600, 300))
            .with_colour(ColourMode::Mono)
            .with_expected_pages(3);
        assert_eq!(options.media.as_deref(), Some("iso_a4_210x297mm"));
        assert_eq!(options.orientation, Some(Orientation::Landscape));
        assert_eq!(options.quality, Some(Quality::High));
        assert_eq!(options.resolution, Some(Resolution { x_dpi: 600, y_dpi: 300 }));
        assert_eq!((options.colour, options.expected_pages), (ColourMode::Mono, Some(3)));
        assert_eq!(Resolution::square(300), Resolution::new(300, 300));
    }

    #[test]
    fn photo_is_a_quality_with_a_stable_name() {
        assert_eq!(Quality::Photo.name(), "photo");
        assert_eq!(Quality::from_name(" Photo "), Some(Quality::Photo));
        assert_eq!(Quality::from_name("high"), Some(Quality::High));
        assert_eq!(Quality::from_name("best"), None);
        assert!(Quality::ALL.iter().all(|q| Quality::from_name(q.name()) == Some(*q)));
        assert_eq!(JobOptions::new().with_quality(Quality::Photo).quality, Some(Quality::Photo));
    }

    #[test]
    fn an_explicit_colour_choice_beats_the_legacy_grayscale_flag() {
        assert!(ColourMode::Auto.is_mono(true) && !ColourMode::Auto.is_mono(false));
        assert!(!ColourMode::Colour.is_mono(true));
        assert!(ColourMode::Mono.is_mono(false));
    }
}
