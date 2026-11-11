//! Tests for installing, updating and removing apps.

use super::*;
use craft_ui::testing::trial;

#[trial]
fn swap_keeps_the_old_copy_until_the_new_one_is_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("app");
    let staged = dir.path().join("app.new");
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join("v"), "old").unwrap();
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("v"), "new").unwrap();
    swap_into_place(&staged, &dest).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("v")).unwrap(), "new");
    assert!(!staged.exists() && !dir.path().join("app.old").exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    // A missing staged copy leaves the installed one alone.
    assert!(swap_into_place(&staged, &dest).is_err());
    assert_eq!(std::fs::read_to_string(dest.join("v")).unwrap(), "new");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[trial]
fn a_release_without_a_checksum_is_refused_before_anything_is_fetched() {
    use crate::http::tests::{Reply, body, serve};
    let base = App::by_id("wordcraft").unwrap();
    let wordcraft = App { icon_png: Some(b"\x89PNG\r\n\x1a\nfake-png-bytes"), ..*base };
    let app = &wordcraft;
    let (body, _) = body();
    let (url, requests) = serve(body.clone(), vec![Reply::Full]);
    // Neither a per-file digest nor a SHA256SUMS.txt in the release.
    let asset = Asset { name: "wordcraft-0.3.1-test.pkg".into(), size: body.len() as u64, url, sha256: None };
    let release = Release { assets: vec![asset.clone()], ..Release::default() };
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let kind = if cfg!(windows) { PackageKind::Dmg } else { PackageKind::Msi };
    let mut phases = Vec::new();
    let mut phase = |p: JobPhase| phases.push(p);
    let mut job = Job {
        http: &http,
        paths: &paths,
        stop: &Stop::default(),
        phase: &mut phase,
        install_locations: &InstallLocations::default(),
        replace: false,
    };
    let err = install(app, &release, &asset, kind, None, None, &mut job).unwrap_err();
    assert!(err.contains("publishes no checksum"), "{err}");
    assert!(!download_path(&paths, &asset).exists());
    assert!(requests.lock().unwrap().is_empty(), "nothing was fetched");
    assert!(!phases.contains(&JobPhase::Installing));
}

#[cfg(unix)]
fn tarball(path: &Path, entries: &[&str]) {
    let f = std::fs::File::create(path).unwrap();
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::fast()));
    for name in entries {
        let mut h = tar::Header::new_gnu();
        h.set_size(2);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, name, &b"hi"[..]).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap();
}

#[cfg(target_os = "freebsd")]
#[trial]
fn remove_files_refuses_escaping_paths() {
    assert!(freebsd::remove_files(&[PathBuf::from("/etc/shadow")], false).is_err());
    if let Some(home) = crate::folders::home_dir() {
        let prefix = home.join(".local");
        assert!(freebsd::remove_files(&[prefix.join("../escaped")], false).is_err());
    }
}

#[cfg(target_os = "linux")]
#[trial]
fn a_tarball_binary_that_is_a_symlink_out_of_the_tree_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let base = App::by_id("wordcraft").unwrap();
    let wordcraft = App { icon_png: Some(b"\x89PNG\r\n\x1a\nfake-png-bytes"), ..*base };
    let app = &wordcraft;
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());

    // A working install the rejected update must leave untouched.
    let good = dir.path().join("wordcraft-0.3.0-linux-x86_64.tar.gz");
    tarball(&good, &["wordcraft-0.3.0-linux-x86_64/bin/wordcraft"]);
    let bin = linux::install_tree(app, &good, &paths).unwrap();
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "hi");

    // A file outside the install tree that the archive tries to chmod through a symlink.
    let outside = dir.path().join("victim");
    std::fs::write(&outside, b"secret").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();

    // A tarball whose bin/wordcraft is a symlink to that external file.
    let file = dir.path().join("evil.tar.gz");
    let f = std::fs::File::create(&file).unwrap();
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::fast()));
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    link.set_cksum();
    b.append_link(&mut link, "wordcraft-0.3.0-linux-x86_64/bin/wordcraft", &outside).unwrap();
    // A second real file so the top folder is detected.
    let mut reg = tar::Header::new_gnu();
    reg.set_size(2);
    reg.set_mode(0o644);
    reg.set_cksum();
    b.append_data(&mut reg, "wordcraft-0.3.0-linux-x86_64/share/applications/ai.storyteller.wordcraft.desktop", &b"hi"[..]).unwrap();
    b.into_inner().unwrap().finish().unwrap();

    let result = linux::install_tree(app, &file, &paths);
    assert!(result.is_err(), "a symlink binary must be rejected: {result:?}");
    assert!(result.unwrap_err().contains("symlink"));
    // The external file keeps its original mode: the chmod never reached it.
    let mode = std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the external file's permissions must be untouched");
    // The rejected update left the previously installed, working binary fully intact.
    assert!(bin.is_file(), "the rejected update must not remove the working binary");
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "hi");
}

