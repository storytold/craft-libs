//! The journal a command-line scan leaves beside its pages (`DIR/scan-job.json`), and the read-only
//! views over it. Nothing here opens a scanner.
//!
//! The file is a small versioned wrapper around `craft_scan::job::ScanJobJournal`:
//!
//! ```json
//! { "version": 1, "id": "scan-...", "started": 1760000000000, "updated": 1760000012345,
//!   "request": { "device": "...", "source": "feeder", "dpi": 200, "color": "gray", "pages": 20, ... },
//!   "journal": { ...ScanJobJournal... }, "view": { ...craft_job::JobView... }, "attempts": [ ... ] }
//! ```
//!
//! It never holds page bytes. `request` is what `scan --continue` repeats, because the journal itself
//! does not carry the settings. Times are milliseconds since the Unix epoch. A reader ignores fields it
//! does not know, defaults the ones an older writer lacked, and refuses a `version` newer than its own.
//! Whether the page files named by the journal are still on disk is not the journal's business; this
//! module counts them ([`page_files_on_disk`]) so the commands can say when they differ.

use std::path::{Path, PathBuf};

use craft_scan as scan;
use craft_scan::{
    Color, Intent, PageLength, PageSize, Region, Rotation, ScanSettings, Source,
    job::{ExitClass, ScanJobJournal, ScanJobState},
};
use serde_json::{Value, json};

/// The journal's file name inside a scan output folder.
pub const JOB_FILE: &str = "scan-job.json";
/// The wrapper version this build writes and the newest it reads.
pub const JOB_FILE_VERSION: u32 = 1;

/// Exit status of an app's `scan` command by [`ExitClass`]. 0 completed; 10 needs attention (not finished,
/// finished short, or outcome unknown); 11 failed; 12 cancelled. 1 stays "the command itself failed"
/// (bad arguments, an unreadable folder): no job state is implied.
pub const fn exit_code(class: ExitClass) -> u8 {
    match class {
        ExitClass::Completed => 0,
        ExitClass::Failed => 11,
        ExitClass::Cancelled => 12,
        // `needs_attention`, and any class a later craft-scan adds: a person looks first.
        _ => 10,
    }
}

/// The stable name of a job state, for the files and the listings.
pub fn state_name(state: ScanJobState) -> &'static str {
    state.name()
}

/// One journal file, as read or about to be written.
#[derive(Clone, Debug)]
pub struct JobRecord {
    /// Stable across `--continue`; unique enough to tell two folders' jobs apart.
    pub id: String,
    pub started: u64,
    pub updated: u64,
    /// The request fields (see [`request_json`]).
    pub request: Value,
    pub journal: ScanJobJournal,
    /// `craft_job::JobView` as JSON, as of the last write.
    pub view: Value,
    /// The attempt timeline across runs (each entry is a `craft_scan::job::AttemptRecord`).
    pub attempts: Vec<Value>,
}

