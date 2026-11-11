//! The panel's commands, its panel-menu rows, the row menu and the eye's label-colour menu.

use craft_theme::{LabelColor, Srgba};
use craft_ui::contract::{CommandSet, Entry};
use craft_ui::list::{ListWords, label_color_label};
use craft_ui::menu::{Menu, PanelRow};
use craft_ui::runtime::commands::CommandRef;
use egui::{Response, Ui};
use std::sync::Arc;

use super::LayersPanelWords;
use super::Session;
use super::context::PanelContext;
use super::drag::stacking_event;
use super::events::{LayersEvent, MenuTarget};
use crate::ui::options::LayersSettings;
use crate::ui::thumbs::ThumbContents;
use crate::ui::tree::LayerTree;

/// The commands of the Layers panel: its panel-menu rows, the stacking keys and the effects keys.
///
/// They are the app's commands. Settings commands run with
/// [`settings_action`](LayersPanelCommands::settings_action); the others become events with
/// [`event`](LayersPanelCommands::event), which the app applies like the panel's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "layersPanel")]
#[non_exhaustive]
pub enum LayersPanelCommands {
    /// Show or hide the filter bar.
    #[command(label = "Show Filters")]
    ShowFilters,
    /// Open the Panel Options dialog.
    #[command(label = "Panel Options…")]
    PanelOptions,
    /// Close every group.
    #[command(label = "Collapse All Groups")]
    CollapseAll,
    /// Thumbnails show the layer's bounds instead of the document.
    #[command(label = "Clip Thumbnails to Layer Bounds")]
    ClipThumbnails,
    /// Move the selected layers up one place.
    #[command(label = "Bring Forward", default = "Cmd+]")]
    MoveUp,
    /// Move the selected layers down one place.
    #[command(label = "Send Backward", default = "Cmd+[")]
    MoveDown,
    /// Move the selected layers to the top.
    #[command(label = "Bring to Front", default = "Cmd+Shift+]")]
    MoveToTop,
    /// Move the selected layers to the bottom.
    #[command(label = "Send to Back", default = "Cmd+Shift+[")]
    MoveToBottom,
    /// Open the effects and Smart Filter rows of the active layer.
    #[command(label = "Show Effects", default = "Shift+Right")]
    ShowEffects,
    /// Close the effects and Smart Filter rows of the active layer.
    #[command(label = "Hide Effects", default = "Shift+Left")]
    HideEffects,
    /// Open the effects and Smart Filter rows of every layer.
    #[command(label = "Show All Effects", default = "Shift+Alt+Right")]
    ShowAllEffects,
    /// Close the effects and Smart Filter rows of every layer.
    #[command(label = "Hide All Effects", default = "Shift+Alt+Left")]
    HideAllEffects,
}

/// What a settings command asks the app to do besides the change it made to the settings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayersMenuAction {
    /// The settings changed: the app saves them.
    SettingsChanged,
    /// Show the Layers panel options dialog.
    OpenOptions,
    /// Close every group.
    CollapseAll,
}

impl LayersPanelCommands {
    /// Runs a settings command on `settings`; `None` for a command that is not one.
    pub fn settings_action(self, settings: &mut LayersSettings) -> Option<LayersMenuAction> {
        match self {
            LayersPanelCommands::ShowFilters => {
                settings.show_filters = !settings.show_filters;
                Some(LayersMenuAction::SettingsChanged)
            }
            LayersPanelCommands::ClipThumbnails => {
                settings.options.thumbnail_contents = match settings.options.thumbnail_contents {
                    ThumbContents::LayerBounds => ThumbContents::EntireDocument,
                    ThumbContents::EntireDocument => ThumbContents::LayerBounds,
                };
                Some(LayersMenuAction::SettingsChanged)
            }
            LayersPanelCommands::PanelOptions => Some(LayersMenuAction::OpenOptions),
            LayersPanelCommands::CollapseAll => Some(LayersMenuAction::CollapseAll),
            _ => None,
        }
    }

