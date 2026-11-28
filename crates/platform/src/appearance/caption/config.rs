//! The window managers' configuration files and GTK's settings file. None of them announces a change, so each is
//! looked at (its size, times, inode and permissions) once a second while the app is active, every ten seconds
//! otherwise and at once when the app becomes active, and read again only when that changed. A file replaced by a
//! rename, removed, made again or made unreadable is seen as such. A file over 1 MiB, or not UTF-8, is no answer
//! rather than a cut-off one. A file is opened without waiting and read only when it is a regular file: a FIFO with no
//! writer, a device or a socket in its place is no answer, and holds up neither this worker's other sources nor the
//! watcher's end.
//!
//! | Window manager | Files, by its documented search | Value |
//! |---|---|---|
//! | KWin | `kwinrc` in the user's folder, then each `$XDG_CONFIG_DIRS` (KDE's cascade and locks) | `ButtonsOnLeft`, `ButtonsOnRight` |
//! | xfwm4 | `xfce4/xfconf/xfce-perchannel-xml/xfwm4.xml`, the user's then the system's | `general` `button_layout`, as Xfconf last saved it |
//! | Openbox | the first of `openbox/rc.xml` in the user's folder and `$XDG_CONFIG_DIRS` | `<theme><titleLayout>` |
//! | labwc | the first of `labwc/rc.xml` in the user's folder and `$XDG_CONFIG_DIRS` | `<theme><titlebar><layout>` |
//! | Budgie on labwc | `budgie-desktop/labwc/rc.xml`, which its labwc bridge writes | the same |
//! | any | `gtk-3.0/settings.ini`, the user's then the system's | `[Settings] gtk-decoration-layout` |
//!
//! Where no file of KWin's, Openbox's or labwc's exists, the window manager runs on its built-in defaults, which are
//! published as its layout; in a sandbox a missing file says nothing (it may only be out of the sandbox's reach), and
//! system folders are the runtime's, so only the user's files are read. A window manager started with another
//! configuration file (Openbox's `--config-file`, labwc's `-C`) is not followed, except Budgie's known folder.

use super::super::files::{ini_value, read_text};
use super::super::{LayoutSource, RawLayout, Shared};
use super::Env;
use craft_host::caption_layout::{LayoutDesktop, LayoutReading, MAX_LAYOUT_BYTES, kwin_layout};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The largest configuration file read.
const CAP: u64 = 1 << 20;
const ACTIVE: Duration = Duration::from_secs(1);
const INACTIVE: Duration = Duration::from_secs(10);

/// What a file source reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// KWin's `kwinrc`, every file of the cascade.
    Kwin,
    /// xfwm4's channel file, the first that has the property.
    Xfwm,
    /// Openbox's `rc.xml`, the first that exists.
    Openbox,
    /// labwc's `rc.xml`, the first that exists.
    Labwc,
    /// GTK's `settings.ini`, the first that has the key.
    Gtk,
}

/// One source and the files it reads, most important first.
#[derive(Debug)]
pub(super) struct Watched {
    kind: Kind,
    paths: Vec<PathBuf>,
    /// The layout when no file exists: the window manager's built-in default, outside a sandbox.
    default: Option<RawLayout>,
}

impl Watched {
    fn source(&self) -> LayoutSource {
        if self.kind == Kind::Gtk { LayoutSource::GtkConfig } else { LayoutSource::WmConfig }
    }
}

