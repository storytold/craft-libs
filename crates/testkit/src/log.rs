//! Capture of the records a test logs through the `log` crate.
//!
//! The module installs one `log::Log` implementation for the process. `log::set_logger` can be
//! called once per process, so a test binary uses one of two modes, and the first call that
//! installs the logger fixes the mode for the rest of the process:
//!
//! - Per thread: [`warnings`] and [`records`] run a closure and return its result with the records
//!   logged on the calling thread while it ran. Records from other threads are not captured.
//! - Process wide: [`capture_global`] returns a [`GlobalCapture`] whose [`GlobalCapture::drain`]
//!   takes the records logged on every thread since the last drain.
//!
//! A call for the other mode returns [`CaptureError::ModeConflict`]. When another logger is already
//! installed, every call returns [`CaptureError::LoggerAlreadySet`], runs no closure and leaves the
//! global maximum level as it is.
//!
//! A capture raises the global maximum level (`log::max_level`) to the level it asks for and never
//! lowers it, so a capture does not hide records from code that raised the level earlier. Each
//! capture filters by its own level.
//!
//! A capture keeps at most [`MAX_RECORDS`] records and [`MAX_BYTES`] bytes of message and target
//! text. Formatting a message stops at the first write that would take it past [`MAX_BYTES`]. A
//! record over either limit is dropped and counted, and the capture then returns
//! [`CaptureError::Overflow`] with the number dropped instead of a partial list.

use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use ::log::{Level, LevelFilter, Log, Metadata, Record};

/// The most records one capture keeps.
pub const MAX_RECORDS: usize = 4096;

/// The most bytes of message and target text one capture keeps.
pub const MAX_BYTES: usize = 1 << 20;

/// One captured record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRecord {
    /// The record's level.
    pub level: Level,
    /// The record's target, by default the module path of the call.
    pub target: String,
    /// The formatted message.
    pub message: String,
}

/// Why a capture could not run or could not return its records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaptureError {
    /// Another logger was installed in this process before this module's logger.
    LoggerAlreadySet,
    /// This process captures in the other mode: per thread or process wide.
    ModeConflict,
    /// [`warnings`] or [`records`] was called inside a capture on the same thread.
    NestedCapture,
    /// A [`GlobalCapture`] is still alive in this process.
    GlobalCaptureActive,
    /// The capture reached [`MAX_RECORDS`] or [`MAX_BYTES`] and dropped this many records.
    Overflow {
        /// The number of records dropped.
        dropped: usize,
    },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoggerAlreadySet => f.write_str("another logger is already installed in this process"),
            Self::ModeConflict => f.write_str("this process already captures logs in the other mode (per thread or process wide)"),
            Self::NestedCapture => f.write_str("a log capture is already running on this thread"),
            Self::GlobalCaptureActive => f.write_str("a process-wide log capture is already alive"),
            Self::Overflow { dropped } => {
                write!(f, "the log capture dropped {dropped} records over its limit of {MAX_RECORDS} records or {MAX_BYTES} bytes")
            }
        }
    }
}

impl std::error::Error for CaptureError {}

const MODE_NONE: u8 = 0;
const MODE_THREAD: u8 = 1;
const MODE_GLOBAL: u8 = 2;

/// The installed mode, read by the logger on every record.
static MODE: AtomicU8 = AtomicU8::new(MODE_NONE);

/// Serializes installation and changes of the global maximum level.
static INSTALL: Mutex<()> = Mutex::new(());

/// The bits of [`GLOBAL_STATE`] that hold the level.
const LEVEL_BITS: u32 = 3;
const LEVEL_MASK: u64 = (1 << LEVEL_BITS) - 1;

/// The process-wide capture's session number shifted left by [`LEVEL_BITS`], with its level as a
/// `LevelFilter` number in the low bits (`0`, `Off`, when no capture is alive). The logger reads
/// both in one load, so a record checked against one capture is never kept by a later one.
static GLOBAL_STATE: AtomicU64 = AtomicU64::new(0);

static GLOBAL: Mutex<Buffer> = Mutex::new(Buffer::new());

