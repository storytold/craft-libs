//! The exception files and the ratchet: `design/exceptions/<crate>/<family>.toml`.
//!
//! Every allowed site (a `// design: allow` marker, an `#[expect(clippy::disallowed_..)]`, an `[[exempt]]`
//! path, a bridge wrapper, a counted allowance) is an entry with a name and a reason. A site no entry
//! covers fails the check, so removing one exception and adding another cannot cancel out. Counts may
//! fall and never rise: an entry that lists more sites than exist fails too, so a fall is written down
//! and a later rise is a diff line.
//!
//! An entry may carry `expires`, the group or release that removes it. `design/exceptions/milestones.toml`
//! lists the milestones that have landed (`done = [..]`); an entry whose milestone landed fails.
//!
//! ```toml
//! [[exception]]
//! name = "window-egui-dependency"   # unique across all files
//! kind = "dependency"               # marker, exempt, expect, probe, bridge, shim, pointer-only, canvas-tool, menu-entry, dependency, feature
//! rule = "graph/egui"               # rule id or family; `*` for any
//! path = "craft-window->egui"       # file, directory or subject the check names; omit for the whole crate
//! reason = "native window code still calls egui until the cut-over"
//! count = 1                         # default 1
//! expires = "cutover-3"             # required for bridge and shim
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use serde::Deserialize;
use syn::visit::{self, Visit};

use crate::Error;
use crate::layers::{finding, read_toml, relative, sorted_entries};
use crate::markers::MIN_REASON;
use crate::report::Finding;

/// A site that carries an exception but has no entry, or more sites than the entry lists.
pub const UNLISTED: &str = "exception/unlisted";
/// An entry that lists more sites than exist, or none: lower its count or remove it.
pub const STALE: &str = "exception/stale";
/// An entry whose `expires` milestone has landed.
pub const EXPIRED: &str = "exception/expired";
/// An entry that is malformed: no reason, a duplicate name, a bridge or shim without an expiry.
pub const INVALID: &str = "exception/invalid";

/// The rule family of a probe exemption site (`probe/exempt/<case>`), for the entries of kind `probe`.
pub const PROBE_EXEMPT: &str = "probe/exempt";

/// The `kind` values an entry may have.
pub const KINDS: [&str; 11] =
    ["marker", "exempt", "expect", "probe", "bridge", "shim", "pointer-only", "canvas-tool", "menu-entry", "dependency", "feature"];

/// Kinds that must say when they go away.
const EXPIRING_KINDS: [&str; 2] = ["bridge", "shim"];

const MILESTONES_FILE: &str = "milestones.toml";

/// One listed exception.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    /// Unique name.
    pub name: String,
    /// One of [`KINDS`].
    pub kind: String,
    /// Rule id, family or `*`.
    pub rule: String,
    /// File, directory or check subject; `None` covers the whole crate.
    pub path: Option<String>,
    /// Why the exception is allowed.
    pub reason: String,
    /// How many sites it covers.
    pub count: usize,
    /// The milestone that removes it.
    pub expires: Option<String>,
    /// The file it came from, relative to the root.
    pub file: PathBuf,
    /// The crate directory it sits in.
    pub krate: String,
}

/// Where a site came from, which decides the rule id of an uncovered one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// A breach of the named rule that an entry may excuse; uncovered, it is reported under that rule.
    Violation(&'static str),
    /// A place the source already allows; uncovered, it is `exception/unlisted`.
    Allowance,
}

/// A site found in the sources or by a check, to be matched against the entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Site {
    /// The rule the site breaks or allows.
    pub rule: String,
    /// File path or check subject (`craft-window->egui`).
    pub subject: String,
    /// 1-based line, 1 for a subject that is not a file.
    pub line: usize,
    /// Violation or allowance.
    pub origin: Origin,
}

impl Site {
    /// A breach found by a check.
    pub fn violation(rule: &'static str, subject: impl Into<String>) -> Self {
        Self { rule: rule.to_owned(), subject: subject.into(), line: 1, origin: Origin::Violation(rule) }
    }
}