/// The files `desktop`'s window manager and GTK keep the layout in, for `env`.
pub(super) fn watched(env: &Env, desktop: LayoutDesktop) -> Vec<Watched> {
    let Some(home) = env.config_home.as_deref() else { return Vec::new() };
    let all =
        |relative: &str| std::iter::once(home).chain(env.config_dirs.iter().map(PathBuf::as_path)).map(|dir| dir.join(relative)).collect::<Vec<_>>();
    let built_in = |raw: RawLayout| (!env.sandboxed).then_some(raw);
    let mut sources = Vec::new();
    match desktop {
        LayoutDesktop::Plasma | LayoutDesktop::Kwin => {
            sources.push(Watched { kind: Kind::Kwin, paths: all("kwinrc"), default: built_in(RawLayout::Kwin { left: None, right: None }) })
        }
        LayoutDesktop::Xfce | LayoutDesktop::Xfwm => {
            sources.push(Watched { kind: Kind::Xfwm, paths: all("xfce4/xfconf/xfce-perchannel-xml/xfwm4.xml"), default: None })
        }
        LayoutDesktop::Openbox => {
            sources.push(Watched { kind: Kind::Openbox, paths: all("openbox/rc.xml"), default: built_in(RawLayout::Openbox("NLIMC".into())) });
        }
        LayoutDesktop::Labwc => {
            sources.push(Watched {
                kind: Kind::Labwc,
                paths: all("labwc/rc.xml"),
                default: built_in(RawLayout::Labwc("icon:iconify,max,close".into())),
            });
        }
        LayoutDesktop::BudgieLabwc => {
            sources.push(Watched { kind: Kind::Labwc, paths: vec![home.join("budgie-desktop/labwc/rc.xml")], default: None })
        }
        _ => {}
    }
    sources.push(Watched { kind: Kind::Gtk, paths: all("gtk-3.0/settings.ini"), default: None });
    sources
}

/// Follows the files of `desktop` until the watcher is dropped.
pub(super) async fn watch(shared: &Shared, env: &Env, desktop: LayoutDesktop) {
    let sources = watched(env, desktop);
    let mut stamps: Vec<Option<Vec<Stamp>>> = vec![None; sources.len()];
    loop {
        for (source, last) in sources.iter().zip(&mut stamps) {
            let stamp: Vec<Stamp> = source.paths.iter().map(|path| Stamp::of(path)).collect();
            if last.as_ref() != Some(&stamp) {
                *last = Some(stamp);
                shared.publish_caption(source.source(), read(source));
            }
        }
        let wait = if shared.active.load(Ordering::Acquire) { ACTIVE } else { INACTIVE };
        futures_lite::future::race(
            async {
                async_io::Timer::after(wait).await;
            },
            async {
                let _ = shared.nudge.1.recv().await;
            },
        )
        .await;
    }
}

/// What a file's metadata says, to tell that it changed: its size, change times (to the nanosecond), inode (a rename
/// over it is a new one) and permissions. `None` when it does not exist.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp(Option<Meta>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Meta {
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    file: (u64, u64),
    mode: u32,
}

impl Stamp {
    fn of(path: &Path) -> Stamp {
        Stamp(std::fs::metadata(path).ok().map(|m| Meta {
            len: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            file: (m.dev(), m.ino()),
            mode: m.mode(),
        }))
    }
}

/// A file's text, at most [`CAP`], read as every settings file on the worker is (`files::read_text`): `Ok(None)` when it
/// does not exist, `Err` when it exists but can't be read whole, a FIFO or another file that is not a regular one too,
/// whose open does not wait.
fn read_file(path: &Path) -> Result<Option<String>, ()> {
    read_text(path, CAP)
}

/// The source's layout now, `None` when its files give none.
fn read(source: &Watched) -> Option<LayoutReading> {
    let reading = |raw: RawLayout, origin: String| Some(LayoutReading::new(source.source(), raw, origin));
    let defaulted = |what: &str| source.default.clone().and_then(|raw| reading(raw, format!("{what}'s built-in default: no configuration file")));
    match source.kind {
        Kind::Kwin => {
            // KDE reads the least important file first; one that exists but can't be read leaves the value unknown.
            let mut texts = Vec::new();
            for path in source.paths.iter().rev() {
                texts.extend(read_file(path).ok()?);
            }
            if texts.is_empty() {
                return defaulted("KWin");
            }
            let user = source.paths.first().map_or_else(String::new, |path| path.display().to_string());
            reading(kwin_layout(&texts.iter().map(String::as_str).collect::<Vec<_>>()), format!("{user} and the system's kwinrc"))
        }
        Kind::Xfwm => source.paths.iter().find_map(|path| {
            let value = xfwm_channel_layout(&read_file(path).ok()??)?;
            reading(RawLayout::Xfwm(value), format!("{} (as Xfconf last saved it)", path.display()))
        }),
        Kind::Openbox | Kind::Labwc => {
            let what = if source.kind == Kind::Openbox { "Openbox" } else { "labwc" };
            // The first file that exists is the one in force; one that can't be read leaves the value unknown.
            let Some((path, text)) = source.paths.iter().find_map(|path| read_file(path).transpose().map(|text| (path, text))) else {
                return defaulted(what);
            };
            let text = text.ok()?;
            let raw = if source.kind == Kind::Openbox {
                RawLayout::Openbox(openbox_title_layout(&text)?.unwrap_or_else(|| "NLIMC".into()))
            } else {
                RawLayout::Labwc(labwc_title_layout(&text)?.unwrap_or_else(|| "icon:iconify,max,close".into()))
            };
            reading(raw, path.display().to_string())
        }
        Kind::Gtk => source.paths.iter().find_map(|path| {
            let text = read_file(path).ok()??;
            let value = ini_value(&text, "Settings", "gtk-decoration-layout")?;
            (value.len() <= MAX_LAYOUT_BYTES).then(|| reading(RawLayout::Names(value.to_owned()), path.display().to_string())).flatten()
        }),
    }
}

