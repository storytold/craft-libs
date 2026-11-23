//! `ControlChannel` deadlines and the loopback server's limits: connections, request size, authentication.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use craft_automation::client::ControlClient;
use craft_automation::control::{ControlChannel, ControlRequest, ControlResponse, Reply, expired_error, result_reply, timeout_error};
use craft_automation::protocol::MAX_RESPONSE_BYTES;
use craft_automation::security::{CHALLENGE_METHOD, MAX_CONNECTIONS, MAX_REQUEST_BYTES, ProofRole, generate_nonce, handshake_proof, proof_matches};
use craft_automation::server;
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::trial;
use egui::Context;
use serde_json::{Value, json};

/// An expired queued request is replied `expired_error` and never dispatched; a dispatched one that times out gets
/// `timeout_error`.
#[trial]
fn deadlines_distinguish_expired_from_timed_out() {
    let (tx, rx) = channel();
    let mut channel = ControlChannel::new(rx);

    let (mut stale, stale_reply) = ControlRequest::new("ui.inspect", json!({}));
    stale.deadline = Some(motion::now() - Duration::from_secs(1));
    let stale_dispatch = stale.dispatch.clone();
    let (live, _live_reply) = ControlRequest::new("ui.key", json!({}));
    let live_dispatch = live.dispatch.clone();
    tx.send(stale).expect("sent");
    tx.send(live).expect("sent");

    let next = channel.next_request().expect("the live request is dispatched");
    assert_eq!(next.method, "ui.key");
    assert!(channel.next_request().is_none());
    assert_eq!(stale_reply.try_recv().expect("a reply"), json!({"ok": false, "error": expired_error()}));
    assert_eq!(stale_dispatch.give_up(), expired_error());
    assert_eq!(live_dispatch.give_up(), timeout_error());
    assert!(timeout_error().starts_with("timeout:") && expired_error().starts_with("timeout:"));

    // A request the transport abandoned before the UI thread claimed it is not run.
    let (abandoned, abandoned_reply) = ControlRequest::new("ui.click", json!({}));
    let _ = abandoned.dispatch.give_up();
    tx.send(abandoned).expect("sent");
    assert!(channel.next_request().is_none());
    assert_eq!(abandoned_reply.try_recv().expect("a reply")["error"], json!(expired_error()));
}

/// A request sent without a deadline runs even when nobody waits for its reply any more: a fire-and-forget step
/// (a key press, a release) still happens.
#[trial]
fn a_request_without_a_deadline_is_dispatched_after_its_reply_receiver_is_dropped() {
    let (tx, rx) = channel();
    let mut channel = ControlChannel::new(rx);

    let (request, reply) = ControlRequest::new("ui.key", json!({"key": "Escape"}));
    assert!(request.deadline.is_none(), "a new request has no deadline");
    drop(reply);
    tx.send(request).expect("sent");

    let next = channel.next_request().expect("the deadline-free request is dispatched");
    assert_eq!(next.method, "ui.key");
    assert_eq!(next.params, json!({"key": "Escape"}));
    assert!(channel.next_request().is_none(), "the request runs once");
}