impl JobRecord {
    /// The file's JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "version": JOB_FILE_VERSION,
            "id": self.id,
            "started": self.started,
            "updated": self.updated,
            "request": self.request,
            "journal": serde_json::to_value(&self.journal).unwrap_or(Value::Null),
            "view": self.view,
            "attempts": self.attempts,
        })
    }

    /// Parse the file's JSON. Unknown fields are ignored and missing optional ones defaulted; a `version`
    /// newer than [`JOB_FILE_VERSION`] is refused with a message that says to upgrade.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let version = value.get("version").and_then(Value::as_u64).unwrap_or(1);
        if version > u64::from(JOB_FILE_VERSION) {
            return Err(format!(
                "the job file is version {version}, newer than this build understands (version {JOB_FILE_VERSION}); upgrade the app to read or continue it"
            ));
        }
        let journal = value.get("journal").ok_or("the job file has no \"journal\"")?;
        let journal: ScanJobJournal =
            serde_json::from_value(journal.clone()).map_err(|error| format!("the job file's journal is not readable: {error}"))?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        Ok(Self {
            id: text("id"),
            started: number("started"),
            updated: number("updated"),
            request: value.get("request").cloned().unwrap_or(Value::Null),
            journal,
            view: value.get("view").cloned().unwrap_or(Value::Null),
            attempts: value.get("attempts").and_then(Value::as_array).cloned().unwrap_or_default(),
        })
    }

    /// The state to act on. A journal that says `acquiring` (or `preparing`, with attempts made) was left
    /// by a process that ended mid-scan, for example one that was killed: the scanner may have moved, so
    /// it is an unknown outcome.
    pub fn effective_state(&self) -> ScanJobState {
        match self.journal.state {
            ScanJobState::Acquiring => ScanJobState::NeedsAttention,
            ScanJobState::Preparing if self.journal.attempts_total > 0 => ScanJobState::NeedsAttention,
            state => state,
        }
    }

    /// True when [`Self::effective_state`] differs from what was written because the writer never finished.
    pub fn was_cut_off(&self) -> bool {
        self.effective_state() != self.journal.state
    }

    /// The exit class of [`Self::effective_state`].
    pub fn exit_class(&self) -> ExitClass {
        self.effective_state().exit_class()
    }

    /// Pages the journal says are kept.
    pub fn pages_kept(&self) -> usize {
        self.journal.pages.len()
    }

    /// The request was a manual-duplex reverse pass: its pages are only written at the end, so the page
    /// count on disk says nothing before then.
    pub fn is_reverse(&self) -> bool {
        self.request.get("reverse").and_then(Value::as_bool).unwrap_or(false)
    }

    /// `Title: state, 12 of 40 pages, 1 attempt, jam`, the same text as `ScanJobSnapshot::summary_line`
    /// (the journal has no such method), from the effective state.
    pub fn summary_line(&self) -> String {
        let journal = &self.journal;
        let pages = match journal.expected_pages {
            Some(total) => format!("{} of {} pages", journal.pages.len(), total),
            None => format!("{} pages", journal.pages.len()),
        };
        let attempts = if journal.attempts_total == 1 { "1 attempt".to_string() } else { format!("{} attempts", journal.attempts_total) };
        let mut line = format!("{}: {}, {pages}, {attempts}", journal.title, state_name(self.effective_state()));
        if let Some(failure) = &journal.last_error {
            line.push_str(", ");
            line.push_str(failure.info.kind.as_str());
        }
        line
    }

    /// The `last error kind` column, or `-`.
    pub fn last_error_kind(&self) -> &'static str {
        self.journal.last_error.as_ref().map_or("-", |failure| failure.info.kind.as_str())
    }
}

/// `page-001.bmp` ... files in `dir`: how many, or the reason they cannot be listed (a gap in the numbers).
pub fn page_files_on_disk(dir: &Path) -> Result<usize, String> {
    scan::listed_page_files(dir).map(|files| files.len()).map_err(|error| error.to_string())
}

/// What to tell a person when the page files and the journal disagree; `None` when they agree (or when the
/// job is a reverse pass that has not written its pages yet).
pub fn page_mismatch(dir: &Path, record: &JobRecord) -> Option<String> {
    let kept = record.pages_kept();
    match page_files_on_disk(dir) {
        Err(reason) => Some(format!("the page files cannot be checked against the journal: {reason}")),
        // A reverse pass writes its pages (interleaved with the fronts) only at the end, so counts do not compare.
        Ok(_) if record.is_reverse() => None,
        Ok(on_disk) if on_disk != kept => Some(format!(
            "the journal keeps {kept} page(s) but {on_disk} page file(s) are in the folder; the journal does not verify files, so check the folder before continuing"
        )),
        Ok(_) => None,
    }
}

/// Write `record` to `dir/scan-job.json` atomically: a temporary file in the same folder, then a rename.
pub fn write_record(dir: &Path, record: &JobRecord) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&record.to_json()).map_err(|error| error.to_string())?;
    let path = dir.join(JOB_FILE);
    let temp = dir.join(format!("{JOB_FILE}.tmp"));
    std::fs::write(&temp, text.as_bytes()).map_err(|error| format!("{}: {error}", temp.display()))?;
    std::fs::rename(&temp, &path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {error}", path.display())
    })
}

/// Read `dir/scan-job.json`.
pub fn read_record(dir: &Path) -> Result<JobRecord, String> {
    let path = dir.join(JOB_FILE);
    let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| format!("{}: not valid JSON: {error}", path.display()))?;
    JobRecord::from_json(&value).map_err(|error| format!("{}: {error}", path.display()))
}

