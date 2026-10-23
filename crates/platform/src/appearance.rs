//! Desktop colour preference through the XDG Settings portal.
//!
//! Derived from PdfCraft's activation-safe subscription and GNOME schema fallback,
//! CADCraft's BSD support, and ArtCraft Launcher's ReadOne/Read compatibility.
//! Palettes, manual choices and the fallback when no preference exists belong to the host.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// Desktop color preference observed independently of the application palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// The desktop requests a dark appearance.
    Dark,
    /// The desktop requests a light appearance.
    Light,
}

struct Shared {
    value: AtomicU8,
    active: AtomicBool,
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
    fn publish(&self, theme: Option<Theme>) {
        let code = match theme {
            Some(Theme::Dark) => 1,
            Some(Theme::Light) => 2,
            None => 0,
        };
        if self.value.swap(code, Ordering::AcqRel) != code && self.active.load(Ordering::Acquire) {
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
        let shared = Arc::new(Shared { value: AtomicU8::new(0), active: AtomicBool::new(active), wake: Box::new(wake) });
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
                    source(worker_shared),
                ));
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

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod portal {
    use super::{Shared, Theme};
    use futures_lite::StreamExt;
    use std::{sync::Arc, time::Duration};
    use zbus::zvariant::{OwnedValue, Value};
    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const IFACE: &str = "org.freedesktop.portal.Settings";
    const STANDARD: &str = "org.freedesktop.appearance";
    const GNOME: &str = "org.gnome.desktop.interface";
    const KEY: &str = "color-scheme";
    const TIMEOUT: Duration = Duration::from_secs(2);

    // Outer None means unknown/malformed; Some(None) is an explicit no-preference answer.
    fn decode(namespace: &str, value: &Value<'_>) -> Option<Option<Theme>> {
        match (namespace, value) {
            (STANDARD, Value::U32(n)) => Some(match n {
                1 => Some(Theme::Dark),
                2 => Some(Theme::Light),
                _ => None,
            }),
            (GNOME, Value::Str(s)) => Some(match s.as_str() {
                "prefer-dark" => Some(Theme::Dark),
                "prefer-light" => Some(Theme::Light),
                _ => None,
            }),
            _ => None,
        }
    }
    fn reply(namespace: &str, value: &Value<'_>, legacy: bool) -> Option<Option<Theme>> {
        if legacy {
            let Value::Value(inner) = value else { return None };
            decode(namespace, inner)
        } else {
            decode(namespace, value)
        }
    }
    fn combined(standard: Option<Option<Theme>>, gnome: Option<Option<Theme>>) -> Option<Option<Theme>> {
        if standard.is_none() && gnome.is_none() { None } else { Some(standard.flatten().or(gnome.flatten())) }
    }
    async fn bounded<T>(future: impl std::future::Future<Output = T>) -> Option<T> {
        futures_lite::future::race(async { Some(future.await) }, async {
            async_io::Timer::after(TIMEOUT).await;
            None
        })
        .await
    }
    async fn read(connection: &zbus::Connection, namespace: &str) -> Option<Option<Theme>> {
        for (method, legacy) in [("ReadOne", false), ("Read", true)] {
            if let Some(Ok(message)) = bounded(connection.call_method(Some(DEST), PATH, Some(IFACE), method, &(namespace, KEY))).await
                && let Ok(value) = message.body().deserialize::<OwnedValue>()
                && let Some(answer) = reply(namespace, &value, legacy)
            {
                return Some(answer);
            }
        }
        None
    }
    fn rule() -> zbus::Result<zbus::MatchRule<'static>> {
        Ok(zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(DEST)?
            .path(PATH)?
            .interface(IFACE)?
            .member("SettingChanged")?
            .arg(1, KEY)?
            .build())
    }
    fn owner_rule() -> zbus::Result<zbus::MatchRule<'static>> {
        Ok(zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, DEST)?
            .build())
    }
    pub(super) async fn watch(shared: Arc<Shared>) {
        let mut retry = Duration::from_secs(5);
        let mut fallback_used = false;
        loop {
            let mut connected = false;
            let _ = session(&shared, &mut fallback_used, &mut connected).await;
            if connected {
                retry = Duration::from_secs(5);
            }
            // Keep the last valid answer during disconnect; a new explicit no-preference clears it.
            async_io::Timer::after(retry).await;
            retry = (retry * 2).min(Duration::from_secs(300));
        }
    }
    // Keep Nix/profile PATH entries, but never execute a helper from a relative search folder.
    fn gsettings_exe(path: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
        let dirs = path.map(std::env::split_paths).into_iter().flatten().filter(|p| p.is_absolute());
        dirs.chain(["/usr/bin", "/bin", "/usr/local/bin"].map(std::path::PathBuf::from)).map(|p| p.join("gsettings")).find(|p| p.is_file())
    }
    async fn fallback(shared: &Shared, used: &mut bool) {
        if *used {
            return;
        }
        *used = true;
        if let Some(exe) = gsettings_exe(std::env::var_os("PATH").as_deref())
            && let Some(text) = crate::process::capture(&exe, &["get", GNOME, KEY], TIMEOUT).await
        {
            let text = text.trim().trim_matches('\'');
            if let Some(answer) = decode(GNOME, &Value::from(text)) {
                shared.publish(answer);
            }
        }
    }
    async fn session(shared: &Shared, fallback_used: &mut bool, connected: &mut bool) -> Option<()> {
        let connection = match zbus::connection::Builder::session() {
            Ok(builder) => bounded(builder.method_timeout(TIMEOUT).build()).await.and_then(Result::ok),
            Err(_) => None,
        };
        let Some(connection) = connection else {
            fallback(shared, fallback_used).await;
            return None;
        };
        watch_connection(shared, fallback_used, connected, connection).await
    }
    async fn watch_connection(shared: &Shared, fallback_used: &mut bool, connected: &mut bool, connection: zbus::Connection) -> Option<()> {
        // MatchRule installs before activation: unlike a proxy subscription it needs no current owner.
        let mut changes = bounded(zbus::MessageStream::for_match_rule(rule().ok()?, &connection, Some(64))).await?.ok()?;
        let mut owners = bounded(zbus::MessageStream::for_match_rule(owner_rule().ok()?, &connection, Some(8))).await?.ok()?;
        let mut standard = read(&connection, STANDARD).await;
        let mut gnome = read(&connection, GNOME).await;
        match combined(standard, gnome) {
            Some(answer) => {
                *connected = true;
                shared.publish(answer);
            }
            None => {
                fallback(shared, fallback_used).await;
                return None;
            }
        }
        loop {
            let message =
                futures_lite::future::race(async { changes.next().await.map(|m| (false, m)) }, async { owners.next().await.map(|m| (true, m)) })
                    .await?;
            let (owner, message) = message;
            let message = message.ok()?;
            if owner {
                // Activation is already covered by the initial read. Loss/replacement reconnects.
                if let Ok((_, old, _)) = message.body().deserialize::<(String, String, String)>()
                    && !old.is_empty()
                {
                    return None;
                }
                continue;
            }
            let Ok((namespace, key, value)) = message.body().deserialize::<(String, String, OwnedValue)>() else { continue };
            if key != KEY {
                continue;
            }
            let Some(answer) = decode(&namespace, &value) else { continue };
            match namespace.as_str() {
                STANDARD => standard = Some(answer),
                GNOME => gnome = Some(answer),
                _ => continue,
            }
            if let Some(answer) = combined(standard, gnome) {
                shared.publish(answer);
            }
        }
    }
    #[cfg(test)]
    #[path = "appearance_bus_tests.rs"]
    mod bus_tests;
    #[cfg(test)]
    mod tests {
        #[test]
        fn gsettings_respects_absolute_nix_style_path_without_using_relative_folders() -> Result<(), Box<dyn std::error::Error>> {
            struct Fixture(std::path::PathBuf);
            impl Drop for Fixture {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
            let directory = std::env::temp_dir().join(format!("craft-platform-gsettings-{}-{nonce}", std::process::id()));
            std::fs::create_dir(&directory)?;
            let fixture = Fixture(directory);
            let helper = fixture.0.join("gsettings");
            std::fs::write(&helper, b"fixture")?;
            let path = std::env::join_paths([std::path::Path::new("relative-untrusted"), fixture.0.as_path()])?;
            assert_eq!(super::gsettings_exe(Some(path.as_os_str())), Some(helper));
            assert!(super::gsettings_exe(Some(std::ffi::OsStr::new("relative-untrusted"))).is_none_or(|p| p.is_absolute()));
            Ok(())
        }

        use super::{GNOME, STANDARD, combined, decode, reply};
        use crate::appearance::{Theme, Watcher};
        use std::{
            sync::{Arc, atomic::Ordering},
            time::Duration,
        };
        use zbus::zvariant::Value;
        #[test]
        fn no_preference_is_an_answer_and_gnome_only_fills_missing_preference() {
            assert_eq!(combined(None, None), None);
            assert_eq!(combined(Some(None), None), Some(None));
            assert_eq!(combined(Some(None), Some(Some(Theme::Dark))), Some(Some(Theme::Dark)));
            assert_eq!(combined(Some(Some(Theme::Light)), Some(Some(Theme::Dark))), Some(Some(Theme::Light)));
            assert_eq!(combined(None, Some(None)), Some(None));
        }
        #[test]
        fn malformed_is_distinct_from_no_preference_and_legacy_is_exact() {
            assert_eq!(decode(STANDARD, &Value::U32(0)), Some(None));
            assert_eq!(decode(STANDARD, &Value::U32(77)), Some(None));
            assert_eq!(decode(STANDARD, &Value::from("dark")), None);
            assert_eq!(reply(STANDARD, &Value::Value(Box::new(Value::U32(1))), true), Some(Some(Theme::Dark)));
            assert_eq!(reply(STANDARD, &Value::U32(1), true), None);
            assert_eq!(reply(STANDARD, &Value::Value(Box::new(Value::Value(Box::new(Value::U32(1))))), true), None);
            assert_eq!(decode(STANDARD, &Value::Value(Box::new(Value::U32(1)))), None);
            assert_eq!(decode(STANDARD, &Value::I32(1)), None);
            assert_eq!(decode(GNOME, &Value::from("default")), Some(None));
            assert_eq!(decode(GNOME, &Value::U32(1)), None);

            assert_eq!(reply(GNOME, &Value::from("prefer-light"), false), Some(Some(Theme::Light)));
        }
        #[test]
        fn manual_cache_changes_do_not_wake_and_drop_cancels_pending_source() -> Result<(), Box<dyn std::error::Error>> {
            let (tx, rx) = std::sync::mpsc::channel();
            let (cancelled, cancellation) = std::sync::mpsc::channel();
            struct Cancelled(std::sync::mpsc::Sender<()>);
            impl Drop for Cancelled {
                fn drop(&mut self) {
                    let _ = self.0.send(());
                }
            }
            let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed = Arc::clone(&wakes);
            let watcher = Watcher::spawn(
                false,
                move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                },
                move |shared| async move {
                    let _cancelled = Cancelled(cancelled);
                    shared.publish(Some(Theme::Dark));
                    let _ = tx.send(());
                    futures_lite::future::pending::<()>().await;
                },
            )
            .ok_or("worker creation failed")?;
            rx.recv_timeout(Duration::from_secs(2))?;
            assert_eq!(watcher.current(), Some(Theme::Dark));
            assert_eq!(wakes.load(Ordering::Relaxed), 0);
            watcher.set_active(true);
            watcher.shared.publish(Some(Theme::Dark));
            assert_eq!(wakes.load(Ordering::Relaxed), 0);
            watcher.shared.publish(Some(Theme::Light));
            assert_eq!(watcher.current(), Some(Theme::Light));
            watcher.shared.publish(Some(Theme::Dark));
            assert_eq!(watcher.current(), Some(Theme::Dark));
            watcher.shared.publish(None);
            assert_eq!(watcher.current(), None);
            assert_eq!(wakes.load(Ordering::Relaxed), 3);
            drop(watcher);
            cancellation.recv_timeout(Duration::from_secs(2))?;
            Ok(())
        }
        #[test]
        fn drop_does_not_wait_for_a_synchronously_blocked_source() -> Result<(), Box<dyn std::error::Error>> {
            let (started, ready) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let (exited, exit) = std::sync::mpsc::channel();
            struct Release(Option<std::sync::mpsc::Sender<()>>);
            impl Drop for Release {
                fn drop(&mut self) {
                    if let Some(release) = self.0.take() {
                        let _ = release.send(());
                    }
                }
            }
            // Declared before the watcher: every error/unwind releases the synchronous poll.
            let release = Release(Some(release));
            let watcher = Watcher::spawn(
                false,
                || {},
                move |_| async move {
                    let _ = started.send(());
                    let _ = blocked.recv();
                    let _ = exited.send(());
                },
            )
            .ok_or("worker creation failed")?;
            ready.recv_timeout(Duration::from_secs(2))?;
            let (dropped, drop_result) = std::sync::mpsc::channel();
            let dropper = std::thread::spawn(move || {
                drop(watcher);
                let _ = dropped.send(());
            });
            let bounded = drop_result.recv_timeout(Duration::from_secs(1)).is_ok();
            // Release before asserting, including when an unbounded implementation regresses.
            drop(release);
            exit.recv_timeout(Duration::from_secs(2))?;
            assert!(dropper.join().is_ok(), "watcher drop panicked");
            assert!(bounded, "watcher drop waited for a blocked synchronous poll");
            Ok(())
        }
    }
}
/// Owned callback that supplies an observed theme and updates egui native-window hints.
#[cfg(feature = "egui")]
pub type SystemThemeFn = Box<dyn Fn(&egui::Context) -> Option<egui::Theme>>;
/// Create an egui theme callback that owns the subscription and wakes its first context.
/// Return `None` when no desktop watcher can start; the host retains its fallback policy.
#[cfg(feature = "egui")]
pub fn egui_service() -> Option<SystemThemeFn> {
    let context = Arc::new(std::sync::OnceLock::<egui::Context>::new());
    let worker_context = Arc::clone(&context);
    let watcher = Watcher::start(move || {
        if let Some(ctx) = worker_context.get() {
            ctx.request_repaint();
        }
    })?;
    Some(Box::new(move |ctx| {
        let _ = context.set(ctx.clone());
        sync_native_window_theme(ctx, watcher.current());
        watcher.current().map(|t| match t {
            Theme::Dark => egui::Theme::Dark,
            Theme::Light => egui::Theme::Light,
        })
    }))
}

