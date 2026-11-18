//! AI agents in the open window: the launcher's switch, the connection file, and where an app's
//! `<id>-cli mcp` sends each tool call.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_control::agents::{
    AgentSettings, AgentSwitch, CONNECTION_FILE, Connection, Consent, HeadlessFolders, Live, LiveConfig, Route, TOKEN_FILE, WhenAppClosed, Window,
    agents_allowed, find_window, read_connection, remove_connection, remove_stale_connection, write_connection,
};

fn temp_dir(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(&format!("craft-agents-{tag}-")).tempdir().expect("a temp dir")
}

/// Waits up to five seconds for `done`.
fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while !done() {
        if started.elapsed() > Duration::from_secs(5) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Writes `text` as an owner-only token file, as an app's own token writer does.
fn write_token(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

/// An app accepting agents in `dir`: a listener on a free loopback port, its token file and its
/// connection file. Dropping the listener leaves the file behind, as a crashed app would.
fn publish(dir: &Path) -> (TcpListener, Connection) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    write_token(&dir.join(TOKEN_FILE), &format!("{}\n", TOKEN.to_ascii_uppercase()));
    let c = Connection::new(listener.local_addr().unwrap().port(), dir.join(TOKEN_FILE), "Test 1.0");
    write_connection(dir, &c).unwrap();
    (listener, c)
}

// ---- The launcher's switch ----------------------------------------------------------------------

/// With the `AI-MCP` flag only the launcher's switch decides; without it the app's earlier consent.
#[test]
fn with_the_flag_only_the_launchers_switch_lets_agents_in() {
    assert!(agents_allowed(false, false, true), "without the flag, the app's earlier consent");
    assert!(!agents_allowed(false, true, false), "the launcher's list means nothing without the flag");
    assert!(agents_allowed(true, true, false));
    assert!(!agents_allowed(true, false, true), "no second switch in the app");
    assert!(AgentSwitch::fixed(true, true).allows(false));
    assert!(!AgentSwitch::fixed(false, true).allows(false));
}

/// The switch follows the launcher's list within a poll, wakes the app once per change rather than
/// once per poll, and its thread ends once the last clone is dropped.
#[test]
fn the_switch_wakes_once_per_change() {
    let launcher = Arc::new(AtomicBool::new(false));
    let probes = Arc::new(AtomicUsize::new(0));
    let wakes = Arc::new(AtomicUsize::new(0));
    // Held by the watch thread through its probe: gone once the thread has ended.
    let alive = Arc::new(());
    let probe = {
        let (launcher, probes, alive) = (launcher.clone(), probes.clone(), alive.clone());
        move || {
            let _ = &alive;
            probes.fetch_add(1, Ordering::Relaxed);
            launcher.load(Ordering::Relaxed)
        }
    };
    let wake = {
        let wakes = wakes.clone();
        move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        }
    };
    let switch = AgentSwitch::watch_with(true, probe, Duration::from_millis(5), wake);
    let clone = switch.clone();
    assert!(!switch.launcher());

    launcher.store(true, Ordering::Relaxed);
    assert!(wait_for(|| clone.launcher()), "the switch follows the launcher");
    assert!(switch.allows(false));
    let polled = probes.load(Ordering::Relaxed);
    assert!(wait_for(|| probes.load(Ordering::Relaxed) > polled + 10));
    launcher.store(false, Ordering::Relaxed);
    assert!(wait_for(|| !switch.launcher()));
    assert!(wait_for(|| wakes.load(Ordering::Relaxed) == 2));
    assert_eq!(wakes.load(Ordering::Relaxed), 2, "one wake per change, not per poll");

    drop(switch);
    drop(clone);
    assert!(wait_for(|| Arc::strong_count(&alive) == 1), "the watch ended with the last switch");
    let after_end = probes.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(probes.load(Ordering::Relaxed), after_end, "nothing probes after the watch ended");
}

// ---- The connection file ------------------------------------------------------------------------

/// A published connection reads back in its documented shape, and only the process that wrote it
/// removes it.
#[test]
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

/// A damaged, foreign or port-0 file reads as none; a directory in its place is an error and is
/// left alone; a file nobody listens on is removed as stale and a live one kept.
#[test]
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

// ---- The CLI's side -----------------------------------------------------------------------------

/// What the stand-in app made: a window client (its window) or a headless session (its folders),
/// with counts of each.
#[derive(Default)]
struct Made {
    connects: AtomicUsize,
    sessions: AtomicUsize,
}

type TestLive = Live<Window, HeadlessFolders>;

