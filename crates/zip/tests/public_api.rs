//! The crate used the way a dependent app uses it: only the public API, from outside the crate.
//!
//! The unit tests reach into module internals; these cases check that the seam an app actually
//! consumes (a budget, a preflight, a bounded read) is enough to hold the line, and
//! that the examples in the crate README work as written.

use craft_zip::preflight::{self as zip, Limits as ZipLimits};
use craft_zip::{Budget, Exceeded};

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
