//! The single opaque inclusion/exclusion part of a mail-merge template.
use std::borrow::Cow;
use std::collections::HashSet;

use crate::DocxError;
use crate::package::MAX_EMBEDDED_XML;
use crate::xml;

pub(crate) fn recipient_kind(kind: &str) -> bool {
    // ECMA-376 §17.14.28 and its Transitional form, plus both legacy spellings.
    matches!(
        kind,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/recipientData"
            | "http://schemas.openxmlformats.org/officeDocument/2006/mailMergeRecipientData"
            | "http://schemas.openxmlformats.org/officeDocument/2006/relationships/mailMergeRecipientData"
            | "http://purl.oclc.org/ooxml/officeDocument/relationships/mailMergeRecipientData"
    )
}

/// Resolve only a package-local URI, refusing traversal above the package and ambiguous URI
/// syntax. Unsupported encodings refuse saving rather than guessing which ZIP entry they mean.
pub(crate) fn recipient_target(source: &str, target: &str) -> Option<String> {
    if target.is_empty()
        || target.trim() != target
        || target.starts_with("//")
        || target.rsplit('/').next().is_some_and(|part| matches!(part, "." | ".."))
        || target.chars().any(|c| c.is_control() || matches!(c, ':' | '\\' | '?' | '#' | '%'))
    {
        return None;
    }
    let dir = source.rsplit_once('/').map_or("", |(d, _)| d);
    let mut segments: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).collect() };
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "" => return None,
            "." => {}
            ".." => {
                segments.pop()?;
            }
            s => segments.push(s),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// Check the bounded root vocabulary, but never interpret, regenerate or evaluate record keys.
/// The original bytes are copied unchanged. Returns whether the root uses the Strict dialect.
pub(crate) fn validate_recipient(bytes: &[u8], content_type: &str) -> Result<bool, DocxError> {
    if bytes.len() > MAX_EMBEDDED_XML {
        return Err(DocxError::MailMerge("recipient data exceeds 1 MiB".into()));
    }
    // Import parsing repairs some malformed XML. A retained leaf instead needs validation of
    // the complete bytes we will copy, including namespaces, encoding and document grammar.
    let text = recipient_text(bytes)?;
    check_recipient_limits(&text, bytes)?;
    let parsed =
        roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { nodes_limit: MAX_EMBEDDED_XML as u32, ..Default::default() })
            .map_err(|e| DocxError::MailMerge(format!("invalid recipient data: {e}")))?;
    let root = parsed.root_element().tag_name();
    let namespace = match root.namespace() {
        Some("http://schemas.openxmlformats.org/wordprocessingml/2006/main" | "http://purl.oclc.org/ooxml/wordprocessingml/main") => {
            content_type.eq_ignore_ascii_case("application/vnd.openxmlformats-officedocument.wordprocessingml.mailMergeRecipientData+xml")
        }
        Some("http://schemas.microsoft.com/office/word/2006/wordml") => {
            content_type.eq_ignore_ascii_case("application/vnd.ms-word.mailMergeRecipientData+xml")
        }
        _ => false,
    };
    let valid = root.name() == "recipients" && namespace;
    if !valid {
        return Err(DocxError::MailMerge("unsupported recipient data content type or root".into()));
    }
    Ok(root.namespace() == Some("http://purl.oclc.org/ooxml/wordprocessingml/main"))
}

