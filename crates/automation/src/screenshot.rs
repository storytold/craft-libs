//! `ui.screenshot`: bounded, cancellable capture requests over [`WindowHost::screenshot`].
//!
//! The slot correlates one capture with one image. This queue owns what the slot leaves to the
//! app: request identity (token, output path, reply, the viewport it shoots and the region of it,
//! refused at admission when outside the window and cropped in [`screenshot_reply`]), deadlines and
//! settling, the admission budget ([`MAX_PENDING`] requests, [`MAX_IMAGE_BYTES`] per image,
//! [`MAX_TOTAL_BYTES`] in all), the document identity fixed at admission, each viewport's geometry
//! epoch recorded at issue so an image is saved only for the layout it was rendered from (a size,
//! scale, zoom or minimised change between issue and collection is a retry, at most
//! [`MAX_RETRIES`]), and a monotonic clock. A request for a viewport whose geometry is not recorded
//! yet waits instead of failing; one whose window is not open fails with [`WINDOW_CLOSED`].
//!
//! Phases: [`Captures::issue`] runs in the UI pass (settling counts real layout passes; a
//! discarded pass is skipped); [`Captures::note_windows`] (or [`Captures::note_geometry`] for a
//! synthetic geometry) and [`Captures::drain`] run in the logic phase. The image is encoded and written on the UI thread by
//! [`ControlChannel::finish_captures`](crate::control::ControlChannel::finish_captures), never
//! inside the callback.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use base64::Engine as _;
use craft_ui::runtime::windows::{ScreenshotFacts, ScreenshotSlot, WindowHost};
use egui::{ColorImage, Rect, ViewportId};
use serde_json::Value;
use web_time::Instant;

use crate::control::ControlResponse;
use crate::protocol::Params;

/// Requests outstanding at once (issued, completed, waiting for geometry or waiting to be issued).
pub const MAX_PENDING: usize = 4;
/// Largest single image the slot may retain, and the largest a request may reserve.
pub const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
/// Aggregate reserved bytes over every outstanding request.
pub const MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// A request expires after this long, including retries and settling.
pub const DEADLINE: Duration = Duration::from_secs(8);
/// UI passes a request waits before it is issued, on top of the animation delay.
pub const SETTLE_PASSES: u32 = 3;
/// Retries after the geometry changed while a capture was in flight.
pub const MAX_RETRIES: u8 = 2;
/// The base delay that lets popup open and close animations finish.
const BASE_SETTLE_MS: f64 = 60.0;

/// The document a screenshot belongs to, fixed at admission. `generation` must be the app's
/// active-document generation: it changes on every switch of the active document, including a
/// switch away and back, so it is not a per-document counter. `revision` is recorded for
/// attribution only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DocumentId {
    /// The app's active-document generation at admission.
    pub generation: u64,
    /// The document revision at admission; recorded for attribution only.
    pub revision: u64,
}

impl DocumentId {
    /// The document of active-document `generation` at `revision`.
    pub const fn new(generation: u64, revision: u64) -> Self {
        Self { generation, revision }
    }

    /// Whether both ids name the same active document (same `generation`).
    pub fn same_document(self, other: Self) -> bool {
        self.generation == other.generation
    }
}

/// The layout of the viewport a capture is rendered from: size in points, pixels per point,
/// zoom, the texture side limit and whether the window is minimised. Window position is not part
/// of it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CaptureGeometry {
    /// Viewport size in logical points, `[width, height]`.
    pub size_points: [f32; 2],
    /// Physical pixels per point, including zoom.
    pub pixels_per_point: f32,
    /// The UI zoom factor.
    pub zoom: f32,
    /// The renderer's largest texture side, in pixels.
    pub max_texture_side: usize,
    /// Whether the window is minimised.
    pub minimized: bool,
    /// The active document, if any; not part of the layout identity.
    pub document: Option<DocumentId>,
}

impl CaptureGeometry {
    /// A visible, unzoomed viewport of `size_points` at one pixel per point, with no active document.
    pub const fn new(size_points: [f32; 2], max_texture_side: usize) -> Self {
        Self { size_points, pixels_per_point: 1.0, zoom: 1.0, max_texture_side, minimized: false, document: None }
    }

    /// The geometry the window reported this pass.
    pub fn from_facts(facts: ScreenshotFacts, document: Option<DocumentId>) -> Self {
        Self {
            size_points: [facts.size_points.x, facts.size_points.y],
            pixels_per_point: facts.pixels_per_point,
            zoom: facts.zoom,
            max_texture_side: facts.max_texture_side,
            minimized: facts.minimized,
            document,
        }
    }

