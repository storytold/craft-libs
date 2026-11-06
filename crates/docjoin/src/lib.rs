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

pub use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet};

/// The longest fragment or item read, in bytes.
pub const MAX_FILE: usize = 256 * 1024;
/// The longest doc read, in bytes.
pub const MAX_DOC: usize = 16 * 1024 * 1024;
/// The most fragments or items read for one target.
pub const MAX_FILES: usize = 10_000;
/// The highest item number. Numbers stay small, so deriving the next one can't overflow.
pub const MAX_NUMBER: u32 = 1_000_000;

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
    /// The heading line the log is under, `## Progress log`. A doc line matches it exactly, or
    /// followed by ` (…)`; it appears once in the doc.
    pub heading: &'static str,
    /// New entries go at the top (true) or the bottom.
    pub newest_first: bool,
    /// The fold also writes this doc's status line (`**Last updated:**`, `**Change:**`).
    pub status: bool,
}

impl Log {
    /// Newest first, no status line. Fill the rest: `Log { dir, doc, heading, ..Log::DEFAULT }`.
    pub const DEFAULT: Self = Self { dir: "", doc: "", heading: "", newest_first: true, status: false };
}

/// A table fed by items.
#[derive(Debug, Clone, Copy)]
pub struct Table {
    /// Item folder, `docs/gaps.d` (or `docs/file-format-parity.d/stills` for one of several tables).
    pub dir: &'static str,
    /// The doc, `docs/gaps.md`.
    pub doc: &'static str,
    /// The heading line the table is under, matched as [`Log::heading`] is.
    pub heading: &'static str,
    /// Each column: its header as the doc has it, and the item field that fills it. The field
    /// `number` is the item's number (from its file name); `body` is the item's body.
    pub columns: &'static [(&'static str, &'static str)],
    /// Items with `status: closed` stay in the table (true) or are left out.
    pub show_closed: bool,
    /// The fold also writes this doc's status line when the table's items change.
    pub status: bool,
    /// Which table under the heading, from 0, when the section has more than one.
    pub nth: usize,
}

impl Table {
    /// Closed items shown, no status line, the first table under the heading. Fill the rest:
    /// `Table { dir, doc, heading, columns, ..Table::DEFAULT }`.
    pub const DEFAULT: Self = Self { dir: "", doc: "", heading: "", columns: &[], show_closed: true, status: false, nth: 0 };
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
    /// What is wrong but doesn't stop the join: items that share a number.
    pub warnings: Vec<String>,
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
    rank: Option<i32>,
    closed: bool,
    time: i64,
    fields: front::Parsed<'a>,
}

/// The key of the status line. A doc whose status the fold writes has it on exactly one line.
const STATUS_KEY: &str = "**Last updated:**";

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

/// A line that would start a heading (and so end the log's section), up to three spaces in.
fn is_heading(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    line.len() - t.len() <= 3 && t.starts_with('#') && (heading_level(t).is_some() || t.trim_end().bytes().all(|b| b == b'#'))
}

fn fragment<'a>(f: &'a File) -> Result<Fragment<'a>, String> {
    guard(f)?;
    let name = file_name(&f.path);
    let stem = name
        .strip_suffix(".md")
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        .ok_or_else(|| format!("{}: a fragment is `<issue>-<branch-slug>.md`, lower case, digits and `-`", f.path))?;
    let p = front::parse(&f.text, &["kind"]).map_err(|e| format!("{}: {e}", f.path))?;
    let kind = Kind::parse(p.get("kind")).map_err(|e| format!("{}: {e}", f.path))?;
    let lines: Vec<&str> = p.body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Err(format!("{}: is empty", f.path));
    }
    // A heading would end the log's section; a status line would be a second one.
    if let Some(bad) = lines.iter().find(|l| is_heading(l) || l.contains(STATUS_KEY)) {
        return Err(format!("{}: {bad:?} is a heading or a status line; a fragment is only its log entry", f.path));
    }
    // A code fence or an HTML comment would swallow the rest of the doc as it renders.
    if let Some(bad) = lines.iter().find(|l| {
        let t = l.trim_start();
        t.starts_with("```") || t.starts_with("~~~") || l.contains("<!--")
    }) {
        return Err(format!("{}: {bad:?} opens a code fence or an HTML comment; a fragment is only its log entry", f.path));
    }
    Ok(Fragment { path: &f.path, name: stem, time: f.time, kind, lines })
}

fn fragments<'a>(files: &'a [File], log: &Log, errors: &mut Vec<String>) -> Vec<Fragment<'a>> {
    let found = in_dir(files, log.dir);
    if found.len() > MAX_FILES {
        errors.push(format!("{}: more than {MAX_FILES} fragments", log.dir));
        return Vec::new();
    }
    let mut out = Vec::new();
    for f in found {
        match fragment(f) {
            Ok(fragment) => out.push(fragment),
            Err(e) => errors.push(e),
        }
    }
    out
}

/// A table row's cells, trimmed; `None` when the line isn't a row. A `|` after an odd number of
/// backslashes stays inside its cell (`\|`); `\\|` is a backslash, then the separator. The trailing
/// `|` is optional, as in GFM.
pub fn cells(row: &str) -> Option<Vec<String>> {
    let inner = row.trim().strip_prefix('|')?;
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    let mut closed = false;
    for c in inner.chars() {
        closed = c == '|' && !escaped;
        if closed {
            out.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    if !closed {
        out.push(cell.trim().to_string());
    }
    Some(out)
}

fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    (hashes > 0 && matches!(line.as_bytes().get(hashes), Some(b' ' | b'\t'))).then_some(hashes)
}

/// A doc line is the configured heading when it equals it, or equals it followed by ` (…)`
/// (`## Point entry (≈ 45%)` for `## Point entry`).
fn heading_matches(line: &str, heading: &str) -> bool {
    let l = line.trim_end();
    l == heading || l.strip_prefix(heading).is_some_and(|r| r.starts_with(" (") && r.ends_with(')'))
}

/// The lines under `heading`, up to the next heading of its level or above: (first line after the
/// heading, end). The heading must appear once: with two, the fold and the check could each guard
/// a different copy.
fn section(lines: &[&str], heading: &str) -> Result<(usize, usize), String> {
    let found: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| heading_matches(l, heading)).map(|(i, _)| i).collect();
    let at = match found.as_slice() {
        [] => return Err(format!("has no `{heading}` heading")),
        [at] => *at,
        [a, b, ..] => {
            return Err(format!("has `{heading}` {} times (lines {} and {}); a configured heading appears once", found.len(), a + 1, b + 1));
        }
    };
    let level = heading_level(heading).unwrap_or(usize::MAX);
    let end = lines.iter().enumerate().skip(at + 1).find(|(_, l)| heading_level(l).is_some_and(|n| n <= level)).map_or(lines.len(), |(i, _)| i);
    Ok((at + 1, end))
}

