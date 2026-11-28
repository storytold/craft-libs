//! `craft_platform::autosave`.
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::autosave::*;

/// A fresh folder under the test temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("craft-autosave-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn doc(id: u64, revision: u64, dirty: bool) -> AutosaveDoc {
    AutosaveDoc { id, revision, dirty }
}

fn meta(name: &str, revision: u64) -> RecoveryMeta {
    RecoveryMeta { name: name.into(), original: None, extension: "pcraft".into(), revision, protected: false }
}

// Nothing is due before the interval; then only dirty documents with a new revision and no
// autosave in flight; a failure returns "Autosave failed: …".
#[test]
fn due_picks_dirty_changed_documents_each_interval() {
    let minute = Duration::from_secs(60);
    let t0 = Instant::now();
    let mut timer = Autosave::new(Some(minute));
    let docs = |rev| [doc(1, rev, true), doc(2, 1, false)];
    assert!(timer.due(t0, docs(1)).save.is_empty(), "the first call starts the clock");
    assert_eq!(timer.deadline(), Some(t0 + minute));
    assert!(timer.due(t0 + minute / 2, docs(1)).save.is_empty());
    assert_eq!(timer.due(t0 + minute, docs(1)).save, vec![1]);
    assert!(timer.due(t0 + minute * 2, docs(1)).save.is_empty(), "still in flight");
    assert_eq!(timer.finished(1, 1, Ok(())), None);
    assert!(timer.due(t0 + minute * 3, docs(1)).save.is_empty(), "revision already autosaved");
    assert_eq!(timer.due(t0 + minute * 4, docs(2)).save, vec![1]);
    assert_eq!(timer.finished(1, 2, Err("disk full".into())).as_deref(), Some("Autosave failed: disk full"));
    assert_eq!(timer.due(t0 + minute * 5, docs(2)).save, vec![1], "a failed revision is tried again");
    timer.forget(1);
    assert_eq!(timer.finished(1, 2, Ok(())), None, "a result after forget is ignored");

    // A document that turns clean without a save is handed back for discarding, once.
    let mut timer = Autosave::new(Some(minute));
    timer.due(t0, docs(1));
    assert_eq!(timer.due(t0 + minute, docs(1)).save, vec![1]);
    assert_eq!(timer.finished(1, 1, Ok(())), None);
    let clean = timer.due(t0 + minute * 2, [doc(1, 1, false)]);
    assert_eq!(clean.discard, vec![1]);
    assert!(timer.due(t0 + minute * 3, [doc(1, 1, false)]).discard.is_empty());

    // An old job's result does not complete the new job of the same document.
    let mut timer = Autosave::new(Some(minute));
    timer.due(t0, docs(1));
    assert_eq!(timer.due(t0 + minute, docs(1)).save, vec![1]);
    timer.forget(1);
    assert_eq!(timer.due(t0 + minute * 2, docs(2)).save, vec![1]);
    assert_eq!(timer.finished(1, 1, Err("late".into())), None, "revision 1 is not the one in flight");
    assert!(timer.due(t0 + minute * 3, docs(2)).save.is_empty(), "revision 2 is still in flight");
    assert_eq!(timer.finished(1, 2, Ok(())), None);

    timer.set_interval(None);
    assert!(timer.due(t0 + minute * 9, docs(3)).save.is_empty());
    assert_eq!(timer.deadline(), None);
}

