//! Agents in the open window: the launcher's switch, the connection file, the link host and the
//! Agents page.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use craft_automation::agents::AgentLinkStatus;
use craft_automation::agents::{
    AgentLink, AgentSettings, AgentSwitch, CONNECTION_FILE, Connection, LinkProblem, PROBE_TIMEOUT, TOKEN_FILE, agents_allowed, probe,
    read_connection, remove_connection, remove_stale_connection, write_connection,
};
use craft_automation::server;
use craft_automation::ui::agents::AgentsPage;
use craft_host::{AppInfo, Host, PickId, Picked, ScriptedFiles};
use craft_ui::contract::Role;
use craft_ui::patterns::preferences::SettingsDraft;
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::Fixture;
use craft_ui::testing::trial;
use egui::Context;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

fn host() -> WindowHost {
    WindowHost::of(&Context::default())
}

fn on(port: u32) -> AgentSettings {
    let mut s = AgentSettings::default();
    s.control_port = port;
    s
}

/// A port nothing listens on: bound, then closed.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Off publishes nothing; on publishes the bound port and this process, and something answers there; a folder-only
/// change keeps the port and gives no new channel; off removes the file and frees the port.
#[trial]
fn turning_agents_on_publishes_a_reachable_connection_and_off_removes_it() {
    let dir = temp_dir("link");
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "MyApp 1.0");
    let off = link.sync(None, &host);
    assert!(off.requests.is_none() && !off.stopped && !off.changed);
    assert_eq!(read_connection(dir.path()).unwrap(), None);

    let started = link.sync(Some(&on(0)), &host);
    assert!(started.requests.is_some() && started.changed);
    let port = link.status().port.expect("listening");
    let c = read_connection(dir.path()).unwrap().expect("published");
    assert_eq!((c.port, c.pid, c.app.as_str()), (port, std::process::id(), "MyApp 1.0"));
    assert!(probe(port, PROBE_TIMEOUT));
    let token = std::fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap();

    let mut folders = on(0);
    folders.read_root = "/in".into();
    let kept = link.sync(Some(&folders), &host);
    assert!(kept.requests.is_none() && !kept.stopped && kept.changed, "a folder change keeps the server");
    assert_eq!(link.status().port, Some(port));
    assert_eq!(std::fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap(), token);

    let stopped = link.sync(None, &host);
    assert!(stopped.stopped && stopped.changed);
    assert_eq!(link.status().port, None);
    assert!(!dir.path().join(CONNECTION_FILE).exists());
    TcpListener::bind(("127.0.0.1", port)).expect("the port is free");
}

/// A new port restarts the server with the same token; a taken port is named, publishes nothing, and is tried again
/// without a setting change until it is free.
#[trial]
fn a_new_port_restarts_with_the_same_token_and_a_taken_port_is_retried() {
    let dir = temp_dir("link-port");
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x").retry_after(Duration::ZERO);
    assert!(link.sync(Some(&on(0)), &host).requests.is_some());
    let token = std::fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap();

    let wanted = free_port();
    let moved = link.sync(Some(&on(u32::from(wanted))), &host);
    assert!(moved.stopped && moved.requests.is_some());
    assert_eq!(link.status().port, Some(wanted));
    assert_eq!(read_connection(dir.path()).unwrap().expect("published").port, wanted);
    assert_eq!(std::fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap(), token, "the token is kept across restarts");

    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let taken_port = taken.local_addr().unwrap().port();
    let failed = link.sync(Some(&on(u32::from(taken_port))), &host);
    assert!(failed.stopped && failed.requests.is_none());
    assert!(matches!(link.status().problem, Some(LinkProblem::Listen { port, .. }) if port == taken_port), "{:?}", link.status());
    assert_eq!(link.status().port, None);
    assert_eq!(read_connection(dir.path()).unwrap(), None, "nothing listens without a file, and no file without a listener");

    // Still taken: the retry fails the same way and changes nothing.
    let again = link.sync(Some(&on(u32::from(taken_port))), &host);
    assert!(again.requests.is_none() && !again.changed);
    drop(taken);
    let retried = link.sync(Some(&on(u32::from(taken_port))), &host);
    assert!(retried.requests.is_some() && retried.changed, "freeing the port is enough");
    assert_eq!(link.status().port, Some(taken_port));
    assert_eq!(link.status().problem, None);
    assert_eq!(std::fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap(), token);

    // A port setting outside 0..=65535 is a problem, not a random port.
    let out = link.sync(Some(&on(70_000)), &host);
    assert!(out.requests.is_none());
    assert_eq!(link.status().problem, Some(LinkProblem::PortOutOfRange(70_000)));
    assert_eq!(link.status().port, None);
    assert_eq!(read_connection(dir.path()).unwrap(), None, "the listener stopped and no file is left");
}

