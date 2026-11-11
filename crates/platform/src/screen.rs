//! Screen capture for the screen colour picker (F04.01): every monitor's pixels and ICC profile,
//! taken once when the picker opens. The UI side is `craft_color::ui::screen_picker`.
//!
//! Backends: X11 (`GetImage` per RandR monitor, `_ICC_PROFILE` properties), Wayland (the
//! Screenshot portal's `PickColor`, where the compositor's own picker returns one colour), macOS (CoreGraphics window-list image per display, the
//! image's colour space), Windows (`xcap` per monitor, no profile). Web and sessions without a
//! display have no capture: [`capture`] returns [`ScreenError::Unsupported`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use craft_host::Wake;

/// One monitor's capture.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenImage {
    /// Where the monitor sits in the virtual desktop, in physical pixels.
    pub origin: [i32; 2],
    /// Pixels per row and rows.
    pub size: [u32; 2],
    /// The monitor's scale factor (points to pixels).
    pub scale: f32,
    /// Straight RGBA, row-major, `size[0] * size[1] * 4` bytes.
    pub rgba: Arc<Vec<u8>>,
    /// The monitor's ICC profile bytes when the system has one; the picker converts samples to
    /// sRGB through it.
    pub icc_profile: Option<Arc<Vec<u8>>>,
    /// The monitor's name as the system gives it ("DP-1", "Built-in Retina Display").
    pub name: String,
}

/// What a finished capture holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Capture {
    /// Every monitor's pixels, primary first; the picker shows them and the user picks one.
    Images(Vec<ScreenImage>),
    /// The colour the user picked in the compositor's own picker (Wayland), as sRGB, 0 to 1 each.
    Color([f32; 3]),
}

/// Why a capture failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenError {
    /// No backend on this platform (web) or display session.
    Unsupported,
    /// The user declined the portal's request (Wayland) or the OS's screen-recording permission
    /// (macOS).
    Denied,
    /// [`CaptureHandle::cancel`] was called before the capture finished.
    Cancelled,
    /// Something else, with the backend's message.
    Failed(String),
}

/// A capture in flight on a worker thread; dropping it cancels.
pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    result: Receiver<Result<Capture, ScreenError>>,
}

impl CaptureHandle {
    /// The capture, once it has finished; `None` while it is still running.
    pub fn try_take(&self) -> Option<Result<Capture, ScreenError>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) if self.stop.load(Ordering::Relaxed) => Some(Err(ScreenError::Cancelled)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if self.stop.load(Ordering::Relaxed) => Some(Err(ScreenError::Cancelled)),
            Err(TryRecvError::Disconnected) => Some(Err(ScreenError::Failed("the capture thread ended without a result".into()))),
        }
    }

    /// Stops a capture that has not finished; a later `try_take` gives `Cancelled`.
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Starts capturing on a worker thread: every monitor's image, in monitor order with the primary
/// first (sizes validated against [`MAX_PIXELS`] per image), or on Wayland the one colour the
/// compositor's picker returns. `scale` is the app's native pixels per point, the monitors'
/// scale on X11, which reports none. `wake` runs once the result (or a failure) is ready to
/// take, so the UI polls with [`CaptureHandle::try_take`] only in the frame that follows.
pub fn capture(scale: f32, wake: Wake) -> CaptureHandle {
    spawn(wake, move |stop| run(stop, scale))
}

/// Runs `job` on the capture thread and calls `wake` after its result is queued.
fn spawn(wake: Wake, job: impl FnOnce(&AtomicBool) -> Result<Capture, ScreenError> + Send + 'static) -> CaptureHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_stop = stop.clone();
    let fallback = tx.clone();
    let worker_wake = wake.clone();
    let spawned = std::thread::Builder::new().name("screen-capture".into()).spawn(move || {
        let _ = tx.send(job(&worker_stop));
        worker_wake.wake();
    });
    if let Err(e) = spawned {
        let _ = fallback.send(Err(ScreenError::Failed(format!("could not start the capture thread: {e}"))));
        wake.wake();
    }
    CaptureHandle { stop, result: rx }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn run(stop: &AtomicBool, scale: f32) -> Result<Capture, ScreenError> {
    backend::capture(stop, scale)
}

#[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
fn run(stop: &AtomicBool, _scale: f32) -> Result<Capture, ScreenError> {
    backend::capture(stop).map(Capture::Images)
}

/// The scale a monitor image gets: the app's native pixels per point, or 1 when it has none.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn usable_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

/// The portal's `PickColor` answer as sRGB, `Failed` when a channel is outside 0 to 1.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn picked_color(rgb: [f64; 3]) -> Result<Capture, ScreenError> {
    if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(ScreenError::Failed("the desktop color picker returned an invalid color".into()));
    }
    Ok(Capture::Color(rgb.map(|v| v as f32)))
}