// The deadline wakes the window only while something waits to be autosaved: none when every
// document is clean, autosaved or being written; back on an edit or a failed write.
#[test]
fn the_deadline_is_none_while_nothing_waits_to_be_autosaved() {
    let minute = Duration::from_secs(60);
    let t0 = Instant::now();
    let mut timer = Autosave::new(Some(minute));
    timer.due(t0, [doc(1, 1, false)]);
    assert_eq!(timer.deadline(), None, "a clean document has nothing to save");
    timer.due(t0 + minute / 2, [doc(1, 2, true)]);
    assert!(timer.deadline().is_some(), "an edit brings the deadline back");
    assert_eq!(timer.due(t0 + minute, [doc(1, 2, true)]).save, vec![1]);
    assert_eq!(timer.deadline(), None, "the revision is being written");
    assert_eq!(timer.finished(1, 2, Ok(())), None);
    timer.due(t0 + minute * 2, [doc(1, 2, true)]);
    assert_eq!(timer.deadline(), None, "the revision is autosaved");
    timer.due(t0 + minute * 5 / 2, [doc(1, 3, true)]);
    assert!(timer.deadline().is_some());
    assert_eq!(timer.due(t0 + minute * 3, [doc(1, 3, true)]).save, vec![1]);
    assert!(timer.finished(1, 3, Err("disk full".into())).is_some());
    assert!(timer.deadline().is_some(), "a failed revision is tried again");
}

// An autosave after the recovery folder was removed (a cleaner, the user) makes it again.
#[test]
fn a_write_after_the_folder_was_removed_makes_it_again() {
    let scratch = Scratch::new();
    let dir = scratch.0.join("Recovery");
    let store = RecoveryStore::open(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    store.write(1, &meta("a.psd", 1), b"work").unwrap();
    assert_eq!(std::fs::read(dir.join(format!("{}.pcraft", store.key(1)))).unwrap(), b"work");
}

// A copy written by a run that is gone is a leftover; one of a live instance (its lock held) is
// not; discard removes copy and sidecar.
#[test]
fn leftovers_skip_live_instances() {
    let scratch = Scratch::new();
    let dir = scratch.0.join("Recovery");
    let first = RecoveryStore::open(&dir).unwrap();
    first.write(1, &meta("a.psd", 3), b"first").unwrap();
    first.write(2, &meta("b.psd", 1), b"second").unwrap();
    first.discard(2).unwrap();
    assert!(!dir.join(format!("{}.pcraft", first.key(2))).exists());
    assert!(!dir.join(format!("{}.json", first.key(2))).exists());

    let second = RecoveryStore::open(&dir).unwrap();
    assert!(second.leftovers().is_empty(), "the first run is still alive");
    assert!(first.leftovers().is_empty(), "a store never offers its own copies");

    let key = first.key(1);
    drop(first);
    let found = second.leftovers();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].key, key);
    assert_eq!(found[0].meta, meta("a.psd", 3));
    assert_eq!(std::fs::read(&found[0].path).unwrap(), b"first");

    // Recovering adopts the key: the next autosave replaces the copy in place.
    second.adopt(7, &found[0]);
    assert_eq!(second.key(7), key);
    second.write(7, &meta("a.psd", 4), b"third").unwrap();
    assert!(second.leftovers().is_empty(), "adopted copies now belong to the live run");
    second.remove(&found[0]).unwrap();
    assert!(!found[0].path.exists());
}

/// The sidecar of an earlier run, as `RecoveryStore::write` lays it out, written by hand.
fn sidecar(name: &str, instance: Option<&str>) -> String {
    serde_json::json!({"name": name, "original": null, "extension": "pcraft", "revision": 1, "instance": instance}).to_string()
}

/// The leftover listed under `key`.
fn listed(store: &RecoveryStore, key: &str) -> Option<RecoveryEntry> {
    store.leftovers().into_iter().find(|e| e.key == key)
}

// The recovery folder is untrusted: a planted sidecar over MAX_SIDECAR is never read whole (its
// copy is still offered, under its key), while one at the cap is read.
#[test]
fn a_sidecar_over_the_size_cap_is_ignored_and_one_under_it_is_read() {
    let scratch = Scratch::new();
    let store = RecoveryStore::open(scratch.0.join("Recovery")).unwrap();
    let cap = usize::try_from(MAX_SIDECAR).unwrap();
    std::fs::write(store.dir().join("big.pcraft"), b"copy").unwrap();
    let oversized = sidecar(&"x".repeat(cap), None);
    assert!(oversized.len() > cap);
    std::fs::write(store.dir().join("big.json"), &oversized).unwrap();
    let entry = listed(&store, "big").unwrap();
    assert_eq!((entry.meta.name.as_str(), entry.meta.revision), ("big", 0), "the oversized sidecar was read");
    assert_eq!(std::fs::read(&entry.path).unwrap(), b"copy");

    let mut at_cap = sidecar("work.pdf", None);
    at_cap.push_str(&" ".repeat(cap - at_cap.len()));
    std::fs::write(store.dir().join("big.json"), &at_cap).unwrap();
    assert_eq!(listed(&store, "big").unwrap().meta.name, "work.pdf");
}

