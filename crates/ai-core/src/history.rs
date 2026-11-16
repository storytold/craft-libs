//! The local request history: what the data looks like, and how its lines combine into entries.
//!
//! The log is append-only and each job writes two lines, a [`StartRecord`] when it is submitted
//! and an [`EndRecord`] when it reaches a final state, joined by a `run` id. Two lines instead of
//! one rewritten line keeps the file append-only, which is the only safe way for several apps to
//! write it at once. A job whose app closed before the end was seen has a start and no end, and
//! is shown as unfinished rather than guessed at.
//!
//! What is never here: keys or other secrets, images or any media bytes, error messages (only the
//! [`ErrorKind`]), and prompt text unless the app was told to keep it. Costs are exactly what the
//! provider reported, or absent. This module is I/O free so a wasm widget can use it.

use serde::{Deserialize, Serialize};

use crate::connection::Locality;
use crate::consent::DisclosureItem;
use crate::error::ErrorKind;
use crate::job::{CostEstimate, Usage};

/// Version written into every line. A reader skips lines it does not understand, so a newer app
/// adding a field never breaks an older one.
pub const HISTORY_SCHEMA: u32 = 1;

/// Longest prompt kept when the opt-in is on, in characters.
pub const MAX_PROMPT_CHARS: usize = 2000;

/// How a job ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Failed { kind: ErrorKind },
    Cancelled,
}

/// Written when a job is submitted (or refused by the provider).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRecord {
    pub v: u32,
    /// Joins this line to its [`EndRecord`]. Unique across apps and runs.
    pub run: String,
    /// Unix milliseconds.
    pub at_ms: u64,
    pub app: String,
    pub role_id: String,
    /// The feature as the user knows it, such as "Generative Fill".
    pub role_label: String,
    pub capability: String,
    pub connection: String,
    pub connection_label: String,
    pub model: String,
    pub locality: Locality,
    /// `scheme://host[:port]`, or the route name when there is no endpoint.
    pub destination: String,
    /// The same items the consent screen showed: kinds and sizes, never the data.
    #[serde(default)]
    pub sent: Vec<DisclosureItem>,
    /// Parameter names only.
    #[serde(default)]
    pub params: Vec<String>,
    /// The provider's id for the job, so a charge can be matched to its billing page. Absent when
    /// the provider refused the job before giving one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_job: Option<String>,
    /// The price the provider quoted before the job, when it can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<CostEstimate>,
    /// Only when the app opted in to keeping prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// Written when a job reaches a final state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndRecord {
    pub v: u32,
    pub run: String,
    pub at_ms: u64,
    pub outcome: Outcome,
    /// What the provider reported, or `None`. Never an estimate of ours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// One line of the log. Records are short-lived (built, written or merged once), so the size gap
/// between the two variants is not worth a `Box` at every use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum HistoryRecord {
    Start(StartRecord),
    End(EndRecord),
}

/// A job as the history list shows it: the start line and, if seen, the end line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub start: StartRecord,
    pub end: Option<EndRecord>,
}

impl HistoryEntry {
    /// `None` while unfinished, or if the clock moved backwards.
    pub fn duration_ms(&self) -> Option<u64> {
        let end = self.end.as_ref()?;
        end.at_ms.checked_sub(self.start.at_ms)
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.end.as_ref().map(|e| &e.outcome)
    }

    /// The cost the provider reported, in millionths of the currency.
    pub fn reported_cost(&self) -> Option<(u64, &str)> {
        let usage = self.end.as_ref()?.usage.as_ref()?;
        Some((usage.cost_micros?, usage.currency.as_deref()?))
    }
}

/// Entries newest first, plus how many lines could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryLog {
    pub entries: Vec<HistoryEntry>,
    pub skipped: usize,
}

/// Joins start and end lines by run id. An end with no start (its start was rotated away) is
/// dropped; a start with no end stays, unfinished. If a run somehow has two ends the first wins,
/// because terminal states are stable. Result is newest first.
pub fn merge(records: Vec<HistoryRecord>, skipped: usize) -> HistoryLog {
    let mut entries: Vec<HistoryEntry> = Vec::new();
    let mut index = std::collections::HashMap::new();
    let mut ends = Vec::new();
    for record in records {
        match record {
            HistoryRecord::Start(start) => {
                // A repeated run id would be a bug in a writer; keep the first.
                if !index.contains_key(&start.run) {
                    index.insert(start.run.clone(), entries.len());
                    entries.push(HistoryEntry { start, end: None });
                }
            }
            HistoryRecord::End(end) => ends.push(end),
        }
    }
    for end in ends {
        if let Some(&i) = index.get(&end.run)
            && let Some(entry) = entries.get_mut(i)
            && entry.end.is_none()
        {
            entry.end = Some(end);
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.start.at_ms));
    HistoryLog { entries, skipped }
}

/// Keeps a prompt to [`MAX_PROMPT_CHARS`] characters, on a character boundary.
pub fn clip_prompt(text: &str) -> String {
    text.chars().take(MAX_PROMPT_CHARS).collect()
}

pub mod report;

#[cfg(test)]
mod tests;
