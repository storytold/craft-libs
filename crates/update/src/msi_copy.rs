//! Whether a running Windows copy was installed by its MSI, and the `msiexec` properties its
//! self-update needs to keep that copy's context.
//!
//! Every Craft App MSI registers its exe under `App Paths` (HKLM when installed for everyone,
//! HKCU for one account). A copy whose exe one of those entries names is an MSI copy, wherever its
//! folder is: `Program Files\ArtCraft\<App>`, the older `Program Files\<App>`, a folder the person
//! chose. Recognising it only reads the registry, so it is cheap enough for any status check.
//!
//! Which context to update in is a separate, slower question that only an update asks. Windows
//! Installer's own registration answers it ([`crate::msi_context`]), as it does for the launcher;
//! only when that lookup cannot run at all do the product's own uninstall records answer, and
//! only when they all agree. The answer must
//! match the hive that named this exe: a package writes `App Paths` under `HKMU`, which is always
//! the hive of its own context, so a registration in the other context belongs to another copy and
//! the update stops. So a copy installed for everyone is never unpacked over without permission,
//! and a per-user copy never gains a second, per-machine twin or takes another copy's context.
//!
//! The launcher's own per-user MSI is still recognised by the folder it records in HKCU (or its
//! default folder) and updated exactly as before, with the package's own default context. An app
//! copy in that kind of folder is judged by its registration first ([`classify`]).

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use crate::msi_context::MsiScope;

/// How an MSI copy was recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MsiCopy {
    /// An `App Paths` entry in the hive of `hive` names this exe.
    Registered { hive: MsiScope },
    /// Entries in both hives name it: the update stops rather than guess.
    Conflicting,
    /// The launcher's own per-user package, found by its recorded or default folder.
    Legacy,
}

const CONFLICTING: &str =
    "this app is registered both for your account and for everyone, so it can't tell which copy to update. Use the app's installer instead.";
const ANOTHER_COPY: &str = "Windows lists this app for a different account setting than this copy, so it can't be sure which copy it would update. Use the app's installer instead.";
const NOT_ONE_PRODUCT: &str =
    "Windows lists this app's installation more than once or not at all, so it can't tell which copy to update. Use the app's installer instead.";

/// The `msiexec` properties for `copy`'s update (as [`crate::msi_migrate`] uses them). `product_codes` are the app's Windows Installer
/// registrations; `probe` asks Windows Installer for one product's context, and `record_scopes`
/// lists the context of every uninstall record of that product, read only when the lookup cannot
/// run. Exactly one product must be registered: with two, either could be the copy that runs. The
/// records must agree on one context. Whichever answers, it must be the context of the hive that
/// named this exe, or the registration is another copy's.
#[cfg(test)]
pub(crate) fn update_arguments(
    copy: MsiCopy,
    product_codes: &[String],
    probe: impl FnOnce(&str) -> Result<MsiScope, crate::msi_context::ProbeError>,
    record_scopes: impl FnOnce(&str) -> Vec<MsiScope>,
) -> Result<&'static [&'static str], String> {
    Ok(resolve(copy, product_codes, probe, record_scopes)?.map_or(&[][..], |(scope, _)| crate::msi_context::scope_arguments(scope)))
}

/// The confirmed context and product code of `copy`, by the rules of [`update_arguments`]. `None`
/// for the launcher's own package, which keeps its package default and is never moved.
pub(crate) fn resolve(
    copy: MsiCopy,
    product_codes: &[String],
    probe: impl FnOnce(&str) -> Result<MsiScope, crate::msi_context::ProbeError>,
    record_scopes: impl FnOnce(&str) -> Vec<MsiScope>,
) -> Result<Option<(MsiScope, String)>, String> {
    let hive = match copy {
        MsiCopy::Legacy => return Ok(None),
        MsiCopy::Conflicting => return Err(CONFLICTING.into()),
        MsiCopy::Registered { hive } => hive,
    };
    let [code] = product_codes else { return Err(NOT_ONE_PRODUCT.into()) };
    let fallback = |code: &str| crate::msi_context::single_scope(record_scopes(code));
    let scope = crate::msi_context::update_scope(true, Some(code), probe, fallback)?.ok_or(NOT_ONE_PRODUCT)?;
    if scope != hive {
        return Err(ANOTHER_COPY.into());
    }
    Ok(Some((scope, code.clone())))
}