/// Folders with a journal among `dirs`: just those given. With none given, the current folder and its
/// immediate subfolders (symbolic links are not followed). Never a deeper walk.
pub fn candidate_dirs(dirs: &[PathBuf], current: &Path) -> Vec<PathBuf> {
    if !dirs.is_empty() {
        return dirs.to_vec();
    }
    let mut found = vec![current.to_path_buf()];
    let mut children = Vec::new();
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir() && !kind.is_symlink()) {
                children.push(entry.path());
            }
        }
    }
    children.sort();
    found.extend(children);
    found
}

/// The journals found in `candidates` (those without `scan-job.json` are skipped; one that cannot be read
/// is reported with its reason, not hidden).
pub fn find_jobs(candidates: &[PathBuf]) -> Vec<(PathBuf, Result<JobRecord, String>)> {
    candidates.iter().filter(|dir| dir.join(JOB_FILE).is_file()).map(|dir| (dir.clone(), read_record(dir))).collect()
}

/// One line per job: `id  state  pages/expected  attempts  last-error  updated  DIR`, plus a note when the
/// page files on disk do not match the journal.
pub fn list_line(dir: &Path, record: &JobRecord) -> String {
    let expected = record.journal.expected_pages.map_or_else(|| "-".to_string(), |n| n.to_string());
    let mut line = format!(
        "{}  {}  {}/{}  {} attempt(s)  {}  {}  {}",
        record.id,
        state_name(record.effective_state()),
        record.pages_kept(),
        expected,
        record.journal.attempts_total,
        record.last_error_kind(),
        iso8601(record.updated),
        dir.display()
    );
    if page_mismatch(dir, record).is_some() {
        let on_disk = page_files_on_disk(dir).map_or_else(|_| "?".to_string(), |n| n.to_string());
        line.push_str(&format!("  [{on_disk} page file(s) on disk, journal keeps {}]", record.pages_kept()));
    }
    if record.was_cut_off() {
        line.push_str("  [the scan was cut off: outcome unknown]");
    }
    line
}

/// One job as the machine array of `scan jobs --json` and the `scan_jobs` tool.
pub fn job_json(dir: &Path, record: &JobRecord) -> Value {
    let mut value = record.to_json();
    value["dir"] = json!(dir.display().to_string());
    value["state"] = json!(state_name(record.effective_state()));
    value["written_state"] = json!(state_name(record.journal.state));
    value["exit_class"] = json!(record.exit_class().name());
    value["exit_code"] = json!(exit_code(record.exit_class()));
    value["cut_off"] = json!(record.was_cut_off());
    value["page_files"] = page_files_on_disk(dir).map_or(Value::Null, |n| json!(n));
    value["page_mismatch"] = page_mismatch(dir, record).map_or(Value::Null, Value::String);
    value["updated_text"] = json!(iso8601(record.updated));
    value
}

/// A failed read as a row of the machine array.
pub fn unreadable_json(dir: &Path, reason: &str) -> Value {
    json!({ "dir": dir.display().to_string(), "unreadable": reason })
}

