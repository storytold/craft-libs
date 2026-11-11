//! Tests of the live preview.

use craft_ui::canvas::PixelRect;
use craft_ui::testing::trial;

use craft_imaging::ui::live_preview::{DraftScale, LivePreview, PreviewKey, PreviewLane, PreviewSession};

const REGION: PixelRect = PixelRect::new(0, 0, 100, 100);
const VIEW: PixelRect = PixelRect::new(0, 0, 50, 50);

fn session(n: u64) -> PreviewSession {
    PreviewSession::new(n, 1, 7)
}

fn started(lane: PreviewLane, n: u64) -> LivePreview {
    let mut live = LivePreview::new(lane);
    live.begin(session(n), REGION);
    live
}

// One session: the same settings hash gives the same key twice; another hash another key; the
// key carries the lane's tag bit and never equals `PreviewKey::NONE`.

// A Dialog and a Properties preview with the same session and settings give different keys, and
// each treats the other's key as foreign (`owns` false, `switch` None).

// After a switch to a preview, `begin` with another session: the old key is foreign, `uncovered`
// returns None, and the new region applies.

// A preview back to NONE: the whole region, visible or not; nothing stays covered.

// A key of an earlier session or another lane on either side gives None; so do NONE to NONE and
// no session.
#[trial]
fn live_preview_switch_with_a_foreign_key_returns_none() {
    let mut old = started(PreviewLane::Dialog, 1);
    let stale = old.key(1, 0.0);
    let mut live = started(PreviewLane::Dialog, 2);
    let key = live.key(1, 0.0);
    let mut props = started(PreviewLane::Properties, 2);
    let other_lane = props.key(1, 0.0);
    assert_eq!(live.switch(stale, key, VIEW), None);
    assert_eq!(live.switch(key, stale, VIEW), None);
    assert_eq!(live.switch(PreviewKey::NONE, other_lane, VIEW), None);
    assert_eq!(live.switch(PreviewKey::NONE, PreviewKey::NONE, VIEW), None);
    let mut none = LivePreview::new(PreviewLane::Dialog);
    assert_eq!(none.switch(PreviewKey::NONE, key, VIEW), None);
    assert_eq!(none.key(1, 0.0), PreviewKey::NONE);
}

// After a switch, a pan that shows more of the region gives that visible part once; the same
// view again gives None; a view inside the covered rect gives None; another key gives None.
#[trial]
fn live_preview_uncovered_returns_the_newly_visible_part_once() {
    let mut live = started(PreviewLane::Dialog, 1);
    let (key, other) = (live.key(1, 0.0), live.key(2, 0.0));
    live.switch(PreviewKey::NONE, key, VIEW);
    assert_eq!(live.uncovered(key, VIEW), None);
    let panned = PixelRect::new(25, 0, 75, 50);
    assert_eq!(live.uncovered(key, panned), Some(panned));
    assert_eq!(live.uncovered(key, panned), None);
    assert_eq!(live.uncovered(key, PixelRect::new(30, 0, 60, 40)), None);
    assert_eq!(live.uncovered(other, PixelRect::new(0, 0, 100, 100)), None);
}

/// One drag of `control` on `item`: `drafts` full-size renders of `ms` each, started at `start`
/// and ended at `end`. Returns the scale the drag started with.
fn drag(d: &mut DraftScale, item: u64, start: f64, end: f64, ms: f64, drafts: u64) -> Option<f32> {
    let scale = d.scale(Some((item, "exposure")), true, Some(0), start);
    for key in 1..=drafts + 1 {
        d.observe(key, ms, None);
    }
    d.scale(None, true, None, end);
    scale
}

// After the retry time, a full-size drag within budget forgets the verdict (the next drag is full
// size at once); a slow retry doubles the wait to 60 s, capped at 240 s.
#[trial]
fn draft_scale_a_fast_retry_forgets_the_verdict() {
    let slow_twice = |d: &mut DraftScale| {
        drag(d, 1, 0.0, 10.0, 25.0, 4);
        drag(d, 1, 20.0, 100.0, 25.0, 4);
    };
    let mut d = DraftScale::default();
    slow_twice(&mut d);
    assert_eq!(drag(&mut d, 1, 140.0, 150.0, 5.0, 4), Some(1.0));
    assert_eq!(d.scale(Some((1, "exposure")), true, Some(0), 151.0), Some(1.0));

    let mut d = DraftScale::default();
    slow_twice(&mut d);
    assert_eq!(drag(&mut d, 1, 140.0, 150.0, 25.0, 4), Some(1.0));
    assert_eq!(d.scale(Some((1, "exposure")), true, Some(0), 200.0), Some(DraftScale::SMALL));
    d.scale(None, true, None, 201.0);
    assert_eq!(d.scale(Some((1, "exposure")), true, Some(0), 211.0), Some(1.0));
}
