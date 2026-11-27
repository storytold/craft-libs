//! `plugin.json`: what an add-in is, how to start it, and what it adds to an app.
//!
//! One manifest can serve several apps: `apps` says which, and `uiFor.<app>` gives an app its own
//! ribbon groups and menu items in place of `ui`. [`Manifest::parse`] reads it for one app
//! ([`Host`]) and checks every limit; what a menu name or a ribbon tab means stays with the app.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::requires::Requires;
use crate::{MANIFEST, valid_id};

/// Most functions, commands, events, ribbon groups, buttons and menu items in a manifest.
pub const MAX_DECLARATIONS: usize = 1_000;
/// Longest tab, group and label text (characters).
pub const MAX_LABEL: usize = 64;

/// The app a manifest is read for.
#[derive(Clone, Copy, Debug)]
pub struct Host<'a> {
    /// The app's id in a manifest's `apps` and `uiFor` (`gridcraft`).
    pub id: &'a str,
    /// The app's name, for messages (`GridCraft`).
    pub name: &'a str,
    /// The right-click menus an add-in may add items to (`cell`, `row`, …). The first is the
    /// default, used by an item that names none.
    pub menus: &'a [&'a str],
}

/// `plugin.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Unique id, e.g. `org.example.finance` (letters, digits, `.`, `-`, `_`).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// The add-in's web page (documentation, support): an `https://` or `http://` URL.
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub author: String,
    /// The apps the add-in is made for (`gridcraft`, `wordcraft`, …); empty: any.
    #[serde(default)]
    pub apps: Vec<String>,
    /// The program to start; empty for an add-in the app runs another way
    /// ([`Manifest::parse_without_program`]).
    #[serde(default)]
    pub run: RunSpec,
    /// Functions formulas call by name. An app without formulas lists them and doesn't call them.
    #[serde(default)]
    pub functions: Vec<FunctionDecl>,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    /// Ribbon groups and menu items that run the add-in's commands: `uiFor.<app>` when the
    /// manifest has one for the app it was read for, else `ui`.
    #[serde(default)]
    pub ui: UiDecl,
    /// Per-app ribbon groups and menu items (`{"gridcraft": {…}, "wordcraft": {…}}`), as written.
    #[serde(default)]
    pub ui_for: BTreeMap<String, Json>,
    /// A scripting language the add-in runs macros in (protocol 1.1: `macros/*`).
    #[serde(default)]
    pub macros: Option<MacrosDecl>,
    /// The document events the add-in wants (protocol 1.2): `documentOpened`, `documentSaved`,
    /// `documentClosed`. Names an app doesn't send are kept and ignored.
    #[serde(default)]
    pub events: Vec<String>,
    /// A co-editing transport (protocol 2): `{"transport": "mqtt", "label": "Live (MQTT)"}`.
    #[serde(default)]
    pub collab: Option<CollabDecl>,
    /// What the add-in needs (manifest 1.3): app versions, protocol, host features, other add-ins.
    #[serde(default, skip_serializing_if = "Requires::is_empty")]
    pub requires: Requires,
}

/// A transport add-in's declaration (protocol 2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CollabDecl {
    /// The kind of transport (`mqtt`, `loopback`, `y-websocket`…).
    pub transport: String,
    /// What the app calls it where people pick a transport.
    #[serde(default)]
    pub label: String,
}

/// An add-in that runs macros (`"macros": {"language": "VBA"}`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MacrosDecl {
    pub language: String,
}

/// How to start the add-in. A `command` containing a path separator is relative to the add-in's
/// folder; a bare name is looked up on `PATH`. The add-in runs in its folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDecl {
    /// The name formulas use, e.g. `ACME.PRICE`; upper-cased by [`Manifest::parse`].
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Shown in the function list, e.g. `ACME.PRICE(ticker, [date])`.
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub min_args: usize,
    #[serde(default = "max_args_default")]
    pub max_args: usize,
    /// Recalculated on every recalculation (results are not cached).
    #[serde(default)]
    pub volatile: bool,
    /// How long a call may take; the app sets the default and the most.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

fn max_args_default() -> usize {
    MAX_ARGS
}

/// Most arguments a function takes.
pub const MAX_ARGS: usize = 255;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDecl {
    /// Unique within the add-in.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// Params documentation, like the app's own commands.
    #[serde(default)]
    pub params: String,
}

