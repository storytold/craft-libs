//! TEMPORARY: moves an app installed for one account to an install for everyone during its own
//! update. Remove this module two releases after the app packages become per-machine only
//! (craft-libs#422, step 4); from then on there are no per-user copies left to move.
//!
//! The released packages refuse a per-machine install while a per-user copy exists, so the move
//! removes first and installs second:
//!
//! 1. Remove the per-user copy (no permission prompt). If that fails nothing has changed.
//! 2. Install the new version for everyone (Windows asks for permission).
//! 3. If that fails or permission is refused, install the new version for the account again, in
//!    the folder it had. The person always keeps a working app; settings and documents live
//!    outside the program folder and are never touched.
//!
//! Which copies move: one in a per-user default folder goes to `Program Files\ArtCraft\<App>`; one
//! in a folder the person chose outside their profile moves in place; one in a chosen folder
//! inside the profile keeps updating for the account, since program files for everyone must not
//! live in one person's profile. The launcher's own package never moves.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use crate::msi_context::{MsiScope, scope_arguments};

/// What an MSI self-update does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Update in place with these `msiexec` properties.
    Update(&'static [&'static str]),
    /// Remove the per-user product, install for everyone into `folder`, and fall back to the
    /// account in `old_folder` if that fails.
    Move { product_code: String, folder: PathBuf, old_folder: PathBuf },
}

/// Where this account's files live. `roots` are `USERPROFILE`, `LOCALAPPDATA` and `APPDATA`, which
/// may be redirected away from one another; `defaults` are the app's per-user default folders
/// ([`default_folders`]). All resolved the way the exe's folder is, so links and `subst` drives
/// compare as the same place.
#[derive(Clone, Debug, Default)]
pub(crate) struct Profile {
    pub roots: Vec<PathBuf>,
    pub defaults: Vec<PathBuf>,
}

/// The per-user default folders of `app_name` under `local_app_data`: the suite folder
/// `Programs\ArtCraft\<App>`, then the older `Programs\<App>`.
pub(crate) fn default_folders(local_app_data: &Path, app_name: &str) -> [PathBuf; 2] {
    let programs = local_app_data.join("Programs");
    [programs.join("ArtCraft").join(app_name), programs.join(app_name)]
}

/// The step for a copy whose exe is in `dir`. `resolved` is its confirmed context and product
/// code (`None` for the launcher's own package); `program_files` is the root a new install for
/// everyone goes under, for this package's architecture.
pub(crate) fn plan(resolved: Option<(MsiScope, String)>, dir: &Path, app_name: &str, profile: &Profile, program_files: Option<&Path>) -> Step {
    let Some((scope, product_code)) = resolved else { return Step::Update(&[]) };
    if scope == MsiScope::PerMachine {
        return Step::Update(scope_arguments(scope));
    }
    let stay = Step::Update(scope_arguments(MsiScope::PerUser));
    let folder = if profile.defaults.iter().any(|default| same_path(default, dir)) {
        match program_files {
            Some(root) => root.join("ArtCraft").join(app_name),
            None => return stay,
        }
    } else if profile.roots.iter().any(|root| is_within(dir, root)) {
        return stay;
    } else {
        dir.to_path_buf()
    };
    Step::Move { product_code, folder, old_folder: dir.to_path_buf() }
}

/// Windows paths compare without regard to case or a trailing separator.
fn normal(path: &Path) -> String {
    path.as_os_str().to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    normal(a) == normal(b)
}

/// Whether `path` is `root` or inside it, by whole folder names.
fn is_within(path: &Path, root: &Path) -> bool {
    let (path, root) = (normal(path), normal(root));
    !root.is_empty() && (path == root || path.strip_prefix(&root).is_some_and(|rest| rest.starts_with('\\')))
}

/// Moves a copy in the only safe order: `remove` the per-user copy (stop if that fails, nothing has
/// changed), install `to_everyone`, and only if that fails `keep_log` of it and install
/// `back_to_account`. The error says where the app is now.
pub(crate) fn run_move(
    remove: impl FnOnce() -> Result<(), String>,
    to_everyone: impl FnOnce() -> Result<(), String>,
    keep_log: impl FnOnce(),
    back_to_account: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    remove()?;
    let Err(installed) = to_everyone() else { return Ok(()) };
    keep_log();
    Err(move_failure(&installed, back_to_account()))
}

