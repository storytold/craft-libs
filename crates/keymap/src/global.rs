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
    /// (Ctrl+8 for Ctrl+[). A symbol the layout types with Shift is refused with Shift in the chord
    /// (Ctrl+Shift+. on French AZERTY), since the keymap writes that press without Shift (Ctrl+.).
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
    /// The operating system or another application holds the chord. On X11 also a chord whose
    /// earlier grab this set could not release after a keyboard layout change moved its key.
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
    /// The entry is leaving the set, and an entry that stays may now hold its hotkey id
    /// ([`GlobalHotkeys::regrab`]): no event goes to it.
    retired: bool,
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
    /// Grabs (hotkey and slot) of entries that left the set while no key of the current layout
    /// led global-hotkey to their key code ([`KeyMap::release`]). Each later change tries again.
    stuck: Vec<(HotKey, u64)>,
    /// Hotkey ids of entries that left the set while held, whose release the system still sends
    /// ([`STALE_RELEASES`]). The first release of each id after that is dropped.
    stale: Vec<u32>,
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
            Ok(manager) => Ok(GlobalHotkeys {
                manager,
                platform: Platform::current(),
                entries: Vec::new(),
                pending: Vec::new(),
                session,
                stuck: Vec::new(),
                stale: Vec::new(),
            }),
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
        let keys = KeyMap::read(&self.session)?;
        let (hotkey, slot) = keys.grab(hotkey, chord)?;
        let _ = self.release(Some(&keys), []);
        // global-hotkey reports each event with the id of the hotkey registered: an entry on another
        // key with the same id (after a layout change) would take this chord's events.
        if self.entries.iter().any(|e| e.slot == slot || e.hotkey.id() == hotkey.id()) {
            return Err(GlobalError::Duplicate(chord));
        }
        if self.stuck.iter().any(|(stuck, _)| stuck.id() == hotkey.id()) {
            return Err(GlobalError::Taken(chord));
        }
        self.manager.register(hotkey).map_err(|e| os_error(e, chord))?;
        self.entries.push(Entry { hotkey, chord, command: command.into(), down: false, slot, retired: false });
        Ok(())
    }

    /// Unregisters `chord` and returns the command it ran. A chord that is held down gets its
    /// release from the next [`poll`](GlobalHotkeys::poll). The chord leaves the set also when
    /// the system reports an error. On X11, when the keyboard mapping cannot be read, its grab
    /// stays until a later change can release it, and the error is returned.
    pub fn unregister(&mut self, chord: &Chord) -> Result<CommandRef, GlobalError> {
        // Compared by the chord's own hotkey: the one registered may add Shift ([`KeyMap::grab`]).
        let id = |chord: Chord| hotkey(chord, self.platform).ok().map(|h| h.id());
        let Some(index) = id(*chord).and_then(|wanted| self.entries.iter().position(|e| id(e.chord) == Some(wanted))) else {
            return Err(GlobalError::NotRegistered(*chord));
        };
        let keys = KeyMap::read(&self.session);
        let grab = (self.entries[index].hotkey, self.entries[index].slot);
        let result = self.release(keys.as_ref().ok(), [grab]).map_err(|e| os_error(e, *chord)).and(keys.map(|_| ()));
        // Events the system sent until the chord was unregistered (on X11 the grab thread sends them
        // before `unregister` returns) still go to this command.
        self.flush();
        // Found again by its slot: an entry before it may have left the set ([`GlobalHotkeys::regrab`]).
        let Some(index) = self.entries.iter().position(|e| e.slot == grab.1) else {
            return Err(GlobalError::NotRegistered(*chord));
        };
        let entry = self.entries.remove(index);
        self.release_if_down(&entry);
        result.map(|()| entry.command)
    }

    /// Replaces the registered chords with `bindings`, and returns those that were refused. A
    /// binding that was already registered for the same command stays registered, so a chord held
    /// down through the change is not released. When the keyboard mapping cannot be read, every
    /// binding is refused and the set stays as it is, except that an empty `bindings` still clears
    /// it; the grabs then stay until a later change can release them.
    pub fn replace(&mut self, bindings: impl IntoIterator<Item = (CommandRef, Chord)>) -> Vec<Refused> {
        let keys = KeyMap::read(&self.session);
        let (wanted, mut refused) = match resolve(bindings.into_iter().collect(), &keys, self.platform) {
            Ok(resolved) => resolved,
            Err(refused) => return refused,
        };
        let keep = kept(&self.entries, &wanted);
        let gone: Vec<(HotKey, u64)> = self.entries.iter().zip(&keep).filter(|(_, kept)| kept.is_none()).map(|(e, _)| (e.hotkey, e.slot)).collect();
        let _ = self.release(keys.as_ref().ok(), gone);
        // Matched again: an entry may have left the set ([`GlobalHotkeys::regrab`]).
        let keep = kept(&self.entries, &wanted);
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
        let (mut new, new_ids) = new_entries(&slots, &wanted);
        for result in &mut new {
            if let Ok(entry) = result
                && self.stuck.iter().any(|(stuck, _)| stuck.id() == entry.hotkey.id())
            {
                *result = Err(GlobalError::Taken(entry.chord));
            }
        }
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
        for (slot, (command, chord, _, _)) in slots.into_iter().zip(wanted) {
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
        let keys = KeyMap::read(&self.session).ok();
        let grabs: Vec<(HotKey, u64)> = self.entries.iter().map(|e| (e.hotkey, e.slot)).collect();
        let _ = self.release(keys.as_ref(), grabs);
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

    /// Releases `grabs` (registered hotkeys and their slots) and tries again the grabs in `stuck`.
    /// With the current mapping `keys`, each grab goes through [`KeyMap::release`], and one it finds
    /// no hotkey for stays in `stuck`. global-hotkey looks each key code up again, so when the
    /// mapping has changed by the time it is read again, every grab sent stays in `stuck` too. A
    /// stuck grab whose slot an entry holds is dropped: the system let that entry grab the key, so
    /// the old grab is gone. Without a mapping (only on X11) nothing is sent and `grabs` go to
    /// `stuck`: global-hotkey would look their key codes up in a mapping that may have changed.
    /// When the mapping changed while the grabs were released, the stuck grabs are tried once more
    /// in the new mapping, and then an entry whose grab a release may have removed is grabbed
    /// again ([`GlobalHotkeys::regrab`]).
    fn release(&mut self, keys: Option<&KeyMap>, grabs: impl IntoIterator<Item = (HotKey, u64)>) -> Result<(), global_hotkey::Error> {
        let grabs: Vec<(HotKey, u64)> = grabs.into_iter().collect();
        let leaving: Vec<u64> = grabs.iter().map(|&(_, slot)| slot).collect();
        let held: Vec<u64> = self.entries.iter().map(|e| e.slot).collect();
        let (mut hotkeys, sent, stuck) = to_release(keys, std::mem::take(&mut self.stuck), grabs, &held);
        self.stuck = stuck;
        let Some(keys) = keys else { return Ok(()) };
        let result = self.unregister_many(&hotkeys);
        if sent.is_empty() {
            return result;
        }
        let now = KeyMap::read(&self.session).ok();
        self.stuck.extend(unconfirmed(sent, keys, now.as_ref()));
        let Some(now) = now.filter(|now| now != keys) else { return result };
        // The entries that stay hold their slots; those leaving may share a stuck grab's slot.
        let staying: Vec<u64> = held.into_iter().filter(|slot| !leaving.contains(slot)).collect();
        let (again, sent, stuck) = to_release(Some(&now), std::mem::take(&mut self.stuck), [], &staying);
        self.stuck = stuck;
        let mut after = None;
        if !sent.is_empty() {
            let _ = self.unregister_many(&again);
            let read = KeyMap::read(&self.session).ok();
            self.stuck.extend(unconfirmed(sent, &now, read.as_ref()));
            after = read.filter(|after| *after != now);
            hotkeys.extend(again);
        }
        // When the mapping changed again meanwhile, the releases may have been looked up in either.
        let maps: Vec<&KeyMap> = std::iter::once(&now).chain(after.as_ref()).collect();
        self.regrab(&maps, &hotkeys, &leaving);
        result
    }

    /// Grabs again the entries (other than those whose slot is in `leaving`) whose grab one of
    /// `released` leads to in one of the mappings `maps`: when global-hotkey looked a release up in
    /// that mapping, it removed the entry's grab. Each is registered through the hotkey that leads
    /// to its key in the last of `maps` ([`KeyMap::release`]), which becomes its hotkey. When the
    /// release was looked up in another mapping, the grab is still there, global-hotkey refuses the
    /// new one as registered, and the entry keeps its hotkey. global-hotkey gives the same error
    /// when another client grabbed the key in between, which this cannot tell apart. An entry that
    /// cannot be grabbed again (no key leads there, or its hotkey id is a staying entry's or a
    /// stuck grab's) leaves the set, and its grab goes to `stuck`.
    fn regrab(&mut self, maps: &[&KeyMap], released: &[HotKey], leaving: &[u64]) {
        let Some(&now) = maps.last() else { return };
        let mut hits: Vec<usize> = maps.iter().flat_map(|map| hit(&self.entries, released, leaving, map)).collect();
        hits.sort_unstable();
        hits.dedup();
        if !hits.is_empty() {
            // Events that came under the entries' hotkeys so far go to them before the ids change.
            self.flush();
            // An entry below may take the hotkey id of one leaving, whose grab is gone. Its later
            // events, until the leaving one is removed, go to it.
            retire(&mut self.entries, leaving);
        }
        for i in hits.into_iter().rev() {
            let entry = &self.entries[i];
            let free = |id: u32| {
                !self.stuck.iter().any(|(stuck, _)| stuck.id() == id)
                    && !self.entries.iter().enumerate().any(|(j, e)| j != i && !leaving.contains(&e.slot) && e.hotkey.id() == id)
            };
            let hotkey = now.release(entry.hotkey, entry.slot).filter(|hotkey| free(hotkey.id()));
            match hotkey.map(|hotkey| (hotkey, self.manager.register(hotkey))) {
                Some((hotkey, Ok(()))) => {
                    let entry = &mut self.entries[i];
                    entry.hotkey = hotkey;
                    // The removed grab sends no release of a press under the old hotkey.
                    if entry.down {
                        entry.down = false;
                        let event = GlobalEvent { command: entry.command.clone(), chord: entry.chord, state: KeyState::Released };
                        push_pending(&mut self.pending, event);
                    }
                }
                Some((_, Err(global_hotkey::Error::AlreadyRegistered(_)))) => {}
                _ => {
                    let entry = self.entries.remove(i);
                    self.stuck.push((entry.hotkey, entry.slot));
                    self.release_if_down(&entry);
                }
            }
        }
    }

    /// Unregisters `hotkeys` from the system, in one request on X11. The entries go whether or
    /// not the system confirms: the app asked for them to go. Returns the last error.
    fn unregister_many(&self, hotkeys: &[HotKey]) -> Result<(), global_hotkey::Error> {
        if hotkeys.is_empty() {
            return Ok(());
        }
        if BATCH {
            return self.manager.unregister_all(hotkeys);
        }
        let mut result = Ok(());
        for &hotkey in hotkeys {
            if let Err(error) = self.manager.unregister(hotkey) {
                result = Err(error);
            }
        }
        result
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
        drain(&mut self.entries, &mut self.stale, queue, &mut self.pending);
    }

    /// Gives `entry`, which has left the set, its release when it is held.
    fn release_if_down(&mut self, entry: &Entry) {
        if entry.down {
            push_pending(&mut self.pending, GlobalEvent { command: entry.command.clone(), chord: entry.chord, state: KeyState::Released });
            if STALE_RELEASES {
                if self.stale.len() >= MAX_GLOBAL {
                    self.stale.remove(0);
                }
                self.stale.push(entry.hotkey.id());
            }
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
/// `per_keycode` keysyms. XKB lists group 1 levels 1 and 2, then group 2 levels 1 and 2, then
/// the higher levels (AltGr) and groups; a layout with one group repeats it as group 2.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
#[derive(PartialEq)]
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

    /// The hotkey to register for `chord` (whose own hotkey is `hotkey`), and the key code and
    /// modifiers global-hotkey grabs for it as one number. It grabs the first key code whose
    /// keysyms hold the key's keysym. The chord is refused when that key types the keysym only at
    /// a third or later level (AltGr): the grab would then go to another key than the one `chord`
    /// names. A letter may also be in group 2 when the key types no letter, digit or other key a
    /// chord names in group 1. Letters and digits may be at level 2 (`A` is the
    /// shifted letter, and French AZERTY types digits with Shift). A symbol at level 2 is the
    /// press with Shift, which the keymap writes without Shift (Ctrl+. for Ctrl+Shift+; on French
    /// AZERTY): the hotkey adds Shift, and the chord is refused when it holds Shift itself. Other
    /// keys must be at level 1.
    fn grab(&self, hotkey: HotKey, chord: Chord) -> Result<(HotKey, u64), GlobalError> {
        let keysym = x11_keysym(hotkey.key).ok_or(GlobalError::Unbindable(chord))?;
        let (keycode, keysyms) = self.first_key(keysym).ok_or(GlobalError::Unbindable(chord))?;
        let level = match keysyms.iter().position(|&k| k == keysym) {
            Some(0) => 1,
            Some(1) => 2,
            // The second pair is group 2 of a layout with two groups (US after Russian), or, in a
            // mapping set through the core protocol, the levels Mode_switch selects, as AltGr
            // does. Only a Latin letter counts there, on a key that types no key a chord names at
            // levels 1 and 2 (a Cyrillic letter): the grab would otherwise take that key's chord
            // (with US and French, A is on the US Q key).
            Some(2 | 3) if chord.key.is_letter() && !keysyms.iter().take(2).any(|&k| names_a_key(k)) => 1,
            _ => return Err(GlobalError::Unbindable(chord)),
        };
        let grabbed = if level == 1 || chord.key.is_letter() || chord.key.is_digit() {
            hotkey
        } else if chord.key.is_symbol() && !hotkey.mods.contains(Modifiers::SHIFT) {
            HotKey::new(Some(hotkey.mods | Modifiers::SHIFT), hotkey.key)
        } else {
            return Err(GlobalError::Unbindable(chord));
        };
        Ok((grabbed, (keycode << 32) | u64::from(grabbed.mods.bits())))
    }

    /// The hotkey that makes global-hotkey release the grab of `hotkey` recorded as `slot`. Its
    /// `unregister` looks the key code up again in the current mapping, so when a layout change
    /// moved `hotkey`'s key it is the hotkey with the same modifiers on a key whose keysym now
    /// leads to the grabbed key code. `None` when no key global-hotkey knows leads there.
    fn release(&self, hotkey: HotKey, slot: u64) -> Option<HotKey> {
        let keycode = slot >> 32;
        let leads_there = |key: Code| x11_keysym(key).and_then(|keysym| self.first_key(keysym)).is_some_and(|(at, _)| at == keycode);
        if leads_there(hotkey.key) {
            return Some(hotkey);
        }
        Key::ALL
            .iter()
            .filter_map(|&key| code(key).and_then(backend_code))
            .find(|&key| leads_there(key))
            .map(|key| HotKey::new(Some(hotkey.mods), key))
    }

    /// The slot global-hotkey registers or releases for `hotkey` as it is: the first key code whose
    /// keysyms hold its key's keysym, and its modifiers.
    fn lands(&self, hotkey: HotKey) -> Option<u64> {
        let (keycode, _) = self.first_key(x11_keysym(hotkey.key)?)?;
        Some((keycode << 32) | u64::from(hotkey.mods.bits()))
    }

    /// The first key code whose keysyms hold `keysym`, as global-hotkey looks it up, and its keysyms.
    fn first_key(&self, keysym: u32) -> Option<(u64, &[u32])> {
        let (index, keysyms) = self.keysyms.chunks(self.per_keycode.max(1)).enumerate().find(|(_, k)| k.contains(&keysym))?;
        Some((u64::from(self.min_keycode) + index as u64, keysyms))
    }
}

/// Elsewhere the system grabs one key for each hotkey id.
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
#[derive(PartialEq)]
struct KeyMap;

#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
impl KeyMap {
    fn read(_session: &Session) -> Result<KeyMap, GlobalError> {
        Ok(KeyMap)
    }

    fn grab(&self, hotkey: HotKey, _chord: Chord) -> Result<(HotKey, u64), GlobalError> {
        Ok((hotkey, u64::from(hotkey.id())))
    }

    fn release(&self, hotkey: HotKey, _slot: u64) -> Option<HotKey> {
        Some(hotkey)
    }

    fn lands(&self, hotkey: HotKey) -> Option<u64> {
        Some(u64::from(hotkey.id()))
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

/// Whether `keysym` is the keysym of a key a chord can name, or a lowercase Latin letter.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn names_a_key(keysym: u32) -> bool {
    (0x61..=0x7a).contains(&keysym)
        || Key::ALL.iter().filter_map(|&key| code(key).and_then(backend_code)).any(|code| x11_keysym(code) == Some(keysym))
}

/// Whether hotkeys are registered in batches: global-hotkey's X11 key-grab thread handles one
/// request every 50 ms, while `RegisterHotKey` and the Carbon calls return at once.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
const BATCH: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
const BATCH: bool = false;

/// Whether the system still sends the release of a hotkey unregistered while held: global-hotkey's
/// Windows backend starts a thread at each press that sends the release once the key comes up.
const STALE_RELEASES: bool = cfg!(target_os = "windows");

/// A binding [`GlobalHotkeys::replace`] can register: its command and chord, the hotkey to
/// register and its slot ([`KeyMap::grab`]).
type Wanted = (CommandRef, Chord, HotKey, u64);

/// A registered hotkey and its slot ([`KeyMap::grab`]).
type Grab = (HotKey, u64);

/// Splits `bindings` into those `keys` can grab and the refused ones. When the keyboard mapping
/// could not be read, `Err` holds every binding as refused, and the set is to stay as it is; an
/// empty `bindings` still clears it.
fn resolve(
    bindings: Vec<(CommandRef, Chord)>,
    keys: &Result<KeyMap, GlobalError>,
    platform: Platform,
) -> Result<(Vec<Wanted>, Vec<Refused>), Vec<Refused>> {
    if let Err(error) = keys
        && !bindings.is_empty()
    {
        return Err(bindings.into_iter().map(|(command, chord)| Refused { command, chord, error: error.clone() }).collect());
    }
    let mut wanted = Vec::new();
    let mut refused = Vec::new();
    for (command, chord) in bindings {
        match hotkey(chord, platform).and_then(|hotkey| keys.as_ref().map_err(Clone::clone)?.grab(hotkey, chord)) {
            Ok((hotkey, slot)) => wanted.push((command, chord, hotkey, slot)),
            Err(error) => refused.push(Refused { command, chord, error }),
        }
    }
    Ok((wanted, refused))
}

/// The hotkeys [`GlobalHotkeys::release`] sends to release `grabs` and the grabs in `stuck`, the
/// grabs they release, and the grabs that stay stuck. A stuck grab on a slot in `held` (the slots
/// entries hold) is dropped ([`still_held`]). Without a mapping `keys`, nothing is sent.
fn to_release(keys: Option<&KeyMap>, stuck: Vec<Grab>, grabs: impl IntoIterator<Item = Grab>, held: &[u64]) -> (Vec<HotKey>, Vec<Grab>, Vec<Grab>) {
    let Some(keys) = keys else {
        let mut stuck = stuck;
        stuck.extend(grabs);
        return (Vec::new(), Vec::new(), stuck);
    };
    let mut hotkeys = Vec::new();
    let mut sent = Vec::new();
    let mut left = Vec::new();
    for (hotkey, slot) in still_held(stuck, held).into_iter().chain(grabs) {
        match keys.release(hotkey, slot) {
            Some(release) => {
                hotkeys.push(release);
                sent.push((hotkey, slot));
            }
            None => left.push((hotkey, slot)),
        }
    }
    (hotkeys, sent, left)
}

/// The grabs of `stuck` that may still be held: those whose slot is not in `held`, the slots
/// entries hold.
fn still_held(mut stuck: Vec<(HotKey, u64)>, held: &[u64]) -> Vec<(HotKey, u64)> {
    stuck.retain(|(_, slot)| !held.contains(slot));
    stuck
}

/// The grabs of `sent`, released through hotkeys looked up in `before`, that may still be held:
/// all of them when the mapping read afterwards (`after`, `None` when it could not be read)
/// differs, since global-hotkey may have looked the key codes up in another mapping.
fn unconfirmed(sent: Vec<(HotKey, u64)>, before: &KeyMap, after: Option<&KeyMap>) -> Vec<(HotKey, u64)> {
    if after == Some(before) { Vec::new() } else { sent }
}

/// The positions in `entries` of those whose slot one of the hotkeys in `released` leads to in
/// the mapping `now`, other than those whose slot is in `leaving`.
fn hit(entries: &[Entry], released: &[HotKey], leaving: &[u64], now: &KeyMap) -> Vec<usize> {
    let slots: Vec<u64> = released.iter().filter_map(|&hotkey| now.lands(hotkey)).collect();
    (0..entries.len()).filter(|&i| slots.contains(&entries[i].slot) && !leaving.contains(&entries[i].slot)).collect()
}

/// Marks the entries whose slot is in `leaving` as retired, so [`translate`] gives no event to
/// them.
fn retire(entries: &mut [Entry], leaving: &[u64]) {
    for entry in entries.iter_mut().filter(|e| leaving.contains(&e.slot)) {
        entry.retired = true;
    }
}

/// For each binding in `wanted` that has no kept entry in `slots`, a new entry, or
/// [`GlobalError::Duplicate`] when a kept entry or an earlier binding grabs the same key or
/// registers the same hotkey id. Also returns the slot of each.
fn new_entries(slots: &[Option<Entry>], wanted: &[Wanted]) -> (Vec<Result<Entry, GlobalError>>, Vec<u64>) {
    let mut taken: Vec<u64> = slots.iter().flatten().map(|e| e.slot).collect();
    let mut hotkey_ids: Vec<u32> = slots.iter().flatten().map(|e| e.hotkey.id()).collect();
    let mut new = Vec::new();
    let mut new_ids = Vec::new();
    for (kept, (command, chord, hotkey, slot)) in slots.iter().zip(wanted) {
        if kept.is_some() {
            continue;
        }
        let result = if taken.contains(slot) || hotkey_ids.contains(&hotkey.id()) {
            Err(GlobalError::Duplicate(*chord))
        } else {
            taken.push(*slot);
            hotkey_ids.push(hotkey.id());
            Ok(Entry { hotkey: *hotkey, chord: *chord, command: command.clone(), down: false, slot: *slot, retired: false })
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
fn kept(entries: &[Entry], wanted: &[Wanted]) -> Vec<Option<usize>> {
    let mut taken = vec![false; wanted.len()];
    entries
        .iter()
        .map(|entry| {
            let slot = wanted
                .iter()
                .zip(&taken)
                .position(|((command, chord, _, id), taken)| !taken && *id == entry.slot && *chord == entry.chord && *command == entry.command)?;
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
/// chord that was down is released first, since its release may be among the dropped events, and
/// `stale` is emptied for the same reason.
fn drain(entries: &mut [Entry], stale: &mut Vec<u32>, queue: Queue, out: &mut Vec<GlobalEvent>) {
    if queue.overflowed {
        stale.clear();
        for entry in entries.iter_mut().filter(|e| e.down) {
            entry.down = false;
            push_pending(out, GlobalEvent { command: entry.command.clone(), chord: entry.chord, state: KeyState::Released });
        }
    }
    for event in queue.events {
        if let Some(event) = translate(entries, stale, event) {
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
/// releases without a press. A release whose id is in `stale` belongs to an entry that left the
/// set while held: it is dropped, so it cannot end a hold of a newer entry with the same id. A
/// retired entry gets no event ([`retire`]).
fn translate(entries: &mut [Entry], stale: &mut Vec<u32>, event: GlobalHotKeyEvent) -> Option<GlobalEvent> {
    if event.state == HotKeyState::Released
        && let Some(i) = stale.iter().position(|&id| id == event.id)
    {
        stale.remove(i);
        return None;
    }
    let entry = entries.iter_mut().find(|e| !e.retired && e.hotkey.id() == event.id)?;
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
        Entry { hotkey, chord: chord(text), command: command.into(), down: false, slot: u64::from(hotkey.id()), retired: false }
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
            .filter_map(|e| translate(&mut entries, &mut Vec::new(), e))
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
        let want = |text: &str, command: &str| {
            let hotkey = hk(text, Platform::Windows);
            (CommandRef::from(command), chord(text), hotkey, u64::from(hotkey.id()))
        };
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
        let want = |command: &str, text: &str, slot| (CommandRef::from(command), chord(text), hk(text, Platform::Windows), slot);
        let wanted =
            vec![want("zoom.fit", "Ctrl+6", 7), want("zoom.out", "Ctrl+-", 7), want("zoom.in", "Ctrl+F7", 8), want("view.reset", "Ctrl+F8", 8)];
        let (new, ids) = new_entries(&slots, &wanted);
        assert_eq!(ids, [7, 8, 8]);
        assert!(matches!(&new[0], Err(GlobalError::Duplicate(c)) if *c == chord("Ctrl+-")), "{:?}", new[0].as_ref().err());
        assert!(matches!(&new[1], Ok(e) if e.chord == chord("Ctrl+F7") && e.slot == 8));
        assert!(matches!(&new[2], Err(GlobalError::Duplicate(c)) if *c == chord("Ctrl+F8")));
    }

    #[test]
    fn a_new_binding_with_the_hotkey_id_of_another_is_a_duplicate() {
        // Ctrl+. was grabbed as Ctrl+Shift+. on one key; after a layout change Ctrl+Shift+. is the
        // same hotkey on another key, and events carry only the id.
        let mut dot = entry("Ctrl+Shift+.", "zoom.in");
        dot.chord = chord("Ctrl+.");
        dot.slot = 59;
        let shift_dot = dot.hotkey;
        let slots = vec![Some(dot), None, None, None, None];
        let want = |command: &str, text: &str, slot| (CommandRef::from(command), chord(text), hk(text, Platform::Windows), slot);
        let wanted = vec![
            (CommandRef::from("zoom.in"), chord("Ctrl+."), shift_dot, 59),
            want("view.big", "Ctrl+Shift+.", 60),
            want("zoom.in", "Ctrl+F7", 61),
            want("zoom.out", "Ctrl+F8", 62),
            want("view.reset", "Ctrl+F8", 63),
        ];
        let (new, ids) = new_entries(&slots, &wanted);
        assert_eq!(ids, [60, 61, 62, 63]);
        assert!(matches!(&new[0], Err(GlobalError::Duplicate(c)) if *c == chord("Ctrl+Shift+.")), "{:?}", new[0].as_ref().err());
        assert!(matches!(&new[1], Ok(e) if e.slot == 61));
        assert!(matches!(&new[2], Ok(e) if e.slot == 62));
        assert!(matches!(&new[3], Err(GlobalError::Duplicate(c)) if *c == chord("Ctrl+F8")));
    }

    #[test]
    fn without_a_keyboard_mapping_grabs_stay_stuck_and_nothing_is_sent() {
        let f9 = entry("Ctrl+F9", "record.toggle");
        let f10 = entry("Ctrl+F10", "mic.mute");
        let stuck = vec![(f10.hotkey, f10.slot)];
        let (hotkeys, sent, left) = to_release(None, stuck, [(f9.hotkey, f9.slot)], &[]);
        assert!(hotkeys.is_empty());
        assert!(sent.is_empty());
        assert_eq!(left, [(f10.hotkey, f10.slot), (f9.hotkey, f9.slot)]);
    }

    #[test]
    fn events_for_other_ids_are_ignored() {
        let mut entries = vec![entry("Ctrl+F9", "record.toggle")];
        let other = hk("Ctrl+F10", Platform::Windows);
        assert_eq!(translate(&mut entries, &mut Vec::new(), GlobalHotKeyEvent { id: other.id(), state: HotKeyState::Pressed }), None);
        assert_eq!(translate(&mut [], &mut Vec::new(), GlobalHotKeyEvent { id: other.id(), state: HotKeyState::Released }), None);
    }

    #[test]
    fn a_release_of_an_entry_that_left_while_held_does_not_end_a_newer_hold() {
        // `Ctrl+F9` was held when it moved to another command; the old press's release comes late.
        let mut entries = vec![entry("Ctrl+F9", "mic.talk")];
        let mut stale = vec![entries[0].hotkey.id()];
        let press = event(&entries[0], HotKeyState::Pressed);
        let release = event(&entries[0], HotKeyState::Released);
        let states: Vec<_> = [press, release, release]
            .into_iter()
            .filter_map(|e| translate(&mut entries, &mut stale, e))
            .map(|e| (e.command.key(), e.state))
            .collect();
        assert_eq!(states, [("mic.talk".to_string(), KeyState::Pressed), ("mic.talk".to_string(), KeyState::Released)]);
        assert!(stale.is_empty());
        // A stale release that comes before the next press is dropped as well, and the press counts.
        let mut stale = vec![entries[0].hotkey.id()];
        let states: Vec<_> = [release, press].into_iter().filter_map(|e| translate(&mut entries, &mut stale, e)).map(|e| e.state).collect();
        assert_eq!(states, [KeyState::Pressed]);
        assert!(stale.is_empty());
        // After an overflow nothing is known of the late releases.
        let mut stale = vec![entries[0].hotkey.id()];
        drain(&mut entries, &mut stale, Queue { events: VecDeque::new(), overflowed: true }, &mut Vec::new());
        assert!(stale.is_empty());
    }

    #[test]
    fn an_entry_grabbed_again_under_the_hotkey_id_of_one_leaving_gets_its_events() {
        // Ctrl+Q leaves the set; Ctrl+A stays and is grabbed again through Ctrl+Q's hotkey after a
        // layout change, while the Ctrl+Q entry is still in the set until its removal.
        let quit = entry("Ctrl+Q", "app.quit");
        let mut select = entry("Ctrl+A", "select.all");
        select.hotkey = quit.hotkey;
        let leaving = [quit.slot];
        let mut entries = vec![quit, select];
        retire(&mut entries, &leaving);
        assert!(entries[0].retired);
        assert!(!entries[1].retired);
        let press = event(&entries[1], HotKeyState::Pressed);
        let release = event(&entries[1], HotKeyState::Released);
        let mut out = Vec::new();
        drain(&mut entries, &mut Vec::new(), Queue { events: VecDeque::from([press, release]), overflowed: false }, &mut out);
        let states: Vec<_> = out.iter().map(|e| (e.command.key(), e.state)).collect();
        assert_eq!(states, [("select.all".to_string(), KeyState::Pressed), ("select.all".to_string(), KeyState::Released)]);
        assert!(!entries[0].down);
    }

    #[test]
    fn replace_leaves_the_set_alone_when_the_keyboard_mapping_cannot_be_read() {
        let failed: Result<KeyMap, GlobalError> = Err(GlobalError::Os("cannot read the keyboard mapping".to_string()));
        let bindings = vec![(CommandRef::from("record.toggle"), chord("Ctrl+F9")), (CommandRef::from("mic.mute"), chord("Ctrl+F10"))];
        match resolve(bindings, &failed, Platform::Linux) {
            Err(refused) => {
                let chords: Vec<_> = refused.iter().map(|r| (r.chord, r.error.clone())).collect();
                let error = GlobalError::Os("cannot read the keyboard mapping".to_string());
                assert_eq!(chords, [(chord("Ctrl+F9"), error.clone()), (chord("Ctrl+F10"), error)]);
            }
            Ok(_) => panic!("bindings were resolved without a keyboard mapping"),
        }
        // An empty replacement clears the set without the mapping.
        assert!(matches!(resolve(Vec::new(), &failed, Platform::Linux), Ok((wanted, refused)) if wanted.is_empty() && refused.is_empty()));
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
        drain(&mut entries, &mut Vec::new(), Queue { events: VecDeque::from([press]), overflowed: false }, &mut out);
        assert_eq!(out.len(), 1);
        // The release was among the dropped events; the next press of the chord still counts.
        let mute = event(&entries[1], HotKeyState::Pressed);
        drain(&mut entries, &mut Vec::new(), Queue { events: VecDeque::from([mute, press]), overflowed: true }, &mut out);
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

    /// A core keyboard mapping from key code 8 with six keysyms per key code: group 1 levels 1 and
    /// 2, group 2 levels 1 and 2, and group 1 levels 3 and 4 (AltGr). A layout with one group is
    /// given as `[l1, l2, l1, l2, l3, l4]`, as the X server repeats it.
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    fn key_map(keys: &[(u8, [u32; 6])]) -> KeyMap {
        let mut keysyms = vec![0; 6 * 248];
        for &(keycode, syms) in keys {
            let at = 6 * usize::from(keycode - 8);
            keysyms[at..at + 6].copy_from_slice(&syms);
        }
        KeyMap { min_keycode: 8, per_keycode: 6, keysyms }
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    fn grab(map: &KeyMap, text: &str) -> Result<(HotKey, u64), GlobalError> {
        map.grab(hk(text, Platform::Linux), chord(text))
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    fn slot(map: &KeyMap, text: &str) -> Result<u64, GlobalError> {
        grab(map, text).map(|(_, slot)| slot)
    }

    // Keysyms from X11's keysymdef.h, at the evdev key codes of the keys in xkeyboard-config's `us`,
    // `de`, `fr` and `ru` layouts.
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const PARENLEFT: u32 = 0x28;
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const UDIAERESIS: u32 = 0xfc;
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const TRADEMARK: u32 = 0xac9;
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const CYRILLIC_EF: [u32; 2] = [0x6c6, 0x6e6];
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    const CYRILLIC_HA: [u32; 2] = [0x6c8, 0x6e8];

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_refuses_a_key_the_layout_types_only_with_altgr() {
        // German: `[` is AltGr+8, and the key at the US `[` position types `ü`.
        let de = key_map(&[
            (17, [0x38, PARENLEFT, 0x38, PARENLEFT, 0x5b, TRADEMARK]),
            (34, [UDIAERESIS, 0xdc, UDIAERESIS, 0xdc, 0, 0]),
            (38, [0x61, 0x41, 0x61, 0x41, 0, 0]),
        ]);
        assert_eq!(slot(&de, "Ctrl+["), Err(GlobalError::Unbindable(chord("Ctrl+["))));
        assert!(slot(&de, "Ctrl+8").is_ok());
        assert!(slot(&de, "Ctrl+A").is_ok());
        // US: `[` has a key of its own.
        let us = key_map(&[(17, [0x38, 0x2a, 0x38, 0x2a, 0, 0]), (34, [0x5b, 0x7b, 0x5b, 0x7b, 0, 0])]);
        assert!(slot(&us, "Ctrl+[").is_ok());
        assert_ne!(slot(&us, "Ctrl+["), slot(&us, "Ctrl+8"));
        // A key the layout lacks is refused.
        assert_eq!(slot(&us, "Ctrl+]"), Err(GlobalError::Unbindable(chord("Ctrl+]"))));
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
        let fr = key_map(&[(15, [0x2d, 0x36, 0x2d, 0x36, 0, 0]), (17, [0x5f, 0x38, 0x5f, 0x38, 0, 0])]);
        assert_eq!(slot(&fr, "Ctrl+-"), slot(&fr, "Ctrl+6"));
        assert!(slot(&fr, "Ctrl+6").is_ok());
        assert_ne!(slot(&fr, "Ctrl+6"), slot(&fr, "Ctrl+Shift+6"));
        assert_ne!(slot(&fr, "Ctrl+6"), slot(&fr, "Ctrl+8"));
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_grabs_a_symbol_typed_with_shift_as_the_keymap_writes_it() {
        // French AZERTY types `;` and, with Shift, `.` on one key. The keymap writes that press
        // with Shift as Ctrl+., so the grab adds Shift, and Ctrl+Shift+. never matches it.
        let fr = key_map(&[(59, [0x3b, 0x2e, 0x3b, 0x2e, 0, 0])]);
        match grab(&fr, "Ctrl+.") {
            Ok((hotkey, _)) => assert_eq!(hotkey.mods, Modifiers::CONTROL | Modifiers::SHIFT),
            Err(e) => panic!("Ctrl+.: {e}"),
        }
        assert_eq!(slot(&fr, "Ctrl+Shift+."), Err(GlobalError::Unbindable(chord("Ctrl+Shift+."))));
        assert_eq!(slot(&fr, "Ctrl+."), slot(&fr, "Ctrl+Shift+;"));
        assert_ne!(slot(&fr, "Ctrl+."), slot(&fr, "Ctrl+;"));
        // A symbol at level 1 keeps the chord's own modifiers, Shift included.
        assert_eq!(grab(&fr, "Ctrl+Shift+;").map(|(h, _)| h), Ok(hk("Ctrl+Shift+;", Platform::Linux)));
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_reads_the_second_group_as_its_own_layout() {
        // Russian as group 1 and US as group 2: the Latin letters are the second pair.
        let ru_us = key_map(&[(34, [CYRILLIC_HA[0], CYRILLIC_HA[1], 0x5b, 0x7b, 0, 0]), (38, [CYRILLIC_EF[0], CYRILLIC_EF[1], 0x61, 0x41, 0, 0])]);
        assert_eq!(slot(&ru_us, "Ctrl+A").map(|s| s >> 32), Ok(38));
        // A symbol there may be one Mode_switch types, as AltGr does, so it is refused.
        assert_eq!(slot(&ru_us, "Ctrl+["), Err(GlobalError::Unbindable(chord("Ctrl+["))));
        let mode_switch = key_map(&[(17, [0x38, PARENLEFT, 0x5b, 0, 0, 0])]);
        assert_eq!(slot(&mode_switch, "Ctrl+["), Err(GlobalError::Unbindable(chord("Ctrl+["))));
        // The AltGr levels come after the second group.
        let de = key_map(&[(17, [0x38, PARENLEFT, 0x38, PARENLEFT, 0x5b, TRADEMARK])]);
        assert_eq!(slot(&de, "Ctrl+["), Err(GlobalError::Unbindable(chord("Ctrl+["))));
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_releases_a_grab_after_the_layout_moved_its_key() {
        // Ctrl+Q grabbed key code 24 on US. On AZERTY key 24 types A and Q is on key 38, so the
        // grab at 24 is released through Ctrl+A, which leads global-hotkey to key code 24.
        let us = key_map(&[(24, [0x71, 0x51, 0x71, 0x51, 0, 0]), (38, [0x61, 0x41, 0x61, 0x41, 0, 0])]);
        let (q, slot_us) = match grab(&us, "Ctrl+Q") {
            Ok(grab) => grab,
            Err(e) => panic!("Ctrl+Q: {e}"),
        };
        assert_eq!(us.release(q, slot_us), Some(q));
        let fr = key_map(&[(24, [0x61, 0x41, 0x61, 0x41, 0, 0]), (38, [0x71, 0x51, 0x71, 0x51, 0, 0])]);
        assert_eq!(fr.release(q, slot_us), Some(hk("Ctrl+A", Platform::Linux)));
        assert_ne!(slot(&fr, "Ctrl+Q"), Ok(slot_us));
        // With no key global-hotkey knows at key code 24, the grab cannot be released yet.
        let ru = key_map(&[(24, [0x6ca, 0x6ea, 0x6ca, 0x6ea, 0, 0])]);
        assert_eq!(ru.release(q, slot_us), None);
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_keeps_a_release_unconfirmed_when_the_mapping_changed_meanwhile() {
        let us = || key_map(&[(24, [0x71, 0x51, 0x71, 0x51, 0, 0]), (38, [0x61, 0x41, 0x61, 0x41, 0, 0])]);
        let fr = key_map(&[(24, [0x61, 0x41, 0x61, 0x41, 0, 0]), (38, [0x71, 0x51, 0x71, 0x51, 0, 0])]);
        let q = hk("Ctrl+Q", Platform::Linux);
        let sent = vec![(q, (24 << 32) | u64::from(q.mods.bits()))];
        assert!(unconfirmed(sent.clone(), &us(), Some(&us())).is_empty());
        assert_eq!(unconfirmed(sent.clone(), &us(), Some(&fr)), sent);
        assert_eq!(unconfirmed(sent.clone(), &us(), None), sent);
        // A stuck grab whose slot an entry now holds is gone: the system let the entry grab it.
        let mut held = entry("Ctrl+A", "select.all");
        held.slot = sent[0].1;
        assert!(still_held(sent.clone(), &[held.slot]).is_empty());
        assert_eq!(still_held(sent.clone(), &[entry("Ctrl+A", "select.all").slot]), sent);
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_grabs_again_an_entry_a_release_looked_up_in_a_new_mapping_removed() {
        // Ctrl+Q at key code 24 and Ctrl+A at 38 were grabbed on US. Ctrl+Q is released through
        // Ctrl+Q, but by then AZERTY puts Q on key 38: the release removes Ctrl+A's grab.
        let us = key_map(&[(24, [0x71, 0x51, 0x71, 0x51, 0, 0]), (38, [0x61, 0x41, 0x61, 0x41, 0, 0])]);
        let fr = key_map(&[(24, [0x61, 0x41, 0x61, 0x41, 0, 0]), (38, [0x71, 0x51, 0x71, 0x51, 0, 0])]);
        let (q, q_slot) = match grab(&us, "Ctrl+Q") {
            Ok(grab) => grab,
            Err(e) => panic!("Ctrl+Q: {e}"),
        };
        let (a, a_slot) = match grab(&us, "Ctrl+A") {
            Ok(grab) => grab,
            Err(e) => panic!("Ctrl+A: {e}"),
        };
        let mut select = entry("Ctrl+A", "select.all");
        (select.hotkey, select.slot) = (a, a_slot);
        let mut other = entry("Ctrl+F9", "record.toggle");
        other.slot = (70 << 32) | u64::from(other.hotkey.mods.bits());
        let entries = [other, select];
        assert_eq!(us.release(q, q_slot), Some(q));
        assert!(hit(&entries, &[q], &[q_slot], &us).is_empty());
        assert_eq!(hit(&entries, &[q], &[q_slot], &fr), [1]);
        // An entry that leaves the set itself is not grabbed again.
        assert!(hit(&entries, &[q], &[q_slot, a_slot], &fr).is_empty());
        // Ctrl+Q's own grab at 24 is then stuck; tried again on AZERTY, it goes through Ctrl+A. The
        // slot of the leaving Ctrl+Q entry does not count as held.
        let (again, retried, left) = to_release(Some(&fr), vec![(q, q_slot)], [], &[a_slot]);
        assert_eq!((again, retried), (vec![a], vec![(q, q_slot)]));
        assert!(left.is_empty());
        // It is grabbed again through the key that leads to key code 38 on AZERTY.
        assert_eq!(fr.release(a, a_slot), Some(q));
    }

    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn x11_refuses_a_second_group_letter_on_a_key_another_chord_names() {
        // US as group 1 and French as group 2: A is first found on the US Q key, so a grab would
        // take Ctrl+Q.
        let us_fr = key_map(&[(24, [0x71, 0x51, 0x61, 0x41, 0, 0]), (38, [0x61, 0x41, 0x71, 0x51, 0, 0])]);
        assert_eq!(slot(&us_fr, "Ctrl+A"), Err(GlobalError::Unbindable(chord("Ctrl+A"))));
        assert_eq!(slot(&us_fr, "Ctrl+Q").map(|s| s >> 32), Ok(24));
        // A core mapping with A at the Mode_switch level of the 8 key.
        let mode_switch = key_map(&[(17, [0x38, PARENLEFT, 0x41, 0, 0, 0])]);
        assert_eq!(slot(&mode_switch, "Ctrl+A"), Err(GlobalError::Unbindable(chord("Ctrl+A"))));
        // A key without a symbol in group 1 still takes the letter.
        let empty = key_map(&[(38, [0, 0, 0x61, 0x41, 0, 0])]);
        assert_eq!(slot(&empty, "Ctrl+A").map(|s| s >> 32), Ok(38));
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