/// The most pixels one monitor image may hold (64 M); larger captures fail with `Failed`.
pub const MAX_PIXELS: usize = 64 * 1024 * 1024;

/// The most monitors one capture covers.
#[cfg(any(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")), target_os = "macos", windows))]
const MAX_MONITORS: usize = 16;

/// Pixel count of a `width` x `height` image, within `1..=MAX_PIXELS`.
#[cfg(any(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")), target_os = "macos", windows, test))]
fn pixel_count(width: usize, height: usize) -> Result<usize, ScreenError> {
    width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= MAX_PIXELS)
        .ok_or_else(|| ScreenError::Failed("the screen capture exceeds the 64 MP memory budget".into()))
}

/// Adds one image's pixels to the running total of a capture; the sum stays within `MAX_PIXELS`.
#[cfg(any(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")), target_os = "macos", windows))]
fn add_to_budget(total: usize, width: usize, height: usize) -> Result<usize, ScreenError> {
    total
        .checked_add(pixel_count(width, height)?)
        .filter(|&n| n <= MAX_PIXELS)
        .ok_or_else(|| ScreenError::Failed("the monitors together exceed the 64 MP memory budget".into()))
}

/// Validates the row stride of a padded pixel buffer and returns the buffer length it implies.
#[cfg(any(target_os = "macos", test))]
fn checked_stride(width: usize, height: usize, stride: usize) -> Result<usize, ScreenError> {
    pixel_count(width, height)?;
    let row = width.checked_mul(4).ok_or_else(|| ScreenError::Failed("invalid screen row".into()))?;
    if stride < row || stride > row.saturating_add(4096) {
        return Err(ScreenError::Failed("invalid screen row stride".into()));
    }
    stride
        .checked_mul(height)
        .filter(|&n| n <= MAX_PIXELS.saturating_mul(4))
        .ok_or_else(|| ScreenError::Failed("the screen buffer exceeds the memory budget".into()))
}

