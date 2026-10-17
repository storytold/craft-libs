//! The crate used the way a dependent app uses it: only the public API, from outside the crate.
//!
//! The unit tests reach into module internals; these cases check that the seam an app actually
//! consumes — a budget, a preflight, a bounded read, an XML check — is enough to hold the line, and
//! that the examples in the crate README work as written.

use craft_archive::xml::{self, Depth, Dtd, Limits as XmlLimits};
use craft_archive::zip::{self, Limits as ZipLimits};
use craft_archive::{Budget, Exceeded};

/// A reader that never runs out of bytes, standing in for a decompression bomb.
struct Endless;

impl std::io::Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        for byte in buf.iter_mut() {
            *byte = 0x41;
        }
        Ok(buf.len())
    }
}

#[test]
fn the_readme_budget_example_works() {
    let mut total = Budget::new("document", 512 * 1024 * 1024);
    assert_eq!(total.charge(1024), Ok(()));
    assert_eq!(total.remaining(), 512 * 1024 * 1024 - 1024);
}

#[test]
fn a_shared_budget_bounds_an_import_of_several_entries() {
    // An app's whole-import limit, charged by each entry as it is read.
    let mut total = Budget::new("document", 1_000);
    let entry = vec![0u8; 600];
    let first = zip::read_entry(std::io::Cursor::new(entry.as_slice()), 600, 4_096, &mut total).unwrap();
    assert_eq!(first.len(), 600);

    // Six hundred more bytes do not fit in the four hundred that remain.
    let second = zip::read_entry(std::io::Cursor::new(entry.as_slice()), 600, 4_096, &mut total).err();
    assert!(second.is_some(), "the total budget must stop the second entry");
    // And the budget still reports only the bytes that really arrived.
    assert_eq!(total.used(), 600);
    assert_eq!(total.remaining(), 400);
}

#[test]
fn an_endless_reader_cannot_get_past_the_smaller_bound() {
    let mut total = Budget::new("part", 1_000_000);
    let error = zip::read_entry(Endless, u64::MAX, 4_096, &mut total).err();
    // The declared size is over the per-entry limit, so the entry is refused before any read.
    assert!(matches!(error, Some(zip::ReadError::DeclaredTooLarge { .. })), "{error:?}");
    assert_eq!(total.used(), 0);

    // With a declaration inside the limit, the read stops there even though the reader is endless.
    let mut total = Budget::new("part", 1_000_000);
    let error = zip::read_entry(Endless, 4_096, 4_096, &mut total).err();
    assert!(matches!(error, Some(zip::ReadError::Overrun { limit: 4_096 })), "{error:?}");
    assert_eq!(total.used(), 0, "a bomb charges nothing");
}

#[test]
fn a_budget_refusal_can_be_reported_in_the_apps_own_words() {
    let mut total = Budget::new("worksheet", 10);
    let refused: Exceeded = total.charge(11).expect_err("11 does not fit in 10");
    assert_eq!(refused.what, "worksheet");
    assert_eq!(refused.limit, 10);
    assert_eq!(refused.requested, 11);
    let message = refused.to_string();
    assert!(message.contains("worksheet") && message.contains("10") && message.contains("11"), "{message}");
}

#[test]
fn the_readme_zip_preflight_example_works() {
    // The bytes of no archive at all: the example's shape is what is checked here.
    let limits = ZipLimits { max_entries: 20_000, max_central_directory_bytes: 16 * 1024 * 1024, max_name_bytes: 1_024 };
    assert_eq!(zip::preflight(&[], limits), Err(zip::Error::MissingEndRecord));
}

#[test]
fn the_readme_xml_example_works() {
    let limits = XmlLimits { max_depth: 256, dtd: Dtd::Refuse };
    assert_eq!(xml::check(b"<svg><g/></svg>", &limits), Ok(()));
    assert_eq!(xml::check(b"<!DOCTYPE svg><svg/>", &limits), Err(xml::Error::DtdRefused));
}

#[test]
fn a_recursive_parser_cannot_be_handed_a_document_that_would_exhaust_the_stack() {
    // The hostile case the scan exists for: a quarter of a million open elements must come back as
    // an error. If this scan recursed, or if the app let the bytes reach a recursive parser, the
    // process would abort here instead of failing the test — which is the difference this crate
    // makes. The same input passes the scan with a depth limit above it, proving the limit, not an
    // accident, is what refused it.
    let depth = 250_000;
    let mut bytes = Vec::with_capacity(depth * 9);
    bytes.extend_from_slice(b"<r>");
    for _ in 0..depth {
        bytes.extend_from_slice(b"<a>");
    }
    let app_limits = XmlLimits { max_depth: 1_024, dtd: Dtd::Refuse };
    assert_eq!(xml::check(&bytes, &app_limits), Err(xml::Error::TooDeep { depth: 1_025, limit: 1_024 }));

    let unbounded = XmlLimits { max_depth: depth + 10, dtd: Dtd::Refuse };
    assert_eq!(xml::check(&bytes, &unbounded), Ok(()), "with a higher limit the same bytes pass");
}

#[test]
fn a_depth_counter_and_the_scan_agree_on_a_package_part() {
    let part = br#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>hi</w:t></w:r></w:p></w:body></w:document>"#;
    let limits = XmlLimits { max_depth: 6, dtd: Dtd::Refuse };
    assert!(xml::check(part, &limits).is_ok());

    // A reader counting the same document reaches the same depth and no further.
    let mut depth = Depth::new(6);
    for _ in 0..6 {
        assert_eq!(depth.enter(), Ok(()));
    }
    assert_eq!(depth.current(), 6);
    assert!(depth.enter().is_err());
}
