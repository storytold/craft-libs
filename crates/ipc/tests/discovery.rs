//! A record held by a child dies with the child.
//!
//! `hold_until_killed` returns at once unless `CRAFT_HOLD_DIR` is set.
//! The parent test spawns that child, checks the live record, sends SIGKILL,
//! and checks that `Discovery::scan` removed it. Drop does not run on SIGKILL.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use craft_ipc::{Discovery, InstanceRecord, RuntimeDir};

#[test]
fn hold_until_killed() {
    let Ok(dir) = std::env::var("CRAFT_HOLD_DIR") else {
        return;
    };
    let runtime = RuntimeDir::at(&dir).unwrap();
    let app = "effectcraft";
    let pid = std::process::id();
    let endpoint = runtime.unix_endpoint(app, pid).unwrap();
    let record = InstanceRecord::new(app, "0.8.3", pid, endpoint).unwrap();
    let held = Discovery::publish(&runtime, record).unwrap();
    std::fs::write(runtime.path().join("ready"), b"ready").unwrap();
    let _held = held;
    // park can wake without an unpark. Loop so Drop does not run until SIGKILL.
    loop {
        std::thread::park();
    }
}

#[test]
fn sigkill_drops_the_record() {
    let path = hold_dir();
    let _ = std::fs::remove_dir_all(&path);
    let runtime = RuntimeDir::at(&path).unwrap();
    let exe = std::env::current_exe().unwrap();
    let child = Command::new(exe).args(["--exact", "hold_until_killed", "--test-threads=1"]).env("CRAFT_HOLD_DIR", &path).spawn().unwrap();
    let mut guard = HoldCleanup { dir: path.clone(), child: Some(child) };
    let ready = runtime.path().join("ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if ready.is_file() {
            break;
        }
        let child = guard.child.as_mut().unwrap();
        if let Some(status) = child.try_wait().unwrap() {
            panic!("child exited before ready: {status}");
        }
        if Instant::now() >= deadline {
            panic!("child did not write ready");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let child_pid = guard.child.as_ref().unwrap().id();
    let live = Discovery::scan(&runtime).unwrap();
    assert_eq!(live.len(), 1, "expected one live record");
    assert_eq!(live[0].pid, child_pid);
    assert_eq!(live[0].app, "effectcraft");
    let mut child = guard.child.take().unwrap();
    child.kill().unwrap();
    let status = child.wait().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "SIGKILL so the child's Drop does not unlock");
    }
    #[cfg(not(unix))]
    {
        let _ = status;
    }
    let live = Discovery::scan(&runtime).unwrap();
    assert!(live.is_empty(), "scan should drop the record after SIGKILL, got {live:?}");
    assert!(!runtime.record_file("effectcraft", child_pid).exists(), "record json should be gone");
}

struct HoldCleanup {
    dir: PathBuf,
    child: Option<Child>,
}

impl Drop for HoldCleanup {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn hold_dir() -> PathBuf {
    let mut bytes = [0u8; 4];
    let mut file = std::fs::File::open("/dev/urandom").unwrap();
    std::io::Read::read_exact(&mut file, &mut bytes).unwrap();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    PathBuf::from(format!("/tmp/cl-hold-{hex}"))
}