fn is_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// A list item's marker: `-`, `*` or `+`, or up to nine digits and `.` or `)`, then a space or the
/// line's end.
fn is_list_item(t: &str) -> bool {
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    let rest = match digits {
        0 => t.strip_prefix(['-', '*', '+']),
        1..=9 => t.get(digits..).and_then(|r| r.strip_prefix(['.', ')'])),
        _ => None,
    };
    rest.is_some_and(|r| r.is_empty() || r.starts_with([' ', '\t']))
}

/// A line that starts a block other than a paragraph, and so ends a table (GFM): a heading, a code
/// fence, a block quote, an HTML block or a list item, up to three spaces in.
fn starts_block(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    line.len() - t.len() <= 3 && (is_heading(line) || t.starts_with("```") || t.starts_with("~~~") || t.starts_with(['>', '<']) || is_list_item(t))
}

/// A table line's cells. GFM reads a line with no leading `|` inside a table as a row too.
fn row_cells(line: &str) -> Vec<String> {
    if is_row(line) { cells(line).unwrap_or_default() } else { cells(&format!("|{line}")).unwrap_or_default() }
}

/// The tables in a section, each (header line, first row, end of rows). A table starts at a line
/// that starts with `|` and runs, as in GFM, to the first blank line or line that starts another
/// block: every line before that renders as a row, with or without a leading `|`.
fn tables_in(lines: &[&str], start: usize, end: usize) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut i = start;
    while i < end {
        if !lines.get(i).is_some_and(|l| is_row(l)) {
            i += 1;
            continue;
        }
        let header = i;
        i += 1;
        while i < end && lines.get(i).is_some_and(|l| !l.trim().is_empty() && !starts_block(l)) {
            i += 1;
        }
        out.push((header, (header + 2).min(i), i));
    }
    out
}

/// Table `nth` (from 0) in a section: (header line, first row, end of rows).
fn table_in(lines: &[&str], start: usize, end: usize, nth: usize) -> Option<(usize, usize, usize)> {
    tables_in(lines, start, end).get(nth).copied()
}

/// Every line in a joined table's rows is a row of the header's width: GFM renders a line that
/// follows a table as one of its rows, so a stray line would be a row nobody joined.
fn check_rows(lines: &[&str], (header, first, end): (usize, usize, usize), name: &str) -> Result<(), String> {
    let width = lines.get(header).map_or(0, |l| row_cells(l).len());
    for (n, line) in lines.get(first..end).unwrap_or_default().iter().enumerate() {
        let got = row_cells(line).len();
        if got != width {
            return Err(format!("{name} row {}: {got} cells, {width} columns: {:?}; a table runs to the first blank line", n + 1, line.trim_end()));
        }
    }
    Ok(())
}

fn is_entry(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('|') || t.starts_with("- ") || t.starts_with("* ")
}

/// Where the fold puts a log's new entries.
struct Insert {
    at: usize,
    /// The log is a table of this many columns.
    width: Option<usize>,
    /// A blank line goes before / after the new entries (a log with no entries yet).
    pad_before: bool,
    pad_after: bool,
}

fn blank_at(lines: &[&str], i: usize) -> bool {
    lines.get(i).is_some_and(|l| l.trim().is_empty())
}

fn log_insert(lines: &[&str], log: &Log) -> Result<Insert, String> {
    let (start, end) = section(lines, log.heading)?;
    let first = (start..end).find(|&i| lines.get(i).is_some_and(|l| is_entry(l)));
    if first.and_then(|i| lines.get(i)).is_some_and(|l| is_row(l)) {
        let table = table_in(lines, start, end, 0).ok_or("log table vanished")?;
        check_rows(lines, table, &format!("`{}`", log.heading))?;
        let (header, first_row, rows_end) = table;
        let width = lines.get(header).and_then(|l| cells(l)).map_or(0, |c| c.len());
        let at = if log.newest_first { first_row } else { rows_end };
        return Ok(Insert { at, width: Some(width), pad_before: false, pad_after: false });
    }
    let at = match (log.newest_first, first) {
        (true, Some(i)) => i,
        _ => {
            let mut e = end;
            while e > start && blank_at(lines, e - 1) {
                e -= 1;
            }
            e
        }
    };
    let pad_before = first.is_none() && at.checked_sub(1).is_some_and(|p| !blank_at(lines, p));
    let pad_after = lines.get(at).is_some_and(|l| !l.trim().is_empty() && !is_entry(l));
    Ok(Insert { at, width: None, pad_before, pad_after })
}

fn join_log<'a>(lines: &mut Vec<&'a str>, log: &Log, frags: &[Fragment<'a>]) -> Result<(), String> {
    let insert = log_insert(lines, log)?;
    if frags.is_empty() {
        return Ok(());
    }
    let mut order: Vec<&Fragment> = frags.iter().collect();
    order.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.name.cmp(b.name)));
    if log.newest_first {
        order.reverse();
    }
    for f in &order {
        match insert.width {
            Some(width) => {
                if let Some(bad) = f.lines.iter().find(|l| cells(l).map(|c| c.len()) != Some(width)) {
                    return Err(format!("{}: {bad:?} isn't a row of the log's {width} columns", f.path));
                }
            }
            None => {
                if f.lines.first().is_some_and(|l| !is_entry(l)) {
                    return Err(format!("{}: the entry starts with {:?}, not a `- ` bullet", f.path, f.lines.first().copied().unwrap_or_default()));
                }
            }
        }
    }
    let mut new: Vec<&str> = Vec::new();
    if insert.pad_before {
        new.push("");
    }
    new.extend(order.iter().flat_map(|f| f.lines.iter().copied()));
    if insert.pad_after {
        new.push("");
    }
    let at = insert.at.min(lines.len());
    lines.splice(at..at, new);
    Ok(())
}

/// A doc's status line: `**Last updated:** YYYY-MM-DD · **Change:** …`, fields separated by ` · `.
struct Status {
    at: usize,
    parts: Vec<String>,
    /// The part holding `**Last updated:**`.
    date: usize,
    /// The parts holding `**Change:**` and its value (which may itself hold ` · `).
    change: Option<(usize, usize)>,
}

fn lead(part: &str) -> &str {
    part.trim_start_matches(['>', ' '])
}

fn is_field(part: &str) -> bool {
    let t = lead(part);
    t.starts_with("**") && t.contains(":**")
}

