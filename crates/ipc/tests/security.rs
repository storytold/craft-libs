//! Auth refusal on the local socket. These tests speak bytes themselves.
//! They do not use the client module.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use craft_ipc::server::{Activate, Activation, ControlSink, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::{LinkError, RuntimeDir};

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Sink;

impl ControlSink for Sink {
    fn submit(&self, _method: String, _params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(json!({}));
        Some(rx)
    }
}

struct NoActivate;

impl Activate for NoActivate {
    fn activate(&self, _token: Option<String>, _from_pid: Option<u32>, _open: Option<Value>) -> Activation {
        Activation::No
    }
}

struct NoFrames;

impl Frames for NoFrames {
    fn open(&self, _params: &serde_json::Map<String, Value>) -> Result<FrameOpen, LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }

    fn render(&self, _ticket: &str, _header: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }

    fn close_ticket(&self, _ticket: &str) {}
}

struct Running {
    _tmp: Tmp,
    server: Server,
    socket: PathBuf,
    token: String,
}

fn boot() -> Running {
    boot_with(None)
}

fn boot_with(tcp_port: Option<u16>) -> Running {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let mixed = (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x9E37_79B9);
    let path = PathBuf::from(format!("/tmp/cl-srv-{mixed:08x}"));
    let _ = std::fs::remove_dir_all(&path);
    let runtime = RuntimeDir::at(&path).unwrap();
    let server = Server::start(ServerConfig {
        app: "ec".into(),
        app_version: "0.0.1".into(),
        instance: None,
        runtime: Some(runtime),
        tcp_port,
        sink: Arc::new(Sink),
        activate: Arc::new(NoActivate),
        frames: Arc::new(NoFrames),
    })
    .unwrap();
    let socket = match &server.record().endpoint {
        craft_ipc::Endpoint::Unix { path } => PathBuf::from(path),
        other => panic!("expected a unix endpoint, got {other:?}"),
    };
    let token = server.record().token.clone();
    Running { _tmp: Tmp(path), server, socket, token }
}

fn read_closed_line(socket: &Path, payload: &[u8]) -> Value {
    let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.write_all(payload).unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap_or_else(|err| panic!("socket did not close after auth failure: {err}"));
    let lines: Vec<_> = text.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(!lines[0].contains("bad JSON"), "{text}");
    serde_json::from_str(lines[0]).unwrap()
}

fn assert_auth_failure(value: &Value) {
    assert_eq!(value["id"], json!(0), "{value}");
    assert_eq!(value["ok"], json!(false), "{value}");
    assert_eq!(value["error"], json!("authentication required"), "{value}");
}

#[test]
fn no_token_is_auth_failure_not_bad_json() {
    let running = boot();
    let value = read_closed_line(&running.socket, b"{\"id\":1,\"method\":\"engine.commands\",\"params\":{}}\n");
    assert_auth_failure(&value);
    running.server.shutdown().unwrap();
}

#[test]
fn wrong_token_is_auth_failure() {
    let running = boot();
    let mut bad = "ab".repeat(32);
    if bad == running.token {
        bad = "cd".repeat(32);
    }
    let line = json!({"id": 4, "method": "auth", "params": {"token": bad}});
    let mut bytes = serde_json::to_vec(&line).unwrap();
    bytes.push(b'\n');
    let value = read_closed_line(&running.socket, &bytes);
    assert_auth_failure(&value);
    running.server.shutdown().unwrap();
}

#[test]
fn malformed_first_line_is_auth_failure() {
    let running = boot();
    let value = read_closed_line(&running.socket, b"{\n");
    assert_auth_failure(&value);
    running.server.shutdown().unwrap();
}

#[test]
fn tcp_is_off_when_the_port_is_none() {
    let running = boot();
    assert_eq!(running.server.tcp_port(), None);
    running.server.shutdown().unwrap();
}

fn tcp_exchange(port: u16, payload: &[u8]) -> String {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.write_all(payload).unwrap();
    let mut text = String::new();
    let _ = stream.read_to_string(&mut text);
    text
}

#[test]
fn tcp_without_a_token_is_refused_and_runs_nothing() {
    let running = boot_with(Some(0));
    let port = running.server.tcp_port().expect("tcp port");
    assert_ne!(port, 0);
    let text = tcp_exchange(port, b"{\"id\":7,\"method\":\"engine.commands\",\"params\":{}}\n");
    let first = text.lines().find(|line| !line.is_empty()).unwrap_or_else(|| panic!("tcp closed with no refusal line"));
    assert_auth_failure(&serde_json::from_str(first).unwrap());
    assert!(!text.contains("\"result\""), "{text}");
    let bad_json = tcp_exchange(port, b"{\n");
    assert!(!bad_json.contains("bad JSON"), "{bad_json}");
    assert!(bad_json.contains("authentication required"), "{bad_json}");
    let mut wrong = "ab".repeat(32);
    if wrong == running.token {
        wrong = "cd".repeat(32);
    }
    let line = json!({"id": 0, "method": "auth", "params": {"token": wrong}});
    let mut bytes = serde_json::to_vec(&line).unwrap();
    bytes.push(b'\n');
    assert_auth_failure(&serde_json::from_str(tcp_exchange(port, &bytes).lines().next().unwrap()).unwrap());
    running.server.shutdown().unwrap();
}

