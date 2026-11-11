//! Why a file dialog may never appear on Linux.
//!
//! `rfd` talks to the XDG file-chooser portal through `libdbus-1`, which it loads at runtime, and
//! falls back to `zenity`. When neither works (a minimal window-manager session without a portal
//! backend) it logs the failure and answers "nothing picked", which an app would take for Cancel:
//! the menu item does nothing and says nothing. [`start`] looks for the pieces once, in the
//! background, and [`known_problem`] returns the cause when a dialog came back empty and none of
//! them is there.
//!
//! The probe only ever explains an empty answer; it never stops a dialog from opening. Anything
//! it can't tell counts as present, so a working system never sees the cause.

// Only Linux probes; the types and helpers stay testable on every platform.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::{Once, OnceLock};

/// What the probe found. `None` means it couldn't tell, which counts as present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Findings {
    /// `libdbus-1.so.3` is in the linker cache (`rfd` loads it for the portal).
    libdbus: Option<bool>,
    /// The session bus has the portal's `FileChooser` interface (a portal and a backend for it).
    file_chooser_portal: Option<bool>,
    /// `zenity` is on `PATH` (`rfd`'s fallback).
    zenity: bool,
}

/// The technical cause for `findings`, or `None` when some file dialog should work.
// pc: apps/photocraft/src/file_dialog_probe.rs:30 problem
fn problem(findings: Findings) -> Option<String> {
    let portal = findings.libdbus != Some(false) && findings.file_chooser_portal != Some(false);
    if portal || findings.zenity {
        return None;
    }
    let cause = if findings.libdbus == Some(false) { "libdbus is missing" } else { "no desktop portal offers a file chooser" };
    Some(cause.to_owned())
}

static RESULT: OnceLock<Option<String>> = OnceLock::new();

/// The cause found by [`start`], once it has finished.
// pc: apps/photocraft/src/file_dialog_probe.rs:44 known_problem
pub(crate) fn known_problem() -> Option<String> {
    RESULT.get().cloned().flatten()
}

/// Whether `program` is an executable file in one of the `path` directories.
// pc: apps/photocraft/src/file_dialog_probe.rs:50 on_path
fn on_path(program: &str, path: &std::ffi::OsStr) -> bool {
    std::env::split_paths(path).any(|dir| dir.join(program).is_file())
}

/// Looks for the file dialog backends on a helper thread (the portal call can take a moment).
/// Runs once per process, and only on Linux.
// pc: apps/photocraft/src/file_dialog_probe.rs:56 start
pub(crate) fn start() {
    #[cfg(target_os = "linux")]
    {
        static STARTED: Once = Once::new();
        STARTED.call_once(|| {
            let spawned = std::thread::Builder::new().name("file dialog probe".into()).spawn(|| {
                let findings = Findings {
                    libdbus: linker_cache_has("libdbus-1.so.3"),
                    file_chooser_portal: file_chooser_portal(),
                    zenity: std::env::var_os("PATH").is_some_and(|path| on_path("zenity", &path)),
                };
                let _ = RESULT.set(problem(findings));
            });
            if let Err(e) = spawned {
                eprintln!("craft-dialogs: couldn't start the file dialog probe: {e}");
            }
        });
    }
}

/// The `ldconfig -p` architecture tag for this build, when it is one we can tell apart from a
/// 32-bit library of the same name (`libc6,x86-64` against plain `libc6` for i386).
fn ldconfig_arch_tag() -> Option<&'static str> {
    if cfg!(target_arch = "x86_64") {
        Some("x86-64")
    } else if cfg!(target_arch = "aarch64") {
        Some("AArch64")
    } else {
        None
    }
}

/// Whether `ldconfig -p` output lists `soname` for `arch_tag`. Lines look like
/// `\tlibdbus-1.so.3 (libc6,x86-64) => /lib/x86_64-linux-gnu/libdbus-1.so.3`.
// pc: apps/photocraft/src/linux_libs.rs:97 ldconfig_has
fn ldconfig_has(output: &str, soname: &str, arch_tag: Option<&str>) -> bool {
    output.lines().any(|line| {
        let Some((name, rest)) = line.trim_start().split_once(' ') else { return false };
        if name != soname {
            return false;
        }
        arch_tag.is_none_or(|tag| {
            let flags = rest.split_once('(').and_then(|(_, r)| r.split_once(')')).map_or("", |(f, _)| f);
            flags.split(',').any(|f| f.trim() == tag)
        })
    })
}

