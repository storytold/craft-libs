//! Smoke test of the shell: a real git repo, a fragment, `--check`, then `--write`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_docjoin::{Config, Log, Table, cli};
use std::path::Path;
use std::process::Command;

const CFG: Config = Config {
    logs: &[Log { dir: "docs/progress.d", doc: "ROADMAP.md", heading: "## Progress log", status: true, ..Log::DEFAULT }],
    tables: &[Table { dir: "docs/gaps.d", doc: "docs/gaps.md", heading: "## Ranked", columns: &[("#", "number"), ("Gap", "gap")], ..Table::DEFAULT }],
};

fn git(root: &Path, args: &[&str]) {
    git_at(root, "2026-01-01T00:00:00Z", args);
}

fn git_at(root: &Path, date: &str, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_AUTHOR_DATE", date)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn repo(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("docjoin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    dir
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_string()).collect()
}

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn check_then_fold_in_a_real_repo() {
    let dir = repo("smoke");
    let root = dir.as_path();
    write(root, "ROADMAP.md", "# R\n\n> **Last updated:** 2026-01-01 · **Change:** minor\n\n## Progress log\n\n- 2026-01-01: start\n");
    write(root, "docs/gaps.md", "## Ranked\n\n| # | Gap |\n|---|---|\n| 1 | One |\n");
    write(root, "docs/gaps.d/1-one.md", "---\ngap: One\n---\n");
    write(root, "docs/gaps.d/README.md", "how\n");
    write(root, "docs/progress.d/README.md", "how\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["checkout", "-q", "-b", "pr"]);
    write(root, "docs/progress.d/7-thing.md", "- 2026-02-02: thing\n");
    git(root, &["add", "-A"]);
    git_at(root, "2026-02-02T12:00:00Z", &["commit", "-q", "-m", "pr"]);

    cli::run(root, &CFG, &args(&["--check", "--base", "main"])).unwrap();

    write(root, "docs/gaps.md", "## Ranked\n\n| # | Gap |\n|---|---|\n| 1 | One by hand |\n");
    let err = cli::run(root, &CFG, &args(&["--check", "--base", "main"])).unwrap_err();
    assert!(err.contains("edited by hand"), "{err}");
    git(root, &["checkout", "-q", "docs/gaps.md"]);

    cli::run(root, &CFG, &args(&["--write"])).unwrap();
    let roadmap = std::fs::read_to_string(root.join("ROADMAP.md")).unwrap();
    assert!(roadmap.contains("## Progress log\n\n- 2026-02-02: thing\n- 2026-01-01: start\n"), "{roadmap}");
    assert!(!roadmap.contains("2026-01-01 · **Change"), "status date moves: {roadmap}");
    assert!(!root.join("docs/progress.d/7-thing.md").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

const ROADMAP: &str = "# R\n\n> **Last updated:** 2026-01-01 · **Change:** minor\n\n## Progress log\n\n- 2026-01-01: start\n";

#[test]
fn a_fragment_is_dated_by_the_commit_that_added_it() {
    let dir = repo("added");
    let root = dir.as_path();
    write(root, "ROADMAP.md", ROADMAP);
    write(root, "docs/gaps.md", "## Ranked\n\n| # | Gap |\n|---|---|\n| 1 | One |\n");
    write(root, "docs/gaps.d/1-one.md", "---\ngap: One\n---\n");
    write(root, "docs/progress.d/1-a.md", "- a\n");
    git_at(root, "2026-02-01T12:00:00Z", &["add", "-A"]);
    git_at(root, "2026-02-01T12:00:00Z", &["commit", "-q", "-m", "a"]);
    write(root, "docs/progress.d/2-b.md", "- b\n");
    git_at(root, "2026-02-02T12:00:00Z", &["add", "-A"]);
    git_at(root, "2026-02-02T12:00:00Z", &["commit", "-q", "-m", "b"]);
    write(root, "docs/progress.d/1-a.md", "- a, reworded\n");
    git_at(root, "2026-02-03T12:00:00Z", &["commit", "-q", "-am", "a again"]);
    cli::run(root, &CFG, &args(&["--write"])).unwrap();
    let roadmap = std::fs::read_to_string(root.join("ROADMAP.md")).unwrap();
    assert!(roadmap.contains("## Progress log\n\n- b\n- a, reworded\n- 2026-01-01: start\n"), "{roadmap}");
    assert!(roadmap.contains("**Last updated:** 2026-02-02 ·"), "{roadmap}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn migrate_takes_only_a_plain_name() {
    let dir = repo("name");
    let root = dir.as_path();
    write(root, "ROADMAP.md", ROADMAP);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    for bad in ["../x", "X", "a b", ""] {
        let err = cli::run(root, &CFG, &args(&["migrate", "main", "main", bad])).unwrap_err();
        assert!(err.contains("[a-z0-9-]"), "{err}");
    }
    let err = cli::run(root, &CFG, &args(&["--check", "--base", "--output=x"])).unwrap_err();
    assert!(!root.join("x").exists(), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn symlinks_are_skipped() {
    let dir = repo("link");
    let root = dir.as_path();
    write(root, "ROADMAP.md", ROADMAP);
    write(root, "docs/gaps.md", "## Ranked\n\n| # | Gap |\n|---|---|\n");
    write(root, "docs/progress.d/README.md", "how\n");
    write(root, "elsewhere.md", "- from a link\n");
    std::fs::create_dir_all(root.join("docs/progress.d")).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere.md"), root.join("docs/progress.d/1-link.md")).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    cli::run(root, &CFG, &args(&["--write"])).unwrap();
    assert!(!std::fs::read_to_string(root.join("ROADMAP.md")).unwrap().contains("from a link"));
    assert!(root.join("docs/progress.d/1-link.md").symlink_metadata().is_ok());
    // Nor read from git: a pull request removing the link folds nothing.
    git(root, &["checkout", "-q", "-b", "pr"]);
    git(root, &["rm", "-q", "docs/progress.d/1-link.md"]);
    git(root, &["commit", "-q", "-m", "unlink"]);
    cli::run(root, &CFG, &args(&["--check", "--base", "main"])).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cutover_leaves_started_tables_alone() {
    const TWO: Config = Config {
        logs: &[],
        tables: &[
            Table { dir: "docs/a.d", doc: "docs/t.md", heading: "## A", columns: &[("#", "number"), ("Gap", "gap")], ..Table::DEFAULT },
            Table { dir: "docs/b.d", doc: "docs/t.md", heading: "## B", columns: &[("#", "number"), ("Gap", "gap")], ..Table::DEFAULT },
        ],
    };
    let dir = repo("cutover");
    let root = dir.as_path();
    write(root, "docs/t.md", "## A\n\n| # | Gap |\n|---|---|\n| 1 | One |\n\n## B\n\n| # | Gap |\n|---|---|\n| 1 | Uno |\n| 2 | Dos |\n");
    write(root, "docs/a.d/1-one.md", "---\ngap: One, from its item\n---\n");
    write(root, "docs/a.d/README.md", "how\n");
    cli::run(root, &TWO, &args(&["--cutover"])).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("docs/a.d/1-one.md")).unwrap(), "---\ngap: One, from its item\n---\n");
    assert_eq!(std::fs::read_dir(root.join("docs/a.d")).unwrap().count(), 2);
    assert!(root.join("docs/b.d/2-dos.md").exists());
    let t = std::fs::read_to_string(root.join("docs/t.md")).unwrap();
    assert!(t.contains("| 1 | One, from its item |") && t.contains("| 2 | Dos |"), "{t}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Makes the file `path` read-only for `f`; `None` when that doesn't stop writes (running as root).
#[cfg(unix)]
fn read_only<T>(path: &Path, f: impl FnOnce() -> T) -> Option<T> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let blocked = std::fs::OpenOptions::new().append(true).open(path).is_err();
    let out = blocked.then(f);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    out
}

/// A doc that can't be written keeps its fragments, so nothing is lost and a rerun folds them
/// once; a doc written before it has its own fragments deleted, so a rerun doesn't fold them twice.
#[cfg(unix)]
#[test]
fn a_failed_doc_write_keeps_its_fragments() {
    const LOGS: Config = Config {
        logs: &[
            Log { dir: "docs/progress.d", doc: "ROADMAP.md", heading: "## Progress log", ..Log::DEFAULT },
            Log { dir: "docs/sub/other.d", doc: "docs/sub/other.md", heading: "## Log", ..Log::DEFAULT },
        ],
        tables: &[],
    };
    let dir = repo("order");
    let root = dir.as_path();
    write(root, "ROADMAP.md", ROADMAP);
    write(root, "docs/sub/other.md", "## Log\n\n- old\n");
    write(root, "docs/progress.d/README.md", "how\n");
    write(root, "docs/sub/other.d/README.md", "how\n");
    write(root, "docs/progress.d/1-a.md", "- a\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    // Not committed: only the working tree has it.
    write(root, "docs/sub/other.d/2-b.md", "- b\n");
    let Some(err) = read_only(&root.join("docs/sub/other.md"), || cli::run(root, &LOGS, &args(&["--write"]))) else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    assert!(err.is_err());
    assert_eq!(std::fs::read_to_string(root.join("docs/sub/other.md")).unwrap(), "## Log\n\n- old\n");
    assert!(root.join("docs/sub/other.d/2-b.md").exists(), "the fragment's entry is lost");
    assert!(std::fs::read_to_string(root.join("ROADMAP.md")).unwrap().contains("- a\n"));
    assert!(!root.join("docs/progress.d/1-a.md").exists(), "a rerun would fold it again");
    cli::run(root, &LOGS, &args(&["--write"])).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("docs/sub/other.md")).unwrap(), "## Log\n\n- b\n- old\n");
    assert_eq!(std::fs::read_to_string(root.join("ROADMAP.md")).unwrap().matches("- a\n").count(), 1);
    assert!(!root.join("docs/sub/other.d/2-b.md").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A cutover that can't write every item of a table writes none of them: one item left behind
/// would make the table look started, and the fold would drop the rest of its rows.
#[test]
fn a_failed_cutover_leaves_the_table_unstarted() {
    const ONE: Config = Config {
        logs: &[],
        tables: &[Table { dir: "docs/b.d", doc: "docs/t.md", heading: "## B", columns: &[("#", "number"), ("Gap", "gap")], ..Table::DEFAULT }],
    };
    let dir = repo("cutover-fail");
    let root = dir.as_path();
    let doc = "## B\n\n| # | Gap |\n|---|---|\n| 1 | Uno |\n| 2 | Dos |\n";
    write(root, "docs/t.md", doc);
    write(root, "docs/b.d/README.md", "how\n");
    // A folder where the second item goes: its write fails.
    std::fs::create_dir_all(root.join("docs/b.d/2-dos.md")).unwrap();
    assert!(cli::run(root, &ONE, &args(&["--cutover"])).is_err());
    assert!(!root.join("docs/b.d/1-uno.md").exists(), "the table now looks started");
    std::fs::remove_dir(root.join("docs/b.d/2-dos.md")).unwrap();
    cli::run(root, &ONE, &args(&["--cutover"])).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("docs/t.md")).unwrap(), doc);
    assert!(root.join("docs/b.d/1-uno.md").exists() && root.join("docs/b.d/2-dos.md").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `migrate` never overwrites a different pending fragment of the same name; rerun, it writes the
/// same one again.
#[test]
fn migrate_keeps_a_different_fragment_of_its_name() {
    let dir = repo("migrate-clash");
    let root = dir.as_path();
    write(root, "ROADMAP.md", ROADMAP);
    write(root, "docs/progress.d/README.md", "how\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["checkout", "-q", "-b", "pr"]);
    write(root, "ROADMAP.md", &ROADMAP.replace("## Progress log\n\n", "## Progress log\n\n- 2026-02-02: from the pr\n"));
    git(root, &["commit", "-q", "-am", "pr"]);
    git(root, &["checkout", "-q", "main"]);
    write(root, "docs/progress.d/5-x.md", "- someone else's\n");
    let err = cli::run(root, &CFG, &args(&["migrate", "main", "pr", "5-x"])).unwrap_err();
    assert!(err.contains("docs/progress.d/5-x.md"), "{err}");
    assert_eq!(std::fs::read_to_string(root.join("docs/progress.d/5-x.md")).unwrap(), "- someone else's\n");
    std::fs::remove_file(root.join("docs/progress.d/5-x.md")).unwrap();
    cli::run(root, &CFG, &args(&["migrate", "main", "pr", "5-x"])).unwrap();
    cli::run(root, &CFG, &args(&["migrate", "main", "pr", "5-x"])).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("docs/progress.d/5-x.md")).unwrap(), "- 2026-02-02: from the pr\n");
    let _ = std::fs::remove_dir_all(&dir);
}
