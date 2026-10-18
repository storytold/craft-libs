#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Contributor and AI-model credits for a Craft app's About window (craftrules
//! `standards/contributors.md`).
//!
//! Every app credits everyone who contributed (GitHub login always; display and real names only from
//! the person's own `contributors/people.toml` entry) and the AI models named in `Co-Authored-By`
//! trailers. The tables are baked into the binary at build time, so nothing is read at run time.
//!
//! - In the app's `build.rs` (feature `build`): [`build::write_credits`] reads
//!   `contributors/contributors.json` and `contributors/people.toml` and writes `credits.rs` into
//!   `OUT_DIR`, with `TOTAL_COMMITS`, `CONTRIBUTORS` and `MODELS`.
//! - In the app: `include!(concat!(env!("OUT_DIR"), "/credits.rs"));`, then [`sorted`],
//!   [`Contributor::name`], [`Contributor::summary`] and the [`fmt`] helpers to draw the list.
//!
//! The About window itself (layout, translation, links, automation ids) stays in each app. Every
//! English string this crate hands out is in [`UI_STRINGS`], so an app's catalog test can check
//! that each one is translated.

use std::cmp::Ordering;

#[cfg(feature = "build")]
pub mod build;

/// One contributor, as baked by [`build::write_credits`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributor {
    pub login: &'static str,
    /// From the person's verified people.toml entry only.
    pub display_name: Option<&'static str>,
    /// From the person's verified people.toml entry only.
    pub real_name: Option<&'static str>,
    pub prs: u64,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub binary_added: u64,
    pub binary_deleted: u64,
    /// ISO 8601 UTC; empty for someone credited only through a merged PR.
    pub first_commit: &'static str,
    pub last_commit: &'static str,
}

/// An AI model credited in `Co-Authored-By` trailers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub company: &'static str,
    pub model: &'static str,
    pub version: &'static str,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

/// Which name to show. Display and real names fall back to the `@username` when not given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NameMode {
    #[default]
    Username,
    DisplayName,
    RealName,
}

impl NameMode {
    pub const ALL: [NameMode; 3] = [NameMode::Username, NameMode::DisplayName, NameMode::RealName];

    /// English label (in [`UI_STRINGS`]).
    pub fn label(self) -> &'static str {
        match self {
            NameMode::Username => "Username",
            NameMode::DisplayName => "Display name",
            NameMode::RealName => "Real name",
        }
    }

    /// A stable camelCase id, for automation ids or saved settings.
    pub fn id(self) -> &'static str {
        match self {
            NameMode::Username => "username",
            NameMode::DisplayName => "displayName",
            NameMode::RealName => "realName",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortKey {
    Name,
    Prs,
    Commits,
    LinesAdded,
    LinesDeleted,
    LinesDelta,
    BinaryAdded,
    BinaryDeleted,
    #[default]
    FirstCommit,
    LastCommit,
}

impl SortKey {
    pub const ALL: [SortKey; 10] = [
        SortKey::Name,
        SortKey::Prs,
        SortKey::Commits,
        SortKey::LinesAdded,
        SortKey::LinesDeleted,
        SortKey::LinesDelta,
        SortKey::BinaryAdded,
        SortKey::BinaryDeleted,
        SortKey::FirstCommit,
        SortKey::LastCommit,
    ];

    /// English menu label and table header. The headers that are symbols (`PRs`, `+LOC`, `−LOC`,
    /// `ΔLOC`, `+Bin`, `−Bin`) are usually shown as they are; the others are in [`UI_STRINGS`].
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            SortKey::Name => ("Name (A–Z)", "Name"),
            SortKey::Prs => ("Merged PRs", "PRs"),
            SortKey::Commits => ("Commits", "Commits"),
            SortKey::LinesAdded => ("Lines added", "+LOC"),
            SortKey::LinesDeleted => ("Lines deleted", "−LOC"),
            SortKey::LinesDelta => ("Line delta", "ΔLOC"),
            SortKey::BinaryAdded => ("Binary assets added", "+Bin"),
            SortKey::BinaryDeleted => ("Binary assets removed", "−Bin"),
            SortKey::FirstCommit => ("First commit", "First"),
            SortKey::LastCommit => ("Last commit", "Last"),
        }
    }

    /// A stable camelCase id, for automation ids or saved settings.
    pub fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Prs => "prs",
            SortKey::Commits => "commits",
            SortKey::LinesAdded => "linesAdded",
            SortKey::LinesDeleted => "linesDeleted",
            SortKey::LinesDelta => "linesDelta",
            SortKey::BinaryAdded => "binaryAdded",
            SortKey::BinaryDeleted => "binaryDeleted",
            SortKey::FirstCommit => "firstCommit",
            SortKey::LastCommit => "lastCommit",
        }
    }

    /// Names and dates read naturally oldest/A first; counts read biggest first.
    pub fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::FirstCommit | SortKey::LastCommit)
    }

    /// Whether the column holds counts (usually aligned right).
    pub fn numeric(self) -> bool {
        !self.default_ascending()
    }

    /// The table cell for `c`. The name column gives the login; show [`Contributor::name`] there.
    pub fn cell(self, c: &Contributor) -> String {
        match self {
            SortKey::Name => c.login.to_string(),
            SortKey::Prs => fmt::group(c.prs),
            SortKey::Commits => fmt::group(c.commits),
            SortKey::LinesAdded => fmt::group(c.lines_added),
            SortKey::LinesDeleted => fmt::group(c.lines_deleted),
            SortKey::LinesDelta => fmt::signed(c.lines_delta()),
            SortKey::BinaryAdded => fmt::group(c.binary_added),
            SortKey::BinaryDeleted => fmt::group(c.binary_deleted),
            SortKey::FirstCommit => fmt::day(c.first_commit).to_string(),
            SortKey::LastCommit => fmt::day(c.last_commit).to_string(),
        }
    }
}

