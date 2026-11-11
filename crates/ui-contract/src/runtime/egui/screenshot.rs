//! One-shot screenshot slots and the facts a screenshot is measured against.
//!
//! The application retains request ids, output paths, deadlines, settling policy, geometry epochs and aggregate
//! admission budgets. This module correlates one screenshot command with one slot, checks the retained image
//! against a per-image byte bound, and wakes the owning viewport.
//!
//! [`issue`] creates the slot, sends the screenshot command to the viewport and asks for a pass while the window
//! is hidden. The command's callback holds only a weak reference to the slot, so dropping or cancelling the
//! request releases its image and makes late results inert.
//!
//! Every retry is a new [`issue`] call; a callback is one-shot, and a slot ignores completions after its result
//! was taken, cancelled or dropped. The slot does not track whether a request is still open: the application's
//! request record keeps that pending/closed distinction.
//!
//! A completion that publishes its result just before a concurrent [`ScreenshotSlot::cancel`] or drop can still
//! deliver its single wakeup afterwards. That extra repaint is harmless: if the cancellation won, the image is
//! released and a later [`ScreenshotSlot::take`] returns `None`; if `take` won first, the caller already owns the
//! result. A completion that observes a cancelled or taken slot stores no image and does not wake. This module
//! never waits for another thread while holding the slot lock.
//!
//! [`ScreenshotSlot`] holds a strong [`Context`]. Keep it in the application's request record; storing it in
//! `Context`-owned data forms a `Context` to slot to `Context` cycle that keeps both alive.

use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use egui::{Color32, ColorImage, Context, ScreenshotCallback, Vec2, ViewportCommand, ViewportId};

use super::viewports;

/// Why a completed screenshot was rejected and released.
///
/// Match the known variants and use a wildcard arm for a future rejection reason.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenshotError {
    /// The checked `width * height` pixel count, or its `width * height * size_of::<Color32>()` byte count,
    /// overflowed `usize`.
    #[non_exhaustive]
    DimensionsOverflow {
        /// Reported width in pixels.
        width: usize,
        /// Reported height in pixels.
        height: usize,
    },
    /// The pixel buffer does not match the reported image dimensions, or the image has zero width or height.
    #[non_exhaustive]
    MalformedImage {
        /// Reported width in pixels.
        width: usize,
        /// Reported height in pixels.
        height: usize,
        /// Length of the pixel buffer.
        pixels: usize,
    },
    /// The checked image size exceeds the slot's per-image byte bound.
    #[non_exhaustive]
    ImageTooLarge {
        /// Size of the image in bytes.
        bytes: usize,
        /// The slot's per-image byte bound.
        max_bytes: usize,
    },
}

impl core::fmt::Display for ScreenshotError {
    /// A human-readable rejection reason; the wording is informative only.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DimensionsOverflow { width, height } => {
                write!(f, "screenshot dimensions {width}×{height} overflow the checked pixel or byte count")
            }
            Self::MalformedImage { width, height, pixels } => write!(f, "screenshot has {pixels} pixels for its {width}×{height} dimensions"),
            Self::ImageTooLarge { bytes, max_bytes } => write!(f, "screenshot uses {bytes} bytes, over the {max_bytes}-byte per-image limit"),
        }
    }
}

impl std::error::Error for ScreenshotError {}

enum SlotState {
    Pending,
    Ready(Result<Arc<ColorImage>, ScreenshotError>),
    Taken,
    Cancelled,
}

struct Shared {
    state: Mutex<SlotState>,
    /// The live request owns the strong context; the callback only upgrades it briefly to wake.
    context: Context,
    viewport: ViewportId,
    max_image_bytes: usize,
}

/// The live request side of a screenshot. Keep it alive until the result is taken or the request is cancelled;
/// dropping it cancels the completion and releases any retained image.
///
/// The type is `Send + Sync`; the application may hold it in its request record and poll [`Self::take`] from its
/// logic pass.
#[must_use = "the slot must be kept alive to receive and take the screenshot"]
pub struct ScreenshotSlot {
    shared: Arc<Shared>,
}

/// What a screenshot of the running viewport measures this pass.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenshotFacts {
    /// The viewport's inner size in points.
    pub size_points: Vec2,
    /// Pixels per point: the native scale times the interface zoom.
    pub pixels_per_point: f32,
    /// The interface zoom factor.
    pub zoom: f32,
    /// The renderer's largest texture side, in pixels.
    pub max_texture_side: usize,
    /// Whether the window is minimized.
    pub minimized: bool,
}

/// Issues one screenshot of `viewport`: a fresh slot bounded to `max_image_bytes` for its one image, the viewport
/// command, and a pass even while the window is hidden. `max_image_bytes` bounds the single retained image only;
/// the application reserves its aggregate budget before issuing the request.
pub(crate) fn issue(ctx: &Context, viewport: ViewportId, max_image_bytes: usize) -> ScreenshotSlot {
    let (slot, callback) = ScreenshotSlot::with_callback(ctx, viewport, max_image_bytes);
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::Screenshot(callback));
    viewports::paint_while_hidden(ctx, viewport);
    slot
}

/// What a screenshot of the running viewport measures this pass.
pub(crate) fn facts(ctx: &Context) -> ScreenshotFacts {
    let window = viewports::current(ctx);
    let size_points = ctx.input(|input| window.inner.map_or_else(|| input.viewport_rect().size(), |inner| inner.size()));
    let max_texture_side = ctx.input(|input| input.max_texture_side);
    let zoom = ctx.zoom_factor();
    let pixels_per_point =
        window.native_scale.filter(|scale| scale.is_finite() && *scale > 0.0).map_or_else(|| ctx.pixels_per_point(), |scale| scale * zoom);
    ScreenshotFacts { size_points, pixels_per_point, zoom, max_texture_side, minimized: window.minimized }
}

