//! A host that logs `ureq_proto` at `TRACE` would record tokens: nothing is sent then. Its own
//! test binary, since it installs the process's logger.

// The crate is empty on wasm, like `tests/backend.rs`.
#![cfg(not(target_arch = "wasm32"))]

use craft_cloud_google_drive::Error;
use craft_cloud_google_drive::http::{Method, Request, Transport, Ureq, wire_traced};

struct WireOnly;

impl log::Log for WireOnly {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("ureq_proto")
    }
    fn log(&self, _: &log::Record) {}
    fn flush(&self) {}
}

#[test]
fn nothing_is_sent_while_the_wire_is_traced() {
    assert!(!wire_traced(), "no logger yet");
    log::set_logger(&WireOnly).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    assert!(wire_traced());
    // A port nothing listens on: an answer other than the refusal would mean it tried.
    let req = Request::new(Method::Get, "http://127.0.0.1:9/").bearer("secret-token");
    match Ureq::new("test").send(req) {
        Err(Error::Invalid(why)) => assert!(why.contains("TRACE"), "{why}"),
        other => panic!("sent anyway: {other:?}"),
    }
    log::set_max_level(log::LevelFilter::Debug);
    assert!(!wire_traced(), "DEBUG is redacted by ureq; nothing is refused");
}