/// English template of [`Contributor::summary`] (in [`UI_STRINGS`]). Translate it, then fill it with
/// [`Contributor::summary_in`].
pub const SUMMARY: &str =
    "@{login}: {prs} PRs, {commits} commits, +{added} / −{deleted} lines (Δ {delta}), +{binAdded} / −{binDeleted} binary assets, {first} – {last}";

/// Every English string this crate hands out for display: the [`NameMode`] and [`SortKey`] labels
/// (the word headers, not the symbols) and [`SUMMARY`].
pub const UI_STRINGS: &[&str] = &[
    "Username",
    "Display name",
    "Real name",
    "Name (A–Z)",
    "Merged PRs",
    "Commits",
    "Lines added",
    "Lines deleted",
    "Line delta",
    "Binary assets added",
    "Binary assets removed",
    "First commit",
    "Last commit",
    "Name",
    "First",
    "Last",
    SUMMARY,
];

impl Contributor {
    pub fn name(&self, mode: NameMode) -> String {
        let chosen = match mode {
            NameMode::Username => None,
            NameMode::DisplayName => self.display_name,
            NameMode::RealName => self.real_name,
        };
        chosen.map_or_else(|| format!("@{}", self.login), str::to_string)
    }

    pub fn lines_delta(&self) -> i128 {
        i128::from(self.lines_added) - i128::from(self.lines_deleted)
    }

    /// The values [`SUMMARY`]'s placeholders stand for, formatted.
    pub fn summary_fields(&self) -> [(&'static str, String); 10] {
        [
            ("login", self.login.to_string()),
            ("prs", fmt::group(self.prs)),
            ("commits", fmt::group(self.commits)),
            ("added", fmt::group(self.lines_added)),
            ("deleted", fmt::group(self.lines_deleted)),
            ("delta", fmt::signed(self.lines_delta())),
            ("binAdded", fmt::group(self.binary_added)),
            ("binDeleted", fmt::group(self.binary_deleted)),
            ("first", fmt::day(self.first_commit).to_string()),
            ("last", fmt::day(self.last_commit).to_string()),
        ]
    }

    /// One line with everything we know, for tooltips, in English.
    pub fn summary(&self) -> String {
        self.summary_in(SUMMARY)
    }

    /// [`Self::summary`] from a (translated) `template` with [`SUMMARY`]'s `{placeholders}`. Unknown
    /// placeholders are left as they are.
    pub fn summary_in(&self, template: &str) -> String {
        fmt::fill(template, &self.summary_fields())
    }
}

/// Alphabetical key: case-insensitive, ignoring the `@` of usernames.
fn name_key(name: &str) -> String {
    name.trim_start_matches('@').to_lowercase()
}

fn compare(a: &Contributor, b: &Contributor, mode: NameMode, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => name_key(&a.name(mode)).cmp(&name_key(&b.name(mode))),
        SortKey::Prs => a.prs.cmp(&b.prs),
        SortKey::Commits => a.commits.cmp(&b.commits),
        SortKey::LinesAdded => a.lines_added.cmp(&b.lines_added),
        SortKey::LinesDeleted => a.lines_deleted.cmp(&b.lines_deleted),
        SortKey::LinesDelta => a.lines_delta().cmp(&b.lines_delta()),
        SortKey::BinaryAdded => a.binary_added.cmp(&b.binary_added),
        SortKey::BinaryDeleted => a.binary_deleted.cmp(&b.binary_deleted),
        SortKey::FirstCommit => a.first_commit.cmp(b.first_commit),
        SortKey::LastCommit => a.last_commit.cmp(b.last_commit),
    }
}

