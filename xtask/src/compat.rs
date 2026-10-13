//! Cross-application compatibility validation runner and report generator.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatConfig {
    pub candidate_revision: String,
    pub consumers: Vec<String>,
    pub consumers_dir: PathBuf,
    pub skip_baseline: bool,
    pub dry_run: bool,
    pub all_targets: bool,
    pub quick: bool,
    pub clean_head_only: bool,
    pub json_output: bool,
    pub output_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainInfo {
    pub rustc: String,
    pub cargo: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub version: String,
    pub candidate_revision: String,
    pub candidate_is_dirty: bool,
    pub candidate_provenance: String,
    pub comparisons_completed: usize,
    pub timestamp: String,
    pub toolchain: ToolchainInfo,
    pub overall_status: String,
    pub consumers: BTreeMap<String, ConsumerReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerReport {
    pub repository: String,
    pub target_package: String,
    pub consumer_commit: String,
    pub consumer_is_dirty: bool,
    pub original_dependency_pin: String,
    pub candidate_revision: String,
    pub candidate_is_dirty: bool,
    pub resolved_candidate_dependency: String,
    pub baseline_command: Option<String>,
    pub baseline_exit_code: Option<i32>,
    pub baseline_result: String,
    pub baseline_duration_seconds: f64,
    pub candidate_command: String,
    pub candidate_exit_code: Option<i32>,
    pub candidate_result: String,
    pub candidate_duration_seconds: f64,
    pub status: String,
    pub total_duration_seconds: f64,
    pub log_file: Option<String>,
    pub notes: Option<String>,
}

pub struct WorktreeGuard {
    worktrees: Vec<(PathBuf, PathBuf)>, // (repo_dir, worktree_path)
    cleanup_dirs: Vec<PathBuf>,
}

impl WorktreeGuard {
    pub fn new() -> Self {
        Self { worktrees: Vec::new(), cleanup_dirs: Vec::new() }
    }

    pub fn register_cleanup_dir(&mut self, dir: PathBuf) {
        self.cleanup_dirs.push(dir);
    }

    pub fn prune(repo_dir: &Path) {
        let _ = Command::new("git").current_dir(repo_dir).args(["worktree", "prune"]).output();
    }

    pub fn create(&mut self, repo_dir: &Path, worktree_path: &Path, commit_ish: &str) -> Result<(), String> {
        Self::prune(repo_dir);

        if worktree_path.exists() {
            let _ = Self::remove_worktree(repo_dir, worktree_path);
        }
        let output = Command::new("git")
            .current_dir(repo_dir)
            .args(["worktree", "add", "--detach", &worktree_path.to_string_lossy(), commit_ish])
            .output()
            .map_err(|e| format!("Failed to spawn git worktree add: {e}"))?;

        if !output.status.success() {
            return Err(format!("git worktree add failed in {}: {}", repo_dir.display(), String::from_utf8_lossy(&output.stderr).trim()));
        }

        self.worktrees.push((repo_dir.to_path_buf(), worktree_path.to_path_buf()));
        Ok(())
    }

    fn remove_worktree(repo_dir: &Path, worktree_path: &Path) -> Result<(), String> {
        let _ = Command::new("git").current_dir(repo_dir).args(["worktree", "remove", "--force", &worktree_path.to_string_lossy()]).output();

        if worktree_path.exists() {
            let _ = fs::remove_dir_all(worktree_path);
        }
        Self::prune(repo_dir);
        Ok(())
    }
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        for (repo_dir, worktree_path) in self.worktrees.drain(..) {
            let _ = Self::remove_worktree(&repo_dir, &worktree_path);
        }
        for dir in self.cleanup_dirs.drain(..) {
            if dir.exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }
}

pub struct ConsumerSpec {
    pub name: &'static str,
    pub default_repo_dir: &'static str,
    pub ui_crate: &'static str,
}

pub const KNOWN_CONSUMERS: &[ConsumerSpec] = &[
    ConsumerSpec { name: "photocraft", default_repo_dir: "photocraft", ui_crate: "photocraft-ui-egui" },
    ConsumerSpec { name: "vectorcraft", default_repo_dir: "vectorcraft", ui_crate: "vectorcraft-ui-egui" },
    ConsumerSpec { name: "designcraft", default_repo_dir: "designcraft", ui_crate: "designcraft-ui-egui" },
    ConsumerSpec { name: "pdfcraft", default_repo_dir: "pdfcraft", ui_crate: "pdfcraft-ui-egui" },
    ConsumerSpec { name: "lightcraft", default_repo_dir: "lightcraft", ui_crate: "lightcraft-ui-egui" },
    ConsumerSpec { name: "filmcraft", default_repo_dir: "filmcraft", ui_crate: "filmcraft-ui-egui" },
    ConsumerSpec { name: "effectcraft", default_repo_dir: "effectcraft", ui_crate: "effectcraft-ui-egui" },
    ConsumerSpec { name: "cadcraft", default_repo_dir: "cadcraft", ui_crate: "cadcraft-ui-egui" },
    ConsumerSpec { name: "soundcraft", default_repo_dir: "soundcraft", ui_crate: "soundcraft-ui-egui" },
];

pub fn get_toolchain_info() -> ToolchainInfo {
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let cargo = Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let target = Command::new("rustc")
        .args(["-vV"])
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            s.lines().find(|l| l.starts_with("host:")).map(|l| l.trim_start_matches("host:").trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());

    ToolchainInfo { rustc, cargo, target }
}

pub fn check_git_dirty(dir: &Path) -> bool {
    Command::new("git").current_dir(dir).args(["status", "--porcelain"]).output().map(|o| !o.stdout.is_empty()).unwrap_or(false)
}

pub fn resolve_commit(repo_dir: &Path, commit_ish: &str) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(repo_dir)
        .args(["rev-parse", "--verify", &format!("{commit_ish}^{{commit}}")])
        .output()
        .map_err(|e| format!("git rev-parse failed to spawn: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "Commit `{commit_ish}` could not be resolved in {}: {}",
            repo_dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn inspect_resolved_craft_ui(dir: &Path, target_pkg: Option<&str>, config_override: Option<&str>) -> Result<String, String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(dir);
    cmd.arg("tree");
    if config_override.is_none() {
        cmd.arg("--locked");
    } else {
        cmd.arg("--offline");
    }
    if let Some(pkg) = target_pkg {
        cmd.args(["-p", pkg]);
    }
    cmd.args(["-i", "craft-ui"]);
    if let Some(cfg) = config_override {
        cmd.args(["--config", cfg]);
    }

    let out = cmd.output().map_err(|e| format!("cargo tree failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo tree returned error: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Some(first_line) = stdout.lines().find(|l| l.contains("craft-ui")) {
        Ok(first_line.trim().to_string())
    } else {
        Err("craft-ui not found in cargo tree output".into())
    }
}

fn extract_test_summary(stdout: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(stdout);
    for line in s.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("test result:") {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn create_dirty_snapshot(root_dir: &Path) -> Option<String> {
    // Attempt isolated index snapshotting to capture untracked files, staged changes,
    // and unstaged changes without altering the developer's working directory or primary index.
    let temp_index = std::env::temp_dir().join(format!("craft-compat-index-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos()));
    let _ = fs::remove_file(&temp_index);

    let snapshot_res = (|| {
        // Stage everything into isolated temporary index (respects .gitignore)
        let add_status = Command::new("git").current_dir(root_dir).env("GIT_INDEX_FILE", &temp_index).args(["add", "-A"]).status().ok()?;
        if !add_status.success() {
            return None;
        }

        // Write tree from isolated index
        let write_tree_out = Command::new("git").current_dir(root_dir).env("GIT_INDEX_FILE", &temp_index).args(["write-tree"]).output().ok()?;
        if !write_tree_out.status.success() {
            return None;
        }
        let tree_sha = String::from_utf8_lossy(&write_tree_out.stdout).trim().to_string();
        if tree_sha.len() != 40 {
            return None;
        }

        // Commit tree pointing to HEAD as parent
        let commit_tree_out = Command::new("git")
            .current_dir(root_dir)
            .args(["commit-tree", &tree_sha, "-p", "HEAD", "-m", "compat dirty snapshot (including untracked)"])
            .output()
            .ok()?;
        if !commit_tree_out.status.success() {
            return None;
        }
        let commit_sha = String::from_utf8_lossy(&commit_tree_out.stdout).trim().to_string();
        if commit_sha.len() == 40 { Some(commit_sha) } else { None }
    })();

    let _ = fs::remove_file(&temp_index);
    snapshot_res
}

pub fn run_compat(root_dir: &Path, config: CompatConfig) -> Result<bool, String> {
    println!("Craft Suite Compatibility Validation");
    println!("=======================================================\n");

    let head_sha = resolve_commit(root_dir, "HEAD")?;
    let candidate_sha = resolve_commit(root_dir, &config.candidate_revision)?;
    let candidate_is_dirty = check_git_dirty(root_dir);

    if candidate_is_dirty && !config.clean_head_only && candidate_sha != head_sha {
        return Err(format!(
            "Working tree is dirty but requested --revision ({}) does not match HEAD ({}). Commit/stash uncommitted changes, or specify --clean-head-only.",
            config.candidate_revision, head_sha
        ));
    }

    let (candidate_target_commit, candidate_provenance) = if candidate_is_dirty {
        if config.clean_head_only {
            (candidate_sha.clone(), format!("clean_head_dirty_ignored ({candidate_sha})"))
        } else if let Some(snapshot) = create_dirty_snapshot(root_dir) {
            (snapshot.clone(), format!("dirty_snapshot ({snapshot})"))
        } else {
            return Err("Failed to create isolated dirty snapshot of uncommitted working tree changes.".into());
        }
    } else {
        (candidate_sha.clone(), format!("clean_commit ({candidate_sha})"))
    };

    println!("Target shared library: craft-libs ({})", candidate_sha);
    println!("Candidate provenance:  {}", candidate_provenance);
    if candidate_is_dirty && !config.clean_head_only {
        println!("Working tree status:   DIRTY (uncommitted changes snapshotted into candidate worktree)");
    } else if candidate_is_dirty {
        println!("Working tree status:   DIRTY (testing committed HEAD; local uncommitted changes ignored)");
    } else {
        println!("Working tree status:   CLEAN");
    }

    let toolchain = get_toolchain_info();
    println!("Rust toolchain:        {}", toolchain.rustc);
    println!("Cargo version:         {}", toolchain.cargo);
    println!("Host target:           {}", toolchain.target);
    println!("Consumers to test:     {}\n", config.consumers.join(", "));

    let mut guard = WorktreeGuard::new();
    let compat_root = root_dir.join("target").join("compat");
    let worktrees_dir = compat_root.join("worktrees");
    let logs_dir = compat_root.join("logs");
    let run_id = format!("{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let run_worktrees_dir = worktrees_dir.join(&run_id);
    let run_logs_dir = logs_dir.join(&run_id);
    fs::create_dir_all(&run_worktrees_dir).map_err(|e| format!("create run worktrees dir: {e}"))?;
    fs::create_dir_all(&run_logs_dir).map_err(|e| format!("create run logs dir: {e}"))?;
    guard.register_cleanup_dir(run_worktrees_dir.clone());

    // Prune stale worktrees from root_dir
    WorktreeGuard::prune(root_dir);

    // Create isolated worktree for candidate craft-libs revision
    let candidate_craft_libs_dir = run_worktrees_dir.join(format!("craft-libs-{}", &candidate_target_commit[0..8]));
    println!("Creating candidate craft-libs worktree at {}", candidate_craft_libs_dir.display());
    guard.create(root_dir, &candidate_craft_libs_dir, &candidate_target_commit)?;
    let candidate_ui_crate_path = candidate_craft_libs_dir.join("crates").join("ui");

    if !candidate_ui_crate_path.exists() {
        return Err(format!("crates/ui does not exist in candidate revision {}", candidate_target_commit));
    }

    let patch_config = format!("patch.\"https://github.com/storytold/craft-libs\".craft-ui.path=\"{}\"", candidate_ui_crate_path.display());

    let mut consumer_reports = BTreeMap::new();
    let mut any_regression = false;

    for consumer_name in &config.consumers {
        let start_time = Instant::now();
        println!("-------------------------------------------------------");
        println!("Validating consumer: {consumer_name}");
        println!("-------------------------------------------------------");

        let spec = match KNOWN_CONSUMERS.iter().find(|c| c.name == consumer_name) {
            Some(s) => s,
            None => {
                eprintln!("Unknown consumer: {consumer_name} (skipping)");
                consumer_reports.insert(
                    consumer_name.clone(),
                    ConsumerReport {
                        repository: consumer_name.clone(),
                        target_package: "unknown".into(),
                        consumer_commit: "unknown".into(),
                        consumer_is_dirty: false,
                        original_dependency_pin: "unknown".into(),
                        candidate_revision: candidate_sha.clone(),
                        candidate_is_dirty,
                        resolved_candidate_dependency: "unknown".into(),
                        baseline_command: None,
                        baseline_exit_code: None,
                        baseline_result: "SKIP".into(),
                        baseline_duration_seconds: 0.0,
                        candidate_command: "none".into(),
                        candidate_exit_code: None,
                        candidate_result: "SKIP".into(),
                        candidate_duration_seconds: 0.0,
                        status: "SKIPPED".into(),
                        total_duration_seconds: 0.0,
                        log_file: None,
                        notes: Some(format!("Unknown consumer name `{consumer_name}`")),
                    },
                );
                continue;
            }
        };

        let repo_dir = config.consumers_dir.join(spec.default_repo_dir);
        if !repo_dir.exists() {
            println!("  [SKIP] Consumer checkout not found at {}", repo_dir.display());
            consumer_reports.insert(
                consumer_name.clone(),
                ConsumerReport {
                    repository: spec.default_repo_dir.into(),
                    target_package: spec.ui_crate.into(),
                    consumer_commit: "missing".into(),
                    consumer_is_dirty: false,
                    original_dependency_pin: "missing".into(),
                    candidate_revision: candidate_sha.clone(),
                    candidate_is_dirty,
                    resolved_candidate_dependency: "missing".into(),
                    baseline_command: None,
                    baseline_exit_code: None,
                    baseline_result: "SKIP".into(),
                    baseline_duration_seconds: 0.0,
                    candidate_command: "none".into(),
                    candidate_exit_code: None,
                    candidate_result: "SKIP".into(),
                    candidate_duration_seconds: 0.0,
                    status: "SKIPPED".into(),
                    total_duration_seconds: 0.0,
                    log_file: None,
                    notes: Some(format!("Repository directory not found at {}", repo_dir.display())),
                },
            );
            continue;
        }

        let consumer_is_dirty = check_git_dirty(&repo_dir);
        let baseline_commit = match resolve_commit(&repo_dir, "HEAD") {
            Ok(c) => c,
            Err(e) => {
                println!("  [ERROR] Failed to get HEAD of {}: {e}", repo_dir.display());
                consumer_reports.insert(
                    consumer_name.clone(),
                    ConsumerReport {
                        repository: spec.default_repo_dir.into(),
                        target_package: spec.ui_crate.into(),
                        consumer_commit: "unknown".into(),
                        consumer_is_dirty,
                        original_dependency_pin: "unknown".into(),
                        candidate_revision: candidate_sha.clone(),
                        candidate_is_dirty,
                        resolved_candidate_dependency: "unknown".into(),
                        baseline_command: None,
                        baseline_exit_code: None,
                        baseline_result: "INFRASTRUCTURE_FAILURE".into(),
                        baseline_duration_seconds: 0.0,
                        candidate_command: "none".into(),
                        candidate_exit_code: None,
                        candidate_result: "INFRASTRUCTURE_FAILURE".into(),
                        candidate_duration_seconds: 0.0,
                        status: "INFRASTRUCTURE_FAILURE".into(),
                        total_duration_seconds: start_time.elapsed().as_secs_f64(),
                        log_file: None,
                        notes: Some(format!("Failed to resolve HEAD: {e}")),
                    },
                );
                continue;
            }
        };

        // 1. Inspect and test Baseline in an isolated worktree
        let baseline_worktree_dir = run_worktrees_dir.join(format!("{consumer_name}-base-{}", &baseline_commit[0..8]));
        println!("  Creating baseline worktree: {}", baseline_worktree_dir.display());
        if let Err(e) = guard.create(&repo_dir, &baseline_worktree_dir, "HEAD") {
            println!("  [ERROR] Baseline worktree creation failed: {e}");
            consumer_reports.insert(
                consumer_name.clone(),
                ConsumerReport {
                    repository: spec.default_repo_dir.into(),
                    target_package: spec.ui_crate.into(),
                    consumer_commit: baseline_commit,
                    consumer_is_dirty,
                    original_dependency_pin: "worktree-creation-failed".into(),
                    candidate_revision: candidate_sha.clone(),
                    candidate_is_dirty,
                    resolved_candidate_dependency: "none".into(),
                    baseline_command: None,
                    baseline_exit_code: None,
                    baseline_result: "INFRASTRUCTURE_FAILURE".into(),
                    baseline_duration_seconds: 0.0,
                    candidate_command: "none".into(),
                    candidate_exit_code: None,
                    candidate_result: "INFRASTRUCTURE_FAILURE".into(),
                    candidate_duration_seconds: 0.0,
                    status: "INFRASTRUCTURE_FAILURE".into(),
                    total_duration_seconds: start_time.elapsed().as_secs_f64(),
                    log_file: None,
                    notes: Some(format!("Baseline worktree creation failed (isolated execution required): {e}")),
                },
            );
            continue;
        }

        let baseline_resolved = match inspect_resolved_craft_ui(&baseline_worktree_dir, Some(spec.ui_crate), None) {
            Ok(r) => r,
            Err(e) => {
                println!("  [WARN] Failed to inspect baseline craft-ui: {e}");
                format!("INSPECT_FAILED ({e})")
            }
        };

        println!("  Baseline commit:       {}{}", &baseline_commit[0..8], if consumer_is_dirty { " (DIRTY)" } else { "" });
        println!("  Baseline dependency:   {}", baseline_resolved);

        let shared_target_dir = repo_dir.join("target");
        let mut test_args: Vec<String> = if config.all_targets {
            vec!["test".into(), "--locked".into(), "-p".into(), spec.ui_crate.to_string(), "--all-targets".into()]
        } else {
            vec!["test".into(), "--locked".into(), "-p".into(), spec.ui_crate.to_string(), "--lib".into()]
        };

        if config.quick {
            test_args.extend(["--config".into(), "profile.dev.package.\"*\".opt-level=0".into()]);
        }

        let baseline_start = Instant::now();
        let mut baseline_cmd_str = None;
        let mut baseline_code = None;
        let baseline_result = if config.skip_baseline {
            println!("  Baseline execution:    SKIPPED (--skip-baseline)");
            "SKIPPED".to_string()
        } else if config.dry_run {
            println!("  Baseline execution:    SKIPPED (dry run)");
            "DRY_RUN".to_string()
        } else {
            let cmd_text = format!("cargo {}", test_args.join(" "));
            baseline_cmd_str = Some(cmd_text);
            print!("  Running baseline tests: ");
            let out = Command::new("cargo").current_dir(&baseline_worktree_dir).args(&test_args).env("CARGO_TARGET_DIR", &shared_target_dir).output();

            match out {
                Ok(o) if o.status.success() => {
                    if let Some(summary) = extract_test_summary(&o.stdout) {
                        println!("PASS ({summary})");
                    } else {
                        println!("PASS");
                    }
                    baseline_code = o.status.code();
                    fs::write(run_logs_dir.join(format!("{consumer_name}-baseline.log")), &o.stdout).ok();
                    "PASS".to_string()
                }
                Ok(o) => {
                    let mut log = o.stdout;
                    log.extend_from_slice(&o.stderr);
                    if let Some(summary) = extract_test_summary(&log) {
                        println!("FAIL ({summary})");
                    } else {
                        println!("FAIL (exit code: {:?})", o.status.code());
                    }
                    baseline_code = o.status.code();
                    fs::write(run_logs_dir.join(format!("{consumer_name}-baseline.log")), &log).ok();
                    "FAIL".to_string()
                }
                Err(e) => {
                    println!("INFRASTRUCTURE_FAILURE ({e})");
                    "INFRASTRUCTURE_FAILURE".to_string()
                }
            }
        };
        let baseline_duration = baseline_start.elapsed().as_secs_f64();

        // 2. Candidate isolated test
        let cand_worktree_dir = run_worktrees_dir.join(format!("{consumer_name}-cand-{}", &candidate_sha[0..8]));
        println!("  Creating candidate worktree: {}", cand_worktree_dir.display());
        if let Err(e) = guard.create(&repo_dir, &cand_worktree_dir, "HEAD") {
            println!("  [ERROR] Worktree creation failed: {e}");
            consumer_reports.insert(
                consumer_name.clone(),
                ConsumerReport {
                    repository: spec.default_repo_dir.into(),
                    target_package: spec.ui_crate.into(),
                    consumer_commit: baseline_commit,
                    consumer_is_dirty,
                    original_dependency_pin: baseline_resolved,
                    candidate_revision: candidate_sha.clone(),
                    candidate_is_dirty,
                    resolved_candidate_dependency: "worktree-creation-failed".into(),
                    baseline_command: baseline_cmd_str,
                    baseline_exit_code: baseline_code,
                    baseline_result,
                    baseline_duration_seconds: baseline_duration,
                    candidate_command: "none".into(),
                    candidate_exit_code: None,
                    candidate_result: "INFRASTRUCTURE_FAILURE".into(),
                    candidate_duration_seconds: 0.0,
                    status: "INFRASTRUCTURE_FAILURE".into(),
                    total_duration_seconds: start_time.elapsed().as_secs_f64(),
                    log_file: None,
                    notes: Some(format!("Worktree creation failed: {e}")),
                },
            );
            continue;
        }

        // 3. Strict provenance verification: target package must resolve to candidate path
        let expected_snippet = candidate_ui_crate_path.to_string_lossy();
        let candidate_resolved = match inspect_resolved_craft_ui(&cand_worktree_dir, Some(spec.ui_crate), Some(&patch_config)) {
            Ok(r) => {
                if !r.contains(expected_snippet.as_ref()) {
                    println!("  [ERROR] Provenance mismatch: expected '{}' in '{}'", expected_snippet, r);
                    format!("PROVENANCE_MISMATCH ({r})")
                } else {
                    r
                }
            }
            Err(e) => {
                println!("  [ERROR] Candidate dependency resolution failed: {e}");
                format!("RESOLUTION_FAILED ({e})")
            }
        };
        println!("  Candidate dependency:  {}", candidate_resolved);

        let candidate_start = Instant::now();
        let mut cand_args: Vec<String> = test_args.iter().map(|arg| if arg == "--locked" { "--offline".to_string() } else { arg.clone() }).collect();
        cand_args.extend(["--config".into(), patch_config.clone()]);
        let cand_cmd_text = format!("cargo {}", cand_args.join(" "));
        let mut candidate_code = None;
        let mut log_path = None;

        let candidate_result = if candidate_resolved.starts_with("PROVENANCE_MISMATCH") || candidate_resolved.starts_with("RESOLUTION_FAILED") {
            "INFRASTRUCTURE_FAILURE".to_string()
        } else if config.dry_run {
            println!("  Candidate execution:   SKIPPED (dry run)");
            "DRY_RUN".to_string()
        } else {
            print!("  Running candidate tests: ");
            let out = Command::new("cargo").current_dir(&cand_worktree_dir).args(&cand_args).env("CARGO_TARGET_DIR", &shared_target_dir).output();

            let log_file_name = format!("{consumer_name}-candidate.log");
            let target_log = run_logs_dir.join(&log_file_name);
            log_path = Some(target_log.to_string_lossy().to_string());

            match out {
                Ok(o) if o.status.success() => {
                    if let Some(summary) = extract_test_summary(&o.stdout) {
                        println!("PASS ({summary})");
                    } else {
                        println!("PASS");
                    }
                    candidate_code = o.status.code();
                    fs::write(&target_log, &o.stdout).ok();
                    "PASS".to_string()
                }
                Ok(o) => {
                    let mut log = o.stdout;
                    log.extend_from_slice(&o.stderr);
                    if let Some(summary) = extract_test_summary(&log) {
                        println!("FAIL ({summary})");
                    } else {
                        println!("FAIL (exit code: {:?})", o.status.code());
                    }
                    candidate_code = o.status.code();
                    fs::write(&target_log, &log).ok();
                    "FAIL".to_string()
                }
                Err(e) => {
                    println!("INFRASTRUCTURE_FAILURE ({e})");
                    "INFRASTRUCTURE_FAILURE".to_string()
                }
            }
        };
        let candidate_duration = candidate_start.elapsed().as_secs_f64();

        // Classify outcome
        let status = classify_status(&baseline_result, &candidate_result);
        if status == "CANDIDATE_REGRESSION" {
            any_regression = true;
        }

        let total_duration = start_time.elapsed().as_secs_f64();
        println!("  Outcome:               {}\n", status);

        let notes = if status == "FIXED_IN_CANDIDATE" {
            Some("Observed outcome: baseline failed on committed pin and passed on candidate; causality should be verified against environmental variance or test flakiness.".into())
        } else {
            None
        };

        consumer_reports.insert(
            consumer_name.clone(),
            ConsumerReport {
                repository: spec.default_repo_dir.into(),
                target_package: spec.ui_crate.into(),
                consumer_commit: baseline_commit,
                consumer_is_dirty,
                original_dependency_pin: baseline_resolved,
                candidate_revision: candidate_sha.clone(),
                candidate_is_dirty,
                resolved_candidate_dependency: candidate_resolved,
                baseline_command: baseline_cmd_str,
                baseline_exit_code: baseline_code,
                baseline_result,
                baseline_duration_seconds: baseline_duration,
                candidate_command: cand_cmd_text,
                candidate_exit_code: candidate_code,
                candidate_result,
                candidate_duration_seconds: candidate_duration,
                status: status.to_string(),
                total_duration_seconds: total_duration,
                log_file: log_path,
                notes,
            },
        );
    }

    let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => format!("{}", d.as_secs()),
        Err(_) => "0".into(),
    };

    let comparisons_completed = consumer_reports
        .values()
        .filter(|r| matches!(r.status.as_str(), "COMPATIBLE" | "CANDIDATE_REGRESSION" | "BASELINE_FAILURE" | "FIXED_IN_CANDIDATE"))
        .count();

    let provenance_verified = consumer_reports.values().filter(|r| r.status == "PROVENANCE_VERIFIED").count();

    let overall_status = if any_regression {
        "FAIL_REGRESSION"
    } else if consumer_reports.values().any(|r| r.status == "INFRASTRUCTURE_FAILURE") {
        "INFRASTRUCTURE_FAILURE"
    } else if config.dry_run {
        if provenance_verified == 0 { "VACUOUS_SUCCESS_REJECTED" } else { "PASS_PROVENANCE_VERIFIED" }
    } else if comparisons_completed == 0 {
        "VACUOUS_SUCCESS_REJECTED"
    } else {
        "PASS"
    };

    let report = CompatibilityReport {
        version: "1.0".into(),
        candidate_revision: candidate_sha,
        candidate_is_dirty,
        candidate_provenance,
        comparisons_completed,
        timestamp,
        toolchain,
        overall_status: overall_status.into(),
        consumers: consumer_reports,
    };

    let report_json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialize report: {e}"))?;
    let report_path = config.output_file.unwrap_or_else(|| compat_root.join("report.json"));
    fs::write(&report_path, &report_json).map_err(|e| format!("write report: {e}"))?;

    println!("=======================================================");
    println!("Compatibility Summary");
    println!("=======================================================");
    println!("{:<14} {:<10} {:<10} {:<22}", "Consumer", "Baseline", "Candidate", "Status");
    println!("{:-<14} {:-<10} {:-<10} {:-<22}", "", "", "", "");
    for (name, r) in &report.consumers {
        println!("{:<14} {:<10} {:<10} {:<22}", name, r.baseline_result, r.candidate_result, r.status);
    }
    println!("\nReport written to: {}", report_path.display());

    if config.json_output {
        println!("\n```json\n{report_json}\n```");
    }

    if overall_status == "VACUOUS_SUCCESS_REJECTED" {
        if config.dry_run {
            eprintln!("\nError: Compatibility dry-run completed with 0 verified consumer dependencies (all consumers were missing or skipped).");
        } else {
            eprintln!(
                "\nError: Compatibility validation completed with 0 successful comparisons (all consumers were skipped, missing, or unvalidated)."
            );
            eprintln!("Non-vacuous success policy requires at least one completed comparison (or specify --dry-run with available consumers).");
        }
        return Ok(false);
    }

    Ok(!any_regression && (overall_status == "PASS" || overall_status == "PASS_PROVENANCE_VERIFIED"))
}

pub fn classify_status(baseline_result: &str, candidate_result: &str) -> &'static str {
    match (baseline_result, candidate_result) {
        ("PASS", "PASS") => "COMPATIBLE",
        ("PASS", "FAIL") => "CANDIDATE_REGRESSION",
        ("FAIL", "FAIL") => "BASELINE_FAILURE",
        ("FAIL", "PASS") => "FIXED_IN_CANDIDATE",
        ("SKIPPED", "PASS") => "COMPATIBLE",
        ("SKIPPED", "FAIL") => "CANDIDATE_REGRESSION",
        ("SKIPPED", "SKIPPED") => "SKIPPED",
        ("DRY_RUN", "DRY_RUN") | ("SKIPPED", "DRY_RUN") => "PROVENANCE_VERIFIED",
        (b, c) if b == "INFRASTRUCTURE_FAILURE" || c == "INFRASTRUCTURE_FAILURE" => "INFRASTRUCTURE_FAILURE",
        (b, c) if b == "INCOMPLETE" || c == "INCOMPLETE" => "INCOMPLETE",
        _ => "INFRASTRUCTURE_FAILURE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toolchain_info_non_empty() {
        let info = get_toolchain_info();
        assert!(!info.rustc.is_empty());
        assert!(!info.cargo.is_empty());
        assert!(!info.target.is_empty());
    }

    #[test]
    fn test_resolve_valid_and_invalid_commit() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let valid = resolve_commit(&root, "HEAD");
        assert!(valid.is_ok());
        let sha = valid.unwrap();
        assert_eq!(sha.len(), 40);

        let invalid = resolve_commit(&root, "0000000000000000000000000000000000000000");
        assert!(invalid.is_err());
    }

    #[test]
    fn test_classify_status_matrix() {
        assert_eq!(classify_status("PASS", "PASS"), "COMPATIBLE");
        assert_eq!(classify_status("PASS", "FAIL"), "CANDIDATE_REGRESSION");
        assert_eq!(classify_status("FAIL", "FAIL"), "BASELINE_FAILURE");
        assert_eq!(classify_status("FAIL", "PASS"), "FIXED_IN_CANDIDATE");
        assert_eq!(classify_status("SKIPPED", "PASS"), "COMPATIBLE");
        assert_eq!(classify_status("SKIPPED", "FAIL"), "CANDIDATE_REGRESSION");
        assert_eq!(classify_status("DRY_RUN", "DRY_RUN"), "PROVENANCE_VERIFIED");
        assert_eq!(classify_status("SKIPPED", "DRY_RUN"), "PROVENANCE_VERIFIED");
        assert_eq!(classify_status("INFRASTRUCTURE_FAILURE", "PASS"), "INFRASTRUCTURE_FAILURE");
        assert_eq!(classify_status("PASS", "INFRASTRUCTURE_FAILURE"), "INFRASTRUCTURE_FAILURE");
        assert_eq!(classify_status("ERROR", "PASS"), "INFRASTRUCTURE_FAILURE");
    }

    #[test]
    fn test_worktree_guard_lifecycle() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let wt = std::env::temp_dir().join(format!("craft-wt-test-{}", std::process::id()));
        if wt.exists() {
            let _ = fs::remove_dir_all(&wt);
        }

        let mut guard = WorktreeGuard::new();
        let created = guard.create(&root, &wt, "HEAD");
        assert!(created.is_ok());
        assert!(wt.exists());

        drop(guard);
        assert!(!wt.exists());
    }

    #[test]
    fn test_non_vacuous_evaluation() {
        let mut reports = BTreeMap::new();
        reports.insert(
            "photocraft".to_string(),
            ConsumerReport {
                repository: "photocraft".into(),
                target_package: "photocraft-ui-egui".into(),
                consumer_commit: "missing".into(),
                consumer_is_dirty: false,
                original_dependency_pin: "missing".into(),
                candidate_revision: "HEAD".into(),
                candidate_is_dirty: false,
                resolved_candidate_dependency: "missing".into(),
                baseline_command: None,
                baseline_exit_code: None,
                baseline_result: "SKIP".into(),
                baseline_duration_seconds: 0.0,
                candidate_command: "none".into(),
                candidate_exit_code: None,
                candidate_result: "SKIP".into(),
                candidate_duration_seconds: 0.0,
                status: "SKIPPED".into(),
                total_duration_seconds: 0.0,
                log_file: None,
                notes: None,
            },
        );

        let comparisons = reports
            .values()
            .filter(|r| matches!(r.status.as_str(), "COMPATIBLE" | "CANDIDATE_REGRESSION" | "BASELINE_FAILURE" | "FIXED_IN_CANDIDATE"))
            .count();

        assert_eq!(comparisons, 0, "No comparisons completed when all are skipped");
    }

    #[test]
    fn test_dirty_snapshot_exact_coverage() {
        let tmp = std::env::temp_dir().join(format!("craft-compat-snap-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        struct TempGuard(PathBuf);
        impl Drop for TempGuard {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _guard = TempGuard(tmp.clone());
        let td = &tmp;

        // Initialize a test git repo
        Command::new("git").current_dir(td).args(["init"]).output().unwrap();
        Command::new("git").current_dir(td).args(["config", "user.name", "Compat Test"]).output().unwrap();
        Command::new("git").current_dir(td).args(["config", "user.email", "test@craft.local"]).output().unwrap();

        // 1. Setup .gitignore and initial tracked files
        fs::write(td.join(".gitignore"), "target/\n*.tmp\n").unwrap();
        fs::write(td.join("tracked.txt"), "initial content\n").unwrap();
        fs::write(td.join("to_delete.txt"), "content to be deleted\n").unwrap();
        fs::write(td.join("build.sh"), "#!/bin/sh\necho building\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(td.join("build.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        }

        Command::new("git").current_dir(td).args(["add", ".gitignore", "tracked.txt", "to_delete.txt", "build.sh"]).output().unwrap();
        Command::new("git").current_dir(td).args(["commit", "-m", "initial commit"]).output().unwrap();

        // 2. Introduce dirty state:
        // a. Untracked file (required by build)
        fs::write(td.join("untracked_module.rs"), "pub fn candidate_helper() -> bool { true }\n").unwrap();
        // b. Modified tracked file
        fs::write(td.join("tracked.txt"), "modified candidate content\n").unwrap();
        // c. Deleted tracked file
        fs::remove_file(td.join("to_delete.txt")).unwrap();
        // d. Ignored file (should NOT be snapshotted)
        fs::write(td.join("ignored.tmp"), "scratch data\n").unwrap();
        fs::create_dir_all(td.join("target")).unwrap();
        fs::write(td.join("target").join("build_artifact"), "binary blob\n").unwrap();

        let snapshot_sha = create_dirty_snapshot(td).expect("dirty snapshot must succeed");
        assert_eq!(snapshot_sha.len(), 40);

        // 3. Inspect tree of the created snapshot commit
        let ls_tree_out = Command::new("git").current_dir(td).args(["ls-tree", "-r", &snapshot_sha]).output().unwrap();
        let tree_listing = String::from_utf8_lossy(&ls_tree_out.stdout);

        // Assert inclusions
        assert!(tree_listing.contains("untracked_module.rs"), "Snapshot must include untracked source files: {tree_listing}");
        assert!(tree_listing.contains("tracked.txt"), "Snapshot must include modified tracked files: {tree_listing}");
        assert!(tree_listing.contains(".gitignore"), "Snapshot must include .gitignore: {tree_listing}");

        // Assert exclusions
        assert!(!tree_listing.contains("to_delete.txt"), "Snapshot must respect deleted tracked files: {tree_listing}");
        assert!(!tree_listing.contains("ignored.tmp"), "Snapshot must exclude files matching .gitignore: {tree_listing}");
        assert!(!tree_listing.contains("build_artifact"), "Snapshot must exclude ignored target directory: {tree_listing}");

        // Assert permissions preservation on Unix
        #[cfg(unix)]
        assert!(
            tree_listing.contains("100755") && tree_listing.contains("build.sh"),
            "Executable mode must be preserved in snapshot: {tree_listing}"
        );

        // 4. Verify checkout fidelity: checking out the snapshot tree reproduces exact files
        let test_checkout = td.join("checkout_verify");
        fs::create_dir_all(&test_checkout).unwrap();
        Command::new("git").current_dir(td).args(["worktree", "add", test_checkout.to_str().unwrap(), &snapshot_sha]).output().unwrap();

        assert_eq!(fs::read_to_string(test_checkout.join("tracked.txt")).unwrap(), "modified candidate content\n");
        assert!(test_checkout.join("untracked_module.rs").exists());
        assert!(!test_checkout.join("to_delete.txt").exists());
        assert!(!test_checkout.join("ignored.tmp").exists());
        assert!(!test_checkout.join("target/build_artifact").exists());

        Command::new("git").current_dir(td).args(["worktree", "remove", "--force", test_checkout.to_str().unwrap()]).output().unwrap();

        // 5. Verify the original working directory status and primary index were untouched
        let status_out = Command::new("git").current_dir(td).args(["status", "--porcelain"]).output().unwrap();
        let status = String::from_utf8_lossy(&status_out.stdout);
        assert!(status.contains("?? untracked_module.rs"), "Untracked file must remain untracked in repo status: {status}");
        assert!(status.contains(" M tracked.txt"), "Tracked file must remain unstaged modified in repo status: {status}");
        assert!(status.contains(" D to_delete.txt"), "Deleted file must remain unstaged deletion in repo status: {status}");
    }

    #[test]
    fn test_dry_run_vacuous_rejected_when_empty() {
        let mut reports = BTreeMap::new();
        reports.insert(
            "photocraft".to_string(),
            ConsumerReport {
                repository: "photocraft".into(),
                target_package: "photocraft-ui-egui".into(),
                consumer_commit: "missing".into(),
                consumer_is_dirty: false,
                original_dependency_pin: "missing".into(),
                candidate_revision: "HEAD".into(),
                candidate_is_dirty: false,
                resolved_candidate_dependency: "missing".into(),
                baseline_command: None,
                baseline_exit_code: None,
                baseline_result: "SKIP".into(),
                baseline_duration_seconds: 0.0,
                candidate_command: "none".into(),
                candidate_exit_code: None,
                candidate_result: "SKIP".into(),
                candidate_duration_seconds: 0.0,
                status: "SKIPPED".into(),
                total_duration_seconds: 0.0,
                log_file: None,
                notes: None,
            },
        );

        let provenance_verified = reports.values().filter(|r| r.status == "PROVENANCE_VERIFIED").count();
        assert_eq!(provenance_verified, 0, "No provenance verified when consumer is skipped");
    }
}