impl Status {
    fn find<S: AsRef<str>>(lines: &[S]) -> Result<Self, String> {
        let found: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.as_ref().contains(STATUS_KEY)).map(|(i, _)| i).collect();
        let at = match found.as_slice() {
            [] => return Err(format!("has no `{STATUS_KEY}` status line")),
            [at] => *at,
            [a, b, ..] => return Err(format!("has `{STATUS_KEY}` on lines {} and {}; keep one status line", a + 1, b + 1)),
        };
        let line = lines.get(at).map(AsRef::as_ref).unwrap_or_default();
        let shape = || format!("line {}: the status line isn't `{STATUS_KEY} YYYY-MM-DD · **Change:** …` with ` · ` between its fields", at + 1);
        let parts: Vec<String> = line.split(" · ").map(str::to_string).collect();
        let date = parts.iter().position(|p| lead(p).starts_with(STATUS_KEY)).ok_or_else(shape)?;
        let value = parts.get(date).and_then(|p| lead(p).strip_prefix(STATUS_KEY)).unwrap_or_default().trim();
        if line.matches(STATUS_KEY).count() != 1 || parse_date(value).is_none() {
            return Err(shape());
        }
        // With two, the fold would write one and the check read the other.
        if parts.iter().filter(|p| lead(p).starts_with("**Change:**")).count() > 1 {
            return Err(format!("line {}: the status line has two `**Change:**` fields; keep one", at + 1));
        }
        let change = parts.iter().position(|p| lead(p).starts_with("**Change:**")).map(|c| {
            let end = parts.iter().enumerate().skip(c + 1).find(|(_, p)| is_field(p)).map_or(parts.len(), |(i, _)| i);
            (c, end)
        });
        Ok(Self { at, parts, date, change })
    }

    fn date(&self) -> &str {
        self.parts.get(self.date).and_then(|p| lead(p).strip_prefix(STATUS_KEY)).unwrap_or_default().trim()
    }

    fn change(&self) -> String {
        let Some((c, e)) = self.change else { return String::new() };
        let value = self.parts.get(c..e).unwrap_or_default().join(" · ");
        lead(&value).strip_prefix("**Change:**").unwrap_or_default().trim().to_string()
    }

    /// The line with `date` (unless that is older than the line's) and `kind` written in. The date
    /// is never after `today`: a commit time can be in the future, and a PR could have put one there.
    fn written(&self, date: NaiveDate, kind: Kind, today: NaiveDate) -> String {
        let keep = |i: usize, key: &str| self.parts.get(i).and_then(|p| p.find(key).and_then(|at| p.get(..at))).unwrap_or_default().to_string();
        let date = parse_date(self.date()).map_or(date, |old| old.max(date)).min(today).format("%Y-%m-%d").to_string();
        let mut parts = self.parts.clone();
        let change = format!("**Change:** {}", kind.word());
        match self.change {
            Some((c, e)) => {
                let prefix = keep(c, "**Change:**");
                parts.splice(c..e.min(parts.len()), [format!("{prefix}{change}")]);
            }
            None => parts.insert((self.date + 1).min(parts.len()), change),
        }
        if let Some(p) = parts.get_mut(self.date) {
            *p = format!("{}{STATUS_KEY} {date}", keep(self.date, STATUS_KEY));
        }
        parts.join(" · ")
    }
}

/// The newest commit time and the highest kind of what changed a doc.
type Stamp = Option<(i64, Kind)>;

fn stamp(a: Stamp, time: i64, kind: Kind) -> Stamp {
    Some(a.map_or((time, kind), |(t, k)| (t.max(time), k.max(kind))))
}

fn date_of(time: i64) -> Result<NaiveDate, String> {
    Ok(chrono::DateTime::from_timestamp(time, 0).ok_or("a commit time is out of range")?.date_naive())
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().filter(|_| s.len() == 10)
}

fn item_fields(table: &Table) -> Vec<&'static str> {
    let mut keys = vec!["rank", "status"];
    keys.extend(table.columns.iter().map(|(_, f)| *f).filter(|f| *f != "number" && *f != "body"));
    keys
}

/// An item number: 1 to [`MAX_NUMBER`], plain digits, no sign or leading zero.
fn parse_number(s: &str) -> Option<u32> {
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok().filter(|n| *n <= MAX_NUMBER)
}

fn item<'a>(f: &'a File, table: &Table, allowed: &[&str]) -> Result<Item<'a>, String> {
    guard(f)?;
    let name = file_name(&f.path);
    // Lower case only: `Readme.md` or `1-A.md` is a typo, and on a case-blind disk a clash.
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(format!("{}: an item's file name is lower case (only `README.md` isn't an item)", f.path));
    }
    let stem = name.strip_suffix(".md").ok_or_else(|| format!("{}: an item is a `.md` file", f.path))?;
    let (num, slug) = stem.split_once('-').unwrap_or((stem, ""));
    let number = parse_number(num).ok_or_else(|| format!("{}: an item's number is 1 to {MAX_NUMBER}, with no sign or leading zero", f.path))?;
    if slug.is_empty() || !slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(format!("{}: an item is `<number>-<slug>.md`, slug in lower case, digits and `-`", f.path));
    }
    let fields = front::parse(&f.text, allowed).map_err(|e| format!("{}: {e}", f.path))?;
    if fields.fields.is_empty() && fields.body.is_empty() {
        return Err(format!("{}: is empty", f.path));
    }
    if let Some(missing) = allowed.iter().skip(2).find(|k| fields.get(k).is_none()) {
        return Err(format!("{}: has no `{missing}:` for its column", f.path));
    }
    if !table.columns.iter().any(|(_, f)| *f == "body") && !fields.body.is_empty() {
        return Err(format!("{}: has a body, but its table has no body column", f.path));
    }
    if fields.body.contains(STATUS_KEY) || fields.fields.iter().any(|(_, v)| v.contains(STATUS_KEY)) {
        return Err(format!("{}: holds `{STATUS_KEY}`, which only the status line may", f.path));
    }
    let rank = fields.get("rank").map(|r| r.parse::<i32>().map_err(|_| format!("{}: `rank: {r}` isn't a whole number", f.path))).transpose()?;
    let closed = match fields.get("status") {
        None | Some("open") => false,
        Some("closed") => true,
        Some(other) => return Err(format!("{}: `status: {other}` isn't open or closed", f.path)),
    };
    Ok(Item { path: &f.path, number, rank, closed, time: f.time, fields })
}

/// A table's items, in table order. Two items with the same number (two pull requests that took the
/// same next number, both merged) are a warning, not an error: an error would stop every fold and
/// fail every pull request's check until someone renumbers one.
fn items<'a>(files: &'a [File], table: &Table, errors: &mut Vec<String>, warnings: &mut Vec<String>) -> Vec<Item<'a>> {
    let found = in_dir(files, table.dir);
    if found.len() > MAX_FILES {
        errors.push(format!("{}: more than {MAX_FILES} items", table.dir));
        return Vec::new();
    }
    let allowed = item_fields(table);
    let mut out: Vec<Item> = Vec::new();
    for f in found {
        match item(f, table, &allowed) {
            Ok(item) => {
                if let Some(other) = out.iter().find(|i| i.number == item.number) {
                    warnings.push(same_number(other.path, item.path, item.number));
                }
                out.push(item);
            }
            Err(e) => errors.push(e),
        }
    }
    // Ranked first, by rank; then unranked, by number; a shared number by slug (the path).
    out.sort_by_key(|i| (i.rank.is_none(), i.rank, i.number, i.path));
    out
}

fn same_number(a: &str, b: &str, n: u32) -> String {
    format!("{a} and {b} have the same number {n}; give one the next free number")
}

/// `\` before a `|` is doubled, then the `|` escaped, so [`cells`] reads the value back.
fn escape_pipes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0usize;
    for c in s.chars() {
        match c {
            '\\' => {
                run += 1;
                out.push(c);
            }
            '|' => {
                out.extend(std::iter::repeat_n('\\', run));
                out.push_str("\\|");
                run = 0;
            }
            _ => {
                run = 0;
                out.push(c);
            }
        }
    }
    out
}

