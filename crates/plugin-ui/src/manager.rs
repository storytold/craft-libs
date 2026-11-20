//! The plug-in windows an app shows: the permission prompt, the manager and panel windows.
//!
//! The app owns a [`UiState`] and calls [`draw`] once per frame. `draw` returns the [`Act`]s the user
//! caused. The app carries them out against its plugin host and its session, because only the app
//! knows how. Nothing here touches a session or a plugin host. Every state is a word, never a color
//! alone.
//!
//! The windows are not in an app's automation registry (`ui.elements`). The same actions are app
//! commands, and the control channel drives those.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use craft_plugin_host::lifecycle::{GrantRequest, PluginInfo, PluginState};
use serde_json::{Value, json};

use crate::{GrantAction, GrantOutcome, GrantPrompt, GrantsReview, Permission, PromptAction, UiEvent, WidgetTree, show, show_prompt, show_review};

/// Opens a file dialog. The app injects it so this crate needs no dialog library.
pub type FilePicker = Box<dyn Fn() -> Option<PathBuf> + Send + Sync>;

/// Short text of a plug-in state.
pub fn state_text(state: &PluginState) -> String {
    match state {
        PluginState::AwaitingGrant => "awaiting permission".into(),
        PluginState::Disabled => "disabled".into(),
        PluginState::Faulted { reason } => format!("faulted: {reason}"),
        PluginState::Active { generation } => format!("active (generation {generation})"),
    }
}

/// One installed plug-in as the manager shows it.
#[derive(Clone, Debug)]
pub struct PluginRow {
    pub info: PluginInfo,
    /// The last `host.progress` line, ready to show.
    pub progress: Option<String>,
    /// Ids of the contributions that are panels, so the row can offer "Open panel".
    pub panels: Vec<String>,
}

/// A stored plug-in that did not load at start and has no manifest to list.
#[derive(Clone, Debug)]
pub struct BrokenRow {
    pub id: String,
    pub state: String,
}

/// What the manager window shows. The app builds it when the window is open.
#[derive(Clone, Debug)]
pub struct ManagerView {
    /// App name, as in "This version of Example cannot run".
    pub app: String,
    pub rows: Vec<PluginRow>,
    pub broken: Vec<BrokenRow>,
}

/// What a frame decided. The app carries it out after the frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Answer { id: String, accept: bool, optional: Vec<Permission> },
    Install(String),
    Enable(String),
    Disable(String),
    Remove(String),
    Revoke(String, Permission),
    OpenPanel(String, String),
    Panel { plugin: String, contribution: String, event: Value },
    ClosePanel { plugin: String, contribution: String },
}

struct Pending {
    request: GrantRequest,
    prompt: GrantPrompt,
}

/// One open plug-in panel.
struct PanelWindow {
    title: String,
    tree: Option<WidgetTree>,
    error: Option<String>,
}

impl PanelWindow {
    /// Take what a panel call answered. A missing tree keeps the old one.
    fn apply(&mut self, answer: &Result<Value, String>) {
        match answer {
            Err(e) => self.error = Some(e.clone()),
            Ok(value) => {
                if let Some(tree) = value.get("tree") {
                    match WidgetTree::from_json(&tree.to_string()) {
                        Ok(t) => {
                            self.tree = Some(t);
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("the plug-in sent a bad panel: {e:?}")),
                    }
                }
            }
        }
    }
}

/// Everything the windows remember between frames.
#[derive(Default)]
pub struct UiState {
    pub manager_open: bool,
    /// The last thing that happened, shown in the manager.
    pub message: Option<String>,
    pub install_path: String,
    picker: Option<FilePicker>,
    prompts: VecDeque<Pending>,
    confirm_remove: Option<String>,
    panels: BTreeMap<(String, String), PanelWindow>,
}

impl UiState {
    /// Let the manager window browse for files.
    pub fn set_picker(&mut self, picker: FilePicker) {
        self.picker = Some(picker);
    }

    /// Queue the permission prompt of `request`, replacing an older one for the same plug-in.
    pub fn queue_prompt(&mut self, request: &GrantRequest) {
        let required: Vec<String> = request.required.iter().map(ToString::to_string).collect();
        let optional: Vec<String> = request.optional.iter().map(ToString::to_string).collect();
        match GrantPrompt::new(&required, &optional) {
            Ok(prompt) => {
                self.drop_prompt(&request.plugin_id);
                self.prompts.push_back(Pending { request: request.clone(), prompt });
            }
            Err(e) => self.message = Some(format!("{}: cannot show the permission prompt: {e:?}", request.plugin_id)),
        }
    }

    pub fn drop_prompt(&mut self, plugin_id: &str) {
        self.prompts.retain(|p| p.request.plugin_id != plugin_id);
    }

