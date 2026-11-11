//! `ChannelsCommands` and the menus: the row's context menu (`Menu::context`) and the rows the panel adds to its
//! panel menu (`panel_rows`). Both list the commands one row kind offers.

use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::{EditEvent, Mixed};
use craft_keymap::Mods;
use craft_theme::Srgba;
use craft_ui::contract::{CommandSet, Props as _, ReorderMove, ShowContainer as _};
use craft_ui::menu::{Menu, MenuItem, PanelRow};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use std::sync::Arc;

use super::labels::ChannelsWords;
use super::model::{ChannelKind, ChannelRow, ChannelsEvent, ColorIndicates, OVERLAY_OPACITIES, OverlayPreset, targeted_row};

/// The commands of the Channels panel: its panel-menu rows, its row menu, and the keys of the list.
///
/// They are the app's commands. The panel takes the ones routed to it and reports them as
/// [`ChannelsEvent`]s; the app can also turn one into its event with [`event`](ChannelsCommands::event).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "channelsPanel", scope = "channelsPanel")]
#[non_exhaustive]
pub enum ChannelsCommands {
    /// Create a channel.
    #[command(label = "New Channel…")]
    NewChannel,
    /// Create a spot channel.
    #[command(label = "New Spot Channel…")]
    NewSpotChannel,
    /// Duplicate the targeted channel.
    #[command(label = "Duplicate Channel")]
    Duplicate,
    /// Delete the targeted channel.
    #[command(label = "Delete Channel", icon = "trash")]
    Delete,
    /// Rename the targeted alpha or spot channel in place.
    #[command(label = "Rename Channel…", default = "F2")]
    RenameChannel,
    /// Open the options of the targeted channel.
    #[command(label = "Channel Options…")]
    ChannelOptions,
    /// Merge the targeted spot channel into the colour channels.
    #[command(label = "Merge Spot Channel")]
    MergeSpot,
    /// Turn the targeted spot channel into an alpha channel.
    #[command(label = "Convert to Alpha Channel")]
    ConvertToAlpha,
    /// Leave Quick Mask.
    #[command(label = "Exit Quick Mask")]
    ExitQuickMask,
    /// The targeted channel's colour marks masked areas.
    #[command(label = "Color Indicates Masked Areas")]
    ColorIndicatesMasked,
    /// The targeted channel's colour marks selected areas.
    #[command(label = "Color Indicates Selected Areas")]
    ColorIndicatesSelected,
    /// Overlay colour red.
    #[command(label = "Red")]
    OverlayRed,
    /// Overlay colour green.
    #[command(label = "Green")]
    OverlayGreen,
    /// Overlay colour blue.
    #[command(label = "Blue")]
    OverlayBlue,
    /// Overlay colour cyan.
    #[command(label = "Cyan")]
    OverlayCyan,
    /// Overlay colour magenta.
    #[command(label = "Magenta")]
    OverlayMagenta,
    /// Overlay colour yellow.
    #[command(label = "Yellow")]
    OverlayYellow,
    /// Overlay opacity 25 percent.
    #[command(label = "25%")]
    Opacity25,
    /// Overlay opacity 50 percent.
    #[command(label = "50%")]
    Opacity50,
    /// Overlay opacity 75 percent.
    #[command(label = "75%")]
    Opacity75,
    /// Overlay opacity 100 percent.
    #[command(label = "100%")]
    Opacity100,
    /// Enable or disable the layer mask.
    #[command(label = "Toggle Layer Mask")]
    ToggleLayerMask,
    /// Delete the layer mask.
    #[command(label = "Delete Layer Mask")]
    DeleteLayerMask,
    /// Split the document into one document per channel.
    #[command(label = "Split Channels")]
    SplitChannels,
    /// Merge documents into the channels of one.
    #[command(label = "Merge Channels…")]
    MergeChannels,
    /// Load the lead channel as a selection; Shift adds, Alt subtracts, both intersect.
    #[command(
        label = "Load Channel as Selection",
        default = "Cmd+Enter",
        default = "Cmd+Shift+Enter",
        default = "Cmd+Alt+Enter",
        default = "Cmd+Shift+Alt+Enter"
    )]
    LoadSelection,
    /// Save the selection as a channel.
    #[command(label = "Save Selection as Channel")]
    SaveSelection,
    /// Move the lead alpha or spot channel up one place.
    #[command(label = "Move Channel Up", default = "Cmd+]")]
    MoveUp,
    /// Move the lead alpha or spot channel down one place.
    #[command(label = "Move Channel Down", default = "Cmd+[")]
    MoveDown,
}

