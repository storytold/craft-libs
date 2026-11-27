//! The connection to a real add-in process (`craft-plugin-test-addin`).

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use craft_plugin::push::{self, Incoming};
use craft_plugin::{Connection, HostError, Launch, RunSpec};
use serde_json::{Value as Json, json};

/// Generous: only a test that means to time out waits this long.
const LONG: Duration = Duration::from_secs(30);

/// The push queue is one per process: tests that read it take turns.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn run_spec() -> RunSpec {
    RunSpec { command: env!("CARGO_BIN_EXE_craft-plugin-test-addin").into(), args: vec![] }
}

fn start(id: &str) -> Connection {
    let run = run_spec();
    let dir = std::env::temp_dir();
    Connection::start(&Launch { id, run: &run, dir: &dir, host_id: "testcraft", env: &[("CRAFT_PLUGIN_TEST_EXTRA", "yes")] }).unwrap()
}

/// What `id` pushed, waiting until `enough` says so.
fn pushed(id: &str, enough: impl Fn(&[Incoming]) -> bool) -> Vec<Incoming> {
    let mut got = Vec::new();
    let until = Instant::now() + LONG;
    while !enough(&got) && Instant::now() < until {
        got.extend(push::take().into_iter().filter(|p| p.plugin == id).map(|p| p.incoming));
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

#[test]
fn requests_are_answered_and_errors_keep_the_connection() {
    let mut c = start("it.echo");
    assert_eq!(c.request("echo", json!({"a": [1, 2]}), LONG).unwrap(), json!({"a": [1, 2]}));
    let e = c.request("nope", Json::Null, LONG).unwrap_err();
    assert!(!e.fatal && e.message.contains("unknown method"), "{e:?}");
    // A line that isn't JSON is skipped, not taken for the answer.
    assert_eq!(c.request("noise", Json::Null, LONG).unwrap(), json!("after the noise"));
    assert_eq!(c.request("echo", json!(7), LONG).unwrap(), json!(7));
    c.shutdown();
}

#[test]
fn the_add_in_runs_in_its_folder_and_knows_its_host() {
    let mut c = start("it.env");
    let env = c.request("env", Json::Null, LONG).unwrap();
    assert_eq!(env["host"], "testcraft");
    assert_eq!(env["extra"], "yes");
    let dir = PathBuf::from(env["dir"].as_str().unwrap());
    assert_eq!(dir.canonicalize().unwrap(), std::env::temp_dir().canonicalize().unwrap());
    assert_eq!(PathBuf::from(env["cwd"].as_str().unwrap()).canonicalize().unwrap(), dir.canonicalize().unwrap());
}

#[test]
fn a_missing_program_is_an_error() {
    let run = RunSpec { command: "./no-such-add-in".into(), args: vec![] };
    let dir = std::env::temp_dir();
    let e = Connection::start(&Launch { id: "it.missing", run: &run, dir: &dir, host_id: "testcraft", env: &[] }).err().unwrap();
    assert!(e.contains("no-such-add-in"), "{e}");
}

#[test]
fn the_host_answers_the_add_ins_requests_while_it_works() {
    let mut c = start("it.ask");
    let mut seen = Vec::new();
    let mut host = |method: &str, params: &Json| {
        seen.push(method.to_string());
        if method == "host/echo" { Ok(json!({"got": params["n"]})) } else { Err(HostError::unknown()) }
    };
    assert_eq!(c.request_with("ask", json!({"times": 3}), LONG, &mut host).unwrap(), json!({"got": 2}));
    let refused = c.request_with("ask", json!({"method": "host/teleport"}), LONG, &mut host).unwrap();
    assert_eq!(refused["code"], -32601);
    assert_eq!(seen, ["host/echo", "host/echo", "host/echo", "host/teleport"]);
    // Without a handler every host request is refused, and the request still completes.
    assert_eq!(c.request("ask", Json::Null, LONG).unwrap()["code"], -32601);
}

#[test]
fn a_late_answer_or_an_exit_is_fatal() {
    let mut c = start("it.slow");
    let e = c.request("sleep", json!({"ms": 20_000}), Duration::from_millis(100)).unwrap_err();
    assert!(e.fatal && e.message.contains("sleep took longer"), "{e:?}");
    drop(c);

    let mut c = start("it.exit");
    let e = c.request("exit", Json::Null, LONG).unwrap_err();
    assert!(e.fatal, "{e:?}");
    assert!(e.message.contains("giving up"), "the add-in's last words say why: {}", e.message);
}

#[test]
fn host_calls_extend_a_soft_deadline_and_not_a_hard_one() {
    // Eight host calls 100 ms apart: longer than the timeout in all, shorter between calls.
    let params = json!({"times": 8, "pauseMs": 100});
    let timeout = Duration::from_millis(500);
    let mut host = |_: &str, _: &Json| Ok(Json::Null);

    let mut c = start("it.soft");
    assert!(c.request_bounded("ask", params.clone(), timeout, false, &mut host).is_ok(), "an add-in that keeps calling the host is working");

    let mut c = start("it.hard");
    let e = c.request_bounded("ask", params, timeout, true, &mut host).unwrap_err();
    assert!(e.fatal && e.message.contains("took longer"), "{e:?}");
}

#[test]
fn what_an_add_in_says_on_its_own_is_pushed() {
    let _turn = serial();
    let mut c = start("it.push");
    // Make sure it runs before notifying it.
    c.request("echo", Json::Null, LONG).unwrap();
    c.notify("ping", json!({"n": 1})).unwrap();
    let got = pushed("it.push", |g| !g.is_empty());
    assert_eq!(got, [Incoming::Message(json!({"jsonrpc": "2.0", "method": "heard", "params": {"method": "ping", "params": {"n": 1}}}))]);

    // A handle sends without the connection; the exit is pushed too.
    let sender = c.sender().unwrap();
    sender.send(&json!({"jsonrpc": "2.0", "method": "again"})).unwrap();
    assert_eq!(pushed("it.push", |g| !g.is_empty()).len(), 1);
    c.shutdown();
    let got = pushed("it.push", |g| g.iter().any(|i| matches!(i, Incoming::Exited(_))));
    assert!(got.iter().any(|i| matches!(i, Incoming::Exited(_))), "{got:?}");
}

#[test]
fn a_hard_deadline_stops_host_calls_that_already_wait() {
    // The add-in sends 300 host calls at once; answering takes 10 ms each. All of them wait in
    // the queue when the time is up: the request ends there, it doesn't work through them.
    let mut c = start("it.burst");
    let mut answered = 0;
    let mut host = |_: &str, _: &Json| {
        answered += 1;
        std::thread::sleep(Duration::from_millis(10));
        Ok(Json::Null)
    };
    let e = c.request_bounded("burst", json!({"times": 300}), Duration::from_millis(300), true, &mut host).unwrap_err();
    assert!(e.fatal && e.message.contains("took longer"), "{e:?}");
    assert!(answered < 300, "{answered} host calls answered after the deadline");
}

#[test]
fn an_add_in_that_stops_reading_costs_an_error_not_memory() {
    let mut c = start("it.stall");
    c.request("echo", Json::Null, LONG).unwrap();
    c.notify("stall", Json::Null).unwrap();
    // Far more than the pipe and the queue hold together: sending must start to fail.
    let line = json!({"pad": "x".repeat(1_024)});
    let sent = (0..200_000).take_while(|_| c.notify("more", line.clone()).is_ok()).count();
    assert!(sent < 200_000, "every message was queued");
    let e = c.request("echo", Json::Null, LONG).unwrap_err();
    assert!(e.fatal, "a request can't be queued either: {e:?}");
}

#[test]
fn answers_nobody_asked_for_are_dropped() {
    let _turn = serial();
    let mut c = start("it.chatter");
    c.request("echo", Json::Null, LONG).unwrap();
    c.notify("chatter", Json::Null).unwrap();
    let got = pushed("it.chatter", |g| !g.is_empty());
    assert_eq!(got, [Incoming::Message(json!({"jsonrpc": "2.0", "method": "chattered"}))], "the answers aren't pushed");
    // And they don't wait for the next request, nor count against it.
    assert_eq!(c.request("echo", json!("still here"), LONG).unwrap(), json!("still here"));
}

/// On Unix a relative program path is looked up after the child has changed folder, so a folder
/// given relative to the app must be made absolute first.
#[cfg(unix)]
#[test]
fn a_relative_folder_starts_the_program_in_it() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_craft-plugin-test-addin"));
    let folder = exe.parent().unwrap().canonicalize().unwrap();
    // The folder as seen from here: up to the root, then down.
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let mut relative: PathBuf = cwd.components().skip(1).map(|_| "..").collect();
    relative.extend(folder.components().skip(1));
    assert!(relative.is_relative());
    let run = RunSpec { command: format!("./{}", exe.file_name().unwrap().to_str().unwrap()), args: vec![] };
    let mut c = Connection::start(&Launch { id: "it.relative", run: &run, dir: &relative, host_id: "testcraft", env: &[] }).unwrap();
    let env = c.request("env", Json::Null, LONG).unwrap();
    let told = PathBuf::from(env["dir"].as_str().unwrap());
    assert!(told.is_absolute(), "the add-in is told its folder in full: {}", told.display());
    assert_eq!(told.canonicalize().unwrap(), folder);
}
