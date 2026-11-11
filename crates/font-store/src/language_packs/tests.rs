use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::store::Caller;

const RELEASE: &str = "https://github.com/storytold/craft-i18n/releases/download/language-packs-1/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IoFailure {
    Rename,
    RecordWrite,
    Cleanup,
    OpenStage,
}

thread_local! {
    static IO_FAILURE: Cell<Option<IoFailure>> = const { Cell::new(None) };
    static CLEANUP_ATTEMPTS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    #[cfg(unix)]
    static REPLACE_CLEANUP_PARENT: RefCell<Option<(PathBuf, PathBuf, PathBuf)>> = const { RefCell::new(None) };
    #[cfg(unix)]
    static REPLACE_MOVE_PARENT: RefCell<Option<(PathBuf, bool)>> = const { RefCell::new(None) };
}

struct InjectFailure(Option<IoFailure>);

impl InjectFailure {
    fn new(failure: IoFailure) -> Self {
        Self(IO_FAILURE.replace(Some(failure)))
    }
}

impl Drop for InjectFailure {
    fn drop(&mut self) {
        IO_FAILURE.set(self.0);
    }
}

pub(super) fn fail_io(failure: IoFailure) -> std::io::Result<()> {
    if IO_FAILURE.get() == Some(failure) {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("injected {failure:?} failure")))
    } else {
        Ok(())
    }
}

/// Record attempts and inject deterministic failures or a single ancestor replacement.
pub(super) fn before_cleanup(path: &Path) -> std::io::Result<()> {
    CLEANUP_ATTEMPTS.with_borrow_mut(|attempts| attempts.push(path.to_path_buf()));
    #[cfg(unix)]
    if let Some((parent, moved, outside)) = REPLACE_CLEANUP_PARENT.take() {
        std::fs::rename(&parent, moved)?;
        std::os::unix::fs::symlink(outside, parent)?;
    }
    fail_io(IoFailure::Cleanup)
}

fn take_cleanup_attempts() -> Vec<PathBuf> {
    CLEANUP_ATTEMPTS.with_borrow_mut(std::mem::take)
}

/// Replace the recovery parent immediately before moving a pack, keeping a matching outside ID.
pub(super) fn before_pack_move(stage: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some((outside, matching_id)) = REPLACE_MOVE_PARENT.take() {
        let parent = stage.parent().ok_or_else(|| std::io::Error::other("missing recovery parent"))?;
        let id = stage.file_name().ok_or_else(|| std::io::Error::other("missing recovery ID"))?;
        if matching_id {
            std::fs::create_dir(outside.join(id))?;
        }
        std::fs::rename(parent, parent.with_extension("original"))?;
        std::os::unix::fs::symlink(outside, parent)?;
    }
    #[cfg(not(unix))]
    let _ = stage;
    Ok(())
}

/// A local HTTP server answering 200 with the body set for a path, else 404.
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let r = routes.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let r = r.clone();
                std::thread::spawn(move || answer(stream, &r));
            }
        });
        Self { base, routes }
    }

    fn serve(&self, path: &str, body: &[u8]) {
        self.routes.lock().unwrap().insert(format!("/{path}"), body.to_vec());
    }

    /// A client that maps the release URL onto this server.
    fn client(&self) -> Client {
        let t = Timeouts { connect: Duration::from_secs(2), response: Duration::from_secs(5), overall: Duration::from_secs(10) };
        Client::for_tests(t, vec![(RELEASE.into(), format!("{}release/", self.base))])
    }
}

fn answer(mut s: TcpStream, routes: &Mutex<HashMap<String, Vec<u8>>>) {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut b) {
            Ok(1) => head.push(b[0]),
            _ => return,
        }
    }
    let path = String::from_utf8_lossy(&head).split_whitespace().nth(1).unwrap_or("/").to_string();
    let body = routes.lock().unwrap().get(&path).cloned();
    match body {
        Some(body) => {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(&body);
        }
        None => {
            let _ = write!(s, "HTTP/1.1 404 Nope\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
        }
    }
}

