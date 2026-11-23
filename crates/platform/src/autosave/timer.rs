//! The autosave timer and its interval choices.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// PhotoCraft's default interval, `fileHandling.autosaveMinutes = 10` (minimum 1).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// The intervals Photoshop's File Handling page offers for "Automatically Save Recovery
/// Information Every", in minutes; [`DEFAULT_INTERVAL`] is one of them. Apps offer them as a
/// preferences row with `craft_ui::preferences::SettingRule::minutes(&INTERVAL_CHOICES_MINUTES)`
/// and read the stored choice with [`interval_from_minutes`].
pub const INTERVAL_CHOICES_MINUTES: [u32; 5] = [5, 10, 15, 30, 60];

/// The interval for a stored choice ("10"); `None` when the text is not one of
/// [`INTERVAL_CHOICES_MINUTES`].
pub fn interval_from_minutes(text: &str) -> Option<Duration> {
    let minutes: u32 = text.parse().ok()?;
    INTERVAL_CHOICES_MINUTES.contains(&minutes).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

/// The interval for a stored choice as a count; `None` when it is not one of
/// [`INTERVAL_CHOICES_MINUTES`].
pub fn interval_for_minutes(minutes: u32) -> Option<Duration> {
    INTERVAL_CHOICES_MINUTES.contains(&minutes).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

/// Reads the stored minutes as an integer, or as the text an older settings file holds ("10"),
/// for a field with `#[serde(deserialize_with = "deserialize_minutes")]`; the preferences row
/// writes integers.
pub fn deserialize_minutes<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Count(u32),
        Text(String),
    }
    match Stored::deserialize(deserializer)? {
        Stored::Count(minutes) => Ok(minutes),
        Stored::Text(text) => text.trim().parse().map_err(serde::de::Error::custom),
    }
}

/// The status text of a failed autosave (an error in the status bar; no notice, the next
/// interval tries again). `{error}`: the write error. Catalog id `autosave.failed`.
pub const AUTOSAVE_FAILED: &str = "Autosave failed: {error}";

/// A document as the timer sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutosaveDoc {
    /// The app's document id, stable while the document is open.
    pub id: u64,
    /// Bumped on every edit.
    pub revision: u64,
    /// Has unsaved changes.
    pub dirty: bool,
}

/// What [`Autosave::due`] decided for one call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutosaveDue {
    /// The documents to autosave now; they count as in flight until [`Autosave::finished`].
    pub save: Vec<u64>,
    /// The documents the timer stopped tracking because they are clean (saved, or undone back to
    /// the saved state) or gone. The app discards their recovery copies.
    pub discard: Vec<u64>,
}

/// When to autosave, after PhotoCraft's timer.
///
/// The first [`due`](Self::due) starts the clock; each interval after it, every document that is
/// dirty, whose revision differs from the one last autosaved, and that has no autosave in flight
/// is due. The app writes them (on a job) and reports each with [`finished`](Self::finished).
/// A document that turns clean without a save comes back in [`AutosaveDue::discard`]; saving or
/// closing a document is [`forget`](Self::forget) (and the app discards its copy).
/// With autosave off (`set_interval(None)`) nothing is ever due.
#[derive(Debug)]
pub struct Autosave {
    interval: Option<Duration>,
    next: Option<Instant>,
    saved: HashMap<u64, u64>,
    in_flight: HashMap<u64, u64>,
    due_now: bool,
}

impl Autosave {
    /// `None` turns autosave off (PhotoCraft's `fileHandling.autosave = false`).
    pub fn new(interval: Option<Duration>) -> Self {
        Self { interval, next: None, saved: HashMap::new(), in_flight: HashMap::new(), due_now: false }
    }

    /// A changed preference; the next deadline moves to now plus the new interval.
    ///
    /// The clock restarts at the next [`due`](Self::due) call, which is the caller's "now"; an
    /// unchanged interval leaves it running.
    pub fn set_interval(&mut self, interval: Option<Duration>) {
        if self.interval != interval {
            self.interval = interval;
            self.next = None;
        }
    }

    /// Makes the next [`due`](Self::due) call autosave every dirty document that has no autosave
    /// in flight, whatever the interval says (tests, and an app's "autosave now"). It does not
    /// change a revision already autosaved, and does nothing with autosave off.
    pub fn autosave_now(&mut self) {
        self.due_now = true;
    }

    /// Records `revision` of document `id` as already autosaved, so [`due`](Self::due) does not
    /// write it again until the document changes (PhotoCraft does this for a recovered document
    /// whose recovery copy is still fresh, so closing it does not discard that copy).
    // pc: crates/ui-egui/src/prefs/prefs_ui/startup.rs:177
    pub fn mark_autosaved(&mut self, id: u64, revision: u64) {
        self.in_flight.remove(&id);
        self.saved.insert(id, revision);
    }

    /// The documents to autosave now, and those whose recovery copy to discard.
    // pc: crates/ui-egui/src/prefs/prefs_ui/save.rs:228 autosave
    pub fn due(&mut self, now: Instant, docs: impl IntoIterator<Item = AutosaveDoc>) -> AutosaveDue {
        let docs: Vec<AutosaveDoc> = docs.into_iter().collect();
        // A document that is clean (or gone) has no pending copy to track.
        let dirty = |id: &u64| docs.iter().any(|d| d.id == *id && d.dirty);
        let mut discard: Vec<u64> = self.saved.keys().chain(self.in_flight.keys()).copied().filter(|id| !dirty(id)).collect();
        discard.sort_unstable();
        discard.dedup();
        for id in &discard {
            self.saved.remove(id);
            self.in_flight.remove(id);
        }
        let mut due = AutosaveDue { save: Vec::new(), discard };
        let Some(interval) = self.interval else {
            self.next = None;
            return due;
        };
        let forced = std::mem::take(&mut self.due_now);
        match self.next {
            None if !forced => {
                self.next = Some(now + interval);
                return due;
            }
            Some(next) if !forced && now < next => return due,
            _ => {}
        }
        self.next = Some(now + interval);
        for d in docs {
            if d.dirty && self.saved.get(&d.id) != Some(&d.revision) && !self.in_flight.contains_key(&d.id) {
                self.in_flight.insert(d.id, d.revision);
                due.save.push(d.id);
            }
        }
        due
    }

    /// An autosave ended. On success `revision` is remembered; on failure the status text
    /// ([`AUTOSAVE_FAILED`] filled in) comes back for the status bar.
    pub fn finished(&mut self, id: u64, revision: u64, result: Result<(), String>) -> Option<String> {
        // A result for a document that was forgotten meanwhile, or for a revision other than the
        // one in flight (an older job after `forget` and a new `due`), is ignored.
        if self.in_flight.get(&id) != Some(&revision) {
            return None;
        }
        self.in_flight.remove(&id);
        match result {
            Ok(()) => {
                self.saved.insert(id, revision);
                None
            }
            Err(error) => Some(AUTOSAVE_FAILED.replace("{error}", &error)),
        }
    }

    /// The document was saved or closed.
    // pc: crates/ui-egui/src/prefs/prefs_ui/save.rs:263 (services.discard_autosave)
    pub fn forget(&mut self, id: u64) {
        self.saved.remove(&id);
        self.in_flight.remove(&id);
    }

    /// When the next interval ends, for `request_repaint_after`.
    pub fn deadline(&self) -> Option<Instant> {
        self.next
    }
}
