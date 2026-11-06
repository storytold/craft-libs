#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Join a Craft app's progress-log **fragments** and table **items** into its docs (RFC-0017,
//! RFC-0018), so pull requests add or edit their own small files instead of all editing the same
//! lines of `ROADMAP.md` and `docs/gaps.md`.
//!
//! - A **fragment** (`docs/progress.d/<issue>-<branch-slug>.md`) holds one log entry exactly as it
//!   reads in the log, after optional front matter (`kind: trivial | minor | major`). The fold puts
//!   it under the log's heading and deletes it.
//! - An **item** (`docs/gaps.d/<number>-<slug>.md`) is the lasting source of one table row: one
//!   front-matter field per column, the file's body for the body column, optional `rank:` and
//!   `status:`. The fold rebuilds the table from the items and never deletes them.
//! - A **joined region** (a log, a table's rows, the status line's `Last updated` and `Change`)
//!   is written only by the fold. [`check`] fails a pull request that edits one by hand.
//!
//! The core ([`join`], [`check`], [`items_from_doc`], [`migrate`]) is pure: callers hand it files
//! with their last commit time. [`cli`] is the thin shell an app's xtask calls.

pub mod cli;
mod front;

use std::collections::{BTreeMap, BTreeSet};

/// The longest fragment or item read, in bytes.
pub const MAX_FILE: usize = 256 * 1024;
/// The longest doc read, in bytes.
pub const MAX_DOC: usize = 16 * 1024 * 1024;
/// The most fragments or items read for one target.
pub const MAX_FILES: usize = 10_000;

/// An app's doc layout: its progress logs and its item tables.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub logs: &'static [Log],
    pub tables: &'static [Table],
}

/// A log fed by fragments.
#[derive(Debug, Clone, Copy)]
pub struct Log {
    /// Fragment folder, `docs/progress.d`.
    pub dir: &'static str,
    /// The doc, `ROADMAP.md`.
    pub doc: &'static str,
    /// The heading line the log is under, exactly, `## Progress log`.
    pub heading: &'static str,
    /// New entries go at the top (true) or the bottom.
    pub newest_first: bool,
    /// The fold also writes this doc's status line (`**Last updated:**`, `**Change:**`).
    pub status: bool,
}

/// A table fed by items.
#[derive(Debug, Clone, Copy)]
pub struct Table {
    /// Item folder, `docs/gaps.d` (or `docs/file-format-parity.d/stills` for one of several tables).
    pub dir: &'static str,
    /// The doc, `docs/gaps.md`.
    pub doc: &'static str,
    /// The heading line the table is under, exactly.
    pub heading: &'static str,
    /// Each column: its header as the doc has it, and the item field that fills it. The field
    /// `number` is the item's number (from its file name); `body` is the item's body.
    pub columns: &'static [(&'static str, &'static str)],
    /// Items with `status: closed` stay in the table (true) or are left out.
    pub show_closed: bool,
}

/// A file handed to the core: path from the repo root, text, and the Unix time of the last
/// commit that touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub text: String,
    pub time: i64,
}

/// What a [`join`] produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Joined {
    /// Every configured doc, joined, by path.
    pub docs: BTreeMap<String, String>,
    /// Fragment paths the join consumed (the fold deletes them).
    pub consumed: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Trivial,
    Minor,
    Major,
}

impl Kind {
    fn parse(s: Option<&str>) -> Result<Self, String> {
        match s {
            None | Some("minor") => Ok(Self::Minor),
            Some("trivial") => Ok(Self::Trivial),
            Some("major") => Ok(Self::Major),
            Some(other) => Err(format!("`kind: {other}` isn't trivial, minor or major")),
        }
    }
    fn word(self) -> &'static str {
        match self {
            Self::Trivial => "trivial",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

struct Fragment<'a> {
    path: &'a str,
    name: &'a str,
    time: i64,
    kind: Kind,
    lines: Vec<&'a str>,
}

struct Item<'a> {
    path: &'a str,
    number: u32,
    rank: Option<i64>,
    closed: bool,
    fields: front::Parsed<'a>,
}

