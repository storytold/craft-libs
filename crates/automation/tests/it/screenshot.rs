//! `Captures`: admission budgets, settling, deadlines, cancellation, document identity, the
//! geometry epoch per viewport, the window a capture names, the image checks, and the region a
//! reply is cropped to.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use craft_automation::control::ControlResponse;
use craft_automation::screenshot::{
    CaptureGeometry, Captures, DEADLINE, DocumentId, Finished, MAX_IMAGE_BYTES, MAX_PENDING, MAX_RETRIES, MAX_TOTAL_BYTES, SETTLE_PASSES,
    ScreenshotRequest, WINDOW_CLOSED,
};
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::trial;
use egui::{
    Color32, ColorImage, Context, FullOutput, Pos2, RawInput, Rect, ScreenshotCallback, Vec2, ViewportBuilder, ViewportCommand, ViewportId,
    ViewportIdMap, ViewportInfo,
};

const DOC: Option<DocumentId> = Some(DocumentId::new(1, 1));

fn geometry(width: f32, height: f32) -> CaptureGeometry {
    CaptureGeometry::new([width, height], 16384)
}

fn geometry_of(width: f32, height: f32, document: Option<DocumentId>) -> CaptureGeometry {
    let mut g = geometry(width, height);
    g.document = document;
    g
}

fn admit_for(
    captures: &mut Captures,
    token: u64,
    viewport: ViewportId,
    document: Option<DocumentId>,
    now: Instant,
) -> Result<Receiver<ControlResponse>, String> {
    let (tx, rx) = channel();
    let mut request = ScreenshotRequest::new(token, viewport, tx);
    request.document = document;
    captures.admit(request, now).map(|()| rx).map_err(|(_, e)| e)
}

fn admit(captures: &mut Captures, token: u64, now: Instant) -> Result<Receiver<ControlResponse>, String> {
    admit_for(captures, token, ViewportId::ROOT, None, now)
}

fn filled(size: [usize; 2], color: Color32) -> Arc<ColorImage> {
    Arc::new(ColorImage::filled(size, color))
}

fn image(geometry: &CaptureGeometry) -> Arc<ColorImage> {
    filled(geometry.expected_pixels().expect("a capturable geometry"), Color32::from_rgb(240, 240, 240))
}

/// The callbacks of the screenshot commands `output` carries for `viewport`.
fn screenshots(output: &FullOutput, viewport: ViewportId) -> Vec<ScreenshotCallback> {
    output
        .viewport_output
        .get(&viewport)
        .map(|out| {
            out.commands
                .iter()
                .filter_map(|command| if let ViewportCommand::Screenshot(callback) = command { Some(callback.clone()) } else { None })
                .collect()
        })
        .unwrap_or_default()
}

/// The callbacks of the screenshot commands the context's window host sent to `viewport` since
/// the last pass.
fn sent(ctx: &Context, viewport: ViewportId) -> Vec<ScreenshotCallback> {
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    screenshots(&output, viewport)
}

