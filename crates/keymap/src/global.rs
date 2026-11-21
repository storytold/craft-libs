//! System-wide hotkeys: chords that run a command while another application has the keyboard.
//!
//! [`GlobalHotkeys`] registers chords with the operating system: `RegisterHotKey` on Windows, a
//! Carbon hotkey on macOS and a key grab on the X11 root window on Linux and the BSDs.
//! [`GlobalHotkeys::poll`] returns each press and each release as a [`GlobalEvent`] naming the
//! command. A Wayland session is refused with [`GlobalError::Wayland`]: an X11 grab there sees
//! only keys typed into X11 windows. Other systems are refused with [`GlobalError::Unsupported`].
//!
//! The thread that calls [`GlobalHotkeys::new`] matters. On Windows the hotkey messages go to that
//! thread, which must run a message loop (the UI thread of a winit or eframe app does). On macOS it
//! must be the main thread. On X11 any thread works.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock, PoisonError};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::chord::{Chord, Key, Platform};
use crate::command::CommandRef;

/// The most chords one [`GlobalHotkeys`] holds.
pub const MAX_GLOBAL: usize = 128;

/// The most key events kept between two [`GlobalHotkeys::poll`] calls. Older events are dropped
/// first.
pub const MAX_QUEUED: usize = 256;

/// The most events waiting for [`GlobalHotkeys::poll`] before presses are dropped without their
/// releases: [`MAX_QUEUED`], and one press still held and one release of an earlier press for
/// each of [`MAX_GLOBAL`] chords.
pub const MAX_PENDING: usize = MAX_QUEUED + 2 * MAX_GLOBAL;

/// Whether a [`GlobalEvent`] is the key going down or coming back up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyState {
    Pressed,
    Released,
}

/// A press or a release of a registered chord.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalEvent {
    pub command: CommandRef,
    pub chord: Chord,
    pub state: KeyState,
}

/// Why a chord was not registered, or why [`GlobalHotkeys::new`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlobalError {
    /// The operating system is not Windows, macOS, Linux or a BSD.
    Unsupported,
    /// The desktop session is Wayland, where X11 key grabs miss keys typed into Wayland windows.
    Wayland,
    /// No X11 display could be opened. Holds the connection error.
    NoDisplay(String),
    /// Another [`GlobalHotkeys`] is open in this process.
    AlreadyOpen,
    /// The key has no physical key of its own (`+`, `:`, `|`, `?`, `!`, `{`, `}`), or the system
    /// cannot grab it (`'` on X11). On X11 a key is also refused when the current layout types its
    /// character only with AltGr (`[` on German): the grab would go to the key that types it
    /// (Ctrl+8 for Ctrl+[).
    Unbindable(Chord),
    /// A chord without Cmd, Ctrl or Alt whose key is not F1 to F35: it would take the key from
    /// typing in every application.
    TypingKey(Chord),
    /// On Windows, a chord with Ctrl and Alt on a letter, digit, symbol or Space key. Windows reports
    /// AltGr as Ctrl+Alt, so the chord would take a character from typing in every application on
    /// layouts with AltGr (Ctrl+Alt+E types `€` on German).
    AltGr(Chord),
    /// This set already holds the chord, or a chord the platform treats as the same one (`Cmd+K`
    /// and `Ctrl+K` off macOS; on X11, two keys the layout types on one key, such as Ctrl+- and
    /// Ctrl+6 on French AZERTY).
    Duplicate(Chord),
    /// The operating system or another application holds the chord.
    Taken(Chord),
    /// The set already holds [`MAX_GLOBAL`] chords.
    TooMany,
    /// [`GlobalHotkeys::unregister`] was given a chord the set does not hold.
    NotRegistered(Chord),
    /// The operating system refused. Holds its message.
    Os(String),
}

impl fmt::Display for GlobalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlobalError::Unsupported => f.write_str("system-wide hotkeys are not available on this operating system"),
            GlobalError::Wayland => f.write_str("system-wide hotkeys are not available in a Wayland session"),
            GlobalError::NoDisplay(e) => write!(f, "no X11 display: {e}"),
            GlobalError::AlreadyOpen => f.write_str("system-wide hotkeys are already open in this process"),
            GlobalError::Unbindable(c) => write!(f, "{c} has no key of its own"),
            GlobalError::TypingKey(c) => write!(f, "{c} needs Cmd, Ctrl or Alt to work system-wide"),
            GlobalError::AltGr(c) => write!(f, "{c} is AltGr on Windows, which types characters on many keyboard layouts"),
            GlobalError::Duplicate(c) => write!(f, "{c} is already registered"),
            GlobalError::Taken(c) => write!(f, "{c} is in use by the system or another application"),
            GlobalError::TooMany => write!(f, "more than {MAX_GLOBAL} system-wide hotkeys"),
            GlobalError::NotRegistered(c) => write!(f, "{c} is not registered"),
            GlobalError::Os(e) => write!(f, "the system refused the hotkey: {e}"),
        }
    }
}

impl std::error::Error for GlobalError {}

/// A binding [`GlobalHotkeys::replace`] could not register.
#[derive(Clone, Debug, PartialEq)]
pub struct Refused {
    pub command: CommandRef,
    pub chord: Chord,
    pub error: GlobalError,
}

type Waker = Arc<dyn Fn() + Send + Sync>;

/// Events from the operating system, waiting for [`GlobalHotkeys::poll`].
struct Shared {
    queue: Mutex<Queue>,
    waker: Mutex<Option<Waker>>,
}

#[derive(Default)]
struct Queue {
    events: VecDeque<GlobalHotKeyEvent>,
    /// Events were dropped since the last drain, so which chords are down is unknown.
    overflowed: bool,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared { queue: Mutex::new(Queue::default()), waker: Mutex::new(None) })
}

fn push(event: GlobalHotKeyEvent) {
    let shared = shared();
    push_bounded(&mut shared.queue.lock().unwrap_or_else(PoisonError::into_inner), event);
    // Called outside the queue lock, so a waker that polls at once does not wait on it.
    let waker = shared.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(wake) = waker {
        wake();
    }
}

fn push_bounded(queue: &mut Queue, event: GlobalHotKeyEvent) {
    if queue.events.len() >= MAX_QUEUED {
        queue.events.pop_front();
        queue.overflowed = true;
    }
    queue.events.push_back(event);
}

fn take_queue() -> Queue {
    std::mem::take(&mut *shared().queue.lock().unwrap_or_else(PoisonError::into_inner))
}

static OPEN: AtomicBool = AtomicBool::new(false);

struct Entry {
    hotkey: HotKey,
    chord: Chord,
    command: CommandRef,
    down: bool,
    /// The key the system grabs for it ([`KeyMap::slot`]). No two entries share one.
    slot: u64,
}

/// System-wide hotkeys, one set per process.
///
/// ```no_run
/// use craft_keymap::global::{GlobalHotkeys, KeyState};
/// use craft_keymap::Chord;
///
/// let mut hotkeys = GlobalHotkeys::new()?;
/// hotkeys.register("record.toggle", Chord::parse("Cmd+Shift+F9").unwrap())?;
/// // An egui app passes `move || ctx.request_repaint()`, so a press arrives while it is idle.
/// hotkeys.set_waker(|| {});
/// // Once a frame:
/// for event in hotkeys.poll() {
///     if event.state == KeyState::Pressed {
///         println!("run {}", event.command.key());
///     }
/// }
/// # Ok::<(), craft_keymap::global::GlobalError>(())
/// ```
pub struct GlobalHotkeys {
    manager: GlobalHotKeyManager,
    platform: Platform,
    entries: Vec<Entry>,
    pending: Vec<GlobalEvent>,
    session: Session,
}

