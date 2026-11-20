//! Screen modes (the F key) and the window actions that enter and leave full screen.

use crate::contract::{BindMut, BodyCx};
use crate::runtime::commands::Commands as _;
use crate::runtime::egui::pass;
use crate::runtime::windows::{WindowAction, WindowHost, os_of};
use crate::style::SurfaceKind;
use craft_behavior::{EditEvent, Mixed};
use craft_host::target::{Os, WindowState};
use egui::Context;
use std::sync::Arc;

/// The screen modes. The shell keeps the current one in its `screen_mode` binding and maps it to the body mode,
/// the document tabs and the scroll bars; its [`ScreenModeSet`] says which of them the F key cycles through. Saved as
/// `"standard"`, `"fullScreenWithMenuBar"`, `"fullScreen"`, `"focus"` and `"immersive"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ScreenMode {
    /// Everything shows.
    #[default]
    Standard,
    /// Every bar, rail and panel stays; the document tabs and the scroll bars hide.
    FullScreenWithMenuBar,
    /// The body only.
    FullScreen,
    /// The body only, with the screen bar; the surround is black.
    Focus,
    /// The body only, with the screen bar, filling the pages' canvas.
    Immersive,
}

/// Which screen modes the F key cycles through. The shell publishes its set to its regions as an env value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui_macros::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum ScreenModeSet {
    /// Standard, Full Screen With Menu Bar and Full Screen.
    #[default]
    Panels,
    /// Standard, Full Screen With Menu Bar, Focus and Immersive; F11 gives Full Screen With Menu Bar.
    Document,
}

impl ScreenModeSet {
    /// The modes the set cycles through, in order.
    pub fn modes(self) -> &'static [ScreenMode] {
        match self {
            ScreenModeSet::Panels => &[ScreenMode::Standard, ScreenMode::FullScreenWithMenuBar, ScreenMode::FullScreen],
            ScreenModeSet::Document => &[ScreenMode::Standard, ScreenMode::FullScreenWithMenuBar, ScreenMode::Focus, ScreenMode::Immersive],
        }
    }
}

impl ScreenMode {
    /// The surface around the body in this mode: `Some(Scrim)` for `Focus` only, `None` for every other mode.
    /// Immersive fills the room with the body's own canvas, so it paints nothing behind it.
    pub fn surround(self) -> Option<SurfaceKind> {
        (self == Self::Focus).then_some(SurfaceKind::Scrim)
    }

    /// Whether the shell shows the body only: Full Screen, Focus and Immersive.
    pub fn body_only(self) -> bool {
        matches!(self, Self::FullScreen | Self::Focus | Self::Immersive)
    }

    /// Whether the mode shows the screen bar: Focus and Immersive.
    pub(super) fn has_screen_bar(self) -> bool {
        matches!(self, Self::Focus | Self::Immersive)
    }

    /// Whether the document tabs show.
    pub fn tabs_visible(self) -> bool {
        self == Self::Standard
    }

    /// Whether the window is in OS full screen.
    pub fn is_full_screen(self) -> bool {
        self != Self::Standard
    }

    /// The mode the F key selects: the one after this in `set`'s order, wrapping to the first. A mode outside the set
    /// steps to Standard.
    pub fn next(self, set: ScreenModeSet) -> Self {
        self.step(set, 1)
    }

    /// The mode Shift+F selects: the one before this in `set`'s order, wrapping to the last. A mode outside the set
    /// steps to Standard.
    pub fn previous(self, set: ScreenModeSet) -> Self {
        self.step(set, set.modes().len().saturating_sub(1))
    }

    /// The mode `by` places after this one in `set`'s order, wrapping.
    fn step(self, set: ScreenModeSet, by: usize) -> Self {
        let modes = set.modes();
        match modes.iter().position(|mode| *mode == self) {
            Some(at) => modes.get((at + by) % modes.len()).copied().unwrap_or_default(),
            None => Self::Standard,
        }
    }
}

/// The screen-mode commands of a window. The modes are radio rows of View > Screen Mode; the cycle commands are
/// the F key and Shift+F. The window's shell takes them and writes its `screen_mode` binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "screenMode", scope = "window")]
#[non_exhaustive]
pub enum ScreenModeCommands {
    /// Everything shows.
    #[command(label = "Standard Screen Mode", kind = "radio", group = "screen_mode")]
    Standard,
    /// Every bar, rail and panel stays; the document tabs and the scroll bars hide.
    #[command(label = "Full Screen Mode With Menu Bar", kind = "radio", group = "screen_mode", alias = "screenMode.fullScreenWithMenuBar")]
    FullScreenWithMenuBar,
    /// The body only.
    #[command(label = "Full Screen Mode", kind = "radio", group = "screen_mode", alias = "screenMode.fullScreen")]
    FullScreen,
    /// The body only, with the screen bar; the surround is black.
    #[command(label = "Focus Screen Mode", kind = "radio", group = "screen_mode")]
    Focus,
    /// The body only, with the screen bar, filling the pages' canvas.
    #[command(label = "Immersive Screen Mode", kind = "radio", group = "screen_mode")]
    Immersive,
    /// The next mode.
    #[command(label = "Cycle Screen Mode", default = "F", icon = "app_window")]
    Cycle,
    /// The previous mode.
    #[command(label = "Cycle Screen Mode Backwards", default = "Shift+F")]
    CycleBack,
}