/// What `scan job DIR` prints: the summary, the facts, the attempt timeline and the next step.
pub fn detail_text(program: &str, dir: &Path, record: &JobRecord) -> String {
    let mut out = String::new();
    let state = record.effective_state();
    out.push_str(&format!("{}\n", record.summary_line()));
    out.push_str(&format!("  id: {}\n  folder: {}\n", record.id, dir.display()));
    out.push_str(&format!("  started: {}   updated: {}\n", iso8601(record.started), iso8601(record.updated)));
    out.push_str(&format!(
        "  state: {}   exit class: {} (exit status {})\n",
        state_name(state),
        record.exit_class().name(),
        exit_code(record.exit_class())
    ));
    if record.was_cut_off() {
        out.push_str("  the file says the scan was running; the process ended without finishing (killed, or the power went): what the scanner did is unknown\n");
    }
    let expected = record.journal.expected_pages.map_or_else(|| "the whole feeder".to_string(), |n| n.to_string());
    out.push_str(&format!("  pages kept: {}   wanted: {}   attempts: {}\n", record.pages_kept(), expected, record.journal.attempts_total));
    if let Some(note) = page_mismatch(dir, record) {
        out.push_str(&format!("  WARNING: {note}\n"));
    }
    if let Some(failure) = &record.journal.last_error {
        out.push_str(&format!("  last error: {} ({}), {}\n", failure.info.kind.as_str(), failure.info.phase.as_str(), failure.info.message));
    }
    for warning in &record.journal.warnings {
        out.push_str(&format!("  warning: {warning}\n"));
    }
    if let Some(device) = record.view.get("device_name").and_then(Value::as_str) {
        out.push_str(&format!("  device: {device}\n"));
    } else if !record.journal.device.is_empty() {
        out.push_str(&format!("  device: {}\n", record.journal.device));
    }
    if !record.attempts.is_empty() {
        out.push_str("  attempts:\n");
        for attempt in &record.attempts {
            out.push_str(&format!("    {}\n", attempt_line(attempt)));
        }
    }
    out.push_str(&format!("  next: {}\n", next_step(program, dir, state)));
    out
}

fn attempt_line(attempt: &Value) -> String {
    let text = |key: &str| attempt.get(key).and_then(Value::as_str).unwrap_or("-");
    let number = |key: &str| attempt.get(key).and_then(Value::as_u64).unwrap_or(0);
    let mut line = format!(
        "#{} {} from {}, {} kept before, +{} page(s), {}",
        number("number"),
        text("continuation"),
        text("source_requested"),
        number("pages_before"),
        number("pages_gained"),
        text("outcome")
    );
    if let Some(kind) = attempt.pointer("/error/info/kind").and_then(Value::as_str) {
        line.push_str(&format!(" ({kind})"));
    }
    if let Some(started) = attempt.get("started").and_then(Value::as_u64) {
        line.push_str(&format!(", at {}", iso8601(started)));
    }
    line
}

/// The sentence `scan job` ends with; `program` is the command the app is invoked as.
pub fn next_step(program: &str, dir: &Path, state: ScanJobState) -> String {
    let d = dir.display();
    match state {
        ScanJobState::Terminal(scan::job::Outcome::Completed) => "nothing; the scan is complete".to_string(),
        ScanJobState::NeedsAttention => format!(
            "look at the scanner first (a repeated acquire moves the carriage and pulls paper); then {program} scan --continue {d} --append|--restart --i-checked-the-scanner --yes"
        ),
        ScanJobState::Interrupted | ScanJobState::Preparing | ScanJobState::Acquiring => {
            format!("fix the cause (jam, cover, empty feeder), then {program} scan --continue {d} --append|--restart --yes")
        }
        _ => format!("{program} scan --continue {d} --restart --yes to scan again (kept pages are moved aside, not deleted)"),
    }
}

/// The request fields of a scan: what `--continue` repeats. Never includes tokens.
pub fn request_json(settings: &ScanSettings, reverse: bool) -> Value {
    let mut object = serde_json::Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            object.insert(key.to_string(), value);
        }
    };
    put("device", Some(json!(settings.device)));
    put("source", Some(json!(settings.source.name())));
    put("dpi", Some(json!(settings.dpi)));
    put("color", Some(json!(settings.color.as_str())));
    put("pages", Some(json!(settings.pages)));
    put("duplex", Some(json!(settings.duplex)));
    put("reverse", Some(json!(reverse)));
    put("dpi_y", settings.dpi_y.map(|v| json!(v)));
    put("brightness", settings.brightness.map(|v| json!(v)));
    put("contrast", settings.contrast.map(|v| json!(v)));
    put("threshold", settings.threshold.map(|v| json!(v)));
    put("rotation", settings.rotation.map(|v| json!(v.name())));
    put("region", settings.region.map(|r| json!({ "left": r.left, "top": r.top, "width": r.width, "height": r.height })));
    put("format", settings.format.as_ref().map(|v| json!(v)));
    put("compression", settings.compression.as_ref().map(|v| json!(v)));
    put("intent", settings.intent.map(|v| json!(v.as_str())));
    put("page_size", settings.page_size.map(|v| json!(v.as_str())));
    put("page_length", (settings.page_length != PageLength::Driver).then(|| json!(settings.page_length.as_str())));
    put("auto_deskew", settings.auto_deskew.map(|v| json!(v)));
    put("orientation", settings.orientation.map(|v| json!(v)));
    put("photometric", settings.photometric.map(|v| json!(v)));
    put("invert", settings.invert.map(|v| json!(v)));
    put("mirror", settings.mirror.map(|v| json!(v)));
    put("lamp", settings.lamp.map(|v| json!(v)));
    Value::Object(object)
}

