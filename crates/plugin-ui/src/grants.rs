//! Permission prompt model and the grants review view (`ABI.md` section 5).
//!
//! The model is plain data and has no egui types, so the host can test install flows without a
//! window. [`show_prompt`] and [`show_review`] draw it. Both use words and text tags to carry
//! meaning, never color alone.

use std::fmt;

use craft_plugin_abi::permissions::PermissionError;
pub use craft_plugin_abi::permissions::{DocScope, Permission, PromptLevel};
use egui::Ui;

/// Tag text shown beside a permission.
pub fn level_tag(level: PromptLevel) -> &'static str {
    match level {
        PromptLevel::Low => "LOW",
        PromptLevel::Normal => "NORMAL",
        PromptLevel::High => "HIGH",
        PromptLevel::OnFirstUse => "ON FIRST USE",
    }
}

/// Sort key. High first, so the risky rows lead the list.
fn rank(level: PromptLevel) -> u8 {
    match level {
        PromptLevel::High => 0,
        PromptLevel::Normal => 1,
        PromptLevel::OnFirstUse => 2,
        PromptLevel::Low => 3,
    }
}

/// Order of rows and grants in the views: high level first, then by wire form.
fn row_order(a: &Permission, b: &Permission) -> std::cmp::Ordering {
    (rank(a.prompt_level()), a.to_string()).cmp(&(rank(b.prompt_level()), b.to_string()))
}

/// Why a permission string was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantError {
    /// The ABI parser refused the string: unknown name or a bad argument.
    Invalid(PermissionError),
    /// A permission the ABI reserves but this feature does not implement (`frames.read`).
    Reserved(String),
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantError::Invalid(e) => e.fmt(f),
            GrantError::Reserved(p) => write!(f, "permission {p:?} is reserved and not available yet"),
        }
    }
}

impl std::error::Error for GrantError {}

/// Parse a manifest permission string with the ABI parser. `frames.read` parses there, but no op
/// uses it yet, so the prompt refuses it.
pub fn parse_permission(s: &str) -> Result<Permission, GrantError> {
    match s.parse::<Permission>() {
        Ok(Permission::FramesRead) => Err(GrantError::Reserved(s.to_owned())),
        Ok(p) => Ok(p),
        Err(e) => Err(GrantError::Invalid(e)),
    }
}

/// One plain sentence for the prompt.
pub fn describe(permission: &Permission) -> String {
    match permission {
        Permission::CommandsRun(None) => "Run any app command. Each run is one undo step.".to_owned(),
        Permission::CommandsRun(Some(g)) => format!("Run app commands matching {g}. Each run is one undo step."),
        Permission::CommandsObserve => "See which commands ran and when the document changed.".to_owned(),
        Permission::CommandsIntercept => "Block or rewrite commands before they run.".to_owned(),
        Permission::DocRead(None) => "Read the document.".to_owned(),
        Permission::DocRead(Some(DocScope::Metadata)) => "Read document metadata only.".to_owned(),
        Permission::DocRead(Some(DocScope::Pixels)) => "Read document pixels.".to_owned(),
        Permission::DocWrite => "Edit the document outside the command system.".to_owned(),
        Permission::UiContribute => "Add menus, panels and dialogs.".to_owned(),
        Permission::StateLocal => "Keep a small private store (1 MiB by default).".to_owned(),
        Permission::LinkCall(app) => {
            format!("Look up assets and links in the app {app} through Craft Link. It cannot run commands, write files or change plug-ins there.")
        }
        Permission::LinkExec { app, glob } => {
            format!("Run commands matching {glob} in the app {app}, with that app's own rights. It can change that app's documents and files.")
        }
        Permission::FsPicked => "Read files you pick in a dialog. It never sees paths.".to_owned(),
        Permission::NetFetch(origin) => format!("Send HTTPS requests to {origin}."),
        Permission::Clock => "Read the real time.".to_owned(),
        Permission::Random => "Get real random numbers.".to_owned(),
        Permission::FramesRead => "Read video frames. Reserved.".to_owned(),
    }
}

/// The user's answer for one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Allowed,
    Declined,
}

/// One line of the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantRow {
    pub permission: Permission,
    pub level: PromptLevel,
    /// Required permissions come from `permissions.required`. Declining one blocks the install.
    pub required: bool,
    pub decision: Decision,
}

/// What the prompt means right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantOutcome {
    /// This many rows still wait for an answer.
    Pending(usize),
    /// The user declined these required permissions. The install cannot go on.
    Blocked(Vec<Permission>),
    /// Install can go on with `granted`. `declined` holds optional permissions the user refused.
    Granted { granted: Vec<Permission>, declined: Vec<Permission> },
}

/// The install-time prompt for one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantPrompt {
    rows: Vec<GrantRow>,
}

impl GrantPrompt {
    /// Build rows from the manifest lists. A permission listed in both lists is required. Rows sort
    /// high level first. Low and on-first-use rows start allowed, the rest start pending.
    pub fn new(required: &[String], optional: &[String]) -> Result<Self, GrantError> {
        let mut rows: Vec<GrantRow> = Vec::new();
        for (list, is_required) in [(required, true), (optional, false)] {
            for s in list {
                let permission = parse_permission(s)?;
                if let Some(row) = rows.iter_mut().find(|r| r.permission == permission) {
                    row.required |= is_required;
                    continue;
                }
                let level = permission.prompt_level();
                let decision = match level {
                    PromptLevel::Low | PromptLevel::OnFirstUse => Decision::Allowed,
                    PromptLevel::Normal | PromptLevel::High => Decision::Pending,
                };
                rows.push(GrantRow { permission, level, required: is_required, decision });
            }
        }
        rows.sort_by(|a, b| row_order(&a.permission, &b.permission));
        Ok(GrantPrompt { rows })
    }

