//! A reviewed bundle held without a session, and installing it by the SHA-256 agreed to.

use crate::test_support::{bundle_bytes, bundle_for};
use crate::{AppInfo, Reviewed, Source, Target, sha256_hex};

const FIGCRAFT: AppInfo<'static> = AppInfo { id: "figcraft", version: "1.0.0", api: 1..=6, kinds: &["command"], accept_legacy: false, kind_api: &[] };

fn github() -> Source {
    Source::Github { repo: "example/tools".into(), tag: "v1.0.0".into(), asset: "tools.craftplugin".into() }
}

#[test]
fn a_reviewed_bundle_is_plain_data_a_worker_can_send() {
    fn sendable<T: Send + Sync + 'static>() {}
    sendable::<Reviewed>();
    let bytes = bundle_bytes("org.example.a", "1.0.0", "read");
    let sha = sha256_hex(&bytes);
    let reviewed = std::thread::spawn(move || Reviewed::new(&FIGCRAFT, bytes, github())).join().unwrap().unwrap();
    assert_eq!(reviewed.sha256, sha);
    assert_eq!(reviewed.manifest.id, "org.example.a");
    assert_eq!(reviewed.target, Target::Named { api: 6 });
    assert_eq!(reviewed.source, github());
    assert_eq!(reviewed.check_sha256(&sha), Ok(()));
    assert_eq!(reviewed.check_sha256(&sha.to_ascii_uppercase()), Ok(()));
}

#[test]
fn reviewing_refuses_what_installing_would() {
    let theirs = bundle_for("org.example.a", "1.0.0", "read", r#"{"photocraft": {"api": 1}}"#);
    assert!(Reviewed::new(&FIGCRAFT, theirs, github()).unwrap_err().contains("can't be installed"));
    assert!(Reviewed::new(&FIGCRAFT, b"not a bundle".to_vec(), github()).is_err());
    let bad = Source::Other { kind: "Bad Kind".into(), location: "x".into() };
    assert!(Reviewed::new(&FIGCRAFT, bundle_bytes("org.example.a", "1.0.0", "read"), bad).is_err());
}

#[test]
fn a_different_sha256_is_refused() {
    let reviewed = Reviewed::new(&FIGCRAFT, bundle_bytes("org.example.a", "1.0.0", "read"), github()).unwrap();
    let other = sha256_hex(b"something else");
    assert!(reviewed.check_sha256(&other).unwrap_err().contains("review it again"));
    assert!(reviewed.check_sha256("abc").is_err());
    // Bytes swapped after review no longer match their recorded SHA-256.
    let mut swapped = reviewed.clone();
    swapped.bytes = bundle_bytes("org.example.a", "2.0.0", "write");
    assert!(swapped.check_sha256(&reviewed.sha256).is_err());
    let new_sha = sha256_hex(&swapped.bytes);
    assert!(swapped.check_sha256(&new_sha).is_err(), "the recorded sha256 must match too");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_store_installs_a_reviewed_bundle_by_the_sha256_agreed_to() {
    use crate::store::{PLUGINS_DIR, Store};
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(PLUGINS_DIR));
    let reviewed = Reviewed::new(&FIGCRAFT, bundle_bytes("org.example.a", "1.0.0", "read"), github()).unwrap();
    let agreed = reviewed.sha256.clone();

    let e = store.install_reviewed_for(&FIGCRAFT, reviewed.clone(), &sha256_hex(b"other")).unwrap_err();
    assert!(e.contains("review it again"), "{e}");
    assert_eq!(store.load("org.example.a"), Ok(None), "nothing written on a mismatch");

    let (installed, target) = store.install_reviewed_for(&FIGCRAFT, reviewed.clone(), &agreed).unwrap();
    assert_eq!(installed.record.sha256, agreed);
    assert_eq!(installed.record.source, github());
    assert_eq!(target, Target::Named { api: 6 });
    assert_eq!(store.load_for(&FIGCRAFT, "org.example.a").unwrap().unwrap().0, installed);

    // The same as install_for with the bytes and source.
    let tmp2 = tempfile::tempdir().unwrap();
    let plain = Store::new(tmp2.path().join(PLUGINS_DIR));
    let (direct, direct_target) = plain.install_for(&FIGCRAFT, &reviewed.bytes, github()).unwrap();
    assert_eq!((direct.record, direct.manifest, direct_target), (installed.record, installed.manifest, target));

    // The target is checked again on install (an app reviewed for one AppInfo, installed for another).
    let other_app = AppInfo { id: "photocraft", ..FIGCRAFT };
    let r2 = Reviewed::new(&FIGCRAFT, bundle_bytes("org.example.b", "1.0.0", "read"), github()).unwrap();
    let sha = r2.sha256.clone();
    assert!(store.install_reviewed_for(&other_app, r2, &sha).is_err());
}
