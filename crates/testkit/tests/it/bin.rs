//! Tests of `craft_testkit::bin`. The runner tests start this package's `craft-testkit-fixture`
//! binary (`tests/fixture/child.rs`).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use craft_testkit::bin::{self, BinPathError, OutputLimitExceeded, Rejection, Source, Stream, resolve_binary_in};
use craft_testkit::bin_path;

const FIXTURE: &str = "craft-testkit-fixture";

/// A folder of its own for one test in the system's temporary folder, removed when the value is
/// dropped. The folder is found when the test runs, so a test run from a relocated nextest archive
/// does not use the build machine's folders.
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(test: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("craft-testkit-bin-{}-{test}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn file(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"").unwrap();
    path
}

fn lookup(vars: &[(&str, &OsString)]) -> impl FnMut(&str) -> Option<OsString> {
    let vars: HashMap<String, OsString> = vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).clone())).collect();
    move |name| vars.get(name).cloned()
}

fn candidates(error: BinPathError) -> Vec<(Source, String, Rejection)> {
    match error {
        BinPathError::NotFound { candidates, .. } => candidates.into_iter().map(|c| (c.source, c.variable, c.rejection)).collect(),
        BinPathError::InvalidName(name) => panic!("unexpected invalid name {name:?}"),
    }
}

fn fixture() -> PathBuf {
    bin_path!("craft-testkit-fixture")
}

fn args(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn limit_error(error: &std::io::Error) -> OutputLimitExceeded {
    *error.get_ref().and_then(|inner| inner.downcast_ref::<OutputLimitExceeded>()).unwrap_or_else(|| panic!("not an output limit error: {error}"))
}

// Binary lookup.

#[test]
fn cli_binary_prefers_relocated_runner_path() {
    let dir = scratch("cli_binary_prefers_relocated_runner_path");
    let relocated = file(&dir, "relocated");
    let built = file(&dir, "built");
    let relocated_value = relocated.clone().into_os_string();
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[("NEXTEST_BIN_EXE_my_program", &relocated_value)])).unwrap();
    assert_eq!(found, relocated);
}

#[test]
fn cli_binary_defaults_to_cargo_path() {
    let dir = scratch("cli_binary_defaults_to_cargo_path");
    let built = file(&dir, "built");
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[])).unwrap();
    assert_eq!(found, built);
}

#[test]
fn cli_executable_is_resolved() {
    let path = fixture();
    assert!(path.is_file(), "{} is not a file", path.display());
}

#[test]
fn hyphenated_name_reads_each_variable_in_order() {
    let mut read = Vec::new();
    let error = resolve_binary_in("my-program", None, |name| {
        read.push(name.to_owned());
        None
    })
    .unwrap_err();
    assert_eq!(read, ["NEXTEST_BIN_EXE_my-program", "CARGO_BIN_EXE_my-program", "NEXTEST_BIN_EXE_my_program"]);
    assert_eq!(
        candidates(error),
        [
            (Source::NextestExact, "NEXTEST_BIN_EXE_my-program".to_owned(), Rejection::NotSet),
            (Source::CargoRuntime, "CARGO_BIN_EXE_my-program".to_owned(), Rejection::NotSet),
            (Source::NextestUnderscore, "NEXTEST_BIN_EXE_my_program".to_owned(), Rejection::NotSet),
            (Source::CargoCompileTime, "CARGO_BIN_EXE_my-program".to_owned(), Rejection::NotSet),
        ]
    );
}

#[test]
fn name_without_hyphen_reads_the_nextest_variable_once() {
    let mut read = Vec::new();
    let _ = resolve_binary_in("tool", None, |name| {
        read.push(name.to_owned());
        None
    });
    assert_eq!(read, ["NEXTEST_BIN_EXE_tool", "CARGO_BIN_EXE_tool"]);
}