#[cfg(target_os = "linux")]
const FLATPAK_ID: &str = "ai.storyteller.soundcraft";
#[cfg(target_os = "linux")]
const ALREADY_THERE: &str = "ai.storyteller.soundcraft already installed";
#[cfg(target_os = "linux")]
const INSTALL_CALL: &str = "install --user -y --noninteractive --reinstall --bundle /x.flatpak";

/// Runs `install` against a stand-in `flatpak` that logs its calls and fails its first `failures`
/// installs with `why`. Returns the result and the calls.
#[cfg(target_os = "linux")]
fn with_fake_flatpak(
    flatpak_version: &str,
    failures: u32,
    why: &str,
    uninstall_exit: u32,
    install: impl FnOnce(&linux::Root<'_>) -> Result<(), String>,
) -> (Result<(), String>, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let (script, calls) = (dir.path().join("flatpak"), dir.path().join("calls"));
    let log = calls.display();
    std::fs::write(
        &script,
        format!(
            r#"echo "$*" >> "{log}"
case "$1" in
--version) echo "Flatpak {flatpak_version}" ;;
uninstall) exit {uninstall_exit} ;;
install)
[ "$(grep -c '^install' "{log}")" -gt {failures} ] && exit 0
echo "error: Failed to install bundle: {why}" >&2
exit 1 ;;
esac
"#
        ),
    )
    .unwrap();
    let flatpak = || {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg(&script);
        cmd
    };
    let queue = crate::queue::Queue::new();
    let root = linux::Root { queue: &queue, run: &|_| Ok(()), flatpak: &flatpak };
    let result = install(&root);
    (result, std::fs::read_to_string(calls).unwrap().lines().map(str::to_string).collect())
}

#[cfg(target_os = "linux")]
fn update_bundle(root: &linux::Root<'_>, system: bool) -> Result<(), String> {
    linux::update_flatpak_bundle_via(root, Path::new("/x.flatpak"), FLATPAK_ID, system, &Stop::default(), &mut |_| {})
}

#[cfg(target_os = "linux")]
#[trial]
fn a_self_update_never_uninstalls_the_app_it_runs_from() {
    // The update landed earlier and never restarted, so only the restart is left.
    let (result, calls) = with_fake_flatpak("1.14.6", 1, ALREADY_THERE, 0, |root| update_bundle(root, false));
    assert_eq!((result, calls), (Ok(()), vec![INSTALL_CALL.to_string()]));

    let (result, calls) = with_fake_flatpak("1.14.6", 0, ALREADY_THERE, 0, |root| update_bundle(root, false));
    assert_eq!((result, calls), (Ok(()), vec![INSTALL_CALL.to_string()]));

    // Anything else is still a failure: a broken bundle, or one holding another app.
    for why in ["Invalid checksum", "ai.storyteller.photocraft already installed"] {
        let (result, calls) = with_fake_flatpak("1.14.6", 1, why, 0, |root| update_bundle(root, false));
        assert_eq!(result, Err(format!("Flatpak couldn't install the bundle: Failed to install bundle: {why}")));
        assert_eq!(calls, [INSTALL_CALL]);
    }
}

