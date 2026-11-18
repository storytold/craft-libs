//! Screen mode: the shell brings its OS window to the bound mode in one pass and follows the OS out of full screen, and
//! its keys reach it with nothing focused and give way to a held key's scope. Focus shows the screen bar and a
//! drop-down slot, and the F key cycles within the shell's mode set.

use craft_host::target::{Os, ShellInfo};
use craft_ui::contract::{Bind, CommandSet, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::take_output;
use craft_ui::shell::{AppShell, ScreenMode, ScreenModeSet, Slot};
use craft_ui::testing::pointer::{moved, press, release};
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

/// What one frame of the Focus trial's shell laid out: the body, the screen bar and the drop-down slot when shown.
#[derive(Clone, Copy, Debug)]
struct Laid {
    body: Rect,
    screen: Option<Rect>,
    drops: Option<Rect>,
    revealed: bool,
    dropped: bool,
}

impl Default for Laid {
    fn default() -> Self {
        Self { body: Rect::NOTHING, screen: None, drops: None, revealed: false, dropped: false }
    }
}

/// One frame with `events` of a document shell with a screen bar and a drop-down slot, its mode bound to `mode` and
/// its drop-down flag to `dropped`; with `rerun` the frame runs a second pass. Returns the last pass's layout and the
/// number of passes.
fn focus_frame(frames: &mut Frames, mode: &mut ScreenMode, dropped: &mut bool, events: Vec<Event>, rerun: bool) -> (Laid, usize) {
    let mut laid = Laid::default();
    let passes = frames.frame(events, |ui| {
        let out = AppShell::new(WindowKey::main())
            .mode_set(ScreenModeSet::Document)
            .screen_mode(Bind::bound(&mut *mode))
            .dropped(Bind::bound(&mut *dropped))
            .top(Slot::new("screen").screen_bar(), |ui, _, _: &mut ()| {
                Text::new("Screen").show(ui);
            })
            .top(Slot::new("ribbon").drops(), |ui, _, _: &mut ()| {
                Text::new("Ribbon").show(ui);
            })
            .body(|ui, _, _| {
                Text::new("Document").show(ui);
            })
            .show(ui, &mut ());
        let rect_of = |key: &str| out.regions.slots.iter().find(|(shown, _)| *shown == key).map(|(_, rect)| *rect);
        laid = Laid { body: out.regions.body, screen: rect_of("screen"), drops: rect_of("ribbon"), revealed: out.revealed, dropped: out.dropped };
        if rerun && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial reruns the pass");
        }
    });
    (laid, passes)
}

fn focus_settle(frames: &mut Frames, mode: &mut ScreenMode, dropped: &mut bool) -> Laid {
    let mut laid = Laid::default();
    for _ in 0..SETTLED_FRAMES {
        laid = focus_frame(frames, mode, dropped, vec![], false).0;
    }
    laid
}

/// A press at `at` whose press frame runs two passes, then the release and a settle; the layout after it.
fn press_at(frames: &mut Frames, mode: &mut ScreenMode, dropped: &mut bool, at: Pos2) -> Laid {
    focus_frame(frames, mode, dropped, vec![moved(at)], false);
    let (_, passes) = focus_frame(frames, mode, dropped, vec![press(at)], true);
    assert_eq!(passes, 2, "the press frame ran a second pass");
    focus_frame(frames, mode, dropped, vec![release(at)], false);
    focus_settle(frames, mode, dropped)
}

/// Focus shows the screen bar above the body and takes its room; the drop-down slot shows while `dropped` is set and
/// covers the body, leaving its rect as it is. A press in the slot keeps it; one press in the body clears the flag,
/// whose frame's second pass, with no press of its own, leaves it cleared. A mode change clears it too. Neither slot
/// counts as a reveal.
#[trial]
fn focus_shows_the_screen_bar_and_a_dropped_slot_covers_the_body_until_a_press_in_it() {
    let mut frames = Frames::new();
    let (mut mode, mut dropped) = (ScreenMode::Standard, false);
    let standard = focus_settle(&mut frames, &mut mode, &mut dropped);
    assert_eq!((standard.screen, standard.drops), (None, None), "Standard shows neither slot");

    mode = ScreenMode::Focus;
    let focus = focus_settle(&mut frames, &mut mode, &mut dropped);
    let screen = focus.screen.expect("Focus shows the screen bar");
    assert_eq!(screen.top(), standard.body.top(), "the screen bar lies at the window's top");
    assert_eq!(focus.body.top(), screen.bottom(), "the body is laid out below the screen bar");
    assert_eq!((focus.drops, focus.revealed, focus.dropped), (None, false, false), "the drop-down slot waits for its flag");

    dropped = true;
    let shown = focus_settle(&mut frames, &mut mode, &mut dropped);
    let drops = shown.drops.expect("the drop-down slot shows while its flag is set");
    assert_eq!(shown.body, focus.body, "the drop-down slot covers the body and leaves its rect");
    assert!(drops.intersects(shown.body) && drops.top() >= screen.bottom(), "it lies over the body, below the bar: {drops:?}");
    assert!(shown.dropped && !shown.revealed, "the shell reports the slot, and it is no reveal");

    let kept = press_at(&mut frames, &mut mode, &mut dropped, drops.center());
    assert!(dropped && kept.drops.is_some(), "a press in the drop-down slot is the slot's own");

    let below = Pos2::new(shown.body.center().x, shown.body.bottom() - 10.0);
    assert!(!drops.contains(below), "the trial presses in the body, clear of the slot");
    let cleared = press_at(&mut frames, &mut mode, &mut dropped, below);
    assert!(!dropped, "a press in the body clears the flag");
    assert_eq!((cleared.drops, cleared.dropped), (None, false), "and the slot folds away");
    assert_eq!(cleared.body, focus.body, "the body keeps its rect");

    dropped = true;
    assert!(focus_settle(&mut frames, &mut mode, &mut dropped).drops.is_some(), "the slot shows again");
    mode = ScreenMode::Immersive;
    let immersive = focus_settle(&mut frames, &mut mode, &mut dropped);
    assert!(!dropped, "a mode change clears the flag");
    assert!(immersive.screen.is_some() && immersive.drops.is_none(), "Immersive keeps the screen bar: {immersive:?}");
}

