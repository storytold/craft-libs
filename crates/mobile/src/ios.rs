//! The C interface CraftKit calls (declared in `swift/CraftKit/include/CraftMobile.h`).
//!
//! Everything is called on the main thread. `craft_start` itself is generated per app by
//! [`crate::export!`]; it calls [`start`].

use std::cell::RefCell;
use std::ffi::c_void;

use eframe::wgpu;
use egui::{Modifiers, Pos2};

use crate::host::{Host, ViewSize};
use crate::platform::Request;
use crate::{Action, CraftApp, Input, PickedFile, Stylus, session};

/// What the shell is asked to do. `text` and `data` are only valid during the call: copy them.
///
/// | kind | request        | text                         | data           |
/// |------|----------------|------------------------------|----------------|
/// | 0    | pick one file  | allowed extensions, `,`-joined | —            |
/// | 1    | pick files     | allowed extensions, `,`-joined | —            |
/// | 2    | export         | suggested file name          | the file       |
/// | 3    | panel state    | a `Snapshot` as JSON         | —              |
///
/// and the rest as [`Request::wire`] lists them (8: write in place, answered with
/// [`craft_write_done`]; 9: busy, `1` or `0`).
pub type Handler = extern "C" fn(context: *mut c_void, kind: u32, text: *const u8, text_len: usize, data: *const u8, data_len: usize);

struct Shell {
    context: *mut c_void,
    handler: Handler,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    /// Files picked so far for the open picker, until `craft_pick_done`.
    static PICKED: RefCell<Vec<PickedFile>> = const { RefCell::new(Vec::new()) };
}

/// Start `A` drawing into `layer`.
///
/// # Safety
/// `layer` must be a valid `CAMetalLayer` that stays alive until [`craft_stop`].
#[expect(unsafe_code, reason = "hands the view's layer to wgpu")]
pub unsafe fn start<A: CraftApp>(layer: *mut c_void, width_px: u32, height_px: u32, scale: f32) -> bool {
    init_logging();
    let Some(size) = ViewSize::new(width_px, height_px, scale) else {
        log::error!("craft-mobile: craft_start needs a positive, finite scale");
        return false;
    };
    if layer.is_null() {
        log::error!("craft-mobile: craft_start needs a layer");
        return false;
    }
    // SAFETY: the caller keeps the layer alive until craft_stop, which drops the host first.
    match unsafe { Host::<A>::new(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer), size, &A::wgpu_configuration()) } {
        Ok(host) => {
            session::install(Box::new(host));
            true
        }
        Err(e) => {
            log::error!("craft-mobile: {e}");
            false
        }
    }
}

/// Where requests go. `context` comes back unchanged with every call; it is never dereferenced.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_set_handler(context: *mut c_void, handler: Handler) {
    SHELL.with(|s| *s.borrow_mut() = Some(Shell { context, handler }));
}

/// Drop the app and its surface. Call before the layer goes away.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_stop() {
    session::stop();
}

/// The view changed size or moved to a screen with another scale.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_resize(width_px: u32, height_px: u32, scale: f32) {
    if let Some(size) = ViewSize::new(width_px, height_px, scale) {
        session::with(|h| h.resize(size));
    }
}

/// Call on every display refresh: draws only when something changed, then delivers requests.
/// Returns whether it drew.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_frame() -> bool {
    let (drew, requests) = session::frame();
    for request in requests {
        deliver(&request);
    }
    drew
}

/// A touch, in points from the view's top-left. `phase`: 0 began, 1 moved, 2 ended,
/// 3 cancelled; anything else is ignored. `force` is 0..=1, or 0 where the device has none.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_touch(id: u64, phase: u32, x: f32, y: f32, force: f32) {
    if let Some(touch) = crate::host::decode_touch(id, phase, x, y, force) {
        session::with(|h| h.touch(touch));
    }
}

/// A file the system handed over (Files, Share, drag and drop). `location` is where the shell
/// can write it back (its file URL), or empty for a copy.
///
/// # Safety
/// `name`, `data` and `location` must point at `name_len`, `data_len` and `location_len`
/// readable bytes (or be null with length 0) for the duration of the call.
#[expect(unsafe_code, reason = "reads the shell's buffers")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_open_file(
    name: *const u8,
    name_len: usize,
    data: *const u8,
    data_len: usize,
    location: *const u8,
    location_len: usize,
) {
    // SAFETY: the caller's contract, passed on.
    let file = unsafe { picked(name, name_len, data, data_len, location, location_len) };
    session::with(|h| h.open_file(file));
}

/// Add one chosen file to the answer for the open picker; finish with [`craft_pick_done`].
///
/// # Safety
/// As [`craft_open_file`].
#[expect(unsafe_code, reason = "reads the shell's buffers")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_pick_add(
    name: *const u8,
    name_len: usize,
    data: *const u8,
    data_len: usize,
    location: *const u8,
    location_len: usize,
) {
    // SAFETY: the caller's contract, passed on.
    let file = unsafe { picked(name, name_len, data, data_len, location, location_len) };
    PICKED.with(|p| p.borrow_mut().push(file));
}