    /// The image size in pixels: the size in points times the point scale, rounded. `None` for a
    /// degenerate viewport (a non-finite or non-positive size, scale or zoom, or a side under one
    /// pixel). A window over the texture limit is not rejected: the renderer may fit it, and
    /// `check_image` accepts a uniform fit.
    pub fn expected_pixels(&self) -> Option<[usize; 2]> {
        let valid = |v: f32| v.is_finite() && v > 0.0;
        let [width, height] = self.size_points;
        if !valid(width) || !valid(height) || !valid(self.pixels_per_point) || !valid(self.zoom) {
            return None;
        }
        let w = (width * self.pixels_per_point).round();
        let h = (height * self.pixels_per_point).round();
        if w < 1.0 || h < 1.0 || w > usize::MAX as f32 || h > usize::MAX as f32 {
            return None;
        }
        Some([w as usize, h as usize])
    }

    /// The identity compared for stale-image rejection: everything but the document.
    fn same_layout(&self, other: &Self) -> bool {
        self.size_points == other.size_points
            && self.pixels_per_point == other.pixels_per_point
            && self.zoom == other.zoom
            && self.max_texture_side == other.max_texture_side
            && self.minimized == other.minimized
    }

    /// RGBA bytes of [`Self::expected_pixels`], from checked arithmetic.
    pub fn expected_bytes(&self) -> Option<usize> {
        let [w, h] = self.expected_pixels()?;
        w.checked_mul(h)?.checked_mul(std::mem::size_of::<egui::Color32>())
    }
}

/// The error a request for a closed window finishes with, after `screenshot {token} failed: `.
pub const WINDOW_CLOSED: &str = "its window is not open";

/// One outstanding request; holds the strong slot.
pub struct PendingCapture {
    token: u64,
    viewport: ViewportId,
    path: Option<String>,
    region: Option<Rect>,
    reply: Option<Sender<ControlResponse>>,
    deadline: Instant,
    ready_at: Instant,
    settle_passes: u32,
    geometry: CaptureGeometry,
    epoch: u64,
    issued_epoch: Option<u64>,
    reserved_bytes: usize,
    document: Option<DocumentId>,
    slot: Option<ScreenshotSlot>,
    retries: u8,
}

impl PendingCapture {
    /// The caller's token for this request.
    pub fn token(&self) -> u64 {
        self.token
    }

    /// Whether the render for this capture was already requested.
    pub fn issued(&self) -> bool {
        self.issued_epoch.is_some()
    }
}

/// A request that is over: an image to save and reply, or an error.
#[non_exhaustive]
pub struct Finished {
    /// The request's token.
    pub token: u64,
    /// File to write the PNG to; `None` inlines it in the reply.
    pub path: Option<String>,
    /// The part of the window to crop the image to, in points; `None` keeps the whole window.
    pub region: Option<egui::Rect>,
    /// The layout the image was captured from; the crop and the reply's `pixels_per_point` read it.
    pub geometry: CaptureGeometry,
    /// Where to send the answer.
    pub reply: Option<Sender<ControlResponse>>,
    /// The captured image, or the error text for the reply.
    pub result: Result<Arc<ColorImage>, String>,
}

impl Finished {
    /// A finished request for `token`: `result` answers `reply`, and an image is written to `path` when given,
    /// cropped to `region` as `geometry` scales it.
    pub fn new(
        token: u64,
        path: Option<String>,
        region: Option<egui::Rect>,
        geometry: CaptureGeometry,
        reply: Option<Sender<ControlResponse>>,
        result: Result<Arc<ColorImage>, String>,
    ) -> Self {
        Self { token, path, region, geometry, reply, result }
    }
}

/// A screenshot request as the control surface received it.
#[non_exhaustive]
pub struct ScreenshotRequest {
    /// The caller's token: one per pending request.
    pub token: u64,
    /// Where to write the PNG, when the reply is a file.
    pub path: Option<String>,
    /// The part of the window to capture, in points; `None` captures the whole window.
    pub region: Option<Rect>,
    /// The viewport to capture.
    pub viewport: ViewportId,
    /// Where the reply goes.
    pub reply: Sender<ControlResponse>,
    /// The document the capture belongs to.
    pub document: Option<DocumentId>,
    /// The settle delay in milliseconds of animation time.
    pub settle_ms: f64,
}

