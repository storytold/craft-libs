//! `cargo xtask progress [--write | --check [--base <rev>] | --cutover | migrate <old> <pr> <name>]`:
//! the thin shell an app's xtask calls with its [`Config`]. It reads files and asks `git` for
//! commit times and the merge base; everything else is the pure core.
//!
//! - no flag: print each doc as it reads with pending fragments and items joined in.
//! - `--write`: the fold. Write the docs, delete the consumed fragments.
//! - `--check`: CI. Bad fragments or items, or a joined region edited by hand, fail. The base is
//!   the merge base with `--base <rev>`; in GitHub Actions, `HEAD^1`; else the merge base with
//!   `origin/main`.
//! - `--cutover`: once per repo (and again for a table added to the config). Turn every row of each
//!   table whose folder has no items yet into an item file, then fold. Started tables are skipped.
//! - `migrate <old> <pr> <name>`: rewrite a pull request opened before the cutover. `<old>` is its
//!   merge base, `<pr>` its head; the new fragment is `<name>.md`. Run on the PR's branch, merged
//!   with the current `main`.

use crate::{Config, File, MAX_DOC, MAX_FILE, MAX_FILES, NaiveDate, Table, check_report, in_dir, items_from_tables, join, migrate};
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

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Today in UTC, from the system clock: no status date goes past it.
fn today() -> NaiveDate {
    chrono::DateTime::from_timestamp(now(), 0).map_or(NaiveDate::MIN, |t| t.date_naive())
}

fn paths(cfg: &Config) -> Vec<&'static str> {
    let mut p: Vec<&str> = cfg.logs.iter().flat_map(|l| [l.doc, l.dir]).chain(cfg.tables.iter().flat_map(|t| [t.doc, t.dir])).collect();
    p.sort_unstable();
    p.dedup();
    p
}