/// The inverse of [`request_json`]. Missing fields take the same defaults as the command line; a field
/// whose value this build does not understand is an error, not a guess.
pub fn settings_from_request(request: &Value) -> Result<ScanSettings, String> {
    let object = request.as_object().ok_or("the job file has no request to repeat")?;
    let get = |key: &str| object.get(key).filter(|value| !value.is_null());
    let name = |key: &str| -> Result<Option<&str>, String> {
        get(key).map(|value| value.as_str().ok_or_else(|| format!("request.{key} is not text"))).transpose()
    };
    let whole = |key: &str| -> Result<Option<u64>, String> {
        get(key).map(|value| value.as_u64().ok_or_else(|| format!("request.{key} is not a whole number"))).transpose()
    };
    let signed = |key: &str| -> Result<Option<i32>, String> {
        get(key)
            .map(|value| value.as_i64().and_then(|n| i32::try_from(n).ok()).ok_or_else(|| format!("request.{key} is not a whole number")))
            .transpose()
    };
    let unsigned = |key: &str| -> Result<Option<u32>, String> {
        whole(key)?.map(|n| u32::try_from(n).map_err(|_| format!("request.{key} is too large"))).transpose()
    };
    let flag = |key: &str| -> Result<Option<bool>, String> {
        get(key).map(|value| value.as_bool().ok_or_else(|| format!("request.{key} is not true or false"))).transpose()
    };
    let region = match get("region") {
        None => None,
        Some(region) => {
            let part = |key: &str| {
                region
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| format!("request.region.{key} is not a whole number"))
            };
            Some(Region { left: part("left")?, top: part("top")?, width: part("width")?, height: part("height")? })
        }
    };
    Ok(ScanSettings {
        device: name("device")?.unwrap_or("").to_string(),
        source: match name("source")? {
            None => Source::Auto,
            Some(text) => Source::parse(text).ok_or_else(|| format!("request.source '{text}' is not understood"))?,
        },
        dpi: unsigned("dpi")?.unwrap_or(200),
        dpi_y: unsigned("dpi_y")?,
        color: match name("color")? {
            None => Color::Gray,
            Some(text) => Color::parse(text).ok_or_else(|| format!("request.color '{text}' is not understood"))?,
        },
        pages: unsigned("pages")?.unwrap_or(1),
        brightness: signed("brightness")?,
        contrast: signed("contrast")?,
        threshold: signed("threshold")?,
        rotation: name("rotation")?
            .map(|text| Rotation::parse(text).ok_or_else(|| format!("request.rotation '{text}' is not understood")))
            .transpose()?,
        region,
        format: name("format")?.map(str::to_string),
        compression: name("compression")?.map(str::to_string),
        intent: name("intent")?.map(|text| Intent::parse(text).ok_or_else(|| format!("request.intent '{text}' is not understood"))).transpose()?,
        page_size: name("page_size")?
            .map(|text| PageSize::parse(text).ok_or_else(|| format!("request.page_size '{text}' is not understood")))
            .transpose()?,
        page_length: match name("page_length")? {
            None => PageLength::Driver,
            Some(text) => PageLength::parse(text).ok_or_else(|| format!("request.page_length '{text}' is not understood"))?,
        },
        auto_deskew: flag("auto_deskew")?,
        orientation: signed("orientation")?,
        photometric: signed("photometric")?,
        invert: flag("invert")?,
        mirror: flag("mirror")?,
        lamp: flag("lamp")?,
        duplex: flag("duplex")?.unwrap_or(false),
        cancel: None,
        progress: None,
    })
}