impl ScreenshotRequest {
    /// A request for the whole of `viewport` answered on `reply`: no file, no document, no settle delay.
    pub fn new(token: u64, viewport: ViewportId, reply: Sender<ControlResponse>) -> Self {
        Self { token, path: None, region: None, viewport, reply, document: None, settle_ms: 0.0 }
    }
}

/// A request not yet queued: it waits while its viewport's geometry is not recorded.
struct WaitingAdmit {
    token: u64,
    viewport: ViewportId,
    path: Option<String>,
    region: Option<Rect>,
    reply: Sender<ControlResponse>,
    deadline: Instant,
    settle_ms: f64,
    document: Option<DocumentId>,
}

/// What one viewport's window measured: its geometry this pass, `None` while the window is not
/// open, and the epoch that advances on each layout change.
#[derive(Clone, Copy, Default)]
struct Window {
    geometry: Option<CaptureGeometry>,
    epoch: u64,
}

/// The screenshot request queue and its admission budget.
#[derive(Default)]
pub struct Captures {
    records: Vec<PendingCapture>,
    waiting: Vec<WaitingAdmit>,
    /// One entry per recorded viewport; a viewport with no entry was never measured.
    windows: HashMap<ViewportId, Window>,
}

impl Captures {
    /// Requests admitted or waiting for geometry.
    pub fn pending(&self) -> usize {
        self.records.len() + self.waiting.len()
    }

    /// Bytes reserved for admitted images; counted against [`MAX_TOTAL_BYTES`].
    pub fn reserved_bytes(&self) -> usize {
        self.records.iter().map(|r| r.reserved_bytes).sum()
    }

    /// Whether any request is pending.
    pub fn any_pending(&self) -> bool {
        self.pending() > 0
    }

    /// Requests not yet issued (the app keeps repainting while any).
    pub fn any_unissued(&self) -> bool {
        !self.waiting.is_empty() || self.records.iter().any(|r| !r.issued())
    }

    /// The geometry recorded by the last [`Self::note_geometry`] for `viewport`, if any.
    pub fn last_geometry(&self, viewport: ViewportId) -> Option<CaptureGeometry> {
        self.windows.get(&viewport).and_then(|window| window.geometry)
    }

    /// Record this pass's geometry of `viewport`; advances and returns that viewport's epoch when
    /// it changed.
    pub fn note_geometry(&mut self, viewport: ViewportId, geometry: CaptureGeometry) -> u64 {
        let window = self.windows.entry(viewport).or_default();
        let changed = window.geometry.is_none_or(|old| !old.same_layout(&geometry));
        if changed {
            window.epoch = window.epoch.wrapping_add(1);
        }
        window.geometry = Some(geometry);
        window.epoch
    }

    /// The app's one call per pass: for the root and each viewport a pending or waiting request
    /// names, records `CaptureGeometry::from_facts` of `host.screenshot_facts(viewport)`, or marks
    /// the viewport closed when that is `None`. Viewports no request names are forgotten, so a
    /// window measured for an earlier request is measured afresh for the next.
    pub fn note_windows(&mut self, host: &WindowHost, document: Option<DocumentId>) {
        let mut named = vec![ViewportId::ROOT];
        for viewport in self.records.iter().map(|r| r.viewport).chain(self.waiting.iter().map(|w| w.viewport)) {
            if !named.contains(&viewport) {
                named.push(viewport);
            }
        }
        self.windows.retain(|viewport, _| named.contains(viewport));
        for viewport in named {
            match host.screenshot_facts(viewport) {
                Some(facts) => {
                    self.note_geometry(viewport, CaptureGeometry::from_facts(facts, document));
                }
                None if host.is_open(viewport) => {}
                None => self.windows.entry(viewport).or_default().geometry = None,
            }
        }
    }