fn config(dir: &Path, consent: Consent, made: &Arc<Made>) -> LiveConfig<Window, HeadlessFolders> {
    let (a, b) = (made.clone(), made.clone());
    let mut cfg = LiveConfig::new(
        "Test",
        dir.to_path_buf(),
        move |_: &Path| consent.clone(),
        move |w: &Window| {
            a.connects.fetch_add(1, Ordering::Relaxed);
            Ok(w.clone())
        },
        move |f: &HeadlessFolders| {
            b.sessions.fetch_add(1, Ordering::Relaxed);
            Ok(f.clone())
        },
    );
    cfg.poll = Duration::from_millis(5);
    cfg
}

fn settings(when: WhenAppClosed, read: &str, write: &str) -> AgentSettings {
    let mut s = AgentSettings::default();
    s.when_app_closed = when;
    s.read_root = read.into();
    s.write_root = write.into();
    s
}

fn allowed(when: WhenAppClosed) -> Consent {
    Consent::new(true, settings(when, "/app-in", "/app-out"))
}

fn headless_folders(route: Result<Route<Window, HeadlessFolders>, String>) -> HeadlessFolders {
    match route {
        Ok(Route::Headless(f)) => f,
        Ok(Route::Window(w)) => panic!("expected headless, got the window {w:?}"),
        Err(e) => panic!("expected headless, got the error {e}"),
    }
}

fn window(route: Result<Route<Window, HeadlessFolders>, String>) -> Window {
    match route {
        Ok(Route::Window(w)) => w,
        Ok(Route::Headless(f)) => panic!("expected the window, went headless with {f:?}"),
        Err(e) => panic!("expected the window, got the error {e}"),
    }
}

fn paths(p: &[&str]) -> Vec<PathBuf> {
    p.iter().map(PathBuf::from).collect()
}

/// An app accepting agents takes the call; the same port, token and process keep the client, a
/// restarted app (another process on the same port and token) gets a new one.
#[test]
fn an_open_app_takes_the_call_and_keeps_its_client_until_it_restarts() {
    let dir = temp_dir("window");
    let (_listener, c) = publish(dir.path());
    let made = Arc::new(Made::default());
    // Even without consent: an app publishes its connection only while it accepts agents.
    let live = TestLive::new(config(dir.path(), Consent::default(), &made));
    let w = window(live.resolve());
    assert_eq!(
        (w.port, w.pid, w.token.as_str(), w.addr()),
        (c.port, c.pid, TOKEN.to_ascii_uppercase().as_str(), c.addr()),
        "the token is trimmed, otherwise as written"
    );
    window(live.resolve());
    assert_eq!(made.connects.load(Ordering::Relaxed), 1, "the same window keeps its client");
    assert!(matches!(live.current(), Some(Route::Window(_))));

    write_connection(dir.path(), &Connection { pid: c.pid.wrapping_add(1), ..c.clone() }).unwrap();
    assert_eq!(window(live.resolve()).pid, c.pid.wrapping_add(1));
    assert_eq!(made.connects.load(Ordering::Relaxed), 2, "a restarted app gets a new client");
    assert_eq!(made.sessions.load(Ordering::Relaxed), 0);
}

/// No app accepts agents and the person doesn't let them use it: headless with only the command's
/// folders, never the app's.
#[test]
fn without_consent_a_closed_app_runs_headless_with_the_commands_folders_only() {
    let dir = temp_dir("no-consent");
    let made = Arc::new(Made::default());
    let consent = Consent::new(false, settings(WhenAppClosed::Launch, "/app-in", "/app-out"));
    let live = TestLive::new(config(dir.path(), consent.clone(), &made));
    assert_eq!(headless_folders(live.resolve()), HeadlessFolders::default(), "no folders given, none used");

    let mut cfg = config(dir.path(), consent, &made);
    cfg.read_roots = paths(&["/cli-in"]);
    cfg.write_root = Some(PathBuf::from("/cli-out"));
    let f = headless_folders(TestLive::new(cfg).resolve());
    assert_eq!((f.read, f.write), (paths(&["/cli-in"]), Some(PathBuf::from("/cli-out"))));
}

