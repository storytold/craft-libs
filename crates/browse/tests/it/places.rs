//! `PlacesTree` with `FakeDirectories` and `ManualJobs`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::keyed;
use craft_browse::places::{FakeDirectories, FolderCache, Place, PlaceGroup, PlaceGroupKind, PlaceKind, Places, StandardFolder};
use craft_browse::ui::places::{PlacesEvent, PlacesTree};
use craft_host::{ManualJobs, Wake};
use craft_ui::contract::{ListDrag, Role, ShowCollection as _};
use craft_ui::list::{List, ListRow};
use craft_ui::testing::access::name;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

fn path(s: &str) -> PathBuf {
    PathBuf::from(s)
}

/// `/home/u` holds Photos and Docs; Photos holds 2024 and 2025; 2024 holds Trip.
fn fake() -> FakeDirectories {
    FakeDirectories::new()
        .standard_folder(StandardFolder::Home, "/home/u")
        .folder("/home/u/Docs")
        .folder("/home/u/Photos/2025")
        .folder("/home/u/Photos/2024/Trip")
}

fn home() -> Places {
    let place = Place { path: path("/home/u"), name: "u".into(), kind: PlaceKind::Standard(StandardFolder::Home) };
    Places { groups: vec![PlaceGroup { kind: PlaceGroupKind::Places, places: vec![place] }], owner: Some((0, 0)), ..Places::default() }
}

struct Rig {
    places: Places,
    cache: FolderCache,
    jobs: Arc<ManualJobs>,
    current: Option<PathBuf>,
}

fn rig(current: Option<&str>, places: Places) -> (Fixture, Rc<RefCell<Rig>>) {
    let jobs = Arc::new(ManualJobs::new());
    let fake = Arc::new(fake());
    let cache = FolderCache::new(fake.clone(), jobs.clone(), Wake::new(|| {}));
    let state = Rc::new(RefCell::new(Rig { places, cache, jobs, current: current.map(path) }));
    let shown = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut rig = shown.borrow_mut();
        let Rig { places, cache, current, .. } = &mut *rig;
        let mut tree = PlacesTree::new("Places", places, cache);
        if let Some(current) = current.as_deref() {
            tree = tree.current(current);
        }
        let _ = tree.show(ui);
    });
    settle(&mut fixture, &state);
    (fixture, state)
}

/// Runs frames, finishing the listing jobs between them, until nothing is waiting.
fn settle(fixture: &mut Fixture, state: &Rc<RefCell<Rig>>) {
    for _ in 0..6 {
        fixture.idle(2);
        state.borrow().jobs.run_all();
    }
    fixture.idle(2);
}

fn rows(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::TREE_ITEM).iter().filter_map(|row| name(fixture.tree(), row.node).map(str::to_owned)).collect()
}

fn click_row(fixture: &mut Fixture, label: &str) {
    let row = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown: {:?}", rows(fixture)));
    fixture.click(&row);
}

/// `current` deep in Home: the rows open once on the way down; collapsing one keeps it closed on the next passes.
#[trial]
fn the_tree_reveals_the_browsed_folder_once_and_collapse_sticks() {
    let (mut fixture, state) = rig(Some("/home/u/Photos/2024/Trip"), home());
    let shown = rows(&fixture);
    for name in ["Home", "Docs", "Photos", "2024", "Trip", "2025"] {
        assert!(shown.iter().any(|row| row == name), "{name} is shown: {shown:?}");
    }

    click_row(&mut fixture, "Photos");
    fixture.key(Key::ArrowLeft, Modifiers::NONE);
    settle(&mut fixture, &state);
    assert!(!rows(&fixture).iter().any(|row| row == "Trip"), "Photos is closed");
    settle(&mut fixture, &state);
    assert!(!rows(&fixture).iter().any(|row| row == "2024"), "the reveal does not open it again");
}

/// The disclosure triangle of a folder opens it and closes it again.
#[trial]
fn the_disclosure_opens_and_closes_a_folder() {
    let (mut fixture, state) = rig(None, home());
    let shown = |fixture: &Fixture, name: &str| rows(fixture).iter().any(|row| row == name);
    if shown(&fixture, "Docs") {
        let collapse = fixture.by_label("Collapse Home").expect("an open folder's triangle collapses it");
        fixture.click(&collapse);
        settle(&mut fixture, &state);
    }
    assert!(!shown(&fixture, "Docs"), "Home starts closed");
    let expand = fixture.by_label("Expand Home").expect("a closed folder's triangle expands it");
    fixture.click(&expand);
    settle(&mut fixture, &state);
    assert!(shown(&fixture, "Docs") && shown(&fixture, "Photos"), "the triangle opens Home: {:?}", rows(&fixture));
    let collapse = fixture.by_label("Collapse Home").expect("the open folder's triangle collapses it");
    fixture.click(&collapse);
    settle(&mut fixture, &state);
    assert!(!shown(&fixture, "Docs"), "the triangle closes Home again: {:?}", rows(&fixture));
}

/// A row dragged from another list drops onto a folder row the host accepts, and only there.
#[trial]
fn a_dragged_row_drops_onto_a_folder_the_host_accepts() {
    let jobs = Arc::new(ManualJobs::new());
    let cache = FolderCache::new(Arc::new(fake()), jobs.clone(), Wake::new(|| {}));
    let state = Rc::new(RefCell::new(Rig { places: home(), cache, jobs, current: Some(path("/home/u/Photos/2024/Trip")) }));
    let dropped: Rc<RefCell<Vec<(PathBuf, Option<Vec<u32>>)>>> = Rc::default();
    let (shown, seen) = (state.clone(), dropped.clone());
    let mut fixture = Fixture::new(move |ui| {
        let photos = [7u32];
        let _ = List::<u32>::new("Photos").natural_height(true).drag_payload(|drag: &ListDrag<u32>| Some(drag.clone())).show_items(
            ui,
            &keyed(&photos, |n: &u32| *n),
            &mut |_, _| ListRow::new("Photo 7"),
        );
        let mut rig = shown.borrow_mut();
        let Rig { places, cache, current, .. } = &mut *rig;
        let mut tree = PlacesTree::new("Places", places, cache).accepts(|folder, _: &ListDrag<u32>| folder.starts_with("/home/u/Photos"));
        if let Some(current) = current.as_deref() {
            tree = tree.current(current);
        }
        for event in tree.show(ui).events {
            if let PlacesEvent::Dropped { place, payload, .. } = event {
                seen.borrow_mut().push((place, payload.get::<ListDrag<u32>>().map(|drag| drag.keys.clone())));
            }
        }
    });
    settle(&mut fixture, &state);
    let photo = fixture.by_label("Photo 7").expect("the photo row is shown").center();
    for (folder, place) in [("Docs", None), ("2024", Some("/home/u/Photos/2024"))] {
        let row = fixture.by_label(folder).unwrap_or_else(|| panic!("{folder} is shown: {:?}", rows(&fixture))).center();
        fixture.drag_at(photo, row - photo);
        assert_eq!(
            dropped.borrow_mut().drain(..).collect::<Vec<_>>(),
            place.map(|place| (path(place), Some(vec![7]))).into_iter().collect::<Vec<_>>()
        );
    }
}