    /// Admit a request for `viewport` against the budget. `Err` returns the reply sender with the
    /// error text:
    /// `screenshot token {token} is already pending`,
    /// `too many screenshots pending (at most {MAX_PENDING})`,
    /// `screenshot {token} failed: {WINDOW_CLOSED}`,
    /// `the viewport is not capturable ({w}×{h} points at {ppp} px/pt, texture side {side})`,
    /// `a {N} MiB screenshot is over the {MAX_IMAGE_BYTES / 1 MiB} MiB per-image limit` (`N` the
    /// expected bytes / 1 MiB, integer division), or
    /// `screenshot budget exhausted: {total} bytes reserved of a {MAX_TOTAL_BYTES}-byte aggregate limit`, or
    /// `screenshot {token} failed: the region must lie inside the {w}×{h}-point window`. The budget is reserved for
    /// the whole window, region or not: the slot retains the whole image until it is cropped.
    /// While the viewport's geometry is not recorded the request waits.
    pub fn admit(&mut self, request: ScreenshotRequest, now: Instant) -> Result<(), (Sender<ControlResponse>, String)> {
        let ScreenshotRequest { token, path, region, viewport, reply, document, settle_ms } = request;
        if self.records.iter().any(|r| r.token == token) || self.waiting.iter().any(|w| w.token == token) {
            return Err((reply, format!("screenshot token {token} is already pending")));
        }
        if self.pending() >= MAX_PENDING {
            return Err((reply, format!("too many screenshots pending (at most {MAX_PENDING})")));
        }
        let request = WaitingAdmit { token, viewport, path, region, reply, deadline: now + DEADLINE, settle_ms, document };
        match self.windows.get(&viewport).copied() {
            None => {
                self.waiting.push(request);
                Ok(())
            }
            Some(Window { geometry: None, .. }) => Err((request.reply, window_closed_error(token))),
            Some(Window { geometry: Some(geometry), epoch }) => self.admit_with_geometry(request, geometry, epoch),
        }
    }

    /// Queues `request` at `geometry` and `epoch`, what its viewport's window recorded.
    fn admit_with_geometry(&mut self, request: WaitingAdmit, geometry: CaptureGeometry, epoch: u64) -> Result<(), (Sender<ControlResponse>, String)> {
        let WaitingAdmit { token, viewport, path, region, reply, deadline, settle_ms, document } = request;
        // The request's clock started when it arrived.
        let arrived = deadline - DEADLINE;
        if self.records.iter().any(|r| r.token == token) {
            return Err((reply, format!("screenshot token {token} is already pending")));
        }
        if self.records.len() >= MAX_PENDING {
            return Err((reply, format!("too many screenshots pending (at most {MAX_PENDING})")));
        }
        let expected = match image_budget(&geometry) {
            Ok(bytes) => bytes,
            Err(e) => return Err((reply, e)),
        };
        if let Err(e) = check_region(token, region, &geometry) {
            return Err((reply, e));
        }
        if let Err(e) = aggregate_budget(self.reserved_bytes(), expected) {
            return Err((reply, e));
        }
        // `Duration::from_secs_f64` panics on NaN, infinity and negatives.
        let settle = if settle_ms.is_finite() { settle_ms.clamp(0.0, 2000.0) } else { BASE_SETTLE_MS };
        self.records.push(PendingCapture {
            token,
            viewport,
            path,
            region,
            reply: Some(reply),
            deadline,
            ready_at: arrived + Duration::from_secs_f64(settle / 1000.0),
            settle_passes: 0,
            geometry,
            epoch,
            issued_epoch: None,
            reserved_bytes: expected,
            document,
            slot: None,
            retries: 0,
        });
        Ok(())
    }

    /// Move waiting requests into the queue once their viewport's geometry is known, fail those
    /// whose window is closed, and expire the rest at their deadline.
    pub fn admit_waiting(&mut self, now: Instant) {
        if self.waiting.is_empty() {
            return;
        }
        let parked = std::mem::take(&mut self.waiting);
        let mut still_waiting = Vec::new();
        for w in parked {
            if now >= w.deadline {
                let _ = w.reply.send(serde_json::json!({"ok": false, "error": timeout_error(w.token)}));
                continue;
            }
            let admitted = match self.windows.get(&w.viewport).copied() {
                None => {
                    still_waiting.push(w);
                    continue;
                }
                Some(Window { geometry: None, .. }) => Err((w.reply, window_closed_error(w.token))),
                Some(Window { geometry: Some(geometry), epoch }) => self.admit_with_geometry(w, geometry, epoch),
            };
            if let Err((reply, e)) = admitted {
                let _ = reply.send(serde_json::json!({"ok": false, "error": e}));
            }
        }
        self.waiting = still_waiting;
    }

    /// One real UI pass happened (settling).
    pub fn note_pass(&mut self) {
        for r in &mut self.records {
            r.settle_passes = r.settle_passes.saturating_add(1);
        }
    }

