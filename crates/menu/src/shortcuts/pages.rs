//! What the Keyboard Shortcuts dialog adds for Photoshop's "Keyboard Shortcuts and Menus": the
//! Shortcuts For areas, the app's own tabs beside the shortcuts, and the dialog's state as
//! automation reads it.
//!
//! Undo, Accept and Go to Conflict, the Set row and Summarize… are the dialog's already; the Set
//! row's `.kys` export is an [`super::Options::export_formats`] entry the app writes with
//! [`craft_keymap::import::kys::write`].

use std::collections::BTreeMap;

use serde::Serialize;

use craft_keymap::{CommandDef, CommandKind, CommandRef, Keymap};

use super::state::{Review, Target};
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
    /// Whether Hold Shortcuts is on, with the hold setting shown.
    pub holds: Option<bool>,
    /// The command a hold is being recorded for.
    pub recording_hold: Option<CommandRef>,
    /// The import review, while it shows.
    pub review: Option<ReviewSnapshot>,
    /// The chord in the New Shortcut field, as canonical text, while there is one.
    pub pressed: Option<String>,
    /// The commands that hold the pressed chord now.
    pub holders: Vec<CommandRef>,
    /// The selected command's keys as canonical text, as `remove` names them.
    pub keys: Vec<String>,
}

/// The import review as automation reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ReviewSnapshot {
    /// The new set's name.
    pub name: String,
    /// The set's entries, in source order.
    pub entries: Vec<ReviewEntry>,
    /// The undecided codes the plan decoded or the user answered.
    pub codes: Vec<ReviewCode>,
    /// How many entries are in each [`ReviewEntry::state`].
    pub counts: BTreeMap<&'static str, usize>,
}

/// One entry of the set under review.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ReviewEntry {
    /// The source's path of the entry, as the set names it.
    pub item: String,
    /// The entry's line in the source file, when the reader gives one.
    pub line: Option<usize>,
    /// The command the entry is matched to.
    pub target: Option<CommandRef>,
    /// Whether the entry is imported.
    pub included: bool,
    /// `ready`, `unmatched`, `unreadable` or `leftOut`.
    pub state: &'static str,
}

/// An undecided key or modifier code of the set under review.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ReviewCode {
    /// The code, as `key:N` or `mods:N`.
    pub code: String,
    /// The chord text the code stands for; `None`: No equivalent.
    pub text: Option<String>,
}

impl ReviewSnapshot {
    fn of(review: &Review) -> ReviewSnapshot {
        let report = &review.plan.report;
        let entries: Vec<ReviewEntry> = review
            .set
            .entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let (item, line) = (entry.item(), entry.line);
                // Report rows name their entry by its item and line, as `Review::entry_at` finds it.
                let here = |row_item: &str, row_line: Option<usize>| row_item == item && row_line == line;
                let included = review.included.get(i).copied().unwrap_or(true);
                let target = review
                    .plan
                    .commands
                    .iter()
                    .find(|p| here(&p.item, p.line) || p.bindings.iter().any(|b| here(&b.item, b.line)))
                    .map(|p| p.command.clone())
                    .or_else(|| report.matched_without_change.iter().find(|m| here(&m.item, m.line)).map(|m| m.command.clone()));
                let state = if !included {
                    "leftOut"
                } else if report.unreadable.iter().any(|u| here(&u.item, u.line)) {
                    "unreadable"
                } else if target.is_none() || report.unmatched.iter().any(|u| here(&u.item, u.line)) {
                    "unmatched"
                } else {
                    "ready"
                };
                ReviewEntry { item, line, target, included, state }
            })
            .collect();
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for entry in &entries {
            *counts.entry(entry.state).or_default() += 1;
        }
        let codes = report.codes.iter().map(|c| ReviewCode { code: c.code.clone(), text: c.text.clone() }).collect();
        ReviewSnapshot { name: review.name.clone(), entries, codes, counts }
    }
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
                Target::Cell { command, .. } | Target::HoldCell { command, .. } | Target::Field { command } => Some(command.clone()),
                Target::Search | Target::Code { .. } => None,
            }),
            set: self.set.clone(),
            can_undo: self.can_undo(),
            // The switch sets the setting and the keymap's holds in one step, so the keymap's flag is the setting's.
            holds: self.holds_shown.then(|| keymap.holds()),
            recording_hold: self.recording.as_ref().and_then(|r| match &r.target {
                Target::HoldCell { command, .. } => Some(command.clone()),
                _ => None,
            }),
            review: self.review.as_ref().map(ReviewSnapshot::of),
            pressed: self.field.as_ref().map(|f| f.binding.to_string()),
            holders: self.field.as_ref().map(|f| f.holders.iter().map(|h| h.command.clone()).collect()).unwrap_or_default(),
            keys: self.selected.as_ref().map(|c| keymap.bindings(c).iter().map(ToString::to_string).collect()).unwrap_or_default(),
        })
    }
}
