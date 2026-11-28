//! The viewer's window: the shell with the options bar over the dock of the shown workspace, and the settings it keeps.

use super::card::Look;
use super::dock::GalleryDock;
use super::inspect::Inspect;
use super::options::options_bar;
use super::panels::ThemePanels;
use super::persist::Persisted;
use super::variants::{VariantsEvent, VariantsIn};
use crate::viewer::{Catalog, GalleryCommands, GallerySettings, PanelId, Viewer, Workspace, theme_of};
use craft_behavior::dock::{PanelState, Workspace as Dock};
use craft_theme::AppSizes;
use craft_theme_editor::{ThemeSubject, parse_sizes};
use craft_ui::contract::{Bind, BodyCx, CommandSet as _, Tracked};
use craft_ui::docking::DockConfig;
use craft_ui::patterns::dock::DockArea;
use craft_ui::popover::DropVeil;
use craft_ui::runtime::commands::{CommandInput, CommandStates};
use craft_ui::runtime::drag::OsFile;
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::{FrameInput, FrameOutput};
use craft_ui::runtime::toast::{self, Toast};
use craft_ui::shell::AppShell;
use craft_ui::shell::app::ShellApp;
use craft_ui::shell::embed::CloseDecision;
use egui::Ui;
use std::collections::HashSet;
use std::sync::Arc;
use web_time::Instant;

/// The viewer as the root window's app.
pub struct Gallery {
    viewer: Viewer,
    persisted: Persisted,
    /// What came of the files last dropped on the theme workspace.
    drop_note: Option<String>,
    /// The notes already shown as toasts.
    noted: HashSet<String>,
    /// A command changed the viewer since the settings were last compared.
    command_ran: bool,
    /// The app-scope command states of the front component's stories, and that component: rebuilt only when another
    /// component comes to the front, so an idle frame keeps one revision.
    command_states: (Option<&'static str>, Tracked<CommandStates>),
}

impl Gallery {
    /// The viewer of the stories linked into the binary, in the state the gallery's settings hold.
    pub fn open() -> Self {
        Gallery::new(Catalog::linked(), Persisted::open())
    }

    /// The viewer of `catalog` over `persisted`.
    pub fn new(catalog: Catalog, persisted: Persisted) -> Self {
        let viewer = Viewer::new(catalog, persisted.get());
        Gallery { viewer, persisted, drop_note: None, noted: HashSet::new(), command_ran: false, command_states: (None, Tracked::default()) }
    }

    /// Shows what the settings and a dropped file have to say, once each, as toasts.
    fn announce(&mut self, ctx: &egui::Context) {
        let notes = self.persisted.notes().into_iter().chain(self.drop_note.take());
        for note in notes {
            if self.noted.insert(note.clone()) {
                toast::post(ctx, &WindowKey::main(), Toast::new(note));
            }
        }
    }
}

/// Whether an input event can have changed what a control edits: hover, focus and pointer motion alone cannot, the
/// release or key that ends an edit can.
fn is_edit(event: &egui::Event) -> bool {
    !matches!(event, egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_) | egui::Event::PointerGone | egui::Event::WindowFocused(_))
}

/// The app layer a dropped `sizes.json` holds.
fn read_dropped(file: &OsFile) -> Result<AppSizes, String> {
    let bytes = match (&file.bytes, &file.path) {
        (Some(bytes), _) => bytes.to_vec(),
        (None, Some(path)) => std::fs::read(path).map_err(|error| format!("{}: {error}", file.name))?,
        (None, None) => return Err(format!("{}: the file has no content", file.name)),
    };
    parse_sizes(&file.name, &bytes)
}

/// The registered name of the component whose tab is in front and selected in the shown workspace's dock.
fn front_component(viewer: &Viewer) -> Option<&'static str> {
    let dock = if viewer.workspace == Workspace::Theme { &viewer.theme_dock } else { &viewer.dock };
    viewer.browser.selected_type().filter(|type_name| dock.layout().panel_state(&PanelId::component(type_name)) == PanelState::Front)
}

/// Gives an open Variants view its centre tab: the front centre tab of the components workspace when the view opens,
/// and the view closes when its tab is no longer in front. The dock layout says which tab is in front, before any body
/// runs.
fn place_variants(viewer: &mut Viewer) {
    let front =
        |panel: &PanelId| matches!(panel, PanelId::Overview | PanelId::Component(_)) && viewer.dock.layout().panel_state(panel) == PanelState::Front;
    let owner = match (&viewer.variants, &viewer.variants_tab) {
        (None, _) => None,
        (Some(_), Some(tab)) if front(tab) => Some(tab.clone()),
        (Some(_), Some(_)) => None,
        (Some(_), None) => viewer.dock.layout().panels().into_iter().find(|panel| front(panel)).cloned(),
    };
    if owner.is_none() {
        viewer.variants = None;
    }
    viewer.variants_tab = owner;
}