    /// Ids of the plug-ins that wait for an answer, oldest first.
    pub fn waiting(&self) -> Vec<String> {
        self.prompts.iter().map(|p| p.request.plugin_id.clone()).collect()
    }

    /// Name of the plug-in dialog that is open, for `ui.inspect`.
    pub fn dialog_label(&self) -> Option<String> {
        if !self.prompts.is_empty() {
            Some("Plug-in permissions".into())
        } else if self.manager_open {
            Some("Plug-ins".into())
        } else {
            None
        }
    }

    pub fn panel_open(&self, plugin_id: &str, contribution_id: &str) -> bool {
        self.panels.contains_key(&(plugin_id.to_owned(), contribution_id.to_owned()))
    }

    /// Open a panel window with the answer to the `open` event.
    pub fn open_panel_window(&mut self, plugin_id: &str, contribution_id: &str, title: &str, answer: Result<Value, String>) {
        let mut window = PanelWindow { title: title.to_owned(), tree: None, error: None };
        window.apply(&answer);
        self.panels.insert((plugin_id.to_owned(), contribution_id.to_owned()), window);
    }

    /// Show the answer to a panel event in its window, if the window is still open.
    pub fn apply_panel_answer(&mut self, plugin_id: &str, contribution_id: &str, answer: Result<Value, String>) {
        if let Some(window) = self.panels.get_mut(&(plugin_id.to_owned(), contribution_id.to_owned())) {
            window.apply(&answer);
        }
    }

    pub fn close_panel_window(&mut self, plugin_id: &str, contribution_id: &str) {
        self.panels.remove(&(plugin_id.to_owned(), contribution_id.to_owned()));
    }
}

/// Draw the plug-in windows. `view` runs only when the manager window is open. Returns the acts the
/// user caused this frame, in the order the windows draw.
pub fn draw(ctx: &egui::Context, state: &mut UiState, view: impl FnOnce() -> ManagerView) -> Vec<Act> {
    let mut acts = Vec::new();
    prompt_window(state, ctx, &mut acts);
    if state.manager_open {
        manager_window(state, ctx, &view(), &mut acts);
    }
    panel_windows(state, ctx, &mut acts);
    acts
}

fn prompt_window(state: &mut UiState, ctx: &egui::Context, acts: &mut Vec<Act>) {
    let Some(front) = state.prompts.front_mut() else { return };
    let mut action = None;
    egui::Window::new("Plug-in permissions").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        let r = &front.request;
        ui.strong(format!("{} {}", r.plugin_id, r.version));
        if let Some(p) = &r.publisher {
            ui.label(format!("Publisher: {p} (not verified)"));
        }
        if r.risk.is_risky() {
            ui.colored_label(
                egui::Color32::from_rgb(230, 159, 0),
                format!("WARNING: this replaces the installed plug-in, and its old permissions are dropped. Reason: {}.", r.risk),
            );
        } else if r.new_required.len() != r.required.len() {
            let names: Vec<String> = r.new_required.iter().map(ToString::to_string).collect();
            ui.label(format!("This update asks for new permissions: {}", names.join(", ")));
        }
        ui.label("Nothing of this plug-in has run. It runs after you answer.");
        action = show_prompt(ui, &mut front.prompt);
    });
    let id = front.request.plugin_id.clone();
    match action {
        Some(PromptAction::Confirm) => {
            if let GrantOutcome::Granted { granted, .. } = front.prompt.outcome() {
                let optional = front.request.optional.iter().filter(|p| granted.iter().any(|g| g.to_string() == p.to_string())).cloned().collect();
                acts.push(Act::Answer { id, accept: true, optional });
            }
        }
        Some(PromptAction::Cancel) => acts.push(Act::Answer { id, accept: false, optional: Vec::new() }),
        None => {}
    }
}

fn manager_window(state: &mut UiState, ctx: &egui::Context, view: &ManagerView, acts: &mut Vec<Act>) {
    let mut open = true;
    egui::Window::new("Plug-ins").open(&mut open).default_width(560.0).show(ctx, |ui| {
        install_row(state, ui, acts);
        if let Some(m) = &state.message {
            ui.label(m);
        }
        ui.separator();
        if view.rows.is_empty() && view.broken.is_empty() {
            ui.label("No plug-ins are installed.");
        }
        egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
            for row in &view.rows {
                plugin_row(state, ui, &view.app, row, acts);
            }
            for broken in &view.broken {
                ui.group(|ui| {
                    ui.strong(&broken.id);
                    ui.label(format!("State: {}", broken.state));
                    if ui.button("Remove").clicked() {
                        acts.push(Act::Remove(broken.id.clone()));
                    }
                });
            }
        });
    });
    if !open {
        state.manager_open = false;
    }
}

