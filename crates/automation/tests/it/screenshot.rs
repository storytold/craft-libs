//! `Captures`: admission budgets, settling, deadlines, cancellation, document identity, the
//! geometry epoch and the image checks.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use craft_automation::control::ControlResponse;
use craft_automation::screenshot::{
    CaptureGeometry, Captures, DEADLINE, DocumentId, Finished, MAX_IMAGE_BYTES, MAX_PENDING, MAX_RETRIES, MAX_TOTAL_BYTES, SETTLE_PASSES,
};
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::trial;
use egui::{Color32, ColorImage, Context, ScreenshotCallback, ViewportCommand, ViewportId};

const DOC: Option<DocumentId> = Some(DocumentId::new(1, 1));

fn geometry(width: f32, height: f32) -> CaptureGeometry {
    CaptureGeometry::new([width, height], 16384)
}

fn geometry_of(width: f32, height: f32, document: Option<DocumentId>) -> CaptureGeometry {
    let mut g = geometry(width, height);
    g.document = document;
    g
}

fn admit_for(captures: &mut Captures, token: u64, document: Option<DocumentId>, now: Instant) -> Result<Receiver<ControlResponse>, String> {
    let (tx, rx) = channel();
    captures.admit(token, None, tx, document, 0.0, now).map(|()| rx).map_err(|(_, e)| e)
}

fn admit(captures: &mut Captures, token: u64, now: Instant) -> Result<Receiver<ControlResponse>, String> {
    admit_for(captures, token, None, now)
}

fn filled(size: [usize; 2], color: Color32) -> Arc<ColorImage> {
    Arc::new(ColorImage::filled(size, color))
}

fn image(geometry: &CaptureGeometry) -> Arc<ColorImage> {
    filled(geometry.expected_pixels().expect("a capturable geometry"), Color32::from_rgb(240, 240, 240))
}

/// The callbacks of the screenshot commands the context's window host sent since the last pass.
fn sent(ctx: &Context) -> Vec<ScreenshotCallback> {
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    output
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|out| {
            out.commands
                .iter()
                .filter_map(|command| if let ViewportCommand::Screenshot(callback) = command { Some(callback.clone()) } else { None })
                .collect()
        })
        .unwrap_or_default()
}

/// Settles and issues through the context's window host: the requests that finished, and the
/// callbacks of the captures sent.
fn settle_and_issue(captures: &mut Captures, ctx: &Context, now: Instant, document: Option<DocumentId>) -> (Vec<Finished>, Vec<ScreenshotCallback>) {
    for _ in 0..SETTLE_PASSES {
        captures.note_pass();
    }
    let finished = captures.issue(&WindowHost::of(ctx), now, document);
    (finished, sent(ctx))
}

fn issue_one(captures: &mut Captures, ctx: &Context, now: Instant) -> ScreenshotCallback {
    let (finished, mut callbacks) = settle_and_issue(captures, ctx, now, None);
    assert!(finished.is_empty());
    callbacks.pop().expect("a capture command was sent")
}

fn error(finished: &Finished) -> &str {
    finished.result.as_ref().err().map(String::as_str).unwrap_or_else(|| panic!("screenshot {} succeeded", finished.token))
}

/// At most `MAX_PENDING` requests and `MAX_IMAGE_BYTES` per image; the aggregate limit is inclusive.
#[trial]
fn admission_holds_the_count_and_byte_budgets() {
    let now = motion::now();
    let mut captures = Captures::default();
    // 16 MiB each: the count limit binds before the aggregate.
    captures.note_geometry(geometry(2000.0, 2000.0));
    for token in 0..MAX_PENDING as u64 {
        admit(&mut captures, token, now).expect("within the count");
    }
    let e = admit(&mut captures, 99, now).expect_err("the queue is full");
    assert!(e.contains("too many"), "{e}");
    captures.cancel_all("test");

    captures.note_geometry(geometry(8000.0, 5000.0));
    let e = admit(&mut captures, 100, now).expect_err("over the per-image limit");
    assert!(e.contains("per-image"), "{e}");

    // 4 × 64 MiB is exactly the aggregate limit.
    captures.note_geometry(geometry(4096.0, 4096.0));
    for token in 200..204 {
        admit(&mut captures, token, now).expect("within the aggregate");
    }
    assert_eq!(captures.reserved_bytes(), MAX_TOTAL_BYTES);
    const { assert!(MAX_IMAGE_BYTES < MAX_TOTAL_BYTES) };
}