/// The reverse of [`escape_pipes`] on a cell [`cells`] read.
fn unescape_pipes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0usize;
    for c in s.chars() {
        match c {
            '\\' => run += 1,
            '|' => {
                out.extend(std::iter::repeat_n('\\', run / 2));
                out.push('|');
                run = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', run));
                run = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', run));
    out
}

fn escape(cell: &str) -> String {
    let text = cell.trim().replace("\r\n", "\n").replace('\r', "\n");
    text.split('\n').map(|l| escape_pipes(l.trim_end())).collect::<Vec<_>>().join("<br>")
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
    // A closed item shown is struck through: every cell but its number, unless empty or struck.
    let strike = item.closed && table.show_closed;
    let cells: Vec<String> = table
        .columns
        .iter()
        .zip(cells)
        .map(|((_, field), c)| if strike && *field != "number" && !c.is_empty() && !c.starts_with("~~") { format!("~~{c}~~") } else { c })
        .collect();
    // An empty cell is `| |`, as people write it.
    let cells: Vec<String> = cells.iter().map(|c| if c.is_empty() { " ".to_string() } else { format!(" {c} ") }).collect();
    format!("|{}|", cells.join("|"))
}

fn is_separator(line: &str) -> bool {
    cells(line).is_some_and(|c| !c.is_empty() && c.iter().all(|c| c.contains('-') && c.bytes().all(|b| matches!(b, b'-' | b':' | b' '))))
}

fn check_header(lines: &[&str], header: usize, table: &Table) -> Result<(), String> {
    let have = lines.get(header).and_then(|l| cells(l)).unwrap_or_default();
    let want: Vec<&str> = table.columns.iter().map(|(h, _)| *h).collect();
    if have != want {
        return Err(format!("`{}`: the table's columns are {:?}, the config says {:?}", table.heading, have, want));
    }
    if !lines.get(header + 1).is_some_and(|l| is_separator(l)) {
        return Err(format!("`{}`: the table's header has no `|---|` line under it", table.heading));
    }
    Ok(())
}

/// The configured table in `lines`: (first row, end of rows).
fn place(lines: &[&str], table: &Table) -> Result<(usize, usize), String> {
    let (h, first, end) = locate(lines, table)?;
    check_rows(lines, (h, first, end), &format!("`{}`", table.heading))?;
    Ok((first, end))
}

/// [`place`], with rows of any width (a pull request's rows that `migrate` reports on). The
/// table's header may head only one table in its section: a copy above it would take its place.
fn locate(lines: &[&str], table: &Table) -> Result<(usize, usize, usize), String> {
    let (s, e) = section(lines, table.heading)?;
    let all = tables_in(lines, s, e);
    let want: Vec<&str> = table.columns.iter().map(|(h, _)| *h).collect();
    let same: Vec<usize> = all.iter().map(|t| t.0).filter(|h| lines.get(*h).is_some_and(|l| row_cells(l) == want)).collect();
    if let [a, b, ..] = same.as_slice() {
        return Err(format!(
            "has {} tables headed {want:?} under `{}` (lines {} and {}); keep only the joined one",
            same.len(),
            table.heading,
            a + 1,
            b + 1
        ));
    }
    let (h, first, end) = all.get(table.nth).copied().ok_or_else(|| match table.nth {
        0 => format!("has no table under `{}`", table.heading),
        n => format!("has no table {} under `{}`", n + 1, table.heading),
    })?;
    check_header(lines, h, table)?;
    Ok((h, first, end))
}

/// How many tables the section of `table` holds.
fn table_count(lines: &[&str], table: &Table) -> Option<usize> {
    section(lines, table.heading).ok().map(|(s, e)| tables_in(lines, s, e).len())
}

fn table_name(table: &Table) -> String {
    match table.nth {
        0 => format!("the table under `{}`", table.heading),
        n => format!("table {} under `{}`", n + 1, table.heading),
    }
}

fn writes_status(cfg: &Config, doc: &str) -> bool {
    cfg.logs.iter().any(|l| l.doc == doc && l.status) || cfg.tables.iter().any(|t| t.doc == doc && t.status)
}

/// The rows the fold writes for `table`, and, when the table has a status line and the rows
/// differ from the doc's, the newest commit time among the items that changed them.
fn table_rows(table: &Table, found: &[Item], current: &[&str]) -> (Vec<String>, Option<i64>) {
    let shown: Vec<&Item> = found.iter().filter(|i| table.show_closed || !i.closed).collect();
    let rows: Vec<String> = shown.iter().map(|i| render(table, i)).collect();
    if !table.status || rows.iter().map(String::as_str).eq(current.iter().map(|l| l.trim_end())) {
        return (rows, None);
    }
    let changed = shown.iter().zip(&rows).filter(|(_, r)| !current.iter().any(|c| c.trim_end() == r.as_str())).map(|(i, _)| i.time).max();
    let time = changed.or_else(|| found.iter().map(|i| i.time).max()).unwrap_or(0);
    (rows, Some(time))
}

fn join_doc(cfg: &Config, files: &[File], file: &File, today: NaiveDate, out: &mut Joined, errors: &mut Vec<String>) {
    let doc = file.path.as_str();
    let mut lines: Vec<&str> = file.text.lines().collect();
    let mut owned_rows: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut changed: Stamp = None;
    for log in cfg.logs.iter().filter(|l| l.doc == doc) {
        let frags = fragments(files, log, errors);
        if let Err(e) = join_log(&mut lines, log, &frags) {
            errors.push(format!("{doc} {e}"));
        }
        out.consumed.extend(frags.iter().map(|f| f.path.to_string()));
        if log.status {
            for f in &frags {
                changed = stamp(changed, f.time, f.kind);
            }
        }
    }
    for table in cfg.tables.iter().filter(|t| t.doc == doc) {
        let found = items(files, table, errors, &mut out.warnings);
        match place(&lines, table) {
            Ok((first, end)) => {
                let (rows, time) = table_rows(table, &found, lines.get(first..end).unwrap_or_default());
                if let Some(time) = time {
                    changed = stamp(changed, time, Kind::Minor);
                }
                owned_rows.push((first, end, rows));
            }
            Err(e) => errors.push(format!("{doc} {e}")),
        }
    }
    // Tables are replaced bottom-up, so earlier line numbers stay right.
    owned_rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let mut joined: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (first, end, rows) in owned_rows {
        if joined.get(first..end).is_some() {
            joined.splice(first..end, rows);
        }
    }
    if writes_status(cfg, doc) {
        let written = Status::find(&joined).and_then(|s| match changed {
            Some((time, kind)) => Ok((s.at, s.written(date_of(time)?, kind, today))),
            None => Ok((s.at, String::new())),
        });
        match written {
            Ok((at, line)) if !line.is_empty() => {
                if let Some(l) = joined.get_mut(at) {
                    *l = line;
                }
            }
            Ok(_) => {}
            Err(e) => errors.push(format!("{doc} {e}")),
        }
    }
    let mut text = joined.join("\n");
    if file.text.ends_with('\n') {
        text.push('\n');
    }
    out.docs.insert(doc.to_string(), text);
}

fn docs_of(cfg: &Config) -> BTreeSet<&'static str> {
    cfg.logs.iter().map(|l| l.doc).chain(cfg.tables.iter().map(|t| t.doc)).collect()
}

/// Every configured doc with its fragments and items joined in. Errors list every bad file. No
/// status date is written after `today`.
pub fn join(cfg: &Config, files: &[File], today: NaiveDate) -> Result<Joined, Vec<String>> {
    let (out, errors) = join_all(cfg, files, today);
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

/// [`join`] as far as it gets, and its errors: bad files are left out.
fn join_all(cfg: &Config, files: &[File], today: NaiveDate) -> (Joined, Vec<String>) {
    let mut errors = Vec::new();
    let by_path: BTreeMap<&str, &File> = files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut out = Joined::default();
    for doc in docs_of(cfg) {
        match by_path.get(doc) {
            Some(file) => join_doc(cfg, files, file, today, &mut out, &mut errors),
            None => errors.push(format!("{doc}: not found")),
        }
    }
    (out, errors)
}

const HAND: &str = "was edited by hand. Only the fold writes it: add a fragment or edit the item file instead";

/// The fragments of `log` in `base` that the pull request deleted: only the fold does that.
fn consumed<'a>(base: &'a [File], head: &BTreeSet<&str>, log: &Log, errors: &mut Vec<String>) -> Vec<Fragment<'a>> {
    let mut out = Vec::new();
    for f in in_dir(base, log.dir).into_iter().filter(|f| !head.contains(f.path.as_str())) {
        match fragment(f) {
            Ok(fragment) => out.push(fragment),
            Err(e) => errors.push(format!("{} was deleted, but the fold couldn't have folded it: {e}", f.path)),
        }
    }
    out
}

