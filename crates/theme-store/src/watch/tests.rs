#![cfg(not(target_arch = "wasm32"))]

use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use craft_theme::{Mode, Selection, ThemeId};

use super::{Intervals, Op, OpOutput, SuiteWatch};
use crate::{
    Accepted, ParseCounts, Problem, SelectionChange, SelectionError, Store, StoreError, StoreOptions, SuiteDir,
    testing::{FailAt, FailingFs, SAMPLE_THEME_ID, SAMPLE_THEME_JSON},
};

const FAST: Intervals = Intervals { stat: Duration::from_millis(5), verify: Duration::from_millis(20) };

fn options() -> StoreOptions {
    StoreOptions::new("test 1.0").lock_timeout(Duration::from_millis(100))
}

fn sel(theme: &str, mode: Mode) -> Selection {
    Selection { theme: ThemeId::parse(theme).unwrap(), mode }
}

fn write_selection(dir: &Path, base: &Accepted, change: SelectionChange) -> Accepted {
    let mut store = Store::open(&SuiteDir::at(dir), options());
    store.load_selection().unwrap();
    store.update(base, change).unwrap()
}

fn theme_json(id: &str) -> String {
    format!(r##"{{"schema":"craft-theme","schema_version":1,"id":"{id}","name":"{id}","dark":{{"text":"#ffffff","background":"#000000"}}}}"##)
}

/// A wake callback and how often it ran.
fn counter() -> (Arc<AtomicUsize>, Box<dyn Fn() + Send + Sync>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = Arc::clone(&wakes);
    (wakes, Box::new(move || _ = w.fetch_add(1, Ordering::SeqCst)))
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn result(watch: &SuiteWatch, id: u64) -> Result<OpOutput, StoreError> {
    wait_for("the result", || watch.snapshot().result(id).is_some());
    watch.snapshot().result(id).unwrap().clone()
}

#[test]
fn unchanged_files_are_parsed_once_and_never_republished() {
    let tmp = tempfile::tempdir().unwrap();
    let library = tmp.path().join("library");
    fs::create_dir_all(&library).unwrap();
    for n in 0..10_000 {
        let id = format!("t{n:05}");
        fs::write(library.join(format!("{id}.json")), theme_json(&id)).unwrap();
    }
    write_selection(tmp.path(), &Accepted::default(), SelectionChange::whole(sel("t04242", Mode::Dark)));

    let (wakes, wake) = counter();
    let watch = SuiteWatch::inline(Store::open(&SuiteDir::at(tmp.path()), options()), wake);
    let first = watch.snapshot();
    assert_eq!((first.generation, first.index.len()), (1, 10_000));
    assert_eq!(first.active.as_ref().map(|r| r.theme.id.as_str()), Some("t04242"));
    assert!(first.problems.is_empty());
    let loaded = ParseCounts { selection: 1, themes: 10_000 };
    assert_eq!(watch.parse_counts(), loaded);
    // Without a worker the caller is in its frame already: no wake.
    assert_eq!(wakes.load(Ordering::SeqCst), 0);

    // 100 timer ticks, every 20th with the byte comparison (the 500 ms / 10 s ratio).
    for tick in 1..=100 {
        watch.poll_once(tick % 20 == 0);
    }
    assert_eq!(watch.parse_counts(), loaded);
    assert_eq!(watch.generation(), 1);
    assert_eq!(wakes.load(Ordering::SeqCst), 0);

    // A picker opens after a file was added by hand: only the new file is read.
    fs::write(library.join("t10000.json"), theme_json("t10000")).unwrap();
    watch.request(Op::Rescan);
    assert_eq!(watch.parse_counts(), ParseCounts { selection: 1, themes: 10_001 });
    assert_eq!(watch.snapshot().index.len(), 10_001);
}

#[test]
fn an_external_write_bumps_the_generation_and_wakes_once() {
    let tmp = tempfile::tempdir().unwrap();
    let first = write_selection(tmp.path(), &Accepted::default(), SelectionChange::whole(sel("craft.graphite", Mode::Dark)));
    let (wakes, wake) = counter();
    let watch = SuiteWatch::spawn(Store::open(&SuiteDir::at(tmp.path()), options()), wake, FAST);
    wait_for("the first load", || watch.generation() == 1);
    // Several stamp checks and byte comparisons with nothing changed.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!((watch.generation(), wakes.load(Ordering::SeqCst)), (1, 1));

    write_selection(tmp.path(), &first, SelectionChange::mode(Mode::Light));
    wait_for("the change", || watch.generation() == 2);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!((watch.generation(), wakes.load(Ordering::SeqCst)), (2, 2));
    let snapshot = watch.snapshot();
    assert_eq!(snapshot.generation, 2);
    assert_eq!(snapshot.accepted.suite, sel("craft.graphite", Mode::Light).into());
}

#[test]
fn a_same_metadata_replacement_is_found_by_the_byte_comparison() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("selection.json");
    write_selection(tmp.path(), &Accepted::default(), SelectionChange::whole(sel("craft.graphite", Mode::Dark)));
    let (_, wake) = counter();
    let watch = SuiteWatch::inline(Store::open(&SuiteDir::at(tmp.path()), options()), wake);
    assert_eq!(watch.generation(), 1);

    // Same length, same file, modification time put back: only the bytes differ.
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let replaced = text.replace("craft.graphite", "craft.artcraft");
    assert_eq!(replaced.len(), text.len());
    fs::write(&path, replaced).unwrap();
    fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();

    watch.poll_once(false);
    assert_eq!(watch.generation(), 1, "the stamp check can't see this change");
    watch.poll_once(true);
    assert_eq!(watch.generation(), 2);
    assert_eq!(watch.snapshot().accepted.suite, sel("craft.artcraft", Mode::Dark).into());
}

#[test]
fn damaged_files_are_problems_and_keep_the_last_good_state() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&SuiteDir::at(tmp.path()), options());
    store.import(theme_json("one").as_bytes(), false).unwrap();
    let base = store.accepted().clone();
    store.update(&base, SelectionChange::whole(sel("one", Mode::Dark))).unwrap();
    let (_, wake) = counter();
    let watch = SuiteWatch::inline(Store::open(&SuiteDir::at(tmp.path()), options()), wake);
    let good = watch.snapshot();
    assert_eq!(good.active.as_ref().map(|r| r.theme.id.as_str()), Some("one"));

    // The active theme's file deleted by hand.
    fs::remove_file(tmp.path().join("library/one.json")).unwrap();
    watch.poll_once(false);
    let missing = watch.snapshot();
    assert_eq!(missing.generation, good.generation + 1);
    assert!(matches!(&missing.problems[..], [Problem::Theme { stem, error: StoreError::NotFound(_) }] if stem == "one"), "{:?}", missing.problems);
    assert_eq!(missing.accepted, good.accepted);
    assert_eq!(missing.active.as_ref().map(|r| r.theme.id.as_str()), Some("one"));
    assert!(missing.theme(&ThemeId::parse("one").unwrap()).is_some());

    // A damaged selection.json on top.
    fs::write(tmp.path().join("selection.json"), "{ damaged").unwrap();
    watch.poll_once(false);
    let damaged = watch.snapshot();
    assert_eq!(damaged.accepted, good.accepted);
    assert!(damaged.problems.iter().any(|p| matches!(p, Problem::Selection(SelectionError::Corrupt(_)))));
    // The same problems on the next ticks are not news.
    watch.poll_once(true);
    assert_eq!(watch.generation(), damaged.generation);
}

