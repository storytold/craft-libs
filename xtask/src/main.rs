//! Workspace tooling for craft-libs: `cargo xtask <command>`.

mod ci;
mod compat;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compat::{CompatConfig, run_compat};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              Run craft-libs quality gates (fmt --check, clippy -D warnings, test --workspace)
  compat [opts]   Validate cross-application compatibility against consuming applications

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