/// The files directly in `dir` (a `README.md` there is not one), sorted by path.
fn in_dir<'a>(files: &'a [File], dir: &str) -> Vec<&'a File> {
    let mut found: Vec<&File> = files
        .iter()
        .filter(|f| f.path.strip_prefix(dir).and_then(|r| r.strip_prefix('/')).is_some_and(|name| !name.contains('/') && name != "README.md"))
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn guard(f: &File) -> Result<(), String> {
    if f.text.len() > MAX_FILE {
        return Err(format!("{}: longer than {MAX_FILE} bytes", f.path));
    }
    Ok(())
}

fn fragments<'a>(files: &'a [File], log: &Log, errors: &mut Vec<String>) -> Vec<Fragment<'a>> {
    let found = in_dir(files, log.dir);
    if found.len() > MAX_FILES {
        errors.push(format!("{}: more than {MAX_FILES} fragments", log.dir));
        return Vec::new();
    }
    let mut out = Vec::new();
    for f in found {
        let name = file_name(&f.path);
        let parsed = guard(f)
            .and_then(|()| {
                let stem = name
                    .strip_suffix(".md")
                    .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
                stem.ok_or_else(|| format!("{}: a fragment is `<issue>-<branch-slug>.md`, lower case, digits and `-`", f.path))
            })
            .and_then(|stem| front::parse(&f.text, &["kind"]).map(|p| (stem, p)).map_err(|e| format!("{}: {e}", f.path)))
            .and_then(|(stem, p)| {
                let kind = Kind::parse(p.get("kind")).map_err(|e| format!("{}: {e}", f.path))?;
                let lines: Vec<&str> = p.body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
                if lines.is_empty() {
                    return Err(format!("{}: is empty", f.path));
                }
                Ok(Fragment { path: &f.path, name: stem, time: f.time, kind, lines })
            });
        match parsed {
            Ok(fragment) => out.push(fragment),
            Err(e) => errors.push(e),
        }
    }
    out
}

/// A table row's cells, trimmed; `None` when the line isn't a row. `\|` stays inside a cell.
pub fn cells(row: &str) -> Option<Vec<String>> {
    let inner = row.trim().strip_prefix('|')?.strip_suffix('|')?;
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        if c == '|' && !escaped {
            out.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    out.push(cell.trim().to_string());
    Some(out)
}

fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    (hashes > 0 && line.as_bytes().get(hashes) == Some(&b' ')).then_some(hashes)
}

/// The lines under `heading`, up to the next heading of its level or above: (first line after the
/// heading, end).
fn section(lines: &[&str], heading: &str) -> Result<(usize, usize), String> {
    let at = lines.iter().position(|l| l.trim_end() == heading).ok_or_else(|| format!("has no `{heading}` heading"))?;
    let level = heading_level(heading).unwrap_or(usize::MAX);
    let end = lines.iter().enumerate().skip(at + 1).find(|(_, l)| heading_level(l).is_some_and(|n| n <= level)).map_or(lines.len(), |(i, _)| i);
    Ok((at + 1, end))
}

/// The table in a section: (header line, first row, end of rows).
fn table_in(lines: &[&str], start: usize, end: usize) -> Option<(usize, usize, usize)> {
    let header = (start..end).find(|&i| lines.get(i).is_some_and(|l| l.trim_start().starts_with('|')))?;
    let first = header + 2;
    let rows_end = (first..end).find(|&i| !lines.get(i).is_some_and(|l| l.trim_start().starts_with('|'))).unwrap_or(end);
    Some((header, first.min(rows_end.max(first)), rows_end.max(first)))
}

fn is_entry(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('|') || t.starts_with("- ") || t.starts_with("* ")
}

