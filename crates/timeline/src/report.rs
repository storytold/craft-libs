//! What an import or export could not represent exactly.

/// Severity of a report entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Level {
    /// Informational (an assumption the importer made).
    Info,
    /// Something was dropped or approximated.
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReportEntry {
    pub level: Level,
    pub message: String,
    /// How many times this message occurred.
    pub count: u32,
}

/// What an import/export could not represent exactly. Identical messages are merged with a count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Report {
    pub entries: Vec<ReportEntry>,
}

impl Report {
    pub fn push(&mut self, level: Level, message: impl Into<String>) {
        let message = message.into();
        if let Some(e) = self.entries.iter_mut().find(|e| e.level == level && e.message == message) {
            e.count += 1;
        } else {
            self.entries.push(ReportEntry { level, message, count: 1 });
        }
    }
    /// Add a warning while bounding distinct diagnostics. `limit` counts every distinct entry already
    /// in the report, informational ones included, exactly like FilmCraft's `MAX_REPORT_ENTRIES`
    /// budget this replaces; past it a new message is counted under "further diagnostics omitted".
    /// Existing messages continue to be counted.
    pub fn warn_limited(&mut self, limit: Option<usize>, message: impl Into<String>) {
        self.push_limited(Level::Warning, limit, message);
    }
    /// Add an informational diagnostic under the same distinct-entry budget as [`Report::warn_limited`].
    pub fn info_limited(&mut self, limit: Option<usize>, message: impl Into<String>) {
        self.push_limited(Level::Info, limit, message);
    }
    /// Add a diagnostic at `level`, counting a new message past `limit` distinct entries under
    /// "further diagnostics omitted"; a message already in the report keeps its own count.
    pub fn push_limited(&mut self, level: Level, limit: Option<usize>, message: impl Into<String>) {
        let mut message = message.into();
        if let Some(limit) = limit
            && self.entries.len() >= limit
            && !self.entries.iter().any(|e| e.level == level && e.message == message)
        {
            message = "further diagnostics omitted".into();
        }
        self.push(level, message);
    }
    pub fn warn(&mut self, message: impl Into<String>) {
        self.push(Level::Warning, message);
    }
    pub fn info(&mut self, message: impl Into<String>) {
        self.push(Level::Info, message);
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn warnings(&self) -> impl Iterator<Item = &ReportEntry> {
        self.entries.iter().filter(|e| e.level == Level::Warning)
    }
    pub fn has_warnings(&self) -> bool {
        self.warnings().next().is_some()
    }
    /// Whether any entry contains `needle` (case-insensitive).
    pub fn mentions(&self, needle: &str) -> bool {
        let n = needle.to_ascii_lowercase();
        self.entries.iter().any(|e| e.message.to_ascii_lowercase().contains(&n))
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for e in &self.entries {
            let lvl = match e.level {
                Level::Info => "info",
                Level::Warning => "warning",
            };
            if e.count > 1 {
                writeln!(f, "{lvl}: {} (×{})", e.message, e.count)?;
            } else {
                writeln!(f, "{lvl}: {}", e.message)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_entries_share_one_budget_and_keep_existing_counts() {
        let mut r = Report::default();
        r.info_limited(Some(2), "a");
        r.warn_limited(Some(2), "b");
        r.info_limited(Some(2), "c");
        r.info_limited(Some(2), "a");
        r.warn_limited(Some(2), "a");
        r.push_limited(Level::Warning, None, "d");
        let got: Vec<_> = r.entries.iter().map(|e| (e.level, e.message.as_str(), e.count)).collect();
        assert_eq!(
            got,
            [
                (Level::Info, "a", 2),
                (Level::Warning, "b", 1),
                (Level::Info, "further diagnostics omitted", 1),
                (Level::Warning, "further diagnostics omitted", 1),
                (Level::Warning, "d", 1),
            ]
        );
    }
}
