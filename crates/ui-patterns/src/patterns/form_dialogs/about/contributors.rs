//! The Contributors tab's rules: which name shows, what the table sorts by, the order of the rows and how a
//! contributor's figures are written. The ordering and the shares are pure.

use super::AboutWords;
use super::credits::{Contributor, ModelCredit};
use crate::contract::ColumnSort;
use crate::table::sort_rows;
use craft_behavior::ColumnKey;
use craft_i18n::{CalendarDate, DateFormat, DateStyle, DateTime, Format};
use std::borrow::Cow;
use std::cmp::Ordering;

/// Which name the Contributors tab shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameMode {
    /// `@login`.
    #[default]
    Username,
    /// The consented display name, else `@login`.
    DisplayName,
    /// The consented real name, else `@login`.
    RealName,
}

impl Contributor {
    /// The name `mode` asks for, or `@login` when there is none.
    pub fn name(&self, mode: NameMode) -> Cow<'_, str> {
        let chosen = match mode {
            NameMode::Username => None,
            NameMode::DisplayName => self.display_name.as_ref(),
            NameMode::RealName => self.real_name.as_ref(),
        };
        chosen.map_or_else(|| Cow::Owned(format!("@{}", self.login)), |name| Cow::Borrowed(name.as_ref()))
    }
}

/// What the Contributors tab sorts by. Names and dates start with the first (A first, oldest first) and counts with
/// the biggest ([`default_ascending`](ContributorSort::default_ascending)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContributorSort {
    /// The name [`NameMode`] selects.
    Name,
    /// Merged pull requests.
    Prs,
    /// Commits.
    Commits,
    /// Lines added.
    LinesAdded,
    /// Lines deleted.
    LinesDeleted,
    /// Lines added minus lines deleted.
    LinesDelta,
    /// Binary assets added.
    BinaryAdded,
    /// Binary assets deleted.
    BinaryDeleted,
    /// The date of the first commit.
    #[default]
    FirstCommit,
    /// The date of the latest commit.
    LastCommit,
}

const NAME: ColumnKey = ColumnKey::new("name");
const PRS: ColumnKey = ColumnKey::new("prs");
const COMMITS: ColumnKey = ColumnKey::new("commits");
const LINES_ADDED: ColumnKey = ColumnKey::new("linesAdded");
const LINES_DELETED: ColumnKey = ColumnKey::new("linesDeleted");
const LINES_DELTA: ColumnKey = ColumnKey::new("linesDelta");
const BINARY_ADDED: ColumnKey = ColumnKey::new("binaryAdded");
const BINARY_DELETED: ColumnKey = ColumnKey::new("binaryDeleted");
const FIRST_COMMIT: ColumnKey = ColumnKey::new("firstCommit");
const LAST_COMMIT: ColumnKey = ColumnKey::new("lastCommit");

impl ContributorSort {
    /// Every key in the order the sort list and the table columns show them.
    pub const ALL: [ContributorSort; 10] = [
        ContributorSort::Name,
        ContributorSort::Prs,
        ContributorSort::Commits,
        ContributorSort::LinesAdded,
        ContributorSort::LinesDeleted,
        ContributorSort::LinesDelta,
        ContributorSort::BinaryAdded,
        ContributorSort::BinaryDeleted,
        ContributorSort::FirstCommit,
        ContributorSort::LastCommit,
    ];

    /// Whether this key sorts ascending when first chosen: names and dates do, counts do not.
    pub fn default_ascending(self) -> bool {
        matches!(self, ContributorSort::Name | ContributorSort::FirstCommit | ContributorSort::LastCommit)
    }

    /// The table column that sorts by this key.
    pub(super) fn column(self) -> ColumnKey {
        match self {
            ContributorSort::Name => NAME,
            ContributorSort::Prs => PRS,
            ContributorSort::Commits => COMMITS,
            ContributorSort::LinesAdded => LINES_ADDED,
            ContributorSort::LinesDeleted => LINES_DELETED,
            ContributorSort::LinesDelta => LINES_DELTA,
            ContributorSort::BinaryAdded => BINARY_ADDED,
            ContributorSort::BinaryDeleted => BINARY_DELETED,
            ContributorSort::FirstCommit => FIRST_COMMIT,
            ContributorSort::LastCommit => LAST_COMMIT,
        }
    }