fn join_log<'a>(lines: &mut Vec<&'a str>, log: &Log, frags: &[Fragment<'a>]) -> Result<(), String> {
    let (start, end) = section(lines, log.heading)?;
    let first = (start..end).find(|&i| lines.get(i).is_some_and(|l| is_entry(l)));
    let is_table = first.and_then(|i| lines.get(i)).is_some_and(|l| l.trim_start().starts_with('|'));
    let mut order: Vec<&Fragment> = frags.iter().collect();
    order.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.name.cmp(b.name)));
    if log.newest_first {
        order.reverse();
    }
    let new: Vec<&str> = order.iter().flat_map(|f| f.lines.iter().copied()).collect();
    if is_table {
        let (header, first_row, rows_end) = table_in(lines, start, end).ok_or("log table vanished")?;
        let width = lines.get(header).and_then(|l| cells(l)).map_or(0, |c| c.len());
        for f in &order {
            if let Some(bad) = f.lines.iter().find(|l| cells(l).map(|c| c.len()) != Some(width)) {
                return Err(format!("{}: {bad:?} isn't a row of the log's {width} columns", f.path));
            }
        }
        let at = if log.newest_first { first_row } else { rows_end };
        lines.splice(at..at, new);
    } else {
        for f in &order {
            if f.lines.first().is_some_and(|l| !is_entry(l)) {
                return Err(format!("{}: the entry starts with {:?}, not a `- ` bullet", f.path, f.lines.first().copied().unwrap_or_default()));
            }
        }
        let at = match (log.newest_first, first) {
            (true, Some(i)) => i,
            _ => {
                let mut e = end;
                while e > start && lines.get(e - 1).is_some_and(|l| l.trim().is_empty()) {
                    e -= 1;
                }
                e
            }
        };
        lines.splice(at..at, new);
    }
    Ok(())
}

/// Replaces `**<field>:** <value>` (value up to ` · ` or the line's end) in `line`.
fn set_field(line: &str, field: &str, value: &str) -> Option<String> {
    let key = format!("**{field}:**");
    let at = line.find(&key)? + key.len();
    let rest = line.get(at..)?;
    let end = rest.find(" · ").unwrap_or(rest.len());
    Some(format!("{}{key} {value}{}", line.get(..at - key.len())?, rest.get(end..)?))
}

fn status_line(lines: &mut [String], frags: &[Fragment]) -> Result<(), String> {
    let (Some(time), Some(kind)) = (frags.iter().map(|f| f.time).max(), frags.iter().map(|f| f.kind).max()) else {
        return Ok(());
    };
    let date = chrono::DateTime::from_timestamp(time, 0).ok_or("a fragment's commit time is out of range")?.format("%Y-%m-%d").to_string();
    let line = lines.iter_mut().find(|l| l.contains("**Last updated:**")).ok_or("has no `**Last updated:**` status line")?;
    let dated = set_field(line, "Last updated", &date).ok_or("status line")?;
    *line = set_field(&dated, "Change", kind.word()).unwrap_or_else(|| {
        let at = dated.find("**Last updated:**").unwrap_or(0) + "**Last updated:** ".len() + date.len();
        format!("{} · **Change:** {}{}", dated.get(..at).unwrap_or(&dated), kind.word(), dated.get(at..).unwrap_or_default())
    });
    Ok(())
}

fn item_fields(table: &Table) -> Vec<&'static str> {
    let mut keys = vec!["rank", "status"];
    keys.extend(table.columns.iter().map(|(_, f)| *f).filter(|f| *f != "number" && *f != "body"));
    keys
}

fn items<'a>(files: &'a [File], table: &Table, errors: &mut Vec<String>) -> Vec<Item<'a>> {
    let found = in_dir(files, table.dir);
    if found.len() > MAX_FILES {
        errors.push(format!("{}: more than {MAX_FILES} items", table.dir));
        return Vec::new();
    }
    let allowed = item_fields(table);
    let has_body = table.columns.iter().any(|(_, f)| *f == "body");
    let mut out: Vec<Item> = Vec::new();
    for f in found {
        let name = file_name(&f.path);
        let parsed = guard(f).and_then(|()| {
            let stem = name.strip_suffix(".md").ok_or_else(|| format!("{}: an item is a `.md` file", f.path))?;
            let (num, slug) = stem.split_once('-').unwrap_or((stem, ""));
            let number: u32 = num.parse().map_err(|_| format!("{}: an item is `<number>-<slug>.md`", f.path))?;
            if slug.is_empty() || !slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Err(format!("{}: an item is `<number>-<slug>.md`, slug in lower case, digits and `-`", f.path));
            }
            let fields = front::parse(&f.text, &allowed).map_err(|e| format!("{}: {e}", f.path))?;
            if fields.fields.is_empty() && fields.body.is_empty() {
                return Err(format!("{}: is empty", f.path));
            }
            if let Some(missing) = allowed.iter().skip(2).find(|k| fields.get(k).is_none()) {
                return Err(format!("{}: has no `{missing}:` for its column", f.path));
            }
            if !has_body && !fields.body.is_empty() {
                return Err(format!("{}: has a body, but its table has no body column", f.path));
            }
            let rank =
                fields.get("rank").map(|r| r.parse::<i64>().map_err(|_| format!("{}: `rank: {r}` isn't a whole number", f.path))).transpose()?;
            let closed = match fields.get("status") {
                None | Some("open") => false,
                Some("closed") => true,
                Some(other) => return Err(format!("{}: `status: {other}` isn't open or closed", f.path)),
            };
            Ok(Item { path: &f.path, number, rank, closed, fields })
        });
        match parsed {
            Ok(item) => {
                if let Some(other) = out.iter().find(|i| i.number == item.number) {
                    errors.push(format!("{} and {} have the same number {}; take the next free one", other.path, item.path, item.number));
                } else {
                    out.push(item);
                }
            }
            Err(e) => errors.push(e),
        }
    }
    out.sort_by_key(|i| (i.rank.unwrap_or(i64::MAX), i.number));
    out
}

