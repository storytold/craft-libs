//! `LinkMonitor` over `FakeLinkFiles` and `ManualJobs`.

use craft_ui::testing::trial;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use craft_docpath::links::{FakeLinkFiles, FileInfo, FolderEntry, Link, LinkFiles, LinkIdentity, LinkStatus};
use craft_host::{ManualJobs, Wake};

use super::{LinkChange, LinkMonitor};

/// A `FakeLinkFiles` the test can replace while a monitor holds it ("the files changed").
struct Disk(Mutex<FakeLinkFiles>);

impl Disk {
    fn new(files: FakeLinkFiles) -> Arc<Self> {
        Arc::new(Self(Mutex::new(files)))
    }

    fn change(&self, files: FakeLinkFiles) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = files;
    }

    fn identity_of(&self, path: &str) -> LinkIdentity {
        self.identity(Path::new(path)).unwrap()
    }
}

impl LinkFiles for Disk {
    fn stat(&self, path: &Path) -> Option<FileInfo> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).stat(path)
    }

    fn identity(&self, path: &Path) -> io::Result<LinkIdentity> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).identity(path)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<FolderEntry>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).list(dir)
    }
}

fn link(key: u32, stored: &str, identity: LinkIdentity) -> Link<u32> {
    Link { key, name: stored.into(), stored: stored.into(), identity: Some(identity), page: None, embedded: false }
}

fn monitor(files: &Arc<Disk>, jobs: &Arc<ManualJobs>) -> LinkMonitor<u32> {
    LinkMonitor::new(files.clone(), jobs.clone(), Wake::none())
}

// A file changes once: one LinkChange Ok -> Modified, and nothing on the following passes.
#[trial]
fn the_monitor_reports_a_change_once() {
    let files = Disk::new(FakeLinkFiles::new().with("/p/a.png", b"abc", 1));
    let a = link(1, "/p/a.png", files.identity_of("/p/a.png"));
    let jobs = Arc::new(ManualJobs::new());
    let mut m = monitor(&files, &jobs);
    m.set(&[a], None);
    jobs.run_all();
    assert_eq!(m.take_changes(), vec![LinkChange { key: 1, before: None, status: LinkStatus::Ok }]);

    files.change(FakeLinkFiles::new().with("/p/a.png", b"abcd", 2));
    m.window_focused();
    jobs.run_all();
    assert_eq!(m.take_changes(), vec![LinkChange { key: 1, before: Some(LinkStatus::Ok), status: LinkStatus::Modified }]);
    assert_eq!(m.status(&1), Some(LinkStatus::Modified));

    m.window_focused();
    jobs.run_all();
    assert!(m.take_changes().is_empty());
    assert!(m.take_changes().is_empty());
}

// `window_focused` checks every link again; unchanged links report nothing.

// A check that finished before the link was updated is stale when it is taken in: it reports nothing, and the newer check decides.
#[trial]
fn a_check_older_than_known_now_is_dropped() {
    let files = Disk::new(FakeLinkFiles::new().with("/p/a.png", b"abc", 1));
    let old = link(1, "/p/a.png", files.identity_of("/p/a.png"));
    files.change(FakeLinkFiles::new().with("/p/a.png", b"abcd", 2));
    let updated = link(1, "/p/a.png", files.identity_of("/p/a.png"));
    let jobs = Arc::new(ManualJobs::new());
    let mut m = monitor(&files, &jobs);
    m.set(&[old], None);
    jobs.run_all(); // saw the changed file: Modified, not yet taken in
    m.set(&[updated], None); // the app updated the link to the new file
    m.known_now(&1, LinkStatus::Ok);
    jobs.run_all();
    assert!(m.take_changes().is_empty(), "the stale Modified is not reported");
    assert_eq!(m.status(&1), Some(LinkStatus::Ok));
}
