//! Performing an update: fetch the release tag into the updater's own clone, build it with the
//! configured command, copy the output into a new release folder and point the served symbolic
//! link at it in one atomic `rename`. The site keeps serving the previous release until that
//! last step, and a failed fetch or build swaps nothing.
//!
//! Every program runs without a shell, with arguments from the configuration file and the
//! release tag (already held to `v<version>`, see [`crate::release::Release::tag`]), never from
//! a web request.

use super::config::{Build, source_version};
use crate::Error;
use crate::release::Release;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Performs an update. `craft-updated` uses [`CommandRunner`]; tests use stand-ins.
pub trait Runner: Send + Sync {
    /// Makes `release` live, writing progress to `log`.
    fn run(&self, release: &Release, log: &File) -> Result<(), Error>;
}

/// The real update: git, the build command, copy and swap.
pub struct CommandRunner {
    pub build: Build,
}

/// Most files and folder depth copied from a build's output.
const MAX_FILES: usize = 100_000;
const MAX_DEPTH: usize = 32;

impl Runner for CommandRunner {
    fn run(&self, release: &Release, log: &File) -> Result<(), Error> {
        let b = &self.build;
        let git = |args: &[&str]| {
            let mut all = vec!["-C".to_string(), b.source_dir.display().to_string(), "-c".into(), "advice.detachedHead=false".into()];
            all.extend(args.iter().map(|a| a.to_string()));
            run_command(log, "git", &all, &b.source_dir, Duration::from_secs(600))
        };
        git(&["fetch", "--tags", "--no-recurse-submodules", &b.remote])?;
        let tag_ref = format!("refs/tags/{}", release.tag);
        git(&["rev-parse", "--verify", "--quiet", &format!("{tag_ref}^{{commit}}")])?;
        git(&["checkout", "--detach", &tag_ref])?;
        match source_version(&b.source_dir) {
            Ok(v) if v == release.version => {}
            Ok(v) => note(log, &format!("warning: {} says version {v}, the tag says {}", b.source_dir.join("Cargo.toml").display(), release.version)),
            Err(e) => note(log, &format!("warning: {e}")),
        }

        let output = b.source_dir.join(&b.output_dir);
        if output.exists() {
            fs::remove_dir_all(&output).map_err(|e| fail(log, format!("couldn't clear the old build output {}: {e}", output.display())))?;
        }
        let (program, args) = b.command.split_first().ok_or_else(|| Error::Update("build.command is empty".into()))?;
        let timeout = Duration::from_secs(b.timeout_minutes.saturating_mul(60));
        run_command(log, program, args, &b.source_dir.join(&b.work_dir), timeout)?;
        if !output.join("index.html").is_file() {
            return Err(fail(log, format!("the build finished but {} has no index.html", output.display())));
        }

        fs::create_dir_all(&b.releases_dir).map_err(|e| fail(log, format!("couldn't create {}: {e}", b.releases_dir.display())))?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let target = b.releases_dir.join(format!("{}-{stamp}", release.version));
        note(log, &format!("copying {} to {}", output.display(), target.display()));
        let mut files = 0;
        copy_tree(&output, &target, 0, &mut files).map_err(|e| {
            let _ = fs::remove_dir_all(&target);
            fail(log, e)
        })?;
        note(log, &format!("pointing {} at {}", b.live_link.display(), target.display()));
        swap_live(&b.live_link, &target).map_err(|e| fail(log, e))?;
        prune_releases(&b.releases_dir, &target, b.keep_releases, log);
        note(log, &format!("{} is live", release.version));
        Ok(())
    }
}

fn note(log: &File, line: &str) {
    let mut log = log;
    let _ = writeln!(log, "{line}");
}

fn fail(log: &File, message: String) -> Error {
    note(log, &format!("error: {message}"));
    Error::Update(message)
}

/// Runs `program args…` in `dir` with its output appended to `log`, killing it after `timeout`.
pub fn run_command(log: &File, program: &str, args: &[String], dir: &Path, timeout: Duration) -> Result<(), Error> {
    note(log, &format!("$ {program} {}", args.join(" ")));
    let stdout = log.try_clone().map_err(|e| Error::Update(format!("the update log: {e}")))?;
    let stderr = log.try_clone().map_err(|e| Error::Update(format!("the update log: {e}")))?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| fail(log, format!("couldn't start {program}: {e}")))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(fail(log, format!("{program} failed ({status})"))),
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(fail(log, format!("{program} took longer than {} minutes and was stopped", timeout.as_secs() / 60)));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(fail(log, format!("waiting for {program}: {e}"))),
        }
    }
}

