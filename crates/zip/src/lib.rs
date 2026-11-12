#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! ZIP archives from untrusted files, read within limits and written.
//!
//! The `zip` crate does the decoding. This crate is what every reader of a ZIP-based format
//! otherwise writes again around it:
//!
//! - [`Archive::open`] refuses an archive that declares more entries than the caller allows
//!   before `zip` parses them ([`check_entry_count`]), and tells whether two entries share a name.
//! - [`Archive::read`] reads an entry within a per-entry limit and the archive's total
//!   [`Budget`], counting the bytes that really arrive, not the size the archive declares.
//! - [`Writer`] writes stored and deflated entries, ZIP64 where an entry needs it, with the same
//!   timestamp on every entry so equal input gives equal bytes.
//! - [`preflight`] is a stricter check of the whole end-record and central-directory structure
//!   on the raw bytes, for a caller that wants nothing ambiguous to reach a ZIP reader at all.
//!
//! Every limit is the caller's number. Entry-name rules (which names are safe, which spellings
//! are one part) stay with the format that knows them.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod archive;
mod guard;
pub mod preflight;
mod write;

pub use archive::{Archive, Limits};
pub use guard::check_entry_count;
#[cfg(any(test, feature = "test-support"))]
pub use guard::zip64_entry_count_sample;
pub use write::{Compression, Writer};

/// The payload of a ZIP's first entry when it is a stored file named `mimetype` of at most 256
/// bytes: how OpenDocument, EPUB, IDML, OpenRaster and Krita mark their archives so the type is
/// readable at a fixed offset without unzipping.
pub fn stored_mimetype(bytes: &[u8]) -> Option<&[u8]> {
    let u16_at = |o: usize| bytes.get(o..o + 2).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_le_bytes);
    let u32_at = |o: usize| bytes.get(o..o + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()).map(u32::from_le_bytes);
    if u32_at(0)? != 0x0403_4b50 || u16_at(8)? != 0 {
        return None;
    }
    let size = usize::try_from(u32_at(18)?).ok()?;
    let name_len = usize::from(u16_at(26)?);
    let extra_len = usize::from(u16_at(28)?);
    if bytes.get(30..30 + name_len)? != b"mimetype" || size > 256 {
        return None;
    }
    let start = 30 + name_len + extra_len;
    bytes.get(start..start.checked_add(size)?)
}

/// Why an archive was not read or not written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a ZIP archive the `zip` crate can index.
    #[error("not a ZIP archive: {0}")]
    NotZip(String),
    /// The archive declares more entries than [`Limits::max_entries`].
    #[error("the archive declares {declared} entries, more than {limit}")]
    TooManyEntries { declared: u64, limit: u64 },
    /// More end of central directory records than an archive can have.
    #[error("more than {0} end of central directory records")]
    TooManyEndRecords(usize),
    /// An entry declares, or expands to, more than the limit given for it.
    #[error("{name} is larger than {limit} bytes")]
    EntryTooLarge { name: String, limit: u64 },
    /// The entries read so far have used up [`Limits::max_total_bytes`].
    #[error("{name} does not fit in what is left of the {limit} bytes the archive may expand to")]
    BudgetSpent { name: String, limit: u64 },
    /// An entry could not be decoded (a failed checksum, an unknown method, encryption).
    #[error("{name}: {msg}")]
    Entry { name: String, msg: String },
    /// The archive could not be written.
    #[error("the ZIP archive could not be written: {0}")]
    Write(String),
}

impl Error {
    /// Was a limit exceeded (as opposed to the archive being broken)?
    pub fn is_limit(&self) -> bool {
        matches!(self, Error::TooManyEntries { .. } | Error::TooManyEndRecords(_) | Error::EntryTooLarge { .. } | Error::BudgetSpent { .. })
    }
}

use std::fmt;

/// A named limit and the total charged against it so far.
///
/// The name is what a breach message should mention ("document", "part", "images"), and the limit
/// is the application's own number. The crate never invents either, so one budget can cover a
/// whole import and be reported in the application's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    what: &'static str,
    limit: u64,
    used: u64,
}

impl Budget {
    /// A budget named `what` that admits up to `limit` units in total.
    pub const fn new(what: &'static str, limit: u64) -> Self {
        Self { what, limit, used: 0 }
    }

    /// Charge `n` units, or refuse without changing the total.
    ///
    /// A refused charge leaves the total untouched, so a caller may carry on with a smaller
    /// charge, and the amount refused is reported in [`Exceeded::requested`].
    pub fn charge(&mut self, n: u64) -> Result<(), Exceeded> {
        if n > self.remaining() {
            return Err(Exceeded { what: self.what, limit: self.limit, requested: n });
        }
        self.used = self.used.saturating_add(n);
        Ok(())
    }

    /// The units still available; zero when the budget is exhausted.
    pub const fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    /// The units charged so far.
    pub const fn used(&self) -> u64 {
        self.used
    }

    /// The limit this budget was created with.
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// The name this budget was created with.
    pub const fn what(&self) -> &'static str {
        self.what
    }
}

/// A charge that a [`Budget`] refused: what was limited, the limit, and the amount asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exceeded {
    /// The budget's name, so a message can say which limit was hit.
    pub what: &'static str,
    /// The limit that was already reached.
    pub limit: u64,
    /// The amount that did not fit.
    pub requested: u64,
}

impl fmt::Display for Exceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the {} limit is {}; another {} was requested", self.what, self.limit, self.requested)
    }
}

impl std::error::Error for Exceeded {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_charge_and_report_the_remainder() {
        let mut budget = Budget::new("document", 10);
        assert_eq!(budget.what(), "document");
        assert_eq!(budget.limit(), 10);
        assert_eq!(budget.remaining(), 10);
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.charge(4), Ok(()));
        assert_eq!(budget.used(), 4);
        assert_eq!(budget.remaining(), 6);
        assert_eq!(budget.charge(6), Ok(()));
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn a_refused_charge_leaves_the_total_untouched() {
        let mut budget = Budget::new("part", 100);
        assert_eq!(budget.charge(60), Ok(()));
        let exceeded = budget.charge(41).err();
        assert_eq!(exceeded, Some(Exceeded { what: "part", limit: 100, requested: 41 }));
        assert_eq!(budget.used(), 60);
        // The refused amount is not charged, so the budget stays usable.
        assert_eq!(budget.charge(40), Ok(()));
        assert_eq!(budget.used(), 100);
        assert!(budget.charge(1).is_err());
    }

    #[test]
    fn a_zero_budget_admits_nothing_and_the_message_names_the_limit() {
        let mut budget = Budget::new("entries", 0);
        assert_eq!(budget.remaining(), 0);
        assert_eq!(budget.charge(0), Ok(()));
        let message = budget.charge(1).err().map(|e| e.to_string());
        assert_eq!(message.as_deref(), Some("the entries limit is 0; another 1 was requested"));
    }

    #[test]
    fn a_limit_at_the_maximum_does_not_overflow() {
        let mut budget = Budget::new("total", u64::MAX);
        assert_eq!(budget.charge(u64::MAX), Ok(()));
        assert_eq!(budget.remaining(), 0);
        assert!(budget.charge(u64::MAX).is_err());
    }
}
