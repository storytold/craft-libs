//! `cargo xtask progress [--write | --check [--base <rev>] | --cutover | migrate <old> <pr> <name>]`:
//! the thin shell an app's xtask calls with its [`Config`]. It reads files and asks `git` for
//! commit times and the merge base; everything else is the pure core.
//!
//! - no flag: print each doc as it reads with pending fragments and items joined in.
//! - `--write`: the fold. Write the docs, delete the consumed fragments.
//! - `--check`: CI. Bad fragments or items, or a joined region edited by hand, fail. The base is
//!   the merge base with `--base <rev>`; in GitHub Actions, `HEAD^1`; else the merge base with
//!   `origin/main`.
//! - `--cutover`: once per repo. Turn every table row into an item file, then fold.
//! - `migrate <old> <pr> <name>`: rewrite a pull request opened before the cutover. `<old>` is its
//!   merge base, `<pr>` its head; the new fragment is `<name>.md`. Run on the PR's branch, merged
//!   with the current `main`.

use crate::{Config, File, MAX_DOC, MAX_FILE, MAX_FILES, check, items_from_doc, join, migrate};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn paths(cfg: &Config) -> Vec<&'static str> {
    let mut p: Vec<&str> = cfg.logs.iter().flat_map(|l| [l.doc, l.dir]).chain(cfg.tables.iter().flat_map(|t| [t.doc, t.dir])).collect();
    p.sort_unstable();
    p.dedup();
    p
}

/// The last commit time of each path under `paths`, at `rev`.
fn times(root: &Path, rev: &str, paths: &[&str]) -> Result<BTreeMap<String, i64>, String> {
    let mut args = vec!["log", "--format=%x01%ct", "--name-only", rev, "--"];
    args.extend(paths);
    let mut out = BTreeMap::new();
    let mut now = 0;
    for line in git(root, &args)?.lines() {
        if let Some(t) = line.strip_prefix('\u{1}') {
            now = t.trim().parse().unwrap_or(0);
        } else if !line.is_empty() {
            out.entry(line.to_string()).or_insert(now);
        }
    }
    Ok(out)
}

/// The configured docs and every file in the configured folders, from the working tree.
fn working(root: &Path, cfg: &Config) -> Result<Vec<File>, String> {
    let known = times(root, "HEAD", &paths(cfg)).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let mut names = Vec::new();
    for p in paths(cfg) {
        let full = root.join(p);
        if full.is_file() {
            names.push(p.to_string());
        } else if full.is_dir() {
            for entry in std::fs::read_dir(&full).map_err(|e| format!("{p}: {e}"))? {
                let entry = entry.map_err(|e| format!("{p}: {e}"))?;
                if entry.file_type().map_err(|e| format!("{p}: {e}"))?.is_file() {
                    names.push(format!("{p}/{}", entry.file_name().to_string_lossy()));
                }
                if names.len() > MAX_FILES * 4 {
                    return Err(format!("{p}: too many files"));
                }
            }
        }
    }
    let mut out = Vec::new();
    for name in names {
        let full = root.join(&name);
        let cap = if paths(cfg).contains(&name.as_str()) { MAX_DOC } else { MAX_FILE };
        if std::fs::metadata(&full).map_err(|e| format!("{name}: {e}"))?.len() > cap as u64 {
            return Err(format!("{name}: longer than {cap} bytes"));
        }
        let text = std::fs::read_to_string(&full).map_err(|e| format!("{name}: {e}"))?;
        let time = known.get(&name).copied().unwrap_or(now);
        out.push(File { path: name, text, time });
    }
    Ok(out)
}

/// The same files at `rev`, from git.
fn at_rev(root: &Path, cfg: &Config, rev: &str) -> Result<Vec<File>, String> {
    let ps = paths(cfg);
    let known = times(root, rev, &ps)?;
    let mut args = vec!["ls-tree", "-r", "--name-only", rev, "--"];
    args.extend(&ps);
    let mut out = Vec::new();
    for name in git(root, &args)?.lines().filter(|l| !l.is_empty()) {
        // Only the configured docs and the files directly in a configured folder.
        if !ps.iter().any(|p| name == *p || name.strip_prefix(p).and_then(|r| r.strip_prefix('/')).is_some_and(|r| !r.contains('/'))) {
            continue;
        }
        if out.len() > MAX_FILES * 4 {
            return Err(format!("{rev}: too many files"));
        }
        let text = git(root, &["show", &format!("{rev}:{name}")])?;
        out.push(File { path: name.to_string(), time: known.get(name).copied().unwrap_or(0), text });
    }
    Ok(out)
}

