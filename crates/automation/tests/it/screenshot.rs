//! `Captures`: the geometry epoch.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use craft_automation::control::ControlResponse;
use craft_automation::screenshot::{CaptureGeometry, Captures, MAX_RETRIES, SETTLE_PASSES};
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::trial;
use egui::{Color32, ColorImage, Context, ScreenshotCallback, ViewportCommand, ViewportId};

fn geometry(width: f32, height: f32) -> CaptureGeometry {
    CaptureGeometry::new([width, height], 16384)
}

fn admit(captures: &mut Captures, token: u64, now: Instant) -> Result<Receiver<ControlResponse>, String> {
    let (tx, rx) = channel();
    captures.admit(token, None, tx, None, 0.0, now).map(|()| rx).map_err(|(_, e)| e)
}

fn image(geometry: &CaptureGeometry) -> Arc<ColorImage> {
    let [w, h] = geometry.expected_pixels().expect("a capturable geometry");
    Arc::new(ColorImage::filled([w, h], Color32::from_rgb(240, 240, 240)))
}

/// Settles and issues the one request through the context's window host, and returns the callback of
/// the screenshot command the host sent.
fn issue_one(captures: &mut Captures, ctx: &Context, now: Instant) -> ScreenshotCallback {
    for _ in 0..SETTLE_PASSES {
        captures.note_pass();
    }
    let finished = captures.issue(&WindowHost::of(ctx), now, None);
    assert!(finished.is_empty());
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let mut callbacks: Vec<ScreenshotCallback> = output
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|out| {
            out.commands
                .iter()
                .filter_map(|command| if let ViewportCommand::Screenshot(callback) = command { Some(callback.clone()) } else { None })
                .collect()
        })
        .unwrap_or_default();
    callbacks.pop().expect("a capture command was sent")
}

/// A geometry change between issue and collection retries up to `MAX_RETRIES`, then fails.
#[trial]
fn geometry_change_in_flight_retries() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    captures.note_geometry(geometry(100.0, 80.0));
    let rx = admit(&mut captures, 7, now).expect("admitted");

    for attempt in 0..=MAX_RETRIES {
        let callback = issue_one(&mut captures, &ctx, now);
        captures.note_geometry(geometry(100.0 + f32::from(attempt) + 1.0, 80.0));
        callback.complete(image(&geometry(100.0 + f32::from(attempt), 80.0)));
        let finished = captures.drain(now, captures.last_geometry(), None);
        if attempt < MAX_RETRIES {
            assert!(finished.is_empty(), "attempt {attempt} retries");
            assert_eq!(captures.pending(), 1);
        } else {
            assert_eq!(finished.len(), 1);
            assert_eq!(finished[0].result.as_ref().unwrap_err(), "screenshot 7 failed: the layout changed while it was captured");
        }
    }
    assert_eq!(captures.pending(), 0);
    assert!(rx.try_recv().is_err(), "the caller replies from the Finished, not the queue");
}