impl ScreenModeCommands {
    /// The command that selects `mode`.
    pub const fn of(mode: ScreenMode) -> Self {
        match mode {
            ScreenMode::Standard => ScreenModeCommands::Standard,
            ScreenMode::FullScreenWithMenuBar => ScreenModeCommands::FullScreenWithMenuBar,
            ScreenMode::FullScreen => ScreenModeCommands::FullScreen,
            ScreenMode::Focus => ScreenModeCommands::Focus,
            ScreenMode::Immersive => ScreenModeCommands::Immersive,
        }
    }

    /// The mode the command leaves a window in that is in `current`.
    pub fn apply(self, current: ScreenMode, set: ScreenModeSet) -> ScreenMode {
        match self {
            ScreenModeCommands::Standard => ScreenMode::Standard,
            ScreenModeCommands::FullScreenWithMenuBar => ScreenMode::FullScreenWithMenuBar,
            ScreenModeCommands::FullScreen => ScreenMode::FullScreen,
            ScreenModeCommands::Focus => ScreenMode::Focus,
            ScreenModeCommands::Immersive => ScreenMode::Immersive,
            ScreenModeCommands::Cycle => current.next(set),
            ScreenModeCommands::CycleBack => current.previous(set),
        }
    }
}

/// The shell's record of how its window follows the screen mode, kept between passes.
///
/// The window is brought to the mode in one pass, with the actions of [`sequence`]; the shell acts whenever the
/// bound mode and the mode last applied differ in full screen, whoever wrote the bound mode (the F key, a menu
/// command, an app writing Standard on Escape). When the OS leaves full screen on its own (the macOS green button),
/// the mode follows it back to Standard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ScreenModeFollow {
    /// The mode last sent to the window.
    applied: ScreenMode,
    /// The window was in OS full screen in the last pass that had its facts.
    os_full: bool,
    /// The window was maximized when it went to full screen, and returns to it on Windows.
    was_maximized: bool,
}

impl ScreenModeFollow {
    /// Reads the mode from `bind` and applies this pass's screen-mode commands. With `os`, follows the OS window and
    /// brings it to the mode; an embedded shell passes false and its mode only changes what its own region shows.
    /// Writes the mode back under `name` when this pass changed it. Returns the mode and whether it differs from the mode
    /// the last pass applied, whoever wrote it.
    pub(super) fn follow(
        &mut self,
        ctx: &Context,
        cx: &mut BodyCx<'_>,
        mut bind: BindMut<'_, ScreenMode>,
        name: Arc<str>,
        os: bool,
        set: ScreenModeSet,
    ) -> (ScreenMode, bool) {
        let before = match bind.read(cx) {
            Some(Mixed::Same(held)) => held,
            _ => ScreenMode::default(),
        };
        let last = self.applied;
        let mut mode = before;
        while let Some((command, _)) = cx.take_command::<ScreenModeCommands>() {
            mode = command.apply(mode, set);
        }
        if os {
            mode = self.follow_window(ctx, mode);
        }
        self.applied = mode;
        if mode != before {
            let event = EditEvent::set(Mixed::Same(before), mode, name, None);
            let _ = bind.write(cx, &event);
        }
        (mode, mode != last)
    }

    /// Follows the OS window out of full screen and sends the actions that bring it to `mode`. Returns the mode the
    /// window is in.
    fn follow_window(&mut self, ctx: &Context, mut mode: ScreenMode) -> ScreenMode {
        let host = WindowHost::of(ctx);
        let viewport = pass::current_viewport(ctx);
        let state = host.facts(viewport).map(|facts| facts.state);
        if let Some(state) = state {
            let in_full_screen = state == WindowState::Fullscreen;
            let left = self.os_full && !in_full_screen;
            self.os_full = in_full_screen;
            if left && mode.is_full_screen() && self.applied.is_full_screen() {
                mode = ScreenMode::Standard;
                self.applied = mode;
            }
        }
        if mode.is_full_screen() != self.applied.is_full_screen() {
            let keep_apart = os_of(ctx) == Os::Windows;
            let (actions, was_maximized) = sequence(keep_apart, state.unwrap_or(WindowState::Normal), self.was_maximized, mode.is_full_screen());
            self.was_maximized = was_maximized;
            for action in actions {
                host.act(viewport, action);
            }
        }
        mode
    }
}

/// The window actions that bring a window in `state` to full screen (`enter`) or back, in the order they are sent in
/// one pass, and whether the window is to be maximized again when it leaves full screen.
///
/// With `keep_apart` (Windows), full screen and maximized never overlap: a maximized window is restored before it goes
/// full screen and maximized again after it leaves.
///
/// WORKAROUND(winit#4756): winit on Windows keeps the maximized style on a window that goes full screen from
/// maximized, and the shell then draws the taskbar black instead of hiding it; on the way out the window stays
/// maximized at the monitor's full size, over the taskbar. Remove `keep_apart` once the winit in Cargo.lock has the
/// fix; check first with the reproduction gist of that issue (`--max`, no `--fix`: full screen must have no
/// WS_MAXIMIZE and hide the taskbar).
fn sequence(keep_apart: bool, state: WindowState, was_maximized: bool, enter: bool) -> (Vec<WindowAction>, bool) {
    if enter {
        if keep_apart && state == WindowState::Maximized {
            (vec![WindowAction::SetMaximized(false), WindowAction::SetFullscreen(true)], true)
        } else {
            (vec![WindowAction::SetFullscreen(true)], was_maximized)
        }
    } else if keep_apart && was_maximized {
        (vec![WindowAction::SetFullscreen(false), WindowAction::SetMaximized(true)], false)
    } else {
        (vec![WindowAction::SetFullscreen(false)], false)
    }
}