fn write_all(root: &Path, files: &[File]) -> Result<(), String> {
    for f in files {
        let full = root.join(&f.path);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", f.path))?;
        }
        std::fs::write(&full, &f.text).map_err(|e| format!("{}: {e}", f.path))?;
    }
    Ok(())
}

fn fold(root: &Path, cfg: &Config, files: &[File]) -> Result<usize, String> {
    let joined = join(cfg, files).map_err(|e| e.join("\n"))?;
    let changed: Vec<File> = joined
        .docs
        .iter()
        .filter(|(p, t)| files.iter().find(|f| &f.path == *p).is_none_or(|f| &f.text != *t))
        .map(|(p, t)| File { path: p.clone(), text: t.clone(), time: 0 })
        .collect();
    write_all(root, &changed)?;
    for path in &joined.consumed {
        std::fs::remove_file(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(joined.consumed.len())
}

/// Runs the command `args` (without the program name) in the repo at `root`.
pub fn run(root: &Path, cfg: &Config, args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => {
            let files = working(root, cfg)?;
            let joined = join(cfg, &files).map_err(|e| e.join("\n"))?;
            for (doc, text) in &joined.docs {
                println!("==> {doc} <==");
                print!("{text}");
            }
            eprintln!("progress: {} fragment(s) pending (nothing changed; --write folds them)", joined.consumed.len());
            Ok(())
        }
        ["--write"] => {
            let n = fold(root, cfg, &working(root, cfg)?)?;
            eprintln!("progress: folded {n} fragment(s)");
            Ok(())
        }
        ["--check", rest @ ..] => {
            let base = match rest {
                ["--base", rev] => git(root, &["merge-base", "HEAD", rev])?.trim().to_string(),
                // CI checks out the commit being tested alone; its first parent is the base it
                // merges into (a pull request's merge commit, or the previous `main`).
                [] if std::env::var_os("GITHUB_ACTIONS").is_some() => {
                    if git(root, &["rev-parse", "--verify", "-q", "HEAD^1"]).is_err() {
                        let head = git(root, &["rev-parse", "HEAD"])?;
                        git(root, &["fetch", "-q", "--no-tags", "--depth=2", "origin", head.trim()])?;
                    }
                    "HEAD^1".to_string()
                }
                [] => git(root, &["merge-base", "HEAD", "origin/main"])?.trim().to_string(),
                _ => return Err("usage: progress --check [--base <rev>]".into()),
            };
            let mb = base;
            let errors = check(cfg, &at_rev(root, cfg, mb.trim())?, &working(root, cfg)?);
            if errors.is_empty() {
                eprintln!("progress: fragments, items and joined regions are fine");
                Ok(())
            } else {
                Err(errors.join("\n"))
            }
        }
        ["--cutover"] => {
            let files = working(root, cfg)?;
            let (items, notes) = items_from_doc(cfg, &files).map_err(|e| e.join("\n"))?;
            write_all(root, &items)?;
            for n in &notes {
                eprintln!("note: {n}");
            }
            fold(root, cfg, &working(root, cfg)?)?;
            eprintln!("progress: {} item file(s) written", items.len());
            Ok(())
        }
        ["migrate", old, pr, name] => {
            let files = working(root, cfg)?;
            let (old_files, pr_files) = (at_rev(root, cfg, old)?, at_rev(root, cfg, pr)?);
            let mut written = 0;
            for doc in old_files.iter().filter(|f| cfg.logs.iter().any(|l| l.doc == f.path) || cfg.tables.iter().any(|t| t.doc == f.path)) {
                let Some(head) = pr_files.iter().find(|f| f.path == doc.path) else { continue };
                let (out, notes) = migrate(cfg, &files, doc, head, name);
                write_all(root, &out)?;
                written += out.len();
                for n in notes {
                    eprintln!("note: {n}");
                }
            }
            eprintln!("progress: {written} file(s) written; restore the docs themselves to main's copy");
            Ok(())
        }
        _ => Err("usage: progress [--write | --check [--base <rev>] | --cutover | migrate <old> <pr> <name>]".into()),
    }
}