impl ChannelsCommands {
    /// Every command of the set.
    pub const ALL: [ChannelsCommands; 29] = [
        ChannelsCommands::NewChannel,
        ChannelsCommands::NewSpotChannel,
        ChannelsCommands::Duplicate,
        ChannelsCommands::Delete,
        ChannelsCommands::RenameChannel,
        ChannelsCommands::ChannelOptions,
        ChannelsCommands::MergeSpot,
        ChannelsCommands::ConvertToAlpha,
        ChannelsCommands::ExitQuickMask,
        ChannelsCommands::ColorIndicatesMasked,
        ChannelsCommands::ColorIndicatesSelected,
        ChannelsCommands::OverlayRed,
        ChannelsCommands::OverlayGreen,
        ChannelsCommands::OverlayBlue,
        ChannelsCommands::OverlayCyan,
        ChannelsCommands::OverlayMagenta,
        ChannelsCommands::OverlayYellow,
        ChannelsCommands::Opacity25,
        ChannelsCommands::Opacity50,
        ChannelsCommands::Opacity75,
        ChannelsCommands::Opacity100,
        ChannelsCommands::ToggleLayerMask,
        ChannelsCommands::DeleteLayerMask,
        ChannelsCommands::SplitChannels,
        ChannelsCommands::MergeChannels,
        ChannelsCommands::LoadSelection,
        ChannelsCommands::SaveSelection,
        ChannelsCommands::MoveUp,
        ChannelsCommands::MoveDown,
    ];

    /// The overlay colour the command sets.
    pub fn overlay_color(self) -> Option<OverlayPreset> {
        match self {
            ChannelsCommands::OverlayRed => Some(OverlayPreset::Red),
            ChannelsCommands::OverlayGreen => Some(OverlayPreset::Green),
            ChannelsCommands::OverlayBlue => Some(OverlayPreset::Blue),
            ChannelsCommands::OverlayCyan => Some(OverlayPreset::Cyan),
            ChannelsCommands::OverlayMagenta => Some(OverlayPreset::Magenta),
            ChannelsCommands::OverlayYellow => Some(OverlayPreset::Yellow),
            _ => None,
        }
    }

    /// The overlay opacity in percent the command sets.
    pub fn overlay_opacity(self) -> Option<u8> {
        match self {
            ChannelsCommands::Opacity25 => Some(25),
            ChannelsCommands::Opacity50 => Some(50),
            ChannelsCommands::Opacity75 => Some(75),
            ChannelsCommands::Opacity100 => Some(100),
            _ => None,
        }
    }

    fn of_preset(preset: OverlayPreset) -> ChannelsCommands {
        match preset {
            OverlayPreset::Red => ChannelsCommands::OverlayRed,
            OverlayPreset::Green => ChannelsCommands::OverlayGreen,
            OverlayPreset::Blue => ChannelsCommands::OverlayBlue,
            OverlayPreset::Cyan => ChannelsCommands::OverlayCyan,
            OverlayPreset::Magenta => ChannelsCommands::OverlayMagenta,
            OverlayPreset::Yellow => ChannelsCommands::OverlayYellow,
        }
    }

    fn of_opacity(percent: u8) -> Option<ChannelsCommands> {
        match percent {
            25 => Some(ChannelsCommands::Opacity25),
            50 => Some(ChannelsCommands::Opacity50),
            75 => Some(ChannelsCommands::Opacity75),
            100 => Some(ChannelsCommands::Opacity100),
            _ => None,
        }
    }