/// A job's reply goes to the requests waiting on that job only, as the app built it (an error with partial results
/// keeps both); the other job's waiter keeps waiting.
#[trial]
fn a_finished_job_answers_only_its_own_waiters() {
    let (_tx, rx) = channel();
    let mut channel = ControlChannel::new(rx);
    let (first, first_reply) = ControlRequest::new("engine.execute", json!({}));
    let (second, second_reply) = ControlRequest::new("engine.execute", json!({}));
    channel.settle(first, Reply::AfterJob(1), None, 0.0);
    channel.settle(second, Reply::AfterJob(2), None, 0.0);

    let partial = json!({"ok": false, "error": "cancelled", "result": {"files": ["done.png"]}});
    channel.job_finished(1, &partial);
    assert_eq!(first_reply.try_recv().expect("job 1 replied"), partial);
    assert!(second_reply.try_recv().is_err(), "job 2 still runs");
    assert_eq!(channel.waiting(), 1);

    channel.job_finished(2, &result_reply(Ok(json!(7))));
    assert_eq!(second_reply.try_recv().expect("job 2 replied"), json!({"ok": true, "result": 7}));
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn read_json(reader: &mut impl BufRead) -> Option<Value> {
    let mut line = String::new();
    (reader.read_line(&mut line).ok()? > 0).then(|| serde_json::from_str(&line).expect("a JSON line"))
}

fn host() -> WindowHost {
    WindowHost::of(&Context::default())
}

/// A connection that keeps sending without authenticating is closed at the deadline, whatever it sends, and nothing it
/// sent is dispatched.
#[trial]
fn an_unauthenticated_connection_is_closed_at_the_deadline_whatever_it_sends() {
    // A short deadline, so the test does not wait out the real one.
    let deadline = Duration::from_millis(400);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bound");
    let (rx, server) = server::serve_with_deadline(listener, TOKEN.into(), host(), deadline).expect("started");
    // Blank lines and partial frames, one byte every 20 ms: each read succeeds long before the idle timeout.
    let mut stream = TcpStream::connect(server.addr).expect("connected");
    let started = motion::now();
    let mut closed = false;
    for i in 0..1000 {
        let byte: &[u8] = if i % 5 == 0 { b"\n" } else { b" " };
        if stream.write_all(byte).is_err() {
            closed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let served = started.elapsed();
    assert!(closed, "the trickle did not keep the connection open");
    assert!(served < deadline + Duration::from_secs(3), "the trickle did not extend the deadline: {served:?}");
    assert!(rx.try_recv().is_err(), "nothing was dispatched");
}

/// Whether the server closed the connection: the next read is the end of the stream, not a timeout or an error.
fn is_closed(reader: &mut impl BufRead) -> bool {
    let mut line = String::new();
    matches!(reader.read_line(&mut line), Ok(0))
}

/// With `MAX_CONNECTIONS` connections open, the next one is told `connection limit reached` and closed; once one of
/// the open ones ends, its place serves a new connection.
#[trial]
fn connections_past_the_limit_are_refused_with_the_reason() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let connect = || {
        let stream = TcpStream::connect(server.addr).expect("connected");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        let reader = BufReader::new(stream.try_clone().expect("cloned"));
        (stream, reader)
    };
    // Unauthenticated, so each holds its place until it closes or the authentication deadline passes.
    let mut open: Vec<TcpStream> = (0..MAX_CONNECTIONS).map(|_| connect().0).collect();

    let (_extra, mut reader) = connect();
    let refused = read_json(&mut reader).expect("a refusal");
    assert_eq!(refused, json!({"id": null, "ok": false, "error": "connection limit reached"}));
    assert!(is_closed(&mut reader), "the refused connection is closed");

    drop(open.pop());
    // The server frees the place when it reads the close; a new connection is served once it has.
    let started = motion::now();
    let served = loop {
        let (mut stream, mut reader) = connect();
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).expect("written");
        let reply = read_json(&mut reader).expect("a reply");
        if reply["ok"] == true || started.elapsed() > Duration::from_secs(5) {
            break reply;
        }
        assert_eq!(reply["error"], "connection limit reached", "{reply}");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(served["result"]["authenticated"], true, "the freed place serves: {served}");
    assert!(rx.try_recv().is_err(), "nothing was dispatched");
    drop(open);
}

/// A first frame that is not JSON, and one with a wrong token, each get the `authentication required` refusal and the
/// connection closes at once, without waiting for the deadline and without dispatching anything: a client cannot keep
/// guessing on one connection.
#[trial]
fn a_refused_first_frame_closes_the_connection() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let wrong = "f".repeat(TOKEN.len());
    for (first, id) in [("not json".to_owned(), Value::Null), (json!({"id": 4, "method": "auth", "params": {"token": wrong}}).to_string(), json!(4))]
    {
        let mut stream = TcpStream::connect(server.addr).expect("connected");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
        writeln!(stream, "{first}").expect("written");
        assert_eq!(read_json(&mut reader).expect("a refusal"), json!({"id": id, "ok": false, "error": "authentication required"}), "{first}");
        // The server may already have closed while this is written.
        let _ = writeln!(stream, "{}", json!({"id": 5, "method": "auth", "params": {"token": TOKEN}}));
        assert!(is_closed(&mut reader), "{first}: the connection is closed, the second guess unanswered");
    }
    assert!(rx.try_recv().is_err(), "nothing was dispatched");
}

/// A web page can reach loopback: a browser `POST` whose body is a valid `auth` line is refused at its request line
/// and the connection closed, so neither the body's `auth` nor a request after it reaches the app.
#[trial]
fn a_browser_post_never_reaches_the_app() {
    // Far longer than the trial, so the close comes from the refusal, not the deadline.
    let (rx, server) = server::serve_with_deadline(TcpListener::bind("127.0.0.1:0").expect("bound"), TOKEN.into(), host(), Duration::from_secs(60))
        .expect("started");
    let mut stream = TcpStream::connect(server.addr).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
    let body = format!("{}\n{}\n", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}}), json!({"id": 2, "method": "test.ping"}));
    write!(stream, "POST / HTTP/1.1\r\n").expect("written");
    let refused = read_json(&mut reader).expect("a reply to the request line");
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"], "authentication required", "{refused}");
    // The rest of what the browser sends; the server may already have closed.
    let _ = write!(stream, "Host: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = stream.flush();
    let mut rest = String::new();
    let closed = match reader.read_line(&mut rest) {
        Ok(read) => read == 0,
        Err(e) => !matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut),
    };
    assert!(closed, "the connection closes after the refusal, answering nothing more: {rest:?}");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "nothing reached the app");
}

