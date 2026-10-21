use std::process::Command;

#[test]
fn json_stdout_is_one_document_even_for_unverified_consumers() {
    let missing = std::env::temp_dir().join(format!("missing-compat-consumer-{}", std::process::id()));
    assert!(!missing.exists());
    let report_file = missing.with_extension("json");
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["compat", "--consumer"])
        .arg(&missing)
        .args(["--target-pkg", "fixture", "--json", "--output"])
        .arg(&report_file)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let saved = std::fs::read(&report_file).unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&saved).unwrap(), report);
    std::fs::remove_file(&report_file).unwrap();
    assert_eq!(report["comparisons_completed"], 0);
    assert_ne!(report["overall_status"], "PASS");
    assert_eq!(report["cleanup_errors"], serde_json::json!([]));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Compatibility Summary"));
}

#[test]
fn malformed_cli_exits_before_compatibility_execution() {
    for option in ["--revision", "--consumer", "--target-pkg", "--consumers", "--consumers-dir", "--output"] {
        let out = Command::new(env!("CARGO_BIN_EXE_xtask")).args(["compat", option, "--json"]).output().unwrap();
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(error.contains("requires a value"));
        assert!(!error.contains("Craft Suite"));
    }
}

/// Exercise the full runner with a Git-pinned dependency and a standalone consumer.
/// Git URL rewriting keeps the fixture independent of the network and real upstream revisions.
#[test]
fn standalone_consumer_compares_all_executables_without_mutating_checkout() {
    use std::fs;
    use std::path::Path;
    let root = std::env::temp_dir().join(format!("craft-compat-e2e-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let remote = root.join("remote");
    let consumer = root.join("consumer");
    for repo in [&remote, &consumer] {
        fs::create_dir_all(repo.join("src")).unwrap();
        assert!(Command::new("git").arg("init").arg(repo).output().unwrap().status.success());
    }
    let commit = |repo: &Path| {
        assert!(Command::new("git").current_dir(repo).args(["add", "."]).output().unwrap().status.success());
        assert!(
            Command::new("git")
                .current_dir(repo)
                .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture"])
                .output()
                .unwrap()
                .status
                .success()
        );
        let out = Command::new("git").current_dir(repo).args(["rev-parse", "HEAD"]).output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    fs::write(remote.join("Cargo.toml"), "[package]\nname = \"craft-docpath\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
    fs::write(remote.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let revision = commit(&remote);
    let git_config = root.join("gitconfig");
    let file_url = format!("file:///{}", remote.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
    assert!(
        Command::new("git")
            .args(["config", "--file"])
            .arg(&git_config)
            .arg(format!("url.{file_url}.insteadOf"))
            .arg("https://github.com/storytold/craft-libs")
            .output()
            .unwrap()
            .status
            .success()
    );
    let prepare = |program: &str| {
        let mut command = Command::new(program);
        command
            .current_dir(&consumer)
            .env("CARGO_HOME", root.join("cargo-home"))
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("CARGO_NET_GIT_FETCH_WITH_CLI", "true");
        command
    };
    fs::write(consumer.join("Cargo.toml"), format!("[package]\nname = \"compat-e2e\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\ncraft-docpath = {{ git = \"https://github.com/storytold/craft-libs\", rev = \"{revision}\" }}\n")).unwrap();
    fs::write(consumer.join("src/lib.rs"), "#[test] fn unit() {}\n").unwrap();
    fs::create_dir_all(consumer.join("tests")).unwrap();
    fs::write(consumer.join("tests/a.rs"), "#[test] fn same() { assert_eq!(1, 2); }\n").unwrap();
    fs::write(consumer.join("tests/b.rs"), "#[test] fn same() {}\n").unwrap();
    fs::create_dir_all(consumer.join(".cargo")).unwrap();
    let config = "[build]\njobs = 1\n";
    fs::write(consumer.join(".cargo/config.toml"), config).unwrap();
    let lock = prepare("cargo").arg("generate-lockfile").output().unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    commit(&consumer);
    fs::write(consumer.join("Cargo.lock"), "uncommitted invalid lockfile\n").unwrap();
    let output = prepare(env!("CARGO_BIN_EXE_xtask"))
        .args(["compat", "--consumer"])
        .arg(&consumer)
        .args(["--target-pkg", "compat-e2e", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["overall_status"], "PASS");
    let result = &report["consumers"]["consumer"];
    assert_eq!(result["status"], "BASELINE_FAILURE");
    assert_eq!(result["baseline_passed"], 2);
    assert_eq!(result["candidate_passed"], 2);
    assert_eq!(result["baseline_failed"], 1);
    assert_eq!(result["candidate_failed"], 1);
    assert_eq!(report["comparisons_completed"], 1);
    assert_eq!(report["cleanup_errors"], serde_json::json!([]));
    assert_eq!(fs::read_to_string(consumer.join("Cargo.lock")).unwrap(), "uncommitted invalid lockfile\n");
    assert_eq!(fs::read_to_string(consumer.join(".cargo/config.toml")).unwrap(), config);
    let worktrees = Command::new("git").current_dir(&consumer).args(["worktree", "list", "--porcelain"]).output().unwrap();
    assert_eq!(String::from_utf8(worktrees.stdout).unwrap().matches("worktree ").count(), 1);
    fs::remove_dir_all(root).unwrap();
}
