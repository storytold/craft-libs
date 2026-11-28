//! Preserve a mail-merge main document, never open its external sources or run its query.
use std::collections::BTreeMap;

use craft_wordmodel::Document;

use super::PartRels;
use crate::DocxError;
use crate::package::{MAIL_MERGE_RELS, MAX_EMBEDDED_RELS, MAX_EMBEDDED_XML, Rels, SETTINGS_ORIG};
use crate::xml::{self, Node, W};

/// Keep the settings part's relationship map with the original settings. Both survive native
/// saves/recovery through the existing passthrough store; unrelated relationships are not emitted.
pub(crate) fn keep_rels(rels: &Rels) -> Result<Vec<u8>, DocxError> {
    if rels.list.len() > MAX_EMBEDDED_RELS {
        return Err(DocxError::Limit("too many mail merge settings relationships".into()));
    }
    let mut w = W::new();
    w.open("Relationships", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships")]);
    // Budget escaped attributes before allocating the serialized relationship map.
    let too_large = || DocxError::Limit("mail merge settings relationships exceed 1 MiB".into());
    let mut remaining = MAX_EMBEDDED_XML.saturating_sub(w.s.len()).saturating_sub("</Relationships>".len());
    for r in &rels.list {
        remaining = remaining.checked_sub(br#"<Relationship Id="" Type="" Target=""/>"#.len()).ok_or_else(too_large)?;
        if r.external {
            remaining = remaining.checked_sub(br#" TargetMode="External""#.len()).ok_or_else(too_large)?;
        }
        for value in [&r.id, &r.kind, &r.target] {
            for byte in value.bytes() {
                let size = match byte {
                    b'&' => 5,
                    b'<' | b'>' | b'\t' => 4,
                    b'"' | b'\'' => 6,
                    b'\n' | b'\r' => 5,
                    _ => 1,
                };
                remaining = remaining.checked_sub(size).ok_or_else(too_large)?;
            }
        }
    }
    for r in &rels.list {
        let mut attrs = vec![("Id", r.id.as_str()), ("Type", r.kind.as_str()), ("Target", r.target.as_str())];
        if r.external {
            attrs.push(("TargetMode", "External"));
        }
        w.empty("Relationship", &attrs);
    }
    w.close("Relationships");
    let bytes = w.into_bytes();
    if bytes.len() > MAX_EMBEDDED_XML {
        return Err(DocxError::Limit("mail merge settings relationships exceed 1 MiB".into()));
    }
    Ok(bytes)
}

/// Rebind external sources and the single embedded recipient leaf to newly allocated IDs.
pub(super) fn write(doc: &Document, out: &mut PartRels, embeds: &mut super::embed::EmbedWriter<'_>) -> Result<Option<String>, DocxError> {
    let missing_settings = || {
        if doc.passthrough.contains_key(MAIL_MERGE_RELS) {
            Err(DocxError::MailMerge("retained mail merge settings are missing".into()))
        } else {
            Ok(None)
        }
    };
    let Some(orig) = doc.passthrough.get(SETTINGS_ORIG) else { return missing_settings() };
    let settings = match xml::parse(orig) {
        Ok(settings) => settings,
        Err(_) if !doc.passthrough.contains_key(MAIL_MERGE_RELS) => return Ok(None),
        Err(error) => return Err(DocxError::MailMerge(format!("unreadable retained settings: {error}"))),
    };
    let Some(original) = settings.child("w:mailMerge") else { return missing_settings() };
    if settings.children("w:mailMerge").count() != 1 {
        return Err(DocxError::MailMerge("multiple mailMerge elements".into()));
    }
    let rels = doc
        .passthrough
        .get(MAIL_MERGE_RELS)
        .map(|bytes| {
            if bytes.len() > MAX_EMBEDDED_XML {
                return Err(DocxError::MailMerge("retained source relationships exceed 1 MiB".into()));
            }
            xml::parse(bytes).map_err(|error| DocxError::MailMerge(format!("unreadable retained source relationships: {error}")))
        })
        .transpose()?;
    let mut by_id = BTreeMap::new();
    for rel in rels.iter().flat_map(|root| root.els()) {
        if let Some(id) = rel.attr("Id") {
            by_id.entry(id).and_modify(|entry| *entry = None).or_insert(Some(rel));
        }
    }
    let mut merge = original.clone();
    let mut ids = BTreeMap::<String, String>::new();
    let recipient_id = original.child("w:odso").and_then(|o| o.child("w:recipientData")).and_then(|r| r.attr("r:id"));
    let mut recipient = None;
    let mut pending = vec![&mut merge];
    while let Some(e) = pending.pop() {
        if e.name.starts_with("?:") || e.attrs.iter().any(|(k, _)| k.starts_with("?:")) {
            return Err(DocxError::MailMerge("unrecognised namespace in mailMerge".into()));
        }
        let is_recipient = e.name == "w:recipientData";
        if is_recipient
            && (recipient.is_some()
                || recipient_id.is_none_or(|id| id.trim().is_empty())
                || e.attrs.iter().filter(|(name, _)| name == "r:id").count() != 1
                || e.attr("r:id") != recipient_id)
        {
            return Err(DocxError::MailMerge("expected one recipient data reference".into()));
        }
        let source_type = match e.name.as_str() {
            "w:dataSource" | "w:src" => Some("mailMergeSource"),
            "w:headerSource" => Some("mailMergeHeaderSource"),
            _ => None,
        };
        if source_type.is_some()
            && (e.attrs.iter().filter(|(name, _)| name == "r:id").count() != 1 || e.attr("r:id").is_none_or(|id| id.trim().is_empty()))
        {
            return Err(DocxError::MailMerge("expected one nonempty source relationship ID".into()));
        }
        for (name, id) in &mut e.attrs {
            if name != "r:id" {
                continue;
            }
            let rel = match by_id.get(id.as_str()) {
                Some(Some(rel)) => rel,
                Some(None) => return Err(DocxError::MailMerge(format!("ambiguous source relationship {id}"))),
                None => return Err(DocxError::MailMerge(format!("missing source relationship {id}"))),
            };
            let kind = rel.attr("Type").unwrap_or("");
            let target = rel.attr("Target").unwrap_or("");
            let external = rel.attr("TargetMode").is_some_and(|v| v.eq_ignore_ascii_case("External"));
            let source = ["mailMergeSource", "mailMergeHeaderSource"].iter().any(|suffix| {
                source_type.is_none_or(|expected| expected == *suffix)
                    && (kind == format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{suffix}")
                        || kind == format!("http://purl.oclc.org/ooxml/officeDocument/relationships/{suffix}"))
            });
            let supported = if is_recipient {
                rel.attr("TargetMode").is_none_or(|v| v.eq_ignore_ascii_case("Internal")) && crate::mail_merge::recipient_kind(kind)
            } else {
                external && source
            };
            if !supported || target.trim().is_empty() {
                return Err(DocxError::MailMerge(format!("unsupported or empty mail merge relationship {id}")));
            }
            if let Some(new) = ids.get(id) {
                *id = new.clone();
                continue;
            }
            let new = if is_recipient {
                let new = out.add(crate::mail_merge::WORD_RECIPIENT_REL, "", false);
                recipient = Some((new.clone(), target));
                new
            } else {
                out.add(kind, target, true)
            };
            ids.insert(id.clone(), new.clone());
            *id = new;
        }
        pending.extend(e.kids.iter_mut().filter_map(|n| match n {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        }));
    }
    // Only write the part after validating all of the relationship references. A caller
    // that explicitly discards an unsupported mail merge must leave no orphaned recipient data.
    if let Some((id, target)) = recipient {
        if settings_is_strict(orig)? {
            return Err(DocxError::MailMerge("write-back of Strict mail merge recipient settings is not supported".into()));
        }
        let Some((_, _, path, _)) = out.list.iter_mut().find(|(rid, ..)| rid == &id) else {
            return Err(DocxError::MailMerge("missing allocated recipient data ID".into()));
        };
        let written = embeds.recipient_data(target)?;
        *path = craft_opc::relative("word/settings.xml", &written);
    }
    // Original producers may use other prefixes. The canonical subtree declares its own aliases.
    merge.attrs.push(("xmlns:w".into(), "http://schemas.openxmlformats.org/wordprocessingml/2006/main".into()));
    merge.attrs.push(("xmlns:r".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships".into()));
    Ok(Some(merge.to_xml()))
}

const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

fn settings_is_strict(settings: &[u8]) -> Result<bool, DocxError> {
    let mut reader = quick_xml::NsReader::from_reader(settings);
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|e| DocxError::MailMerge(format!("unreadable settings: {e}")))?;
        match event {
            quick_xml::events::Event::Start(_) | quick_xml::events::Event::Empty(_) => {
                return match namespace {
                    quick_xml::name::ResolveResult::Bound(ns) if ns.as_ref() == b"http://purl.oclc.org/ooxml/wordprocessingml/main" => Ok(true),
                    quick_xml::name::ResolveResult::Bound(ns) if ns.as_ref() == b"http://schemas.openxmlformats.org/wordprocessingml/2006/main" => {
                        Ok(false)
                    }
                    _ => Err(DocxError::MailMerge("unsupported settings namespace".into())),
                };
            }
            quick_xml::events::Event::Eof => return Err(DocxError::MailMerge("missing settings root".into())),
            _ => {}
        }
    }
}

/// Match source relationship types to the settings vocabulary actually emitted after merging.
pub(super) fn source_relationships(settings: &[u8], rels: PartRels) -> Result<PartRels, DocxError> {
    let namespace = if settings_is_strict(settings)? { STRICT } else { TRANSITIONAL };
    let mut out = PartRels::default();
    for (id, kind, target, external) in rels.list {
        let kind = match kind.rsplit_once('/') {
            Some((prefix, suffix)) if [TRANSITIONAL, STRICT].contains(&prefix) && matches!(suffix, "mailMergeSource" | "mailMergeHeaderSource") => {
                format!("{namespace}/{suffix}")
            }
            _ => kind,
        };
        out.add_with_id(&id, &kind, &target, external);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Rel;

    #[test]
    fn relationship_byte_budget_accounts_for_escaped_attributes() {
        let mut rels = Rels::default();
        rels.list.push(Rel { id: "source&".into(), kind: "mailMergeSource".into(), target: "é<&>\"'\t\n\r".into(), external: true });
        let size = keep_rels(&rels).unwrap().len();
        rels.list[0].target.push_str(&"x".repeat(MAX_EMBEDDED_XML - size));
        assert_eq!(keep_rels(&rels).unwrap().len(), MAX_EMBEDDED_XML, "the exact encoded byte budget remains usable");
        rels.list[0].target.push('x');
        assert!(matches!(keep_rels(&rels), Err(DocxError::Limit(_))), "one encoded byte over the budget is refused");
    }
}
