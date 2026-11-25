//! Report the decoded samples and selected metadata of explicitly supplied RAW files.
//! `cargo run --locked -p craft-raw --example decode_report -- [--linearization] [--] <file> [<file> ...]`
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod report;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::io::{self, Write};
    use std::process::ExitCode;

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match report::run(std::env::args_os().skip(1), &mut out, report::MAX_INPUT_BYTES, &craft_raw::Limits::default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let stderr = io::stderr();
            let mut err = stderr.lock();
            match error {
                report::RunError::Usage => {
                    let _ = writeln!(err, "usage: decode_report [--linearization] [--] <file> [<file> ...]");
                }
                report::RunError::Output(error) => {
                    let _ = writeln!(err, "decode_report: report output failed ({:?})", error.kind());
                }
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