/// All entries and the milestones that have landed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Exceptions {
    /// Entries of every file.
    pub entries: Vec<Entry>,
    /// Milestones that have landed.
    pub done: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    exception: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    name: String,
    kind: String,
    rule: String,
    path: Option<String>,
    reason: Option<String>,
    count: Option<usize>,
    expires: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMilestones {
    #[serde(default)]
    done: Vec<String>,
}

/// Whether `selector` (a rule id, family or `*`) covers `rule`.
pub(crate) fn rule_covers(selector: &str, rule: &str) -> bool {
    selector == "*" || crate::rules::selector_matches(selector, rule)
}

fn subject_covers(entry: &str, subject: &str) -> bool {
    subject == entry || subject.strip_prefix(entry).is_some_and(|rest| rest.starts_with('/'))
}

impl Exceptions {
    /// Reads `dir` (`design/exceptions`); a missing directory holds no entries.
    ///
    /// Returns the entries and the `exception/invalid` findings. Finding paths are relative to `root`.
    pub fn load(root: &Path, dir: &Path) -> Result<(Self, Vec<Finding>), Error> {
        let mut found = Self::default();
        let mut findings = Vec::new();
        let milestones = dir.join(MILESTONES_FILE);
        if milestones.is_file() {
            found.done = read_toml::<RawMilestones>(&milestones)?.done.into_iter().collect();
        }
        for crate_dir in sorted_entries(dir)?.into_iter().filter(|p| p.is_dir()) {
            let krate = crate_dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            for file in sorted_entries(&crate_dir)?.into_iter().filter(|p| p.extension().is_some_and(|e| e == "toml")) {
                let shown = relative(root, &file);
                for raw in read_toml::<RawFile>(&file)?.exception {
                    found.push(raw, &shown, &krate, &mut findings);
                }
            }
        }
        Ok((found, findings))
    }

    fn push(&mut self, raw: RawEntry, file: &Path, krate: &str, findings: &mut Vec<Finding>) {
        let mut bad = |message: String| findings.push(finding(INVALID, file.to_owned(), format!("`{}`: {message}", raw.name)));
        let reason = raw.reason.as_deref().map(str::trim).unwrap_or_default().to_owned();
        if reason.chars().count() < MIN_REASON {
            bad(format!("needs a `reason` of at least {MIN_REASON} characters"));
        }
        if !KINDS.contains(&raw.kind.as_str()) {
            bad(format!("unknown kind `{}`", raw.kind));
        }
        if EXPIRING_KINDS.contains(&raw.kind.as_str()) && raw.expires.is_none() {
            bad(format!("a `{}` entry names the milestone that removes it in `expires`", raw.kind));
        }
        if raw.count == Some(0) {
            bad("`count` is at least 1; remove the entry instead".to_owned());
        }
        if self.entries.iter().any(|e| e.name == raw.name) {
            bad("name is already used".to_owned());
        }
        self.entries.push(Entry {
            name: raw.name.clone(),
            kind: raw.kind.clone(),
            rule: raw.rule.clone(),
            path: raw.path.clone(),
            reason,
            count: raw.count.unwrap_or(1),
            expires: raw.expires.clone(),
            file: file.to_owned(),
            krate: krate.to_owned(),
        });
    }

    /// The listed count per rule, summed over all files: the ratchet total.
    pub fn total(&self, rule: &str) -> usize {
        self.entries.iter().filter(|e| rule_covers(&e.rule, rule)).map(|e| e.count).sum()
    }

