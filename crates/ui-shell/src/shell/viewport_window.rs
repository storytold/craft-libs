//! `ViewportWindow`: an app window in its own OS window where windows exist, else a dialog layer over the parent.

#[cfg(feature = "testing")]
mod stories;

use crate::contract::{Bind, BodyCx, NoValue, Outcome, Overlay, Pattern, Props, SaveKey, ShowContainer as _};
use crate::dialog::Dialog;
use crate::runtime::env::Label;
use crate::runtime::overlay::CloseReason;
use egui::{Response, Ui};

/// What a window pass reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app stops showing the window when it asks to close"]
pub struct ViewportWindowOutput {
    /// The window's response.
    pub response: Response,
    /// How the window asked to close this pass: its close action, Escape (once a popup in it has had the key) or the
    /// OS close button. `None` while it stays open. The window stays until the app stops showing it.
    pub close_requested: Option<CloseReason>,
}

impl Outcome for ViewportWindowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A window of the app with a body the app fills.
///
/// The app calls [`show`](ViewportWindow::show) on every pass the window should exist and stops calling it to close
/// the window, which it does when the output asks for it.
///
/// - **Placement.** The window opens in its own OS window where there are any, else as a dialog layer over its
///   parent. [`overlay`](ViewportWindow::overlay) changes that layer, which also gives the OS window its limits and
///   place: `size_limits`, `placed` and `resizable`.
/// - **App-modal.** By default the app's windows ignore input while the window is open, and a press on one raises
///   this window. `app_modal(false)` makes it a tool window that blocks nothing.
/// - **Keys.** Escape asks to close, unless a popup in the window is open.
#[must_use = "show the window with `show`"]
#[derive(Pattern, Props)]
pub struct ViewportWindow {
    title: Label,
    /// Whether the app's other windows ignore input while this one is open.
    #[prop]
    app_modal: bool,
    /// Changes the layer the window has where windows are unavailable, whose limits and place its OS window takes.
    overlay: Option<Box<dyn FnOnce(Overlay) -> Overlay>>,
    /// Keeps the window's place and size across runs under `key`.
    #[prop]
    remember: Option<SaveKey>,
}

impl ViewportWindow {
    /// An app-modal window titled `title`.
    pub fn new(title: impl Into<Label>) -> Self {
        ViewportWindow { title: title.into(), app_modal: true, overlay: None, remember: None }
    }

    /// Changes the layer the window falls back to, which starts as the resizable dialog layer: the smallest and
    /// largest size (`Overlay::size_limits`), the place (`Overlay::placed`) and the resizability. Calls compose in
    /// order.
    pub fn overlay(mut self, change: impl FnOnce(Overlay) -> Overlay + 'static) -> Self {
        self.overlay = Some(match self.overlay.take() {
            Some(earlier) => Box::new(move |overlay| change(earlier(overlay))),
            None => Box::new(change),
        });
        self
    }

    /// Shows the window for this pass with `body` as its content.
    pub fn show(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> ViewportWindowOutput {
        let ViewportWindow { title, app_modal, overlay, remember } = self;
        let mut dialog = Dialog::new(title).resizable(true);
        if let Some(change) = overlay {
            dialog = dialog.overlay(change);
        }
        if !app_modal {
            dialog = dialog.modeless();
        }
        dialog = dialog.viewport();
        if let Some(key) = remember {
            dialog = dialog.remember(key);
        }
        let mut open = true;
        let shown = dialog.opened(Bind::bound(&mut open)).show(ui, |ui, cx| body(ui, cx));
        ViewportWindowOutput { response: shown.output.response, close_requested: shown.output.closed }
    }
}