/// Bound work before the validating parser compares attributes or builds a tree.
fn check_recipient_limits(text: &str, bytes: &[u8]) -> Result<(), DocxError> {
    use quick_xml::events::Event;

    // A plain reader avoids namespace resolution and duplicate-attribute comparisons here.
    // The subsequent parser validates the complete XML and namespace grammar.
    let mut reader = quick_xml::Reader::from_str(text);
    let mut depth = 0usize;
    // Bound the aggregate number of namespace declarations. The second parser copies
    // in-scope namespace bindings when an element introduces new bindings.
    const MAX_NAMESPACE_DECLARATIONS: usize = 128;
    let mut declarations_left = MAX_NAMESPACE_DECLARATIONS;
    let mut first_event = true;
    loop {
        let event = reader.read_event().map_err(|e| DocxError::MailMerge(format!("invalid recipient data: {e}")))?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                if depth >= xml::MAX_DEPTH {
                    return Err(DocxError::MailMerge("recipient XML exceeds nesting limit".into()));
                }
                let mut names = HashSet::new();
                for attribute in element.attributes().with_checks(false) {
                    let attribute = attribute.map_err(|e| DocxError::MailMerge(format!("invalid recipient attribute: {e}")))?;
                    let name = attribute.key.into_inner();
                    if names.len() >= xml::MAX_ATTRS || !names.insert(name) {
                        return Err(DocxError::MailMerge("too many or duplicate recipient XML attributes".into()));
                    }
                    if name == b"xmlns" || name.starts_with(b"xmlns:") {
                        if attribute.value.len() > 256 {
                            return Err(DocxError::MailMerge("recipient XML namespace URI is too long".into()));
                        }
                        declarations_left = declarations_left
                            .checked_sub(1)
                            .ok_or_else(|| DocxError::MailMerge("too many recipient XML namespace declarations".into()))?;
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::Decl(declaration) => {
                if !first_event {
                    return Err(DocxError::MailMerge("unexpected recipient XML declaration".into()));
                }
                check_declaration(&declaration, bytes)?;
            }
            Event::PI(pi) if pi.target().eq_ignore_ascii_case(b"xml") => {
                return Err(DocxError::MailMerge("reserved processing instruction in recipient XML".into()));
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => return Ok(()),
            _ => {}
        }
        first_event = false;
    }
}

fn check_declaration(declaration: &quick_xml::events::BytesDecl<'_>, bytes: &[u8]) -> Result<(), DocxError> {
    let invalid = || DocxError::MailMerge("invalid recipient XML declaration".into());
    let raw = std::str::from_utf8(declaration).map_err(|_| invalid())?;
    let start = quick_xml::events::BytesStart::from_content(raw, 3);
    let mut attrs = start.attributes();
    let version = attrs.next().transpose().map_err(|_| invalid())?.ok_or_else(invalid)?;
    if version.key.as_ref() != b"version" || version.value.as_ref() != b"1.0" {
        return Err(invalid());
    }
    let mut next = attrs.next().transpose().map_err(|_| invalid())?;
    if let Some(encoding) = next.as_ref().filter(|a| a.key.as_ref() == b"encoding") {
        let expected = match bytes.get(..2) {
            Some([0xFF, 0xFE] | [0xFE, 0xFF]) => b"UTF-16".as_slice(),
            _ => b"UTF-8".as_slice(),
        };
        if !encoding.value.eq_ignore_ascii_case(expected) {
            return Err(DocxError::MailMerge("unsupported or mismatched recipient XML encoding".into()));
        }
        next = attrs.next().transpose().map_err(|_| invalid())?;
    }
    if let Some(standalone) = next
        && (standalone.key.as_ref() != b"standalone" || !matches!(standalone.value.as_ref(), b"yes" | b"no") || attrs.next().is_some())
    {
        return Err(invalid());
    }
    Ok(())
}

/// Decode only supported Unicode input, without repairing bytes that will later be copied.
fn recipient_text(bytes: &[u8]) -> Result<Cow<'_, str>, DocxError> {
    let little_endian = match bytes.get(..2) {
        Some([0xFF, 0xFE]) => Some(true),
        Some([0xFE, 0xFF]) => Some(false),
        _ => None,
    };
    if let Some(le) = little_endian {
        let rest = bytes.get(2..).unwrap_or_default();
        let (pairs, remainder) = rest.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(DocxError::MailMerge("incomplete UTF-16 recipient data".into()));
        }
        let units = pairs.iter().map(|pair| if le { u16::from_le_bytes(*pair) } else { u16::from_be_bytes(*pair) });
        let text = char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|e| DocxError::MailMerge(format!("invalid UTF-16 recipient data: {e}")))?;
        Ok(Cow::Owned(text))
    } else {
        std::str::from_utf8(bytes).map(Cow::Borrowed).map_err(|e| DocxError::MailMerge(format!("invalid UTF-8 recipient data: {e}")))
    }
}