#[test]
fn nextest_exact_name_precedes_underscore() {
    let dir = scratch("nextest_exact_name_precedes_underscore");
    let underscore = file(&dir, "underscore").into_os_string();
    let hyphen = file(&dir, "hyphen").into_os_string();
    let runtime = file(&dir, "runtime").into_os_string();
    let found = resolve_binary_in(
        "my-program",
        None,
        lookup(&[("NEXTEST_BIN_EXE_my_program", &underscore), ("NEXTEST_BIN_EXE_my-program", &hyphen), ("CARGO_BIN_EXE_my-program", &runtime)]),
    )
    .unwrap();
    assert_eq!(found.into_os_string(), hyphen);
}

#[test]
fn runtime_cargo_precedes_nextest_underscore() {
    let dir = scratch("runtime_cargo_precedes_nextest_underscore");
    let underscore = file(&dir, "underscore").into_os_string();
    let runtime = file(&dir, "runtime").into_os_string();
    let found = resolve_binary_in("my-program", None, lookup(&[("NEXTEST_BIN_EXE_my_program", &underscore), ("CARGO_BIN_EXE_my-program", &runtime)]))
        .unwrap();
    assert_eq!(found.into_os_string(), runtime);
}

#[test]
fn hyphen_and_underscore_binaries_resolve_to_their_own_files() {
    // A package with both `cli-tool` and `cli_tool`: the underscore variable names `cli_tool`.
    let dir = scratch("hyphen_and_underscore_binaries_resolve_to_their_own_files");
    let hyphen = file(&dir, "cli-tool").into_os_string();
    let underscore = file(&dir, "cli_tool").into_os_string();
    let vars = [
        ("NEXTEST_BIN_EXE_cli-tool", &hyphen),
        ("NEXTEST_BIN_EXE_cli_tool", &underscore),
        ("CARGO_BIN_EXE_cli-tool", &hyphen),
        ("CARGO_BIN_EXE_cli_tool", &underscore),
    ];
    assert_eq!(resolve_binary_in("cli-tool", None, lookup(&vars)).unwrap().into_os_string(), hyphen);
    assert_eq!(resolve_binary_in("cli_tool", None, lookup(&vars)).unwrap().into_os_string(), underscore);
    // Cargo alone sets only the exact names.
    let cargo = [("CARGO_BIN_EXE_cli-tool", &hyphen), ("CARGO_BIN_EXE_cli_tool", &underscore)];
    assert_eq!(resolve_binary_in("cli-tool", None, lookup(&cargo)).unwrap().into_os_string(), hyphen);
    assert_eq!(resolve_binary_in("cli_tool", None, lookup(&cargo)).unwrap().into_os_string(), underscore);
}

#[test]
fn nextest_hyphen_fallback() {
    let dir = scratch("nextest_hyphen_fallback");
    let hyphen = file(&dir, "hyphen").into_os_string();
    let runtime = file(&dir, "runtime").into_os_string();
    let found =
        resolve_binary_in("my-program", None, lookup(&[("NEXTEST_BIN_EXE_my-program", &hyphen), ("CARGO_BIN_EXE_my-program", &runtime)])).unwrap();
    assert_eq!(found.into_os_string(), hyphen);
}

#[test]
fn runtime_cargo_after_nextest() {
    let dir = scratch("runtime_cargo_after_nextest");
    let runtime = file(&dir, "runtime").into_os_string();
    let built = file(&dir, "built");
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[("CARGO_BIN_EXE_my-program", &runtime)])).unwrap();
    assert_eq!(found.into_os_string(), runtime);
}

#[test]
fn resolver_returns_error_without_candidates() {
    let error = resolve_binary_in("my-program", None, lookup(&[])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("`my-program`"), "{message}");
    for variable in ["NEXTEST_BIN_EXE_my_program", "NEXTEST_BIN_EXE_my-program", "CARGO_BIN_EXE_my-program", "(compile time)"] {
        assert!(message.contains(variable), "{variable} missing from: {message}");
    }
}

#[test]
fn empty_candidate_falls_through() {
    let dir = scratch("empty_candidate_falls_through");
    let built = file(&dir, "built");
    let empty = OsString::new();
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[("NEXTEST_BIN_EXE_my_program", &empty)])).unwrap();
    assert_eq!(found, built);
}

