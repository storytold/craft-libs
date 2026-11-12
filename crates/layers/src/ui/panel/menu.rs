//! The panel's commands and keys, its panel-menu rows, the row menu with its Label Color submenu and the eye's
//! label-colour menu.

use craft_theme::LabelColor;
use craft_ui::contract::{CommandSet, Entry, Show as _, ShowContainer as _};
use craft_ui::display::Divider;
use craft_ui::list::{ListWords, label_color_label};
use craft_ui::menu::{Menu, MenuItem, PanelRow, entry_rows};
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
use crate::ui::tree::{LayerRowKey, LayerTree};

/// The commands of the Layers panel: its panel-menu rows and the stacking keys.
///
/// They are the app's commands; the effects keys are the panel's own ([`LayersPanelKeys`]). Settings commands run with
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
}

/// The effects keys of the Layers panel, on the panel's own scope: they work while focus is anywhere in the panel, and
/// the panel reports them as [`LayersEvent::FxOpen`]. In the rows they are Shift with Left and Right, which the list
/// leaves to the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "layersPanel", scope = "layersPanel")]
#[non_exhaustive]
pub enum LayersPanelKeys {
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

    /// The event a stacking command makes over `tree`'s selection; `None` when there is nothing to do. `label` names
    /// the undo step of a move.
    pub fn event<T: LayerTree>(self, tree: &T, label: impl Into<Arc<str>>) -> Option<LayersEvent<T::Id>> {
        match self {
            LayersPanelCommands::MoveUp | LayersPanelCommands::MoveDown | LayersPanelCommands::MoveToTop | LayersPanelCommands::MoveToBottom => {
                stacking_event(tree, self, label.into())
            }
            _ => None,
        }
    }
}

impl LayersPanelKeys {
    /// The event the key makes over `tree`'s active layer; `None` when it has no effects or Smart Filters.
    pub(super) fn event<T: LayerTree>(self, tree: &T) -> Option<LayersEvent<T::Id>> {
        let (open, all) = match self {
            LayersPanelKeys::ShowEffects => (true, false),
            LayersPanelKeys::HideEffects => (false, false),
            LayersPanelKeys::ShowAllEffects => (true, true),
            LayersPanelKeys::HideAllEffects => (false, true),
        };
        let layer = tree.selection().active?;
        let info = tree.info(&layer);
        (info.has_effects || info.has_smart_filters).then_some(LayersEvent::FxOpen { layer, open, all })
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

/// The colour `rows` share, `None` when they have none or differ: the check in the label colour menus.
pub(super) fn shared_label<T: LayerTree>(tree: &T, rows: &[T::Id]) -> Option<LabelColor> {
    let mut labels = rows.iter().map(|id| tree.info(id).label);
    let first = labels.next().flatten();
    if labels.all(|label| label == first) { first } else { None }
}

/// The label colour rows of the eye column's menu and the row menu's Label Color submenu: None, a rule, then each
/// colour on its own tint, the current one checked. Returns the colour a row picked.
fn eye_rows(ui: &mut Ui, words: &ListWords, current: Option<LabelColor>) -> Option<Option<LabelColor>> {
    let mut picked = None;
    if MenuItem::new(label_color_label(None, words)).check(current.is_none()).show(ui).ran {
        picked = Some(None);
    }
    Divider::horizontal().show(ui);
    for color in LabelColor::ALL {
        if MenuItem::new(label_color_label(Some(color), words)).check(current == Some(color)).tint(color.swatch()).show(ui).ran {
            picked = Some(Some(color));
        }
    }
    picked
}

/// What a row menu pick ran: the host's command, or a colour of the Label Color submenu.
enum RowPick {
    Command(String),
    Color(Option<LabelColor>),
}

/// The layers the row menu's Label Color submenu colours: every selected layer when the menu acts on the selection,
/// else the row's layer. `None` where the submenu is not shown: on a thumbnail, whose menu is the host's own, and on
/// effects and Smart Filter rows.
fn label_rows<Id: Clone>(target: &MenuTarget<Id>, selected: &[Id]) -> Option<Vec<Id>> {
    let LayerRowKey::Layer(layer) = &target.row else { return None };
    if target.part.is_some() {
        return None;
    }
    Some(if target.on_selection { selected.to_vec() } else { vec![layer.clone()] })
}

/// Shows the menu the last context request chose, hanging from `opener` (the list). A right-click
/// on a row opens it at the pointer. With `label_colors`, a layer row's menu ends with the Label Color submenu.
pub(super) fn show_row_menus<T: LayerTree>(
    ui: &mut Ui,
    cx: &PanelContext<'_, T>,
    opener: &Response,
    row_menu: Option<&RowMenu<'_, T::Id>>,
    label_colors: bool,
    session: &Session<T::Id>,
) -> Vec<LayersEvent<T::Id>> {
    let words = cx.words;
    match session.menu.clone() {
        Some(OpenMenu::Row(target)) => {
            let Some(menu) = row_menu else { return Vec::new() };
            let entries = menu(&target);
            let colored = if label_colors { label_rows(&target, &cx.selection.layers) } else { None };
            let current = colored.as_deref().and_then(|rows| shared_label(cx.tree, rows));
            let shown = Menu::context(words.row_menu.clone(), opener).show(ui, |ui, _| {
                let ran = entry_rows(ui, &entries).and_then(|at| entries.get(at)).and_then(Entry::value).cloned();
                let mut color = None;
                if colored.is_some() {
                    let _ = Menu::submenu_row(ui, words.label_color_menu.clone(), |ui, _| {
                        if let Some(picked) = eye_rows(ui, cx.list_words, current) {
                            color = Some(picked);
                        }
                    });
                }
                ran.map(RowPick::Command).or(color.map(RowPick::Color))
            });
            match shown.inner.flatten() {
                Some(RowPick::Command(command)) => vec![LayersEvent::Menu { target, command }],
                Some(RowPick::Color(color)) => colored.map(|rows| LayersEvent::LabelColor { rows, color }).into_iter().collect(),
                None => Vec::new(),
            }
        }
        Some(OpenMenu::Eye { rows, current }) => {
            let picked = Menu::context(words.label_color_menu.clone(), opener).show(ui, |ui, _| eye_rows(ui, cx.list_words, current)).inner.flatten();
            picked.map(|color| LayersEvent::LabelColor { rows, color }).into_iter().collect()
        }
        None => Vec::new(),
    }
}
