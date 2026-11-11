//! Desktop colour, contrast and motion preferences and GNOME's window-button layout through the
//! XDG Settings portal, plus the browser's contrast and motion media queries on the web.
//!
//! Derived from PdfCraft's activation-safe subscription and GNOME schema fallback,
//! CADCraft's BSD support, and ArtCraft Launcher's ReadOne/Read compatibility.
//! Palettes, manual choices and the fallback when no preference exists belong to the host.
mod reader;
pub use reader::{Reading, SystemReader};
use std::{
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU8, Ordering},
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
    active: AtomicBool,
    /// True once the source has answered, found no answer, or been cancelled.
    settled: Mutex<bool>,
    settled_changed: Condvar,
    /// GNOME's `button-layout` as the desktop reported it; `None` until an answer arrives.
    layout: Mutex<Option<String>>,
    /// GNOME's `icon-theme` name as the desktop reported it; `None` until an answer arrives.
    icons: Mutex<Option<String>>,
    /// `XDG_CURRENT_DESKTOP` names GNOME; read once when the watcher starts.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    gnome: bool,
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    wake: Box<dyn Fn() + Send + Sync>,
}
impl Shared {
    fn get(&self) -> Option<Theme> {
        match self.value.load(Ordering::Acquire) {
            1 => Some(Theme::Dark),
            2 => Some(Theme::Light),
            _ => None,
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
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
        self.settle();
        if changed && self.active.load(Ordering::Acquire) {
            (self.wake)();
        }
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_code(&self, slot: &AtomicU8, code: u8) {
        if slot.swap(code, Ordering::AcqRel) != code && self.active.load(Ordering::Acquire) {
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
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_motion(&self, reduced: bool) {
        self.publish_code(&self.motion, if reduced { 2 } else { 1 });
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn publish_layout(&self, layout: String) {
        self.publish_text(&self.layout, layout);
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
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    stop: async_channel::Sender<()>,
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    worker: Option<std::thread::JoinHandle<()>>,
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    finished: std::sync::mpsc::Receiver<()>,
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
    pub fn start_active(active: bool, wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        {
            Self::spawn(active, wake, portal::watch)
        }
        #[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
        {
            let _ = (active, wake);
            None
        }
    }
    /// Read the cached preference without blocking; `None` means no known preference.
    pub fn current(&self) -> Option<Theme> {
        self.shared.get()
    }
    /// GNOME's `org.gnome.desktop.wm.preferences` `button-layout` as the desktop reports it, such
    /// as `appmenu:minimize,close`; an empty string is an answer too. `None` until the portal or
    /// the `gsettings` fallback has answered, and on targets without a desktop watcher.
    ///
    /// It comes over the same connection as the theme, and a change runs the same wake callback.
    /// Only a GNOME-family window manager follows this setting; `craft_window::policy::LayoutWatch`
    /// decides when it applies. [`Watcher::wait_first`] does not wait for it.
    pub fn button_layout(&self) -> Option<String> {
        self.shared.layout.lock().unwrap_or_else(PoisonError::into_inner).clone()
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
    /// [`Watcher::wait_first`] returns after the first read of it. macOS and Windows have no
    /// watcher: reading their settings needs platform bindings, and this crate forbids `unsafe`.
    /// On the web use `browser_contrast`.
    pub fn contrast(&self) -> Option<Contrast> {
        match self.shared.contrast.load(Ordering::Acquire) {
            1 => Some(Contrast::Standard),
            2 => Some(Contrast::High),
            _ => None,
        }
    }
    /// Whether the desktop asks for reduced motion, from the portal's `org.freedesktop.appearance`
    /// `reduced-motion`, or GNOME's `org.gnome.desktop.interface` `enable-animations` (inverted)
    /// when the portal has no `reduced-motion` key. `None` until one answers, when neither key
    /// exists, and on targets without a desktop watcher.
    ///
    /// Wake and [`Watcher::wait_first`] behave as for [`Watcher::contrast`]. On the web use
    /// `browser_reduced_motion`.
    pub fn reduced_motion(&self) -> Option<bool> {
        match self.shared.motion.load(Ordering::Acquire) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }
    /// Block until the first answer is known, then return it; at most `timeout`.
    ///
    /// Returns early, with `current()`, when the portal is absent and the `gsettings` fallback
    /// found nothing, or when the desktop reports no preference. Call it once while building the
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
    /// Changing this flag does not itself wake the host or clear the cached preference.
    pub fn set_active(&self, active: bool) {
        self.shared.active.store(active, Ordering::Release);
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    fn spawn<F: std::future::Future<Output = ()> + 'static>(
        active: bool,
        wake: impl Fn() + Send + Sync + 'static,
        source: impl FnOnce(Arc<Shared>) -> F + Send + 'static,
    ) -> Option<Self> {
        let gnome = is_gnome_session(&|k| std::env::var(k).ok());
        let shared = Arc::new(Shared {
            value: AtomicU8::new(0),
            contrast: AtomicU8::new(0),
            motion: AtomicU8::new(0),
            active: AtomicBool::new(active),
            settled: Mutex::new(false),
            settled_changed: Condvar::new(),
            layout: Mutex::new(None),
            icons: Mutex::new(None),
            gnome,
            wake: Box::new(wake),
        });
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
        Some(Self { shared, stop, worker: Some(worker), finished })
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.set_active(false);
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        {
            self.shared.settle();
            self.stop.close();
            if let Some(worker) = self.worker.take() {
                let _ = self.finished.recv_timeout(std::time::Duration::from_millis(100));
                // Acknowledgement precedes thread-local destructors. They can block too, so
                // never join solely on the acknowledgement (or channel disconnection).
                if worker.is_finished() {
                    let _ = worker.join();
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
mod portal;
