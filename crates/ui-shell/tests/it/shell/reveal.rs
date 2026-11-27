//! Tab in a window shell: it hides and shows the chrome, and a screen-mode change clears what it did. A command reveal
//! pins an edge until the pointer moves, and `chrome_state` reads what the shell left.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::env::{RevealEdge, RootEnv, WindowKey};
use craft_ui::runtime::focus::{KeyUse, host_canvas};
use craft_ui::runtime::frame::{ChromeToggle, request_chrome_toggle};
use craft_ui::shell::{AppShell, ChromeState, ScreenMode, Slot, chrome_state};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Pos2, Sense, ViewportId, pos2};

/// Where the pointer rests, well away from every strip and from the rail a reveal shows.
const HERE: Pos2 = pos2(300.0, 200.0);
/// A pointer position other than [`HERE`], as far from the strips.
const THERE: Pos2 = pos2(320.0, 210.0);

/// A window whose root lets a bare Tab toggle the chrome, as an app with a document canvas sets it.
fn window() -> Frames {
    let mut frames = Frames::new();
    let mut root = RootEnv::default();
    root.tab_toggles_chrome = true;
    frames.root(root);
    frames
}

/// What one pass of the shell showed.
struct Seen {
    slots: Vec<&'static str>,
    revealed: bool,
    panels_hidden: bool,
}

/// One frame of a shell with a tool rail, an options bar its own flag hides and a document canvas in the body, with
/// nothing focused, so Tab reaches the shell.
fn pass(windows: &mut Frames, mode: &mut ScreenMode, events: Vec<Event>) -> Seen {
    let mut seen = Seen { slots: Vec::new(), revealed: false, panels_hidden: false };
    windows.frame(events, |ui| {
        let out = AppShell::new(WindowKey::main())
            .screen_mode(Bind::bound(&mut *mode))
            .start(
                Slot::new("tools").reveal(),
                |_| Some(0),
                |ui, _, _: &mut ()| {
                    Text::new("Tools").show(ui);
                },
            )
            .options_bar(
                false,
                |_| Some(0),
                |ui, _, _: &mut ()| {
                    Text::new("Options").show(ui);
                },
            )
            .body(|ui, _, _| {
                let canvas = ui.scope_id().with("document");
                ui.interact(ui.max_rect(), canvas, Sense::click());
                host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
            })
            .show(ui, &mut ());
        seen = Seen { slots: out.regions.slots.iter().map(|(key, _)| *key).collect(), revealed: out.revealed, panels_hidden: out.panels_hidden };
    });
    seen
}

/// Returns how many slots show and whether a reveal shows them.
fn shown(windows: &mut Frames, mode: &mut ScreenMode, events: Vec<Event>) -> (usize, bool) {
    let seen = pass(windows, mode, events);
    (seen.slots.len(), seen.revealed)
}

fn settled(windows: &mut Frames, mode: &mut ScreenMode) -> Seen {
    (0..3).map(|_| pass(windows, mode, vec![])).last().expect("three passes")
}

fn settle(windows: &mut Frames, mode: &mut ScreenMode) -> (usize, bool) {
    let seen = settled(windows, mode);
    (seen.slots.len(), seen.revealed)
}

