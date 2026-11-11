//! The canvas preset's slot keys and `CanvasPanel`, which lays out the canvas panel in the body of a document editor's
//! shell. The editor's slots are the canvas setters of `AppShell` (`title_bar_if`, `options_bar`, `tools`, `status`).
//!
//! ```text
//! +-----------------------------------------+
//! | title                                   |
//! | options                                 |
//! +-------+---------------------------------+
//! | tools |  body (dock with canvas panel)  |
//! +-------+---------------------------------+
//! | status                                  |
//! +-----------------------------------------+
//!
//! canvas panel:  +----------------------+------+
//!                | document tabs        | panel|
//!                +----------------------+ rail |
//!                | document             |      |
//!                +----------------------+------+
//! ```

#[cfg(feature = "testing")]
mod stories;

use super::app_shell::{RegionBody, RegionFn, ShellRegion};
use super::words::ShellWords;
use crate::contract::{BodyCx, NoValue, Outcome, PaneEdge, Props as _, ShellPane, ShowContainer as _};
use crate::runtime::env::{BodyMode, Env, Label, ShellEntry};
use egui::{Rect, Response, Ui};

/// The slot keys of the preset.
pub mod slots {
    use crate::runtime::env::SlotKey;

    /// The title bar: the menu bar and window controls.
    pub const TITLE: SlotKey = "title";
    /// The options bar, below the title.
    pub const OPTIONS: SlotKey = "options";
    /// The tool rail at the start edge.
    pub const TOOLS: SlotKey = "tools";
    /// The status bar, the lowest bottom bar.
    pub const STATUS: SlotKey = "status";
}

/// What a [`CanvasPanel`] drew.
#[derive(Debug)]
#[non_exhaustive]
pub struct CanvasPanelOutput {
    /// The response of the document region.
    pub response: Response,
    /// The rail's rect, when it showed.
    pub rail: Option<Rect>,
    /// The tab strip's rect, when it showed.
    pub tabs: Option<Rect>,
    /// The document area.
    pub document: Rect,
}

impl Outcome for CanvasPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The canvas panel of a dock: the panel rail at the end edge, spanning the tabs and the document; then, in what is
/// left, the document tab strip flush against the document; then the document area, which the shell's backdrop shows
/// through. Fill the rail with `rails::PanelRail` and the tab strip with `tabs::DocumentTabs`.
///
/// - **Tabs.** They show in normal mode when the host says so (`ShellCx::tabs_visible`), or while documents are
///   [`opening`](CanvasPanel::opening) (a file that is still loading has a tab). Body-only mode shows neither tabs nor
///   rail, but the rail shows during a reveal that shows the dock.
/// - **Rail.** It is skipped when the dock is hidden (`ShellCx::dock_visible`, or the panels are hidden with
///   Shift+Tab).
///
/// With no documents the host shows its start screen instead and does not use the panel. Outside a shell the panel
/// shows everything.
#[must_use = "show the panel with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct CanvasPanel<'a> {
    rail: Option<(Label, RegionFn<'a>)>,
    /// The document tab strip.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    tabs: Option<RegionBody<'a>>,
    /// Documents are opening: the tab strip shows even when the host hides the tabs, in normal mode.
    #[prop]
    opening: bool,
}

impl<'a> CanvasPanel<'a> {
    /// A panel with no rail and no tab strip.
    pub fn new() -> Self {
        CanvasPanel { rail: None, tabs: None, opening: false }
    }

    /// The panel rail, named `label`.
    pub fn rail(mut self, label: impl Into<Label>, add: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.rail = Some((label.into(), Box::new(add)));
        self
    }

    /// Lays the panel out in `ui`, the body of the dock's canvas panel, with `document` in the area left.
    pub fn show(self, ui: &mut Ui, mut document: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> CanvasPanelOutput {
        let CanvasPanel { rail, tabs, opening } = self;
        let env = Env::of(ui);
        let words = env.words::<ShellWords>();
        let shell = env.get::<ShellEntry>().0;
        let dock = shell.as_ref().is_none_or(|shell| shell.dock_visible());
        let normal = shell.as_ref().is_none_or(|shell| shell.body_mode() == BodyMode::Normal);
        let tabs_shown = shell.as_ref().is_none_or(|shell| shell.tabs_visible()) || (opening && normal);

        let mut rail_rect = None;
        if let (Some((label, mut add)), true) = (rail, dock) {
            let out =
                ShellRegion::pane(ShellPane::new("panel_rail").edge(PaneEdge::End)).id_salt("panel_rail").label(label).show(ui, |ui, cx| add(ui, cx));
            rail_rect = Some(out.response.rect);
        }
        let mut tabs_rect = None;
        if let (Some(mut add), true) = (tabs.map(|RegionBody(add)| add), tabs_shown) {
            let out = ShellRegion::pane(ShellPane::new("document_tabs").edge(PaneEdge::Top))
                .id_salt("document_tabs")
                .label(words.document_tabs.clone())
                .show(ui, |ui, cx| add(ui, cx));
            tabs_rect = Some(out.response.rect);
        }
        let out = ShellRegion::pane(ShellPane::new("document")).id_salt("document").label(words.document.clone()).show(ui, |ui, cx| document(ui, cx));
        CanvasPanelOutput { document: out.response.rect, response: out.output.response, rail: rail_rect, tabs: tabs_rect }
    }
}

impl Default for CanvasPanel<'_> {
    fn default() -> Self {
        Self::new()
    }
}
