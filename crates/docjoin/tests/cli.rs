//! Smoke test of the shell: a real git repo, a fragment, `--check`, then `--write`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_docjoin::{Config, Log, Table, cli};
use std::path::Path;
use std::process::Command;

const CFG: Config = Config {
    logs: &[Log { dir: "docs/progress.d", doc: "ROADMAP.md", heading: "## Progress log", newest_first: true, status: true }],
    tables: &[Table { dir: "docs/gaps.d", doc: "docs/gaps.md", heading: "## Ranked", columns: &[("#", "number"), ("Gap", "gap")], ..Table::DEFAULT }],
};

fn git(root: &Path, args: &[&str]) {
    let ok =
        Command::new("git").arg("-C").arg(root).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).status().unwrap().success();
    assert!(ok, "git {args:?}");
}

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn check_then_fold_in_a_real_repo() {
    let dir = std::env::temp_dir().join(format!("docjoin-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.as_path();
    git(root, &["init", "-q", "-b", "main"]);
    write(root, "ROADMAP.md", "# R\n\n> **Last updated:** 2026-01-01 · **Change:** minor\n\n## Progress log\n\n- 2026-01-01: start\n");
    write(root, "docs/gaps.md", "## Ranked\n\n| # | Gap |\n|---|---|\n| 1 | One |\n");
    write(root, "docs/gaps.d/1-one.md", "---\ngap: One\n---\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["checkout", "-q", "-b", "pr"]);
    write(root, "docs/progress.d/7-thing.md", "- 2026-02-02: thing\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "pr"]);

    let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
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
