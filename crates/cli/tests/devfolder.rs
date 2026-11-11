//! Developer mode against the real plugin host: a folder of `.wasm` files, real file system
//! events, real reloads. The plugins are small WAT modules (no cargo), so the test is about the
//! watcher and the reload, not the toolchain.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use craft_cli::devfolder::{DevEvent, DevFolder};
use craft_cli::run::{DevHost, Headless};
use serde_json::{Value, json};

fn leb(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// A craft-abi 1 plugin whose command `ver` answers `{"result": <digit>}`.
fn plugin(id: &str, digit: char) -> Vec<u8> {
    let wat_text = r#"
(module
  (import "craft_host" "call" (func (param i32 i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1 16)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 64) "{\"result\":@}")
  (func (export "craft_abi") (result i32) i32.const 1)
  (func (export "craft_alloc") (param $len i32) (param $align i32) (result i32)
    (local $p i32)
    (if (i32.gt_u (i32.add (global.get $heap) (local.get $len)) (i32.const 60000)) (then (global.set $heap (i32.const 4096))))
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $p))
  (func (export "craft_free") (param i32 i32 i32))
  (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64)
    i64.const 51539607616))
"#
    .replace('@', &digit.to_string());
    let mut wasm = wat::parse_str(wat_text).unwrap();
    let manifest = json!({
        "schema": 1, "id": id, "name": id, "version": "1.0.0",
        "abi": { "craft": 1 },
        "contributions": [{ "kind": "command", "id": "ver", "entry": 0 }]
    });
    let data = serde_json::to_vec(&manifest).unwrap();
    let mut body = Vec::new();
    leb("craft.manifest".len(), &mut body);
    body.extend_from_slice(b"craft.manifest");
    body.extend_from_slice(&data);
    wasm.push(0);
    leb(body.len(), &mut wasm);
    wasm.extend_from_slice(&body);
    wasm
}

/// Write a file and push its modified time forward, so two writes in one test never share a stamp.
fn write_at(path: &Path, bytes: &[u8], bump_secs: u64) {
    std::fs::write(path, bytes).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(bump_secs)).unwrap();
}

fn folder(dir: &Path) -> (DevFolder<DevHost>, Arc<Headless>) {
    let host = Arc::new(Headless::new("devtest", false));
    (DevFolder::new(dir, DevHost { host: Arc::clone(&host), optional: Vec::new() }).unwrap(), host)
}

fn answer(host: &Headless, id: &str) -> Value {
    host.invoke(id, "ver", &json!({})).unwrap()
}

#[test]
fn a_changed_file_reloads_the_plugin_and_the_new_code_answers() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ver.wasm");
    let (mut dev, host) = folder(dir.path());

    write_at(&file, &plugin("dev.test.ver", '1'), 0);
    let events = dev.scan().unwrap();
    let [DevEvent::Loaded { installed, .. }] = events.as_slice() else { panic!("{events:?}") };
    assert_eq!((installed.generation, installed.replaced), (1, None));
    assert_eq!(answer(&host, "dev.test.ver"), json!(1));

    assert!(dev.scan().unwrap().is_empty(), "an unchanged file is not reloaded");

    write_at(&file, &plugin("dev.test.ver", '2'), 5);
    let events = dev.scan().unwrap();
    let [DevEvent::Loaded { installed, .. }] = events.as_slice() else { panic!("{events:?}") };
    assert_eq!((installed.generation, installed.replaced), (2, Some(1)), "the reload replaces generation 1");
    assert_eq!(answer(&host, "dev.test.ver"), json!(2), "calls now reach the new code");

    // The host reported both calls to the app (begin and end), the hook where a real app opens and closes its undo checkpoint.
    let events = host.services.events();
    let begins = events.iter().filter(|e| matches!(e, craft_cli::run::Event::CallBegin { contribution } if contribution == "ver")).count();
    let ends = events.iter().filter(|e| matches!(e, craft_cli::run::Event::CallEnd { outcome } if outcome == "completed")).count();
    assert_eq!((begins, ends), (2, 2));
}