/// An asset whose file name, link tag or link file name isn't the release's is refused before
/// the download, and so is one in another app's releases or with a foreign `SHA256SUMS.txt`.
#[trial]
fn install_rejects_an_artifact_whose_version_is_not_the_advertised_one() {
    // The release advertises 99.0.0, but the selected artifact is an older official build
    // (filename and URL tag say 0.4.0). The guard must refuse before any download.
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let phase = &mut |_| ();
    let locations = InstallLocations::default();
    let app = App::by_id("photocraft").unwrap();
    let release = Release { version: version::parse("99.0.0"), tag: "v99.0.0".into(), ..Release::default() };
    let asset = Asset {
        name: "photocraft-0.4.0-linux-x86_64.AppImage".into(),
        url: "https://github.com/storytold/photocraft/releases/download/v0.4.0/photocraft-0.4.0-linux-x86_64.AppImage".into(),
        ..Asset::default()
    };
    let stop = Stop::default();
    let mut job = Job { http: &http, paths: &paths, stop: &stop, phase, install_locations: &locations, replace: false };
    let result = install_at(app, &release, &asset, PackageKind::AppImage, None, None, &mut job, None);
    assert!(result.is_err(), "mismatched artifact version must be rejected");
    assert!(result.unwrap_err().contains("does not match the advertised version"));

    // A matching artifact passes the guard (it fails later for unrelated reasons, not the bind).
    let good = Asset {
        name: "photocraft-99.0.0-linux-x86_64.AppImage".into(),
        url: "https://github.com/storytold/photocraft/releases/download/v99.0.0/photocraft-99.0.0-linux-x86_64.AppImage".into(),
        ..Asset::default()
    };
    assert!(bind_artifact_version(app, &release, &good).is_ok());

    // URL filename mismatch is rejected (CWE-494).
    let bad_url_name = Asset {
        name: "photocraft-99.0.0-linux-x86_64.AppImage".into(),
        url: "https://github.com/storytold/photocraft/releases/download/v99.0.0/other.AppImage".into(),
        ..Asset::default()
    };
    assert!(bind_artifact_version(app, &release, &bad_url_name).is_err());

    // URL tag mismatch is rejected.
    let bad_tag = Asset {
        name: "photocraft-99.0.0-linux-x86_64.AppImage".into(),
        url: "https://github.com/storytold/photocraft/releases/download/unparsed_tag/photocraft-99.0.0-linux-x86_64.AppImage".into(),
        ..Asset::default()
    };
    assert!(bind_artifact_version(app, &release, &bad_tag).is_err());

    // storytold/craft-launcher#764: another app's release with the right tag and name passes
    // the bind (it reads only the path after `/releases/download/`), but the check before the
    // download refuses it.
    let other_repo = Asset {
        name: "photocraft-99.0.0-linux-x86_64.AppImage".into(),
        url: "https://github.com/storytold/vectorcraft/releases/download/v99.0.0/photocraft-99.0.0-linux-x86_64.AppImage".into(),
        ..Asset::default()
    };
    assert!(bind_artifact_version(app, &release, &other_repo).is_ok());
    let err = install_at(app, &release, &other_repo, PackageKind::AppImage, None, None, &mut job, None).unwrap_err();
    assert!(err.contains("isn't the one its release publishes"), "{err}");
    assert!(check_download_link(app, &release, &good).is_ok());

    // The checksums the download is verified against must come from the release too.
    let mut foreign_sums = release.clone();
    foreign_sums.assets.push(Asset {
        name: "SHA256SUMS.txt".into(),
        url: "https://github.com/storytold/vectorcraft/releases/download/v99.0.0/SHA256SUMS.txt".into(),
        ..Asset::default()
    });
    assert!(check_download_link(app, &foreign_sums, &good).is_err());

    // A release the feed reads from a renamed repository records it, and its links are
    // checked against that one; the same links are refused for a release that doesn't.
    let renamed =
        release::release_from_sums("storytold/photocraft-next", "v0.5.0", &[("a".repeat(64), "photocraft-0.5.0-linux-x86_64.AppImage".into())]);
    assert!(check_download_link(app, &renamed, &renamed.assets[0]).is_ok());
    let unrecorded = Release { repo: String::new(), ..renamed.clone() };
    assert!(check_download_link(app, &unrecorded, &unrecorded.assets[0]).is_err());

    // A `+` written plainly, as an older launcher's feed and saved releases have it, passes.
    let plus = release::release_from_sums(app.repo, "v1.0.0+b.1", &[("a".repeat(64), "photocraft-1.0.0+b.1-linux-x86_64.AppImage".into())]);
    let mut plain = plus.clone();
    for a in &mut plain.assets {
        a.url = a.url.replace("%2B", "+");
    }
    assert!(check_download_link(app, &plain, &plain.assets[0]).is_ok());
}

#[cfg(unix)]
#[trial]
fn an_oversized_pax_record_is_refused_before_tar_buffers_it() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    // A PAX header claiming 1 MiB + 1 byte of records, compressed to almost nothing.
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(tar::EntryType::XHeader);
    header.set_size((1 << 20) + 1);
    header.set_path("pax").unwrap();
    header.set_cksum();
    let file = dir.path().join("pax.tar.gz");
    let mut gz = flate2::write::GzEncoder::new(std::fs::File::create(&file).unwrap(), flate2::Compression::best());
    gz.write_all(header.as_bytes()).unwrap();
    gz.write_all(&vec![b'a'; (1 << 20) + 512]).unwrap();
    gz.finish().unwrap();
    let err = tree::unpack(App::by_id("wordcraft").unwrap(), &file, &dir.path().join("staging")).unwrap_err();
    assert!(err.contains("oversized metadata"), "{err}");
}

