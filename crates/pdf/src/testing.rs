//! Test support (feature `test-support`): builds small synthetic PDF files in code, so that crates
//! which read PDF files can test with a real file instead of a binary fixture.
//!
//! ```
//! use craft_pdf::testing::Pdf;
//!
//! let bytes = Pdf::new()
//!     .obj("<< /Type /Catalog /Pages 2 0 R >>")
//!     .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
//!     .obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>")
//!     .stream("", b"0 0 50 50 re f")
//!     .build();
//! assert!(bytes.starts_with(b"%PDF-1.7\n"));
//! ```

pub use crate::cos::testkit::Pdf;
