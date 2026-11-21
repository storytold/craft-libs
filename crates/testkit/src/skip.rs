//! Skip-or-fail for a test that needs something a test environment may lack: a program, a corpus
//! file, the craft-fonts faces.
//!
//! An optional absence prints one line to stderr, `SKIPPED: <reason>`, and the test returns and
//! passes. When the variable the caller names is exactly `1`, the absence fails the test instead
//! and no skip line is printed. Unset, empty, `0`, `true`, ` 1` and `1\n` all leave the test
//! optional. The caller passes the name its repository uses, such as `CRAFT_MEDIA_REQUIRE_ORACLES`
//! or `CRAFT_PDF_ORACLES_REQUIRED`; [`REQUIRED_VAR`] is the default name.
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

use std::ffi::OsStr;

/// The default name of the variable that makes a skip fail the test.
pub const REQUIRED_VAR: &str = "CRAFT_TEST_REQUIRED";

/// The variable `craft-fonts-build` reads, named by [`craft_fonts`] when it fails a test.
pub const FONTS_REQUIRED_VAR: &str = "CRAFT_FONTS_REQUIRED";

/// Whether the variable `var` is set to exactly `1`. Any other value, a value that is not valid
/// Unicode, and an unset variable return `false`.
pub fn required(var: &str) -> bool {
    is_required_value(std::env::var_os(var).as_deref())
}

fn is_required_value(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// Skip the calling test: print `SKIPPED: <reason>` and return `false`, or fail the test when
/// `required_var` is exactly `1`. The caller returns from the test itself:
/// `if !skip(reason, var) { return; }`.
pub fn skip(reason: &str, required_var: &str) -> bool {
    skip_with(required(required_var), reason, required_var)
}

/// Fail the test when `required` is set, otherwise print the skip line and return `false`. It
/// fails the test with a panic.
#[allow(clippy::panic)]
fn skip_with(required: bool, reason: &str, required_var: &str) -> bool {
    if required {
        panic!("{reason}, and {required_var}=1 forbids skipping this test");
    }
    eprint!("{}", skip_line(reason));
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
/// one face parses.
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

    /// Unset, empty, `0`, `true`, a leading space and a trailing line feed are optional; only
    /// exactly `1` is required.
    #[test]
    fn only_exactly_1_is_required() {
        assert!(!is_required_value(None));
        for value in ["", "0", "true", "TRUE", "yes", "on", " 1", "1 ", "1\n", "01", "11"] {
            assert!(!is_required_value(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(is_required_value(Some(OsStr::new("1"))));
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_unicode_is_optional() {
        use std::os::unix::ffi::OsStrExt as _;
        assert!(!is_required_value(Some(OsStr::from_bytes(b"1\xff"))));
        assert!(!is_required_value(Some(OsStr::from_bytes(b"\xff"))));
    }

    /// Absence is never silent, and which of the two loud forms it takes is decided by the
    /// variable: exactly `1` fails the test, and `0` (fatal under the any-value rule) skips.
    #[test]
    fn a_missing_oracle_is_fatal_exactly_when_it_is_required() {
        let var = "CRAFT_PDF_ORACLES_REQUIRED";
        assert!(!skip_with(is_required_value(Some(OsStr::new("0"))), "qpdf is not on PATH", var));
        assert!(!skip_with(is_required_value(Some(OsStr::new(""))), "qpdf is not on PATH", var));

        let failed = std::panic::catch_unwind(|| skip_with(is_required_value(Some(OsStr::new("1"))), "qpdf is not on PATH", var));
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

    /// The variable is read from the environment, and only exactly `1` fails the test.
    #[test]
    fn skip_reads_its_variable_from_the_environment() {
        for value in [None, Some(""), Some("0"), Some("true"), Some(" 1")] {
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
