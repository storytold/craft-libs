//! Skip-or-fail for a test that needs something a test environment may lack: a program, a corpus
//! file, the craft-fonts faces.
//!
//! An optional absence prints one line to stderr, `SKIPPED: <reason>`, and the test returns and
//! passes. When the variable the caller names is `1`, the absence fails the test instead and no
//! skip line is printed. Unset and empty leave the test optional. Any other value, such as `0`,
//! `true`, ` 1`, `1\n` or a value that is not valid Unicode, is a configuration error: the absence
//! fails the test with the variable's name and value. The caller passes the name its repository
//! uses, such as `CRAFT_MEDIA_REQUIRE_ORACLES` or `CRAFT_PDF_ORACLES_REQUIRED`; [`REQUIRED_VAR`] is
//! the default name.
//! Optional skips ignore stderr write failures.
//!
//! ```no_run
//! use craft_testkit::skip::{REQUIRED_VAR, skip};
//!
//! fn find_qpdf() -> Result<std::path::PathBuf, String> {
//!     Err("qpdf is not on PATH".into())
//! }
//!
//! // A test returning `()`: bind the program or return.
//! let qpdf = craft_testkit::or_skip!(find_qpdf(), REQUIRED_VAR);
//! # let _ = qpdf;
//!
//! // The same rule without a `Result` to match on.
//! let gpu_available = false;
//! if !gpu_available && !skip("no GPU adapter", REQUIRED_VAR) {
//!     return;
//! }
//! ```
//!
//! A reason that holds a carriage return or a line feed is printed with them written as `\r` and
//! `\n`, so one skip is one line. Each line is written with one call while stderr is locked, so
//! lines from tests running in parallel do not mix; their order is not fixed.

use std::ffi::{OsStr, OsString};
use std::fmt;

/// The default name of the variable that makes a skip fail the test.
pub const REQUIRED_VAR: &str = "CRAFT_TEST_REQUIRED";

/// The variable `craft-fonts-build` reads, named by [`craft_fonts`] when it fails a test.
pub const FONTS_REQUIRED_VAR: &str = "CRAFT_FONTS_REQUIRED";

/// A required variable name that cannot be read, or a value that is not unset, empty or `1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequiredError {
    /// The variable name is empty or holds `=` or a NUL character.
    BadName { variable: String },
    /// The value is not valid Unicode.
    NotUnicode { variable: String },
    /// The value is valid Unicode but not empty or `1`.
    BadValue { variable: String, value: String },
}

impl fmt::Display for RequiredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const FIX: &str = "set it to 1 to fail a test that lacks a prerequisite, or leave it unset or empty to skip the test";
        match self {
            Self::BadName { variable } => write!(f, "{variable:?} cannot name a variable: it is empty or holds `=` or a NUL character"),
            Self::NotUnicode { variable } => write!(f, "{variable} is not valid Unicode; {FIX}"),
            Self::BadValue { variable, value } => write!(f, "{variable}={value:?} is not empty or 1; {FIX}"),
        }
    }
}

impl std::error::Error for RequiredError {}

/// Whether the variable `var` makes a missing prerequisite fail the test. Unset and empty return
/// `Ok(false)`, and `1` returns `Ok(true)`. Any other value, including `0` and a value that is not
/// valid Unicode, returns an error that names the variable. A name that is empty or holds `=` or a
/// NUL character returns an error before the environment is read.
pub fn required(var: &str) -> Result<bool, RequiredError> {
    required_with(var, |name| std::env::var_os(name))
}

/// [`required`] with the variable lookup passed in.
fn required_with(variable: &str, lookup: impl FnOnce(&str) -> Option<OsString>) -> Result<bool, RequiredError> {
    if variable.is_empty() || variable.contains(['=', '\0']) {
        return Err(RequiredError::BadName { variable: variable.to_owned() });
    }
    parse_required(variable, lookup(variable).as_deref())
}

