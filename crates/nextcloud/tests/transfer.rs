use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use craft_nextcloud::testing::{FakeServer, Fault};
use craft_nextcloud::{Client, Error, RemotePath};

fn client(server: &FakeServer) -> Client {
    Client::new(server.account(), FakeServer::PASSWORD.into(), Arc::new(rustls_rustcrypto::provider())).unwrap()
}

fn path(client: &Client, name: &str) -> RemotePath {
    client.account().root().join(name).unwrap()
}

fn thousand() -> Vec<u8> {
    (0..1000).map(|i| i as u8).collect()
}

fn download(client: &Client, p: &RemotePath) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    client.download_to(p, &mut out, &AtomicBool::new(false), &mut |_, _| {}).map(|_| out)
}

#[test]
fn preview_bytes_and_missing() {
    let server = FakeServer::start();
    server.add_file("/a b.jpg", vec![1]);
    server.add_file("/other.jpg", vec![1]);
    server.set_preview("/a b.jpg", b"\xFF\xD8jpeg".to_vec());
    let client = client(&server);
    assert_eq!(client.preview(&path(&client, "a b.jpg"), 256).unwrap(), Some(b"\xFF\xD8jpeg".to_vec()));
    assert_eq!(client.preview(&path(&client, "other.jpg"), 256).unwrap(), None);
}

#[test]
fn range_returns_slice() {
    let server = FakeServer::start();
    server.add_file("/f", thousand());
    let client = client(&server);
    assert_eq!(client.read_range(&path(&client, "f"), 10..20).unwrap(), thousand()[10..20]);
}

#[test]
fn range_past_eof_returns_short_slice() {
    let server = FakeServer::start();
    server.add_file("/f", thousand());
    let client = client(&server);
    assert_eq!(client.read_range(&path(&client, "f"), 990..2000).unwrap(), thousand()[990..]);
    assert!(client.read_range(&path(&client, "f"), 2000..3000).unwrap().is_empty());
}

#[test]
fn empty_range_sends_nothing_and_huge_is_too_large() {
    let server = FakeServer::start();
    server.add_file("/f", thousand());
    let client = client(&server);
    assert!(client.read_range(&path(&client, "f"), 5..5).unwrap().is_empty());
    assert_eq!(server.requests(), 0);
    assert!(matches!(client.read_range(&path(&client, "f"), 0..(64 << 20) + 1), Err(Error::TooLarge)));
    assert_eq!(server.requests(), 0);
}

#[test]
fn server_ignoring_range_still_works() {
    let server = FakeServer::start();
    server.add_file("/f", thousand());
    let client = client(&server);
    server.fail_next(Fault::IgnoreRange);
    assert_eq!(client.read_range(&path(&client, "f"), 10..20).unwrap(), thousand()[10..20]);
    // Past the end of the file the full body is shorter than the range start.
    server.fail_next(Fault::IgnoreRange);
    assert!(client.read_range(&path(&client, "f"), 2000..2010).unwrap().is_empty());
}

#[test]
fn ignored_range_far_into_a_file_is_malformed() {
    let server = FakeServer::start();
    server.add_file("/f", thousand());
    let client = client(&server);
    server.fail_next(Fault::IgnoreRange);
    let start = (64 << 20) + 1;
    assert!(matches!(client.read_range(&path(&client, "f"), start..start + 10), Err(Error::Malformed(_))));
}

#[test]
fn hostile_content_range() {
    let server = FakeServer::start();
    server.add_file("/f", thousand());
    let client = client(&server);
    let read = |range, header| {
        server.fail_next(Fault::ContentRange(header));
        client.read_range(&path(&client, "f"), range)
    };
    // Reversed, another start, unparsable.
    for (range, header) in [(100..200, "bytes 100-49/50"), (10..20, "bytes 11-20/1000"), (10..20, "bytes nonsense")] {
        assert!(matches!(read(range, header), Err(Error::Malformed(_))), "{header}");
    }
    // Declares 20 bytes; the body (Content-Length) holds the 10 the file has.
    assert!(matches!(read(990..1010, "bytes 990-1009/2000"), Err(Error::Unreachable(_))));
    // Declares 5 bytes of a 10-byte body: only those are returned.
    assert_eq!(read(10..20, "bytes 10-14/15").unwrap(), thousand()[10..15]);
}

#[test]
fn download_streams_and_reports_progress() {
    let server = FakeServer::start();
    let bytes: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
    server.add_file("/big", bytes.clone());
    let client = client(&server);
    let mut out = Vec::new();
    let mut last = None;
    let n = client.download_to(&path(&client, "big"), &mut out, &AtomicBool::new(false), &mut |d, t| last = Some((d, t))).unwrap();
    assert_eq!(n, 1_048_576);
    assert_eq!(out, bytes);
    assert_eq!(last, Some((1_048_576, Some(1_048_576))));
}

