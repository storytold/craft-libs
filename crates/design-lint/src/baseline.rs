//! A baseline of finding counts per file and rule, so an existing code base can gate new findings only.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::Error;
use crate::report::{Finding, Report, Severity};
use crate::rules::PARSE;

/// Error findings counted per file and rule.
///
/// `parse` findings are never part of a baseline: a file that does not parse always fails the check.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Baseline {
    counts: BTreeMap<String, BTreeMap<String, usize>>,
}

/// A file and rule whose finding count is above the baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Increase {
    /// The file.
    pub path: PathBuf,
    /// The rule id.
    pub rule: String,
    /// The count in the baseline; 0 for a file or rule it does not list.
    pub baseline: usize,
    /// The count in the current report.
    pub now: usize,
}

/// The result of comparing a [`Report`] with a [`Baseline`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BaselineCheck {
    /// Every file and rule whose count rose above the baseline.
    pub increases: Vec<Increase>,
    /// Findings that make the check fail whatever the baseline says (`parse`).
    pub unbaselined: Vec<Finding>,
    /// Findings of the files and rules in `increases`.
    pub new_findings: Vec<Finding>,
    /// Total findings in the current report.
    pub now: usize,
    /// Total findings in the baseline.
    pub baseline: usize,
}

const HEADER: &str = "# craft-design-lint baseline: finding counts per file and rule.\n# `check --baseline` fails when a count rises; regenerate with `check --write-baseline`.\n\n";

fn counted(f: &Finding) -> bool {
    f.severity == Severity::Error && f.rule != PARSE
}

fn key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Baseline {
    /// The counts of the error findings in `report`.
    pub fn from_report(report: &Report) -> Self {
        let mut counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        for f in report.findings.iter().filter(|f| counted(f)) {
            *counts.entry(key(&f.path)).or_default().entry(f.rule.to_owned()).or_default() += 1;
        }
        Self { counts }
    }

    /// Total findings in the baseline.
    pub fn total(&self) -> usize {
        self.counts.values().flat_map(BTreeMap::values).sum()
    }

    /// The baseline as a TOML file: one table per file, one count per rule.
    pub fn to_toml(&self) -> String {
        let mut doc = DocumentMut::new();
        for (path, rules) in &self.counts {
            let mut table = Table::new();
            for (rule, n) in rules {
                table[rule] = value(i64::try_from(*n).unwrap_or(i64::MAX));
            }
            doc[path] = Item::Table(table);
        }
        format!("{HEADER}{doc}")
    }

    /// Reads a baseline file written by [`Baseline::to_toml`]; fails with [`Error::Io`] or
    /// [`Error::Toml`].
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_owned(), source })?;
        Self::parse(path, &text)
    }

    /// Parses baseline text; `origin` names the file in errors.
    pub fn parse(origin: &Path, text: &str) -> Result<Self, Error> {
        #[derive(Deserialize)]
        struct Raw(BTreeMap<String, BTreeMap<String, usize>>);
        let Raw(counts) = toml::from_str(text).map_err(|e| Error::Toml { path: origin.to_owned(), message: e.to_string() })?;
        Ok(Self { counts })
    }

    /// Compares `report` with the baseline: a file and rule fails when it has more findings than listed.
    pub fn check(&self, report: &Report) -> BaselineCheck {
        let now = Self::from_report(report);
        let mut increases = Vec::new();
        for (path, rules) in &now.counts {
            for (rule, n) in rules {
                let before = self.counts.get(path).and_then(|r| r.get(rule)).copied().unwrap_or(0);
                if *n > before {
                    increases.push(Increase { path: PathBuf::from(path), rule: rule.clone(), baseline: before, now: *n });
                }
            }
        }
        let new_findings = report
            .findings
            .iter()
            .filter(|f| counted(f) && increases.iter().any(|i| key(&f.path) == key(&i.path) && i.rule == f.rule))
            .cloned()
            .collect();
        let unbaselined = report.findings.iter().filter(|f| f.severity == Severity::Error && f.rule == PARSE).cloned().collect();
        BaselineCheck { increases, unbaselined, new_findings, now: now.total(), baseline: self.total() }
    }
}

impl BaselineCheck {
    /// 0 when nothing rose and every file parses, 1 otherwise.
    pub fn exit_code(&self) -> u8 {
        u8::from(!self.increases.is_empty() || !self.unbaselined.is_empty())
    }

    /// The findings of rising counts, the rises, and a closing line.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for f in self.unbaselined.iter().chain(&self.new_findings) {
            let _ = writeln!(out, "{f}");
        }
        for i in &self.increases {
            let _ = writeln!(out, "{}: [{}] {} findings, baseline {}", i.path.display(), i.rule, i.now, i.baseline);
        }
        let verdict = if self.exit_code() == 0 { "ok" } else { "failed" };
        let _ = write!(
            out,
            "design-lint: baseline {verdict}: {} findings now, {} in the baseline, {} rising",
            self.now,
            self.baseline,
            self.increases.len()
        );
        if self.increases.is_empty() && self.now < self.baseline {
            out.push_str("; run `check --write-baseline` to lower it");
        }
        out.push('\n');
        out
    }
}
