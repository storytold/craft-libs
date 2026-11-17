//! The per-step JSON report of a gate run.
//!
//! Schema 1 holds the schema number and one object per step, in run order:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "steps": [
//!     { "step": "fmt", "command": "cargo fmt --all -- --check", "status": "ok", "duration_ms": 812 },
//!     { "step": "test", "command": null, "status": "fail", "duration_ms": 4031 },
//!     { "step": "wasm", "command": null, "status": "skip", "duration_ms": null }
//!   ]
//! }
//! ```
//!
//! `status` is `ok`, `fail`, `canceled` or `skip`. `command` is `null` when the step recorded none,
//! and `duration_ms` is `null` for a step that did not start. Machine, GPU and other app-specific
//! details stay in the app's own report.
//!
//! The file is replaced atomically with `craft_atomic`: a reader sees the previous complete report or
//! the new one, never a partial file, and a failed write leaves the previous report in place.

use std::io;
use std::path::Path;

use craft_atomic::{AtomicFs, RealFs, RenameRetry};
use serde_json::{Value, json};

use crate::runner::StepRecord;

/// The schema number the report carries.
pub const SCHEMA: u64 = 1;

/// The report of `records` as JSON.
pub fn to_json(records: &[StepRecord]) -> Value {
    let steps: Vec<Value> = records
        .iter()
        .map(|r| {
            json!({
                "step": r.step,
                "command": r.command,
                "status": r.status.as_str(),
                "duration_ms": r.duration.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            })
        })
        .collect();
    json!({ "schema": SCHEMA, "steps": steps })
}

/// Writes the report of `records` to `path`, replacing any earlier report atomically.
pub fn write(path: &Path, records: &[StepRecord]) -> io::Result<()> {
    write_custom(&RealFs, RenameRetry::platform(), path, records)
}

fn write_custom(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, records: &[StepRecord]) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(&to_json(records)).map_err(io::Error::other)?;
    bytes.push(b'\n');
    craft_atomic::atomic_write_custom(fs, retry, path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Error, RunOptions, Step, run_steps_to};
    use craft_atomic::NewFilePermissions;
    use std::fs::{File, Permissions};

    /// The real file system with every rename failing.
    struct FailingRename;

    impl AtomicFs for FailingRename {
        fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
            RealFs.create_new(path, permissions)
        }
        fn sync(&self, file: &File) -> io::Result<()> {
            RealFs.sync(file)
        }
        fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
            Err(io::Error::other("injected rename failure"))
        }
        fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
            RealFs.hard_link(from, to)
        }
        fn remove(&self, path: &Path) -> io::Result<()> {
            RealFs.remove(path)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            RealFs.sync_dir(dir)
        }
        fn set_permissions(&self, file: &File, perms: Permissions) -> io::Result<()> {
            RealFs.set_permissions(file, perms)
        }
        #[cfg(unix)]
        fn set_group(&self, file: &File, gid: u32) -> io::Result<()> {
            RealFs.set_group(file, gid)
        }
    }

    fn keys(v: &Value) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().expect("an object").keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }

    #[test]
    fn report_shape_and_atomic_replacement() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("ci.json");
        let mut steps = vec![
            Step::new("fmt", |log| {
                log.command("cargo fmt --all -- --check");
                Ok(())
            }),
            Step::new("test", |_| Err("exited with 101".to_owned())),
            Step::new("wasm", |_| Ok(())).with_command("cargo check --target wasm32-unknown-unknown"),
        ];
        let options = RunOptions::default().with_report(&path);
        let result = run_steps_to(&mut steps, &options, &mut Vec::new(), &mut Vec::new());
        assert!(matches!(result, Err(Error::Step { .. })));

        let written = std::fs::read(&path).unwrap();
        let report: Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(keys(&report), ["schema", "steps"]);
        assert_eq!(report["schema"], 1);
        let rows = report["steps"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        for row in rows {
            assert_eq!(keys(row), ["command", "duration_ms", "status", "step"]);
        }
        assert_eq!(rows[0]["step"], "fmt");
        assert_eq!(rows[0]["command"], "cargo fmt --all -- --check");
        assert_eq!(rows[0]["status"], "ok");
        assert!(rows[0]["duration_ms"].is_u64());
        assert_eq!(rows[1]["command"], Value::Null);
        assert_eq!(rows[1]["status"], "fail");
        assert!(rows[1]["duration_ms"].is_u64());
        assert_eq!(rows[2]["status"], "skip");
        assert_eq!(rows[2]["command"], "cargo check --target wasm32-unknown-unknown");
        assert_eq!(rows[2]["duration_ms"], Value::Null);

        // A failed replacement keeps the previous complete report and returns the error.
        let records = vec![StepRecord { step: "fmt".into(), command: None, status: crate::runner::Status::Ok, duration: None }];
        let no_retry = RenameRetry { attempts: 0, first_delay_ms: 0 };
        assert!(write_custom(&FailingRename, no_retry, &path, &records).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), written);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "a staging file was left behind");
    }

    #[test]
    fn report_errors_never_pass_a_failed_run() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let unwritable = dir.path().join("missing").join("ci.json");
        let options = RunOptions::default().with_report(&unwritable);

        let mut passing = vec![Step::new("fmt", |_| Ok(()))];
        let mut out = Vec::new();
        let result = run_steps_to(&mut passing, &options, &mut out, &mut Vec::new());
        assert!(matches!(result, Err(Error::Report(_))), "{result:?}");
        let out = String::from_utf8_lossy(&out);
        assert!(out.starts_with("\nCI summary:\n  FAIL  report: ") && out.ends_with("\n  ok    fmt\n") && !out.contains("passed"), "{out}");

        let mut failing = vec![Step::new("fmt", |_| Err("bad".to_owned()))];
        let mut err = Vec::new();
        let result = run_steps_to(&mut failing, &options, &mut Vec::new(), &mut err);
        assert_eq!(result, Err(Error::Step { step: "fmt".into(), cause: "bad".into() }));
        assert!(String::from_utf8_lossy(&err).contains("report: "));
    }
}