#[test]
fn operations_run_on_the_worker_and_report_their_results() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, wake) = counter();
    let watch = SuiteWatch::spawn(Store::open(&SuiteDir::at(tmp.path()), options()), wake, FAST);

    let base = watch.snapshot().accepted.clone();
    let id = watch.request(Op::Update { base, change: SelectionChange::whole(sel("craft.graphite", Mode::Dark)) });
    assert!(matches!(result(&watch, id), Ok(OpOutput::Done)));
    let mine = watch.snapshot().accepted.clone();
    assert_eq!(mine.suite, sel("craft.graphite", Mode::Dark).into());

    // Another app changes the mode; this app's mode change made on the old state conflicts.
    let theirs = write_selection(tmp.path(), &mine, SelectionChange::mode(Mode::Light));
    let change = SelectionChange::mode(Mode::System);
    let id = watch.request(Op::Update { base: mine.clone(), change });
    match result(&watch, id) {
        Err(StoreError::Conflict { disk, .. }) => assert_eq!(disk, theirs),
        other => panic!("{other:?}"),
    }
    assert_eq!(watch.snapshot().accepted, theirs);

    let id = watch.request(Op::Import { bytes: Arc::from(SAMPLE_THEME_JSON.as_bytes()), as_copy: false });
    assert!(matches!(result(&watch, id), Ok(OpOutput::Imported(t)) if t.as_str() == SAMPLE_THEME_ID));
    wait_for("the index", || watch.snapshot().index.iter().any(|e| e.id.as_str() == SAMPLE_THEME_ID));
    let id = watch.request(Op::Export(ThemeId::parse(SAMPLE_THEME_ID).unwrap()));
    assert!(matches!(result(&watch, id), Ok(OpOutput::Exported(bytes)) if &*bytes == SAMPLE_THEME_JSON.as_bytes()));
    watch.request(Op::AppTheme(Some(ThemeId::parse(SAMPLE_THEME_ID).unwrap())));
    wait_for("the app theme", || watch.snapshot().app_theme.is_some());
    assert_eq!(watch.snapshot().app_theme.as_ref().map(|r| r.theme.id.clone()), Some(ThemeId::parse(SAMPLE_THEME_ID).unwrap()));
}

