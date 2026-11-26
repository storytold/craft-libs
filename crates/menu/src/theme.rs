//! The Appearance menu: "Sync with System", the light and dark themes as radio rows, and a row
//! that steps to the next mode.
//!
//! The Mac layout puts the submenu in the app menu ([`crate::ItemRole::Appearance`]); Windows and
//! Linux keep it where the app puts it. Choosing a row reports [`THEME_COMMAND`] with the choice;
//! the app stores it and installs the theme. The app passes the theme rows, so this crate needs
//! no theme store.

use craft_keymap::CommandRef;
use craft_theme::ThemeId;

use crate::model::{Item, ItemRole, Node, tidy};

/// The command every Appearance row runs: `{"mode":"system"}`, `{"mode":"next"}` or
/// `{"theme": "<id>", "dark": <bool>}`.
pub const THEME_COMMAND: &str = "app.theme";

/// What an Appearance row chose.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThemeChoice {
    /// Follow the system's light or dark mode.
    System,
    /// A theme in its light (`dark: false`) or dark mode.
    #[non_exhaustive]
    Theme {
        /// The theme.
        id: ThemeId,
        /// Whether its dark mode is chosen.
        dark: bool,
    },
    /// Step to the next mode.
    NextMode,
}

impl ThemeChoice {
    /// The theme `id` in its dark mode when `dark`, else its light mode.
    pub fn theme(id: ThemeId, dark: bool) -> Self {
        ThemeChoice::Theme { id, dark }
    }

    /// The choice a command made, `None` for any other command or an unknown parameter.
    pub fn from_command(command: &CommandRef) -> Option<ThemeChoice> {
        if command.id != THEME_COMMAND {
            return None;
        }
        if let Some(mode) = command.params.get("mode") {
            return match mode.as_str()? {
                "system" => Some(ThemeChoice::System),
                "next" => Some(ThemeChoice::NextMode),
                _ => None,
            };
        }
        let id = ThemeId::parse(command.params.get("theme")?.as_str()?).ok()?;
        let dark = command.params.get("dark")?.as_bool()?;
        Some(ThemeChoice::Theme { id, dark })
    }

    /// The command that makes this choice.
    pub fn command(&self) -> CommandRef {
        let params = match self {
            ThemeChoice::System => serde_json::json!({ "mode": "system" }),
            ThemeChoice::NextMode => serde_json::json!({ "mode": "next" }),
            ThemeChoice::Theme { id, dark } => serde_json::json!({ "theme": id.as_str(), "dark": dark }),
        };
        CommandRef::with_params(THEME_COMMAND, params)
    }
}

/// One theme row: the theme, its mode and its shown name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ThemeRow {
    /// The theme.
    pub id: ThemeId,
    /// Whether the row is the theme's dark mode.
    pub dark: bool,
    /// The name shown.
    pub name: String,
}

impl ThemeRow {
    /// A row for `id` in its light or dark mode, shown as `name`.
    pub fn new(id: ThemeId, dark: bool, name: impl Into<String>) -> ThemeRow {
        ThemeRow { id, dark, name: name.into() }
    }
}

/// The Appearance submenu.
///
/// Rows: Sync with System, a separator, the light rows, a separator, the dark rows, a separator and
/// the mode toggle. A separator shows only between non-empty groups. The current choice is the
/// selected radio row (a check mark on macOS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeMenu {
    rows: Vec<ThemeRow>,
    current: ThemeChoice,
    favorites: Vec<ThemeId>,
}

impl ThemeMenu {
    /// The menu of `rows` (light and dark in the order shown), with `current` selected.
    pub fn new(rows: Vec<ThemeRow>, current: ThemeChoice) -> Self {
        ThemeMenu { rows, current, favorites: Vec::new() }
    }

    /// The themes [`ThemeMenu::favorite_items`] lists.
    pub fn favorites(mut self, ids: &[ThemeId]) -> Self {
        self.favorites = ids.to_vec();
        self
    }

    fn row_item(&self, row: &ThemeRow) -> Node {
        let on = matches!(&self.current, ThemeChoice::Theme { id, dark } if *id == row.id && *dark == row.dark);
        Node::Item(Item::new(ThemeChoice::Theme { id: row.id.clone(), dark: row.dark }.command(), row.name.as_str()).radio(on))
    }

    /// The light rows, then the dark rows, that pass `keep`, with a separator between the groups.
    fn mode_groups(&self, keep: impl Fn(&ThemeRow) -> bool) -> Vec<Node> {
        let group = |dark: bool| self.rows.iter().filter(|r| r.dark == dark && keep(r)).map(|r| self.row_item(r)).collect::<Vec<_>>();
        let mut nodes = group(false);
        nodes.push(Node::Separator);
        nodes.extend(group(true));
        nodes
    }

    /// The rows as radio items running [`THEME_COMMAND`], with the mode toggle last.
    pub fn items(&self, labels: &ThemeLabels) -> Vec<Node> {
        let system = ThemeChoice::System;
        let mut nodes = vec![Node::Item(Item::new(system.command(), labels.system.as_str()).radio(self.current == system)), Node::Separator];
        nodes.extend(self.mode_groups(|_| true));
        nodes.push(Node::Separator);
        nodes.push(Node::Item(Item::new(ThemeChoice::NextMode.command(), labels.toggle.as_str())));
        tidy(&mut nodes);
        nodes
    }

    /// The rows for a quick menu: the Appearance Settings row running `settings`, then the
    /// favourite themes (see [`ThemeMenu::favorites`]), light then dark.
    pub fn favorite_items(&self, labels: &ThemeLabels, settings: CommandRef) -> Vec<Node> {
        let mut nodes = vec![Node::Item(Item::new(settings, labels.settings.as_str())), Node::Separator];
        nodes.extend(self.mode_groups(|r| self.favorites.contains(&r.id)));
        tidy(&mut nodes);
        nodes
    }

    /// The "Appearance" submenu, tagged [`crate::ItemRole::Appearance`].
    pub fn submenu(&self, labels: &ThemeLabels) -> Node {
        Node::submenu(labels.theme.as_str(), self.items(labels)).role(ItemRole::Appearance)
    }
}

craft_ui::words! {
    /// The Appearance menu's labels, English by default.
    #[non_exhaustive]
    pub struct ThemeLabels {
        theme: "theme.menu" = "Appearance";
        system: "theme.system" = "Sync with System";
        toggle: "theme.toggle" = "Next Mode";
        settings: "theme.settings" = "Appearance Settings…";
    }
}