/// Settles and issues through the context's window host: the requests that finished, and the
/// callbacks of the root captures sent.
fn settle_and_issue(captures: &mut Captures, ctx: &Context, now: Instant, document: Option<DocumentId>) -> (Vec<Finished>, Vec<ScreenshotCallback>) {
    for _ in 0..SETTLE_PASSES {
        captures.note_pass();
    }
    let finished = captures.issue(&WindowHost::of(ctx), now, document);
    (finished, sent(ctx, ViewportId::ROOT))
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
    captures.note_geometry(egui::ViewportId::ROOT, geometry(2000.0, 2000.0));
    for token in 0..MAX_PENDING as u64 {
        admit(&mut captures, token, now).expect("within the count");
    }
    let e = admit(&mut captures, 99, now).expect_err("the queue is full");
    assert!(e.contains("too many"), "{e}");
    captures.cancel_all("test");

    captures.note_geometry(egui::ViewportId::ROOT, geometry(8000.0, 5000.0));
    let e = admit(&mut captures, 100, now).expect_err("over the per-image limit");
    assert!(e.contains("per-image"), "{e}");

    // 4 × 64 MiB is exactly the aggregate limit.
    captures.note_geometry(egui::ViewportId::ROOT, geometry(4096.0, 4096.0));
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
    captures.note_geometry(egui::ViewportId::ROOT, g);
    assert!(captures.drain(now, None).is_empty());
    assert_eq!(captures.reserved_bytes(), 800 * 600 * 4, "admitted with a reservation");
    let e = admit(&mut captures, 1, now).expect_err("the token is pending");
    assert!(e.contains("already pending"), "{e}");
    let (_, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    assert_eq!(callbacks.len(), 1, "then it is issued");

    let mut parked = Captures::default();
    let rx = admit(&mut parked, 3, now).expect("waits");
    assert!(parked.drain(now + DEADLINE, None).is_empty(), "the waiting request answers itself");
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
    captures.note_geometry(egui::ViewportId::ROOT, geometry(800.0, 600.0));
    admit(&mut captures, 61, now).expect("admitted");
    captures.note_geometry(egui::ViewportId::ROOT, geometry(8000.0, 5000.0));
    let (finished, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    assert!(callbacks.is_empty(), "nothing is captured under the stale reservation");
    assert_eq!(finished.len(), 1);
    assert!(error(&finished[0]).contains("per-image"), "{}", error(&finished[0]));
    assert_eq!((captures.pending(), captures.reserved_bytes()), (0, 0));

    // 3 × 64 MiB fit; 3 × 100 MiB do not.
    captures.note_geometry(egui::ViewportId::ROOT, geometry(4096.0, 4096.0));
    for token in 70..73 {
        admit(&mut captures, token, now).expect("admitted");
    }
    captures.note_geometry(egui::ViewportId::ROOT, geometry(5000.0, 5000.0));
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
    captures.note_geometry(egui::ViewportId::ROOT, geometry(800.0, 600.0));
    admit(&mut captures, 7, now).expect("admitted");
    let mut issued = 0;
    for pass in 1..=SETTLE_PASSES + 2 {
        captures.note_pass();
        assert!(captures.issue(&host, now, None).is_empty());
        issued += sent(&ctx, ViewportId::ROOT).len();
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
    captures.note_geometry(egui::ViewportId::ROOT, g);
    let rx = admit(&mut captures, 1, now).expect("admitted");
    issue_one(&mut captures, &ctx, now).complete(image(&g));
    assert!(rx.try_recv().is_err(), "no reply from the callback");
    let finished = captures.drain(now, None);
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
        captures.note_geometry(egui::ViewportId::ROOT, g);
        admit(&mut captures, 2, now).expect("admitted");
        if issued {
            issue_one(&mut captures, &ctx, now).complete(image(&g));
        }
        let finished = captures.drain(late, None);
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
    captures.note_geometry(egui::ViewportId::ROOT, g);
    admit(&mut captures, 3, now).expect("admitted");
    let callback = issue_one(&mut captures, &ctx, now);
    let finished = captures.cancel_all("test");
    assert_eq!(finished.len(), 1);
    assert!(error(&finished[0]).contains("cancelled"));
    assert_eq!((captures.pending(), captures.reserved_bytes()), (0, 0));
    callback.complete(image(&g));
    assert!(captures.drain(now, None).is_empty());
}

/// Images completed in reverse order stay with their own tokens.
#[trial]
fn out_of_order_completions_keep_their_tokens() {
    let ctx = Context::default();
    let now = motion::now();
    let mut captures = Captures::default();
    let g = geometry(800.0, 600.0);
    captures.note_geometry(egui::ViewportId::ROOT, g);
    admit(&mut captures, 11, now).expect("admitted");
    admit(&mut captures, 12, now).expect("admitted");
    let (_, callbacks) = settle_and_issue(&mut captures, &ctx, now, None);
    let Ok([first, second]) = <[ScreenshotCallback; 2]>::try_from(callbacks) else { panic!("two captures") };
    second.complete(filled([800, 600], Color32::RED));
    first.complete(filled([800, 600], Color32::BLUE));
    let mut finished = captures.drain(now, None);
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
        captures.note_geometry(egui::ViewportId::ROOT, g);
        admit_for(&mut captures, 21, ViewportId::ROOT, DOC, now).expect("admitted");
        let (_, mut callbacks) = settle_and_issue(&mut captures, &ctx, now, DOC);
        callbacks.pop().expect("one capture").complete(image(&g));
        let current = geometry_of(800.0, 600.0, after);
        captures.note_geometry(egui::ViewportId::ROOT, current);
        let finished = captures.drain(now, after);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].result.is_ok(), captured, "{after:?}");
        if !captured {
            assert!(error(&finished[0]).contains("document changed"), "{}", error(&finished[0]));
        }
    }

    // The pass began in generation 1 and ended in 2: no capture of a mixed frame.
    let mut captures = Captures::default();
    captures.note_geometry(egui::ViewportId::ROOT, g);
    admit_for(&mut captures, 23, ViewportId::ROOT, DOC, now).expect("admitted");
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
    captures.note_geometry(egui::ViewportId::ROOT, geometry(100.0, 80.0));
    let rx = admit(&mut captures, 7, now).expect("admitted");

    for attempt in 0..=MAX_RETRIES {
        let callback = issue_one(&mut captures, &ctx, now);
        captures.note_geometry(egui::ViewportId::ROOT, geometry(100.0 + f32::from(attempt) + 1.0, 80.0));
        callback.complete(image(&geometry(100.0 + f32::from(attempt), 80.0)));
        let finished = captures.drain(now, None);
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
    captures.note_geometry(egui::ViewportId::ROOT, g);
    admit(&mut captures, 32, now).expect("admitted");
    issue_one(&mut captures, &ctx, now).complete(image(&g));
    captures.note_geometry(egui::ViewportId::ROOT, geometry(900.0, 700.0));
    captures.note_geometry(egui::ViewportId::ROOT, g);
    assert!(captures.drain(now, None).is_empty(), "the epoch advanced: retry");
    assert_eq!(captures.pending(), 1);
    issue_one(&mut captures, &ctx, now).complete(image(&g));
    let finished = captures.drain(now, None);
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
        captures.note_geometry(egui::ViewportId::ROOT, g);
        admit(&mut captures, 41, now).expect("admitted");
        issue_one(&mut captures, &ctx, now).complete(filled(size, Color32::BLACK));
        let finished = captures.drain(now, None);
        assert_eq!(finished.len(), 1);
        match expected {
            Ok(()) => assert!(finished[0].result.is_ok(), "{size:?}"),
            Err(text) => assert!(error(&finished[0]).contains(text), "{size:?}: {}", error(&finished[0])),
        }
    }
}