/// The last commit time of each path under `paths`, at `rev`; with `added`, the time of the commit
/// that added it (a fragment edited later keeps its place in the log).
fn times(root: &Path, rev: &str, paths: &[&str], added: bool) -> Result<BTreeMap<String, i64>, String> {
    let mut args = vec!["log", "--format=%x01%ct", "--name-only"];
    if added {
        args.push("--diff-filter=A");
    }
    // A rev that starts with `-` is still a rev, never an option.
    args.extend(["--end-of-options", rev, "--"]);
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

/// Commit times for the files at `rev`: fragments by the commit that added them, the rest by the
/// last commit that touched them.
fn known_times(root: &Path, cfg: &Config, rev: &str) -> Result<BTreeMap<String, i64>, String> {
    let mut known = times(root, rev, &paths(cfg), false)?;
    let logs: Vec<&str> = cfg.logs.iter().map(|l| l.dir).collect();
    if !logs.is_empty() {
        known.extend(times(root, rev, &logs, true)?);
    }
    Ok(known)
}

/// Not following a symlink: one could point anywhere on the machine.
fn kind(path: &Path) -> Option<std::fs::FileType> {
    std::fs::symlink_metadata(path).ok().map(|m| m.file_type())
}

/// The configured docs and every file in the configured folders, from the working tree. Symlinks
/// are skipped.
fn working(root: &Path, cfg: &Config) -> Result<Vec<File>, String> {
    let known = known_times(root, cfg, "HEAD").unwrap_or_default();
    let now = now();
    let mut names = Vec::new();
    for p in paths(cfg) {
        let full = root.join(p);
        let t = kind(&full);
        if t.is_some_and(|t| t.is_file()) {
            names.push(p.to_string());
        } else if t.is_some_and(|t| t.is_dir()) {
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

/// The same files at `rev`, from git. Symlinks are skipped.
fn at_rev(root: &Path, cfg: &Config, rev: &str) -> Result<Vec<File>, String> {
    let ps = paths(cfg);
    let known = known_times(root, cfg, rev)?;
    let mut args = vec!["ls-tree", "-r", "--end-of-options", rev, "--"];
    args.extend(&ps);
    let mut out = Vec::new();
    let listing = git(root, &args)?;
    // `<mode> <type> <object>\t<path>`; a symlink's mode is 120000.
    let entries = listing.lines().filter_map(|l| l.split_once('\t')).filter(|(meta, _)| !meta.starts_with("120000") && meta.contains(" blob "));
    for (_, name) in entries {
        // Only the configured docs and the files directly in a configured folder.
        if !ps.iter().any(|p| name == *p || name.strip_prefix(p).and_then(|r| r.strip_prefix('/')).is_some_and(|r| !r.contains('/'))) {
            continue;
        }
        if out.len() > MAX_FILES * 4 {
            return Err(format!("{rev}: too many files"));
        }
        let text = git(root, &["show", "--end-of-options", &format!("{rev}:{name}")])?;
        out.push(File { path: name.to_string(), time: known.get(name).copied().unwrap_or(0), text });
    }
    Ok(out)
}

/// Writes `text` to `path` whole or not at all: into a file beside it, then renamed over it, so a
/// failed write never leaves a doc cut short. A read-only file is refused, as a plain write would.
fn write_file(root: &Path, path: &str, text: &str) -> Result<(), String> {
    let full = root.join(path);
    let err = |e: std::io::Error| format!("{path}: {e}");
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    if std::fs::metadata(&full).is_ok_and(|m| m.permissions().readonly()) {
        return Err(format!("{path}: is read-only"));
    }
    let mut tmp = full.clone().into_os_string();
    tmp.push(".docjoin-tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let done = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &full));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done.map_err(err)
}

fn write_all(root: &Path, files: &[File]) -> Result<(), String> {
    for f in files {
        write_file(root, &f.path, &f.text)?;
    }
    Ok(())
}

/// Writes files that weren't there before, all or none: if one fails, those already written are
/// removed again.
fn write_new(root: &Path, files: &[File]) -> Result<(), String> {
    for (i, f) in files.iter().enumerate() {
        if let Err(e) = write_file(root, &f.path, &f.text) {
            for done in files.iter().take(i) {
                let _ = std::fs::remove_file(root.join(&done.path));
            }
            return Err(e);
        }
    }
    Ok(())
}

fn warn(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

fn fold(root: &Path, cfg: &Config, files: &[File]) -> Result<usize, String> {
    let joined = join(cfg, files, today()).map_err(|e| e.join("\n"))?;
    warn(&joined.warnings);
    // The docs that fold a fragment: those of the logs whose folder holds it.
    let docs_of = |path: &str| -> Vec<&str> {
        cfg.logs
            .iter()
            .filter(|l| path.strip_prefix(l.dir).and_then(|r| r.strip_prefix('/')).is_some_and(|r| !r.contains('/')))
            .map(|l| l.doc)
            .collect()
    };
    // A fragment is deleted once every doc it went into is written. If a doc write fails, the
    // docs written before it have lost their fragments and the rest keep theirs: a rerun folds
    // each once, and a fragment git doesn't have yet is never lost.
    let mut written: Vec<&str> = Vec::new();
    let mut deleted: Vec<&str> = Vec::new();
    for (doc, text) in &joined.docs {
        if files.iter().find(|f| &f.path == doc).is_none_or(|f| &f.text != text) {
            write_file(root, doc, text)?;
        }
        written.push(doc);
        for path in &joined.consumed {
            if !deleted.contains(&path.as_str()) && docs_of(path).iter().all(|d| written.contains(d)) {
                std::fs::remove_file(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
                deleted.push(path);
            }
        }
    }
    Ok(joined.consumed.len())
}

/// Runs the command `args` (without the program name) in the repo at `root`.
pub fn run(root: &Path, cfg: &Config, args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => {
            let files = working(root, cfg)?;
            let joined = join(cfg, &files, today()).map_err(|e| e.join("\n"))?;
            warn(&joined.warnings);
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
                ["--base", rev] => git(root, &["merge-base", "--end-of-options", "HEAD", rev])?.trim().to_string(),
                // CI checks out the commit being tested alone; its first parent is the base it
                // merges into (a pull request's merge commit, or the previous `main`).
                [] if std::env::var_os("GITHUB_ACTIONS").is_some() => {
                    // `actions/checkout` fetches the merge commit alone, by its sha, with no
                    // branch; `--deepen` would deepen nothing. Fetch that commit two deep.
                    if git(root, &["rev-parse", "--verify", "-q", "HEAD^1"]).is_err() {
                        let head = git(root, &["rev-parse", "--verify", "HEAD"])?;
                        git(root, &["fetch", "-q", "--no-tags", "--depth=2", "origin", head.trim()])?;
                    }
                    "HEAD^1".to_string()
                }
                [] => git(root, &["merge-base", "HEAD", "origin/main"])?.trim().to_string(),
                _ => return Err("usage: progress --check [--base <rev>]".into()),
            };
            let report = check_report(cfg, &at_rev(root, cfg, &base)?, &working(root, cfg)?, today());
            warn(&report.warnings);
            let errors = report.errors;
            if errors.is_empty() {
                eprintln!("progress: fragments, items and joined regions are fine");
                Ok(())
            } else {
                Err(errors.join("\n"))
            }
        }
        ["--cutover"] => {
            let files = working(root, cfg)?;
            // A table that already has items is the fold's: its rows are not read again.
            let (fresh, started): (Vec<Table>, Vec<Table>) = cfg.tables.iter().partition(|t| in_dir(&files, t.dir).is_empty());
            for t in &started {
                eprintln!("progress: skipped `{}` in {}: {} already has items", t.heading, t.doc, t.dir);
            }
            let (items, notes) = items_from_tables(&fresh, &files).map_err(|e| e.join("\n"))?;
            // All of a table's items or none: one left behind would make the table look started.
            write_new(root, &items)?;
            for n in &notes {
                eprintln!("note: {n}");
            }
            fold(root, cfg, &working(root, cfg)?)?;
            eprintln!("progress: {} item file(s) written", items.len());
            Ok(())
        }
        ["migrate", old, pr, name] => {
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Err(format!("migrate: the fragment name {name:?} must be [a-z0-9-]+, like `123-my-branch`"));
            }
            let files = working(root, cfg)?;
            let (old_files, pr_files) = (at_rev(root, cfg, old)?, at_rev(root, cfg, pr)?);
            let mut out: Vec<File> = Vec::new();
            for doc in old_files.iter().filter(|f| cfg.logs.iter().any(|l| l.doc == f.path) || cfg.tables.iter().any(|t| t.doc == f.path)) {
                let Some(head) = pr_files.iter().find(|f| f.path == doc.path) else { continue };
                let (files_out, notes) = migrate(cfg, &files, doc, head, name);
                out.extend(files_out);
                for n in notes {
                    eprintln!("note: {n}");
                }
            }
            // A new fragment never replaces a different one of its name (another pull request's,
            // or this one's edited since the last run). Item files are meant to be updated.
            for f in out.iter().filter(|f| cfg.logs.iter().any(|l| f.path.strip_prefix(l.dir).is_some_and(|r| r.starts_with('/')))) {
                let full = root.join(&f.path);
                let clash = kind(&full).is_some() && std::fs::read_to_string(&full).ok().as_deref() != Some(f.text.as_str())
                    || out.iter().any(|g| g.path == f.path && g.text != f.text);
                if clash {
                    return Err(format!("migrate: {} already holds a different fragment; pick another name", f.path));
                }
            }
            write_all(root, &out)?;
            eprintln!("progress: {} file(s) written; restore the docs themselves to main's copy", out.len());
            Ok(())
        }
        _ => Err("usage: progress [--write | --check [--base <rev>] | --cutover | migrate <old> <pr> <name>]".into()),
    }
}