/// Milliseconds since the Unix epoch, now (0 if the clock is before 1970).
pub fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `2026-10-14T09:30:00Z` for a millisecond timestamp (UTC). 0 reads as `-`.
pub fn iso8601(millis: u64) -> String {
    if millis == 0 {
        return "-".to_string();
    }
    let secs = millis / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// `scan jobs`: the listing for the folders in `dirs` (or the default ones), as text lines.
pub fn jobs_listing(dirs: &[PathBuf], current: &Path) -> (Vec<String>, Vec<Value>) {
    let mut lines = Vec::new();
    let mut rows = Vec::new();
    for (dir, record) in find_jobs(&candidate_dirs(dirs, current)) {
        match record {
            Ok(record) => {
                lines.push(list_line(&dir, &record));
                rows.push(job_json(&dir, &record));
            }
            Err(reason) => {
                lines.push(format!("{}  unreadable: {reason}", dir.display()));
                rows.push(unreadable_json(&dir, &reason));
            }
        }
    }
    (lines, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_scan::job::{ScanJob, ScanJobConfig};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("craft-scan-jobs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(state_settings: ScanSettings) -> JobRecord {
        let job = ScanJob::new(state_settings.clone(), ScanJobConfig::default().with_title("Invoices"));
        let snapshot = job.snapshot();
        JobRecord {
            id: "scan-1".into(),
            started: 1_760_000_000_000,
            updated: 1_760_000_012_000,
            request: request_json(&state_settings, false),
            journal: snapshot.into_journal(),
            view: Value::Null,
            attempts: Vec::new(),
        }
    }

    #[test]
    fn exit_codes_are_distinct_and_stable() {
        assert_eq!(exit_code(ExitClass::Completed), 0);
        assert_eq!(exit_code(ExitClass::NeedsAttention), 10);
        assert_eq!(exit_code(ExitClass::Failed), 11);
        assert_eq!(exit_code(ExitClass::Cancelled), 12);
    }

    #[test]
    fn the_journal_round_trips_through_an_atomic_write() {
        let dir = temp("roundtrip");
        let settings = ScanSettings { device: "wia:x".into(), pages: 3, dpi: 300, ..ScanSettings::default() };
        let written = record(settings);
        write_record(&dir, &written).unwrap();
        assert!(!dir.join("scan-job.json.tmp").exists(), "the temporary file is renamed away");
        let read = read_record(&dir).unwrap();
        assert_eq!(read.id, "scan-1");
        assert_eq!(read.journal, written.journal);
        assert_eq!(read.journal.expected_pages, Some(3));
        let again = settings_from_request(&read.request).unwrap();
        assert_eq!((again.device.as_str(), again.dpi, again.pages), ("wia:x", 300, 3));
        // A second write replaces the first.
        write_record(&dir, &written).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_version_is_refused_and_unknown_fields_are_ignored() {
        let mut value = record(ScanSettings::default()).to_json();
        value["something_new"] = json!({ "a": 1 });
        value["journal"]["also_new"] = json!(true);
        assert!(JobRecord::from_json(&value).is_ok());
        value["version"] = json!(JOB_FILE_VERSION + 1);
        let error = JobRecord::from_json(&value).unwrap_err();
        assert!(error.contains("newer") && error.contains("upgrade"), "{error}");
    }

    #[test]
    fn missing_optional_fields_default() {
        let value = json!({ "version": 1, "journal": { "title": "T", "state": "interrupted" } });
        let record = JobRecord::from_json(&value).unwrap();
        assert_eq!(record.journal.state, ScanJobState::Interrupted);
        assert_eq!(record.pages_kept(), 0);
        assert!(record.attempts.is_empty());
        assert!(JobRecord::from_json(&json!({ "version": 1 })).is_err());
    }

    #[test]
    fn a_job_left_acquiring_reads_as_needs_attention() {
        let mut record = record(ScanSettings::default());
        record.journal.state = ScanJobState::Acquiring;
        assert_eq!(record.effective_state(), ScanJobState::NeedsAttention);
        assert!(record.was_cut_off());
        assert_eq!(record.exit_class(), ExitClass::NeedsAttention);
        record.journal.state = ScanJobState::Interrupted;
        assert!(!record.was_cut_off());
    }

    #[test]
    fn the_summary_line_matches_the_snapshot_text() {
        let settings = ScanSettings { pages: 40, ..ScanSettings::default() };
        let job = ScanJob::new(settings, ScanJobConfig::default().with_title("Invoices"));
        let expected = job.snapshot().summary_line();
        assert_eq!(record(ScanSettings { pages: 40, ..ScanSettings::default() }).summary_line(), expected);
        assert!(expected.starts_with("Invoices: preparing, 0 of 40 pages, 0 attempts"));
    }

    #[test]
    fn page_files_are_compared_with_the_journal() {
        let dir = temp("files");
        let record = record(ScanSettings { pages: 2, ..ScanSettings::default() });
        // The journal keeps none and the folder has none.
        assert!(page_mismatch(&dir, &record).is_none());
        std::fs::write(dir.join("page-001.bmp"), b"x").unwrap();
        let note = page_mismatch(&dir, &record).unwrap();
        assert!(note.contains("1 page file"), "{note}");
        assert!(list_line(&dir, &record).contains("1 page file(s) on disk"));
        // A gap in the numbering is reported, not hidden.
        std::fs::write(dir.join("page-003.bmp"), b"x").unwrap();
        assert!(page_mismatch(&dir, &record).unwrap().contains("cannot be checked"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jobs_are_found_in_the_given_folders_and_one_level_down() {
        let root = temp("find");
        let a = root.join("a");
        let deep = root.join("b").join("c");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&deep).unwrap();
        write_record(&a, &record(ScanSettings::default())).unwrap();
        write_record(&deep, &record(ScanSettings::default())).unwrap();
        std::fs::write(root.join("b").join(JOB_FILE), b"{ not json").unwrap();
        let found = find_jobs(&candidate_dirs(&[], &root));
        let dirs: Vec<_> = found.iter().map(|(dir, _)| dir.clone()).collect();
        assert_eq!(dirs, vec![a.clone(), root.join("b")], "two levels down is not searched");
        assert!(found[0].1.is_ok());
        assert!(found[1].1.as_ref().unwrap_err().contains("not valid JSON"));
        let (lines, rows) = jobs_listing(std::slice::from_ref(&a), &root);
        assert_eq!(lines.len(), 1);
        assert_eq!(rows[0]["state"], "preparing");
        assert!(lines[0].starts_with("scan-1  preparing  0/1"), "{}", lines[0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_request_survives_every_field() {
        let settings = ScanSettings {
            device: "wia:x".into(),
            source: Source::Feeder,
            dpi: 150,
            dpi_y: Some(300),
            color: Color::Rgb,
            pages: 0,
            brightness: Some(-5),
            rotation: Some(Rotation::Landscape),
            region: Some(Region { left: 100, top: 200, width: 3000, height: 4000 }),
            format: Some("jpeg".into()),
            intent: Some(Intent::Text),
            page_size: Some(PageSize::A4),
            auto_deskew: Some(true),
            lamp: Some(false),
            duplex: true,
            ..ScanSettings::default()
        };
        let again = settings_from_request(&request_json(&settings, false)).unwrap();
        assert_eq!(again, settings);
        let bad = json!({ "color": "purple" });
        assert!(settings_from_request(&bad).unwrap_err().contains("color"));
    }

    #[test]
    fn the_page_length_request_survives_and_the_default_leaves_the_journal_unchanged() {
        let plain = ScanSettings::default();
        assert!(request_json(&plain, false).get("page_length").is_none(), "a driver length writes nothing, so old journals are unchanged");
        assert_eq!(settings_from_request(&request_json(&plain, false)).unwrap().page_length, PageLength::Driver);
        for mode in [PageLength::Auto, PageLength::AutoInfer] {
            let settings = ScanSettings { page_length: mode, page_size: Some(PageSize::Legal), ..ScanSettings::default() };
            let json = request_json(&settings, false);
            assert_eq!(json["page_length"], mode.as_str());
            assert_eq!(settings_from_request(&json).unwrap(), settings);
        }
        assert!(settings_from_request(&json!({ "page_length": "exact" })).unwrap_err().contains("page_length"));
    }

    #[test]
    fn times_read_as_utc_text() {
        assert_eq!(iso8601(0), "-");
        assert_eq!(iso8601(1_760_000_000_000), "2025-10-09T08:53:20Z");
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00Z");
    }
}
