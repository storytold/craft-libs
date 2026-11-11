//! The viewer's window: the shell with the options bar over the dock of the shown workspace, and the settings it keeps.

use super::dock::GalleryDock;
use super::options::options_bar;
use super::panels::Panels;
use super::persist::Persisted;
use crate::theme::{ThemeSubject, parse_sizes};
use crate::viewer::{Browser, Catalog, GalleryCommands, GallerySettings, Viewer, Workspace, theme_of};
use craft_behavior::dock::Workspace as Dock;
use craft_theme::AppSizes;
use craft_ui::contract::{Bind, BodyCx, CommandSet as _, ShowContainer as _};
use craft_ui::docking::DockConfig;
use craft_ui::patterns::dock::DockArea;
use craft_ui::popover::DropVeil;
use craft_ui::runtime::drag::OsFile;
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::{FrameInput, FrameOutput};
use craft_ui::runtime::toast::{self, Toast};
use craft_ui::shell::AppShell;
use craft_ui::shell::app::{CloseDecision, ShellApp};
use egui::Ui;
use std::collections::HashSet;
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
}

impl Gallery {
    /// The viewer of the stories linked into the binary, in the state the gallery's settings hold.
    pub fn open() -> Self {
        Gallery::new(Catalog::linked(), Persisted::open())
    }

    /// The viewer of `catalog` over `persisted`.
    pub fn new(catalog: Catalog, persisted: Persisted) -> Self {
        let viewer = Viewer::new(catalog, persisted.get());
        Gallery { viewer, persisted, drop_note: None, noted: HashSet::new(), command_ran: false }
    }

    /// Shows what the settings and a dropped file have to say, once each, as toasts.
    fn announce(&mut self, ctx: &egui::Context) {
        let notes = self.persisted.notes().into_iter().chain(self.drop_note.take());
        for note in notes {
            if self.noted.insert(note.clone()) {
                toast::post(ctx, Toast::new(note));
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

/// The dock of the shown workspace over the viewer's state.
fn body(ui: &mut Ui, _: &mut BodyCx<'_>, viewer: &mut Viewer) {
    let Viewer { env, workspace, preview, browser, themes, theme_dock, mock, variants, variants_tab, .. } = viewer;
    let subject = ThemeSubject::new(theme_of(&env.theme), env.appearance(ui.ctx()));
    let in_theme = *workspace == Workspace::Theme;
    let Browser { catalog, filter, selection, open, dock, story_of, generation, cells, open_request, scroll_to, followed, tree_seen, level, .. } =
        browser;
    let dock: &mut Dock<_> = if in_theme { theme_dock } else { dock };
    let panels = Panels {
        catalog,
        filter,
        selection,
        open,
        story_of,
        cells,
        open_request,
        scroll_to,
        followed,
        tree_seen,
        generation: *generation,
        themes,
        subject,
        level,
        variants,
        variants_tab,
        env,
        revealed: None,
    };
    let mut content = GalleryDock { panels, preview: *preview, app: mock };
    // Component tabs are documents: they close from their ×.
    let config = DockConfig::default().tab_close_buttons(true);
    DockArea::new(if in_theme { "Theme dock" } else { "Components dock" }, Bind::bound(&mut *dock), &mut content, config).show(ui);
    if let Some(type_name) = content.panels.open_request.take() {
        let _ = dock.set_visible(crate::viewer::PanelId::Component(type_name), true, Some(crate::viewer::PanelId::Overview), false, |panel| {
            panel.permissions()
        });
    }
    // A closed component tab does not come back from the layout's record of closed panels.
    dock.hidden.retain(|(panel, _)| !matches!(panel, crate::viewer::PanelId::Component(_)));
}

impl ShellApp for Gallery {
    fn frame_input(&mut self) -> FrameInput {
        FrameInput::new(self.viewer.env.clone())
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
                .options_bar(true, |ui, _, viewer: &mut Viewer| options_bar(ui, viewer))
                .body_label("Gallery")
                .body(body)
                .show(ui, viewer);
        };
        let dropped = if in_theme {
            let viewer = &mut self.viewer;
            DropVeil::new("Drop an app's sizes.json to load it").show(ui, |ui, _| shell(ui, viewer)).dropped.clone()
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
