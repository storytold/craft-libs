//! The hooks an app with its own eframe `App` calls to host the runtime: [`install`] once, then [`before_input`] and
//! [`logic`] each frame and [`before_ui`] each pass, in that order. `app::run`'s driver is such an app and calls the
//! same hooks. What the app calls where, and what the install takes over, is in docs/craft-framework.md section 2.2.
//!
//! The window's keys that no component took (an Escape while nothing has framework focus) are read with
//! [`crate::runtime::keys::with_window_intents`], last in the app's UI.

use crate::fonts::{Stack, UiFonts};
#[cfg(feature = "app")]
use crate::runtime::egui::memory;
use crate::runtime::frame::{FrameInput, FrameOutput, set_input, take_output};
use crate::runtime::windows;
use crate::runtime::{InstallError, InstallSpec};
use egui::Context;

/// What the app decides when the root window is asked to close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseDecision {
    /// The window may close; the platform quit takes it from there.
    #[default]
    Close,
    /// The window stays open, brought forward, while the app asks (for example about unsaved documents) and closes
    /// it with `WindowAction::Close` once the answer allows.
    Hold,
}

/// Installs the runtime on `ctx`, then `fonts`. Call it once, when the app is created (eframe's `CreationContext`).
/// Returns the installed font stack; an error when the context has the runtime already or `spec` replaces one style
/// key twice. An app that installs its fonts again later (a language change) calls [`UiFonts::install`] itself.
pub fn install(ctx: &Context, spec: InstallSpec, fonts: &UiFonts<'_>) -> Result<Stack, InstallError> {
    crate::install(ctx, spec)?;
    Ok(fonts.install(ctx))
}

/// Before each frame, from eframe's `App::raw_input_hook`: hands `app` what the runtime accumulated since the last
/// frame (fired commands, announcements, the chrome toggle) and gives the coming passes the input `app` returns
/// (the root env, the command states, the host's services). The app reads the output first, so the input it returns
/// already reflects it.
pub fn before_input(ctx: &Context, app: impl FnOnce(FrameOutput) -> FrameInput) {
    let input = app(take_output(ctx));
    set_input(ctx, input);
}

/// Each frame, from eframe's `App::logic`, after the app's own logic: runs the root window's close path. When the
/// root window was asked to close and no modal layer refuses it, `close_requested` decides; then the platform quit
/// takes the close where the platform has one. eframe runs `logic` while the window is minimized or hidden too, so
/// the close path runs there and nowhere else.
pub fn logic(ctx: &Context, close_requested: impl FnOnce() -> CloseDecision) {
    if windows::root_close_requested(ctx) && close_requested() == CloseDecision::Hold {
        windows::hold_close(ctx);
    }
    windows::logic(ctx);
}

/// Each pass of the root window, from eframe's `App::ui`, before the app's UI: hands the runtime the root's native
/// window where the runtime drives it (Windows), and fits the root window to its monitor once the monitor is known.
#[cfg(feature = "app")]
pub fn before_ui(ctx: &Context, frame: &eframe::Frame) {
    #[cfg(windows)]
    windows::route_root(ctx, || craft_window::native::NativeWindow::of_handle(frame));
    #[cfg(not(windows))]
    let _ = frame;
    fit_once(ctx);
}

/// Whether the fit has run for this context.
#[cfg(feature = "app")]
#[derive(Default)]
struct Fitted(bool);

/// Runs [`fit_first`] from the first passes until its monitor is known, then never again for this context.
#[cfg(feature = "app")]
fn fit_once(ctx: &Context) {
    if memory::with::<Fitted, _>(ctx, |fitted| fitted.0) {
        return;
    }
    if fit_first(ctx) {
        memory::with::<Fitted, _>(ctx, |fitted| fitted.0 = true);
    }
}

/// Maximizes the root window, once its monitor is known, when it is larger than the monitor's usable area (Photoshop
/// opens maximized on a small display). Never on macOS, which keeps new windows inside the screen's visible frame
/// itself, and never for a window that is already maximized or in full screen. Returns whether the monitor was
/// known, which is when the fit has run.
#[cfg(feature = "app")]
fn fit_first(ctx: &Context) -> bool {
    use crate::runtime::windows::{WindowAction, WindowHost};
    use craft_host::target::{Os, WindowState};
    use craft_window::desktop::fits;
    use egui::ViewportId;
    let host = WindowHost::of(ctx);
    let monitor = ctx.input(|input| input.viewport().monitor_size);
    let (Some(facts), Some(monitor)) = (host.facts(ViewportId::ROOT), monitor) else { return false };
    let outer = facts.outer.unwrap_or(facts.inner);
    if windows::os_of(ctx) != Os::Mac
        && facts.state == WindowState::Normal
        && monitor.is_finite()
        && outer.is_finite()
        && !fits(outer.size(), monitor)
    {
        host.act(ViewportId::ROOT, WindowAction::SetMaximized(true));
    }
    true
}
