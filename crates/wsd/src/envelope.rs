//! SOAP 1.2 envelopes with WS-Addressing headers: build a request, read a reply.
//!
//! Everything service-specific (the action URIs, the body namespace, extra namespace declarations)
//! is passed in, so the same code serves print, scan and any other WSD service.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use roxmltree::Node;

use crate::error::{Error, Result};
use crate::fault::parse_fault;
use crate::ns;
use crate::xml;

/// A random version-4 style UUID (std only: random hasher keys, the clock and a counter).
pub fn new_uuid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let half = |salt: u64| {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(salt ^ nanos);
        h.write_u64(tick);
        h.finish()
    };
    let (a, b) = (half(1), half(2));
    let hi = (a & 0xffff_ffff_ffff_0fff) | 0x4000;
    let lo = (b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", hi >> 32, (hi >> 16) & 0xffff, hi & 0xffff, lo >> 48, lo & 0xffff_ffff_ffff)
}

/// The opening of every envelope we write; `body` goes between the Body tags.
///
/// `extra_ns` is a list of `(prefix, namespace)` pairs declared on the envelope next to `s` (SOAP)
/// and `wsa` (WS-Addressing), for the prefixes the `body` uses.
pub fn build_envelope(to: &str, action_uri: &str, message_id: &str, extra_ns: &[(&str, &str)], body: &str) -> String {
    build_envelope_with_headers(to, action_uri, message_id, extra_ns, "", body)
}

/// Like [`build_envelope`], with `headers` (already well-formed header blocks, such as the
/// reference parameters of an endpoint reference) added after the standard ones.
pub fn build_envelope_with_headers(to: &str, action_uri: &str, message_id: &str, extra_ns: &[(&str, &str)], headers: &str, body: &str) -> String {
    let extra: String = extra_ns.iter().map(|(p, u)| format!(" xmlns:{p}=\"{}\"", xml::esc(u))).collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{soap}\" xmlns:wsa=\"{wsa}\"{extra}><s:Header><wsa:To>{to}</wsa:To><wsa:Action>{action}</wsa:Action><wsa:MessageID>urn:uuid:{id}</wsa:MessageID><wsa:ReplyTo><wsa:Address>{anon}</wsa:Address></wsa:ReplyTo>{headers}</s:Header><s:Body>{body}</s:Body></s:Envelope>",
        soap = ns::SOAP,
        wsa = ns::WSA,
        to = xml::esc(to),
        action = xml::esc(action_uri),
        id = xml::esc(message_id),
        anon = ns::ANONYMOUS,
    )
}

/// What a reply must correspond to.
#[derive(Clone, Copy, Debug)]
pub struct Expect<'a> {
    /// The `wsa:Action` the reply must carry (when it carries one).
    pub action: &'a str,
    /// The request's message id (without `urn:uuid:`); a reply's `RelatesTo` must match when present.
    pub message_id: &'a str,
    /// The namespace and local name of the Body's first element.
    pub body_ns: &'a str,
    pub body_local: &'a str,
}

/// Parse a reply envelope and hand the response element to `read`.
///
/// A SOAP fault becomes [`Error::Fault`], whatever the HTTP status was. A reply whose action
/// header, correlation id or body element does not match the request is a protocol error. `E` is
/// the caller's error type, so a service crate can read its own messages with its own errors.
pub fn read_reply<T, E: From<Error>>(bytes: &[u8], expect: Expect<'_>, read: impl FnOnce(Node<'_, '_>) -> Result<T, E>) -> Result<T, E> {
    read_reply_hinted(bytes, expect, None, read)
}

/// [`read_reply`] for a caller that has the HTTP reply's `Retry-After`: a fault in the body carries
/// it ([`crate::Fault::retry_after`]).
pub fn read_reply_hinted<T, E: From<Error>>(
    bytes: &[u8],
    expect: Expect<'_>,
    retry_after: Option<Duration>,
    read: impl FnOnce(Node<'_, '_>) -> Result<T, E>,
) -> Result<T, E> {
    let text = xml::checked_text(bytes)?;
    let doc = xml::parse(text)?;
    let envelope = doc.root_element();
    let (body, header) = open(envelope)?;
    let payload = body.children().find(|c| c.is_element()).ok_or_else(|| Error::Protocol("the Body is empty".into()))?;
    if xml::is(payload, ns::SOAP, "Fault") {
        return Err(Error::from_fault(parse_fault(payload).with_retry_after(retry_after)).into());
    }
    check_header(header, expect.action, expect.message_id)?;
    if !xml::is(payload, expect.body_ns, expect.body_local) {
        return Err(Error::Protocol(format!("expected {}, found {}", expect.body_local, payload.tag_name().name())).into());
    }
    read(payload)
}

/// Like [`read_reply`] for an operation whose answer has no payload (WS-Eventing `Unsubscribe`): the
/// Body may be empty, or hold anything but a fault, which is ignored. Faults, the action header and
/// the correlation id are checked as for any reply.
pub fn read_ack<E: From<Error>>(bytes: &[u8], action: &str, message_id: &str) -> Result<(), E> {
    read_ack_hinted(bytes, action, message_id, None)
}