/// A request line over `MAX_REQUEST_BYTES` without a newline gets an error reply and the connection is closed.
#[trial]
fn an_over_long_request_is_refused_and_closes_the_connection() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let mut stream = TcpStream::connect(server.addr).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
    writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).expect("written");
    assert_eq!(read_json(&mut reader).expect("a reply")["ok"], true);
    // The server may close while this is still being written.
    let _ = stream.write_all(&vec![b'x'; MAX_REQUEST_BYTES + 16]);
    let reply = read_json(&mut reader).expect("a reply");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().expect("an error").starts_with("request exceeds"), "{reply}");
    assert!(read_json(&mut reader).is_none(), "the connection is closed");
    assert!(rx.try_recv().is_err());
}

/// Answers the next request, which must be `test.ping`, with `pong`.
fn answer_one_ping(rx: Receiver<ControlRequest>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let req = rx.recv_timeout(Duration::from_secs(5)).expect("a request");
        assert_eq!(req.method, "test.ping");
        let pong: ControlResponse = json!({"ok": true, "result": "pong"});
        req.reply.send(pong).expect("replied");
    })
}

/// Authenticates with the one-frame token, sends `test.ping` and returns its reply.
fn ping(addr: SocketAddr) -> Value {
    let mut stream = TcpStream::connect(addr).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
    writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).expect("written");
    assert_eq!(read_json(&mut reader).expect("a reply")["ok"], true);
    writeln!(stream, "{}", json!({"id": 2, "method": "test.ping"})).expect("written");
    read_json(&mut reader).expect("a reply")
}