/// Parses `text` as XML with no DTD (so no entities) and at most 65 536 nodes; `None` for anything else.
fn xml(text: &str) -> Option<roxmltree::Document<'_>> {
    let options = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 65_536, ..roxmltree::ParsingOptions::default() };
    roxmltree::Document::parse_with_options(text, options).ok()
}

/// The element child of `node` named `name` (namespaces aside).
fn child<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| child.is_element() && child.tag_name().name() == name)
}

/// An element's text, trimmed, at most `MAX_LAYOUT_BYTES` long; empty for an empty element.
fn text_of(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let text = node.text().unwrap_or_default().trim();
    (text.len() <= MAX_LAYOUT_BYTES).then(|| text.to_owned())
}

/// Openbox's `<theme><titleLayout>`: `None` for a file that is not XML or a value too long, `Some(None)` when it sets
/// none (Openbox's default applies).
pub(super) fn openbox_title_layout(text: &str) -> Option<Option<String>> {
    let document = xml(text)?;
    match child(document.root_element(), "theme").and_then(|theme| child(theme, "titleLayout")) {
        Some(layout) => text_of(layout).map(Some),
        None => Some(None),
    }
}

/// labwc's `<theme><titlebar><layout>`: as [`openbox_title_layout`]; a value without its one mandatory colon is
/// malformed.
pub(super) fn labwc_title_layout(text: &str) -> Option<Option<String>> {
    let document = xml(text)?;
    match child(document.root_element(), "theme").and_then(|theme| child(theme, "titlebar")).and_then(|bar| child(bar, "layout")) {
        Some(layout) => text_of(layout).filter(|layout| layout.matches(':').count() == 1).map(Some),
        None => Some(None),
    }
}

/// `button_layout` of the `general` property of an Xfconf channel file for `xfwm4`.
pub(super) fn xfwm_channel_layout(text: &str) -> Option<String> {
    let document = xml(text)?;
    let channel = document.root_element();
    if channel.tag_name().name() != "channel" || channel.attribute("name") != Some("xfwm4") {
        return None;
    }
    let value = xfconf_property(xfconf_property(channel, "general")?, "button_layout")?.attribute("value")?;
    (value.len() <= MAX_LAYOUT_BYTES).then(|| value.trim().to_owned())
}

/// The `<property name="name">` child of an Xfconf channel file's `node`.
fn xfconf_property<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| child.is_element() && child.tag_name().name() == "property" && child.attribute("name") == Some(name))
}

