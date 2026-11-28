//! The XDG Settings portal: reading colour scheme, contrast, motion, text scale, button layout and icons, and
//! watching for changes.

use super::files::{kde_factor_reduces, reduced_motion_from_files};
use super::{Contrast, Shared, Theme};
use futures_lite::{StreamExt, future::zip};
use std::{path::PathBuf, sync::Arc, time::Duration};
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
const KDE: &str = "org.kde.kdeglobals.KDE";
const KDE_MOTION_KEY: &str = "AnimationDurationFactor";
// Read on GNOME-like sessions only (`Shared::gnome`): elsewhere the schema's key holds its default
// or a GNOME session's value, not what this desktop applies to its own text.
const TEXT_SCALE_KEY: &str = "text-scaling-factor";
/// The text scales applied; outside them a desktop's factor is no scaling, never clamped.
const TEXT_SCALES: std::ops::RangeInclusive<f64> = 0.5..=3.0;
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
// preference), GNOME's `enable-animations` boolean, inverted, or KDE's `AnimationDurationFactor`
// (0 makes every animation instant). `None` means malformed.
fn decode_motion(namespace: &str, value: &Value<'_>) -> Option<bool> {
    match (namespace, value) {
        (STANDARD, Value::U32(n)) => Some(*n == 1),
        (GNOME, Value::Bool(enabled)) => Some(!enabled),
        (KDE, Value::F64(factor)) => kde_factor_reduces(*factor),
        // KConfig hands an entry read without a type over as its text.
        (KDE, Value::Str(text)) => text.as_str().trim().parse::<f64>().ok().and_then(kde_factor_reduces),
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
// GNOME's `text-scaling-factor`, a double. Outer `None` means malformed; `Some(None)` is a
// factor that is not finite or outside `TEXT_SCALES`, which means no scaling.
fn decode_text_scale(value: &Value<'_>) -> Option<Option<f32>> {
    let Value::F64(factor) = value else { return None };
    Some((factor.is_finite() && TEXT_SCALES.contains(factor)).then_some(*factor as f32))
}
fn text_scale_reply(value: &Value<'_>, legacy: bool) -> Option<Option<f32>> {
    decode_text_scale(unwrap_legacy(value, legacy)?)
}
// `gsettings get` prints the double as text (`1.25`); text that isn't a number is malformed.
fn gsettings_text_scale(text: &str) -> Option<Option<f32>> {
    decode_text_scale(&Value::F64(text.trim().parse().ok()?))
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
/// The one-shot reads used when the portal can't answer, each at most once per watcher: the
/// `gsettings` reads and the `PATH` they search, and the motion settings files under the config
/// folder.
pub(super) struct Fallbacks {
    theme_used: bool,
    layout_used: bool,
    icons_used: bool,
    motion_used: bool,
    text_scale_used: bool,
    path: Option<std::ffi::OsString>,
    /// `$XDG_CONFIG_HOME`, or `~/.config` when that is empty or unset.
    config_home: Option<PathBuf>,
}
impl Fallbacks {
    fn from_env() -> Self {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").filter(|home| !home.is_empty()).map(|home| PathBuf::from(home).join(".config")));
        Self {
            theme_used: false,
            layout_used: false,
            icons_used: false,
            motion_used: false,
            text_scale_used: false,
            path: std::env::var_os("PATH"),
            config_home,
        }
    }
    /// The settings files' motion answer, for a session with no bus or a portal that knows none
    /// of the three motion keys.
    fn motion(&mut self, shared: &Shared) {
        if std::mem::replace(&mut self.motion_used, true) {
            return;
        }
        if let Some(reduced) = self.config_home.as_deref().and_then(reduced_motion_from_files) {
            shared.publish_motion(reduced);
        }
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
    async fn text_scale(&mut self, shared: &Shared) {
        if std::mem::replace(&mut self.text_scale_used, true) || !shared.gnome {
            return;
        }
        if let Some(text) = self.get(GNOME, TEXT_SCALE_KEY).await
            && let Some(answer) = gsettings_text_scale(&text)
        {
            shared.publish_text_scale(answer);
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
        // Motion first: it is a file read, and `wait_first` settles with the theme.
        fallbacks.motion(shared);
        fallbacks.theme(shared).await;
        fallbacks.layout(shared).await;
        fallbacks.icons(shared).await;
        fallbacks.text_scale(shared).await;
        return None;
    };
    watch_connection(shared, fallbacks, connected, connection).await
}
enum Event {
    Setting(zbus::Result<zbus::Message>),
    Owner(zbus::Result<zbus::Message>),
}
/// The latest valid answer from each source of a setting that combines several namespaces.
struct Answers {
    standard: Option<Option<Theme>>,
    gnome: Option<Option<Theme>>,
    motion: Option<bool>,
    gnome_motion: Option<bool>,
    kde_motion: Option<bool>,
}
impl Answers {
    /// The three motion keys as one: any asking for reduced motion wins, then any answering at
    /// all; `None` when none answers.
    fn reduced_motion(&self) -> Option<bool> {
        let answers = [self.motion, self.gnome_motion, self.kde_motion];
        if answers.contains(&Some(true)) { Some(true) } else { answers.into_iter().flatten().next() }
    }
    fn publish_motion(&self, shared: &Shared) {
        if let Some(reduced) = self.reduced_motion() {
            shared.publish_motion(reduced);
        }
    }
    /// Replaces the three motion answers with a fresh read and publishes them.
    fn reread_motion(&mut self, shared: &Shared, (motion, gnome_motion, kde_motion): MotionReads) {
        (self.motion, self.gnome_motion, self.kde_motion) = (motion, gnome_motion, kde_motion);
        self.publish_motion(shared);
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
            (GNOME, TEXT_SCALE_KEY) if shared.gnome => {
                if let Some(scale) = decode_text_scale(value) {
                    shared.publish_text_scale(scale);
                }
            }
            (STANDARD, MOTION_KEY) | (GNOME, ANIMATIONS_KEY) | (KDE, KDE_MOTION_KEY) => {
                let Some(reduced) = decode_motion(namespace, value) else { return };
                match namespace {
                    STANDARD => self.motion = Some(reduced),
                    GNOME => self.gnome_motion = Some(reduced),
                    _ => self.kde_motion = Some(reduced),
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
/// The three motion keys as read: the portal's, GNOME's and KDE's.
type MotionReads = (Option<bool>, Option<bool>, Option<bool>);
async fn read_motion(connection: &zbus::Connection) -> MotionReads {
    let (motion, (gnome_motion, kde_motion)) = zip(
        read_with(connection, STANDARD, MOTION_KEY, |value, legacy| motion_reply(STANDARD, value, legacy)),
        zip(
            read_with(connection, GNOME, ANIMATIONS_KEY, |value, legacy| motion_reply(GNOME, value, legacy)),
            read_with(connection, KDE, KDE_MOTION_KEY, |value, legacy| motion_reply(KDE, value, legacy)),
        ),
    )
    .await;
    (motion, gnome_motion, kde_motion)
}
fn is_motion_key(namespace: &str, key: &str) -> bool {
    matches!((namespace, key), (STANDARD, MOTION_KEY) | (GNOME, ANIMATIONS_KEY) | (KDE, KDE_MOTION_KEY))
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
/// GNOME's text scale, read on GNOME-like sessions only (`TEXT_SCALE_KEY`).
async fn read_text_scale(connection: &zbus::Connection, gnome: bool) -> Option<Option<f32>> {
    if gnome { read_with(connection, GNOME, TEXT_SCALE_KEY, text_scale_reply).await } else { None }
}
/// Publishes the contrast, motion and text scale of the first concurrent read.
fn publish_first_reads(shared: &Shared, fallbacks: &mut Fallbacks, contrast: Option<Contrast>, answers: &Answers, text_scale: Option<Option<f32>>) {
    if let Some(contrast) = contrast {
        shared.publish_contrast(contrast);
    }
    match answers.reduced_motion() {
        Some(reduced) => shared.publish_motion(reduced),
        // A portal that knows none of the keys (a bare window manager's) leaves the files to answer.
        None => fallbacks.motion(shared),
    }
    if let Some(scale) = text_scale {
        shared.publish_text_scale(scale);
    }
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
        subscribe(&connection, rule(KDE_MOTION_KEY), 8).await?,
        subscribe(&connection, rule(TEXT_SCALE_KEY), 8).await?,
    ];
    let mut owners = subscribe(&connection, owner_rule(), 8).await?;
    // Concurrent reads, so contrast, motion and text scale are known by the time the theme
    // settles `wait_first` without adding round trips before it.
    let (((standard, gnome), (contrast, (motion, gnome_motion, kde_motion))), text_scale) = zip(
        zip(
            zip(read(&connection, STANDARD, shared.gnome), read(&connection, GNOME, shared.gnome)),
            zip(read_with(&connection, STANDARD, CONTRAST_KEY, contrast_reply), read_motion(&connection)),
        ),
        read_text_scale(&connection, shared.gnome),
    )
    .await;
    let mut answers = Answers { standard, gnome, motion, gnome_motion, kde_motion };
    publish_first_reads(shared, fallbacks, contrast, &answers, text_scale);
    match combined(standard, gnome) {
        Some(answer) => {
            *connected = true;
            shared.publish(answer);
        }
        None => {
            fallbacks.theme(shared).await;
            fallbacks.layout(shared).await;
            fallbacks.icons(shared).await;
            fallbacks.text_scale(shared).await;
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
    // A portal without GNOME's keys answered nothing for the text scale either.
    if text_scale.is_none() {
        fallbacks.text_scale(shared).await;
    }
    follow(shared, &connection, &mut settings, &mut owners, &mut answers).await
}
/// Follows the settings after the first read until the portal's owner goes away or a stream ends.
async fn follow(
    shared: &Shared,
    connection: &zbus::Connection,
    settings: &mut [zbus::MessageStream],
    owners: &mut zbus::MessageStream,
    answers: &mut Answers,
) -> Option<()> {
    loop {
        let message = match next_event(settings, owners).await? {
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
            if is_motion_key(&namespace, &key) {
                // KDE announces a new animation speed only as `reduced-motion`, and its own key has
                // no value while the speed is at its default, so all three keys are read again
                // rather than kept at what they last said; one answer is published per change.
                answers.reread_motion(shared, read_motion(connection).await);
            } else {
                answers.apply(shared, &namespace, &key, &value);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{ANIMATIONS_KEY, Answers, GNOME, KDE, KDE_MOTION_KEY, MOTION_KEY, STANDARD, decode, decode_motion};
    use crate::appearance::is_gnome_session;
    use crate::appearance::{Theme, Watcher};
    use std::time::Duration;
    use zbus::zvariant::Value;
    // The portal's, GNOME's and KDE's keys each decode to reduced or not; malformed values say
    // nothing. Combined, any one asking for reduced motion wins, all answering otherwise is not
    // reduced, and none answering publishes nothing.
    #[test]
    fn motion_answers_combine_any_off_wins() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(decode_motion(STANDARD, &Value::U32(1)), Some(true));
        assert_eq!(decode_motion(STANDARD, &Value::U32(0)), Some(false));
        assert_eq!(decode_motion(STANDARD, &Value::U32(2)), Some(false), "a value not defined yet is no preference");
        assert_eq!(decode_motion(GNOME, &Value::Bool(false)), Some(true), "GNOME's switch is inverted");
        assert_eq!(decode_motion(GNOME, &Value::Bool(true)), Some(false));
        assert_eq!(decode_motion(KDE, &Value::F64(0.0)), Some(true), "KDE's factor 0 makes animations instant");
        assert_eq!(decode_motion(KDE, &Value::F64(0.5)), Some(false));
        assert_eq!(decode_motion(KDE, &Value::from("0")), Some(true));
        for malformed in [Value::F64(f64::NAN), Value::F64(f64::INFINITY), Value::from("fast"), Value::Bool(false)] {
            assert_eq!(decode_motion(KDE, &malformed), None, "{malformed:?} is malformed");
        }
        assert_eq!(decode_motion(STANDARD, &Value::Bool(true)), None);
        assert_eq!(decode_motion(GNOME, &Value::U32(0)), None);

        let (send, sent) = std::sync::mpsc::channel();
        let watcher = Watcher::spawn(
            false,
            || {},
            move |shared| async move {
                let _ = send.send(shared);
            },
        )
        .ok_or("worker creation failed")?;
        let shared = sent.recv_timeout(Duration::from_secs(2))?;
        let mut answers = Answers { standard: None, gnome: None, motion: None, gnome_motion: None, kde_motion: None };
        answers.apply(&shared, KDE, "SingleClick", &Value::F64(0.0));
        answers.apply(&shared, KDE, KDE_MOTION_KEY, &Value::from("fast"));
        assert_eq!(watcher.reduced_motion(), None, "no key answered, so nothing is published");

        let keys = [
            (STANDARD, MOTION_KEY, Value::U32(1), Value::U32(0)),
            (GNOME, ANIMATIONS_KEY, Value::Bool(false), Value::Bool(true)),
            (KDE, KDE_MOTION_KEY, Value::F64(0.0), Value::F64(1.0)),
        ];
        for (namespace, key, _, not_reduced) in &keys {
            answers.apply(&shared, namespace, key, not_reduced);
        }
        assert_eq!(watcher.reduced_motion(), Some(false), "every key answering 'not reduced' is not reduced");
        for (namespace, key, reduced, not_reduced) in &keys {
            answers.apply(&shared, namespace, key, reduced);
            assert_eq!(watcher.reduced_motion(), Some(true), "{namespace} {key} alone asking for reduced motion wins");
            answers.apply(&shared, namespace, key, not_reduced);
            assert_eq!(watcher.reduced_motion(), Some(false), "{namespace} {key} back to 'not reduced'");
        }
        Ok(())
    }
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
    // A desktop's text scale is applied only when it is a usable factor: Large Text's 1.25 and the
    // ends of the range are, while a factor that isn't finite or lies outside 0.5..=3.0 is no
    // scaling (not a clamped one), and text `gsettings` prints that isn't a number is no answer.
    #[test]
    fn gnome_text_scaling_factor_reads_in_range_or_not_at_all() {
        use super::{decode_text_scale, gsettings_text_scale, text_scale_reply};
        for (text, scale) in [("1.25\n", 1.25), ("1.0", 1.0), ("0.5", 0.5), ("3.0", 3.0)] {
            assert_eq!(gsettings_text_scale(text), Some(Some(scale)), "{text}");
            assert_eq!(decode_text_scale(&Value::F64(f64::from(scale))), Some(Some(scale)), "{text}");
        }
        for out_of_range in ["nan", "inf", "-2", "0", "0.49", "3.01", "100"] {
            assert_eq!(gsettings_text_scale(out_of_range), Some(None), "{out_of_range}");
        }
        for not_a_number in ["", "uint32 0", "'1.25'", "1,25"] {
            assert_eq!(gsettings_text_scale(not_a_number), None, "{not_a_number}");
        }
        // The portal answers a double; any other type is malformed. The legacy `Read` wraps it once more.
        assert_eq!(decode_text_scale(&Value::U32(1)), None);
        assert_eq!(decode_text_scale(&Value::from("1.25")), None);
        assert_eq!(text_scale_reply(&Value::Value(Box::new(Value::F64(1.5))), true), Some(Some(1.5)));
        assert_eq!(text_scale_reply(&Value::F64(1.5), true), None);
    }
    #[test]
    fn the_generation_rises_once_per_appearance_change_and_never_for_the_layout() -> Result<(), Box<dyn std::error::Error>> {
        use crate::appearance::{Contrast, SystemReader};
        let (done, finished) = std::sync::mpsc::channel();
        let watcher = Watcher::spawn(
            false,
            || {},
            move |shared| async move {
                shared.publish(Some(Theme::Dark));
                shared.publish(Some(Theme::Dark));
                shared.publish_contrast(Contrast::High);
                shared.publish_contrast(Contrast::High);
                shared.publish_motion(true);
                shared.publish_text_scale(Some(1.25));
                shared.publish_text_scale(Some(1.25));
                shared.publish_layout("appmenu:close".to_owned());
                let _ = done.send(());
            },
        )
        .ok_or("worker creation failed")?;
        finished.recv_timeout(Duration::from_secs(2))?;
        assert_eq!(watcher.generation(), 4, "theme, contrast, motion and text scale each changed once");
        assert_eq!((watcher.current(), watcher.contrast()), (Some(Theme::Dark), Some(Contrast::High)));
        assert_eq!(watcher.text_scale(), Some(1.25));

        let reader = SystemReader::unwatched();
        let before = reader.generation();
        reader.report_theme(Theme::Dark);
        reader.report_theme(Theme::Dark);
        assert_eq!(reader.generation(), before + 1, "a repeated report is no change");
        assert_eq!(reader.read().theme, Some(Theme::Dark));
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
