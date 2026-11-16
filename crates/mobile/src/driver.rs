//! The render thread's side of a shell whose UI thread must never wait on a frame (Android).
//!
//! The shell's calls become [`Message`]s; the thread that owns the host applies them in order
//! with [`Driver::handle`] and draws with [`Driver::frame`]. Nothing here knows about JNI, so the
//! rules (files that arrive before the app is up, a view that comes and goes while the app keeps
//! running, the open picker's answer) are tested without a device.

use std::borrow::Cow;
use std::sync::mpsc::SyncSender;

use eframe::wgpu;

use crate::host::{HostError, HostOps, Touch, ViewSize};
use crate::platform::Request;
use crate::{Action, Input, PickedFile, Power, session};

/// A native window the host can draw into, kept alive by whoever holds this value.
pub(crate) trait Window: Send + 'static {
    /// The window as wgpu takes it. Valid for as long as `self` is.
    fn target(&self) -> wgpu::SurfaceTargetUnsafe;
}

/// Where the requests from each frame go.
pub(crate) trait Shell: Send + 'static {
    fn deliver(&mut self, request: &Request);
}

/// Builds the app drawing into `target`.
///
/// # Safety
/// `target` must stay valid until the host is dropped or detached.
pub(crate) type MakeHost = unsafe fn(target: wgpu::SurfaceTargetUnsafe, size: ViewSize) -> Result<Box<dyn HostOps>, HostError>;

/// What the shell asked for, in the order it asked.
pub(crate) enum Message<W, S> {
    /// Draw into `window`: start the app with `make` if it isn't running, else move it there.
    Attach {
        window: W,
        size: ViewSize,
        make: MakeHost,
    },
    /// The view changed size or density.
    Resize(ViewSize),
    /// The window is going away: stop using it, then answer on `done`. The app keeps running.
    Detach {
        done: SyncSender<()>,
    },
    /// Drop the app.
    Stop,
    /// Where requests go from now on; `None` keeps them.
    Shell(Option<S>),
    Touch(Touch),
    /// A file the system handed over.
    OpenFile(PickedFile),
    /// One file chosen in the open picker.
    PickAdd(PickedFile),
    /// The open picker closed: answer it with the files added since it opened.
    PickDone,
    Action(Action),
    /// Keys, text, clipboard, trackpad, stylus and life-cycle signals.
    Input(Input),
    /// Answer on `done` once everything posted before has been handled (the shell waits for the
    /// app's suspend this way).
    Sync {
        done: SyncSender<()>,
    },
}

/// The state of the render thread, apart from the host (which lives in [`session`]).
pub(crate) struct Driver<W, S> {
    /// The window the host draws into; it outlives the host's surface.
    window: Option<W>,
    shell: Option<S>,
    /// Files picked so far for the open picker.
    picked: Vec<PickedFile>,
    /// Files handed over before the app was up; it opens them when it starts.
    early: Vec<PickedFile>,
    /// The last panel state, for a shell that starts listening later.
    state: Option<String>,
    /// Whether the app last said it was busy: then it runs without a window too.
    busy: bool,
    /// The device's last power state, for an app that starts after the shell reported it.
    power: Option<Power>,
}

impl<W, S> Default for Driver<W, S> {
    fn default() -> Self {
        Self { window: None, shell: None, picked: Vec::new(), early: Vec::new(), state: None, busy: false, power: None }
    }
}

impl<W: Window, S: Shell> Driver<W, S> {
    /// Whether frames are wanted: an app is running and has a window.
    pub(crate) fn drawing(&self) -> bool {
        self.window.is_some() && running()
    }

    /// Whether the app runs without a window: it is busy with work the user is waiting for.
    pub(crate) fn busy(&self) -> bool {
        self.busy && running()
    }

    /// The shortest time between drawn frames the power state allows (hot or saving power), if any.
    pub(crate) fn frame_interval(&self) -> Option<std::time::Duration> {
        let rate = self.power?.max_frame_rate()?;
        Some(std::time::Duration::from_secs(1) / rate)
    }

