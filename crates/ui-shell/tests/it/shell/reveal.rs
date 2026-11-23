//! Tab in a window shell: it hides and shows the chrome, and a screen-mode change clears what it did.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::env::{RootEnv, WindowKey};
use craft_ui::runtime::focus::{KeyUse, host_canvas};
use craft_ui::shell::{AppShell, ScreenMode, Slot};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Sense};

/// A window whose root lets a bare Tab toggle the chrome, as an app with a document canvas sets it.
fn window() -> Frames {
    let mut frames = Frames::new();
    let mut root = RootEnv::default();
    root.tab_toggles_chrome = true;
    frames.root(root);
    frames
}

/// One frame of a shell with a tool rail, an options bar its own flag hides and a document canvas in the body, with
/// nothing focused, so Tab reaches the shell. Returns how many slots show and whether a reveal shows them.
fn shown(windows: &mut Frames, mode: &mut ScreenMode, events: Vec<Event>) -> (usize, bool) {
    let mut result = (0, false);
    windows.frame(events, |ui| {
        let out = AppShell::new(WindowKey::main())
            .screen_mode(Bind::bound(&mut *mode))
            .start(Slot::new("tools").reveal(), |ui, _, _: &mut ()| {
                Text::new("Tools").show(ui);
            })
            .options_bar(false, |ui, _, _: &mut ()| {
                Text::new("Options").show(ui);
            })
            .body(|ui, _, _| {
                let canvas = ui.scope_id().with("document");
                ui.interact(ui.max_rect(), canvas, Sense::click());
                host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
            })
            .show(ui, &mut ());
        result = (out.regions.slots.len(), out.revealed);
    });
    result
}

fn settle(windows: &mut Frames, mode: &mut ScreenMode) -> (usize, bool) {
    let mut last = (0, false);
    for _ in 0..3 {
        last = shown(windows, mode, vec![]);
    }
    last
}

fn tab(windows: &mut Frames, mode: &mut ScreenMode) -> (usize, bool) {
    let key = |pressed| Event::Key { key: Key::Tab, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    shown(windows, mode, vec![key(true)]);
    shown(windows, mode, vec![key(false)]);
    settle(windows, mode)
}

#[trial]
fn tab_hides_and_shows_the_chrome_and_a_screen_mode_change_clears_it() {
    let mut windows = window();
    let mut mode = ScreenMode::Standard;
    assert_eq!(settle(&mut windows, &mut mode), (1, false), "the rail shows; the options bar stays hidden by its flag");
    assert_eq!(tab(&mut windows, &mut mode).0, 0, "Tab hides the chrome");
    assert_eq!(tab(&mut windows, &mut mode).0, 1, "Tab shows it again, and the slot hidden by its flag stays hidden");

    mode = ScreenMode::FullScreen;
    assert_eq!(settle(&mut windows, &mut mode), (0, false), "full screen shows the body only");
    assert_eq!(
        tab(&mut windows, &mut mode),
        (2, true),
        "Tab in body-only mode reveals every slot that reveals, the options bar its flag hides in normal mode included"
    );
    assert_eq!(mode, ScreenMode::FullScreen, "the reveal leaves the screen mode as it is");
    assert_eq!(tab(&mut windows, &mut mode), (0, false), "Tab ends the reveal");

    tab(&mut windows, &mut mode);
    mode = ScreenMode::Standard;
    assert_eq!(settle(&mut windows, &mut mode), (1, false), "a screen-mode change ends a reveal");

    assert_eq!(tab(&mut windows, &mut mode).0, 0, "Tab hides the chrome");
    mode = ScreenMode::FullScreenWithMenuBar;
    assert_eq!(settle(&mut windows, &mut mode).0, 1, "a screen-mode change clears the user's hiding");
}
