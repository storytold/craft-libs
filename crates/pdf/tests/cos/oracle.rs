//! External oracle processes: independent implementations the object layer's output is checked
//! against (from PdfCraft's `pdfcraft-oracle`).
//!
//! **A missing oracle is either a declared skip or a failure, never a silent pass.**
//! [`REQUIRED_VAR`] decides which: set it where the tools are installed, so their absence fails
//! the run; without it, a missing tool prints why and the test returns.
//!
//! Test scaffolding, compiled into this test binary only.
#![allow(dead_code)]

use std::fmt;
use std::process::Command;

/// Setting this environment variable to any value turns a missing oracle from a skip into a
/// failure. CI sets it on the runners that install the tools.
///
/// Any value counts, including `0` — the same convention as `CRAFT_FONTS_REQUIRED`, which this
/// deliberately mirrors. The surprising half of that convention errs toward *requiring* an oracle,
/// which is the safe direction for a gate whose whole purpose is to stop absence passing quietly.
pub const REQUIRED_VAR: &str = "CRAFT_PDF_ORACLES_REQUIRED";

/// An external program found on `PATH`, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oracle {
    program: &'static str,
    version: String,
}

impl Oracle {
    /// A fresh [`Command`] for the program. Callers add their own arguments.
    pub fn command(&self) -> Command {
        Command::new(self.program)
    }

    /// The program name, as it is looked up on `PATH`.
    pub fn program(&self) -> &'static str {
        self.program
    }

    /// The first line the program printed when asked for its version, for test output. Empty when
    /// it printed nothing recognisable; presence of the binary is what decides availability, not
    /// whether its version could be parsed.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// An oracle that could not be used, and whether that is fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absent {
    program: &'static str,
    cause: String,
    required: bool,
}

impl Absent {
    /// Whether this absence must fail the run rather than skip it, i.e. whether [`REQUIRED_VAR`]
    /// was set when the probe ran.
    pub fn required(&self) -> bool {
        self.required
    }

    /// The program that was looked for.
    pub fn program(&self) -> &'static str {
        self.program
    }

    /// How to install it, so a skip notice tells the reader what to do about it.
    fn install_hint(program: &str) -> &'static str {
        match program {
            "qpdf" => "apt install qpdf / brew install qpdf",
            "pdftoppm" | "pdftotext" => "apt install poppler-utils / brew install poppler",
            _ => "install it and put it on PATH",
        }
    }
}

impl fmt::Display for Absent {
    /// Reads as an instruction either way: required absences are failures and name the gate that
    /// made them one, optional absences name the install command.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.required {
            write!(
                f,
                "oracle `{}` is required but {}: {} is set, so this is a failure rather than a skip ({})",
                self.program,
                self.cause,
                REQUIRED_VAR,
                Self::install_hint(self.program),
            )
        } else {
            write!(
                f,
                "skipping: oracle `{}` {} ({}); set {} to make this a failure instead",
                self.program,
                self.cause,
                Self::install_hint(self.program),
                REQUIRED_VAR,
            )
        }
    }
}

/// Look for `program` on `PATH`, asking it for its version to confirm it runs.
///
/// Presence is decided by whether the program could be started at all, not by its exit status: a
/// tool that runs but reports its version oddly is still an oracle.
pub fn find(program: &'static str, version_arg: &str) -> Result<Oracle, Absent> {
    let required = std::env::var_os(REQUIRED_VAR).is_some();
    match Command::new(program).arg(version_arg).output() {
        Ok(out) => {
            // Poppler's tools print their version to stderr, qpdf to stdout.
            let text = if out.stdout.is_empty() { &out.stderr } else { &out.stdout };
            let version = String::from_utf8_lossy(text).lines().next().unwrap_or_default().trim().to_owned();
            Ok(Oracle { program, version })
        }
        Err(e) => Err(Absent { program, cause: format!("is not on PATH ({e})"), required }),
    }
}

/// qpdf: structural validation (`--check`) and the independent owner-password oracle.
pub fn qpdf() -> Result<Oracle, Absent> {
    find("qpdf", "--version")
}

/// Poppler's `pdftoppm`: renders a page to pixels, for comparing against our own raster output.
pub fn pdftoppm() -> Result<Oracle, Absent> {
    find("pdftoppm", "-v")
}