/// A request that arrives before any geometry waits instead of failing, and is admitted under its
/// token and reservation once the first pass records the geometry; it times out if none comes.
#[trial]
fn a_request_before_the_first_geometry_waits() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let rx = admit(&mut captures, 1, now).expect("waits, not rejected");
    assert_eq!(captures.pending(), 1);
    assert_eq!(captures.reserved_bytes(), 0, "no geometry, no reservation");
    assert!(captures.any_unissued(), "the app keeps painting for it");
    assert!(rx.try_recv().is_err());

    let g = geometry(800.0, 600.0);
    captures.note_geometry(g);
    assert!(captures.drain(now, Some(g), None).is_empty());
    assert_eq!(captures.reserved_bytes(), 800 * 600 * 4, "admitted with a reservation");
    let e = admit(&mut captures, 1, now).expect_err("the token is pending");
    assert!(e.contains("already pending"), "{e}");
    let (_, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    assert_eq!(callbacks.len(), 1, "then it is issued");

    let mut parked = Captures::default();
    let rx = admit(&mut parked, 3, now).expect("waits");
    assert!(parked.drain(now + DEADLINE, None, None).is_empty(), "the waiting request answers itself");
    let reply = rx.try_recv().expect("exactly one answer");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().is_some_and(|e| e.contains("timed out")), "{reply}");
    assert_eq!(parked.pending(), 0);
}

/// A layout that grows between admission and issue is checked again: no capture runs under the old
/// reservation, and a request that no longer fits fails and releases its budget.
#[trial]
fn a_grown_layout_is_revalidated_before_issue() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    captures.note_geometry(geometry(800.0, 600.0));
    admit(&mut captures, 61, now).expect("admitted");
    captures.note_geometry(geometry(8000.0, 5000.0));
    let (finished, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    assert!(callbacks.is_empty(), "nothing is captured under the stale reservation");
    assert_eq!(finished.len(), 1);
    assert!(error(&finished[0]).contains("per-image"), "{}", error(&finished[0]));
    assert_eq!((captures.pending(), captures.reserved_bytes()), (0, 0));

    // 3 × 64 MiB fit; 3 × 100 MiB do not.
    captures.note_geometry(geometry(4096.0, 4096.0));
    for token in 70..73 {
        admit(&mut captures, token, now).expect("admitted");
    }
    captures.note_geometry(geometry(5000.0, 5000.0));
    let (finished, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    assert!(callbacks.is_empty(), "the grown layout settles before any capture");
    assert!(!finished.is_empty(), "a request no longer fits the aggregate");
    assert!(finished.iter().all(|f| error(f).contains("budget exhausted")));
    assert_eq!(captures.pending() + finished.len(), 3);
    assert!(captures.reserved_bytes() <= MAX_TOTAL_BYTES);
}

/// A request is issued after `SETTLE_PASSES` real passes, exactly once.
#[trial]
fn settling_counts_passes_and_issues_once() {
    let ctx = Context::default();
    let now = motion::now();
    let host = WindowHost::of(&ctx);
    let mut captures = Captures::default();
    captures.note_geometry(geometry(800.0, 600.0));
    admit(&mut captures, 7, now).expect("admitted");
    let mut issued = 0;
    for pass in 1..=SETTLE_PASSES + 2 {
        captures.note_pass();
        assert!(captures.issue(&host, now, None).is_empty());
        issued += sent(&ctx).len();
        assert_eq!(issued, usize::from(pass >= SETTLE_PASSES), "after pass {pass}");
    }
    assert!(!captures.any_unissued());
}

/// The image is handed back by the next drain, never from the callback, and the budget is released
/// then.
#[trial]
fn a_completed_image_is_handed_back_by_drain() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let g = geometry(800.0, 600.0);
    captures.note_geometry(g);
    let rx = admit(&mut captures, 1, now).expect("admitted");
    issue_one(&mut captures, &ctx, now).complete(image(&g));
    assert!(rx.try_recv().is_err(), "no reply from the callback");
    let finished = captures.drain(now, Some(g), None);
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_ok());
    assert!(finished[0].reply.is_some());
    assert_eq!((captures.pending(), captures.reserved_bytes()), (0, 0));
    assert!(captures.cancel_all("nothing left").is_empty(), "a drained request is not answered twice");
}

/// The deadline comes first: a request never issued, or an image that arrives late, is a timeout.
#[trial]
fn a_request_past_its_deadline_times_out() {
    let ctx = Context::default();
    let now = motion::now();
    let late = now + DEADLINE + Duration::from_millis(1);
    let g = geometry(800.0, 600.0);
    for issued in [false, true] {
        let mut captures = Captures::default();
        captures.note_geometry(g);
        admit(&mut captures, 2, now).expect("admitted");
        if issued {
            issue_one(&mut captures, &ctx, now).complete(image(&g));
        }
        let finished = captures.drain(late, Some(g), None);
        assert_eq!(finished.len(), 1, "issued: {issued}");
        assert!(error(&finished[0]).contains("timed out"), "{}", error(&finished[0]));
        assert!(finished[0].reply.is_some());
        assert_eq!(captures.pending(), 0);
    }
}

/// Cancelling releases the budget at once; an image that arrives afterwards is dropped.
#[trial]
fn cancellation_releases_the_budget_and_drops_a_late_image() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let g = geometry(800.0, 600.0);
    captures.note_geometry(g);
    admit(&mut captures, 3, now).expect("admitted");
    let callback = issue_one(&mut captures, &ctx, now);
    let finished = captures.cancel_all("test");
    assert_eq!(finished.len(), 1);
    assert!(error(&finished[0]).contains("cancelled"));
    assert_eq!((captures.pending(), captures.reserved_bytes()), (0, 0));
    callback.complete(image(&g));
    assert!(captures.drain(now, Some(g), None).is_empty());
}

