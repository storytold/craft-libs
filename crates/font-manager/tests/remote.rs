//! The preview service client against the shared test vectors and a local fake service.
//!
//! `data/Tiny-glyf.woff2` is `crates/fonts-build/tests/fixture/subset/Tiny-glyf.ttf` compressed
//! with fontTools 4.67.0: `fonttools ttLib.woff2 compress -o Tiny-glyf.woff2 Tiny-glyf.ttf`.
//! `data/preview-vectors.json` is shared with the preview service, unchanged, so the client's
//! URLs and the service's cache keys can't drift.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use craft_font_manager::craft_font_catalog::Os;
use craft_font_manager::remote::{Codepoints, DiskCache, Instance, MAX_CODEPOINTS, Preview, PreviewError, PreviewKey, Previews, SampleHash, Subset};
use craft_font_manager::store::{FakeStore, Installer, SuiteStore};
use craft_font_manager::{Catalog, FontEnv};
use craft_font_store::{Client, is_font};

const WOFF2: &[u8] = include_bytes!("data/Tiny-glyf.woff2");

fn sha(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn temp(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("remote").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn canonical_codepoints_match_the_shared_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("data/preview-vectors.json")).unwrap();
    let vectors = vectors["vectors"].as_array().unwrap();
    assert!(vectors.len() > 10);
    for v in vectors {
        let text = v["text"].as_str().unwrap();
        let expected = v["codepoints"].as_str().unwrap();
        let codepoints = Codepoints::from_text(text);
        assert_eq!(codepoints.encode(), expected, "{text:?}");
        let key = PreviewKey::new(&sha(0xab), Instance::List, Subset::Codepoints(codepoints));
        match key {
            Some(key) => assert_eq!(key.path(), format!("v1/faces/{}/list/u/{expected}.woff2", sha(0xab))),
            None => assert_eq!(expected, "", "only the empty set has no URL"),
        }
    }
}

#[test]
fn subset_urls() {
    let key = |inst, subset| PreviewKey::new(&sha(0x0a), inst, subset).map(|k| k.url("https://fonts.example.com/"));
    let base = format!("https://fonts.example.com/v1/faces/{}", sha(0x0a));
    assert_eq!(key(Instance::List, Subset::Sample(SampleHash::of("Hello"))), Some(format!("{base}/list/sample-185f8db32271.woff2")));
    assert_eq!(key(Instance::List, Subset::Ascii), Some(format!("{base}/list/ascii.woff2")));
    assert_eq!(key(Instance::Var, Subset::Sample(SampleHash::of("Hello"))), Some(format!("{base}/var/sample-185f8db32271.woff2")));
    assert_eq!(key(Instance::Var, Subset::Codepoints(Codepoints::from_text("ab"))), Some(format!("{base}/var/u/61-62.woff2")));

    let upper = PreviewKey::new(&"AB".repeat(32), Instance::List, Subset::Sample(SampleHash::of("Hello"))).unwrap();
    assert_eq!(upper.sha256(), "ab".repeat(32));
    assert!(PreviewKey::new("../../etc", Instance::List, Subset::Sample(SampleHash::of("Hello"))).is_none());
    assert!(PreviewKey::new(&"g".repeat(64), Instance::List, Subset::Sample(SampleHash::of("Hello"))).is_none());
    let many: Codepoints = ('\u{4e00}'..).take(MAX_CODEPOINTS + 1).collect();
    assert!(PreviewKey::new(&sha(1), Instance::List, Subset::Codepoints(many.clone())).is_none());
    let mut fits = many;
    fits.retain(|c| c != '\u{4e00}');
    assert!(PreviewKey::new(&sha(1), Instance::List, Subset::Codepoints(fits)).is_some());
}

