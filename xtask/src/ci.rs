use std::path::Path;
use std::process::Command;

pub fn run(root_dir: &Path) -> Result<(), String> {
    println!("Running craft-libs quality gates...\n");

    // 1. fmt check
    print!("1. Checking formatting (cargo fmt --check): ");
    let fmt_status =
        Command::new("cargo").current_dir(root_dir).args(["fmt", "--check"]).status().map_err(|e| format!("cargo fmt failed to spawn: {e}"))?;

    if !fmt_status.success() {
        return Err("cargo fmt --check failed".into());
    }
    println!("OK");

    // 2. clippy
    print!("2. Running Clippy (cargo clippy --workspace --all-targets -- -D warnings): ");
    let clippy_status = Command::new("cargo")
        .current_dir(root_dir)
        .args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
        .status()
        .map_err(|e| format!("cargo clippy failed to spawn: {e}"))?;

    if !clippy_status.success() {
        return Err("cargo clippy failed with warnings or errors".into());
    }
    println!("OK");

    // 3. test
    print!("3. Running tests (cargo test --workspace): ");
    let test_status =
        Command::new("cargo").current_dir(root_dir).args(["test", "--workspace"]).status().map_err(|e| format!("cargo test failed to spawn: {e}"))?;

    if !test_status.success() {
        return Err("cargo test --workspace failed".into());
    }
    println!("OK");

    println!("\nAll craft-libs quality gates passed.");
    Ok(())
}