/// One frame with `events` of a shell whose screen mode is bound to `mode` and cycles within `set`; with `rerun` the
/// frame runs a second pass.
fn cycled(windows: &Windows, mode: &mut ScreenMode, set: ScreenModeSet, events: Vec<Event>, rerun: bool) -> Out {
    windows.frame(events, vec![], |ui| {
        AppShell::new(WindowKey::main())
            .mode_set(set)
            .screen_mode(Bind::bound(&mut *mode))
            .body(|ui, _, _| {
                Text::new("Document").show(ui);
            })
            .show(ui, &mut ());
        if rerun && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial reruns the pass");
        }
    })
}

/// Presses and releases F with nothing focused, the press frame running two passes, then settles; the full screen
/// and maximize commands those frames sent.
fn press_f(windows: &Windows, mode: &mut ScreenMode, set: ScreenModeSet) -> Vec<ViewportCommand> {
    let mut sent = screen_commands(&cycled(windows, mode, set, vec![key(Key::F, true)], true));
    sent.extend(screen_commands(&cycled(windows, mode, set, vec![key(Key::F, false)], false)));
    for _ in 0..SETTLED_FRAMES {
        sent.extend(screen_commands(&cycled(windows, mode, set, vec![], false)));
    }
    sent
}

/// F steps once per press through the shell's mode set and wraps: `Document` cycles Standard, Full Screen With Menu
/// Bar, Focus, Immersive and back, `Panels` reaches Full Screen, and a mode outside the set steps to Standard. Entering
/// Focus from Standard sends one full screen command; Focus to Immersive sends none.
#[trial]
fn the_cycle_follows_the_window_mode_set() {
    let windows = Windows::new();
    windows.root_info(root(false, false));
    let mut mode = ScreenMode::Standard;
    for _ in 0..SETTLED_FRAMES {
        cycled(&windows, &mut mode, ScreenModeSet::Document, vec![], false);
    }
    let mut seen = Vec::new();
    for _ in 0..4 {
        press_f(&windows, &mut mode, ScreenModeSet::Document);
        seen.push(mode);
    }
    assert_eq!(
        seen,
        [ScreenMode::FullScreenWithMenuBar, ScreenMode::Focus, ScreenMode::Immersive, ScreenMode::Standard],
        "F steps once per press through the Document set and wraps"
    );

    press_f(&windows, &mut mode, ScreenModeSet::Panels);
    press_f(&windows, &mut mode, ScreenModeSet::Panels);
    assert_eq!(mode, ScreenMode::FullScreen, "the Panels set reaches Full Screen");
    press_f(&windows, &mut mode, ScreenModeSet::Document);
    assert_eq!(mode, ScreenMode::Standard, "a mode outside the Document set steps to Standard");

    for _ in 0..SETTLED_FRAMES {
        cycled(&windows, &mut mode, ScreenModeSet::Document, vec![], false);
    }
    mode = ScreenMode::Focus;
    assert_eq!(
        screen_commands(&cycled(&windows, &mut mode, ScreenModeSet::Document, vec![], false)),
        [ViewportCommand::Fullscreen(true)],
        "Standard to Focus sends one full screen command"
    );
    windows.root_info(root(true, false));
    for _ in 0..SETTLED_FRAMES {
        cycled(&windows, &mut mode, ScreenModeSet::Document, vec![], false);
    }
    let sent = press_f(&windows, &mut mode, ScreenModeSet::Document);
    assert_eq!(mode, ScreenMode::Immersive, "F in Focus gives Immersive");
    assert!(sent.is_empty(), "Focus to Immersive stays in full screen and sends nothing: {sent:?}");
}
