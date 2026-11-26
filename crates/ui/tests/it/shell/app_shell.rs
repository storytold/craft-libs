//! The app shell's own rules: the reveal state machine.

use craft_i18n::Direction;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::runtime::env::{RootEnv, WindowKey};
use craft_ui::shell::{AppShell, AppShellOutput, ScreenMode, Slot};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Pos2, Ui, pos2};
use std::cell::{Cell, RefCell};

const CENTER: Pos2 = pos2(400.0, 300.0);
const TOP_STRIP: Pos2 = pos2(400.0, 1.0);

/// One window of a shell with a revealing slot on the top, start and end edges and a status bar that never
/// reveals, driven by the pointer and keys as a user would.
struct Window {
    frames: Frames,
    mode: Cell<ScreenMode>,
    shown: RefCell<Option<AppShellOutput>>,
}

impl Window {
    /// A window in `mode`, its canvas focused so that Tab and F reach the shell.
    fn new(mode: ScreenMode, direction: Direction) -> Self {
        let mut frames = Frames::new();
        let mut root = RootEnv::default();
        root.tab_toggles_chrome = true;
        root.direction = direction;
        frames.root(root);
        let mut window = Window { frames, mode: Cell::new(mode), shown: RefCell::new(None) };
        window.pass(Vec::new());
        window.pass(Vec::new());
        window.pass(vec![moved(CENTER)]);
        window.pass(vec![press(CENTER)]);
        window.pass(vec![release(CENTER)]);
        window
    }

    fn pass(&mut self, events: Vec<Event>) {
        let mut show = |ui: &mut Ui| {
            let mut mode = self.mode.get();
            let out = AppShell::new(WindowKey::main())
                .screen_mode(Bind::bound(&mut mode))
                .title(Slot::new("title").reveal(), |ui, _, _| {
                    Text::new("Title").show(ui);
                })
                .start(Slot::new("tools").reveal(), |ui, _, _| {
                    Text::new("Tools").show(ui);
                })
                .end(Slot::new("inspector").reveal(), |ui, _, _| {
                    Text::new("Inspector").show(ui);
                })
                .bottom(Slot::new("status"), |ui, _, _| {
                    Text::new("Status").show(ui);
                })
                .body(|ui, _, _| {
                    Canvas::new("Document").show(ui, |_, _| ());
                })
                .show(ui, &mut ());
            self.mode.set(mode);
            *self.shown.borrow_mut() = Some(out);
        };
        self.frames.frame(events, &mut show);
    }

    /// The keys of the slots the last pass showed.
    fn slots(&self) -> Vec<&'static str> {
        self.shown.borrow().as_ref().expect("a pass ran").regions.slots.iter().map(|(key, _)| *key).collect()
    }
}

#[trial]
fn a_held_button_freezes_the_edge_reveal() {
    let mut window = Window::new(ScreenMode::FullScreen, Direction::Ltr);
    window.pass(vec![moved(TOP_STRIP)]);
    window.pass(vec![press(TOP_STRIP)]);
    window.pass(vec![moved(CENTER)]);
    assert_eq!(window.slots(), ["title"], "frozen: the reveal does not end while the button is held");
    window.pass(vec![release(CENTER)]);
    window.pass(Vec::new());
    assert!(window.slots().is_empty(), "the reveal ends once the button is up and the pointer is away");

    let mut window = Window::new(ScreenMode::FullScreen, Direction::Ltr);
    window.pass(vec![moved(CENTER)]);
    window.pass(vec![press(CENTER)]);
    window.pass(vec![moved(TOP_STRIP)]);
    assert!(window.slots().is_empty(), "frozen: the reveal does not start while the button is held");
}