/// [`read_ack`] for a caller that has the HTTP reply's `Retry-After` (see [`read_reply_hinted`]).
pub fn read_ack_hinted<E: From<Error>>(bytes: &[u8], action: &str, message_id: &str, retry_after: Option<Duration>) -> Result<(), E> {
    let text = xml::checked_text(bytes)?;
    let doc = xml::parse(text)?;
    let (body, header) = open(doc.root_element())?;
    if let Some(payload) = body.children().find(|c| c.is_element())
        && xml::is(payload, ns::SOAP, "Fault")
    {
        return Err(Error::from_fault(parse_fault(payload).with_retry_after(retry_after)).into());
    }
    check_header(header, action, message_id)?;
    Ok(())
}

fn open<'a, 'i>(envelope: Node<'a, 'i>) -> Result<(Node<'a, 'i>, Option<Node<'a, 'i>>)> {
    if !xml::is(envelope, ns::SOAP, "Envelope") {
        return Err(Error::Protocol("the reply is not a SOAP 1.2 envelope".into()));
    }
    let body = xml::child(envelope, ns::SOAP, "Body").ok_or_else(|| Error::Protocol("the envelope has no Body".into()))?;
    Ok((body, xml::child(envelope, ns::SOAP, "Header")))
}

fn check_header(header: Option<Node<'_, '_>>, action: &str, message_id: &str) -> Result<()> {
    let Some(header) = header else { return Ok(()) };
    if let Some(a) = addressing(header, "Action")
        && a != action
    {
        return Err(Error::Protocol(format!("the reply action is {a:?}, expected {action:?}")));
    }
    if let Some(r) = addressing(header, "RelatesTo") {
        let got = r.strip_prefix("urn:uuid:").unwrap_or(&r);
        if !got.eq_ignore_ascii_case(message_id) {
            return Err(Error::Protocol("the reply answers a different request (RelatesTo does not match)".into()));
        }
    }
    Ok(())
}