fn install_row(state: &mut UiState, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        ui.label("File:");
        ui.add(egui::TextEdit::singleline(&mut state.install_path).hint_text("path to a .wasm plug-in").desired_width(320.0));
        if state.picker.is_some() && ui.button("Browse...").clicked() {
            let picked = state.picker.as_ref().and_then(|p| p());
            if let Some(path) = picked {
                state.install_path = path.to_string_lossy().into_owned();
            }
        }
        let ready = !state.install_path.trim().is_empty();
        if ui.add_enabled(ready, egui::Button::new("Install")).clicked() {
            acts.push(Act::Install(state.install_path.trim().to_owned()));
        }
    });
}

fn plugin_row(state: &mut UiState, ui: &mut egui::Ui, app: &str, row: &PluginRow, acts: &mut Vec<Act>) {
    let info = &row.info;
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.strong(format!("{} {}", info.id, info.version));
            ui.label(format!("State: {}", state_text(&info.state)));
        });
        if !info.unsupported_kinds.is_empty() {
            ui.label(format!("Partly supported. This version of {app} cannot run: {}.", info.unsupported_kinds.join(", ")));
        }
        if info.update_pending {
            ui.label("A newer version waits for your permission.");
        }
        if let Some(l) = &info.legacy {
            ui.label(format!("Older plug-in format ({:?}). It keeps its original limits:", l.abi));
            for (name, value) in &l.limits {
                ui.label(format!("  {name}: {value}"));
            }
        }
        if let PluginState::Faulted { reason } = &info.state {
            ui.label(format!("Why it stopped: {reason}"));
        }
        if let Some(progress) = &row.progress {
            ui.label(format!("Progress: {progress}"));
        }
        if !info.commands.is_empty() {
            ui.label(format!("Commands: {}", info.commands.join(", ")));
        }
        for h in &info.hooks.registered {
            ui.label(format!("Hook active: {h}"));
        }
        let left_out =
            info.hooks.skipped.iter().map(|s| (&s.contribution, &s.reason)).chain(info.surfaces.skipped.iter().map(|s| (&s.contribution, &s.reason)));
        for (name, reason) in left_out {
            ui.label(format!("Left out: {name} ({reason})"));
        }
        for s in &info.surfaces.registered {
            ui.horizontal(|ui| {
                ui.label(format!("Provides: {s}"));
                if row.panels.contains(s) && ui.button("Open panel").clicked() {
                    acts.push(Act::OpenPanel(info.id.clone(), s.clone()));
                }
            });
        }
        if let Ok(mut review) = GrantsReview::new(&info.id, &info.grants) {
            ui.collapsing(format!("Permissions ({})", review.granted().len()), |ui| {
                for action in show_review(ui, &mut review) {
                    let GrantAction::Revoke { permission, .. } = action;
                    acts.push(Act::Revoke(info.id.clone(), permission));
                }
            });
        }
        ui.horizontal(|ui| {
            if info.enabled {
                if ui.button("Disable").clicked() {
                    acts.push(Act::Disable(info.id.clone()));
                }
            } else if ui.button("Enable").clicked() {
                acts.push(Act::Enable(info.id.clone()));
            }
            if state.confirm_remove.as_deref() == Some(info.id.as_str()) {
                ui.label("Remove it and its saved data?");
                if ui.button("Yes, remove").clicked() {
                    state.confirm_remove = None;
                    acts.push(Act::Remove(info.id.clone()));
                }
                if ui.button("Keep").clicked() {
                    state.confirm_remove = None;
                }
            } else if ui.button("Remove").clicked() {
                state.confirm_remove = Some(info.id.clone());
            }
        });
    });
}

fn panel_windows(state: &mut UiState, ctx: &egui::Context, acts: &mut Vec<Act>) {
    for ((plugin, contribution), window) in &mut state.panels {
        let mut open = true;
        let mut events: Vec<UiEvent> = Vec::new();
        egui::Window::new(format!("{} ({plugin})", window.title)).id(egui::Id::unique(("plugin-panel", plugin, contribution))).open(&mut open).show(
            ctx,
            |ui| {
                if let Some(e) = &window.error {
                    ui.label(format!("Panel error: {e}"));
                }
                if let Some(tree) = window.tree.as_mut() {
                    events = show(ui, tree);
                }
            },
        );
        for e in events {
            acts.push(Act::Panel {
                plugin: plugin.clone(),
                contribution: contribution.clone(),
                event: json!({ "event": "action", "widget": e.widget_id, "value": e.value }),
            });
        }
        if !open {
            acts.push(Act::ClosePanel { plugin: plugin.clone(), contribution: contribution.clone() });
        }
    }
}
