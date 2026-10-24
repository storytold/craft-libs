//! The process-wide registry with nothing registered: every operation is `Unsupported`.

use craft_scan::{ScanError, ScanSettings};

#[test]
fn without_a_backend_everything_is_unsupported() {
    assert!(!craft_scan::supported());
    assert!(craft_scan::backend_names().is_empty());
    assert_eq!(craft_scan::scanner_list().err(), Some(ScanError::Unsupported));
    assert_eq!(craft_scan::scanners().err(), Some(ScanError::Unsupported));
    assert_eq!(craft_scan::scanner_details("any:id").err(), Some(ScanError::Unsupported));
    assert_eq!(craft_scan::status("any:id").err(), Some(ScanError::Unsupported));
    assert_eq!(craft_scan::diagnostic_dump("").err(), Some(ScanError::Unsupported));
    assert_eq!(craft_scan::acquire(&ScanSettings::default()).err(), Some(ScanError::Unsupported));
}

#[test]
fn the_unsupported_text_is_unchanged() {
    assert_eq!(ScanError::Unsupported.to_string(), "scanning is only supported on Windows (WIA) in this version");
}