/// Ten syncs with the same settings start one server.
#[trial]
fn an_unchanged_sync_starts_nothing() {
    let dir = temp_dir("link-same");
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x");
    let updates: Vec<_> = (0..10).map(|_| link.sync(Some(&on(0)), &host)).collect();
    assert_eq!(updates.iter().filter(|u| u.requests.is_some()).count(), 1);
    assert_eq!(updates.iter().filter(|u| u.changed).count(), 1);
    assert!(updates.iter().all(|u| !u.stopped));
    assert!(link.status().port.is_some());
}

/// Allow Agent Control off, no settings folder and an unusable token file each keep agents out: no channel, no file,
/// and the port is free.
#[trial]
fn allow_agent_control_off_and_no_settings_folder_keep_agents_out() {
    let host = host();
    let dir = temp_dir("link-off");
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x");
    assert!(link.sync(Some(&on(0)), &host).requests.is_some());
    let port = link.status().port.expect("listening");
    let mut off = on(0);
    off.allow_agent_control = false;
    let refused = link.sync(Some(&off), &host);
    assert!(refused.stopped && refused.requests.is_none());
    assert_eq!(link.status().problem, Some(LinkProblem::NotAllowed));
    assert!(!dir.path().join(CONNECTION_FILE).exists());
    TcpListener::bind(("127.0.0.1", port)).expect("the port is free");

    let mut homeless = AgentLink::new(None, "x");
    assert!(homeless.sync(Some(&on(0)), &host).requests.is_none());
    assert_eq!(homeless.status().problem, Some(LinkProblem::NoSettingsFolder));

    let bad = temp_dir("link-token");
    std::fs::create_dir(bad.path().join(TOKEN_FILE)).unwrap();
    let wanted = free_port();
    let mut link = AgentLink::new(Some(bad.path().to_path_buf()), "x");
    assert!(link.sync(Some(&on(u32::from(wanted))), &host).requests.is_none());
    assert!(matches!(link.status().problem, Some(LinkProblem::TokenFile(_))), "{:?}", link.status());
    assert!(!bad.path().join(CONNECTION_FILE).exists());
    TcpListener::bind(("127.0.0.1", wanted)).expect("nothing listens");
    let status = serde_json::to_value(link.status()).unwrap();
    assert!(status["error"].as_str().unwrap().starts_with("Agents can't connect: the token file"), "{status}");
}

/// A token file that can't be written (here the settings folder is a file) keeps agents out: the problem is reported,
/// nothing is published and the port is let go.
#[trial]
fn a_token_file_that_cannot_be_written_keeps_agents_out() {
    let dir = temp_dir("link-token-folder");
    let settings = dir.path().join("not-a-folder");
    std::fs::write(&settings, b"").unwrap();
    let port = free_port();
    let mut link = AgentLink::new(Some(settings), "x");
    assert!(link.sync(Some(&on(u32::from(port))), &host()).requests.is_none());
    assert_eq!(link.status().port, None);
    assert!(matches!(link.status().problem, Some(LinkProblem::TokenFile(_))), "{:?}", link.status());
    TcpListener::bind(("127.0.0.1", port)).expect("the port was let go");
}

/// A refused token file is not tried again with the same settings: a retry can't succeed until someone changes the
/// file, and on Windows each check starts `whoami` and PowerShell on the UI thread, about 300 ms, which a retry every
/// few seconds turned into a stall that often (photocraft#17006). Changing the settings tries again.
#[trial]
fn a_refused_token_file_is_not_retried_until_the_settings_change() {
    let dir = temp_dir("link-token-no-retry");
    std::fs::create_dir(dir.path().join(TOKEN_FILE)).unwrap();
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x").retry_after(Duration::ZERO);
    let wanted = free_port();
    assert!(link.sync(Some(&on(u32::from(wanted))), &host).requests.is_none());
    assert!(matches!(link.status().problem, Some(LinkProblem::TokenFile(_))), "{:?}", link.status());

    // The file is fixed, but the same settings don't look at it again.
    std::fs::remove_dir(dir.path().join(TOKEN_FILE)).unwrap();
    let same = link.sync(Some(&on(u32::from(wanted))), &host);
    assert!(same.requests.is_none() && !same.changed, "no retry with the same settings");
    assert!(matches!(link.status().problem, Some(LinkProblem::TokenFile(_))), "{:?}", link.status());

    let changed = link.sync(Some(&on(0)), &host);
    assert!(changed.requests.is_some(), "a settings change tries again");
    assert_eq!(link.status().problem, None);
}