#[test]
fn relative_candidate_falls_through() {
    let dir = scratch("relative_candidate_falls_through");
    let built = file(&dir, "built");
    // `Cargo.toml` exists relative to the test's working folder, the package root.
    let relative = OsString::from("Cargo.toml");
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[("NEXTEST_BIN_EXE_my_program", &relative)])).unwrap();
    assert_eq!(found, built);
}

#[test]
fn missing_candidate_falls_through() {
    let dir = scratch("missing_candidate_falls_through");
    let built = file(&dir, "built");
    let missing = dir.join("gone").into_os_string();
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[("NEXTEST_BIN_EXE_my_program", &missing)])).unwrap();
    assert_eq!(found, built);
}

#[test]
fn directory_candidate_falls_through() {
    let dir = scratch("directory_candidate_falls_through");
    let built = file(&dir, "built");
    let folder = dir.as_os_str().to_owned();
    let found = resolve_binary_in("my-program", built.to_str(), lookup(&[("NEXTEST_BIN_EXE_my_program", &folder)])).unwrap();
    assert_eq!(found, built);
}

#[test]
fn all_candidates_invalid_names_reasons() {
    let dir = scratch("all_candidates_invalid_names_reasons");
    let empty = OsString::new();
    let relative = OsString::from("target/debug/my-program");
    let folder = dir.as_os_str().to_owned();
    let built = dir.join("built");
    let error = resolve_binary_in(
        "my-program",
        built.to_str(),
        lookup(&[("NEXTEST_BIN_EXE_my_program", &empty), ("NEXTEST_BIN_EXE_my-program", &relative), ("CARGO_BIN_EXE_my-program", &folder)]),
    )
    .unwrap_err();
    let message = error.to_string();
    assert_eq!(
        candidates(error).into_iter().map(|(_, _, rejection)| rejection).collect::<Vec<_>>(),
        [Rejection::Relative, Rejection::NotAFile, Rejection::Empty, Rejection::Missing]
    );
    for part in ["set to an empty value", "target/debug/my-program (a relative path)", "(not a regular file)", "(does not exist)"] {
        assert!(message.contains(part), "{part} missing from: {message}");
    }
    assert!(message.contains(&built.display().to_string()), "{message}");
}

#[test]
fn archive_path_beats_missing_compile_time_path() {
    // A nextest archive extracted on another machine: the compile-time path names the build
    // machine's folder, which does not exist here, and nextest names the extracted binary.
    let dir = scratch("archive_path_beats_missing_compile_time_path");
    let build_machine = dir.join("build-machine").join("target").join("debug").join(FIXTURE);
    let extracted = fixture().into_os_string();
    let found = resolve_binary_in(FIXTURE, build_machine.to_str(), lookup(&[("NEXTEST_BIN_EXE_craft_testkit_fixture", &extracted)])).unwrap();
    assert_eq!(found.as_os_str(), extracted);
    let output = bin::run(&found, &args(&["out", "3"]), &[], bin::DEFAULT_MAX_OUTPUT).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"xxx");
}

#[test]
fn missing_compile_time_path_is_reported_before_spawn() {
    let dir = scratch("missing_compile_time_path_is_reported_before_spawn");
    let build_machine = dir.join("build-machine").join(FIXTURE);
    let error = resolve_binary_in(FIXTURE, build_machine.to_str(), lookup(&[])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("NEXTEST_BIN_EXE_craft_testkit_fixture: not set"), "{message}");
    assert!(message.contains(&format!("(compile time): {} (does not exist)", build_machine.display())), "{message}");
    assert_eq!(candidates(error).last().map(|(source, _, rejection)| (*source, *rejection)), Some((Source::CargoCompileTime, Rejection::Missing)));
}