/// The shell finished a write in place (request kind 8) to `location`: `error` is null (or
/// empty) when it wrote the file, else why it couldn't.
///
/// # Safety
/// `location` and `error` must point at `location_len` and `error_len` readable bytes (or be
/// null with length 0) during the call.
#[expect(unsafe_code, reason = "reads the shell's buffers")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_write_done(location: *const u8, location_len: usize, error: *const u8, error_len: usize) {
    // SAFETY: the caller's contract, passed on.
    let (location, error) = unsafe { (bytes(location, location_len), bytes(error, error_len)) };
    let Some(location) = crate::Location::new(String::from_utf8_lossy(location)) else { return };
    let result = if error.is_empty() { Ok(()) } else { Err(String::from_utf8_lossy(error).into_owned()) };
    input(Input::WriteDone { location, result });
}

/// Answer the open picker with the files added since it was shown; none means cancelled.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_pick_done() {
    let files = PICKED.with(|p| std::mem::take(&mut *p.borrow_mut()));
    session::with(|h| h.answer_pick(files));
}

/// A native control was used: an [`Action`] as JSON. Returns false when it isn't one.
///
/// # Safety
/// `json` must point at `len` readable bytes (or be null with length 0) during the call.
#[expect(unsafe_code, reason = "reads the shell's buffer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_action(json: *const u8, len: usize) -> bool {
    // SAFETY: the caller's contract, passed on.
    let bytes = unsafe { bytes(json, len) };
    match serde_json::from_slice::<Action>(bytes) {
        Ok(action) => session::with(|h| h.action(action)).is_some(),
        Err(e) => {
            log::warn!("craft-mobile: not an action: {e}");
            false
        }
    }
}

/// # Safety
/// `ptr` must point at `len` readable bytes for `'a`, or be null (then `len` is ignored).
#[expect(unsafe_code, reason = "the one place a shell's buffer becomes a slice")]
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() {
        return &[];
    }
    // SAFETY: non-null, and the caller guarantees `len` readable bytes that outlive 'a; u8 has
    // no alignment or validity requirements.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// # Safety
/// As [`bytes`], for all three buffers.
#[expect(unsafe_code, reason = "reads the shell's buffers")]
unsafe fn picked(name: *const u8, name_len: usize, data: *const u8, data_len: usize, location: *const u8, location_len: usize) -> PickedFile {
    // SAFETY: the caller's contract, passed on; all are copied before returning.
    let (name, data, location) = unsafe { (bytes(name, name_len), bytes(data, data_len), bytes(location, location_len)) };
    PickedFile {
        name: String::from_utf8_lossy(name).into_owned(),
        bytes: data.to_vec(),
        location: crate::Location::new(String::from_utf8_lossy(location)),
    }
}

fn deliver(request: &Request) {
    let (kind, text, data) = request.wire();
    let data = data.unwrap_or_default();
    let Ok(kind) = u32::try_from(kind) else { return };
    SHELL.with(|s| {
        if let Some(shell) = s.borrow().as_ref() {
            (shell.handler)(shell.context, kind, text.as_ptr(), text.len(), data.as_ptr(), data.len());
        }
    });
}

/// The `log` crate's output to the unified log (subsystem: the app's bundle id), once.
fn init_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let subsystem = std::env::var("CRAFT_LOG_SUBSYSTEM").unwrap_or_else(|_| "dev.craft".into());
        if let Err(e) = oslog::OsLogger::new(&subsystem).level_filter(log::LevelFilter::Info).init() {
            eprintln!("craft-mobile: no unified logging: {e}");
        }
    });
}

fn input(input: Input) {
    session::with(|h| h.input(input));
}

/// Modifier keys from the shell's bitmask: 1 shift, 2 control, 4 option, 8 command.
fn modifiers(bits: u32) -> Modifiers {
    let command = bits & 8 != 0;
    Modifiers { shift: bits & 1 != 0, ctrl: bits & 2 != 0, alt: bits & 4 != 0, mac_cmd: command, command }
}

fn point(x: f32, y: f32) -> Option<Pos2> {
    (x.is_finite() && y.is_finite()).then(|| Pos2::new(x, y))
}

/// Where the app keeps its own data (Application Support). Call before `craft_start`.
///
/// # Safety
/// `path` must point at `len` readable bytes of UTF-8 (or be null) during the call.
#[expect(unsafe_code, reason = "reads the shell's buffer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_set_data_dir(path: *const u8, len: usize) {
    // SAFETY: the caller's contract, passed on.
    let path = unsafe { bytes(path, len) };
    if let Ok(path) = std::str::from_utf8(path) {
        crate::platform::set_data_dir(path.into());
    }
}