/// The error a failed move reports, saying where the app now is.
pub(crate) fn move_failure(installed: &str, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => format!("couldn't move the app to an install for everyone ({installed}); it was updated for your account instead"),
        Err(again) => format!(
            "couldn't move the app to an install for everyone ({installed}), and reinstalling it for your account failed too ({again}). Run its installer to get it back; your settings and documents are untouched"
        ),
    }
}

#[cfg(windows)]
pub(crate) mod windows {
    use std::path::{Path, PathBuf};

    use super::{Profile, Step, plan, run_move};
    use crate::app::App;
    use crate::install::windows::{reinstall, remove_product};
    use crate::msi_context::{MsiScope, scope_arguments};

    fn env_path(name: &str) -> Option<PathBuf> {
        std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
    }

    /// The step for `exe`'s update, asking Windows Installer for its context.
    pub(crate) fn step(app: &App, exe: &Path) -> Result<Step, String> {
        let resolved = crate::msi_copy::windows::resolve(app, exe)?;
        let dir = exe.parent().ok_or_else(|| format!("{} isn't in a folder", app.name))?;
        let dir = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        // Resolved like `dir`, whole path at a time, so a profile or a default folder reached
        // through a junction or a `subst` drive still matches.
        let canonical = |p: PathBuf| dunce::canonicalize(&p).unwrap_or(p);
        let profile = Profile {
            roots: ["USERPROFILE", "LOCALAPPDATA", "APPDATA"].into_iter().filter_map(env_path).map(canonical).collect(),
            defaults: crate::folders::data_local_dir()
                .map(|local| super::default_folders(&local, app.name).into_iter().map(canonical).collect())
                .unwrap_or_default(),
        };
        // This process has the package's architecture (the helper is a copy of the app), so its
        // own `ProgramFiles` is the right root: `Program Files (x86)` for a 32-bit app.
        Ok(plan(resolved, &dir, app.name, &profile, env_path("ProgramFiles").as_deref()))
    }