/// Agents allowed, app closed, "headless": the app's agent folders, each replaced by the command's.
#[test]
fn allowed_and_closed_runs_headless_with_the_apps_folders_unless_the_command_gives_its_own() {
    let dir = temp_dir("headless");
    let made = Arc::new(Made::default());
    let f = headless_folders(TestLive::new(config(dir.path(), allowed(WhenAppClosed::Headless), &made)).resolve());
    assert_eq!((f.read, f.write), (paths(&["/app-in"]), Some(PathBuf::from("/app-out"))));

    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Headless), &made);
    cfg.read_roots = paths(&["/cli-a", "/cli-b"]);
    let f = headless_folders(TestLive::new(cfg).resolve());
    assert_eq!((f.read, f.write), (paths(&["/cli-a", "/cli-b"]), Some(PathBuf::from("/app-out"))));

    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Headless), &made);
    cfg.write_root = Some(PathBuf::from("/cli-out"));
    let f = headless_folders(TestLive::new(cfg).resolve());
    assert_eq!((f.read, f.write), (paths(&["/app-in"]), Some(PathBuf::from("/cli-out"))));
}

/// Agents allowed, app closed, "error": the call fails with words that name the app; the command's
/// choice wins over the app's.
#[test]
fn the_closed_app_choice_decides_and_the_command_overrides_it() {
    let dir = temp_dir("error");
    let made = Arc::new(Made::default());
    let err = TestLive::new(config(dir.path(), allowed(WhenAppClosed::Error), &made)).resolve().err().unwrap();
    assert_eq!(err, "Test isn't running: open it, then try again.");

    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Error), &made);
    cfg.when_closed = Some(WhenAppClosed::Headless);
    headless_folders(TestLive::new(cfg).resolve());
    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Headless), &made);
    cfg.when_closed = Some(WhenAppClosed::Error);
    assert!(TestLive::new(cfg).resolve().is_err());
}

/// Once headless, the session stays headless even when the app starts, so documents don't move
/// under the agent.
#[test]
fn a_headless_session_stays_headless() {
    let dir = temp_dir("sticky");
    let made = Arc::new(Made::default());
    let live = TestLive::new(config(dir.path(), Consent::default(), &made));
    headless_folders(live.resolve());
    let _app = publish(dir.path());
    headless_folders(live.resolve());
    assert_eq!((made.sessions.load(Ordering::Relaxed), made.connects.load(Ordering::Relaxed)), (1, 0), "one session, no window");
}

/// A connection file left by an app that crashed counts as a closed app.
#[test]
fn a_crashed_apps_connection_file_counts_as_closed() {
    let dir = temp_dir("crashed");
    drop(publish(dir.path()).0);
    let made = Arc::new(Made::default());
    headless_folders(TestLive::new(config(dir.path(), Consent::default(), &made)).resolve());
    assert_eq!(find_window(dir.path(), "Test").unwrap(), None);
}

/// A token file that isn't a token, is too big, is missing, isn't a regular file or (on Unix)
/// others can read fails the call and names the app; nothing is sent to the window.
#[test]
fn a_bad_token_file_fails_the_call() {
    let dir = temp_dir("token");
    let (_listener, _c) = publish(dir.path());
    let made = Arc::new(Made::default());
    for bad in ["", "  \n", "not a token", &"a".repeat(5000)] {
        write_token(&dir.path().join(TOKEN_FILE), bad);
        let err = TestLive::new(config(dir.path(), Consent::default(), &made)).resolve().err().unwrap();
        assert!(err.starts_with("Test's agent token: "), "{err}");
    }
    // Any token the app's server accepts is passed on: upper case, dashes.
    write_token(&dir.path().join(TOKEN_FILE), "AB12-cd34-EF56-0000-1111-2222-3333-4444");
    assert_eq!(find_window(dir.path(), "Test").unwrap().unwrap().token, "AB12-cd34-EF56-0000-1111-2222-3333-4444");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let token = dir.path().join(TOKEN_FILE);
        write_token(&token, TOKEN);
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = find_window(dir.path(), "Test").err().unwrap();
        assert!(err.contains("readable by group or others"), "{err}");
        // A FIFO would block a plain open until something writes to it.
        std::fs::remove_file(&token).unwrap();
        assert!(std::process::Command::new("mkfifo").arg(&token).status().unwrap().success());
        let err = find_window(dir.path(), "Test").err().unwrap();
        assert!(err.contains("not a regular file"), "{err}");
    }
    std::fs::remove_file(dir.path().join(TOKEN_FILE)).unwrap();
    assert!(find_window(dir.path(), "Test").is_err());
    assert_eq!(made.connects.load(Ordering::Relaxed), 0);
}