/// How a copy was installed, from what is known about it. The launcher's own per-user package in
/// its recorded or default folder keeps its update exactly as before, whatever else is registered
/// (a stale second record must not stop it). An app is judged by its `App Paths` registration
/// first, so a registered copy always passes the context checks; the folder rule only covers an
/// app copy that no registration names. `registration` is only read when it can decide.
pub(crate) fn classify(is_launcher: bool, in_legacy_folder: bool, registration: impl FnOnce() -> Option<MsiCopy>) -> Option<MsiCopy> {
    if is_launcher && in_legacy_folder {
        return Some(MsiCopy::Legacy);
    }
    registration().or_else(|| in_legacy_folder.then_some(MsiCopy::Legacy))
}

/// Which registrations name `exe`. `registered` holds every `App Paths` target with the context
/// of the hive it came from; both sides are expected in one canonical form.
pub(crate) fn match_registration(exe: &Path, registered: &[(MsiScope, PathBuf)]) -> Option<MsiCopy> {
    let mut hives = registered.iter().filter(|(_, target)| same_path(target, exe)).map(|(hive, _)| *hive);
    let first = hives.next()?;
    Some(if hives.all(|hive| hive == first) { MsiCopy::Registered { hive: first } } else { MsiCopy::Conflicting })
}

/// The one exe the app's registrations name after an update. The package may have moved an exact
/// old default folder, so the helper starts this rather than the path it was given. `None` unless
/// every registration names the same file.
pub(crate) fn registered_exe(registered: &[(MsiScope, PathBuf)]) -> Option<PathBuf> {
    let mut targets = registered.iter().map(|(_, target)| target);
    let first = targets.next()?;
    targets.all(|t| same_path(t, first)).then(|| first.clone())
}

/// Windows paths compare without regard to case.
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

