use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use craft_nextcloud::testing::{FakeServer, Fault};
use craft_nextcloud::{Client, Error, Fetched, OriginalsCache, RemotePath, Usage};

fn client(server: &FakeServer) -> Client {
    Client::new(server.account(), FakeServer::PASSWORD.into(), Arc::new(rustls_rustcrypto::provider())).unwrap()
}

fn path(client: &Client, name: &str) -> RemotePath {
    client.account().root().join(name).unwrap()
}

fn fetch(cache: &OriginalsCache, client: &Client, p: &RemotePath) -> Result<Fetched, Error> {
    cache.fetch(client, p, &AtomicBool::new(false), &mut |_, _| {})
}

/// Names of the files in `dir` and its subfolders.
fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            out.extend(files(&e.path()));
        } else {
            out.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

#[test]
fn miss_then_hit() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", b"raw bytes".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let p = path(&client, "a.ARW");

    let first = fetch(&cache, &client, &p).unwrap();
    assert!(!first.stale);
    assert_eq!(fs::read(&first.path).unwrap(), b"raw bytes");
    assert_eq!(server.requests(), 2);

    let second = fetch(&cache, &client, &p).unwrap();
    assert_eq!(second, first);
    assert_eq!(server.requests(), 3);

    let etag = client.stat(&p).unwrap().etag;
    assert_eq!(cache.cached(&p, &etag), Some(first.path));
    assert_eq!(cache.cached(&p, "other"), None);
}

#[test]
fn stale_etag_replaces_old_copy() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", b"old".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let p = path(&client, "a.ARW");

    let old = fetch(&cache, &client, &p).unwrap();
    server.add_file("/a.ARW", b"new".to_vec());
    let new = fetch(&cache, &client, &p).unwrap();
    assert_ne!(new.path, old.path);
    assert_eq!(fs::read(&new.path).unwrap(), b"new");
    assert!(!old.path.exists());
    assert_eq!(fs::read_dir(new.path.parent().unwrap()).unwrap().count(), 1);
    assert_eq!(cache.usage(), Usage { total: 3, pinned: 0 });
}

#[test]
fn unreachable_returns_stale_copy() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", b"old".to_vec());
    server.add_file("/b.ARW", b"b".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let a = fetch(&cache, &client, &path(&client, "a.ARW")).unwrap();
    drop(server);

    let stale = fetch(&cache, &client, &path(&client, "a.ARW")).unwrap();
    assert_eq!(stale, Fetched { path: a.path, stale: true });
    assert_eq!(fs::read(&stale.path).unwrap(), b"old");
    assert!(matches!(fetch(&cache, &client, &path(&client, "b.ARW")), Err(Error::Unreachable(_))));
}

#[test]
fn maintenance_mode_returns_stale_copy() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", b"old".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let p = path(&client, "a.ARW");
    let a = fetch(&cache, &client, &p).unwrap();

    server.fail_next(Fault::Maintenance);
    assert_eq!(fetch(&cache, &client, &p).unwrap(), Fetched { path: a.path, stale: true });
    server.fail_next(Fault::Maintenance);
    assert!(matches!(fetch(&cache, &client, &path(&client, "b.ARW")), Err(Error::RateLimited)));
}

#[test]
fn eviction_respects_pins_and_holds() {
    let server = FakeServer::start();
    for (i, name) in ["/a.ARW", "/b.ARW", "/c.ARW", "/d.ARW"].into_iter().enumerate() {
        server.add_file(name, vec![i as u8; 100]);
    }
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 250).unwrap();
    let client = client(&server);
    let [a, b, c, d] = ["a.ARW", "b.ARW", "c.ARW", "d.ARW"].map(|n| path(&client, n));
    cache.pin(&a).unwrap();
    let _hold = cache.hold(&b);

    let [fa, fb, fc] = [&a, &b, &c].map(|p| fetch(&cache, &client, p).unwrap().path);
    // Over budget, but the only unprotected entry is the one just fetched.
    assert!(fc.exists());
    let fd = fetch(&cache, &client, &d).unwrap().path;
    assert!(fa.exists() && fb.exists() && fd.exists());
    assert!(!fc.exists());
    assert_eq!(cache.usage(), Usage { total: 300, pinned: 100 });
}

