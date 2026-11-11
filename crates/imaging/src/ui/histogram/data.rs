//! The data a histogram draws: channels, bins and how tall they are drawn.

use craft_theme::Rgba;
use craft_ui::style::{StyleColor, StyleCx};
use egui::Color32;

/// The colour of a channel in curves and histograms: a data token.
///
/// The colours are data colours, not style colours: the ribbons of a histogram are composed from their channel
/// values (sums and products), which needs the colour itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChannelColor {
    /// The composite, CMYK Black and Lab Lightness: `data.luma` (`text.muted` in a histogram).
    #[default]
    Neutral,
    /// `data.red`.
    Red,
    /// `data.green`.
    Green,
    /// `data.blue`.
    Blue,
    /// `data.cyan`.
    Cyan,
    /// `data.magenta`, also Lab a.
    Magenta,
    /// `data.yellow`, also Lab b.
    Yellow,
}

/// `rgba` as the colour egui paints.
pub(super) fn color32(rgba: Rgba) -> Color32 {
    let [r, g, b, a] = rgba.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl ChannelColor {
    /// Every channel colour, in declaration order.
    pub const ALL: [ChannelColor; 7] = [
        ChannelColor::Neutral,
        ChannelColor::Red,
        ChannelColor::Green,
        ChannelColor::Blue,
        ChannelColor::Cyan,
        ChannelColor::Magenta,
        ChannelColor::Yellow,
    ];

    /// The channel's data token: `data.luma` for a neutral channel.
    fn token(self, cx: &StyleCx<'_>) -> Rgba {
        let data = &cx.tokens.color.data;
        match self {
            ChannelColor::Neutral => data.luma,
            ChannelColor::Red => data.red,
            ChannelColor::Green => data.green,
            ChannelColor::Blue => data.blue,
            ChannelColor::Cyan => data.cyan,
            ChannelColor::Magenta => data.magenta,
            ChannelColor::Yellow => data.yellow,
        }
    }

    /// The channel's data token: `data.luma` for a neutral channel.
    pub fn tone(self, cx: &StyleCx<'_>) -> Color32 {
        color32(self.token(cx))
    }

    /// The colour of the channel's curve: `text.normal` for a neutral channel, otherwise its data token.
    pub fn curve(self, cx: &StyleCx<'_>) -> StyleColor {
        match self {
            ChannelColor::Neutral => cx.color(cx.tokens.color.text.normal),
            other => cx.color(other.token(cx)),
        }
    }

    /// The light end of the channel's gradient bars: white for a neutral channel, otherwise its data token. The
    /// dark end is black in every theme (image data).
    pub fn bar(self, cx: &StyleCx<'_>) -> Color32 {
        match self {
            ChannelColor::Neutral => Color32::from_rgb(u8::MAX, u8::MAX, u8::MAX),
            other => other.tone(cx),
        }
    }

    /// The name in story data and dumps.
    pub const fn name(self) -> &'static str {
        match self {
            ChannelColor::Neutral => "neutral",
            ChannelColor::Red => "red",
            ChannelColor::Green => "green",
            ChannelColor::Blue => "blue",
            ChannelColor::Cyan => "cyan",
            ChannelColor::Magenta => "magenta",
            ChannelColor::Yellow => "yellow",
        }
    }
}

/// How tall the bins are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HistogramScale {
    /// The tallest bin (the tallest interior bin of an RGB plot, whose edge bins are clipped pixels) at 1/1.1 of the
    /// height: Photoshop's and PhotoCraft's.
    #[default]
    Peak,
    /// Camera Raw's: the 95th-percentile column at two thirds of the height; taller spikes run off the top.
    Typical,
}

/// How a [`HistogramData`] is drawn by [`paint_bins`](super::paint_bins).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HistogramLook {
    /// One channel as a filled area in its colour.
    Tone,
    /// One grey, `text.muted` at half opacity, in every channel, to sit behind a curve.
    Backdrop,
    /// Overlapping ribbons: filled bands with exact overlap colours and an outline each.
    Rgb,
}

/// A count a histogram's bin holds.
pub trait BinCount: Copy {
    /// The count as a height before scaling.
    fn height(self) -> f32;
}

impl BinCount for u32 {
    fn height(self) -> f32 {
        self as f32
    }
}

impl BinCount for u64 {
    fn height(self) -> f32 {
        self as f32
    }
}

/// The bins of a histogram and how tall they are drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct HistogramData {
    pub(super) plot: Plot,
    pub(super) scale: HistogramScale,
}

/// What a [`HistogramData`] holds.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Plot {
    Tone { bins: Vec<f32>, channel: ChannelColor },
    Channels { channels: Vec<(Vec<f32>, ChannelColor)>, blend: Blend },
}

/// How the fills of overlapping channels combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Blend {
    /// Channel-wise sum, saturating: light on the dark scope (red over green is yellow, all three grey).
    Add,
    /// Channel-wise product: inks on paper.
    Multiply,
}

/// Channels a plot draws; the overlap masks are 32-bit.
const MAX_CHANNELS: usize = 32;

impl HistogramData {
    /// One channel's bins, drawn across the plot width, in `channel`'s colour.
    pub fn tone<B: BinCount>(bins: &[B], channel: ChannelColor) -> Self {
        Self { plot: Plot::Tone { bins: bins.iter().map(|b| b.height()).collect(), channel }, scale: HistogramScale::Peak }
    }

    /// Red, green and blue bins, drawn as ribbons. The three slices should have one length.
    pub fn rgb<B: BinCount>(channels: [&[B]; 3]) -> Self {
        let colors = [ChannelColor::Red, ChannelColor::Green, ChannelColor::Blue];
        let channels = channels.iter().zip(colors).map(|(bins, color)| (bins.iter().map(|b| b.height()).collect(), color)).collect();
        Self { plot: Plot::Channels { channels, blend: Blend::Add }, scale: HistogramScale::Peak }
    }

    /// Any number of channels, each `(bins, color)`, as opaque ribbons whose fills multiply where they overlap: the
    /// inks of a CMYK or spot-colour image. The slices should have one length.
    pub fn channels<B: BinCount>(channels: &[(&[B], ChannelColor)]) -> Self {
        let channels = channels.iter().take(MAX_CHANNELS).map(|(bins, color)| (bins.iter().map(|b| b.height()).collect(), *color)).collect();
        Self { plot: Plot::Channels { channels, blend: Blend::Multiply }, scale: HistogramScale::Peak }
    }

    /// How tall the bins are drawn; [`HistogramScale::Peak`] by default.
    #[must_use]
    pub fn scale(mut self, scale: HistogramScale) -> Self {
        self.scale = scale;
        self
    }

    /// The number of bins per channel.
    pub fn bin_count(&self) -> usize {
        match &self.plot {
            Plot::Tone { bins, .. } => bins.len(),
            Plot::Channels { channels, .. } => channels.first().map_or(0, |(bins, _)| bins.len()),
        }
    }

    /// Whether the data is in the RGB look: channels drawn as ribbons on the scope.
    pub fn is_ribbons(&self) -> bool {
        matches!(self.plot, Plot::Channels { .. })
    }
}

/// A range of whole levels (bin positions) a plot has selected, both ends included.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LevelRange {
    /// The first level.
    pub start: f32,
    /// The last level, not before `start`.
    pub end: f32,
}

impl LevelRange {
    /// The range between two levels, in either order.
    pub fn new(a: f32, b: f32) -> Self {
        Self { start: a.min(b), end: a.max(b) }
    }
}
