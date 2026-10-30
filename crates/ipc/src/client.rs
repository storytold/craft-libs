//! Craft Link client.
//!
//! Connects to one instance record, authenticates on local sockets and pipes,
//! and exchanges JSON lines. Push events that arrive before a reply are kept
//! until [`Client::next_event`].

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ConnectOptions, GenericFilePath, Stream};
use serde_json::{Map, Value, json};

use crate::error::LinkError;
use crate::wire;
use crate::{Endpoint, InstanceRecord};

/// Identity sent in `link.hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloClient {
    pub app: String,
    pub app_version: String,
    pub pid: u32,
    pub instance: String,
    pub protocol_min: u32,
    pub protocol_max: u32,
}

/// One push line: `event`, `seq`, and `data`.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub event: String,
    pub seq: u64,
    pub data: Value,
}

/// A connected Craft Link session.
pub struct Client {
    record: InstanceRecord,
    reader: BufReader<Transport>,
    next_id: u64,
    events: VecDeque<Event>,
    hello: Option<HelloClient>,
}

impl Client {
    /// Connect to `record.endpoint`. Every transport sends auth id 0 with the record token first.
    pub fn connect(record: &InstanceRecord) -> Result<Self, LinkError> {
        let reader = open_reader(record)?;
        Ok(Self { record: record.clone(), reader, next_id: 1, events: VecDeque::new(), hello: None })
    }

    /// Connect to the loopback TCP control port published in `record` (`record.tcp_port()`),
    /// with the same token handshake. `Err(Invalid)` when the app opened no TCP port.
    pub fn connect_tcp(record: &InstanceRecord) -> Result<Self, LinkError> {
        let Some(port) = record.tcp_port() else {
            return Err(LinkError::Invalid("the instance record has no tcp_port; start the app with --control".into()));
        };
        let mut tcp = record.clone();
        tcp.endpoint = Endpoint::Tcp { port };
        Self::connect(&tcp)
    }

    /// Send one request. `id` starts at 1 and increases by one each call.
    /// Lines with `event` and a different id are buffered.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, LinkError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or_else(|| LinkError::Invalid("request id overflow".into()))?;
        let msg = json!({"id": id, "method": method, "params": params});
        write_json(&mut self.reader, &msg)?;
        let deadline = Instant::now() + Duration::from_secs(wire::REPLY_DEADLINE_SECS);
        loop {
            let value = match read_json(&mut self.reader, deadline)? {
                Some(value) => value,
                None => return Err(LinkError::Closed),
            };
            if !value.is_object() {
                return Err(LinkError::Invalid("reply is not an object".into()));
            }
            if id_is(&value, id) {
                return map_reply(&value);
            }
            if value.get("event").is_some() {
                self.events.push_back(parse_event(&value)?);
                continue;
            }
            return Err(LinkError::Invalid("reply id does not match the request".into()));
        }
    }

    /// `link.hello`. Checks `pid` and `instance` against the connected record.
    /// A successful hello is stored and sent again by [`Client::reconnect`].
    pub fn hello(&mut self, identity: &HelloClient) -> Result<Value, LinkError> {
        let result = self.call("link.hello", hello_params(identity))?;
        check_identity(&result, &self.record)?;
        self.hello = Some(identity.clone());
        Ok(result)
    }

    /// `events.subscribe`. Returns `result.seq`.
    pub fn subscribe(&mut self, events: &[&str]) -> Result<u64, LinkError> {
        let result = self.call("events.subscribe", json!({"events": events}))?;
        result.get("seq").and_then(Value::as_u64).ok_or_else(|| LinkError::Invalid("events.subscribe result has no seq".into()))
    }

    /// `events.unsubscribe`.
    pub fn unsubscribe(&mut self) -> Result<(), LinkError> {
        let _result = self.call("events.unsubscribe", json!({}))?;
        Ok(())
    }

    /// The next buffered event, or the next event read before `timeout`.
    /// `None` waits up to [`wire::REPLY_DEADLINE_SECS`]. A zero duration does not read.
    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<Event>, LinkError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        if timeout.is_some_and(|wait| wait.is_zero()) {
            return Ok(None);
        }
        let wait = timeout.unwrap_or(Duration::from_secs(wire::REPLY_DEADLINE_SECS));
        let deadline = Instant::now() + wait;
        if Instant::now() >= deadline {
            return Ok(None);
        }
        match read_json(&mut self.reader, deadline) {
            Ok(None) => Err(LinkError::Closed),
            Ok(Some(value)) => {
                if value.get("event").is_some() {
                    Ok(Some(parse_event(&value)?))
                } else {
                    Err(LinkError::Invalid("expected an event line".into()))
                }
            }
            Err(LinkError::Timeout) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Connect to the stored record again. Does not scan discovery.
    /// Sends auth again. Sends the stored hello when one succeeded.
    pub fn reconnect(&mut self) -> Result<(), LinkError> {
        let reader = open_reader(&self.record)?;
        self.reader = reader;
        self.events.clear();
        self.next_id = 1;
        if let Some(identity) = self.hello.clone() {
            self.hello(&identity)?;
        }
        Ok(())
    }
}

