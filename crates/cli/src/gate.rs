//! Run the install gate and print what it found. One place, so `build` and `inspect` agree.

use std::fmt::Write as _;

use craft_plugin_abi::{HostPolicy, PolicyReport, check_package};

/// The policy a desktop app applies.
pub fn policy() -> HostPolicy {
    HostPolicy::native()
}

pub fn check(bytes: &[u8]) -> PolicyReport {
    check_package(bytes, &policy())
}

/// `ACCEPTED`, or `REJECTED` and one line per violation.
pub fn summary(report: &PolicyReport) -> String {
    if let Some(abi) = report.legacy {
        return format!("LEGACY: exports use the {} prefix, so the package loads through its shim and skips this gate", abi.export_prefix());
    }
    if report.is_accepted() {
        return "ACCEPTED".to_owned();
    }
    let mut out = format!("REJECTED ({} violation{})", report.violations.len(), if report.violations.len() == 1 { "" } else { "s" });
    for v in &report.violations {
        let _ = write!(out, "\n  - {v}");
    }
    out
}