/// Copies a build's output; symbolic links are refused rather than followed.
fn copy_tree(from: &Path, to: &Path, depth: usize, files: &mut usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{} is nested too deeply", from.display()));
    }
    fs::create_dir(to).map_err(|e| format!("couldn't create {}: {e}", to.display()))?;
    let entries = fs::read_dir(from).map_err(|e| format!("couldn't read {}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("couldn't read {}: {e}", from.display()))?;
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let dest = to.join(entry.file_name());
        *files += 1;
        if *files > MAX_FILES {
            return Err(format!("the build output has more than {MAX_FILES} files"));
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest, depth + 1, files)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &dest).map_err(|e| format!("couldn't copy {}: {e}", entry.path().display()))?;
        } else {
            return Err(format!("{} isn't a regular file or folder", entry.path().display()));
        }
    }
    Ok(())
}

/// Points `link` at `target` atomically: a new link beside it, renamed over it. `link` must be a
/// symbolic link already, or not exist yet.
#[cfg(unix)]
pub fn swap_live(link: &Path, target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(link) {
        Ok(meta) if !meta.file_type().is_symlink() => {
            return Err(format!(
                "{} exists and isn't a symbolic link. Move the folder into build.releases_dir and replace it with a link to it (see the README)",
                link.display()
            ));
        }
        _ => {}
    }
    let name = link.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "live".into());
    let staged = link.with_file_name(format!(".{name}.craft-update-{}", std::process::id()));
    let _ = fs::remove_file(&staged);
    std::os::unix::fs::symlink(target, &staged).map_err(|e| format!("couldn't create {}: {e}", staged.display()))?;
    fs::rename(&staged, link).map_err(|e| {
        let _ = fs::remove_file(&staged);
        format!("couldn't replace {}: {e}", link.display())
    })
}

#[cfg(not(unix))]
pub fn swap_live(_link: &Path, _target: &Path) -> Result<(), String> {
    Err("swapping the served folder atomically needs symbolic links; craft-updated performs updates on Unix only".into())
}

/// Removes release folders beyond the newest `keep`, never `current`, and only folders this
/// crate named (`<version>-<unix time>`).
fn prune_releases(dir: &Path, current: &Path, keep: usize, log: &File) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut releases: Vec<(u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (version, stamp) = name.rsplit_once('-')?;
            crate::Version::parse(version)?;
            Some((stamp.parse().ok()?, e.path()))
        })
        .collect();
    releases.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (_, path) in releases.into_iter().skip(keep) {
        if path != current {
            note(log, &format!("removing the old release {}", path.display()));
            if let Err(e) = fs::remove_dir_all(&path) {
                note(log, &format!("warning: couldn't remove {}: {e}", path.display()));
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_live_link_is_swapped_and_a_real_folder_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("releases/0.4.0-1"), dir.path().join("releases/0.6.1-2"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let live = dir.path().join("live");
        swap_live(&live, &a).unwrap();
        assert_eq!(fs::read_link(&live).unwrap(), a);
        swap_live(&live, &b).unwrap();
        assert_eq!(fs::read_link(&live).unwrap(), b);
        let folder = dir.path().join("www");
        fs::create_dir(&folder).unwrap();
        assert!(swap_live(&folder, &b).unwrap_err().contains("isn't a symbolic link"));
        assert!(folder.is_dir() && !folder.is_symlink());
    }

    #[test]
    fn old_releases_are_pruned_but_never_the_live_one_or_foreign_folders() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["0.3.0-1", "0.4.0-2", "0.5.0-3", "0.6.1-4", "keep-me", "backup-5"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        let log = tempfile::tempfile().unwrap();
        prune_releases(dir.path(), &dir.path().join("0.3.0-1"), 2, &log);
        let mut left: Vec<String> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["0.3.0-1", "0.5.0-3", "0.6.1-4", "backup-5", "keep-me"]);
    }

    #[test]
    fn build_output_with_symlinks_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        fs::create_dir_all(out.join("assets")).unwrap();
        fs::write(out.join("index.html"), "<html>").unwrap();
        fs::write(out.join("assets/app.wasm"), [0u8, 97, 115, 109]).unwrap();
        let mut n = 0;
        copy_tree(&out, &dir.path().join("copy"), 0, &mut n).unwrap();
        assert_eq!(fs::read(dir.path().join("copy/assets/app.wasm")).unwrap(), [0u8, 97, 115, 109]);
        std::os::unix::fs::symlink("/etc/passwd", out.join("leak")).unwrap();
        let mut n = 0;
        assert!(copy_tree(&out, &dir.path().join("copy2"), 0, &mut n).is_err());
    }

    #[test]
    fn a_command_that_overruns_its_timeout_is_stopped() {
        let log = tempfile::tempfile().unwrap();
        let started = Instant::now();
        let err = run_command(&log, "sleep", &["30".into()], Path::new("/"), Duration::from_millis(300)).unwrap_err();
        assert!(err.to_string().contains("was stopped"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(run_command(&log, "true", &[], Path::new("/"), Duration::from_secs(10)).is_ok());
        assert!(run_command(&log, "false", &[], Path::new("/"), Duration::from_secs(10)).is_err());
    }
}
