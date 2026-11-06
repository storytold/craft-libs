//! The JNI interface `craftkit` calls: the `external fun`s of `dev.craft.kit.CraftNative`.
//!
//! Kotlin calls these on the main thread, and none of them waits on a frame: each posts a
//! message to the render thread, which owns the app (in its thread-local session), draws at the
//! display's pace, and calls back with requests through the handler's
//! `onRequest(kind: Int, text: String, data: ByteArray?)` (kinds as [`Request::wire`] lists
//! them, the same as on iOS). Two calls wait: `detach`, because Android requires that a surface
//! is no longer used once `surfaceDestroyed` returns, and `suspend`, because Android may freeze or
//! end the process once `onStop` returns, and the app's recovery copies must be written by then.
//!
//! `start` itself is generated per app by [`crate::export!`]; it calls [`start`].

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Once, OnceLock};
use std::time::Duration;

use eframe::wgpu;
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jfloat, jint, jlong};
use jni::{JNIEnv, JavaVM};
use ndk::native_window::NativeWindow;
use wgpu::rwh::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

use crate::driver::{self, Driver, Message};
use crate::host::{Host, HostError, HostOps, ViewSize};
use crate::platform::Request;
use crate::{Action, CraftApp, Input, Location, PickedFile, Stylus, keycodes};

#[doc(hidden)]
pub use jni;

/// How often an idle render thread looks for frames egui scheduled itself (animations, worker
/// threads finishing); input and native controls wake it at once.
const IDLE_POLL: Duration = Duration::from_millis(16);

/// How often a render thread without a window looks for frames while the app is busy (finishing
/// an export in the background): it doesn't draw, so it needn't keep the display's pace.
const BUSY_POLL: Duration = Duration::from_millis(50);

/// How long `surfaceDestroyed` waits for the render thread to let go of the window (it may be
/// in the middle of a slow frame). Well under Android's 5 s ANR limit.
const DETACH_WAIT: Duration = Duration::from_secs(3);

/// How long `onStop` waits for the app's suspend (recovery copies of unsaved work). The app is
/// leaving the screen, so no input waits on the main thread meanwhile; until `onStop` returns,
/// Android doesn't count the process as cached, so it isn't frozen or ended first.
const SUSPEND_WAIT: Duration = Duration::from_secs(4);

/// The window of the shell's `SurfaceView`, held (its reference count raised) until dropped.
struct AndroidWindow(NativeWindow);

impl driver::Window for AndroidWindow {
    fn target(&self) -> wgpu::SurfaceTargetUnsafe {
        wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(self.0.ptr().cast())),
        }
    }
}

/// The Kotlin object that hears requests, called from the render thread.
struct KotlinShell {
    vm: JavaVM,
    handler: GlobalRef,
}

impl driver::Shell for KotlinShell {
    fn deliver(&mut self, request: &Request) {
        if let Err(e) = self.call(request) {
            log::error!("craft-mobile: delivering a request to Kotlin: {e}");
        }
    }
}

impl Drop for KotlinShell {
    fn drop(&mut self) {
        // The handler's global reference is released on this thread, which must be attached.
        if let Err(e) = self.vm.attach_current_thread_permanently() {
            log::error!("craft-mobile: attaching the render thread: {e}");
        }
    }
}

impl KotlinShell {
    fn call(&self, request: &Request) -> jni::errors::Result<()> {
        // The render thread stays attached: it calls back after most frames.
        let mut env = self.vm.attach_current_thread_permanently()?;
        let (kind, text, data) = driver::wire(request);
        // A local frame, because an attached native thread never returns to Java to free them.
        let called = env.with_local_frame(4, |env| -> jni::errors::Result<()> {
            let text = env.new_string(text)?;
            let data = match data {
                Some(bytes) => JObject::from(env.byte_array_from_slice(bytes)?),
                None => JObject::null(),
            };
            let args = [JValue::Int(kind), JValue::Object(&text), JValue::Object(&data)];
            env.call_method(&self.handler, "onRequest", "(ILjava/lang/String;[B)V", &args)?;
            Ok(())
        });
        // A Kotlin exception must not stay pending on this thread: log it and carry on.
        if env.exception_check()? {
            env.exception_describe()?;
            env.exception_clear()?;
        }
        called
    }
}

type AndroidMessage = Message<AndroidWindow, KotlinShell>;

