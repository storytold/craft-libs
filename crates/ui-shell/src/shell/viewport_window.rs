//! `ViewportWindow`: an app window in its own OS window where windows exist, else a dialog layer over the parent.

#[cfg(feature = "testing")]
mod stories;

use crate::contract::{Bind, BodyCx, NoValue, Outcome, Overlay, Pattern, Props, SaveKey, ShowContainer as _};
use crate::dialog::Dialog;
use crate::runtime::env::Label;
use crate::runtime::overlay::{Backdrop, CloseReason};
use egui::{Response, Ui};

/// What a window pass reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app stops showing the window when it asks to close"]
pub struct ViewportWindowOutput {
    /// The window's response.
    pub response: Response,
    /// How the window asked to close this pass: its close action, the OS close button, or in an app-modal window
    /// Escape (once a popup in it has had the key). `None` while it stays open. The window stays until the app stops showing it.
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
/// - **App-modal.** By default the window blocks nothing. [`app_modal`](ViewportWindow::app_modal) makes the app's
///   other windows ignore input while it is open, and a press on one raises this window.
/// - **Keys.** Escape asks to close an app-modal window, unless a popup in it is open; a tool window ignores it.
#[must_use = "show the window with `show`"]
#[derive(Pattern, Props)]
pub struct ViewportWindow {
    title: Label,
    /// What lies over the app's other windows while this one is open and blocks them; `None` blocks nothing.
    modal: Option<Backdrop>,
    /// Changes the layer the window has where windows are unavailable, whose limits and place its OS window takes.
    overlay: Option<Box<dyn FnOnce(Overlay) -> Overlay>>,
    /// Keeps the window's place and size across runs under `key`.
    #[prop]
    remember: Option<SaveKey>,
}

impl ViewportWindow {
    /// A window titled `title` that blocks nothing.
    pub fn new(title: impl Into<Label>) -> Self {
        ViewportWindow { title: title.into(), modal: None, overlay: None, remember: None }
    }

    /// Makes the app's other windows ignore input while this one is open, with `backdrop` over them where the window is
    /// a layer (`Backdrop::Scrim` dims them, `Backdrop::None` leaves them as they are).
    #[must_use]
    pub fn app_modal(mut self, backdrop: Backdrop) -> Self {
        self.modal = Some(backdrop);
        self
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
        let ViewportWindow { title, modal, overlay, remember } = self;
        let mut dialog = Dialog::new(title).resizable(true);
        if let Some(change) = overlay {
            dialog = dialog.overlay(change);
        }
        dialog = match modal {
            Some(backdrop) => dialog.backdrop(backdrop),
            None => dialog.modeless(),
        };
        dialog = dialog.viewport();
        if let Some(key) = remember {
            dialog = dialog.remember(key);
        }
        let mut open = true;
        let shown = dialog.opened(Bind::bound(&mut open)).show(ui, |ui, cx| body(ui, cx));
        ViewportWindowOutput { response: shown.output.response, close_requested: shown.output.closed }
    }
}
