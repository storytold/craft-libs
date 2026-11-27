//! The add-in the integration tests start as a real child process. It answers `echo` with its
//! params, and misbehaves on request: `ask` calls the host back before answering, `sleep` answers
//! late, `exit` dies with a last word on standard error, `say` pushes a notification first.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufRead, Write};

use serde_json::{Value as Json, json};

fn send(out: &mut impl Write, msg: &Json) -> bool {
    writeln!(out, "{msg}").and_then(|_| out.flush()).is_ok()
}

/// Asks the host `times` times and returns its last answer (the result or the error).
fn ask(lines: &mut impl Iterator<Item = std::io::Result<String>>, out: &mut impl Write, params: &Json) -> Json {
    let times = params.get("times").and_then(Json::as_u64).unwrap_or(1).min(1_000);
    let pause = params.get("pauseMs").and_then(Json::as_u64).unwrap_or(0).min(10_000);
    let mut last = Json::Null;
    for i in 0..times {
        std::thread::sleep(std::time::Duration::from_millis(pause));
        let method = params.get("method").and_then(Json::as_str).unwrap_or("host/echo");
        if !send(out, &json!({"jsonrpc": "2.0", "id": format!("a{i}"), "method": method, "params": {"n": i}})) {
            break;
        }
        let Some(Ok(line)) = lines.next() else { break };
        let answer: Json = serde_json::from_str(&line).unwrap_or(Json::Null);
        last = answer.get("result").or_else(|| answer.get("error")).cloned().unwrap_or(Json::Null);
    }
    last
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Json::Null);
        let Some(id) = msg.get("id").cloned() else {
            match method {
                "shutdown" => break,
                // A notification from the app comes back as one from the add-in.
                _ => {
                    send(&mut out, &json!({"jsonrpc": "2.0", "method": "heard", "params": {"method": method, "params": params}}));
                }
            }
            continue;
        };
        let reply = match method {
            "echo" => Ok(params),
            "env" => Ok(json!({
                "host": std::env::var("CRAFT_HOST").ok(),
                "dir": std::env::var("CRAFT_PLUGIN_DIR").ok(),
                "extra": std::env::var("CRAFT_PLUGIN_TEST_EXTRA").ok(),
                "cwd": std::env::current_dir().ok().map(|p| p.display().to_string()),
            })),
            "ask" => Ok(ask(&mut lines, &mut out, &params)),
            "sleep" => {
                std::thread::sleep(std::time::Duration::from_millis(params.get("ms").and_then(Json::as_u64).unwrap_or(0).min(60_000)));
                Ok(Json::Null)
            }
            "exit" => {
                let _ = writeln!(std::io::stderr(), "giving up");
                std::process::exit(3)
            }
            "noise" => {
                let _ = writeln!(out, "this line is not JSON");
                Ok(json!("after the noise"))
            }
            other => Err(format!("unknown method {other}")),
        };
        let answer = match reply {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(message) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}}),
        };
        if !send(&mut out, &answer) {
            break;
        }
    }
}
