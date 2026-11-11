//! `ChannelRow` and `ChannelKind`: the rows the app passes in, and what the panel reports
//! (`ChannelsEvent`, `ChannelsOutput`).

use craft_behavior::EditEvent;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_keymap::Mods;
use craft_theme::Srgba;
use craft_ui::contract::{NoValue, Outcome, ReorderMove, SweepStep};
use egui::Response;

use super::footer::ChannelsFooterButton;
use super::menu::ChannelsCommands;

/// What a channel row is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ChannelKind {
    /// The composite (RGB, CMYK, Lab): targeting it targets every colour channel.
    Composite,
    /// One colour channel (Red, Cyan, …); listed only when there is more than one and the
    /// document isn't Multichannel (the app's rule).
    Color,
    /// An alpha channel with its Color Indicates setting.
    Alpha(ColorIndicates),
    /// A spot channel with its ink (encoded sRGB): italic, with a swatch of the ink at the thumbnail's corner.
    Spot(Srgba),
    /// Quick Mask, while it is on, with its Color Indicates setting: italic, last.
    QuickMask(ColorIndicates),
    /// The selected layer's mask, a temporary channel listed while the layer is selected: italic,
    /// named "{layer} Mask" by the app; the flag says the mask is enabled (its menu offers Disable, else Enable).
    LayerMask(bool),
}

impl ChannelKind {
    /// Alpha and spot channels rename in place (F2, a double click on the name).
    pub fn renames(self) -> bool {
        matches!(self, ChannelKind::Alpha(_) | ChannelKind::Spot(_))
    }

    /// Alpha and spot channels reorder (drag, Cmd/Ctrl+] and [).
    pub fn moves(self) -> bool {
        self.renames()
    }

    /// Alpha, spot, Quick Mask and layer-mask rows have Channel Options (a double click on the
    /// thumbnail, Enter).
    pub fn has_options(self) -> bool {
        !matches!(self, ChannelKind::Composite | ChannelKind::Color)
    }

    /// Spot, Quick Mask and layer-mask rows draw their name italic (temporary or special
    /// channels, PhotoCraft).
    pub fn italic(self) -> bool {
        matches!(self, ChannelKind::Spot(_) | ChannelKind::QuickMask(_) | ChannelKind::LayerMask(_))
    }
}

/// Channel Options › Color Indicates; it lives in the egui-free [`crate::selection`] with the
/// Quick Mask rules.
pub use crate::selection::ColorIndicates;

/// One of the six overlay colours of the context menu's Overlay Color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OverlayPreset {
    /// `#ff0000`.
    Red,
    /// `#00c000`.
    Green,
    /// `#0050ff`.
    Blue,
    /// `#00d0e0`.
    Cyan,
    /// `#e000c0`.
    Magenta,
    /// `#f0d000`.
    Yellow,
}

impl OverlayPreset {
    /// Every preset, in the menu's order.
    pub const ALL: [OverlayPreset; 6] =
        [OverlayPreset::Red, OverlayPreset::Green, OverlayPreset::Blue, OverlayPreset::Cyan, OverlayPreset::Magenta, OverlayPreset::Yellow];

    /// The preset's colour, 8-bit sRGB (PhotoCraft's values, `channels_panel.rs:584-589`).
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            OverlayPreset::Red => [0xff, 0x00, 0x00],
            OverlayPreset::Green => [0x00, 0xc0, 0x00],
            OverlayPreset::Blue => [0x00, 0x50, 0xff],
            OverlayPreset::Cyan => [0x00, 0xd0, 0xe0],
            OverlayPreset::Magenta => [0xe0, 0x00, 0xc0],
            OverlayPreset::Yellow => [0xf0, 0xd0, 0x00],
        }
    }
}

/// The overlay opacities of the context menu's Overlay Opacity, in percent.
pub const OVERLAY_OPACITIES: [u8; 4] = [25, 50, 75, 100];

/// One row of the Channels panel, built by the app once per pass in display order: Composite,
/// the colour channels, the layer mask, the alpha and spot channels, Quick Mask (PhotoCraft's).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChannelRow<K> {
    /// The app's key of the channel: the key of the row, its thumbnail and its events.
    pub key: K,
    /// What the channel is.
    pub kind: ChannelKind,
    /// The name as shown: the app translates built-in names (Red, Quick Mask) and keeps alpha
    /// and spot names verbatim.
    pub name: String,
    /// The eye: off leaves the cell empty.
    pub visible: bool,
    /// The app's highlight: the targeted channel, or every colour row while the composite is
    /// targeted (PhotoCraft).
    pub targeted: bool,
}

