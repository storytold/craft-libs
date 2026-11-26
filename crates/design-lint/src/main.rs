//! The `craft-design-lint` command: checks an app's sources against the design rules.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use craft_design_lint::{Baseline, CLIPPY_TEMPLATE, Config, Error, check, check_clippy, write_clippy};

const USAGE: &str = "usage:
  craft-design-lint check [--config <path>] [--summary] [--deny-unused] [--baseline <path> | --write-baseline <path>]
                                                              report literal sizes, named colours, style mutation and crate-wide clippy
                                                              allows (default config: design-lint.toml); --write-baseline records the
                                                              finding counts, --baseline fails only when a count rises
  craft-design-lint structure [--root <dir>] [--config <path>]  layer map, imports, exceptions, crate graph, features and clones
  craft-design-lint clippy [--write] [--clippy-toml <path>]   print the template, or merge it into clippy.toml
  craft-design-lint check-clippy [--clippy-toml <path>]       fail unless clippy.toml carries the template and no other
                                                              clippy.toml or CLIPPY_CONF_DIR shadows it";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e @ Error::ClippyShadowed(_)) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
        Err(Error::ClippyDrift(drift)) => {
            for d in &drift {
                eprintln!("{d}");
            }
            eprintln!("design-lint: {} template entries missing or changed in clippy.toml; run `craft-design-lint clippy --write`", drift.len());
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("design-lint: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, Error> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(usage());
    };
    if command == "check" {
        return run_check(rest);
    }
    if command == "structure" {
        return craft_design_lint::run_structure(rest);
    }
    let mut write = false;
    let mut clippy_toml = PathBuf::from("clippy.toml");
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        match (command.as_str(), arg.as_str()) {
            ("clippy", "--write") => write = true,
            ("clippy" | "check-clippy", "--clippy-toml") => match rest.next() {
                Some(path) => clippy_toml = PathBuf::from(path),
                None => return Ok(usage()),
            },
            _ => return Ok(usage()),
        }
    }
    match command.as_str() {
        "clippy" if write => {
            let changed = write_clippy(&clippy_toml)?;
            println!("design-lint: {} {}", clippy_toml.display(), if changed { "updated" } else { "already current" });
        }
        "clippy" => print!("{CLIPPY_TEMPLATE}"),
        "check-clippy" => {
            check_clippy(&clippy_toml)?;
            println!("design-lint: {} carries the template", clippy_toml.display());
        }
        _ => return Ok(usage()),
    }
    Ok(ExitCode::SUCCESS)
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn run_check(args: &[String]) -> Result<ExitCode, Error> {
    let mut config_path = PathBuf::from("design-lint.toml");
    let mut summary = false;
    let mut deny_unused = false;
    let mut baseline: Option<PathBuf> = None;
    let mut write_baseline: Option<PathBuf> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = |args: &mut std::slice::Iter<String>| args.next().map(PathBuf::from);
        match arg.as_str() {
            "--summary" => summary = true,
            "--deny-unused" => deny_unused = true,
            "--config" => match value(&mut args) {
                Some(path) => config_path = path,
                None => return Ok(usage()),
            },
            "--baseline" => match value(&mut args) {
                Some(path) => baseline = Some(path),
                None => return Ok(usage()),
            },
            "--write-baseline" => match value(&mut args) {
                Some(path) => write_baseline = Some(path),
                None => return Ok(usage()),
            },
            _ => return Ok(usage()),
        }
    }
    if baseline.is_some() && write_baseline.is_some() {
        return Ok(usage());
    }
    let mut config = Config::load(&config_path)?;
    config.deny_unused |= deny_unused;
    let root = config_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let report = check(&config, root)?;
    if let Some(path) = write_baseline {
        if report.findings.iter().any(|f| f.rule == "parse") {
            print!("{}", report.render());
            eprintln!("design-lint: not writing a baseline while a file does not parse");
            return Ok(ExitCode::from(1));
        }
        let baseline = Baseline::from_report(&report);
        std::fs::write(&path, baseline.to_toml()).map_err(|source| Error::Io { path: path.clone(), source })?;
        println!("design-lint: wrote {} with {} findings", path.display(), baseline.total());
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(path) = baseline {
        let result = Baseline::load(&path)?.check(&report);
        print!("{}", result.render());
        return Ok(ExitCode::from(result.exit_code()));
    }
    print!("{}", if summary { report.render_with_table() } else { report.render() });
    Ok(ExitCode::from(report.exit_code()))
}
