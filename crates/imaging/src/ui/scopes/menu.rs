//! [`ScopeCommand`]: the scope panel's menu rows and keys, and [`ScopePanel::panel_rows`] and
//! [`ScopePanel::run_command`].

use super::panel::{ScopePanel, ScopePanelState, ScopeView};
use craft_ui::contract::CommandSet;
use craft_ui::menu::PanelRow;
use craft_ui::runtime::commands::{CommandRef, CommandState};

/// The commands of the scope panel, run on the panel that holds the focus: its panel-menu rows, its context-menu rows
/// and the two clipping-warning keys. S, the host's, is not among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "scopes", scope = "panel.scopes")]
#[non_exhaustive]
pub enum ScopeCommand {
    /// Recount at cache level 1.
    #[command(label = "Uncached Refresh")]
    Refresh,
    /// The plot alone.
    #[command(label = "Compact View", kind = "radio", group = "scopes.view")]
    CompactView,
    /// The Channel row, the plot, the Source row and the statistics.
    #[command(label = "Expanded View", kind = "radio", group = "scopes.view")]
    ExpandedView,
    /// Expanded plus one plot per colour channel.
    #[command(label = "All Channels View", kind = "radio", group = "scopes.view")]
    AllChannelsView,
    /// Show the statistics grid.
    #[command(label = "Show Statistics", kind = "toggle")]
    ShowStatistics,
    /// Single channels in their colour in Expanded and All Channels View.
    #[command(label = "Show Channels in Color", kind = "toggle")]
    ShowChannelsInColor,
    /// Read the hover sample as Lab.
    #[command(label = "Show Lab Color Readouts", kind = "toggle")]
    LabReadouts,
    /// Show the vectorscope under the plot.
    #[command(label = "Show Vectorscope", kind = "toggle")]
    ShowVectorscope,
    /// Count the vectorscope from the selection.
    #[command(label = "Show Selected Region", kind = "toggle")]
    SelectedRegion,
    /// Put red at 3 o'clock.
    #[command(label = "Show Red at 3 o'clock", kind = "toggle")]
    RedRight,
    /// Show the skin-tone line.
    #[command(label = "Show Skin Tone Indicator", kind = "toggle")]
    SkinTone,
    /// Toggle the shadow clipping warning.
    #[command(label = "Shadow Clipping Warning", kind = "toggle", default = "U")]
    ShadowWarning,
    /// Toggle the highlight clipping warning.
    #[command(label = "Highlight Clipping Warning", kind = "toggle", default = "O")]
    HighlightWarning,
}

/// What a chosen command asks the app to do beyond the settings change it made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScopeMenuAction {
    /// The command changed [`ScopeSettings`](super::ScopeSettings): persist them.
    SettingsChanged,
    /// Uncached Refresh: recount at cache level 1.
    Refresh,
}

impl ScopeCommand {
    /// The commands of the scope menu (right-click on a plot, the Menu key, Shift+F10): Show Lab Color Readouts, Show
    /// Vectorscope; with the vectorscope on, Show Selected Region (only when `has_selection`), Show Red at 3 o'clock
    /// and Show Skin Tone Indicator.
    pub fn context_menu(state: &ScopePanelState, has_selection: bool) -> Vec<ScopeCommand> {
        let mut commands = vec![ScopeCommand::LabReadouts, ScopeCommand::ShowVectorscope];
        if state.settings.vectorscope {
            if has_selection {
                commands.push(ScopeCommand::SelectedRegion);
            }
            commands.extend([ScopeCommand::RedRight, ScopeCommand::SkinTone]);
        }
        commands
    }

    /// The state of every command: radio and check marks from the settings.
    pub(super) fn states(state: &ScopePanelState, has_selection: bool) -> Vec<(ScopeCommand, CommandState)> {
        let settings = &state.settings;
        let checked = |on: bool| CommandState::new().checked(on);
        vec![
            (ScopeCommand::Refresh, CommandState::new()),
            (ScopeCommand::CompactView, checked(settings.view == ScopeView::Compact)),
            (ScopeCommand::ExpandedView, checked(settings.view == ScopeView::Expanded)),
            (ScopeCommand::AllChannelsView, checked(settings.view == ScopeView::AllChannels)),
            (ScopeCommand::ShowStatistics, checked(settings.show_statistics)),
            (ScopeCommand::ShowChannelsInColor, checked(settings.show_channels_in_color)),
            (ScopeCommand::LabReadouts, checked(settings.lab_readouts)),
            (ScopeCommand::ShowVectorscope, checked(settings.vectorscope)),
            (ScopeCommand::SelectedRegion, checked(settings.selected_region).enabled(has_selection)),
            (ScopeCommand::RedRight, checked(settings.red_right)),
            (ScopeCommand::SkinTone, checked(settings.skin_tone)),
            (ScopeCommand::ShadowWarning, checked(settings.shadow_warning)),
            (ScopeCommand::HighlightWarning, checked(settings.highlight_warning)),
        ]
    }
}

impl ScopePanel<'_> {
    /// The panel menu's rows for the docking host: Uncached Refresh; a separator; Compact View, Expanded View, All
    /// Channels View (radio marks); a separator; Show Statistics, Show Channels in Color (check marks). A row's label,
    /// mark and enabled state come from the command, which the panel publishes each pass. Run a chosen row with
    /// [`run_command`](Self::run_command).
    pub fn panel_rows() -> Vec<PanelRow> {
        let command = |command: ScopeCommand| PanelRow::Command(CommandRef::from(command));
        vec![
            command(ScopeCommand::Refresh),
            PanelRow::Separator,
            command(ScopeCommand::CompactView),
            command(ScopeCommand::ExpandedView),
            command(ScopeCommand::AllChannelsView),
            PanelRow::Separator,
            command(ScopeCommand::ShowStatistics),
            command(ScopeCommand::ShowChannelsInColor),
        ]
    }

    /// Runs a command on `state`: a view, a check, a warning or Refresh.
    pub fn run_command(state: &mut ScopePanelState, command: ScopeCommand) -> Option<ScopeMenuAction> {
        let settings = &mut state.settings;
        let flip = |flag: &mut bool| *flag = !*flag;
        match command {
            ScopeCommand::Refresh => return Some(ScopeMenuAction::Refresh),
            ScopeCommand::CompactView => settings.view = ScopeView::Compact,
            ScopeCommand::ExpandedView => settings.view = ScopeView::Expanded,
            ScopeCommand::AllChannelsView => settings.view = ScopeView::AllChannels,
            ScopeCommand::ShowStatistics => flip(&mut settings.show_statistics),
            ScopeCommand::ShowChannelsInColor => flip(&mut settings.show_channels_in_color),
            ScopeCommand::LabReadouts => flip(&mut settings.lab_readouts),
            ScopeCommand::ShowVectorscope => flip(&mut settings.vectorscope),
            ScopeCommand::SelectedRegion => flip(&mut settings.selected_region),
            ScopeCommand::RedRight => flip(&mut settings.red_right),
            ScopeCommand::SkinTone => flip(&mut settings.skin_tone),
            ScopeCommand::ShadowWarning => flip(&mut settings.shadow_warning),
            ScopeCommand::HighlightWarning => flip(&mut settings.highlight_warning),
        }
        Some(ScopeMenuAction::SettingsChanged)
    }
}