impl GlobalHotkeys {
    /// Opens the process's set of system-wide hotkeys. Fails with [`GlobalError::AlreadyOpen`]
    /// while another set is open, with [`GlobalError::Wayland`] or [`GlobalError::NoDisplay`]
    /// where no X11 grab can work, and with [`GlobalError::Unsupported`] on other systems than
    /// Windows, macOS, Linux and the BSDs.
    pub fn new() -> Result<GlobalHotkeys, GlobalError> {
        let session = check_session()?;
        if OPEN.swap(true, Ordering::AcqRel) {
            return Err(GlobalError::AlreadyOpen);
        }
        static HANDLER: Once = Once::new();
        HANDLER.call_once(|| GlobalHotKeyEvent::set_event_handler(Some(push)));
        take_queue();
        match GlobalHotKeyManager::new().map_err(|e| GlobalError::Os(e.to_string())).and_then(|manager| check_worker(&manager).map(|()| manager)) {
            Ok(manager) => Ok(GlobalHotkeys { manager, platform: Platform::current(), entries: Vec::new(), pending: Vec::new(), session }),
            Err(e) => {
                OPEN.store(false, Ordering::Release);
                Err(e)
            }
        }
    }

    /// Calls `wake` from the thread that receives each key event, so an app that draws only on
    /// input can poll. It replaces an earlier waker.
    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        *shared().waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(wake));
    }

    /// Removes the waker.
    pub fn clear_waker(&mut self) {
        *shared().waker.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Registers `chord` to run `command`.
    pub fn register(&mut self, command: impl Into<CommandRef>, chord: Chord) -> Result<(), GlobalError> {
        if self.entries.len() >= MAX_GLOBAL {
            return Err(GlobalError::TooMany);
        }
        let hotkey = hotkey(chord, self.platform)?;
        let slot = KeyMap::read(&self.session)?.slot(&hotkey, chord)?;
        if self.entries.iter().any(|e| e.slot == slot) {
            return Err(GlobalError::Duplicate(chord));
        }
        self.manager.register(hotkey).map_err(|e| os_error(e, chord))?;
        self.entries.push(Entry { hotkey, chord, command: command.into(), down: false, slot });
        Ok(())
    }

    /// Unregisters `chord` and returns the command it ran. A chord that is held down gets its
    /// release from the next [`poll`](GlobalHotkeys::poll). The chord leaves the set also when
    /// the system reports an error.
    pub fn unregister(&mut self, chord: &Chord) -> Result<CommandRef, GlobalError> {
        let index = match hotkey(*chord, self.platform) {
            Ok(hotkey) => self.entries.iter().position(|e| e.hotkey.id() == hotkey.id()),
            Err(_) => None,
        };
        let Some(index) = index else {
            return Err(GlobalError::NotRegistered(*chord));
        };
        let result = self.manager.unregister(self.entries[index].hotkey).map_err(|e| os_error(e, *chord));
        // Events the system sent until the chord was unregistered (on X11 the grab thread sends them
        // before `unregister` returns) still go to this command.
        self.flush();
        let entry = self.entries.remove(index);
        self.release_if_down(&entry);
        result.map(|()| entry.command)
    }

    /// Replaces the registered chords with `bindings`, and returns those that were refused. A
    /// binding that was already registered for the same command stays registered, so a chord held
    /// down through the change is not released.
    pub fn replace(&mut self, bindings: impl IntoIterator<Item = (CommandRef, Chord)>) -> Vec<Refused> {
        let mut refused = Vec::new();
        let mut wanted = Vec::new();
        let keys = KeyMap::read(&self.session);
        for (command, chord) in bindings {
            let slot = hotkey(chord, self.platform).and_then(|hotkey| keys.as_ref().map_err(Clone::clone)?.slot(&hotkey, chord));
            match slot {
                Ok(slot) => wanted.push((command, chord, slot)),
                Err(error) => refused.push(Refused { command, chord, error }),
            }
        }
        let keep = kept(&self.entries, &wanted);
        let gone: Vec<HotKey> = self.entries.iter().zip(&keep).filter(|(_, kept)| kept.is_none()).map(|(e, _)| e.hotkey).collect();
        self.unregister_many(&gone);
        // Events the system sent for these chords until they were unregistered go to the
        // commands they ran.
        self.flush();
        let mut slots: Vec<Option<Entry>> = wanted.iter().map(|_| None).collect();
        for (entry, kept) in std::mem::take(&mut self.entries).into_iter().zip(keep) {
            match kept {
                Some(slot) => slots[slot] = Some(entry),
                None => self.release_if_down(&entry),
            }
        }
        // The new bindings, checked against the kept ones and each other, so the batch below never
        // holds a key that a kept entry or another new one grabs.
        let kept_ids: Vec<u64> = slots.iter().flatten().map(|e| e.slot).collect();
        let (mut new, new_ids) = new_entries(&slots, &wanted, self.platform);
        // Registered in rounds that fill the room left under `MAX_GLOBAL`, so a chord the system
        // refuses leaves its place to the next one.
        let candidates: Vec<usize> = (0..new.len()).filter(|&i| new[i].is_ok()).collect();
        let room = MAX_GLOBAL.saturating_sub(slots.iter().flatten().count());
        let left = in_rounds(&candidates, room, |round| {
            self.register_many(&mut new, round);
            round.iter().filter(|&&i| new[i].is_ok()).count()
        });
        for &i in left {
            new[i] = Err(GlobalError::TooMany);
        }
        // A second copy of a chord that no copy registered gets the first copy's refusal.
        for i in 0..new.len() {
            let Err(GlobalError::Duplicate(chord)) = new[i] else { continue };
            if !kept_ids.contains(&new_ids[i])
                && let Some(error) = first_refusal(&new, &new_ids, new_ids[i])
            {
                new[i] = Err(error.for_chord(chord));
            }
        }
        // The set in the order of `bindings`, the kept entries in their places.
        let mut new = new.into_iter();
        for (slot, (command, chord, _)) in slots.into_iter().zip(wanted) {
            let result = match slot {
                Some(entry) => Ok(entry),
                None => new.next().unwrap_or(Err(GlobalError::TooMany)),
            };
            match result {
                Ok(entry) => self.entries.push(entry),
                Err(error) => refused.push(Refused { command, chord, error }),
            }
        }
        refused
    }

    /// Unregisters every chord.
    pub fn clear(&mut self) {
        let hotkeys: Vec<HotKey> = self.entries.iter().map(|e| e.hotkey).collect();
        self.unregister_many(&hotkeys);
        // Every event the system sent for these chords goes to the commands they ran.
        self.flush();
        for entry in std::mem::take(&mut self.entries) {
            self.release_if_down(&entry);
        }
    }

    /// Registers the entries of `new` at the positions `at` with the system, and replaces each one
    /// the system refuses with its error. On X11 they go to the key-grab thread in one request; it
    /// handles one request every 50 ms. When that request fails, the batch is unregistered and
    /// registered again one by one. No entry of the batch grabs a key that another entry grabs
    /// ([`new_entries`]), so unregistering the batch leaves the other entries' grabs in place.
    fn register_many(&self, new: &mut [Result<Entry, GlobalError>], at: &[usize]) {
        let hotkeys: Vec<HotKey> = at.iter().filter_map(|&i| new[i].as_ref().ok().map(|e| e.hotkey)).collect();
        if hotkeys.is_empty() || (BATCH && self.manager.register_all(&hotkeys).is_ok()) {
            return;
        }
        if BATCH {
            // Which chord failed is not reported, so the batch is undone and registered one by one.
            let _ = self.manager.unregister_all(&hotkeys);
        }
        for &i in at {
            let result = &mut new[i];
            let (hotkey, chord) = match result {
                Ok(entry) => (entry.hotkey, entry.chord),
                Err(_) => continue,
            };
            if let Err(error) = self.manager.register(hotkey) {
                *result = Err(os_error(error, chord));
            }
        }
    }

    /// Unregisters `hotkeys` from the system, in one request on X11. The entries go whether or
    /// not the system confirms: the app asked for them to go.
    fn unregister_many(&self, hotkeys: &[HotKey]) {
        if hotkeys.is_empty() {
            return;
        }
        if BATCH {
            let _ = self.manager.unregister_all(hotkeys);
            return;
        }
        for &hotkey in hotkeys {
            let _ = self.manager.unregister(hotkey);
        }
    }

    /// The registered chords and their commands, in registration order.
    pub fn bindings(&self) -> impl Iterator<Item = (&CommandRef, Chord)> {
        self.entries.iter().map(|e| (&e.command, e.chord))
    }

    /// The presses and releases since the last call, in order. A chord held down is pressed once;
    /// a release without its press is left out. When more than [`MAX_QUEUED`] events arrived
    /// since the last call, every chord that was down is released first. When more than
    /// [`MAX_QUEUED`] events wait here, the oldest press whose release also waits is dropped with
    /// that release. Past [`MAX_PENDING`] events the oldest press is dropped; releases never are.
    pub fn poll(&mut self) -> Vec<GlobalEvent> {
        self.flush();
        std::mem::take(&mut self.pending)
    }

    /// Moves the system's events into `pending`, matched to the current entries.
    fn flush(&mut self) {
        let queue = take_queue();
        drain(&mut self.entries, queue, &mut self.pending);
    }

    fn release_if_down(&mut self, entry: &Entry) {
        if entry.down {
            push_pending(&mut self.pending, GlobalEvent { command: entry.command.clone(), chord: entry.chord, state: KeyState::Released });
        }
    }
}

