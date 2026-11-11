#![cfg(not(target_arch = "wasm32"))]

use craft_ui::testing::trial;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use craft_theme::{ContrastChoice, ModeChoice, MotionChoice, Selection, Size, TextSize, ThemeId, ThemePair};

use super::SuiteWatch;
use crate::{Accepted, SelectionChange, Store, StoreOptions, SuiteDir};

fn options() -> StoreOptions {
    StoreOptions::new("test 1.0").lock_timeout(Duration::from_millis(100))
}

fn id(s: &str) -> ThemeId {
    ThemeId::parse(s).unwrap()
}

/// `theme` in both slots, with the defaults for every field but the mode.
fn sel(theme: &str, mode: ModeChoice) -> Selection {
    Selection {
        theme: ThemePair::same(id(theme)),
        mode,
        contrast: ContrastChoice::System,
        motion: MotionChoice::System,
        size: Size::Compact,
        text_size: TextSize::Small,
    }
}

fn write_selection(dir: &Path, base: &Accepted, change: SelectionChange) -> Accepted {
    let mut store = Store::open(&SuiteDir::at(dir), options());
    store.load_selection().unwrap();
    store.update(base, change).unwrap()
}

/// A wake callback and how often it ran.
fn counter() -> (Arc<AtomicUsize>, Box<dyn Fn() + Send + Sync>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = Arc::clone(&wakes);
    (wakes, Box::new(move || _ = w.fetch_add(1, Ordering::SeqCst)))
}

/// Waits for `done`. The limit only ends a hung test: the first use of the built-in themes parses
/// all of them (seconds in a debug build on a loaded machine) and the worker would spend the limit
/// on that, so it is done before the clock starts.
fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let _ = craft_theme::builtins();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[trial]
fn a_same_metadata_replacement_is_found_by_the_byte_comparison() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("selection.json");
    write_selection(tmp.path(), &Accepted::default(), SelectionChange::whole(sel("craft.graphite", ModeChoice::Dark)));
    let (_, wake) = counter();
    let watch = SuiteWatch::inline(Store::open(&SuiteDir::at(tmp.path()), options()), wake);
    assert_eq!(watch.generation(), 0);

    // Same length, same file, modification time put back: only the bytes differ.
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let replaced = text.replace("craft.graphite", "craft.artcraft");
    assert_eq!(replaced.len(), text.len());
    fs::write(&path, replaced).unwrap();
    fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();

    watch.poll_once(false);
    assert_eq!(watch.generation(), 0, "the stamp check can't see this change");
    watch.poll_once(true);
    assert_eq!(watch.generation(), 1);
    assert_eq!(watch.snapshot().accepted.suite, sel("craft.artcraft", ModeChoice::Dark).into());
}

/// selection.json with a chosen theme and mode, written three times: revision 3.
fn chosen_on_disk(dir: &Path) -> Accepted {
    let mut store = Store::open(&SuiteDir::at(dir), options());
    store.update(&Accepted::default(), SelectionChange::whole(sel("craft.artcraft", ModeChoice::System))).unwrap();
    let base = store.accepted().clone();
    store.update(&base, SelectionChange::themes(ThemePair::same(id("craft.studio")))).unwrap();
    let base = store.accepted().clone();
    store.update(&base, SelectionChange::whole(sel("craft.graphite", ModeChoice::Dark))).unwrap()
}

fn settle(client: &mut crate::SuiteClient) {
    wait_for("the change", || {
        client.sync();
        !client.is_pending()
    });
}

#[trial]
fn a_pick_in_the_first_frame_merges_against_the_disk_state() {
    for change in [SelectionChange::mode(ModeChoice::Light), SelectionChange::themes(ThemePair::same(id("craft.artcraft")))] {
        let tmp = tempfile::tempdir().unwrap();
        let disk = chosen_on_disk(tmp.path());
        assert_eq!(disk.revision, 3);
        // No worker, no poll: generation 0 is all the client has.
        let (_, watch) = SuiteWatch::build(Store::open(&SuiteDir::at(tmp.path()), options()), Box::new(|| {}));
        let mut client = crate::SuiteClient::new(watch);
        assert_eq!((client.snapshot().generation, client.accepted()), (0, &disk));
        client.change(change.clone());
        settle(&mut client);
        assert!(client.conflict().is_none() && client.failure().is_none(), "{:?}", client.conflict());
        assert_eq!(client.accepted().revision, 4);
        assert_eq!(client.accepted().suite, change.applied_to(&disk.suite));
    }

    // The same with the worker running.
    let tmp = tempfile::tempdir().unwrap();
    chosen_on_disk(tmp.path());
    let mut client = crate::SuiteClient::new(SuiteWatch::open(&SuiteDir::at(tmp.path()), options(), || {}));
    client.change(SelectionChange::mode(ModeChoice::Light));
    settle(&mut client);
    assert!(client.conflict().is_none());
    assert_eq!(client.accepted().suite, sel("craft.graphite", ModeChoice::Light).into());
}