/// A release whose tag has a `+` (semver build metadata) has `%2B` in its links. Both checks
/// before the download accept them, so `install_at` gets as far as the download, which the
/// cancelled job ends before anything is fetched. A link with another escape is still refused.
#[trial]
fn install_accepts_a_plus_release_up_to_the_download() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let phase = &mut |_| ();
    let locations = InstallLocations::default();
    let app = App::by_id("photocraft").unwrap();
    let name = "photocraft-1.0.0+build.1-linux-x86_64.AppImage";
    let release = release::release_from_sums(app.repo, "v1.0.0+build.1", &[("a".repeat(64), name.to_string())]);
    let asset = release.assets[0].clone();
    assert!(asset.url.ends_with("/download/v1.0.0%2Bbuild.1/photocraft-1.0.0%2Bbuild.1-linux-x86_64.AppImage"), "{}", asset.url);
    assert_eq!(bind_artifact_version(app, &release, &asset), Ok(()));

    let stop = Stop::default();
    stop.cancel();
    let mut job = Job { http: &http, paths: &paths, stop: &stop, phase, install_locations: &locations, replace: false };
    let result = install_at(app, &release, &asset, PackageKind::AppImage, None, None, &mut job, None);
    assert_eq!(result.unwrap_err(), crate::protocol::CANCELLED);

    let other_escape = Asset { url: asset.url.replace("%2B", "%2b"), ..asset };
    assert!(bind_artifact_version(app, &release, &other_escape).is_err());
}

/// A download that fails its checksum (twice, as a resumed file gets one fresh try) replaces
/// nothing: the installed copy keeps its bytes, the install never starts, the record isn't
/// committed, and the bad file isn't kept for the next try.
#[trial]
fn a_download_that_fails_its_checksum_replaces_nothing() {
    use crate::http::tests::{Reply, body, serve};
    let app = App::by_id("photocraft").unwrap();
    let (body, sha) = body();
    let (url, requests) = serve(body.clone(), vec![Reply::Corrupt, Reply::Corrupt]);
    let name = "photocraft-0.5.0-linux-x86_64.AppImage";
    let asset = Asset { name: name.into(), size: body.len() as u64, url, sha256: Some(sha) };
    let release = Release { version: version::parse("0.5.0"), tag: "v0.5.0".into(), assets: vec![asset.clone()], ..Release::default() };
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let old = dir.path().join("Apps").join("PhotoCraft.AppImage");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, b"the installed 0.4.0").unwrap();
    let installed =
        Installed { version: version::parse("0.4.0"), source: InstallSource::Launcher, location: old.display().to_string(), package: None };
    let previous = record("0.4.0", PackageKind::AppImage, old.clone(), vec![], false);
    let mut phases = Vec::new();
    let mut phase = |p: JobPhase| phases.push(p);
    let locations = InstallLocations::default();
    let stop = Stop::default();
    let mut job = Job { http: &http, paths: &paths, stop: &stop, phase: &mut phase, install_locations: &locations, replace: false };
    let mut committed = false;
    let mut commit = |_: &Record| {
        committed = true;
        Ok(())
    };
    let result = install_at(app, &release, &asset, PackageKind::AppImage, Some(&installed), Some(&previous), &mut job, Some(&mut commit));
    assert!(result.as_ref().is_err_and(|e| e.contains("checksum")), "{result:?}");
    assert_eq!(std::fs::read(&old).unwrap(), b"the installed 0.4.0");
    assert!(!phases.contains(&JobPhase::Installing), "{phases:?}");
    assert!(!committed);
    assert!(!download_path(&paths, &asset).exists());
    assert_eq!(requests.lock().unwrap().len(), 2);
}

/// A running installed copy isn't replaced; the stop is cancelled in case the check stops
/// working. Windows finds a process by its file name, so the running test binary stands in for
/// the copy. Elsewhere `pgrep -f` matches the command line, so a probe runs with the copy's path.
#[trial]
fn install_refuses_while_the_installed_copy_is_running() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let phase = &mut |_| ();
    let locations = InstallLocations::default();
    let app = App::by_id("photocraft").unwrap();
    let name = "photocraft-0.5.0-linux-x86_64.AppImage";
    let release = release::release_from_sums(app.repo, "v0.5.0", &[("a".repeat(64), name.to_string())]);
    #[cfg(windows)]
    let location = std::env::current_exe().unwrap().display().to_string();
    #[cfg(unix)]
    let location = dir.path().join("PhotoCraft.AppImage").display().to_string();
    #[cfg(unix)]
    let _probe = crate::process::tests::RunningProbe::start(&location);
    assert!(process::is_running(Path::new(&location)), "{location} should be seen as running");
    let installed = Installed { version: version::parse("0.4.0"), source: InstallSource::Launcher, location, package: None };
    let stop = Stop::default();
    stop.cancel();
    let mut job = Job { http: &http, paths: &paths, stop: &stop, phase, install_locations: &locations, replace: false };
    let result = install_at(app, &release, &release.assets[0], PackageKind::AppImage, Some(&installed), None, &mut job, None);
    assert!(result.as_ref().is_err_and(|e| e.contains("is running")), "{result:?}");
}