    /// The key a table column sorts by.
    pub(super) fn of_column(column: &ColumnKey) -> Option<ContributorSort> {
        ContributorSort::ALL.into_iter().find(|key| key.column() == *column)
    }

    /// The sort list's entry and the table's column head.
    pub(super) fn labels(self, words: &AboutWords) -> (&crate::runtime::env::Label, &crate::runtime::env::Label) {
        match self {
            ContributorSort::Name => (&words.sort_name, &words.head_name),
            ContributorSort::Prs => (&words.sort_prs, &words.head_prs),
            ContributorSort::Commits => (&words.sort_commits, &words.commits),
            ContributorSort::LinesAdded => (&words.sort_lines_added, &words.head_lines_added),
            ContributorSort::LinesDeleted => (&words.sort_lines_deleted, &words.head_lines_deleted),
            ContributorSort::LinesDelta => (&words.sort_line_delta, &words.head_line_delta),
            ContributorSort::BinaryAdded => (&words.sort_binary_added, &words.head_binary_added),
            ContributorSort::BinaryDeleted => (&words.sort_binary_deleted, &words.head_binary_deleted),
            ContributorSort::FirstCommit => (&words.sort_first, &words.head_first),
            ContributorSort::LastCommit => (&words.sort_last, &words.head_last),
        }
    }

    /// Counts line up at the reading end of their column; names and dates start it.
    pub(super) fn numeric(self) -> bool {
        !self.default_ascending()
    }
}

/// Alphabetical key: case-insensitive, ignoring the `@` of usernames.
fn name_key(name: &str) -> String {
    name.trim_start_matches('@').to_lowercase()
}

fn compare(a: &Contributor, b: &Contributor, mode: NameMode, sort: ContributorSort) -> Ordering {
    // Someone credited only through a merged pull request has no date and sorts after everyone with one.
    let dated = |date: &str| (date.is_empty(), date.to_owned());
    match sort {
        ContributorSort::Name => name_key(&a.name(mode)).cmp(&name_key(&b.name(mode))),
        ContributorSort::Prs => a.prs.cmp(&b.prs),
        ContributorSort::Commits => a.commits.cmp(&b.commits),
        ContributorSort::LinesAdded => a.lines_added.cmp(&b.lines_added),
        ContributorSort::LinesDeleted => a.lines_deleted.cmp(&b.lines_deleted),
        ContributorSort::LinesDelta => a.lines_delta().cmp(&b.lines_delta()),
        ContributorSort::BinaryAdded => a.binary_added.cmp(&b.binary_added),
        ContributorSort::BinaryDeleted => a.binary_deleted.cmp(&b.binary_deleted),
        ContributorSort::FirstCommit => dated(&a.first_commit).cmp(&dated(&b.first_commit)),
        ContributorSort::LastCommit => dated(&a.last_commit).cmp(&dated(&b.last_commit)),
    }
}

/// The positions of `list` in display order, which the table keys its rows by. Names compare case-insensitively without the `@`; someone with no commit date sorts after
/// everyone with one; ties fall back to the name, then the login, in either direction, so the order is stable.
pub(super) fn sorted_indices(list: &[Contributor], mode: NameMode, sort: ContributorSort, ascending: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..list.len()).collect();
    order.sort_by(|&a, &b| compare(&list[a], &list[b], mode, ContributorSort::Name).then_with(|| list[a].login.cmp(&list[b].login)));
    let by = if ascending { ColumnSort::ascending(sort.column()) } else { ColumnSort::descending(sort.column()) };
    sort_rows(&mut order, Some(&by), |&a, &b, column| {
        ContributorSort::of_column(column).map_or(Ordering::Equal, |key| compare(&list[a], &list[b], mode, key))
    });
    order
}

/// The date of an ISO 8601 timestamp in the region's date style, "—" for none; a timestamp that does not parse shows
/// its first ten characters.
pub(crate) fn commit_day(dates: &DateFormat, iso: &str) -> String {
    if iso.is_empty() {
        return "—".to_owned();
    }
    match iso_date(iso) {
        Some(date) => dates.format(DateTime { date, hour: 0, minute: 0, second: 0 }, DateStyle::DATE),
        None => iso.get(..10).unwrap_or(iso).to_owned(),
    }
}