fn parse_required(variable: &str, value: Option<&OsStr>) -> Result<bool, RequiredError> {
    let Some(value) = value else {
        return Ok(false);
    };
    if value.is_empty() {
        return Ok(false);
    }
    if value == OsStr::new("1") {
        return Ok(true);
    }
    Err(match value.to_str() {
        Some(text) => RequiredError::BadValue { variable: variable.to_owned(), value: text.to_owned() },
        None => RequiredError::NotUnicode { variable: variable.to_owned() },
    })
}

/// Skip the calling test: print `SKIPPED: <reason>` and return `false`, or fail the test when
/// `required_var` is `1`. A value of `required_var` that [`required`] rejects fails the test with
/// the reason and the error. The caller returns from the test itself:
/// `if !skip(reason, var) { return; }`.
///
/// The `SKIPPED:` line is written to the process's standard error, not through `eprint!`, so the
/// test harness does not capture it: it shows in a default run of a passing test, and
/// `--show-output` does not list it under the test's name. A failed write of the line is ignored.
#[allow(clippy::panic)]
pub fn skip(reason: &str, required_var: &str) -> bool {
    match required(required_var) {
        Ok(required) => skip_with(required, reason, required_var),
        Err(error) => panic!("{reason}, and {error}"),
    }
}

/// Fail the test when `required` is set, otherwise print the skip line and return `false`. It
/// fails the test with a panic.
#[allow(clippy::panic)]
fn skip_with(required: bool, reason: &str, required_var: &str) -> bool {
    use std::io::Write as _;
    if required {
        panic!("{reason}, and {required_var}=1 forbids skipping this test");
    }
    // Written to the process's stderr directly: `eprint!` is captured by the test harness and
    // would hide the line from a default run.
    let _ = std::io::stderr().lock().write_all(skip_line(reason).as_bytes());
    false
}

/// The skip line for `reason`, with its carriage returns and line feeds escaped, ending in one
/// line feed.
fn skip_line(reason: &str) -> String {
    let mut line = String::with_capacity("SKIPPED: \n".len() + reason.len());
    line.push_str("SKIPPED: ");
    for c in reason.chars() {
        match c {
            '\r' => line.push_str("\\r"),
            '\n' => line.push_str("\\n"),
            c => line.push(c),
        }
    }
    line.push('\n');
    line
}

/// Whether a test that needs the craft-fonts faces can run. Returns `true` when `present` is set.
/// Without the faces it prints a skip line and returns `false`, or fails the test when `required`
/// is set. The caller passes both values from its build: a build constant, an `option_env!`
/// value, or the table `craft-fonts-build` generates.
pub fn craft_fonts(present: bool, required: bool) -> bool {
    present || skip_with(required, "craft-fonts faces unavailable (build with CRAFT_FONTS_DIR)", FONTS_REQUIRED_VAR)
}

/// Whether one of `faces` maps every character of `text` that is not whitespace. Each face is
/// font data and the index of the face in a collection (`0` for a single font). A face that does
/// not parse covers nothing, and the characters must all come from the same face. Each combining
/// mark is a character of its own. Text that is empty or only whitespace is covered when at least
/// one face parses. Needs the `font-coverage` feature.
#[cfg(feature = "font-coverage")]
pub fn craft_fonts_cover(text: &str, faces: &[(&[u8], u32)]) -> bool {
    use skrifa::MetadataProvider as _;
    faces.iter().any(|&(data, index)| {
        let Ok(font) = skrifa::FontRef::from_index(data, index) else {
            return false;
        };
        let map = font.charmap();
        text.chars().filter(|c| !c.is_whitespace()).all(|c| map.map(c).is_some())
    })
}