#[test]
fn disk_cache_drops_the_least_recently_used() {
    let dir = temp("cache");
    let cache = DiskCache::new(&dir, 300);
    let bytes = |b: u8| vec![b; 100];
    cache.put("a", &bytes(1));
    cache.put("b", &bytes(2));
    cache.put("c", &bytes(3));
    assert_eq!(cache.get("a"), Some(bytes(1)));
    cache.put("d", &bytes(4));
    assert!(!cache.contains("b"), "b was used least recently");
    assert!(cache.contains("a") && cache.contains("c") && cache.contains("d"));
    assert_eq!(cache.size(), 300);
    assert_eq!(cache.get("b"), None);

    // Larger than the whole cache: not kept, and nothing else goes for it.
    cache.put("e", &vec![5; 301]);
    assert!(!cache.contains("e") && cache.contains("a"));

    // Another session reads the same entries, and a smaller limit trims them at once.
    std::fs::write(dir.join(".tmp-1-1"), b"half").unwrap();
    let files = || std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".woff2")).count();
    assert_eq!(files(), 3);
    let again = DiskCache::new(&dir, 300);
    assert_eq!(again.size(), 300);
    assert_eq!(again.get("d"), Some(bytes(4)));
    let smaller = DiskCache::new(&dir, 200);
    assert_eq!(smaller.size(), 200);
    assert_eq!(files(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// What the fake service answers for a path.
#[derive(Clone)]
enum Reply {
    Ok(Vec<u8>),
    /// Waits for [`Service::release`], then answers with the fixture.
    Held,
}

/// A local HTTP server standing in for the preview service.
struct Service {
    base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    log: Arc<Mutex<Vec<String>>>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Service {
    fn start() -> Service {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let service = Service { base, routes: Arc::default(), log: Arc::default(), gate: Arc::default() };
        let (routes, log, gate) = (service.routes.clone(), service.log.clone(), service.gate.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (routes, log, gate) = (routes.clone(), log.clone(), gate.clone());
                std::thread::spawn(move || answer(stream, &routes, &log, &gate));
            }
        });
        service
    }

    fn serve(&self, key: &PreviewKey, reply: Reply) {
        self.routes.lock().unwrap().insert(format!("/{}", key.path()), reply);
    }

    fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn release(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
}

fn answer(mut s: TcpStream, routes: &Mutex<HashMap<String, Reply>>, log: &Mutex<Vec<String>>, gate: &(Mutex<bool>, Condvar)) {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut b) {
            Ok(1) => head.push(b[0]),
            _ => return,
        }
    }
    let path = String::from_utf8_lossy(&head).split_whitespace().nth(1).unwrap_or("/").to_string();
    log.lock().unwrap().push(path.clone());
    let reply = routes.lock().unwrap().get(&path).cloned();
    let body = match reply {
        Some(Reply::Ok(body)) => body,
        Some(Reply::Held) => {
            let mut open = gate.0.lock().unwrap();
            while !*open {
                open = gate.1.wait(open).unwrap();
            }
            WOFF2.to_vec()
        }
        None => {
            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return;
        }
    };
    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: font/woff2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(&body);
}

