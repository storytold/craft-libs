//! `design-lint.toml`: which directories the checker reads and which paths are exempt.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Error;
use crate::rules::{PARSE, selector_matches};

/// What to check. Paths are relative to the root passed to [`crate::check`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Config {
    /// Directories or files to check.
    pub paths: Vec<PathBuf>,
    /// Also check test files and `#[cfg(test)]` code.
    pub include_tests: bool,
    /// Report `marker/unused` as an error instead of a warning.
    pub deny_unused: bool,
    /// Path-wide allowances, each with a reason.
    pub exempt: Vec<Exemption>,
    /// Rule ids or families reported as warnings instead of errors (an app's config for `props/shadow`).
    pub warn: Vec<String>,
}

/// A path-wide allowance: findings of `rules` under `path` are not reported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Exemption {
    /// A file or directory, relative to the root passed to [`crate::check`]; matched by path components.
    pub path: PathBuf,
    /// Rule ids or family prefixes (`literal-size`); empty means every rule.
    pub rules: Vec<String>,
    /// Why the path is exempt; at least three characters.
    pub reason: String,
}

impl Exemption {
    /// Whether this exemption silences `rule` in the file at `path` (relative to the root). `parse` is never silenced.
    pub fn covers(&self, path: &Path, rule: &str) -> bool {
        rule != PARSE && path.starts_with(&self.path) && (self.rules.is_empty() || self.rules.iter().any(|r| selector_matches(r, rule)))
    }
}

impl fmt::Display for Exemption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rules = if self.rules.is_empty() { "all rules".to_owned() } else { self.rules.join(", ") };
        write!(f, "{} ({rules}): {}", self.path.display(), self.reason)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    lint: RawLint,
    #[serde(default)]
    exempt: Vec<RawExempt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExempt {
    path: PathBuf,
    #[serde(default)]
    rules: Vec<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLint {
    paths: Vec<PathBuf>,
    #[serde(default, rename = "include-tests")]
    include_tests: bool,
    #[serde(default, rename = "deny-unused")]
    deny_unused: bool,
    #[serde(default)]
    warn: Vec<String>,
}

impl Config {
    /// A config that checks `paths` and skips tests.
    pub fn new(paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self { paths: paths.into_iter().map(Into::into).collect(), include_tests: false, deny_unused: false, exempt: Vec::new(), warn: Vec::new() }
    }

    /// Reads a `design-lint.toml` file.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_owned(), source })?;
        Self::parse(path, &text)
    }

    /// Parses config text; `origin` names the file in errors.
    ///
    /// An `[[exempt]]` without a reason of at least three characters, or naming a rule that does not exist (or `parse`), is an error.
    pub fn parse(origin: &Path, text: &str) -> Result<Self, Error> {
        let fail = |message: String| Error::Toml { path: origin.to_owned(), message };
        let raw: Raw = toml::from_str(text).map_err(|e| fail(e.to_string()))?;
        let mut exempt = Vec::new();
        for e in raw.exempt {
            let shown = e.path.display();
            let reason = e.reason.as_deref().map(str::trim).unwrap_or_default();
            if reason.chars().count() < crate::markers::MIN_REASON {
                return Err(fail(format!("[[exempt]] for `{shown}` needs a `reason` of at least {} characters", crate::markers::MIN_REASON)));
            }
            if let Some(unknown) = e.rules.iter().find(|r| !crate::RULES.iter().any(|known| selector_matches(r, known))) {
                return Err(fail(format!("[[exempt]] for `{shown}` names unknown rule `{unknown}`")));
            }
            if e.rules.iter().any(|r| r == PARSE) {
                return Err(fail(format!("[[exempt]] for `{shown}` names `{PARSE}`; files that do not parse cannot be exempt")));
            }
            exempt.push(Exemption { path: e.path, rules: e.rules, reason: reason.to_owned() });
        }
        if let Some(unknown) = raw.lint.warn.iter().find(|r| !crate::RULES.iter().any(|known| selector_matches(r, known))) {
            return Err(fail(format!("`warn` names unknown rule `{unknown}`")));
        }
        let fixed = [PARSE, crate::markers::EMPTY, crate::markers::INVALID, crate::markers::UNUSED, crate::UNUSED_EXEMPTION];
        if let Some(r) = raw.lint.warn.iter().find(|r| fixed.iter().any(|id| selector_matches(r, id))) {
            return Err(fail(format!("`warn` names `{r}`; parse, marker and exemption findings are always errors or follow `deny-unused`")));
        }
        Ok(Self { paths: raw.lint.paths, include_tests: raw.lint.include_tests, deny_unused: raw.lint.deny_unused, exempt, warn: raw.lint.warn })
    }
}