/// Records kept within [`MAX_RECORDS`] and [`MAX_BYTES`], with a count of those dropped.
struct Buffer {
    records: Vec<CapturedRecord>,
    bytes: usize,
    dropped: usize,
    /// For the process-wide buffer: whether a [`GlobalCapture`] is alive, and its session.
    alive: bool,
    session: u64,
}

impl Buffer {
    const fn new() -> Self {
        Self { records: Vec::new(), bytes: 0, dropped: 0, alive: false, session: 0 }
    }

    /// Keeps `record`, or counts it as dropped when it is `None` or over a limit.
    fn push(&mut self, record: Option<CapturedRecord>) {
        let Some(record) = record else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        let size = record.message.len().saturating_add(record.target.len());
        if self.records.len() >= MAX_RECORDS || size > MAX_BYTES.saturating_sub(self.bytes) {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.bytes += size;
        self.records.push(record);
    }

    /// The kept records, or the overflow, leaving the buffer empty.
    fn take(&mut self) -> Result<Vec<CapturedRecord>, CaptureError> {
        let records = std::mem::take(&mut self.records);
        let dropped = std::mem::take(&mut self.dropped);
        self.bytes = 0;
        if dropped > 0 { Err(CaptureError::Overflow { dropped }) } else { Ok(records) }
    }
}

/// The per-thread capture running on a thread.
struct Scope {
    level: LevelFilter,
    buffer: Buffer,
}

thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// A `fmt::Write` that keeps at most [`MAX_BYTES`] bytes and fails the write that would pass them.
struct Bounded {
    text: String,
    over: bool,
}

impl fmt::Write for Bounded {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if s.len() > MAX_BYTES.saturating_sub(self.text.len()) {
            self.over = true;
            return Err(fmt::Error);
        }
        self.text.push_str(s);
        Ok(())
    }
}

/// The record with its message formatted, or `None` when its message or target passes
/// [`MAX_BYTES`].
fn capture(record: &Record) -> Option<CapturedRecord> {
    let target = record.target();
    if target.len() > MAX_BYTES {
        return None;
    }
    let mut message = Bounded { text: String::new(), over: false };
    // A `Display` implementation's own error leaves the text written so far.
    let _ = fmt::write(&mut message, *record.args());
    if message.over {
        return None;
    }
    Some(CapturedRecord { level: record.level(), target: target.to_owned(), message: message.text })
}

struct Capture;

static CAPTURE: Capture = Capture;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn thread_level() -> LevelFilter {
    SCOPE.try_with(|s| s.try_borrow().ok().and_then(|s| s.as_ref().map(|s| s.level))).ok().flatten().unwrap_or(LevelFilter::Off)
}

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata) -> bool {
        match MODE.load(Ordering::Acquire) {
            MODE_THREAD => metadata.level() <= thread_level(),
            MODE_GLOBAL => metadata.level() as u64 <= GLOBAL_STATE.load(Ordering::Acquire) & LEVEL_MASK,
            _ => false,
        }
    }

    fn log(&self, record: &Record) {
        let level = record.level();
        // Each message is formatted before any capture state is borrowed or locked, so a
        // `Display` implementation that logs does not find that state in use.
        match MODE.load(Ordering::Acquire) {
            MODE_THREAD => {
                if level > thread_level() {
                    return;
                }
                let captured = capture(record);
                let _ = SCOPE.try_with(|s| {
                    if let Ok(mut s) = s.try_borrow_mut()
                        && let Some(scope) = s.as_mut()
                        && level <= scope.level
                    {
                        scope.buffer.push(captured);
                    }
                });
            }
            MODE_GLOBAL => {
                let state = GLOBAL_STATE.load(Ordering::Acquire);
                if level as u64 > state & LEVEL_MASK {
                    return;
                }
                let captured = capture(record);
                let mut buffer = lock(&GLOBAL);
                if buffer.alive && buffer.session == state >> LEVEL_BITS {
                    buffer.push(captured);
                }
            }
            _ => {}
        }
    }

    fn flush(&self) {}
}