    pub fn rows(&self) -> &[GrantRow] {
        &self.rows
    }

    /// Record an answer. Returns false when the prompt has no such permission.
    pub fn set(&mut self, permission: &Permission, decision: Decision) -> bool {
        match self.rows.iter_mut().find(|r| &r.permission == permission) {
            Some(row) => {
                row.decision = decision;
                true
            }
            None => false,
        }
    }

    /// Allow every row.
    pub fn allow_all(&mut self) {
        self.rows.iter_mut().for_each(|r| r.decision = Decision::Allowed);
    }

    pub fn outcome(&self) -> GrantOutcome {
        let blocked: Vec<Permission> =
            self.rows.iter().filter(|r| r.required && r.decision == Decision::Declined).map(|r| r.permission.clone()).collect();
        if !blocked.is_empty() {
            return GrantOutcome::Blocked(blocked);
        }
        let pending = self.rows.iter().filter(|r| r.decision == Decision::Pending).count();
        if pending > 0 {
            return GrantOutcome::Pending(pending);
        }
        let pick = |d: Decision| self.rows.iter().filter(|r| r.decision == d).map(|r| r.permission.clone()).collect();
        GrantOutcome::Granted { granted: pick(Decision::Allowed), declined: pick(Decision::Declined) }
    }
}

/// What the user chose with the prompt's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptAction {
    /// The user pressed Install. Only possible when the outcome is `Granted`.
    Confirm,
    Cancel,
}

/// Draw the prompt. The caller shows it in a window or panel. Returns the button the user pressed.
pub fn show_prompt(ui: &mut Ui, prompt: &mut GrantPrompt) -> Option<PromptAction> {
    ui.heading("Permissions");
    let mut changes: Vec<(Permission, Decision)> = Vec::new();
    egui::Grid::new("craft_grants_prompt").num_columns(3).striped(true).show(ui, |ui| {
        for row in prompt.rows() {
            ui.vertical(|ui| {
                ui.strong(level_tag(row.level));
                ui.label(if row.required { "required" } else { "optional" });
            });
            ui.vertical(|ui| {
                ui.strong(row.permission.to_string());
                ui.label(describe(&row.permission));
            });
            ui.horizontal(|ui| {
                let allow = ui.selectable_label(row.decision == Decision::Allowed, "Allow");
                let decline = ui.selectable_label(row.decision == Decision::Declined, "Decline");
                if allow.clicked() {
                    changes.push((row.permission.clone(), Decision::Allowed));
                }
                if decline.clicked() {
                    changes.push((row.permission.clone(), Decision::Declined));
                }
            });
            ui.end_row();
        }
    });
    for (p, d) in changes {
        prompt.set(&p, d);
    }
    let outcome = prompt.outcome();
    match &outcome {
        GrantOutcome::Pending(n) => {
            ui.label(format!("{n} permission(s) still need an answer."));
        }
        GrantOutcome::Blocked(list) => {
            let names: Vec<String> = list.iter().map(ToString::to_string).collect();
            ui.label(format!("Install is blocked. Required and declined: {}", names.join(", ")));
        }
        GrantOutcome::Granted { .. } => {}
    }
    let mut action = None;
    ui.horizontal(|ui| {
        let ready = matches!(outcome, GrantOutcome::Granted { .. });
        if ui.add_enabled(ready, egui::Button::new("Install")).clicked() {
            action = Some(PromptAction::Confirm);
        }
        if ui.button("Cancel").clicked() {
            action = Some(PromptAction::Cancel);
        }
    });
    action
}

/// The grants a plugin holds now, with a revoke button on each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantsReview {
    pub plugin_id: String,
    granted: Vec<Permission>,
}

/// A change the user asked for in the review view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantAction {
    Revoke { plugin_id: String, permission: Permission },
}

impl GrantsReview {
    pub fn new(plugin_id: &str, granted: &[String]) -> Result<Self, GrantError> {
        let mut list = granted.iter().map(|s| parse_permission(s)).collect::<Result<Vec<_>, _>>()?;
        list.sort_by(row_order);
        list.dedup();
        Ok(GrantsReview { plugin_id: plugin_id.to_owned(), granted: list })
    }

    pub fn granted(&self) -> &[Permission] {
        &self.granted
    }

    /// Drop a grant from the view. Returns false when it was not listed.
    pub fn revoke(&mut self, permission: &Permission) -> bool {
        let before = self.granted.len();
        self.granted.retain(|p| p != permission);
        self.granted.len() != before
    }
}

/// Draw the review list. A click on Revoke removes the row and returns the action for the host to
/// persist. The host owns the real grant store.
pub fn show_review(ui: &mut Ui, review: &mut GrantsReview) -> Vec<GrantAction> {
    ui.heading(format!("Permissions of {}", review.plugin_id));
    if review.granted.is_empty() {
        ui.label("This plugin holds no permissions.");
        return Vec::new();
    }
    let mut clicked: Vec<Permission> = Vec::new();
    egui::Grid::new("craft_grants_review").num_columns(3).striped(true).show(ui, |ui| {
        for p in &review.granted {
            ui.strong(level_tag(p.prompt_level()));
            ui.vertical(|ui| {
                ui.strong(p.to_string());
                ui.label(describe(p));
            });
            if ui.button("Revoke").clicked() {
                clicked.push(p.clone());
            }
            ui.end_row();
        }
    });
    let plugin_id = review.plugin_id.clone();
    clicked.into_iter().filter(|p| review.revoke(p)).map(|permission| GrantAction::Revoke { plugin_id: plugin_id.clone(), permission }).collect()
}