fn escape(cell: &str) -> String {
    cell.trim().replace('|', "\\|").replace("\r\n", "\n").split('\n').map(str::trim_end).collect::<Vec<_>>().join("<br>")
}

fn render(table: &Table, item: &Item) -> String {
    let cells: Vec<String> = table
        .columns
        .iter()
        .map(|(_, field)| match *field {
            "number" => item.number.to_string(),
            "body" => escape(item.fields.body),
            other => escape(item.fields.get(other).unwrap_or_default()),
        })
        .collect();
    // An empty cell is `| |`, as people write it.
    let cells: Vec<String> = cells.iter().map(|c| if c.is_empty() { " ".to_string() } else { format!(" {c} ") }).collect();
    format!("|{}|", cells.join("|"))
}

fn check_header(lines: &[&str], header: usize, table: &Table) -> Result<(), String> {
    let have = lines.get(header).and_then(|l| cells(l)).unwrap_or_default();
    let want: Vec<&str> = table.columns.iter().map(|(h, _)| *h).collect();
    if have != want {
        return Err(format!("`{}`: the table's columns are {:?}, the config says {:?}", table.heading, have, want));
    }
    Ok(())
}

/// Every configured doc with its fragments and items joined in. Errors list every bad file.
pub fn join(cfg: &Config, files: &[File]) -> Result<Joined, Vec<String>> {
    let mut errors = Vec::new();
    let by_path: BTreeMap<&str, &File> = files.iter().map(|f| (f.path.as_str(), f)).collect();
    let docs: BTreeSet<&str> = cfg.logs.iter().map(|l| l.doc).chain(cfg.tables.iter().map(|t| t.doc)).collect();
    let mut out = Joined::default();
    for doc in docs {
        let Some(file) = by_path.get(doc) else {
            errors.push(format!("{doc}: not found"));
            continue;
        };
        let mut lines: Vec<&str> = file.text.lines().collect();
        let mut owned_rows: Vec<(usize, usize, Vec<String>)> = Vec::new();
        let mut status_frags: Vec<Fragment> = Vec::new();
        for log in cfg.logs.iter().filter(|l| l.doc == doc) {
            let frags = fragments(files, log, &mut errors);
            if let Err(e) = join_log(&mut lines, log, &frags) {
                errors.push(format!("{doc} {e}"));
            }
            out.consumed.extend(frags.iter().map(|f| f.path.to_string()));
            if log.status {
                status_frags.extend(frags);
            }
        }
        for table in cfg.tables.iter().filter(|t| t.doc == doc) {
            let found = items(files, table, &mut errors);
            let place = section(&lines, table.heading)
                .and_then(|(s, e)| table_in(&lines, s, e).ok_or_else(|| format!("has no table under `{}`", table.heading)))
                .and_then(|(h, first, end)| check_header(&lines, h, table).map(|()| (first, end)));
            match place {
                Ok((first, end)) => {
                    owned_rows.push((first, end, found.iter().filter(|i| table.show_closed || !i.closed).map(|i| render(table, i)).collect()))
                }
                Err(e) => errors.push(format!("{doc} {e}")),
            }
        }
        // Tables are replaced bottom-up, so earlier line numbers stay right.
        owned_rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        let mut joined: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        for (first, end, rows) in owned_rows {
            if let Some(range) = joined.get(first..end).map(|_| first..end) {
                joined.splice(range, rows);
            }
        }
        if let Err(e) = status_line(&mut joined, &status_frags) {
            errors.push(format!("{doc} {e}"));
        }
        let mut text = joined.join("\n");
        if file.text.ends_with('\n') {
            text.push('\n');
        }
        out.docs.insert(doc.to_string(), text);
    }
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

/// The joined regions of `text` (the doc at `doc`), each with a name for errors.
fn regions(cfg: &Config, doc: &str, text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for log in cfg.logs.iter().filter(|l| l.doc == doc) {
        if let Ok((s, e)) = section(&lines, log.heading) {
            out.push((format!("`{}`", log.heading), lines.get(s..e).unwrap_or_default().join("\n")));
        }
        if log.status {
            let line = lines.iter().find(|l| l.contains("**Last updated:**")).copied().unwrap_or_default();
            for field in ["Last updated", "Change"] {
                let key = format!("**{field}:**");
                let value =
                    line.find(&key).and_then(|at| line.get(at + key.len()..)).map(|r| r.split(" · ").next().unwrap_or(r).trim()).unwrap_or_default();
                out.push((format!("the status line's {key}"), value.to_string()));
            }
        }
    }
    for table in cfg.tables.iter().filter(|t| t.doc == doc) {
        if let Some((_, first, end)) = section(&lines, table.heading).ok().and_then(|(s, e)| table_in(&lines, s, e)) {
            out.push((format!("the table under `{}`", table.heading), lines.get(first..end).unwrap_or_default().join("\n")));
        }
    }
    out
}

/// What CI runs on a pull request: `base` is the merge base's files, `head` the pull request's.
/// Fails on any bad fragment or item, and on a joined region the pull request changed unless it
/// now reads as the fold would write it (the fold's own pull request).
pub fn check(cfg: &Config, base: &[File], head: &[File]) -> Vec<String> {
    let mut errors = match join(cfg, head) {
        Ok(_) => Vec::new(),
        Err(e) => return e,
    };
    // Commit times are not used here: CI's checkout is shallow, so they aren't known.
    let head_paths: BTreeSet<&str> = head.iter().map(|f| f.path.as_str()).collect();
    let in_logs = |f: &File| cfg.logs.iter().any(|l| f.path.starts_with(&format!("{}/", l.dir)));
    // The fragments this pull request removed: only the fold does that.
    let consumed: Vec<&File> = base.iter().filter(|f| in_logs(f) && !head_paths.contains(f.path.as_str())).collect();
    let mut consumed_lines: Vec<String> = Vec::new();
    for f in &consumed {
        if let Ok(p) = front::parse(&f.text, &["kind"]) {
            consumed_lines.extend(p.body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).map(str::to_string));
        }
    }
    let entry_lines = |text: &str| -> Vec<String> {
        let mut v: Vec<String> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
        v.sort_unstable();
        v
    };
    let items_only: Vec<File> = head.iter().filter(|f| !in_logs(f)).cloned().collect();
    for doc in cfg.logs.iter().map(|l| l.doc).chain(cfg.tables.iter().map(|t| t.doc)).collect::<BTreeSet<_>>() {
        let text_of = |files: &[File]| files.iter().find(|f| f.path == doc).map(|f| f.text.clone());
        let (Some(base_text), Some(head_text)) = (text_of(base), text_of(head)) else { continue };
        // The tables as the fold would write them from this pull request's items.
        let mut tables_from: Vec<File> = items_only.iter().filter(|f| f.path != doc).cloned().collect();
        tables_from.push(File { path: doc.to_string(), text: base_text.clone(), time: 0 });
        let want = join(&Config { logs: &[], tables: cfg.tables }, &tables_from).ok().and_then(|j| j.docs.get(doc).cloned()).unwrap_or_default();
        let (b, w) = (regions(cfg, doc, &base_text), regions(cfg, doc, &want));
        for (name, value) in regions(cfg, doc, &head_text) {
            let in_region = |r: &[(String, String)]| r.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
            let before = in_region(&b).unwrap_or_default();
            let ok = value == before
                || if name.starts_with("the status line") {
                    !consumed.is_empty()
                } else if name.starts_with("the table") {
                    in_region(&w).is_some_and(|v| v == value)
                } else {
                    let mut expected = entry_lines(&before);
                    expected.extend(consumed_lines.iter().cloned());
                    expected.sort_unstable();
                    !consumed.is_empty() && entry_lines(&value) == expected
                };
            if !ok {
                errors.push(format!("{doc}: {name} was edited by hand. Only the fold writes it: add a fragment or edit the item file instead"));
            }
        }
    }
    errors
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "item".into() } else { out }
}