    pub(crate) fn handle(&mut self, message: Message<W, S>) {
        match message {
            Message::Attach { window, size, make } => self.attach(window, size, make),
            Message::Resize(size) => {
                session::with(|h| h.resize(size));
            }
            Message::Detach { done } => {
                session::with(|h| h.detach());
                self.window = None;
                // The shell may have stopped waiting; then nobody needs the answer.
                let _ = done.send(());
            }
            Message::Stop => {
                session::stop();
                self.window = None;
                self.picked.clear();
                self.state = None;
                self.busy = false;
            }
            Message::Shell(shell) => {
                self.shell = shell;
                // A shell that starts listening later still sees the panels.
                if let (Some(shell), Some(json)) = (&mut self.shell, &self.state) {
                    shell.deliver(&Request::State(json.clone()));
                }
            }
            Message::Touch(touch) => {
                session::with(|h| h.touch(touch));
            }
            Message::OpenFile(file) => {
                if session::with(|h| h.open_file(file.clone())).is_none() {
                    self.early.push(file);
                }
            }
            Message::PickAdd(file) => self.picked.push(file),
            Message::PickDone => {
                let files = std::mem::take(&mut self.picked);
                session::with(|h| h.answer_pick(files));
            }
            Message::Action(action) => {
                session::with(|h| h.action(action));
            }
            Message::Input(input) => {
                if let Input::Power(power) = input {
                    self.power = Some(power);
                }
                session::with(|h| h.input(input));
            }
            Message::Sync { done } => {
                // The shell may have stopped waiting; then nobody needs the answer.
                let _ = done.send(());
            }
        }
    }

    #[expect(unsafe_code, reason = "hands the window to the host; the driver keeps it alive")]
    fn attach(&mut self, window: W, size: ViewSize, make: MakeHost) {
        let target = window.target();
        let fresh = !running();
        let attached = if fresh {
            // SAFETY: `window` is kept in `self.window` until the host detaches or is dropped
            // (`Detach`, `Stop`, or the next `Attach`, whose host drops the old surface before
            // the old window is released here).
            unsafe { make(target, size) }.map(session::install)
        } else {
            // SAFETY: as above.
            session::with(|h| unsafe { h.attach(target, size) }).unwrap_or(Err(HostError::Unsupported))
        };
        match attached {
            Ok(()) => {
                self.window = Some(window);
                // A new app hears the power state the shell reported before it started.
                if let (true, Some(power)) = (fresh, self.power) {
                    session::with(|h| h.input(Input::Power(power)));
                }
                for file in std::mem::take(&mut self.early) {
                    session::with(|h| h.open_file(file));
                }
            }
            Err(e) => {
                log::error!("craft-mobile: {e}");
                self.window = None;
            }
        }
    }

    /// Run a frame if one is due and hand the requests to the shell. Returns whether it drew.
    /// Without a window the host runs the app only while it's busy (or a native control waits),
    /// and doesn't draw.
    pub(crate) fn frame(&mut self) -> bool {
        let (drew, requests) = session::frame();
        for request in &requests {
            match request {
                Request::State(json) => self.state = Some(json.clone()),
                Request::Busy(busy) => self.busy = *busy,
                _ => {}
            }
            if let Some(shell) = &mut self.shell {
                shell.deliver(request);
            }
        }
        // Without a window nothing was drawn, whatever ran.
        drew && self.window.is_some()
    }
}

fn running() -> bool {
    session::with(|_| ()).is_some()
}

/// A request as the shells receive it: see [`Request::wire`].
pub(crate) fn wire(request: &Request) -> (i32, Cow<'_, str>, Option<&[u8]>) {
    request.wire()
}