/// The render thread's inbox, starting the thread on first use. `None` if it couldn't start.
fn render_thread() -> Option<&'static Sender<AndroidMessage>> {
    static RENDER: OnceLock<Option<Sender<AndroidMessage>>> = OnceLock::new();
    RENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            match std::thread::Builder::new().name("craft-render".into()).spawn(move || run(&rx)) {
                Ok(_) => Some(tx),
                Err(e) => {
                    log::error!("craft-mobile: couldn't start the render thread: {e}");
                    None
                }
            }
        })
        .as_ref()
}

/// The render thread: apply what the shell asked, draw if a frame is due, then wait. Drawing
/// paces itself (presenting waits for the display); idle, it sleeps until a message arrives or
/// the next poll. Without a window it polls only while the app is busy.
fn run(rx: &Receiver<AndroidMessage>) {
    let mut driver = Driver::default();
    let mut drew = false;
    let mut last_drawn = std::time::Instant::now();
    loop {
        // Hot or saving power: draw at most at the rate the power state allows.
        if drew {
            if let Some(wait) = driver.frame_interval().and_then(|interval| interval.checked_sub(last_drawn.elapsed())) {
                std::thread::sleep(wait);
            }
            last_drawn = std::time::Instant::now();
        }
        let poll = if driver.drawing() { Some(IDLE_POLL) } else { driver.busy().then_some(BUSY_POLL) };
        let first = if drew {
            rx.try_recv().ok()
        } else if let Some(poll) = poll {
            match rx.recv_timeout(poll) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            }
        };
        for message in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok())) {
            driver.handle(message);
        }
        drew = driver.frame();
    }
}

fn post(message: AndroidMessage) -> bool {
    let Some(render) = render_thread() else {
        return false;
    };
    let sent = render.send(message).is_ok();
    if !sent {
        log::error!("craft-mobile: the render thread has stopped");
    }
    sent
}

fn init_logging() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("craft"));
        // A panic on the render thread would otherwise vanish from logcat.
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("{info}");
            default(info);
        }));
    });
}

fn view_size(width_px: jint, height_px: jint, scale: jfloat) -> Option<ViewSize> {
    ViewSize::new(u32::try_from(width_px).ok()?, u32::try_from(height_px).ok()?, scale)
}

const fn jboolean(value: bool) -> jboolean {
    if value { JNI_TRUE } else { JNI_FALSE }
}

/// `CraftNative.start`: start `A` drawing into `surface` (an `android.view.Surface`), or move
/// the running app there. Returns whether the render thread took the request; it reports
/// failures to set up the GPU in the log.
#[expect(unsafe_code, reason = "takes the Surface's native window")]
#[must_use]
pub fn start<A: CraftApp>(env: &JNIEnv<'_>, surface: &JObject<'_>, width_px: jint, height_px: jint, scale: jfloat) -> jboolean {
    init_logging();
    let Some(size) = view_size(width_px, height_px, scale) else {
        log::error!("craft-mobile: start needs a size and a positive, finite scale");
        return JNI_FALSE;
    };
    if surface.is_null() {
        log::error!("craft-mobile: start needs a Surface");
        return JNI_FALSE;
    }
    // SAFETY: `env` is this thread's JNIEnv for the duration of the call, and `surface` is a
    // non-null local reference the caller passed, which Kotlin types as `android.view.Surface`.
    // `ANativeWindow_fromSurface` takes its own reference, which `NativeWindow` releases on drop.
    let Some(window) = (unsafe { NativeWindow::from_surface(env.get_raw(), surface.as_raw()) }) else {
        log::error!("craft-mobile: the Surface has no native window");
        return JNI_FALSE;
    };
    jboolean(post(Message::Attach { window: AndroidWindow(window), size, make: make::<A> }))
}

/// Builds `A` on the render thread (the host is not `Send`).
///
/// # Safety
/// `target` must stay valid until the host is dropped or detached.
#[expect(unsafe_code, reason = "a driver::MakeHost")]
unsafe fn make<A: CraftApp>(target: wgpu::SurfaceTargetUnsafe, size: ViewSize) -> Result<Box<dyn HostOps>, HostError> {
    // SAFETY: the caller's contract is `Host::new`'s.
    let host = unsafe { Host::<A>::new(target, size, &A::wgpu_configuration()) }?;
    Ok(Box::new(host))
}

