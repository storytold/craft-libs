//! `Http::new` as an app gets it, through the public API only.

use craft_ui::testing::trial;
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

#[trial]
fn the_remote_agents_refuse_plain_http_and_loopback_links_use_the_direct_agents() {
    let (base, seen) = serve();
    let http = Http::new("craft-update-tests/0").unwrap();
    // A loopback link goes to the direct agents, which serve plain HTTP in every build.
    let api = http.get_api(&format!("{base}/repos/storytold/x/releases"), None);
    assert!(matches!(api, Ok(Fetched::Body { .. })), "{api:?}");
    let redirect = http.redirect_target(&format!("{base}/releases/latest"));
    assert!(matches!(redirect, Err(ref e) if e.contains("HTTP 200")), "{redirect:?}");
    assert_eq!(http.get_text(&format!("{base}/SHA256SUMS.txt")).unwrap().as_deref(), Some("{}"));
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    // Any other plain-HTTP link is refused before it connects, in debug builds too. An upper-case scheme isn't a
    // loopback link, so it goes to a remote agent.
    assert!(http.get_text(&format!("HTTP{}", &base[4..])).is_err_and(|e| e.contains("refused plain HTTP")));
    assert_eq!(seen.load(Ordering::SeqCst), 3, "a refused request never connects");
}
