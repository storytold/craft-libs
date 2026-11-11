//! `ui.screenshot`: bounded, cancellable capture requests over [`WindowHost::screenshot`].
//!
//! The slot correlates one capture with one image. This queue owns what the slot leaves to the
//! app: request identity (token, output path, reply), deadlines and settling, the admission budget
//! ([`MAX_PENDING`] requests, [`MAX_IMAGE_BYTES`] per image, [`MAX_TOTAL_BYTES`] in all), the
//! document identity fixed at admission, the geometry epoch recorded at issue so an image is saved
//! only for the layout it was rendered from (a size, scale, zoom or minimised change between issue
//! and collection is a retry, at most [`MAX_RETRIES`]), and a monotonic clock. A request that
//! arrives before the first UI pass recorded the geometry waits instead of failing.
//!
//! Phases: [`Captures::issue`] runs in the UI pass (settling counts real layout passes; a
//! discarded pass is skipped); [`Captures::note_geometry`] and [`Captures::drain`] run in the
//! logic phase. The image is encoded and written on the UI thread by
//! [`ControlChannel::finish_captures`](crate::control::ControlChannel::finish_captures), never
//! inside the callback.

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use base64::Engine as _;
use craft_ui::runtime::windows::{ScreenshotFacts, ScreenshotSlot, WindowHost};
use egui::{ColorImage, ViewportId};

use crate::control::ControlResponse;

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

/// The layout of the root viewport a capture is rendered from: size in points, pixels per point,
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

/// One outstanding request; holds the strong slot.
pub struct PendingCapture {
    token: u64,
    path: Option<String>,
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
    /// Where to send the answer.
    pub reply: Option<Sender<ControlResponse>>,
    /// The captured image, or the error text for the reply.
    pub result: Result<Arc<ColorImage>, String>,
}

impl Finished {
    /// A finished request for `token`: `result` answers `reply`, and an image is written to `path` when given.
    pub fn new(token: u64, path: Option<String>, reply: Option<Sender<ControlResponse>>, result: Result<Arc<ColorImage>, String>) -> Self {
        Self { token, path, reply, result }
    }
}

/// A request admitted before any geometry was recorded.
struct WaitingAdmit {
    token: u64,
    path: Option<String>,
    reply: Sender<ControlResponse>,
    deadline: Instant,
    settle_ms: f64,
    document: Option<DocumentId>,
}

/// The screenshot request queue and its admission budget.
#[derive(Default)]
pub struct Captures {
    records: Vec<PendingCapture>,
    waiting: Vec<WaitingAdmit>,
    geometry: Option<CaptureGeometry>,
    epoch: u64,
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

    /// The geometry recorded by the last [`Self::note_geometry`], if any.
    pub fn last_geometry(&self) -> Option<CaptureGeometry> {
        self.geometry
    }