/// A server on port 0 reports the port it bound, serves, and after it is dropped the port binds again at once; dropping
/// one server leaves another one serving.
#[trial]
fn a_listening_server_stops_when_dropped_and_frees_its_port() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let (other_rx, other) = server::listen(0, TOKEN.into(), host()).expect("started");
    let addr = server.addr;
    assert_ne!(addr.port(), 0);
    let handler = answer_one_ping(rx);
    assert_eq!(ping(addr), json!({"id": 2, "ok": true, "result": "pong"}));
    handler.join().expect("handler ran");

    drop(server);
    TcpListener::bind(addr).expect("the port is free once the server is dropped");

    let handler = answer_one_ping(other_rx);
    assert_eq!(ping(other.addr), json!({"id": 2, "ok": true, "result": "pong"}));
    handler.join().expect("handler ran");
}

/// A connection that authenticates with the challenge handshake learns the server holds the token, never sends it,
/// and then has its requests dispatched.
#[trial]
fn a_challenge_authenticated_connection_is_served() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let handler = answer_one_ping(rx);
    let mut stream = TcpStream::connect(server.addr).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
    let client_nonce = generate_nonce().expect("a nonce");
    writeln!(stream, "{}", json!({"id": 1, "method": CHALLENGE_METHOD, "params": {"clientNonce": client_nonce}})).expect("written");
    let challenge = read_json(&mut reader).expect("a reply");
    let server_nonce = challenge["result"]["serverNonce"].as_str().expect("a server nonce").to_owned();
    let server_proof = challenge["result"]["serverProof"].as_str().expect("a server proof");
    assert!(proof_matches(TOKEN, ProofRole::Server, &client_nonce, &server_nonce, server_proof), "{challenge}");
    let proof = handshake_proof(TOKEN, ProofRole::Client, &client_nonce, &server_nonce).expect("a proof");
    writeln!(stream, "{}", json!({"id": 2, "method": "auth", "params": {"proof": proof}})).expect("written");
    assert_eq!(read_json(&mut reader).expect("a reply")["result"]["authenticated"], true);
    writeln!(stream, "{}", json!({"id": 3, "method": "test.ping"})).expect("written");
    assert_eq!(read_json(&mut reader).expect("a reply"), json!({"id": 3, "ok": true, "result": "pong"}));
    handler.join().expect("handler ran");
}

/// Sends one state of a states file and saves its capture as `<label>-<id>.png`: the client sample of
/// docs/porting.md, section 6.4.
fn capture_state(client: &mut ControlClient, doc: &Value, state: &Value, label: &str) -> Result<Value, String> {
    let set = |client: &mut ControlClient, method: &str, params: Value| client.call(method, params).map(|_| ());
    set(client, "ui.resize", doc["size"].clone())?;
    set(client, "ui.set", json!({"theme": doc["theme"]}))?;
    for step in state["steps"].as_array().into_iter().flatten() {
        client.call(step["method"].as_str().ok_or("a step needs a method")?, step["params"].clone())?;
    }
    let id = state["id"].as_str().ok_or("a state needs an id")?;
    client.call("ui.screenshot", json!({"path": format!("{label}-{id}.png")}))
}

/// The documented states-file client sends the set-up requests and each step in order, then asks for the capture,
/// against a server and a method table built as the guide shows.
#[trial]
fn the_documented_capture_client_sends_setup_steps_and_capture() {
    let (requests, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let addr = server.addr;
    let doc = json!({
        "size": {"width": 1280, "height": 800},
        "theme": "pro",
        "states": [{"id": "pencil", "title": "Pencil tool", "steps": [{"method": "ui.set", "params": {"tool": "Pencil"}}]}],
    });
    let client_doc = doc.clone();
    let client = std::thread::spawn(move || {
        let mut client = ControlClient::connect(addr, TOKEN).expect("connected");
        capture_state(&mut client, &client_doc, &client_doc["states"][0], "new")
    });
    let mut channel = ControlChannel::new(requests);
    let mut seen: Vec<String> = Vec::new();
    let started = motion::now();
    while seen.len() < 4 && started.elapsed() < Duration::from_secs(10) {
        while let Some(req) = channel.next_request() {
            seen.push(format!("{} {}", req.method, req.params));
            let reply = match req.method.as_str() {
                "ui.screenshot" => Reply::from_result(Ok(json!({"path": req.params["path"], "width": 1280, "height": 800}))),
                _ => Reply::from_result(Ok(Value::Null)),
            };
            channel.settle(req, reply, None, 0.0);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let reply = client.join().expect("client ran").expect("the capture replied");
    assert_eq!(
        seen,
        [
            r#"ui.resize {"height":800,"width":1280}"#,
            r#"ui.set {"theme":"pro"}"#,
            r#"ui.set {"tool":"Pencil"}"#,
            r#"ui.screenshot {"path":"new-pencil.png"}"#,
        ]
    );
    assert_eq!(reply, json!({"path": "new-pencil.png", "width": 1280, "height": 800}));
}

/// Connects and authenticates with the one-frame token.
fn authenticated(addr: SocketAddr) -> (TcpStream, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(addr).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
    writeln!(stream, "{}", json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}})).expect("written");
    assert_eq!(read_json(&mut reader).expect("a reply")["ok"], true);
    (stream, reader)
}