/// A hardware key went down (`pressed`) or up. `name` is egui's key name (`A`, `Enter`,
/// `ArrowLeft`); `modifiers` a bitmask: 1 shift, 2 control, 4 option, 8 command.
///
/// # Safety
/// `name` must point at `len` readable bytes (or be null) during the call.
#[expect(unsafe_code, reason = "reads the shell's buffer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_key(name: *const u8, len: usize, pressed: bool, modifier_bits: u32) {
    // SAFETY: the caller's contract, passed on.
    let name = unsafe { bytes(name, len) };
    input(Input::Key { key: String::from_utf8_lossy(name).into_owned(), pressed, modifiers: modifiers(modifier_bits) });
}

/// Text typed: the on-screen keyboard, dictation, or a hardware key that makes a character.
///
/// # Safety
/// `text` must point at `len` readable bytes (or be null) during the call.
#[expect(unsafe_code, reason = "reads the shell's buffer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_text(text: *const u8, len: usize) {
    // SAFETY: the caller's contract, passed on.
    let text = unsafe { bytes(text, len) };
    input(Input::Text(String::from_utf8_lossy(text).into_owned()));
}

/// Paste text the shell read from the clipboard because the user asked to paste.
///
/// # Safety
/// As [`craft_text`].
#[expect(unsafe_code, reason = "reads the shell's buffer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_paste_text(text: *const u8, len: usize) {
    // SAFETY: the caller's contract, passed on.
    let text = unsafe { bytes(text, len) };
    input(Input::PasteText(String::from_utf8_lossy(text).into_owned()));
}

/// Paste an image (RGBA8, unpremultiplied, `width × height`) read from the clipboard.
///
/// # Safety
/// `rgba` must point at `len` readable bytes (or be null) during the call.
#[expect(unsafe_code, reason = "reads the shell's buffer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn craft_paste_image(rgba: *const u8, len: usize, width: u32, height: u32) {
    // SAFETY: the caller's contract, passed on.
    let rgba = unsafe { bytes(rgba, len) };
    input(Input::PasteImage { width, height, rgba: rgba.to_vec() });
}

/// Paste: the clipboard still holds the image the app copied last, so the shell didn't read it.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_paste_own() {
    input(Input::PasteOwnImage);
}

/// A trackpad or mouse pointer hovering at (x, y) points; `inside` false when it left the view.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_hover(x: f32, y: f32, inside: bool) {
    match (inside, point(x, y)) {
        (true, Some(pos)) => input(Input::Hover(pos)),
        (false, _) => input(Input::HoverGone),
        (true, None) => {}
    }
}

/// Scrolling at (x, y) by (dx, dy) points; positive moves content right and down.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_scroll(x: f32, y: f32, dx: f32, dy: f32) {
    if let (Some(pos), true) = (point(x, y), dx.is_finite() && dy.is_finite()) {
        input(Input::Scroll { pos, delta: egui::vec2(dx, dy) });
    }
}

/// A secondary click (right click, two-finger trackpad click) at (x, y) points.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_secondary_click(x: f32, y: f32) {
    if let Some(pos) = point(x, y) {
        input(Input::SecondaryClick(pos));
    }
}

/// A stylus sample, sent instead of `craft_touch` for stylus contact. `phase` as for touches;
/// `pressure` 0...1; `altitude` radians from the screen (π/2 upright); `azimuth` radians
/// clockwise from +x; `eraser` when the eraser end or mode is in use.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_stylus(id: u64, phase: u32, x: f32, y: f32, pressure: f32, altitude: f32, azimuth: f32, eraser: bool) {
    let Some(touch) = crate::host::decode_touch(id, phase, x, y, pressure) else { return };
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    input(Input::Stylus(Stylus {
        id,
        phase: touch.phase,
        pos: touch.pos,
        pressure: touch.force,
        altitude: finite(altitude),
        azimuth: finite(azimuth),
        eraser,
    }));
}

/// The app is leaving the screen and may be terminated: the app writes recovery data now. Call
/// while the scene is still active or inactive, inside a background task.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_suspend() {
    input(Input::Suspend);
}

/// The app is back on screen.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_resume() {
    input(Input::Resume);
}

/// The device's power state: `thermal` is `ProcessInfo.ThermalState.rawValue` (0 nominal,
/// 1 fair, 2 serious, 3 critical), `low_power` whether Low Power Mode is on. Call after
/// `craft_start` and whenever either changes.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_power(thermal: u32, low_power: bool) {
    input(Input::Power(crate::Power { thermal: crate::Thermal::from_apple(thermal), low_power }));
}

/// The OS is short of memory: the app drops what it can rebuild.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn craft_memory_warning() {
    input(Input::MemoryWarning);
}
