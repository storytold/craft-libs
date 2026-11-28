//! `HostStyle`: an app's own egui code shown inside a framework part keeps the style and the background it has outside,
//! and runs once per pass.

use crate::runtime::egui::{alloc, memory, style};
use crate::runtime::{layout_memory, measure};
use egui::{Id, Rect, Ui, UiBuilder, Vec2};
use std::sync::Arc;

/// The egui style of an app's own `Ui`, kept where a framework part takes the app's egui code into a `Ui` of its own:
/// the framework's regions set their layout spacing on the `Ui`s they hand out, and app code shown in one (a dock
/// panel's body that the app draws with egui) lays out and paints as it would outside the part when it runs through
/// [`HostStyle::show`] on [`HostStyle::paint_panel`]'s background.
#[derive(Clone, Debug)]
pub struct HostStyle(Arc<egui::Style>);

/// What the last real run of the app's code under an id took and returned (`HostStyle::show`).
#[derive(Clone, Default)]
struct LastRun<R> {
    size: Option<Vec2>,
    out: R,
}

impl HostStyle {
    /// The style `ui` has now.
    pub fn of(ui: &Ui) -> Self {
        HostStyle(style::style_of(ui))
    }

    /// Runs `run` on `ui` with the kept style, and gives `ui` its own style back after it. For code the framework asks
    /// for a measure itself (a host view measured at a width); code a part shows goes through [`HostStyle::show`].
    pub fn run<R>(&self, ui: &mut Ui, run: impl FnOnce(&mut Ui) -> R) -> R {
        let own = style::style_of(ui);
        style::set_style(ui, &self.0);
        let shown = run(ui);
        style::set_style(ui, &own);
        shown
    }

    /// Shows the app's code `run` in `ui` as [`HostStyle::run`] runs it, in the real run of a pass only: in a child of
    /// `ui` with its layout and the rest of its room, scoped by `id`, which `ui` then takes. A measuring run
    /// (`runtime::measure`) does not run it: `ui` takes the size the code took in its last real run, and the call
    /// returns what that run returned (nothing and `R::default()` before the first). The app's code registers its
    /// widgets with egui and acts on input, so a run in a measuring run would show egui each widget twice in one pass,
    /// at two places.
    pub fn show<R: Clone + Default + Send + Sync + 'static>(&self, ui: &mut Ui, id: Id, run: impl FnOnce(&mut Ui) -> R) -> R {
        let ctx = ui.ctx().clone();
        if measure::active(&ctx) {
            let last = memory::peek_clone::<LastRun<R>>(&ctx, id).unwrap_or_default();
            if let Some(size) = last.size {
                alloc::space(ui, size);
            }
            return last.out;
        }
        let mut child = alloc::child(ui, UiBuilder::new().max_rect(ui.available_rect_before_wrap()).layout(*ui.layout()).scope_id(id));
        let out = self.run(&mut child, run);
        let rect = child.min_rect();
        // The child writes its widget rect when it drops.
        drop(child);
        alloc::advance_past(ui, rect);
        let size = rect.size();
        // A pass a kept measurement serves learns here that the code took another size since, and runs again.
        layout_memory::backstop(&ctx, id, size);
        memory::put(&ctx, id, LastRun { size: Some(size), out: out.clone() });
        out
    }

    /// Fills `rect` on `ui` with the kept style's panel fill (`Visuals::panel_fill`), as an egui panel of the app paints
    /// its background.
    pub fn paint_panel(&self, ui: &Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, self.0.visuals.panel_fill);
    }
}
