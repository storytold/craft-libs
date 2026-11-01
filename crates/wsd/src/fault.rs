//! SOAP 1.2 faults, parsed into a service-neutral value.
//!
//! A WSD service reports problems as a fault whose sub-code names the cause. This module reads the
//! structure and leaves the meaning of the sub-code (a namespace plus a local name) to the service
//! crate, which maps it onto its own enumeration.

use std::fmt;
use std::time::Duration;

use roxmltree::Node;

use crate::ns;
use crate::xml;

/// A resolved sub-code QName: the namespace (if the prefix was declared) and the local name.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SubCode {
    pub ns: Option<String>,
    pub local: String,
}

/// How many nested sub-codes are kept.
const MAX_NESTED: usize = 8;

impl SubCode {
    /// A sub-code `{ns}local`.
    pub fn new(ns: Option<&str>, local: impl Into<String>) -> Self {
        Self { ns: ns.map(str::to_owned), local: local.into() }
    }

    /// Is this exactly `{ns}local`? Prefixes never matter, namespaces do.
    pub fn is(&self, ns_uri: &str, local: &str) -> bool {
        self.ns.as_deref() == Some(ns_uri) && self.local == local
    }

    /// `wsa:ActionNotSupported`, in either WS-Addressing namespace.
    pub fn is_action_not_supported(&self) -> bool {
        self.local == "ActionNotSupported" && (self.ns.as_deref() == Some(ns::WSA) || self.ns.as_deref() == Some(ns::WSA_FINAL))
    }
}

/// Who the fault blames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// `soap:Sender`: the request was wrong.
    Sender,
    /// `soap:Receiver`: the device failed.
    Receiver,
    /// Another, or missing, top-level code.
    Other,
}

/// A parsed fault. Build one with [`Fault::new`] (the struct is `non_exhaustive`, so it can grow).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fault {
    pub role: Role,
    /// The first (outermost) sub-code; empty when the fault names none.
    pub code: SubCode,
    /// The sub-codes nested below `code`, outermost first (at most 8). Services that refine a
    /// code (a reason below a category) read them here.
    pub nested: Vec<SubCode>,
    /// The first reason text, capped.
    pub reason: String,
    /// The text of the detail, if any, capped.
    pub detail: String,
    /// The wait the device asked for with an HTTP `Retry-After` on the reply that carried this
    /// fault. The SOAP layer does not read it; [`crate::SoapClient`] fills it in from the transport.
    pub retry_after: Option<Duration>,
}

impl Fault {
    /// A fault with no nested sub-codes and no `Retry-After`.
    pub fn new(role: Role, code: SubCode, reason: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { role, code, nested: Vec::new(), reason: reason.into(), detail: detail.into(), retry_after: None }
    }

    /// The same fault with the device's `Retry-After`.
    #[must_use]
    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    /// Is the outermost sub-code exactly `{ns_uri}local`?
    pub fn is(&self, ns_uri: &str, local: &str) -> bool {
        self.code.is(ns_uri, local)
    }

    /// Is `{ns_uri}local` the outermost sub-code or any nested one?
    pub fn has_code(&self, ns_uri: &str, local: &str) -> bool {
        self.code.is(ns_uri, local) || self.nested.iter().any(|c| c.is(ns_uri, local))
    }

    /// Did the device blame the sender (the request was wrong)?
    pub fn is_sender(&self) -> bool {
        self.role == Role::Sender
    }

    /// Did the device blame itself?
    pub fn is_receiver(&self) -> bool {
        self.role == Role::Receiver
    }
}

impl fmt::Display for Fault {
    /// `the device refused: {sub-code}`, then the reason in parentheses and the detail in brackets when present.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the device refused: {}", self.code.local)?;
        if !self.reason.is_empty() {
            write!(f, " ({})", self.reason)?;
        }
        if !self.detail.is_empty() {
            write!(f, " [{}]", self.detail)?;
        }
        Ok(())
    }
}

