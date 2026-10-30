//! `FrameFetcher` against a real server with a fake `FrameSource`: cache, deadline, dedupe,
//! parallel connections, and a peer that goes away.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use craft_ipc::bulk::{FrameFormat, FrameLayout, FrameRequest, FrameSource, FrameSpec, FrameTickets, RenderedFrame};
use craft_ipc::fetch::{FetchOptions, Fetched, FrameFetcher, FrameKey};
use craft_ipc::server::{Activate, Activation, ControlSink, Frames, Server, ServerConfig};
use craft_ipc::{InstanceRecord, LinkError, RuntimeDir};

const W: u32 = 4;
const H: u32 = 3;
const STRIDE: u32 = 16;

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct NoSink;

impl ControlSink for NoSink {
    fn submit(&self, _method: String, _params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        None
    }
}

struct NoActivate;

impl Activate for NoActivate {
    fn activate(&self, _token: Option<String>, _from_pid: Option<u32>, _open: Option<Value>) -> Activation {
        Activation::No
    }
}

/// The first pixel byte is the frame number asked for, so a wrong frame cannot match by accident.
struct Fake {
    epoch: AtomicU64,
    delay_ms: AtomicU64,
    renders: AtomicUsize,
    times: Mutex<Vec<f64>>,
    fail_time: Mutex<Option<f64>>,
}

impl FrameSource for Fake {
    fn open(&self, _spec: &FrameSpec) -> Result<FrameLayout, LinkError> {
        Ok(FrameLayout { w: W, h: H, stride: STRIDE, len: u64::from(STRIDE * H) })
    }

    fn render(&self, spec: &FrameSpec, request: &FrameRequest) -> Result<RenderedFrame, LinkError> {
        self.renders.fetch_add(1, Ordering::SeqCst);
        self.times.lock().unwrap().push(request.time_s);
        let delay = self.delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        if *self.fail_time.lock().unwrap() == Some(request.time_s) {
            return Err(LinkError::NotReady("comp is busy".into()));
        }
        let mut pixels = vec![0u8; (STRIDE * H) as usize];
        pixels[0] = request.time_s as u8;
        Ok(RenderedFrame {
            epoch: self.epoch.load(Ordering::SeqCst).max(request.epoch),
            w: W,
            h: H,
            stride: STRIDE,
            format: spec.format,
            render_ms: 1.0,
            pixels,
        })
    }

    fn close(&self, _spec: &FrameSpec) {}
}

struct Rig {
    _tmp: Tmp,
    server: Option<Server>,
    record: InstanceRecord,
    fake: Arc<Fake>,
}

fn boot() -> Rig {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = PathBuf::from(format!("/tmp/cl-fch-{:08x}", (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x9E37)));
    let _ = std::fs::remove_dir_all(&path);
    let fake = Arc::new(Fake {
        epoch: AtomicU64::new(1),
        delay_ms: AtomicU64::new(0),
        renders: AtomicUsize::new(0),
        times: Mutex::new(Vec::new()),
        fail_time: Mutex::new(None),
    });
    let tickets = Arc::new(FrameTickets::new(Arc::clone(&fake) as Arc<dyn FrameSource>));
    let server = Server::start(ServerConfig {
        app: "ec".into(),
        app_version: "0.8.3".into(),
        instance: None,
        runtime: Some(RuntimeDir::at(&path).unwrap()),
        tcp_port: None,
        sink: Arc::new(NoSink),
        activate: Arc::new(NoActivate),
        frames: tickets as Arc<dyn Frames>,
    })
    .unwrap();
    let record = server.record().clone();
    Rig { _tmp: Tmp(path), server: Some(server), record, fake }
}

fn connect(rig: &Rig, connections: usize) -> FrameFetcher {
    let options = FetchOptions { connections, io_timeout: Duration::from_secs(2), ..FetchOptions::default() };
    FrameFetcher::connect(&rig.record, "link-a", FrameFormat::Rgba8Premul, 1.0, options).unwrap()
}

const fn key(frame: i64) -> FrameKey {
    FrameKey { frame, scale_milli: 1000 }
}