/// The entry point android-activity's `NativeActivity` glue requires (eframe links it through
/// winit). Never called: the app runs in craftkit's Compose activity, never a `NativeActivity`.
#[expect(unsafe_code, reason = "the symbol android-activity's glue links against")]
#[expect(clippy::no_mangle_with_rust_abi, reason = "android-activity declares it with the Rust ABI")]
#[unsafe(no_mangle)]
fn android_main(_app: winit::platform::android::activity::AndroidApp) {
    init_logging();
    log::error!("craft-mobile: started as a NativeActivity; host the app with craftkit's CraftCanvas instead");
}

/// `CraftNative.setHandler(handler)`: where requests go; `null` to stop hearing them.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_setHandler(env: JNIEnv<'_>, _class: JClass<'_>, handler: JObject<'_>) {
    init_logging();
    if handler.is_null() {
        post(Message::Shell(None));
        return;
    }
    let shell = env.get_java_vm().and_then(|vm| Ok(KotlinShell { vm, handler: env.new_global_ref(&handler)? }));
    match shell {
        Ok(shell) => {
            post(Message::Shell(Some(shell)));
        }
        Err(e) => log::error!("craft-mobile: setHandler: {e}"),
    }
}

/// `CraftNative.detach()`: the surface is going away. Returns once the render thread no longer
/// uses it (or after [`DETACH_WAIT`]); the app keeps running until the next `start`.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_detach(_env: JNIEnv<'_>, _class: JClass<'_>) {
    let (done, detached) = mpsc::sync_channel(1);
    if post(Message::Detach { done }) && detached.recv_timeout(DETACH_WAIT).is_err() {
        log::warn!("craft-mobile: the render thread didn't let go of the surface in {DETACH_WAIT:?}");
    }
}

/// `CraftNative.suspend()` (from `onStop`): the app left the screen and Android may end it
/// without warning. Returns once the app has written its recovery data (or after [`SUSPEND_WAIT`]).
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_suspend(_env: JNIEnv<'_>, _class: JClass<'_>) {
    let (done, finished) = mpsc::sync_channel(1);
    if post(Message::Input(Input::Suspend)) && post(Message::Sync { done }) && finished.recv_timeout(SUSPEND_WAIT).is_err() {
        log::warn!("craft-mobile: the app's suspend didn't finish in {SUSPEND_WAIT:?}");
    }
}

/// `CraftNative.resume()` (from `onStart`): the app is back on screen.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_resume(_env: JNIEnv<'_>, _class: JClass<'_>) {
    post(Message::Input(Input::Resume));
}

/// `CraftNative.memoryWarning()` (from `onTrimMemory`, `onLowMemory`): let go of caches.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_memoryWarning(_env: JNIEnv<'_>, _class: JClass<'_>) {
    post(Message::Input(Input::MemoryWarning));
}

/// `CraftNative.power(thermalStatus, powerSave)`: `PowerManager`'s thermal status (`-1` where the
/// API level has none) and whether Battery Saver is on. Call at start and whenever either changes.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_power(_env: JNIEnv<'_>, _class: JClass<'_>, thermal_status: jint, power_save: jboolean) {
    init_logging();
    let power = crate::Power { thermal: crate::Thermal::from_android(thermal_status), low_power: power_save != JNI_FALSE };
    post(Message::Input(Input::Power(power)));
}

/// `CraftNative.setDataDir(path)`: the app's own folder (`Context.filesDir`). Call before `start`:
/// the app reads it as it starts.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_setDataDir(mut env: JNIEnv<'_>, _class: JClass<'_>, path: JString<'_>) {
    init_logging();
    if let Some(path) = string(&mut env, &path) {
        crate::platform::set_data_dir(path.into());
    }
}

/// `CraftNative.key(keyCode, pressed, metaState)`: a hardware key, in Android's `KeyEvent` terms.
/// Returns whether the app takes the key; Android keeps the rest (Back, volume).
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_key(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    code: jint,
    pressed: jboolean,
    meta: jint,
) -> jboolean {
    let Some(key) = keycodes::key(code) else {
        return JNI_FALSE;
    };
    let input = Input::Key { key: key.name().to_owned(), pressed: pressed != JNI_FALSE, modifiers: keycodes::modifiers(meta) };
    jboolean(post(Message::Input(input)))
}