#[test]
fn a_failed_write_leaves_the_accepted_state() {
    let tmp = tempfile::tempdir().unwrap();
    let before = write_selection(tmp.path(), &Accepted::default(), SelectionChange::whole(sel("craft.graphite", Mode::Dark)));
    let failing = options().file_system(Box::new(FailingFs(FailAt::Rename)));
    let watch = SuiteWatch::open(&SuiteDir::at(tmp.path()), failing, || {});
    let id = watch.request(Op::Update { base: before.clone(), change: SelectionChange::whole(sel("craft.artcraft", Mode::Light)) });
    assert!(matches!(result(&watch, id), Err(StoreError::Io(_))));
    assert_eq!(watch.snapshot().accepted, before);
}

#[test]
fn the_memory_watch_applies_operations_at_once() {
    let watch = SuiteWatch::memory();
    assert_eq!(watch.generation(), 0);
    let id = watch.request(Op::Import { bytes: Arc::from(SAMPLE_THEME_JSON.as_bytes()), as_copy: false });
    let snapshot = watch.snapshot();
    assert_eq!(snapshot.generation, 1);
    assert!(matches!(snapshot.result(id), Some(Ok(OpOutput::Imported(_)))));
    assert_eq!(snapshot.index.len(), 1);
    let sample = sel(SAMPLE_THEME_ID, Mode::Light);
    let id = watch.request(Op::Update { base: snapshot.accepted.clone(), change: SelectionChange::whole(sample.clone()) });
    let snapshot = watch.snapshot();
    assert!(matches!(snapshot.result(id), Some(Ok(OpOutput::Done))));
    assert_eq!(snapshot.accepted.suite, sample.clone().into());
    assert!(snapshot.theme(&sample.theme).is_some());
    let id = watch.request(Op::Delete(sample.theme));
    assert!(matches!(watch.snapshot().result(id), Some(Err(StoreError::InUse(_)))));
}