fn temp(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("craft-language-packs-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A tar of `files` (name, kind, bytes), with raw header names so hostile ones can be written.
fn tar_of(files: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, kind, bytes) in files {
        let mut h = tar::Header::new_old();
        let field = &mut h.as_old_mut().name;
        field[..name.len()].copy_from_slice(name.as_bytes());
        h.set_entry_type(*kind);
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        if *kind == tar::EntryType::Symlink || *kind == tar::EntryType::Link {
            h.set_link_name("/etc/passwd").unwrap();
        }
        h.set_cksum();
        b.append(&h, *bytes).unwrap();
    }
    b.into_inner().unwrap()
}

fn zst(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

const FILE: tar::EntryType = tar::EntryType::Regular;

/// A good pack archive for `locale`, with `marker` in its word list.
fn pack_archive(locale: &str, marker: &str) -> Vec<u8> {
    let toml = format!("locale = \"{locale}\"\nname = \"Test\"\nlicence = \"MIT\"\nclass = \"Embeddable\"\n");
    let dic = format!("2\nhuis\n{marker}\n");
    zst(&tar_of(&[
        ("pack.toml", FILE, toml.as_bytes()),
        ("spelling.aff", FILE, b"SET UTF-8\n"),
        ("spelling.dic", FILE, dic.as_bytes()),
        ("hyphenation.pat", FILE, b"1na n2at\n"),
        ("LICENCES/", tar::EntryType::Directory, b""),
        ("LICENCES/COPYING", FILE, b"Test licence."),
    ]))
}

fn entry(locale: &str, url: &str, archive: &[u8]) -> serde_json::Value {
    serde_json::json!({"locale": locale, "name": "Nederlands", "version": "abc", "url": url, "sha256": sha256_hex(archive),
        "size": archive.len(), "licence": "MIT", "class": "Embeddable", "spelling": true, "hyphenation": true})
}

/// The index entry for `archive` as `locale`.
fn pack_entry(locale: &str, archive: &[u8]) -> PackEntry {
    serde_json::from_value(entry(locale, &format!("{RELEASE}{locale}.tar.zst"), archive)).unwrap()
}

fn index_json(packs: &[serde_json::Value]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"format": 1, "release": "language-packs-1", "packs": packs})).unwrap()
}

/// A store whose network is on, a server with the index and the `nl` pack, and the pin.
fn setup(name: &str) -> (FontStore, Server, PackIndexPin, Vec<u8>) {
    let server = Server::start();
    let archive = pack_archive("nl", "zzmarker");
    let index = index_json(&[entry("nl", &format!("{RELEASE}nl.tar.zst"), &archive)]);
    server.serve("release/nl.tar.zst", &archive);
    server.serve("release/index.json", &index);
    let sha: &'static str = Box::leak(sha256_hex(&index).into_boxed_str());
    let pin = PackIndexPin { url: "https://github.com/storytold/craft-i18n/releases/download/language-packs-1/index.json", sha256: sha };
    let store = FontStore::new(temp(name), Caller::User).with_client(server.client());
    store.enable_network().unwrap();
    (store, server, pin, archive)
}

#[test]
fn removal_io_failures_preserve_the_pack_on_every_platform() {
    for failure in [IoFailure::Rename, IoFailure::RecordWrite] {
        let (store, _server, pin, _) = setup(&format!("remove-injected-{failure:?}"));
        let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
        let got = store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
        let dictionary = store.language_packs_dir().join("nl/spelling.dic");
        let before_dictionary = std::fs::read(&dictionary).unwrap();
        let before_record = std::fs::read(store.dir().join(INSTALLED)).unwrap();
        let injected = InjectFailure::new(failure);
        let result = store.remove_language_pack("nl");
        drop(injected);
        assert!(matches!(result, Err(Error::Io { .. })), "{failure:?}: {result:?}");
        assert_eq!(std::fs::read(&dictionary).unwrap(), before_dictionary, "{failure:?}");
        assert_eq!(std::fs::read(store.dir().join(INSTALLED)).unwrap(), before_record, "{failure:?}");
        assert_eq!(store.installed_language_packs(), vec![got]);
        assert!(std::fs::read_dir(store.language_packs_dir().join(".removing")).unwrap().next().is_none());
        std::fs::remove_dir_all(store.dir()).unwrap();
    }
}

