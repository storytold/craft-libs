//! Cross-application compatibility validation runner and report generator.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatConfig {
    pub candidate_revision: String,
    pub consumers: Vec<ConsumerTarget>,
    pub skip_baseline: bool,
    pub json_output: bool,
    pub output_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerTarget {
    pub name: String,
    pub repo_dir: PathBuf,
    pub target_package: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub version: String,
    pub candidate_revision: String,
    pub candidate_is_dirty: bool,
    pub comparisons_completed: usize,
    pub timestamp: String,
    pub overall_status: String,
    pub consumers: BTreeMap<String, ConsumerReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerReport {
    pub repository: String,
    pub target_package: String,
    pub consumer_commit: String,
    pub pinned_craft_crates: Vec<String>,
    pub baseline_result: String,
    pub baseline_passed: usize,
    pub baseline_failed: usize,
    pub candidate_result: String,
    pub candidate_passed: usize,
    pub candidate_failed: usize,
    pub status: String,
    pub duration_seconds: f64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompatStatus {
    Compatible,
    CandidateRegression,
    BaselineFailure,
    FixedInCandidate,
    VacuousExecutionRejected,
    InfrastructureFailure,
    IncompletePatchCoverage,
    Skipped,
}

impl fmt::Display for CompatStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompatStatus::Compatible => write!(f, "COMPATIBLE"),
            CompatStatus::CandidateRegression => write!(f, "CANDIDATE_REGRESSION"),
            CompatStatus::BaselineFailure => write!(f, "BASELINE_FAILURE"),
            CompatStatus::FixedInCandidate => write!(f, "FIXED_IN_CANDIDATE"),
            CompatStatus::VacuousExecutionRejected => write!(f, "VACUOUS_EXECUTION_REJECTED"),
            CompatStatus::InfrastructureFailure => write!(f, "INFRASTRUCTURE_FAILURE"),
            CompatStatus::IncompletePatchCoverage => write!(f, "INCOMPLETE_PATCH_COVERAGE"),
            CompatStatus::Skipped => write!(f, "SKIPPED"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestSummary {
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
}

pub struct WorktreeGuard {
    worktrees: Vec<(PathBuf, PathBuf)>, // (repo_dir, worktree_path)
}

impl WorktreeGuard {
    pub fn new() -> Self {
        Self { worktrees: Vec::new() }
    }

    pub fn prune_compat_worktrees(repo_dir: &Path) {
        let _ = Command::new("git").current_dir(repo_dir).args(["worktree", "prune"]).output();
    }

    pub fn create(&mut self, repo_dir: &Path, worktree_path: &Path, commit_ish: &str) -> Result<(), String> {
        Self::prune_compat_worktrees(repo_dir);

        if worktree_path.exists() {
            let _ = Self::remove(repo_dir, worktree_path);
        }

        let output = Command::new("git")
            .current_dir(repo_dir)
            .args(["worktree", "add", "--detach", &worktree_path.to_string_lossy(), commit_ish])
            .output()
            .map_err(|e| format!("failed to spawn git worktree add: {e}"))?;

        if !output.status.success() {
            return Err(format!("git worktree add failed in {}: {}", repo_dir.display(), String::from_utf8_lossy(&output.stderr).trim()));
        }

        self.worktrees.push((repo_dir.to_path_buf(), worktree_path.to_path_buf()));
        Ok(())
    }

    pub fn remove(repo_dir: &Path, worktree_path: &Path) -> Result<(), String> {
        let _ = Command::new("git").current_dir(repo_dir).args(["worktree", "remove", "--force", &worktree_path.to_string_lossy()]).output();

        if worktree_path.exists() {
            let _ = fs::remove_dir_all(worktree_path);
        }
        Self::prune_compat_worktrees(repo_dir);
        Ok(())
    }
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        for (repo_dir, worktree_path) in self.worktrees.drain(..) {
            let _ = Self::remove(&repo_dir, &worktree_path);
        }
    }
}

pub struct KnownConsumer {
    pub name: &'static str,
    pub dir_name: &'static str,
    pub default_target_pkg: &'static str,
}

pub const KNOWN_CONSUMERS: &[KnownConsumer] = &[
    KnownConsumer { name: "photocraft", dir_name: "photocraft", default_target_pkg: "photocraft-ui-egui" },
    KnownConsumer { name: "vectorcraft", dir_name: "vectorcraft", default_target_pkg: "vectorcraft-ui-egui" },
    KnownConsumer { name: "designcraft", dir_name: "designcraft", default_target_pkg: "designcraft-ui-egui" },
    KnownConsumer { name: "pdfcraft", dir_name: "pdfcraft", default_target_pkg: "pdfcraft-ui-egui" },
    KnownConsumer { name: "lightcraft", dir_name: "lightcraft", default_target_pkg: "lightcraft-ui-egui" },
    KnownConsumer { name: "filmcraft", dir_name: "filmcraft", default_target_pkg: "filmcraft-ui-egui" },
    KnownConsumer { name: "effectcraft", dir_name: "effectcraft", default_target_pkg: "effectcraft-ui-egui" },
    KnownConsumer { name: "cadcraft", dir_name: "cadcraft", default_target_pkg: "cadcraft-ui-egui" },
    KnownConsumer { name: "soundcraft", dir_name: "soundcraft", default_target_pkg: "soundcraft-ui-egui" },
];

pub fn check_git_dirty(dir: &Path) -> bool {
    Command::new("git").current_dir(dir).args(["status", "--porcelain"]).output().map(|o| !o.stdout.is_empty()).unwrap_or(false)
}

pub fn resolve_commit(repo_dir: &Path, commit_ish: &str) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(repo_dir)
        .args(["rev-parse", "--verify", &format!("{commit_ish}^{{commit}}")])
        .output()
        .map_err(|e| format!("git rev-parse failed: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "commit `{commit_ish}` could not be resolved in {}: {}",
            repo_dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn discover_candidate_crates(candidate_root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let crates_dir = candidate_root.join("crates");
    if !crates_dir.is_dir() {
        return Err(format!("crates/ directory not found in {}", candidate_root.display()));
    }

    let mut map = BTreeMap::new();
    let entries = fs::read_dir(&crates_dir).map_err(|e| format!("read crates dir: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let cargo_toml = path.join("Cargo.toml");
        if cargo_toml.is_file()
            && let Some(name) = fs::read_to_string(&cargo_toml).ok().and_then(|c| extract_package_name(&c))
        {
            map.insert(name, path);
        }
    }
    Ok(map)
}

fn extract_package_name(cargo_toml_content: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml_content.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && trimmed.starts_with('[') {
            break;
        }
        if in_package
            && trimmed.starts_with("name")
            && let Some((_, val)) = trimmed.split_once('=')
        {
            let name = val.trim().trim_matches('"').trim_matches('\'').trim();
            return Some(name.to_string());
        }
    }
    None
}

pub fn parse_pinned_craft_crates(lock_content: &str) -> Vec<String> {
    let mut pinned = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_source: Option<String> = None;

    for line in lock_content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if let (Some(name), Some(source)) = (current_name.take(), current_source.take()) {
                let is_craft = source.contains("storytold/craft-libs") || (name.starts_with("craft-") && source.contains("git+"));
                if is_craft {
                    pinned.push(name);
                }
            }
            continue;
        }
        if let Some((_, val)) = trimmed.split_once("name =") {
            let name = val.trim().trim_matches('"');
            current_name = Some(name.to_string());
        } else if let Some((_, val)) = trimmed.split_once("source =") {
            let source = val.trim().trim_matches('"');
            current_source = Some(source.to_string());
        }
    }
    if let (Some(name), Some(source)) = (current_name, current_source) {
        let is_craft = source.contains("storytold/craft-libs") || (name.starts_with("craft-") && source.contains("git+"));
        if is_craft {
            pinned.push(name);
        }
    }
    pinned.sort();
    pinned.dedup();
    pinned
}

pub fn generate_patch_config(pinned_crates: &[String], candidate_crates: &BTreeMap<String, PathBuf>) -> Result<String, Vec<String>> {
    let mut missing = Vec::new();
    let mut patch_entries = Vec::new();

    for pinned in pinned_crates {
        if let Some(crate_path) = candidate_crates.get(pinned) {
            // Forward slashes prevent Windows path escaping issues
            let path_str = crate_path.to_string_lossy().replace('\\', "/");
            patch_entries.push(format!("{pinned} = {{ path = \"{path_str}\" }}"));
        } else {
            missing.push(pinned.clone());
        }
    }

    if !missing.is_empty() {
        return Err(missing);
    }

    let mut toml = String::from("[patch.\"https://github.com/storytold/craft-libs\"]\n");
    for entry in patch_entries {
        toml.push_str(&entry);
        toml.push('\n');
    }
    Ok(toml)
}

pub fn verify_candidate_provenance(worktree_dir: &Path, target_pkg: &str, checked_crate: &str, candidate_path_snippet: &str) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(worktree_dir);
    cmd.args(["tree", "--offline", "-p", target_pkg, "-i", checked_crate]);

    let out = cmd.output().map_err(|e| format!("cargo tree failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo tree check failed for {checked_crate}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let normalized_snippet = candidate_path_snippet.replace('\\', "/");
    let normalized_stdout = stdout.replace('\\', "/");

    if !normalized_stdout.contains(&normalized_snippet) {
        return Err(format!("candidate provenance mismatch: expected `{}` in cargo tree output:\n{}", normalized_snippet, stdout));
    }

    Ok(())
}

pub fn parse_test_summary(output: &[u8]) -> TestSummary {
    let text = String::from_utf8_lossy(output);
    let mut summary = TestSummary::default();

    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("test result:") {
            // Format: "test result: ok. 1822 passed; 0 failed; 10 ignored; 0 measured; 0 filtered out; finished in 12.34s"
            for part in trimmed.split(';') {
                let part = part.trim();
                if part.contains("passed") {
                    if let Some(num_str) = part.split_whitespace().find(|w| w.chars().all(|c| c.is_ascii_digit())) {
                        summary.passed = num_str.parse().unwrap_or(0);
                    }
                } else if part.contains("failed") {
                    if let Some(num_str) = part.split_whitespace().find(|w| w.chars().all(|c| c.is_ascii_digit())) {
                        summary.failed = num_str.parse().unwrap_or(0);
                    }
                } else if part.contains("ignored") {
                    let num_str = part.split_whitespace().find(|w| w.chars().all(|c| c.is_ascii_digit()));
                    if let Some(num_str) = num_str {
                        summary.ignored = num_str.parse().unwrap_or(0);
                    }
                }
            }
            break;
        }
    }
    summary
}

pub fn classify_outcome(baseline: Option<&TestSummary>, candidate: &TestSummary) -> CompatStatus {
    if candidate.passed == 0 && candidate.failed == 0 {
        return CompatStatus::VacuousExecutionRejected;
    }

    match baseline {
        Some(base) => {
            if base.passed == 0 && base.failed == 0 {
                return CompatStatus::VacuousExecutionRejected;
            }

            if base.failed == 0 && candidate.failed == 0 {
                CompatStatus::Compatible
            } else if base.failed == 0 && candidate.failed > 0 {
                CompatStatus::CandidateRegression
            } else if base.failed > 0 && candidate.failed == 0 {
                CompatStatus::FixedInCandidate
            } else if candidate.failed > base.failed {
                // Fails more tests than baseline
                CompatStatus::CandidateRegression
            } else {
                // Exact same failures present in baseline
                CompatStatus::BaselineFailure
            }
        }
        None => {
            if candidate.failed == 0 {
                CompatStatus::Compatible
            } else {
                CompatStatus::CandidateRegression
            }
        }
    }
}

pub fn run_compat(root_dir: &Path, config: CompatConfig) -> Result<bool, String> {
    println!("Craft Suite Compatibility Verification");
    println!("=======================================================\n");

    let candidate_sha = resolve_commit(root_dir, &config.candidate_revision)?;
    let candidate_is_dirty = check_git_dirty(root_dir);

    println!("Target shared library: craft-libs ({candidate_sha})");
    println!("Working tree status:   {}", if candidate_is_dirty { "DIRTY" } else { "CLEAN" });

    let candidate_crates = discover_candidate_crates(root_dir)?;
    println!("Candidate crates:      {}\n", candidate_crates.keys().cloned().collect::<Vec<_>>().join(", "));

    let mut guard = WorktreeGuard::new();
    let compat_root = root_dir.join("target").join("compat");
    let worktrees_dir = compat_root.join("worktrees");
    fs::create_dir_all(&worktrees_dir).map_err(|e| format!("create worktrees dir: {e}"))?;

    let mut reports = BTreeMap::new();
    let mut any_regression = false;

    for target in &config.consumers {
        let start = Instant::now();
        println!("-------------------------------------------------------");
        println!("Validating consumer: {}", target.name);
        println!("-------------------------------------------------------");

        if !target.repo_dir.exists() {
            println!("  [SKIP] Consumer checkout not found at {}", target.repo_dir.display());
            reports.insert(
                target.name.clone(),
                ConsumerReport {
                    repository: target.repo_dir.to_string_lossy().to_string(),
                    target_package: target.target_package.clone().unwrap_or_else(|| "unknown".into()),
                    consumer_commit: "missing".into(),
                    pinned_craft_crates: Vec::new(),
                    baseline_result: "SKIP".into(),
                    baseline_passed: 0,
                    baseline_failed: 0,
                    candidate_result: "SKIP".into(),
                    candidate_passed: 0,
                    candidate_failed: 0,
                    status: CompatStatus::Skipped.to_string(),
                    duration_seconds: 0.0,
                    notes: Some("Repository directory not found".into()),
                },
            );
            continue;
        }

        let consumer_commit = match resolve_commit(&target.repo_dir, "HEAD") {
            Ok(c) => c,
            Err(e) => {
                println!("  [ERROR] Failed to resolve HEAD in {}: {e}", target.repo_dir.display());
                reports.insert(
                    target.name.clone(),
                    ConsumerReport {
                        repository: target.repo_dir.to_string_lossy().to_string(),
                        target_package: target.target_package.clone().unwrap_or_else(|| "unknown".into()),
                        consumer_commit: "unknown".into(),
                        pinned_craft_crates: Vec::new(),
                        baseline_result: "INFRASTRUCTURE_FAILURE".into(),
                        baseline_passed: 0,
                        baseline_failed: 0,
                        candidate_result: "INFRASTRUCTURE_FAILURE".into(),
                        candidate_passed: 0,
                        candidate_failed: 0,
                        status: CompatStatus::InfrastructureFailure.to_string(),
                        duration_seconds: start.elapsed().as_secs_f64(),
                        notes: Some(format!("resolve HEAD failed: {e}")),
                    },
                );
                continue;
            }
        };

        // Determine target package
        let target_package = match &target.target_package {
            Some(pkg) => pkg.clone(),
            None => match KNOWN_CONSUMERS.iter().find(|c| c.name == target.name) {
                Some(k) => k.default_target_pkg.to_string(),
                None => {
                    println!("  [ERROR] No target package specified for {}", target.name);
                    continue;
                }
            },
        };

        // Inspect consumer Cargo.lock for all pinned craft-libs crates
        let lock_path = target.repo_dir.join("Cargo.lock");
        let lock_content = match fs::read_to_string(&lock_path) {
            Ok(c) => c,
            Err(e) => {
                println!("  [ERROR] Failed to read Cargo.lock: {e}");
                continue;
            }
        };
        let pinned_crates = parse_pinned_craft_crates(&lock_content);
        println!("  Pinned craft crates:   {}", pinned_crates.join(", "));

        // Verify complete patch coverage
        let patch_toml = match generate_patch_config(&pinned_crates, &candidate_crates) {
            Ok(p) => p,
            Err(missing) => {
                let err_msg = format!("candidate is missing pinned crates: {}", missing.join(", "));
                println!("  [ERROR] Incomplete patch coverage: {err_msg}");
                reports.insert(
                    target.name.clone(),
                    ConsumerReport {
                        repository: target.repo_dir.to_string_lossy().to_string(),
                        target_package: target_package.clone(),
                        consumer_commit: consumer_commit.clone(),
                        pinned_craft_crates: pinned_crates.clone(),
                        baseline_result: "SKIP".into(),
                        baseline_passed: 0,
                        baseline_failed: 0,
                        candidate_result: "INCOMPLETE_PATCH_COVERAGE".into(),
                        candidate_passed: 0,
                        candidate_failed: 0,
                        status: CompatStatus::IncompletePatchCoverage.to_string(),
                        duration_seconds: start.elapsed().as_secs_f64(),
                        notes: Some(err_msg),
                    },
                );
                any_regression = true;
                continue;
            }
        };

        let run_id = format!("{}-{}", std::process::id(), Instant::now().elapsed().as_nanos());
        let consumer_worktree = worktrees_dir.join(format!("compat-wt-{}-{run_id}", target.name));

        println!("  Creating worktree:     {}", consumer_worktree.display());
        if let Err(e) = guard.create(&target.repo_dir, &consumer_worktree, "HEAD") {
            println!("  [ERROR] Worktree creation failed: {e}");
            reports.insert(
                target.name.clone(),
                ConsumerReport {
                    repository: target.repo_dir.to_string_lossy().to_string(),
                    target_package: target_package.clone(),
                    consumer_commit: consumer_commit.clone(),
                    pinned_craft_crates: pinned_crates.clone(),
                    baseline_result: "INFRASTRUCTURE_FAILURE".into(),
                    baseline_passed: 0,
                    baseline_failed: 0,
                    candidate_result: "INFRASTRUCTURE_FAILURE".into(),
                    candidate_passed: 0,
                    candidate_failed: 0,
                    status: CompatStatus::InfrastructureFailure.to_string(),
                    duration_seconds: start.elapsed().as_secs_f64(),
                    notes: Some(format!("worktree create failed: {e}")),
                },
            );
            any_regression = true;
            continue;
        }

        // Shared target directory avoids rebuilding external dependencies from scratch
        let shared_target = target.repo_dir.join("target");

        // 1. Run Baseline Tests (if not skipped)
        let mut baseline_summary = None;
        let mut baseline_status_str = "SKIPPED".to_string();

        if !config.skip_baseline {
            print!("  Running baseline tests: ");
            let out = Command::new("cargo")
                .current_dir(&consumer_worktree)
                .args(["test", "--locked", "-p", &target_package])
                .env("CARGO_TARGET_DIR", &shared_target)
                .output();

            match out {
                Ok(o) => {
                    let mut combined = o.stdout;
                    combined.extend_from_slice(&o.stderr);
                    let sum = parse_test_summary(&combined);
                    if o.status.success() {
                        println!("PASS ({} passed, {} failed)", sum.passed, sum.failed);
                        baseline_status_str = "PASS".into();
                    } else {
                        println!("FAIL ({} passed, {} failed)", sum.passed, sum.failed);
                        baseline_status_str = "FAIL".into();
                    }
                    baseline_summary = Some(sum);
                }
                Err(e) => {
                    println!("INFRASTRUCTURE_FAILURE ({e})");
                    baseline_status_str = "INFRASTRUCTURE_FAILURE".into();
                }
            }
        }

        // 2. Apply Complete Patch Configuration to worktree
        let cargo_config_dir = consumer_worktree.join(".cargo");
        fs::create_dir_all(&cargo_config_dir).ok();
        let cargo_config_file = cargo_config_dir.join("config.toml");
        if let Err(e) = fs::write(&cargo_config_file, &patch_toml) {
            println!("  [ERROR] Failed to write .cargo/config.toml: {e}");
            continue;
        }

        // 3. Verify Candidate Provenance via cargo tree
        let mut provenance_failed = false;
        for pinned in &pinned_crates {
            if let Some(crate_path) = candidate_crates.get(pinned) {
                let snippet = crate_path.file_name().unwrap_or_default().to_string_lossy();
                if let Err(e) = verify_candidate_provenance(&consumer_worktree, &target_package, pinned, &snippet) {
                    println!("  [ERROR] Provenance check failed for {pinned}: {e}");
                    provenance_failed = true;
                    break;
                }
            }
        }

        if provenance_failed {
            reports.insert(
                target.name.clone(),
                ConsumerReport {
                    repository: target.repo_dir.to_string_lossy().to_string(),
                    target_package: target_package.clone(),
                    consumer_commit: consumer_commit.clone(),
                    pinned_craft_crates: pinned_crates.clone(),
                    baseline_result: baseline_status_str,
                    baseline_passed: baseline_summary.as_ref().map(|s| s.passed).unwrap_or(0),
                    baseline_failed: baseline_summary.as_ref().map(|s| s.failed).unwrap_or(0),
                    candidate_result: "INFRASTRUCTURE_FAILURE".into(),
                    candidate_passed: 0,
                    candidate_failed: 0,
                    status: CompatStatus::InfrastructureFailure.to_string(),
                    duration_seconds: start.elapsed().as_secs_f64(),
                    notes: Some("provenance verification failed".into()),
                },
            );
            any_regression = true;
            continue;
        }

        println!("  Provenance verified:   all pinned crates resolve to candidate paths");

        // 4. Run Candidate Tests
        print!("  Running candidate tests: ");
        let cand_out = Command::new("cargo")
            .current_dir(&consumer_worktree)
            .args(["test", "-p", &target_package])
            .env("CARGO_TARGET_DIR", &shared_target)
            .output();

        let (candidate_status_str, candidate_summary) = match cand_out {
            Ok(o) => {
                let mut combined = o.stdout;
                combined.extend_from_slice(&o.stderr);
                let sum = parse_test_summary(&combined);
                if o.status.success() {
                    println!("PASS ({} passed, {} failed)", sum.passed, sum.failed);
                    ("PASS".to_string(), sum)
                } else {
                    println!("FAIL ({} passed, {} failed)", sum.passed, sum.failed);
                    ("FAIL".to_string(), sum)
                }
            }
            Err(e) => {
                println!("INFRASTRUCTURE_FAILURE ({e})");
                ("INFRASTRUCTURE_FAILURE".to_string(), TestSummary::default())
            }
        };

        // 5. Classify Outcome
        let outcome = classify_outcome(baseline_summary.as_ref(), &candidate_summary);
        println!("  Outcome:               {outcome}\n");

        if matches!(outcome, CompatStatus::CandidateRegression | CompatStatus::VacuousExecutionRejected | CompatStatus::InfrastructureFailure) {
            any_regression = true;
        }

        reports.insert(
            target.name.clone(),
            ConsumerReport {
                repository: target.repo_dir.to_string_lossy().to_string(),
                target_package,
                consumer_commit,
                pinned_craft_crates: pinned_crates,
                baseline_result: baseline_status_str,
                baseline_passed: baseline_summary.as_ref().map(|s| s.passed).unwrap_or(0),
                baseline_failed: baseline_summary.as_ref().map(|s| s.failed).unwrap_or(0),
                candidate_result: candidate_status_str,
                candidate_passed: candidate_summary.passed,
                candidate_failed: candidate_summary.failed,
                status: outcome.to_string(),
                duration_seconds: start.elapsed().as_secs_f64(),
                notes: None,
            },
        );
    }

    let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => format!("{}", d.as_secs()),
        Err(_) => "0".into(),
    };

    let comparisons_completed = reports
        .values()
        .filter(|r| matches!(r.status.as_str(), "COMPATIBLE" | "CANDIDATE_REGRESSION" | "BASELINE_FAILURE" | "FIXED_IN_CANDIDATE"))
        .count();

    let overall_status = if any_regression {
        "FAIL_REGRESSION"
    } else if comparisons_completed == 0 {
        "VACUOUS_SUCCESS_REJECTED"
    } else {
        "PASS"
    };

    let report = CompatibilityReport {
        version: "2.0".into(),
        candidate_revision: candidate_sha,
        candidate_is_dirty,
        comparisons_completed,
        timestamp,
        overall_status: overall_status.into(),
        consumers: reports,
    };

    println!("=======================================================");
    println!("Compatibility Summary");
    println!("=======================================================");
    println!("{:<14} {:<10} {:<10} {:<24}", "Consumer", "Baseline", "Candidate", "Status");
    println!("{:-<14} {:-<10} {:-<10} {:-<24}", "", "", "", "");
    for (name, r) in &report.consumers {
        println!("{:<14} {:<10} {:<10} {:<24}", name, r.baseline_result, r.candidate_result, r.status);
    }

    let report_json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialize report: {e}"))?;
    if let Some(path) = config.output_file {
        fs::write(&path, &report_json).map_err(|e| format!("write report: {e}"))?;
        println!("\nReport written to: {}", path.display());
    }

    if config.json_output {
        println!("\n```json\n{report_json}\n```");
    }

    Ok(!any_regression && comparisons_completed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pinned_craft_crates_multi() {
        let sample_lock = r#"
version = 4

[[package]]
name = "craft-ui"
version = "0.1.0"
source = "git+https://github.com/storytold/craft-libs.git?rev=5c874d51eeb3de70e735c8ce87a59a7dedd47675#5c874d51eeb3de70e735c8ce87a59a7dedd47675"

[[package]]
name = "craft-atomic"
version = "0.1.0"
source = "git+https://github.com/storytold/craft-libs.git?rev=5c874d51eeb3de70e735c8ce87a59a7dedd47675#5c874d51eeb3de70e735c8ce87a59a7dedd47675"

[[package]]
name = "egui"
version = "0.31.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let pinned = parse_pinned_craft_crates(sample_lock);
        assert_eq!(pinned, vec!["craft-atomic", "craft-ui"]);
    }

    #[test]
    fn test_incomplete_patch_coverage_detection() {
        let pinned = vec!["craft-ui".to_string(), "craft-settings".to_string(), "craft-atomic".to_string()];
        let mut candidate_crates = BTreeMap::new();
        candidate_crates.insert("craft-ui".to_string(), PathBuf::from("/path/to/craft-ui"));
        candidate_crates.insert("craft-atomic".to_string(), PathBuf::from("/path/to/craft-atomic"));
        // missing craft-settings

        let err = generate_patch_config(&pinned, &candidate_crates).unwrap_err();
        assert_eq!(err, vec!["craft-settings".to_string()]);
    }

    #[test]
    fn test_generate_patch_config_windows_path_normalization() {
        let pinned = vec!["craft-ui".to_string()];
        let mut candidate_crates = BTreeMap::new();
        candidate_crates.insert("craft-ui".to_string(), PathBuf::from(r"C:\Users\builder\crates\ui"));

        let toml = generate_patch_config(&pinned, &candidate_crates).expect("patch config");
        assert!(toml.contains("craft-ui = { path = \"C:/Users/builder/crates/ui\" }"));
        assert!(!toml.contains(r"C:\Users\builder"));
    }

    #[test]
    fn test_classification_passing_baseline_and_candidate() {
        let base = TestSummary { passed: 100, failed: 0, ignored: 0 };
        let cand = TestSummary { passed: 100, failed: 0, ignored: 0 };
        assert_eq!(classify_outcome(Some(&base), &cand), CompatStatus::Compatible);
    }

    #[test]
    fn test_classification_candidate_regression() {
        let base = TestSummary { passed: 100, failed: 0, ignored: 0 };
        let cand = TestSummary { passed: 98, failed: 2, ignored: 0 };
        assert_eq!(classify_outcome(Some(&base), &cand), CompatStatus::CandidateRegression);
    }

    #[test]
    fn test_classification_failing_baseline_same_failures() {
        let base = TestSummary { passed: 98, failed: 2, ignored: 0 };
        let cand = TestSummary { passed: 98, failed: 2, ignored: 0 };
        assert_eq!(classify_outcome(Some(&base), &cand), CompatStatus::BaselineFailure);
    }

    #[test]
    fn test_classification_failing_baseline_with_candidate_regression() {
        // Base had 1 failure; candidate introduced 3 failures (fails on top)
        let base = TestSummary { passed: 99, failed: 1, ignored: 0 };
        let cand = TestSummary { passed: 97, failed: 3, ignored: 0 };
        assert_eq!(classify_outcome(Some(&base), &cand), CompatStatus::CandidateRegression);
    }

    #[test]
    fn test_classification_fixed_in_candidate() {
        let base = TestSummary { passed: 95, failed: 5, ignored: 0 };
        let cand = TestSummary { passed: 100, failed: 0, ignored: 0 };
        assert_eq!(classify_outcome(Some(&base), &cand), CompatStatus::FixedInCandidate);
    }

    #[test]
    fn test_classification_vacuous_rejected() {
        let base = TestSummary { passed: 0, failed: 0, ignored: 0 };
        let cand = TestSummary { passed: 0, failed: 0, ignored: 0 };
        assert_eq!(classify_outcome(Some(&base), &cand), CompatStatus::VacuousExecutionRejected);
    }

    #[test]
    fn test_worktree_guard_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("test-compat-wt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let mut guard = WorktreeGuard::new();
        let wt_path = temp_dir.join("sub-wt");
        fs::create_dir_all(&wt_path).unwrap();
        guard.worktrees.push((temp_dir.clone(), wt_path.clone()));

        drop(guard);
        assert!(!wt_path.exists());
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