#[cfg(unix)]
#[test]
fn an_unreadable_theme_in_use_is_read_again_until_it_reads() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("library/one.json");
    let mut store = Store::open(&SuiteDir::at(tmp.path()), options());
    store.import(theme_json("one").as_bytes(), false).unwrap();
    let base = store.accepted().clone();
    store.update(&base, SelectionChange::whole(sel("one", Mode::Dark))).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&file).is_ok() {
        // Running as root: permissions don't stop reads.
        return;
    }
    let (_, wake) = counter();
    let watch = SuiteWatch::inline(Store::open(&SuiteDir::at(tmp.path()), options()), wake);
    let unreadable = watch.snapshot();
    assert!(unreadable.active.is_none());
    assert!(matches!(&unreadable.problems[..], [Problem::Theme { error: StoreError::Io(_), .. }]), "{:?}", unreadable.problems);

    // Same stamp, readable now: the next tick reads it.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    watch.poll_once(false);
    let readable = watch.snapshot();
    assert_eq!(readable.active.as_ref().map(|r| r.theme.id.as_str()), Some("one"));
    assert_eq!(readable.index.len(), 1);
    assert!(readable.problems.is_empty(), "{:?}", readable.problems);
}

#[test]
fn a_deleted_app_theme_clears_while_the_suite_theme_keeps_its_last_version() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&SuiteDir::at(tmp.path()), options());
    store.import(theme_json("one").as_bytes(), false).unwrap();
    store.import(theme_json("two").as_bytes(), false).unwrap();
    let base = store.accepted().clone();
    store.update(&base, SelectionChange::whole(sel("one", Mode::Dark))).unwrap();
    let (_, wake) = counter();
    let watch = SuiteWatch::inline(Store::open(&SuiteDir::at(tmp.path()), options()), wake);
    let two = ThemeId::parse("two").unwrap();
    watch.request(Op::AppTheme(Some(two.clone())));
    assert_eq!(watch.snapshot().app_theme.as_ref().map(|r| &r.theme.id), Some(&two));
    // The same choice again is not news.
    let generation = watch.generation();
    watch.request(Op::AppTheme(Some(two.clone())));
    assert_eq!(watch.generation(), generation);

    fs::remove_file(tmp.path().join("library/one.json")).unwrap();
    fs::remove_file(tmp.path().join("library/two.json")).unwrap();
    watch.poll_once(false);
    let snapshot = watch.snapshot();
    assert_eq!(snapshot.active.as_ref().map(|r| r.theme.id.as_str()), Some("one"));
    assert!(snapshot.app_theme.is_none());
    assert!(snapshot.theme(&two).is_none());
    let missing: Vec<_> = snapshot.problems.iter().filter(|p| matches!(p, Problem::Theme { error: StoreError::NotFound(_), .. })).collect();
    assert_eq!(missing.len(), 2, "{:?}", snapshot.problems);
}

#[test]
fn dropping_releases_the_worker_every_time() {
    let tmp = tempfile::tempdir().unwrap();
    for _ in 0..50 {
        let token = Arc::new(());
        let held = Arc::clone(&token);
        let watch = SuiteWatch::open(&SuiteDir::at(tmp.path()), options(), move || {
            let _ = &held;
        });
        watch.request(Op::Rescan);
        drop(watch);
        assert_eq!(Arc::strong_count(&token), 1);
    }
}

#[test]
fn dropping_stops_and_joins_the_worker() {
    let tmp = tempfile::tempdir().unwrap();
    let token = Arc::new(());
    let held = Arc::clone(&token);
    let watch = SuiteWatch::open(&SuiteDir::at(tmp.path()), options(), move || {
        let _ = &held;
    });
    watch.request(Op::Rescan);
    let start = Instant::now();
    drop(watch);
    assert!(start.elapsed() < Duration::from_secs(1));
    // The wake callback lives as long as the worker: only the test's own reference is left.
    assert_eq!(Arc::strong_count(&token), 1);
}

#[test]
fn a_watch_can_be_shared_between_threads() {
    fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<SuiteWatch>();
}