#[test]
fn corrupt_index_is_rebuilt() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", b"raw".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let client = client(&server);
    let p = path(&client, "a.ARW");
    let fetched = fetch(&OriginalsCache::open(dir.path(), 1 << 20).unwrap(), &client, &p).unwrap();

    fs::write(dir.path().join("index.json"), b"{not json").unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let etag = client.stat(&p).unwrap().etag;
    assert_eq!(cache.cached(&p, &etag), Some(fetched.path));
    assert_eq!(cache.usage(), Usage { total: 3, pinned: 0 });
}

#[test]
fn cancel_leaves_no_file() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", vec![7; 300_000]);
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let cancel = AtomicBool::new(false);

    let res = cache.fetch(&client, &path(&client, "a.ARW"), &cancel, &mut |_, _| cancel.store(true, Relaxed));
    assert!(matches!(res, Err(Error::Cancelled)), "{res:?}");
    assert_eq!(files(dir.path()), ["index.json"]);
    assert_eq!(cache.usage(), Usage { total: 0, pinned: 0 });
}

#[test]
fn open_sweeps_orphan_stage_files() {
    let dir = tempfile::tempdir().unwrap();
    let acct = dir.path().join("0123456789abcdef");
    fs::create_dir(&acct).unwrap();
    fs::write(acct.join(".x.123-0.craft_tmp"), b"partial").unwrap();
    fs::write(acct.join("notes.txt"), b"not ours").unwrap();
    fs::write(dir.path().join(".index.json.123-1.craft_tmp"), b"{").unwrap();

    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    assert_eq!(files(dir.path()), ["index.json", "notes.txt"]);
    assert_eq!(cache.usage(), Usage { total: 0, pinned: 0 });
}

#[test]
fn hostile_names_stay_inside_dir() {
    let server = FakeServer::start();
    server.add_file("/..evil.ARW", b"e".to_vec());
    server.add_file("/x.A R W", b"x".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);

    for (name, ext) in [("..evil.ARW", "arw"), ("x.A R W", "bin")] {
        let fetched = fetch(&cache, &client, &path(&client, name)).unwrap().path;
        assert_eq!(fetched.parent().unwrap().parent().unwrap(), dir.path());
        let file = fetched.file_name().unwrap().to_str().unwrap();
        let (stem, got) = file.split_once('.').unwrap();
        let (hash, etag) = stem.split_once('-').unwrap();
        assert_eq!((hash.len(), etag.len(), got), (64, 16, ext), "{file}");
        assert!(format!("{hash}{etag}").bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "{file}");
    }
}

#[test]
fn dropping_holds_releases_them() {
    let server = FakeServer::start();
    server.add_file("/b.ARW", vec![1; 100]);
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let b = path(&client, "b.ARW");
    let (first, second) = (cache.hold(&b), cache.hold(&b));
    let file = fetch(&cache, &client, &b).unwrap().path;

    drop(first);
    cache.set_budget(0);
    assert!(file.exists(), "still held by the second hold");
    drop(second);
    cache.set_budget(0);
    assert!(!file.exists());
    assert_eq!(cache.usage(), Usage { total: 0, pinned: 0 });
}

#[test]
fn hits_do_not_rewrite_the_index() {
    let server = FakeServer::start();
    server.add_file("/a.ARW", b"raw".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let cache = OriginalsCache::open(dir.path(), 1 << 20).unwrap();
    let client = client(&server);
    let p = path(&client, "a.ARW");
    fetch(&cache, &client, &p).unwrap();
    let index = dir.path().join("index.json");

    fs::write(&index, b"marker").unwrap();
    let etag = client.stat(&p).unwrap().etag;
    assert!(cache.cached(&p, &etag).is_some());
    assert!(!fetch(&cache, &client, &p).unwrap().stale);
    assert_eq!(fs::read(&index).unwrap(), b"marker");
    // The next write carries the bumped last use along.
    cache.pin(&p).unwrap();
    assert!(String::from_utf8(fs::read(&index).unwrap()).unwrap().contains("\"last_use\""));
}

#[test]
fn open_survives_unwritable_index_and_unreadable_account_dir() {
    let dir = tempfile::tempdir().unwrap();
    // A folder where the index file goes can't be replaced by a file.
    fs::create_dir(dir.path().join("index.json")).unwrap();
    let bad = dir.path().join("fedcba9876543210");
    fs::create_dir(&bad).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();
    }

    let opened = OriginalsCache::open(dir.path(), 1 << 20);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(opened.unwrap().usage(), Usage { total: 0, pinned: 0 });
}