#[test]
fn a_broken_file_is_rejected_and_the_running_version_keeps_answering() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ver.wasm");
    let (mut dev, host) = folder(dir.path());
    write_at(&file, &plugin("dev.test.ver", '1'), 0);
    dev.scan().unwrap();

    write_at(&file, b"\0asm half written", 5);
    let events = dev.scan().unwrap();
    let [DevEvent::Rejected { reason, .. }] = events.as_slice() else { panic!("{events:?}") };
    assert!(reason.contains("rejected") || reason.contains("malformed") || reason.contains("gate"), "{reason}");
    assert_eq!(answer(&host, "dev.test.ver"), json!(1), "the failed reload leaves version 1 running");
    assert!(dev.scan().unwrap().is_empty(), "the same broken bytes are not retried every scan");

    // The developer finishes the file.
    write_at(&file, &plugin("dev.test.ver", '3'), 10);
    let events = dev.scan().unwrap();
    let [DevEvent::Loaded { installed, .. }] = events.as_slice() else { panic!("{events:?}") };
    assert_eq!(installed.replaced, Some(1));
    assert_eq!(answer(&host, "dev.test.ver"), json!(3));
}

#[test]
fn deleting_the_file_removes_the_plugin_and_other_files_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ver.wasm");
    std::fs::write(dir.path().join("notes.txt"), "not a plugin").unwrap();
    let (mut dev, host) = folder(dir.path());
    write_at(&file, &plugin("dev.test.ver", '1'), 0);
    assert_eq!(dev.scan().unwrap().len(), 1, "only the .wasm file produced an event");

    std::fs::remove_file(&file).unwrap();
    let events = dev.scan().unwrap();
    assert!(matches!(events.as_slice(), [DevEvent::Removed { plugin_id, .. }] if plugin_id == "dev.test.ver"), "{events:?}");
    assert!(host.invoke("dev.test.ver", "ver", &json!({})).is_err(), "the plugin is gone");
    assert!(host.manager.list().is_empty());
}

#[test]
fn two_files_are_two_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let (mut dev, host) = folder(dir.path());
    write_at(&dir.path().join("a.wasm"), &plugin("dev.test.a", '4'), 0);
    write_at(&dir.path().join("b.wasm"), &plugin("dev.test.b", '5'), 0);
    assert_eq!(dev.scan().unwrap().len(), 2);
    assert_eq!(answer(&host, "dev.test.a"), json!(4));
    assert_eq!(answer(&host, "dev.test.b"), json!(5));
}

#[test]
fn the_folder_must_exist() {
    let host = Arc::new(Headless::new("devtest", false));
    assert!(DevFolder::new("/no/such/folder", DevHost { host, optional: Vec::new() }).is_err());
}

/// The file system events reach the watcher. The poll interval is 60 s, so a reload inside the
/// wait below can only come from a notify event.
#[test]
fn watch_reloads_on_file_system_events_not_just_on_the_poll() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ver.wasm");
    let (mut dev, host) = folder(dir.path());
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let stop_in_thread = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        dev.watch(&stop_in_thread, Duration::from_secs(60), |events| {
            let _ = tx.send(events);
        })
    });

    // The first scan finds nothing. Then the developer saves version 1, and later version 2.
    std::thread::sleep(Duration::from_millis(300));
    write_at(&file, &plugin("dev.test.ver", '1'), 0);
    let events = rx.recv_timeout(Duration::from_secs(10)).expect("no reload event after the first save");
    assert!(matches!(events.as_slice(), [DevEvent::Loaded { installed, .. }] if installed.generation == 1), "{events:?}");
    assert_eq!(answer(&host, "dev.test.ver"), json!(1));

    write_at(&file, &plugin("dev.test.ver", '2'), 5);
    let events = rx.recv_timeout(Duration::from_secs(10)).expect("no reload event after the second save");
    assert!(matches!(events.as_slice(), [DevEvent::Loaded { installed, .. }] if installed.replaced == Some(1)), "{events:?}");
    assert_eq!(answer(&host, "dev.test.ver"), json!(2));

    stop.store(true, Ordering::Relaxed);
    // `watch` checks the flag after each wake-up. A last write wakes it.
    write_at(&dir.path().join("wake.wasm"), b"x", 0);
    handle.join().unwrap().unwrap();
}
