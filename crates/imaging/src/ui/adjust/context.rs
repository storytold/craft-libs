//! [`AdjustContext`], what every adjustment editor shares with its host, and the tone channels of
//! Levels and Curves ([`ToneSpace`], [`ToneChannel`], [`ToneChannels`]).

use craft_theme::Srgba;
use craft_ui::runtime::env::Label;

use super::AdjustWords;
use crate::ui::histogram::ChannelColor;

/// Where an adjustment editor is shown and how its sliders look: one value for every editor.
///
/// ```
/// use craft_imaging::ui::adjust::AdjustContext;
/// // Image › Adjustments › Hue/Saturation, with Preferences › Interface › Dynamic Color Sliders on.
/// let dialog = AdjustContext::dialog(true);
/// assert!(dialog.is_dialog());
/// // The same editor on an adjustment layer in the Properties panel.
/// let panel = AdjustContext::properties(true).gray(true);
/// assert!(!panel.is_dialog() && panel.is_gray());
/// ```
// pc: crates/ui-egui/src/dialogs/adjust_editors.rs:105-124 `EditorCx` (`dialog`,
// `dynamic_color_sliders`, `gray`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AdjustContext {
    dialog: bool,
    dynamic_color_sliders: bool,
    gray: bool,
}

impl AdjustContext {
    /// An editor in an Image › Adjustments dialog. `dynamic` is Preferences › Interface › Dynamic
    /// Color Sliders: colour tracks on the colour sliders, the plain track when off.
    pub fn dialog(dynamic: bool) -> Self {
        Self { dialog: true, dynamic_color_sliders: dynamic, gray: false }
    }

    /// An editor in the Properties panel (an adjustment layer). `dynamic` as in
    /// [`dialog`](Self::dialog).
    pub fn properties(dynamic: bool) -> Self {
        Self { dialog: false, dynamic_color_sliders: dynamic, gray: false }
    }

    /// A Grayscale (or Duotone, Bitmap) document: Levels and Curves list one Gray channel, the
    /// Neutral Gray picker is off, and Curves' ⌘-click on the image reads the grey mean.
    pub fn gray(mut self, gray: bool) -> Self {
        self.gray = gray;
        self
    }

    /// Whether the editor is in a dialog.
    pub fn is_dialog(&self) -> bool {
        self.dialog
    }

    /// Whether colour sliders draw their colour tracks.
    pub fn dynamic_color_sliders(&self) -> bool {
        self.dynamic_color_sliders
    }

    /// Whether the document is grey.
    pub fn is_gray(&self) -> bool {
        self.gray
    }

    /// `stops` when the colour tracks are on, else `None` (the plain track).
    // pc: crates/ui-egui/src/dialogs/adjust_editors/basic.rs:46 `track_stops`.
    pub fn track<'t>(&self, stops: &'t [Srgba]) -> Option<&'t [Srgba]> {
        self.dynamic_color_sliders.then_some(stops)
    }
}

/// The channel space a Levels or Curves value addresses.
// pc: crates/ui-egui/src/dialogs/adjust_editors.rs:127 `space_of`; engine `adjust_params.rs`
// `channel_keys`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToneSpace {
    /// RGB (and Grayscale with [`AdjustContext::gray`]): the composite, then Red, Green, Blue.
    #[default]
    Rgb,
    /// CMYK: the composite, then Cyan, Magenta, Yellow, Black.
    Cmyk,
    /// Lab: Lightness, a, b; no composite.
    Lab,
}

/// One channel of a tone editor's Channel select.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToneChannel {
    /// The composite of RGB ("RGB") or CMYK ("CMYK").
    Composite(ToneSpace),
    /// Red.
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Cyan.
    Cyan,
    /// Magenta.
    Magenta,
    /// Yellow.
    Yellow,
    /// CMYK Black.
    Black,
    /// Lab Lightness.
    Lightness,
    /// Lab a.
    A,
    /// Lab b.
    B,
    /// The one channel of a grey document.
    Gray,
    /// A channel the host names, shown exactly as given (a user-named alpha channel, a layer mask
    /// or Quick Mask isolated in the Channels panel, storytold/photocraft#5335), in `color`.
    #[non_exhaustive]
    Named {
        /// The name as the host gave it.
        name: String,
        /// The channel's colour.
        color: ChannelColor,
    },
}

impl ToneChannel {
    /// A channel the host names, shown as `name` in `color`.
    pub fn named(name: impl Into<String>, color: ChannelColor) -> Self {
        Self::Named { name: name.into(), color }
    }

