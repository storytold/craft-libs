//! The per-user plug-in store.

use crate::sha256_hex;
use crate::store::{PLUGINS_DIR, SOURCE_FILE, Source, Store, folder_name, plugins_dir};
use crate::test_support::bundle_bytes;

fn github() -> Source {
    Source::Github { repo: "example/tools".into(), tag: "v1.0.0".into(), asset: "tools.craftplugin".into() }
}

#[test]
fn folders_are_hashes_of_the_id() {
    assert_eq!(folder_name("Org.Example.X"), folder_name("org.example.x"));
    assert_ne!(folder_name("org.example.x"), folder_name("org.example.y"));
    for id in ["../../etc", "a/b", "C:\\x", ""] {
        let name = folder_name(id);
        assert!(name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit()), "{id}: {name}");
    }
    assert_eq!(plugins_dir(std::path::Path::new("/s")), std::path::Path::new("/s").join(PLUGINS_DIR));
}

#[test]
fn install_load_list_update_remove() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(PLUGINS_DIR));
    assert_eq!(store.load("org.example.a"), Ok(None));
    assert_eq!(store.list(), (vec![], vec![]));

    let v1 = bundle_bytes("org.example.a", "1.0.0", "read");
    let installed = store.install(&v1, github()).unwrap();
    assert_eq!(installed.dir, store.dir_of("org.example.a"));
    assert_eq!((installed.record.version.as_str(), installed.record.sha256.clone()), ("1.0.0", sha256_hex(&v1)));
    let loaded = store.load("ORG.EXAMPLE.A").unwrap().unwrap();
    assert_eq!(loaded, installed);
    assert_eq!(loaded.bundle.assets[0].name, "icon.png");

    let v2 = bundle_bytes("org.example.a", "1.1.0", "write");
    store.install(&v2, Source::File { path: "a.craftplugin".into() }).unwrap();
    let loaded = store.load("org.example.a").unwrap().unwrap();
    assert_eq!((loaded.manifest.version.as_str(), &loaded.record.source), ("1.1.0", &Source::File { path: "a.craftplugin".into() }));
    let files: Vec<String> = std::fs::read_dir(&loaded.dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(files.len(), 2, "the old bundle is gone: {files:?}");

    store
        .install(&bundle_bytes("org.example.b", "2.0.0", "read"), Source::Catalog { catalog: "https://x".into(), package: "org.example.b".into() })
        .unwrap();
    let (all, problems) = store.list();
    assert!(problems.is_empty(), "{problems:?}");
    let mut ids: Vec<&str> = all.iter().map(|i| i.manifest.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["org.example.a", "org.example.b"]);

    assert_eq!(store.remove("org.example.a"), Ok(true));
    assert_eq!(store.remove("org.example.a"), Ok(false));
    assert_eq!(store.load("org.example.a"), Ok(None));
    assert_eq!(store.list().0.len(), 1);
}

#[test]
fn nothing_is_written_for_a_bad_bundle_or_source() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(PLUGINS_DIR));
    assert!(store.install(b"FCPLUGB1 junk", github()).is_err());
    let mut cut = bundle_bytes("org.example.a", "1.0.0", "read");
    cut.truncate(cut.len() - 1);
    assert!(store.install(&cut, github()).is_err());
    let bad_source = Source::File { path: "a\nb".into() };
    assert!(store.install(&bundle_bytes("org.example.a", "1.0.0", "read"), bad_source).is_err());
    assert!(!store.root().exists(), "nothing written");
}

#[test]
fn damaged_folders_are_reported_not_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path());
    let a = store.install(&bundle_bytes("org.example.a", "1.0.0", "read"), github()).unwrap();
    let b = store.install(&bundle_bytes("org.example.b", "1.0.0", "read"), github()).unwrap();
    let c = store.install(&bundle_bytes("org.example.c", "1.0.0", "read"), github()).unwrap();
    // A bundle changed after it was installed.
    let file = a.dir.join(format!("{}.craftplugin", a.record.sha256));
    let mut bytes = std::fs::read(&file).unwrap();
    if let Some(last) = bytes.last_mut() {
        *last ^= 1;
    }
    std::fs::write(&file, bytes).unwrap();
    // A record moved into another plug-in's folder.
    std::fs::copy(c.dir.join(SOURCE_FILE), b.dir.join(SOURCE_FILE)).unwrap();
    // A record that isn't JSON, and one naming a path instead of a hash.
    std::fs::write(c.dir.join(SOURCE_FILE), "{").unwrap();
    let d = store.install(&bundle_bytes("org.example.d", "1.0.0", "read"), github()).unwrap();
    let record = std::fs::read_to_string(d.dir.join(SOURCE_FILE)).unwrap().replace(&d.record.sha256, "../../../../etc/passwd");
    std::fs::write(d.dir.join(SOURCE_FILE), record).unwrap();
    // Folders that aren't the store's are ignored.
    std::fs::create_dir_all(tmp.path().join("not-a-plugin")).unwrap();

    let (ok, problems) = store.list();
    assert!(ok.is_empty(), "{ok:?}");
    assert_eq!(problems.len(), 4, "{problems:?}");
    assert!(store.load("org.example.a").unwrap_err().contains("changed"));
    assert!(store.load("org.example.b").is_err());
}
