//! Argument parsing and the output of each subcommand.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use craft_plugin_host::store::Source;
use serde_json::Value;

use crate::bindgen;
use crate::build::{self, BuildOutput};
use crate::devfolder::{DevEvent, DevFolder};
use crate::error::{CliError, Result, io_at};
use crate::gate;
use crate::inspect::{self, inspect_bytes};
use crate::new::{self, NewOptions, TemplateKind};
use crate::run::{DevHost, Headless, Installed, parse_grants, parse_invoke};
use crate::watch::{self, WatchOptions};

#[derive(Parser)]
#[command(name = "craft", about = "Developer tools for Craft plugins", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a plugin package from a template.
    New(NewArgs),
    /// Build a plugin for wasm32 and run the install gate on it.
    Build {
        /// The plugin package directory.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Cargo features to enable, comma separated. A package can hold two versions of a plugin behind a feature.
        #[arg(long, value_delimiter = ',', value_name = "FEATURES")]
        features: Vec<String>,
        /// Name the output target/<OUT_NAME>.wasm instead of target/<package>.wasm.
        #[arg(long, value_name = "OUT_NAME")]
        out_name: Option<String>,
    },
    /// Run the install gate on one or more .wasm files and print one verdict each. Exit 1 if any is rejected.
    Check {
        #[arg(required = true)]
        wasm: Vec<PathBuf>,
    },
    /// Print a plugin's manifest, ops, permissions and gate report.
    Inspect {
        wasm: PathBuf,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Load a plugin into a headless host and call one contribution.
    Run(RunArgs),
    /// Rebuild a plugin when its source changes.
    Watch {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Copy each accepted build into this folder, for an app in developer mode or `craft dev`.
        #[arg(long, value_name = "FOLDER")]
        install_into: Option<PathBuf>,
    },
    /// Developer mode in a headless host: load every .wasm in a folder and reload on change.
    Dev(DevArgs),
    /// Generate a typed Rust client from an app's command list.
    Bindgen(BindgenArgs),
}

#[derive(Args)]
struct NewArgs {
    /// Package name: lowercase letters, digits, '-'.
    name: String,
    /// Contribution kinds to include: command, hook, effect, panel. Comma separated.
    #[arg(long, default_value = "command", value_name = "KINDS")]
    kind: String,
    /// Parent directory. The package goes to <DIR>/<NAME>.
    #[arg(long, default_value = ".", value_name = "DIR")]
    dir: PathBuf,
    /// Manifest id. Default: dev.local.<NAME>.
    #[arg(long)]
    id: Option<String>,
    /// Path to crates/plugin-sdk. Default: the craft-libs checkout this craft was built from (fixed at build time).
    #[arg(long, value_name = "PATH")]
    sdk_path: Option<PathBuf>,
}

#[derive(Args)]
struct RunArgs {
    /// Required. The apps' own headless sessions are not wired in; this runs the plugin host with no app.
    #[arg(long)]
    headless: bool,
    /// The app name `host.info` reports.
    #[arg(long, default_value = "headless")]
    app: String,
    wasm: PathBuf,
    /// The contribution to call, and its JSON. A command takes its params. A hook or panel takes its whole request.
    /// The id may be the contribution id (`say`), or a dotted tail such as `hello.say`.
    #[arg(long, num_args = 1..=2, value_names = ["ID", "JSON"], required = true)]
    invoke: Vec<String>,
    /// Grant an optional permission. Required permissions are always granted in a headless run.
    #[arg(long, value_name = "PERMISSION")]
    grant: Vec<String>,
}

#[derive(Args)]
struct DevArgs {
    folder: PathBuf,
    #[arg(long, default_value = "headless")]
    app: String,
    /// After each load, call this contribution. Same meaning as `craft run --invoke`.
    #[arg(long, num_args = 1..=2, value_names = ["ID", "JSON"])]
    invoke: Vec<String>,
    #[arg(long, value_name = "PERMISSION")]
    grant: Vec<String>,
    /// Rescan interval in milliseconds, on top of file system events.
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,
}