/// The first pixel of a PNG, decoded.
fn first_pixel(png: &[u8]) -> [u8; 4] {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info().expect("a PNG");
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).expect("a frame");
    [pixels[0], pixels[1], pixels[2], pixels[3]]
}

/// A region is checked against the window at admission (and again when the layout changed before issue) and refused
/// when it does not lie inside; the whole window is reserved for it. The reply crops it at the image's own scale to
/// the window, so a fitted image is cropped at its fit, and both reply shapes carry the window's point scale.
#[trial]
fn a_region_is_cropped_at_the_image_scale_and_one_outside_the_window_is_refused() {
    use craft_automation::control::ControlChannel;
    use craft_automation::screenshot::ScreenshotParams;
    use serde_json::json;

    let params = |value| ScreenshotParams::from_params(&value);
    let region = Rect::from_min_max(Pos2::new(10.0, 5.0), Pos2::new(60.0, 30.0));
    assert_eq!(params(json!({"path": "a.png", "region": [10, 5, 60, 30]})).map(|p| (p.path, p.region)), Ok((Some("a.png".to_owned()), Some(region))));
    for bad in [json!({"region": [60, 5, 10, 30]}), json!({"region": [10, 5, 60]}), json!({"region": [10, 5, 60, "30"]}), json!({"zoom": 2})] {
        assert!(params(bad.clone()).is_err(), "{bad} is refused");
    }

    let ctx = Context::default();
    let now = motion::now();
    let (_transport, requests) = channel();
    let mut control = ControlChannel::new(requests);
    let mut g = geometry(100.0, 50.0);
    g.pixels_per_point = 2.0;
    control.captures.note_geometry(ViewportId::ROOT, g);
    let request = |token: u64, region: Rect| {
        let (tx, rx) = channel();
        let mut request = ScreenshotRequest::new(token, ViewportId::ROOT, tx);
        request.region = Some(region);
        (request, rx)
    };

    let (outside, _) = request(1, Rect::from_min_max(Pos2::new(90.0, 40.0), Pos2::new(110.0, 60.0)));
    let e = control.captures.admit(outside, now).map_err(|(_, e)| e).expect_err("outside the window");
    assert!(e.contains("inside"), "{e}");
    assert_eq!(control.captures.pending(), 0);

    let (inside, rx) = request(2, region);
    control.captures.admit(inside, now).map_err(|(_, e)| e).expect("inside the window");
    assert_eq!(control.captures.reserved_bytes(), 200 * 100 * 4, "the whole window is reserved");
    let mut picture = ColorImage::filled([200, 100], Color32::BLUE);
    picture.pixels[10 * 200 + 20] = Color32::RED;
    issue_one(&mut control.captures, &ctx, now).complete(Arc::new(picture));
    let finished = control.captures.drain(now, None);
    let mut written = Vec::new();
    control.finish_captures(finished, &mut |_, bytes| {
        written = bytes.to_vec();
        Ok(())
    });
    let reply = rx.try_recv().expect("a reply");
    assert_eq!((reply["result"]["width"].clone(), reply["result"]["height"].clone()), (json!(100), json!(50)), "{reply}");
    assert_eq!(reply["result"]["pixels_per_point"], 2.0);
    assert!(written.is_empty(), "no path, no file");
    use base64::Engine as _;
    let png = base64::engine::general_purpose::STANDARD.decode(reply["result"]["base64"].as_str().expect("an inline PNG")).expect("base64");
    assert_eq!(first_pixel(&png), Color32::RED.to_array(), "the crop starts at the region's corner at two pixels per point");

    // A fitted surface: 600×300 points through a 400-pixel texture side come back as 400×200.
    let mut fitted = geometry(600.0, 300.0);
    fitted.max_texture_side = 400;
    control.captures.note_geometry(ViewportId::ROOT, fitted);
    let (mut half, rx) = request(3, Rect::from_min_max(Pos2::new(300.0, 150.0), Pos2::new(600.0, 300.0)));
    half.path = Some("half.png".into());
    control.captures.admit(half, now).map_err(|(_, e)| e).expect("inside the window");
    issue_one(&mut control.captures, &ctx, now).complete(filled([400, 200], Color32::WHITE));
    let finished = control.captures.drain(now, None);
    control.finish_captures(finished, &mut |path, bytes| {
        assert_eq!(path, "half.png");
        written = bytes.to_vec();
        Ok(())
    });
    let reply = rx.try_recv().expect("a reply");
    assert_eq!((reply["result"]["width"].clone(), reply["result"]["height"].clone()), (json!(200), json!(100)), "{reply}");
    assert_eq!(reply["result"]["pixels_per_point"], 1.0);
    assert!(!written.is_empty(), "the PNG went to the writer");

    // A region that a shrink before issue leaves outside is refused then, not cut to a smaller picture.
    let (late, _reply) = request(4, region);
    control.captures.admit(late, now).map_err(|(_, e)| e).expect("inside the window");
    control.captures.note_geometry(ViewportId::ROOT, geometry(40.0, 20.0));
    let (finished, callbacks) = settle_and_issue(&mut control.captures, &ctx, now, None);
    assert!(callbacks.is_empty(), "nothing is captured for it");
    assert!(finished.len() == 1 && error(&finished[0]).contains("inside"), "refused at issue");
    assert_eq!(control.captures.pending(), 0);
}

