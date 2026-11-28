//! Bounded, borrowed inspection of definite-length ASN.1 and X.509 wire fields.
//!
//! These views are evidence, not validated certificates or signature/trust verdicts. They do
//! not replace [`crate::x509::Certificate`]. Unknown algorithms and original encodings are kept.
//! The profile follows Craft Anything's inspection reader (including Afloat's raw-field and
//! non-minimal-length work); callers retain their existing semantic and verification checks.

use crate::der::{Tlv, tag};

pub mod cms;

/// Maximum siblings read by one cursor, matching the inspection consumer's bound.
pub const MAX_CHILDREN: usize = 4096;
/// Maximum nesting accepted when obtaining child cursors. Root elements have depth zero.
pub const MAX_DEPTH: usize = 32;

/// An inspection stopped without establishing that the remaining bytes are complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InspectionError {
    #[error("malformed or unsupported definite-length element")]
    Malformed,
    #[error("inspection child limit reached")]
    ChildLimit,
    #[error("inspection depth limit reached")]
    DepthLimit,
    #[error("missing or unexpected certificate field")]
    Field,
}

/// One borrowed wire element. Redundant definite lengths are readable but explicitly noted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element<'a> {
    tlv: Tlv<'a>,
    depth: usize,
    non_minimal_length: bool,
}

impl<'a> Element<'a> {
    pub fn tag(&self) -> u8 {
        self.tlv.tag
    }
    pub fn raw(&self) -> &'a [u8] {
        self.tlv.raw
    }
    pub fn value(&self) -> &'a [u8] {
        self.tlv.value
    }
    pub fn non_minimal_length(&self) -> bool {
        self.non_minimal_length
    }

    /// Inspect children without allocating a child list. Primitive values are not interpreted as children.
    pub fn children(&self) -> Result<Cursor<'a>, InspectionError> {
        if self.tlv.tag & 0x20 == 0 {
            return Err(InspectionError::Field);
        }
        // Prevent callers following nested wire fields from silently exceeding the profile.
        if self.depth >= MAX_DEPTH {
            return Err(InspectionError::DepthLimit);
        }
        Ok(Cursor::at_depth(self.tlv.value, self.depth + 1))
    }
}

/// A non-recursive sibling cursor with a hard per-list work bound and no child-list allocation.
///
/// Successful reads allocate no memory; the underlying parser may allocate an error message.
/// `Ok(None)` means complete input, never a malformed/limited prefix. Errors are sticky and
/// leave the offending remainder available. Separate cursors have separate sibling budgets;
/// this does not impose a global budget on an application's traversal of many lists.
#[derive(Clone, Debug)]
pub struct Cursor<'a> {
    remaining: &'a [u8],
    count: usize,
    depth: usize,
    stopped: Option<InspectionError>,
}

impl<'a> Cursor<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self::at_depth(input, 0)
    }
    fn at_depth(input: &'a [u8], depth: usize) -> Self {
        Self { remaining: input, count: 0, depth, stopped: None }
    }
    /// Bytes not yet inspected. An empty remainder alone is not a recursive DER validation.
    pub fn remaining(&self) -> &'a [u8] {
        self.remaining
    }
    pub fn next_element(&mut self) -> Result<Option<Element<'a>>, InspectionError> {
        if let Some(error) = self.stopped {
            return Err(error);
        }
        if self.remaining.is_empty() {
            return Ok(None);
        }
        if self.count == MAX_CHILDREN {
            self.stopped = Some(InspectionError::ChildLimit);
            return Err(InspectionError::ChildLimit);
        }
        let input = self.remaining;
        let (tlv, rest) = match Tlv::parse(input) {
            Ok(value) => value,
            Err(_) => {
                self.stopped = Some(InspectionError::Malformed);
                return Err(InspectionError::Malformed);
            }
        };
        let non_minimal_length = input.get(1).is_some_and(|b| b & 0x80 != 0) && (input.get(2) == Some(&0) || tlv.value.len() < 128);
        self.remaining = rest;
        self.count += 1;
        Ok(Some(Element { tlv, depth: self.depth, non_minimal_length }))
    }
    fn required(&mut self, expected: u8) -> Result<Element<'a>, InspectionError> {
        self.next_element()?.filter(|e| e.tag() == expected).ok_or(InspectionError::Field)
    }
}

/// Borrowed required X.509 wire fields, without algorithm, signature, key or trust validation.
///
/// The BIT STRING is retained whole, including its unused-bit count. Both AlgorithmIdentifier
/// encodings are retained independently: their equality and parameter policy are the verifier's
/// responsibility. Optional/trailing fields remain in bounded cursors rather than being silently
/// reported as complete. Parsing this view does not walk nested names, keys or extensions.
#[derive(Clone, Debug)]
pub struct CertificateView<'a> {
    pub certificate: Element<'a>,
    pub tbs: Element<'a>,
    pub version: Option<Element<'a>>,
    pub serial: Element<'a>,
    pub signed_algorithm: Element<'a>,
    pub issuer: Element<'a>,
    pub validity: Element<'a>,
    pub subject: Element<'a>,
    pub spki: Element<'a>,
    pub signature_algorithm: Element<'a>,
    pub signature_bits: Element<'a>,
    pub remaining_certificate_fields: Cursor<'a>,
    pub remaining_tbs_fields: Cursor<'a>,
}

impl<'a> CertificateView<'a> {
    /// Inspect required fields of the first certificate; return bytes following its outer TLV.
    /// Readable prefixes do not imply complete optional fields. Inspect the returned cursors to
    /// distinguish complete, malformed, and limited tails. Existing strict verification stays separate.
    pub fn parse(input: &'a [u8]) -> Result<(Self, &'a [u8]), InspectionError> {
        let mut roots = Cursor::new(input);
        let certificate = roots.required(tag::SEQUENCE)?;
        let mut outer = certificate.children()?;
        let tbs = outer.required(tag::SEQUENCE)?;
        let signature_algorithm = outer.required(tag::SEQUENCE)?;
        let signature_bits = outer.required(tag::BIT_STRING)?;
        let mut fields = tbs.children()?;
        let first = fields.next_element()?.ok_or(InspectionError::Field)?;
        let (version, serial) = if first.tag() == tag::ctx(0) {
            (Some(first), fields.required(tag::INTEGER)?)
        } else if first.tag() == tag::INTEGER {
            (None, first)
        } else {
            return Err(InspectionError::Field);
        };
        let signed_algorithm = fields.required(tag::SEQUENCE)?;
        let issuer = fields.required(tag::SEQUENCE)?;
        let validity = fields.required(tag::SEQUENCE)?;
        let subject = fields.required(tag::SEQUENCE)?;
        let spki = fields.required(tag::SEQUENCE)?;
        Ok((
            Self {
                certificate,
                tbs,
                version,
                serial,
                signed_algorithm,
                issuer,
                validity,
                subject,
                spki,
                signature_algorithm,
                signature_bits,
                remaining_certificate_fields: outer,
                remaining_tbs_fields: fields,
            },
            roots.remaining(),
        ))
    }
}