/// The code the backend grabs for `code`, or `None` when it cannot grab the key. global-hotkey's
/// X11 backend grabs `IntlBackslash` and `Backslash` as the one `backslash` keysym, so both chords
/// get one hotkey id there and the second is refused as a duplicate. It looks `Quote` up as the
/// `leftsinglequotemark` keysym, which the `'` key does not type.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn backend_code(code: Code) -> Option<Code> {
    match code {
        Code::IntlBackslash => Some(Code::Backslash),
        Code::Quote => None,
        code => Some(code),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
fn backend_code(code: Code) -> Option<Code> {
    Some(code)
}

/// The X11 connection [`KeyMap::read`] reads the keyboard mapping through.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
type Session = x11rb::rust_connection::RustConnection;

#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
struct Session;

/// The keyboard mapping of the X11 display: for each key code from `min_keycode` on,
/// `per_keycode` keysyms, group 1 levels 1 and 2 first.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
struct KeyMap {
    min_keycode: u8,
    per_keycode: usize,
    keysyms: Vec<u32>,
}

#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
impl KeyMap {
    fn read(display: &Session) -> Result<KeyMap, GlobalError> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt;
        let setup = display.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let os = |e: &dyn fmt::Display| GlobalError::Os(format!("cannot read the keyboard mapping: {e}"));
        let reply = display.get_keyboard_mapping(min, max.saturating_sub(min).saturating_add(1)).map_err(|e| os(&e))?.reply().map_err(|e| os(&e))?;
        Ok(KeyMap { min_keycode: min, per_keycode: usize::from(reply.keysyms_per_keycode), keysyms: reply.keysyms })
    }

    /// The key code and modifiers global-hotkey grabs for `hotkey`, as one number. It grabs the
    /// first key code whose keysyms hold the key's keysym. The chord is refused when that key
    /// types the keysym only at a third or later level (AltGr): the grab would then go to another
    /// key than the one `chord` names. Letters and digits may be at level 2 (`A` is the shifted
    /// letter, and French AZERTY types digits with Shift). Other keys may be at level 2 when the
    /// chord holds Shift (Ctrl+Shift+. on French AZERTY), and must be at level 1 otherwise.
    fn slot(&self, hotkey: &HotKey, chord: Chord) -> Result<u64, GlobalError> {
        let keysym = x11_keysym(hotkey.key).ok_or(GlobalError::Unbindable(chord))?;
        let (index, keysyms) =
            self.keysyms.chunks(self.per_keycode.max(1)).enumerate().find(|(_, k)| k.contains(&keysym)).ok_or(GlobalError::Unbindable(chord))?;
        let levels = if chord.key.is_letter() || chord.key.is_digit() || hotkey.mods.contains(Modifiers::SHIFT) { 2 } else { 1 };
        if !keysyms[..levels.min(keysyms.len())].contains(&keysym) {
            return Err(GlobalError::Unbindable(chord));
        }
        let keycode = u64::from(self.min_keycode) + index as u64;
        Ok((keycode << 32) | u64::from(hotkey.mods.bits()))
    }
}

/// Elsewhere the system grabs one key for each hotkey id.
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
struct KeyMap;

#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
impl KeyMap {
    fn read(_session: &Session) -> Result<KeyMap, GlobalError> {
        Ok(KeyMap)
    }

    fn slot(&self, hotkey: &HotKey, _chord: Chord) -> Result<u64, GlobalError> {
        Ok(u64::from(hotkey.id()))
    }
}