/// Images completed in reverse order stay with their own tokens.
#[trial]
fn out_of_order_completions_keep_their_tokens() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let g = geometry(800.0, 600.0);
    captures.note_geometry(g);
    admit(&mut captures, 11, now).expect("admitted");
    admit(&mut captures, 12, now).expect("admitted");
    let (_, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    let Ok([first, second]) = <[ScreenshotCallback; 2]>::try_from(callbacks) else { panic!("two captures") };
    second.complete(filled([800, 600], Color32::RED));
    first.complete(filled([800, 600], Color32::BLUE));
    let mut finished = captures.drain(now, Some(g), None);
    finished.sort_by_key(|f| f.token);
    let colors: Vec<_> = finished.iter().map(|f| (f.token, f.result.as_ref().expect("captured").pixels[0])).collect();
    assert_eq!(colors, [(11, Color32::BLUE), (12, Color32::RED)]);
}

/// A capture belongs to the active-document generation it was admitted in: another generation at
/// drain, or a pass that ends in another generation, fails it; a new revision does not.
#[trial]
fn a_capture_follows_the_document_generation() {
    let ctx = Context::default();
    let now = motion::now();
    let other = Some(DocumentId::new(2, 1));
    let edited = Some(DocumentId::new(1, 2));
    let g = geometry_of(800.0, 600.0, DOC);
    for (after, captured) in [(other, false), (edited, true)] {
        let mut captures = Captures::default();
        captures.note_geometry(g);
        admit_for(&mut captures, 21, DOC, now).expect("admitted");
        let (_, mut callbacks) = settle_and_issue(&mut captures, &ctx, now, DOC);
        callbacks.pop().expect("one capture").complete(image(&g));
        let current = geometry_of(800.0, 600.0, after);
        captures.note_geometry(current);
        let finished = captures.drain(now, Some(current), after);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].result.is_ok(), captured, "{after:?}");
        if !captured {
            assert!(error(&finished[0]).contains("document changed"), "{}", error(&finished[0]));
        }
    }

    // The pass began in generation 1 and ended in 2: no capture of a mixed frame.
    let mut captures = Captures::default();
    captures.note_geometry(g);
    admit_for(&mut captures, 23, DOC, now).expect("admitted");
    let (finished, callbacks) = settle_and_issue(&mut captures, &ctx, now, other);
    assert!(callbacks.is_empty());
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_err());
    assert_eq!(captures.pending(), 0);
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

/// A resize and back while the image is in flight retries, even though the values match again,
/// and the retry is accepted.
#[trial]
fn a_resize_and_back_retries_then_accepts() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let g = geometry(800.0, 600.0);
    captures.note_geometry(g);
    admit(&mut captures, 32, now).expect("admitted");
    issue_one(&mut captures, &ctx, now).complete(image(&g));
    captures.note_geometry(geometry(900.0, 700.0));
    captures.note_geometry(g);
    assert!(captures.drain(now, Some(g), None).is_empty(), "the epoch advanced: retry");
    assert_eq!(captures.pending(), 1);
    issue_one(&mut captures, &ctx, now).complete(image(&g));
    let finished = captures.drain(now, Some(g), None);
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_ok());
}

/// The image must be the issued viewport or its uniform texture-side fit.
#[trial]
fn the_image_must_match_the_issued_viewport() {
    let ctx = Context::default();
    let now = motion::now();
    let viewport = geometry(800.0, 600.0);
    let mut fitted = geometry(6000.0, 3375.0);
    fitted.max_texture_side = 4096;
    let cases: [(CaptureGeometry, [usize; 2], Result<(), &str>); 4] = [
        (viewport, [400, 200], Err("400×200, not the issued")),
        (viewport, [1600, 1200], Err("larger than")),
        (viewport, [100, 75], Err("texture-side fit")),
        (fitted, [4096, 2304], Ok(())),
    ];
    for (g, size, expected) in cases {
        let mut captures = Captures::default();
        captures.note_geometry(g);
        admit(&mut captures, 41, now).expect("admitted");
        issue_one(&mut captures, &ctx, now).complete(filled(size, Color32::BLACK));
        let finished = captures.drain(now, Some(g), None);
        assert_eq!(finished.len(), 1);
        match expected {
            Ok(()) => assert!(finished[0].result.is_ok(), "{size:?}"),
            Err(text) => assert!(error(&finished[0]).contains(text), "{size:?}: {}", error(&finished[0])),
        }
    }
}

/// A minimised window has no surface: the request fails before anything is issued.
#[trial]
fn a_minimized_window_is_refused_before_issue() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let mut g = geometry(800.0, 600.0);
    g.minimized = true;
    captures.note_geometry(g);
    admit(&mut captures, 52, now).expect("admitted");
    let (finished, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    assert!(callbacks.is_empty());
    assert_eq!(finished.len(), 1);
    assert!(error(&finished[0]).contains("minimized"));
}
