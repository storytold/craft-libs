//! The RAW Develop workspace: a full-window editor of the Camera Raw settings over the app's develop.

mod commands;
mod controls;
mod filmstrip;
mod scopes;
mod source;

use craft_behavior::{EditEvent, PixelSize};
use craft_ui::canvas::NavigationOptions;
use craft_ui::contract::Pattern;
use craft_ui::primitives::Checker;
use craft_ui::runtime::env::Label;

pub use commands::RawDevelopCommands;
pub use controls::{RawDevelopPanel, RawDevelopPanelOutput};
pub use filmstrip::{RawFilmItem, RawFilmstrip};
pub use scopes::{RawFloatingScope, RawScopeStrip, RawScopeStripOutput};
pub use source::{Demosaic, RawDevelopSource, RawPreviewRequest};

use crate::raw::RawDevelopSettings;
use crate::ui::scopes::{ScopePanelState, ScopeSettings};

/// Where the workspace's result goes: the footer's buttons follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawTarget {
    /// A filter on a layer: OK and Cancel.
    Filter,
    /// A smart filter: OK and Cancel.
    SmartFilter,
    /// Raws being opened: Open, Cancel and Done.
    OpenRaw,
}

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RawDevelopOpen {
    /// The raw's or the layer's name: the title's subject.
    pub subject: String,
    /// The picture's size in document pixels.
    pub size: PixelSize,
    /// Where the result goes.
    pub target: RawTarget,
    /// The settings to start from.
    pub settings: RawDevelopSettings,
    /// The settings Reset restores.
    pub original: RawDevelopSettings,
    /// The demosaic, while raws are being opened.
    pub demosaic: Option<Demosaic>,
    /// The filmstrip tiles, one per raw.
    pub filmstrip: Vec<RawFilmItem>,
    /// The raw shown.
    pub current: usize,
    /// The scope's state.
    pub scope: ScopePanelState,
    /// The user's navigation options.
    pub navigation: NavigationOptions,
    /// The user's transparency grid.
    pub checkerboard: Option<Checker>,
}

impl RawDevelopOpen {
    /// Opens on `subject` of `size` with `settings`, for `target`.
    pub fn new(subject: impl Into<String>, size: PixelSize, target: RawTarget, settings: RawDevelopSettings) -> Self {
        let _ = (subject.into(), size, target, settings);
        todo!("dl.K66")
    }
}

/// What the workspace reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RawDevelopEvent {
    /// The settings changed.
    Settings(EditEvent<RawDevelopSettings>),
    /// A filmstrip pick: the app keeps the current raw's settings and calls `show_item`.
    Show(usize),
    /// Load Settings asks the app for a file.
    LoadSettings,
    /// Save Settings asks the app for a file.
    SaveSettings,
    /// The scope's settings changed: the app persists them.
    ScopeSettings(ScopeSettings),
}

/// How the workspace finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawFinish {
    /// OK of a filter or a smart filter.
    Ok,
    /// Open of raws being opened.
    Open,
    /// Done of raws being opened.
    Done,
}

/// What OK answers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RawDevelopResult {
    /// The button that finished it.
    pub finish: RawFinish,
    /// The settings at the end.
    pub settings: RawDevelopSettings,
    /// The demosaic, while raws are being opened.
    pub demosaic: Option<Demosaic>,
}

/// The RAW Develop workspace, titled "RAW Develop (name)" or "RAW Develop Filter (layer)".
#[derive(Pattern)]
pub struct RawDevelopWorkspace {
    settings: RawDevelopSettings,
}

impl RawDevelopWorkspace {
    /// A workspace over `open`.
    pub fn new(open: RawDevelopOpen) -> Self {
        let _ = open;
        todo!("dl.K66")
    }

    /// The settings as they are now.
    pub fn session(&self) -> &RawDevelopSettings {
        &self.settings
    }

    /// Loads `settings` as one history step.
    pub fn load(&mut self, settings: RawDevelopSettings) {
        let _ = settings;
        todo!("dl.K66")
    }

    /// Shows raw `current` with its `settings`, the `original` Reset restores and its `demosaic`.
    pub fn show_item(&mut self, current: usize, settings: RawDevelopSettings, original: RawDevelopSettings, demosaic: Option<Demosaic>) {
        let _ = (current, settings, original, demosaic);
        todo!("dl.K66")
    }

    /// Sets the picture of filmstrip tile `index` and whether the raw is edited.
    pub fn set_item_picture(&mut self, index: usize, picture: egui::load::SizedTexture, edited: bool) {
        let _ = (index, picture, edited);
        todo!("dl.K66")
    }

    /// Shows `message` in the status line.
    pub fn status(&mut self, message: Option<Label>) {
        let _ = message;
        todo!("dl.K66")
    }
}

// P3(dialogs-w17): dl.K66 — the workspace's trait surface; `show` and the automation fields are written in phase 3.
impl craft_ui::dialog::FullWindowWorkspace for RawDevelopWorkspace {
    type Source = dyn RawDevelopSource;
    type Event = RawDevelopEvent;
    type Result = RawDevelopResult;

    const KEY: &'static str = "filter.cameraRaw";

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        cx: craft_ui::dialog::WorkspaceCx<'_, Self::Source>,
    ) -> craft_ui::dialog::WorkspacePass<Self::Event, Self::Result> {
        let _ = (ui, cx);
        todo!("dl.K66")
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        todo!("dl.K66")
    }

    fn set_json_field(&mut self, ctx: &egui::Context, source: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        let _ = (ctx, source, field, value);
        todo!("dl.K66")
    }

    fn confirm(&mut self, ctx: &egui::Context, source: &Self::Source) -> Option<Self::Result> {
        let _ = (ctx, source);
        todo!("dl.K66")
    }
}