/// LXQt's chosen window manager, `[General] window_manager` of its `session.conf`, the user's then the system's; `None`
/// outside LXQt.
pub(super) fn lxqt_window_manager(env: &Env) -> Option<String> {
    let lxqt = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .any(|names| names.split(':').any(|name| name.trim().eq_ignore_ascii_case("lxqt")));
    if !lxqt {
        return None;
    }
    let dirs = env.config_home.iter().chain(&env.config_dirs).cloned().chain((!env.sandboxed).then(|| PathBuf::from("/usr/share")));
    dirs.filter_map(|dir| read_file(&dir.join("lxqt/session.conf")).ok().flatten())
        .find_map(|text| ini_value(&text, "General", "window_manager").filter(|wm| !wm.is_empty()).map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::{Env, labwc_title_layout, openbox_title_layout, watch, watched, xfwm_channel_layout};
    use crate::appearance::fake_bus::eventually;
    use crate::appearance::{LayoutSource, RawLayout, Shared, Watcher};
    use craft_host::caption_layout::LayoutDesktop;
    use std::path::Path;

    fn env(home: &Path, system: &Path, sandboxed: bool) -> Env {
        Env {
            sandboxed,
            config_home: Some(home.to_owned()),
            config_dirs: if sandboxed { Vec::new() } else { vec![system.to_owned()] },
            path: None,
            x11: false,
        }
    }

    fn watcher(env: Env, desktop: LayoutDesktop) -> Option<Watcher> {
        Watcher::spawn_shared(Shared::new(true, Box::new(|| {})), move |shared| async move {
            watch(&shared, &env, desktop).await;
        })
    }

    fn layout(watcher: &Watcher) -> Option<RawLayout> {
        watcher.caption_layout().map(|reading| reading.raw)
    }

    fn kwin(left: Option<&str>, right: Option<&str>) -> Option<RawLayout> {
        Some(RawLayout::Kwin { left: left.map(str::to_owned), right: right.map(str::to_owned) })
    }

    /// Writes `text` to `path` the way settings tools do: to a new file, then renamed over the old one.
    fn replace(path: &Path, text: &str) -> std::io::Result<()> {
        let fresh = path.with_extension("new");
        std::fs::write(&fresh, text)?;
        std::fs::rename(fresh, path)
    }

    // KWin's file is followed while the app runs, with no focus change: a change replaced by a rename, the user's file
    // removed (the system's then answers), every file gone (KWin's own defaults). The user's file wins over the
    // system's. A few seconds: each step waits for the once-a-second look at the files.
    #[test]
    fn kwinrc_is_followed_while_the_app_runs() -> Result<(), Box<dyn std::error::Error>> {
        let home = crate::temp::TempDir::new("craft-caption-home")?;
        let system = crate::temp::TempDir::new("craft-caption-system")?;
        let group = "[org.kde.kdecoration2]\n";
        std::fs::write(system.path().join("kwinrc"), format!("{group}ButtonsOnLeft=X\nButtonsOnRight=IA\n"))?;
        std::fs::write(home.path().join("kwinrc"), format!("{group}ButtonsOnRight=IAX\n"))?;
        let watcher = watcher(env(home.path(), system.path(), false), LayoutDesktop::Plasma).ok_or("worker creation failed")?;
        assert!(eventually(|| layout(&watcher) == kwin(Some("X"), Some("IAX"))), "the user's key wins, the system's fills in");
        replace(&home.path().join("kwinrc"), &format!("{group}ButtonsOnLeft=\nButtonsOnRight=X\n"))?;
        assert!(eventually(|| layout(&watcher) == kwin(Some(""), Some("X"))), "a renamed-over file is read again");
        std::fs::remove_file(home.path().join("kwinrc"))?;
        assert!(eventually(|| layout(&watcher) == kwin(Some("X"), Some("IA"))), "without the user's file the system's answers");
        std::fs::remove_file(system.path().join("kwinrc"))?;
        assert!(eventually(|| layout(&watcher) == kwin(None, None)), "with no file KWin's defaults stand");
        Ok(())
    }

    // A sandbox reads only the user's files, and a file it can't see says nothing: no system default, no built-in
    // default taken for the host's setting. About a second: the file's arrival waits for the next look.
    #[test]
    fn a_sandbox_reads_only_the_users_files_and_takes_no_default_for_the_hosts_setting() -> Result<(), Box<dyn std::error::Error>> {
        let home = crate::temp::TempDir::new("craft-caption-home")?;
        let system = crate::temp::TempDir::new("craft-caption-system")?;
        std::fs::write(system.path().join("kwinrc"), "[org.kde.kdecoration2]\nButtonsOnLeft=X\n")?;
        std::fs::create_dir_all(home.path().join("gtk-3.0"))?;
        std::fs::write(home.path().join("gtk-3.0/settings.ini"), "[Settings]\ngtk-decoration-layout=menu:close\n")?;
        let sandboxed = env(home.path(), system.path(), true);
        assert!(watched(&sandboxed, LayoutDesktop::Plasma).iter().all(|source| source.paths.iter().all(|path| path.starts_with(home.path()))));
        let watcher = watcher(sandboxed, LayoutDesktop::Openbox).ok_or("worker creation failed")?;
        // GTK's file is read in the same look as Openbox's would be, and ranks after it.
        assert!(eventually(|| watcher.caption_layout().is_some()), "the files were looked at");
        assert_eq!(
            watcher.caption_layout().map(|reading| reading.source),
            Some(LayoutSource::GtkConfig),
            "Openbox's built-in default is not taken for the host's"
        );
        std::fs::create_dir_all(home.path().join("openbox"))?;
        std::fs::write(home.path().join("openbox/rc.xml"), "<openbox_config><theme><titleLayout>CLIM</titleLayout></theme></openbox_config>")?;
        assert!(eventually(|| layout(&watcher) == Some(RawLayout::Openbox("CLIM".into()))), "a file given to the sandbox is read");
        Ok(())
    }

    // Openbox's and labwc's first file is the one in force; GTK's file stands in after the window manager's, and a
    // file that is too big, not a file or not XML is no answer. A few seconds: each step waits for the next look.
    #[test]
    fn the_window_managers_files_rank_before_gtks_and_bad_files_are_no_answer() -> Result<(), Box<dyn std::error::Error>> {
        let home = crate::temp::TempDir::new("craft-caption-home")?;
        let system = crate::temp::TempDir::new("craft-caption-system")?;
        for dir in [home.path(), system.path()] {
            std::fs::create_dir_all(dir.join("labwc"))?;
            std::fs::create_dir_all(dir.join("gtk-3.0"))?;
        }
        std::fs::write(
            system.path().join("labwc/rc.xml"),
            "<labwc_config><theme><titlebar><layout>close:</layout></titlebar></theme></labwc_config>",
        )?;
        std::fs::write(home.path().join("gtk-3.0/settings.ini"), "[Settings]\ngtk-decoration-layout=close,minimize:maximize\n")?;
        let watcher = watcher(env(home.path(), system.path(), false), LayoutDesktop::Labwc).ok_or("worker creation failed")?;
        assert!(eventually(|| layout(&watcher) == Some(RawLayout::Labwc("close:".into()))), "the system's file, the first that exists");
        std::fs::write(
            home.path().join("labwc/rc.xml"),
            "<labwc_config><theme><titlebar><layout>:iconify,close</layout></titlebar></theme></labwc_config>",
        )?;
        assert!(eventually(|| layout(&watcher) == Some(RawLayout::Labwc(":iconify,close".into()))), "the user's file is the first");
        std::fs::write(home.path().join("labwc/rc.xml"), format!("<labwc_config>{}</labwc_config>", " ".repeat(1 << 20)))?;
        assert!(
            eventually(|| watcher.caption_layout().map(|reading| reading.source) == Some(LayoutSource::GtkConfig)),
            "a file over 1 MiB is no answer"
        );
        assert_eq!(layout(&watcher), Some(RawLayout::Names("close,minimize:maximize".into())));
        std::fs::remove_file(home.path().join("labwc/rc.xml"))?;
        std::fs::create_dir(home.path().join("labwc/rc.xml"))?;
        // GTK's file changes too, so its new value shows that the files were looked at again.
        std::fs::write(home.path().join("gtk-3.0/settings.ini"), "[Settings]\ngtk-decoration-layout=close:\n")?;
        let gtk = || watcher.caption_layout().filter(|reading| reading.source == LayoutSource::GtkConfig).map(|reading| reading.raw);
        assert!(
            eventually(|| gtk() == Some(RawLayout::Names("close:".into()))),
            "a folder in its place is no answer: {:?}",
            watcher.caption_layout()
        );
        Ok(())
    }

    /// Ends a FIFO's open, which a reader blocked in waits on, when the sources were not dropped; see the trial below.
    struct Unblock {
        fifo: std::path::PathBuf,
        dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for Unblock {
        fn drop(&mut self) {
            if !self.dropped.load(std::sync::atomic::Ordering::Acquire) {
                // A reader waits in its open: this one completes it, and the reader sees the end of the file.
                let _ = std::fs::OpenOptions::new().write(true).open(&self.fifo);
            }
        }
    }

    /// Sets its flag when dropped, as the worker drops the sources it runs.
    struct Dropped(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }

    // A FIFO with no writer in a configuration file's place is no answer and holds nothing up: its open does not wait,
    // the next file is read, another source on the same worker goes on, and dropping the watcher ends the sources.
    #[test]
    fn a_fifo_in_a_configuration_files_place_holds_up_no_source() -> Result<(), Box<dyn std::error::Error>> {
        use crate::appearance::Theme;
        let home = crate::temp::TempDir::new("craft-caption-home")?;
        let system = crate::temp::TempDir::new("craft-caption-system")?;
        let fifo = home.path().join("kwinrc");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status()?.success(), "a FIFO made");
        std::fs::create_dir_all(home.path().join("gtk-3.0"))?;
        std::fs::write(home.path().join("gtk-3.0/settings.ini"), "[Settings]\ngtk-decoration-layout=close:minimize\n")?;
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _unblock = Unblock { fifo, dropped: std::sync::Arc::clone(&dropped) };
        let (flag, env) = (Dropped(std::sync::Arc::clone(&dropped)), env(home.path(), system.path(), false));
        let watcher = Watcher::spawn_shared(Shared::new(true, Box::new(|| {})), move |shared| async move {
            let _flag = flag;
            let other = async {
                async_io::Timer::after(std::time::Duration::from_millis(20)).await;
                shared.publish(Some(Theme::Dark));
                std::future::pending::<()>().await;
            };
            futures_lite::future::zip(watch(&shared, &env, LayoutDesktop::Kwin), other).await;
        })
        .ok_or("worker creation failed")?;
        assert!(eventually(|| watcher.current() == Some(Theme::Dark)), "another source on the worker goes on");
        assert!(eventually(|| layout(&watcher) == Some(RawLayout::Names("close:minimize".into()))), "the next file: {:?}", watcher.caption_layout());
        drop(watcher);
        assert!(eventually(|| dropped.load(std::sync::atomic::Ordering::Acquire)), "dropping the watcher ended the sources");
        Ok(())
    }

    #[test]
    fn the_xml_values_are_read_by_element_and_hostile_files_are_refused() {
        assert_eq!(
            openbox_title_layout(
                "<openbox_config xmlns=\"http://openbox.org/3.4/rc\"><theme><titleLayout> NLIMC </titleLayout></theme></openbox_config>"
            ),
            Some(Some("NLIMC".into()))
        );
        assert_eq!(openbox_title_layout("<openbox_config><theme><titleLayout/></theme></openbox_config>"), Some(Some(String::new())), "no button");
        assert_eq!(openbox_title_layout("<openbox_config><theme/></openbox_config>"), Some(None), "unset: Openbox's default");
        assert_eq!(
            labwc_title_layout("<labwc_config><theme><titlebar><layout>icon:iconify,max,close</layout></titlebar></theme></labwc_config>"),
            Some(Some("icon:iconify,max,close".into()))
        );
        assert_eq!(
            labwc_title_layout("<labwc_config><theme><titlebar><layout>close,max</layout></titlebar></theme></labwc_config>"),
            None,
            "labwc's colon is mandatory"
        );
        assert_eq!(labwc_title_layout("<labwc_config/>"), Some(None));
        let channel = "<channel name=\"xfwm4\" version=\"1.0\"><property name=\"general\" type=\"empty\"><property name=\"theme\" type=\"string\" value=\"Default\"/><property name=\"button_layout\" type=\"string\" value=\"O|HMC\"/></property></channel>";
        assert_eq!(xfwm_channel_layout(channel).as_deref(), Some("O|HMC"));
        assert_eq!(xfwm_channel_layout(&channel.replace("xfwm4", "xsettings")), None, "another channel's file");
        let entity = "<!DOCTYPE x [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;\">]><openbox_config><theme><titleLayout>&b;</titleLayout></theme></openbox_config>";
        for hostile in [entity, "<openbox_config><theme><titleLayout>NLIMC</titleLayout>", "not xml", ""] {
            assert_eq!(openbox_title_layout(hostile), None, "{hostile}");
        }
        let long = format!("<openbox_config><theme><titleLayout>{}</titleLayout></theme></openbox_config>", "C".repeat(5000));
        assert_eq!(openbox_title_layout(&long), None, "longer than a layout");
    }
}