/// Supply the portal answer for native window decorations without changing egui's theme choice
/// or either palette. A missing portal answer falls back to the current toolkit system theme;
/// when both are absent the native pin is released. Linux/BSD window backends cannot reliably
/// resolve `SystemDefault` themselves.
/// The output hook runs after egui's own window-theme synchronization, so it cannot be overwritten
/// by a later automatic `SetTheme(SystemDefault)` in the same pass.
#[cfg(feature = "egui")]
pub fn sync_native_window_theme(ctx: &egui::Context, observed: Option<Theme>) {
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    {
        ctx.add_plugin(NativeWindowTheme::default());
        ctx.with_plugin(|plugin: &mut NativeWindowTheme| plugin.observed = observed);
    }
    #[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
    let _ = (ctx, observed);
}

#[cfg(all(feature = "egui", unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[derive(Default)]
struct NativeWindowTheme {
    observed: Option<Theme>,
    sent: std::collections::HashMap<egui::ViewportId, egui::SystemTheme>,
}
#[cfg(all(feature = "egui", unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
impl egui::Plugin for NativeWindowTheme {
    fn debug_name(&self) -> &'static str {
        "desktop window appearance"
    }
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        if !ctx.options(|o| o.sync_window_theme) {
            // Another owner may alter the native theme while synchronization is disabled.
            self.sent.clear();
            return;
        }
        let desired = match ctx.options(|o| o.theme_preference) {
            egui::ThemePreference::Dark => egui::SystemTheme::Dark,
            egui::ThemePreference::Light => egui::SystemTheme::Light,
            egui::ThemePreference::System => match self.observed.or_else(|| {
                ctx.system_theme().map(|theme| match theme {
                    egui::Theme::Dark => Theme::Dark,
                    egui::Theme::Light => Theme::Light,
                })
            }) {
                Some(Theme::Dark) => egui::SystemTheme::Dark,
                Some(Theme::Light) => egui::SystemTheme::Light,
                None => egui::SystemTheme::SystemDefault,
            },
        };
        // FullOutput can carry child-window commands too. Rewrite every viewport so a deferred
        // or immediate child cannot reintroduce egui's inherited hint after its own UI pass.
        for (id, viewport) in &mut output.viewport_output {
            let had_sync = viewport.commands.iter().any(|command| matches!(command, egui::ViewportCommand::SetTheme(_)));
            viewport.commands.retain(|command| !matches!(command, egui::ViewportCommand::SetTheme(_)));
            if had_sync || self.sent.get(id) != Some(&desired) {
                viewport.commands.push(egui::ViewportCommand::SetTheme(desired));
                self.sent.insert(*id, desired);
            }
        }
        // Forget destroyed native windows: a reused viewport identifier must receive its hint.
        self.sent.retain(|id, _| output.viewport_output.contains_key(id));
    }
}