    /// Record this pass's geometry; advances and returns the epoch when it changed.
    pub fn note_geometry(&mut self, geometry: CaptureGeometry) -> u64 {
        let changed = self.geometry.is_none_or(|old| !old.same_layout(&geometry));
        if changed {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.geometry = Some(geometry);
        self.epoch
    }

    /// Admit a request against the budget. `Err` returns the reply sender with the error text:
    /// `screenshot token {token} is already pending`,
    /// `too many screenshots pending (at most {MAX_PENDING})`,
    /// `the viewport is not capturable ({w}×{h} points at {ppp} px/pt, texture side {side})`,
    /// `a {N} MiB screenshot is over the {MAX_IMAGE_BYTES / 1 MiB} MiB per-image limit` (`N` the
    /// expected bytes / 1 MiB, integer division), or
    /// `screenshot budget exhausted: {total} bytes reserved of a {MAX_TOTAL_BYTES}-byte aggregate limit`. With no geometry yet the request waits.
    pub fn admit(
        &mut self,
        token: u64,
        path: Option<String>,
        reply: Sender<ControlResponse>,
        document: Option<DocumentId>,
        settle_ms: f64,
        now: Instant,
    ) -> Result<(), (Sender<ControlResponse>, String)> {
        if self.records.iter().any(|r| r.token == token) || self.waiting.iter().any(|w| w.token == token) {
            return Err((reply, format!("screenshot token {token} is already pending")));
        }
        if self.pending() >= MAX_PENDING {
            return Err((reply, format!("too many screenshots pending (at most {MAX_PENDING})")));
        }
        if self.geometry.is_none() {
            self.waiting.push(WaitingAdmit { token, path, reply, deadline: now + DEADLINE, settle_ms, document });
            return Ok(());
        }
        self.admit_with_geometry(token, path, reply, document, settle_ms, now)
    }

    fn admit_with_geometry(
        &mut self,
        token: u64,
        path: Option<String>,
        reply: Sender<ControlResponse>,
        document: Option<DocumentId>,
        settle_ms: f64,
        now: Instant,
    ) -> Result<(), (Sender<ControlResponse>, String)> {
        if self.records.iter().any(|r| r.token == token) {
            return Err((reply, format!("screenshot token {token} is already pending")));
        }
        if self.records.len() >= MAX_PENDING {
            return Err((reply, format!("too many screenshots pending (at most {MAX_PENDING})")));
        }
        let Some(geometry) = self.geometry else {
            return Err((reply, "the window geometry is not known yet; try again after a frame has been drawn".into()));
        };
        let expected = match image_budget(&geometry) {
            Ok(bytes) => bytes,
            Err(e) => return Err((reply, e)),
        };
        if let Err(e) = aggregate_budget(self.reserved_bytes(), expected) {
            return Err((reply, e));
        }
        // `Duration::from_secs_f64` panics on NaN, infinity and negatives.
        let settle = if settle_ms.is_finite() { settle_ms.clamp(0.0, 2000.0) } else { BASE_SETTLE_MS };
        self.records.push(PendingCapture {
            token,
            path,
            reply: Some(reply),
            deadline: now + DEADLINE,
            ready_at: now + Duration::from_secs_f64(settle / 1000.0),
            settle_passes: 0,
            geometry,
            epoch: self.epoch,
            issued_epoch: None,
            reserved_bytes: expected,
            document,
            slot: None,
            retries: 0,
        });
        Ok(())
    }

    /// Move waiting requests into the queue once geometry is known; expire the rest.
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
            if self.geometry.is_none() {
                still_waiting.push(w);
                continue;
            }
            // The request's clock started when it arrived.
            let arrived = w.deadline - DEADLINE;
            if let Err((reply, e)) = self.admit_with_geometry(w.token, w.path, w.reply, w.document, w.settle_ms, arrived) {
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

    /// UI phase: issue due requests through `host` (a fresh slot each), return the ones that
    /// expired meanwhile.
    pub fn issue(&mut self, host: &WindowHost, now: Instant, document: Option<DocumentId>) -> Vec<Finished> {
        let mut finished = Vec::new();
        let Some(current) = self.geometry else { return finished };
        // A document switch inside the pass means neither the drawn frame nor the pass's
        // document identity is trustworthy.
        let pass_changed = !same_activation(current.document, document);
        for i in 0..self.records.len() {
            if self.records[i].issued() {
                continue;
            }
            let token = self.records[i].token;
            if now >= self.records[i].deadline {
                finished.push(finish(&mut self.records[i], Err(timeout_error(token))));
                continue;
            }
            if !same_activation(document, self.records[i].document) || !same_activation(current.document, self.records[i].document) {
                finished.push(finish(&mut self.records[i], Err(document_changed_error(token))));
                continue;
            }
            if pass_changed {
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
            if self.records[i].epoch != self.epoch || !current.same_layout(&self.records[i].geometry) {
                let others: usize = self.records.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.reserved_bytes).sum();
                let epoch = self.epoch;
                if let Err(e) = revalidate_reservation(&mut self.records[i], others, current, epoch) {
                    finished.push(finish(&mut self.records[i], Err(e)));
                }
                continue;
            }
            let slot = host.screenshot(ViewportId::ROOT, MAX_IMAGE_BYTES);
            let r = &mut self.records[i];
            r.slot = Some(slot);
            r.issued_epoch = Some(self.epoch);
        }
        self.records.retain(|r| !finished.iter().any(|f| f.token == r.token));
        finished
    }

    /// Logic phase: collect completed images (retrying after an epoch advance), expire the late.
    pub fn drain(&mut self, now: Instant, current: Option<CaptureGeometry>, document: Option<DocumentId>) -> Vec<Finished> {
        // Requests that arrived before the first UI pass start here, once its geometry is known.
        self.admit_waiting(now);
        let mut finished = Vec::new();
        for i in 0..self.records.len() {
            let token = self.records[i].token;
            // The deadline comes before the image: a callback that arrives late is a timeout.
            if now >= self.records[i].deadline {
                finished.push(finish(&mut self.records[i], Err(timeout_error(token))));
                continue;
            }
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
            let Some(current) = current else {
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the window geometry is unknown"))));
                continue;
            };
            // An epoch advance since issue means the image may belong to an intermediate layout,
            // even if the values changed back.
            let epoch_advanced = self.records[i].issued_epoch.is_none_or(|issued| issued != self.epoch);
            let pass_mixed = !same_activation(current.document, document);
            if epoch_advanced || !current.same_layout(&self.records[i].geometry) || pass_mixed {
                slot.cancel();
                if self.records[i].retries >= MAX_RETRIES {
                    finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the layout changed while it was captured"))));
                    continue;
                }
                self.records[i].retries = self.records[i].retries.saturating_add(1);
                let others: usize = self.records.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.reserved_bytes).sum();
                let epoch = self.epoch;
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

/// Finalise a request at a new geometry: recheck the per-image and aggregate limits (excluding
/// its old reservation), take the new reservation and restart settling.
fn revalidate_reservation(r: &mut PendingCapture, others: usize, geometry: CaptureGeometry, epoch: u64) -> Result<(), String> {
    let expected = image_budget(&geometry)?;
    aggregate_budget(others, expected)?;
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
    Finished { token: r.token, path: r.path.take(), reply: r.reply.take(), result }
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

/// The reply for a finished image:
/// - an image over [`MAX_IMAGE_BYTES`] as RGBA (or whose size overflows):
///   `{"ok": false, "error": "screenshot {token} failed: the image exceeds the {MAX_IMAGE_BYTES / 1 MiB} MiB encode budget"}`;
/// - no `path`: `{"ok": true, "result": {"base64": <standard base64 of the PNG>, "mime": "image/png", "width", "height"}}`,
///   with no size limit beyond the encode budget;
/// - a `path`: the PNG goes to `write(path, bytes)`, the app's file authority (limited to its
///   authorized workspace), and the reply is
///   `{"ok": true, "result": {"path", "width", "height"}}`;
/// - an encode or write error `e`: `{"ok": false, "error": e}`.
///
/// Flatten the pixels to RGBA and drop the image before encoding, so the two copies don't overlap.
pub fn screenshot_reply(token: u64, image: Arc<ColorImage>, path: Option<&str>, write: &mut PngWriter<'_>) -> ControlResponse {
    let [w, h] = image.size;
    let Some(bytes) = w.checked_mul(h).and_then(|n| n.checked_mul(4)).filter(|&n| n <= MAX_IMAGE_BYTES) else {
        return serde_json::json!({
            "ok": false,
            "error": format!("screenshot {token} failed: the image exceeds the {} MiB encode budget", MAX_IMAGE_BYTES / (1 << 20)),
        });
    };
    let mut rgba = Vec::with_capacity(bytes);
    rgba.extend(image.pixels.iter().flat_map(|c| c.to_array()));
    drop(image);
    let png = match encode_png(w, h, &rgba) {
        Ok(png) => png,
        Err(e) => return serde_json::json!({"ok": false, "error": e}),
    };
    let Some(path) = path else {
        return serde_json::json!({
            "ok": true,
            "result": {"base64": base64::engine::general_purpose::STANDARD.encode(png), "mime": "image/png", "width": w, "height": h},
        });
    };
    match write(path, &png) {
        Ok(()) => serde_json::json!({"ok": true, "result": {"path": path, "width": w, "height": h}}),
        Err(e) => serde_json::json!({"ok": false, "error": e}),
    }
}