const ROOT_SIZE: Vec2 = Vec2::new(800.0, 600.0);
const CHILD_SIZE: Vec2 = Vec2::new(400.0, 300.0);

thread_local! {
    /// The screenshot callbacks the child's own passes sent out.
    static CHILD_SHOTS: RefCell<Vec<ScreenshotCallback>> = const { RefCell::new(Vec::new()) };
}

fn child() -> ViewportId {
    ViewportId::from_hash_of("craft_automation.screenshot.child")
}

/// A window of `size` at one pixel per point, as the integration reports it.
fn window_info(size: Vec2) -> ViewportInfo {
    ViewportInfo { inner_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), native_pixels_per_point: Some(1.0), ..ViewportInfo::default() }
}

/// A renderer like eframe's: the child's pass runs nested inside the root's.
fn render_child(ctx: &Context, viewport: egui::ImmediateViewport<'_>) {
    let mut show = viewport.viewport_ui_cb;
    let mut viewports = ViewportIdMap::default();
    viewports.insert(viewport.ids.this, ViewportInfo { parent: Some(viewport.ids.parent), ..window_info(CHILD_SIZE) });
    let input =
        RawInput { viewport_id: viewport.ids.this, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, CHILD_SIZE)), viewports, ..RawInput::default() };
    let mut output = ctx.run_ui(input, |ui| show(ui));
    output.textures_delta.clear();
    CHILD_SHOTS.with(|shots| shots.borrow_mut().extend(screenshots(&output, viewport.ids.this)));
}