/// Whether `c` has no date for `key` (someone credited only through a merged PR).
fn undated(c: &Contributor, key: SortKey) -> bool {
    match key {
        SortKey::FirstCommit => c.first_commit.is_empty(),
        SortKey::LastCommit => c.last_commit.is_empty(),
        _ => false,
    }
}

/// Contributors in display order. Ties fall back to the name (A–Z) so the order is stable. In a
/// date sort, contributors without that date come after everyone with one, in both directions.
pub fn sorted(list: &[Contributor], mode: NameMode, key: SortKey, ascending: bool) -> Vec<&Contributor> {
    let mut v: Vec<&Contributor> = list.iter().collect();
    v.sort_by(|a, b| {
        let o = compare(a, b, mode, key);
        let o = if ascending { o } else { o.reverse() };
        undated(a, key).cmp(&undated(b, key)).then(o).then_with(|| compare(a, b, mode, SortKey::Name)).then_with(|| a.login.cmp(b.login))
    });
    v
}

/// A model's share of all commits, in percent (0–100). `total_commits` is the repository's total.
pub fn model_share(m: &Model, models: &[Model], total_commits: u64) -> f64 {
    let assisted = models.iter().map(|m| m.commits).max().unwrap_or(0).max(1);
    100.0 * m.commits as f64 / total_commits.max(assisted) as f64
}