/// A token file the check refuses as it is (here a damaged token; on Windows also one an older app wrote with its
/// folder's access list, open to Administrators) is the link's own: it is replaced with a new token, so agents can
/// connect instead of staying out until someone deletes the file (photocraft#17006).
#[trial]
fn a_refused_token_file_is_replaced_with_a_new_token() {
    let dir = temp_dir("link-token-refresh");
    let file = dir.path().join(TOKEN_FILE);
    std::fs::write(&file, "not a token\n").unwrap();
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x");
    assert!(link.sync(Some(&on(0)), &host).requests.is_some(), "{:?}", link.status());
    assert_eq!(link.status().problem, None);
    let token = std::fs::read_to_string(&file).unwrap();
    assert_ne!(token.trim(), "not a token");
    assert_eq!(token.trim().len(), 64, "a new token: {token:?}");
    assert_eq!(read_connection(dir.path()).unwrap().expect("published").port, link.status().port.expect("listening"));
}

/// A retry of a taken port reuses the token its first try checked instead of reading the token file again (on Windows
/// that check is about 300 ms on the UI thread, photocraft#17006); a start that succeeded reads it afresh next time.
#[trial]
fn a_taken_port_retry_reuses_the_checked_token() {
    let dir = temp_dir("link-token-reuse");
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x").retry_after(Duration::ZERO);
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(taken.local_addr().unwrap().port());
    assert!(link.sync(Some(&on(port)), &host).requests.is_none());
    assert!(matches!(link.status().problem, Some(LinkProblem::Listen { .. })), "{:?}", link.status());

    // Reading the file now fails, so a retry that read it would report a token file problem.
    std::fs::remove_file(dir.path().join(TOKEN_FILE)).unwrap();
    std::fs::create_dir(dir.path().join(TOKEN_FILE)).unwrap();
    drop(taken);
    let retried = link.sync(Some(&on(port)), &host);
    assert!(retried.requests.is_some(), "{:?}", link.status());
    assert_eq!(link.status().problem, None);

    // Once a start succeeded, the next one reads the file again.
    let moved = link.sync(Some(&on(0)), &host);
    assert!(moved.requests.is_none());
    assert!(matches!(link.status().problem, Some(LinkProblem::TokenFile(_))), "{:?}", link.status());
}

/// The checked token is kept only for retries with the same settings: turning agents off and on again reads the token
/// file again, so a server never listens with a token the file no longer holds.
#[trial]
fn a_settings_change_reads_the_token_file_again() {
    let dir = temp_dir("link-token-settings");
    let host = host();
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x").retry_after(Duration::ZERO);
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(taken.local_addr().unwrap().port());
    assert!(link.sync(Some(&on(port)), &host).requests.is_none());
    assert!(matches!(link.status().problem, Some(LinkProblem::Listen { .. })), "{:?}", link.status());

    assert!(!link.sync(None, &host).stopped, "nothing was running");
    // While agents are off the token file changes; here it can't be read at all, which the next start must report.
    std::fs::remove_file(dir.path().join(TOKEN_FILE)).unwrap();
    std::fs::create_dir(dir.path().join(TOKEN_FILE)).unwrap();
    drop(taken);
    let back = link.sync(Some(&on(port)), &host);
    assert!(back.requests.is_none(), "started with the token the file no longer holds");
    assert!(matches!(link.status().problem, Some(LinkProblem::TokenFile(_))), "{:?}", link.status());
}

/// Dropping the link stops its server and removes its connection file.
#[trial]
fn dropping_the_link_removes_its_connection_file() {
    let dir = temp_dir("link-drop");
    let mut link = AgentLink::new(Some(dir.path().to_path_buf()), "x");
    assert!(link.sync(Some(&on(0)), &host()).requests.is_some());
    let port = link.status().port.expect("listening");
    drop(link);
    assert!(!dir.path().join(CONNECTION_FILE).exists());
    TcpListener::bind(("127.0.0.1", port)).expect("the port is free");
}

/// A server the launch options started is reported with its port, and the settings change nothing.
#[trial]
fn from_launch_reports_the_launch_flags() {
    let (_requests, server) = server::listen(0, "0".repeat(64), host()).expect("started");
    let port = server.addr.port();
    let mut link = AgentLink::from_launch(server);
    assert_eq!((link.status().port, link.status().launch_flags), (Some(port), true));
    let update = link.sync(None, &host());
    assert!(!update.stopped && !update.changed && update.requests.is_none());
    assert_eq!(link.status().port, Some(port));
    assert_eq!(serde_json::to_value(link.status()).unwrap(), serde_json::json!({"port": port, "launchFlags": true, "error": null}));
}