fn tabbed(windows: &mut Frames, mode: &mut ScreenMode) -> Seen {
    let key = |pressed| Event::Key { key: Key::Tab, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    pass(windows, mode, vec![key(true)]);
    pass(windows, mode, vec![key(false)]);
    settled(windows, mode)
}

fn tab(windows: &mut Frames, mode: &mut ScreenMode) -> (usize, bool) {
    let seen = tabbed(windows, mode);
    (seen.slots.len(), seen.revealed)
}

/// Asks the window's shell for `toggle` in its next pass, as a command handler does.
fn command(windows: &Frames, toggle: ChromeToggle) {
    request_chrome_toggle(&windows.ctx, ViewportId::ROOT, toggle);
}

fn reveal_start(windows: &Frames) {
    command(windows, ChromeToggle::Reveal(Some(RevealEdge::Start)));
}

fn state(windows: &Frames) -> (bool, bool, Option<RevealEdge>) {
    let state = chrome_state(&windows.ctx, ViewportId::ROOT);
    (state.hidden, state.panels_hidden, state.revealed)
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

#[trial]
fn a_command_reveal_stays_until_the_pointer_moves() {
    let mut windows = window();
    let mut mode = ScreenMode::FullScreen;
    shown(&mut windows, &mut mode, vec![moved(HERE)]);
    assert!(settled(&mut windows, &mut mode).slots.is_empty(), "full screen shows the body only");

    reveal_start(&windows);
    assert_eq!(settled(&mut windows, &mut mode).slots, ["tools"], "the command shows the start rail with the pointer far from its strip");
    assert_eq!(settled(&mut windows, &mut mode).slots, ["tools"], "the pin holds while the pointer rests");
    shown(&mut windows, &mut mode, vec![moved(THERE)]);
    assert!(settled(&mut windows, &mut mode).slots.is_empty(), "a move of the pointer ends the pin");

    reveal_start(&windows);
    settled(&mut windows, &mut mode);
    // The pin is where the last move left the pointer: THERE.
    shown(&mut windows, &mut mode, vec![press(THERE)]);
    shown(&mut windows, &mut mode, vec![moved(HERE)]);
    assert_eq!(settled_slots(&mut windows, &mut mode, 1), ["tools"], "a held button freezes the pin");
    shown(&mut windows, &mut mode, vec![release(HERE)]);
    assert!(settled(&mut windows, &mut mode).slots.is_empty(), "the pin ends once the button is up and the pointer has moved");

    reveal_start(&windows);
    assert_eq!(settled(&mut windows, &mut mode).slots, ["tools"]);
    command(&windows, ChromeToggle::Reveal(None));
    assert!(settled(&mut windows, &mut mode).slots.is_empty(), "Reveal(None) ends the pin");

    reveal_start(&windows);
    settled(&mut windows, &mut mode);
    assert_eq!(tabbed(&mut windows, &mut mode).slots.len(), 2, "Tab reveals every slot that reveals");
    assert!(tabbed(&mut windows, &mut mode).slots.is_empty(), "the second Tab ends the reveal and the pin with it");

    reveal_start(&windows);
    settled(&mut windows, &mut mode);
    mode = ScreenMode::Standard;
    assert_eq!(settled(&mut windows, &mut mode).slots, ["tools"], "normal mode shows the rail");
    mode = ScreenMode::FullScreen;
    assert!(settled(&mut windows, &mut mode).slots.is_empty(), "a screen-mode change ended the pin");

    mode = ScreenMode::Standard;
    settled(&mut windows, &mut mode);
    command(&windows, ChromeToggle::PanelsKeepTools);
    settled(&mut windows, &mut mode);
    reveal_start(&windows);
    settled(&mut windows, &mut mode);
    assert_eq!(state(&windows).2, Some(RevealEdge::Start), "hidden panels take the pin as well");
    shown(&mut windows, &mut mode, vec![moved(THERE)]);
    settled(&mut windows, &mut mode);
    assert_eq!(state(&windows).2, None, "a move ends it there too");
}

/// The slots after `passes` more passes.
fn settled_slots(windows: &mut Frames, mode: &mut ScreenMode, passes: usize) -> Vec<&'static str> {
    (0..passes).map(|_| pass(windows, mode, vec![])).last().expect("a pass").slots
}

#[trial]
fn chrome_state_reads_the_hiding_the_shell_left_and_show_panels_ends_it() {
    let mut windows = window();
    let mut mode = ScreenMode::Standard;
    assert_eq!(chrome_state(&windows.ctx, ViewportId::ROOT), ChromeState::default(), "no shell has run");
    let seen = settled(&mut windows, &mut mode);
    assert_eq!(state(&windows), (false, false, None));
    assert!(!seen.panels_hidden);

    let seen = tabbed(&mut windows, &mut mode);
    assert_eq!(state(&windows), (true, false, None), "Tab hid the chrome");
    assert!(!seen.panels_hidden && !seen.revealed);
    tabbed(&mut windows, &mut mode);
    assert_eq!(state(&windows), (false, false, None), "Tab showed it");

    command(&windows, ChromeToggle::PanelsKeepTools);
    let seen = settled(&mut windows, &mut mode);
    assert_eq!(state(&windows), (false, true, None), "Shift+Tab hid the panels");
    assert!(seen.panels_hidden, "the pass's output says the same");
    command(&windows, ChromeToggle::ShowPanels);
    let seen = settled(&mut windows, &mut mode);
    assert_eq!(state(&windows), (false, false, None), "ShowPanels ended it");
    assert!(!seen.panels_hidden);
    command(&windows, ChromeToggle::ShowPanels);
    settled(&mut windows, &mut mode);
    assert_eq!(state(&windows), (false, false, None), "ShowPanels with the panels shown does nothing");

    mode = ScreenMode::FullScreen;
    shown(&mut windows, &mut mode, vec![moved(HERE)]);
    settled(&mut windows, &mut mode);
    reveal_start(&windows);
    let seen = settled(&mut windows, &mut mode);
    assert_eq!(state(&windows), (false, false, Some(RevealEdge::Start)), "a command reveal names its edge");
    assert!(seen.revealed && seen.slots == ["tools"], "the pass's output shows the revealed rail");

    assert_eq!(
        chrome_state(&windows.ctx, ViewportId::from_hash_of("other window")),
        ChromeState::default(),
        "a viewport with no shell reads the default"
    );
}