/// The keysym global-hotkey 0.9.0 looks up for each code [`code`] and [`backend_code`] return
/// (its `keycode_to_x11_keysym`), from X11's `keysymdef.h`.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn x11_keysym(code: Code) -> Option<u32> {
    const LETTERS: [Code; 26] = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    const DIGITS: [Code; 10] =
        [Code::Digit0, Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4, Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9];
    const FUNCTION: [Code; 35] = [
        Code::F1,
        Code::F2,
        Code::F3,
        Code::F4,
        Code::F5,
        Code::F6,
        Code::F7,
        Code::F8,
        Code::F9,
        Code::F10,
        Code::F11,
        Code::F12,
        Code::F13,
        Code::F14,
        Code::F15,
        Code::F16,
        Code::F17,
        Code::F18,
        Code::F19,
        Code::F20,
        Code::F21,
        Code::F22,
        Code::F23,
        Code::F24,
        Code::F25,
        Code::F26,
        Code::F27,
        Code::F28,
        Code::F29,
        Code::F30,
        Code::F31,
        Code::F32,
        Code::F33,
        Code::F34,
        Code::F35,
    ];
    let at = |list: &[Code], first: u32| list.iter().position(|&c| c == code).map(|i| first + i as u32);
    // `A` (0x41) to `Z`, `0` (0x30) to `9`, `F1` (0xffbe) to `F35`.
    if let Some(keysym) = at(&LETTERS, 0x41).or_else(|| at(&DIGITS, 0x30)).or_else(|| at(&FUNCTION, 0xffbe)) {
        return Some(keysym);
    }
    Some(match code {
        Code::Escape => 0xff1b,
        Code::Tab => 0xff09,
        Code::Backspace => 0xff08,
        Code::Enter => 0xff0d,
        Code::Space => 0x20,
        Code::Insert => 0xff63,
        Code::Delete => 0xffff,
        Code::Home => 0xff50,
        Code::End => 0xff57,
        Code::PageUp => 0xff55,
        Code::PageDown => 0xff56,
        Code::ArrowUp => 0xff52,
        Code::ArrowDown => 0xff54,
        Code::ArrowLeft => 0xff51,
        Code::ArrowRight => 0xff53,
        Code::Minus => 0x2d,
        Code::Equal => 0x3d,
        Code::BracketLeft => 0x5b,
        Code::BracketRight => 0x5d,
        Code::Backslash => 0x5c,
        Code::Semicolon => 0x3b,
        Code::Backquote => 0x60,
        Code::Comma => 0x2c,
        Code::Period => 0x2e,
        Code::Slash => 0x2f,
        _ => return None,
    })
}

/// Whether hotkeys are registered in batches: global-hotkey's X11 key-grab thread handles one
/// request every 50 ms, while `RegisterHotKey` and the Carbon calls return at once.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
const BATCH: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
const BATCH: bool = false;

/// For each binding in `wanted` that has no kept entry in `slots`, a new entry, or
/// [`GlobalError::Duplicate`] when a kept entry or an earlier binding grabs the same key. Also
/// returns the slot of each.
fn new_entries(slots: &[Option<Entry>], wanted: &[(CommandRef, Chord, u64)], platform: Platform) -> (Vec<Result<Entry, GlobalError>>, Vec<u64>) {
    let mut taken: Vec<u64> = slots.iter().flatten().map(|e| e.slot).collect();
    let mut new = Vec::new();
    let mut new_ids = Vec::new();
    for (kept, (command, chord, slot)) in slots.iter().zip(wanted) {
        if kept.is_some() {
            continue;
        }
        let result = if taken.contains(slot) {
            Err(GlobalError::Duplicate(*chord))
        } else {
            taken.push(*slot);
            hotkey(*chord, platform).map(|hotkey| Entry { hotkey, chord: *chord, command: command.clone(), down: false, slot: *slot })
        };
        new.push(result);
        new_ids.push(*slot);
    }
    (new, new_ids)
}

/// The refusal of the first binding in `new` with slot `id`, when none with that slot registered.
fn first_refusal(new: &[Result<Entry, GlobalError>], ids: &[u64], id: u64) -> Option<GlobalError> {
    let mut same = new.iter().zip(ids).filter(|(_, i)| **i == id).map(|(r, _)| r);
    match same.next()? {
        Ok(_) => None,
        Err(GlobalError::Duplicate(_)) => None,
        Err(error) => (!same.any(|r| r.is_ok())).then(|| error.clone()),
    }
}

impl GlobalError {
    /// The same refusal for another chord.
    fn for_chord(self, chord: Chord) -> GlobalError {
        match self {
            GlobalError::Unbindable(_) => GlobalError::Unbindable(chord),
            GlobalError::TypingKey(_) => GlobalError::TypingKey(chord),
            GlobalError::AltGr(_) => GlobalError::AltGr(chord),
            GlobalError::Duplicate(_) => GlobalError::Duplicate(chord),
            GlobalError::Taken(_) => GlobalError::Taken(chord),
            GlobalError::NotRegistered(_) => GlobalError::NotRegistered(chord),
            other => other,
        }
    }
}

/// Hands `candidates` to `register` in rounds no larger than the room left, where `register`
/// returns how many of a round it registered, and returns the candidates no round took.
fn in_rounds(candidates: &[usize], mut room: usize, mut register: impl FnMut(&[usize]) -> usize) -> &[usize] {
    let mut at = 0;
    while at < candidates.len() && room > 0 {
        let round = &candidates[at..(at + room).min(candidates.len())];
        room -= register(round).min(round.len());
        at += round.len();
    }
    &candidates[at..]
}

/// For each entry, the index of the binding in `wanted` it already registers (same chord, same
/// command), or `None` when the entry is to go. Each binding keeps at most one entry.
fn kept(entries: &[Entry], wanted: &[(CommandRef, Chord, u64)]) -> Vec<Option<usize>> {
    let mut taken = vec![false; wanted.len()];
    entries
        .iter()
        .map(|entry| {
            let slot = wanted
                .iter()
                .zip(&taken)
                .position(|((command, chord, id), taken)| !taken && *id == entry.slot && *chord == entry.chord && *command == entry.command)?;
            taken[slot] = true;
            Some(slot)
        })
        .collect()
}

impl Drop for GlobalHotkeys {
    fn drop(&mut self) {
        self.clear();
        self.clear_waker();
        take_queue();
        OPEN.store(false, Ordering::Release);
    }
}

fn os_error(error: global_hotkey::Error, chord: Chord) -> GlobalError {
    match error {
        global_hotkey::Error::AlreadyRegistered(_) => GlobalError::Taken(chord),
        other => GlobalError::Os(other.to_string()),
    }
}

/// Matches a drained queue to `entries` and appends the result to `out`. After an overflow, every
/// chord that was down is released first, since its release may be among the dropped events.
fn drain(entries: &mut [Entry], queue: Queue, out: &mut Vec<GlobalEvent>) {
    if queue.overflowed {
        for entry in entries.iter_mut().filter(|e| e.down) {
            entry.down = false;
            push_pending(out, GlobalEvent { command: entry.command.clone(), chord: entry.chord, state: KeyState::Released });
        }
    }
    for event in queue.events {
        if let Some(event) = translate(entries, event) {
            push_pending(out, event);
        }
    }
}

/// Appends to the events waiting for [`GlobalHotkeys::poll`]. Beyond [`MAX_QUEUED`] it drops the
/// oldest press whose release also waits, together with that release. A press still held and a
/// release of a press the app already has are kept, so every press the app gets has its release.
/// Each chord has at most one of each, so the list holds at most [`MAX_PENDING`] events; past
/// that the oldest press is dropped. A release is never dropped.
fn push_pending(pending: &mut Vec<GlobalEvent>, event: GlobalEvent) {
    if pending.len() >= MAX_QUEUED {
        if let Some((i, j)) = oldest_pair(pending) {
            pending.remove(j);
            pending.remove(i);
        } else if pending.len() >= MAX_PENDING
            && let Some(i) = pending.iter().position(|e| e.state == KeyState::Pressed)
        {
            pending.remove(i);
        }
    }
    pending.push(event);
}

/// The positions of the oldest press and its release in `pending`.
fn oldest_pair(pending: &[GlobalEvent]) -> Option<(usize, usize)> {
    pending.iter().enumerate().filter(|(_, e)| e.state == KeyState::Pressed).find_map(|(i, pressed)| {
        let j = pending[i + 1..].iter().position(|e| e.chord == pressed.chord && e.command == pressed.command)?;
        let release = &pending[i + 1 + j];
        (release.state == KeyState::Released).then_some((i, i + 1 + j))
    })
}

