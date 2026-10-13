//! A bounded reader for the one XML shape all three protocols share.
//!
//! WebDAV, CardDAV and OCS all answer with a small tree whose useful content sits in leaf text.
//! Rather than model each schema, the reader walks the document once and reports every leaf as a
//! local name plus its text, grouped by an enclosing element the caller names. That is enough for
//! a `multistatus` of `response` elements and for an `ocs` envelope alike.
//!
//! Namespaces are handled by ignoring prefixes and matching on local names. Nextcloud, Sabre and
//! the RFCs disagree about prefixes (`d:`, `D:`, `DAV:`), and no element name we care about is
//! ambiguous once the prefix is gone.
//!
//! Bounds: the element depth and the number of groups are both capped, so a deeply nested or
//! endlessly repeating document cannot exhaust memory or the stack.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{Error, Result};

/// Deepest element nesting the reader will follow.
const MAX_DEPTH: usize = 64;
/// Most groups the reader will collect from one document.
const MAX_GROUPS: usize = 10_000;

/// One group of leaves, keyed by local element name.
///
/// A repeated name keeps every value, because `href` legitimately appears more than once in a
/// WebDAV response and dropping the extras would hide information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Group {
    leaves: Vec<(String, String)>,
}

impl Group {
    /// The first value for `name`, trimmed. Absent and empty are not distinguished, because no
    /// caller here needs to.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.leaves.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }

    /// Every value for `name`, in document order.
    ///
    /// A WebDAV `response` may legitimately carry more than one `href`, so this is part of the
    /// reader's contract even where no caller in this crate needs it yet.
    #[allow(dead_code)]
    pub(crate) fn all(&self, name: &str) -> impl Iterator<Item = &str> {
        self.leaves.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
    }

    /// True when any leaf named `name` is present, even if empty. WebDAV marks a collection with
    /// an empty `<d:collection/>`, so presence is the whole signal.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.leaves.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    }
}

/// Split `doc` into groups, one per element whose local name is `group`.
///
/// Every leaf inside a group is recorded. Leaves outside any group are dropped. An empty element
/// is recorded with an empty value, which is what makes [`Group::has`] work.
pub(crate) fn groups(doc: &[u8], group: &str) -> Result<Vec<Group>> {
    let mut reader = Reader::from_reader(doc);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut out: Vec<Group> = Vec::new();
    let mut buf = Vec::new();
    // The element path, by local name, so the reader knows when a group opens and closes.
    let mut path: Vec<String> = Vec::new();
    let mut group_at: Option<usize> = None;
    let mut current = Group::default();
    // Text of the element currently open, accumulated because quick-xml can split it.
    let mut text = String::new();
    let mut leaf_open = false;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if path.len() >= MAX_DEPTH {
                    return Err(Error::malformed("xml", format!("nesting deeper than {MAX_DEPTH}")));
                }
                // An element that opens while a leaf was open means the outer one was not a leaf.
                if leaf_open {
                    text.clear();
                }
                leaf_open = true;
                text.clear();
                path.push(name.clone());
                if group_at.is_none() && name.eq_ignore_ascii_case(group) {
                    group_at = Some(path.len());
                    current = Group::default();
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                if group_at.is_some() {
                    current.leaves.push((name, String::new()));
                }
                leaf_open = false;
                text.clear();
            }
            Ok(Event::Text(e)) => {
                if leaf_open && group_at.is_some() {
                    match e.decode() {
                        Ok(s) => text.push_str(&s),
                        // Lenient by design: an undecodable run is skipped, not fatal.
                        Err(err) => log::debug!("skipping undecodable XML text: {err}"),
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if leaf_open && group_at.is_some() {
                    text.push_str(&String::from_utf8_lossy(e.as_ref()));
                }
            }
            Ok(Event::End(_)) => {
                let closed = path.pop();
                if let (Some(name), Some(depth)) = (closed.as_deref(), group_at) {
                    if leaf_open && path.len() >= depth {
                        current.leaves.push((name.to_owned(), std::mem::take(&mut text)));
                    }
                    if path.len() < depth {
                        if out.len() >= MAX_GROUPS {
                            return Err(Error::malformed("xml", format!("more than {MAX_GROUPS} {group} elements")));
                        }
                        out.push(std::mem::take(&mut current));
                        group_at = None;
                    }
                }
                leaf_open = false;
                text.clear();
            }
            Ok(_) => {}
            Err(e) => return Err(Error::malformed("xml", e.to_string())),
        }
    }
    Ok(out)
}

/// Read the single outermost group, for documents that carry exactly one (the OCS envelope).
pub(crate) fn one_group(doc: &[u8], group: &str) -> Result<Group> {
    groups(doc, group)?.into_iter().next().ok_or_else(|| Error::malformed("xml", format!("no <{group}> element")))
}

/// Strip any namespace prefix from a qualified name.
fn local_name(qname: &[u8]) -> String {
    let bytes = match qname.iter().rposition(|b| *b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Escape text for an XML body. Only the five characters that matter.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}