fn trimmed<'a>(lines: &[&'a str], range: (usize, usize)) -> Vec<&'a str> {
    lines.get(range.0..range.1).unwrap_or_default().iter().map(|l| l.trim_end()).collect()
}

/// Whether `mid` is `blocks` one after another, each whole and in its own order, the blocks in any
/// order. A search with backtracking (one block can start like another), bounded in steps.
fn is_blocks(mid: &[&str], blocks: &[&[&str]]) -> bool {
    if blocks.iter().map(|b| b.len()).sum::<usize>() != mid.len() {
        return false;
    }
    let mut used = vec![false; blocks.len()];
    let mut chosen: Vec<usize> = Vec::new();
    let (mut pos, mut from) = (0, 0);
    for _ in 0..100_000 {
        if pos == mid.len() {
            return true;
        }
        let rest = mid.get(pos..).unwrap_or_default();
        let next = (from..blocks.len()).find(|&j| !used.get(j).copied().unwrap_or(true) && blocks.get(j).is_some_and(|b| rest.starts_with(b)));
        match next {
            Some(j) => {
                if let Some(u) = used.get_mut(j) {
                    *u = true;
                }
                chosen.push(j);
                pos += blocks.get(j).map_or(0, |b| b.len());
                from = 0;
            }
            None => {
                let Some(j) = chosen.pop() else { return false };
                if let Some(u) = used.get_mut(j) {
                    *u = false;
                }
                pos -= blocks.get(j).map_or(0, |b| b.len());
                from = j + 1;
            }
        }
    }
    false
}

/// The log in the head must be the base's with the deleted fragments' entries put in as one block
/// where the fold puts them: each entry whole, the entries in any order, as commit times aren't
/// known on CI's shallow checkout.
fn check_log(doc: &str, base: &[&str], head: &[&str], log: &Log, consumed: &[Fragment], errors: &mut Vec<String>) {
    let (Ok(bs), Ok(hs)) = (section(base, log.heading), section(head, log.heading)) else { return };
    let (b, h) = (trimmed(base, bs), trimmed(head, hs));
    let hand = format!("{doc}: `{}` {HAND}", log.heading);
    if consumed.is_empty() {
        if b != h {
            errors.push(hand);
        }
        return;
    }
    let at = log_insert(base, log).map_or(b.len(), |i| i.at.saturating_sub(bs.0).min(b.len()));
    let (pre, post) = b.split_at(at);
    let blocks: Vec<&[&str]> = consumed.iter().map(|f| f.lines.as_slice()).collect();
    let fits = h.len() >= pre.len() + post.len() && h.starts_with(pre) && h.ends_with(post) && {
        let mid: Vec<&str> = h.get(pre.len()..h.len() - post.len()).unwrap_or_default().iter().copied().filter(|l| !l.trim().is_empty()).collect();
        is_blocks(&mid, &blocks)
    };
    if fits {
        return;
    }
    let missing: Vec<&Fragment> = consumed.iter().filter(|f| !f.lines.iter().all(|l| h.contains(l))).collect();
    if missing.is_empty() {
        errors.push(hand);
    }
    for f in missing {
        errors.push(format!("{} was deleted but its entry isn't in `{}` of {doc}; only the fold removes a fragment", f.path, log.heading));
    }
}

/// The status line may move only in a pull request that folded fragments into the doc or changed
/// (or rebuilt) a table the line follows, and only to the `Change` the fold writes. Its date never
/// goes back and is never after `today`: the fold never moves a date back, so a future one would
/// stick.
fn check_status(doc: &str, base: &[&str], head: &[&str], want: Option<Kind>, today: NaiveDate, errors: &mut Vec<String>) {
    let Ok(h) = Status::find(head) else { return };
    let (bd, bc) = Status::find(base).map(|b| (b.date().to_string(), b.change())).unwrap_or_default();
    let (hd, hc) = (h.date(), h.change());
    if bd == hd && bc == hc {
        return;
    }
    let Some(want) = want else {
        for (field, same) in [("**Last updated:**", bd == hd), ("**Change:**", bc == hc)] {
            if !same {
                errors.push(format!("{doc}: the status line's {field} {HAND}"));
            }
        }
        return;
    };
    if hc != want.word() {
        errors.push(format!("{doc}: the status line's **Change:** is `{hc}`; the fold writes `{}`", want.word()));
    }
    if hd == bd {
        return;
    }
    let (Some(new), old) = (parse_date(hd), parse_date(&bd)) else { return };
    if new > today {
        errors.push(format!("{doc}: the status line's **Last updated:** is {hd}, after today ({today})"));
    } else if old.is_some_and(|old| new < old.min(today)) {
        errors.push(format!("{doc}: the status line's **Last updated:** went back from {bd} to {hd}"));
    }
}

/// `doc` with its tables as the fold would write them from the head's items.
fn tables_folded(cfg: &Config, doc: &str, base_text: &str, head: &[File], today: NaiveDate) -> Option<String> {
    let in_logs = |f: &File| cfg.logs.iter().any(|l| f.path.starts_with(&format!("{}/", l.dir)));
    let mut from: Vec<File> = head.iter().filter(|f| !in_logs(f) && f.path != doc).cloned().collect();
    from.push(File { path: doc.to_string(), text: base_text.to_string(), time: 0 });
    join_all(&Config { logs: &[], tables: cfg.tables }, &from, today).0.docs.get(doc).cloned()
}

