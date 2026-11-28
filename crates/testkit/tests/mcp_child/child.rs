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
    ("close_keeps_stderr_written_before_exit", close_keeps_stderr_written_before_exit),
    ("skip_filters_exclude_tests_from_the_child_runner", skip_filters_exclude_tests_from_the_child_runner),
    ("thread_count_options_preserve_child_test_selection", thread_count_options_preserve_child_test_selection),
    ("close_drains_stdout_before_child_exit", close_drains_stdout_before_child_exit),
];

pub fn run() -> ExitCode {
    if let Some(mode) = std::env::var_os(CHILD_VAR) {
        return serve(&mode.to_string_lossy());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut filters = Vec::new();
    let mut skipped = Vec::new();
    let mut options = args.iter();
    while let Some(arg) = options.next() {
        if arg == "--test-threads" {
            if !options.next().is_some_and(|value| value.parse::<usize>().is_ok_and(|count| count > 0)) {
                eprintln!("--test-threads requires a positive integer");
                return ExitCode::FAILURE;
            }
        } else if let Some(value) = arg.strip_prefix("--test-threads=") {
            if !value.parse::<usize>().is_ok_and(|count| count > 0) {
                eprintln!("--test-threads requires a positive integer");
                return ExitCode::FAILURE;
            }
        } else if arg == "--skip" {
            let Some(filter) = options.next().filter(|value| !value.is_empty() && !value.starts_with('-')) else {
                eprintln!("--skip requires a test name");
                return ExitCode::FAILURE;
            };
            skipped.push(filter.as_str());
        } else if let Some(filter) = arg.strip_prefix("--skip=") {
            if filter.is_empty() || filter.starts_with('-') {
                eprintln!("--skip requires a test name");
                return ExitCode::FAILURE;
            }
            skipped.push(filter);
        } else if !arg.starts_with('-') {
            filters.push(arg.as_str());
        }
    }
    let chosen: Vec<&(&str, Test)> =
        TESTS.iter().filter(|(name, _)| filters.iter().all(|f| name.contains(*f)) && !skipped.iter().any(|f| name.contains(*f))).collect();
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
/// exit when its stdin closes; `exit` exits at once with code 3; `goodbye` writes `FLOOD` bytes and
/// `BYE` on stderr and exits without reading.
fn serve(mode: &str) -> ExitCode {
    if mode == "exit" {
        return ExitCode::from(3);
    }
    if mode == "goodbye" {
        let mut err = std::io::stderr();
        let written = err.write_all(&vec![b'e'; FLOOD]).and_then(|()| err.write_all(b"BYE"));
        return if written.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
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
    if mode == "stdout-after-eof" {
        let line = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"data": "x".repeat(FLOOD)}}).to_string();
        for _ in 0..8 {
            if writeln!(out, "{line}").is_err() {
                return ExitCode::FAILURE;
            }
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

fn skip_filters_exclude_tests_from_the_child_runner() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    for args in [vec!["--list", "--skip", "stderr_flood_does_not_block"], vec!["--list", "--skip=stderr_flood_does_not_block"]] {
        let output = std::process::Command::new(&exe).args(args).output().map_err(|e| e.to_string())?;
        check(output.status.success(), || String::from_utf8_lossy(&output.stderr).into_owned())?;
        let listed = String::from_utf8_lossy(&output.stdout);
        check(!listed.contains("stderr_flood_does_not_block"), || format!("the skipped test was selected: {listed}"))?;
        check(listed.lines().count() == TESTS.len() - 1, || format!("other tests were filtered out: {listed}"))?;
    }
    for args in
        [vec!["--list", "--skip"], vec!["--skip", "--list"], vec!["--list", "--skip", ""], vec!["--list", "--skip="], vec!["--list", "--skip=--list"]]
    {
        let output = std::process::Command::new(&exe).args(&args).output().map_err(|e| e.to_string())?;
        check(!output.status.success(), || format!("an invalid skip filter was accepted: {args:?}"))?;
    }
    Ok(())
}

fn thread_count_options_preserve_child_test_selection() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    for args in [vec!["--list", "--test-threads", "1"], vec!["--list", "--test-threads=2"]] {
        let output = std::process::Command::new(&exe).args(args).output().map_err(|error| error.to_string())?;
        check(output.status.success(), || String::from_utf8_lossy(&output.stderr).into_owned())?;
        let listed = String::from_utf8_lossy(&output.stdout);
        check(listed.lines().count() == TESTS.len(), || format!("thread count changed test selection: {listed}"))?;
    }
    for args in [
        vec!["--list", "--test-threads"],
        vec!["--list", "--test-threads", "0"],
        vec!["--list", "--test-threads="],
        vec!["--list", "--test-threads=bad"],
        vec!["--test-threads", "--list"],
    ] {
        let output = std::process::Command::new(&exe).args(&args).output().map_err(|error| error.to_string())?;
        check(!output.status.success(), || format!("an invalid thread count was accepted: {args:?}"))?;
    }
    Ok(())
}

fn close_drains_stdout_before_child_exit() -> Result<(), String> {
    let mut client = start("stdout-after-eof", McpLimits::default())?;
    client.initialize(PROTOCOL_VERSION).map_err(|error| error.to_string())?;
    let outcome = client.close().map_err(|error| error.to_string())?;
    check(!outcome.forced && outcome.status.is_some_and(|status| status.success()), || format!("close: {outcome:?}"))
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

fn close_keeps_stderr_written_before_exit() -> Result<(), String> {
    let limits = McpLimits { stderr_tail_bytes: 4096, close_grace: Duration::ZERO, ..McpLimits::default() };
    let mut c = start("goodbye", limits)?;
    // The child's stdout ends when it exits, after its stderr is written.
    let err = c.initialize(PROTOCOL_VERSION).err();
    check(matches!(err, Some(McpError::Ended | McpError::Io { .. })), || format!("initialize: {err:?}"))?;
    let out = c.close().map_err(|e| e.to_string())?;
    check(out.stderr.len() == 4096 && out.stderr.ends_with(b"eeeBYE"), || format!("kept {} bytes of stderr", out.stderr.len()))
}