/// `CraftNative.text(text)`: text typed (the on-screen keyboard, or a hardware key's character).
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_text(mut env: JNIEnv<'_>, _class: JClass<'_>, text: JString<'_>) {
    if let Some(text) = string(&mut env, &text).filter(|t| !t.is_empty()) {
        post(Message::Input(Input::Text(text)));
    }
}

/// `CraftNative.pasteText(text)`: paste, after the user asked to.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_pasteText(mut env: JNIEnv<'_>, _class: JClass<'_>, text: JString<'_>) {
    if let Some(text) = string(&mut env, &text) {
        post(Message::Input(Input::PasteText(text)));
    }
}

/// `CraftNative.pasteOwn()`: paste; the clipboard still holds the image the app copied last.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_pasteOwn(_env: JNIEnv<'_>, _class: JClass<'_>) {
    post(Message::Input(Input::PasteOwnImage));
}

/// `CraftNative.pasteImage(rgba, width, height)`: paste an image (RGBA8, unpremultiplied).
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_pasteImage(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    rgba: JByteArray<'_>,
    width: jint,
    height: jint,
) {
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return;
    };
    match env.convert_byte_array(&rgba) {
        Ok(rgba) => {
            post(Message::Input(Input::PasteImage { width, height, rgba }));
        }
        Err(e) => log::error!("craft-mobile: pasteImage: {e}"),
    }
}

/// `CraftNative.hover(x, y, inside)`: a mouse or trackpad pointer (or a hovering stylus) moved
/// without a button down, in dp; `inside` false when it left the view.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_hover(_env: JNIEnv<'_>, _class: JClass<'_>, x: jfloat, y: jfloat, inside: jboolean) {
    match (inside != JNI_FALSE, point(x, y)) {
        (true, Some(pos)) => {
            post(Message::Input(Input::Hover(pos)));
        }
        (false, _) => {
            post(Message::Input(Input::HoverGone));
        }
        (true, None) => {}
    }
}

/// `CraftNative.scroll(x, y, dx, dy)`: a wheel or trackpad scroll at (x, y), in dp; positive
/// moves content right and down.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_scroll(_env: JNIEnv<'_>, _class: JClass<'_>, x: jfloat, y: jfloat, dx: jfloat, dy: jfloat) {
    if let (Some(pos), true) = (point(x, y), dx.is_finite() && dy.is_finite()) {
        post(Message::Input(Input::Scroll { pos, delta: egui::vec2(dx, dy) }));
    }
}

/// `CraftNative.secondaryClick(x, y)`: a right click (or the trackpad's), in dp.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_secondaryClick(_env: JNIEnv<'_>, _class: JClass<'_>, x: jfloat, y: jfloat) {
    if let Some(pos) = point(x, y) {
        post(Message::Input(Input::SecondaryClick(pos)));
    }
}

/// `CraftNative.stylus(id, phase, x, y, pressure, altitude, azimuth, eraser)`: a stylus sample,
/// sent instead of a touch. Phases and units as `touch`; angles as [`Stylus`] has them.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_stylus(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    id: jlong,
    phase: jint,
    x: jfloat,
    y: jfloat,
    pressure: jfloat,
    altitude: jfloat,
    azimuth: jfloat,
    eraser: jboolean,
) {
    let Ok(phase) = u32::try_from(phase) else {
        return;
    };
    let Some(touch) = driver::touch(id.cast_unsigned(), phase, x, y, pressure) else {
        return;
    };
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    post(Message::Input(Input::Stylus(Stylus {
        id: touch.id,
        phase: touch.phase,
        pos: touch.pos,
        pressure: touch.force,
        altitude: finite(altitude),
        azimuth: finite(azimuth),
        eraser: eraser != JNI_FALSE,
    })));
}

fn point(x: f32, y: f32) -> Option<egui::Pos2> {
    (x.is_finite() && y.is_finite()).then(|| egui::pos2(x, y))
}

/// A Kotlin string, copied out of the JVM.
fn string(env: &mut JNIEnv<'_>, text: &JString<'_>) -> Option<String> {
    env.get_string(text).map(String::from).map_err(|e| log::error!("craft-mobile: reading a string from Kotlin: {e}")).ok()
}

/// `CraftNative.stop()`: drop the app.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_stop(_env: JNIEnv<'_>, _class: JClass<'_>) {
    post(Message::Stop);
}

