//! Workspace tooling for craft-libs: `cargo xtask <command>`.

mod ci;
mod compat;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compat::{CompatConfig, ConsumerTarget, KNOWN_CONSUMERS, run_compat};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              Run craft-libs quality gates (fmt --check, clippy -D warnings, test --workspace)
  compat [opts]   Validate cross-application compatibility against consuming applications

options for `compat`:
  --revision <REV>      Target craft-libs revision to validate (commit, branch, or HEAD; default: HEAD)
  --consumers <LIST>    Comma-separated list of consumers to test, or 'all' (default: photocraft,vectorcraft)
  --consumer <PATH>     Direct path to a specific consumer repository directory
  --target-pkg <PKG>    Explicit target package to test (default: looked up from known consumers)
  --consumers-dir <DIR> Directory containing consumer checkouts (default: ..)
  --skip-baseline       Skip running baseline checks, only test candidate revision
  --json                Emit machine-readable JSON report to stdout
  --output <FILE>       File path to write JSON report
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
            if let Err(error) = validate_compat_args(&args[1..]) {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
            let mut revision = "HEAD".to_string();
            let mut consumer_names: Vec<String> = vec!["photocraft".to_string(), "vectorcraft".to_string()];
            let mut consumers_dir = root.parent().unwrap_or(&root).to_path_buf();
            let mut direct_consumer_path: Option<PathBuf> = None;
            let mut explicit_target_pkg: Option<String> = None;
            let mut skip_baseline = false;
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
                    "--consumer" => {
                        if i + 1 < args.len() {
                            direct_consumer_path = Some(PathBuf::from(&args[i + 1]));
                            i += 1;
                        }
                    }
                    "--target-pkg" => {
                        if i + 1 < args.len() {
                            explicit_target_pkg = Some(args[i + 1].clone());
                            i += 1;
                        }
                    }
                    "--consumers" => {
                        if i + 1 < args.len() {
                            let val = &args[i + 1];
                            if val == "all" {
                                consumer_names = KNOWN_CONSUMERS.iter().map(|c| c.name.to_string()).collect();
                            } else {
                                consumer_names = val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
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

            let consumers = if let Some(direct_path) = direct_consumer_path {
                let name = direct_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                vec![ConsumerTarget { name, repo_dir: direct_path, target_package: explicit_target_pkg }]
            } else {
                consumer_names
                    .into_iter()
                    .map(|name| {
                        let known = KNOWN_CONSUMERS.iter().find(|k| k.name == name);
                        let dir_name = known.map(|k| k.dir_name).unwrap_or(&name);
                        let repo_dir = consumers_dir.join(dir_name);
                        let target_package = explicit_target_pkg.clone().or_else(|| known.map(|k| k.default_target_pkg.to_string()));
                        ConsumerTarget { name, repo_dir, target_package }
                    })
                    .collect()
            };

            let config = CompatConfig { candidate_revision: revision, consumers, skip_baseline, json_output, output_file };

            match run_compat(&root, config) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => {
                    eprintln!("\nCompatibility verification detected regressions or unverified runs.");
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

/// Validate value-taking arguments before resolving revisions or creating worktrees.
fn validate_compat_args(args: &[String]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        if matches!(args[i].as_str(), "--revision" | "--consumer" | "--target-pkg" | "--consumers" | "--consumers-dir" | "--output") {
            let value =
                args.get(i + 1).filter(|v| !v.trim().is_empty() && !v.starts_with('-')).ok_or_else(|| format!("{} requires a value", args[i]))?;
            if args[i] == "--consumers" && value.split(',').any(|name| name.trim().is_empty()) {
                return Err("--consumers requires a nonempty list".into());
            }
            i += 1;
        }
        i += 1;
    }
    Ok(())
}

fn root_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has parent workspace root").to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_option_shaped_values_are_rejected() {
        for option in ["--revision", "--consumer", "--target-pkg", "--consumers", "--consumers-dir", "--output"] {
            for tail in [vec![], vec!["--json".into()], vec!["".into()]] {
                let mut args = vec![option.into()];
                args.extend(tail);
                assert!(validate_compat_args(&args).is_err(), "{args:?}");
            }
            assert!(validate_compat_args(&[option.into(), "valid".into()]).is_ok());
        }
        assert!(validate_compat_args(&["--consumers".into(), ",,".into()]).is_err());
    }
}