const WAIT: Duration = Duration::from_secs(2);

#[test]
fn fetch_returns_the_frame_and_the_second_ask_is_a_cache_hit() {
    let rig = boot();
    let fetcher = connect(&rig, 1);
    let layout = fetcher.layout();
    assert_eq!((layout.w, layout.h, layout.stride), (W, H, STRIDE));
    let Fetched::Frame(first) = fetcher.fetch(key(7), 7.0, 1, WAIT) else { panic!("no frame") };
    assert_eq!(first.pixels[0], 7);
    assert_eq!(first.epoch, 1);
    let Fetched::Frame(second) = fetcher.fetch(key(7), 7.0, 1, WAIT) else { panic!("no frame") };
    assert!(Arc::ptr_eq(&first, &second), "a hit returns the cached frame");
    assert_eq!(rig.fake.renders.load(Ordering::SeqCst), 1);
    let stats = fetcher.stats();
    assert_eq!((stats.hits, stats.renders), (1, 1));
}

#[test]
fn a_late_frame_misses_the_deadline_then_lands_in_the_cache() {
    let rig = boot();
    rig.fake.delay_ms.store(300, Ordering::SeqCst);
    let fetcher = connect(&rig, 1);
    let t0 = Instant::now();
    assert!(matches!(fetcher.fetch(key(3), 3.0, 1, Duration::from_millis(40)), Fetched::TimedOut));
    assert!(t0.elapsed() < Duration::from_millis(200), "the call must not wait for the render: {:?}", t0.elapsed());
    assert!(fetcher.cached(key(3), 1).is_none(), "not there yet");
    let deadline = Instant::now() + Duration::from_secs(3);
    while fetcher.cached(key(3), 1).is_none() {
        assert!(Instant::now() < deadline, "the late frame never reached the cache");
        std::thread::sleep(Duration::from_millis(10));
    }
    let Fetched::Frame(frame) = fetcher.fetch(key(3), 3.0, 1, Duration::from_millis(10)) else { panic!("no frame") };
    assert_eq!(frame.pixels[0], 3);
    assert_eq!(rig.fake.renders.load(Ordering::SeqCst), 1, "the timed-out ask and the retry share one render");
    assert_eq!(fetcher.stats().timeouts, 1);
}

#[test]
fn a_newer_epoch_asked_for_renders_again_and_an_older_cached_one_does_not_count() {
    let rig = boot();
    let fetcher = connect(&rig, 1);
    let Fetched::Frame(old) = fetcher.fetch(key(1), 1.0, 1, WAIT) else { panic!("no frame") };
    assert_eq!(old.epoch, 1);
    rig.fake.epoch.store(5, Ordering::SeqCst);
    // Epoch 1 is still good for a caller that wants epoch 1.
    assert!(fetcher.cached(key(1), 1).is_some());
    assert!(fetcher.cached(key(1), 2).is_none(), "an epoch 1 frame cannot answer a request for epoch 2");
    let Fetched::Frame(fresh) = fetcher.fetch(key(1), 1.0, 2, WAIT) else { panic!("no frame") };
    assert_eq!(fresh.epoch, 5, "the source may answer with a newer epoch");
    assert_eq!(rig.fake.renders.load(Ordering::SeqCst), 2);
    // The newer frame answers an older request too.
    assert!(fetcher.cached(key(1), 1).is_some_and(|f| f.epoch == 5));
}

#[test]
fn two_asks_for_one_key_render_once() {
    let rig = boot();
    rig.fake.delay_ms.store(120, Ordering::SeqCst);
    let fetcher = Arc::new(connect(&rig, 2));
    let a = {
        let fetcher = Arc::clone(&fetcher);
        std::thread::spawn(move || matches!(fetcher.fetch(key(9), 9.0, 1, WAIT), Fetched::Frame(_)))
    };
    let b = {
        let fetcher = Arc::clone(&fetcher);
        std::thread::spawn(move || matches!(fetcher.fetch(key(9), 9.0, 1, WAIT), Fetched::Frame(_)))
    };
    assert!(a.join().unwrap() && b.join().unwrap());
    assert_eq!(rig.fake.renders.load(Ordering::SeqCst), 1);
}

