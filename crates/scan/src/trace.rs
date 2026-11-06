//! Optional JSONL phase trace for the scan backend's `acquire`.
//!
//! Set `CRAFT_SCAN_TRACE=1` (stderr) or to a file path to append one JSON object per line.
//! Events name phases only; no device ids, names, ports, or image bytes.

use std::fs::{File, OpenOptions};
use std::io::{Write, stderr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::HandlingStatus;

/// Where acquire trace lines go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceDest {
    Off,
    Stderr,
    File(PathBuf),
}

/// Parse `CRAFT_SCAN_TRACE`. Empty and `0`/`false`/`off` disable tracing.
pub fn trace_dest_from_str(value: &str) -> TraceDest {
    let trimmed = value.trim();
    if trimmed.is_empty() || matches!(trimmed.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off") {
        return TraceDest::Off;
    }
    if matches!(trimmed.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on" | "stderr") {
        return TraceDest::Stderr;
    }
    TraceDest::File(PathBuf::from(trimmed))
}

/// Environment variable that turns the acquire trace on.
pub const TRACE_ENV: &str = "CRAFT_SCAN_TRACE";

/// The name [`TRACE_ENV`] had before this crate moved to craft-libs; read when it is not set.
pub const LEGACY_TRACE_ENV: &str = "PDFCRAFT_SCAN_TRACE";

/// Read `CRAFT_SCAN_TRACE` from the environment, or the older `PDFCRAFT_SCAN_TRACE` when it is not set.
pub fn trace_dest_from_env() -> TraceDest {
    match std::env::var(TRACE_ENV).or_else(|_| std::env::var(LEGACY_TRACE_ENV)) {
        Ok(value) => trace_dest_from_str(&value),
        Err(_) => TraceDest::Off,
    }
}

/// Phase logger for one acquire attempt.
pub struct AcquireTrace {
    dest: TraceDest,
    file: Mutex<Option<File>>,
}

impl AcquireTrace {
    /// A trace that writes where the trace environment variable says (off when it is unset).
    pub fn from_env() -> Self {
        Self::new(trace_dest_from_env())
    }

    /// A trace writing to `dest`. A file that cannot be opened for appending turns the trace off instead of failing.
    pub fn new(dest: TraceDest) -> Self {
        let file = match &dest {
            TraceDest::File(path) => open_trace_file(path),
            _ => None,
        };
        let dest = match dest {
            TraceDest::File(_path) if file.is_none() => TraceDest::Off,
            other => other,
        };
        Self { dest, file: Mutex::new(file) }
    }

    /// Whether lines are written at all.
    pub fn enabled(&self) -> bool {
        !matches!(self.dest, TraceDest::Off)
    }

    /// Writes one JSON line with the timestamp, `name` as `phase`, and each field as a string value. Write errors are
    /// ignored. Nothing is written when the trace is off.
    pub fn phase(&self, name: &str, fields: &[(&str, &str)]) {
        if matches!(self.dest, TraceDest::Off) {
            return;
        }
        let line = format_line(name, fields);
        match &self.dest {
            TraceDest::Off => {}
            TraceDest::Stderr => {
                let mut out = stderr().lock();
                let _ = writeln!(out, "{line}");
            }
            TraceDest::File(_) => {
                if let Ok(mut guard) = self.file.lock()
                    && let Some(file) = guard.as_mut()
                {
                    let _ = writeln!(file, "{line}");
                    let _ = file.flush();
                }
            }
        }
    }

    /// Writes a `status_read` line for a handling-status read taken `when`; `feeder_ready` is `unknown` when not reported.
    pub fn status(&self, when: &str, status: HandlingStatus) {
        self.phase(
            "status_read",
            &[
                ("when", when),
                ("feeder_ready", status.feeder_ready.map_or("unknown", bool_str)),
                ("flat_ready", bool_str(status.flat_ready)),
                ("duplex_ready", bool_str(status.duplex_ready)),
                ("cover_open", bool_str(status.cover_open)),
                ("path_cover_open", bool_str(status.path_cover_open)),
                ("jam", bool_str(status.jam)),
                ("multiple_feed", bool_str(status.multiple_feed)),
                ("attention", bool_str(status.attention)),
                ("lamp_error", bool_str(status.lamp_error)),
            ],
        );
    }
}

/// `"true"` or `"false"`.
fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Opens `path` for appending, creating it; `None` on any error.
fn open_trace_file(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// One trace line: a JSON object with `ts` (milliseconds since the Unix epoch), `phase` and the fields, all as strings.
fn format_line(phase: &str, fields: &[(&str, &str)]) -> String {
    let ts = timestamp_ms();
    let mut out = String::from("{\"ts\":");
    json_string(&mut out, &ts);
    out.push_str(",\"phase\":");
    json_string(&mut out, phase);
    for (key, value) in fields {
        out.push(',');
        json_string(&mut out, key);
        out.push(':');
        json_string(&mut out, value);
    }
    out.push('}');
    out
}

/// Milliseconds since the Unix epoch as text, or "0" when the clock is before it.
///
/// On `wasm32-unknown-unknown` there is no clock and `SystemTime::now` panics, so the same "0" is returned there.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn timestamp_ms() -> String {
    "0".into()
}

/// Milliseconds since the Unix epoch as text, or "0" when the clock is before it.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn timestamp_ms() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis().to_string(),
        Err(_) => "0".into(),
    }
}

/// Appends `text` to `buf` as a quoted JSON string, escaping quotes, backslashes and control characters.
fn json_string(buf: &mut String, text: &str) {
    buf.push('"');
    for ch in text.chars() {
        match ch {
            '"' | '\\' => {
                buf.push('\\');
                buf.push(ch);
            }
            c if c.is_control() => {
                buf.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_dest_parses_common_values() {
        assert_eq!(trace_dest_from_str(""), TraceDest::Off);
        assert_eq!(trace_dest_from_str("0"), TraceDest::Off);
        assert_eq!(trace_dest_from_str("false"), TraceDest::Off);
        assert_eq!(trace_dest_from_str("1"), TraceDest::Stderr);
        assert_eq!(trace_dest_from_str("TRUE"), TraceDest::Stderr);
        assert_eq!(trace_dest_from_str("stderr"), TraceDest::Stderr);
        let path = trace_dest_from_str(r"C:\temp\scan-trace.jsonl");
        assert_eq!(path, TraceDest::File(PathBuf::from(r"C:\temp\scan-trace.jsonl")));
    }

    #[test]
    fn format_line_escapes_quotes_and_names_phase() {
        let line = format_line("device_open", &[("connection", "wsd"), ("note", "say \"hi\"")]);
        assert!(line.starts_with("{\"ts\":\""));
        assert!(line.contains("\"phase\":\"device_open\""));
        assert!(line.contains("\"connection\":\"wsd\""));
        assert!(line.contains("\"note\":\"say \\\"hi\\\"\""));
    }

    /// Runs only under a wasm test runner: there the clock is absent and the line carries the fallback.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[test]
    fn timestamp_is_the_zero_fallback_without_a_clock() {
        assert_eq!(timestamp_ms(), "0");
        assert!(format_line("p", &[]).starts_with("{\"ts\":\"0\""));
    }
}