/// `CraftNative.resize(widthPx, heightPx, scale)`: the surface changed size or density.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_resize(_env: JNIEnv<'_>, _class: JClass<'_>, width_px: jint, height_px: jint, scale: jfloat) {
    if let Some(size) = view_size(width_px, height_px, scale) {
        post(Message::Resize(size));
    }
}

/// `CraftNative.touch(id, phase, x, y, force)`: a touch in points (dp) from the view's top-left.
/// `phase`: 0 began, 1 moved, 2 ended, 3 cancelled. `force` is 0..=1, or 0 where the device
/// has none.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_touch(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    id: jlong,
    phase: jint,
    x: jfloat,
    y: jfloat,
    force: jfloat,
) {
    let Ok(phase) = u32::try_from(phase) else {
        return;
    };
    if let Some(touch) = driver::touch(id.cast_unsigned(), phase, x, y, force) {
        post(Message::Touch(touch));
    }
}

/// A file from Kotlin, copied out of the JVM. `location` (a content URI the app may write to)
/// may be null.
fn picked_file(env: &mut JNIEnv<'_>, name: &JString<'_>, data: &JByteArray<'_>, location: &JString<'_>) -> Option<PickedFile> {
    let file = env.get_string(name).map(String::from).and_then(|name| {
        let bytes = env.convert_byte_array(data)?;
        let location = optional_string(env, location)?.and_then(Location::new);
        Ok(PickedFile { name, bytes, location })
    });
    file.map_err(|e| log::error!("craft-mobile: reading a file from Kotlin: {e}")).ok()
}

/// A Kotlin `String?` as a Rust one.
fn optional_string(env: &mut JNIEnv<'_>, text: &JString<'_>) -> jni::errors::Result<Option<String>> {
    if text.is_null() { Ok(None) } else { env.get_string(text).map(|s| Some(s.into())) }
}

/// `CraftNative.openFile(name, data, location)`: a file the system handed over ("Open with",
/// Share); `location` is its content URI when the app may write it back, else null.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_openFile(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    name: JString<'_>,
    data: JByteArray<'_>,
    location: JString<'_>,
) {
    init_logging();
    if let Some(file) = picked_file(&mut env, &name, &data, &location) {
        post(Message::OpenFile(file));
    }
}

/// `CraftNative.pickAdd(name, data, location)`: one file chosen in the open picker; finish with
/// `pickDone`.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_pickAdd(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    name: JString<'_>,
    data: JByteArray<'_>,
    location: JString<'_>,
) {
    if let Some(file) = picked_file(&mut env, &name, &data, &location) {
        post(Message::PickAdd(file));
    }
}

/// `CraftNative.writeDone(location, error)`: a write in place (request kind 8) ended; `error`
/// is null when the file was written.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_writeDone(mut env: JNIEnv<'_>, _class: JClass<'_>, location: JString<'_>, error: JString<'_>) {
    let answer = optional_string(&mut env, &location).and_then(|location| Ok((location, optional_string(&mut env, &error)?)));
    match answer {
        Ok((Some(location), error)) => {
            if let Some(location) = Location::new(location) {
                post(Message::Input(Input::WriteDone { location, result: error.map_or(Ok(()), Err) }));
            }
        }
        Ok((None, _)) => log::warn!("craft-mobile: writeDone without a location"),
        Err(e) => log::error!("craft-mobile: writeDone: {e}"),
    }
}

/// `CraftNative.pickDone()`: answer the open picker with the files added since it opened; none
/// means cancelled.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_pickDone(_env: JNIEnv<'_>, _class: JClass<'_>) {
    post(Message::PickDone);
}

/// `CraftNative.action(json)`: a native control was used; an [`Action`] as JSON. Returns false
/// when it isn't one.
#[expect(unsafe_code, reason = "exported to Kotlin")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_craft_kit_CraftNative_action(mut env: JNIEnv<'_>, _class: JClass<'_>, json: JString<'_>) -> jboolean {
    let json: String = match env.get_string(&json) {
        Ok(json) => json.into(),
        Err(e) => {
            log::error!("craft-mobile: action: {e}");
            return JNI_FALSE;
        }
    };
    match serde_json::from_str::<Action>(&json) {
        Ok(action) => jboolean(post(Message::Action(action))),
        Err(e) => {
            log::warn!("craft-mobile: not an action: {e}");
            JNI_FALSE
        }
    }
}