/// The results until `n` have come, or two seconds.
fn wait_for(previews: &Previews, n: usize) -> Vec<Preview> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut out = Vec::new();
    while out.len() < n && Instant::now() < deadline {
        out.extend(previews.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    out
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn previews_come_from_the_service_then_the_cache_and_only_with_consent() {
    let service = Service::start();
    let dir = temp("service");
    let store = Arc::new(FakeStore::new(&dir, HashMap::new()));
    let sample = PreviewKey::new(&sha(1), Instance::List, Subset::Sample(SampleHash::of("Hello"))).unwrap();
    let broken = PreviewKey::new(&sha(2), Instance::List, Subset::Sample(SampleHash::of("Hello"))).unwrap();
    service.serve(&sample, Reply::Ok(WOFF2.to_vec()));
    service.serve(&broken, Reply::Ok(b"wOF2 but not really".to_vec()));
    let previews = Previews::new(store.clone());

    // No service: nothing happens.
    assert!(!previews.available());
    previews.request(sample.clone(), 1);
    let got = wait_for(&previews, 1);
    assert_eq!(got.first().map(|p| p.font.clone().unwrap_err()), Some(PreviewError::NoService));

    // A service, but previews off: no request.
    store.set_preview_service(Some(&service.base)).unwrap();
    store.set_previews_enabled(false).unwrap();
    assert!(!previews.available());
    previews.request(sample.clone(), 1);
    let got = wait_for(&previews, 1);
    assert_eq!(got.first().map(|p| p.font.clone().unwrap_err()), Some(PreviewError::PreviewsOff));
    assert!(service.requests().is_empty());

    // Previews on (the default), installs not allowed: decoded, checked and cached anyway.
    store.set_previews_enabled(true).unwrap();
    assert!(previews.available() && !store.downloads_allowed());
    previews.request(sample.clone(), 1);
    previews.request(broken.clone(), 1);
    let got = wait_for(&previews, 2);
    let font = |key: &PreviewKey| got.iter().find(|p| &p.key == key).map(|p| p.font.clone());
    assert!(font(&sample).unwrap().is_ok_and(|f| is_font(&f)));
    assert_eq!(font(&broken), Some(Err(PreviewError::NotAFont)));
    assert_eq!(service.requests().len(), 2);
    let cache = previews.cache().unwrap();
    assert!(cache.dir().starts_with(&dir) && cache.contains(&sample.path()) && !cache.contains(&broken.path()));

    // A new session reads the disk cache, even with previews off.
    store.set_previews_enabled(false).unwrap();
    let later = Previews::new(store.clone());
    later.request(sample.clone(), 1);
    assert!(wait_for(&later, 1).first().is_some_and(|p| p.font.as_ref().is_ok_and(|f| is_font(f))));
    assert_eq!(service.requests().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_suite_store_and_its_previews_share_the_service_down_state() {
    let root = temp("suite");
    let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(root.join("store")), ..FontEnv::default() };
    let store = SuiteStore::for_user(Arc::new(Catalog::new(env).with_font_service(false))).unwrap();
    let client = store.http_client().unwrap();
    let svc = "https://fonts.example.com";
    assert!(!client.service_is_down(svc) && !store.client().service_is_down(svc));
    client.mark_service_down(svc);
    assert!(store.client().service_is_down(svc));
    // The pool's client has its own timeouts and the same down state.
    let pool_client = client.with_timeouts(craft_font_store::Timeouts::default());
    assert!(pool_client.service_is_down(svc));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_service_marked_down_gets_no_requests() {
    let service = Service::start();
    let dir = temp("down");
    let store = Arc::new(FakeStore::new(&dir, HashMap::new()));
    store.set_preview_service(Some(&service.base)).unwrap();
    store.allow_downloads(true).unwrap();
    let key = PreviewKey::new(&sha(1), Instance::List, Subset::Sample(SampleHash::of("Hello"))).unwrap();
    service.serve(&key, Reply::Ok(WOFF2.to_vec()));
    // The client is shared with, say, the store's installs, which found the service unreachable.
    let client = Client::default();
    client.mark_service_down(&service.base);
    let previews = Previews::new(store.clone()).with_fetcher(Arc::new(client));
    previews.request(key, 1);
    let got = wait_for(&previews, 1);
    assert!(matches!(got.first().map(|p| p.font.clone()), Some(Err(PreviewError::Failed(_)))));
    assert!(service.requests().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn highest_priority_first_and_cancelled_requests_deliver_nothing() {
    let service = Service::start();
    let dir = temp("cancel");
    let store = Arc::new(FakeStore::new(&dir, HashMap::new()));
    store.allow_downloads(true).unwrap();
    store.set_preview_service(Some(&service.base)).unwrap();
    let keys: Vec<PreviewKey> = (1..=4).map(|n| PreviewKey::new(&sha(n), Instance::List, Subset::Sample(SampleHash::of("Hello"))).unwrap()).collect();
    let [held, low, high, dropped] = [&keys[0], &keys[1], &keys[2], &keys[3]];
    service.serve(held, Reply::Held);
    for key in [low, high, dropped] {
        service.serve(key, Reply::Ok(WOFF2.to_vec()));
    }
    let previews = Previews::new(store.clone()).with_cache(None).with_workers(1);

    previews.request(held.clone(), 9);
    wait_until(|| !service.requests().is_empty());
    previews.request(low.clone(), 1);
    previews.request(dropped.clone(), 1);
    previews.request(high.clone(), 5);
    assert_eq!(previews.pending(), 4);
    previews.cancel(dropped);
    assert!(!previews.is_pending(dropped));
    // The running request stops while the service still sends nothing.
    previews.cancel(held);

    let got = wait_for(&previews, 2);
    assert_eq!(got.iter().map(|p| &p.key).collect::<Vec<_>>(), [high, low]);
    let path = |k: &PreviewKey| format!("/{}", k.path());
    assert_eq!(service.requests(), [path(held), path(high), path(low)]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(previews.poll().is_empty());
    assert_eq!(previews.pending(), 0);
    service.release();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sample_hash_is_the_sha256_prefix_of_the_exact_text() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("data/preview-vectors.json")).unwrap();
    let vectors = vectors["sample_hash"]["vectors"].as_array().unwrap();
    assert!(vectors.len() >= 4);
    for v in vectors {
        let text = v["text"].as_str().unwrap();
        assert_eq!(SampleHash::of(text).as_str(), v["h"].as_str().unwrap(), "{text:?}");
    }
    assert_ne!(SampleHash::of("Hello\n"), SampleHash::of("Hello"));
    let a = PreviewKey::new(&sha(1), Instance::List, Subset::Sample(SampleHash::of("a"))).unwrap();
    let b = PreviewKey::new(&sha(1), Instance::List, Subset::Sample(SampleHash::of("b"))).unwrap();
    assert_ne!(a.path(), b.path(), "the disk cache key names the text");
}

#[test]
fn a_404_comes_back_as_not_found() {
    let service = Service::start();
    let dir = temp("notfound");
    let store = Arc::new(FakeStore::new(&dir, HashMap::new()));
    store.set_preview_service(Some(&service.base)).unwrap();
    let key = PreviewKey::new(&sha(1), Instance::List, Subset::Sample(SampleHash::of("Hello"))).unwrap();
    let previews = Previews::new(store.clone());
    previews.request(key, 1);
    let got = wait_for(&previews, 1);
    assert_eq!(got.first().map(|p| p.font.clone().unwrap_err()), Some(PreviewError::NotFound));
    let _ = std::fs::remove_dir_all(dir);
}