#[test]
fn bare_path_is_not_given_an_exe_suffix() {
    let dir = scratch("bare_path_is_not_given_an_exe_suffix");
    file(&dir, "my-program.exe");
    let bare = dir.join("my-program").into_os_string();
    let error = resolve_binary_in("my-program", None, lookup(&[("NEXTEST_BIN_EXE_my-program", &bare)])).unwrap_err();
    assert_eq!(candidates(error)[0].2, Rejection::Missing);
}

#[test]
fn exe_path_is_used_verbatim() {
    let root = scratch("exe_path_is_used_verbatim");
    let dir = root.join("a folder é");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = file(&dir, "my program.exe").into_os_string();
    let found = resolve_binary_in("my-program", None, lookup(&[("NEXTEST_BIN_EXE_my_program", &exe)])).unwrap();
    assert_eq!(found.into_os_string(), exe);
}

#[cfg(unix)]
#[test]
fn non_utf8_candidate_is_preserved() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let dir = scratch("non_utf8_candidate_is_preserved");
    let mut name = dir.as_os_str().to_owned().into_vec();
    name.extend_from_slice(b"/bin-\xff");
    let path = OsString::from_vec(name);
    match std::fs::write(&path, b"") {
        Ok(()) => {
            let found = resolve_binary_in("my-program", None, lookup(&[("NEXTEST_BIN_EXE_my-program", &path)])).unwrap();
            assert_eq!(found.as_os_str().as_bytes(), path.as_bytes());
        }
        // APFS refuses names that are not UTF-8 with EILSEQ (92 on macOS), so the
        // file cannot exist there. The rejected candidate still carries the bytes unchanged.
        Err(error) if cfg!(target_os = "macos") && error.raw_os_error() == Some(92) => {
            let error = resolve_binary_in("my-program", None, lookup(&[("NEXTEST_BIN_EXE_my-program", &path)])).unwrap_err();
            let BinPathError::NotFound { candidates, .. } = error else { panic!("expected NotFound, got {error:?}") };
            let value = candidates[0].value.as_ref().map(|value| value.as_bytes());
            assert_eq!(value, Some(path.as_bytes()));
        }
        Err(error) => panic!("could not create {path:?}: {error}"),
    }
}

#[test]
fn invalid_binary_name_is_an_error() {
    for name in ["", "a=b", "a\0b"] {
        let error = resolve_binary_in(name, None, |_| panic!("no variable is read for {name:?}")).unwrap_err();
        assert_eq!(error, BinPathError::InvalidName(name.to_owned()));
    }
}

#[test]
fn bin_path_fails_the_test_with_the_variables_named() {
    let payload = std::panic::catch_unwind(|| bin_path!("craft-testkit-no-such-binary")).unwrap_err();
    let message = payload.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("`craft-testkit-no-such-binary`"), "{message}");
    for variable in
        ["NEXTEST_BIN_EXE_craft_testkit_no_such_binary", "NEXTEST_BIN_EXE_craft-testkit-no-such-binary", "CARGO_BIN_EXE_craft-testkit-no-such-binary"]
    {
        assert!(message.contains(variable), "{variable} missing from: {message}");
    }
}

// Running a binary.

#[test]
fn nonzero_exit_is_returned_as_output() {
    let output = bin::run(&fixture(), &args(&["exit", "3"]), &[], bin::DEFAULT_MAX_OUTPUT).unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(output.stderr, b"bye");
    assert!(output.stdout.is_empty());
}

#[test]
fn at_cap_succeeds() {
    let output = bin::run(&fixture(), &args(&["out", "4096"]), &[], 4096).unwrap();
    assert_eq!(output.stdout.len(), 4096);
    let output = bin::run(&fixture(), &args(&["err", "4096"]), &[], 4096).unwrap();
    assert_eq!(output.stderr.len(), 4096);
}

#[test]
fn zero_cap_rejects_one_byte() {
    let output = bin::run(&fixture(), &args(&["out", "0"]), &[], 0).unwrap();
    assert!(output.status.success());
    let error = bin::run(&fixture(), &args(&["out", "1"]), &[], 0).unwrap_err();
    assert_eq!(limit_error(&error), OutputLimitExceeded { stream: Stream::Stdout, limit: 0 });
}

