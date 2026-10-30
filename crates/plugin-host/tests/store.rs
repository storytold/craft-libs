//! The same suite runs on both stores, so they cannot drift apart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use craft_plugin_host::store::*;
use serde_json::json;

fn plugin(id: &str, version: &str, bytes: &[u8]) -> StoredPlugin {
    StoredPlugin {
        id: id.into(),
        version: version.into(),
        bytes: bytes.to_vec(),
        enabled: true,
        installed_at_ms: 42,
        source: Source::DevFolder,
        grants: vec![StoredGrant {
            plugin_id: id.into(),
            publisher: Some("Ada".into()),
            permission: "commands.run:app.*".into(),
            granted_at_ms: 99,
            decided_by: Decider::User,
        }],
    }
}

fn packages<S: PackageStore>(s: &S) {
    assert!(s.load_all().unwrap().is_empty());
    let a = plugin("org.a", "1.0.0", &[0, 97, 115, 109, 255, 0]);
    let b = plugin("org.b", "2.0.0", b"bytes");
    s.save(&a).unwrap();
    s.save(&b).unwrap();
    assert_eq!(s.load_all().unwrap(), vec![a.clone(), b.clone()], "bytes, grants, source, and flags round trip, sorted by id");
    let a2 = StoredPlugin { version: "1.1.0".into(), enabled: false, bytes: vec![1, 2], grants: vec![], ..a };
    s.save(&a2).unwrap();
    assert_eq!(s.load_all().unwrap(), vec![a2, b.clone()], "save replaces by id");
    s.remove("org.a").unwrap();
    s.remove("org.never").unwrap();
    assert_eq!(s.load_all().unwrap(), vec![b]);
}

fn state<S: StateStore>(s: &S) {
    assert_eq!(s.get("p", "k").unwrap(), None);
    s.put("p", "k", &json!({"a": [1, 2]}), 100).unwrap();
    assert_eq!(s.get("p", "k").unwrap(), Some(json!({"a": [1, 2]})));
    assert_eq!(s.get("q", "k").unwrap(), None, "plugins do not share keys");
    // The cost of an entry is key bytes plus the JSON bytes of the value: "k" + `{"a":[1,2]}` = 12.
    s.put("p", "k", &json!({"a": [1, 2]}), 12).unwrap();
    assert!(matches!(s.put("p", "k", &json!({"a": [1, 22]}), 12), Err(StoreError::QuotaExceeded { needed: 13, quota: 12 })));
    assert_eq!(s.get("p", "k").unwrap(), Some(json!({"a": [1, 2]})), "a refused write changes nothing");
    assert!(matches!(s.put("p", "j", &json!(1), 12), Err(StoreError::QuotaExceeded { needed: 14, .. })));
    s.put("p", "j", &json!(1), 14).unwrap();
    s.clear("p").unwrap();
    s.clear("never").unwrap();
    assert_eq!(s.get("p", "k").unwrap(), None);
    assert_eq!(s.get("p", "j").unwrap(), None);
}

#[test]
fn memory_store_packages() {
    packages(&MemoryStore::new());
}

#[test]
fn memory_store_state() {
    state(&MemoryStore::new());
}

#[cfg(not(target_arch = "wasm32"))]
mod dir {
    use super::*;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("craft-plugin-host-store-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Tmp(p)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn dir_store_packages() {
        let t = Tmp::new();
        packages(&DirStore::new(&t.0));
    }

    #[test]
    fn dir_store_state() {
        let t = Tmp::new();
        state(&DirStore::new(&t.0));
    }

    #[test]
    fn dir_store_survives_a_restart_and_leaves_no_temp_files() {
        let t = Tmp::new();
        let a = plugin("org.a", "1.0.0", b"wasm");
        {
            let s = DirStore::new(&t.0);
            s.save(&a).unwrap();
            s.put("org.a", "k", &json!(7), 1000).unwrap();
        }
        let s = DirStore::new(&t.0);
        assert_eq!(s.load_all().unwrap(), vec![a]);
        assert_eq!(s.get("org.a", "k").unwrap(), Some(json!(7)));
        fn walk(p: &std::path::Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(p).unwrap() {
                let e = e.unwrap().path();
                if e.is_dir() {
                    walk(&e, out);
                } else {
                    out.push(e.strip_prefix(p.ancestors().last().unwrap()).unwrap().display().to_string());
                }
            }
        }
        let mut files = Vec::new();
        walk(&t.0, &mut files);
        assert!(files.iter().all(|f| !f.ends_with(".tmp")), "{files:?}");
        assert_eq!(files.len(), 3, "package, meta, state: {files:?}");
    }

    #[test]
    fn dir_store_refuses_ids_that_could_leave_its_directory() {
        let t = Tmp::new();
        let s = DirStore::new(&t.0);
        for id in ["..", ".", ".hidden", "a/b", "", "a b"] {
            let p = plugin(id, "1", b"x");
            assert!(matches!(s.save(&p), Err(StoreError::BadId(_))), "save {id:?}");
            assert!(matches!(s.put(id, "k", &json!(1), 100), Err(StoreError::BadId(_))), "put {id:?}");
            assert!(matches!(s.remove(id), Err(StoreError::BadId(_))), "remove {id:?}");
        }
        assert!(!t.0.exists() || std::fs::read_dir(&t.0).unwrap().next().is_none(), "nothing was written");
        let outside = t.0.parent().unwrap().join(format!("escape-{}", std::process::id()));
        assert!(!outside.exists());
    }

    #[test]
    fn dir_store_reports_corrupt_files_instead_of_guessing() {
        let t = Tmp::new();
        let s = DirStore::new(&t.0);
        s.save(&plugin("org.a", "1", b"x")).unwrap();
        s.put("org.a", "k", &json!(1), 100).unwrap();
        std::fs::write(t.0.join("state/org.a/state.json"), b"{not json").unwrap();
        assert!(matches!(s.get("org.a", "k"), Err(StoreError::Corrupt(_))));
        assert!(matches!(s.put("org.a", "k", &json!(1), 100), Err(StoreError::Corrupt(_))), "a write does not overwrite what it cannot read");
        std::fs::write(t.0.join("plugins/org.a/meta.json"), b"[]").unwrap();
        assert!(matches!(s.load_all(), Err(StoreError::Corrupt(_))));
        // A directory whose meta names another plugin is refused too.
        std::fs::write(
            t.0.join("plugins/org.a/meta.json"),
            br#"{"id":"org.other","version":"1","enabled":true,"installed_at_ms":0,"source":"file","grants":[]}"#,
        )
        .unwrap();
        assert!(matches!(s.load_all(), Err(StoreError::Corrupt(m)) if m.contains("org.other")));
    }
}
