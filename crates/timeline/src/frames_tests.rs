//! The frame server.

use craft_ui::testing::trial;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cache_tests::{TestFrame, TestKey};
use crate::frames::{CancelToken, FrameRenderer, FrameServer};

struct Recorder {
    rendered: Mutex<Vec<(u8, i64)>>,
    ready: AtomicBool,
}

impl FrameRenderer for Recorder {
    type Key = TestKey;
    type Frame = TestFrame;
    type Job = ();

    fn render(&self, key: &TestKey, _: &(), _: &CancelToken) -> Option<TestFrame> {
        if !self.ready.load(Ordering::Relaxed) {
            return None;
        }
        self.rendered.lock().ok()?.push((key.series, key.frame));
        Some(TestFrame(10))
    }
}

/// A server without workers, so `pump` runs the jobs.
fn server() -> (FrameServer<Recorder>, Arc<Recorder>) {
    let renderer = Arc::new(Recorder { rendered: Mutex::new(Vec::new()), ready: AtomicBool::new(true) });
    (FrameServer::new(renderer.clone(), 0, 1 << 20), renderer)
}

fn rendered(renderer: &Recorder) -> Vec<i64> {
    renderer.rendered.lock().map(|r| r.iter().map(|(_, f)| *f).collect()).unwrap_or_default()
}

// (1, 1) while workers keep up, else lead and stride as FilmCraft.

// 14 at 1x, 24 above 1.5x.

// a priority-0 request drops this view's queued priority-0 jobs.
#[trial]
fn an_on_screen_request_replaces_older_ones() {
    let (frames, renderer) = server();
    frames.request(TestKey::new(0, 1), (), 0);
    frames.request(TestKey::new(0, 2), (), 0);
    frames.request(TestKey::new(0, 3), (), 1);
    frames.request(TestKey::new(1, 4), (), 0);
    frames.request(TestKey::new(0, 5), (), 0);
    assert_eq!(frames.queue_len(), 3);
    assert_eq!(frames.pump(Duration::from_secs(5)), 3);
    assert_eq!(rendered(&renderer), [4, 5, 3]);
}

// preroll_ready is true once the next six frames are cached.