/// The dock of the shown workspace over the viewer's state.
fn body(ui: &mut Ui, _: &mut BodyCx<'_>, viewer: &mut Viewer) {
    place_variants(viewer);
    let selected = front_component(viewer).map(PanelId::component);
    let Viewer { env, workspace, preview, browser, dock, themes, theme_dock, specs, mock, variants, variants_tab, inspect_layers, .. } = viewer;
    let appearance = env.appearance(ui.ctx());
    let subject = ThemeSubject::new(theme_of(env.theme_id()), appearance);
    let in_theme = *workspace == Workspace::Theme;
    let dock: &mut Arc<Dock<_>> = if in_theme { theme_dock } else { dock };
    let front = |panel: &PanelId| dock.layout().panel_state(panel) == PanelState::Front;
    let inspect = Inspect {
        on: !in_theme && variants.is_none() && front(&PanelId::Inspect),
        front: [Some(PanelId::Overview), selected].into_iter().flatten().filter(|panel| front(panel)).collect(),
    };
    let window = Look { mode: Some(appearance.mode), direction: Some(env.direction), set: Some(env.component_set()), ..Look::default() };
    let variants_in = (*variants).zip(variants_tab.clone()).map(|(axis, tab)| (tab, VariantsIn::new(axis, window)));
    let mut content = GalleryDock {
        browser,
        inspect,
        layers: inspect_layers,
        variants: variants_in,
        theme: ThemePanels { themes, subject, revealed: None },
        preview: *preview,
        app: mock,
        opened: Vec::new(),
        variants_out: Vec::new(),
    };
    // Component tabs are documents: they close from their ×.
    let config = DockConfig::default().tab_close_buttons(true);
    DockArea::new(if in_theme { "Theme dock" } else { "Components dock" }, Bind::bound(&mut *dock), specs, &mut content, config).show(ui);
    let GalleryDock { opened, variants_out, .. } = content;
    for type_name in opened {
        let _ = Arc::make_mut(dock).set_visible(PanelId::component(type_name), true, Some(PanelId::Overview), false, |panel| panel.permissions());
    }
    for event in variants_out {
        if let VariantsEvent::Use(look) = event {
            if let Some(mode) = look.mode {
                env.mode = mode.into();
            }
            if let Some(direction) = look.direction {
                env.direction = direction;
            }
            if let Some(set) = look.set {
                env.values.set(set);
            }
        }
        *variants = None;
    }
    // A closed component tab does not come back from the layout's record of closed panels.
    let component = |panel: &PanelId| matches!(panel, PanelId::Component(_));
    if dock.hidden().iter().any(|(panel, _)| component(panel)) {
        Arc::make_mut(dock).forget_hidden(component);
    }
}

impl ShellApp for Gallery {
    fn frame_input(&mut self) -> FrameInput {
        // App-scope command states are one input per frame, so only the stories of the component shown reach it; the
        // Overview, the Theme dock and a frame with no such tab give none.
        let browser = &self.viewer.browser;
        let front = front_component(&self.viewer);
        if front != self.command_states.0 {
            let stories = front
                .into_iter()
                .flat_map(|type_name| browser.catalog.stories_of(type_name).iter().map(|&at| &browser.catalog.entries[at].registration.story));
            self.command_states = (front, Tracked::new(craft_ui::testing::Story::command_states(stories)));
        }
        FrameInput::new(self.viewer.env.clone()).commands(CommandInput::new().states(&self.command_states.1))
    }

    fn output(&mut self, output: FrameOutput) {
        for fired in output.fired {
            if let Some(command) = GalleryCommands::from_ref(&fired.command) {
                self.command_ran = true;
                self.viewer.run(command);
            }
        }
    }

    fn tick(&mut self, now: Instant) -> Option<Instant> {
        let (deadline, merged) = self.persisted.tick(now);
        if let Some(settings) = merged {
            self.viewer.restore(&settings);
        }
        deadline
    }

    fn close_requested(&mut self) -> CloseDecision {
        self.persisted.save();
        CloseDecision::Close
    }

    fn ui(&mut self, ui: &mut Ui) {
        let in_theme = self.viewer.workspace == Workspace::Theme;
        let shell = |ui: &mut Ui, viewer: &mut Viewer| {
            AppShell::new(WindowKey::main())
                .options_bar(true, |_| None, |ui, _, viewer: &mut Viewer| options_bar(ui, viewer))
                .body_label("Gallery")
                .body(body)
                .show(ui, viewer);
        };
        let dropped = if in_theme {
            let viewer = &mut self.viewer;
            DropVeil::new("Drop an app's sizes.json to load it").show_with_files(ui, |ui, _| shell(ui, viewer)).dropped.clone()
        } else {
            shell(ui, &mut self.viewer);
            None
        };
        if let Some(file) = dropped.as_ref().and_then(|dropped| dropped.files.first()) {
            self.drop_note = Some(match read_dropped(file) {
                Ok(sizes) => {
                    self.viewer.themes.edit_sizes(|layer| {
                        *layer = sizes;
                        true
                    });
                    format!("Loaded {}", file.name)
                }
                Err(message) => message,
            });
        }
        self.viewer.sync_sizes();
        self.announce(ui.ctx());
        // The controls change the viewer only on input, and a command only in `output`: a pass with neither has
        // nothing new to save.
        let edited = std::mem::take(&mut self.command_ran) || ui.ctx().input(|input| input.events.iter().any(is_edit));
        if edited && self.persisted.set(GallerySettings::of(&self.viewer)) {
            // The pass after a change ticks the store, which names when the save is due.
            ui.ctx().request_repaint();
        }
    }
}
