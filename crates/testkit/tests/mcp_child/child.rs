use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

use craft_testkit::mcp::{McpChild, McpError, McpLimits, PROTOCOL_VERSION};
use serde_json::{Value, json};

/// The server the child acts as.
const CHILD_VAR: &str = "CRAFT_TESTKIT_MCP_CHILD";
/// Text the `server` child returns from every tool call.
const ECHO_VAR: &str = "CRAFT_TESTKIT_MCP_ECHO";
/// Bytes the `flood` child writes on stderr before each reply, more than a pipe holds.
const FLOOD: usize = 1 << 20;

type Test = fn() -> Result<(), String>;

const TESTS: &[(&str, Test)] = &[
    ("a_spawned_server_answers_and_closes_cleanly", a_spawned_server_answers_and_closes_cleanly),
    ("silent_spawned_child_hits_deadline", silent_spawned_child_hits_deadline),
    ("child_exits_before_initialize", child_exits_before_initialize),
    ("stderr_flood_does_not_block", stderr_flood_does_not_block),
    ("close_forces_and_reaps_unresponsive_child", close_forces_and_reaps_unresponsive_child),
];

pub fn run() -> ExitCode {
    if let Some(mode) = std::env::var_os(CHILD_VAR) {
        return serve(&mode.to_string_lossy());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let chosen: Vec<&(&str, Test)> = TESTS.iter().filter(|(name, _)| filters.iter().all(|f| name.contains(f.as_str()))).collect();
    if args.iter().any(|a| a == "--list") {
        for (name, _) in chosen {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    println!("\nrunning {} tests", chosen.len());
    let mut failed = 0;
    for (name, test) in &chosen {
        match test() {
            Ok(()) => println!("test {name} ... ok"),
            Err(why) => {
                failed += 1;
                println!("test {name} ... FAILED\n    {why}");
            }
        }
    }
    println!("\ntest result: {}. {} passed; {failed} failed", if failed == 0 { "ok" } else { "FAILED" }, chosen.len() - failed);
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Runs as the child: `server` answers `initialize` with the version asked for and every tool call
/// with `ECHO_VAR`, and exits when its stdin closes; `flood` also writes `FLOOD` bytes and `END` on
/// stderr before each reply; `silent` reads and never answers; `stubborn` answers but does not
/// exit when its stdin closes; `exit` exits at once with code 3.
fn serve(mode: &str) -> ExitCode {
    if mode == "exit" {
        return ExitCode::from(3);
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        if mode == "silent" || req.get("id").is_none() {
            continue;
        }
        let result = match req["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion": req["params"]["protocolVersion"], "capabilities": {}, "serverInfo": {"name": "child", "version": "0"}})
            }
            Some("tools/call") => {
                let text = std::env::var(ECHO_VAR).unwrap_or_default();
                json!({"content": [{"type": "text", "text": text}], "isError": false})
            }
            _ => json!(null),
        };
        if mode == "flood" {
            let mut err = std::io::stderr();
            if err.write_all(&vec![b'e'; FLOOD]).and_then(|()| err.write_all(b"END")).is_err() {
                return ExitCode::FAILURE;
            }
        }
        let reply = json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            return ExitCode::FAILURE;
        }
    }
    if mode == "stubborn" {
        // Waits without a stdin to read until the parent stops it; ends on its own after ten
        // minutes so a parent that fails to stop it leaves nothing running.
        let (_keep, wait) = std::sync::mpsc::channel::<()>();
        let _ = wait.recv_timeout(Duration::from_secs(600));
    }
    ExitCode::SUCCESS
}

fn start(mode: &str, limits: McpLimits) -> Result<McpChild, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    McpChild::spawn_with_limits(exe, std::iter::empty::<&str>(), [(CHILD_VAR, mode), (ECHO_VAR, "hello from the child")], limits)
        .map_err(|e| format!("spawn {mode}: {e}"))
}

fn check(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

fn a_spawned_server_answers_and_closes_cleanly() -> Result<(), String> {
    let before = std::env::var_os(ECHO_VAR);
    let mut c = start("server", McpLimits::default())?;
    let init = c.initialize(PROTOCOL_VERSION).map_err(|e| e.to_string())?;
    check(init["serverInfo"]["name"] == "child", || format!("initialize: {init}"))?;
    let r = c.call_tool("echo", json!({})).map_err(|e| e.to_string())?;
    check(r["content"][0]["text"] == "hello from the child", || format!("the child did not get its variable: {r}"))?;
    check(std::env::var_os(ECHO_VAR) == before, || "the test process's environment changed".into())?;
    let out = c.close().map_err(|e| e.to_string())?;
    check(!out.forced && out.status.is_some_and(|s| s.success()), || format!("close: {out:?}"))
}

fn silent_spawned_child_hits_deadline() -> Result<(), String> {
    let limits = McpLimits { initialize_timeout: Duration::from_millis(200), ..McpLimits::default() };
    let mut c = start("silent", limits)?;
    let err = c.initialize(PROTOCOL_VERSION).err();
    check(matches!(err, Some(McpError::Deadline { .. })), || format!("initialize: {err:?}"))?;
    check(c.closed().is_some(), || "the connection is still open".into())?;
    let out = c.close().map_err(|e| e.to_string())?;
    check(out.forced && out.status.is_some(), || format!("the silent child was not stopped: {out:?}"))
}

fn child_exits_before_initialize() -> Result<(), String> {
    let mut c = start("exit", McpLimits::default())?;
    // The write of `initialize` and the end of stdout race; either one is reported.
    let err = c.initialize(PROTOCOL_VERSION).err();
    check(matches!(err, Some(McpError::Ended | McpError::Io { .. })), || format!("initialize: {err:?}"))?;
    let out = c.close().map_err(|e| e.to_string())?;
    check(!out.forced && out.status.and_then(|s| s.code()) == Some(3), || format!("close: {out:?}"))
}

fn stderr_flood_does_not_block() -> Result<(), String> {
    let limits = McpLimits {
        stderr_tail_bytes: 4096,
        initialize_timeout: Duration::from_secs(20),
        request_timeout: Duration::from_secs(20),
        ..McpLimits::default()
    };
    let mut c = start("flood", limits)?;
    c.initialize(PROTOCOL_VERSION).map_err(|e| e.to_string())?;
    c.call_tool("echo", json!({})).map_err(|e| e.to_string())?;
    let out = c.close().map_err(|e| e.to_string())?;
    check(!out.forced, || format!("close: {out:?}"))?;
    check(out.stderr.len() == 4096 && out.stderr.ends_with(b"eeeEND"), || format!("kept {} bytes of stderr", out.stderr.len()))
}

fn close_forces_and_reaps_unresponsive_child() -> Result<(), String> {
    let limits = McpLimits { close_grace: Duration::from_millis(200), ..McpLimits::default() };
    let mut c = start("stubborn", limits)?;
    c.initialize(PROTOCOL_VERSION).map_err(|e| e.to_string())?;
    let out = c.close().map_err(|e| e.to_string())?;
    check(out.forced && out.status.is_some_and(|s| !s.success()), || format!("close: {out:?}"))
}