pub(crate) enum Transport {
    Local(Stream),
    Tcp(TcpStream),
}

impl Transport {
    pub(crate) fn arm_recv_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Local(stream) => stream.set_recv_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    pub(crate) fn arm_send_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Local(stream) => stream.set_send_timeout(timeout),
            Self::Tcp(stream) => stream.set_write_timeout(timeout),
        }
    }
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Local(stream) => stream.read(buf),
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Local(stream) => stream.write(buf),
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Local(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

pub(crate) fn open_reader(record: &InstanceRecord) -> Result<BufReader<Transport>, LinkError> {
    let (transport, needs_auth) = open_transport(&record.endpoint)?;
    let mut reader = BufReader::new(transport);
    if needs_auth {
        authenticate(&mut reader, &record.token)?;
    }
    Ok(reader)
}

fn open_transport(endpoint: &Endpoint) -> Result<(Transport, bool), LinkError> {
    match endpoint {
        Endpoint::Unix { path } => Ok((Transport::Local(connect_unix(path)?), true)),
        Endpoint::Pipe { name } => Ok((Transport::Local(connect_pipe(name)?), true)),
        Endpoint::Tcp { port } => {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, *port));
            let wait = Duration::from_secs(wire::REPLY_DEADLINE_SECS);
            match TcpStream::connect_timeout(&addr, wait) {
                Ok(stream) => Ok((Transport::Tcp(stream), true)),
                Err(err) if is_timeout(&err) => Err(LinkError::Timeout),
                Err(err) => Err(err.into()),
            }
        }
    }
}

fn connect_unix(path: &str) -> Result<Stream, LinkError> {
    let name = path.to_fs_name::<GenericFilePath>().map_err(LinkError::io)?;
    ConnectOptions::new().name(name).connect_sync().map_err(LinkError::io)
}

/// Named pipes are the Windows endpoint. Other platforms cannot open one.
/// This returns [`LinkError::Invalid`] instead of pretending the pipe connected.
#[cfg(not(windows))]
fn connect_pipe(name: &str) -> Result<Stream, LinkError> {
    let _ = name;
    Err(LinkError::Invalid("named pipes are a Windows endpoint and are not connected on this platform".into()))
}

#[cfg(windows)]
fn connect_pipe(name: &str) -> Result<Stream, LinkError> {
    use interprocess::local_socket::GenericNamespaced;
    let socket_name = name.to_ns_name::<GenericNamespaced>().map_err(LinkError::io)?;
    ConnectOptions::new().name(socket_name).connect_sync().map_err(LinkError::io)
}

