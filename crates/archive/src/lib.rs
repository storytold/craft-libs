#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Parsing budgets for archives and XML from untrusted files, with no dependencies.
//!
//! # Threat model
//!
//! Importers open files the app did not write: packages (IDML, XLSX, DOCX, PPTX, ORA, KRA) and
//! standalone XML (SVG, XFA, TTML). Two shapes of hostile input matter before any parser has had
//! a chance to refuse.
//!
//! * **Memory and time exhaustion through a ZIP central directory.** A ZIP reader starts at the
//!   end of the file: `zip::ZipArchive::new` reads the end records and then reads and indexes
//!   *every* central-directory entry before the caller can count them, and a ZIP64 archive may
//!   declare billions. That work happens before an application limit can be applied, so the entry
//!   count and the directory size have to be bounded on the raw bytes first. [`zip::preflight`]
//!   checks the end records and walks the declared directory without allocating; [`zip::read_entry`]
//!   then bounds each entry's inflation, because a declared size is attacker-controlled and a
//!   decompression bomb expands past whatever the archive claimed. Both charge [`Budget`], the
//!   running total of real bytes.
//! * **Stack exhaustion through recursive XML parsing.** A recursive parser recurses once per
//!   element, so a few hundred thousand nested tags abort the process instead of returning an
//!   error. [`xml::check`] measures nesting in one iterative pass over the bytes before the parser
//!   runs, and [`xml::Depth`] does the same for parsers that count while they parse. The same pass
//!   finds the `<!DOCTYPE`, because an internal subset hands the parser entity expansion and
//!   content-model recursion that the application must opt into explicitly.
//!
//! Neither module parses a format. A file this crate accepts may still be malformed and be
//! rejected by the parser; a file it refuses is never handed to a parser that could die on it.
//!
//! # Where the numbers live
//!
//! Every limit is a parameter or an application constant. The crate holds no numbers of its own,
//! so the caller says what "too big" means for its format and use, and a breach names the limit
//! that was hit ([`Exceeded`] and each module's error type) so the application can word the
//! message and decide whether the import fails or a part is dropped.
//!
//! What stays in the applications: entry-name rules and part allowlists; whether a breach fails
//! the import or skips a part; the choice of XML parser and its own node limit; node, text and
//! model budgets inside each parser; the application's own tree depth limits (math, content
//! models, styles); OPC relationships and content types. Image-decode admission is deliberately
//! not here: its consumers and its checks depend on each decoder.
//!
//! # Guarantees
//!
//! * No dependencies, no `unsafe`, no panic path on input. Offsets and lengths taken from a file
//!   are read through `get` and combined with checked arithmetic, never by indexing or slicing, and
//!   malformed input is an error rather than a panic.
//! * The scans are iterative and linear: one pass over the XML bytes, and a central-directory walk
//!   whose iterations are bounded by the declared directory size as well as by the entry count.
//!
//! # Example
//!
//! ```rust
//! use craft_archive::zip::{self, Limits};
//!
//! # fn check(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! // The numbers are the application's. Bound the directory before the ZIP reader indexes it.
//! let limits = Limits { max_entries: 20_000, max_central_directory_bytes: 16 * 1024 * 1024, max_name_bytes: 1_024 };
//! let directory = zip::preflight(bytes, limits)?;
//! println!("{} entries, {} directory bytes", directory.entries, directory.central_directory_bytes);
//! # Ok(())
//! # }
//! ```

pub mod xml;
pub mod zip;

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