/// A touch as the shells pass it: see [`crate::host::decode_touch`].
pub(crate) fn touch(id: u64, phase: u32, x: f32, y: f32, force: f32) -> Option<Touch> {
    crate::host::decode_touch(id, phase, x, y, force)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex, PoisonError, mpsc};

    use egui::{Pos2, TouchPhase};
    use wgpu::rwh::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

    use super::*;

    /// A window nobody draws into: the fake host never looks at its handle.
    struct FakeWindow;

    impl Window for FakeWindow {
        fn target(&self) -> wgpu::SurfaceTargetUnsafe {
            wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(NonNull::dangling())),
            }
        }
    }

    #[derive(Default)]
    struct Shared {
        log: Vec<String>,
        requests: Vec<Request>,
    }

    thread_local! {
        /// What the fake host saw, per test thread (the session is per thread too).
        static SEEN: RefCell<Shared> = RefCell::default();
    }

    fn seen() -> Vec<String> {
        SEEN.with(|s| s.borrow().log.clone())
    }

    fn note(line: String) {
        SEEN.with(|s| s.borrow_mut().log.push(line));
    }

    struct FakeHost;

    impl HostOps for FakeHost {
        fn input(&mut self, input: crate::Input) {
            note(format!("input {input:?}"));
        }
        fn resize(&mut self, size: ViewSize) {
            note(format!("resize {}", size.width_px));
        }
        fn touch(&mut self, touch: Touch) {
            note(format!("touch {}", touch.id));
        }
        fn frame(&mut self) -> bool {
            note("frame".into());
            true
        }
        fn open_file(&mut self, file: PickedFile) {
            note(format!("open {}", file.name));
        }
        fn action(&mut self, action: Action) {
            note(format!("action {action:?}"));
        }
        fn answer_pick(&mut self, files: Vec<PickedFile>) {
            note(format!("picked {}", files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(",")));
        }
        fn take_requests(&mut self) -> Vec<Request> {
            SEEN.with(|s| std::mem::take(&mut s.borrow_mut().requests))
        }
        fn detach(&mut self) {
            note("detach".into());
        }
        #[expect(unsafe_code, reason = "implements the trait's unsafe method; the fake touches no handle")]
        unsafe fn attach(&mut self, _target: wgpu::SurfaceTargetUnsafe, _size: ViewSize) -> Result<(), HostError> {
            note("attach".into());
            Ok(())
        }
    }

    #[expect(unsafe_code, reason = "a MakeHost; the fake touches no handle")]
    #[expect(clippy::unnecessary_wraps, reason = "matches the host factory's signature")]
    unsafe fn make(_target: wgpu::SurfaceTargetUnsafe, _size: ViewSize) -> Result<Box<dyn HostOps>, HostError> {
        note("make".into());
        Ok(Box::new(FakeHost))
    }

    #[derive(Clone, Default)]
    struct FakeShell(Arc<Mutex<Vec<String>>>);

    impl FakeShell {
        fn heard(&self) -> Vec<String> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
        }
    }

    impl Shell for FakeShell {
        fn deliver(&mut self, request: &Request) {
            let line = match request {
                Request::PickFiles { multiple, .. } => format!("pick {multiple}"),
                Request::Export { name, .. } => format!("export {name}"),
                Request::State(json) => format!("state {json}"),
                other => format!("{other:?}"),
            };
            self.0.lock().unwrap_or_else(PoisonError::into_inner).push(line);
        }
    }

    type TestDriver = Driver<FakeWindow, FakeShell>;

    fn size(width_px: u32) -> ViewSize {
        ViewSize { width_px, height_px: 600, scale: 2.0 }
    }

    fn attach(driver: &mut TestDriver) {
        driver.handle(Message::Attach { window: FakeWindow, size: size(800), make });
    }

    fn file(name: &str) -> PickedFile {
        PickedFile { name: name.into(), bytes: vec![1], location: None }
    }

    #[test]
    fn nothing_draws_before_the_app_starts() {
        let mut driver = TestDriver::default();
        assert!(!driver.drawing());
        assert!(!driver.frame());
        driver.handle(Message::Touch(Touch { id: 1, phase: TouchPhase::Start, pos: Pos2::ZERO, force: 0.0 }));
        assert!(seen().is_empty(), "no app, so nobody saw the touch");
        attach(&mut driver);
        assert!(driver.drawing());
        assert!(driver.frame());
        assert_eq!(seen(), ["make", "frame"]);
    }

    /// "Open with" launches the app with a file before its view exists: the app opens it when
    /// it starts, not never.
    #[test]
    fn a_file_handed_over_before_the_start_opens_when_the_app_is_up() {
        let mut driver = TestDriver::default();
        driver.handle(Message::OpenFile(file("early.png")));
        attach(&mut driver);
        driver.handle(Message::OpenFile(file("later.png")));
        assert_eq!(seen(), ["make", "open early.png", "open later.png"]);
    }

    /// Android takes the window away whenever another activity covers the app (the system
    /// picker does): the app keeps running and moves to the next window.
    #[test]
    fn the_app_outlives_its_window() {
        let mut driver = TestDriver::default();
        attach(&mut driver);
        let (done, answered) = mpsc::sync_channel(1);
        driver.handle(Message::Detach { done });
        assert_eq!(answered.try_recv(), Ok(()), "the UI thread hears the window is free");
        assert!(!driver.drawing());
        assert!(!driver.frame(), "nothing drawn without a window");
        attach(&mut driver);
        assert!(driver.drawing());
        assert_eq!(seen(), ["make", "detach", "frame", "attach"], "made once, then moved");
        driver.handle(Message::Stop);
        assert!(!driver.drawing());
        attach(&mut driver);
        assert_eq!(seen().last().map(String::as_str), Some("make"), "a stopped app starts afresh");
    }

    /// A busy app keeps running after its window goes (an export finishing in the background),
    /// until it says it's done.
    #[test]
    fn a_busy_app_runs_without_its_window_until_it_is_done() {
        let mut driver = TestDriver::default();
        attach(&mut driver);
        let shell = FakeShell::default();
        driver.handle(Message::Shell(Some(shell.clone())));
        SEEN.with(|s| s.borrow_mut().requests = vec![Request::Busy(true)]);
        driver.frame();
        let (done, _answered) = mpsc::sync_channel(1);
        driver.handle(Message::Detach { done });
        assert!(!driver.drawing() && driver.busy());
        SEEN.with(|s| s.borrow_mut().requests = vec![Request::Export { name: "a.png".into(), bytes: vec![] }, Request::Busy(false)]);
        driver.frame();
        assert!(!driver.busy() && !driver.drawing(), "idle without a window: the render thread stops polling");
        assert_eq!(seen(), ["make", "frame", "detach", "frame"]);
        assert_eq!(shell.heard(), ["Busy(true)", "export a.png", "Busy(false)"]);
    }

    /// The shell reports the power state as soon as it can, which may be before the app starts.
    #[test]
    fn an_app_hears_the_power_state_reported_before_it_started() {
        let mut driver = TestDriver::default();
        let hot = Power { thermal: crate::Thermal::Serious, low_power: false };
        driver.handle(Message::Input(Input::Power(hot)));
        attach(&mut driver);
        let (done, _answered) = mpsc::sync_channel(1);
        driver.handle(Message::Detach { done });
        attach(&mut driver);
        assert_eq!(seen(), ["make", "input Power(Power { thermal: Serious, low_power: false })", "detach", "attach"], "once, when made");
        assert_eq!(driver.frame_interval(), Some(std::time::Duration::from_secs(1) / 60), "and draws at most 60 frames a second");
    }

    #[test]
    fn the_picker_answers_with_every_file_added_then_starts_over() {
        let mut driver = TestDriver::default();
        attach(&mut driver);
        driver.handle(Message::PickAdd(file("a.png")));
        driver.handle(Message::PickAdd(file("b.psd")));
        driver.handle(Message::PickDone);
        driver.handle(Message::PickDone);
        assert_eq!(seen(), ["make", "picked a.png,b.psd", "picked "], "a second answer is a cancel");
    }

    #[test]
    fn requests_go_to_the_shell_and_a_late_shell_still_sees_the_panels() {
        let mut driver = TestDriver::default();
        attach(&mut driver);
        SEEN.with(|s| s.borrow_mut().requests = vec![Request::State("{}".into())]);
        driver.frame();
        let shell = FakeShell::default();
        driver.handle(Message::Shell(Some(shell.clone())));
        SEEN.with(|s| s.borrow_mut().requests = vec![Request::Export { name: "a.psd".into(), bytes: vec![] }]);
        driver.frame();
        assert_eq!(shell.heard(), ["state {}", "export a.psd"]);
    }

    /// What the shell sends reaches the running app in the order it was sent: a native
    /// control, a new size, the answer to a write in place.
    #[test]
    fn the_shells_calls_reach_the_app_in_order() {
        let mut driver = TestDriver::default();
        attach(&mut driver);
        driver.handle(Message::Action(Action::Save));
        driver.handle(Message::Resize(size(1024)));
        let location = crate::Location::new("content://docs/1").map(|location| Input::WriteDone { location, result: Err("read-only".into()) });
        driver.handle(Message::Input(location.unwrap_or(Input::Resume)));
        assert_eq!(
            seen(),
            ["make", "action Save", "resize 1024", r#"input WriteDone { location: Location("content://docs/1"), result: Err("read-only") }"#]
        );
    }

    /// The shell waits for the app's suspend (recovery writes) before Android may freeze it.
    #[test]
    fn a_sync_answers_after_what_came_before() {
        let mut driver = TestDriver::default();
        attach(&mut driver);
        driver.handle(Message::Input(Input::Suspend));
        let (done, answered) = mpsc::sync_channel(1);
        driver.handle(Message::Sync { done });
        assert_eq!(seen(), ["make", "input Suspend"]);
        assert_eq!(answered.try_recv(), Ok(()));
    }

    #[test]
    fn requests_cross_as_kind_text_and_data() {
        let pick = Request::PickFiles { multiple: true, extensions: vec!["png".into(), "psd".into()] };
        assert_eq!(wire(&pick), (1, "png,psd".into(), None));
        assert_eq!(wire(&Request::PickFiles { multiple: false, extensions: vec![] }), (0, "".into(), None));
        let export = Request::Export { name: "a.psd".into(), bytes: vec![7] };
        assert_eq!(wire(&export), (2, "a.psd".into(), Some(&[7_u8][..])));
        assert_eq!(wire(&Request::State("{}".into())), (3, "{}".into(), None));
        assert_eq!(wire(&Request::Keyboard(true)), (7, "1".into(), None));
        let image = Request::CopyImage { width: 1, height: 2, rgba: vec![0; 8] };
        assert_eq!(wire(&image), (5, "1×2".into(), Some(&[0_u8; 8][..])));
    }

    #[test]
    fn touches_decode_as_the_ios_interface_does() {
        assert_eq!(touch(1, 0, 1.0, 2.0, 0.5).map(|t| (t.phase, t.pos, t.force)), Some((TouchPhase::Start, Pos2::new(1.0, 2.0), 0.5)));
        assert_eq!(touch(1, 3, 0.0, 0.0, 9.0).map(|t| (t.phase, t.force)), Some((TouchPhase::Cancel, 1.0)));
        assert_eq!(touch(1, 1, 0.0, 0.0, f32::NAN).map(|t| t.force), Some(0.0));
        assert!(touch(1, 4, 0.0, 0.0, 0.0).is_none());
        assert!(touch(1, 1, f32::INFINITY, 0.0, 0.0).is_none());
    }
}