/// Binds the `Ok` value of a `Result` probe, or skips the calling test under the rule of
/// [`skip`](crate::skip::skip) and returns from it. The probe is evaluated once, and its error is
/// the skip reason, so the error type implements `Display`.
///
/// `or_skip!(probe, var)` returns `()` and belongs in a test returning `()`.
/// `or_skip!(probe, var, value)` returns `value`, for example `Ok(())` in a test that returns a
/// `Result`.
#[macro_export]
macro_rules! or_skip {
    ($probe:expr, $required_var:expr) => {
        $crate::or_skip!($probe, $required_var, ())
    };
    ($probe:expr, $required_var:expr, $ret:expr) => {
        match $probe {
            ::core::result::Result::Ok(value) => value,
            ::core::result::Result::Err(absent) => {
                let _ = $crate::skip::skip(&::std::string::ToString::to_string(&absent), $required_var);
                return $ret;
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::process::{Command, Output};

    /// The case the child process runs; unset in an ordinary test run.
    const CHILD_CASE: &str = "CRAFT_TESTKIT_SKIP_CHILD";
    /// A variable no test sets, for skips that must stay optional.
    const NEVER_SET: &str = "CRAFT_TESTKIT_SKIP_NEVER_SET";

    /// Runs `child` below in a new process of this test binary, with `REQUIRED_VAR` set to
    /// `required` or removed, so no test changes this process's environment.
    fn run_child(case: &str, required: Option<&str>) -> Output {
        run_child_os(case, required.map(OsStr::new))
    }

    /// [`run_child`] with a value that need not be valid Unicode.
    fn run_child_os(case: &str, required: Option<&OsStr>) -> Output {
        let exe = std::env::current_exe().expect("the test binary's path");
        let mut cmd = Command::new(exe);
        cmd.args(["skip::tests::child", "--exact", "--nocapture", "--test-threads=1"]).env(CHILD_CASE, case);
        match required {
            Some(value) => cmd.env(REQUIRED_VAR, value),
            None => cmd.env_remove(REQUIRED_VAR),
        };
        cmd.output().expect("run the test binary")
    }

    fn stderr(out: &Output) -> String {
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    #[test]
    fn child() {
        let Some(case) = std::env::var_os(CHILD_CASE) else {
            return;
        };
        match case.to_str() {
            Some("skip-without-tail") => assert!(!skip("probe absent", REQUIRED_VAR)),
            Some("skip") => {
                assert!(!skip("probe absent", REQUIRED_VAR));
                eprintln!("returned from skip");
            }
            Some("or_skip") => {
                let value: u32 = crate::or_skip!(Err::<u32, &str>("probe absent"), REQUIRED_VAR);
                eprintln!("ran on with {value}");
            }
            Some("threads") => std::thread::scope(|s| {
                for i in 0..8 {
                    s.spawn(move || skip(&format!("thread {i}\r\nsecond half"), REQUIRED_VAR));
                }
            }),
            other => panic!("unknown child case {other:?}"),
        }
    }

    /// Values other than unset, empty and `1`.
    const INVALID: [&str; 11] = ["0", "true", "TRUE", "yes", "on", " 1", "1 ", "1\n", "01", "11", " "];

    /// A value that is not valid Unicode: a lone surrogate on Windows, a byte that is not UTF-8
    /// elsewhere.
    fn not_unicode() -> std::ffi::OsString {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt as _;
            std::ffi::OsString::from_wide(&[u16::from(b'1'), 0xD800])
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::ffi::OsStringExt as _;
            std::ffi::OsString::from_vec(b"1\xff".to_vec())
        }
    }

    /// Unset and empty are optional, `1` is required, and every other value, `0` included, is an
    /// error that names the variable and the value.
    #[test]
    fn only_unset_empty_and_1_are_valid() {
        let var = "CRAFT_PDF_ORACLES_REQUIRED";
        assert_eq!(parse_required(var, None), Ok(false));
        assert_eq!(parse_required(var, Some(OsStr::new(""))), Ok(false));
        assert_eq!(parse_required(var, Some(OsStr::new("1"))), Ok(true));
        for value in INVALID {
            let error = parse_required(var, Some(OsStr::new(value)));
            assert_eq!(error, Err(RequiredError::BadValue { variable: var.into(), value: value.into() }), "{value:?}");
        }
        let message = RequiredError::BadValue { variable: var.into(), value: "1\n".into() }.to_string();
        assert!(message.starts_with("CRAFT_PDF_ORACLES_REQUIRED=\"1\\n\" is not empty or 1; "), "{message}");
        assert_eq!(message.lines().count(), 1, "{message}");
    }

    #[test]
    fn a_value_that_is_not_unicode_is_an_error() {
        let value = not_unicode();
        assert_eq!(parse_required(REQUIRED_VAR, Some(&value)), Err(RequiredError::NotUnicode { variable: REQUIRED_VAR.into() }));
        let message = RequiredError::NotUnicode { variable: REQUIRED_VAR.into() }.to_string();
        assert!(message.starts_with("CRAFT_TEST_REQUIRED is not valid Unicode; "), "{message}");
    }

    #[test]
    fn bad_variable_names_are_rejected_before_reading() {
        for name in ["", "A=B", "A\0B"] {
            let result = required_with(name, |_| panic!("the variable was read"));
            assert_eq!(result, Err(RequiredError::BadName { variable: name.to_owned() }), "{name:?}");
            assert_eq!(required(name), Err(RequiredError::BadName { variable: name.to_owned() }), "{name:?}");

            let failed = std::panic::catch_unwind(|| skip("probe absent", name));
            let message = failed.expect_err("a bad name fails the test");
            let message = message.downcast_ref::<String>().expect("a formatted message");
            assert!(message.starts_with("probe absent, and "), "{message}");
            assert!(message.ends_with(" cannot name a variable: it is empty or holds `=` or a NUL character"), "{message}");
        }
    }

    /// Absence is never silent, and which of the two loud forms it takes is decided by the
    /// variable: `1` fails the test, and unset or empty skips.
    #[test]
    fn a_missing_oracle_is_fatal_exactly_when_it_is_required() {
        let var = "CRAFT_PDF_ORACLES_REQUIRED";
        assert!(!skip_with(parse_required(var, None) == Ok(true), "qpdf is not on PATH", var));
        assert!(!skip_with(parse_required(var, Some(OsStr::new(""))) == Ok(true), "qpdf is not on PATH", var));

        let failed = std::panic::catch_unwind(|| skip_with(parse_required(var, Some(OsStr::new("1"))) == Ok(true), "qpdf is not on PATH", var));
        let message = failed.expect_err("a required absence fails the test");
        let message = message.downcast_ref::<String>().expect("a formatted message");
        assert!(message.contains("qpdf is not on PATH"), "{message}");
        assert!(message.contains("CRAFT_PDF_ORACLES_REQUIRED=1"), "{message}");
    }

    #[test]
    fn a_skip_line_is_one_line_with_its_reason_escaped() {
        assert_eq!(skip_line("qpdf is not on PATH"), "SKIPPED: qpdf is not on PATH\n");
        let line = skip_line("first\r\nsecond\nthird\r");
        assert_eq!(line, "SKIPPED: first\\r\\nsecond\\nthird\\r\n");
        assert_eq!(line.lines().count(), 1);
        assert_eq!(skip_line(""), "SKIPPED: \n");
    }

    /// The variable is read from the environment: unset and empty skip, and `1` fails the test.
    #[test]
    fn skip_reads_its_variable_from_the_environment() {
        for value in [None, Some("")] {
            let out = run_child("skip", value);
            let err = stderr(&out);
            assert!(out.status.success(), "{value:?}: {err}");
            assert_eq!(err.matches("SKIPPED: probe absent\n").count(), 1, "{value:?}: {err}");
            assert!(err.contains("returned from skip"), "{value:?}: {err}");
        }
        let out = run_child("skip", Some("1"));
        let err = stderr(&out);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains("probe absent, and CRAFT_TEST_REQUIRED=1 forbids skipping this test"), "{err}");
        assert!(!err.contains("SKIPPED:"), "{err}");
        assert!(!err.contains("returned from skip"), "{err}");
    }

    /// Any other value fails the test with the reason, the variable and the value, and prints no
    /// skip line.
    #[test]
    fn an_invalid_variable_fails_the_test() {
        for value in ["0", "true", "yes", " 1", "1\n"] {
            let out = run_child("skip", Some(value));
            let err = stderr(&out);
            assert!(!out.status.success(), "{value:?}: {err}");
            let expected = format!("probe absent, and CRAFT_TEST_REQUIRED={value:?} is not empty or 1; ");
            assert!(err.contains(&expected), "{value:?}: {err}");
            assert!(!err.contains("SKIPPED:"), "{value:?}: {err}");
            assert!(!err.contains("returned from skip"), "{value:?}: {err}");
        }

        let out = run_child_os("skip", Some(&not_unicode()));
        let err = stderr(&out);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains("probe absent, and CRAFT_TEST_REQUIRED is not valid Unicode; "), "{err}");
        assert!(!err.contains("SKIPPED:"), "{err}");

        for value in ["0", "true"] {
            let out = run_child("or_skip", Some(value));
            let err = stderr(&out);
            assert!(!out.status.success(), "{value:?}: {err}");
            assert!(err.contains(&format!("CRAFT_TEST_REQUIRED={value:?} is not empty or 1")), "{value:?}: {err}");
            assert!(!err.contains("SKIPPED:"), "{value:?}: {err}");
            assert!(!err.contains("ran on"), "{value:?}: {err}");
        }
    }

    #[test]
    fn or_skip_returns_from_the_test_when_optional_and_fails_when_required() {
        let out = run_child("or_skip", None);
        let err = stderr(&out);
        assert!(out.status.success(), "{err}");
        assert!(err.contains("SKIPPED: probe absent\n"), "{err}");
        assert!(!err.contains("ran on"), "{err}");

        let out = run_child("or_skip", Some("1"));
        let err = stderr(&out);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains("CRAFT_TEST_REQUIRED=1"), "{err}");
        assert!(!err.contains("SKIPPED:"), "{err}");
        assert!(!err.contains("ran on"), "{err}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_optional_skip_survives_a_stderr_write_failure() {
        let full = std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["skip::tests::child", "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_CASE, "skip-without-tail")
            .env_remove(REQUIRED_VAR)
            .stderr(full)
            .output()
            .unwrap();
        assert!(output.status.success(), "optional skip failed: {:?}, stdout={}", output.status, String::from_utf8_lossy(&output.stdout));
    }

    #[test]
    fn the_skip_line_shows_in_a_run_that_captures_test_output() {
        let exe = std::env::current_exe().expect("the test binary's path");
        let out = Command::new(exe)
            .args(["skip::tests::child", "--exact", "--test-threads=1"])
            .env(CHILD_CASE, "skip")
            .env_remove(REQUIRED_VAR)
            .output()
            .expect("run the test binary");
        let err = stderr(&out);
        assert!(out.status.success(), "{err}");
        assert!(err.contains("SKIPPED: probe absent\n"), "{err}");
    }

    #[test]
    fn skip_lines_from_parallel_threads_stay_whole() {
        let out = run_child("threads", None);
        let err = stderr(&out);
        assert!(out.status.success(), "{err}");
        let mut lines: Vec<&str> = err.lines().filter(|l| l.contains("SKIPPED") || l.contains("second half")).collect();
        lines.sort_unstable();
        let expected: Vec<String> = (0..8).map(|i| format!("SKIPPED: thread {i}\\r\\nsecond half")).collect();
        assert_eq!(lines, expected, "{err}");
    }

    #[test]
    fn or_skip_yields_the_value_and_evaluates_the_probe_once() {
        let probes = Cell::new(0);
        let probe = || {
            probes.set(probes.get() + 1);
            Ok::<u32, String>(7)
        };
        let value = crate::or_skip!(probe(), NEVER_SET);
        assert_eq!(value, 7);
        assert_eq!(probes.get(), 1);
    }

    #[test]
    fn or_skip_returns_the_given_value_from_a_result_test() {
        fn run(probes: &Cell<u32>) -> Result<u32, String> {
            let value: u32 = crate::or_skip!(
                {
                    probes.set(probes.get() + 1);
                    Err::<u32, String>("probe absent".into())
                },
                NEVER_SET,
                Ok(0)
            );
            Ok(value + 1)
        }
        let probes = Cell::new(0);
        assert_eq!(run(&probes), Ok(0));
        assert_eq!(probes.get(), 1);
    }

    #[test]
    fn craft_fonts_runs_skips_or_fails() {
        assert!(craft_fonts(true, false));
        assert!(craft_fonts(true, true));
        assert!(!craft_fonts(false, false));
        let failed = std::panic::catch_unwind(|| craft_fonts(false, true));
        let message = failed.expect_err("missing required faces fail the test");
        let message = message.downcast_ref::<String>().expect("a formatted message");
        assert!(message.contains("CRAFT_FONTS_REQUIRED=1"), "{message}");
    }

    /// A font holding only a format 12 `cmap` table that maps each range of code points.
    #[cfg(feature = "font-coverage")]
    fn face(ranges: &[(u32, u32)]) -> Vec<u8> {
        let subtable_len = 16 + 12 * ranges.len() as u32;
        let mut data = Vec::new();
        data.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        for v in [1u16, 16, 0, 0] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        data.extend_from_slice(b"cmap");
        for v in [0u32, 28, 12 + subtable_len] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        for v in [0u16, 1, 3, 10] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        data.extend_from_slice(&12u32.to_be_bytes());
        data.extend_from_slice(&12u16.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        for v in [subtable_len, 0, ranges.len() as u32] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        let mut glyph = 1u32;
        for &(start, end) in ranges {
            for v in [start, end, glyph] {
                data.extend_from_slice(&v.to_be_bytes());
            }
            glyph += end - start + 1;
        }
        data
    }

    #[test]
    #[cfg(feature = "font-coverage")]
    fn craft_fonts_cover_the_advertised_scripts() {
        let latin_greek_cyrillic = face(&[(0x41, 0x5A), (0x61, 0x7A), (0x391, 0x3C9), (0x410, 0x44F)]);
        let faces: &[(&[u8], u32)] = &[(&latin_greek_cyrillic, 0)];
        for sample in ["Craft", "Αλφα", "Привет", "Craft Αλφα Привет"] {
            assert!(craft_fonts_cover(sample, faces), "{sample}");
        }
        assert!(!craft_fonts_cover("日本", faces));
        assert!(!craft_fonts_cover("Craft!", faces));
    }

    #[test]
    #[cfg(feature = "font-coverage")]
    fn coverage_needs_one_face_that_parses_and_maps_every_character() {
        let a = face(&[(u32::from('A'), u32::from('A'))]);
        let b = face(&[(u32::from('B'), u32::from('B'))]);
        let e = face(&[(u32::from('e'), u32::from('e'))]);
        let e_acute = face(&[(u32::from('e'), u32::from('e')), (0x301, 0x301)]);
        let garbage: &[u8] = b"not a font";

        assert!(!craft_fonts_cover("", &[]));
        assert!(!craft_fonts_cover("A", &[]));
        assert!(!craft_fonts_cover("", &[(garbage, 0)]));
        assert!(!craft_fonts_cover("A", &[(garbage, 0)]));
        assert!(!craft_fonts_cover("A", &[(&a, 1)]), "a single font has no face 1");

        assert!(craft_fonts_cover("", &[(&a, 0)]));
        assert!(craft_fonts_cover(" \t\n\u{3000}", &[(&a, 0)]));
        assert!(craft_fonts_cover(" A A ", &[(&a, 0)]));
        assert!(craft_fonts_cover("A", &[(garbage, 0), (&a, 0)]));

        assert!(craft_fonts_cover("A", &[(&a, 0), (&b, 0)]));
        assert!(craft_fonts_cover("B", &[(&a, 0), (&b, 0)]));
        assert!(!craft_fonts_cover("AB", &[(&a, 0), (&b, 0)]), "each face covers only part of the text");

        assert!(!craft_fonts_cover("e\u{301}", &[(&e, 0)]), "the combining mark is not mapped");
        assert!(craft_fonts_cover("e\u{301}", &[(&e_acute, 0)]));
    }
}
