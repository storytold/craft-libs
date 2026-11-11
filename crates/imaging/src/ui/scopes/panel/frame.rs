//! The data the host passes the scope panel: counted channels, the frame, the source and the readout.

use crate::ui::histogram::ChannelColor;
use crate::ui::scopes::clipping::ClippingFrame;
use crate::ui::scopes::vectorscope::VectorscopeFrame;

/// The Channel key of [`HistogramSet::colors`]'s overlay entry, kept in
/// [`ScopeSettings::channel`].
pub const COLORS_CHANNEL: &str = "colors";

/// One channel's counted histogram, as the host counted it (colour management and the count are
/// the host's).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScopeChannel<'a> {
    /// A stable key ("rgb", "red", "cyan", "luminosity" …), kept in [`ScopeSettings::channel`].
    /// The first channel of a [`HistogramSet`] is the composite.
    pub key: &'a str,
    /// The name in the Channel list and the value text, translated by the host ("Red",
    /// "Lightness").
    pub label: &'a str,
    /// The colour the channel draws in when coloured (`Neutral` for the composite and
    /// Luminosity).
    pub ink: ChannelColor,
    /// One count per level, usually 256.
    pub bins: &'a [u32],
    /// One of the colour mode's colour channels: drawn in All Channels View and overlaid by the
    /// Colors entry (no composite, Luminosity, alpha, spot or mask channels).
    pub color_channel: bool,
}

/// How the Colors entry overlays the colour channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChannelOverlay {
    /// Light (RGB): fills add where they overlap, on the dark plot
    /// (the RGB look of [`HistogramData`](crate::ui::histogram::HistogramData)).
    #[default]
    Light,
    /// Inks (CMYK): fills multiply where they overlap
    /// (the inks look of [`HistogramData`](crate::ui::histogram::HistogramData)).
    Ink,
}

/// The histograms the host counted for the colour mode, and how fresh they are.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct HistogramSet<'a> {
    pub(crate) channels: &'a [ScopeChannel<'a>],
    pub(crate) original: Option<&'a [ScopeChannel<'a>]>,
    pub(crate) colors: Option<ChannelOverlay>,
    pub(crate) stale: bool,
    pub(crate) cache_level: u32,
}

impl<'a> ScopeChannel<'a> {
    /// A channel `key` named `label` with the counts `bins`, drawn in `ink`.
    pub const fn new(key: &'a str, label: &'a str, ink: ChannelColor, bins: &'a [u32], color_channel: bool) -> Self {
        ScopeChannel { key, label, ink, bins, color_channel }
    }
}

impl<'a> ScopeSource<'a> {
    /// A source `key` named `label`.
    pub const fn new(key: &'a str, label: &'a str) -> Self {
        ScopeSource { key, label }
    }
}

impl<'a> HistogramSet<'a> {
    /// The mode's channels in Channel-list order, the composite first (PhotoCraft's
    /// `channel_options`: RGB, Red, Green, Blue, Luminosity; CMYK, Cyan, Magenta, Yellow, Black,
    /// Luminosity; Lightness, a, b, Luminosity; Gray).
    // pc: crates/ui-egui/src/panels/histogram_panel.rs:123 `channel_options`.
    pub fn new(channels: &'a [ScopeChannel<'a>]) -> Self {
        Self { channels, original: None, colors: None, stale: false, cache_level: 1 }
    }

    /// The histograms before the adjustment that previews now: each channel's original is outlined
    /// over the adjusted one (matched by key; Colors outlines the composite), in `text.normal` at
    /// 0.8, 1 pt.
    // pc: crates/ui-egui/src/panels/histogram_panel.rs:652-654, :695 `outline`.
    pub fn original(mut self, original: Option<&'a [ScopeChannel<'a>]>) -> Self {
        self.original = original;
        self
    }

    /// Offers the Colors entry at the end of the Channel list, overlaying the colour channels as
    /// `overlay` says (RGB and CMYK modes; Lab and Gray have none).
    pub fn colors(mut self, overlay: ChannelOverlay) -> Self {
        self.colors = Some(overlay);
        self
    }

    /// The counts are from a cache or out of date: the plot shows the library histogram's stale
    /// mark, whose click asks for a refresh.
    pub fn stale(mut self, stale: bool) -> Self {
        self.stale = stale;
        self
    }

    /// The cache level the counts were made at (1 = every pixel): shown as Cache Level; above 1
    /// the stale mark shows too.
    pub fn cache_level(mut self, level: u32) -> Self {
        self.cache_level = level.max(1);
        self
    }
}

/// A sample under the pointer, for the hover readout badge, in the form
/// [`ScopeSettings::lab_readouts`] asks for (the host does the colour management).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ScopeReadout {
    /// R, G, B in 0..=1, shown in 0..=255 without decimals.
    Rgb([f32; 3]),
    /// L, a, b, shown with one decimal.
    Lab([f32; 3]),
}

/// Everything the host counted for one pass: the histograms and, for Camera Raw's scope, the
/// vectorscope, the clipping state and the hover sample.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScopeFrame<'a> {
    pub(crate) histogram: HistogramSet<'a>,
    pub(crate) vectorscope: Option<VectorscopeFrame<'a>>,
    pub(crate) clipping: Option<ClippingFrame>,
    pub(crate) readout: Option<ScopeReadout>,
}

impl<'a> ScopeFrame<'a> {
    /// A frame of histograms only (Photoshop's Histogram panel).
    pub fn histogram(histogram: HistogramSet<'a>) -> Self {
        Self { histogram, vectorscope: None, clipping: None, readout: None }
    }

    /// The vectorscope's counts; shown under the plot while [`ScopeSettings::vectorscope`] is on.
    pub fn vectorscope(mut self, vectorscope: VectorscopeFrame<'a>) -> Self {
        self.vectorscope = Some(vectorscope);
        self
    }

    /// The clipped channels: shows the two clipping warning buttons in the plot's top corners.
    pub fn clipping(mut self, clipping: ClippingFrame) -> Self {
        self.clipping = Some(clipping);
        self
    }

    /// The sample under the pointer: the readout badge over the plot while it is passed.
    pub fn readout(mut self, readout: Option<ScopeReadout>) -> Self {
        self.readout = readout;
        self
    }
}

/// A Source-menu entry (Entire Image, Selected Layer, Adjustment Composite): what the host counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScopeSource<'a> {
    /// A stable key, kept in [`ScopeSettings::source`].
    pub key: &'a str,
    /// The name, translated by the host.
    pub label: &'a str,
}