#[test]
fn tcp_with_the_record_token_works_and_the_record_publishes_the_port() {
    let running = boot_with(Some(0));
    let port = running.server.tcp_port().expect("tcp port");
    match &running.server.record().endpoint {
        craft_ipc::Endpoint::Unix { .. } => {}
        other => panic!("the instance record must keep the unix socket, got {other:?}"),
    }
    assert_eq!(running.server.record().tcp_port(), Some(port));
    let mut record = running.server.record().clone();
    record.endpoint = craft_ipc::Endpoint::Tcp { port };
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let auth = json!({"id": 0, "method": "auth", "params": {"token": running.token}});
    let mut bytes = serde_json::to_vec(&auth).unwrap();
    bytes.extend_from_slice(b"\n{\"id\":7,\"method\":\"engine.commands\",\"params\":{}}\n");
    stream.write_all(&bytes).unwrap();
    let mut buf = Vec::new();
    while buf.iter().filter(|b| **b == b'\n').count() < 2 {
        let mut byte = [0u8; 1];
        assert_ne!(stream.read(&mut byte).unwrap(), 0, "tcp closed before a reply");
        buf.push(byte[0]);
    }
    let text = String::from_utf8(buf).unwrap();
    let mut lines = text.lines();
    let ok: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(ok["ok"], json!(true), "{ok}");
    let value: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(value["id"], json!(7), "{value}");
    assert_eq!(value["ok"], json!(true), "{value}");
    assert_eq!(value["result"], json!({}), "{value}");
    // The library client does the same from the published record.
    let mut client = craft_ipc::client::Client::connect_tcp(running.server.record()).unwrap();
    assert_eq!(client.call("engine.commands", json!({})).unwrap(), json!({}));
    running.server.shutdown().unwrap();
}

#[test]
fn tcp_is_not_published_when_it_is_off() {
    let running = boot();
    assert_eq!(running.server.record().tcp_port(), None);
    assert!(craft_ipc::client::Client::connect_tcp(running.server.record()).is_err());
    running.server.shutdown().unwrap();
}

#[test]
fn silent_tcp_connections_hit_the_unauthenticated_cap() {
    let running = boot_with(Some(0));
    let port = running.server.tcp_port().expect("tcp port");
    let silent: Vec<_> = (0..craft_ipc::wire::MAX_UNAUTHENTICATED).map(|_| std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
    std::thread::sleep(Duration::from_millis(400));
    let text = tcp_exchange(port, b"");
    assert!(text.contains("too many unauthenticated connections"), "{text}");
    drop(silent);
    running.server.shutdown().unwrap();
}

#[test]
fn other_uid_is_refused_when_the_env_is_set() {
    let Ok(user) = std::env::var("CRAFT_TEST_OTHER_USER") else {
        return;
    };
    let running = boot();
    // 0600 blocks a non-owner before the peer check. Loosen it so this user
    // can connect. The assertion is still the auth failure line.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&running.socket, std::fs::Permissions::from_mode(0o666)).unwrap();
    }
    let script = r#"
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(5)
s.connect(sys.argv[1])
token = sys.argv[2]
msg = '{"id":1,"method":"auth","params":{"token":"%s"}}\n' % token
s.sendall(msg.encode())
data = b""
while b"\n" not in data:
    chunk = s.recv(4096)
    if not chunk:
        break
    data += chunk
sys.stdout.buffer.write(data)
"#;
    let output = run_as(&user, script, &running.socket, &running.token);
    if !output.status.success() {
        panic!("run as {user} failed: status {:?} stderr {}", output.status.code(), String::from_utf8_lossy(&output.stderr));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains("bad JSON"), "{text}");
    let value: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_auth_failure(&value);
    running.server.shutdown().unwrap();
}

fn run_as(user: &str, script: &str, socket: &Path, token: &str) -> std::process::Output {
    let args = ["-u", user, "--", "python3", "-c", script];
    match std::process::Command::new("runuser").args(args).arg(socket).arg(token).output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::process::Command::new("sudo").args(args).arg(socket).arg(token).output().unwrap_or_else(|err| {
                panic!("sudo failed to start: {err}");
            })
        }
        Err(err) => panic!("runuser failed to start: {err}"),
    }
}
