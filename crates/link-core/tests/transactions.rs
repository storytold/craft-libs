use craft_link_core::*;
use serde_json::json;
use uuid::Uuid;

fn manifest() -> Manifest {
    serde_json::from_value(json!({"schema":1,"application":"test.app","version":"1","object_types":{"rectangle":["name","opacity"]},"representations":["native"],"read":true,"write":true,"history":"broker","transactions":"broker","lifecycle":"manual","authentication":"test","filesystem":"project","fidelity":"properties"})).unwrap()
}
fn change() -> Change {
    Change {
        object: Uuid::new_v4(),
        document: Uuid::new_v4(),
        application: "test.app".into(),
        kind: "rectangle".into(),
        expected: 0,
        value: json!({"name":"before","opacity":1.0}),
        native: None,
    }
}
fn commit(s: &mut Store, c: Change) -> Receipt {
    s.commit(Transaction { operation: Uuid::new_v4(), changes: vec![c] }).unwrap()
}

#[test]
fn durable_idempotent_commit_and_noop() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.register(manifest()).unwrap();
    let mut c = change();
    let blob = s.put_blob(b"opaque native file with unknown features").unwrap();
    c.native = Some(NativePayload { hash: blob.clone(), format: "test-native".into(), application: "test.app".into() });
    let tx = Transaction { operation: Uuid::new_v4(), changes: vec![c.clone()] };
    let r = s.commit(tx.clone()).unwrap();
    assert_eq!(s.commit(tx.clone()).unwrap(), r);
    let mut bad = tx;
    bad.changes[0].value = json!({"name":"different"});
    assert!(matches!(s.commit(bad), Err(Error::Conflict(_))));
    drop(s);
    let mut s = Store::open(d.path()).unwrap();
    assert_eq!(s.object(c.object).unwrap().revision, 1);
    assert_eq!(s.blob(&blob).unwrap(), b"opaque native file with unknown features");
    c.expected = 1;
    let noop = commit(&mut s, c);
    assert_eq!(noop.sequence, r.sequence);
    assert_eq!(s.events(0).unwrap().len(), 1);
    assert!(s.blob("../outside").is_err());
}
#[test]
fn multi_object_conflict_rolls_back_all_and_retains_intent() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.register(manifest()).unwrap();
    let mut a = change();
    let mut b = change();
    commit(&mut s, a.clone());
    commit(&mut s, b.clone());
    a.expected = 1;
    a.value = json!({"name":"attempt A"});
    b.expected = 0;
    b.value = json!({"name":"attempt B"});
    assert!(matches!(s.commit(Transaction { operation: Uuid::new_v4(), changes: vec![a.clone(), b.clone()] }), Err(Error::Conflict(_))));
    assert_eq!(s.object(a.object).unwrap().value["name"], "before");
    assert_eq!(s.object(b.object).unwrap().revision, 1);
    assert_eq!(s.conflicts().unwrap().len(), 1);
}
#[test]
fn graph_cycle_rejected_and_only_descendants_invalidated() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.register(manifest()).unwrap();
    let mut a = change();
    let b = change();
    let c = change();
    let unrelated = change();
    for x in [&a, &b, &c, &unrelated] {
        commit(&mut s, x.clone());
    }
    s.link(Uuid::new_v4(), a.object, b.object, "semantic").unwrap();
    s.link(Uuid::new_v4(), b.object, c.object, "source").unwrap();
    assert!(s.link(Uuid::new_v4(), c.object, a.object, "semantic").is_err());
    a.expected = 1;
    a.value = json!({"name":"after"});
    let r = commit(&mut s, a);
    assert_eq!(r.invalidated, [b.object, c.object].into());
    assert!(!r.invalidated.contains(&unrelated.object));
}
#[test]
fn undo_redo_are_monotonic_idempotent_and_guarded() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.register(manifest()).unwrap();
    let mut c = change();
    commit(&mut s, c.clone());
    c.expected = 1;
    c.value = json!({"name":"after"});
    commit(&mut s, c.clone());
    let op = Uuid::new_v4();
    let u = s.history(op, c.object, 2, false).unwrap();
    assert_eq!(u.revisions[&c.object], 3);
    assert_eq!(s.object(c.object).unwrap().value["name"], "before");
    assert_eq!(s.history(op, c.object, 2, false).unwrap(), u);
    assert!(s.history(Uuid::new_v4(), c.object, 2, true).is_err());
    let r = s.history(Uuid::new_v4(), c.object, 3, true).unwrap();
    assert_eq!(r.revisions[&c.object], 4);
    assert_eq!(s.object(c.object).unwrap().value["name"], "after");
}
#[test]
fn capabilities_and_missing_snapshots_fail_closed() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.register(manifest()).unwrap();
    let mut c = change();
    c.value = json!({"arbitraryNativeFeature":false});
    assert!(matches!(s.commit(Transaction { operation: Uuid::new_v4(), changes: vec![c.clone()] }), Err(Error::Capability(_))));
    c.value = json!({"name":"ok"});
    c.native = Some(NativePayload { hash: "0".repeat(64), format: "test-native".into(), application: "test.app".into() });
    assert!(matches!(s.commit(Transaction { operation: Uuid::new_v4(), changes: vec![c] }), Err(Error::Missing(_))));
    assert!(s.objects().unwrap().is_empty());
    let mut m = manifest();
    m.schema = 2;
    assert!(s.register(m).is_err());
}
#[test]
fn independent_writers_cannot_overwrite_concurrent_edits() {
    let d = tempfile::tempdir().unwrap();
    let mut a = Store::open(d.path()).unwrap();
    a.register(manifest()).unwrap();
    let mut c = change();
    commit(&mut a, c.clone());
    let mut b = Store::open(d.path()).unwrap();
    c.expected = 1;
    c.value = json!({"name":"winner"});
    commit(&mut a, c.clone());
    c.value = json!({"name":"loser"});
    assert!(matches!(b.commit(Transaction { operation: Uuid::new_v4(), changes: vec![c.clone()] }), Err(Error::Conflict(_))));
    assert_eq!(a.object(c.object).unwrap().value["name"], "winner");
}
#[test]
fn crash_uncommitted_sqlite_transaction_recovers() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.register(manifest()).unwrap();
    let c = change();
    commit(&mut s, c.clone());
    drop(s);
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_child"])
        .env("CRAFTLINK_CRASH_DB", d.path())
        .env("CRAFTLINK_CRASH_OBJECT", c.object.to_string())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        d.path().join("precommit-reached").is_file(),
        "child failed before reaching the crash injection point: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = Store::open(d.path()).unwrap();
    assert_eq!(s.object(c.object).unwrap().revision, 1);
    assert_eq!(s.events(0).unwrap().len(), 1);
}
#[test]
#[ignore]
fn crash_child() {
    let Some(dir) = std::env::var_os("CRAFTLINK_CRASH_DB") else {
        return;
    };
    let mut store = Store::open(&std::path::PathBuf::from(dir)).unwrap();
    let object = store.object(std::env::var("CRAFTLINK_CRASH_OBJECT").unwrap().parse().unwrap()).unwrap();
    let change = Change {
        object: object.object,
        document: object.document,
        application: object.application,
        kind: object.kind,
        expected: object.revision,
        value: json!({"name":"uncommitted intent","opacity":0.5}),
        native: None,
    };
    let _ = store.commit_with_precommit(Transaction { operation: Uuid::new_v4(), changes: vec![change] }, || {
        let marker =
            std::fs::File::create(std::path::PathBuf::from(std::env::var_os("CRAFTLINK_CRASH_DB").unwrap()).join("precommit-reached")).unwrap();
        marker.sync_all().unwrap();
        std::process::abort()
    });
}

