use std::sync::Arc;

use craft_nextcloud::testing::{FakeServer, Fault};
use craft_nextcloud::{Client, Error};

fn client(server: &FakeServer) -> Client {
    Client::new(server.account(), FakeServer::PASSWORD.into(), Arc::new(rustls_rustcrypto::provider())).unwrap()
}

#[test]
fn list_returns_children() {
    let server = FakeServer::start();
    server.add_dir("/Photos");
    let id = server.add_file("/Photos/a.ARW", vec![0; 10]);
    server.add_dir("/Photos/sub");
    let client = client(&server);
    let mut entries = client.list(&client.account().root().join("Photos").unwrap()).unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries.len(), 2);
    let (a, sub) = (&entries[0], &entries[1]);
    assert_eq!(a.name, "a.ARW");
    assert_eq!(a.size, 10);
    assert!(!a.is_dir);
    assert!(!a.etag.is_empty() && !a.etag.contains('"'));
    assert_eq!(a.fileid, Some(id));
    assert!(a.mtime.is_some());
    assert!(sub.is_dir);
}

#[test]
fn list_round_trips_awkward_names() {
    let server = FakeServer::start();
    server.add_file("/x/a b%#?é.jpg", vec![1]);
    let client = client(&server);
    let dir = client.account().root().join("x").unwrap();
    let entries = client.list(&dir).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a b%#?é.jpg");
    assert_eq!(client.stat(&dir.join(&entries[0].name).unwrap()).unwrap().name, "a b%#?é.jpg");
}

#[test]
fn stat_root_and_missing() {
    let server = FakeServer::start();
    let client = client(&server);
    let root = client.account().root();
    let entry = client.stat(&root).unwrap();
    assert!(entry.is_dir);
    assert_eq!(entry.name, "");
    assert!(matches!(client.stat(&root.join("nope").unwrap()), Err(Error::NotFound)));
}

#[test]
fn hostile_names_are_skipped() {
    let server = FakeServer::start();
    server.add_file("/x/good.jpg", vec![1]);
    for name in ["..", "%2F", "a%00b", "%2E%2E"] {
        server.add_raw_entry("/x", name);
    }
    let client = client(&server);
    let entries = client.list(&client.account().root().join("x").unwrap()).unwrap();
    assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["good.jpg"]);
}

#[test]
fn rewritten_hrefs_are_malformed_not_empty() {
    let server = FakeServer::start();
    server.add_file("/x/a.jpg", vec![1]);
    let client = client(&server);
    let dir = client.account().root().join("x").unwrap();
    assert_eq!(client.list(&dir).unwrap().len(), 1);
    // Behind a proxy that rewrites paths no href is under the requested folder.
    server.set_href_prefix("/nextcloud");
    for dir in [dir, client.account().root()] {
        assert!(matches!(client.list(&dir), Err(Error::Malformed(_))), "{dir}");
    }
}

#[test]
fn other_account_is_rejected() {
    let server = FakeServer::start();
    let client = client(&server);
    let other = "nextcloud://bob@elsewhere/".parse::<craft_nextcloud::RemotePath>().unwrap();
    assert!(matches!(client.list(&other), Err(Error::Malformed(_))));
}

#[test]
fn oversized_listing_is_too_large() {
    let server = FakeServer::start();
    let client = client(&server);
    server.fail_next(Fault::Oversize);
    assert!(matches!(client.list(&client.account().root()), Err(Error::TooLarge)));
}

#[test]
fn malformed_listing() {
    let server = FakeServer::start();
    let client = client(&server);
    server.fail_next(Fault::Garbage);
    assert!(matches!(client.list(&client.account().root()), Err(Error::Malformed(_))));
}