#[test]
fn a_recovery_stage_whose_capability_cannot_open_is_not_left_behind() {
    let (store, _server, pin, _) = setup("remove-open-recovery-failure");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let injected = InjectFailure::new(IoFailure::OpenStage);
    let result = store.language_pack_removal_stage(&[]);
    drop(injected);
    assert!(matches!(result, Err(Error::Io { .. })), "{:?}", result.as_ref().map(|stage| &stage.dir));
    assert!(std::fs::read_dir(store.language_packs_dir().join(".removing")).unwrap().next().is_none());
    assert!(store.language_packs_dir().join("nl/spelling.dic").exists());
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn a_failed_removal_rollback_keeps_a_recoverable_pack_outside_download_cleanup() {
    let (store, _server, pin, _) = setup("remove-rollback-failure");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    let got = store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let before_record = std::fs::read(store.dir().join(INSTALLED)).unwrap();
    let dest = store.language_packs_dir().join("nl");
    let before_dictionary = std::fs::read(dest.join("spelling.dic")).unwrap();
    let stage = store.language_pack_removal_stage(&[]).unwrap();
    let stage_dir = stage.dir.clone();
    let backup = stage_dir.join("nl");
    std::fs::rename(&dest, &backup).unwrap();
    // A nonempty replacement directory prevents restoration on Unix and Windows.
    // A replacement file can be overwritten by a directory rename on Windows.
    std::fs::create_dir(&dest).unwrap();
    let replacement = dest.join("replacement");
    std::fs::write(&replacement, b"replacement").unwrap();
    let failed = io("saving the Language pack list")(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected record failure"));
    let result = finish_pack_removal(stage, &backup, &dest, true, Err(failed));
    assert!(matches!(result, Err(Error::Io { .. })));
    let error = result.unwrap_err().to_string();
    assert!(error.contains(&backup.display().to_string()), "{error}");
    let id = stage_dir.file_name().unwrap().to_str().unwrap();
    assert!(error.contains(&format!("recovery stage {id}")), "{error}");
    #[cfg(unix)]
    {
        let old = std::time::SystemTime::now() - Duration::from_secs(7200);
        std::fs::File::open(&stage_dir).unwrap().set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
    }
    store.sweep_staging();
    drop(store.lock().unwrap());
    assert_eq!(std::fs::read(backup.join("spelling.dic")).unwrap(), before_dictionary);
    assert_eq!(std::fs::read(&replacement).unwrap(), b"replacement");
    assert_eq!(std::fs::read(store.dir().join(INSTALLED)).unwrap(), before_record);
    std::fs::remove_dir_all(&dest).unwrap();
    std::fs::rename(&backup, &dest).unwrap();
    assert_eq!(store.installed_language_packs(), vec![got]);
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn committed_removal_cleanup_survives_interruption_and_does_not_reuse_a_recorded_id() {
    let (store, _server, pin, _) = setup("remove-committed-cleanup");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let mut installed = store.read_installed().unwrap();
    let stage = store.language_pack_removal_stage(&installed.committed_removals).unwrap();
    let id = stage.dir.file_name().unwrap().to_str().unwrap().to_string();
    let backup = stage.dir.join("nl");
    std::fs::rename(store.language_packs_dir().join("nl"), &backup).unwrap();
    installed.packs.clear();
    installed.committed_removals.push(id.clone());
    store.write_installed(&installed).unwrap();
    stage.keep_for_recovery(); // Simulate exiting after the atomic record commit, before Drop.
    let before_dictionary = std::fs::read(backup.join("spelling.dic")).unwrap();
    let before_record = std::fs::read(store.dir().join(INSTALLED)).unwrap();
    {
        let _injected = InjectFailure::new(IoFailure::Cleanup);
        let _lock = store.lock().unwrap();
        assert_eq!(std::fs::read(backup.join("spelling.dic")).unwrap(), before_dictionary);
        assert_eq!(std::fs::read(store.dir().join(INSTALLED)).unwrap(), before_record);
        assert_eq!(store.read_record().unwrap().committed_removals.as_slice(), std::slice::from_ref(&id));
        store.write_installed(&installed).unwrap();
        assert_eq!(store.read_record().unwrap().committed_removals.as_slice(), std::slice::from_ref(&id));
        assert_eq!(std::fs::read(store.dir().join(INSTALLED)).unwrap(), before_record);
    }
    let _lock = store.lock().unwrap();
    assert!(!backup.exists(), "a committed leftover is cleaned at the next locked store operation");
    let installed = store.read_installed().unwrap();
    assert_eq!(installed.committed_removals.as_slice(), std::slice::from_ref(&id));
    // A restarted PID/counter could propose the old name after cleanup removed its directory.
    // Its still-recorded receipt must exclude it until the next record update prunes it.
    let parent = store.pack_removal_dir().unwrap();
    let fresh = Stage::create_in(&parent, &installed.committed_removals, [id.clone(), "0-0".into()].into_iter()).unwrap();
    assert_eq!(fresh.dir.file_name().unwrap(), "0-0");
    drop(fresh);
    store.write_installed(&installed).unwrap();
    assert!(store.read_record().unwrap().committed_removals.is_empty());
    assert!(store.installed_language_packs().is_empty());
    drop(_lock);
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn committed_cleanup_attempts_are_bounded_and_rotate_past_failures() {
    let (store, _server, _, _) = setup("remove-bounded-cleanup");
    let parent = store.language_packs_dir().join(".removing");
    let ids: Vec<_> = (0..9).map(|n| format!("0-{n}")).collect();
    for id in &ids {
        std::fs::create_dir_all(parent.join(id)).unwrap();
        std::fs::write(parent.join(id).join("keep"), id).unwrap();
    }
    let mut installed = Installed { committed_removals: ids.clone(), ..Installed::default() };
    store.write_installed(&installed).unwrap();
    let before = std::fs::read(store.dir().join(INSTALLED)).unwrap();
    take_cleanup_attempts();
    {
        let _injected = InjectFailure::new(IoFailure::Cleanup);
        for expected in [&ids[..4], &ids[4..8], &ids[8..9]] {
            drop(store.lock().unwrap());
            let attempts = take_cleanup_attempts();
            assert_eq!(attempts.len(), 4, "cleanup work is bounded even when every deletion fails");
            for id in expected {
                assert!(attempts.contains(&parent.join(id)), "receipt {id} gets a turn despite failed earlier receipts");
            }
        }
        let reopened = FontStore::new(store.dir().to_path_buf(), Caller::User);
        drop(reopened.lock().unwrap());
        assert_eq!(std::fs::read(store.dir().join(INSTALLED)).unwrap(), before, "recreating the store never loses receipts");
        for id in &ids {
            assert_eq!(std::fs::read(parent.join(id).join("keep")).unwrap(), id.as_bytes());
        }
        // Adding one failed receipt per lock must not strand the oldest entries at a moving
        // tail. An ever-increasing counter modulo the growing length eventually does that.
        take_cleanup_attempts();
        for arrivals in [1, 4] {
            let store = FontStore::new(store.dir().to_path_buf(), Caller::User);
            let mut revisited_oldest = false;
            for n in 0..64 {
                for offset in 0..arrivals {
                    let id = format!("{arrivals}-{}", n * arrivals + offset);
                    std::fs::create_dir(parent.join(&id)).unwrap();
                    std::fs::write(parent.join(&id).join("keep"), &id).unwrap();
                    installed.committed_removals.push(id);
                }
                store.write_installed(&installed).unwrap();
                drop(store.lock().unwrap());
                let attempts = take_cleanup_attempts();
                assert_eq!(attempts.len(), 4);
                if n >= 40 && attempts.contains(&parent.join(&ids[0])) {
                    revisited_oldest = true;
                }
            }
            assert!(revisited_oldest, "old receipts keep getting turns with {arrivals} new failures per lock");
        }
        // A later record write can prune many already-deleted backups between rounds.
        for id in &installed.committed_removals[..installed.committed_removals.len() - 2] {
            std::fs::remove_dir_all(parent.join(id)).unwrap();
        }
        store.write_installed(&installed).unwrap();
        installed = store.read_record().unwrap();
        assert_eq!(installed.committed_removals.len(), 2);
        drop(store.lock().unwrap());
        let attempts = take_cleanup_attempts();
        assert_eq!(attempts.len(), 2);
        for id in &installed.committed_removals {
            assert!(attempts.contains(&parent.join(id)), "shrinking the list does not skip retained receipts");
        }
    }
    for _ in 0..installed.committed_removals.len() {
        drop(store.lock().unwrap());
    }
    assert!(std::fs::read_dir(parent).unwrap().next().is_none());
    store.write_installed(&installed).unwrap();
    assert!(store.read_record().unwrap().committed_removals.is_empty());
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[cfg(unix)]
#[test]
fn recovery_cleanup_does_not_follow_a_replaced_parent_symlink() {
    let (store, _server, pin, _) = setup("remove-symlink-parent");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let outside = temp("remove-symlink-outside");
    std::fs::create_dir(outside.join("11-12")).unwrap();
    std::fs::write(outside.join("11-12/keep"), b"untouched").unwrap();
    std::os::unix::fs::symlink(&outside, store.language_packs_dir().join(".removing")).unwrap();
    let mut record = store.read_installed().unwrap();
    record.committed_removals.push("11-12".into());
    store.write_installed(&record).unwrap();
    drop(store.lock().unwrap());
    assert!(matches!(store.remove_language_pack("nl"), Err(Error::Io { .. })));
    assert_eq!(std::fs::read(outside.join("11-12/keep")).unwrap(), b"untouched");
    assert!(store.language_packs_dir().join("nl/spelling.dic").is_file());
    std::fs::remove_file(store.language_packs_dir().join(".removing")).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// A replacement after validation must not redirect recursive deletion to the new target.
#[cfg(unix)]
#[test]
fn residual_cleanup_stays_anchored_after_recovery_parent_replacement() {
    let (store, _server, _pin, _) = setup("remove-parent-race");
    let parent = store.language_packs_dir().join(".removing");
    let moved = store.language_packs_dir().join(".removing-old");
    let outside = temp("remove-parent-race-outside");
    for base in [&parent, &outside] {
        std::fs::create_dir_all(base.join("11-12")).unwrap();
        std::fs::write(base.join("11-12/keep"), b"untouched").unwrap();
    }
    let mut record = Installed::default();
    record.committed_removals.push("11-12".into());
    store.write_installed(&record).unwrap();
    REPLACE_CLEANUP_PARENT.set(Some((parent.clone(), moved.clone(), outside.clone())));
    drop(store.lock().unwrap());
    assert_eq!(std::fs::read(outside.join("11-12/keep")).unwrap(), b"untouched");
    assert!(!moved.join("11-12").exists(), "the originally opened recovery folder is cleaned");
    std::fs::remove_file(parent).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// Pack moves and cleanup must agree on the originally opened recovery directory.
#[cfg(unix)]
#[test]
fn removal_move_stays_with_its_cleanup_anchor_after_parent_replacement() {
    let (store, _server, pin, _) = setup("remove-move-parent-race");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let outside = temp("remove-move-parent-race-outside");
    std::fs::write(outside.join("keep"), b"untouched").unwrap();
    REPLACE_MOVE_PARENT.set(Some((outside.clone(), true)));
    store.remove_language_pack("nl").unwrap();
    assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"untouched");
    for entry in std::fs::read_dir(&outside).unwrap().flatten() {
        assert!(!entry.path().join("nl/spelling.dic").exists(), "the dictionary must never move into the replacement recovery target");
    }
    assert!(std::fs::read_dir(store.language_packs_dir().join(".removing.original")).unwrap().next().is_none());
    std::fs::remove_file(store.language_packs_dir().join(".removing")).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// An unavailable display path must not prune a receipt for a backup held by its capability.
#[cfg(unix)]
#[test]
fn committed_receipt_survives_parent_replacement_and_failed_immediate_cleanup() {
    let (store, _server, pin, _) = setup("remove-receipt-parent-race");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let dictionary = std::fs::read(store.language_packs_dir().join("nl/spelling.dic")).unwrap();
    let outside = temp("remove-receipt-parent-race-outside");
    REPLACE_MOVE_PARENT.set(Some((outside.clone(), false)));
    let injected = InjectFailure::new(IoFailure::Cleanup);
    store.remove_language_pack("nl").unwrap();
    drop(injected);
    let record = store.read_record().unwrap();
    assert_eq!(record.committed_removals.len(), 1, "the committed receipt must survive a missing path under the replacement parent");
    let id = &record.committed_removals[0];
    let parent = store.language_packs_dir().join(".removing");
    let original = parent.with_extension("original");
    assert_eq!(std::fs::read(original.join(id).join("nl/spelling.dic")).unwrap(), dictionary);
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    std::fs::remove_file(&parent).unwrap();
    std::fs::rename(original, &parent).unwrap();
    drop(store.lock().unwrap());
    assert!(!parent.join(id).exists(), "the persisted receipt drives a later anchored retry");
    store.write_installed(&record).unwrap();
    assert!(store.read_record().unwrap().committed_removals.is_empty());
    std::fs::remove_dir_all(outside).unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// Removing a stale record needs no recovery directory when there is no folder to move.
#[test]
fn removing_a_missing_pack_folder_only_updates_the_record() {
    let (store, _server, pin, _) = setup("remove-missing-folder");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    std::fs::remove_dir_all(store.language_packs_dir().join("nl")).unwrap();
    let blocker = store.language_packs_dir().join(".removing");
    std::fs::write(&blocker, b"not a directory").unwrap();
    store.remove_language_pack("nl").expect("no recovery folder is needed for a record-only removal");
    assert!(store.read_record().unwrap().packs.is_empty());
    assert_eq!(std::fs::read(blocker).unwrap(), b"not a directory");
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// Immediate stage cleanup must not follow a recovery parent replaced after stage creation.
#[cfg(unix)]
#[test]
fn recovery_stage_drop_stays_anchored_after_parent_replacement() {
    let (store, _server, _pin, _) = setup("remove-stage-parent-race");
    let stage = store.language_pack_removal_stage(&[]).unwrap();
    let id = stage.dir.file_name().unwrap().to_owned();
    std::fs::write(stage.dir.join("owned"), b"staged").unwrap();
    let outside = temp("remove-stage-parent-race-outside");
    std::fs::create_dir(outside.join(&id)).unwrap();
    std::fs::write(outside.join(&id).join("keep"), b"untouched").unwrap();
    let parent = store.language_packs_dir().join(".removing");
    let moved = store.language_packs_dir().join(".removing-old");
    std::fs::rename(&parent, &moved).unwrap();
    std::os::unix::fs::symlink(&outside, &parent).unwrap();
    drop(stage);
    assert_eq!(std::fs::read(outside.join(&id).join("keep")).unwrap(), b"untouched");
    assert!(!moved.join(&id).exists(), "the original stage is cleaned via its retained parent");
    std::fs::remove_file(parent).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// Retained Windows handles prevent replacement of every recovery ancestor during cleanup.
#[cfg(windows)]
#[test]
fn recovery_cleanup_locks_all_windows_ancestors_until_removal_finishes() {
    let (store, _server, _pin, _) = setup("remove-windows-parent-locks");
    let parent = store.language_packs_dir().join(".removing");
    let outside = temp("remove-windows-parent-outside");
    std::fs::create_dir_all(parent.join("11-12")).unwrap();
    std::fs::create_dir_all(outside.join("11-12")).unwrap();
    std::fs::write(outside.join("11-12/keep"), b"untouched").unwrap();
    let root = PackRemovalRoot::open(store.dir()).unwrap();
    let packs = store.language_packs_dir();
    for ancestor in [parent.as_path(), packs.as_path(), store.dir()] {
        assert!(
            std::fs::rename(ancestor, ancestor.with_extension("replaced")).is_err(),
            "an open ancestor cannot be replaced: {}",
            ancestor.display()
        );
    }
    root.remove("11-12", &parent).unwrap();
    assert!(!parent.join("11-12").exists());
    assert_eq!(std::fs::read(outside.join("11-12/keep")).unwrap(), b"untouched");
    drop(root);
    std::fs::remove_dir_all(outside).unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

/// A malformed optional receipt must not discard valid receipts or installed-pack metadata.
#[test]
fn residual_one_bad_receipt_preserves_valid_metadata_and_cleanup() {
    let (store, _server, pin, _) = setup("remove-mixed-receipts");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    store.install_pack_from(false, &index, index.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    let original = store.read_record().unwrap();
    let parent = store.language_packs_dir().join(".removing");
    std::fs::create_dir_all(parent.join("11-12")).unwrap();
    std::fs::write(parent.join("11-12/keep"), b"committed").unwrap();
    let mut raw = serde_json::to_value(&original).unwrap();
    raw["committed_removals"] = serde_json::json!(["11-12", "../outside", null, 42, true, {}, ["99-99"]]);
    std::fs::write(store.dir().join(INSTALLED), serde_json::to_vec(&raw).unwrap()).unwrap();
    let record = store.read_record().expect("one invalid receipt must not invalidate the whole record");
    assert_eq!(record.committed_removals, ["11-12"]);
    assert_eq!(serde_json::to_value(&record.packs).unwrap(), serde_json::to_value(&original.packs).unwrap());
    drop(store.lock().unwrap());
    assert!(!parent.join("11-12").exists());
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn network_off_pins_and_checksums_are_enforced() {
    let (store, server, pin, archive) = setup("pins");
    let source = IndexSource::Pinned(pin);
    store.disable_network().unwrap();
    assert!(matches!(store.fetch_pack_index_from(&source, &Cancel::new()), Err(Error::NetworkOff)));
    store.enable_network().unwrap();
    // Another index than the pinned one.
    let wrong = IndexSource::Pinned(PackIndexPin { sha256: "0000000000000000000000000000000000000000000000000000000000000000", ..pin });
    assert!(matches!(store.fetch_pack_index_from(&wrong, &Cancel::new()), Err(Error::Verify { error: VerifyError::Sha256, .. })));
    let index = store.fetch_pack_index_from(&source, &Cancel::new()).unwrap();
    assert_eq!(store.cached_pack_index_from(&wrong), None, "the cache is the pinned index only");
    let nl = index.pack("nl").unwrap().clone();
    // An archive that isn't the one the index names.
    server.serve("release/nl.tar.zst", &pack_archive("nl", "tampered"));
    assert!(store.install_pack_from(false, &index, &nl, &Cancel::new(), &mut |_| {}).is_err());
    server.serve("release/nl.tar.zst", &archive);
    let bad_sha = PackEntry { sha256: sha256_hex(b"other"), ..nl.clone() };
    assert!(matches!(
        store.install_pack_from(false, &index, &bad_sha, &Cancel::new(), &mut |_| {}),
        Err(Error::Verify { error: VerifyError::Sha256, .. })
    ));
    assert!(store.installed_language_packs().is_empty() && !store.language_packs_dir().join("nl").exists());
    // Hand-made entries are checked again.
    for e in [PackEntry { locale: "../nl".into(), ..nl.clone() }, PackEntry { url: "http://example.org/nl.tar.zst".into(), ..nl.clone() }] {
        assert!(store.install_pack_from(false, &index, &e, &Cancel::new(), &mut |_| {}).is_err());
    }
    store.disable_network().unwrap();
    assert!(matches!(store.install_pack_from(false, &index, &nl, &Cancel::new(), &mut |_| {}), Err(Error::NetworkOff)));
}

#[test]
fn hostile_archives_are_refused() {
    let toml: &[u8] = b"locale = \"nl\"\nlicence = \"MIT\"\n";
    let nl = PackEntry { spelling: false, hyphenation: false, ..pack_entry("nl", b"") };
    let big = vec![0u8; 10];
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("traversal", zst(&tar_of(&[("pack.toml", FILE, toml), ("../evil", FILE, b"x")]))),
        ("deep traversal", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/../../evil", FILE, b"x")]))),
        ("absolute", zst(&tar_of(&[("pack.toml", FILE, toml), ("/tmp/evil", FILE, b"x")]))),
        ("symlink", zst(&tar_of(&[("pack.toml", FILE, toml), ("spelling.dic", tar::EntryType::Symlink, b"")]))),
        ("hard link", zst(&tar_of(&[("pack.toml", FILE, toml), ("spelling.dic", tar::EntryType::Link, b"")]))),
        ("too deep", zst(&tar_of(&[("pack.toml", FILE, toml), ("a/b/c", FILE, b"x")]))),
        ("duplicate", zst(&tar_of(&[("pack.toml", FILE, toml), ("pack.toml", FILE, toml)]))),
        ("backslash", zst(&tar_of(&[("pack.toml", FILE, toml), ("..\\evil", FILE, b"x")]))),
        ("no pack.toml", zst(&tar_of(&[("spelling.dic", FILE, b"1\nx\n")]))),
        ("not zstd", tar_of(&[("pack.toml", FILE, toml)])),
        ("not tar", zst(b"hello, this is not a tar archive at all")),
        ("empty", Vec::new()),
        // Only the contract's names.
        ("other root file", zst(&tar_of(&[("pack.toml", FILE, toml), ("readme.txt", FILE, b"x")]))),
        ("other folder", zst(&tar_of(&[("pack.toml", FILE, toml), ("extra/", tar::EntryType::Directory, b"")]))),
        ("nested licence", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/sub/x", FILE, b"x")]))),
        ("hidden licence", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/.x", FILE, b"x")]))),
        ("space in licence", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/a b", FILE, b"x")]))),
        ("trailing dot", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/COPYING.", FILE, b"x")]))),
        ("reserved CON", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/CON", FILE, b"x")]))),
        ("reserved nul.txt", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/nul.txt", FILE, b"x")]))),
        ("reserved com1", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/com1.md", FILE, b"x")]))),
        // pack.toml must name the entry's locale.
        ("other locale", zst(&tar_of(&[("pack.toml", FILE, b"locale = \"de\"\n")]))),
        ("no locale", zst(&tar_of(&[("pack.toml", FILE, b"licence = \"MIT\"\n")]))),
        ("bad toml", zst(&tar_of(&[("pack.toml", FILE, b"locale = \n")]))),
        // Nothing may follow the archive: not another frame, not raw bytes.
        ("second archive", [zst(&tar_of(&[("pack.toml", FILE, toml)])), zst(&tar_of(&[("spelling.dic", FILE, b"1\nx\n")]))].concat()),
        ("trailing bytes", [zst(&tar_of(&[("pack.toml", FILE, toml)])), b"garbage".to_vec()].concat()),
        ("truncated", zst(&tar_of(&[("pack.toml", FILE, toml), ("spelling.dic", FILE, &big)]))[..20].to_vec()),
    ];
    for (what, archive) in cases {
        let root = temp("hostile");
        let dest = root.join("nl");
        assert!(unpack(&archive, &dest, &nl).is_err(), "{what}");
        assert!(!root.join("evil").exists() && !root.parent().unwrap().join("evil").exists(), "{what}");
    }
    // Too many entries.
    let names: Vec<String> = (0..=MAX_PACK_ENTRIES).map(|i| format!("LICENCES/f{i}")).collect();
    let mut files: Vec<(&str, tar::EntryType, &[u8])> = vec![("pack.toml", FILE, toml)];
    files.extend(names.iter().map(|n| (n.as_str(), FILE, &b""[..])));
    assert!(unpack(&zst(&tar_of(&files)), &temp("many").join("nl"), &nl).is_err());
    // A header claiming a file larger than the cap is refused before anything is read.
    let mut t = tar::Header::new_gnu();
    t.set_path("pack.toml").unwrap();
    t.set_size(toml.len() as u64);
    t.set_cksum();
    let mut b = tar::Builder::new(Vec::new());
    b.append(&t, toml).unwrap();
    let mut tarball = b.into_inner().unwrap();
    tarball.truncate(tarball.len() - 1024); // drop the end marker
    let mut h = tar::Header::new_gnu();
    h.set_path("spelling.dic").unwrap();
    h.set_size(MAX_PACK_FILE_BYTES + 1);
    h.set_cksum();
    tarball.extend_from_slice(h.as_bytes());
    tarball.extend_from_slice(&[0u8; 1024]);
    let err = unpack(&zst(&tarball), &temp("huge").join("nl"), &nl).unwrap_err();
    assert!(err.contains("larger than"), "{err}");
    // A good one unpacks.
    let dest = temp("good").join("nl");
    let good = pack_archive("nl", "x");
    unpack(&good, &dest, &pack_entry("nl", &good)).unwrap();
    assert!(dest.join("LICENCES/COPYING").is_file());
    // A tar split over several zstd frames is read to the end.
    let tarball = tar_of(&[("pack.toml", FILE, toml), ("LICENCES/", tar::EntryType::Directory, b""), ("LICENCES/COPYING", FILE, b"t")]);
    let (a, b) = tarball.split_at(700);
    let dest = temp("frames").join("nl");
    unpack(&[zst(a), zst(b)].concat(), &dest, &nl).unwrap();
    assert!(dest.join("LICENCES/COPYING").is_file());
    // Empty frames are read past, but only so many.
    let one = zst(&tar_of(&[("pack.toml", FILE, toml)]));
    let empty = zst(b"");
    unpack(&[one.clone(), empty.clone(), empty.clone()].concat(), &temp("empties").join("nl"), &nl).unwrap();
    let many: Vec<u8> = std::iter::once(one).chain(std::iter::repeat_n(empty, MAX_PACK_FRAMES)).flatten().collect();
    let err = unpack(&many, &temp("frames-cap").join("nl"), &nl).unwrap_err();
    assert!(err.contains("frames"), "{err}");
    // The index's promises are checked against the files.
    let bare = zst(&tar_of(&[("pack.toml", FILE, toml)]));
    for e in [PackEntry { spelling: true, ..nl.clone() }, PackEntry { hyphenation: true, ..nl.clone() }] {
        let err = unpack(&bare, &temp("promise").join("nl"), &e).unwrap_err();
        assert!(err.contains("promises"), "{err}");
    }
}