fn authenticate(reader: &mut BufReader<Transport>, token: &str) -> Result<(), LinkError> {
    let request = json!({"id": 0, "method": "auth", "params": {"token": token}});
    write_json(reader, &request)?;
    let deadline = Instant::now() + Duration::from_secs(wire::REPLY_DEADLINE_SECS);
    let expected = wire::auth_success(&json!(0));
    loop {
        let line = match pull_line(reader, deadline) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Err(LinkError::Auth),
            Err(LinkError::Timeout) => return Err(LinkError::Timeout),
            Err(LinkError::TooLarge) => return Err(LinkError::TooLarge),
            Err(LinkError::Io(text)) => return Err(LinkError::Io(text)),
            Err(_) => return Err(LinkError::Auth),
        };
        let Ok(text) = std::str::from_utf8(&line) else {
            return Err(LinkError::Auth);
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Err(LinkError::Auth);
        };
        if value == expected {
            return Ok(());
        }
        return Err(LinkError::Auth);
    }
}

pub(crate) fn write_json(reader: &mut BufReader<Transport>, value: &Value) -> Result<(), LinkError> {
    reader.get_ref().arm_send_timeout(Some(Duration::from_secs(wire::REPLY_DEADLINE_SECS)))?;
    wire::write_line(reader.get_mut(), value)
}

pub(crate) fn read_json(reader: &mut BufReader<Transport>, deadline: Instant) -> Result<Option<Value>, LinkError> {
    loop {
        let Some(bytes) = pull_line(reader, deadline)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|err| LinkError::Invalid(err.to_string()))?;
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        return Ok(Some(serde_json::from_str(text)?));
    }
}

/// Same line limit as [`wire::read_line`]. The io error kind is kept so a
/// socket timeout becomes [`LinkError::Timeout`] instead of a generic io string.
fn pull_line(reader: &mut BufReader<Transport>, deadline: Instant) -> Result<Option<Vec<u8>>, LinkError> {
    let max = wire::MAX_LINE;
    let limit = u64::try_from(max).map_err(|_| LinkError::TooLarge)?.saturating_add(1);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LinkError::Timeout);
        }
        reader.get_ref().arm_recv_timeout(Some(remaining))?;
        let mut buf = Vec::new();
        match reader.take(limit).read_until(b'\n', &mut buf) {
            Ok(0) if buf.is_empty() => return Ok(None),
            Ok(_) => return finish_line(buf, max),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) if is_timeout(&err) => return Err(LinkError::Timeout),
            Err(err) => return Err(err.into()),
        }
    }
}

fn finish_line(mut buf: Vec<u8>, max: usize) -> Result<Option<Vec<u8>>, LinkError> {
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    } else if buf.len() > max {
        return Err(LinkError::TooLarge);
    }
    if buf.len() > max {
        return Err(LinkError::TooLarge);
    }
    Ok(Some(buf))
}

fn is_timeout(err: &std::io::Error) -> bool {
    if matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) {
        return true;
    }
    // Linux EAGAIN 11, ETIMEDOUT 110. macOS EAGAIN 35, ETIMEDOUT 60.
    // Windows ERROR_SEM_TIMEOUT 121, WAIT_TIMEOUT 258.
    matches!(err.raw_os_error(), Some(11 | 35 | 60 | 110 | 121 | 258))
}

fn hello_params(identity: &HelloClient) -> Value {
    json!({
        "client": {
            "app": identity.app,
            "app_version": identity.app_version,
            "pid": identity.pid,
            "instance": identity.instance,
        },
        "protocol": {"min": identity.protocol_min, "max": identity.protocol_max},
    })
}

fn check_identity(result: &Value, record: &InstanceRecord) -> Result<(), LinkError> {
    let got_pid = result.get("pid");
    if got_pid.and_then(Value::as_u64) != Some(u64::from(record.pid)) {
        return Err(LinkError::Mismatch {
            field: "pid".into(),
            expected: record.pid.to_string(),
            got: got_pid.map(json_text).unwrap_or_else(|| "missing".into()),
        });
    }
    let got_instance = result.get("instance");
    if got_instance.and_then(Value::as_str) != Some(record.instance.as_str()) {
        return Err(LinkError::Mismatch {
            field: "instance".into(),
            expected: record.instance.clone(),
            got: got_instance.map(json_text).unwrap_or_else(|| "missing".into()),
        });
    }
    Ok(())
}

fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn id_is(value: &Value, id: u64) -> bool {
    value.get("id").and_then(Value::as_u64) == Some(id)
}

fn parse_event(value: &Value) -> Result<Event, LinkError> {
    let event = value.get("event").and_then(Value::as_str).ok_or_else(|| LinkError::Invalid("event line has no event name".into()))?;
    let seq = value.get("seq").and_then(Value::as_u64).ok_or_else(|| LinkError::Invalid("event line has no seq".into()))?;
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    Ok(Event { event: event.to_string(), seq, data })
}

pub(crate) fn map_reply(value: &Value) -> Result<Value, LinkError> {
    let Some(ok) = value.get("ok").and_then(Value::as_bool) else {
        return Err(LinkError::Invalid("reply has no ok flag".into()));
    };
    if ok {
        return value.get("result").cloned().ok_or_else(|| LinkError::Invalid("reply has no result".into()));
    }
    match value.get("error") {
        Some(Value::String(text)) => Err(LinkError::Invalid(text.clone())),
        Some(Value::Object(map)) => map_error_object(map),
        Some(_) => Err(LinkError::Invalid("error is not a string or an object".into())),
        None => Err(LinkError::Invalid("reply has no error".into())),
    }
}

fn map_error_object(map: &Map<String, Value>) -> Result<Value, LinkError> {
    let code = map.get("code").and_then(Value::as_str).unwrap_or("");
    let message = map.get("message").and_then(Value::as_str).filter(|text| !text.is_empty()).unwrap_or(code).to_string();
    match code {
        "protocol_unsupported" => {
            let data = map.get("data").and_then(Value::as_object);
            let server_min = protocol_bound(data, "server_min")?;
            let server_max = protocol_bound(data, "server_max")?;
            Err(LinkError::Protocol { server_min, server_max })
        }
        "auth_required" => Err(LinkError::Auth),
        "bad_params" => Err(LinkError::BadParams(message)),
        "payload_too_large" => Err(LinkError::TooLarge),
        "payload_invalid" => Err(LinkError::PayloadInvalid(message)),
        "media_missing" => {
            let paths = map
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get("missing"))
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            Err(LinkError::MediaMissing { paths })
        }
        "not_ready" => Err(LinkError::NotReady(message)),
        "link_unknown" => Err(LinkError::Unknown(message)),
        "busy" => Err(LinkError::Busy(message)),
        _ => Err(LinkError::Invalid(if message.is_empty() { "unknown error".into() } else { message })),
    }
}

