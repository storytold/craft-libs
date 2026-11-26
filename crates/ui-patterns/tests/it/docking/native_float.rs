//! Float in a docked tab's menu, in a dock that opens its floats in OS windows: the panel's window opens hidden, as the
//! palettes yield to the open menu, and shows once the menu is gone.

use super::Numbered;
use craft_behavior::dock::{Layout, Node, Permissions, Workspace};
use craft_host::target::{Caps, Os, ShellInfo};
use craft_ui::contract::{Bind, BodyCx, Role, Tracked};
use craft_ui::docking::{DockConfig, DockPanels};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::InstallSpec;
use craft_ui::testing::access::{AccessTree, focused_key, name};
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::windows::Windows;
use egui::{Event, Key, Modifiers, Pos2, Rect, Ui, ViewportCommand, ViewportId, ViewportInfo};
use std::sync::Arc;

const FLOATED: usize = 1;
const LEFT: usize = 2;
/// The main window on the desktop.
const MAIN_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(900.0, 700.0) };
/// Where the OS puts the float's window.
const FLOAT_AT: Rect = Rect { min: Pos2::new(200.0, 180.0), max: Pos2::new(440.0, 480.0) };
/// The frames a menu's fade or a focus move may take.
const SETTLED: usize = 60;

/// The panel left behind asks for a second pass when it first shows, as egui's own containers do (`Grid`): in the frame
/// that opens the other panel's window.
struct Panels<'a> {
    left_shown: &'a mut bool,
}

impl DockPanels<usize> for Panels<'_> {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel == LEFT && !std::mem::replace(self.left_shown, true) {
            ui.ctx().request_discard("the panel left behind shows first");
        }
    }
}

/// A dock of one group of two panels, in a host that keeps the workspace and applies the dock's actions after its pass.
struct Host {
    windows: Windows,
    workspace: Arc<Workspace<usize>>,
    specs: Tracked<Numbered>,
    left_shown: bool,
    /// What the main window's last pass produced.
    tree: Option<AccessTree>,
    /// The float's OS window and whether it is visible: as its first builder opened it, then as the commands left it.
    window: Option<(ViewportId, bool)>,
}

impl Host {
    fn new() -> Self {
        let shell = ShellInfo::default().os(Os::Windows).caps(Caps::native(Os::Windows));
        let windows = Windows::with(InstallSpec::new(shell, Arc::new(FakeSystem::new())));
        windows.root_info(info(MAIN_AT));
        windows.child_info(info(FLOAT_AT));
        let workspace = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![FLOATED, LEFT], 0)), Vec::new())));
        let mut host = Host { windows, workspace, specs: Tracked::new(Numbered), left_shown: false, tree: None, window: None };
        for _ in 0..3 {
            host.frame(vec![]);
        }
        host
    }

    fn frame(&mut self, events: Vec<Event>) {
        let shown = Arc::clone(&self.workspace);
        let mut actions = Vec::new();
        let (specs, left_shown) = (&self.specs, &mut self.left_shown);
        let out = self.windows.frame(events, vec![], |ui| {
            let mut panels = Panels { left_shown: &mut *left_shown };
            actions = DockArea::new("Dock", Bind::given(Arc::clone(&shown)), specs, &mut panels, DockConfig::default()).show(ui).actions;
        });
        for action in actions {
            Arc::make_mut(&mut self.workspace).apply(action, false, |_| Permissions::ALL).expect("the dock's action applies");
        }
        if self.window.is_none() {
            let opened = out.builders.iter().find(|(viewport, _)| *viewport != ViewportId::ROOT);
            self.window = opened.map(|(viewport, builder)| (*viewport, builder.visible != Some(false)));
        }
        for (viewport, command) in &out.commands {
            if let (ViewportCommand::Visible(visible), Some((window, shown))) = (command, &mut self.window)
                && viewport == window
            {
                *shown = *visible;
            }
        }
        self.tree = out.trees.into_iter().find(|(viewport, _)| *viewport == ViewportId::ROOT).map(|(_, tree)| tree);
    }

    fn tree(&self) -> &AccessTree {
        self.tree.as_ref().expect("the main window ran a pass")
    }

    /// A frame with `event`, then the frames the app asks for.
    fn settle(&mut self, event: Event) {
        self.frame(vec![event]);
        for _ in 0..SETTLED {
            if !self.windows.ctx.has_requested_repaint() {
                return;
            }
            self.frame(vec![]);
        }
    }

    /// Where the main window shows the `role` named `name`.
    fn at(&self, name: &str, role: Role) -> Option<Pos2> {
        let tree = self.tree();
        let key = tree.with_role(role).into_iter().find(|key| tree.name_of(*key) == Some(name))?;
        tree.bounds_of(key).map(|rect| rect.center())
    }

    fn assert_floats_in_a_shown_window(&self) {
        let layout = self.workspace.layout();
        assert!(layout.floating.iter().any(|window| window.contains(&FLOATED)), "the panel floats: {layout:?}");
        assert!(self.window.is_some_and(|(_, shown)| shown), "its OS window shows: {:?}", self.window);
    }
}

fn info(rect: Rect) -> ViewportInfo {
    ViewportInfo {
        focused: Some(true),
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    }
}

/// Float clicked in the menu a secondary click on the tab opens.
#[trial]
fn float_in_a_docked_tabs_menu_shows_the_panels_os_window() {
    let mut host = Host::new();
    let tab = host.at("Panel 1", Role::TAB).expect("the docked tab shows");
    for event in [moved(tab), secondary(tab, true), secondary(tab, false)] {
        host.settle(event);
    }
    let row = host.at("Float", Role::MENU_ITEM).expect("the tab menu's Float row");
    for event in [moved(row), press(row), release(row)] {
        host.settle(event);
    }
    host.assert_floats_in_a_shown_window();
}

/// Float picked with Enter in the menu Shift+F10 opens on the focused tab.
#[trial]
fn float_in_a_tab_menu_opened_by_the_keyboard_shows_the_panels_os_window() {
    let mut host = Host::new();
    host.settle(key_press(Key::Tab, Modifiers::NONE));
    host.settle(key_press(Key::F10, Modifiers::SHIFT));
    let rows = host.tree().with_role(Role::MENU_ITEM).len();
    assert!(rows > 0, "the tab menu opened");
    for _ in 0..rows {
        if name(host.tree(), focused_key(host.tree())) == Some("Float") {
            break;
        }
        host.settle(key_press(Key::ArrowDown, Modifiers::NONE));
    }
    host.settle(key_press(Key::Enter, Modifiers::NONE));
    host.assert_floats_in_a_shown_window();
}
