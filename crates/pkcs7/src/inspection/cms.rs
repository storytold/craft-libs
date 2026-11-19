//! Borrowed CMS wire prefixes. These do not validate signed attributes or authenticate signers.
//! Optional/remaining fields are explicit cursors, governed by the parent inspection bounds.
//!
//! Each `parse` starts a new depth root. Callers traversing nested structures must retain their
//! whole-document work/depth policy; reparsing a slice does not carry a previous cursor's budget.
//! The required wire tags and exact SignedData OID are this view's structural profile, not a
//! replacement for a consumer's acceptance or diagnostic policy. Existing parsers remain unchanged.

use super::{Cursor, Element, InspectionError};
use crate::der::tag;

/// Required SignedData fields inside ContentInfo, without copying certificates or content.
/// All optional certificate/CRL/signer sets remain in `remaining_fields` for bounded inspection.
#[derive(Clone, Debug)]
pub struct SignedDataView<'a> {
    pub content_info: Element<'a>,
    pub signed_data: Element<'a>,
    pub version: Element<'a>,
    pub digest_algorithms: Element<'a>,
    pub encapsulated_content: Element<'a>,
    pub remaining_fields: Cursor<'a>,
    pub remaining_explicit_fields: Cursor<'a>,
    pub remaining_content_info_fields: Cursor<'a>,
}

impl<'a> SignedDataView<'a> {
    /// Read the required prefix and retain every tail. Matching the ContentInfo OID is not a
    /// signature check. Encapsulated content, certificates, attributes and identities are unvalidated.
    pub fn parse(input: &'a [u8]) -> Result<(Self, &'a [u8]), InspectionError> {
        let mut roots = Cursor::new(input);
        let content_info = roots.required(tag::SEQUENCE)?;
        let mut fields = content_info.children()?;
        let kind = fields.required(tag::OID)?;
        if kind.value() != [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 7, 2] {
            return Err(InspectionError::Field);
        }
        let explicit = fields.required(tag::ctx(0))?;
        let mut wrapped = explicit.children()?;
        let signed_data = wrapped.required(tag::SEQUENCE)?;
        let mut inner = signed_data.children()?;
        let version = inner.required(tag::INTEGER)?;
        let digest_algorithms = inner.required(tag::SET)?;
        let encapsulated_content = inner.required(tag::SEQUENCE)?;
        Ok((
            Self {
                content_info,
                signed_data,
                version,
                digest_algorithms,
                encapsulated_content,
                remaining_fields: inner,
                remaining_explicit_fields: wrapped,
                remaining_content_info_fields: fields,
            },
            roots.remaining(),
        ))
    }
}

/// One SignerInfo's raw required fields, not an authentication or identity-match result.
/// The identity remains its original issuer/serial SEQUENCE or context-specific key identifier.
/// Version consistency, algorithm policy, signed-attribute validation and re-encoding remain the
/// caller's responsibility; this view never substitutes an encapsulated content type or digest.
#[derive(Clone, Debug)]
pub struct SignerInfoView<'a> {
    pub signer_info: Element<'a>,
    pub version: Element<'a>,
    pub identifier: Element<'a>,
    pub digest_algorithm: Element<'a>,
    pub signed_attributes: Option<Element<'a>>,
    pub signature_algorithm: Element<'a>,
    pub signature: Element<'a>,
    pub remaining_fields: Cursor<'a>,
}

impl<'a> SignerInfoView<'a> {
    /// Read one signer's required wire fields, retaining any optional tail and subsequent signers.
    /// An absent signature field is an error, not an implicitly empty authenticated signature.
    pub fn parse(input: &'a [u8]) -> Result<(Self, &'a [u8]), InspectionError> {
        let mut roots = Cursor::new(input);
        let signer_info = roots.required(tag::SEQUENCE)?;
        let mut fields = signer_info.children()?;
        let version = fields.required(tag::INTEGER)?;
        let identifier = fields.next_element()?.ok_or(InspectionError::Field)?;
        if identifier.tag() != tag::SEQUENCE && identifier.tag() != tag::ctx_prim(0) {
            return Err(InspectionError::Field);
        }
        let digest_algorithm = fields.required(tag::SEQUENCE)?;
        let next = fields.next_element()?.ok_or(InspectionError::Field)?;
        let (signed_attributes, signature_algorithm) = if next.tag() == tag::ctx(0) {
            (Some(next), fields.required(tag::SEQUENCE)?)
        } else if next.tag() == tag::SEQUENCE {
            (None, next)
        } else {
            return Err(InspectionError::Field);
        };
        let signature = fields.required(tag::OCTET_STRING)?;
        Ok((
            Self { signer_info, version, identifier, digest_algorithm, signed_attributes, signature_algorithm, signature, remaining_fields: fields },
            roots.remaining(),
        ))
    }
}
