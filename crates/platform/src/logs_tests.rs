//! Behaviour tests of `logs`.
use super::*;

const NAMES: LogNames = LogNames::new("photocraft");

#[cfg(not(target_arch = "wasm32"))]
fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("craft-logs-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read log")
}

// Each start shifts app.log to app.1.log and app.2.log; while another instance holds the owner lock, a start writes app-<pid>.log and leaves the chain alone.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rotation_keeps_previous_logs_and_a_running_instance_log() {
    let dir = temp_dir("rotate");
    for run in 1..=4 {
        let LogFile { path, .. } = rotate(&dir, NAMES).expect("rotate");
        assert_eq!(path, dir.join("photocraft.log"));
        assert_eq!(read(&path), "");
        std::fs::write(&path, format!("run {run}")).expect("write");
    }
    assert_eq!(read(&dir.join("photocraft.log")), "run 4");
    assert_eq!(read(&dir.join("photocraft.1.log")), "run 3");
    assert_eq!(read(&dir.join("photocraft.2.log")), "run 2");
    assert!(!dir.join("photocraft.3.log").exists(), "only {KEEP} old logs are kept");

    // A start while the first instance runs leaves its log in place.
    let mut first = rotate(&dir, NAMES).expect("first start");
    first.file.write_all(b"before\n").expect("write");
    for _ in 0..=KEEP {
        let second = rotate(&dir, NAMES).expect("second start");
        assert_eq!(second.path, dir.join(format!("photocraft-{}.log", std::process::id())));
    }
    first.file.write_all(b"after\n").expect("write");
    assert_eq!(read(&dir.join("photocraft.log")), "before\nafter\n");
    assert_eq!(read(&dir.join("photocraft.1.log")), "run 4", "the chain did not move");

    // Once it has exited, the next start rotates again.
    drop(first);
    let third = rotate(&dir, NAMES).expect("third start");
    assert_eq!(third.path, dir.join("photocraft.log"));
    assert_eq!(read(&dir.join("photocraft.1.log")), "before\nafter\n");

    assert_eq!(NAMES.instance_pid("photocraft-123.log"), Some(123));
    assert_eq!(NAMES.instance_pid("photocraft-12a.log"), None);
    assert_eq!(NAMES.instance_pid("photocraft.1.log"), None);
    let _ = std::fs::remove_dir_all(&dir);

    // A file where the directory should be is an error.
    let blocked = temp_dir("blocked");
    std::fs::write(&blocked, "a file").expect("block");
    assert!(rotate(&blocked, NAMES).is_err());
    let _ = std::fs::remove_file(&blocked);
}