// A copy whose sidecar never got written (a crash between the two writes) or can't be parsed is
// still the user's work: it is offered under its key, and listing deletes nothing.
#[test]
fn an_orphan_copy_and_a_broken_sidecar_are_offered_and_listing_removes_nothing() {
    let scratch = Scratch::new();
    let store = RecoveryStore::open(scratch.0.join("Recovery")).unwrap();
    let files = ["orphan.pdf", "broken.pcraft", "broken.json"];
    for file in files {
        let bytes: &[u8] = if file.ends_with(".json") { b"{\"name\": " } else { b"work" };
        std::fs::write(store.dir().join(file), bytes).unwrap();
    }
    for _ in 0..2 {
        let orphan = listed(&store, "orphan").unwrap();
        assert_eq!((orphan.meta.name.as_str(), orphan.path.file_name().and_then(|n| n.to_str())), ("orphan", Some("orphan.pdf")));
        let broken = listed(&store, "broken").unwrap();
        assert_eq!((broken.meta.name.as_str(), broken.path.file_name().and_then(|n| n.to_str())), ("broken", Some("broken.pcraft")));
    }
    for file in files {
        assert!(store.dir().join(file).exists(), "{file} was removed by listing");
    }
}

// The copy is written before its sidecar. When the sidecar can't be written (here a folder stands
// at its name) the write reports the failure, and the copy stays and is offered after a crash.
#[test]
fn a_failed_sidecar_write_keeps_the_copy_offered() {
    let scratch = Scratch::new();
    let dir = scratch.0.join("Recovery");
    let crashed = RecoveryStore::open(&dir).unwrap();
    let key = crashed.key(1);
    std::fs::create_dir(dir.join(format!("{key}.json"))).unwrap();
    assert!(crashed.write(1, &meta("a.pdf", 2), b"work").is_err());
    let copy = dir.join(format!("{key}.pcraft"));
    assert_eq!(std::fs::read(&copy).unwrap(), b"work");
    drop(crashed);

    let next = RecoveryStore::open(&dir).unwrap();
    let entry = listed(&next, &key).unwrap();
    assert_eq!(entry.path, copy);
}

// The lock name in a sidecar is untrusted. One that leaves the folder is never opened: here it
// would name a lock another run holds outside the folder, which would hide the copy, so the copy
// is offered as a plain leftover and the outside file is left alone.
#[test]
fn a_sidecar_naming_a_lock_outside_the_folder_is_a_plain_leftover() {
    let scratch = Scratch::new();
    let store = RecoveryStore::open(scratch.0.join("Recovery")).unwrap();
    let outside = scratch.0.join("instance-held.lock");
    let held = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&outside).unwrap();
    held.try_lock().unwrap();
    // Makes the escaping name resolve on every platform, were it ever opened.
    std::fs::create_dir(store.dir().join("instance-x")).unwrap();
    for (key, instance) in [("up", "instance-x/../../instance-held.lock"), ("back", r"instance-x\..\..\instance-held.lock")] {
        std::fs::write(store.dir().join(format!("{key}.pcraft")), b"work").unwrap();
        std::fs::write(store.dir().join(format!("{key}.json")), sidecar("a.pdf", Some(instance))).unwrap();
        let entry = listed(&store, key);
        assert_eq!(entry.map(|e| e.meta.name), Some("a.pdf".to_owned()), "{instance}");
    }
    assert!(outside.exists());
}
