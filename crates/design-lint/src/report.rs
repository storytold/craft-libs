//! Findings and their rustc-style output.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::path::PathBuf;

use crate::config::Exemption;

/// How serious a finding is. Only errors make the exit code non-zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// Fails the check: the exit code is non-zero.
    Error,
    /// Reported only.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
        })
    }
}

/// One rule violation at a source position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    /// File path, relative to the root passed to [`crate::check`] when it is under it.
    pub path: PathBuf,
    /// First directory below the configured path the file sits in, `(root)` for files directly in it.
    pub area: String,
    /// 1-based line of the first token.
    pub line: usize,
    /// 1-based column of the first token, in characters.
    pub column: usize,
    /// Line of the last token; markers cover findings by their start line.
    pub end_line: usize,
    /// How serious the finding is.
    pub severity: Severity,
    /// Rule id such as `literal-size/vec2`.
    pub rule: &'static str,
    /// What is wrong and, where the rule has one, the fix.
    pub message: String,
}

impl Finding {
    /// An error finding with no file yet.
    pub fn new(rule: &'static str, line: usize, column: usize, end_line: usize, message: String) -> Self {
        Self { path: PathBuf::new(), area: String::new(), line, column, end_line, severity: Severity::Error, rule, message }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: {}[{}]: {}", self.path.display(), self.line, self.column, self.severity, self.rule, self.message)
    }
}

/// The result of [`crate::check`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    /// Findings sorted by path, line and column.
    pub findings: Vec<Finding>,
    /// Files that were parsed.
    pub files: usize,
    /// Markers found in those files.
    pub markers: usize,
    /// Path exemptions in the config.
    pub exemptions: Vec<Exemption>,
    /// `allow` and `expect` attributes naming a `clippy::disallowed_*` lint.
    pub clippy_allows: usize,
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("{n} {one}") } else { format!("{n} {one}s") }
}

impl Report {
    /// 0 when there is no error finding, 1 otherwise.
    pub fn exit_code(&self) -> u8 {
        u8::from(self.findings.iter().any(|f| f.severity == Severity::Error))
    }

    /// Counts per rule id, sorted by id.
    pub fn counts_by_rule(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for f in &self.findings {
            *counts.entry(f.rule).or_insert(0) += 1;
        }
        counts
    }

    /// The closing line: totals, counts per rule, markers and exemptions.
    pub fn summary_line(&self) -> String {
        let tail = format!(
            "{}, {}, {}",
            plural(self.markers, "marker"),
            plural(self.exemptions.len(), "exemption"),
            plural(self.clippy_allows, "clippy allow")
        );
        if self.findings.is_empty() {
            return format!("design-lint: no findings in {}; {tail}", plural(self.files, "file"));
        }
        let files = self.findings.iter().map(|f| &f.path).collect::<std::collections::BTreeSet<_>>().len();
        let rules: Vec<String> = self.counts_by_rule().into_iter().map(|(rule, n)| format!("{rule} {n}")).collect();
        format!("design-lint: {} in {} ({}); {tail}", plural(self.findings.len(), "finding"), plural(files, "file"), rules.join(", "))
    }

    /// One line per finding, the exemptions, then the summary line.
    pub fn render(&self) -> String {
        let mut out = self.render_body();
        out.push_str(&self.summary_line());
        out.push('\n');
        out
    }

    /// Like [`Report::render`], with the counts per rule and per area before the summary line.
    pub fn render_with_table(&self) -> String {
        let mut out = self.render_body();
        out.push_str(&self.summary_table());
        out.push('\n');
        out.push_str(&self.summary_line());
        out.push('\n');
        out
    }

    fn render_body(&self) -> String {
        let mut out = String::new();
        for f in &self.findings {
            let _ = writeln!(out, "{f}");
        }
        for e in &self.exemptions {
            let _ = writeln!(out, "exemption: {e}");
        }
        out
    }

    /// Counts per rule and per area, for sizing a port.
    pub fn summary_table(&self) -> String {
        let mut areas: BTreeMap<&str, usize> = BTreeMap::new();
        for f in &self.findings {
            *areas.entry(f.area.as_str()).or_insert(0) += 1;
        }
        let mut areas: Vec<_> = areas.into_iter().collect();
        areas.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let rules = self.counts_by_rule();
        let width = rules.keys().map(|r| r.len()).chain(areas.iter().map(|(a, _)| a.len())).chain(["total".len()]).max().unwrap_or(5);
        let mut out = String::new();
        let _ = writeln!(out, "{:<width$}  {:>8}", "rule", "findings");
        for (rule, n) in &rules {
            let _ = writeln!(out, "{rule:<width$}  {n:>8}");
        }
        let _ = writeln!(out, "{:<width$}  {:>8}", "total", self.findings.len());
        let _ = writeln!(out);
        let _ = writeln!(out, "{:<width$}  {:>8}", "area", "findings");
        for (area, n) in &areas {
            let _ = writeln!(out, "{area:<width$}  {n:>8}");
        }
        out
    }
}
