//! The client against the shared server ([`crate::server::Hub`]) on an ephemeral loopback port.

use super::*;
use crate::ControlRequest;
use crate::server::{Hub, Protocol, Stop, Summary, Waits as ServerWaits};
use socket2::{Domain, Socket, Type};
use std::cell::RefCell;
use std::ffi::OsString;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn no_dir() -> Option<PathBuf> {
    None
}

fn no_env(_: &str) -> Option<OsString> {
    None
}

fn utc(secs: u64) -> String {
    secs.to_string()
}

/// An app with no token files and no environment: a client sends only the token it is given.
const APP: App = App { name: "Probe", data_dir: no_dir, env: no_env, utc, default_port: 0 };

/// An app whose requests all may change its document.
struct Probe;

impl Protocol for Probe {
    fn summary(&self, method: &str, _: &Value) -> Summary {
        Summary { what: format!("`{method}`"), effect: Effect::Changes, command: None, steps: None, long: false }
    }
    fn document(&self) -> &'static str {
        "design"
    }
    fn change_event(&self) -> &'static str {
        "design.changed"
    }
    fn revision_method(&self) -> &'static str {
        "document.revision"
    }
    fn docs(&self) -> &'static str {
        "docs/control-protocol.md"
    }
    fn capabilities(&self) -> Value {
        json!({})
    }
}

/// The app on `listener` behind [`TOKEN`]: its window answers each request with the request's
/// method and parameters. With a `gate`, it answers each request only once the gate is opened (a
/// message, or the sender dropped), and the server says at once that the request is still running.
fn serve(listener: TcpListener, gate: Option<Receiver<()>>) -> Stop {
    let wait = if gate.is_some() { Duration::ZERO } else { Duration::from_secs(10) };
    let (tx, rx) = channel::<ControlRequest>();
    std::thread::spawn(move || {
        for req in rx {
            if let Some(gate) = &gate {
                let _ = gate.recv();
            }
            let _ = req.reply.send(json!({"ok": true, "result": {"method": req.method, "params": req.params}}));
        }
    });
    let waits = ServerWaits { short: wait, long: wait, token: Duration::from_secs(5) };
    let hub = Arc::new(Hub::new(Probe, || {}, 1 << 20).with_waits(waits));
    hub.listen_stoppable(listener, TOKEN.into(), tx, false).unwrap()
}

/// A listener on an ephemeral loopback port.
fn any_port() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").unwrap()
}

/// `port` bound as [`TcpListener::bind`] binds it, but not listening yet: the port stays held, and
/// a connection to it is refused until [`listen`]. A listener just stopped may still hold the port
/// on a loaded machine ([`Stop::stop`] waits at most a couple of seconds), so the bind is retried
/// for up to two seconds.
fn hold(port: u16) -> Socket {
    let started = Instant::now();
    hold_after(port, || {
        if started.elapsed() >= Duration::from_secs(2) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
        true
    })
}

/// [`hold`], calling `busy` each time the port is still taken; the bind is tried again while
/// `busy` returns true.
fn hold_after(port: u16, mut busy: impl FnMut() -> bool) -> Socket {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port)).into();
    loop {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(socket2::Protocol::TCP)).unwrap();
        #[cfg(not(windows))]
        socket.set_reuse_address(true).unwrap();
        match socket.bind(&address) {
            Ok(()) => return socket,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && busy() => {}
            Err(e) => panic!("binding 127.0.0.1:{port}: {e}"),
        }
    }
}

fn listen(socket: Socket) -> TcpListener {
    socket.listen(128).unwrap();
    socket.into()
}

thread_local! {
    /// Opened by [`open_gate`] when the client first reports a wait.
    static GATE: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
}

/// A client's progress report: the app said its request is still running, so its window may
/// answer now.
fn open_gate(_: &str) {
    if let Some(gate) = GATE.with(|g| g.borrow_mut().take()) {
        let _ = gate.send(());
    }
}