    /// Whether the command applies to a targeted row of `kind` (`None`: no row targeted).
    pub fn applies_to(self, kind: Option<ChannelKind>) -> bool {
        use ChannelsCommands as C;
        let any = matches!(self, C::NewChannel | C::NewSpotChannel | C::SplitChannels | C::MergeChannels | C::SaveSelection);
        let Some(kind) = kind else { return any };
        match self {
            C::Duplicate | C::LoadSelection => true,
            C::Delete => !matches!(kind, ChannelKind::Composite | ChannelKind::Color | ChannelKind::QuickMask(_)),
            C::RenameChannel | C::MoveUp | C::MoveDown => kind.moves(),
            C::ChannelOptions => kind.has_options(),
            C::MergeSpot | C::ConvertToAlpha => matches!(kind, ChannelKind::Spot(_)),
            C::ExitQuickMask => matches!(kind, ChannelKind::QuickMask(_)),
            C::ToggleLayerMask | C::DeleteLayerMask => matches!(kind, ChannelKind::LayerMask(_)),
            C::ColorIndicatesMasked | C::ColorIndicatesSelected => matches!(kind, ChannelKind::Alpha(_) | ChannelKind::QuickMask(_)),
            c if c.overlay_color().is_some() || c.overlay_opacity().is_some() => {
                matches!(kind, ChannelKind::Alpha(_) | ChannelKind::QuickMask(_))
            }
            _ => true,
        }
    }

    /// The event the command makes over `rows` with the modifiers `mods` held; `None` when it does nothing there.
    /// Rename has no event: the panel opens the row's rename field.
    pub fn event<K: Clone + PartialEq>(self, rows: &[ChannelRow<K>], mods: Mods) -> Option<ChannelsEvent<K>> {
        let target = targeted_row(rows)?;
        match self {
            ChannelsCommands::LoadSelection => {
                Some(ChannelsEvent::LoadSelection { key: target.key.clone(), operation: SelectionOperation::from_mods(mods) })
            }
            ChannelsCommands::MoveUp | ChannelsCommands::MoveDown => move_event(rows, target, self == ChannelsCommands::MoveUp),
            ChannelsCommands::ChannelOptions => target.kind.has_options().then(|| ChannelsEvent::Options(target.key.clone())),
            ChannelsCommands::RenameChannel => None,
            command => {
                self.applies_to(Some(target.kind)).then(|| ChannelsEvent::Command { command, key: command_key(command).then(|| target.key.clone()) })
            }
        }
    }
}

/// Whether a command acts on a channel; the others act on the document.
fn command_key(command: ChannelsCommands) -> bool {
    !matches!(
        command,
        ChannelsCommands::NewChannel | ChannelsCommands::NewSpotChannel | ChannelsCommands::SplitChannels | ChannelsCommands::MergeChannels
    )
}

/// Whether alpha and spot rows may land between the places `to - 1` and `to` of `remaining` (the rows left once the
/// moved ones are out): after another alpha or spot row, before one, or before Quick Mask.
pub(super) fn lands(remaining: &[ChannelKind], to: usize) -> bool {
    let before = remaining.get(to).is_some_and(|kind| kind.moves() || matches!(kind, ChannelKind::QuickMask(_)));
    let after = to.checked_sub(1).and_then(|at| remaining.get(at)).is_some_and(|kind| kind.moves());
    before || after
}

/// The one-place move of the lead alpha or spot row.
fn move_event<K: Clone + PartialEq>(rows: &[ChannelRow<K>], lead: &ChannelRow<K>, up: bool) -> Option<ChannelsEvent<K>> {
    if !lead.kind.moves() {
        return None;
    }
    let at = rows.iter().position(|row| row.key == lead.key)?;
    let to = if up { at.checked_sub(1)? } else { at + 1 };
    let remaining: Vec<ChannelKind> = rows.iter().filter(|row| row.key != lead.key).map(|row| row.kind).collect();
    if to > remaining.len() || !lands(&remaining, to) {
        return None;
    }
    let mv = ReorderMove::new(lead.key.clone(), vec![lead.key.clone()], to, false);
    Some(ChannelsEvent::Reorder(EditEvent::set(Mixed::Mixed, mv, Arc::from(if up { "Move Channel Up" } else { "Move Channel Down" }), None)))
}