    /// The event a stacking or effects command makes over `tree`'s selection; `None` when there is
    /// nothing to do. `label` names the undo step of a move.
    pub fn event<T: LayerTree>(self, tree: &T, label: impl Into<Arc<str>>) -> Option<LayersEvent<T::Id>> {
        let fx = |open: bool, all: bool| {
            let layer = tree.selection().active?;
            let info = tree.info(&layer);
            (info.has_effects || info.has_smart_filters).then_some(LayersEvent::FxOpen { layer, open, all })
        };
        match self {
            LayersPanelCommands::MoveUp | LayersPanelCommands::MoveDown | LayersPanelCommands::MoveToTop | LayersPanelCommands::MoveToBottom => {
                stacking_event(tree, self, label.into())
            }
            LayersPanelCommands::ShowEffects => fx(true, false),
            LayersPanelCommands::HideEffects => fx(false, false),
            LayersPanelCommands::ShowAllEffects => fx(true, true),
            LayersPanelCommands::HideAllEffects => fx(false, true),
            _ => None,
        }
    }
}

/// The rows the panel adds to its panel menu: Filter Options, Panel Options, Collapse All Groups and
/// Clip Thumbnails to Layer Bounds.
pub fn panel_rows(words: &LayersPanelWords) -> Vec<PanelRow> {
    let command = |command: LayersPanelCommands| PanelRow::Command(CommandRef::from(command));
    vec![
        PanelRow::Submenu(words.filter_options.clone(), vec![command(LayersPanelCommands::ShowFilters)]),
        command(LayersPanelCommands::PanelOptions),
        PanelRow::Separator,
        command(LayersPanelCommands::CollapseAll),
        PanelRow::Separator,
        command(LayersPanelCommands::ClipThumbnails),
    ]
}

/// The menu a context request opened, kept until the next request.
#[derive(Clone, Debug)]
pub(super) enum OpenMenu<Id> {
    Row(MenuTarget<Id>),
    Eye { rows: Vec<Id>, current: Option<LabelColor> },
}

/// The host's menu for a row: its entries for where it was opened.
pub(super) type RowMenu<'a, Id> = dyn Fn(&MenuTarget<Id>) -> Vec<Entry<String>> + 'a;

fn eye_entries(words: &ListWords, current: Option<LabelColor>) -> Vec<Entry<Option<LabelColor>>> {
    let mut entries = vec![Entry::new(None, label_color_label(None, words)).checked(current.is_none()), Entry::separator()];
    for color in LabelColor::ALL {
        let entry =
            Entry::new(Some(color), label_color_label(Some(color), words)).checked(current == Some(color)).swatch(Srgba::from(color.swatch()));
        entries.push(entry);
    }
    entries
}

/// Shows the menu the last context request chose, hanging from `opener` (the list). A right-click
/// on a row opens it at the pointer.
pub(super) fn show_row_menus<T: LayerTree>(
    ui: &mut Ui,
    cx: &PanelContext<'_, T>,
    opener: &Response,
    row_menu: Option<&RowMenu<'_, T::Id>>,
    session: &Session<T::Id>,
) -> Vec<LayersEvent<T::Id>> {
    let words = cx.words;
    match session.menu.clone() {
        Some(OpenMenu::Row(target)) => {
            let Some(menu) = row_menu else { return Vec::new() };
            let entries = menu(&target);
            let picked = Menu::context(words.row_menu.clone(), opener).pick(ui, &entries).picked;
            picked.map(|command| LayersEvent::Menu { target, command }).into_iter().collect()
        }
        Some(OpenMenu::Eye { rows, current }) => {
            let entries = eye_entries(cx.list_words, current);
            let picked = Menu::context(words.label_color_menu.clone(), opener).pick(ui, &entries).picked;
            picked.map(|color| LayersEvent::LabelColor { rows, color }).into_iter().collect()
        }
        None => Vec::new(),
    }
}
