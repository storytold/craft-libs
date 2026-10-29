//! An opt-in trace of what a print job does, for diagnosing a job that was handed to the spooler
//! and never came out (nothing is recorded unless a host asks).
//!
//! A host installs a sink once with [`set_trace`] (the apps forward it to their own logger), or the
//! environment variable `CRAFT_PRINT_TRACE=stderr` writes each line to standard error. Lines carry
//! the elapsed time since the job began, so a hang shows as a gap before the next line.

use std::sync::OnceLock;

type Sink = Box<dyn Fn(&str) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install the trace sink: called once per trace line, on whichever thread produced it. Only the
/// first call has an effect; returns whether this call installed the sink.
pub fn set_trace(sink: impl Fn(&str) + Send + Sync + 'static) -> bool {
    SINK.set(Box::new(sink)).is_ok()
}

fn env_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("CRAFT_PRINT_TRACE").is_ok_and(|v| v.eq_ignore_ascii_case("stderr")))
}

/// Whether anything is listening, so callers can skip building a line.
pub(crate) fn enabled() -> bool {
    SINK.get().is_some() || env_enabled()
}

pub(crate) fn emit(line: &str) {
    if let Some(sink) = SINK.get() {
        sink(line);
    } else if env_enabled() {
        eprintln!("craft-print: {line}");
    }
}

/// The trace of one job: each line is prefixed with the time since the job began.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct JobTrace {
    began: std::time::Instant,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl JobTrace {
    pub(crate) fn begin() -> Self {
        Self { began: std::time::Instant::now() }
    }

    pub(crate) fn line(&self, text: impl FnOnce() -> String) {
        if enabled() {
            emit(&format!("[{:>6} ms] {}", self.began.elapsed().as_millis(), text()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_trace_builds_no_line_without_a_listener() {
        if enabled() {
            return; // another test or the environment installed one
        }
        let built = std::cell::Cell::new(false);
        JobTrace::begin().line(|| {
            built.set(true);
            String::new()
        });
        assert!(!built.get(), "the line is only built when something listens");
    }
}