/// Poppler's `pdftotext`: the text-extraction oracle (`cargo xtask text-oracle` scores word F1
/// against it over the corpus).
pub fn pdftotext() -> Result<Oracle, Absent> {
    find("pdftotext", "-v")
}

/// Binds an oracle, or returns from the calling test after saying on stderr why it was skipped.
///
/// When [`REQUIRED_VAR`] is set the absence fails the test instead. The panic is deliberate and is
/// how a test reports failure; it expands into the caller, which is test code, so the workspace's
/// never-panic rule is not weakened — this crate itself contains no panicking path.
///
/// The macro expands to a `return`, so it belongs in a test function returning `()`. A test that
/// returns `Result` should match on the probe itself.
macro_rules! or_skip {
    ($probe:expr) => {
        match $probe {
            ::core::result::Result::Ok(oracle) => oracle,
            ::core::result::Result::Err(absent) => {
                let absent: $crate::oracle::Absent = absent;
                ::core::assert!(!$crate::oracle::Absent::required(&absent), "{absent}");
                ::std::eprintln!("{absent}");
                return;
            }
        }
    };
}
pub(crate) use or_skip;

#[cfg(test)]
mod tests {
    use super::*;

    /// The one behaviour the crate exists for: absence is never silent, and which of the two loud
    /// forms it takes is decided by the environment variable.
    #[test]
    fn a_missing_oracle_is_fatal_exactly_when_it_is_required() {
        let optional = Absent { program: "qpdf", cause: "is not on PATH".into(), required: false };
        let required = Absent { required: true, ..optional.clone() };

        assert!(!optional.required());
        assert!(required.required());

        // Both say what is wrong and how to fix it; only the required one calls itself a failure.
        let (opt, req) = (optional.to_string(), required.to_string());
        assert!(opt.contains("skipping"), "{opt}");
        assert!(opt.contains("apt install qpdf"), "{opt}");
        assert!(opt.contains(REQUIRED_VAR), "{opt}");
        assert!(req.contains("required"), "{req}");
        assert!(req.contains("failure"), "{req}");
        assert!(!req.contains("skipping"), "{req}");
    }

    /// A tool that does not exist is reported as absent rather than hanging or panicking, and the
    /// reason names the program.
    #[test]
    fn a_program_that_does_not_exist_is_absent_with_a_reason() {
        let Err(absent) = find("pdfcraft-no-such-oracle-binary", "--version") else {
            panic!("a nonexistent program must not be reported as present");
        };
        assert_eq!(absent.program(), "pdfcraft-no-such-oracle-binary");
        let text = absent.to_string();
        assert!(text.contains("pdfcraft-no-such-oracle-binary"), "{text}");
        assert!(text.contains("not on PATH"), "{text}");
    }

    /// Hostile and odd program names must not panic, hang or be mistaken for a working oracle.
    #[test]
    fn hostile_program_names_are_absent_not_a_panic() {
        for name in ["", " ", "\0bad", "../../etc/passwd", &"x".repeat(4096)] {
            // `find` takes a 'static str; leak the generated ones, which is fine in a test.
            let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
            let found = find(name, "--version");
            if let Ok(o) = found {
                // Nothing by these names should run; if one somehow does, it must still be sane.
                assert_eq!(o.program(), name);
            }
        }
    }

    /// The install hint is specific per tool, so a skip notice is actionable.
    #[test]
    fn every_known_oracle_has_an_install_hint() {
        for p in ["qpdf", "pdftoppm", "pdftotext"] {
            let hint = Absent::install_hint(p);
            assert!(hint.contains("install"), "{p}: {hint}");
            assert_ne!(hint, Absent::install_hint("something-else"), "{p} needs its own hint");
        }
    }

    /// Probing is read-only and repeatable: asking twice gives the same answer.
    #[test]
    fn probing_is_stable() {
        let a = qpdf().map(|o| o.program());
        let b = qpdf().map(|o| o.program());
        assert_eq!(a.is_ok(), b.is_ok());
        if let (Ok(a), Ok(b)) = (a, b) {
            assert_eq!(a, b);
        }
    }
}
