//! Workspace tooling for craft-libs: `cargo xtask <command>`.

mod adoption;
mod ci;
mod compat;
mod lifecycle;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compat::{CompatConfig, run_compat};
use lifecycle::{FixtureKind, LifecycleConfig, run_lifecycle};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              Run craft-libs quality gates (fmt --check, clippy -D warnings, test --workspace)
  compat [opts]   Validate cross-application compatibility against consuming applications
  adoption [opts] Report craft-libs dependency adoption across Craft applications
  lifecycle [opts] Run or inspect macOS application lifecycle fixtures

options for `compat`:
  --revision <REV>      Target craft-libs revision to validate (commit, branch, or HEAD; default: HEAD)
  --consumers <LIST>    Comma-separated list of consumers to test, or 'all' (default: photocraft,vectorcraft)
  --all-consumers       Test all known consumers across the Craft suite
  --consumers-dir <DIR> Directory containing consumer checkouts (default: ..)
  --skip-baseline       Skip running baseline checks, only test candidate revision
  --dry-run             Verify worktree creation and dependency resolution without running test suite
  --all-targets         Test all targets including doc-tests and integration tests (default: unit/lib only)
  --clean-head-only     Validate committed HEAD without snapshotting local uncommitted changes
  --quick               Override dev profile optimization (opt-level = 0) for fast dependency compilation
  --json                Emit machine-readable JSON report to stdout
  --output <FILE>       File path to write JSON report (default: target/compat/report.json)

options for `adoption`:
  --target-rev <REV>    Target craft-libs revision to evaluate adoption against (default: HEAD)
  --consumers-dir <DIR> Directory containing consumer checkouts (default: ..)
  --json                Emit machine-readable JSON report to stdout