    /// UI phase: issue due requests through `host` (a fresh slot each), each in its own viewport,
    /// and return the ones that expired or whose window closed meanwhile.
    pub fn issue(&mut self, host: &WindowHost, now: Instant, document: Option<DocumentId>) -> Vec<Finished> {
        let mut finished = Vec::new();
        for i in 0..self.records.len() {
            if self.records[i].issued() {
                continue;
            }
            let token = self.records[i].token;
            if now >= self.records[i].deadline {
                finished.push(finish(&mut self.records[i], Err(timeout_error(token))));
                continue;
            }
            let Some(window) = self.windows.get(&self.records[i].viewport).copied() else { continue };
            let Some(current) = window.geometry else {
                finished.push(finish(&mut self.records[i], Err(window_closed_error(token))));
                continue;
            };
            if !same_activation(document, self.records[i].document) || !same_activation(current.document, self.records[i].document) {
                finished.push(finish(&mut self.records[i], Err(document_changed_error(token))));
                continue;
            }
            // A document switch inside the pass means neither the drawn frame nor the pass's
            // document identity is trustworthy.
            if !same_activation(current.document, document) {
                continue;
            }
            // A minimised window has no surface to read back.
            if current.minimized {
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the window is minimized"))));
                continue;
            }
            if self.records[i].settle_passes < SETTLE_PASSES || now < self.records[i].ready_at {
                continue;
            }
            // No pixels exist yet: finalise the geometry at issue, re-checking the budget, and
            // let the new layout settle for another pass.
            if self.records[i].epoch != window.epoch || !current.same_layout(&self.records[i].geometry) {
                let others: usize = self.records.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.reserved_bytes).sum();
                if let Err(e) = revalidate_reservation(&mut self.records[i], others, current, window.epoch) {
                    finished.push(finish(&mut self.records[i], Err(e)));
                }
                continue;
            }
            let slot = host.screenshot(self.records[i].viewport, MAX_IMAGE_BYTES);
            let r = &mut self.records[i];
            r.slot = Some(slot);
            r.issued_epoch = Some(window.epoch);
        }
        self.records.retain(|r| !finished.iter().any(|f| f.token == r.token));
        finished
    }

    /// Logic phase: collect completed images (retrying after an epoch advance), expire the late
    /// and fail those whose window closed. Each request is read against the geometry its own
    /// viewport recorded this pass.
    pub fn drain(&mut self, now: Instant, document: Option<DocumentId>) -> Vec<Finished> {
        // Requests that arrived before their viewport was measured start here, once it is.
        self.admit_waiting(now);
        let mut finished = Vec::new();
        for i in 0..self.records.len() {
            let token = self.records[i].token;
            // The deadline comes before the image: a callback that arrives late is a timeout.
            if now >= self.records[i].deadline {
                finished.push(finish(&mut self.records[i], Err(timeout_error(token))));
                continue;
            }
            let window = self.windows.get(&self.records[i].viewport).copied();
            if window.is_some_and(|window| window.geometry.is_none()) {
                finished.push(finish(&mut self.records[i], Err(window_closed_error(token))));
                continue;
            }
            let current = window.and_then(|window| window.geometry);
            if !same_activation(document, self.records[i].document) || !same_activation(current.and_then(|c| c.document), self.records[i].document) {
                finished.push(finish(&mut self.records[i], Err(document_changed_error(token))));
                continue;
            }
            let Some(slot) = self.records[i].slot.take() else { continue };
            let Some(result) = slot.take() else {
                self.records[i].slot = Some(slot);
                continue;
            };
            let image = match result {
                Ok(image) => image,
                Err(e) => {
                    slot.cancel();
                    finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: {e}"))));
                    continue;
                }
            };
            let Some((current, epoch)) = window.and_then(|window| window.geometry.map(|geometry| (geometry, window.epoch))) else {
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the window geometry is unknown"))));
                continue;
            };
            // An epoch advance since issue means the image may belong to an intermediate layout,
            // even if the values changed back.
            let epoch_advanced = self.records[i].issued_epoch.is_none_or(|issued| issued != epoch);
            let pass_mixed = !same_activation(current.document, document);
            if epoch_advanced || !current.same_layout(&self.records[i].geometry) || pass_mixed {
                slot.cancel();
                if self.records[i].retries >= MAX_RETRIES {
                    finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the layout changed while it was captured"))));
                    continue;
                }
                self.records[i].retries = self.records[i].retries.saturating_add(1);
                let others: usize = self.records.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.reserved_bytes).sum();
                match revalidate_reservation(&mut self.records[i], others, current, epoch) {
                    Ok(()) => self.records[i].ready_at = now,
                    Err(e) => finished.push(finish(&mut self.records[i], Err(e))),
                }
                continue;
            }
            if let Err(e) = check_image(&image, &self.records[i].geometry) {
                slot.cancel();
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: {e}"))));
                continue;
            }
            slot.cancel();
            finished.push(finish(&mut self.records[i], Ok(image)));
        }
        self.records.retain(|r| !finished.iter().any(|f| f.token == r.token));
        finished
    }

    /// Fail every outstanding request with `reason` (quit, device lost).
    pub fn cancel_all(&mut self, reason: &str) -> Vec<Finished> {
        let mut finished = Vec::new();
        for w in self.waiting.drain(..) {
            finished.push(Finished {
                token: w.token,
                path: w.path,
                region: w.region,
                // No geometry was recorded for a waiting request; the result is an error, so nothing reads it.
                geometry: CaptureGeometry::new([0.0, 0.0], 0),
                reply: Some(w.reply),
                result: Err(format!("screenshot {} cancelled: {reason}", w.token)),
            });
        }
        for mut r in self.records.drain(..) {
            if let Some(slot) = r.slot.take() {
                slot.cancel();
            }
            finished.push(Finished {
                token: r.token,
                path: r.path.take(),
                region: r.region,
                geometry: r.geometry,
                reply: r.reply.take(),
                result: Err(format!("screenshot {} cancelled: {reason}", r.token)),
            });
        }
        finished
    }
}

