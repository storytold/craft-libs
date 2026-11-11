//! The XDG Settings portal: reading colour scheme, contrast, motion, button layout and icons, and watching for changes.

use super::{Contrast, Shared, Theme};
use futures_lite::{StreamExt, future::zip};
use std::{sync::Arc, time::Duration};
use zbus::zvariant::{OwnedValue, Value};
const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.Settings";
const STANDARD: &str = "org.freedesktop.appearance";
const GNOME: &str = "org.gnome.desktop.interface";
const KEY: &str = "color-scheme";
const WM: &str = "org.gnome.desktop.wm.preferences";
const LAYOUT_KEY: &str = "button-layout";
const ICONS_KEY: &str = "icon-theme";
const CONTRAST_KEY: &str = "contrast";
const MOTION_KEY: &str = "reduced-motion";
const ANIMATIONS_KEY: &str = "enable-animations";
const TIMEOUT: Duration = Duration::from_secs(2);

// Outer None means unknown/malformed; Some(None) is an explicit no-preference answer.
// GNOME 42-46 writes 'default' (portal value 0) for Settings > Appearance > Light, so on GNOME
// both mean light; elsewhere both are no preference.
fn decode(namespace: &str, value: &Value<'_>, gnome: bool) -> Option<Option<Theme>> {
    match (namespace, value) {
        (STANDARD, Value::U32(n)) => Some(match n {
            1 => Some(Theme::Dark),
            2 => Some(Theme::Light),
            0 if gnome => Some(Theme::Light),
            _ => None,
        }),
        (GNOME, Value::Str(s)) => Some(match s.as_str() {
            "prefer-dark" => Some(Theme::Dark),
            "prefer-light" => Some(Theme::Light),
            "default" if gnome => Some(Theme::Light),
            _ => None,
        }),
        _ => None,
    }
}
// The legacy `Read` wraps the value in one more variant; anything else is malformed.
fn unwrap_legacy<'a>(value: &'a Value<'a>, legacy: bool) -> Option<&'a Value<'a>> {
    match (legacy, value) {
        (false, value) => Some(value),
        (true, Value::Value(inner)) => Some(inner),
        (true, _) => None,
    }
}
fn reply(namespace: &str, value: &Value<'_>, legacy: bool, gnome: bool) -> Option<Option<Theme>> {
    decode(namespace, unwrap_legacy(value, legacy)?, gnome)
}
// `None` means malformed. The portal spec treats unknown values as 0, no preference.
fn decode_contrast(namespace: &str, value: &Value<'_>) -> Option<Contrast> {
    match (namespace, value) {
        (STANDARD, Value::U32(n)) => Some(if *n == 1 { Contrast::High } else { Contrast::Standard }),
        _ => None,
    }
}
fn contrast_reply(value: &Value<'_>, legacy: bool) -> Option<Contrast> {
    decode_contrast(STANDARD, unwrap_legacy(value, legacy)?)
}
// Whether motion is reduced: the portal's `reduced-motion` (1 reduced; other values are no
// preference) or GNOME's `enable-animations` boolean, inverted. `None` means malformed.
fn decode_motion(namespace: &str, value: &Value<'_>) -> Option<bool> {
    match (namespace, value) {
        (STANDARD, Value::U32(n)) => Some(*n == 1),
        (GNOME, Value::Bool(enabled)) => Some(!enabled),
        _ => None,
    }
}
fn motion_reply(namespace: &str, value: &Value<'_>, legacy: bool) -> Option<bool> {
    decode_motion(namespace, unwrap_legacy(value, legacy)?)
}
// `None` means malformed. Any string is an answer, the empty one included: what the buttons
// mean, and what an unusable layout falls back to, is craft-window's call.
fn decode_layout(value: &Value<'_>) -> Option<String> {
    let Value::Str(s) = value else { return None };
    Some(s.as_str().trim().to_string())
}
fn layout_reply(value: &Value<'_>, legacy: bool) -> Option<String> {
    decode_layout(unwrap_legacy(value, legacy)?)
}
// `None` means malformed. A blank name is no answer.
fn decode_icons(value: &Value<'_>) -> Option<String> {
    let Value::Str(s) = value else { return None };
    Some(s.as_str().trim().to_string()).filter(|name| !name.is_empty())
}
fn icons_reply(value: &Value<'_>, legacy: bool) -> Option<String> {
    decode_icons(unwrap_legacy(value, legacy)?)
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
async fn read_with<T>(connection: &zbus::Connection, namespace: &str, key: &str, answer: impl Fn(&Value<'_>, bool) -> Option<T>) -> Option<T> {
    for (method, legacy) in [("ReadOne", false), ("Read", true)] {
        if let Some(Ok(message)) = bounded(connection.call_method(Some(DEST), PATH, Some(IFACE), method, &(namespace, key))).await
            && let Ok(value) = message.body().deserialize::<OwnedValue>()
            && let Some(answer) = answer(&value, legacy)
        {
            return Some(answer);
        }
    }
    None
}
async fn read(connection: &zbus::Connection, namespace: &str, gnome: bool) -> Option<Option<Theme>> {
    read_with(connection, namespace, KEY, |value, legacy| reply(namespace, value, legacy, gnome)).await
}
fn rule(key: &'static str) -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DEST)?
        .path(PATH)?
        .interface(IFACE)?
        .member("SettingChanged")?
        .arg(1, key)?
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
/// The one-shot `gsettings` reads used when the portal can't answer, each at most once per
/// watcher, and the `PATH` they search.
pub(super) struct Fallbacks {
    theme_used: bool,
    layout_used: bool,
    icons_used: bool,
    path: Option<std::ffi::OsString>,
}
impl Fallbacks {
    fn from_env() -> Self {
        Self { theme_used: false, layout_used: false, icons_used: false, path: std::env::var_os("PATH") }
    }
    async fn get(&self, schema: &str, key: &str) -> Option<String> {
        let exe = gsettings_exe(self.path.as_deref())?;
        let text = crate::process::capture(&exe, &["get", schema, key], TIMEOUT).await?;
        Some(text.trim().trim_matches('\'').to_string())
    }
    async fn theme(&mut self, shared: &Shared) {
        if std::mem::replace(&mut self.theme_used, true) {
            return;
        }
        if let Some(text) = self.get(GNOME, KEY).await
            && let Some(answer) = decode(GNOME, &Value::from(text.as_str()), shared.gnome)
        {
            shared.publish(answer);
        }
    }
    async fn icons(&mut self, shared: &Shared) {
        if std::mem::replace(&mut self.icons_used, true) {
            return;
        }
        if let Some(text) = self.get(GNOME, ICONS_KEY).await
            && let Some(answer) = decode_icons(&Value::from(text.as_str()))
        {
            shared.publish_icons(answer);
        }
    }
    async fn layout(&mut self, shared: &Shared) {
        if std::mem::replace(&mut self.layout_used, true) {
            return;
        }
        if let Some(text) = self.get(WM, LAYOUT_KEY).await
            && let Some(answer) = decode_layout(&Value::from(text.as_str()))
        {
            shared.publish_layout(answer);
        }
    }
}
pub(super) async fn watch(shared: Arc<Shared>) {
    let mut retry = Duration::from_secs(5);
    let mut fallbacks = Fallbacks::from_env();
    loop {
        let mut connected = false;
        let _ = session(&shared, &mut fallbacks, &mut connected).await;
        // No session bus, no portal and an empty fallback all end here without a publish.
        shared.settle();
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
async fn session(shared: &Shared, fallbacks: &mut Fallbacks, connected: &mut bool) -> Option<()> {
    let connection = match zbus::connection::Builder::session() {
        Ok(builder) => bounded(builder.method_timeout(TIMEOUT).build()).await.and_then(Result::ok),
        Err(_) => None,
    };
    let Some(connection) = connection else {
        fallbacks.theme(shared).await;
        fallbacks.layout(shared).await;
        fallbacks.icons(shared).await;
        return None;
    };
    watch_connection(shared, fallbacks, connected, connection).await
}
enum Event {
    Setting(zbus::Result<zbus::Message>),
    Owner(zbus::Result<zbus::Message>),
}
/// The latest valid answer from each source of a setting that combines two namespaces.
struct Answers {
    standard: Option<Option<Theme>>,
    gnome: Option<Option<Theme>>,
    motion: Option<bool>,
    gnome_motion: Option<bool>,
}
impl Answers {
    fn publish_motion(&self, shared: &Shared) {
        if let Some(reduced) = self.motion.or(self.gnome_motion) {
            shared.publish_motion(reduced);
        }
    }
    /// Apply one `SettingChanged`; malformed values and other settings change nothing.
    fn apply(&mut self, shared: &Shared, namespace: &str, key: &str, value: &Value<'_>) {
        match (namespace, key) {
            (WM, LAYOUT_KEY) => {
                if let Some(layout) = decode_layout(value) {
                    shared.publish_layout(layout);
                }
            }
            (GNOME, ICONS_KEY) => {
                if let Some(icons) = decode_icons(value) {
                    shared.publish_icons(icons);
                }
            }
            (STANDARD, CONTRAST_KEY) => {
                if let Some(contrast) = decode_contrast(namespace, value) {
                    shared.publish_contrast(contrast);
                }
            }
            (STANDARD, MOTION_KEY) | (GNOME, ANIMATIONS_KEY) => {
                let Some(reduced) = decode_motion(namespace, value) else { return };
                if namespace == STANDARD {
                    self.motion = Some(reduced);
                } else {
                    self.gnome_motion = Some(reduced);
                }
                self.publish_motion(shared);
            }
            (STANDARD | GNOME, KEY) => {
                let Some(answer) = decode(namespace, value, shared.gnome) else { return };
                if namespace == STANDARD {
                    self.standard = Some(answer);
                } else {
                    self.gnome = Some(answer);
                }
                if let Some(answer) = combined(self.standard, self.gnome) {
                    shared.publish(answer);
                }
            }
            _ => {}
        }
    }
}
async fn subscribe(connection: &zbus::Connection, rule: zbus::Result<zbus::MatchRule<'static>>, capacity: usize) -> Option<zbus::MessageStream> {
    bounded(zbus::MessageStream::for_match_rule(rule.ok()?, connection, Some(capacity))).await?.ok()
}
async fn next_event(settings: &mut [zbus::MessageStream], owners: &mut zbus::MessageStream) -> Option<Event> {
    let setting = futures_lite::future::poll_fn(|cx| {
        for stream in settings.iter_mut() {
            if let std::task::Poll::Ready(message) = stream.poll_next(cx) {
                return std::task::Poll::Ready(message.map(Event::Setting));
            }
        }
        std::task::Poll::Pending
    });
    futures_lite::future::race(setting, async { owners.next().await.map(Event::Owner) }).await
}
async fn watch_connection(shared: &Shared, fallbacks: &mut Fallbacks, connected: &mut bool, connection: zbus::Connection) -> Option<()> {
    // MatchRule installs before activation: unlike a proxy subscription it needs no current owner.
    let mut settings = [
        subscribe(&connection, rule(KEY), 64).await?,
        subscribe(&connection, rule(LAYOUT_KEY), 8).await?,
        subscribe(&connection, rule(ICONS_KEY), 8).await?,
        subscribe(&connection, rule(CONTRAST_KEY), 8).await?,
        subscribe(&connection, rule(MOTION_KEY), 8).await?,
        subscribe(&connection, rule(ANIMATIONS_KEY), 8).await?,
    ];
    let mut owners = subscribe(&connection, owner_rule(), 8).await?;
    // Concurrent reads, so contrast and motion are known by the time the theme settles
    // `wait_first` without adding round trips before it.
    let ((standard, gnome), (contrast, (motion, gnome_motion))) = zip(
        zip(read(&connection, STANDARD, shared.gnome), read(&connection, GNOME, shared.gnome)),
        zip(
            read_with(&connection, STANDARD, CONTRAST_KEY, contrast_reply),
            zip(
                read_with(&connection, STANDARD, MOTION_KEY, |value, legacy| motion_reply(STANDARD, value, legacy)),
                read_with(&connection, GNOME, ANIMATIONS_KEY, |value, legacy| motion_reply(GNOME, value, legacy)),
            ),
        ),
    )
    .await;
    let mut answers = Answers { standard, gnome, motion, gnome_motion };
    if let Some(contrast) = contrast {
        shared.publish_contrast(contrast);
    }
    answers.publish_motion(shared);
    match combined(standard, gnome) {
        Some(answer) => {
            *connected = true;
            shared.publish(answer);
        }
        None => {
            fallbacks.theme(shared).await;
            fallbacks.layout(shared).await;
            fallbacks.icons(shared).await;
            return None;
        }
    }
    // Read after the theme is published so `wait_first` never waits for the layout.
    match read_with(&connection, WM, LAYOUT_KEY, layout_reply).await {
        Some(layout) => shared.publish_layout(layout),
        None => fallbacks.layout(shared).await,
    }
    match read_with(&connection, GNOME, ICONS_KEY, icons_reply).await {
        Some(icons) => shared.publish_icons(icons),
        None => fallbacks.icons(shared).await,
    }
    loop {
        let message = match next_event(&mut settings, &mut owners).await? {
            Event::Owner(message) => {
                // Activation is already covered by the initial read. Loss/replacement reconnects.
                if let Ok((_, old, _)) = message.ok()?.body().deserialize::<(String, String, String)>()
                    && !old.is_empty()
                {
                    return None;
                }
                continue;
            }
            Event::Setting(message) => message.ok()?,
        };
        if let Ok((namespace, key, value)) = message.body().deserialize::<(String, String, OwnedValue)>() {
            answers.apply(shared, &namespace, &key, &value);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{GNOME, STANDARD, decode};
    use crate::appearance::is_gnome_session;
    use crate::appearance::{Theme, Watcher};
    use std::time::Duration;
    use zbus::zvariant::Value;
    #[test]
    fn gnome_light_setting_reads_as_light_and_zero_is_no_preference_elsewhere() {
        assert_eq!(decode(GNOME, &Value::from("default"), true), Some(Some(Theme::Light)));
        // A GNOME schema installed on another desktop doesn't make that desktop light.
        assert_eq!(decode(GNOME, &Value::from("default"), false), Some(None));
        assert_eq!(decode(GNOME, &Value::from("prefer-dark"), false), Some(Some(Theme::Dark)));
        assert_eq!(decode(GNOME, &Value::from("prefer-light"), false), Some(Some(Theme::Light)));
        assert_eq!(decode(STANDARD, &Value::U32(0), true), Some(Some(Theme::Light)));
        assert_eq!(decode(STANDARD, &Value::U32(0), false), Some(None));
        for gnome in [true, false] {
            assert_eq!(decode(STANDARD, &Value::U32(1), gnome), Some(Some(Theme::Dark)));
            assert_eq!(decode(STANDARD, &Value::U32(2), gnome), Some(Some(Theme::Light)));
        }
        let gnome = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            is_gnome_session(&move |k| pairs.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()))
        };
        assert!(gnome(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")]));
        assert!(gnome(&[("XDG_CURRENT_DESKTOP", "Budgie:GNOME")]));
        assert!(gnome(&[("XDG_CURRENT_DESKTOP", "GNOME-Flashback:GNOME")]));
        assert!(gnome(&[("XDG_CURRENT_DESKTOP", " "), ("XDG_SESSION_DESKTOP", "gnome")]));
        assert!(gnome(&[("GNOME_DESKTOP_SESSION_ID", "x")]));
        assert!(!gnome(&[("XDG_CURRENT_DESKTOP", "KDE")]));
        assert!(!gnome(&[("XDG_CURRENT_DESKTOP", "X-Cinnamon"), ("GNOME_DESKTOP_SESSION_ID", "x")]));
        assert!(!gnome(&[("KDE_FULL_SESSION", "true"), ("GNOME_DESKTOP_SESSION_ID", "x")]));
        assert!(!gnome(&[("XDG_CURRENT_DESKTOP", "i3:GNOME-Flashback")]));
        assert!(!gnome(&[("SWAYSOCK", "/run/sway.sock"), ("XDG_CURRENT_DESKTOP", "GNOME")]));
        assert!(!gnome(&[]));
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
