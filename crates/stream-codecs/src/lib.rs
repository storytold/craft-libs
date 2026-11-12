//! Byte-stream and bi-level image codecs that PDF, PostScript and TIFF share.
//!
//! - [`lzw`]: variable-width LZW, 9 to 12 bits, most significant bit first (PDF `LZWDecode`,
//!   TIFF compression 5), with or without the early code-width change.
//! - [`ascii85`], [`asciihex`], [`runlength`]: the PostScript and PDF text and run-length
//!   encodings (`ASCII85Decode`, `ASCIIHexDecode`, `RunLengthDecode`; PackBits).
//! - [`predictor`]: TIFF horizontal differencing and the PNG row filters.
//! - [`ccitt`]: CCITT Group 3 and Group 4 fax decoding, bounded, and Group 4 encoding.
//! - [`jbig2`]: JBIG2 decoding through `hayro-jbig2`, bounded, and lossless encoding (generic
//!   regions, symbol dictionaries, a dictionary shared by a group of images).
//!
//! The crate knows no container: a caller hands it bytes and parameters. A PDF `/Filter` chain
//! (order, limits across stages, Flate) is `craft_pdf::filters`, built on this crate.
//!
//! Every decoder is bounded by a caller-supplied output limit and never panics on malformed
//! input. A decoder that fails returns what it decoded before the damage ([`Failure`]), so a
//! caller can choose to keep it. CCITT and JBIG2 decoding spend from an allowance the caller
//! shares across a run of images ([`pay`]); neither decodes without one:
//!
//! ```compile_fail
//! let _ = craft_stream_codecs::decode_jbig2(&[], None, 1, 1, 1 << 20);
//! ```
//!
//! ```compile_fail
//! let _ = craft_stream_codecs::decode_ccitt(&[], &craft_stream_codecs::CcittParams::default(), 1);
//! ```
//!
//! ```
//! let mut work = 1 << 20;
//! assert!(craft_stream_codecs::decode_jbig2_within(&[], None, 1, 1, 1 << 20, &mut work).is_err());
//! let p = craft_stream_codecs::CcittParams { columns: 8, ..Default::default() };
//! assert!(craft_stream_codecs::decode_ccitt_within(&[], &p, 1, &mut work).is_ok());
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod ascii85;
pub mod asciihex;
pub mod ccitt;
pub mod jbig2;
pub mod lzw;
mod mq;
pub mod predictor;
pub mod runlength;

pub use ccitt::{CcittParams, MAX_CCITT_COLUMNS, MAX_G4_COLUMNS, decode_ccitt_within, encode_g4};
pub use jbig2::{MAX_JBIG2_COLUMNS, MAX_JBIG2_PIXELS, SharedJbig2, decode_jbig2_within, encode_jbig2, encode_jbig2_generic, encode_jbig2_shared};

/// Parameters of the predictors and of LZW, as a PDF `/DecodeParms` dictionary gives them
/// (ISO 32000-2 Tables 8–9); TIFF's `Predictor`, `SamplesPerPixel`, `BitsPerSample` and
/// `ImageWidth` map onto the same four.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// 1 = none, 2 = TIFF predictor 2, 10–15 = PNG predictors.
    pub predictor: i64,
    pub colors: i64,
    pub bits_per_component: i64,
    pub columns: i64,
    /// LZW only: 1 (default) switches code width one code early, 0 does not.
    pub early_change: i64,
}

impl Default for Params {
    fn default() -> Self {
        Params { predictor: 1, colors: 1, bits_per_component: 8, columns: 1, early_change: 1 }
    }
}

/// Errors from decoding or encoding.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// `filter` names the codec, as PDF names its filter (`LZWDecode`, `CCITTFaxDecode`, …).
    #[error("{filter}: corrupt data: {detail}")]
    Corrupt { filter: &'static str, detail: String },
    #[error("decoded output exceeds the limit of {0} bytes")]
    LimitExceeded(usize),
    #[error("unsupported filter: {0}")]
    Unsupported(String),
    /// A run of images has nothing left to spend ([`pay`]).
    #[error("more image decoding than one operation may do")]
    Spent,
}

/// A decoder failure carrying whatever was decoded before the error.
pub struct Failure {
    pub error: CodecError,
    pub partial: Vec<u8>,
    /// Bytes decoded on the way that `partial` doesn't hold: they took as long.
    pub discarded: usize,
}

impl Failure {
    pub fn corrupt(filter: &'static str, detail: impl Into<String>, partial: Vec<u8>) -> Self {
        Failure { error: CodecError::Corrupt { filter, detail: detail.into() }, partial, discarded: 0 }
    }

    pub fn limit(max: usize) -> Self {
        Failure { error: CodecError::LimitExceeded(max), partial: Vec::new(), discarded: 0 }
    }
}

/// What a byte-stream decoder returns: the decoded bytes, or the failure with its partial output.
pub type Step = Result<Vec<u8>, Failure>;

/// Takes `steps` from `work`, what a run of images may still cost. What it can't pay for spends
/// all of it.
pub fn pay(work: &mut u64, steps: u64) -> Result<(), CodecError> {
    match work.checked_sub(steps) {
        Some(left) => {
            *work = left;
            Ok(())
        }
        None => {
            *work = 0;
            Err(CodecError::Spent)
        }
    }
}

/// PDF and PostScript white-space characters (ISO 32000-2 Table 1).
pub fn is_pdf_whitespace(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}
