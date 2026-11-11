//! Screen modes (the F key) and the window actions that enter and leave full screen.

use crate::contract::{BindMut, BodyCx};
use crate::runtime::commands::Commands as _;
use crate::runtime::egui::pass;
use crate::runtime::windows::{WindowAction, WindowHost, os_of};
use craft_behavior::{EditEvent, Mixed};
use craft_host::target::{Os, WindowState};
use egui::Context;
use std::sync::Arc;

/// The three screen modes. The shell keeps the current one in its `screen_mode` binding and maps it to the body mode,
/// the document tabs and the scroll bars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScreenMode {
    /// Everything shows.
    #[default]
    Standard,
    /// Every bar, rail and panel stays; the document tabs and the scroll bars hide.
    FullScreenWithMenuBar,
    /// The body only.
    FullScreen,
}

impl ScreenMode {
    /// Whether the shell shows the body only.
    pub fn body_only(self) -> bool {
        self == Self::FullScreen
    }

    /// Whether the document tabs show.
    pub fn tabs_visible(self) -> bool {
        self == Self::Standard
    }

    /// Whether the window is in OS full screen.
    pub fn is_full_screen(self) -> bool {
        self != Self::Standard
    }

    /// The mode the F key selects: Standard, Full Screen With Menu Bar, Full Screen, Standard.
    pub fn next(self) -> Self {
        match self {
            Self::Standard => Self::FullScreenWithMenuBar,
            Self::FullScreenWithMenuBar => Self::FullScreen,
            Self::FullScreen => Self::Standard,
        }
    }

    /// The mode Shift+F selects.
    pub fn previous(self) -> Self {
        match self {
            Self::Standard => Self::FullScreen,
            Self::FullScreenWithMenuBar => Self::Standard,
            Self::FullScreen => Self::FullScreenWithMenuBar,
        }
    }
}

/// The screen-mode commands of a window. The three modes are radio rows of View > Screen Mode; the cycle commands are
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
    /// The next mode.
    #[command(label = "Cycle Screen Mode", default = "F")]
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
        }
    }

    /// The mode the command leaves a window in that is in `current`.
    pub fn apply(self, current: ScreenMode) -> ScreenMode {
        match self {
            ScreenModeCommands::Standard => ScreenMode::Standard,
            ScreenModeCommands::FullScreenWithMenuBar => ScreenMode::FullScreenWithMenuBar,
            ScreenModeCommands::FullScreen => ScreenMode::FullScreen,
            ScreenModeCommands::Cycle => current.next(),
            ScreenModeCommands::CycleBack => current.previous(),
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
    /// Writes the mode back under `name` when it changed. Returns the mode and whether it changed.
    pub(super) fn follow(
        &mut self,
        ctx: &Context,
        cx: &mut BodyCx<'_>,
        mut bind: BindMut<'_, ScreenMode>,
        name: Arc<str>,
        os: bool,
    ) -> (ScreenMode, bool) {
        let before = match bind.read(cx) {
            Some(Mixed::Same(held)) => held,
            _ => ScreenMode::default(),
        };
        let mut mode = before;
        while let Some((command, _)) = cx.take_command::<ScreenModeCommands>() {
            mode = command.apply(mode);
        }
        if os {
            mode = self.follow_window(ctx, mode);
        }
        self.applied = mode;
        if mode != before {
            let event = EditEvent::set(Mixed::Same(before), mode, name, None);
            let _ = bind.write(cx, &event);
        }
        (mode, mode != before)
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