/// Unpacks a strided 8-bit-per-channel buffer to straight, opaque RGBA. `order` gives the byte
/// index of R, G, B and A in a source pixel; `premultiplied` colours are divided by alpha;
/// `opaque` ignores the alpha byte.
#[cfg(any(target_os = "macos", test))]
fn unpack_rgba(
    bytes: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    order: [usize; 4],
    premultiplied: bool,
    opaque: bool,
) -> Result<Vec<u8>, ScreenError> {
    let length = checked_stride(width, height, stride)?;
    if bytes.len() < length || order.iter().any(|&i| i > 3) {
        return Err(ScreenError::Failed("invalid screen pixel buffer".into()));
    }
    let mut out = Vec::with_capacity(pixel_count(width, height)?.saturating_mul(4));
    for row in bytes.chunks_exact(stride).take(height) {
        for p in row[..width * 4].as_chunks::<4>().0 {
            let a = if opaque { 255 } else { p[order[3]] };
            for &i in &order[..3] {
                let v = u32::from(p[i]);
                out.push(if premultiplied && a != 255 {
                    if a == 0 { 0 } else { ((v * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 }
                } else {
                    v as u8
                });
            }
            out.push(255);
        }
    }
    Ok(out)
}

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod backend {
    use super::{Arc, AtomicBool, Capture, MAX_MONITORS, Ordering, ScreenError, ScreenImage, add_to_budget, picked_color, pixel_count, usable_scale};

    const MAX_ICC_PROFILE_BYTES: usize = 1024 * 1024;
    /// Length limit of the property reads, in 32-bit units.
    const ICC_PROPERTY_LONG_LENGTH: u32 = 256 * 1024;
    const PORTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

    /// X11 sessions use `GetImage`; Wayland sessions (also when XWayland is present, which
    /// shows only black for other clients) go through the portal's `PickColor`.
    pub fn capture(stop: &AtomicBool, scale: f32) -> Result<Capture, ScreenError> {
        let wayland =
            std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland");
        if wayland {
            pick_color(stop)
        } else if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
            x11(stop, scale).map(Capture::Images)
        } else {
            Err(ScreenError::Unsupported)
        }
    }

    fn failed(e: impl std::fmt::Display) -> ScreenError {
        ScreenError::Failed(e.to_string())
    }

    /// The profile bytes when they are a plausible RGB ICC profile: size field matching, the
    /// `acsp` signature and an `RGB ` data colour space.
    fn icc_profile(bytes: Vec<u8>) -> Option<Vec<u8>> {
        let header_ok = bytes.len() >= 128
            && bytes.len() <= MAX_ICC_PROFILE_BYTES
            && u32::from_be_bytes(bytes[0..4].try_into().ok()?) as usize == bytes.len()
            && &bytes[36..40] == b"acsp"
            && &bytes[16..20] == b"RGB ";
        header_ok.then_some(bytes)
    }

    fn atom<C: x11rb::connection::RequestConnection>(conn: &C, name: &str) -> Option<u32> {
        use x11rb::protocol::xproto::ConnectionExt as _;
        let atom = conn.intern_atom(true, name.as_bytes()).ok()?.reply().ok()?.atom;
        (atom != 0).then_some(atom)
    }

    fn output_profile<C: x11rb::connection::RequestConnection>(conn: &C, output: u32, atom: u32) -> Option<Vec<u8>> {
        use x11rb::protocol::{randr::ConnectionExt as _, xproto::AtomEnum};
        let reply = conn.randr_get_output_property(output, atom, AtomEnum::CARDINAL, 0, ICC_PROPERTY_LONG_LENGTH, false, false).ok()?.reply().ok()?;
        if reply.type_ != u32::from(AtomEnum::CARDINAL) || reply.format != 8 || reply.bytes_after != 0 {
            return None;
        }
        icc_profile(reply.data)
    }

    fn root_profile<C: x11rb::connection::RequestConnection>(conn: &C, root: u32, atom: u32) -> Option<Vec<u8>> {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
        let reply = conn.get_property(false, root, atom, AtomEnum::ANY, 0, ICC_PROPERTY_LONG_LENGTH).ok()?.reply().ok()?;
        if reply.format != 8 || reply.bytes_after != 0 {
            return None;
        }
        icc_profile(reply.value)
    }

    struct Region {
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        primary: bool,
        name: String,
        outputs: Vec<u32>,
    }

    fn x11(stop: &AtomicBool, scale: f32) -> Result<Vec<ScreenImage>, ScreenError> {
        use x11rb::{
            connection::Connection,
            image::{Image, PixelLayout},
            protocol::{
                randr::ConnectionExt as _,
                xproto::{ConnectionExt as _, VisualClass},
            },
        };
        let (conn, index) = x11rb::connect(None).map_err(failed)?;
        let screen = conn.setup().roots.get(index).ok_or_else(|| failed("the X11 screen is missing"))?;
        let (root_w, root_h) = (screen.width_in_pixels, screen.height_in_pixels);
        pixel_count(usize::from(root_w), usize::from(root_h))?;

        let monitors = conn.randr_get_monitors(screen.root, true).ok().and_then(|c| c.reply().ok()).map(|r| r.monitors).unwrap_or_default();
        let mut regions = Vec::new();
        for m in monitors {
            let name = conn
                .get_atom_name(m.name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                .unwrap_or_default();
            regions.push(Region { x: m.x, y: m.y, width: m.width, height: m.height, primary: m.primary, name, outputs: m.outputs });
        }
        if regions.is_empty() {
            regions.push(Region { x: 0, y: 0, width: root_w, height: root_h, primary: true, name: "Screen".into(), outputs: Vec::new() });
        }
        if regions.len() > MAX_MONITORS {
            return Err(failed("too many monitors for the screen picker"));
        }
        regions.sort_by_key(|r| !r.primary);
        regions[0].primary = true;
        let mut total = 0usize;
        for r in &regions {
            let inside = r.x >= 0
                && r.y >= 0
                && i32::from(r.x) + i32::from(r.width) <= i32::from(root_w)
                && i32::from(r.y) + i32::from(r.height) <= i32::from(root_h);
            if !inside {
                return Err(failed("invalid X11 monitor geometry"));
            }
            total = add_to_budget(total, usize::from(r.width), usize::from(r.height))?;
        }

        let profile_atom = atom(&conn, "_ICC_PROFILE");
        let byte = |v: u16| ((u32::from(v) + 128) / 257) as u8;
        let mut images = Vec::new();
        for r in regions {
            if stop.load(Ordering::Relaxed) {
                return Err(ScreenError::Cancelled);
            }
            let (image, visual_id) =
                Image::get(&conn, screen.root, r.x, r.y, r.width, r.height).map_err(|e| failed(format!("screen capture failed: {e}")))?;
            let visual = screen
                .allowed_depths
                .iter()
                .flat_map(|d| &d.visuals)
                .find(|v| v.visual_id == visual_id)
                .ok_or_else(|| failed("unsupported X11 screen visual"))?;
            if visual.class != VisualClass::TRUE_COLOR {
                return Err(failed("the screen picker needs an X11 TrueColor visual"));
            }
            let layout = PixelLayout::from_visual_type(*visual).map_err(failed)?;
            let mut rgba = Vec::with_capacity(pixel_count(usize::from(r.width), usize::from(r.height))? * 4);
            for row in 0..r.height {
                for col in 0..r.width {
                    let (red, green, blue) = layout.decode(image.get_pixel(col, row));
                    rgba.extend_from_slice(&[byte(red), byte(green), byte(blue), 255]);
                }
            }
            let profile = profile_atom
                .and_then(|a| r.outputs.iter().find_map(|&o| output_profile(&conn, o, a)))
                .or_else(|| if r.primary { profile_atom.and_then(|a| root_profile(&conn, screen.root, a)) } else { None });
            images.push(ScreenImage {
                origin: [i32::from(r.x), i32::from(r.y)],
                size: [u32::from(r.width), u32::from(r.height)],
                scale: usable_scale(scale),
                rgba: Arc::new(rgba),
                icc_profile: profile.map(Arc::new),
                name: r.name,
            });
        }
        Ok(images)
    }

    /// The Screenshot portal's `PickColor`: the compositor's own picker asks the user and returns
    /// one colour. A client cannot place windows over the monitors on Wayland, so the picker
    /// shows no capture view of its own.
    // pc: apps/photocraft/src/screen_color.rs:196 `wayland_color`.
    fn pick_color(stop: &AtomicBool) -> Result<Capture, ScreenError> {
        use futures_lite::StreamExt;
        use std::collections::HashMap;
        use zbus::{
            blocking::Proxy,
            zvariant::{OwnedObjectPath, OwnedValue, Value},
        };
        const PORTAL: &str = "org.freedesktop.portal.Desktop";
        let conn = zbus::blocking::connection::Builder::session()
            .map_err(failed)?
            .method_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(failed)?;
        let unique =
            conn.unique_name().ok_or_else(|| failed("the portal connection has no unique name"))?.as_str().trim_start_matches(':').replace('.', "_");
        let token = format!("craft_screen_{}", std::process::id());
        let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
        let request = Proxy::new(&conn, PORTAL, path.as_str(), "org.freedesktop.portal.Request").map_err(failed)?;
        // Subscribe before the call: the response can arrive before the call returns.
        let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(failed)?;
        let screenshot = Proxy::new(&conn, PORTAL, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot").map_err(failed)?;
        let options: HashMap<&str, Value<'_>> = HashMap::from([("handle_token", Value::from(token.as_str()))]);
        let _handle: OwnedObjectPath =
            screenshot.call("PickColor", &("", options)).map_err(|e| failed(format!("the desktop portal does not support color picking: {e}")))?;

        let deadline = std::time::Instant::now() + PORTAL_TIMEOUT;
        let reply = loop {
            if stop.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline {
                let _: Result<(), _> = request.call("Close", &());
                return Err(ScreenError::Cancelled);
            }
            let next = futures_lite::future::block_on(futures_lite::future::race(async { Some(replies.next().await) }, async {
                async_io::Timer::after(std::time::Duration::from_millis(50)).await;
                None
            }));
            if let Some(message) = next {
                break message.ok_or_else(|| failed("the desktop portal closed without a result"))?;
            }
        };
        let (code, values): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(failed)?;
        match code {
            0 => {}
            1 => return Err(ScreenError::Cancelled),
            _ => return Err(failed("the desktop color picker could not sample this screen")),
        }
        let value = values.get("color").ok_or_else(|| failed("the desktop color picker returned no color"))?;
        let (r, g, b): (f64, f64, f64) = value.try_clone().map_err(failed)?.try_into().map_err(|e: zbus::zvariant::Error| failed(e))?;
        picked_color([r, g, b])
    }
}

#[cfg(windows)]
mod backend {
    use super::{Arc, AtomicBool, MAX_MONITORS, Ordering, ScreenError, ScreenImage, add_to_budget};

    /// `xcap` captures each monitor through GDI. It gives no ICC profile, so samples are taken as
    /// sRGB.
    pub fn capture(stop: &AtomicBool) -> Result<Vec<ScreenImage>, ScreenError> {
        let failed = |e: &dyn std::fmt::Display| ScreenError::Failed(e.to_string());
        let mut monitors = xcap::Monitor::all().map_err(|e| failed(&e))?;
        if monitors.is_empty() || monitors.len() > MAX_MONITORS {
            return Err(ScreenError::Failed("no supported screen is available".into()));
        }
        monitors.sort_by_key(|m| !m.is_primary().unwrap_or(false));
        let mut total = 0usize;
        let mut images = Vec::new();
        for m in monitors {
            if stop.load(Ordering::Relaxed) {
                return Err(ScreenError::Cancelled);
            }
            let scale = m.scale_factor().map_err(|e| failed(&e))?;
            if !scale.is_finite() || scale <= 0.0 || scale > 16.0 {
                return Err(ScreenError::Failed("invalid display scale".into()));
            }
            let (x, y) = (m.x().map_err(|e| failed(&e))?, m.y().map_err(|e| failed(&e))?);
            let name = m.name().unwrap_or_default();
            let captured = m.capture_image().map_err(|e| ScreenError::Failed(format!("screen capture failed: {e}")))?;
            let (width, height) = (captured.width(), captured.height());
            total = add_to_budget(total, width as usize, height as usize)?;
            images.push(ScreenImage { origin: [x, y], size: [width, height], scale, rgba: Arc::new(captured.into_raw()), icc_profile: None, name });
        }
        Ok(images)
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use super::{Arc, AtomicBool, MAX_MONITORS, Ordering, ScreenError, ScreenImage, add_to_budget, checked_stride, pixel_count, unpack_rgba};
    use core_graphics2::{display::get_active_display_list, image::CGImageAlphaInfo as Alpha, window};

    /// Captures each display's composed image at native resolution. Without the Screen Recording
    /// permission the system prompt is shown once; a refusal is `Denied`.
    pub fn capture(stop: &AtomicBool) -> Result<Vec<ScreenImage>, ScreenError> {
        let failed = |m: &str| ScreenError::Failed(m.into());
        if !window::preflight_screen_capture_access() && !window::request_screen_capture_access() {
            return Err(ScreenError::Denied);
        }
        let mut displays = get_active_display_list(MAX_MONITORS + 1).ok_or_else(|| failed("could not enumerate the displays"))?;
        if displays.is_empty() || displays.len() > MAX_MONITORS {
            return Err(failed("no supported screen is available"));
        }
        displays.sort_by_key(|d| !d.is_main());
        let mut total = 0usize;
        for display in &displays {
            let mode = display.copy_display_mode().ok_or_else(|| failed("the display mode is unavailable"))?;
            total = add_to_budget(total, mode.pixel_width(), mode.pixel_height())?;
        }
        let mut images = Vec::new();
        for (n, display) in displays.into_iter().enumerate() {
            if stop.load(Ordering::Relaxed) {
                return Err(ScreenError::Cancelled);
            }
            let bounds = display.bounds();
            let (w, h) = (bounds.size.width, bounds.size.height);
            let (x, y) = (bounds.origin.x, bounds.origin.y);
            if ![w, h, x, y].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
                return Err(failed("invalid display geometry"));
            }
            let image = window::new_image(
                bounds,
                window::CGWindowListOption::OnScreenOnly,
                window::kCGNullWindowID,
                window::CGWindowImageOption::BestResolution | window::CGWindowImageOption::ShouldBeOpaque,
            )
            .ok_or(ScreenError::Denied)?;
            let (width, height) = (image.width(), image.height());
            pixel_count(width, height)?;
            let profile = image.color_space().and_then(|space| space.copy_icc_data());
            if profile.as_ref().is_some_and(|p| p.len() > 1024 * 1024) {
                return Err(failed("the display profile exceeds the memory budget"));
            }
            let info = image.bitmap_info().bits();
            if image.bits_per_component() != 8 || image.bits_per_pixel() != 32 || info & (1 << 8) != 0 {
                return Err(failed("unsupported screen capture pixel format"));
            }
            let (first, premultiplied, opaque) = match image.alpha_info() {
                Alpha::AlphaPremultipliedFirst => (true, true, false),
                Alpha::AlphaPremultipliedLast => (false, true, false),
                Alpha::AlphaFirst => (true, false, false),
                Alpha::AlphaLast => (false, false, false),
                Alpha::AlphaNoneSkipFirst => (true, false, true),
                Alpha::AlphaNoneSkipLast => (false, false, true),
                _ => return Err(failed("unsupported screen capture alpha layout")),
            };
            let little = match info & 0x7000 {
                0 => cfg!(target_endian = "little"),
                0x2000 => true,
                0x4000 => false,
                _ => return Err(failed("unsupported screen capture byte order")),
            };
            let order = match (first, little) {
                (true, true) => [2, 1, 0, 3],
                (true, false) => [1, 2, 3, 0],
                (false, true) => [3, 2, 1, 0],
                (false, false) => [0, 1, 2, 3],
            };
            let stride = image.bytes_per_row();
            checked_stride(width, height, stride)?;
            let bytes = image.data_provider().and_then(|p| p.copy_data()).ok_or_else(|| failed("the screen image has no pixels"))?;
            let rgba = unpack_rgba(bytes.bytes(), width, height, stride, order, premultiplied, opaque)?;
            let scale = width as f32 / w as f32;
            images.push(ScreenImage {
                origin: [(x as f32 * scale).round() as i32, (y as f32 * scale).round() as i32],
                size: [width as u32, height as u32],
                scale,
                rgba: Arc::new(rgba),
                icc_profile: profile.map(|p| Arc::new(p.bytes().to_vec())),
                name: if display.is_main() { "Main Display".into() } else { format!("Display {}", n + 1) },
            });
        }
        Ok(images)
    }
}

#[cfg(not(any(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")), target_os = "macos", windows)))]
mod backend {
    use super::{AtomicBool, ScreenError, ScreenImage};

    pub fn capture(_stop: &AtomicBool) -> Result<Vec<ScreenImage>, ScreenError> {
        Err(ScreenError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A premultiplied BGRA capture with a stride unpacks to straight RGBA in row order.
    #[test]
    fn unpacks_strided_premultiplied_pixels() {
        // Two rows of one BGRA pixel with 4 bytes of row padding; alpha ignored (opaque).
        assert_eq!(
            unpack_rgba(&[10, 20, 30, 0, 99, 99, 99, 99, 40, 50, 60, 0, 99, 99, 99, 99], 1, 2, 8, [2, 1, 0, 3], false, true).unwrap(),
            [30, 20, 10, 255, 60, 50, 40, 255]
        );
        // Premultiplied ARGB at half alpha is divided back out.
        assert_eq!(unpack_rgba(&[128, 64, 32, 16], 1, 1, 4, [1, 2, 3, 0], true, false).unwrap(), [128, 64, 32, 255]);
        assert_eq!(unpack_rgba(&[0, 0, 0, 0], 1, 1, 4, [0, 1, 2, 3], true, false).unwrap(), [0, 0, 0, 255]);
        assert_eq!(unpack_rgba(&[50, 100, 150, 128], 1, 1, 4, [0, 1, 2, 3], true, false).unwrap(), [100, 199, 255, 255]);
        // Truncated buffers and oversized strides fail.
        assert!(unpack_rgba(&[0; 3], 1, 1, 4, [0, 1, 2, 3], false, true).is_err());
        assert!(checked_stride(1, 1, usize::MAX).is_err());
    }
}