#[test]
fn truncated_download_fails() {
    let server = FakeServer::start();
    server.add_file("/f", vec![7; 100_000]);
    let client = client(&server);
    server.fail_next(Fault::Truncate);
    assert!(matches!(download(&client, &path(&client, "f")), Err(Error::Unreachable(_))));
}

#[test]
fn cancelled_download() {
    let server = FakeServer::start();
    server.add_file("/f", vec![7; 1 << 20]);
    let client = client(&server);
    let cancel = AtomicBool::new(false);
    let mut out = Vec::new();
    let res = client.download_to(&path(&client, "f"), &mut out, &cancel, &mut |_, _| cancel.store(true, Relaxed));
    assert!(matches!(res, Err(Error::Cancelled)));
    assert!(out.len() < 1 << 20);
}

#[test]
fn writer_errors_are_io() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let server = FakeServer::start();
    server.add_file("/f", vec![7; 100]);
    let client = client(&server);
    let res = client.download_to(&path(&client, "f"), &mut Broken, &AtomicBool::new(false), &mut |_, _| {});
    assert!(matches!(res, Err(Error::Io(_))));
}

#[test]
fn missing_download_is_not_found() {
    let server = FakeServer::start();
    let client = client(&server);
    assert!(matches!(download(&client, &path(&client, "nope")), Err(Error::NotFound)));
}

#[test]
fn stalled_download_trips_stall_limit() {
    let server = FakeServer::start();
    server.add_file("/f", vec![7; 100_000]);
    let client = client(&server).with_stall_limit(Duration::from_millis(300)).unwrap();
    server.fail_next(Fault::Stall);
    let start = Instant::now();
    assert!(matches!(download(&client, &path(&client, "f")), Err(Error::Unreachable(_))));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn previews_reuse_one_connection() {
    let server = FakeServer::start();
    server.add_file("/p.jpg", vec![1]);
    server.set_preview("/p.jpg", vec![9; 10]);
    let client = client(&server);
    for _ in 0..10 {
        assert!(client.preview(&path(&client, "p.jpg"), 128).unwrap().is_some());
    }
    assert_eq!((server.connections(), server.requests()), (1, 10));
}

#[test]
fn eight_workers_keep_their_connections() {
    let server = FakeServer::start();
    server.add_file("/p.jpg", vec![1]);
    server.set_preview("/p.jpg", vec![9; 10]);
    let client = client(&server);
    let p = path(&client, "p.jpg");
    let round = Barrier::new(8);
    thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                for _ in 0..20 {
                    round.wait();
                    assert!(client.preview(&p, 128).unwrap().is_some());
                }
            });
        }
    });
    assert_eq!(server.requests(), 160);
    assert!(server.connections() <= 8, "{} connections", server.connections());
}

#[test]
fn downloads_count_only_whole_file_gets() {
    let server = FakeServer::start();
    server.add_file("/p.jpg", thousand());
    server.set_preview("/p.jpg", vec![9; 10]);
    let client = client(&server);
    let p = path(&client, "p.jpg");
    client.list(&client.account().root()).unwrap();
    client.stat(&p).unwrap();
    client.preview(&p, 128).unwrap();
    client.read_range(&p, 0..10).unwrap();
    assert_eq!(server.downloads(), 0);
    download(&client, &p).unwrap();
    assert_eq!((server.downloads(), server.requests()), (1, 5));
}

#[test]
fn create_uploads_a_new_file_and_never_replaces_one() {
    let server = FakeServer::start();
    server.add_dir("/Photos");
    server.add_file("/Photos/taken.dng", vec![1]);
    let client = client(&server);
    let root = client.account().root();
    let new = root.join("Photos").unwrap().join("a b-HDR.dng").unwrap();
    assert!(client.create(&new, &thousand()).unwrap());
    assert_eq!(server.file("/Photos/a b-HDR.dng"), Some(thousand()));
    assert_eq!(client.stat(&new).unwrap().size, 1000);
    assert!(!client.create(&new, b"other").unwrap(), "taken now");
    assert!(!client.create(&root.join("Photos").unwrap().join("taken.dng").unwrap(), b"other").unwrap());
    assert_eq!(server.file("/Photos/a b-HDR.dng"), Some(thousand()));
    assert_eq!(server.file("/Photos/taken.dng"), Some(vec![1]));
}

#[test]
fn create_errors() {
    let server = FakeServer::start();
    let client = client(&server);
    let root = client.account().root();
    assert!(matches!(client.create(&root.join("gone").unwrap().join("x.dng").unwrap(), b"x"), Err(Error::NotFound)));
    for (status, why) in [(403, "no permission"), (413, "larger than the server accepts"), (507, "out of space")] {
        server.fail_next(Fault::Status(status));
        match client.create(&path(&client, "x.dng"), b"x") {
            Err(e @ Error::Refused(_)) => assert!(e.to_string().contains(why), "{status}: {e}"),
            other => panic!("{status}: {other:?}"),
        }
    }
}