fn unescape(cell: &str) -> String {
    cell.replace("<br>", "\n").replace("\\|", "|")
}

/// One item file from a table row: `rank` keeps the row's place, `number` is the row's number or
/// `fallback` when it has none.
fn item_from_row(table: &Table, row: &[String], rank: Option<i64>, fallback: u32) -> (u32, String, String) {
    let mut number = None;
    let mut fields = Vec::new();
    let mut body = String::new();
    let mut title = String::new();
    for ((_, field), cell) in table.columns.iter().zip(row) {
        match *field {
            "number" => number = cell.chars().filter(char::is_ascii_digit).collect::<String>().parse::<u32>().ok(),
            "body" => body = unescape(cell),
            other => {
                if title.is_empty() && !cell.is_empty() {
                    title = cell.clone();
                }
                fields.push(format!("{other}: {}", cell.replace("\\|", "|")));
            }
        }
    }
    let number = number.unwrap_or(fallback);
    let mut text = String::from("---\n");
    if let Some(rank) = rank {
        text.push_str(&format!("rank: {rank}\n"));
    }
    if title.trim_start().starts_with("~~") {
        text.push_str("status: closed\n");
    }
    for f in fields {
        text.push_str(&f);
        text.push('\n');
    }
    text.push_str("---\n");
    if !body.is_empty() {
        text.push('\n');
        text.push_str(&body);
        text.push('\n');
    }
    (number, format!("{}/{number}-{}.md", table.dir, slug(&title.replace('~', ""))), text)
}