/// A real `ldconfig -p` listing starts with "N libs found in cache": a count, then the cache's name.
// pc: apps/photocraft/src/linux_libs.rs:116 ldconfig_output_is_valid
fn ldconfig_output_is_valid(output: &str) -> bool {
    output.lines().next().is_some_and(|l| l.split_whitespace().next().is_some_and(|n| n.parse::<u32>().is_ok()))
}

/// Whether the linker cache lists `soname`; `None` when the cache can't be read, which proves
/// nothing (musl, NixOS, some containers).
// pc: apps/photocraft/src/linux_libs.rs:247 read_ldconfig, :270 cached
#[cfg(target_os = "linux")]
fn linker_cache_has(soname: &str) -> Option<bool> {
    // `/sbin` isn't on a normal user's PATH on Debian. The listing is parsed, so it must be the
    // untranslated one.
    ["/sbin/ldconfig", "/usr/sbin/ldconfig", "ldconfig"].into_iter().find_map(|cmd| {
        let out = std::process::Command::new(cmd).arg("-p").env("LC_ALL", "C").output().ok().filter(|out| out.status.success())?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        ldconfig_output_is_valid(&text).then(|| ldconfig_has(&text, soname, ldconfig_arch_tag()))
    })
}

/// Whether the portal's `FileChooser` interface is on the session bus: `Some(false)` only when the
/// bus or the interface is definitely absent, `None` for anything inconclusive (a slow reply).
///
/// Asks the portal object to introspect itself rather than reading the interface's `version`
/// property: xdg-desktop-portal exports `FileChooser` only when a backend offers it, and without
/// one a property read fails with `InvalidArgs` ("No such interface"). The interface list answers
/// the question directly.
// pc: apps/photocraft/src/file_dialog_probe.rs:79 file_chooser_portal
#[cfg(target_os = "linux")]
fn file_chooser_portal() -> Option<bool> {
    const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    let Ok(conn) = zbus::blocking::connection::Builder::session().and_then(|b| b.method_timeout(REPLY_TIMEOUT).build()) else {
        return Some(false);
    };
    let reply = conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.DBus.Introspectable"),
        "Introspect",
        &(),
    );
    match reply {
        Ok(message) => message.body().deserialize::<String>().ok().map(|xml| interface_listed(&xml)),
        Err(e) => classify_error(&e),
    }
}

/// Whether introspection XML lists the portal's `FileChooser` interface (not a longer name such as
/// `FileChooser2`).
// pc: apps/photocraft/src/file_dialog_probe.rs:102 interface_listed
#[cfg(target_os = "linux")]
fn interface_listed(xml: &str) -> bool {
    ["\"", "'"].iter().any(|quote| xml.contains(&format!("<interface name={quote}org.freedesktop.portal.FileChooser{quote}")))
}

/// What a failed call says about the interface: `Some(false)` when the D-Bus error means the
/// service, object or interface isn't there, `None` for anything else (a timeout, a dropped
/// connection, an unknown error), which counts as present.
// pc: apps/photocraft/src/file_dialog_probe.rs:110 classify_error
#[cfg(target_os = "linux")]
fn classify_error(e: &zbus::Error) -> Option<bool> {
    use zbus::fdo::Error as Fdo;
    // Names a bus daemon or portal answers with when the thing asked about is absent.
    const ABSENT: [&str; 7] =
        ["ServiceUnknown", "NameHasNoOwner", "UnknownObject", "UnknownInterface", "UnknownMethod", "UnknownProperty", "InvalidArgs"];
    match e {
        zbus::Error::FDO(fdo) => matches!(
            **fdo,
            Fdo::ServiceUnknown(_)
                | Fdo::NameHasNoOwner(_)
                | Fdo::UnknownObject(_)
                | Fdo::UnknownInterface(_)
                | Fdo::UnknownMethod(_)
                | Fdo::UnknownProperty(_)
                | Fdo::InvalidArgs(_)
        )
        .then_some(false),
        zbus::Error::MethodError(name, _, _) => ABSENT.iter().any(|n| name.as_str().ends_with(n)).then_some(false),
        _ => None,
    }
}
