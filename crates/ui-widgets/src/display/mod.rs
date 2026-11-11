//! Display and status components: `Badge`, `Kbd`, `Progress`, `Spinner`, `Text`, `Icon`, `Picture` and `Divider`.
//!
//! None of them takes input or a Tab stop. A tooltip, or a label no text paints, gives the root a hover sense so the
//! tooltip can show.

mod badge;
mod divider;
mod icon;
mod kbd;
mod picture;
mod progress;
mod spinner;
mod text;
mod words;

pub use badge::{Badge, BadgeStyle};
pub use divider::{Divider, DividerStyle};
pub use icon::{Icon, IconStyle};
pub use kbd::{ChordWrite, Kbd, KbdStyle};
pub use picture::{Picture, PictureSize, PictureStyle};
pub use progress::{Progress, ProgressStyle};
pub use spinner::{Spinner, SpinnerSize, SpinnerStyle};
pub use text::{Elide, Shade, Text, TextStyle};
pub use words::DisplayWords;

use crate::contract::Common;
use craft_behavior::Sense;

/// The sense of a display leaf: hover when the instance has a tooltip or `label_is_tip` says its label is shown
/// as one, nothing otherwise.
fn display_sense(common: &Common, label_is_tip: bool) -> Sense {
    if common.tooltip.is_some() || (label_is_tip && !common.label.as_str().is_empty()) { Sense::HOVER } else { Sense::empty() }
}