    /// Matches `sites` against the entries and returns the findings.
    ///
    /// Each site goes to the most specific entry that covers it (longest path, then exact rule). A site
    /// without an entry, or beyond the entry's count, is a finding; an entry with fewer sites than its count
    /// is `exception/stale`; an entry whose milestone landed is `exception/expired`.
    pub fn apply(&self, sites: &[Site]) -> Vec<Finding> {
        let mut used = vec![0_usize; self.entries.len()];
        let mut findings = Vec::new();
        for site in sites {
            if site.rule == INVALID {
                findings.push(invalid_site(site));
                continue;
            }
            match self.best_entry(site) {
                Some(i) if used[i] < self.entries[i].count => used[i] += 1,
                Some(i) => findings.push(uncovered(site, Some(&self.entries[i]))),
                None => findings.push(uncovered(site, None)),
            }
        }
        for (entry, &n) in self.entries.iter().zip(&used) {
            if entry.expires.as_ref().is_some_and(|m| self.done.contains(m)) {
                let message = format!(
                    "`{}` expired with milestone `{}`: remove the entry and the code it excuses",
                    entry.name,
                    entry.expires.as_deref().unwrap_or_default()
                );
                findings.push(finding(EXPIRED, entry.file.clone(), message));
            } else if n < entry.count {
                let advice = if n == 0 { "remove the entry".to_owned() } else { format!("lower `count` to {n}") };
                let message = format!("`{}` lists {} site(s) of `{}` but {n} exist: {advice}", entry.name, entry.count, entry.rule);
                findings.push(finding(STALE, entry.file.clone(), message));
            }
        }
        findings
    }

    fn best_entry(&self, site: &Site) -> Option<usize> {
        let covers = |e: &Entry| rule_covers(&e.rule, &site.rule) && e.path.as_deref().is_none_or(|p| subject_covers(p, &site.subject));
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| covers(e))
            .max_by_key(|(_, e)| (e.path.as_ref().map_or(0, |p| p.len() + 1), e.rule == site.rule))
            .map(|(i, _)| i)
    }
}

/// A site that is itself malformed; no entry can excuse it.
fn invalid_site(site: &Site) -> Finding {
    let message = format!("`exempt(..)` needs a `reason` of at least {MIN_REASON} characters; the exemption is counted in `design/exceptions`");
    let mut f = Finding::new(INVALID, site.line, 1, site.line, message);
    f.path = PathBuf::from(&site.subject);
    f.area = "(exceptions)".to_owned();
    f
}

fn uncovered(site: &Site, full: Option<&Entry>) -> Finding {
    let (rule, message) = match (site.origin, full) {
        (Origin::Violation(rule), None) => (rule, format!("`{}` breaks `{rule}`; fix it or list an exception", site.subject)),
        (Origin::Violation(rule), Some(e)) => {
            (rule, format!("`{}` breaks `{rule}` beyond the {} site(s) of exception `{}`", site.subject, e.count, e.name))
        }
        (Origin::Allowance, None) => (UNLISTED, format!("`{}` is allowed here but no exception entry lists it", site.rule)),
        (Origin::Allowance, Some(e)) => (UNLISTED, format!("`{}` has more sites than the {} that exception `{}` lists", site.rule, e.count, e.name)),
    };
    let mut f = Finding::new(rule, site.line, 1, site.line, message);
    f.path = PathBuf::from(&site.subject);
    f.area = "(exceptions)".to_owned();
    f
}

/// Allowance sites in one source file: `// design: allow[..]` markers and `allow`/`expect` attributes
/// that name a `clippy::disallowed_*` lint. `shown` is the path the sites report.
pub fn scan_sites(shown: &str, src: &str) -> Vec<Site> {
    scan_parsed(shown, src, syn::parse_file(src).ok().as_ref())
}

/// [`scan_sites`] with `src` already parsed into `file` (`None` when it does not parse).
pub(crate) fn scan_parsed(shown: &str, src: &str, file: Option<&syn::File>) -> Vec<Site> {
    let mut sites = marker_sites(shown, src);
    if let Some(file) = file {
        let mut scan = AttrScan { shown, sites: Vec::new() };
        scan.visit_file(file);
        sites.append(&mut scan.sites);
    }
    sites
}

/// One site per rule a marker names, or one for `*` when it names none.
fn marker_sites(shown: &str, src: &str) -> Vec<Site> {
    const PREFIX: &str = "design: allow";
    let mut sites = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let Some(comment) = line.find("//").map(|i| &line[i..]) else { continue };
        let Some(at) = comment.find(PREFIX) else { continue };
        let rest = &comment[at + PREFIX.len()..];
        let rules: Vec<String> = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            Some((list, _)) => list.split(',').map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()).collect(),
            None => vec!["*".to_owned()],
        };
        for rule in rules {
            sites.push(Site { rule, subject: shown.to_owned(), line: n + 1, origin: Origin::Allowance });
        }
    }
    sites
}