/// A reply over `MAX_RESPONSE_BYTES` becomes an error reply with the request's id on its own line, and the next
/// request on the connection is still served.
#[trial]
fn an_oversized_reply_keeps_its_framing_and_id_and_the_connection_serves_on() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let handler = std::thread::spawn(move || {
        let first = rx.recv_timeout(Duration::from_secs(5)).expect("a request");
        assert_eq!(first.method, "test.large");
        assert!(first.deadline.is_some_and(|d| d > motion::now()), "native requests carry their reply deadline");
        first.reply.send(json!({"ok": true, "result": "x".repeat(MAX_RESPONSE_BYTES)})).expect("replied");
        let second = rx.recv_timeout(Duration::from_secs(5)).expect("a request");
        assert_eq!(second.method, "test.small");
        second.reply.send(json!({"ok": true, "result": "still serving"})).expect("replied");
    });
    let (mut stream, mut reader) = authenticated(server.addr);
    writeln!(stream, "{}", json!({"id": 2, "method": "test.large"})).expect("written");
    let rejected = read_json(&mut reader).expect("a reply");
    assert_eq!(rejected["id"], 2);
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"].as_str().expect("an error").contains("operation may have completed"), "{rejected}");
    writeln!(stream, "{}", json!({"id": 3, "method": "test.small"})).expect("written");
    let accepted = read_json(&mut reader).expect("a reply");
    assert_eq!(accepted["id"], 3);
    assert_eq!(accepted["result"], "still serving");
    handler.join().expect("handler ran");
}

/// A request that isn't valid JSON is answered with `id: null` and a `bad JSON` error, and the connection keeps
/// serving.
#[trial]
fn a_bad_json_request_gets_a_null_id_and_the_connection_serves_on() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let handler = std::thread::spawn(move || {
        let req = rx.recv_timeout(Duration::from_secs(5)).expect("a request");
        assert_eq!(req.method, "test.after");
        req.reply.send(json!({"ok": true, "result": "still serving"})).expect("replied");
    });
    let (mut stream, mut reader) = authenticated(server.addr);
    writeln!(stream, "{{not json}}").expect("written");
    let bad = read_json(&mut reader).expect("a reply");
    let bad = bad.as_object().expect("an object");
    assert_eq!(bad.get("id"), Some(&Value::Null), "{bad:?}");
    assert_eq!(bad.get("ok"), Some(&Value::Bool(false)));
    assert!(bad.get("error").and_then(Value::as_str).expect("an error").starts_with("bad JSON: "), "{bad:?}");
    writeln!(stream, "{}", json!({"id": 7, "method": "test.after"})).expect("written");
    let after = read_json(&mut reader).expect("a reply");
    assert_eq!(after["id"], 7);
    assert_eq!(after["result"], "still serving");
    handler.join().expect("handler ran");
}

