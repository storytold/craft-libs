use std::process::Command;

#[test]
fn generation_and_drift_check_handle_paths_with_spaces_without_modifying_drift() {
    let dir = std::env::temp_dir().join(format!("craft installer test {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("installer ui.wxs");
    let run = |mode| Command::new(env!("CARGO_BIN_EXE_craft-installer")).arg(mode).args(["--app", "gridcraft"]).arg(&path).output().unwrap();
    assert!(run("--write").status.success());
    assert!(run("--check").status.success());
    std::fs::write(&path, "local change").unwrap();
    assert!(!run("--check").status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "local change");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_profile_cannot_write_a_file() {
    let path = std::env::temp_dir().join(format!("craft-invalid-profile-{}.wxs", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_craft-installer")).args(["--write", "--app", "unknown-app"]).arg(&path).output().unwrap();
    assert!(!output.status.success());
    assert!(!path.exists());
}