/// Number and date formatting for the credits.
pub mod fmt {
    /// `1234567` → `1,234,567`.
    pub fn group(n: u64) -> String {
        let s = n.to_string();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, ch) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(ch);
        }
        out
    }

    /// A grouped number with its sign: `+1,000`, `−1,000` (a minus sign, not a hyphen).
    pub fn signed(n: i128) -> String {
        let g = group(u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX));
        if n < 0 { format!("−{g}") } else { format!("+{g}") }
    }

    /// The date of an ISO 8601 timestamp, or `—` for none.
    pub fn day(iso: &str) -> &str {
        if iso.is_empty() { "—" } else { iso.get(..10).unwrap_or(iso) }
    }

    /// `template` with each `{key}` of `fields` replaced by its value, in one pass (a value that
    /// contains `{…}` is not filled again). Unknown placeholders are left as they are.
    pub fn fill(template: &str, fields: &[(&str, String)]) -> String {
        let mut out = String::with_capacity(template.len() + 32);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after.find('}').and_then(|close| fields.iter().find(|(k, _)| *k == &after[..close]).map(|(_, v)| (close, v))) {
                Some((close, v)) => {
                    out.push_str(v);
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(
        login: &'static str,
        display: Option<&'static str>,
        real: Option<&'static str>,
        added: u64,
        deleted: u64,
        first: &'static str,
    ) -> Contributor {
        Contributor {
            login,
            display_name: display,
            real_name: real,
            prs: 0,
            commits: 1,
            lines_added: added,
            lines_deleted: deleted,
            binary_added: 0,
            binary_deleted: 0,
            first_commit: first,
            last_commit: first,
        }
    }

    fn people() -> [Contributor; 4] {
        [
            c("zed", None, None, 5, 50, "2026-01-02T00:00:00Z"),
            c("Bob", Some("Ann"), None, 10, 0, "2026-01-01T00:00:00Z"),
            c("alice", None, None, 7, 1, ""),
            c("carol", None, Some("Dana"), 1, 0, "2026-01-03T00:00:00Z"),
        ]
    }

    fn logins(v: &[&Contributor]) -> Vec<&'static str> {
        v.iter().map(|c| c.login).collect()
    }

    #[test]
    fn names_fall_back_to_the_username() {
        let p = c("bob", Some("Bobby"), None, 0, 0, "");
        assert_eq!(p.name(NameMode::Username), "@bob");
        assert_eq!(p.name(NameMode::DisplayName), "Bobby");
        assert_eq!(p.name(NameMode::RealName), "@bob");
    }

    #[test]
    fn alphabetical_ignores_at_and_case() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]);
        let v = sorted(&all, NameMode::DisplayName, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]); // "Ann" sorts with the @usernames
        let v = sorted(&all, NameMode::RealName, SortKey::Name, false);
        assert_eq!(logins(&v), ["zed", "carol", "Bob", "alice"]); // "Dana" between bob and zed
    }

    #[test]
    fn delta_and_dates() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::LinesDelta, false);
        assert_eq!(logins(&v), ["Bob", "alice", "carol", "zed"]);
        let v = sorted(&all, NameMode::Username, SortKey::FirstCommit, true);
        assert_eq!(logins(&v), ["Bob", "zed", "carol", "alice"]); // undated last
        // Newest first: the undated one is still last, not first.
        let v = sorted(&all, NameMode::Username, SortKey::FirstCommit, false);
        assert_eq!(logins(&v), ["carol", "zed", "Bob", "alice"]);
        let v = sorted(&all, NameMode::Username, SortKey::LastCommit, false);
        assert_eq!(logins(&v), ["carol", "zed", "Bob", "alice"]);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt::group(1234567), "1,234,567");
        assert_eq!(fmt::group(12), "12");
        assert_eq!(fmt::signed(-1000), "−1,000");
        assert_eq!(fmt::signed(i128::MIN), format!("−{}", fmt::group(u64::MAX)));
        assert_eq!(fmt::day("2026-10-07T18:45:23Z"), "2026-10-07");
        assert_eq!(fmt::day(""), "—");
    }

    #[test]
    fn summary_fills_every_placeholder_once() {
        let mut p = c("ada", None, None, 1500, 20, "2026-10-01T00:00:00Z");
        p.prs = 3;
        assert_eq!(p.summary(), "@ada: 3 PRs, 1 commits, +1,500 / −20 lines (Δ +1,480), +0 / −0 binary assets, 2026-10-01 – 2026-10-01");
        // A translated template, its own order and an unknown placeholder; a `{…}` in a value is not filled again.
        let p = c("{prs}", None, None, 0, 0, "");
        assert_eq!(p.summary_in("{commits} × {login} {nope} {"), "1 × {prs} {nope} {");
    }

    #[test]
    fn every_label_is_listed() {
        for m in NameMode::ALL {
            assert!(UI_STRINGS.contains(&m.label()), "{m:?}");
        }
        for k in SortKey::ALL {
            let (menu, header) = k.label();
            assert!(UI_STRINGS.contains(&menu), "{k:?}");
            let symbol = header.starts_with(['+', '−', 'Δ']) || header == "PRs";
            assert_eq!(UI_STRINGS.contains(&header), !symbol, "{k:?} header {header}");
        }
    }

    #[test]
    fn model_share_uses_the_larger_total() {
        let m = |commits| Model { company: "A", model: "M", version: "1", commits, lines_added: 0, lines_deleted: 0 };
        let models = [m(50), m(10)];
        assert_eq!(model_share(&models[0], &models, 100), 50.0);
        // A stale total smaller than a model's own count can't give more than 100 %.
        assert_eq!(model_share(&models[0], &models, 20), 100.0);
        assert_eq!(model_share(&m(0), &[], 0), 0.0);
    }

    #[test]
    fn ids_are_unique() {
        let ids: std::collections::BTreeSet<_> = SortKey::ALL.iter().map(|k| k.id()).collect();
        assert_eq!(ids.len(), SortKey::ALL.len());
        let ids: std::collections::BTreeSet<_> = NameMode::ALL.iter().map(|m| m.id()).collect();
        assert_eq!(ids.len(), NameMode::ALL.len());
    }
}
