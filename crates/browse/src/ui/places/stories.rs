//! Stories of `PlacesTree`: groups with an open root, a loading row, an empty group and hidden places. The folders
//! are a `FakeDirectories` whose listings run on a `ManualJobs`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use craft_host::{ManualJobs, Wake};
use craft_ui::contract::{Entry, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{PlacesEvent, PlacesTree};
use crate::places::{FakeDirectories, FolderCache, PlaceGroupKind, Places, PlacesInput, PlacesSettings, StandardFolder, Volume};

fn path(text: &str) -> PathBuf {
    PathBuf::from(text)
}

/// `/home/u` holds Documents, Pictures and Photos; Photos holds 2024 and 2025; 2024 holds Trip and Studio; the card
/// holds DCIM and the archive Old.
fn directories() -> FakeDirectories {
    let mut fake = FakeDirectories::new()
        .standard_folder(StandardFolder::Home, "/home/u")
        .standard_folder(StandardFolder::Documents, "/home/u/Documents")
        .standard_folder(StandardFolder::Pictures, "/home/u/Pictures")
        .standard_folder(StandardFolder::Music, "/home/u/Music")
        .volume(Volume { path: path("/media/card"), name: "Camera Card".to_owned(), network: false, removable: true })
        .volume(Volume { path: path("/mnt/archive"), name: "Archive".to_owned(), network: true, removable: false });
    for folder in [
        "/home/u/Documents",
        "/home/u/Music",
        "/home/u/Photos/2024/Studio",
        "/home/u/Photos/2024/Trip",
        "/home/u/Photos/2025",
        "/home/u/Pictures/Screenshots",
        "/media/card/DCIM",
        "/mnt/archive/Old",
    ] {
        fake = fake.folder(folder);
    }
    fake
}

/// What a story keeps between passes.
struct Demo {
    cache: FolderCache,
    jobs: Arc<ManualJobs>,
    settings: PlacesSettings,
    current: Option<PathBuf>,
    last: Option<String>,
}

/// The demo as story data: compared by identity.
#[derive(Clone)]
struct Shared(Arc<Mutex<Demo>>);

impl PartialEq for Shared {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What a story sets up.
struct Setup {
    groups: &'static [PlaceGroupKind],
    settings: fn() -> PlacesSettings,
    current: Option<&'static str>,
    /// Listings wait until the story's end: the tree shows its loading row.
    slow: bool,
}

fn full_settings() -> PlacesSettings {
    PlacesSettings {
        favorites: vec![path("/home/u/Photos/2024/Studio")],
        hidden: vec![path("/home/u/Music")],
        recent: vec![path("/home/u/Photos/2025"), path("/media/card/DCIM")],
        session_root: None,
    }
}

fn hidden_settings() -> PlacesSettings {
    PlacesSettings { favorites: Vec::new(), hidden: vec![path("/home/u/Music"), path("/home/u/Documents")], recent: Vec::new(), session_root: None }
}

fn empty_settings() -> PlacesSettings {
    PlacesSettings { favorites: Vec::new(), hidden: Vec::new(), recent: Vec::new(), session_root: None }
}

const ALL_GROUPS: &[PlaceGroupKind] =
    &[PlaceGroupKind::Favorites, PlaceGroupKind::Places, PlaceGroupKind::Volumes, PlaceGroupKind::Network, PlaceGroupKind::Recent];

fn show_places(ui: &mut Ui, data: &mut StoryData, setup: Setup) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let demo = data
        .value("demo", {
            let jobs = Arc::new(ManualJobs::new());
            let repaint = ctx.clone();
            let cache = FolderCache::new(Arc::new(directories()), jobs.clone(), Wake::new(move || repaint.request_repaint()));
            Shared(Arc::new(Mutex::new(Demo { cache, jobs, settings: (setup.settings)(), current: setup.current.map(path), last: None })))
        })
        .clone();
    let mut demo = demo.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !setup.slow && demo.jobs.run_all() > 0 {
        ctx.request_repaint();
    }
    let current = demo.current.clone();
    let places = {
        let Demo { cache, settings, .. } = &mut *demo;
        let unlisted = current.as_deref().and_then(|current| cache.unlisted(current)).map(<[PathBuf]>::to_vec).unwrap_or_default();
        let roots = cache.roots().cloned();
        let (standard, volumes) = roots.map_or((Vec::new(), Vec::new()), |roots| (roots.standard, roots.volumes));
        let places = Places::resolve(&PlacesInput {
            standard: &standard,
            volumes: &volumes,
            settings,
            browsing: current.as_deref(),
            unlisted: &unlisted,
            groups: setup.groups,
        });
        settings.session_root = places.session_root.clone();
        places
    };
    ui.horizontal_wrapped(|ui| {
        Text::new("Photo 7").show(ui);
        Text::new("Drag this photo onto a folder under Photos").shade(Shade::Muted).show(ui);
    });
    let mut out = None;
    {
        let Demo { cache, .. } = &mut *demo;
        Stack::new().room_height(380.0).show(ui, |ui, _| {
            let mut tree =
                PlacesTree::new("Places", &places, &mut *cache).row_menu(|_| vec![Entry::new(String::from("folder.rename"), "Rename Folder…")]);
            if let Some(current) = current.as_deref() {
                tree = tree.current(current);
            }
            let shown = tree.show(ui);
            let response = shown.response.clone();
            out = Some(shown);
            response
        });
    }
    let events = out.as_ref().map(|out| out.events.clone()).unwrap_or_default();
    for event in &events {
        demo.settings.apply(event);
        if let PlacesEvent::Open(folder) = event {
            demo.current = Some(folder.clone());
        }
        demo.last = Some(format!("{event:?}"));
    }
    if let Some(current) = demo.current.as_deref() {
        Text::new(format!("Browsing: {}", current.display())).shade(Shade::Muted).show(ui);
    }
    let last = demo.last.as_deref().map_or_else(|| String::from("No event yet"), |event| format!("Last event: {event}"));
    Text::new(last).shade(Shade::Muted).show(ui);
    out.map(StoryOutput::from).unwrap_or_default()
}

#[story(PlacesTree<'static>, covers = [Default, Many], drive = Drive::Never)]
fn places_tree(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_places(ui, data, Setup { groups: ALL_GROUPS, settings: full_settings, current: Some("/home/u/Photos/2024/Trip"), slow: false })
}

#[story(PlacesTree<'static>, covers = [], drive = Drive::Never)]
fn places_tree_loading(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_places(ui, data, Setup { groups: ALL_GROUPS, settings: full_settings, current: Some("/home/u/Photos/2024/Trip"), slow: true })
}

#[story(PlacesTree<'static>, covers = [Empty], drive = Drive::Never)]
fn places_tree_empty_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_places(ui, data, Setup { groups: &[PlaceGroupKind::Places, PlaceGroupKind::Recent], settings: empty_settings, current: None, slow: false })
}

#[story(PlacesTree<'static>, covers = [], drive = Drive::Never)]
fn places_tree_hidden(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_places(ui, data, Setup { groups: &[PlaceGroupKind::Places], settings: hidden_settings, current: None, slow: false })
}
