//! Native macOS application lifecycle regression harness.
//!
//! Formalizes the regression fixtures identified in Issue #3 and PR #5 into a shared
//! test harness that validates native OS interactions across Craft applications.
//!
//! Specifically validates:
//! 1. Window startup and event loop initialization.
//! 2. Red window close button (exercises `craft_quit::on_close_request` to prevent AppKit
//!    `NSRangeException` and `-[_NSTouchBarFinderObservation invalidate]` crashes).
//! 3. Native `Cmd+Q` termination.
//! 4. Programmatic close (`ViewportCommand::Close`).
//! 5. Unsaved document cancellation (window remains alive and interactive).
//! 6. Unsaved document save & quit (`App::on_exit` executes and persists state to disk).
//! 7. Minimized window quit.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixtureKind {
    Startup,
    RedWindowClose,
    CmdQQuit,
    ProgrammaticClose,
    UnsavedCancel,
    UnsavedSave,
    MinimizedQuit,
}

impl FixtureKind {
    pub const ALL: &[FixtureKind] = &[
        FixtureKind::Startup,
        FixtureKind::RedWindowClose,
        FixtureKind::CmdQQuit,
        FixtureKind::ProgrammaticClose,
        FixtureKind::UnsavedCancel,
        FixtureKind::UnsavedSave,
        FixtureKind::MinimizedQuit,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FixtureKind::Startup => "startup",
            FixtureKind::RedWindowClose => "red_window_close",
            FixtureKind::CmdQQuit => "cmd_q_quit",
            FixtureKind::ProgrammaticClose => "programmatic_close",
            FixtureKind::UnsavedCancel => "unsaved_cancel",
            FixtureKind::UnsavedSave => "unsaved_save",
            FixtureKind::MinimizedQuit => "minimized_quit",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            FixtureKind::Startup => "Startup",
            FixtureKind::RedWindowClose => "Red Window Close",
            FixtureKind::CmdQQuit => "Cmd+Q Quit",
            FixtureKind::ProgrammaticClose => "Programmatic Close",
            FixtureKind::UnsavedCancel => "Unsaved Document Cancel",
            FixtureKind::UnsavedSave => "Unsaved Document Save",
            FixtureKind::MinimizedQuit => "Minimized Window Quit",
        }
    }

    pub fn trigger_mechanism(self) -> &'static str {
        match self {
            FixtureKind::Startup => "eframe::run_native",
            FixtureKind::RedWindowClose => "on_close_request(ctx)",
            FixtureKind::CmdQQuit => "NSApp terminate:",
            FixtureKind::ProgrammaticClose => "ViewportCommand::Close",
            FixtureKind::UnsavedCancel => "ViewportCommand::CancelClose",
            FixtureKind::UnsavedSave => "Save & on_exit flush",
            FixtureKind::MinimizedQuit => "Minimize -> terminate:",
        }
    }

    pub fn expected_outcome(self) -> &'static str {
        match self {
            FixtureKind::Startup => "Init event loop without panic",
            FixtureKind::RedWindowClose => "Clean exit (status 0, no NSRangeException)",
            FixtureKind::CmdQQuit => "Clean exit (status 0)",
            FixtureKind::ProgrammaticClose => "Clean exit (status 0)",
            FixtureKind::UnsavedCancel => "Window remains open and responsive",
            FixtureKind::UnsavedSave => "State persisted to disk, exit 0",
            FixtureKind::MinimizedQuit => "Clean exit (status 0)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEnvironment {
    pub os_name: String,
    pub os_version: String,
    pub arch: String,
    pub model: String,
    pub has_touch_bar: bool,
    pub is_macos: bool,
}