/// A second `auth` on an authenticated connection is refused as already authenticated without reaching the app, and
/// the connection keeps serving.
#[trial]
fn a_repeated_auth_is_answered_already_authenticated_without_dispatching() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let handler = std::thread::spawn(move || {
        let req = rx.recv_timeout(Duration::from_secs(5)).expect("a request");
        assert_eq!(req.method, "test.after", "a repeated auth must not reach the app");
        req.reply.send(json!({"ok": true, "result": "still serving"})).expect("replied");
    });
    let (mut stream, mut reader) = authenticated(server.addr);
    writeln!(stream, "{}", json!({"id": "again", "method": "auth", "params": {"token": TOKEN}})).expect("written");
    let again = read_json(&mut reader).expect("a reply");
    assert_eq!(again["id"], "again");
    assert_eq!(again["ok"], false);
    assert_eq!(again["error"], "already authenticated");
    writeln!(stream, "{}", json!({"id": 9, "method": "test.after"})).expect("written");
    let after = read_json(&mut reader).expect("a reply");
    assert_eq!(after["id"], 9);
    assert_eq!(after["result"], "still serving");
    handler.join().expect("handler ran");
}

/// Ported from WordCraft (#7317). A request that is complete JSON but ends at EOF with no newline (the client wrote it
/// and shut its end down) still reaches the app and is answered: a client cannot know it was dropped.
#[trial]
fn a_complete_json_request_at_eof_can_reach_the_app() {
    let (rx, server) = server::listen(0, TOKEN.into(), host()).expect("started");
    let handler = std::thread::spawn(move || {
        let req = rx.recv_timeout(Duration::from_secs(5)).expect("the request reached the app");
        assert_eq!(req.method, "test.once");
        req.reply.send(json!({"ok": true, "result": "once"})).expect("replied");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "it ran exactly once");
    });
    let (mut stream, mut reader) = authenticated(server.addr);
    stream.write_all(json!({"id": 3, "method": "test.once"}).to_string().as_bytes()).expect("written");
    stream.shutdown(std::net::Shutdown::Write).expect("shut down");
    let reply = read_json(&mut reader).expect("a reply");
    assert_eq!((&reply["id"], &reply["result"]), (&json!(3), &json!("once")));
    let mut rest = String::new();
    assert_eq!(reader.read_line(&mut rest).expect("closed, not timed out"), 0, "closed after the last request: {rest:?}");
    handler.join().expect("handler ran");
}

/// Ported from WordCraft (#7317). Connections that never authenticate cannot fill the server: past the limit a new
/// one is refused, and a slot freed by a closed connection is admitted again.
#[trial]
fn idle_unauthenticated_connections_are_limited() {
    use craft_automation::security::MAX_CONNECTIONS;
    // Far longer than the trial, so only the limit refuses.
    let (rx, server) = server::serve_with_deadline(TcpListener::bind("127.0.0.1:0").expect("bound"), TOKEN.into(), host(), Duration::from_secs(60))
        .expect("started");
    let idle: Vec<TcpStream> = (0..MAX_CONNECTIONS).map(|_| TcpStream::connect(server.addr).expect("connected")).collect();
    // Admission is counted by the accept thread: it is done once the next connection is answered.
    let refused = |addr| {
        let stream = TcpStream::connect(addr).expect("connected");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        read_json(&mut BufReader::new(stream))
    };
    let reply = refused(server.addr).expect("a refusal");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().is_some_and(|e| e.contains("limit")), "{reply}");
    drop(idle);
    // The freed slots go back as the closed connections are noticed.
    let started = motion::now();
    loop {
        let mut stream = TcpStream::connect(server.addr).expect("connected");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
        // A refused connection may already be closed when the request is written (Windows resets it).
        let auth = json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}});
        if writeln!(stream, "{auth}").is_ok() && read_json(&mut reader).is_some_and(|r| r["ok"] == true) {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no slot came back");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(rx.try_recv().is_err());
}
