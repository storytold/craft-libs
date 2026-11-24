//! `HostStyle`: an app's own egui code shown inside a framework part keeps the style and the background it has outside.

use crate::runtime::egui::style;
use egui::{Rect, Ui};
use std::sync::Arc;

/// The egui style of an app's own `Ui`, kept where a framework part takes the app's egui code into a `Ui` of its own:
/// the framework's regions set their layout spacing on the `Ui`s they hand out, and app code shown in one (a dock
/// panel's body that the app draws with egui) lays out and paints as it would outside the part when it runs through
/// [`HostStyle::run`] on [`HostStyle::paint_panel`]'s background.
#[derive(Clone, Debug)]
pub struct HostStyle(Arc<egui::Style>);

impl HostStyle {
    /// The style `ui` has now.
    pub fn of(ui: &Ui) -> Self {
        HostStyle(style::style_of(ui))
    }

    /// Runs `run` on `ui` with the kept style, and gives `ui` its own style back after it.
    pub fn run<R>(&self, ui: &mut Ui, run: impl FnOnce(&mut Ui) -> R) -> R {
        let own = style::style_of(ui);
        style::set_style(ui, &self.0);
        let shown = run(ui);
        style::set_style(ui, &own);
        shown
    }

    /// Fills `rect` on `ui` with the kept style's panel fill (`Visuals::panel_fill`), as an egui panel of the app paints
    /// its background.
    pub fn paint_panel(&self, ui: &Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, self.0.visuals.panel_fill);
    }
}