/// The text of a WS-Addressing header block (either namespace), if present.
pub fn addressing(header: Node<'_, '_>, local: &str) -> Option<String> {
    header.children().find(|c| xml::is(*c, ns::WSA, local) || xml::is(*c, ns::WSA_FINAL, local)).map(xml::text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVC: &str = "urn:example:service";
    const ACT: &str = "urn:example:service/DoIt";
    const ACT_RESP: &str = "urn:example:service/DoItResponse";

    fn reply(action: &str, relates: Option<&str>, body: &str) -> String {
        let rel = relates.map(|r| format!("<wsa:RelatesTo>urn:uuid:{r}</wsa:RelatesTo>")).unwrap_or_default();
        format!(
            "<s:Envelope xmlns:s=\"{}\" xmlns:wsa=\"{}\" xmlns:svc=\"{SVC}\"><s:Header><wsa:Action>{action}</wsa:Action>{rel}</s:Header><s:Body>{body}</s:Body></s:Envelope>",
            ns::SOAP,
            ns::WSA
        )
    }

    fn expect(id: &str) -> Expect<'_> {
        Expect { action: ACT_RESP, message_id: id, body_ns: SVC, body_local: "DoItResponse" }
    }

    fn read(bytes: &[u8], id: &str) -> Result<()> {
        read_reply(bytes, expect(id), |_| Ok(()))
    }

    #[test]
    fn uuids_are_well_formed_and_distinct() {
        let a = new_uuid();
        let b = new_uuid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
        assert_eq!(a.chars().nth(14), Some('4'));
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn requests_are_well_formed_with_the_required_headers() {
        let env = build_envelope("http://127.0.0.1/p?x=<&>", ACT, "abc", &[("svc", SVC)], "<svc:DoItRequest/>");
        let doc = xml::parse(&env).unwrap();
        let root = doc.root_element();
        assert!(xml::is(root, ns::SOAP, "Envelope"));
        let h = xml::child(root, ns::SOAP, "Header").unwrap();
        assert_eq!(xml::text(xml::child(h, ns::WSA, "To").unwrap()), "http://127.0.0.1/p?x=<&>");
        assert_eq!(xml::text(xml::child(h, ns::WSA, "Action").unwrap()), ACT);
        assert_eq!(xml::text(xml::child(h, ns::WSA, "MessageID").unwrap()), "urn:uuid:abc");
        let rt = xml::child(h, ns::WSA, "ReplyTo").unwrap();
        assert_eq!(xml::text(xml::child(rt, ns::WSA, "Address").unwrap()), ns::ANONYMOUS);
        let b = xml::child(root, ns::SOAP, "Body").unwrap();
        assert!(xml::child(b, SVC, "DoItRequest").is_some());
    }

    #[test]
    fn a_matching_reply_is_read() {
        let r = reply(ACT_RESP, Some("abc"), "<svc:DoItResponse/>");
        let name = read_reply::<_, Error>(r.as_bytes(), expect("abc"), |n| Ok(n.tag_name().name().to_owned())).unwrap();
        assert_eq!(name, "DoItResponse");
        // No header at all is tolerated; correlation is checked only when present.
        let bare = format!("<s:Envelope xmlns:s=\"{}\" xmlns:svc=\"{SVC}\"><s:Body><svc:DoItResponse/></s:Body></s:Envelope>", ns::SOAP);
        assert!(read(bare.as_bytes(), "abc").is_ok());
    }

    #[test]
    fn mismatched_replies_are_protocol_errors() {
        let cases = [
            reply("urn:other", Some("abc"), "<svc:DoItResponse/>"),
            reply(ACT_RESP, Some("zzz"), "<svc:DoItResponse/>"),
            reply(ACT_RESP, Some("abc"), "<svc:OtherResponse/>"),
            reply(ACT_RESP, Some("abc"), ""),
            reply(ACT_RESP, Some("abc"), "<x:DoItResponse xmlns:x=\"urn:other\"/>"),
            "<Envelope/>".to_owned(),
            format!("<s:Envelope xmlns:s=\"{}\"/>", ns::SOAP),
        ];
        for c in cases {
            assert!(matches!(read(c.as_bytes(), "abc"), Err(Error::Protocol(_))), "{c}");
        }
        assert!(matches!(read(b"not xml", "abc"), Err(Error::Xml(_))));
        assert!(matches!(read(b"", "abc"), Err(Error::Xml(_))));
    }

    #[test]
    fn faults_win_over_everything() {
        let f = reply(
            ns::FAULT_ACTION,
            Some("other-id"),
            "<s:Fault><s:Code><s:Value>s:Receiver</s:Value><s:Subcode><s:Value>svc:Busy</s:Value></s:Subcode></s:Code><s:Reason><s:Text>busy</s:Text></s:Reason></s:Fault>",
        );
        match read(f.as_bytes(), "abc") {
            Err(Error::Fault(fault)) => {
                assert!(fault.code.is(SVC, "Busy"));
                assert_eq!(fault.reason, "busy");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn deep_and_doctype_replies_are_refused() {
        let deep = reply(ACT_RESP, Some("abc"), &format!("{}{}", "<a>".repeat(100), "</a>".repeat(100)));
        assert!(matches!(read(deep.as_bytes(), "abc"), Err(Error::Xml(_))));
        let dtd = format!("<!DOCTYPE x [<!ENTITY e \"1\">]>{}", reply("a", None, ""));
        assert!(matches!(read(dtd.as_bytes(), "abc"), Err(Error::Xml(_))));
    }
    #[test]
    fn extra_headers_are_placed_after_the_standard_ones() {
        let env =
            build_envelope_with_headers("http://127.0.0.1/p", ACT, "abc", &[("svc", SVC)], "<svc:Token>t&amp;1</svc:Token>", "<svc:DoItRequest/>");
        let doc = xml::parse(&env).unwrap();
        let h = xml::child(doc.root_element(), ns::SOAP, "Header").unwrap();
        let names: Vec<_> = h.children().filter(|c| c.is_element()).map(|c| c.tag_name().name().to_owned()).collect();
        assert_eq!(names, ["To", "Action", "MessageID", "ReplyTo", "Token"]);
        assert_eq!(xml::text(xml::child(h, SVC, "Token").unwrap()), "t&1");
        // No extra headers: identical to the plain builder.
        assert_eq!(build_envelope("u", ACT, "i", &[], "<b/>"), build_envelope_with_headers("u", ACT, "i", &[], "", "<b/>"));
    }

    #[test]
    fn an_ack_may_have_an_empty_body_but_not_a_fault_or_a_wrong_correlation() {
        let ack = |a: &str, rel: Option<&str>, body: &str| read_ack::<Error>(reply(a, rel, body).as_bytes(), ACT_RESP, "abc");
        assert!(ack(ACT_RESP, Some("abc"), "").is_ok());
        assert!(ack(ACT_RESP, Some("abc"), "<svc:Anything/>").is_ok());
        assert!(ack(ACT_RESP, None, "").is_ok());
        assert!(matches!(ack("urn:other", Some("abc"), ""), Err(Error::Protocol(_))));
        assert!(matches!(ack(ACT_RESP, Some("zzz"), ""), Err(Error::Protocol(_))));
        let fault = "<s:Fault><s:Code><s:Value>s:Sender</s:Value></s:Code><s:Reason><s:Text>no</s:Text></s:Reason></s:Fault>";
        assert!(matches!(ack(ns::FAULT_ACTION, Some("abc"), fault), Err(Error::Fault(_))));
        assert!(matches!(read_ack::<Error>(b"<x/>", ACT_RESP, "abc"), Err(Error::Protocol(_))));
        assert!(matches!(read_ack::<Error>(b"junk", ACT_RESP, "abc"), Err(Error::Xml(_))));
    }
}
