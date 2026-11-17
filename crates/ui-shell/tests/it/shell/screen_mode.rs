//! Screen mode: the shell brings its OS window to the bound mode in one pass and follows the OS out of full screen, and
//! its keys reach it with nothing focused and give way to a held key's scope.

use craft_host::target::{Os, ShellInfo};
use craft_ui::contract::{Bind, CommandSet, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::take_output;
use craft_ui::shell::{AppShell, ScreenMode};
use craft_ui::testing::windows::FakeSystem;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::windows::{Out, Windows};
use egui::{Event, Key, Modifiers, Pos2, Rect, ViewportCommand, ViewportId, ViewportInfo};
use std::sync::Arc;

const WINDOW: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(800.0, 600.0) };
const SETTLED_FRAMES: usize = 3;

fn root(fullscreen: bool, maximized: bool) -> ViewportInfo {
    ViewportInfo {
        focused: Some(true),
        inner_rect: Some(WINDOW),
        outer_rect: Some(WINDOW),
        native_pixels_per_point: Some(1.0),
        fullscreen: Some(fullscreen),
        maximized: Some(maximized),
        ..ViewportInfo::default()
    }
}

/// One frame of a shell whose screen mode is bound to `mode`.
fn frame(windows: &Windows, mode: &mut ScreenMode) -> Out {
    windows.frame(vec![], vec![], |ui| {
        AppShell::new(WindowKey::main())
            .screen_mode(Bind::bound(mode))
            .body(|ui, _, _| {
                Text::new("Document").show(ui);
            })
            .show(ui, &mut ());
    })
}

fn settle(windows: &Windows, mode: &mut ScreenMode) {
    for _ in 0..SETTLED_FRAMES {
        frame(windows, mode);
    }
}

/// The full screen and maximize commands the frame sent to the root window, in order.
fn screen_commands(out: &Out) -> Vec<ViewportCommand> {
    out.commands
        .iter()
        .filter(|(viewport, command)| {
            *viewport == ViewportId::ROOT && matches!(command, ViewportCommand::Fullscreen(_) | ViewportCommand::Maximized(_))
        })
        .map(|(_, command)| command.clone())
        .collect()
}

#[trial]
fn an_os_exit_from_full_screen_writes_standard() {
    let windows = Windows::new();
    windows.root_info(root(false, false));
    let mut mode = ScreenMode::Standard;
    settle(&windows, &mut mode);
    mode = ScreenMode::FullScreen;
    frame(&windows, &mut mode);
    windows.root_info(root(true, false));
    settle(&windows, &mut mode);
    assert_eq!(mode, ScreenMode::FullScreen);
    windows.root_info(root(false, false));
    let mut sent = Vec::new();
    for _ in 0..SETTLED_FRAMES {
        sent.extend(screen_commands(&frame(&windows, &mut mode)));
    }
    assert_eq!(mode, ScreenMode::Standard, "the window left full screen on its own, as the macOS green button does");
    assert!(!sent.contains(&ViewportCommand::Fullscreen(true)), "the shell does not send the window back: {sent:?}");
}

#[trial]
fn on_windows_a_maximized_window_leaves_maximized_before_full_screen_and_returns_to_it() {
    let windows = Windows::with(InstallSpec::new(ShellInfo::default().os(Os::Windows), Arc::new(FakeSystem::new())));
    windows.root_info(root(false, true));
    let mut mode = ScreenMode::Standard;
    settle(&windows, &mut mode);
    mode = ScreenMode::FullScreen;
    assert_eq!(
        screen_commands(&frame(&windows, &mut mode)),
        [ViewportCommand::Maximized(false), ViewportCommand::Fullscreen(true)],
        "full screen and maximized never overlap, and both go out in one pass"
    );
    windows.root_info(root(true, false));
    settle(&windows, &mut mode);
    mode = ScreenMode::Standard;
    assert_eq!(screen_commands(&frame(&windows, &mut mode)), [ViewportCommand::Fullscreen(false), ViewportCommand::Maximized(true)]);
}

/// An application command on F, and a momentary command whose held F12 makes the `trialHeld` scope's keys work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "trialKeys")]
enum TrialKeys {
    #[command(label = "App F", default = "F")]
    AppF,
    #[command(label = "Hold", kind = "momentary", default = "F12", holds = "trialHeld")]
    Hold,
}

/// The command F runs while F12 is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "trialHeld", scope = "trialHeld")]
enum HeldKeys {
    #[command(label = "Next", default = "F")]
    Next,
}

/// One frame of a shell over a plain body, with nothing focused, its mode bound to `mode`; the ids of the commands
/// that reached the application since the last frame.
fn keyed(frames: &mut Frames, mode: &mut ScreenMode, events: Vec<Event>) -> Vec<String> {
    frames.frame(events, |ui| {
        AppShell::new(WindowKey::main())
            .screen_mode(Bind::bound(&mut *mode))
            .body(|ui, _, _| {
                Text::new("Document").show(ui);
            })
            .show(ui, &mut ());
    });
    take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect()
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

/// Presses and releases `pressed`, then settles; the commands that reached the application.
fn tap(frames: &mut Frames, mode: &mut ScreenMode, pressed: Key) -> Vec<String> {
    let mut fired = keyed(frames, mode, vec![key(pressed, true)]);
    fired.extend(keyed(frames, mode, vec![key(pressed, false)]));
    for _ in 0..SETTLED_FRAMES {
        fired.extend(keyed(frames, mode, vec![]));
    }
    fired
}

/// With nothing focused, a key both the window and the application bind runs the window's command: the shell's root
/// takes the keys.
#[trial]
fn a_window_command_takes_its_key_with_nothing_focused() {
    let mut frames = Frames::new();
    let mut mode = ScreenMode::Standard;
    for _ in 0..SETTLED_FRAMES {
        keyed(&mut frames, &mut mode, vec![]);
    }
    let fired = tap(&mut frames, &mut mode, Key::F);
    assert_eq!(mode, ScreenMode::FullScreenWithMenuBar, "F cycles the screen mode");
    assert!(!fired.iter().any(|id| id == TrialKeys::AppF.id()), "the application's F does not run: {fired:?}");
}

/// While a momentary command's key is held, its scope's keys come before the window's; once it is up, the window's
/// key runs again.
#[trial]
fn a_held_keys_scope_takes_its_keys_before_the_window() {
    let mut frames = Frames::new();
    let mut mode = ScreenMode::Standard;
    for _ in 0..SETTLED_FRAMES {
        keyed(&mut frames, &mut mode, vec![]);
    }
    keyed(&mut frames, &mut mode, vec![key(Key::F12, true)]);
    let fired = tap(&mut frames, &mut mode, Key::F);
    assert_eq!(mode, ScreenMode::Standard, "F with F12 held leaves the screen mode");
    assert_eq!(fired.iter().filter(|id| *id == HeldKeys::Next.id()).count(), 1, "and runs the held scope's command: {fired:?}");
    keyed(&mut frames, &mut mode, vec![key(Key::F12, false)]);
    let fired = tap(&mut frames, &mut mode, Key::F);
    assert_eq!(mode, ScreenMode::FullScreenWithMenuBar, "F with F12 up cycles the screen mode");
    assert!(!fired.iter().any(|id| id == HeldKeys::Next.id()), "{fired:?}");
}