#[derive(Args)]
struct BindgenArgs {
    /// The app name. Without --from, runs `<APP>-cli commands --json`, so that program must be on PATH.
    #[arg(long)]
    app: String,
    /// Read the command list from this JSON file instead (`-` for stdin).
    #[arg(long, value_name = "FILE")]
    from: Option<PathBuf>,
    /// Write the module here. Default: standard output.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

fn print_build(out: &BuildOutput) {
    let name = out.wasm.as_ref().unwrap_or(&out.cargo_artifact);
    print!("{}", inspect::render_text(&inspect_bytes(&name.display().to_string(), &out.bytes)));
    match &out.wasm {
        Some(wasm) => println!("\nwrote {}", wasm.display()),
        None => println!("\nnot written: the gate rejected the module (cargo output: {})", out.cargo_artifact.display()),
    }
}

fn cmd_new(a: NewArgs) -> Result<ExitCode> {
    let kinds = new::parse_kinds(&a.kind)?;
    let dir = new::create(&NewOptions { name: a.name.clone(), parent: a.dir, kinds: kinds.clone(), plugin_id: a.id, sdk_path: a.sdk_path })?;
    let names: Vec<&str> = kinds.iter().map(|k: &TemplateKind| k.name()).collect();
    println!("created {} ({})", dir.display(), names.join(", "));
    println!("next: cd {} && craft build", dir.display());
    Ok(ExitCode::SUCCESS)
}

fn cmd_build(dir: PathBuf, opts: build::BuildOptions) -> Result<ExitCode> {
    let out = build::build_with(&dir, &opts)?;
    print_build(&out);
    Ok(if out.accepted() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

fn cmd_inspect(wasm: PathBuf, json: bool) -> Result<ExitCode> {
    let i = inspect::inspect_file(&wasm)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inspect::to_json(&i))?);
    } else {
        print!("{}", inspect::render_text(&i));
    }
    Ok(if i.accepted() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

fn cmd_check(files: &[PathBuf]) -> Result<ExitCode> {
    let mut all_accepted = true;
    for path in files {
        match std::fs::read(path).map_err(|e| io_at(path, e)) {
            Ok(bytes) => {
                let report = gate::check(&bytes);
                all_accepted &= report.is_accepted();
                println!("{}: {}", path.display(), gate::summary(&report));
            }
            Err(e) => {
                all_accepted = false;
                eprintln!("craft: {e}");
            }
        }
    }
    Ok(if all_accepted { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

fn describe(installed: &Installed) -> String {
    let replaced = installed.replaced.map(|g| format!(", replacing generation {g}")).unwrap_or_default();
    let granted = match (installed.granted.is_empty(), installed.replaced) {
        (false, _) => format!("granted {}", installed.granted.join(", ")),
        (true, Some(_)) => "earlier grants kept".to_owned(),
        (true, None) => "no permission prompt".to_owned(),
    };
    format!("{} generation {}{replaced}; {granted}", installed.plugin_id, installed.generation)
}

fn print_result(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn cmd_run(a: RunArgs) -> Result<ExitCode> {
    if !a.headless {
        return Err(CliError::new("only --headless is implemented. Launching an app to host the plugin is not supported; add --headless"));
    }
    let (target, arg) = parse_invoke(&a.invoke)?;
    let grants = parse_grants(&a.grant)?;
    let bytes = std::fs::read(&a.wasm).map_err(|e| io_at(&a.wasm, e))?;
    let host = Headless::new(&a.app, true);
    let installed = host.install(&bytes, Source::File, &grants)?;
    eprintln!("craft: loaded {}", describe(&installed));
    let result = host.invoke(&installed.plugin_id, &target, &arg)?;
    print_result(&result)?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_watch(dir: PathBuf, install_into: Option<PathBuf>) -> Result<ExitCode> {
    if let Some(folder) = &install_into {
        std::fs::create_dir_all(folder).map_err(|e| io_at(folder, e))?;
    }
    let stop = AtomicBool::new(false);
    watch::watch(&WatchOptions { dir, install_into, debounce: Duration::from_millis(300) }, &stop, |result| match result {
        Ok(out) => {
            print_build(out);
            println!("--- waiting for changes");
        }
        Err(e) => eprintln!("craft: build failed: {e}\n--- waiting for changes"),
    })?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_dev(a: DevArgs) -> Result<ExitCode> {
    let grants = parse_grants(&a.grant)?;
    let invoke = if a.invoke.is_empty() { None } else { Some(parse_invoke(&a.invoke)?) };
    let host = Arc::new(Headless::new(&a.app, true));
    let mut folder = DevFolder::new(&a.folder, DevHost { host: Arc::clone(&host), optional: grants })?;
    eprintln!("craft: watching {} (Ctrl-C to stop)", a.folder.display());
    let stop = AtomicBool::new(false);
    folder.watch(&stop, Duration::from_millis(a.poll_ms), |events| {
        for event in events {
            match event {
                DevEvent::Loaded { path, installed } => {
                    eprintln!("craft: {} -> {}", path.display(), describe(&installed));
                    if let Some((target, arg)) = &invoke {
                        match host.invoke(&installed.plugin_id, target, arg) {
                            Ok(v) => {
                                if let Err(e) = print_result(&v) {
                                    eprintln!("craft: {e}");
                                }
                            }
                            Err(e) => eprintln!("craft: invoke {target}: {e}"),
                        }
                    }
                }
                DevEvent::Rejected { path, reason } => eprintln!("craft: {} rejected: {reason}", path.display()),
                DevEvent::Removed { path, plugin_id } => eprintln!("craft: {} deleted, removed {plugin_id}", path.display()),
                DevEvent::RemoveFailed { path, plugin_id, reason } => {
                    eprintln!("craft: {} deleted, cannot remove {plugin_id}: {reason}", path.display())
                }
            }
        }
    })?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_bindgen(a: BindgenArgs) -> Result<ExitCode> {
    let list = match &a.from {
        Some(path) if path.as_os_str() == "-" => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            serde_json::from_str(&text)?
        }
        Some(path) => serde_json::from_slice(&std::fs::read(path).map_err(|e| io_at(path, e))?)?,
        None => bindgen::fetch_headless(&a.app)?,
    };
    let rows = bindgen::parse_command_list(&list)?;
    let module = bindgen::generate(&a.app, &rows)?;
    match &a.out {
        Some(path) => {
            std::fs::write(path, &module).map_err(|e| io_at(path, e))?;
            eprintln!("craft: wrote {} commands to {}", rows.len(), path.display());
        }
        None => print!("{module}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::New(a) => cmd_new(a),
        Command::Build { dir, features, out_name } => cmd_build(dir, build::BuildOptions { features, out_name }),
        Command::Check { wasm } => cmd_check(&wasm),
        Command::Inspect { wasm, json } => cmd_inspect(wasm, json),
        Command::Run(a) => cmd_run(a),
        Command::Watch { dir, install_into } => cmd_watch(dir, install_into),
        Command::Dev(a) => cmd_dev(a),
        Command::Bindgen(a) => cmd_bindgen(a),
    }
}

pub fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if let Err(print_err) = err.print() {
                eprintln!("craft: {print_err}");
                return ExitCode::from(2);
            }
            return ExitCode::from(u8::try_from(err.exit_code()).unwrap_or(2));
        }
    };
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("craft: error: {e}");
            ExitCode::from(1)
        }
    }
}