/// Matches one event from the system to the entry it belongs to, and drops repeated presses and
/// releases without a press.
fn translate(entries: &mut [Entry], event: GlobalHotKeyEvent) -> Option<GlobalEvent> {
    let entry = entries.iter_mut().find(|e| e.hotkey.id() == event.id)?;
    let state = match event.state {
        HotKeyState::Pressed if !entry.down => KeyState::Pressed,
        HotKeyState::Released if entry.down => KeyState::Released,
        HotKeyState::Pressed | HotKeyState::Released => return None,
    };
    entry.down = state == KeyState::Pressed;
    Some(GlobalEvent { command: entry.command.clone(), chord: entry.chord, state })
}

/// The system hotkey for `chord` on `platform`.
fn hotkey(chord: Chord, platform: Platform) -> Result<HotKey, GlobalError> {
    let code = code(chord.key).and_then(backend_code).ok_or(GlobalError::Unbindable(chord))?;
    let mods = chord.mods.effective(platform);
    if !(mods.cmd || mods.ctrl || mods.alt) && !chord.key.is_function_key() {
        return Err(GlobalError::TypingKey(chord));
    }
    let types = chord.key.is_letter() || chord.key.is_digit() || chord.key.is_symbol() || chord.key == Key::Space;
    if platform == Platform::Windows && (mods.cmd || mods.ctrl) && mods.alt && types {
        return Err(GlobalError::AltGr(chord));
    }
    let mut out = Modifiers::empty();
    if mods.cmd {
        out |= if platform.is_mac() { Modifiers::META } else { Modifiers::CONTROL };
    }
    if mods.ctrl {
        out |= Modifiers::CONTROL;
    }
    if mods.alt {
        out |= Modifiers::ALT;
    }
    if mods.shift {
        out |= Modifiers::SHIFT;
    }
    Ok(HotKey::new(Some(out), code))
}

/// The key code for `key`, named by its position on a US keyboard. Keys typed with Shift (`+`,
/// `:`) have none. On Windows and X11 the system grabs a letter at the key that types it in the
/// current layout; on macOS it grabs the key at the US position.
fn code(key: Key) -> Option<Code> {
    use Key::*;
    Some(match key {
        A => Code::KeyA,
        B => Code::KeyB,
        C => Code::KeyC,
        D => Code::KeyD,
        E => Code::KeyE,
        F => Code::KeyF,
        G => Code::KeyG,
        H => Code::KeyH,
        I => Code::KeyI,
        J => Code::KeyJ,
        K => Code::KeyK,
        L => Code::KeyL,
        M => Code::KeyM,
        N => Code::KeyN,
        O => Code::KeyO,
        P => Code::KeyP,
        Q => Code::KeyQ,
        R => Code::KeyR,
        S => Code::KeyS,
        T => Code::KeyT,
        U => Code::KeyU,
        V => Code::KeyV,
        W => Code::KeyW,
        X => Code::KeyX,
        Y => Code::KeyY,
        Z => Code::KeyZ,
        Num0 => Code::Digit0,
        Num1 => Code::Digit1,
        Num2 => Code::Digit2,
        Num3 => Code::Digit3,
        Num4 => Code::Digit4,
        Num5 => Code::Digit5,
        Num6 => Code::Digit6,
        Num7 => Code::Digit7,
        Num8 => Code::Digit8,
        Num9 => Code::Digit9,
        F1 => Code::F1,
        F2 => Code::F2,
        F3 => Code::F3,
        F4 => Code::F4,
        F5 => Code::F5,
        F6 => Code::F6,
        F7 => Code::F7,
        F8 => Code::F8,
        F9 => Code::F9,
        F10 => Code::F10,
        F11 => Code::F11,
        F12 => Code::F12,
        F13 => Code::F13,
        F14 => Code::F14,
        F15 => Code::F15,
        F16 => Code::F16,
        F17 => Code::F17,
        F18 => Code::F18,
        F19 => Code::F19,
        F20 => Code::F20,
        F21 => Code::F21,
        F22 => Code::F22,
        F23 => Code::F23,
        F24 => Code::F24,
        F25 => Code::F25,
        F26 => Code::F26,
        F27 => Code::F27,
        F28 => Code::F28,
        F29 => Code::F29,
        F30 => Code::F30,
        F31 => Code::F31,
        F32 => Code::F32,
        F33 => Code::F33,
        F34 => Code::F34,
        F35 => Code::F35,
        Escape => Code::Escape,
        Tab => Code::Tab,
        Backspace => Code::Backspace,
        Enter => Code::Enter,
        Space => Code::Space,
        Insert => Code::Insert,
        Delete => Code::Delete,
        Home => Code::Home,
        End => Code::End,
        PageUp => Code::PageUp,
        PageDown => Code::PageDown,
        Up => Code::ArrowUp,
        Down => Code::ArrowDown,
        Left => Code::ArrowLeft,
        Right => Code::ArrowRight,
        Minus => Code::Minus,
        Equals => Code::Equal,
        OpenBracket => Code::BracketLeft,
        CloseBracket => Code::BracketRight,
        Backslash => Code::Backslash,
        Semicolon => Code::Semicolon,
        Quote => Code::Quote,
        Backtick => Code::Backquote,
        Comma => Code::Comma,
        Period => Code::Period,
        Slash => Code::Slash,
        IntlBackslash => Code::IntlBackslash,
        Plus | Colon | Pipe | Questionmark | Exclamationmark | OpenCurlyBracket | CloseCurlyBracket => return None,
    })
}

#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn check_session() -> Result<Session, GlobalError> {
    if is_wayland(std::env::var_os("WAYLAND_DISPLAY").as_deref(), std::env::var_os("XDG_SESSION_TYPE").as_deref()) {
        return Err(GlobalError::Wayland);
    }
    open_display(None)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn check_session() -> Result<Session, GlobalError> {
    Ok(Session)
}

/// global-hotkey has no backend for other systems (Android, iOS): it accepts every chord there
/// and never reports a key.
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
fn check_session() -> Result<Session, GlobalError> {
    Err(GlobalError::Unsupported)
}