impl HostEnvironment {
    pub fn detect() -> Self {
        let is_macos = cfg!(target_os = "macos");
        let arch = std::env::consts::ARCH.to_string();

        let os_version = if is_macos {
            Command::new("sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        } else {
            std::env::consts::OS.to_string()
        };

        let model = if is_macos {
            Command::new("sysctl")
                .args(["-n", "hw.model"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        } else {
            "generic".into()
        };

        // Touch Bar Mac identification: MacBookPro13,* to MacBookPro17,1
        let has_touch_bar = model.starts_with("MacBookPro13,")
            || model.starts_with("MacBookPro14,")
            || model.starts_with("MacBookPro15,")
            || model.starts_with("MacBookPro16,")
            || model.starts_with("MacBookPro17,1");

        Self { os_name: std::env::consts::OS.to_string(), os_version, arch, model, has_touch_bar, is_macos }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FixtureStatus {
    Specified,
    Passed,
    Failed,
    Skipped,
}

impl std::fmt::Display for FixtureStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixtureStatus::Specified => write!(f, "SPECIFIED"),
            FixtureStatus::Passed => write!(f, "PASSED"),
            FixtureStatus::Failed => write!(f, "FAILED"),
            FixtureStatus::Skipped => write!(f, "SKIPPED"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureExecutionResult {
    pub fixture: String,
    pub trigger: String,
    pub expected: String,
    pub status: FixtureStatus,
    pub exit_status: Option<i32>,
    pub duration_ms: u64,
    pub state_persisted: bool,
    pub appkit_crash_detected: bool,
    pub passed: bool,
    pub diagnostic_details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleReport {
    pub version: String,
    pub timestamp: String,
    pub host: HostEnvironment,
    pub target: String,
    pub summary: String,
    pub fixtures: Vec<FixtureExecutionResult>,
}

#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    pub target_consumer: String,
    pub app_binary: Option<PathBuf>,
    pub fixtures: Vec<FixtureKind>,
    pub json_output: bool,
    pub output_file: Option<PathBuf>,
}

/// Executes a lifecycle fixture against a compiled application binary if provided,
/// or returns the contract specification inventory without simulated execution.
pub fn execute_fixture(fixture: FixtureKind, app_binary: Option<&Path>, temp_dir: &Path) -> FixtureExecutionResult {
    let state_file = temp_dir.join(format!("{}_state.json", fixture.name()));

    let Some(bin) = app_binary else {
        // Contract specification inventory mode: no process execution.
        return FixtureExecutionResult {
            fixture: fixture.display_name().to_string(),
            trigger: fixture.trigger_mechanism().to_string(),
            expected: fixture.expected_outcome().to_string(),
            status: FixtureStatus::Specified,
            exit_status: None,
            duration_ms: 0,
            state_persisted: false,
            appkit_crash_detected: false,
            passed: false,
            diagnostic_details: "Contract specification inventory; launch executable required for execution.".into(),
        };
    };

    let start = Instant::now();
    let mut cmd = Command::new(bin);
    cmd.env("CRAFT_LIFECYCLE_FIXTURE", fixture.name());
    cmd.env("CRAFT_LIFECYCLE_TMPDIR", temp_dir);
    cmd.env("CRAFT_LIFECYCLE_STATE_FILE", &state_file);
    cmd.env("CRAFT_LIFECYCLE_PROTOCOL_VERSION", "1");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return FixtureExecutionResult {
                fixture: fixture.display_name().to_string(),
                trigger: fixture.trigger_mechanism().to_string(),
                expected: fixture.expected_outcome().to_string(),
                status: FixtureStatus::Failed,
                exit_status: None,
                duration_ms: start.elapsed().as_millis() as u64,
                state_persisted: false,
                appkit_crash_detected: false,
                passed: false,
                diagnostic_details: format!("Failed to spawn executable: {e}"),
            };
        }
    };

    // Bounded execution with timeout to prevent hanging on GUI applications
    let timeout = Duration::from_millis(5000);
    let poll_interval = Duration::from_millis(25);
    let mut timed_out = false;
    let mut exit_status = None;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                return FixtureExecutionResult {
                    fixture: fixture.display_name().to_string(),
                    trigger: fixture.trigger_mechanism().to_string(),
                    expected: fixture.expected_outcome().to_string(),
                    status: FixtureStatus::Failed,
                    exit_status: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    state_persisted: false,
                    appkit_crash_detected: false,
                    passed: false,
                    diagnostic_details: format!("Failed waiting for child process: {e}"),
                };
            }
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    if timed_out {
        return FixtureExecutionResult {
            fixture: fixture.display_name().to_string(),
            trigger: fixture.trigger_mechanism().to_string(),
            expected: fixture.expected_outcome().to_string(),
            status: FixtureStatus::Failed,
            exit_status: None,
            duration_ms,
            state_persisted: false,
            appkit_crash_detected: false,
            passed: false,
            diagnostic_details: "Process execution timed out after 5000ms (unresponsive or did not exit)".into(),
        };
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_end(&mut stderr);
    }

    let code = exit_status.and_then(|s| s.code());
    let stdout_str = String::from_utf8_lossy(&stdout);
    let stderr_str = String::from_utf8_lossy(&stderr);
    let combined_output = format!("{stdout_str}\n{stderr_str}");

    let appkit_crash = combined_output.contains("NSRangeException")
        || combined_output.contains("-[_NSTouchBarFinderObservation invalidate]")
        || combined_output.contains("SIGSEGV")
        || combined_output.contains("SIGBUS");

    let ack_marker = format!("[CRAFT_LIFECYCLE] ACK: {}", fixture.name());
    let has_ack = combined_output.contains(&ack_marker);
    let state_persisted = state_file.exists() && fs::metadata(&state_file).map(|m| m.len()).unwrap_or(0) > 0;

    let (passed, status, diagnostic_details) = if exit_status.map(|s| !s.success()).unwrap_or(true) {
        let details = format!(
            "Process failed with exit status {:?}{}{}",
            code,
            if appkit_crash { " (AppKit crash detected)" } else { "" },
            if stderr_str.trim().is_empty() { String::new() } else { format!(": {}", stderr_str.trim()) }
        );
        (false, FixtureStatus::Failed, details)
    } else if appkit_crash {
        (false, FixtureStatus::Failed, "AppKit crash detected in process output".into())
    } else if !has_ack {
        (false, FixtureStatus::Failed, format!("No lifecycle adapter handshake detected (expected '{ack_marker}' in output)"))
    } else if fixture == FixtureKind::UnsavedSave && !state_persisted {
        (false, FixtureStatus::Failed, format!("State file was not persisted or is empty at {}", state_file.display()))
    } else if fixture == FixtureKind::UnsavedCancel && !combined_output.contains("[CRAFT_LIFECYCLE] CANCEL_CLOSE_ACK") {
        (false, FixtureStatus::Failed, "CancelClose did not confirm window preservation ('[CRAFT_LIFECYCLE] CANCEL_CLOSE_ACK' missing)".into())
    } else {
        (true, FixtureStatus::Passed, format!("Exited cleanly with status 0 in {duration_ms}ms (handshake verified)"))
    };

    FixtureExecutionResult {
        fixture: fixture.display_name().to_string(),
        trigger: fixture.trigger_mechanism().to_string(),
        expected: fixture.expected_outcome().to_string(),
        status,
        exit_status: code,
        duration_ms,
        state_persisted,
        appkit_crash_detected: appkit_crash,
        passed,
        diagnostic_details,
    }
}

pub fn run_lifecycle(root_dir: &Path, config: LifecycleConfig) -> Result<bool, String> {
    if config.fixtures.is_empty() {
        return Err("No fixtures selected for lifecycle verification.".into());
    }

    let host = HostEnvironment::detect();
    if !host.is_macos {
        eprintln!("==========================================================================================");
        eprintln!("macOS Native Application Lifecycle Regression Matrix");
        eprintln!("Host OS '{}' is not macOS. Lifecycle fixtures require an AppKit execution environment.", host.os_name);
        eprintln!("Skipping lifecycle suite: UNSUPPORTED_PLATFORM");
        eprintln!("==========================================================================================");
        return Err(format!("lifecycle verification requires macOS (host is '{}')", host.os_name));
    }

    let run_id =
        format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let temp_dir = root_dir.join("target").join("lifecycle").join(format!("tmp-{run_id}"));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("create tmp dir: {e}"))?;

    if !config.json_output {
        println!("==========================================================================================");
        println!("macOS Native Application Lifecycle Regression Matrix");
        println!(
            "Host: {} ({}, {}) | Touch Bar: {} | Target: {}",
            host.model,
            host.arch,
            host.os_version,
            if host.has_touch_bar { "YES" } else { "NO" },
            config.target_consumer
        );
        if let Some(bin) = &config.app_binary {
            println!("Mode: LIVE_EXECUTION (binary: {})", bin.display());
        } else {
            println!("Mode: SPECIFICATION_INVENTORY (no --app-binary provided; contract inventory only)");
        }
        println!("==========================================================================================");
        println!("{:<24} {:<28} {:<12} {:<10}", "Fixture", "Trigger Mechanism", "Status", "Duration");
        println!("{:-<24} {:-<28} {:-<12} {:-<10}", "", "", "", "");
    }

    let mut results = Vec::new();

    for fixture in &config.fixtures {
        let res = execute_fixture(*fixture, config.app_binary.as_deref(), &temp_dir);
        if !config.json_output {
            println!("{:<24} {:<28} {:<12} {}ms", res.fixture, res.trigger, res.status.to_string(), res.duration_ms);
        }
        results.push(res);
    }
    if !config.json_output {
        println!("==========================================================================================");
    }

    let total_fixtures = config.fixtures.len();
    let (summary, success) = if config.app_binary.is_none() {
        (format!("SPECIFICATION_INVENTORY ({} fixtures specified, 0 executed)", total_fixtures), true)
    } else {
        let passed_count = results.iter().filter(|r| r.status == FixtureStatus::Passed).count();
        let failed_count = results.iter().filter(|r| r.status == FixtureStatus::Failed).count();
        if failed_count > 0 || total_fixtures == 0 {
            (format!("FIXTURES_FAILED ({failed_count} of {} failed)", total_fixtures), false)
        } else {
            (format!("ALL_FIXTURES_PASSED ({passed_count} of {} passed)", total_fixtures), true)
        }
    };

    if !config.json_output {
        println!("Lifecycle Summary: {summary}\n");
    }

    let report = LifecycleReport {
        version: "1.0".into(),
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_default(),
        host,
        target: config.target_consumer,
        summary,
        fixtures: results,
    };

    let report_json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialize: {e}"))?;

    let output_path = config.output_file.unwrap_or_else(|| root_dir.join("target").join("lifecycle").join("report.json"));
    if let Some(parent) = output_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&output_path, &report_json).map_err(|e| format!("failed to write report to {}: {e}", output_path.display()))?;

    if config.json_output {
        println!("{report_json}");
    } else {
        println!("Report written to {}", output_path.display());
    }

    // Clean up temporary files
    let _ = fs::remove_dir_all(&temp_dir);

    Ok(success)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_environment_detect() {
        let host = HostEnvironment::detect();
        assert!(!host.arch.is_empty());
        assert!(!host.os_name.is_empty());
    }

    #[test]
    fn test_fixture_inventory_without_binary() {
        let tmp = std::env::temp_dir().join("craft_test_lifecycle_inv");
        let _ = fs::create_dir_all(&tmp);

        for fixture in FixtureKind::ALL {
            let res = execute_fixture(*fixture, None, &tmp);
            assert_eq!(res.status, FixtureStatus::Specified);
            assert!(!res.passed, "inventory mode must never claim passing validation");
            assert_eq!(res.exit_status, None);
            assert_eq!(res.duration_ms, 0);
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_fixture_live_execution_failure_reports_failed() {
        let tmp = std::env::temp_dir().join("craft_test_lifecycle_fail");
        let _ = fs::create_dir_all(&tmp);

        let false_bin = Path::new("/usr/bin/false");
        if false_bin.exists() {
            let res = execute_fixture(FixtureKind::Startup, Some(false_bin), &tmp);
            assert_eq!(res.status, FixtureStatus::Failed, "real process failure must report FAILED, not UNEXECUTED");
            assert!(!res.passed);
            assert_eq!(res.exit_status, Some(1));
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_negative_control_true_rejected() {
        let tmp = std::env::temp_dir().join(format!("craft_test_lifecycle_true_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);

        let true_bin = Path::new("/usr/bin/true");
        if true_bin.exists() {
            for fixture in FixtureKind::ALL {
                let res = execute_fixture(*fixture, Some(true_bin), &tmp);
                assert_eq!(res.status, FixtureStatus::Failed, "Negative control /usr/bin/true MUST NOT pass fixture {}", fixture.display_name());
                assert!(!res.passed);
                assert!(res.diagnostic_details.contains("handshake") || res.diagnostic_details.contains("exit status"));
            }
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_mock_adapter_handshake_flow() {
        let tmp = std::env::temp_dir().join(format!("craft_test_lifecycle_mock_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);

        // Create a mock script that acts as an instrumented Craft lifecycle adapter
        let script_path = tmp.join("mock_app.sh");
        let script = r#"#!/bin/sh
if [ "$CRAFT_LIFECYCLE_FIXTURE" = "startup" ]; then
    echo "[CRAFT_LIFECYCLE] ACK: startup"
    exit 0
elif [ "$CRAFT_LIFECYCLE_FIXTURE" = "unsaved_save" ]; then
    echo "[CRAFT_LIFECYCLE] ACK: unsaved_save"
    echo '{"saved": true}' > "$CRAFT_LIFECYCLE_STATE_FILE"
    exit 0
elif [ "$CRAFT_LIFECYCLE_FIXTURE" = "unsaved_cancel" ]; then
    echo "[CRAFT_LIFECYCLE] ACK: unsaved_cancel"
    echo "[CRAFT_LIFECYCLE] CANCEL_CLOSE_ACK: window preserved"
    exit 0
else
    echo "unknown"
    exit 1
fi
"#;
        fs::write(&script_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        // 1. Startup fixture passes with handshake
        let res_startup = execute_fixture(FixtureKind::Startup, Some(&script_path), &tmp);
        assert_eq!(res_startup.status, FixtureStatus::Passed);
        assert!(res_startup.passed);

        // 2. UnsavedSave fixture passes when state file is persisted
        let res_save = execute_fixture(FixtureKind::UnsavedSave, Some(&script_path), &tmp);
        assert_eq!(res_save.status, FixtureStatus::Passed);
        assert!(res_save.passed);
        assert!(res_save.state_persisted);

        // 3. UnsavedCancel passes when cancel acknowledgment is emitted
        let res_cancel = execute_fixture(FixtureKind::UnsavedCancel, Some(&script_path), &tmp);
        assert_eq!(res_cancel.status, FixtureStatus::Passed);
        assert!(res_cancel.passed);

        let _ = fs::remove_dir_all(&tmp);
    }
}