#[test]
fn capability_revocation_while_waiting_for_writer_is_observed() {
    use std::sync::mpsc;
    use std::time::Duration;
    let d = tempfile::tempdir().unwrap();
    let mut store = Store::open(d.path()).unwrap();
    store.register(manifest()).unwrap();
    let connection = rusqlite::Connection::open(d.path().join("craftlink.sqlite")).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut revoked = manifest();
    revoked.write = false;
    connection
        .execute("UPDATE applications SET manifest=? WHERE id=?", rusqlite::params![serde_json::to_string(&revoked).unwrap(), revoked.application])
        .unwrap();
    let (ready, wait) = mpsc::channel();
    let (done, result) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        done.send(store.commit(Transaction { operation: Uuid::new_v4(), changes: vec![change()] })).unwrap();
    });
    wait.recv().unwrap();
    assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
    connection.execute_batch("COMMIT").unwrap();
    assert!(matches!(result.recv_timeout(Duration::from_secs(5)).unwrap(), Err(Error::Capability(_))));
    worker.join().unwrap();
    assert!(Store::open(d.path()).unwrap().objects().unwrap().is_empty());
}

#[test]
fn blob_reads_reject_oversize_and_nonregular_entries() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path()).unwrap();
    let hash = "0".repeat(64);
    let path = d.path().join("blobs").join(&hash);
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(128 * 1024 * 1024 + 1).unwrap();
    drop(file);
    assert!(matches!(store.blob(&hash), Err(Error::Invalid(_))));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(store.blob(&hash), Err(Error::Invalid(_))));
}

#[cfg(unix)]
#[test]
fn blob_reads_refuse_external_symlink() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = Store::open(d.path()).unwrap();
    let hash = store.put_blob(b"native").unwrap();
    let path = d.path().join("blobs").join(&hash);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(outside.path().join("private"), b"native").unwrap();
    std::os::unix::fs::symlink(outside.path().join("private"), path).unwrap();
    assert!(matches!(store.blob(&hash), Err(Error::Invalid(_))));
}

#[test]
fn simultaneous_first_open_serializes_schema_initialization() {
    let d = tempfile::tempdir().unwrap();
    let start = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = d.path().to_owned();
        let start = start.clone();
        threads.push(std::thread::spawn(move || {
            start.wait();
            Store::open(&path).map(|s| s.objects().unwrap())
        }));
    }
    start.wait();
    for thread in threads {
        assert!(thread.join().unwrap().unwrap().is_empty());
    }
}

#[test]
fn batched_invalidation_matches_union_for_shared_descendants() {
    let (a, b, c, d, e) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let edges = [(a, c), (b, c), (c, d), (d, e)];
    let union = affected(&edges, a).union(&affected(&edges, b)).copied().collect();
    assert_eq!(affected_many(&edges, [a, b]), union);
}