    /// The channel's curve and histogram colour: Red, Green, Blue, Cyan, Magenta, Yellow their own;
    /// the composite, Black, Lightness and Gray neutral; Lab a magenta and b yellow (as
    /// [`ChannelColor`] documents).
    // pc: crates/ui-egui/src/dialogs/adjust_editors/curves.rs:69 `hist_color`, :76 `bar_color`.
    pub fn color(&self) -> ChannelColor {
        match self {
            ToneChannel::Red => ChannelColor::Red,
            ToneChannel::Green => ChannelColor::Green,
            ToneChannel::Blue => ChannelColor::Blue,
            ToneChannel::Cyan => ChannelColor::Cyan,
            ToneChannel::Magenta | ToneChannel::A => ChannelColor::Magenta,
            ToneChannel::Yellow | ToneChannel::B => ChannelColor::Yellow,
            ToneChannel::Named { color, .. } => *color,
            ToneChannel::Composite(_) | ToneChannel::Black | ToneChannel::Lightness | ToneChannel::Gray => ChannelColor::Neutral,
        }
    }

    /// The channel's name in the Channel select, from `words` ("RGB", "Red", "a"); a
    /// [`Named`](Self::Named) channel's own name.
    pub fn label(&self, words: &AdjustWords) -> Label {
        // pc: crates/ui-egui/src/dialogs/adjust_editors/curves.rs:18-41 `tone_channels` (the
        // labels), :50 `channel_picker_opts` (a user-typed name shown as entered).
        match self {
            ToneChannel::Composite(ToneSpace::Rgb) => words.rgb.clone(),
            ToneChannel::Composite(ToneSpace::Cmyk) => words.cmyk.clone(),
            ToneChannel::Composite(ToneSpace::Lab) | ToneChannel::Lightness => words.lightness.clone(),
            ToneChannel::Red => words.red.clone(),
            ToneChannel::Green => words.green.clone(),
            ToneChannel::Blue => words.blue.clone(),
            ToneChannel::Cyan => words.cyan.clone(),
            ToneChannel::Magenta => words.magenta.clone(),
            ToneChannel::Yellow => words.yellow.clone(),
            ToneChannel::Black => words.black.clone(),
            ToneChannel::A => words.lab_a.clone(),
            ToneChannel::B => words.lab_b.clone(),
            ToneChannel::Gray => words.gray.clone(),
            ToneChannel::Named { name, .. } => Label::from(name.clone()),
        }
    }
}

/// The channels a Levels or Curves panel lists, in Channel-select order; a histogram, a Levels
/// value or a curve of the host is addressed by the same index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToneChannels {
    channels: Vec<ToneChannel>,
    gray: bool,
    isolated: bool,
}

impl ToneChannels {
    /// The channels of `space` (PhotoCraft's `tone_channels`, Photoshop's list):
    ///
    /// | Space | Channels |
    /// |---|---|
    /// | RGB | RGB, Red, Green, Blue |
    /// | RGB with `gray` | Gray |
    /// | CMYK | CMYK, Cyan, Magenta, Yellow, Black |
    /// | Lab | Lightness, a, b |
    pub fn of(space: ToneSpace, gray: bool) -> Self {
        // pc: crates/ui-egui/src/dialogs/adjust_editors/curves.rs:18-41 `tone_channels`.
        let channels = match space {
            ToneSpace::Rgb if gray => vec![ToneChannel::Gray],
            ToneSpace::Rgb => vec![ToneChannel::Composite(space), ToneChannel::Red, ToneChannel::Green, ToneChannel::Blue],
            ToneSpace::Cmyk => {
                vec![ToneChannel::Composite(space), ToneChannel::Cyan, ToneChannel::Magenta, ToneChannel::Yellow, ToneChannel::Black]
            }
            ToneSpace::Lab => vec![ToneChannel::Lightness, ToneChannel::A, ToneChannel::B],
        };
        Self { gray: gray && space == ToneSpace::Rgb, channels, isolated: false }
    }

    /// One channel the host isolated (Image › Curves with one Channels-panel target selected,
    /// storytold/photocraft#5335): the select shows only it, Curves overlays no other channel, and the
    /// ⌘-click point and the pickers are the host's to refuse for a mask.
    pub fn isolated(channel: ToneChannel) -> Self {
        // pc: crates/ui-egui/src/dialogs/adjust_editors/curves.rs:251-270 (`isolated_curve`).
        Self { channels: vec![channel], gray: false, isolated: true }
    }

    /// The channels in order.
    pub fn as_slice(&self) -> &[ToneChannel] {
        &self.channels
    }

    /// How many channels there are.
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether the list is empty (never for [`of`](Self::of)).
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Whether the list is a grey document's single Gray channel.
    pub fn is_gray(&self) -> bool {
        self.gray
    }

    /// Whether the list is one isolated channel ([`isolated`](Self::isolated)).
    pub fn is_isolated(&self) -> bool {
        self.isolated
    }

    /// Whether channel `index` is a composite (RGB or CMYK), over which Curves overlays the
    /// other channels' curves.
    pub fn is_composite(&self, index: usize) -> bool {
        matches!(self.channels.get(index), Some(ToneChannel::Composite(_)))
    }
}