fn protocol_bound(data: Option<&Map<String, Value>>, key: &str) -> Result<u32, LinkError> {
    let Some(value) = data.and_then(|map| map.get(key)).and_then(Value::as_u64) else {
        return Err(LinkError::Invalid(format!("protocol_unsupported is missing {key}")));
    };
    u32::try_from(value).map_err(|_| LinkError::Invalid(format!("{key} does not fit in u32")))
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::{BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    use super::*;

    struct Peer {
        path: PathBuf,
        record: InstanceRecord,
        join: Option<JoinHandle<()>>,
    }

    impl Drop for Peer {
        fn drop(&mut self) {
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn sock_path(label: &str) -> PathBuf {
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).unwrap();
        PathBuf::from(format!("/tmp/cl-{label}-{}", crate::auth::encode_hex(&bytes)))
    }

    fn peer<F>(label: &str, script: F) -> Peer
    where
        F: FnOnce(UnixStream, InstanceRecord) + Send + 'static,
    {
        let path = sock_path(label);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let endpoint = Endpoint::Unix { path: path.to_str().unwrap().to_string() };
        let record = InstanceRecord::new("effectcraft", "0.8.3", 4242, endpoint).unwrap();
        let moved = record.clone();
        let join = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
            script(stream, moved);
        });
        Peer { path, record, join: Some(join) }
    }

    fn read_req(reader: &mut BufReader<UnixStream>) -> Value {
        let bytes = crate::wire::read_line(reader, crate::wire::MAX_LINE).unwrap().expect("peer eof");
        serde_json::from_slice(&bytes).unwrap()
    }

    fn write_val(reader: &mut BufReader<UnixStream>, value: &Value) {
        crate::wire::write_line(reader.get_mut(), value).unwrap();
    }

    fn accept_auth(reader: &mut BufReader<UnixStream>, token: &str) {
        let req = read_req(reader);
        assert_eq!(req["method"], "auth");
        assert_eq!(req["id"], json!(0));
        assert_eq!(req["params"]["token"], token);
        // Key order differs from serde_json's BTreeMap on purpose.
        reader.get_mut().write_all(b"{\"result\":{\"authenticated\":true},\"ok\":true,\"id\":0}\n").unwrap();
        reader.get_mut().flush().unwrap();
    }

    #[test]
    fn auth_then_call_parses_the_result() {
        let server = peer("call", |stream, record| {
            let mut reader = BufReader::new(stream);
            accept_auth(&mut reader, &record.token);
            let req = read_req(&mut reader);
            assert_eq!(req["method"], "engine.execute");
            assert_eq!(req["id"], json!(1));
            assert_eq!(req["params"]["command"], "ping");
            let id = req["id"].clone();
            write_val(&mut reader, &crate::wire::reply_ok(&id, json!({"pong": true})));
        });
        let mut client = Client::connect(&server.record).unwrap();
        let result = client.call("engine.execute", json!({"command": "ping"})).unwrap();
        assert_eq!(result, json!({"pong": true}));
    }

    #[test]
    fn event_line_before_the_reply_is_returned_by_next_event() {
        let server = peer("ev", |stream, record| {
            let mut reader = BufReader::new(stream);
            accept_auth(&mut reader, &record.token);
            let req = read_req(&mut reader);
            let id = req["id"].clone();
            write_val(&mut reader, &json!({"event": "doc.changed", "seq": 4, "data": {"doc_id": "abc"}}));
            write_val(&mut reader, &crate::wire::reply_ok(&id, json!({"n": 1})));
        });
        let mut client = Client::connect(&server.record).unwrap();
        assert!(client.next_event(Some(Duration::ZERO)).unwrap().is_none());
        let result = client.call("link.focus", json!({})).unwrap();
        assert_eq!(result, json!({"n": 1}));
        let event = client.next_event(Some(Duration::ZERO)).unwrap().expect("buffered event");
        assert_eq!(event.event, "doc.changed");
        assert_eq!(event.seq, 4);
        assert_eq!(event.data, json!({"doc_id": "abc"}));
        assert!(client.next_event(Some(Duration::ZERO)).unwrap().is_none());
    }

    #[test]
    fn reconnect_sends_auth_again() {
        let path = sock_path("re");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let endpoint = Endpoint::Unix { path: path.to_str().unwrap().to_string() };
        let record = InstanceRecord::new("effectcraft", "0.8.3", 4242, endpoint).unwrap();
        let token = record.token.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut reader = BufReader::new(stream);
                let req = read_req(&mut reader);
                tx.send(req).unwrap();
                write_val(&mut reader, &crate::wire::auth_success(&json!(0)));
            }
        });
        let mut client = Client::connect(&record).unwrap();
        client.reconnect().unwrap();
        drop(client);
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first["method"], "auth");
        assert_eq!(second["method"], "auth");
        assert_eq!(first["params"]["token"], token);
        assert_eq!(second["params"]["token"], token);
        join.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failed_auth_reply_is_auth() {
        let server = peer("bad", |stream, _record| {
            let mut reader = BufReader::new(stream);
            let _req = read_req(&mut reader);
            write_val(&mut reader, &crate::wire::auth_failure());
        });
        let err = match Client::connect(&server.record) {
            Err(err) => err,
            Ok(_client) => panic!("connect should fail auth"),
        };
        assert!(matches!(err, LinkError::Auth), "{err}");
    }

    #[test]
    fn closed_before_auth_reply_is_auth() {
        let server = peer("eof", |stream, _record| {
            let mut reader = BufReader::new(stream);
            let _req = read_req(&mut reader);
            drop(reader);
        });
        let err = match Client::connect(&server.record) {
            Err(err) => err,
            Ok(_client) => panic!("connect should fail when the peer closes"),
        };
        assert!(matches!(err, LinkError::Auth), "{err}");
    }

    #[test]
    fn hello_pid_mismatch_is_mismatch() {
        let server = peer("hi", |stream, record| {
            let mut reader = BufReader::new(stream);
            accept_auth(&mut reader, &record.token);
            let req = read_req(&mut reader);
            assert_eq!(req["method"], "link.hello");
            let id = req["id"].clone();
            let result = json!({
                "app": "effectcraft",
                "app_version": "0.8.3",
                "pid": 4243,
                "instance": record.instance,
                "protocol": 1,
            });
            write_val(&mut reader, &crate::wire::reply_ok(&id, result));
        });
        let mut client = Client::connect(&server.record).unwrap();
        let identity = HelloClient {
            app: "filmcraft".into(),
            app_version: "0.9.1".into(),
            pid: std::process::id(),
            instance: "11111111-1111-4111-8111-111111111111".into(),
            protocol_min: wire::PROTOCOL_MIN,
            protocol_max: wire::PROTOCOL_MAX,
        };
        let err = client.hello(&identity).unwrap_err();
        match err {
            LinkError::Mismatch { field, expected, got } => {
                assert_eq!(field, "pid");
                assert_eq!(expected, "4242");
                assert_eq!(got, "4243");
            }
            other => panic!("expected mismatch, got {other}"),
        }
    }

    #[test]
    fn error_object_and_string_map_onto_link_error() {
        let server = peer("err", |stream, record| {
            let mut reader = BufReader::new(stream);
            accept_auth(&mut reader, &record.token);
            for _ in 0..3 {
                let req = read_req(&mut reader);
                let id = req["id"].clone();
                let method = req["method"].as_str().unwrap().to_string();
                let error = match method.as_str() {
                    "proto" => json!({"code": "protocol_unsupported", "message": "no", "data": {"server_min": 2, "server_max": 5}}),
                    "text" => json!("no such method"),
                    "params" => json!({"code": "bad_params", "message": "missing command", "data": {}}),
                    other => panic!("unexpected method {other}"),
                };
                write_val(&mut reader, &json!({"id": id, "ok": false, "error": error}));
            }
        });
        let mut client = Client::connect(&server.record).unwrap();
        match client.call("proto", json!({})).unwrap_err() {
            LinkError::Protocol { server_min, server_max } => {
                assert_eq!(server_min, 2);
                assert_eq!(server_max, 5);
            }
            other => panic!("{other}"),
        }
        match client.call("text", json!({})).unwrap_err() {
            LinkError::Invalid(text) => assert_eq!(text, "no such method"),
            other => panic!("{other}"),
        }
        match client.call("params", json!({})).unwrap_err() {
            LinkError::BadParams(text) => assert_eq!(text, "missing command"),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn next_event_times_out_when_the_peer_is_quiet() {
        let server = peer("wait", |stream, record| {
            let mut reader = BufReader::new(stream);
            accept_auth(&mut reader, &record.token);
            std::thread::sleep(Duration::from_millis(800));
        });
        let mut client = Client::connect(&server.record).unwrap();
        let started = Instant::now();
        let got = client.next_event(Some(Duration::from_millis(300))).unwrap();
        let elapsed = started.elapsed();
        assert!(got.is_none());
        assert!(elapsed >= Duration::from_millis(200), "returned too fast: {elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "waited too long: {elapsed:?}");
    }
}
