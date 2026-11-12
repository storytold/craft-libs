//! The PDF object layer (COS), for code that edits and saves PDF files. It uses [`crate::crypt`]
//! and [`crate::filters`]; the reader (`read`) is separate and does not use it.
//!
//! Parses the COS object graph lazily and tolerantly, keeps edits in a copy-on-write overlay
//! (cheap snapshots for undo), and writes documents back either incrementally (original bytes
//! untouched) or as a full, garbage-collected rewrite. An app can use the object graph as its
//! document model.
//!
//! Done: every cross-reference form (tables, streams, hybrid `/XRefStm`), object streams, repair
//! by scanning, encryption on load and save (R2–R6, crypt filters, `/EFF`), incremental writing,
//! and full saves that pack objects into compressed object streams with a cross-reference stream.
//! Not yet: streaming very large files within a memory budget, and linearized output.

mod budget;
mod bytes;
mod document;
mod object;
pub mod page_labels;
mod parser;
mod security;
mod writer;

#[cfg(test)]
pub(crate) mod testkit;

pub use crate::crypt::{
    Algorithm, Auth, Method as CryptMethod, NewEncryption, Permissions, SecurityHandler, effective_password, pdfdoc_byte, pdfdoc_char,
};
pub use budget::{DecodeBudget, ImageBudget, MAX_PAGE_DECODED, MAX_PAGE_FORMS, MAX_REDACTED_IMAGE_PIXELS};
pub use bytes::Bytes;
pub use document::{DECODE_PARMS_KEYS, Document, ReadState, Revision, SourceIdentity, XrefEntry};
pub use object::{Dict, MAX_DECODED, MAX_JBIG2_GLOBALS, Name, ObjRef, Object, PdfString, Stream, TextChars};
pub use parser::{Lexer, is_whitespace, parse_indirect};
pub use writer::{SaveOptions, civil_from_days, new_file_id, pdf_date, reachable_objects, serialize, write_full, write_incremental};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CosError {
    #[error("not a PDF file (no %PDF- header)")]
    NotPdf,
    #[error("syntax error at byte {offset}: {detail}")]
    Syntax { offset: usize, detail: String },
    #[error("object {0} is missing")]
    MissingObject(u32),
    #[error("object {0} is not a dictionary")]
    NotADictionary(ObjRef),
    #[error("stream data could not be decoded: {0}")]
    Filter(String),
    #[error("the document is protected by a password")]
    NeedsPassword,
    #[error("the password is incorrect")]
    WrongPassword,
    #[error("unsupported security: {0}")]
    Security(String),
    #[error("internal lock poisoned")]
    Poisoned,
    #[error("this document was opened read-only with a decode limit and cannot be saved")]
    ReadOnlyLimit,
}