/// The bytes a geometry reserves, or the reason it cannot be captured at all.
fn image_budget(geometry: &CaptureGeometry) -> Result<usize, String> {
    let Some(expected) = geometry.expected_bytes() else {
        return Err(format!(
            "the viewport is not capturable ({}×{} points at {} px/pt, texture side {})",
            geometry.size_points[0], geometry.size_points[1], geometry.pixels_per_point, geometry.max_texture_side
        ));
    };
    if expected > MAX_IMAGE_BYTES {
        return Err(format!("a {} MiB screenshot is over the {} MiB per-image limit", expected / (1 << 20), MAX_IMAGE_BYTES / (1 << 20)));
    }
    Ok(expected)
}

/// Check one reservation against the aggregate budget, excluding the request itself.
fn aggregate_budget(others: usize, expected: usize) -> Result<(), String> {
    let total = others.saturating_add(expected);
    if total > MAX_TOTAL_BYTES {
        return Err(format!("screenshot budget exhausted: {total} bytes reserved of a {MAX_TOTAL_BYTES}-byte aggregate limit"));
    }
    Ok(())
}

/// Whether `region` lies inside the window of `geometry`: the request is refused otherwise, never cropped to a
/// smaller image than it asked for. `None` (the whole window) always fits.
fn check_region(token: u64, region: Option<Rect>, geometry: &CaptureGeometry) -> Result<(), String> {
    let Some(region) = region else { return Ok(()) };
    let [width, height] = geometry.size_points;
    if region.min.x >= 0.0 && region.min.y >= 0.0 && region.max.x <= width && region.max.y <= height {
        return Ok(());
    }
    Err(format!("screenshot {token} failed: the region must lie inside the {width}×{height}-point window"))
}

/// Finalise a request at a new geometry: recheck the per-image and aggregate limits (excluding
/// its old reservation) and that its region still fits, take the new reservation and restart settling.
fn revalidate_reservation(r: &mut PendingCapture, others: usize, geometry: CaptureGeometry, epoch: u64) -> Result<(), String> {
    let expected = image_budget(&geometry)?;
    aggregate_budget(others, expected)?;
    check_region(r.token, r.region, &geometry)?;
    r.reserved_bytes = expected;
    r.geometry = geometry;
    r.epoch = epoch;
    r.issued_epoch = None;
    r.settle_passes = 0;
    Ok(())
}

fn finish(r: &mut PendingCapture, result: Result<Arc<ColorImage>, String>) -> Finished {
    if let Some(slot) = r.slot.take() {
        slot.cancel();
    }
    Finished { token: r.token, path: r.path.take(), region: r.region, geometry: r.geometry, reply: r.reply.take(), result }
}

