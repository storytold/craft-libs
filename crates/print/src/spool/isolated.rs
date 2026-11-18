//! The Windows print helper process and its protocol. The module compiles under
//! `cfg(any(windows, test))` so its protocol trials run on every platform.

/// Runs the print helper when the process was started with `--craft-print-helper`: the app's
/// `main` calls it first and exits with the code when it answers `Some`. The helper's reply lines
/// start `CRAFT_PRINT:`.
#[cfg(windows)]
pub fn helper_main() -> Option<i32> {
    todo!("pf.C-PR3")
}
