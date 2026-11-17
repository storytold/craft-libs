//! The canvas preset's slot keys and `CanvasPanel`, which lays out the canvas panel in the body of a document editor's
//! shell. The editor's slots are the canvas setters of `AppShell` (`title_bar_if`, `options_bar`, `status`).
//!
//! ```text
//! +-----------------------------------------+
//! | title                                   |
//! | options                                 |
//! +-----------------------------------------+
//! |  body (dock with toolbar, canvas panel) |
//! +-----------------------------------------+
//! | status                                  |
//! +-----------------------------------------+
//!
//! canvas panel:  +----------------------+
//!                | document tabs        |
//!                +----------------------+
//!                | document             |
//!                +----------------------+
//! ```

#[cfg(feature = "testing")]
mod stories;

use super::app_shell::{RegionBody, ShellRegion};
use super::words::ShellWords;
use crate::contract::{BodyCx, NoValue, Outcome, PaneEdge, Props as _, ShellPane, ShowContainer as _};
use crate::primitives::{CheckerArea, paint_surround};
use crate::runtime::env::{BodyMode, Env, ShellEntry};
use crate::style::{SurfaceKind, SurroundLook};
use egui::{CornerRadius, Rect, Response, Ui};

/// The slot keys of the preset.
pub mod slots {
    use crate::runtime::env::SlotKey;

    /// The title bar: the menu bar and window controls.
    pub const TITLE: SlotKey = "title";
    /// The options bar, below the title.
    pub const OPTIONS: SlotKey = "options";
    /// The status bar, the lowest bottom bar.
    pub const STATUS: SlotKey = "status";
}

/// What a [`CanvasPanel`] drew.
#[derive(Debug)]
#[non_exhaustive]
pub struct CanvasPanelOutput {
    /// The response of the document region.
    pub response: Response,
    /// The tab strip's rect, when it showed.
    pub tabs: Option<Rect>,
    /// The document area.
    pub document: Rect,
    /// The second press of a double click on the document pane's own surface, where no document control took it.
    pub empty_double: bool,
}

impl Outcome for CanvasPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The canvas panel of a dock: the document tab strip flush against the document, then the document area, over the
/// canvas surround. Fill the tab strip with `tabs::DocumentTabs`.
///
/// - **Tabs.** They show in normal mode when the host says so (`ShellCx::tabs_visible`), or while documents are
///   [`opening`](CanvasPanel::opening) (a file that is still loading has a tab). Body-only mode shows no tabs.
/// - **Surround.** The document area shows the canvas surround, and a double click on its empty room is
///   [`empty_double`](CanvasPanelOutput::empty_double).
///
/// With no documents the host shows its start screen instead, or the panel with an empty document area. Outside a
/// shell the panel shows everything.
#[must_use = "show the panel with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct CanvasPanel<'a> {
    /// The document tab strip.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    tabs: Option<RegionBody<'a>>,
    /// Documents are opening: the tab strip shows even when the host hides the tabs, in normal mode.
    #[prop]
    opening: bool,
}

impl<'a> CanvasPanel<'a> {
    /// A panel with no tab strip.
    pub fn new() -> Self {
        CanvasPanel { tabs: None, opening: false }
    }

    /// Lays the panel out in `ui`, the body of the dock's canvas panel, with `document` in the area left.
    pub fn show(self, ui: &mut Ui, mut document: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> CanvasPanelOutput {
        let CanvasPanel { tabs, opening } = self;
        let env = Env::of(ui);
        let words = env.words::<ShellWords>();
        let shell = env.get::<ShellEntry>().0;
        let normal = shell.as_ref().is_none_or(|shell| shell.body_mode() == BodyMode::Normal);
        let tabs_shown = shell.as_ref().is_none_or(|shell| shell.tabs_visible()) || (opening && normal);

        let mut tabs_rect = None;
        if let (Some(mut add), true) = (tabs.map(|RegionBody(add)| add), tabs_shown) {
            // The document tabs sit on the window's background, as tab strips do.
            let out = ShellRegion::pane(ShellPane::new("document_tabs").edge(PaneEdge::Top))
                .surface(SurfaceKind::Background)
                .id_salt("document_tabs")
                .label(words.document_tabs.clone())
                .show(ui, |ui, cx| add(ui, cx));
            tabs_rect = Some(out.response.rect);
        }
        let mut empty_double = false;
        let out =
            ShellRegion::pane(ShellPane::new("document")).id_salt("document").label(words.document.clone()).senses_surface().show(ui, |ui, cx| {
                // Painted first, so a document the closure shows covers it.
                let look = SurroundLook::of(&env.tokens).shown(cx.env.canvas_pattern());
                paint_surround(ui.painter(), CheckerArea::Rect(cx.rect, CornerRadius::ZERO), &look);
                empty_double |= cx.surface_double_pressed();
                document(ui, cx);
            });
        CanvasPanelOutput { document: out.response.rect, response: out.output.response, tabs: tabs_rect, empty_double }
    }
}

impl Default for CanvasPanel<'_> {
    fn default() -> Self {
        Self::new()
    }
}