/// An `App Paths` default value as a path: Windows allows it in quotes.
pub(crate) fn app_path_value(value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"').trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

#[cfg(windows)]
pub(crate) mod windows {
    use std::path::{Path, PathBuf};

    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

    use super::{MsiCopy, MsiScope, app_path_value, match_registration};
    use crate::app::App;

    /// `path` with links and `..` resolved in ordinary drive syntax, or as given when it cannot be
    /// resolved, so registry values and the running exe compare in one form.
    fn canonical(path: &Path) -> PathBuf {
        dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// Every `App Paths` target registered for one of `app`'s exe names, with the context of its
    /// hive. Entries naming a missing file are left out.
    pub(crate) fn registered_exes(app: &App) -> Vec<(MsiScope, PathBuf)> {
        let hives = [
            (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, MsiScope::PerMachine),
            (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, MsiScope::PerMachine),
            (HKEY_CURRENT_USER, 0, MsiScope::PerUser),
        ];
        let mut out: Vec<(MsiScope, PathBuf)> = Vec::new();
        for stem in app.stems() {
            let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{stem}.exe");
            for (hive, flags, scope) in hives {
                let Ok(entry) = RegKey::predef(hive).open_subkey_with_flags(&key, KEY_READ | flags) else { continue };
                let Some(target) = entry.get_value::<String, _>("").ok().as_deref().and_then(app_path_value) else { continue };
                if target.is_file() {
                    let target = canonical(&target);
                    if !out.iter().any(|(s, t)| *s == scope && *t == target) {
                        out.push((scope, target));
                    }
                }
            }
        }
        out
    }

    /// How `exe` was installed by an MSI, or `None` when it wasn't. Reads the registry only.
    pub(crate) fn find(app: &App, exe: &Path) -> Option<MsiCopy> {
        let exe = canonical(exe);
        let is_launcher = app.id == crate::suite::LAUNCHER.id;
        super::classify(is_launcher, is_legacy(app, &exe), || match_registration(&exe, &registered_exes(app)))
    }

    /// The confirmed context and product code of `exe`'s copy, asking Windows Installer; `None`
    /// for the launcher's own package. See [`super::resolve`].
    pub(crate) fn resolve(app: &App, exe: &Path) -> Result<Option<(MsiScope, String)>, String> {
        let copy = find(app, exe).ok_or_else(|| format!("{} isn't registered with the Windows Installer", app.name))?;
        let codes: Vec<String> = crate::detect::windows::msi_installs(app)
            .into_iter()
            .map(|m| m.product_code)
            .filter(|code| crate::msi_context::valid_product_code(code))
            .collect();
        super::resolve(copy, &codes, crate::msi_context::probe, record_scopes)
    }

    /// The context of every uninstall record of `code`: the machine's (both registry views) are
    /// per-machine, the account's own per-user. Two views of one machine record count once.
    fn record_scopes(code: &str) -> Vec<MsiScope> {
        const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
        let roots = [
            (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, MsiScope::PerMachine),
            (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, MsiScope::PerMachine),
            (HKEY_CURRENT_USER, 0, MsiScope::PerUser),
        ];
        let mut scopes = Vec::new();
        for (hive, flags, scope) in roots {
            let found = RegKey::predef(hive).open_subkey_with_flags(format!(r"{UNINSTALL}\{code}"), KEY_READ | flags).is_ok();
            if found && !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        scopes
    }

    /// The launcher's own per-user MSI: the folder its wizard records in HKCU, or its default
    /// `%LOCALAPPDATA%\Programs\<App Name>`.
    fn is_legacy(app: &App, exe: &Path) -> bool {
        let Some(dir) = exe.parent() else { return false };
        let recorded = recorded_dir(app).map(|d| canonical(&d));
        let default = crate::folders::data_local_dir().map(|d| canonical(&d.join("Programs").join(app.name)));
        [recorded, default].into_iter().flatten().any(|d| d == dir)
    }

    /// The install folder the launcher's per-user wizard records in HKCU, when it recorded one.
    fn recorded_dir(app: &App) -> Option<PathBuf> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"Software\Learning Machines LLC\{}", app.name)).ok()?;
        let dir: String = key.get_value("InstallDir").ok()?;
        let dir = dir.trim();
        (!dir.is_empty()).then(|| PathBuf::from(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msi_context::ProbeError;

    const CODE: &str = "{500F5254-26F4-4AAA-B90D-6F3FD9947D14}";

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn a_registration_makes_an_msi_copy_wherever_its_folder_is() {
        let registered = [(MsiScope::PerMachine, p(r"C:\Program Files\GridCraft\gridcraft.exe"))];
        let copy = Some(MsiCopy::Registered { hive: MsiScope::PerMachine });
        assert_eq!(match_registration(&p(r"C:\Program Files\GridCraft\gridcraft.exe"), &registered), copy);
        assert_eq!(match_registration(&p(r"c:\program files\gridcraft\GRIDCRAFT.EXE"), &registered), copy);
        let user = [(MsiScope::PerUser, p(r"D:\Chosen\GridCraft\gridcraft.exe"))];
        assert_eq!(match_registration(&p(r"D:\Chosen\GridCraft\gridcraft.exe"), &user), Some(MsiCopy::Registered { hive: MsiScope::PerUser }));
    }

    #[test]
    fn an_exe_no_registration_names_is_not_an_msi_copy() {
        let registered = [(MsiScope::PerMachine, p(r"C:\Program Files\ArtCraft\GridCraft\gridcraft.exe"))];
        assert_eq!(match_registration(&p(r"D:\Tools\GridCraft\gridcraft.exe"), &registered), None);
        assert_eq!(match_registration(&p(r"D:\Tools\GridCraft\gridcraft.exe"), &[]), None);
    }

    #[test]
    fn a_copy_registered_in_both_hives_stops_before_any_lookup() {
        let exe = p(r"C:\Program Files\ArtCraft\GridCraft\gridcraft.exe");
        let copy = match_registration(&exe, &[(MsiScope::PerMachine, exe.clone()), (MsiScope::PerUser, exe.clone())]).unwrap();
        assert_eq!(copy, MsiCopy::Conflicting);
        assert!(update_arguments(copy, &codes(&[CODE]), |_| panic!("must not look up"), |_| panic!("must not read the records")).is_err());
    }

    #[test]
    fn windows_installer_confirms_the_context_of_the_hive_that_named_the_exe() {
        let copy = MsiCopy::Registered { hive: MsiScope::PerMachine };
        assert_eq!(
            update_arguments(copy, &codes(&[CODE]), |_| Ok(MsiScope::PerMachine), |_| panic!("must not read the records")).unwrap(),
            ["ALLUSERS=1"]
        );
        let copy = MsiCopy::Registered { hive: MsiScope::PerUser };
        assert_eq!(
            update_arguments(copy, &codes(&[CODE]), |_| Ok(MsiScope::PerUser), |_| panic!("must not read the records")).unwrap(),
            ["ALLUSERS=2", "MSIINSTALLPERUSER=1"]
        );
    }

    #[test]
    fn a_registration_in_the_other_context_is_another_copys_and_stops_the_update() {
        for (hive, other) in [(MsiScope::PerUser, MsiScope::PerMachine), (MsiScope::PerMachine, MsiScope::PerUser)] {
            let error =
                update_arguments(MsiCopy::Registered { hive }, &codes(&[CODE]), |_| Ok(other), |_| panic!("must not read the records")).unwrap_err();
            assert_eq!(error, ANOTHER_COPY);
        }
    }

    #[test]
    fn the_uninstall_records_answer_only_when_the_lookup_cannot_run() {
        let copy = MsiCopy::Registered { hive: MsiScope::PerMachine };
        let blocked = |_: &str| Err(ProbeError::Unavailable("the registry could not be read".into()));
        assert_eq!(update_arguments(copy, &codes(&[CODE]), blocked, |_| vec![MsiScope::PerMachine]).unwrap(), ["ALLUSERS=1"]);
        for unclear in [ProbeError::Unknown, ProbeError::Conflicting] {
            assert!(update_arguments(copy, &codes(&[CODE]), |_| Err(unclear), |_| panic!("must not read the records")).is_err());
        }
    }

    #[test]
    fn missing_mixed_or_mismatched_records_stop_the_fallback() {
        let copy = MsiCopy::Registered { hive: MsiScope::PerMachine };
        let blocked = |_: &str| Err(ProbeError::Unavailable("the registry could not be read".into()));
        assert!(update_arguments(copy, &codes(&[CODE]), blocked, |_| vec![]).is_err());
        assert!(update_arguments(copy, &codes(&[CODE]), blocked, |_| vec![MsiScope::PerMachine, MsiScope::PerUser]).is_err());
        let error = update_arguments(copy, &codes(&[CODE]), blocked, |_| vec![MsiScope::PerUser]).unwrap_err();
        assert_eq!(error, ANOTHER_COPY);
    }

    #[test]
    fn the_update_stops_unless_exactly_one_product_is_registered() {
        let copy = MsiCopy::Registered { hive: MsiScope::PerMachine };
        assert!(update_arguments(copy, &[], |_| panic!("must not look up"), |_| panic!("must not read the records")).is_err());
        let two = codes(&[CODE, "{0BE5923F-1AAB-4240-8174-2F0CCC05E14C}"]);
        assert!(update_arguments(copy, &two, |_| panic!("must not look up"), |_| panic!("must not read the records")).is_err());
    }

    #[test]
    fn the_launcher_in_its_own_folder_keeps_its_old_path_whatever_is_registered() {
        let registered = Some(MsiCopy::Registered { hive: MsiScope::PerUser });
        assert_eq!(classify(true, true, || registered), Some(MsiCopy::Legacy));
        assert_eq!(classify(true, false, || registered), registered);
        assert_eq!(classify(true, false, || None), None);
    }

    #[test]
    fn an_app_in_the_old_per_user_folder_is_judged_by_its_registration_first() {
        for registered in [MsiCopy::Registered { hive: MsiScope::PerUser }, MsiCopy::Conflicting] {
            assert_eq!(classify(false, true, || Some(registered)), Some(registered));
        }
        assert_eq!(classify(false, true, || None), Some(MsiCopy::Legacy));
        assert_eq!(classify(false, false, || None), None);
    }

    #[test]
    fn the_launchers_own_package_keeps_its_default() {
        assert!(update_arguments(MsiCopy::Legacy, &[], |_| panic!("must not look up"), |_| panic!("must not read the records")).unwrap().is_empty());
    }

    #[test]
    fn the_helper_restarts_the_one_exe_the_registrations_name() {
        let moved = p(r"C:\Program Files\ArtCraft\GridCraft\gridcraft.exe");
        assert_eq!(registered_exe(&[(MsiScope::PerMachine, moved.clone())]), Some(moved.clone()));
        assert_eq!(registered_exe(&[(MsiScope::PerMachine, moved.clone()), (MsiScope::PerUser, moved.clone())]), Some(moved));
        assert_eq!(registered_exe(&[]), None);
        let two = [(MsiScope::PerMachine, p(r"C:\a\gridcraft.exe")), (MsiScope::PerUser, p(r"C:\b\gridcraft.exe"))];
        assert_eq!(registered_exe(&two), None);
    }

    #[test]
    fn app_paths_values_may_be_quoted() {
        assert_eq!(
            app_path_value(r#""C:\Program Files\ArtCraft\GridCraft\gridcraft.exe""#),
            Some(p(r"C:\Program Files\ArtCraft\GridCraft\gridcraft.exe"))
        );
        assert_eq!(app_path_value("  "), None);
    }
}
