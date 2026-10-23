#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Quit a Craft app the platform's own way.
//!
//! On macOS an eframe app must not leave by closing its window. eframe drops the window while
//! AppKit's run loop is still running, and in the next display cycle AppKit's Touch Bar finder
//! tries to remove a KVO observer from the view that is gone
//! (`-[_NSTouchBarFinderObservation invalidate]`). That is an uncaught Objective-C exception and
//! the app crashes (seen on macOS 27 on a Touch Bar Mac).
//! `-[NSApplication terminate:]`, what ⌘Q does in winit's default menu, ends the process before
//! another display cycle runs.
//!
//! [`quit`] is that quit. With the `egui` feature, [`on_close_request`] routes a window close
//! through it. Put both the app's close decision and this hook in `eframe::App::logic`, with the
//! hook last. eframe calls `logic` even while the window is minimized or hidden, when `ui` does
//! not run. Move any unsaved-changes check out of `ui` too: `logic` runs before `ui`, so a check
//! left in `ui` would happen after the quit. Keep prompt state in the app and render it in `ui`:
//!
//! ```ignore
//! fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
//!     if ctx.input(|i| i.viewport().close_requested())
//!         && self.has_unsaved_changes()
//!         && !self.discard_confirmed
//!     {
//!         ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
//!         self.show_save_prompt = true;
//!     }
//!     craft_quit::on_close_request(ctx); // last, so it sees the app's `CancelClose`
//! }
//!
//! fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!     // ... the app's UI, including the prompt when show_save_prompt is true ...
//! }
//! ```
//!
//! The prompt sends `ViewportCommand::Close` only after Save has succeeded (and cleared the
//! dirty state), or Don't Save has set `discard_confirmed`. Cancel, a cancelled file picker and
//! a failed save keep the app open; reset prompt/confirmation state when the decision is cancelled
//! or a new document is opened. An asynchronous save must finish before requesting Close. If the
//! window is hidden, bring it back before showing the prompt so the user can answer it.
//!
//! The app still saves what it keeps in `eframe::App::on_exit`: winit's
//! `applicationWillTerminate:` runs eframe's exit handling before the process ends. The process
//! ends there, so code after `eframe::run_native` in `main` does not run; move it to `on_exit`.

#[cfg(feature = "egui")]
mod close;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(all(feature = "menu", target_os = "macos"))]
mod menu;

#[cfg(feature = "egui")]
pub use close::{QUIT_MENU_ID, on_close_request, on_menu_event};

/// Replace winit's default macOS app menu with one whose Quit (⌘Q) asks the app first.
///
/// winit's Quit ends the process without asking, so unsaved work is lost. This menu's Quit sends
/// the root window a `ViewportCommand::Close` instead: the app's unsaved-changes check runs as it
/// does for the close button, and [`on_close_request`] then quits. Call it once, from the eframe
/// app creator: winit builds its default menu before that and would replace one installed
/// earlier. `app_name` is shown in the menu ("Quit WordCraft").
///
/// For apps without an app menu of their own. It replaces the whole menu and sets muda's
/// process-wide menu-event handler, so an app that builds its own muda menu should not call it:
/// give that menu's Quit item [`QUIT_MENU_ID`] and pass its events to [`on_menu_event`] instead.
/// muda keeps the first handler set, so this and `craft_menu::mac::install` are alternatives: an
/// app calls one of them.
///
/// On error winit's menu stays: the app still runs, but ⌘Q quits without asking. The app decides
/// whether that is fatal; `?` passes it out of the eframe app creator.
///
/// Other platforms: does nothing and returns `Ok`.
#[cfg(feature = "menu")]
pub fn install_app_menu(ctx: &egui::Context, app_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "macos")]
    menu::install(ctx, app_name)?;
    #[cfg(not(target_os = "macos"))]
    let _ = (ctx, app_name);
    Ok(())
}

/// Quit through the platform's own path, where leaving by closing the window is not safe.
///
/// macOS: schedules `-[NSApplication terminate:]` and returns `true`; the caller keeps its window
/// open and the process ends shortly after, once the current event has been handled. Must be
/// called while the app's event loop is running (any eframe callback is). Return to the event loop
/// before blocking: a blocking native dialog opened in the same callback would run the quit while
/// winit is still handling the event, which panics.
///
/// Other platforms: does nothing and returns `false`; the caller closes its window as usual.
pub fn quit() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::terminate_later();
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
