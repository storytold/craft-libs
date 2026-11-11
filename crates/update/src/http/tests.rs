//! Tests for the update HTTP client.

use super::*;
use craft_ui::testing::trial;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// How the fake server answers one request.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reply {
    /// The whole file.
    Full,
    /// Promise the whole file, send `n` bytes, hang up.
    DropAfter(usize),
    /// Honour `Range` with a 206.
    Ranged,
    /// The right length, wrong bytes.
    Corrupt,
    /// The whole body, already a gzip stream, announced as `Content-Encoding: gzip`.
    Gzipped,
}

/// Serves `body` over plain HTTP following `script`, one reply per connection; records the
/// `Range` header each request carried.
pub(crate) fn serve(body: Vec<u8>, script: Vec<Reply>) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let seen = ranges.clone();
    std::thread::spawn(move || {
        for reply in script {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut range = None;
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                    range = v.trim().trim_end_matches('-').parse::<usize>().ok();
                }
            }
            seen.lock().unwrap().push(range.map(|r| r.to_string()));
            let len = body.len();
            let _ = match reply {
                Reply::Full => {
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n").and_then(|()| stream.write_all(&body))
                }
                Reply::DropAfter(n) => write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
                    .and_then(|()| stream.write_all(&body[..n])),
                Reply::Corrupt => write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
                    .and_then(|()| stream.write_all(&vec![b'x'; len])),
                Reply::Gzipped => write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\n")
                    .and_then(|()| stream.write_all(&body)),
                Reply::Ranged => {
                    let from = range.unwrap_or(0);
                    write!(
                        stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{len}\r\nConnection: close\r\n\r\n",
                        len - from,
                        len - 1
                    )
                    .and_then(|()| stream.write_all(&body[from..]))
                }
            };
        }
    });
    (url, ranges)
}

pub(crate) fn body() -> (Vec<u8>, String) {
    let body: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    let sha = hex(&Sha256::digest(&body));
    (body, sha)
}

fn run(script: Vec<Reply>) -> (Result<Vec<u8>, String>, Vec<Option<String>>) {
    let (body, sha) = body();
    let (url, ranges) = serve(body.clone(), script);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let mut last = (0, 0);
    let result = http.download(&url, &dest, body.len() as u64, &sha, &Stop::default(), &mut |t| {
        if let Transfer::Bytes { done, total } = t {
            last = (done, total);
        }
    });
    let result = result.map(|()| std::fs::read(&dest).unwrap());
    if result.is_ok() {
        assert_eq!(last, (body.len() as u64, body.len() as u64));
    }
    let seen = ranges.lock().unwrap().clone();
    (result, seen)
}

#[trial]
fn a_dropped_connection_resumes_where_it_stopped() {
    let (result, ranges) = run(vec![Reply::DropAfter(300_000), Reply::Ranged]);
    assert_eq!(result.unwrap(), body().0);
    assert_eq!(ranges, [None, Some("300000".to_string())]);
}

#[trial]
fn a_server_that_ignores_ranges_starts_over() {
    let (result, _) = run(vec![Reply::DropAfter(123_456), Reply::Full]);
    assert_eq!(result.unwrap(), body().0);
}

#[trial]
fn corrupt_data_is_fetched_again_and_never_kept() {
    let (result, _) = run(vec![Reply::Corrupt, Reply::Full]);
    assert_eq!(result.unwrap(), body().0);
    let (result, _) = run(vec![Reply::Corrupt, Reply::Corrupt]);
    assert!(result.unwrap_err().contains("checksum"));
}

#[trial]
fn a_gzip_bomb_answer_is_refused_before_parsing() {
    // A few hundred bytes on the wire that decode to more than MAX_SMALL. ureq's own limit
    // sits below the transparent gzip decoder, so only a cap on the decoded stream (CWE-409)
    // refuses this; a wire-byte limit would hand the parser truncated JSON.
    let big = vec![b'x'; (MAX_SMALL as usize) + 1024];
    let gz = gzip(&big);
    assert!(gz.len() * 4 < MAX_SMALL as usize, "no longer a bomb: {} compressed bytes", gz.len());
    let (url, _) = serve(gz, vec![Reply::Gzipped]);
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let err = http.get_text(&url).unwrap_err();
    assert!(err.contains("larger than"), "{err}");
    // A small gzip answer still decodes intact.
    let (url, _) = serve(gzip(b"{\"ok\":true}"), vec![Reply::Gzipped]);
    assert_eq!(http.get_text(&url).unwrap().unwrap(), "{\"ok\":true}");
}

/// `data` as a real gzip stream (what `Reply::Gzipped` serves).
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}