/// The cutover: an item file for every row of every configured table in `docs`, keeping each row's
/// place as its `rank`. A repeated number, or a row with none, takes the next free number; each
/// such change is in the returned notes.
pub fn items_from_doc(cfg: &Config, docs: &[File]) -> Result<(Vec<File>, Vec<String>), Vec<String>> {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    for table in cfg.tables {
        let Some(doc) = docs.iter().find(|d| d.path == table.doc) else {
            errors.push(format!("{}: not found", table.doc));
            continue;
        };
        let lines: Vec<&str> = doc.text.lines().collect();
        let place = section(&lines, table.heading)
            .and_then(|(s, e)| table_in(&lines, s, e).ok_or_else(|| format!("has no table under `{}`", table.heading)))
            .and_then(|(h, first, end)| check_header(&lines, h, table).map(|()| (first, end)));
        let (first, end) = match place {
            Ok(p) => p,
            Err(e) => {
                errors.push(format!("{} {e}", table.doc));
                continue;
            }
        };
        let rows: Vec<Vec<String>> = lines.get(first..end).unwrap_or_default().iter().filter_map(|l| cells(l)).collect();
        let mut used: BTreeSet<u32> = BTreeSet::new();
        let taken: BTreeSet<u32> = rows.iter().filter_map(|r| item_from_row(table, r, None, 0).0.checked_sub(0).filter(|n| *n > 0)).collect();
        let mut next = taken.iter().max().copied().unwrap_or(0) + 1;
        for (i, row) in rows.iter().enumerate() {
            if row.len() != table.columns.len() {
                errors.push(format!("{} `{}` row {}: {} cells, {} columns", table.doc, table.heading, i + 1, row.len(), table.columns.len()));
                continue;
            }
            let rank = i64::try_from(i + 1).ok();
            let (mut number, mut path, mut text) = item_from_row(table, row, rank, 0);
            if number == 0 || used.contains(&number) {
                let numbered = table.columns.iter().any(|(_, f)| *f == "number");
                if numbered {
                    notes.push(format!(
                        "{} `{}` row {}: number {} is {}; it becomes {next}",
                        table.doc,
                        table.heading,
                        i + 1,
                        number,
                        if number == 0 { "missing" } else { "repeated" }
                    ));
                }
                (number, path, text) = item_from_row(table, row, rank, next);
                // The row's own number cell is replaced by the new number.
                path = path.replacen(&format!("/{}-", number), &format!("/{next}-"), 1);
                number = next;
                next += 1;
            }
            used.insert(number);
            out.push(File { path, text, time: doc.time });
        }
    }
    if errors.is_empty() { Ok((out, notes)) } else { Err(errors) }
}