/// Installs the logger in `mode` if no mode is set, then raises the global maximum level to
/// `level`. Nothing changes when the logger cannot be installed or the mode differs.
fn install(mode: u8, level: LevelFilter) -> Result<(), CaptureError> {
    let _guard = lock(&INSTALL);
    match MODE.load(Ordering::Acquire) {
        MODE_NONE => {
            ::log::set_logger(&CAPTURE).map_err(|_| CaptureError::LoggerAlreadySet)?;
            MODE.store(mode, Ordering::Release);
        }
        m if m == mode => {}
        _ => return Err(CaptureError::ModeConflict),
    }
    if ::log::max_level() < level {
        ::log::set_max_level(level);
    }
    Ok(())
}

/// Clears this thread's capture when dropped, including during a panic in the captured closure.
struct ScopeGuard;

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let _ = SCOPE.try_with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                *s = None;
            }
        });
    }
}

/// `f`'s result and the records at `level` or more severe that the calling thread logged while
/// `f` ran.
///
/// Records logged on other threads are not captured. Returns an error without running `f` when
/// another logger is installed, when this process captures process wide, or when a capture is
/// already running on this thread. Returns [`CaptureError::Overflow`] after `f` has run when it
/// logged more than the limits allow; `f`'s result is dropped in that case.
///
/// If `f` panics, the capture ends and the panic continues.
pub fn records<T>(level: LevelFilter, f: impl FnOnce() -> T) -> Result<(T, Vec<CapturedRecord>), CaptureError> {
    let running = SCOPE.try_with(|s| s.try_borrow().map_or(true, |s| s.is_some())).unwrap_or(true);
    if running {
        return Err(CaptureError::NestedCapture);
    }
    install(MODE_THREAD, level)?;
    let started = SCOPE
        .try_with(|s| match s.try_borrow_mut() {
            Ok(mut s) if s.is_none() => {
                *s = Some(Scope { level, buffer: Buffer::new() });
                true
            }
            _ => false,
        })
        .unwrap_or(false);
    if !started {
        return Err(CaptureError::NestedCapture);
    }
    let guard = ScopeGuard;
    let out = f();
    let taken =
        SCOPE.try_with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take()).map(|mut s| s.buffer.take())).ok().flatten().unwrap_or(Ok(Vec::new()));
    drop(guard);
    taken.map(|records| (out, records))
}

/// `f`'s result and the messages of the warnings and errors the calling thread logged while `f`
/// ran.
///
/// The same as [`records`] at `LevelFilter::Warn`, keeping only each record's message.
pub fn warnings<T>(f: impl FnOnce() -> T) -> Result<(T, Vec<String>), CaptureError> {
    records(LevelFilter::Warn, f).map(|(out, records)| (out, records.into_iter().map(|r| r.message).collect()))
}

/// Starts capturing the records at `level` or more severe that any thread logs.
///
/// Returns an error when another logger is installed, when this process captures per thread, or
/// when another [`GlobalCapture`] is alive. Dropping the handle stops the capture and discards
/// the records it holds; the logger stays installed, and a later call starts a new capture.
pub fn capture_global(level: LevelFilter) -> Result<GlobalCapture, CaptureError> {
    let mut buffer = lock(&GLOBAL);
    if buffer.alive {
        return Err(CaptureError::GlobalCaptureActive);
    }
    install(MODE_GLOBAL, level)?;
    let session = buffer.session.wrapping_add(1) & (u64::MAX >> LEVEL_BITS);
    *buffer = Buffer { alive: true, session, ..Buffer::new() };
    GLOBAL_STATE.store((session << LEVEL_BITS) | level as u64, Ordering::Release);
    Ok(GlobalCapture { _private: () })
}

/// A live process-wide capture. See [`capture_global`].
#[derive(Debug)]
pub struct GlobalCapture {
    _private: (),
}

impl GlobalCapture {
    /// The records captured on every thread since the capture started or since the last drain, in
    /// the order they reached the capture.
    ///
    /// Returns [`CaptureError::Overflow`] when records were dropped in that interval; the records
    /// kept in that interval are then discarded, and the next interval starts empty.
    pub fn drain(&self) -> Result<Vec<CapturedRecord>, CaptureError> {
        lock(&GLOBAL).take()
    }
}

impl Drop for GlobalCapture {
    fn drop(&mut self) {
        let mut buffer = lock(&GLOBAL);
        let session = buffer.session;
        GLOBAL_STATE.store(session << LEVEL_BITS, Ordering::Release);
        *buffer = Buffer { session, ..Buffer::new() };
    }
}