#[cfg(all(test, feature = "egui", unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod native_window_tests {
    use super::{Theme, sync_native_window_theme};
    fn pass(ctx: &egui::Context, observed: Option<Theme>, preference: egui::ThemePreference) -> Vec<egui::SystemTheme> {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            sync_native_window_theme(ui.ctx(), observed);
            // Deliberately apply the host's choice AFTER the observer, as real palette adapters do.
            ui.ctx().set_theme(preference);
        });
        output.textures_delta.clear(); // Headless policy test; no renderer consumes uploads.
        output
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .filter_map(|command| match command {
                egui::ViewportCommand::SetTheme(theme) => Some(*theme),
                _ => None,
            })
            .collect()
    }
    #[test]
    fn actual_end_pass_maps_system_manual_and_reset_without_overwriting_palettes() {
        use egui::{SystemTheme as Native, ThemePreference as Choice};
        let ctx = egui::Context::default();
        let marker = egui::Color32::from_rgb(12, 34, 56);
        ctx.all_styles_mut(|s| s.visuals.panel_fill = marker);
        assert_eq!(pass(&ctx, Some(Theme::Light), Choice::System), vec![Native::Light]);
        assert_eq!(ctx.options(|o| o.theme_preference), Choice::System);
        assert!(pass(&ctx, Some(Theme::Light), Choice::System).is_empty());
        assert_eq!(pass(&ctx, Some(Theme::Dark), Choice::System), vec![Native::Dark]);
        // A portal change cannot override a concrete manual native preference.
        assert_eq!(pass(&ctx, Some(Theme::Light), Choice::Dark), vec![Native::Dark]);
        assert_eq!(pass(&ctx, Some(Theme::Dark), Choice::Light), vec![Native::Light]);
        assert_eq!(pass(&ctx, Some(Theme::Dark), Choice::System), vec![Native::Dark]);
        assert_eq!(pass(&ctx, None, Choice::System), vec![Native::SystemDefault]);
        assert!(pass(&ctx, None, Choice::System).is_empty());
        assert_eq!(ctx.style_of(egui::Theme::Dark).visuals.panel_fill, marker);
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals.panel_fill, marker);
    }
    #[test]
    fn native_system_hint_uses_current_toolkit_fallback_without_retaining_stale_input() {
        let ctx = egui::Context::default();
        for (raw_theme, observed, expected) in [
            (Some(egui::Theme::Light), None, egui::SystemTheme::Light),
            (Some(egui::Theme::Dark), None, egui::SystemTheme::Dark),
            (Some(egui::Theme::Dark), Some(Theme::Light), egui::SystemTheme::Light),
            (None, None, egui::SystemTheme::SystemDefault),
        ] {
            let mut output = ctx.run_ui(egui::RawInput { system_theme: raw_theme, ..Default::default() }, |ui| {
                sync_native_window_theme(ui.ctx(), observed);
                ui.ctx().set_theme(egui::ThemePreference::System);
            });
            let hints: Vec<_> = output
                .viewport_output
                .values()
                .flat_map(|v| &v.commands)
                .filter_map(|command| match command {
                    egui::ViewportCommand::SetTheme(theme) => Some(*theme),
                    _ => None,
                })
                .collect();
            output.textures_delta.clear();
            assert_eq!(hints, vec![expected]);
        }
    }
    #[test]
    fn deferred_windows_receive_the_same_hint_and_reset() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let child = egui::ViewportId::from_hash_of("appearance-child");
        for observed in [Some(Theme::Light), Some(Theme::Dark), None] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                sync_native_window_theme(ui.ctx(), observed);
                ui.ctx().set_theme(egui::ThemePreference::System);
                ui.ctx().show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
            });
            output.textures_delta.clear();
            let expected = match observed {
                Some(Theme::Light) => egui::SystemTheme::Light,
                Some(Theme::Dark) => egui::SystemTheme::Dark,
                None => egui::SystemTheme::SystemDefault,
            };
            for id in [egui::ViewportId::ROOT, child] {
                let commands: Vec<_> = output
                    .viewport_output
                    .get(&id)
                    .into_iter()
                    .flat_map(|v| &v.commands)
                    .filter_map(|c| match c {
                        egui::ViewportCommand::SetTheme(t) => Some(*t),
                        _ => None,
                    })
                    .collect();
                assert_eq!(commands, vec![expected]);
            }
        }
    }
    #[test]
    fn custom_palette_system_policy_and_explicit_sync_opt_out_are_preserved() {
        use egui::{SystemTheme as Native, ThemePreference as Choice};
        let ctx = egui::Context::default();
        // Photo/PDF/Sound keep System native policy even when their custom palette is manual.
        ctx.all_styles_mut(|s| s.visuals = egui::Visuals::dark());
        assert_eq!(pass(&ctx, Some(Theme::Light), Choice::System), vec![Native::Light]);
        assert!(ctx.style_of(egui::Theme::Light).visuals.dark_mode);
        ctx.options_mut(|o| o.sync_window_theme = false);
        assert!(pass(&ctx, Some(Theme::Dark), Choice::System).is_empty());
        ctx.options_mut(|o| o.sync_window_theme = true);
        assert_eq!(pass(&ctx, Some(Theme::Dark), Choice::System), vec![Native::Dark]);
    }
}