fn addr(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// Keep this port bound without listening, so another normal test listener cannot take it.
fn reserve_non_listening_port() -> (Socket, u16) {
    let reserved = Socket::new(Domain::IPV4, Type::STREAM, Some(socket2::Protocol::TCP)).unwrap();
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    reserved.bind(&address.into()).unwrap();
    let port = reserved.local_addr().unwrap().as_socket().unwrap().port();
    assert_ne!(port, 0);
    (reserved, port)
}

#[test]
fn a_call_is_one_round_trip_with_the_token() {
    let app = serve(any_port(), None);
    let mut c = Connection::new(APP, &addr(app.port()), Some(TOKEN));
    assert!(!c.is_connected());
    let r = c.call(Request::new("document.inspect", json!({"a": 1}))).unwrap();
    assert_eq!(r, json!({"method": "document.inspect", "params": {"a": 1}}));
    assert!(c.is_connected());
    let serial = c.serial();
    // the next call goes on the same connection
    c.call(Request::new("document.inspect", json!({}))).unwrap();
    assert_eq!(c.serial(), serial);
    // `try_connect` sees the app there and leaves no connection open
    let mut fresh = Connection::new(APP, &addr(app.port()), Some(TOKEN));
    fresh.try_connect().unwrap();
    assert!(!fresh.is_connected());
}

#[test]
fn a_wrong_token_or_none_is_refused() {
    let app = serve(any_port(), None);
    for token in [Some("fedcba9876543210fedcba9876543210"), None] {
        let mut c = Connection::new(APP, &addr(app.port()), token);
        match c.call(Request::new("document.inspect", json!({}))) {
            Err(CallError::Refused(e)) => assert!(e.starts_with("unauthorized"), "{token:?}: {e}"),
            other => panic!("{token:?}: {other:?}"),
        }
    }
}

#[test]
fn an_app_not_running_says_how_to_start_it() {
    let (reserved, port) = reserve_non_listening_port();
    let mut c = Connection::new(APP, &addr(port), Some(TOKEN));
    let e = c.call(Request::new("document.inspect", json!({}))).unwrap_err();
    let CallError::NotRunning(m) = &e else { panic!("{e:?}") };
    assert!(m.starts_with(&format!("no Probe app is listening at 127.0.0.1:{port}")), "{m}");
    assert!(m.contains(&format!("`probe --control {port}`")), "{m}");
    assert_eq!(c.try_connect().unwrap_err().kind(), std::io::ErrorKind::NotConnected);
    drop(reserved);
}

#[test]
fn the_not_running_fixture_keeps_its_port_from_another_listener() {
    let (reserved, port) = reserve_non_listening_port();
    let rival = TcpListener::bind(("127.0.0.1", port));
    assert!(rival.is_err(), "another listener acquired the reserved port");
    drop(reserved);
}

/// A port a listener still holds is bound once that listener closes, not refused at the first try.
#[test]
fn holding_a_port_waits_for_the_listener_on_it_to_close() {
    let mut listener = Some(any_port());
    let port = listener.as_ref().unwrap().local_addr().unwrap().port();
    let mut tries = 0;
    let held = hold_after(port, || {
        tries += 1;
        listener.take().is_some()
    });
    assert_eq!(tries, 1, "the port was free while its listener was open");
    assert_eq!(held.local_addr().unwrap().as_socket().unwrap().port(), port);
}

/// An app that goes away during a call is lost, not "not running"; the next call says it is not
/// running, and once it runs again the next call reaches it.
#[test]
fn an_app_gone_during_a_call_is_lost_and_reached_again_once_back() {
    let app = serve(any_port(), None);
    let port = app.port();
    let mut c = Connection::new(APP, &addr(port), Some(TOKEN));
    c.call(Request::new("document.inspect", json!({}))).unwrap();
    app.stop();
    // Held, not listening, until the app is back, so no other listener takes the port meanwhile.
    let held = hold(port);
    let e = c.call(Request::new("document.inspect", json!({}))).unwrap_err();
    let CallError::Lost(m) = &e else { panic!("{e:?}") };
    assert!(m.contains("may have quit") && m.contains("Inspect the design") && m.contains(&format!("`probe --control {port}`")), "{m}");
    assert!(matches!(c.call(Request::new("document.inspect", json!({}))), Err(CallError::NotRunning(_))));
    let _back = serve(listen(held), None);
    assert_eq!(c.call(Request::new("ui.view", json!({}))).unwrap()["method"], json!("ui.view"));
}

/// A server that reads a request and closes the connection without a reply: the call is lost
/// (after one more try on a fresh connection), and the reply is never taken for "not running".
#[test]
fn a_connection_closed_without_a_reply_is_lost() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(stream).read_line(&mut line);
        }
    });
    let mut c = Connection::new(APP, &addr(port), Some(TOKEN));
    let e = c.call(Request::new("document.inspect", json!({}))).unwrap_err();
    assert!(matches!(e, CallError::Lost(_)), "{e:?}");
    assert!(!c.is_connected());
}

/// A request the app says is still running is waited for through `control.pending`, on the same
/// connection, and its own reply is the call's result.
#[test]
fn a_request_still_running_is_waited_for_through_pending() {
    let (gate, held) = channel();
    let app = serve(any_port(), Some(held));
    GATE.with(|g| *g.borrow_mut() = Some(gate));
    let mut c = Connection::new(APP, &addr(app.port()), Some(TOKEN)).with_progress(open_gate);
    c.set_waits(Waits { pending: Duration::from_secs(10), settle: Duration::from_secs(10), poll: Duration::from_millis(5) });
    let r = c.call(Request::new("solid.slow", json!({"n": 2}))).unwrap();
    assert!(GATE.with(|g| g.borrow().is_none()), "the app never said the request was still running");
    assert_eq!(r, json!({"method": "solid.slow", "params": {"n": 2}}));
    assert!(!c.has_outstanding());
    assert_eq!(c.pending(), json!({"pending": null}));
}

/// Past the wait, the call says the request is still running, and `pending` later has its end.
#[test]
fn a_request_still_running_past_the_wait_is_reported_and_its_end_kept() {
    let (gate, held) = channel();
    let app = serve(any_port(), Some(held));
    let mut c = Connection::new(APP, &addr(app.port()), Some(TOKEN));
    c.set_waits(Waits { pending: Duration::ZERO, settle: Duration::from_secs(10), poll: Duration::from_millis(5) });
    let mut slow = Request::new("solid.slow", json!({}));
    slow.what = "`solid.slow` (now)".into();
    let e = c.call(slow).unwrap_err();
    let CallError::StillRunning(m) = &e else { panic!("{e:?}") };
    assert!(m.starts_with(STILL_RUNNING) && m.contains("`solid.slow` (now)") && m.contains("will still apply"), "{m}");
    assert!(c.has_outstanding());
    gate.send(()).unwrap();
    let end = c.pending();
    assert_eq!(end["done"], json!(true), "{end}");
    assert_eq!(end["reply"]["result"]["method"], json!("solid.slow"), "{end}");
}