fn rows_of(lines: &[&str], table: &Table) -> Option<Vec<String>> {
    place(lines, table).ok().map(|r| trimmed(lines, r).into_iter().map(str::to_string).collect())
}

fn check_doc(cfg: &Config, base: &[File], head: &[File], doc: &str, today: NaiveDate, errors: &mut Vec<String>) {
    let text_of = |files: &[File]| files.iter().find(|f| f.path == doc).map(|f| f.text.clone());
    let (Some(base_text), Some(head_text)) = (text_of(base), text_of(head)) else { return };
    let (bl, hl): (Vec<&str>, Vec<&str>) = (base_text.lines().collect(), head_text.lines().collect());
    let head_paths: BTreeSet<&str> = head.iter().map(|f| f.path.as_str()).collect();
    // The cutover pull request: the base has no folder yet, so nothing was folded there. A
    // README is never deleted and an item never is, so a started folder stays started.
    let started = |dir: &str| base.iter().any(|f| f.path.starts_with(&format!("{dir}/")));
    let mut kinds: Option<Kind> = None;
    let mut exempt = false;
    for log in cfg.logs.iter().filter(|l| l.doc == doc) {
        let gone = consumed(base, &head_paths, log, errors);
        if log.status {
            kinds = gone.iter().map(|f| f.kind).chain(kinds).max();
            exempt |= !started(log.dir);
        }
        if started(log.dir) {
            check_log(doc, &bl, &hl, log, &gone, errors);
        }
    }
    let want = tables_folded(cfg, doc, &base_text, head, today);
    let want_lines: Vec<&str> = want.as_deref().unwrap_or_default().lines().collect();
    let mut table_moved = false;
    for table in cfg.tables.iter().filter(|t| t.doc == doc) {
        let (b, h) = (rows_of(&bl, table), rows_of(&hl, table));
        let changed = b != h;
        if table.status {
            exempt |= !started(table.dir);
            let edited = in_dir(head, table.dir).iter().any(|h| !base.iter().any(|b| b.path == h.path && b.text == h.text));
            table_moved |= changed || edited;
        }
        // A table added to the section, or one taken out, moves which one is joined.
        if started(table.dir) && table_count(&bl, table) != table_count(&hl, table) {
            errors.push(format!(
                "{doc}: the section `{}` has {} table(s), it had {}; the joined table must stay where it is",
                table.heading,
                table_count(&hl, table).unwrap_or(0),
                table_count(&bl, table).unwrap_or(0)
            ));
        }
        if started(table.dir) && changed && (want.is_none() || h != rows_of(&want_lines, table)) {
            errors.push(format!("{doc}: {} {HAND}", table_name(table)));
        }
    }
    if writes_status(cfg, doc) && !exempt {
        let want = if table_moved { kinds.max(Some(Kind::Minor)) } else { kinds };
        check_status(doc, &bl, &hl, want, today, errors);
    }
}

/// What [`check_report`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// What fails the pull request.
    pub errors: Vec<String>,
    /// What was already wrong on the base, which the pull request didn't cause.
    pub warnings: Vec<String>,
}

/// [`check_report`]'s errors.
pub fn check(cfg: &Config, base: &[File], head: &[File], today: NaiveDate) -> Vec<String> {
    check_report(cfg, base, head, today).errors
}

/// The item numbers each of a table's item files takes, by number.
fn numbers<'a>(files: &'a [File], table: &Table) -> BTreeMap<u32, Vec<&'a str>> {
    let mut out: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for f in in_dir(files, table.dir) {
        let stem = file_name(&f.path).split('-').next().unwrap_or_default();
        if let Some(n) = parse_number(stem) {
            out.entry(n).or_default().push(&f.path);
        }
    }
    out
}

/// What CI runs on a pull request: `base` is the merge base's files, `head` the pull request's.
/// Fails on any bad fragment or item the pull request added or changed (what was already wrong on
/// the base is a warning); on a number it gave a second item; on a deleted item, folder README or
/// unfolded fragment; and on a joined region the pull request changed unless it now reads as the
/// fold would write it (the fold's own pull request). `today` bounds the status dates a pull
/// request may write.
pub fn check_report(cfg: &Config, base: &[File], head: &[File], today: NaiveDate) -> Report {
    let (_, head_errors) = join_all(cfg, head, today);
    let (_, base_errors) = join_all(cfg, base, today);
    // An error the base has too, about no file the pull request added or changed, isn't its own.
    let changed: Vec<&str> = head.iter().filter(|h| !base.iter().any(|b| b.path == h.path && b.text == h.text)).map(|h| h.path.as_str()).collect();
    let (old, new): (Vec<String>, Vec<String>) =
        head_errors.into_iter().partition(|e| base_errors.contains(e) && !changed.iter().any(|p| e.contains(p)));
    let mut report = Report { errors: new, warnings: old };
    for table in cfg.tables {
        let before = numbers(base, table);
        for (n, paths) in numbers(head, table) {
            let [a, b, ..] = paths.as_slice() else { continue };
            // Only a number the pull request gave one more item fails it.
            let to = if paths.len() > before.get(&n).map_or(0, Vec::len) { &mut report.errors } else { &mut report.warnings };
            to.push(same_number(a, b, n));
        }
    }
    if !report.errors.is_empty() {
        return report;
    }
    // Commit times are not used here: CI's checkout is shallow, so they aren't known.
    let mut errors = Vec::new();
    let head_paths: BTreeSet<&str> = head.iter().map(|f| f.path.as_str()).collect();
    let dirs: BTreeSet<&str> = cfg.logs.iter().map(|l| l.dir).chain(cfg.tables.iter().map(|t| t.dir)).collect();
    for dir in dirs {
        let readme = format!("{dir}/README.md");
        if base.iter().any(|f| f.path == readme) && !head_paths.contains(readme.as_str()) {
            errors.push(format!("{readme}: was deleted; it keeps the folder started, put it back"));
        }
    }
    // A log folder empties at every fold; its README keeps it started (see `check_doc`).
    for log in cfg.logs {
        let readme = format!("{}/README.md", log.dir);
        if !head_paths.contains(readme.as_str()) {
            errors.push(format!("{readme}: missing; every log folder keeps a README so a fold can't empty it"));
        }
    }
    // ponytail: no README needed in table folders; items are never deleted, so a started one stays started.
    for dir in cfg.tables.iter().map(|t| t.dir).collect::<BTreeSet<_>>() {
        for f in in_dir(base, dir).into_iter().filter(|f| !head_paths.contains(f.path.as_str())) {
            errors.push(format!("{}: was deleted; items are never deleted, set `status: closed` instead", f.path));
        }
    }
    for doc in docs_of(cfg) {
        check_doc(cfg, base, head, doc, today, &mut errors);
    }
    report.errors = errors;
    report
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

fn number_column(table: &Table) -> Option<usize> {
    table.columns.iter().position(|(_, f)| *f == "number")
}

/// The number in a row's number column; `None` when it has none.
fn row_number(table: &Table, row: &[String]) -> Result<Option<u32>, String> {
    let Some(cell) = number_column(table).and_then(|i| row.get(i)) else { return Ok(None) };
    let digits: String = cell.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok(None);
    }
    digits.parse::<u32>().ok().filter(|n| *n <= MAX_NUMBER).map(Some).ok_or_else(|| format!("number {cell} is above {MAX_NUMBER}"))
}

