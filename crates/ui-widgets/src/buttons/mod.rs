//! `Button`, `IconButton`, `ToggleButton`, `SplitButton` and `Link`.

pub mod button;
pub(crate) mod face;
pub mod icon_button;
pub mod link;
pub mod split_button;
#[cfg(feature = "testing")]
mod stories;
pub mod toggle_button;
mod words;

use crate::runtime::env::EnvValue;

pub use button::{Button, ButtonOutput, ButtonStyle};
pub use face::Arrangement;
pub use icon_button::{IconButton, IconButtonStyle, PopupMark};
pub use link::{Link, LinkStyle};
pub use split_button::{SplitButton, SplitButtonStyle, SplitMenu, SplitOutput};
pub use toggle_button::{ToggleButton, ToggleButtonStyle};

/// How much of a ribbon group's content is shown. A group sets it for its content through `Overrides::values`, and
/// styles and measurements keyed on the env follow a change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue)]
#[env(style)]
#[non_exhaustive]
pub enum GroupLevel {
    /// Every control at full size, with its label.
    #[default]
    Full,
    /// Buttons drop their labels.
    Compact,
    /// The group is collapsed into one button.
    Collapsed,
}
