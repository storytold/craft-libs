//! Scheduled polling. A `ManualTicker` stands in for the clock, so no test
//! sleeps to wait for an interval. Real-time waits only bound a failure.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use craft_ipc::client::{Client, HelloClient};
use craft_ipc::refresh::{Change, FileState, Tracker, write_atomic};
use craft_ipc::server::{Activate, Activation, ControlSink, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::watch::{DEEP_INTERVAL, IntervalTicker, Intervals, ManualTicker, Notice, SHALLOW_INTERVAL, Ticker, Watcher};
use craft_ipc::{Discovery, LinkError, RuntimeDir};

const LIMIT: Duration = Duration::from_secs(10);
const SECOND: Duration = Duration::from_secs(1);

struct Tmp(PathBuf);

impl Tmp {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = PathBuf::from(format!("/tmp/cl-watch-{label}-{}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn forward(tx: Sender<Notice>) -> impl Fn(Notice) + Send + Sync + 'static {
    let tx = Mutex::new(tx);
    move |notice| {
        let _ = tx.lock().unwrap().send(notice);
    }
}

struct Rig {
    _tmp: Tmp,
    file: PathBuf,
    clock: Arc<ManualTicker>,
    notices: Receiver<Notice>,
    watcher: Watcher,
}

impl Rig {
    /// A watcher on `a.png` with content "one" and epoch 4, on the contract intervals.
    fn new(label: &str) -> Self {
        let tmp = Tmp::new(label);
        let file = tmp.0.join("a.png");
        std::fs::write(&file, b"one").unwrap();
        let clock = Arc::new(ManualTicker::new());
        let (tx, notices) = channel();
        let watcher = Watcher::new(Arc::clone(&clock) as Arc<dyn Ticker>, Intervals::default(), forward(tx)).unwrap();
        watcher.watch("link-1", Tracker::new(&file, 4).unwrap()).unwrap();
        Self { _tmp: tmp, file, clock, notices, watcher }
    }

    /// Move time and wait until the watcher thread has handled it.
    fn advance(&self, by: Duration) {
        self.clock.advance(by);
        assert!(self.clock.settle(LIMIT), "the watcher did not settle");
    }

    fn quiet(&self) {
        let got = self.notices.try_recv();
        assert!(got.is_err(), "unexpected notice {got:?}");
    }

    fn next(&self) -> Notice {
        self.notices.try_recv().expect("a notice was due")
    }
}

fn changed(link: &str, state: FileState, epoch: u64, bumped: bool, deep: bool) -> Notice {
    Notice::Changed { link_id: link.into(), change: Change { state, epoch, bumped }, deep }
}

#[test]
fn contract_intervals_are_one_and_thirty_seconds() {
    assert_eq!(SHALLOW_INTERVAL, Duration::from_secs(1));
    assert_eq!(DEEP_INTERVAL, Duration::from_secs(30));
    assert_eq!(Intervals::default(), Intervals { shallow: SHALLOW_INTERVAL, deep: DEEP_INTERVAL });
}

#[test]
fn edit_on_disk_bumps_the_epoch_on_the_first_shallow_tick_and_not_before() {
    let rig = Rig::new("shallow");
    write_atomic(&rig.file, b"two!").unwrap();
    rig.advance(Duration::from_millis(500));
    rig.advance(Duration::from_millis(499));
    rig.quiet();
    assert_eq!(rig.watcher.poll_counts().shallow, 0, "no poll before the 1 s interval");
    rig.advance(Duration::from_millis(1));
    assert_eq!(rig.next(), changed("link-1", FileState::Modified, 5, true, false));
    let counts = rig.watcher.poll_counts();
    assert_eq!((counts.shallow, counts.deep), (1, 0));
    // No further edit, no further notice, and the next poll comes one interval later.
    rig.advance(Duration::from_millis(999));
    assert_eq!(rig.watcher.poll_counts().shallow, 1);
    rig.advance(Duration::from_millis(1));
    assert_eq!(rig.watcher.poll_counts().shallow, 2);
    rig.quiet();
    // A second edit bumps again.
    write_atomic(&rig.file, b"three").unwrap();
    rig.advance(SECOND);
    assert_eq!(rig.next(), changed("link-1", FileState::Modified, 6, true, false));
}

#[test]
fn deep_poll_runs_at_thirty_seconds_and_finds_an_edit_that_keeps_size_and_mtime() {
    let rig = Rig::new("deep");
    let mtime = std::fs::metadata(&rig.file).unwrap().modified().unwrap();
    std::fs::write(&rig.file, b"two").unwrap();
    OpenOptions::new().write(true).open(&rig.file).unwrap().set_modified(mtime).unwrap();
    for _ in 1..30 {
        rig.advance(SECOND);
    }
    rig.quiet();
    let counts = rig.watcher.poll_counts();
    assert_eq!((counts.shallow, counts.deep), (29, 0), "shallow polls are blind to this edit");
    rig.advance(SECOND);
    assert_eq!(rig.next(), changed("link-1", FileState::Modified, 5, true, true));
    let counts = rig.watcher.poll_counts();
    assert_eq!((counts.shallow, counts.deep), (29, 1), "the deep poll replaces the shallow poll on its tick");
    // The schedule goes on: shallow at 31 s, deep again at 60 s.
    rig.advance(SECOND);
    assert_eq!(rig.watcher.poll_counts().shallow, 30);
    rig.advance(Duration::from_secs(29));
    assert_eq!(rig.watcher.poll_counts().deep, 2);
}

#[test]
fn a_large_time_jump_is_one_deep_poll_and_the_grid_stays_aligned() {
    let rig = Rig::new("jump");
    rig.advance(Duration::from_secs(100));
    let counts = rig.watcher.poll_counts();
    assert_eq!((counts.shallow, counts.deep), (0, 1));
    rig.advance(Duration::from_millis(500));
    assert_eq!(rig.watcher.poll_counts().shallow, 0, "100.5 s is before the next grid point at 101 s");
    rig.advance(Duration::from_millis(500));
    assert_eq!(rig.watcher.poll_counts().shallow, 1, "101 s is the next grid point");
}

#[test]
fn producer_offline_missing_and_return_are_reported_without_a_bump() {
    let rig = Rig::new("offline");
    // A touch alone is silent.
    let file = OpenOptions::new().write(true).open(&rig.file).unwrap();
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(60)).unwrap();
    drop(file);
    rig.advance(SECOND);
    rig.quiet();
    std::fs::remove_file(&rig.file).unwrap();
    rig.advance(SECOND);
    assert_eq!(rig.next(), changed("link-1", FileState::Missing, 4, false, false));
    rig.advance(SECOND);
    rig.quiet();
    std::fs::write(&rig.file, b"one").unwrap();
    rig.advance(SECOND);
    assert_eq!(rig.next(), changed("link-1", FileState::Current, 4, false, false));
    rig.quiet();
}

#[test]
fn read_errors_are_reported_once_and_recover() {
    let rig = Rig::new("error");
    std::fs::remove_file(&rig.file).unwrap();
    std::fs::create_dir(&rig.file).unwrap();
    rig.advance(SECOND);
    let Notice::PollFailed { link_id, error } = rig.next() else {
        panic!("expected a failure notice");
    };
    assert_eq!(link_id, "link-1");
    assert!(matches!(error, LinkError::Invalid(_)), "{error:?}");
    rig.advance(SECOND);
    rig.quiet();
    std::fs::remove_dir(&rig.file).unwrap();
    std::fs::write(&rig.file, b"different").unwrap();
    rig.advance(SECOND);
    assert_eq!(rig.next(), changed("link-1", FileState::Modified, 5, true, false));
}

#[test]
fn with_tracker_accept_silences_the_next_notice_and_unwatch_stops_polling() {
    let rig = Rig::new("accept");
    write_atomic(&rig.file, b"two!").unwrap();
    rig.advance(SECOND);
    rig.next();
    let state = rig.watcher.with_tracker("link-1", |tracker| {
        tracker.accept();
        tracker.state()
    });
    assert_eq!(state, Some(FileState::Current));
    rig.advance(SECOND);
    rig.quiet();
    assert!(rig.watcher.with_tracker("nope", |_| ()).is_none());
    assert!(rig.watcher.watch("link-1", Tracker::new(&rig.file, 0).unwrap()).is_err(), "duplicate id");
    let tracker = rig.watcher.unwatch("link-1").unwrap();
    assert_eq!(tracker.epoch(), 5);
    write_atomic(&rig.file, b"after unwatch").unwrap();
    rig.advance(SECOND);
    rig.quiet();
    assert!(rig.watcher.unwatch("link-1").is_none());
}

#[test]
fn several_links_are_polled_together() {
    let rig = Rig::new("many");
    let other = rig._tmp.0.join("b.png");
    std::fs::write(&other, b"b").unwrap();
    rig.watcher.watch("link-2", Tracker::new(&other, 0).unwrap()).unwrap();
    write_atomic(&other, b"bb").unwrap();
    rig.advance(SECOND);
    assert_eq!(rig.next(), changed("link-2", FileState::Modified, 1, true, false));
    rig.quiet();
}

#[test]
fn bad_intervals_are_refused() {
    let clock = Arc::new(ManualTicker::new());
    let zero = Intervals { shallow: Duration::ZERO, deep: SECOND };
    assert!(Watcher::new(Arc::clone(&clock) as Arc<dyn Ticker>, zero, |_| ()).is_err());
    let inverted = Intervals { shallow: Duration::from_secs(5), deep: SECOND };
    assert!(Watcher::new(clock as Arc<dyn Ticker>, inverted, |_| ()).is_err());
    assert!(IntervalTicker::new(Duration::ZERO).is_err());
}

#[test]
fn drop_stops_the_thread_at_once_even_with_a_slow_ticker() {
    let ticker = Arc::new(IntervalTicker::new(Duration::from_secs(3600)).unwrap());
    let watcher =
        Watcher::new(ticker as Arc<dyn Ticker>, Intervals { shallow: Duration::from_secs(3600), deep: Duration::from_secs(7200) }, |_| ()).unwrap();
    let (done, finished) = channel();
    std::thread::spawn(move || {
        drop(watcher);
        let _ = done.send(());
    });
    finished.recv_timeout(LIMIT).expect("drop must not wait for the next tick");

    let clock = Arc::new(ManualTicker::new());
    let watcher = Watcher::new(Arc::clone(&clock) as Arc<dyn Ticker>, Intervals::default(), |_| ()).unwrap();
    assert!(clock.settle(LIMIT));
    drop(watcher);
    assert!(clock.wait().is_none(), "a closed ticker stays closed");
    assert!(!clock.settle(Duration::from_millis(1)));
}

#[test]
fn the_native_ticker_drives_a_watcher_without_a_manual_clock() {
    let tmp = Tmp::new("native");
    let file = tmp.0.join("a.png");
    std::fs::write(&file, b"one").unwrap();
    let (tx, notices) = channel();
    let ticker = Arc::new(IntervalTicker::new(Duration::from_millis(10)).unwrap());
    let intervals = Intervals { shallow: Duration::from_millis(10), deep: Duration::from_millis(50) };
    let watcher = Watcher::new(ticker as Arc<dyn Ticker>, intervals, forward(tx)).unwrap();
    watcher.watch("link-1", Tracker::new(&file, 0).unwrap()).unwrap();
    write_atomic(&file, b"two!").unwrap();
    let got = notices.recv_timeout(LIMIT).expect("a notice within the limit");
    assert_eq!(got, changed("link-1", FileState::Modified, 1, true, false));
    drop(watcher);
}

struct NoSink;

impl ControlSink for NoSink {
    fn submit(&self, _method: String, _params: serde_json::Value) -> Option<std::sync::mpsc::Receiver<serde_json::Value>> {
        None
    }
}

struct NoActivate;

impl Activate for NoActivate {
    fn activate(&self, _token: Option<String>, _from_pid: Option<u32>, _open: Option<serde_json::Value>) -> Activation {
        Activation::No
    }
}

struct NoFrames;

impl Frames for NoFrames {
    fn open(&self, _params: &serde_json::Map<String, serde_json::Value>) -> Result<FrameOpen, LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn render(&self, _ticket: &str, _header: &serde_json::Value) -> Result<(serde_json::Value, Vec<u8>), LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn close_ticket(&self, _ticket: &str) {}
}

#[test]
fn a_disk_edit_reaches_a_subscribed_client_as_link_changed() {
    let tmp = Tmp::new("wire");
    let runtime = RuntimeDir::at(tmp.0.join("rt")).unwrap();
    let server = Arc::new(
        Server::start(ServerConfig {
            app: "filmcraft".into(),
            app_version: "0.1.0".into(),
            instance: None,
            runtime: Some(runtime.clone()),
            tcp_port: None,
            sink: Arc::new(NoSink),
            activate: Arc::new(NoActivate),
            frames: Arc::new(NoFrames),
        })
        .unwrap(),
    );
    let record = Discovery::scan(&runtime).unwrap().into_iter().next().unwrap();
    let mut client = Client::connect(&record).unwrap();
    client
        .hello(&HelloClient {
            app: "hub".into(),
            app_version: "0.1.0".into(),
            pid: std::process::id(),
            instance: "550e8400-e29b-41d4-a716-446655440002".into(),
            protocol_min: 1,
            protocol_max: 1,
        })
        .unwrap();
    client.subscribe(&["link.changed"]).unwrap();

    let file = tmp.0.join("still.png");
    std::fs::write(&file, b"v1").unwrap();
    let clock = Arc::new(ManualTicker::new());
    let emitter = Arc::clone(&server);
    let watcher = Watcher::new(Arc::clone(&clock) as Arc<dyn Ticker>, Intervals::default(), move |notice| {
        if let Notice::Changed { link_id, change, .. } = notice
            && change.bumped
        {
            emitter.emit("link.changed", json!({"link_id": link_id, "epoch": change.epoch})).unwrap();
        }
    })
    .unwrap();
    watcher.watch("link-9", Tracker::new(&file, 7).unwrap()).unwrap();

    clock.advance(SECOND);
    assert!(clock.settle(LIMIT));
    assert!(client.next_event(Some(Duration::from_millis(50))).unwrap().is_none(), "no edit, no event");
    write_atomic(&file, b"v2 is longer").unwrap();
    clock.advance(SECOND);
    assert!(clock.settle(LIMIT));
    let event = client.next_event(Some(LIMIT)).unwrap().expect("a link.changed event");
    assert_eq!(event.event, "link.changed");
    assert_eq!(event.data, json!({"link_id": "link-9", "epoch": 8}));

    drop(watcher);
    Arc::try_unwrap(server).ok().expect("the watcher released the server").shutdown().unwrap();
}
