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
