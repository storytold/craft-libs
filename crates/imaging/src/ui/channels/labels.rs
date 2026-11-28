//! `ChannelsWords`: every user-visible word of the Channels panel, and the shortcut chords the rows display.

use craft_keymap::{Chord, Key, Mods, Platform};

use super::model::{ChannelKind, ChannelRow};

craft_ui::words! {
    /// The words of the Channels panel. Channel names are the app's ([`ChannelRow::name`]).
    pub struct ChannelsWords {
        /// The list's name for a screen reader.
        list: "channelsPanel.list" = "Channels";
        /// The eye cell's name: `{row}` is the channel name.
        visibility: "channelsPanel.visibility" = "Visibility {row}";
        /// The eye's tooltip.
        toggle_visibility: "channelsPanel.toggleVisibility" = "Toggle visibility";
        /// The layer-mask row's eye tooltip.
        mask_overlay: "channelsPanel.maskOverlay" = "Show the layer mask as an overlay";
        /// A hidden row's description.
        hidden: "channelsPanel.hidden" = "Hidden";
        /// The footer buttons' tooltips and names.
        delete: "channelsPanel.delete" = "Delete current channel";
        new: "channelsPanel.new" = "Create new channel";
        save_selection: "channelsPanel.saveSelection" = "Save selection as channel";
        load_selection: "channelsPanel.loadSelection" = "Load channel as selection";
        /// The row menu's name.
        row_menu: "channelsPanel.rowMenu" = "Channel";
        /// The context menu.
        duplicate_channel: "channelsPanel.menuDuplicate" = "Duplicate Channel";
        delete_channel: "channelsPanel.menuDelete" = "Delete Channel";
        rename_channel: "channelsPanel.menuRename" = "Rename Channel…";
        channel_options: "channelsPanel.menuOptions" = "Channel Options…";
        merge_spot: "channelsPanel.menuMergeSpot" = "Merge Spot Channel";
        convert_to_alpha: "channelsPanel.menuConvertToAlpha" = "Convert to Alpha Channel";
        indicates_masked: "channelsPanel.menuIndicatesMasked" = "Color Indicates Masked Areas";
        indicates_selected: "channelsPanel.menuIndicatesSelected" = "Color Indicates Selected Areas";
        overlay_color: "channelsPanel.menuOverlayColor" = "Overlay Color";
        red: "channelsPanel.menuRed" = "Red";
        green: "channelsPanel.menuGreen" = "Green";
        blue: "channelsPanel.menuBlue" = "Blue";
        cyan: "channelsPanel.menuCyan" = "Cyan";
        magenta: "channelsPanel.menuMagenta" = "Magenta";
        yellow: "channelsPanel.menuYellow" = "Yellow";
        overlay_opacity: "channelsPanel.menuOverlayOpacity" = "Overlay Opacity";
        /// An opacity entry: `{value}` is the percentage in the locale's digits.
        percent: "channelsPanel.menuPercent" = "{value}%";
        exit_quick_mask: "channelsPanel.menuExitQuickMask" = "Exit Quick Mask";
        quick_mask_options: "channelsPanel.menuQuickMaskOptions" = "Quick Mask Options…";
        enable_layer_mask: "channelsPanel.menuEnableLayerMask" = "Enable Layer Mask";
        disable_layer_mask: "channelsPanel.menuDisableLayerMask" = "Disable Layer Mask";
        delete_layer_mask: "channelsPanel.menuDeleteLayerMask" = "Delete Layer Mask";
        mask_display_options: "channelsPanel.menuMaskDisplayOptions" = "Layer Mask Display Options…";
        new_channel: "channelsPanel.menuNewChannel" = "New Channel…";
        new_spot_channel: "channelsPanel.menuNewSpotChannel" = "New Spot Channel…";
        split_channels: "channelsPanel.menuSplit" = "Split Channels";
        merge_channels: "channelsPanel.menuMerge" = "Merge Channels…";
    }
}

/// The shortcut number each row displays, in row order: the composite 2, then each colour, alpha and spot row the
/// next number while it is at most 9; Quick Mask and layer-mask rows `None`. The panel only displays them; the app's
/// keymap owns the keys.
pub(super) fn shortcut_numbers<K>(rows: &[ChannelRow<K>]) -> Vec<Option<u8>> {
    let mut next = 2u8;
    rows.iter()
        .map(|row| match row.kind {
            ChannelKind::Composite | ChannelKind::Color | ChannelKind::Alpha(_) | ChannelKind::Spot(_) => {
                let number = next;
                next = next.saturating_add(1);
                (number <= 9).then_some(number)
            }
            ChannelKind::QuickMask(_) | ChannelKind::LayerMask(_) => None,
        })
        .collect()
}

/// The chord text of shortcut `number`: Cmd+n on macOS, Ctrl+n elsewhere.
pub(super) fn chord_text(number: u8) -> Option<String> {
    let key = Key::parse(&number.to_string())?;
    Some(Chord::new(Mods::CMD, key).display(Platform::current()))
}