/// The app's own errors reach the agent with what was being done.
#[test]
fn the_apps_errors_reach_the_agent() {
    let dir = temp_dir("app-errors");
    let consent = Consent::default();
    let cfg: LiveConfig<Window, HeadlessFolders> = LiveConfig::new(
        "Test",
        dir.path().to_path_buf(),
        move |_: &Path| consent.clone(),
        |w: &Window| Ok(w.clone()),
        |_: &HeadlessFolders| Err("no such folder".into()),
    );
    assert_eq!(TestLive::new(cfg).resolve().err().unwrap(), "Test can't use the automation roots: no such folder");

    let (_listener, _c) = publish(dir.path());
    let cfg: LiveConfig<Window, HeadlessFolders> = LiveConfig::new(
        "Test",
        dir.path().to_path_buf(),
        |_: &Path| Consent::default(),
        |_: &Window| Err("refused".into()),
        |f: &HeadlessFolders| Ok(f.clone()),
    );
    let live = TestLive::new(cfg);
    assert_eq!(live.resolve().err().unwrap(), "couldn't reach Test's window: refused");
    assert!(live.current().is_none(), "a failed connect leaves no route");
}

/// "Launch" without a way to start the app, a start that fails, and an app that never accepts
/// agents each end with words that say what to do.
#[test]
fn a_launch_that_cannot_happen_or_never_finishes_says_what_to_do() {
    let dir = temp_dir("launch-fails");
    let made = Arc::new(Made::default());
    let err = TestLive::new(config(dir.path(), allowed(WhenAppClosed::Launch), &made)).resolve().err().unwrap();
    assert_eq!(err, "Test isn't running, and this install can't start it from here: open Test, then try again.");

    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Launch), &made);
    cfg.launch = Some(Box::new(|| Err("boom".into())));
    assert_eq!(TestLive::new(cfg).resolve().err().unwrap(), "couldn't start Test: boom");

    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Launch), &made);
    cfg.launch = Some(Box::new(|| Ok(())));
    cfg.launch_timeout = Duration::from_millis(40);
    let err = TestLive::new(cfg).resolve().err().unwrap();
    assert!(err.starts_with("Test didn't accept agents within 0 s of starting."), "{err}");
}

/// "Launch" starts the app once, also for calls that arrive together, and the calls go to its
/// window once it accepts agents.
#[test]
fn a_started_app_takes_the_call_and_concurrent_calls_start_it_once() {
    let dir = temp_dir("launch");
    let made = Arc::new(Made::default());
    let launches = Arc::new(AtomicUsize::new(0));
    let app: Arc<Mutex<Option<TcpListener>>> = Arc::new(Mutex::new(None));
    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Launch), &made);
    let (d, n, slot) = (dir.path().to_path_buf(), launches.clone(), app.clone());
    cfg.launch = Some(Box::new(move || {
        n.fetch_add(1, Ordering::Relaxed);
        let (d, slot) = (d.clone(), slot.clone());
        // The app takes a moment before it accepts agents.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            *slot.lock().unwrap() = Some(publish(&d).0);
        });
        Ok(())
    }));
    let live = Arc::new(TestLive::new(cfg));
    let calls: Vec<_> = (0..4)
        .map(|_| {
            let live = live.clone();
            std::thread::spawn(move || window(live.resolve()))
        })
        .collect();
    let ports: Vec<u16> = calls.into_iter().map(|c| c.join().unwrap().port).collect();
    assert_eq!(launches.load(Ordering::Relaxed), 1, "one start for every call");
    assert!(ports.windows(2).all(|p| p[0] == p[1]), "{ports:?}");
    assert_eq!(made.connects.load(Ordering::Relaxed), 1);
}

/// Calls that queued while a start was under way get its error when the app still isn't there,
/// rather than starting it again; a call made afterwards tries again.
#[test]
fn calls_queued_behind_a_failed_start_share_its_error() {
    let dir = temp_dir("launch-shared");
    let made = Arc::new(Made::default());
    let launches = Arc::new(AtomicUsize::new(0));
    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Launch), &made);
    let entered = Arc::new(AtomicUsize::new(0));
    let (n, e) = (launches.clone(), entered.clone());
    // The app is started but never accepts agents within the timeout. The start waits until every
    // call is under way and then a while longer, so a call that hasn't reached its lookup by the
    // time the start ends is very unlikely (there is no hook between the two).
    cfg.launch = Some(Box::new(move || {
        n.fetch_add(1, Ordering::Relaxed);
        assert!(wait_for(|| e.load(Ordering::Relaxed) == 4), "every call starts");
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    }));
    cfg.launch_timeout = Duration::from_millis(150);
    let live = Arc::new(TestLive::new(cfg));
    let calls: Vec<_> = (0..4)
        .map(|_| {
            let (live, entered) = (live.clone(), entered.clone());
            std::thread::spawn(move || {
                entered.fetch_add(1, Ordering::Relaxed);
                live.resolve()
            })
        })
        .collect();
    let answers: Vec<_> = calls.into_iter().map(|c| c.join().unwrap()).collect();
    assert_eq!(launches.load(Ordering::Relaxed), 1, "one start for the calls that came together");
    for answer in &answers {
        let err = answer.as_ref().err().unwrap();
        assert!(err.starts_with("Test didn't accept agents within"), "{err}");
    }
    assert!(live.resolve().is_err());
    assert_eq!(launches.load(Ordering::Relaxed), 2, "a later call starts the app again");
}