#[cfg(target_os = "macos")]
mod macos_checks {
    use craft_ui::testing::trial;

    use super::macos::{CheckTurn, Expect, Signature, TEAM_ID, signed_by};
    use super::*;
    use crate::process::HelperFailure;

    fn photocraft() -> &'static App {
        crate::SUITE.iter().find(|a| a.id == "photocraft").unwrap()
    }

    /// A minimal `PhotoCraft.app` at `dir`, reporting `version`.
    fn bundle(dir: &Path, version: &str) -> PathBuf {
        let app = dir.join("PhotoCraft.app");
        let contents = app.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string>\
                 <key>CFBundleExecutable</key><string>PhotoCraft</string>\
                 <key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"
            ),
        )
        .unwrap();
        std::fs::write(contents.join("MacOS/PhotoCraft"), version).unwrap();
        app
    }

    /// A release-style disk image holding a `PhotoCraft.app` that reports `version`. The
    /// disk-image helper turns requests away while it is busy (`attach` retries for the same
    /// reason), so `hdiutil create` is tried a few times, and what it said is kept for the
    /// failure message.
    fn dmg(dir: &Path, version: &str) -> PathBuf {
        let src = dir.join(format!("src-{version}"));
        bundle(&src, version);
        let dmg = dir.join(format!("photocraft-{version}-macos-universal.dmg"));
        let _one_at_a_time = super::macos::DISK_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut said = String::new();
        for attempt in 0..5u64 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(400 * attempt));
                let _ = std::fs::remove_file(&dmg);
            }
            let output = std::process::Command::new("hdiutil")
                .args(["create", "-quiet", "-format", "UDZO", "-volname", "PhotoCraft", "-srcfolder"])
                .arg(&src)
                .arg(&dmg)
                .output()
                .unwrap();
            if output.status.success() {
                return dmg;
            }
            said = String::from_utf8_lossy(&output.stderr).trim().to_string();
        }
        panic!("hdiutil create failed after 5 tries: {said}");
    }

    #[trial]
    fn only_the_teams_signature_passes() {
        let dir = tempfile::tempdir().unwrap();
        let app = bundle(dir.path(), "0.6.0");
        assert!(!signed_by(&app, TEAM_ID), "an unsigned bundle passed");

        // A valid signature, but not from the team.
        let ok = std::process::Command::new("codesign").args(["--force", "--sign", "-"]).arg(&app).status().unwrap().success();
        assert!(ok, "codesign failed");
        assert!(!signed_by(&app, TEAM_ID), "an ad-hoc signature passed");

        // A team id can't smuggle anything into the requirement.
        assert!(!signed_by(&app, "X\" or anchor apple generic or \"Y"));
        assert!(!signed_by(&app, ""));
    }

    /// [`macos::install_dmg`] for a job nobody cancels.
    fn install_dmg(
        app: &App,
        dmg: &Path,
        existing: Option<&Installed>,
        destination: Option<&Path>,
        replace: bool,
        paths: &Paths,
        expect: &Expect<'_>,
    ) -> Result<PathBuf, String> {
        let stop = Stop::default();
        macos::install_dmg(app, dmg, existing, destination, replace, paths, expect, CheckTurn { stop: &stop, phase: &mut |_| {} })
    }

    /// [`macos::signature`] for a job nobody cancels.
    fn signature(bundle: &Path, team: &str, run: impl FnOnce(&str, &[&str], std::time::Duration) -> Result<String, HelperFailure>) -> Signature {
        let stop = Stop::default();
        macos::signature(bundle, team, run, CheckTurn { stop: &stop, phase: &mut |_| {} })
    }

    /// [`macos::place_checked`] for a job nobody cancels.
    fn place_checked(
        app: &App,
        staged: &Path,
        dest: &Path,
        expect: &Expect<'_>,
        run: impl FnOnce(&str, &[&str], std::time::Duration) -> Result<String, HelperFailure>,
        running: impl FnOnce(&Path) -> Result<bool, String>,
    ) -> Result<PathBuf, String> {
        let stop = Stop::default();
        macos::place_checked(app, staged, dest, expect, run, running, CheckTurn { stop: &stop, phase: &mut |_| {} })
    }

    /// `codesign`'s own exit codes for the two kinds of refusal, which [`signature`] reads as
    /// answers and not as a check that couldn't finish.
    #[trial]
    fn real_refusals_are_answers_from_codesign() {
        let dir = tempfile::tempdir().unwrap();
        let app = bundle(dir.path(), "0.6.0");
        let unsigned = signature(&app, TEAM_ID, crate::process::run_detailed);
        assert!(matches!(&unsigned, Signature::Failed(reason) if reason.contains("not signed")), "an unsigned bundle: {unsigned:?}");
        let ok = std::process::Command::new("codesign").args(["--force", "--sign", "-"]).arg(&app).status().unwrap().success();
        assert!(ok, "codesign failed");
        assert_eq!(signature(&app, TEAM_ID, crate::process::run_detailed), Signature::Wrong, "an ad-hoc signature");
    }

    fn found_at(bundle: &Path) -> Installed {
        Installed { version: None, source: InstallSource::MacApp, location: bundle.display().to_string(), package: None }
    }

    /// Runs `behind` while a signature check of another app holds the turn, and frees the turn
    /// when `behind` has returned, or when `release` is called.
    fn behind_a_running_check<R>(behind: impl FnOnce(&dyn Fn()) -> R) -> R {
        let (started, has_started) = std::sync::mpsc::channel();
        let (released, wait_for_release) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                signature(Path::new("/private/owned/Other.app"), TEAM_ID, |_, _, _| {
                    started.send(()).unwrap();
                    let _ = wait_for_release.recv();
                    Ok(String::new())
                })
            });
            has_started.recv().unwrap();
            let release = move || {
                let _ = released.send(());
            };
            let result = behind(&release);
            release();
            result
        })
    }

    #[trial]
    fn a_job_canceled_while_it_waits_for_the_signature_check_stops_there() {
        let refused = behind_a_running_check(|_| {
            let bundle = Path::new("/private/owned/PhotoCraft.app");
            let expect = Expect { team: Some(TEAM_ID), version: None, quit_first: false };
            let stop = Stop::default();
            let mut cancel_when_waiting = |p| {
                if p == JobPhase::Waiting {
                    stop.cancel();
                }
            };
            let wait = CheckTurn { stop: &stop, phase: &mut cancel_when_waiting };
            macos::check(bundle, &found_at(bundle), &expect, |_, _, _| Err(HelperFailure::Unfinished("codesign ran".into())), wait)
        });
        assert_eq!(refused, Err(CANCELLED.into()));
    }

    /// A copy waiting to replace the installed 0.5.0, and the installed copy.
    fn staged_and_installed(dir: &Path) -> (PathBuf, PathBuf) {
        (bundle(&dir.join("staged"), "0.6.0"), bundle(&dir.join("Applications"), "0.5.0"))
    }

    fn version_at(app: &Path) -> String {
        std::fs::read_to_string(app.join("Contents/MacOS/PhotoCraft")).unwrap()
    }

    /// The stand-in for `codesign` opens the app, so the app is running only once the check
    /// has started.
    #[trial]
    fn an_app_opened_during_the_signature_check_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let (staged, installed) = staged_and_installed(dir.path());
        let expect = Expect { team: Some(TEAM_ID), version: None, quit_first: true };
        let opened = std::cell::Cell::new(false);
        let run = |_: &str, _: &[&str], _: std::time::Duration| {
            opened.set(true);
            Ok(String::new())
        };
        let result = place_checked(photocraft(), &staged, &installed, &expect, run, |_| Ok(opened.get()));
        assert_eq!(result, Err("PhotoCraft is running. Quit it, then try again.".into()));
        assert_eq!(version_at(&installed), "0.5.0");
    }

    /// Detaches the image at a mount point when dropped, so a failing test leaves none attached.
    struct Attached(PathBuf);

    impl Drop for Attached {
        fn drop(&mut self) {
            if self.0.exists() {
                let _ = std::process::Command::new("hdiutil").args(["detach", "-force", "-quiet"]).arg(&self.0).status();
            }
        }
    }

    #[trial]
    fn an_install_that_ended_partway_does_not_block_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("Applications");
        let current = bundle(&apps, "0.5.0");
        let existing = Installed { version: None, source: InstallSource::MacApp, location: current.display().to_string(), package: None };
        let paths = Paths::under(dir.path());
        let image = dmg(dir.path(), "0.6.0");

        // What an install in a process that has ended left behind: its image still attached,
        // and the copy it was making beside the app.
        let mut ended = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = ended.id();
        ended.wait().unwrap();
        let mount = paths.cache.join(format!("mount-photocraft-{pid}-1"));
        std::fs::create_dir_all(&mount).unwrap();
        // The disk-image helper turns requests away while it is busy, as in `dmg`.
        let attached = {
            let _one_at_a_time = super::macos::DISK_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut said = String::new();
            let mut attached = false;
            for attempt in 0..5u64 {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(400 * attempt));
                }
                let output = std::process::Command::new("hdiutil")
                    .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-noverify", "-mountpoint"])
                    .arg(&mount)
                    .arg(&image)
                    .output()
                    .unwrap();
                if output.status.success() {
                    attached = true;
                    break;
                }
                said = String::from_utf8_lossy(&output.stderr).trim().to_string();
            }
            attached.then_some(()).ok_or(said)
        };
        let _attached = Attached(mount.clone());
        assert_eq!(attached, Ok(()), "hdiutil attach failed after 5 tries");
        let staging = apps.join(format!(".artcraft-launcher-staging-{pid}-Ab12Cd"));
        bundle(&staging, "0.6.0");
        std::fs::write(staging.join(".artcraft-launcher-staging"), "").unwrap();

        let result =
            install_dmg(photocraft(), &image, Some(&existing), None, false, &paths, &Expect { team: None, version: None, quit_first: false });
        assert_eq!(result, Ok(current.clone()));
        assert_eq!(std::fs::read_to_string(current.join("Contents/MacOS/PhotoCraft")).unwrap(), "0.6.0");
        assert!(!mount.exists(), "the earlier install's image is still attached");
        assert_eq!(
            std::fs::read_dir(&apps).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(),
            [std::ffi::OsString::from("PhotoCraft.app")]
        );
        let mounts = std::fs::read_dir(&paths.cache).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("mount-"));
        assert_eq!(mounts.count(), 0);
    }

    #[trial]
    fn only_staging_folders_of_installs_that_ended_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("Applications");
        let (ended, running, ours) = (4_000_001, 4_000_002, std::process::id());
        let staging = |pid: u32, rest: &str| {
            let folder = apps.join(format!(".artcraft-launcher-staging-{pid}-{rest}"));
            bundle(&folder, "0.6.0");
            std::fs::write(folder.join(".artcraft-launcher-staging"), "").unwrap();
            folder
        };
        let left = staging(ended, "Ab12Cd");
        let kept = [
            staging(running, "Ef34Gh"),
            staging(ours, "Ij56Kl"),
            // Not the name or the contents of a staging folder.
            staging(ended, "photo"),
            staging(ended, "Ab12Cd7"),
            staging(ended, "Ab-2Cd"),
            // Without the marker, a folder with something in it may be the user's.
            bundle(&apps.join(format!(".artcraft-launcher-staging-{ended}-Yz34Ab")), "0.6.0"),
            // A backup may hold the only copy of the app.
            bundle(&apps.join(".artcraft-launcher-backup-Mn78Op/old"), "0.5.0"),
            bundle(&apps.join(format!(".artcraft-launcher-staging-{ended}")), "0.6.0"),
            bundle(&apps.join(format!(".artcraft-launcher-staging-x{ended}-Uv12Wx")), "0.6.0"),
            bundle(&apps, "0.5.0"),
        ];
        // An install that stopped before it wrote the marker.
        let empty = apps.join(format!(".artcraft-launcher-staging-{ended}-Zz99Yy"));
        std::fs::create_dir(&empty).unwrap();
        let file = apps.join(format!(".artcraft-launcher-staging-{ended}-file"));
        std::fs::write(&file, "the user's").unwrap();
        let elsewhere = bundle(&dir.path().join("Elsewhere"), "0.4.0");
        let link = apps.join(format!(".artcraft-launcher-staging-{ended}-link"));
        std::os::unix::fs::symlink(elsewhere.parent().unwrap(), &link).unwrap();
        // Staging folders of launcher versions that didn't record the process.
        let old = bundle(&apps.join(".artcraft-launcher-Qr90St"), "0.6.0");
        let old_kept = [
            bundle(&apps.join(".artcraft-launcher-Qr90St7"), "0.6.0"),
            {
                let two = bundle(&apps.join(".artcraft-launcher-St12Uv"), "0.6.0");
                std::fs::write(apps.join(".artcraft-launcher-St12Uv/notes.txt"), "the user's").unwrap();
                two
            },
            {
                let other = apps.join(".artcraft-launcher-Wx34Yz/VectorCraft.app");
                std::fs::create_dir_all(&other).unwrap();
                other
            },
        ];
        let now = std::time::SystemTime::now();

        super::macos::clear_staging(photocraft(), &apps, |pid| pid == ended, now);
        assert!(!left.exists(), "the partial copy of an install that ended is still there");
        assert!(!empty.exists(), "the empty staging folder of an install that ended is still there");
        for path in kept.iter().chain([&file, &link, &old]).chain(&old_kept) {
            assert!(path.exists(), "{} was removed", path.display());
        }
        assert_eq!(std::fs::read_to_string(elsewhere.join("Contents/MacOS/PhotoCraft")).unwrap(), "0.4.0");

        // A day later, with every process still in the same state.
        let day = std::time::Duration::from_secs(24 * 60 * 60);
        super::macos::clear_staging(photocraft(), &apps, |pid| pid == ended, now + day + std::time::Duration::from_secs(60));
        assert!(!old.exists(), "the partial copy of an earlier launcher version's install is still there");
        for path in kept.iter().chain([&file, &link]).chain(&old_kept) {
            assert!(path.exists(), "{} was removed", path.display());
        }
    }

    #[trial]
    fn only_mount_points_of_installs_that_ended_are_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path();
        let (ended, running, ours) = (4_000_001, 4_000_002, std::process::id());
        let folder = |name: String| {
            let path = cache.join(name);
            std::fs::create_dir(&path).unwrap();
            path
        };
        let left = folder(format!("mount-photocraft-{ended}-1"));
        let busy = folder(format!("mount-photocraft-{ended}-2"));
        let own_left = folder(format!("mount-photocraft-{ours}-3"));
        let own_in_use = folder(format!("mount-photocraft-{ours}-4"));
        let _in_use = super::macos::InUse::new(&own_in_use);
        let kept = [
            folder(format!("mount-photocraft-{running}-5")),
            folder(format!("mount-vectorcraft-{ended}-6")),
            folder(format!("mount-photocraft-{ended}-x7")),
            folder(format!("mount-photocraft-{ended}")),
            folder("downloads".into()),
        ];

        let asked = std::cell::RefCell::new(Vec::new());
        super::macos::clear_mounts(
            photocraft(),
            cache,
            |pid| pid == ended,
            |mount| {
                asked.borrow_mut().push(mount.to_path_buf());
                if mount == busy { Err("Resource busy".into()) } else { Ok(()) }
            },
        );
        let mut asked = asked.into_inner();
        asked.sort();
        let mut cleared = vec![left.clone(), busy.clone(), own_left.clone()];
        cleared.sort();
        assert_eq!(asked, cleared);
        assert!(!left.exists() && !own_left.exists());
        for path in kept.iter().chain([&busy, &own_in_use]) {
            assert!(path.exists(), "{} was removed", path.display());
        }
    }

    #[trial]
    fn a_failed_attach_detaches_only_unmounted_attachments_of_its_image() {
        let image = |path: &str, entities: &[(&str, Option<&str>)]| {
            let entities: String = entities
                .iter()
                .map(|(dev, mount)| {
                    let mount = mount.map(|m| format!("<key>mount-point</key><string>{m}</string>")).unwrap_or_default();
                    format!("<dict><key>dev-entry</key><string>{dev}</string>{mount}</dict>")
                })
                .collect();
            format!("<dict><key>image-path</key><string>{path}</string><key>system-entities</key><array>{entities}</array></dict>")
        };
        let ours = "/Users/a/Library/Caches/craft/photocraft-0.6.0.dmg";
        let info = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>images</key><array>{}{}{}{}</array></dict></plist>",
            image(ours, &[("/dev/disk5", None), ("/dev/disk5s1", None), ("/dev/disk9", None)]),
            image(ours, &[("/dev/disk6", None), ("/dev/disk6s1", Some("/Users/a/Library/Caches/craft/mount-photocraft-1-2"))]),
            image("/Users/a/Downloads/other.dmg", &[("/dev/disk7", None)]),
            image(&format!("/private{ours}"), &[("/dev/disk8", None)]),
        );
        let paths = [PathBuf::from(ours)];
        assert_eq!(super::macos::unmounted_attachments(&info, &paths), ["/dev/disk5"]);
        let both = [PathBuf::from(ours), PathBuf::from(format!("/private{ours}"))];
        assert_eq!(super::macos::unmounted_attachments(&info, &both), ["/dev/disk5", "/dev/disk8"]);
        assert!(super::macos::unmounted_attachments("hdiutil: info failed", &paths).is_empty());
    }
}
