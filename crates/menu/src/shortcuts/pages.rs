//! What the Keyboard Shortcuts dialog adds for Photoshop's "Keyboard Shortcuts and Menus": the
//! Shortcuts For areas, the app's own tabs beside the shortcuts, and the dialog's state as
//! automation reads it.
//!
//! Undo, Accept and Go to Conflict, the Set row and Summarize… are the dialog's already; the Set
//! row's `.kys` export is an [`super::Options::export_formats`] entry the app writes with
//! [`craft_keymap::import::kys::write`].

use serde::Serialize;

use craft_keymap::{CommandDef, CommandKind, CommandRef, Keymap};

use super::state::Target;
use super::view::area_key;
use super::{Area, DialogState, ShortcutsDialog};

fn is_tool(b: &CommandDef) -> bool {
    matches!(b.kind, CommandKind::Tool { .. })
}

fn application_menus(b: &CommandDef) -> bool {
    !is_tool(b) && b.paths.first().and_then(|p| p.first()).is_some()
}

fn panel_menus(b: &CommandDef) -> bool {
    !is_tool(b) && !b.scope.is_app()
}

impl Area {
    /// Photoshop's Shortcuts For entries over the catalog: Application Menus (commands with a
    /// menu path in a top menu), Panel Menus (commands whose home scope is a panel) and Tools
    /// ([`craft_keymap::CommandKind::Tool`]). With [`super::Options::area_tabs`] off they are the Show
    /// dropdown's entries, as Photoshop's dropdown.
    pub fn photoshop() -> Vec<Area> {
        vec![
            Area::new("photoshop.menus", "Application Menus", application_menus),
            Area::new("photoshop.panels", "Panel Menus", panel_menus),
            Area::new("photoshop.tools", "Tools", is_tool),
        ]
    }
}

/// A tab of the app's own beside Keyboard Shortcuts (a Menus tab that hides and colours menu
/// items, say). The dialog draws a tab strip when there is one;
/// OK and Cancel apply to every tab, so the body edits the app's own draft and the app commits
/// it on [`super::Outcome::Ok`].
#[non_exhaustive]
pub struct DialogTab<'a> {
    /// The automation id (`menus`, `toolbar`).
    pub id: &'static str,
    /// The tab's label, translated by the app.
    pub label: String,
    /// Draws the tab's body in the dialog's body; returns true when it changed something
    /// (the dialog then reports [`super::Outcome::Changed`]).
    pub body: Box<dyn FnMut(&mut egui::Ui) -> bool + 'a>,
}

impl<'a> DialogTab<'a> {
    /// A tab with the automation id `id`, the translated `label` and the body `body`.
    pub fn new(id: &'static str, label: impl Into<String>, body: impl FnMut(&mut egui::Ui) -> bool + 'a) -> Self {
        DialogTab { id, label: label.into(), body: Box::new(body) }
    }
}

/// The dialog's view of [`DialogTab`]s, whose lifetime parameter is invariant behind `&mut`.
pub(crate) trait TabSet {
    fn len(&self) -> usize;
    /// The tab's automation id and label.
    fn describe(&self, index: usize) -> Option<(&'static str, &str)>;
    /// Draws the tab's body; true when it changed something.
    fn draw(&mut self, index: usize, ui: &mut egui::Ui) -> bool;
}

/// `tabs` with the trait object's lifetime cut to the borrow: `Env` is invariant in its lifetime.
pub(crate) fn shorten<'s, 'a: 's>(tabs: &'s mut (dyn TabSet + 'a)) -> &'s mut (dyn TabSet + 's) {
    tabs
}

struct Tabs<'r, 't>(&'r mut [DialogTab<'t>]);

impl TabSet for Tabs<'_, '_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn describe(&self, index: usize) -> Option<(&'static str, &str)> {
        self.0.get(index).map(|t| (t.id, t.label.as_str()))
    }

    fn draw(&mut self, index: usize, ui: &mut egui::Ui) -> bool {
        self.0.get_mut(index).is_some_and(|t| (t.body)(ui))
    }
}

impl<'a> ShortcutsDialog<'a> {
    /// The app's tabs after Keyboard Shortcuts, in this order. The selected tab is kept in the
    /// [`DialogState`]; Ctrl+Tab and Ctrl+Shift+Tab move between tabs (Ctrl+Page Down/Up on
    /// Windows and Linux too).
    pub fn tabs(mut self, tabs: &'a mut [DialogTab<'_>]) -> Self {
        self.tabs = if tabs.is_empty() { None } else { Some(Box::new(Tabs(tabs))) };
        self
    }
}

/// The dialog's state as automation and tests read it: what a control channel reports for
/// `ui.shortcuts.state` without reading pixels.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct DialogSnapshot {
    /// The tab shown: `shortcuts` or an app tab's id.
    pub tab: String,
    /// The Show entry or Shortcuts For area.
    pub area: String,
    /// The search text.
    pub query: String,
    /// The command selected in the table, if any.
    pub selected: Option<CommandRef>,
    /// Commands whose bindings differ from the defaults, with their binding texts.
    pub overrides: Vec<(CommandRef, Vec<String>)>,
    /// The conflict bar's chord and the commands holding it, while it shows.
    pub conflict: Option<(String, Vec<CommandRef>)>,
    /// Whether a chord is being recorded, and the command it is for.
    pub recording: Option<CommandRef>,
    /// The active set's name, with sets.
    pub set: Option<String>,
    /// Whether there is an edit to undo.
    pub can_undo: bool,
}

impl DialogState {
    /// The state as [`DialogSnapshot`]; `None` before the dialog's first frame.
    pub fn snapshot(&self, keymap: &Keymap) -> Option<DialogSnapshot> {
        if !self.begun() {
            return None;
        }
        let command =
            |key: &str| keymap.catalog().iter().find(|b| b.command.key() == key).map(|b| b.command.clone()).or_else(|| CommandRef::parse_key(key));
        Some(DialogSnapshot {
            tab: self.tab.unwrap_or("shortcuts").to_string(),
            area: area_key(&self.area),
            query: self.query.clone(),
            selected: self.selected.clone(),
            overrides: keymap.overrides().iter().filter_map(|(key, texts)| Some((command(key)?, texts.to_vec()))).collect(),
            conflict: self.conflict.as_ref().map(|c| (c.binding.to_string(), c.holders.iter().map(|h| h.command.clone()).collect())),
            recording: self.recording.as_ref().and_then(|r| match &r.target {
                Target::Cell { command, .. } | Target::Field { command } => Some(command.clone()),
                Target::Search => None,
            }),
            set: self.set.clone(),
            can_undo: self.can_undo(),
        })
    }
}