/// The page over a draft, in a fixture whose host is named `"Studio"` and answers picks when the test says.
struct Page {
    fixture: Fixture,
    draft: Rc<RefCell<SettingsDraft>>,
    files: Arc<ScriptedFiles>,
    browsing: Rc<Cell<Option<PickId>>>,
}

fn page(mcp_flag: bool, status: AgentLinkStatus) -> Page {
    let files = Arc::new(ScriptedFiles::held());
    let draft = Rc::new(RefCell::new(SettingsDraft::default()));
    let browsing = Rc::new(Cell::new(None));
    let (held, issued) = (Rc::clone(&draft), Rc::clone(&browsing));
    let mut fixture = Fixture::new(move |ui| {
        let out = AgentsPage::new("integrations", &mut held.borrow_mut(), &status).mcp_flag(mcp_flag).show(ui);
        if out.browsing.is_some() {
            issued.set(out.browsing);
        }
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "studio", name: "Studio", version: "1.0" }).files(files.clone()).build()));
    fixture.settle();
    Page { fixture, draft, files, browsing }
}

fn shows(fixture: &Fixture, text: &str) -> bool {
    fixture.by_label(text).is_some()
}

/// With the flag the page names the launcher's switch, has no switch of its own, and says where the link stands.
#[trial]
fn with_the_flag_the_page_points_to_the_launcher() {
    let intro_part = "ArtCraft Launcher › Settings › AI agents › Let AI agents use Craft apps";
    let off = page(true, AgentLinkStatus::default());
    let intro = off
        .fixture
        .tree()
        .keys()
        .into_iter()
        .filter_map(|node| off.fixture.tree().name_of(node).map(str::to_owned))
        .find(|name| name.contains(intro_part));
    assert!(intro.is_some(), "the intro names the launcher's switch");
    assert!(shows(&off.fixture, "Off: turn on AI agents in ArtCraft Launcher › Settings › AI agents."));
    assert!(shows(&off.fixture, "Folder agents can open from") && shows(&off.fixture, "Folder agents can save to"));
    assert!(!shows(&off.fixture, "Allow Agent Control"), "no switch of its own");
    assert!(!shows(&off.fixture, "Let AI agents connect (MCP)"));

    let on = page(true, AgentLinkStatus { port: Some(4321), ..AgentLinkStatus::default() });
    assert!(shows(&on.fixture, "Agents can connect on 127.0.0.1:4321."));
}

/// Without the flag only the settings the app had before the switch show.
#[trial]
fn without_the_flag_only_the_earlier_settings_show() {
    let legacy = page(false, AgentLinkStatus::default());
    assert!(shows(&legacy.fixture, "Allow Agent Control") && shows(&legacy.fixture, "Control Port"));
    assert!(!shows(&legacy.fixture, "Folder agents can open from"));
    assert!(!shows(&legacy.fixture, "Off: turn on AI agents in ArtCraft Launcher › Settings › AI agents."));
}

/// Browse on the open-from folder asks the host for a folder; the answer lands in that key of the draft only.
#[trial]
fn browse_puts_the_chosen_folder_in_the_draft() {
    let mut page = page(true, AgentLinkStatus::default());
    let browse: Vec<_> =
        page.fixture.by_role(Role::BUTTON).into_iter().filter(|button| page.fixture.tree().name_of(button.node) == Some("Browse…")).collect();
    assert_eq!(browse.len(), 2, "one Browse per folder");
    page.fixture.click(&browse[0]);
    page.fixture.settle();
    let id = page.browsing.get().expect("Browse filed a pick");
    assert!(page.files.answer_to(id, Picked::Paths(vec![PathBuf::from("/work/in")])), "the pick is open");
    page.fixture.idle(3);
    let draft = page.draft.borrow();
    assert_eq!(draft.get::<String>("integrations.readRoot").as_deref(), Some("/work/in"));
    assert_eq!(draft.get::<String>("integrations.writeRoot"), None, "the other folder is untouched");
}

/// The closed-app choice names the host's app, and a pick writes its saved name.
#[trial]
fn the_closed_app_choice_names_the_app() {
    let mut page = page(true, AgentLinkStatus::default());
    assert!(shows(&page.fixture, "When Studio is closed"));
    let field = page.fixture.by_role(Role::COMBO_BOX).first().copied().expect("the choice shows");
    assert_eq!(page.fixture.tree().value_of(field.node), Some("Open Studio"));
    page.fixture.click(&field);
    page.fixture.idle(3);
    assert!(shows(&page.fixture, "Ask to open Studio"));
    let headless = page.fixture.by_label("Work without a window").expect("the menu lists the choices");
    page.fixture.click(&headless);
    page.fixture.idle(3);
    assert_eq!(page.draft.borrow().get::<String>("integrations.whenAppClosed").as_deref(), Some("headless"));
}