/// The returned image must be the issued viewport (within a pixel) or a uniform renderer fit
/// bounded by `max_texture_side`. A smaller same-aspect image is not accepted: it could be a stale
/// or downscaled surface. A different aspect ratio is a resize.
fn check_image(image: &ColorImage, geometry: &CaptureGeometry) -> Result<(), String> {
    let Some([w, h]) = geometry.expected_pixels() else {
        return Err("the captured geometry has no valid pixel size".into());
    };
    let [iw, ih] = image.size;
    if iw == 0 || ih == 0 {
        return Err("the image has a zero side".into());
    }
    if iw > w.saturating_add(1) || ih > h.saturating_add(1) {
        return Err(format!("the image is {iw}×{ih}, larger than the issued viewport {w}×{h}"));
    }
    let close = |a: usize, b: usize| a.abs_diff(b) <= 1;
    if close(iw, w) && close(ih, h) {
        return Ok(());
    }
    let side = geometry.max_texture_side.max(1);
    let fit = (side as f64 / w.max(h) as f64).min(1.0);
    let fw = (w as f64 * fit).round() as usize;
    let fh = (h as f64 * fit).round() as usize;
    if fw >= 1 && fh >= 1 && close(iw, fw) && close(ih, fh) {
        return Ok(());
    }
    let expected_aspect = w as f32 / h as f32;
    let actual_aspect = iw as f32 / ih as f32;
    if !actual_aspect.is_finite() || (actual_aspect - expected_aspect).abs() > expected_aspect * 0.02 {
        return Err(format!("the image is {iw}×{ih}, not the issued {w}×{h} aspect ratio"));
    }
    Err(format!("the image is {iw}×{ih}, neither the issued {w}×{h} viewport nor its {fw}×{fh} texture-side fit"))
}

/// Whether two optional document identities are the same document in the same activation.
fn same_activation(a: Option<DocumentId>, b: Option<DocumentId>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.same_document(b),
        (None, None) => true,
        _ => false,
    }
}

fn timeout_error(token: u64) -> String {
    format!("screenshot {token} timed out: no frame was presented within {} s", DEADLINE.as_secs())
}

fn window_closed_error(token: u64) -> String {
    format!("screenshot {token} failed: {WINDOW_CLOSED}")
}

fn document_changed_error(token: u64) -> String {
    format!("screenshot {token} failed: the document changed while the capture was queued")
}

/// The settle delay for `animation_time`: a base that lets popup animations end plus the
/// animation time itself.
pub fn settle_ms(animation_time: f32) -> f64 {
    f64::from(animation_time) * 2000.0 + BASE_SETTLE_MS
}

/// Encode unmultiplied RGBA pixels as an 8-bit RGBA PNG (the `png` crate, default
/// compression); an encoder error is its message.
pub fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let (w, h) = (u32::try_from(width).map_err(|e| e.to_string())?, u32::try_from(height).map_err(|e| e.to_string())?);
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Writes a screenshot's PNG bytes to an automation path: the app's file authority, limited to its
/// authorized workspace. `Err` is the reply's error text.
pub type PngWriter<'a> = dyn FnMut(&str, &[u8]) -> Result<(), String> + 'a;

/// A validated `ui.screenshot`: where the PNG goes and which part of the window it shows.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ScreenshotParams {
    /// File to write the PNG to, or `None` to inline it.
    pub path: Option<String>,
    /// The part of the window to capture as `[x0, y0, x1, y1]` in points, finite and ordered; `None` captures the
    /// whole window.
    pub region: Option<egui::Rect>,
}

impl ScreenshotParams {
    /// `{path?, region?}`, no other fields. `path` is a string; `region` is `[x0, y0, x1, y1]` in points, four finite
    /// numbers with `x0 < x1` and `y0 < y1`: ``"region must be [x0, y0, x1, y1] in points, finite, with x0 < x1 and
    /// y0 < y1"`` otherwise. Whether it lies inside the window is checked at admission, against the window's geometry.
    pub fn from_params(params: &Value) -> Result<ScreenshotParams, String> {
        let p = Params(params).only(None, &["path", "region"])?;
        let p = Params(&p);
        let path = p.str_field("path")?.map(str::to_owned);
        let region = p.0.get("region").map(region_param).transpose()?;
        Ok(ScreenshotParams { path, region })
    }
}

/// `region`: `[x0, y0, x1, y1]`, finite and ordered.
fn region_param(region: &Value) -> Result<Rect, String> {
    const SHAPE: &str = "region must be [x0, y0, x1, y1] in points, finite, with x0 < x1 and y0 < y1";
    let number = |v: &Value| v.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
    let corners = match region.as_array().map(Vec::as_slice) {
        Some([x0, y0, x1, y1]) => number(x0).zip(number(y0)).zip(number(x1).zip(number(y1))),
        _ => None,
    };
    corners
        .filter(|((x0, y0), (x1, y1))| x0 < x1 && y0 < y1)
        .map(|((x0, y0), (x1, y1))| Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)))
        .ok_or_else(|| SHAPE.into())
}

