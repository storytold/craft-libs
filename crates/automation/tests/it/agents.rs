//! Agents in the open window: the launcher's switch, the connection file, the link host and the
//! Agents page.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use craft_automation::agents::{
    AgentSwitch, CONNECTION_FILE, Connection, TOKEN_FILE, agents_allowed, read_connection, remove_connection, remove_stale_connection,
    write_connection,
};
use craft_host::Wake;
use craft_ui::runtime::motion;
use craft_ui::testing::trial;

fn temp_dir(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(&format!("craft-agents-{tag}-")).tempdir().expect("a temp dir")
}

/// Waits up to five seconds for `done`.
fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let started = motion::now();
    while !done() {
        if started.elapsed() > Duration::from_secs(5) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// With the `AI-MCP` flag only the launcher's switch decides; without it the app's earlier consent.
#[trial]
fn with_the_flag_only_the_launchers_switch_lets_agents_in() {
    assert!(agents_allowed(false, false, true), "without the flag, the app's earlier consent");
    assert!(!agents_allowed(false, true, false), "the launcher's list means nothing without the flag");
    assert!(agents_allowed(true, true, false));
    assert!(!agents_allowed(true, false, true), "no second switch in the app");
    assert!(AgentSwitch::fixed(true, true).allows(false));
    assert!(!AgentSwitch::fixed(false, true).allows(false));
}

/// The switch follows the launcher's list within a poll, wakes the app once per change rather than once per poll, and
/// its thread ends once the last clone is dropped.
#[trial]
fn the_switch_wakes_once_per_change() {
    let launcher = Arc::new(AtomicBool::new(false));
    let probes = Arc::new(AtomicUsize::new(0));
    let wakes = Arc::new(AtomicUsize::new(0));
    let probe = {
        let (launcher, probes) = (launcher.clone(), probes.clone());
        move || {
            probes.fetch_add(1, Ordering::Relaxed);
            launcher.load(Ordering::Relaxed)
        }
    };
    let wake = {
        let wakes = wakes.clone();
        Wake::new(move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        })
    };
    let switch = AgentSwitch::watch_with(true, probe, Duration::from_millis(5), wake);
    let clone = switch.clone();
    assert!(!switch.launcher());

    launcher.store(true, Ordering::Relaxed);
    assert!(wait_for(|| clone.launcher()), "the switch follows the launcher");
    assert!(switch.allows(false));
    // Many polls with nothing changed.
    let polled = probes.load(Ordering::Relaxed);
    assert!(wait_for(|| probes.load(Ordering::Relaxed) > polled + 10));
    launcher.store(false, Ordering::Relaxed);
    assert!(wait_for(|| !switch.launcher()));
    assert!(wait_for(|| wakes.load(Ordering::Relaxed) == 2));
    assert_eq!(wakes.load(Ordering::Relaxed), 2, "one wake per change, not per poll");

    drop(switch);
    drop(clone);
    // The thread sees the switch gone at its next poll and stops probing.
    std::thread::sleep(Duration::from_millis(50));
    let after_drop = probes.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(probes.load(Ordering::Relaxed), after_drop, "the watch ended with the last switch");
}

/// A published connection reads back in its documented shape, and only the process that wrote it removes it.
#[trial]
fn a_published_connection_reads_back_and_only_its_writer_removes_it() {
    let dir = temp_dir("conn");
    let dir = dir.path();
    assert_eq!(read_connection(dir).unwrap(), None, "no file");
    let c = Connection::new(7878, dir.join(TOKEN_FILE), "MyApp 1.2.3");
    write_connection(dir, &c).unwrap();
    assert_eq!(read_connection(dir).unwrap(), Some(c.clone()));
    assert_eq!(c.addr(), "127.0.0.1:7878");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(CONNECTION_FILE)).unwrap()).unwrap();
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["app", "pid", "port", "tokenFile", "version"]);
    assert_eq!((v["version"].as_u64(), v["port"].as_u64(), v["app"].as_str()), (Some(1), Some(7878), Some("MyApp 1.2.3")));

    remove_connection(dir, c.pid.wrapping_add(1));
    assert!(dir.join(CONNECTION_FILE).is_file(), "another process's file stays");
    remove_connection(dir, c.pid);
    assert!(!dir.join(CONNECTION_FILE).exists());
}

/// A damaged, foreign or port-0 file reads as none; a directory in its place is an error and is left alone; a file
/// nobody listens on is removed as stale and a live one kept.
#[trial]
fn a_damaged_foreign_or_stale_connection_file_counts_as_none() {
    let dir = temp_dir("bad-conn");
    let dir = dir.path();
    for text in [
        &b""[..],
        b"{not json",
        b"[]",
        br#"{"version":2,"pid":1,"port":7878,"tokenFile":"t","app":"x"}"#,
        br#"{"version":1,"pid":1,"port":0,"tokenFile":"t","app":"x"}"#,
        b"\xff\xfe not utf-8",
    ] {
        std::fs::write(dir.join(CONNECTION_FILE), text).unwrap();
        assert_eq!(read_connection(dir).unwrap(), None, "{}", String::from_utf8_lossy(text));
        remove_stale_connection(dir);
        assert!(!dir.join(CONNECTION_FILE).exists(), "an unreadable file is not a connection");
    }

    std::fs::create_dir(dir.join(CONNECTION_FILE)).unwrap();
    assert!(read_connection(dir).is_err());
    remove_stale_connection(dir);
    assert!(dir.join(CONNECTION_FILE).is_dir(), "anything but a regular file is left alone");
    std::fs::remove_dir(dir.join(CONNECTION_FILE)).unwrap();

    let live = TcpListener::bind("127.0.0.1:0").unwrap();
    let live_port = live.local_addr().unwrap().port();
    write_connection(dir, &Connection { pid: 1, ..Connection::new(live_port, dir.join(TOKEN_FILE), "x") }).unwrap();
    remove_stale_connection(dir);
    assert!(dir.join(CONNECTION_FILE).is_file(), "something listens on {live_port}");
    drop(live);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    write_connection(dir, &Connection { pid: 1, ..Connection::new(closed, dir.join(TOKEN_FILE), "x") }).unwrap();
    remove_stale_connection(dir);
    assert!(!dir.join(CONNECTION_FILE).exists(), "nothing listens on {closed}");
}