/// Checks that global-hotkey's X11 key-grab thread is running. The thread exits when the display
/// lacks the XKEYBOARD extension, and `register` then returns `Ok` for every chord without grabbing
/// it. A running thread refuses a key it has no keysym for (`Fn`), and an exited one leaves the
/// request unanswered, which `register` reports as `Ok`.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn check_worker(manager: &GlobalHotKeyManager) -> Result<(), GlobalError> {
    let probe = HotKey::new(None, Code::Fn);
    match manager.register(probe) {
        Err(_) => Ok(()),
        Ok(()) => {
            let _ = manager.unregister(probe);
            Err(GlobalError::Os("the X11 key-grab thread is not running; the display may lack the XKEYBOARD extension".to_string()))
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
fn check_worker(_manager: &GlobalHotKeyManager) -> Result<(), GlobalError> {
    Ok(())
}

/// Whether the session is Wayland, from `WAYLAND_DISPLAY` and `XDG_SESSION_TYPE`.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn is_wayland(wayland_display: Option<&std::ffi::OsStr>, session_type: Option<&std::ffi::OsStr>) -> bool {
    wayland_display.is_some_and(|d| !d.is_empty()) || session_type.is_some_and(|t| t.eq_ignore_ascii_case("wayland"))
}

/// Opens a connection to the X11 display `name` (`DISPLAY` when `None`). The key-grab thread
/// reports no error of its own when it cannot connect, and every registration would then look
/// successful.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn open_display(name: Option<&str>) -> Result<Session, GlobalError> {
    Session::connect(name).map(|(display, _screen)| display).map_err(|e| GlobalError::NoDisplay(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::Mods;

    fn chord(text: &str) -> Chord {
        match Chord::parse(text) {
            Ok(c) => c,
            Err(e) => panic!("{text}: {e}"),
        }
    }

    fn hk(text: &str, platform: Platform) -> HotKey {
        match hotkey(chord(text), platform) {
            Ok(h) => h,
            Err(e) => panic!("{text}: {e}"),
        }
    }

    // The `code` values of the W3C UI Events KeyboardEvent code Values specification
    // (https://www.w3.org/TR/uievents-code/), sections 3.1.1 to 3.1.3, 3.2 and 3.4 to 3.5, for the
    // key at each position on a US keyboard.
    const SPEC: &[(Key, &str)] = &[
        (Key::A, "KeyA"),
        (Key::M, "KeyM"),
        (Key::Z, "KeyZ"),
        (Key::Num0, "Digit0"),
        (Key::Num5, "Digit5"),
        (Key::Num9, "Digit9"),
        (Key::Backtick, "Backquote"),
        (Key::Minus, "Minus"),
        (Key::Equals, "Equal"),
        (Key::OpenBracket, "BracketLeft"),
        (Key::CloseBracket, "BracketRight"),
        (Key::Backslash, "Backslash"),
        (Key::Semicolon, "Semicolon"),
        (Key::Quote, "Quote"),
        (Key::Comma, "Comma"),
        (Key::Period, "Period"),
        (Key::Slash, "Slash"),
        (Key::IntlBackslash, "IntlBackslash"),
        (Key::Backspace, "Backspace"),
        (Key::Enter, "Enter"),
        (Key::Space, "Space"),
        (Key::Tab, "Tab"),
        (Key::Insert, "Insert"),
        (Key::Delete, "Delete"),
        (Key::Home, "Home"),
        (Key::End, "End"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::Up, "ArrowUp"),
        (Key::Down, "ArrowDown"),
        (Key::Left, "ArrowLeft"),
        (Key::Right, "ArrowRight"),
        (Key::Escape, "Escape"),
        (Key::F1, "F1"),
        (Key::F12, "F12"),
        (Key::F24, "F24"),
    ];

    #[test]
    fn keys_map_to_the_spec_code_for_their_position() {
        for &(key, name) in SPEC {
            assert_eq!(code(key).map(|c| c.to_string()).as_deref(), Some(name), "{key:?}");
        }
    }

    #[test]
    fn every_key_with_a_position_maps_to_a_distinct_code() {
        let mut seen = std::collections::HashMap::new();
        let mut missing = Vec::new();
        for &key in Key::ALL {
            match code(key) {
                Some(c) => {
                    if let Some(other) = seen.insert(c, key) {
                        panic!("{key:?} and {other:?} both map to {c}");
                    }
                }
                None => missing.push(key),
            }
        }
        assert_eq!(
            missing,
            [Key::Plus, Key::Colon, Key::Pipe, Key::Questionmark, Key::Exclamationmark, Key::OpenCurlyBracket, Key::CloseCurlyBracket]
        );
        assert_eq!(seen.len() + missing.len(), Key::ALL.len());
    }

    #[test]
    fn modifiers_follow_the_platform() {
        assert_eq!(hk("Cmd+Shift+K", Platform::Mac).mods, Modifiers::META | Modifiers::SHIFT);
        assert_eq!(hk("Cmd+Shift+K", Platform::Windows).mods, Modifiers::CONTROL | Modifiers::SHIFT);
        assert_eq!(hk("Cmd+Shift+K", Platform::Linux).mods, Modifiers::CONTROL | Modifiers::SHIFT);
        assert_eq!(hk("Ctrl+K", Platform::Mac).mods, Modifiers::CONTROL);
        assert_eq!(hk("Cmd+Ctrl+Alt+K", Platform::Mac).mods, Modifiers::META | Modifiers::CONTROL | Modifiers::ALT);
        // Off macOS Ctrl folds into Cmd, so both chords are one hotkey.
        assert_eq!(hk("Ctrl+K", Platform::Windows), hk("Cmd+K", Platform::Windows));
        assert_ne!(hk("Ctrl+K", Platform::Mac), hk("Cmd+K", Platform::Mac));
        assert_eq!(hk("Alt+F4", Platform::Linux).key, Code::F4);
    }

    #[test]
    fn typing_keys_need_a_command_modifier() {
        for text in ["A", "Shift+A", "Space", "Shift+Up", "Enter", "5", "Escape"] {
            assert_eq!(hotkey(chord(text), Platform::Windows), Err(GlobalError::TypingKey(chord(text))), "{text}");
        }
        for text in ["F9", "Shift+F13", "F35", "Alt+A", "Ctrl+Space", "Cmd+5"] {
            assert!(hotkey(chord(text), Platform::Mac).is_ok(), "{text}");
        }
        let plus = Chord { mods: Mods::CMD, key: Key::Plus };
        assert_eq!(hotkey(plus, Platform::Linux), Err(GlobalError::Unbindable(plus)));
    }

    #[test]
    fn windows_refuses_ctrl_alt_on_keys_altgr_types_with() {
        for text in ["Ctrl+Alt+E", "Cmd+Alt+Q", "Ctrl+Alt+Shift+7", "Ctrl+Alt+[", "Ctrl+Alt+-", "Ctrl+Alt+Space"] {
            assert_eq!(hotkey(chord(text), Platform::Windows), Err(GlobalError::AltGr(chord(text))), "{text}");
        }
        // Keys that type nothing, and the same chords where AltGr is not Ctrl+Alt.
        for text in ["Ctrl+Alt+F9", "Ctrl+Alt+Home", "Ctrl+Alt+Enter", "Ctrl+Shift+E", "Alt+E"] {
            assert!(hotkey(chord(text), Platform::Windows).is_ok(), "{text}");
        }
        for platform in [Platform::Mac, Platform::Linux] {
            assert!(hotkey(chord("Ctrl+Alt+E"), platform).is_ok(), "{platform:?}");
        }
    }

    fn entry(text: &str, command: &str) -> Entry {
        let hotkey = hk(text, Platform::Windows);
        Entry { hotkey, chord: chord(text), command: command.into(), down: false, slot: u64::from(hotkey.id()) }
    }

    fn event(entry: &Entry, state: HotKeyState) -> GlobalHotKeyEvent {
        GlobalHotKeyEvent { id: entry.hotkey.id(), state }
    }

    #[test]
    fn a_hold_gives_one_press_and_one_release() {
        let mut entries = vec![entry("Ctrl+F9", "record.toggle"), entry("Ctrl+Shift+M", "mic.mute")];
        let record = event(&entries[0], HotKeyState::Pressed);
        let record_up = event(&entries[0], HotKeyState::Released);
        let mute = event(&entries[1], HotKeyState::Pressed);
        let states: Vec<_> = [record_up, record, record, mute, record_up, record_up]
            .into_iter()
            .filter_map(|e| translate(&mut entries, e))
            .map(|e| (e.command.key(), e.state))
            .collect();
        assert_eq!(
            states,
            [
                ("record.toggle".to_string(), KeyState::Pressed),
                ("mic.mute".to_string(), KeyState::Pressed),
                ("record.toggle".to_string(), KeyState::Released),
            ]
        );
    }

    #[test]
    fn replace_keeps_bindings_that_do_not_change() {
        let entries = vec![entry("Ctrl+F9", "record.toggle"), entry("Ctrl+F10", "mic.mute"), entry("Ctrl+F11", "scene.next")];
        let want = |text: &str, command: &str| (CommandRef::from(command), chord(text), u64::from(hk(text, Platform::Windows).id()));
        let wanted = vec![
            want("Ctrl+F11", "scene.next"),
            // Same chord, another command: registered again.
            want("Ctrl+F10", "scene.prev"),
            want("Ctrl+F9", "record.toggle"),
            // A second copy is not kept, so registering it reports the duplicate.
            want("Ctrl+F9", "record.toggle"),
        ];
        assert_eq!(kept(&entries, &wanted), [Some(2), None, Some(0)]);
        assert_eq!(kept(&entries, &[]), [None, None, None]);
    }

    #[test]
    fn a_refused_chord_leaves_its_room_to_the_next() {
        let candidates: Vec<usize> = (0..10).collect();
        // Room for 4: the first round refuses 1 and 3, so a second round takes 4 and 5.
        let mut rounds = Vec::new();
        let left = in_rounds(&candidates, 4, |round| {
            rounds.push(round.to_vec());
            round.iter().filter(|&&i| i != 1 && i != 3).count()
        });
        assert_eq!(rounds, [vec![0, 1, 2, 3], vec![4, 5]]);
        assert_eq!(left, [6, 7, 8, 9]);
        // No room: nothing is registered.
        assert_eq!(in_rounds(&candidates, 0, |_| panic!("registered with no room")), &candidates[..]);
        // Every chord refused: each is tried once.
        let mut tried = 0;
        let left = in_rounds(&candidates, 4, |round| {
            tried += round.len();
            0
        });
        assert!(left.is_empty());
        assert_eq!(tried, 10);
    }

    #[test]
    fn a_second_copy_of_a_refused_chord_gets_its_refusal() {
        let f9 = entry("Ctrl+F9", "record.toggle");
        let id = f9.slot;
        let taken = vec![Err(GlobalError::Taken(chord("Ctrl+F9"))), Err(GlobalError::Duplicate(chord("Ctrl+F9")))];
        assert_eq!(first_refusal(&taken, &[id, id], id), Some(GlobalError::Taken(chord("Ctrl+F9"))));
        assert_eq!(GlobalError::Taken(chord("Ctrl+F9")).for_chord(chord("Ctrl+F10")), GlobalError::Taken(chord("Ctrl+F10")));
        // When the first copy registered, the second is a duplicate.
        let registered = vec![Ok(f9), Err(GlobalError::Duplicate(chord("Ctrl+F9")))];
        assert_eq!(first_refusal(&registered, &[id, id], id), None);
        assert_eq!(first_refusal(&registered, &[id, id], id + 1), None);
    }

    #[test]
    fn a_new_binding_on_a_kept_or_earlier_key_is_a_duplicate() {
        // Two chords the system grabs on one key: the kept one stays, the new ones are refused
        // before they reach the system.
        let mut six = entry("Ctrl+6", "zoom.fit");
        six.slot = 7;
        let slots = vec![Some(six), None, None, None];
        let wanted = vec![
            (CommandRef::from("zoom.fit"), chord("Ctrl+6"), 7),
            (CommandRef::from("zoom.out"), chord("Ctrl+-"), 7),
            (CommandRef::from("zoom.in"), chord("Ctrl+F7"), 8),
            (CommandRef::from("view.reset"), chord("Ctrl+F8"), 8),
        ];
        let (new, ids) = new_entries(&slots, &wanted, Platform::Windows);
        assert_eq!(ids, [7, 8, 8]);
        assert!(matches!(&new[0], Err(GlobalError::Duplicate(c)) if *c == chord("Ctrl+-")), "{:?}", new[0].as_ref().err());
        assert!(matches!(&new[1], Ok(e) if e.chord == chord("Ctrl+F7") && e.slot == 8));
        assert!(matches!(&new[2], Err(GlobalError::Duplicate(c)) if *c == chord("Ctrl+F8")));
    }

    #[test]
    fn events_for_other_ids_are_ignored() {
        let mut entries = vec![entry("Ctrl+F9", "record.toggle")];
        let other = hk("Ctrl+F10", Platform::Windows);
        assert_eq!(translate(&mut entries, GlobalHotKeyEvent { id: other.id(), state: HotKeyState::Pressed }), None);
        assert_eq!(translate(&mut [], GlobalHotKeyEvent { id: other.id(), state: HotKeyState::Released }), None);
    }

    #[test]
    fn the_queue_keeps_the_newest_events() {
        let mut queue = Queue::default();
        for id in 0..MAX_QUEUED as u32 {
            push_bounded(&mut queue, GlobalHotKeyEvent { id, state: HotKeyState::Pressed });
        }
        assert!(!queue.overflowed);
        for id in MAX_QUEUED as u32..(MAX_QUEUED as u32 + 10) {
            push_bounded(&mut queue, GlobalHotKeyEvent { id, state: HotKeyState::Pressed });
        }
        assert!(queue.overflowed);
        assert_eq!(queue.events.len(), MAX_QUEUED);
        assert_eq!(queue.events.front().map(|e| e.id), Some(10));
        assert_eq!(queue.events.back().map(|e| e.id), Some(MAX_QUEUED as u32 + 9));
    }

    #[test]
    fn an_overflow_releases_the_chords_that_were_down() {
        let mut entries = vec![entry("Ctrl+F9", "record.toggle"), entry("Ctrl+F10", "mic.mute")];
        let mut out = Vec::new();
        let press = event(&entries[0], HotKeyState::Pressed);
        drain(&mut entries, Queue { events: VecDeque::from([press]), overflowed: false }, &mut out);
        assert_eq!(out.len(), 1);
        // The release was among the dropped events; the next press of the chord still counts.
        let mute = event(&entries[1], HotKeyState::Pressed);
        drain(&mut entries, Queue { events: VecDeque::from([mute, press]), overflowed: true }, &mut out);
        let states: Vec<_> = out.iter().map(|e| (e.command.key(), e.state)).collect();
        assert_eq!(
            states,
            [
                ("record.toggle".to_string(), KeyState::Pressed),
                ("record.toggle".to_string(), KeyState::Released),
                ("mic.mute".to_string(), KeyState::Pressed),
                ("record.toggle".to_string(), KeyState::Pressed),
            ]
        );
    }

    #[test]
    fn pending_events_are_capped() {
        let tap = |n: usize, state| GlobalEvent { command: CommandRef::new(n.to_string()), chord: chord("Ctrl+F9"), state };
        let mut pending = Vec::new();
        for n in 0..MAX_QUEUED {
            push_pending(&mut pending, tap(n, KeyState::Pressed));
            push_pending(&mut pending, tap(n, KeyState::Released));
        }
        assert!(pending.len() <= MAX_QUEUED + 1, "{}", pending.len());
        // The oldest taps went first, each press with its release.
        assert_eq!(pending.last(), Some(&tap(MAX_QUEUED - 1, KeyState::Released)));
        assert_eq!(pending[0].state, KeyState::Pressed);
        assert_eq!(pending[0].command, pending[1].command);
    }

    #[test]
    fn a_full_queue_keeps_presses_and_releases_without_their_pair() {
        let talk = |state| GlobalEvent { command: "mic.talk".into(), chord: chord("Ctrl+F9"), state };
        let tap = |state| GlobalEvent { command: "scene.next".into(), chord: chord("Ctrl+F10"), state };
        // The app saw `mic.talk` go down before this queue started, and `record.toggle` is held.
        let held = GlobalEvent { command: "record.toggle".into(), chord: chord("Ctrl+F11"), state: KeyState::Pressed };
        let mut pending = vec![talk(KeyState::Released), held.clone()];
        while pending.len() < MAX_QUEUED {
            pending.push(tap(KeyState::Pressed));
            pending.push(tap(KeyState::Released));
        }
        push_pending(&mut pending, tap(KeyState::Pressed));
        assert_eq!(pending[..2], [talk(KeyState::Released), held]);
        assert_eq!(pending.len(), MAX_QUEUED - 1);
        // With no pair left to drop, the list grows instead of losing half of one.
        let mut presses: Vec<GlobalEvent> =
            (0..MAX_QUEUED).map(|n| GlobalEvent { command: CommandRef::new(n.to_string()), ..tap(KeyState::Pressed) }).collect();
        push_pending(&mut presses, tap(KeyState::Released));
        assert_eq!(presses.len(), MAX_QUEUED + 1);
        // Past `MAX_PENDING` the oldest press goes.
        while presses.len() < MAX_PENDING {
            presses.push(tap(KeyState::Released));
        }
        push_pending(&mut presses, tap(KeyState::Pressed));
        assert_eq!(presses.len(), MAX_PENDING);
        assert_eq!(presses[0].command.key(), "1");
        // Releases are never dropped.
        let mut releases = vec![tap(KeyState::Released); MAX_PENDING];
        push_pending(&mut releases, tap(KeyState::Released));
        assert_eq!(releases.len(), MAX_PENDING + 1);
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_merges_the_iso_key_with_backslash_and_refuses_quote() {
        let id = |key| match hotkey(Chord { mods: Mods::CMD, key }, Platform::Linux) {
            Ok(h) => h.id(),
            Err(e) => panic!("{key:?}: {e}"),
        };
        assert_eq!(id(Key::IntlBackslash), id(Key::Backslash));
        assert_ne!(id(Key::Slash), id(Key::Backslash));
        let quote = Chord { mods: Mods::CMD, key: Key::Quote };
        assert_eq!(hotkey(quote, Platform::Linux), Err(GlobalError::Unbindable(quote)));
    }

    /// A keyboard mapping with two keysyms (levels 1 and 2) and two AltGr keysyms per key code,
    /// starting at key code 8.
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    fn key_map(keys: &[(u8, [u32; 4])]) -> KeyMap {
        let mut keysyms = vec![0; 4 * 248];
        for &(keycode, syms) in keys {
            let at = 4 * usize::from(keycode - 8);
            keysyms[at..at + 4].copy_from_slice(&syms);
        }
        KeyMap { min_keycode: 8, per_keycode: 4, keysyms }
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    fn slot(map: &KeyMap, text: &str) -> Result<u64, GlobalError> {
        map.slot(&hk(text, Platform::Linux), chord(text))
    }

    // Keysyms from X11's keysymdef.h, at the evdev key codes of the keys in xkeyboard-config's `us`,
    // `de` and `fr` layouts.
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const PARENLEFT: u32 = 0x28;
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const UDIAERESIS: u32 = 0xfc;

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_refuses_a_key_the_layout_types_only_with_altgr() {
        // German: `[` is AltGr+8, and the key at the US `[` position types `ü`.
        let de = key_map(&[(17, [0x38, PARENLEFT, 0x5b, 0]), (34, [UDIAERESIS, 0xdc, 0, 0]), (38, [0x61, 0x41, 0, 0])]);
        assert_eq!(slot(&de, "Ctrl+["), Err(GlobalError::Unbindable(chord("Ctrl+["))));
        assert!(slot(&de, "Ctrl+8").is_ok());
        assert!(slot(&de, "Ctrl+A").is_ok());
        // US: `[` has a key of its own.
        let us = key_map(&[(17, [0x38, 0x2a, 0, 0]), (34, [0x5b, 0x7b, 0, 0])]);
        assert!(slot(&us, "Ctrl+[").is_ok());
        assert_ne!(slot(&us, "Ctrl+["), slot(&us, "Ctrl+8"));
        // A key the layout lacks is refused.
        assert_eq!(slot(&us, "Ctrl+]"), Err(GlobalError::Unbindable(chord("Ctrl+]"))));
        // French AZERTY: `.` is Shift+`;`, so only a chord with Shift reaches it.
        let fr = key_map(&[(59, [0x3b, 0x2e, 0, 0])]);
        assert!(slot(&fr, "Ctrl+Shift+.").is_ok());
        assert_eq!(slot(&fr, "Ctrl+."), Err(GlobalError::Unbindable(chord("Ctrl+."))));
        // Every key the backend can grab has a keysym here.
        for &key in Key::ALL {
            if let Ok(hotkey) = hotkey(Chord { mods: Mods::CMD, key }, Platform::Linux) {
                assert!(x11_keysym(hotkey.key).is_some(), "{key:?}");
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_chords_on_one_key_share_a_slot() {
        // French AZERTY types `-` and, with Shift, `6` on one key.
        let fr = key_map(&[(15, [0x2d, 0x36, 0, 0]), (17, [0x5f, 0x38, 0, 0])]);
        assert_eq!(slot(&fr, "Ctrl+-"), slot(&fr, "Ctrl+6"));
        assert!(slot(&fr, "Ctrl+6").is_ok());
        assert_ne!(slot(&fr, "Ctrl+6"), slot(&fr, "Ctrl+Shift+6"));
        assert_ne!(slot(&fr, "Ctrl+6"), slot(&fr, "Ctrl+8"));
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn wayland_sessions_are_recognized() {
        use std::ffi::OsStr;
        assert!(is_wayland(Some(OsStr::new("wayland-0")), None));
        assert!(is_wayland(None, Some(OsStr::new("Wayland"))));
        assert!(is_wayland(Some(OsStr::new("wayland-1")), Some(OsStr::new("x11"))));
        assert!(!is_wayland(Some(OsStr::new("")), Some(OsStr::new("x11"))));
        assert!(!is_wayland(None, Some(OsStr::new("tty"))));
        assert!(!is_wayland(None, None));
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn a_display_that_does_not_exist_is_reported() {
        assert!(matches!(open_display(Some(":4093")), Err(GlobalError::NoDisplay(_))));
        assert!(matches!(open_display(Some("not a display")), Err(GlobalError::NoDisplay(_))));
    }
}