/// How a row is matched across versions: its number, or, in a table without one, its first cell.
fn row_key(table: &Table, row: &[String]) -> Option<String> {
    if number_column(table).is_some() {
        return row_number(table, row).ok().flatten().map(|n| n.to_string());
    }
    row.first().filter(|c| !c.is_empty()).cloned()
}

fn row_title(table: &Table, row: &[String]) -> String {
    table.columns.iter().zip(row).find(|((_, f), c)| *f != "number" && *f != "body" && !c.is_empty()).map(|(_, c)| c.clone()).unwrap_or_default()
}

fn row_closed(table: &Table, row: &[String]) -> bool {
    row_title(table, row).trim_start().starts_with("~~")
}

fn item_path(table: &Table, number: u32, row: &[String]) -> String {
    format!("{}/{number}-{}.md", table.dir, slug(&row_title(table, row).replace('~', "")))
}

/// A cell struck through whole, `~~…~~`, without the strike.
fn unstruck(cell: &str) -> &str {
    cell.strip_prefix("~~").and_then(|c| c.strip_suffix("~~")).filter(|c| !c.is_empty() && !c.contains("~~")).unwrap_or(cell)
}

/// An item file's text from a table row's cells. A closed row's strike-through is the fold's (see
/// [`render`]), so it comes off the fields.
fn item_text(table: &Table, row: &[String], rank: Option<i32>, closed: bool) -> String {
    let mut text = String::from("---\n");
    if let Some(rank) = rank {
        text.push_str(&format!("rank: {rank}\n"));
    }
    if closed {
        text.push_str("status: closed\n");
    }
    let mut body = String::new();
    for ((_, field), cell) in table.columns.iter().zip(row) {
        let cell = if closed { unstruck(cell) } else { cell };
        match *field {
            "number" => {}
            "body" => body = unescape_pipes(&cell.replace("<br>", "\n")),
            other => text.push_str(&format!("{other}: {}\n", unescape_pipes(cell))),
        }
    }
    text.push_str("---\n");
    if !body.is_empty() {
        text.push('\n');
        text.push_str(&body);
        text.push('\n');
    }
    text
}

fn table_items(table: &Table, doc: &File, out: &mut Vec<File>, notes: &mut Vec<String>) -> Result<(), Vec<String>> {
    let lines: Vec<&str> = doc.text.lines().collect();
    let (first, end) = place(&lines, table).map_err(|e| vec![format!("{} {e}", table.doc)])?;
    // Every line in the range is a row; a row of the wrong width is an error, never dropped.
    let rows: Vec<Vec<String>> = lines.get(first..end).unwrap_or_default().iter().map(|l| row_cells(l)).collect();
    let mut errors = Vec::new();
    let mut numbers = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if row.len() != table.columns.len() {
            errors.push(format!("{} `{}` row {}: {} cells, {} columns", table.doc, table.heading, i + 1, row.len(), table.columns.len()));
        }
        match row_number(table, row) {
            Ok(n) => numbers.push(n),
            Err(e) => errors.push(format!("{} `{}` row {}: {e}", table.doc, table.heading, i + 1)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut next = numbers.iter().flatten().max().copied().unwrap_or(0).checked_add(1);
    let mut used: BTreeSet<u32> = BTreeSet::new();
    for (i, (row, own)) in rows.iter().zip(numbers).enumerate() {
        let number = match own.filter(|n| !used.contains(n)) {
            Some(n) => n,
            None => {
                let Some(n) = next.filter(|n| *n <= MAX_NUMBER) else {
                    return Err(vec![format!("{} `{}` row {}: no free number up to {MAX_NUMBER}", table.doc, table.heading, i + 1)]);
                };
                if number_column(table).is_some() {
                    let why = if own.is_some() { "repeated" } else { "missing" };
                    notes.push(format!("{} `{}` row {}: number {} is {why}; it becomes {n}", table.doc, table.heading, i + 1, own.unwrap_or(0)));
                }
                next = n.checked_add(1);
                n
            }
        };
        used.insert(number);
        let rank = i32::try_from(i + 1).ok();
        out.push(File { path: item_path(table, number, row), text: item_text(table, row, rank, row_closed(table, row)), time: doc.time });
    }
    Ok(())
}

/// The cutover: an item file for every row of every configured table in `docs`, keeping each row's
/// place as its `rank`. A repeated number, or a row with none, takes the next free number; each
/// such change is in the returned notes.
pub fn items_from_doc(cfg: &Config, docs: &[File]) -> Result<(Vec<File>, Vec<String>), Vec<String>> {
    items_from_tables(cfg.tables, docs)
}

/// [`items_from_doc`] for some of the configured tables.
pub fn items_from_tables(tables: &[Table], docs: &[File]) -> Result<(Vec<File>, Vec<String>), Vec<String>> {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    for table in tables {
        match docs.iter().find(|d| d.path == table.doc) {
            Some(doc) => {
                if let Err(e) = table_items(table, doc, &mut out, &mut notes) {
                    errors.extend(e);
                }
            }
            None => errors.push(format!("{}: not found", table.doc)),
        }
    }
    if errors.is_empty() { Ok((out, notes)) } else { Err(errors) }
}

/// A row the pull request edited, applied to main's item: only the cells the pull request changed,
/// and of those only where main still has the old value.
fn migrate_edit(table: &Table, item: &Item, old: &[String], pr: &[String], out: &mut Vec<File>, notes: &mut Vec<String>) {
    let cur = cells(&render(table, item)).unwrap_or_default();
    let mut new = cur.clone();
    for (i, (header, _)) in table.columns.iter().enumerate() {
        let (o, p, c) = (old.get(i), pr.get(i), cur.get(i));
        if o == p {
            continue;
        }
        if c != o && c != p {
            notes.push(format!(
                "{}: conflict in `{header}` of {}: main has {:?}, the pull request {:?}; main's is kept, merge it by hand",
                table.doc,
                item.path,
                c.cloned().unwrap_or_default(),
                p.cloned().unwrap_or_default()
            ));
        } else if let (Some(slot), Some(p)) = (new.get_mut(i), p) {
            slot.clone_from(p);
        }
    }
    if new != cur {
        let closed = item.closed || (row_closed(table, &new) && !row_closed(table, &cur));
        out.push(File { path: item.path.to_string(), text: item_text(table, &new, item.rank, closed), time: 0 });
    }
}

fn migrate_table(table: &Table, files: &[File], old: &[&str], pr: &[&str], out: &mut Vec<File>, notes: &mut Vec<String>) {
    let rows = |lines: &[&str]| -> Vec<Vec<String>> {
        locate(lines, table).ok().map(|(_, f, e)| lines.get(f..e).unwrap_or_default().iter().map(|l| row_cells(l)).collect()).unwrap_or_default()
    };
    let (before, after) = (rows(old), rows(pr));
    let mut errs = Vec::new();
    let current = items(files, table, &mut errs, &mut Vec::new());
    let current_keys: Vec<Option<String>> = current.iter().map(|i| row_key(table, &cells(&render(table, i)).unwrap_or_default())).collect();
    let mut next = current.iter().map(|i| i.number).max().unwrap_or(0).checked_add(1);
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
        let key = row_key(table, row);
        let old_row = key.as_ref().and_then(|k| before.iter().find(|b| row_key(table, b).as_ref() == Some(k)));
        if let (Some(old_row), Some(k)) = (old_row, key.as_ref()) {
            match current.iter().zip(&current_keys).find(|(_, ck)| ck.as_ref() == Some(k)) {
                Some((item, _)) => migrate_edit(table, item, old_row, row, out, notes),
                None => notes.push(format!("{}: the pull request edited row {k}, which main no longer has; move it by hand: {row:?}", table.doc)),
            }
            continue;
        }
        let Some(n) = next.filter(|n| *n <= MAX_NUMBER) else {
            notes.push(format!("{}: no free number up to {MAX_NUMBER} for a new row; move it by hand: {row:?}", table.doc));
            continue;
        };
        if let Ok(Some(own)) = row_number(table, row).map(|o| o.filter(|o| *o != n)) {
            notes.push(format!("{}: new row numbered {own} becomes {n}", table.doc));
        }
        out.push(File { path: item_path(table, n, row), text: item_text(table, row, None, row_closed(table, row)), time: 0 });
        next = n.checked_add(1);
    }
    for row in before.iter().filter(|r| !after.contains(r)) {
        let key = row_key(table, row);
        if !after.iter().any(|a| row_key(table, a) == key) {
            notes.push(format!(
                "{}: the pull request removed row {}; set `status: closed` on its item by hand if that was meant",
                table.doc,
                key.unwrap_or_default()
            ));
        }
    }
}

enum Op<'a> {
    Keep,
    Del(&'a str),
    Add(&'a str),
}

/// The most cells the log diff's table may have; a longer log is matched line by line.
const MAX_DIFF: usize = 4_000_000;

/// `a` to `b` as kept, deleted and added lines (a longest common subsequence).
fn diff<'a>(a: &[&'a str], b: &[&'a str]) -> Option<Vec<Op<'a>>> {
    let (n, m) = (a.len(), b.len());
    if (n + 1).saturating_mul(m + 1) > MAX_DIFF {
        return None;
    }
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    let at = |dp: &[u32], i: usize, j: usize| dp.get(i * w + j).copied().unwrap_or(0);
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if a.get(i) == b.get(j) { at(&dp, i + 1, j + 1) + 1 } else { at(&dp, i + 1, j).max(at(&dp, i, j + 1)) };
            if let Some(slot) = dp.get_mut(i * w + j) {
                *slot = v;
            }
        }
    }
    let (mut i, mut j, mut ops) = (0, 0, Vec::new());
    while i < n || j < m {
        match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x == y => {
                ops.push(Op::Keep);
                i += 1;
                j += 1;
            }
            (Some(x), Some(_)) if at(&dp, i + 1, j) >= at(&dp, i, j + 1) => {
                ops.push(Op::Del(x));
                i += 1;
            }
            (_, Some(y)) => {
                ops.push(Op::Add(y));
                j += 1;
            }
            (Some(x), None) => {
                ops.push(Op::Del(x));
                i += 1;
            }
            (None, None) => break,
        }
    }
    Some(ops)
}

