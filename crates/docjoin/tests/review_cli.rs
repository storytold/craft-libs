//! `--check` in GitHub Actions on a pull request's merge commit, checked out the way
//! `actions/checkout` does it: one commit deep, so `HEAD^1` (the base) is not there yet.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_docjoin::{Config, Log, cli};
use std::path::{Path, PathBuf};
use std::process::Command;

const CFG: Config =
    Config { logs: &[Log { dir: "docs/progress.d", doc: "ROADMAP.md", heading: "## Progress log", status: true, ..Log::DEFAULT }], tables: &[] };

const CI_DIR: &str = "DOCJOIN_TEST_CI_DIR";
const CI_HAND: &str = "DOCJOIN_TEST_CI_HAND";

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// An upstream repo with a pull request merged into a `main` that moved on, and a CI checkout of
/// its merge commit. With `hand`, the pull request also edits the log by hand.
fn ci_checkout(top: &Path, hand: bool) -> PathBuf {
    let _ = std::fs::remove_dir_all(top);
    let (up, ci) = (top.join("up"), top.join("ci"));
    std::fs::create_dir_all(&up).unwrap();
    std::fs::create_dir_all(&ci).unwrap();
    git(&up, &["init", "-q", "-b", "main"]);
    write(&up, "ROADMAP.md", "# R\n\n> **Last updated:** 2026-01-01 · **Change:** minor\n\n## Progress log\n\n- 2026-01-01: start\n");
    write(&up, "docs/progress.d/README.md", "how\n");
    git(&up, &["add", "-A"]);
    git(&up, &["commit", "-q", "-m", "base"]);
    git(&up, &["commit", "-q", "--allow-empty", "-m", "main moves on"]);
    git(&up, &["checkout", "-q", "-b", "pr", "HEAD~1"]);
    write(&up, "docs/progress.d/7-thing.md", "- 2026-02-02: thing\n");
    if hand {
        write(&up, "ROADMAP.md", "# R\n\n> **Last updated:** 2026-01-01 · **Change:** minor\n\n## Progress log\n\n- 2026-01-01: started\n");
    }
    git(&up, &["add", "-A"]);
    git(&up, &["commit", "-q", "-m", "pr"]);
    git(&up, &["checkout", "-q", "main"]);
    git(&up, &["merge", "-q", "--no-ff", "pr", "-m", "Merge pr into main"]);
    let merge = git(&up, &["rev-parse", "HEAD"]);
    git(&up, &["update-ref", "refs/pull/1/merge", &merge]);
    git(&up, &["reset", "-q", "--hard", "HEAD^1"]);

    // What actions/checkout does on `pull_request` with the default fetch-depth 1.
    git(&ci, &["init", "-q"]);
    git(&ci, &["remote", "add", "origin", &format!("file://{}", up.display())]);
    git(&ci, &["fetch", "-q", "--no-tags", "--depth=1", "origin", &format!("+{merge}:refs/remotes/pull/1/merge")]);
    git(&ci, &["checkout", "-q", "--detach", "refs/remotes/pull/1/merge"]);
    ci
}

/// Runs in a child process that has `GITHUB_ACTIONS` set (the parent can't set it safely).
#[test]
#[ignore = "run by check_on_a_pull_request_merge_commit_checkout"]
fn check_in_ci_child() {
    let Some(ci) = std::env::var_os(CI_DIR) else { return };
    let result = cli::run(Path::new(&ci), &CFG, &["--check".to_string()]);
    if std::env::var_os(CI_HAND).is_some() {
        assert!(result.unwrap_err().contains("edited by hand"));
    } else {
        result.unwrap();
    }
}

fn check_in_ci(hand: bool) -> String {
    let top = std::env::temp_dir().join(format!("docjoin-ci-shallow-{}-{hand}", std::process::id()));
    let ci = ci_checkout(&top, hand);
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.args(["--exact", "check_in_ci_child", "--include-ignored", "--nocapture"]).env("GITHUB_ACTIONS", "true").env(CI_DIR, &ci);
    if hand {
        child.env(CI_HAND, "1");
    }
    let out = child.output().unwrap();
    let _ = std::fs::remove_dir_all(&top);
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && log.contains("1 passed"), "{log}");
    log
}

#[test]
fn check_on_a_pull_request_merge_commit_checkout() {
    check_in_ci(false);
}

/// The base really is fetched and compared: a hand edit on the same shallow checkout fails.
#[test]
fn check_on_a_merge_commit_checkout_fails_a_hand_edit() {
    check_in_ci(true);
}