/// Read a `soap:Fault` element. Missing parts degrade to empty values; this never fails.
pub fn parse_fault(fault: Node<'_, '_>) -> Fault {
    let code_el = xml::child(fault, ns::SOAP, "Code");
    let role = code_el.and_then(|c| xml::child(c, ns::SOAP, "Value")).and_then(|v| xml::resolve_qname(v, &xml::text(v))).map_or(
        Role::Other,
        |(uri, local)| match (uri.as_deref(), local.as_str()) {
            (Some(ns::SOAP), "Sender") => Role::Sender,
            (Some(ns::SOAP), "Receiver") => Role::Receiver,
            _ => Role::Other,
        },
    );
    // The first sub-code is `code`; the ones nested below it follow, outermost first.
    let mut chain = Vec::new();
    let mut level = code_el.and_then(|c| xml::child(c, ns::SOAP, "Subcode"));
    while let Some(sub) = level {
        if chain.len() > MAX_NESTED {
            break;
        }
        let value = xml::child(sub, ns::SOAP, "Value").and_then(|v| xml::resolve_qname(v, &xml::text(v)));
        chain.push(value.map_or_else(SubCode::default, |(ns, local)| SubCode { ns, local }));
        level = xml::child(sub, ns::SOAP, "Subcode");
    }
    let mut chain = chain.into_iter();
    let code = chain.next().unwrap_or_default();
    let nested: Vec<SubCode> = chain.collect();
    let reason = xml::child(fault, ns::SOAP, "Reason").and_then(|r| xml::child(r, ns::SOAP, "Text")).map(xml::text).unwrap_or_default();
    let detail = xml::child(fault, ns::SOAP, "Detail")
        .map(|d| {
            let mut parts: Vec<String> = Vec::new();
            for n in d.descendants().filter(|n| n.is_text()).take(32) {
                let t = n.text().unwrap_or("").trim();
                if !t.is_empty() {
                    parts.push(t.to_owned());
                }
            }
            parts.join(" ").chars().take(xml::MAX_TEXT).collect()
        })
        .unwrap_or_default();
    Fault { role, code, nested, reason: reason.chars().take(512).collect(), detail, retry_after: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVC: &str = "urn:example:service";

    /// A whole envelope with a sender fault whose sub-code is `sub`; `extra_ns` adds namespace
    /// declarations to the envelope.
    fn fault_xml(sub: &str, extra_ns: &str) -> String {
        format!(
            "<s:Envelope xmlns:s=\"{}\" xmlns:svc=\"{SVC}\" xmlns:wsa=\"{}\"{extra_ns}><s:Body><s:Fault><s:Code><s:Value>s:Sender</s:Value><s:Subcode><s:Value>{sub}</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang=\"en\">bad</s:Text></s:Reason><s:Detail><svc:JobId>7</svc:JobId></s:Detail></s:Fault></s:Body></s:Envelope>",
            ns::SOAP,
            ns::WSA
        )
    }

    /// Parse the first `soap:Fault` in `xml_text`.
    fn parse_one(xml_text: &str) -> Fault {
        let doc = xml::parse(xml_text).unwrap();
        let f = doc.descendants().find(|n| xml::is(*n, ns::SOAP, "Fault")).unwrap();
        parse_fault(f)
    }

    #[test]
    fn the_structure_is_read_and_subcodes_are_matched_by_namespace() {
        let f = parse_one(&fault_xml("svc:Busy", ""));
        assert_eq!(f.role, Role::Sender);
        assert_eq!((f.reason.as_str(), f.detail.as_str()), ("bad", "7"));
        assert!(f.code.is(SVC, "Busy"));
        assert!(f.to_string().contains("Busy"));
        // Another prefix for the same namespace is the same code; a look-alike namespace is not.
        assert!(parse_one(&fault_xml("q:Busy", &format!(" xmlns:q=\"{SVC}\""))).code.is(SVC, "Busy"));
        assert!(!parse_one(&fault_xml("q:Busy", " xmlns:q=\"urn:other\"")).code.is(SVC, "Busy"));
        assert!(parse_one(&fault_xml("wsa:ActionNotSupported", "")).code.is_action_not_supported());
        assert!(!parse_one(&fault_xml("svc:ActionNotSupported", "")).code.is_action_not_supported());
    }

    #[test]
    fn nested_subcodes_are_kept_outermost_first_and_bounded() {
        let nested = |levels: usize| {
            let mut inner = String::new();
            for i in (0..levels).rev() {
                inner = format!("<s:Subcode><s:Value>svc:L{i}</s:Value>{inner}</s:Subcode>");
            }
            format!(
                "<s:Envelope xmlns:s=\"{}\" xmlns:svc=\"{SVC}\"><s:Body><s:Fault><s:Code><s:Value>s:Receiver</s:Value>{inner}</s:Code></s:Fault></s:Body></s:Envelope>",
                ns::SOAP
            )
        };
        let f = parse_one(&nested(3));
        assert!(f.is(SVC, "L0") && f.has_code(SVC, "L0") && f.has_code(SVC, "L2") && !f.has_code(SVC, "L3"));
        assert_eq!(f.nested, vec![SubCode::new(Some(SVC), "L1"), SubCode::new(Some(SVC), "L2")]);
        assert!(!f.is(SVC, "L1"), "is() looks at the outermost code only");
        // A hostile chain is cut off, not followed to the end.
        let deep = parse_one(&nested(30));
        assert_eq!(deep.nested.len(), MAX_NESTED);
        assert!(f.is_receiver() && !f.is_sender());
        assert!(parse_one(&fault_xml("svc:X", "")).nested.is_empty());
    }

    #[test]
    fn constructor_and_retry_after() {
        let f = Fault::new(Role::Sender, SubCode::new(Some(SVC), "Busy"), "r", "d");
        assert_eq!((f.retry_after, f.nested.len()), (None, 0));
        let f = f.with_retry_after(Some(Duration::from_secs(4)));
        assert_eq!(f.retry_after, Some(Duration::from_secs(4)));
        assert!(f.is(SVC, "Busy") && f.is_sender());
        assert_eq!(SubCode::new(None, "x").ns, None);
    }

    #[test]
    fn undeclared_prefixes_bare_faults_and_roles_degrade_quietly() {
        assert_eq!(parse_one(&fault_xml("zz:Busy", "")).code, SubCode::default());
        assert_eq!(parse_one(&fault_xml("", "")).code, SubCode::default());
        let bare = format!("<s:Envelope xmlns:s=\"{}\"><s:Body><s:Fault/></s:Body></s:Envelope>", ns::SOAP);
        let f = parse_one(&bare);
        assert_eq!((f.role, f.reason.as_str(), f.detail.as_str()), (Role::Other, "", ""));
        assert_eq!(parse_one(&fault_xml("svc:X", "").replace("s:Sender", "s:Receiver")).role, Role::Receiver);
    }
}
