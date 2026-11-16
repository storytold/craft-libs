#![cfg(feature = "native")]

//! `Http::new` as an app gets it: an integration test builds the library without `cfg(test)`, so
//! this is the release configuration (or the `local-mirror` one, with that feature on).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_update::http::{Fetched, Http};

/// A plain-HTTP server on loopback that answers every request with `{}`; counts the requests.
fn serve() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            count.fetch_add(1, Ordering::SeqCst);
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
        }
    });
    (base, seen)
}

#[test]
fn the_remote_agents_take_plain_http_only_with_a_local_mirror() {
    let (base, seen) = serve();
    let http = Http::new("craft-update-tests/0").unwrap();
    // `get_api` and `redirect_target` always use the agents for remote hosts.
    let api = http.get_api(&format!("{base}/repos/storytold/x/releases"), None);
    let redirect = http.redirect_target(&format!("{base}/releases/latest"));
    if cfg!(feature = "local-mirror") {
        assert!(matches!(api, Ok(Fetched::Body { .. })), "{api:?}");
        assert!(matches!(redirect, Err(ref e) if e.contains("HTTP 200")), "{redirect:?}");
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    } else {
        assert!(api.as_ref().is_err_and(|e| e.contains("refused plain HTTP")), "{api:?}");
        assert!(redirect.as_ref().is_err_and(|e| e.contains("refused plain HTTP")), "{redirect:?}");
        // An upper-case scheme isn't a loopback link, so it goes to a remote agent too.
        assert!(http.get_text(&format!("HTTP{}", &base[4..])).is_err_and(|e| e.contains("refused plain HTTP")));
        assert_eq!(seen.load(Ordering::SeqCst), 0, "a refused request never connects");
    }
    // A loopback link goes to the direct agents, which serve plain HTTP in every build.
    assert_eq!(http.get_text(&format!("{base}/SHA256SUMS.txt")).unwrap().as_deref(), Some("{}"));
}