/// One frame of a root that shows the child window while `open`, and lists it in the windows the
/// integration reports. Returns the screenshot callbacks sent to the root and to the child.
fn frame(ctx: &Context, open: bool) -> (Vec<ScreenshotCallback>, Vec<ScreenshotCallback>) {
    let mut viewports = ViewportIdMap::default();
    viewports.insert(ViewportId::ROOT, window_info(ROOT_SIZE));
    if open {
        viewports.insert(child(), window_info(CHILD_SIZE));
    }
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, ROOT_SIZE)), viewports, ..RawInput::default() };
    let mut output = ctx.run_ui(input, |ui| {
        if open {
            ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |_, _| {});
        }
    });
    output.textures_delta.clear();
    let mut to_child = screenshots(&output, child());
    to_child.extend(CHILD_SHOTS.with(|shots| std::mem::take(&mut *shots.borrow_mut())));
    (screenshots(&output, ViewportId::ROOT), to_child)
}

/// A capture names its window: it is sized from that window's geometry and shot in that viewport,
/// and once the window is gone it fails at once instead of waiting out the deadline.
#[trial]
fn a_screenshot_names_its_window_and_fails_once_it_closes() {
    let ctx = Context::default();
    ctx.set_embed_viewports(false);
    Context::set_immediate_viewport_renderer(render_child);
    let host = WindowHost::of(&ctx);
    let now = motion::now();
    let mut captures = Captures::default();
    frame(&ctx, true);

    let rx = admit_for(&mut captures, 81, child(), None, now).expect("waits for the child's geometry");
    captures.note_windows(&host, None);
    assert_eq!(captures.last_geometry(child()).map(|g| g.size_points), Some([CHILD_SIZE.x, CHILD_SIZE.y]));
    assert!(captures.drain(now, None).is_empty());
    assert_eq!(captures.reserved_bytes(), 400 * 300 * 4, "reserved at the child's size, not the root's");
    for _ in 0..SETTLE_PASSES {
        captures.note_pass();
    }
    assert!(captures.issue(&host, now, None).is_empty());
    let (to_root, mut to_child) = frame(&ctx, true);
    assert!(to_root.is_empty(), "the root is not shot");
    assert_eq!(to_child.len(), 1, "the child is shot");
    to_child.remove(0).complete(filled([400, 300], Color32::WHITE));
    captures.note_windows(&host, None);
    let finished = captures.drain(now, None);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].result.as_ref().map(|image| image.size), Ok([400, 300]));
    assert!(rx.try_recv().is_err(), "the caller replies from the Finished");

    admit_for(&mut captures, 82, child(), None, now).expect("admitted while the child is open");
    frame(&ctx, false);
    captures.note_windows(&host, None);
    let finished = captures.drain(now, None);
    assert_eq!(finished.len(), 1, "no wait for the deadline");
    assert_eq!(error(&finished[0]), format!("screenshot 82 failed: {WINDOW_CLOSED}"));
    assert_eq!(captures.pending(), 0);
    let e = admit_for(&mut captures, 83, child(), None, now).expect_err("the window is closed");
    assert_eq!(e, format!("screenshot 83 failed: {WINDOW_CLOSED}"));
}

/// A child window the integration lists before it reports a size is open, not closed: its request keeps waiting, and
/// is shot once the size arrives.
#[trial]
fn a_window_listed_without_a_size_keeps_its_request_waiting() {
    let ctx = Context::default();
    ctx.set_embed_viewports(false);
    Context::set_immediate_viewport_renderer(render_child);
    let host = WindowHost::of(&ctx);
    let now = motion::now();
    let mut captures = Captures::default();

    let mut viewports = ViewportIdMap::default();
    viewports.insert(ViewportId::ROOT, window_info(ROOT_SIZE));
    viewports.insert(child(), ViewportInfo::default());
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, ROOT_SIZE)), viewports, ..RawInput::default() };
    ctx.run_ui(input, |_| {}).textures_delta.clear();
    assert!(host.is_open(child()) && host.screenshot_facts(child()).is_none());

    admit_for(&mut captures, 91, child(), None, now).expect("waits for the child's geometry");
    captures.note_windows(&host, None);
    assert!(captures.drain(now, None).is_empty(), "not failed as closed");
    assert_eq!(captures.pending(), 1);

    frame(&ctx, true);
    captures.note_windows(&host, None);
    assert_eq!(captures.last_geometry(child()).map(|g| g.size_points), Some([CHILD_SIZE.x, CHILD_SIZE.y]));
    assert!(captures.drain(now, None).is_empty());
}