impl ScreenshotSlot {
    fn with_callback(ctx: &Context, viewport: ViewportId, max_image_bytes: usize) -> (Self, ScreenshotCallback) {
        let shared = Arc::new(Shared { state: Mutex::new(SlotState::Pending), context: ctx.clone(), viewport, max_image_bytes });
        let weak = Arc::downgrade(&shared);
        let callback = ScreenshotCallback::new(move |image| complete(&weak, image));
        (Self { shared }, callback)
    }

    /// Takes the completed result, at most once.
    ///
    /// Returns `None` while the screenshot is pending, after it was cancelled or dropped, and after a previous
    /// call took the image or error. Invalid images come back as an error so the application can report the
    /// failure with its own request identity.
    pub fn take(&self) -> Option<Result<Arc<ColorImage>, ScreenshotError>> {
        let mut state = lock(&self.shared.state);
        if !matches!(&*state, SlotState::Ready(_)) {
            return None;
        }
        match core::mem::replace(&mut *state, SlotState::Taken) {
            SlotState::Ready(result) => Some(result),
            _ => None,
        }
    }

    /// Cancels the request and releases any completed image. A completion that published its result before this
    /// call may still deliver one harmless repaint; the slot retains no image and a later [`Self::take`] returns
    /// `None`. Any retained image is dropped after the slot mutex is unlocked.
    pub fn cancel(&self) {
        close(&self.shared);
    }

    /// A slot with its callback and no command sent, for trials that stand in for the renderer.
    #[cfg(feature = "testing")]
    pub fn detached(ctx: &Context, viewport: ViewportId, max_image_bytes: usize) -> (Self, ScreenshotCallback) {
        Self::with_callback(ctx, viewport, max_image_bytes)
    }

    /// Completes the slot with `image` as the renderer would, without the callback.
    #[cfg(feature = "testing")]
    pub fn complete(&self, image: Arc<ColorImage>) {
        complete(&Arc::downgrade(&self.shared), image);
    }

    /// Whether the slot mutex is free, proving the wakeup runs after the lock was released.
    #[cfg(feature = "testing")]
    pub fn lock_is_free(&self) -> bool {
        self.shared.state.try_lock().is_ok()
    }
}

impl Drop for ScreenshotSlot {
    /// Cancels the request as the slot goes away. A later completion upgrades no live state and releases its
    /// image, a retained ready image is dropped, and no repaint is requested.
    fn drop(&mut self) {
        close(&self.shared);
    }
}

impl core::fmt::Debug for ScreenshotSlot {
    /// Shows only the viewport, so logging cannot hold or dump screenshot contents.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScreenshotSlot").field("viewport", &self.shared.viewport).finish_non_exhaustive()
    }
}

/// Locks the slot state, recovering from poisoning. The guard must be released before entering the egui context
/// or a repaint observer.
fn lock(state: &Mutex<SlotState>) -> MutexGuard<'_, SlotState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks the slot closed and drops any retained image after releasing the mutex.
fn close(shared: &Shared) {
    let previous = {
        let mut state = lock(&shared.state);
        core::mem::replace(&mut *state, SlotState::Cancelled)
    };
    drop(previous);
}

/// Validates a completed image, stores it, and wakes the owning viewport.
///
/// The slot lock is released before calling into the context, so a repaint observer may call
/// [`ScreenshotSlot::take`] or [`ScreenshotSlot::cancel`] re-entrantly. `Context::request_repaint_of` takes
/// egui's context lock; repaint observers run under it, so completing from one would deadlock.
fn complete(weak: &Weak<Shared>, image: Arc<ColorImage>) {
    let Some(shared) = weak.upgrade() else {
        return;
    };
    let result = checked_image(image, shared.max_image_bytes);
    let should_wake = {
        let mut state = lock(&shared.state);
        if matches!(&*state, SlotState::Pending) {
            *state = SlotState::Ready(result);
            true
        } else {
            false
        }
    };
    if should_wake {
        shared.context.request_repaint_of(shared.viewport);
    }
}

/// Validates `image` against the slot's per-image byte bound before publishing it.
///
/// Zero width or height is malformed before any arithmetic; the pixel count and its byte size use checked
/// multiplication; the buffer length must equal `width * height`; and only then is the byte count compared with
/// `max_image_bytes`, rejecting a count greater than it (equality is accepted).
fn checked_image(image: Arc<ColorImage>, max_image_bytes: usize) -> Result<Arc<ColorImage>, ScreenshotError> {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return Err(ScreenshotError::MalformedImage { width, height, pixels: image.pixels.len() });
    }
    let pixels = width.checked_mul(height).ok_or(ScreenshotError::DimensionsOverflow { width, height })?;
    let bytes = pixels.checked_mul(size_of::<Color32>()).ok_or(ScreenshotError::DimensionsOverflow { width, height })?;
    if image.pixels.len() != pixels {
        return Err(ScreenshotError::MalformedImage { width, height, pixels: image.pixels.len() });
    }
    if bytes > max_image_bytes {
        return Err(ScreenshotError::ImageTooLarge { bytes, max_bytes: max_image_bytes });
    }
    Ok(image)
}