/// The calendar date of the `YYYY-MM-DD` that starts an ISO 8601 timestamp.
fn iso_date(iso: &str) -> Option<CalendarDate> {
    let mut parts = iso.get(..10)?.split('-');
    let (year, month, day) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    CalendarDate::new(year, month, day)
}

/// A model's share of all commits.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CommitShare {
    /// Above zero but under half a percent.
    UnderOne,
    /// A ratio from 0 to 1, written in whole percent.
    Ratio(f64),
}

/// A model's share of all commits; a model with any commits never reads 0 %.
pub(crate) fn commit_share(commits: u64, total: u64) -> CommitShare {
    let ratio = commits as f64 / total.max(1) as f64;
    if commits > 0 && ratio < 0.005 { CommitShare::UnderOne } else { CommitShare::Ratio(ratio.min(1.0)) }
}

/// How the tables write figures: grouped numbers, dates and signed counts in the scope's language.
pub(super) struct Writer<'a> {
    pub(super) format: &'a Format,
    pub(super) dates: &'a DateFormat,
    pub(super) words: &'a AboutWords,
}

impl Writer<'_> {
    /// A grouped count.
    pub(super) fn count(&self, n: u64) -> String {
        self.format.integer(n)
    }

    /// `+1,234` or `−1,234`.
    fn signed(&self, delta: i128) -> String {
        let magnitude = self.count(u64::try_from(delta.unsigned_abs()).unwrap_or(u64::MAX));
        let template = if delta < 0 { &self.words.minus } else { &self.words.plus };
        template.fill(&[("count", &magnitude)])
    }

    /// The cell of the column that sorts by `key`.
    pub(super) fn cell(&self, key: ContributorSort, c: &Contributor) -> String {
        match key {
            ContributorSort::Name => c.login.to_string(),
            ContributorSort::Prs => self.count(c.prs),
            ContributorSort::Commits => self.count(c.commits),
            ContributorSort::LinesAdded => self.count(c.lines_added),
            ContributorSort::LinesDeleted => self.count(c.lines_deleted),
            ContributorSort::LinesDelta => self.signed(c.lines_delta()),
            ContributorSort::BinaryAdded => self.count(c.binary_added),
            ContributorSort::BinaryDeleted => self.count(c.binary_deleted),
            ContributorSort::FirstCommit => commit_day(self.dates, &c.first_commit),
            ContributorSort::LastCommit => commit_day(self.dates, &c.last_commit),
        }
    }

    /// A contributor's tooltip: the counted pull requests and commits, the lines and the active dates.
    pub(super) fn summary(&self, c: &Contributor) -> String {
        let words = self.words;
        let prs = words.prs.text(c.prs, &self.count(c.prs));
        let commits = words.commit_count.text(c.commits, &self.count(c.commits));
        words.summary.fill(&[
            ("login", &c.login),
            ("prs", &prs),
            ("commits", &commits),
            ("added", &self.count(c.lines_added)),
            ("deleted", &self.count(c.lines_deleted)),
            ("delta", &self.signed(c.lines_delta())),
            ("binAdded", &self.count(c.binary_added)),
            ("binDeleted", &self.count(c.binary_deleted)),
            ("first", &commit_day(self.dates, &c.first_commit)),
            ("last", &commit_day(self.dates, &c.last_commit)),
        ])
    }

    /// A model's share cell.
    pub(super) fn share(&self, commits: u64, total: u64) -> String {
        match commit_share(commits, total) {
            CommitShare::UnderOne => self.words.under_one.fill(&[("percent", &self.format.percent(0.01, 0))]),
            CommitShare::Ratio(ratio) => self.format.percent(ratio, 0),
        }
    }

    /// A model's lines cell.
    pub(super) fn lines(&self, model: &ModelCredit) -> String {
        self.words.lines_range.fill(&[("added", &self.count(model.lines_added)), ("deleted", &self.count(model.lines_deleted))])
    }
}