/// The `exempt(case = .., reason = ..)` options among `tokens`, as `(case, reason)`; a missing part is empty.
fn exempt_options(tokens: TokenStream) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut previous_exempt = false;
    for tt in tokens {
        match tt {
            TokenTree::Ident(i) => previous_exempt = i == "exempt",
            TokenTree::Group(g) if previous_exempt => {
                previous_exempt = false;
                let (mut case, mut reason) = (String::new(), String::new());
                let mut key = String::new();
                let mut after_equals = false;
                for part in g.stream() {
                    match &part {
                        TokenTree::Ident(i) if !after_equals => key = i.to_string(),
                        TokenTree::Punct(p) if p.as_char() == '=' => after_equals = true,
                        TokenTree::Punct(p) if p.as_char() == ',' => after_equals = false,
                        TokenTree::Ident(_) | TokenTree::Literal(_) if after_equals => {
                            let text = match &part {
                                TokenTree::Literal(l) => syn::parse_str::<syn::LitStr>(&l.to_string()).map(|l| l.value()).unwrap_or_default(),
                                other => other.to_string(),
                            };
                            match key.as_str() {
                                "case" => case = text,
                                "reason" => reason = text,
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                found.push((case, reason));
            }
            TokenTree::Group(g) => found.extend(exempt_options(g.stream())),
            _ => previous_exempt = false,
        }
    }
    found
}

struct AttrScan<'a> {
    shown: &'a str,
    sites: Vec<Site>,
}

impl AttrScan<'_> {
    /// One site per `exempt(..)` among `tokens`; an exemption without a reason is a malformed site.
    fn exemptions(&mut self, tokens: TokenStream, line: usize) {
        for (case, reason) in exempt_options(tokens) {
            let case = if case.is_empty() { "unnamed" } else { case.as_str() };
            let origin = Origin::Allowance;
            self.sites.push(Site { rule: format!("{PROBE_EXEMPT}/{case}"), subject: self.shown.to_owned(), line, origin });
            if reason.trim().chars().count() < MIN_REASON {
                self.sites.push(Site { rule: INVALID.to_owned(), subject: self.shown.to_owned(), line, origin: Origin::Violation(INVALID) });
            }
        }
    }
}

impl<'ast> Visit<'ast> for AttrScan<'_> {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item.mac.path.segments.last().is_some_and(|s| s.ident == "register_contract") {
            self.exemptions(item.mac.tokens.clone(), syn::spanned::Spanned::span(item).start().line);
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        let name = attr.path();
        if name.is_ident("component")
            && let syn::Meta::List(list) = &attr.meta
        {
            self.exemptions(list.tokens.clone(), syn::spanned::Spanned::span(attr).start().line);
        }
        if (name.is_ident("expect") || name.is_ident("allow"))
            && let syn::Meta::List(list) = &attr.meta
        {
            for lint in disallowed_lints(list.tokens.clone()) {
                let line = syn::spanned::Spanned::span(attr).start().line;
                self.sites.push(Site { rule: format!("clippy::{lint}"), subject: self.shown.to_owned(), line, origin: Origin::Allowance });
            }
        }
        visit::visit_attribute(self, attr);
    }
}

/// The `disallowed_*` lint names in `clippy::disallowed_methods`-style paths of an attribute's arguments.
fn disallowed_lints(tokens: TokenStream) -> Vec<String> {
    let mut out = Vec::new();
    let mut previous_is_clippy = false;
    for tt in tokens {
        match tt {
            TokenTree::Ident(i) if i == "clippy" => previous_is_clippy = true,
            TokenTree::Ident(i) if previous_is_clippy && i.to_string().starts_with("disallowed_") => {
                out.push(i.to_string());
                previous_is_clippy = false;
            }
            TokenTree::Punct(_) => {}
            _ => previous_is_clippy = false,
        }
    }
    out
}