/// A line of a row's menu.
#[derive(Clone, Copy)]
enum Plan {
    Command(ChannelsCommands),
    Separator,
    Colors,
    Opacities,
}

fn overlay_plan() -> [Plan; 4] {
    [Plan::Command(ChannelsCommands::ColorIndicatesMasked), Plan::Command(ChannelsCommands::ColorIndicatesSelected), Plan::Colors, Plan::Opacities]
}

fn document_plan() -> [Plan; 5] {
    [
        Plan::Command(ChannelsCommands::NewChannel),
        Plan::Command(ChannelsCommands::NewSpotChannel),
        Plan::Separator,
        Plan::Command(ChannelsCommands::SplitChannels),
        Plan::Command(ChannelsCommands::MergeChannels),
    ]
}

fn add_commands(lines: &mut Vec<Plan>, commands: &[ChannelsCommands]) {
    lines.extend(commands.iter().map(|c| Plan::Command(*c)));
}

/// The menu lines for a row of `kind`; with no row, the document's.
fn plan(kind: Option<ChannelKind>) -> Vec<Plan> {
    use ChannelsCommands as C;
    let mut lines: Vec<Plan> = Vec::new();
    match kind {
        Some(ChannelKind::Alpha(_)) => {
            add_commands(&mut lines, &[C::Duplicate, C::Delete, C::RenameChannel, C::ChannelOptions]);
            lines.push(Plan::Separator);
            lines.extend(overlay_plan());
        }
        Some(ChannelKind::Spot(_)) => {
            add_commands(&mut lines, &[C::Duplicate, C::Delete, C::RenameChannel, C::ChannelOptions]);
            lines.push(Plan::Separator);
            add_commands(&mut lines, &[C::MergeSpot, C::ConvertToAlpha]);
        }
        Some(ChannelKind::QuickMask(_)) => {
            add_commands(&mut lines, &[C::ExitQuickMask, C::ChannelOptions]);
            lines.extend(overlay_plan());
        }
        Some(ChannelKind::LayerMask(_)) => add_commands(&mut lines, &[C::ToggleLayerMask, C::DeleteLayerMask, C::ChannelOptions]),
        Some(ChannelKind::Composite | ChannelKind::Color) => add_commands(&mut lines, &[C::Duplicate]),
        None => {}
    }
    if kind.is_some() {
        lines.push(Plan::Separator);
    }
    lines.extend(document_plan());
    lines
}

/// The rows the panel adds to its panel menu, for the targeted row of `rows`: the commands of its row menu.
pub fn panel_rows<K>(rows: &[ChannelRow<K>], words: &ChannelsWords) -> Vec<PanelRow> {
    let command = |c: ChannelsCommands| PanelRow::Command(CommandRef::from(c));
    let kind = targeted_row(rows).map(|row| row.kind);
    let mut out = Vec::new();
    for line in plan(kind) {
        match line {
            Plan::Command(c) => out.push(command(c)),
            Plan::Separator => out.push(PanelRow::Separator),
            Plan::Colors => out.push(PanelRow::Submenu(
                words.overlay_color.clone(),
                OverlayPreset::ALL.iter().map(|preset| command(ChannelsCommands::of_preset(*preset))).collect(),
            )),
            Plan::Opacities => out.push(PanelRow::Submenu(
                words.overlay_opacity.clone(),
                OVERLAY_OPACITIES.iter().filter_map(|percent| ChannelsCommands::of_opacity(*percent)).map(command).collect(),
            )),
        }
    }
    out
}