/// Where the add-in's commands appear in the app.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDecl {
    /// Groups of buttons on a ribbon tab: one of the app's tabs, or a new one.
    #[serde(default)]
    pub ribbon: Vec<RibbonGroupDecl>,
    /// Items added to a right-click menu.
    #[serde(default)]
    pub menus: Vec<MenuItemDecl>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RibbonGroupDecl {
    /// The English name of one of the app's tabs adds the group at the end of that tab; any other
    /// name makes a new tab.
    pub tab: String,
    pub group: String,
    pub buttons: Vec<ButtonDecl>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonDecl {
    /// One of the add-in's `commands[].id`.
    pub command: String,
    /// Default: the command's label.
    #[serde(default)]
    pub label: Option<String>,
    /// Default: the command's description.
    #[serde(default)]
    pub tooltip: Option<String>,
    /// An icon name from the app's icon set; unknown names get a default.
    #[serde(default)]
    pub icon: Option<String>,
    /// `large` (default) or `small`.
    #[serde(default)]
    pub size: ButtonSize,
    /// Passed to the command.
    #[serde(default)]
    pub params: Json,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ButtonSize {
    #[default]
    Large,
    Small,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemDecl {
    /// Which right-click menu, one of [`Host::menus`]. [`Manifest::parse`] fills in the app's
    /// default when the manifest names none.
    #[serde(default)]
    pub menu: String,
    pub command: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub params: Json,
}

fn valid_label(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_LABEL
}

fn valid_function_name(s: &str) -> bool {
    s.len() <= 255
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
        && !s.ends_with('.')
}

impl Manifest {
    /// Parses and validates a manifest for `host`.
    pub fn parse(text: &str, host: &Host<'_>) -> Result<Manifest, String> {
        Manifest::parse_inner(text, host, true)
    }

    /// [`Manifest::parse`] for an add-in with no program of its own: `run.command` may be empty.
    pub fn parse_without_program(text: &str, host: &Host<'_>) -> Result<Manifest, String> {
        Manifest::parse_inner(text, host, false)
    }

    fn parse_inner(text: &str, host: &Host<'_>, needs_program: bool) -> Result<Manifest, String> {
        let mut raw: Json = serde_json::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        // The app's own ribbon and menus replace `ui`: another app's `ui` would mean nothing here.
        if let Some(obj) = raw.as_object_mut()
            && let Some(own) = obj.get("uiFor").and_then(|u| u.get(host.id)).cloned()
        {
            obj.insert("ui".into(), own);
        }
        let mut m: Manifest = serde_json::from_value(raw).map_err(|e| format!("{MANIFEST}: {e}"))?;
        if !m.is_for(host.id) {
            return Err(format!("{MANIFEST}: plugin {} is made for {}, not {}", m.id, m.apps.join(", "), host.name));
        }
        if let Some(default) = host.menus.first() {
            for item in m.ui.menus.iter_mut().filter(|i| i.menu.is_empty()) {
                item.menu = (*default).to_string();
            }
        }
        m.validate(host, needs_program)?;
        for f in &mut m.functions {
            f.name = f.name.to_ascii_uppercase();
        }
        Ok(m)
    }

    /// Whether `apps` names the app `host_id` (or nothing).
    pub fn is_for(&self, host_id: &str) -> bool {
        self.apps.is_empty() || self.apps.iter().any(|a| a.eq_ignore_ascii_case(host_id))
    }

    fn validate(&self, host: &Host<'_>, needs_program: bool) -> Result<(), String> {
        self.validate_identity(needs_program)?;
        self.validate_functions()?;
        let commands = self.validate_commands()?;
        self.validate_ui(host, &commands)
    }

    fn validate_identity(&self, needs_program: bool) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!("{MANIFEST}: id `{}` must be 1–128 letters, digits, `.`, `-` or `_`", self.id));
        }
        if self.name.trim().is_empty() {
            return Err(format!("{MANIFEST}: `name` is empty"));
        }
        let homepage = self.homepage.trim();
        if !homepage.is_empty()
            && (!(homepage.starts_with("https://") || homepage.starts_with("http://"))
                || homepage.len() > 2048
                || homepage.contains(char::is_whitespace))
        {
            return Err(format!("{MANIFEST}: `homepage` must be an http(s) URL"));
        }
        if needs_program && self.run.command.trim().is_empty() {
            return Err(format!("{MANIFEST}: `run.command` is empty"));
        }
        if let Some(c) = &self.collab
            && (c.transport.trim().is_empty() || c.transport.len() > 64 || c.label.chars().count() > MAX_LABEL)
        {
            return Err(format!("{MANIFEST}: `collab.transport` needs 1–64 characters and `collab.label` at most {MAX_LABEL}"));
        }
        self.requires.validate(&self.id)?;
        if self.events.len() > MAX_DECLARATIONS {
            return Err(format!("{MANIFEST}: at most {MAX_DECLARATIONS} events"));
        }
        Ok(())
    }

    fn validate_functions(&self) -> Result<(), String> {
        if self.functions.len() > MAX_DECLARATIONS || self.commands.len() > MAX_DECLARATIONS {
            return Err(format!("{MANIFEST}: at most {MAX_DECLARATIONS} functions and {MAX_DECLARATIONS} commands"));
        }
        let mut seen = HashSet::new();
        for f in &self.functions {
            if !valid_function_name(&f.name) {
                return Err(format!("{MANIFEST}: function name `{}` must start with a letter and hold letters, digits, `.` or `_`", f.name));
            }
            if !seen.insert(f.name.to_ascii_uppercase()) {
                return Err(format!("{MANIFEST}: function `{}` is declared twice", f.name));
            }
            if f.min_args > f.max_args || f.max_args > MAX_ARGS {
                return Err(format!("{MANIFEST}: function `{}` needs minArgs ≤ maxArgs ≤ {MAX_ARGS}", f.name));
            }
        }
        Ok(())
    }

    fn validate_commands(&self) -> Result<HashSet<&str>, String> {
        let mut seen = HashSet::new();
        for c in &self.commands {
            if !valid_id(&c.id) {
                return Err(format!("{MANIFEST}: command id `{}` must be letters, digits, `.`, `-` or `_`", c.id));
            }
            if !seen.insert(c.id.as_str()) {
                return Err(format!("{MANIFEST}: command `{}` is declared twice", c.id));
            }
        }
        Ok(seen)
    }

    fn validate_ui(&self, host: &Host<'_>, commands: &HashSet<&str>) -> Result<(), String> {
        let buttons: usize = self.ui.ribbon.iter().map(|g| g.buttons.len()).sum();
        if self.ui.ribbon.len() > MAX_DECLARATIONS || buttons > MAX_DECLARATIONS || self.ui.menus.len() > MAX_DECLARATIONS {
            return Err(format!("{MANIFEST}: at most {MAX_DECLARATIONS} ribbon groups, buttons and menu items"));
        }
        let known = |id: &str, what: &str| {
            if commands.contains(id) { Ok(()) } else { Err(format!("{MANIFEST}: {what} runs `{id}`, which isn't one of the plugin's commands")) }
        };
        for g in &self.ui.ribbon {
            if !valid_label(&g.tab) || !valid_label(&g.group) {
                return Err(format!("{MANIFEST}: ribbon `tab` and `group` need 1–{MAX_LABEL} characters"));
            }
            if g.buttons.is_empty() {
                return Err(format!("{MANIFEST}: ribbon group `{}` has no buttons", g.group));
            }
            for b in &g.buttons {
                known(&b.command, &format!("a button in ribbon group `{}`", g.group))?;
                if b.label.as_deref().is_some_and(|l| !valid_label(l)) {
                    return Err(format!("{MANIFEST}: button labels need 1–{MAX_LABEL} characters"));
                }
            }
        }
        for item in &self.ui.menus {
            known(&item.command, "a menu item")?;
            if !host.menus.contains(&item.menu.as_str()) {
                return Err(format!("{MANIFEST}: {} has no `{}` menu (it has {})", host.name, item.menu, host.menus.join(", ")));
            }
            if item.label.as_deref().is_some_and(|l| !valid_label(l)) {
                return Err(format!("{MANIFEST}: menu item labels need 1–{MAX_LABEL} characters"));
            }
        }
        Ok(())
    }

    /// The command a button or menu item runs.
    pub fn command(&self, id: &str) -> Option<&CommandDecl> {
        self.commands.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests;