options for `lifecycle`:
  --consumer <APP>      Target consumer application (default: photocraft)
  --app-binary <PATH>   Path to built application binary to run live execution against
  --fixtures <LIST>     Comma-separated list of fixtures to run, or 'all' (default: all)
  --json                Emit machine-readable JSON report to stdout
  --output <FILE>       File path to write JSON report (default: target/lifecycle/report.json)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = root_dir();

    match args.first().map(String::as_str) {
        Some("ci") => match ci::run(&root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("compat") => {
            let mut revision = "HEAD".to_string();
            let mut consumers = vec!["photocraft".to_string(), "vectorcraft".to_string()];
            let mut consumers_dir = root.parent().unwrap_or(&root).to_path_buf();
            let mut skip_baseline = false;
            let mut dry_run = false;
            let mut all_targets = false;
            let mut quick = false;
            let mut clean_head_only = false;
            let mut json_output = false;
            let mut output_file = None;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--revision" => {
                        if i + 1 < args.len() {
                            revision = args[i + 1].clone();
                            i += 1;
                        }
                    }
                    "--all-consumers" => {
                        consumers = compat::KNOWN_CONSUMERS.iter().map(|c| c.name.to_string()).collect();
                    }
                    "--consumers" => {
                        if i + 1 < args.len() {
                            let val = &args[i + 1];
                            if val == "all" {
                                consumers = compat::KNOWN_CONSUMERS.iter().map(|c| c.name.to_string()).collect();
                            } else {
                                consumers = val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                            }
                            i += 1;
                        }
                    }
                    "--consumers-dir" => {
                        if i + 1 < args.len() {
                            consumers_dir = PathBuf::from(&args[i + 1]);
                            i += 1;
                        }
                    }
                    "--skip-baseline" => skip_baseline = true,
                    "--dry-run" => dry_run = true,
                    "--all-targets" => all_targets = true,
                    "--quick" => quick = true,
                    "--clean-head-only" => clean_head_only = true,
                    "--json" => json_output = true,
                    "--output" => {
                        if i + 1 < args.len() {
                            output_file = Some(PathBuf::from(&args[i + 1]));
                            i += 1;
                        }
                    }
                    "-h" | "--help" | "help" => {
                        print!("{USAGE}");
                        return ExitCode::SUCCESS;
                    }
                    unknown => {
                        eprintln!("unknown option `{unknown}`\n\n{USAGE}");
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }

            let config = CompatConfig {
                candidate_revision: revision,
                consumers,
                consumers_dir,
                skip_baseline,
                dry_run,
                all_targets,
                quick,
                clean_head_only,
                json_output,
                output_file,
            };

            match run_compat(&root, config) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => {
                    eprintln!("\nCompatibility validation detected one or more regressions.");
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("adoption") => {
            let mut target_rev = "HEAD".to_string();
            let mut consumers_dir = root.parent().unwrap_or(&root).to_path_buf();
            let mut json_output = false;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--target-rev" => {
                        if i + 1 < args.len() {
                            target_rev = args[i + 1].clone();
                            i += 1;
                        }
                    }
                    "--consumers-dir" => {
                        if i + 1 < args.len() {
                            consumers_dir = PathBuf::from(&args[i + 1]);
                            i += 1;
                        }
                    }
                    "--json" => json_output = true,
                    "-h" | "--help" | "help" => {
                        print!("{USAGE}");
                        return ExitCode::SUCCESS;
                    }
                    unknown => {
                        eprintln!("unknown option `{unknown}`\n\n{USAGE}");
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }

            match adoption::run_adoption(&root, &consumers_dir, &target_rev, json_output) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("lifecycle") => {
            let mut target_consumer = "photocraft".to_string();
            let mut app_binary = None;
            let mut fixtures = FixtureKind::ALL.to_vec();
            let mut json_output = false;
            let mut output_file = None;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--consumer" => {
                        if i + 1 < args.len() {
                            target_consumer = args[i + 1].clone();
                            i += 1;
                        }
                    }
                    "--app-binary" | "--binary" => {
                        if i + 1 < args.len() {
                            app_binary = Some(PathBuf::from(&args[i + 1]));
                            i += 1;
                        }
                    }
                    "--fixtures" => {
                        if i + 1 < args.len() {
                            let val = &args[i + 1];
                            if val == "all" {
                                fixtures = FixtureKind::ALL.to_vec();
                            } else {
                                let mut parsed_fixtures = Vec::new();
                                for s in val.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                                    match s {
                                        "startup" => parsed_fixtures.push(FixtureKind::Startup),
                                        "red_close" | "red_window_close" => parsed_fixtures.push(FixtureKind::RedWindowClose),
                                        "cmd_q" | "cmd_q_quit" => parsed_fixtures.push(FixtureKind::CmdQQuit),
                                        "programmatic_close" | "close" => parsed_fixtures.push(FixtureKind::ProgrammaticClose),
                                        "unsaved_cancel" => parsed_fixtures.push(FixtureKind::UnsavedCancel),
                                        "unsaved_save" => parsed_fixtures.push(FixtureKind::UnsavedSave),
                                        "minimized_quit" => parsed_fixtures.push(FixtureKind::MinimizedQuit),
                                        unknown => {
                                            eprintln!(
                                                "unknown fixture name `{unknown}`\n\nValid fixtures: startup, red_window_close, cmd_q_quit, programmatic_close, unsaved_cancel, unsaved_save, minimized_quit"
                                            );
                                            return ExitCode::FAILURE;
                                        }
                                    }
                                }
                                if parsed_fixtures.is_empty() {
                                    eprintln!("error: --fixtures cannot be empty\n\n{USAGE}");
                                    return ExitCode::FAILURE;
                                }
                                fixtures = parsed_fixtures;
                            }
                            i += 1;
                        }
                    }
                    "--json" => json_output = true,
                    "--output" => {
                        if i + 1 < args.len() {
                            output_file = Some(PathBuf::from(&args[i + 1]));
                            i += 1;
                        }
                    }
                    "-h" | "--help" | "help" => {
                        print!("{USAGE}");
                        return ExitCode::SUCCESS;
                    }
                    unknown => {
                        eprintln!("unknown option `{unknown}`\n\n{USAGE}");
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }

            let config = LifecycleConfig { target_consumer, app_binary, fixtures, json_output, output_file };

            match run_lifecycle(&root, config) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => {
                    eprintln!("\nLifecycle regression detected.");
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn root_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has parent workspace root").to_path_buf()
}