#[test]
fn four_connections_render_four_frames_at_once() {
    let rig = boot();
    rig.fake.delay_ms.store(200, Ordering::SeqCst);
    let fetcher = Arc::new(connect(&rig, 4));
    let t0 = Instant::now();
    let handles: Vec<_> = (0..4)
        .map(|n| {
            let fetcher = Arc::clone(&fetcher);
            std::thread::spawn(move || matches!(fetcher.fetch(key(n), n as f64, 1, WAIT), Fetched::Frame(_)))
        })
        .collect();
    for handle in handles {
        assert!(handle.join().unwrap());
    }
    let took = t0.elapsed();
    assert!(took < Duration::from_millis(600), "four 200 ms renders on four connections took {took:?}");
}

#[test]
fn a_frame_error_fails_that_frame_and_the_fetcher_keeps_working() {
    let rig = boot();
    *rig.fake.fail_time.lock().unwrap() = Some(4.0);
    let fetcher = connect(&rig, 1);
    match fetcher.fetch(key(4), 4.0, 1, WAIT) {
        Fetched::Failed(message) => assert!(message.contains("comp is busy"), "{message}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(!fetcher.is_down());
    assert!(matches!(fetcher.fetch(key(5), 5.0, 1, WAIT), Fetched::Frame(_)));
}

#[test]
fn a_peer_that_goes_away_makes_the_fetcher_down_and_asks_return_at_once() {
    let mut rig = boot();
    let fetcher = connect(&rig, 2);
    assert!(matches!(fetcher.fetch(key(1), 1.0, 1, WAIT), Fetched::Frame(_)));
    rig.server.take().unwrap().shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fetcher.is_down() {
        // A miss makes the workers touch the dead socket.
        let _ = fetcher.fetch(key(100), 100.0, 1, Duration::from_millis(50));
        assert!(Instant::now() < deadline, "the fetcher never noticed the peer is gone");
    }
    let t0 = Instant::now();
    assert!(matches!(fetcher.fetch(key(200), 200.0, 1, WAIT), Fetched::Down));
    assert!(t0.elapsed() < Duration::from_millis(100));
    // Frames already in the cache still answer.
    assert!(matches!(fetcher.fetch(key(1), 1.0, 1, WAIT), Fetched::Frame(_)));
}

#[test]
fn the_cache_stays_inside_its_byte_budget() {
    let rig = boot();
    let options = FetchOptions { connections: 1, cache_bytes: usize::try_from(STRIDE * H).unwrap() * 3, ..FetchOptions::default() };
    let fetcher = FrameFetcher::connect(&rig.record, "link-a", FrameFormat::Rgba8Premul, 1.0, options).unwrap();
    for n in 0..6 {
        assert!(matches!(fetcher.fetch(key(n), n as f64, 1, WAIT), Fetched::Frame(_)));
    }
    let kept = (0..6).filter(|n| fetcher.cached(key(*n), 1).is_some()).count();
    assert_eq!(kept, 3, "a budget of three frames keeps three");
    assert!(fetcher.cached(key(5), 1).is_some(), "the newest frame stays");
}

#[test]
fn queued_asks_nobody_waits_for_are_dropped_when_old() {
    let rig = boot();
    rig.fake.delay_ms.store(150, Ordering::SeqCst);
    let options = FetchOptions { connections: 1, max_age: Duration::from_millis(100), ..FetchOptions::default() };
    let fetcher = FrameFetcher::connect(&rig.record, "link-a", FrameFormat::Rgba8Premul, 1.0, options).unwrap();
    // Five asks with no patience: frame 0 starts at once, frames 1 to 4 wait behind it.
    for n in 0..5 {
        assert!(matches!(fetcher.fetch(key(n), n as f64, 1, Duration::from_millis(1)), Fetched::TimedOut));
    }
    std::thread::sleep(Duration::from_millis(900));
    let renders = rig.fake.renders.load(Ordering::SeqCst);
    assert!(renders < 5, "frames that waited longer than max_age must be skipped, got {renders} renders");
}