fn label_of(command: ChannelsCommands, kind: Option<ChannelKind>, words: &ChannelsWords) -> Label {
    use ChannelsCommands as C;
    let word = match command {
        C::NewChannel => &words.new_channel,
        C::NewSpotChannel => &words.new_spot_channel,
        C::Duplicate => &words.duplicate_channel,
        C::Delete => &words.delete_channel,
        C::RenameChannel => &words.rename_channel,
        C::ChannelOptions => match kind {
            Some(ChannelKind::QuickMask(_)) => &words.quick_mask_options,
            Some(ChannelKind::LayerMask(_)) => &words.mask_display_options,
            _ => &words.channel_options,
        },
        C::MergeSpot => &words.merge_spot,
        C::ConvertToAlpha => &words.convert_to_alpha,
        C::ExitQuickMask => &words.exit_quick_mask,
        C::ColorIndicatesMasked => &words.indicates_masked,
        C::ColorIndicatesSelected => &words.indicates_selected,
        C::ToggleLayerMask => match kind {
            Some(ChannelKind::LayerMask(true)) => &words.disable_layer_mask,
            _ => &words.enable_layer_mask,
        },
        C::DeleteLayerMask => &words.delete_layer_mask,
        C::SplitChannels => &words.split_channels,
        C::MergeChannels => &words.merge_channels,
        C::OverlayRed => &words.red,
        C::OverlayGreen => &words.green,
        C::OverlayBlue => &words.blue,
        C::OverlayCyan => &words.cyan,
        C::OverlayMagenta => &words.magenta,
        C::OverlayYellow => &words.yellow,
        C::LoadSelection => &words.load_selection,
        C::SaveSelection => &words.save_selection,
        _ => &words.row_menu,
    };
    word.clone()
}

/// The command a row menu run picked, from its entries.
fn checked(command: ChannelsCommands, kind: Option<ChannelKind>) -> Option<bool> {
    let indicates = match kind {
        Some(ChannelKind::Alpha(indicates) | ChannelKind::QuickMask(indicates)) => Some(indicates),
        _ => None,
    };
    match command {
        ChannelsCommands::ColorIndicatesMasked => Some(indicates == Some(ColorIndicates::MaskedAreas)),
        ChannelsCommands::ColorIndicatesSelected => Some(indicates == Some(ColorIndicates::SelectedAreas)),
        _ => None,
    }
}

/// Shows the context menu of the row `row`, hanging from `opener`, and returns the command that ran.
pub(super) fn show_row_menu<K>(ui: &mut Ui, opener: &Response, row: &ChannelRow<K>, words: &ChannelsWords) -> Option<ChannelsCommands> {
    let kind = Some(row.kind);
    let units = Env::of(ui).units();
    let lines = plan(kind);
    let shown = Menu::context(words.row_menu.clone(), opener).show(ui, |ui, _| {
        use craft_ui::contract::Show as _;
        let mut ran = None;
        for line in &lines {
            match *line {
                Plan::Separator => {
                    craft_ui::display::Divider::horizontal().show(ui);
                }
                Plan::Command(command) => {
                    let mut item = MenuItem::new(label_of(command, kind, words)).enabled(command.applies_to(kind));
                    if let Some(on) = checked(command, kind) {
                        item = item.radio_mark(on);
                    }
                    if item.show(ui).ran {
                        ran = Some(command);
                    }
                }
                Plan::Colors => {
                    Menu::submenu_row(ui, words.overlay_color.clone(), |ui, _| {
                        for preset in OverlayPreset::ALL {
                            let [r, g, b] = preset.rgb();
                            let command = ChannelsCommands::of_preset(preset);
                            let item =
                                MenuItem::new(label_of(command, kind, words)).icon(IconSpec::swatch(Srgba::from(craft_theme::Rgba::rgb(r, g, b))));
                            if item.show(ui).ran {
                                ran = Some(command);
                            }
                        }
                    });
                }
                Plan::Opacities => {
                    Menu::submenu_row(ui, words.overlay_opacity.clone(), |ui, _| {
                        for percent in OVERLAY_OPACITIES {
                            let Some(command) = ChannelsCommands::of_opacity(percent) else { continue };
                            let text = craft_i18n::fmt(words.percent.as_str(), &[("value", &units.number(f64::from(percent), 0..=0))]);
                            if MenuItem::new(text).show(ui).ran {
                                ran = Some(command);
                            }
                        }
                    });
                }
            }
        }
        ran
    });
    shown.inner.flatten()
}
