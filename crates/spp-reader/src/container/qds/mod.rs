//! Qt `QDataStream` values (big-endian, Qt 5 encoding) as the reference app writes them in `.ini`, `.toc`, image headers and `.alg_meta` records.
//!
//! `stream` holds the sequential reader ([`QDataStream`]), `variant` the decoded value types
//! ([`QVariant`], [`QVariantMap`]) and `error` the [`QdsError`] type.

mod error;
mod stream;
mod variant;

#[cfg(test)]
pub(crate) mod test_encoder;

pub use error::QdsError;
pub use stream::QDataStream;
pub use variant::{QVariant, QVariantMap};