#[test]
fn stdout_cap_kills_and_reaps() {
    // The child writes one byte over the limit and then waits forever, so `run` returns only
    // when it stops the child.
    let error = bin::run(&fixture(), &args(&["out-then-wait", "65537"]), &[], 65536).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(limit_error(&error), OutputLimitExceeded { stream: Stream::Stdout, limit: 65536 });
    assert!(error.to_string().contains("more than 65536 bytes to stdout"), "{error}");
}

#[test]
fn stderr_cap_kills_and_reaps() {
    let error = bin::run(&fixture(), &args(&["err-then-wait", "65537"]), &[], 65536).unwrap_err();
    assert_eq!(limit_error(&error), OutputLimitExceeded { stream: Stream::Stderr, limit: 65536 });
}

#[test]
fn simultaneous_full_pipes_complete() {
    // The child fills stderr before it writes to stdout. A reader that waited for the end of
    // stdout first would never see the child exit.
    let n = 1 << 20;
    let output = bin::run(&fixture(), &args(&["err-then-out", &n.to_string()]), &[], n).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stderr.len(), n);
    assert_eq!(output.stdout.len(), n);
}

#[test]
fn stdin_is_null() {
    let output = bin::run(&fixture(), &args(&["stdin"]), &[], bin::DEFAULT_MAX_OUTPUT).unwrap();
    assert_eq!(output.stdout, b"0");
}

/// A variable of the test's environment that a child can be given another value for. Cargo and
/// nextest set `CARGO_PKG_NAME` when they run a test.
fn inherited_variable() -> (OsString, OsString) {
    let harmless = |name: &OsString| {
        let name = name.to_string_lossy().to_ascii_uppercase();
        !name.is_empty() && !name.contains('=') && name != "PATH" && !name.starts_with("LD_") && !name.starts_with("DYLD_")
    };
    let vars: Vec<_> = std::env::vars_os().filter(|(name, value)| harmless(name) && value.to_str().is_some()).collect();
    vars.iter().find(|(name, _)| name == "CARGO_PKG_NAME").or_else(|| vars.first()).cloned().expect("the test runs with at least one UTF-8 variable")
}

#[test]
fn child_env_overrides_inherited() {
    let (name, _) = inherited_variable();
    let output =
        bin::run(&fixture(), &[OsString::from("env"), name.clone()], &[(name.clone(), "first".into()), (name, "second".into())], 64).unwrap();
    assert_eq!(output.stdout, b"second");
}

#[test]
fn unlisted_env_is_inherited() {
    let (name, value) = inherited_variable();
    let output = bin::run(&fixture(), &[OsString::from("env"), name], &[], bin::DEFAULT_MAX_OUTPUT).unwrap();
    assert_eq!(output.stdout, value.to_str().unwrap().as_bytes());
}

#[test]
fn listed_env_reaches_the_child() {
    let name = OsString::from("CRAFT_TESTKIT_BIN_TEST_VALUE");
    let output = bin::run(&fixture(), &[OsString::from("env"), name.clone()], &[(name, "a value".into())], 64).unwrap();
    assert_eq!(output.stdout, b"a value");
}

#[cfg(unix)]
#[test]
fn non_utf8_child_env_round_trips() {
    use std::os::unix::ffi::OsStringExt;
    let name = OsString::from("CRAFT_TESTKIT_BIN_TEST_BYTES");
    let value = OsString::from_vec(b"a\xffb".to_vec());
    let output = bin::run(&fixture(), &[OsString::from("env"), name.clone()], &[(name, value)], 64).unwrap();
    assert_eq!(output.stdout, b"a\xffb");
}

#[test]
fn spawn_failure_names_the_path() {
    let dir = scratch("spawn_failure_names_the_path");
    let missing = dir.join("no-such-program");
    let error = bin::run(&missing, &[], &[], bin::DEFAULT_MAX_OUTPUT).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains(&missing.display().to_string()), "{error}");
}
