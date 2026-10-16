#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ffi::OsStr;
use std::process::ExitCode;

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next();
    let flag = args.next();
    let app = args.next();
    let path = args.next();
    let (Some(mode), Some(flag), Some(app), Some(path)) = (mode, flag, app, path) else {
        return Err("usage: craft-installer --write|--check --app <repository-name> <installer-ui.wxs>".into());
    };
    if flag != OsStr::new("--app") || args.next().is_some() {
        return Err("expected --app, an app name and exactly one output path".into());
    }
    let app = app.to_str().ok_or("app name must be UTF-8")?.parse()?;
    if mode == OsStr::new("--write") {
        craft_installer::write_windows_ui(app, &path).map_err(|e| e.to_string())
    } else if mode == OsStr::new("--check") {
        if craft_installer::check_windows_ui(app, &path).map_err(|e| e.to_string())? {
            Ok(())
        } else {
            Err("installer UI differs from craft-libs; regenerate with --write".into())
        }
    } else {
        Err("expected --write or --check".into())
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