/// Rewrites a pull request opened before the cutover: `old` is the doc at its merge base, `pr` the
/// doc at its head, `files` the current `main` (docs, fragments and items). Returns the files to
/// write on the pull request's branch, and notes on what couldn't be moved (never silently dropped).
pub fn migrate(cfg: &Config, files: &[File], old: &File, pr: &File, name: &str) -> (Vec<File>, Vec<String>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let (old_lines, pr_lines): (Vec<&str>, Vec<&str>) = (old.text.lines().collect(), pr.text.lines().collect());
    for log in cfg.logs.iter().filter(|l| l.doc == old.path) {
        let (Ok((os, oe)), Ok((ps, pe))) = (section(&old_lines, log.heading), section(&pr_lines, log.heading)) else {
            continue;
        };
        let before: BTreeSet<&str> = old_lines.get(os..oe).unwrap_or_default().iter().copied().collect();
        let added: Vec<&str> =
            pr_lines.get(ps..pe).unwrap_or_default().iter().copied().filter(|l| !l.trim().is_empty() && !before.contains(l)).collect();
        if !added.is_empty() {
            out.push(File { path: format!("{}/{name}.md", log.dir), text: format!("{}\n", added.join("\n")), time: 0 });
        }
    }
    for table in cfg.tables.iter().filter(|t| t.doc == old.path) {
        let rows_of = |lines: &[&str]| -> Vec<Vec<String>> {
            section(lines, table.heading)
                .ok()
                .and_then(|(s, e)| table_in(lines, s, e))
                .map(|(_, first, end)| lines.get(first..end).unwrap_or_default().iter().filter_map(|l| cells(l)).collect())
                .unwrap_or_default()
        };
        let (before, after) = (rows_of(&old_lines), rows_of(&pr_lines));
        let mut errs = Vec::new();
        let current = items(files, table, &mut errs);
        let mut next = current.iter().map(|i| i.number).max().unwrap_or(0) + 1;
        for row in after.iter().filter(|r| !before.contains(r)) {
            if row.len() != table.columns.len() {
                notes.push(format!(
                    "{}: a changed row has {} cells, the table {} columns; move it by hand: {row:?}",
                    table.doc,
                    row.len(),
                    table.columns.len()
                ));
                continue;
            }
            let (number, _, _) = item_from_row(table, row, None, 0);
            let was = before.iter().any(|b| item_from_row(table, b, None, 0).0 == number && number > 0);
            match current.iter().find(|i| i.number == number).filter(|_| was) {
                Some(item) => {
                    let (_, _, text) = item_from_row(table, row, item.rank, number);
                    let keep_status =
                        if item.closed && !text.contains("status: closed") { text.replacen("---\n", "---\nstatus: closed\n", 1) } else { text };
                    out.push(File { path: item.path.to_string(), text: keep_status, time: 0 });
                }
                None => {
                    let (_, path, text) = item_from_row(table, row, None, next);
                    let fixed =
                        path.rsplit_once('/').and_then(|(dir, f)| f.split_once('-').map(|(_, s)| format!("{dir}/{next}-{s}"))).unwrap_or(path);
                    if number != next {
                        notes.push(format!("{}: new row numbered {number} becomes {next}", table.doc));
                    }
                    out.push(File { path: fixed, text, time: 0 });
                    next += 1;
                }
            }
        }
        for row in before.iter().filter(|r| !after.contains(r)) {
            let (number, _, _) = item_from_row(table, row, None, 0);
            if !after.iter().any(|a| item_from_row(table, a, None, 0).0 == number) {
                notes.push(format!(
                    "{}: the pull request removed row {number}; set `status: closed` on its item by hand if that was meant",
                    table.doc
                ));
            }
        }
    }
    (out, notes)
}

#[cfg(test)]
mod tests;