/// The pixel columns and rows `[x0, y0, x1, y1)` of `region` in an image of `size` captured at `geometry`. The image
/// scale is `size / geometry.size_points`, never `pixels_per_point`: a renderer that fitted the surface to its
/// texture side returns a smaller image of the same layout. Edges round to the nearest pixel and are clamped to the
/// image; `None` when nothing of the region is left or the geometry has no area.
fn crop_pixels(region: Rect, geometry: &CaptureGeometry, size: [usize; 2]) -> Option<[usize; 4]> {
    let [width, height] = geometry.size_points;
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return None;
    }
    let (sx, sy) = (size[0] as f32 / width, size[1] as f32 / height);
    // `as usize` saturates: a negative or NaN edge is 0, an edge past the image is clamped below.
    let px = |v: f32, scale: f32, max: usize| ((v * scale).round() as usize).min(max);
    let [x0, x1] = [px(region.min.x, sx, size[0]), px(region.max.x, sx, size[0])];
    let [y0, y1] = [px(region.min.y, sy, size[1]), px(region.max.y, sy, size[1])];
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

/// The reply for a finished image, cropped to `region` (in points; `None` keeps the whole image) at the scale of the
/// image to `geometry`, the layout it was captured from:
/// - a region with no pixels left in the image:
///   `{"ok": false, "error": "screenshot {token} failed: the region has no pixels in the captured image"}`;
/// - a crop over [`MAX_IMAGE_BYTES`] as RGBA (or whose size overflows):
///   `{"ok": false, "error": "screenshot {token} failed: the image exceeds the {MAX_IMAGE_BYTES / 1 MiB} MiB encode budget"}`;
/// - no `path`: `{"ok": true, "result": {"base64": <standard base64 of the PNG>, "mime": "image/png", "width", "height",
///   "pixels_per_point"}}`, with no size limit beyond the encode budget;
/// - a `path`: the PNG goes to `write(path, bytes)`, the app's file authority (limited to its
///   authorized workspace), and the reply is
///   `{"ok": true, "result": {"path", "width", "height", "pixels_per_point"}}`;
/// - an encode or write error `e`: `{"ok": false, "error": e}`.
///
/// `width` and `height` are the crop's pixels; `pixels_per_point` is `geometry`'s, the window's scale.
/// Flatten the pixels to RGBA and drop the image before encoding, so the two copies don't overlap.
pub fn screenshot_reply(
    token: u64,
    image: Arc<ColorImage>,
    region: Option<egui::Rect>,
    geometry: CaptureGeometry,
    path: Option<&str>,
    write: &mut PngWriter<'_>,
) -> ControlResponse {
    let [w, h] = image.size;
    let crop = match region {
        None => Some([0, 0, w, h]),
        Some(region) => crop_pixels(region, &geometry, [w, h]),
    };
    let Some([x0, y0, x1, y1]) = crop.filter(|[x0, y0, x1, y1]| x0 < x1 && y0 < y1) else {
        return serde_json::json!({"ok": false, "error": format!("screenshot {token} failed: the region has no pixels in the captured image")});
    };
    let (cw, ch) = (x1 - x0, y1 - y0);
    let Some(bytes) = cw.checked_mul(ch).and_then(|n| n.checked_mul(4)).filter(|&n| n <= MAX_IMAGE_BYTES) else {
        return serde_json::json!({
            "ok": false,
            "error": format!("screenshot {token} failed: the image exceeds the {} MiB encode budget", MAX_IMAGE_BYTES / (1 << 20)),
        });
    };
    let mut rgba = Vec::with_capacity(bytes);
    for y in y0..y1 {
        let row = y.checked_mul(w).and_then(|start| image.pixels.get(start + x0..start + x1));
        let Some(row) = row else {
            return serde_json::json!({"ok": false, "error": format!("screenshot {token} failed: the image has fewer pixels than its size")});
        };
        rgba.extend(row.iter().flat_map(|c| c.to_array()));
    }
    drop(image);
    let png = match encode_png(cw, ch, &rgba) {
        Ok(png) => png,
        Err(e) => return serde_json::json!({"ok": false, "error": e}),
    };
    let ppp = geometry.pixels_per_point;
    let Some(path) = path else {
        return serde_json::json!({
            "ok": true,
            "result": {
                "base64": base64::engine::general_purpose::STANDARD.encode(png),
                "mime": "image/png",
                "width": cw,
                "height": ch,
                "pixels_per_point": ppp,
            },
        });
    };
    match write(path, &png) {
        Ok(()) => serde_json::json!({"ok": true, "result": {"path": path, "width": cw, "height": ch, "pixels_per_point": ppp}}),
        Err(e) => serde_json::json!({"ok": false, "error": e}),
    }
}
