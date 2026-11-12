//! A bytes-only check of a ZIP package's declared entry count before the archive is indexed
//! ([`craft_zip::check_entry_count`]). [`crate::Package::open`] calls it first; a reader of any
//! other ZIP package may too.

/// Refuse `bytes` when the end record the `zip` crate would use declares more than `max_entries`
/// entries.
pub fn check(bytes: &[u8], max_entries: u64) -> Result<(), String> {
    craft_zip::check_entry_count(bytes, max_entries).map_err(|e| match e {
        craft_zip::Error::TooManyEntries { declared, limit } => format!("the package declares {declared} entries, more than {limit}"),
        e => e.to_string(),
    })
}

/// A package of one empty stored file, `a`, whose central directory lists it `entries` times (at
/// most as many as fit in 64 MiB) under a ZIP64 end record that declares them all. Regression
/// input for readers' tests.
pub use craft_zip::zip64_entry_count_sample;
