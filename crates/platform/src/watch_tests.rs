//! `craft_platform::watch`, inline (no worker thread).
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use std::path::PathBuf;

use craft_host::Wake;

use crate::temp::TempDir;
use crate::watch::*;

fn watcher() -> FileWatch<u32> {
    FileWatch::new(Wake::none()).inline()
}

/// Two looks: the first sees the change, the second sees it unchanged and reports it.
fn settle(watch: &mut FileWatch<u32>) {
    watch.tick();
    watch.tick();
}

fn file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// A file rewritten by another writer is reported once it has stayed the same for SETTLE; the
// version recorded with known_now after the app's own save is never reported.
#[test]
fn another_writer_is_a_change_and_an_own_save_is_not() {
    let dir = TempDir::new("craft-watch-test").unwrap();
    let path = file(&dir, "doc.txt", "one");
    let mut watch = watcher();
    watch.known_now(1, &path);
    watch.set([(1, path.clone())]);
    settle(&mut watch);
    assert!(watch.take_changes().is_empty());

    std::fs::write(&path, "two!").unwrap();
    watch.tick();
    assert!(watch.take_changes().is_empty(), "reported only after it stays the same");
    watch.tick();
    let changes = watch.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!((changes[0].key, changes[0].bytes.clone()), (1, None));
    assert_eq!(changes[0].stamp.len(), 4);

    // The app's own save.
    std::fs::write(&path, "three").unwrap();
    watch.known_now(1, &path);
    watch.set([(1, path)]);
    settle(&mut watch);
    assert!(watch.take_changes().is_empty());
}