impl<K> ChannelRow<K> {
    /// A visible, untargeted row.
    pub fn new(key: K, kind: ChannelKind, name: impl Into<String>) -> Self {
        Self { key, kind, name: name.into(), visible: true, targeted: false }
    }
}

/// The row the footer's Delete and Load act on: the targeted layer-mask, alpha, spot or Quick
/// Mask row; else the composite when it is targeted; else the one targeted colour row; `None`
/// with no rows or no target.
pub fn targeted_row<K>(rows: &[ChannelRow<K>]) -> Option<&ChannelRow<K>> {
    let targeted = |kinds: fn(ChannelKind) -> bool| rows.iter().find(|row| row.targeted && kinds(row.kind));
    targeted(|kind| !matches!(kind, ChannelKind::Composite | ChannelKind::Color))
        .or_else(|| targeted(|kind| matches!(kind, ChannelKind::Composite)))
        .or_else(|| targeted(|kind| matches!(kind, ChannelKind::Color)))
}

/// What the Channels panel reports, in the order it happened.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ChannelsEvent<K> {
    /// A click on a row (not with Cmd/Ctrl), or ↑ ↓ to a row: the app targets the channel
    /// (PhotoCraft: Quick Mask targets the composite).
    Target(K),
    /// An eye click or a sweep over several eyes, as the step to apply: `Flip` sets the eye to the step's value as
    /// the pointer crosses it; Esc and focus loss give `Restore` for every eye flipped, with the value from before.
    /// Alt is ignored. The app groups a sweep as one undo step.
    Visibility(SweepStep<K>),
    /// Cmd/Ctrl-click on a row, or Cmd/Ctrl+Enter on the lead: load the channel as a selection
    /// with the operation of Shift and Alt.
    #[non_exhaustive]
    LoadSelection {
        /// The channel.
        key: K,
        /// New, add (Shift), subtract (Alt), intersect (Shift+Alt).
        operation: SelectionOperation,
    },
    /// A rename committed (alpha and spot rows): never empty after trimming, never unchanged.
    #[non_exhaustive]
    Rename {
        /// The channel.
        key: K,
        /// The new name, trimmed.
        text: String,
    },
    /// A double click on the thumbnail, Enter, or Channel Options… in a menu, on a row that
    /// [has options](ChannelKind::has_options): the app opens its Channel Options dialog.
    Options(K),
    /// An alpha or spot row moved: a drag (`Begin`, `Commit` or `Cancel`) or Cmd/Ctrl+] and [
    /// (`Set`). Targets: before or after another alpha or spot row, before Quick Mask.
    Reorder(EditEvent<ReorderMove<K>>),
    /// A click on a footer button; `rows` holds [`targeted_row`]'s key (empty without rows). `mods` are the
    /// modifiers held at the click: Alt on New asks for the channel's options first.
    #[non_exhaustive]
    Footer {
        /// The button.
        button: ChannelsFooterButton,
        /// The rows it acts on.
        rows: Vec<K>,
        /// The modifiers held at the click.
        mods: Mods,
    },
    /// A row dropped on a footer button: New duplicates it, Delete deletes it (the app asks first
    /// for a layer mask), Load Channel as Selection loads it. Refused drops report nothing.
    #[non_exhaustive]
    FooterDrop {
        /// The button.
        button: ChannelsFooterButton,
        /// The dropped rows: the row the pointer grabbed.
        rows: Vec<K>,
    },
    /// A command of the set ran: a pick in the row's context menu, a key, or a panel-menu row routed to the panel.
    #[non_exhaustive]
    Command {
        /// The command.
        command: ChannelsCommands,
        /// The row it was opened on (the targeted row for the panel menu); `None` for the
        /// commands that act on the document (New Channel…, Split Channels).
        key: Option<K>,
    },
}

/// What [`ChannelsPanel::show`](super::ChannelsPanel::show) returns.
#[derive(Debug)]
#[must_use = "the app applies the panel's events"]
#[non_exhaustive]
pub struct ChannelsOutput<K> {
    /// What the user did this pass, in order.
    pub events: Vec<ChannelsEvent<K>>,
    /// The panel.
    pub response: Response,
}

impl<K> Outcome for ChannelsOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