/// Allow Agent Control off refuses agents on the CLI's side too, as the app does by not
/// listening: no start, and only the command's folders.
#[test]
fn allow_agent_control_off_counts_as_not_allowed() {
    let dir = temp_dir("aac-off");
    let made = Arc::new(Made::default());
    let mut s = settings(WhenAppClosed::Launch, "/app-in", "/app-out");
    s.allow_agent_control = false;
    let mut cfg = config(dir.path(), Consent::new(true, s), &made);
    cfg.launch = Some(Box::new(|| panic!("nothing may start the app")));
    cfg.read_roots = paths(&["/cli-in"]);
    let f = headless_folders(TestLive::new(cfg).resolve());
    assert_eq!((f.read, f.write), (paths(&["/cli-in"]), None));
}

/// A window route is dropped once no app accepts agents, so a dead client isn't kept or handed
/// back.
#[test]
fn a_window_that_went_away_is_forgotten() {
    let dir = temp_dir("gone");
    let made = Arc::new(Made::default());
    let live = TestLive::new(config(dir.path(), allowed(WhenAppClosed::Error), &made));
    let (listener, c) = publish(dir.path());
    window(live.resolve());
    drop(listener);
    remove_connection(dir.path(), c.pid);
    assert!(live.resolve().is_err(), "the app is closed and the choice is an error");
    assert!(live.current().is_none(), "the dead window is gone");
}

/// A headless session with the app's agent folders ends when the person stops letting agents in;
/// one with only the command's folders stays.
#[test]
fn a_headless_session_with_the_apps_folders_ends_with_consent() {
    let dir = temp_dir("revoke");
    let made = Arc::new(Made::default());
    let on = Arc::new(AtomicBool::new(true));
    let gate = on.clone();
    let (b, c) = (made.clone(), made.clone());
    let cfg: LiveConfig<Window, HeadlessFolders> = LiveConfig::new(
        "Test",
        dir.path().to_path_buf(),
        move |_: &Path| Consent::new(gate.load(Ordering::Relaxed), settings(WhenAppClosed::Headless, "/app-in", "/app-out")),
        move |w: &Window| {
            b.connects.fetch_add(1, Ordering::Relaxed);
            Ok(w.clone())
        },
        move |f: &HeadlessFolders| {
            c.sessions.fetch_add(1, Ordering::Relaxed);
            Ok(f.clone())
        },
    );
    let live = TestLive::new(cfg);
    assert_eq!(headless_folders(live.resolve()).read, paths(&["/app-in"]));
    on.store(false, Ordering::Relaxed);
    assert_eq!(headless_folders(live.resolve()), HeadlessFolders::default(), "the app's folders went with the consent");
    on.store(true, Ordering::Relaxed);
    assert_eq!(headless_folders(live.resolve()), HeadlessFolders::default(), "a session without the app's folders stays");
    assert_eq!(made.sessions.load(Ordering::Relaxed), 2);
}

/// A huge launch timeout or poll is cut down rather than overflowing the deadline or blocking.
#[test]
fn a_huge_launch_timeout_or_poll_neither_panics_nor_hangs() {
    let dir = temp_dir("huge");
    let made = Arc::new(Made::default());
    let mut cfg = config(dir.path(), allowed(WhenAppClosed::Launch), &made);
    let (d, app) = (dir.path().to_path_buf(), Arc::new(Mutex::new(None)));
    let slot = app.clone();
    cfg.launch = Some(Box::new(move || {
        *slot.lock().unwrap() = Some(publish(&d).0);
        Ok(())
    }));
    cfg.launch_timeout = Duration::MAX;
    cfg.poll = Duration::MAX;
    let started = Instant::now();
    window(TestLive::new(cfg).resolve());
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}