    /// Runs `step` with the downloaded package `msi`.
    pub(crate) fn run(msi: &Path, step: &Step) -> Result<(), String> {
        match step {
            Step::Update(context) => reinstall(msi, context, None),
            Step::Move { product_code, folder, old_folder } => run_move(
                || remove_product(product_code, &msi.with_extension("remove.log")),
                || reinstall(msi, scope_arguments(MsiScope::PerMachine), Some(folder)),
                // The fallback writes a fresh log; keep the failed install's beside it.
                || {
                    let _ = std::fs::rename(crate::install::msi_log(msi), msi.with_extension("everyone.log"));
                },
                || reinstall(msi, scope_arguments(MsiScope::PerUser), Some(old_folder)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "{500F5254-26F4-4AAA-B90D-6F3FD9947D14}";

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn profile() -> Profile {
        Profile {
            roots: vec![p(r"C:\Users\Ann"), p(r"C:\Users\Ann\AppData\Local"), p(r"C:\Users\Ann\AppData\Roaming")],
            defaults: [r"C:\Users\Ann\AppData\Local\Programs\ArtCraft\GridCraft", r"C:\Users\Ann\AppData\Local\Programs\GridCraft"].map(p).to_vec(),
        }
    }

    #[test]
    fn the_default_folders_are_the_suite_folder_then_the_older_one() {
        let local = p(r"C:\Users\Ann\AppData\Local");
        let expected = [local.join("Programs").join("ArtCraft").join("GridCraft"), local.join("Programs").join("GridCraft")];
        assert_eq!(default_folders(&local, "GridCraft"), expected);
    }

    #[test]
    fn a_default_folder_reached_through_a_link_still_moves_to_the_suite_folder() {
        // The Windows layer resolves each default folder whole, so a linked `Programs` arrives
        // here as the real path the exe's folder also resolves to.
        let old = r"D:\Redirected\Programs\ArtCraft\GridCraft";
        let linked = Profile { defaults: vec![p(old)], ..profile() };
        let suite = pf().map(|root| root.join("ArtCraft").join("GridCraft")).unwrap();
        let expected = Step::Move { product_code: CODE.to_string(), folder: suite, old_folder: p(old) };
        assert_eq!(plan(user(), &p(old), "GridCraft", &linked, pf()), expected);
    }

    fn user() -> Option<(MsiScope, String)> {
        Some((MsiScope::PerUser, CODE.to_string()))
    }

    fn moved(folder: &str, old: &str) -> Step {
        Step::Move { product_code: CODE.to_string(), folder: p(folder), old_folder: p(old) }
    }

    fn pf() -> Option<&'static Path> {
        Some(Path::new(r"C:\Program Files"))
    }

    #[test]
    fn a_copy_for_everyone_and_the_launcher_update_in_place() {
        let machine = Some((MsiScope::PerMachine, CODE.to_string()));
        assert_eq!(plan(machine, &p(r"C:\Program Files\GridCraft"), "GridCraft", &profile(), pf()), Step::Update(&["ALLUSERS=1"]));
        assert_eq!(
            plan(None, &p(r"C:\Users\Ann\AppData\Local\Programs\ArtCraft\Launcher"), "ArtCraft Launcher", &profile(), pf()),
            Step::Update(&[])
        );
    }

    #[test]
    fn a_per_user_copy_in_a_default_folder_moves_to_the_suite_folder() {
        for old in [r"C:\Users\Ann\AppData\Local\Programs\ArtCraft\GridCraft", r"c:\users\ann\appdata\local\programs\gridcraft\"] {
            // Joined like the plan joins it: `\` on Windows, `/` where these tests also run.
            let suite = pf().map(|root| root.join("ArtCraft").join("GridCraft")).unwrap();
            let expected = Step::Move { product_code: CODE.to_string(), folder: suite, old_folder: p(old) };
            assert_eq!(plan(user(), &p(old), "GridCraft", &profile(), pf()), expected);
        }
    }

    #[test]
    fn a_chosen_folder_outside_the_profile_moves_in_place() {
        let old = r"D:\Apps\GridCraft";
        assert_eq!(plan(user(), &p(old), "GridCraft", &profile(), pf()), moved(old, old));
    }

    #[test]
    fn a_chosen_folder_inside_the_profile_keeps_updating_for_the_account() {
        for dir in [r"C:\Users\Ann\Tools\GridCraft", r"C:\Users\Ann\AppData\Local\Other\GridCraft", r"C:\Users\Ann\AppData\Roaming\GridCraft"] {
            assert_eq!(plan(user(), &p(dir), "GridCraft", &profile(), pf()), Step::Update(&["ALLUSERS=2", "MSIINSTALLPERUSER=1"]), "{dir}");
        }
    }

    #[test]
    fn a_folder_that_only_shares_a_prefix_with_the_profile_is_outside_it() {
        let old = r"C:\Users\Anna\GridCraft";
        assert_eq!(plan(user(), &p(old), "GridCraft", &profile(), pf()), moved(old, old));
    }

    #[test]
    fn without_a_program_files_folder_a_default_copy_stays_for_the_account() {
        let dir = p(r"C:\Users\Ann\AppData\Local\Programs\ArtCraft\GridCraft");
        assert_eq!(plan(user(), &dir, "GridCraft", &profile(), None), Step::Update(&["ALLUSERS=2", "MSIINSTALLPERUSER=1"]));
    }

    fn steps(results: [Result<(), String>; 3]) -> (Result<(), String>, Vec<&'static str>) {
        let done = std::cell::RefCell::new(Vec::new());
        let [remove, everyone, account] = results;
        let result = run_move(
            || {
                done.borrow_mut().push("remove");
                remove
            },
            || {
                done.borrow_mut().push("everyone");
                everyone
            },
            || done.borrow_mut().push("keep log"),
            || {
                done.borrow_mut().push("account");
                account
            },
        );
        (result, done.into_inner())
    }

    #[test]
    fn a_move_removes_first_and_installs_for_everyone() {
        assert_eq!(steps([Ok(()), Ok(()), Ok(())]), (Ok(()), vec!["remove", "everyone"]));
    }

    #[test]
    fn a_failed_removal_changes_nothing_else() {
        let (result, done) = steps([Err("error 1603".into()), Ok(()), Ok(())]);
        assert_eq!(result, Err("error 1603".into()));
        assert_eq!(done, ["remove"]);
    }

    #[test]
    fn a_refused_install_for_everyone_puts_the_app_back_for_the_account() {
        let (result, done) = steps([Ok(()), Err("cancelled".into()), Ok(())]);
        assert_eq!(done, ["remove", "everyone", "keep log", "account"]);
        assert!(result.is_err_and(|e| e.contains("updated for your account instead")));
    }

    #[test]
    fn a_failed_move_says_where_the_app_is() {
        assert!(move_failure("error 1602", Ok(())).contains("updated for your account instead"));
        let both = move_failure("error 1602", Err("error 1603".into()));
        assert!(both.contains("error 1603") && both.contains("Run its installer"), "{both}");
    }
}
