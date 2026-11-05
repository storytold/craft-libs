#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Developer CLI. See `craft --help`.

fn main() -> std::process::ExitCode {
    craft_cli::cli::main()
}
