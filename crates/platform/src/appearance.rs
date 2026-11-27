//! Desktop colour, contrast, motion and text-scale preferences and the window-button layout through
//! the XDG Settings portal and the desktop's own settings, the system's animation setting on Windows
//! and macOS, plus the browser's contrast and motion media queries on the web.
//!
//! Derived from PdfCraft's activation-safe subscription and GNOME schema fallback,
//! CADCraft's BSD support, and ArtCraft Launcher's ReadOne/Read compatibility.
//! Palettes, manual choices and the fallback when no preference exists belong to the host.
mod reader;
pub use craft_host::caption_layout::{LayoutReading, LayoutSource, RawLayout};
pub use reader::{Reading, SystemReader};
use std::{
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

/// Desktop color preference observed independently of the application palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// The desktop requests a dark appearance.
    Dark,
    /// The desktop requests a light appearance.
    Light,
}

/// Desktop contrast preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contrast {
    /// The desktop has no contrast preference.
    Standard,
    /// The desktop requests higher contrast.
    High,
}

struct Shared {
    value: AtomicU8,
    /// 0 unknown, 1 standard, 2 high.
    contrast: AtomicU8,
    /// 0 unknown, 1 animations allowed, 2 reduced motion.
    motion: AtomicU8,
    /// The text scale's `f32` bits; 0 unknown (no usable scale is 0.0).
    text_scale: AtomicU32,
    /// Raised each time the theme, contrast, reduced motion or text scale changes; layout and icon
    /// changes leave it.
    generation: AtomicU64,
    active: AtomicBool,
    /// True once the source has answered, found no answer, or been cancelled.
    settled: Mutex<bool>,
    settled_changed: Condvar,
    /// GNOME's `button-layout` as the desktop reported it; `None` until an answer arrives.
    layout: Mutex<Option<String>>,
    /// The window-button layout each source gave, at its `LayoutSource::rank`; the best one present decides.
    captions: Mutex<[Option<LayoutReading>; craft_host::caption_layout::LAYOUT_SOURCES]>,
    /// The desktop whose settings place the window buttons, found once (`caption::watch`) for every source, the portal's
    /// keys included; unset until then.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    layout_desktop: std::sync::OnceLock<craft_host::caption_layout::LayoutDesktop>,
    /// Closed once `layout_desktop` is set: it wakes the portal's reads of the layout keys.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    found: (async_channel::Sender<()>, async_channel::Receiver<()>),
    /// Closed when the window-button layout is first asked for (`Watcher::caption_layout`). Until then no source of it
    /// runs: no X11 connection, no file poll, no `gsettings` helper and no portal read of its keys.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    wanted: (async_channel::Sender<()>, async_channel::Receiver<()>),
    /// Wakes the configuration files' poll early, when the app becomes active.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    nudge: (async_channel::Sender<()>, async_channel::Receiver<()>),
    /// Set once the portal has been asked for the layout keys (or found absent), so `gsettings` can leave the window
    /// manager's key to it.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    captions_asked: AtomicBool,
    /// GNOME's `icon-theme` name as the desktop reported it; `None` until an answer arrives.
    icons: Mutex<Option<String>>,
    /// `XDG_CURRENT_DESKTOP` names GNOME; read once when the watcher starts.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    gnome: bool,
    #[cfg(not(target_arch = "wasm32"))]
    wake: Box<dyn Fn() + Send + Sync>,
}
impl Shared {
    #[cfg(not(target_arch = "wasm32"))]
    fn new(active: bool, wake: Box<dyn Fn() + Send + Sync>) -> Self {
        Self {
            value: AtomicU8::new(0),
            contrast: AtomicU8::new(0),
            motion: AtomicU8::new(0),
            text_scale: AtomicU32::new(0),
            generation: AtomicU64::new(0),
            active: AtomicBool::new(active),
            settled: Mutex::new(false),
            settled_changed: Condvar::new(),
            layout: Mutex::new(None),
            captions: Mutex::new(Default::default()),
            #[cfg(all(unix, not(target_os = "macos")))]
            layout_desktop: std::sync::OnceLock::new(),
            #[cfg(all(unix, not(target_os = "macos")))]
            found: async_channel::bounded(1),
            #[cfg(all(unix, not(target_os = "macos")))]
            wanted: async_channel::bounded(1),
            #[cfg(all(unix, not(target_os = "macos")))]
            nudge: async_channel::bounded(1),
            #[cfg(all(unix, not(target_os = "macos")))]
            captions_asked: AtomicBool::new(false),
            icons: Mutex::new(None),
            #[cfg(all(unix, not(target_os = "macos")))]
            gnome: is_gnome_session(&|k| std::env::var(k).ok()),
            wake,
        }
    }
    fn get(&self) -> Option<Theme> {
        match self.value.load(Ordering::Acquire) {
            1 => Some(Theme::Dark),
            2 => Some(Theme::Light),
            _ => None,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn settle(&self) {
        *self.settled.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.settled_changed.notify_all();
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish(&self, theme: Option<Theme>) {
        let code = match theme {
            Some(Theme::Dark) => 1,
            Some(Theme::Light) => 2,
            None => 0,
        };
        let changed = self.value.swap(code, Ordering::AcqRel) != code;
        if changed {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        self.settle();
        if changed && self.active.load(Ordering::Acquire) {
            (self.wake)();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_code(&self, slot: &AtomicU8, code: u8) {
        if slot.swap(code, Ordering::AcqRel) != code {
            self.changed();
        }
    }
    /// Raises the generation for a value just stored, then wakes an active host.
    #[cfg(not(target_arch = "wasm32"))]
    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if self.active.load(Ordering::Acquire) {
            (self.wake)();
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_contrast(&self, contrast: Contrast) {
        self.publish_code(
            &self.contrast,
            match contrast {
                Contrast::Standard => 1,
                Contrast::High => 2,
            },
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_motion(&self, reduced: bool) {
        self.publish_code(&self.motion, if reduced { 2 } else { 1 });
    }
    /// `None` is no scaling: the desktop's value is out of range, or there is none.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_text_scale(&self, scale: Option<f32>) {
        let bits = scale.map_or(0, f32::to_bits);
        if self.text_scale.swap(bits, Ordering::AcqRel) != bits {
            self.changed();
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_layout(&self, layout: String) {
        self.publish_text(&self.layout, layout);
    }
    /// Takes `source`'s layout (`None`: it has none, or no longer has one) and wakes an active host when the best
    /// reading changed.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_caption(&self, source: LayoutSource, reading: Option<LayoutReading>) {
        let mut captions = self.captions.lock().unwrap_or_else(PoisonError::into_inner);
        let best = |captions: &[Option<LayoutReading>]| captions.iter().flatten().next().cloned();
        let before = best(&*captions);
        captions[source.rank()] = reading;
        let after = best(&*captions);
        drop(captions);
        if before != after {
            match &after {
                Some(reading) => log::debug!("window buttons: {:?} from {} ({:?})", reading.raw, reading.origin, reading.source),
                None => log::debug!("window buttons: no source answers; the desktop's default applies"),
            }
            if self.active.load(Ordering::Acquire) {
                (self.wake)();
            }
        }
    }
    /// Whether `source` holds a layout now.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn has_caption(&self, source: LayoutSource) -> bool {
        self.captions.lock().unwrap_or_else(PoisonError::into_inner)[source.rank()].is_some()
    }
    /// Asks for the window-button layout: its sources start looking from now on.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn want_captions(&self) {
        self.wanted.0.close();
    }
    /// Whether the window-button layout has been asked for.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn captions_wanted(&self) -> bool {
        self.wanted.0.is_closed()
    }
    /// Waits until the window-button layout is asked for.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    async fn until_captions_wanted(&self) {
        let _ = self.wanted.1.recv().await;
    }
    /// Sets the desktop whose settings place the window buttons, for every source; the first one set stays. The layout
    /// counts as asked for from then on.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn find_desktop(&self, desktop: craft_host::caption_layout::LayoutDesktop) {
        let _ = self.layout_desktop.set(desktop);
        self.wanted.0.close();
        self.found.0.close();
    }
    /// The desktop found for the layout; `None` before.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn layout_desktop(&self) -> Option<craft_host::caption_layout::LayoutDesktop> {
        self.layout_desktop.get().copied()
    }
    /// Waits until the desktop is found for the layout.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    async fn until_desktop_found(&self) {
        let _ = self.found.1.recv().await;
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_icons(&self, icons: String) {
        // The GNOME key holds a value on every desktop that has the schema (`gsettings` answers
        // "Adwaita" on KDE), but it is the user's setting only on GNOME-like desktops.
        if self.gnome {
            self.publish_text(&self.icons, icons);
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_text(&self, slot: &Mutex<Option<String>>, text: String) {
        let mut current = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if current.as_deref() == Some(text.as_str()) {
            return;
        }
        *current = Some(text);
        drop(current);
        if self.active.load(Ordering::Acquire) {
            (self.wake)();
        }
    }
}
/// An owned subscription. Dropping cancels connection setup, reads, backoff and helper capture,
/// and waits at most 100 ms for acknowledgement. Only a finished worker is joined: an OS call
/// or caller wake callback can block synchronously, so an exceptional unfinished worker is
/// detached and retains its resources until it returns (possibly until process exit).
/// Cache reads never block the UI; callers should keep wake callbacks nonblocking.
pub struct Watcher {
    shared: Arc<Shared>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Worker,
}
/// The thread that feeds a [`Watcher`], and how to stop it.
#[cfg(not(target_arch = "wasm32"))]
enum Worker {
    /// Linux and the BSDs: the portal subscription, raced against `stop` on its own executor.
    #[cfg(all(unix, not(target_os = "macos")))]
    Portal { stop: async_channel::Sender<()>, worker: Option<std::thread::JoinHandle<()>>, finished: std::sync::mpsc::Receiver<()> },
    /// Windows and macOS: the polling loop of `native`, which waits on `stop` between reads.
    #[cfg(any(windows, target_os = "macos"))]
    Native { stop: std::sync::mpsc::Sender<()>, finished: std::sync::mpsc::Receiver<()>, thread: Option<std::thread::JoinHandle<()>> },
}
impl Watcher {
    /// Start an active subscription, or return `None` on unsupported targets or worker failure.
    /// `wake` runs on the worker and must not block.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        Self::start_active(true, wake)
    }
    /// Start observing with explicit control over wake notifications.
    /// An inactive subscription still updates its cache. Unsupported targets and worker
    /// startup failure return `None`; browser targets never start a desktop watcher.
    ///
    /// Linux and the BSDs subscribe to the XDG Settings portal. Windows and macOS start a worker
    /// that reads the system's animation setting once a second and answers
    /// [`reduced_motion`](Self::reduced_motion) only: theme and contrast stay `None` there.
    pub fn start_active(active: bool, wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        {
            // One worker for every source: the portal's subscription and the desktop's own settings.
            Self::spawn(active, wake, |shared| async move {
                futures_lite::future::zip(portal::watch(Arc::clone(&shared)), caption::watch(shared)).await;
            })
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            let shared = Arc::new(Shared::new(active, Box::new(wake)));
            let (stop, finished, thread) = native::spawn(Arc::clone(&shared))?;
            Some(Self { shared, worker: Worker::Native { stop, finished, thread: Some(thread) } })
        }
        #[cfg(not(any(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")), windows, target_os = "macos")))]
        {
            let _ = (active, wake);
            None
        }
    }
    /// Read the cached preference without blocking; `None` means no known preference.
    pub fn current(&self) -> Option<Theme> {
        self.shared.get()
    }
    /// Rises each time the theme, [`Watcher::contrast`], [`Watcher::reduced_motion`] or
    /// [`Watcher::text_scale`] changes, before the wake callback runs; a layout or icon theme
    /// change leaves it. Read the preferences again when it moved, after reading this: a value is
    /// stored before the generation rises.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }
    /// GNOME's `org.gnome.desktop.wm.preferences` `button-layout` as the desktop reports it, such
    /// as `appmenu:minimize,close`; an empty string is an answer too. `None` until the portal or
    /// the `gsettings` fallback has answered, and on targets without a desktop watcher.
    ///
    /// It comes over the same connection as the theme, and a change runs the same wake callback.
    /// Only a GNOME-family window manager follows this setting; [`Watcher::caption_layout`] is the
    /// layout of every desktop, GNOME's included. [`Watcher::wait_first`] does not wait for it.
    pub fn button_layout(&self) -> Option<String> {
        self.shared.layout.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
    /// The window-button layout of the running desktop, from the best of its sources that answers, in this order
    /// (`craft_host::caption_layout::LayoutSource`):
    ///
    /// 1. the settings portal's standard `org.freedesktop.appearance` `button-placement`;
    /// 2. the window manager's own setting: GNOME's, Cinnamon's or MATE's `button-layout` exported by the portal, then
    ///    from `gsettings` (watched with `gsettings monitor`), or xfwm4's from Xfconf over D-Bus;
    /// 3. the window manager's configuration file: KWin's `kwinrc` (with the system's files and their locks), xfwm4's
    ///    channel file, Openbox's or labwc's `rc.xml` (Budgie's in `budgie-desktop/labwc`);
    /// 4. GTK's `Gtk/DecorationLayout` from the X settings manager (not on MATE, whose manager does not follow Marco),
    ///    then GNOME's key where the desktop keeps it in step with its window manager: KDE's GTK bridge, Xfce 4.20 and
    ///    Budgie's labwc bridge do (GTK's own apps in a Flatpak follow it there); on LXQt, Cinnamon and MATE it holds
    ///    GNOME's default and is not read;
    /// 5. GTK's `gtk-decoration-layout` in `gtk-3.0/settings.ini`.
    ///
    /// Which window manager's setting counts follows the session (`LayoutDesktop::detect`), with the running X11
    /// window manager's EWMH name on an X11 session and LXQt's chosen one, found once for every source: the portal is
    /// asked for that window manager's keys only. Each source is followed while the app runs: the portal,
    /// `gsettings monitor`, Xfconf and the X settings manager announce changes, and the files are looked at once a
    /// second (and when the app becomes active). A source that loses its answer gives way to the next; a lost connection
    /// keeps the last answer until it is back.
    ///
    /// The first call starts the sources: until then the watcher reads none of them, so an app that never asks starts
    /// no file poll, helper or X11 connection for it. The first answers arrive a moment after that call.
    ///
    /// In a Flatpak, `gsettings` reads the sandbox's own settings, not the user's, so it is not asked; system
    /// configuration folders are the runtime's, so only the user's files count; Xfconf answers only with
    /// `--talk-name=org.xfce.Xfconf`, a file only where the app was given it (`--filesystem=xdg-config/kwinrc:ro`).
    /// `None` while no source has answered, on a desktop none of them knows, and on targets without a desktop watcher:
    /// the caller then keeps the desktop's default. A change runs the wake callback; [`Watcher::wait_first`] does not
    /// wait for it.
    pub fn caption_layout(&self) -> Option<LayoutReading> {
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        self.shared.want_captions();
        self.shared.captions.lock().unwrap_or_else(PoisonError::into_inner).iter().flatten().next().cloned()
    }
    /// GNOME's `org.gnome.desktop.interface` `icon-theme` name as the desktop reports it, such as
    /// `Adwaita`. `None` until the portal or the `gsettings` fallback has answered, on targets
    /// without a desktop watcher, and always off GNOME-like desktops (`XDG_CURRENT_DESKTOP` names
    /// GNOME), where the key holds the schema's default rather than the user's choice. Other
    /// desktops keep the name in their own files; read those with
    /// `craft_window::Desktop::icon_theme`.
    ///
    /// It comes over the same connection as the theme, a change runs the same wake callback, and
    /// [`Watcher::wait_first`] does not wait for it. A name that is empty or has no theme folder is
    /// the caller's to ignore.
    pub fn icon_theme(&self) -> Option<String> {
        self.shared.icons.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
    /// The desktop's contrast preference, from the portal's `org.freedesktop.appearance`
    /// `contrast`. `None` until the portal answers, when it has no such key, and on targets
    /// without a desktop watcher.
    ///
    /// It comes over the same connection as the theme, a change runs the same wake callback, and
    /// [`Watcher::wait_first`] returns after the first read of it. Always `None` on Windows and
    /// macOS, whose watcher reads the animation setting only. On the web use `browser_contrast`.
    pub fn contrast(&self) -> Option<Contrast> {
        match self.shared.contrast.load(Ordering::Acquire) {
            1 => Some(Contrast::Standard),
            2 => Some(Contrast::High),
            _ => None,
        }
    }
    /// Whether the system asks for reduced motion.
    ///
    /// - Linux and the BSDs: three portal keys, combined so that any one asking for reduced
    ///   motion wins: `org.freedesktop.appearance` `reduced-motion` (1), GNOME's
    ///   `org.gnome.desktop.interface` `enable-animations` (false) and KDE's
    ///   `org.kde.kdeglobals.KDE` `AnimationDurationFactor` (0). With no session bus, or when the
    ///   portal has none of the three, the user's `kdeglobals` and GTK `settings.ini` files under
    ///   `$XDG_CONFIG_HOME` (or `~/.config`) answer instead, read once.
    /// - Windows: Settings › Accessibility › Visual effects › Animation effects (off), through
    ///   `UISettings`; nothing when the Windows Runtime can't be asked.
    /// - macOS: System Settings › Accessibility › Display › Reduce motion, through `NSWorkspace`.
    ///
    /// Windows and macOS are read once a second, so a change shows within a second. `None` until
    /// an answer arrives, when nothing answers, and on targets without a desktop watcher. Wake and
    /// [`Watcher::wait_first`] behave as for [`Watcher::contrast`]. On the web use
    /// `browser_reduced_motion`.
    pub fn reduced_motion(&self) -> Option<bool> {
        match self.shared.motion.load(Ordering::Acquire) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }
    /// GNOME's `org.gnome.desktop.interface` `text-scaling-factor` (Settings ▸ Accessibility ▸
    /// Large Text sets 1.25) through the portal, or the `gsettings` fallback when the portal has no
    /// such key. A finite value within 0.5..=3.0 is the scale; one outside that range or one that
    /// doesn't parse is no scaling and gives `None`, never a clamped value. A change raises the
    /// generation and wakes, as the other appearance values do.
    ///
    /// `None` where the platform does not say: other Linux desktops, macOS, Windows and the web,
    /// which scale the whole display instead.
    pub fn text_scale(&self) -> Option<f32> {
        let bits = self.shared.text_scale.load(Ordering::Acquire);
        (bits != 0).then(|| f32::from_bits(bits))
    }
    /// Block until the first answer is known, then return it; at most `timeout`.
    ///
    /// Returns early, with `current()`, when the portal is absent and the `gsettings` fallback
    /// found nothing, or when the desktop reports no preference; on Windows and macOS after the
    /// first read of the animation setting, always with `None`. Call it once while building the
    /// first frame so a light desktop does not flash dark:
    ///
    /// ```ignore
    /// let theme = watcher.wait_first(std::time::Duration::from_millis(250));
    /// ```
    pub fn wait_first(&self, timeout: Duration) -> Option<Theme> {
        let settled = self.shared.settled.lock().unwrap_or_else(PoisonError::into_inner);
        drop(self.shared.settled_changed.wait_timeout_while(settled, timeout, |settled| !*settled).unwrap_or_else(PoisonError::into_inner));
        self.current()
    }
    /// Enable or suppress future change notifications without stopping observation.
    /// Changing this flag does not itself wake the host or clear the cached preference. Becoming active looks at the
    /// window manager's configuration files again at once.
    pub fn set_active(&self, active: bool) {
        self.shared.active.store(active, Ordering::Release);
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        if active {
            let _ = self.shared.nudge.0.try_send(());
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn spawn<F: std::future::Future<Output = ()> + 'static>(
        active: bool,
        wake: impl Fn() + Send + Sync + 'static,
        source: impl FnOnce(Arc<Shared>) -> F + Send + 'static,
    ) -> Option<Self> {
        Self::spawn_shared(Shared::new(active, Box::new(wake)), source)
    }
    /// [`Watcher::spawn`] with `shared` made by the caller (a test finds the desktop for it).
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn spawn_shared<F: std::future::Future<Output = ()> + 'static>(
        shared: Shared,
        source: impl FnOnce(Arc<Shared>) -> F + Send + 'static,
    ) -> Option<Self> {
        let shared = Arc::new(shared);
        let worker_shared = Arc::clone(&shared);
        let (stop, stopped) = async_channel::bounded::<()>(1);
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("desktop-appearance".into())
            .spawn(move || {
                futures_lite::future::block_on(futures_lite::future::race(
                    async {
                        let _ = stopped.recv().await;
                    },
                    source(Arc::clone(&worker_shared)),
                ));
                worker_shared.settle();
                let _ = done.send(());
            })
            .ok()?;
        Some(Self { shared, worker: Worker::Portal { stop, worker: Some(worker), finished } })
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.set_active(false);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.shared.settle();
            match &mut self.worker {
                #[cfg(all(unix, not(target_os = "macos")))]
                Worker::Portal { stop, worker, finished } => {
                    stop.close();
                    if let Some(worker) = worker.take() {
                        let _ = finished.recv_timeout(std::time::Duration::from_millis(100));
                        // Acknowledgement precedes thread-local destructors. They can block too, so
                        // never join solely on the acknowledgement (or channel disconnection).
                        if worker.is_finished() {
                            let _ = worker.join();
                        }
                    }
                }
                #[cfg(any(windows, target_os = "macos"))]
                Worker::Native { stop, finished, thread } => {
                    // The worker's wait between reads ends at once; a read already in an OS call
                    // can block, so after a bounded wait an unfinished worker is detached.
                    let _ = stop.send(());
                    if let Some(thread) = thread.take() {
                        let _ = finished.recv_timeout(std::time::Duration::from_millis(100));
                        if thread.is_finished() {
                            let _ = thread.join();
                        }
                    }
                }
            }
        }
    }
}

/// The browser's `prefers-contrast: more` media query: `None` outside a browser window or when
/// the browser doesn't know the feature. The query runs on each call; there is no change
/// notification, so read it when building a frame.
#[cfg(target_arch = "wasm32")]
pub fn browser_contrast() -> Option<Contrast> {
    media_matches("(prefers-contrast: more)").map(|more| if more { Contrast::High } else { Contrast::Standard })
}
/// The browser's `prefers-reduced-motion: reduce` media query, read like [`browser_contrast`].
#[cfg(target_arch = "wasm32")]
pub fn browser_reduced_motion() -> Option<bool> {
    media_matches("(prefers-reduced-motion: reduce)")
}
#[cfg(target_arch = "wasm32")]
fn media_matches(query: &str) -> Option<bool> {
    let list = web_sys::window()?.match_media(query).ok()??;
    // A browser that doesn't know the feature serializes the query as `not all`.
    (list.media() != "not all").then(|| list.matches())
}

/// Whether the session is GNOME, by the same rules as `kind_from_env` in craft-window's
/// `desktop.rs` (the crates don't depend on each other, so change both together): a compositor
/// socket or a tiling entry means no; otherwise the first set of `XDG_CURRENT_DESKTOP`,
/// `XDG_SESSION_DESKTOP`, `DESKTOP_SESSION` decides by containing "gnome"; with none set,
/// `KDE_FULL_SESSION` means no and `GNOME_DESKTOP_SESSION_ID` means yes.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn is_gnome_session(var: &dyn Fn(&str) -> Option<String>) -> bool {
    const TILING: &[&str] =
        &["sway", "hyprland", "i3", "niri", "river", "bspwm", "awesome", "dwl", "qtile", "herbstluftwm", "leftwm", "xmonad", "dwm"];
    if ["SWAYSOCK", "I3SOCK", "HYPRLAND_INSTANCE_SIGNATURE", "NIRI_SOCKET"].iter().any(|k| var(k).is_some_and(|v| !v.is_empty())) {
        return false;
    }
    for key in ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"] {
        let Some(v) = var(key).map(|v| v.to_ascii_lowercase()).filter(|v| !v.trim().is_empty()) else { continue };
        return !v.split(':').map(str::trim).any(|entry| TILING.contains(&entry)) && v.contains("gnome");
    }
    var("KDE_FULL_SESSION").is_none() && var("GNOME_DESKTOP_SESSION_ID").is_some()
}

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod caption;
#[cfg(test)]
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod fake_bus;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod files;
#[cfg(any(windows, target_os = "macos"))]
mod native;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod portal;