/// The log lines a pull request added, for its fragment. A line it edited or removed is a note,
/// never a second entry or a silent loss: main keeps its copy, and the edit is made by hand.
fn migrate_log<'a>(log: &Log, before: &[&'a str], after: &[&'a str], notes: &mut Vec<String>) -> Vec<&'a str> {
    let Some(ops) = diff(before, after) else {
        // Too long to diff: a line counts as added once the old log's copies of it are used up.
        let mut left: BTreeMap<&str, usize> = BTreeMap::new();
        for l in before {
            *left.entry(l).or_default() += 1;
        }
        let mut added = Vec::new();
        for l in after {
            match left.get_mut(l) {
                Some(n) if *n > 0 => *n -= 1,
                _ => added.push(*l),
            }
        }
        for (l, n) in left.into_iter().filter(|(_, n)| *n > 0) {
            notes.push(format!("{}: the pull request edited or removed the log line {l:?} ({n}x); main keeps it, change it by hand", log.doc));
        }
        return added;
    };
    let mut added = Vec::new();
    let mut hunk: (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut flush = |hunk: &mut (Vec<&'a str>, Vec<&'a str>), added: &mut Vec<&'a str>| {
        let (dels, adds) = std::mem::take(hunk);
        if dels.is_empty() {
            added.extend(adds);
            return;
        }
        for d in &dels {
            notes.push(format!("{}: the pull request edited or removed the log line {d:?}; main keeps it, change it by hand", log.doc));
        }
        for a in adds {
            notes.push(format!("{}: the log line {a:?} replaces one the pull request edited; it isn't moved, add it by hand", log.doc));
        }
    };
    for op in ops {
        match op {
            Op::Keep => flush(&mut hunk, &mut added),
            Op::Del(l) => hunk.0.push(l),
            Op::Add(l) => hunk.1.push(l),
        }
    }
    flush(&mut hunk, &mut added);
    added
}

/// Rewrites a pull request opened before the cutover: `old` is the doc at its merge base, `pr` the
/// doc at its head, `files` the current `main` (docs, fragments and items). Rows are matched by
/// their number, or by their first cell in a table without one. Returns the files to write on the
/// pull request's branch, and notes on what couldn't be moved (never silently dropped).
pub fn migrate<'a>(cfg: &Config, files: &[File], old: &'a File, pr: &'a File, name: &str) -> (Vec<File>, Vec<String>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let (old_lines, pr_lines): (Vec<&'a str>, Vec<&'a str>) = (old.text.lines().collect(), pr.text.lines().collect());
    for log in cfg.logs.iter().filter(|l| l.doc == old.path) {
        let (Ok((os, oe)), Ok((ps, pe))) = (section(&old_lines, log.heading), section(&pr_lines, log.heading)) else {
            continue;
        };
        let lines = |l: &[&'a str]| -> Vec<&'a str> { l.iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect() };
        let (before, after) = (lines(old_lines.get(os..oe).unwrap_or_default()), lines(pr_lines.get(ps..pe).unwrap_or_default()));
        let added = migrate_log(log, &before, &after, &mut notes);
        if !added.is_empty() {
            out.push(File { path: format!("{}/{name}.md", log.dir), text: format!("{}\n", added.join("\n")), time: 0 });
        }
    }
    for table in cfg.tables.iter().filter(|t| t.doc == old.path) {
        migrate_table(table, files, &old_lines, &pr_lines, &mut out, &mut notes);
    }
    (out, notes)
}

#[cfg(test)]
mod tests;
