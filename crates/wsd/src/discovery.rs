//! WS-Discovery, the message layer: builders and parsers for `Probe`, `ProbeMatches`, `Resolve`,
//! `ResolveMatches`, `Hello` and `Bye`, the matching rules, and a [`discover`] driver.
//!
//! **This module opens no socket and sends nothing by itself.** The driver talks to a
//! [`DiscoverySocket`], a two-method trait (send a datagram, receive one before a deadline). The
//! crate ships an in-memory implementation for tests (`fake`, behind the `testing` feature) and *no* implementation that
//! touches the network: a real UDP socket, with the multicast group join and the choice of
//! interface that come with it, is deliberately left to the application that owns the decision
//! (see `docs/wsd.md`). [`Destination::Multicast`] only says "the discovery group"; the socket
//! implementation decides what that means.
//!
//! Safety rules applied here, whatever the socket:
//!
//! * a datagram is considered only if its source address passes the same private-network policy as
//!   device URLs ([`crate::policy::address_allowed`]);
//! * advertised `XAddrs` become usable [`DeviceUrl`]s only through that policy, so a public address
//!   or host name in an announcement is dropped (and counted), never connected to, and a loopback
//!   address is kept only when the announcement came from loopback;
//! * everything parsed is bounded (datagram size, list lengths, device count), and a message with a
//!   duplicated singular element is refused rather than guessed at;
//! * replies are accepted only if they correlate (`RelatesTo`) with the probe or resolve we sent,
//!   and an `AppSequence` that goes backwards for a known device is ignored.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use roxmltree::Node;

use crate::addressing::EndpointRef;
use crate::cancel::CancelToken;
use crate::clock::Instant;
use crate::envelope::{addressing, new_uuid};
use crate::error::{Error, Result};
use crate::ns;
use crate::policy::{DeviceUrl, address_allowed};
use crate::service::ServiceKind;
use crate::xml::{self, QName};

pub const ACTION_HELLO: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/Hello";
pub const ACTION_BYE: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/Bye";
pub const ACTION_PROBE: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe";
pub const ACTION_PROBE_MATCHES: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/ProbeMatches";
pub const ACTION_RESOLVE: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/Resolve";
pub const ACTION_RESOLVE_MATCHES: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/ResolveMatches";

pub const MATCH_BY_RFC3986: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/rfc3986";
pub const MATCH_BY_UUID: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/uuid";
pub const MATCH_BY_LDAP: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/ldap";
pub const MATCH_BY_STRCMP0: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/strcmp0";
pub const MATCH_BY_NONE: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery/none";

/// The largest UDP payload; bigger "datagrams" are refused.
pub const MAX_DATAGRAM: usize = 65_507;
const MAX_TYPES: usize = 64;
const MAX_SCOPES: usize = 64;
const MAX_XADDRS: usize = 32;
const MAX_MATCHES: usize = 64;
const MAX_TOKEN: usize = 1024;
/// Datagrams examined per listening slice before the rest are counted as a flood and skipped.
const MAX_DATAGRAMS_PER_SLICE: usize = 4096;
const MAX_RESOLVES: usize = 8;
/// How many `Bye`s are remembered per run (see `Collector::byes`).
const MAX_BYES: usize = 1024;
/// The port directed resolves are addressed to: the well-known discovery port, a protocol constant
/// and not a socket. A socket implementation that needs another port maps the destination itself.
const DISCOVERY_PORT: u16 = 3702;

// ---- sequence ----

/// `wsd:AppSequence`: orders announcements from one device across restarts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSequence {
    /// Increases each time the device restarts.
    pub instance_id: u64,
    pub sequence_id: Option<String>,
    /// Increases with every message of the instance.
    pub message_number: u64,
}

impl AppSequence {
    /// A sequence with no `SequenceId`.
    pub fn new(instance_id: u64, message_number: u64) -> Self {
        Self { instance_id, sequence_id: None, message_number }
    }

    /// Does this message come after `prev` from the same device? Sequences with different
    /// `SequenceId`s are not comparable and count as newer; otherwise the pair (instance, number)
    /// must strictly increase.
    pub fn supersedes(&self, prev: &AppSequence) -> bool {
        self.sequence_id != prev.sequence_id || (self.instance_id, self.message_number) > (prev.instance_id, prev.message_number)
    }
}

// ---- targets and queries ----

/// A discoverable device as an announcement or a match describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    /// The endpoint address, usually `urn:uuid:...`; the stable identity of the device.
    pub address: String,
    pub types: Vec<QName>,
    pub scopes: Vec<String>,
    /// The advertised transport addresses, unvalidated. Use [`Self::device_urls`].
    pub xaddrs: Vec<String>,
    pub metadata_version: Option<u64>,
}

impl Target {
    /// Whether the target announces the type `q` (exact namespace and local name).
    pub fn has_type(&self, q: &QName) -> bool {
        self.types.contains(q)
    }

    /// The advertised addresses that pass the connection policy (numeric loopback, link-local or
    /// private `http` URLs), without duplicates, and how many were dropped.
    ///
    /// `from` is the source of the announcement. A loopback address names this machine, so it is
    /// kept only when the announcement itself came from loopback: a device on the network cannot
    /// point the caller at a local service.
    pub fn device_urls(&self, from: IpAddr) -> (Vec<DeviceUrl>, usize) {
        let mut urls: Vec<DeviceUrl> = Vec::new();
        let mut dropped = 0;
        for x in &self.xaddrs {
            match DeviceUrl::parse(x) {
                Ok(u) if !u.reachable_from(from) => dropped += 1,
                Ok(u) if !urls.contains(&u) => urls.push(u),
                Ok(_) => {}
                Err(_) => dropped += 1,
            }
        }
        (urls, dropped)
    }
}

/// How a probe's scopes are compared with a target's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchBy {
    /// The default: the probe scope is a path-prefix of the target scope after normalising case of
    /// scheme and host and trailing slashes; `..` segments never match.
    Rfc3986,
    Uuid,
    Ldap,
    /// Exact string equality.
    Strcmp0,
    /// No scope ever matches: a probe carrying scopes under this rule matches nothing.
    None,
    /// An unrecognised rule URI: nothing matches.
    Unknown(String),
}

impl MatchBy {
    /// The rule a `MatchBy` attribute names; an absent attribute means [`MatchBy::Rfc3986`], and an
    /// unknown URI is kept as [`MatchBy::Unknown`].
    pub fn from_uri(uri: Option<&str>) -> Self {
        match uri.map(str::trim) {
            None | Some(MATCH_BY_RFC3986) => Self::Rfc3986,
            Some(MATCH_BY_UUID) => Self::Uuid,
            Some(MATCH_BY_LDAP) => Self::Ldap,
            Some(MATCH_BY_STRCMP0) => Self::Strcmp0,
            Some(MATCH_BY_NONE) => Self::None,
            Some(other) => Self::Unknown(other.to_owned()),
        }
    }

    /// The rule's URI as written in a `MatchBy` attribute.
    pub fn uri(&self) -> &str {
        match self {
            Self::Rfc3986 => MATCH_BY_RFC3986,
            Self::Uuid => MATCH_BY_UUID,
            Self::Ldap => MATCH_BY_LDAP,
            Self::Strcmp0 => MATCH_BY_STRCMP0,
            Self::None => MATCH_BY_NONE,
            Self::Unknown(u) => u,
        }
    }
}

/// A scope with its scheme and host lowercased and trailing slashes removed, for RFC 3986 prefix matching.
fn normalise_scope(s: &str) -> String {
    let s = s.trim();
    let (head, path) = match s.find("://") {
        Some(i) => {
            let rest = s.get(i + 3..).unwrap_or("");
            let end = rest.find(['/', '?']).unwrap_or(rest.len());
            (
                format!("{}://{}", s.get(..i).unwrap_or("").to_ascii_lowercase(), rest.get(..end).unwrap_or("").to_ascii_lowercase()),
                rest.get(end..).unwrap_or("").to_owned(),
            )
        }
        None => (s.to_ascii_lowercase(), String::new()),
    };
    format!("{head}{}", path.trim_end_matches('/'))
}

/// Does `target` satisfy the probe scope `probe` under `rule`?
pub fn scope_matches(rule: &MatchBy, probe: &str, target: &str) -> bool {
    match rule {
        MatchBy::Rfc3986 => {
            let (p, t) = (normalise_scope(probe), normalise_scope(target));
            if p.split('/').any(|seg| seg == "..") || t.split('/').any(|seg| seg == "..") {
                return false;
            }
            t == p || t.strip_prefix(&p).is_some_and(|rest| rest.starts_with('/'))
        }
        MatchBy::Strcmp0 => probe == target,
        MatchBy::Uuid | MatchBy::Ldap => probe.eq_ignore_ascii_case(target),
        MatchBy::None | MatchBy::Unknown(_) => false,
    }
}

/// What a `Probe` asks for. Empty `types` and `scopes` match every device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeQuery {
    pub types: Vec<QName>,
    pub scopes: Vec<String>,
    pub match_by: MatchBy,
}

impl Default for ProbeQuery {
    fn default() -> Self {
        Self { types: Vec::new(), scopes: Vec::new(), match_by: MatchBy::Rfc3986 }
    }
}

impl ProbeQuery {
    /// Devices announcing the device-level type of this service kind.
    pub fn for_service(kind: ServiceKind) -> Self {
        Self { types: vec![kind.descriptor().device_type_qname()], ..Self::default() }
    }

    /// Every probe type is among the target's types, and every probe scope matches one of its scopes.
    pub fn matches(&self, target: &Target) -> bool {
        self.types.iter().all(|q| target.has_type(q))
            && self.scopes.iter().all(|ps| target.scopes.iter().any(|ts| scope_matches(&self.match_by, ps, ts)))
    }
}

// ---- parsing ----

/// The SOAP header of a discovery message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub action: String,
    pub message_id: Option<String>,
    pub relates_to: Option<String>,
    pub to: Option<String>,
    pub app_sequence: Option<AppSequence>,
}

/// The body of a discovery message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Hello(Target),
    Bye(Target),
    Probe(ProbeQuery),
    Resolve { address: String },
    ProbeMatches(Vec<Target>),
    ResolveMatches(Option<Target>),
}

/// A parsed discovery message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub body: Body,
}

/// Whether `n` is the discovery element `local`, in the WS-Discovery 2005 or 1.1 namespace.
fn is_wsd(n: Node<'_, '_>, local: &str) -> bool {
    xml::is(n, ns::WSD, local) || xml::is(n, ns::WSD_11, local)
}

/// The child elements of `n` named `local` in either discovery namespace.
fn wsd_children<'a, 'i>(n: Node<'a, 'i>, local: &'a str) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children().filter(move |c| is_wsd(*c, local))
}

/// The single child `local` of `n`, if any; a repeated element is a protocol error.
fn only<'a, 'i>(n: Node<'a, 'i>, local: &'a str) -> Result<Option<Node<'a, 'i>>> {
    let mut it = wsd_children(n, local);
    let first = it.next();
    if it.next().is_some() {
        return Err(Error::Protocol(format!("more than one {local} element")));
    }
    Ok(first)
}

/// The tokens of a list element, refusing lists that were cut at the text cap or are too long.
fn tokens(n: Node<'_, '_>, max: usize) -> Result<Vec<String>> {
    let t = xml::text(n);
    if t.chars().count() >= xml::MAX_TEXT {
        return Err(Error::Protocol("a discovery list is too long".into()));
    }
    let out: Vec<String> = t.split_whitespace().map(str::to_owned).collect();
    if out.len() > max || out.iter().any(|s| s.len() > MAX_TOKEN) {
        return Err(Error::Protocol("a discovery list has too many or too long entries".into()));
    }
    Ok(out)
}

/// Read the target a `Hello`, `Bye`, `ProbeMatch` or `ResolveMatch` describes: the EPR address is
/// required, the lists are bounded, and a non-numeric `MetadataVersion` is refused.
fn read_target(n: Node<'_, '_>) -> Result<Target> {
    let epr = n.children().find(|c| xml::is(*c, ns::WSA, "EndpointReference") || xml::is(*c, ns::WSA_FINAL, "EndpointReference"));
    let address = EndpointRef::parse(epr.ok_or_else(|| Error::Protocol("a target has no EndpointReference".into()))?)?.address;
    let types = match only(n, "Types")? {
        Some(t) => {
            // Too many tokens is refused like any other list; undeclared prefixes are skipped.
            if xml::text(t).split_whitespace().count() > MAX_TYPES {
                return Err(Error::Protocol("a discovery list has too many entries".into()));
            }
            xml::qname_list(t, MAX_TYPES)
        }
        None => Vec::new(),
    };
    let scopes = only(n, "Scopes")?.map(|s| tokens(s, MAX_SCOPES)).transpose()?.unwrap_or_default();
    let xaddrs = only(n, "XAddrs")?.map(|s| tokens(s, MAX_XADDRS)).transpose()?.unwrap_or_default();
    let metadata_version = match only(n, "MetadataVersion")? {
        Some(m) => Some(unsigned(&xml::text(m)).ok_or_else(|| Error::Protocol("MetadataVersion is not an unsigned integer".into()))?),
        None => None,
    };
    Ok(Target { address, types, scopes, xaddrs, metadata_version })
}

/// A plain decimal of at most 18 digits (no sign, no spaces), so it always fits a `u64`.
fn unsigned(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 18 || !s.bytes().all(|b| b.is_ascii_digit()) { None } else { s.parse().ok() }
}

/// The header's `AppSequence`, if present. A repeated element or a non-numeric `InstanceId` or
/// `MessageNumber` is a protocol error.
fn read_sequence(header: Node<'_, '_>) -> Result<Option<AppSequence>> {
    let mut it = header.children().filter(|c| is_wsd(*c, "AppSequence"));
    let Some(a) = it.next() else { return Ok(None) };
    if it.next().is_some() {
        return Err(Error::Protocol("more than one AppSequence".into()));
    }
    let get = |name: &str| a.attribute(name).and_then(unsigned);
    match (get("InstanceId"), get("MessageNumber")) {
        (Some(instance_id), Some(message_number)) => {
            Ok(Some(AppSequence { instance_id, sequence_id: a.attribute("SequenceId").map(|s| s.chars().take(MAX_TOKEN).collect()), message_number }))
        }
        _ => Err(Error::Protocol("AppSequence needs numeric InstanceId and MessageNumber".into())),
    }
}

/// Parse one discovery datagram. The body element decides the kind; WS-Discovery 1.1 element
/// namespaces are tolerated.
pub fn parse(bytes: &[u8]) -> Result<Message> {
    if bytes.len() > MAX_DATAGRAM {
        return Err(Error::TooLarge("a discovery datagram", MAX_DATAGRAM));
    }
    let text = xml::checked_text(bytes)?;
    let doc = xml::parse(text)?;
    let env = doc.root_element();
    if !xml::is(env, ns::SOAP, "Envelope") {
        return Err(Error::Protocol("not a SOAP 1.2 envelope".into()));
    }
    let header = xml::child(env, ns::SOAP, "Header").ok_or_else(|| Error::Protocol("the message has no Header".into()))?;
    let body = xml::child(env, ns::SOAP, "Body").ok_or_else(|| Error::Protocol("the message has no Body".into()))?;
    let payload = body.children().find(|c| c.is_element()).ok_or_else(|| Error::Protocol("the Body is empty".into()))?;
    let hdr = Header {
        action: addressing(header, "Action").unwrap_or_default(),
        message_id: addressing(header, "MessageID"),
        relates_to: addressing(header, "RelatesTo"),
        to: addressing(header, "To"),
        app_sequence: read_sequence(header)?,
    };
    let body = if is_wsd(payload, "Hello") {
        Body::Hello(read_target(payload)?)
    } else if is_wsd(payload, "Bye") {
        Body::Bye(read_target(payload)?)
    } else if is_wsd(payload, "Probe") {
        let types = match only(payload, "Types")? {
            Some(t) => {
                if xml::text(t).split_whitespace().count() > MAX_TYPES {
                    return Err(Error::Protocol("a discovery list has too many entries".into()));
                }
                let all = xml::text(t).split_whitespace().count();
                let resolved = xml::qname_list(t, MAX_TYPES);
                if resolved.len() != all {
                    // A probe type we cannot name must not widen the search to "everything".
                    return Err(Error::Protocol("a Probe type uses an undeclared prefix".into()));
                }
                resolved
            }
            None => Vec::new(),
        };
        let (scopes, match_by) = match only(payload, "Scopes")? {
            Some(s) => (tokens(s, MAX_SCOPES)?, MatchBy::from_uri(s.attribute("MatchBy"))),
            None => (Vec::new(), MatchBy::Rfc3986),
        };
        Body::Probe(ProbeQuery { types, scopes, match_by })
    } else if is_wsd(payload, "Resolve") {
        let epr = payload
            .children()
            .find(|c| xml::is(*c, ns::WSA, "EndpointReference") || xml::is(*c, ns::WSA_FINAL, "EndpointReference"))
            .ok_or_else(|| Error::Protocol("Resolve has no EndpointReference".into()))?;
        Body::Resolve { address: EndpointRef::parse(epr)?.address }
    } else if is_wsd(payload, "ProbeMatches") {
        let mut out = Vec::new();
        for m in wsd_children(payload, "ProbeMatch").take(MAX_MATCHES) {
            out.push(read_target(m)?);
        }
        Body::ProbeMatches(out)
    } else if is_wsd(payload, "ResolveMatches") {
        let mut it = wsd_children(payload, "ResolveMatch");
        let first = it.next().map(read_target).transpose()?;
        if it.next().is_some() {
            return Err(Error::Protocol("more than one ResolveMatch".into()));
        }
        Body::ResolveMatches(first)
    } else {
        return Err(Error::Protocol(format!("{} is not a discovery message", payload.tag_name().name())));
    };
    Ok(Message { header: hdr, body })
}

// ---- building ----

/// A complete discovery envelope addressed to `to`, with `body` as the already-serialised body
/// content. `message_id` and `relates_to` are bare UUIDs; the `urn:uuid:` prefix is added here.
fn envelope(to: &str, action: &str, message_id: &str, relates_to: Option<&str>, seq: Option<&AppSequence>, body: &str) -> String {
    let relates = relates_to.map(|r| format!("<wsa:RelatesTo>urn:uuid:{}</wsa:RelatesTo>", xml::esc(r))).unwrap_or_default();
    let seq = seq
        .map(|s| {
            let sid = s.sequence_id.as_ref().map(|i| format!(" SequenceId=\"{}\"", xml::esc(i))).unwrap_or_default();
            format!("<wsd:AppSequence InstanceId=\"{}\" MessageNumber=\"{}\"{sid}/>", s.instance_id, s.message_number)
        })
        .unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{soap}\" xmlns:wsa=\"{wsa}\" xmlns:wsd=\"{wsd}\"><s:Header><wsa:To>{to}</wsa:To><wsa:Action>{action}</wsa:Action><wsa:MessageID>urn:uuid:{id}</wsa:MessageID>{relates}{seq}</s:Header><s:Body>{body}</s:Body></s:Envelope>",
        soap = ns::SOAP,
        wsa = ns::WSA,
        wsd = ns::WSD,
        to = xml::esc(to),
        action = xml::esc(action),
        id = xml::esc(message_id),
    )
}

/// `<wsd:Types>` with each QName's namespace declared on the element itself. Types that cannot be
/// written (empty namespace, unusable local name) are skipped.
fn types_xml(types: &[QName]) -> String {
    let mut decls = String::new();
    let mut names = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for q in types.iter().filter(|q| !q.ns.is_empty() && xml::is_ncname(&q.local)) {
        let i = match seen.iter().position(|n| *n == q.ns) {
            Some(i) => i,
            None => {
                seen.push(&q.ns);
                decls.push_str(&format!(" xmlns:t{}=\"{}\"", seen.len() - 1, xml::esc(&q.ns)));
                seen.len() - 1
            }
        };
        names.push(format!("t{i}:{}", q.local));
    }
    if names.is_empty() { String::new() } else { format!("<wsd:Types{decls}>{}</wsd:Types>", names.join(" ")) }
}

/// The children of a `Hello`, `Bye` or match element describing `t`; `MetadataVersion` is written
/// only when `with_version` is set and the target has one.
fn target_xml(t: &Target, with_version: bool) -> String {
    let scopes = if t.scopes.is_empty() { String::new() } else { format!("<wsd:Scopes>{}</wsd:Scopes>", xml::esc(&t.scopes.join(" "))) };
    let version = match (with_version, t.metadata_version) {
        (true, Some(v)) => format!("<wsd:MetadataVersion>{v}</wsd:MetadataVersion>"),
        _ => String::new(),
    };
    format!(
        "<wsa:EndpointReference><wsa:Address>{}</wsa:Address></wsa:EndpointReference>{}{scopes}<wsd:XAddrs>{}</wsd:XAddrs>{version}",
        xml::esc(&t.address),
        types_xml(&t.types),
        xml::esc(&t.xaddrs.join(" "))
    )
}

/// A `Probe` for multicast to the discovery group.
pub fn probe(message_id: &str, query: &ProbeQuery) -> String {
    let scopes = if query.scopes.is_empty() {
        String::new()
    } else {
        format!("<wsd:Scopes MatchBy=\"{}\">{}</wsd:Scopes>", xml::esc(query.match_by.uri()), xml::esc(&query.scopes.join(" ")))
    };
    let body = format!("<wsd:Probe>{}{scopes}</wsd:Probe>", types_xml(&query.types));
    envelope(ns::DISCOVERY_TO, ACTION_PROBE, message_id, None, None, &body)
}

/// A `Resolve` for one endpoint address.
pub fn resolve(message_id: &str, address: &str) -> String {
    let body = format!("<wsd:Resolve><wsa:EndpointReference><wsa:Address>{}</wsa:Address></wsa:EndpointReference></wsd:Resolve>", xml::esc(address));
    envelope(ns::DISCOVERY_TO, ACTION_RESOLVE, message_id, None, None, &body)
}

/// A `Hello` announcement (device side; used by fakes and tests).
pub fn hello(message_id: &str, seq: &AppSequence, target: &Target) -> String {
    envelope(ns::DISCOVERY_TO, ACTION_HELLO, message_id, None, Some(seq), &format!("<wsd:Hello>{}</wsd:Hello>", target_xml(target, true)))
}

/// A `Bye` announcement (device side). `MetadataVersion` is not part of it.
pub fn bye(message_id: &str, seq: &AppSequence, target: &Target) -> String {
    envelope(ns::DISCOVERY_TO, ACTION_BYE, message_id, None, Some(seq), &format!("<wsd:Bye>{}</wsd:Bye>", target_xml(target, false)))
}

/// A `ProbeMatches` reply (device side) to the probe with id `relates_to`.
pub fn probe_matches(message_id: &str, relates_to: &str, seq: &AppSequence, targets: &[Target]) -> String {
    let inner: String = targets.iter().map(|t| format!("<wsd:ProbeMatch>{}</wsd:ProbeMatch>", target_xml(t, true))).collect();
    envelope(ns::ANONYMOUS, ACTION_PROBE_MATCHES, message_id, Some(relates_to), Some(seq), &format!("<wsd:ProbeMatches>{inner}</wsd:ProbeMatches>"))
}

/// A `ResolveMatches` reply (device side) to the resolve with id `relates_to`.
pub fn resolve_matches(message_id: &str, relates_to: &str, seq: &AppSequence, target: &Target) -> String {
    let body = format!("<wsd:ResolveMatches><wsd:ResolveMatch>{}</wsd:ResolveMatch></wsd:ResolveMatches>", target_xml(target, true));
    envelope(ns::ANONYMOUS, ACTION_RESOLVE_MATCHES, message_id, Some(relates_to), Some(seq), &body)
}

// ---- the socket seam and the driver ----

/// Where a datagram goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// The discovery multicast group. What that means on the wire (group address, port, interface,
    /// hop limit) is the socket implementation's business; this crate has none.
    Multicast,
    /// One host, for directed `Resolve` and for tests.
    Unicast(SocketAddr),
}

impl Destination {
    /// A unicast destination, refused unless the address passes the connection policy.
    pub fn unicast(addr: SocketAddr) -> Result<Self> {
        if addr.port() == 0 || !address_allowed(addr.ip()) {
            return Err(Error::Policy(format!("{addr} is not a loopback, link-local or private address with a port")));
        }
        Ok(Self::Unicast(addr))
    }
}

/// A received datagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datagram {
    pub from: SocketAddr,
    pub payload: Vec<u8>,
}

/// The datagram I/O the driver needs. Implementations own the socket; there is none in this crate.
///
/// Any platform's socket layer (or an in-memory one) can implement it. Failures are reported with
/// [`Error::transport`] / [`Error::transport_at`] so the caller never parses text; an IPv6
/// link-local peer keeps its scope id in the `SocketAddr` of [`Datagram::from`] and
/// [`Destination::Unicast`], which the implementation can use to reply on the right interface.
pub trait DiscoverySocket {
    /// Send one datagram.
    fn send(&self, to: &Destination, payload: &[u8]) -> Result<()>;
    /// The next datagram, or `Ok(None)` when none arrived before `deadline`. An implementation
    /// must not block past the deadline.
    fn recv(&self, deadline: Instant) -> Result<Option<Datagram>>;
}

/// What [`discover`] should do.
#[derive(Clone, Debug)]
pub struct DiscoverOptions {
    pub query: ProbeQuery,
    /// How long to listen in total.
    pub window: Duration,
    /// How many times the probe is sent, evenly spaced over the window (1 to 5).
    pub probe_sends: u32,
    /// Stop collecting after this many devices (1 to 1024).
    pub max_devices: usize,
    /// For a match without any usable address, send a directed `Resolve` to the host it came from.
    pub resolve_missing_xaddrs: bool,
    /// How long to wait for the answers to each of those resolves (more than 0, at most 60 seconds).
    pub resolve_window: Duration,
    /// Also take `Hello` (add) and `Bye` (remove) announcements heard while listening.
    pub accept_announcements: bool,
    pub cancel: Option<CancelToken>,
}

impl Default for DiscoverOptions {
    fn default() -> Self {
        Self {
            query: ProbeQuery::default(),
            window: Duration::from_secs(3),
            probe_sends: 2,
            max_devices: 64,
            resolve_missing_xaddrs: true,
            resolve_window: Duration::from_secs(1),
            accept_announcements: false,
            cancel: None,
        }
    }
}

impl DiscoverOptions {
    /// The default options, probing for devices of this service kind.
    pub fn for_service(kind: ServiceKind) -> Self {
        Self { query: ProbeQuery::for_service(kind), ..Self::default() }
    }
}

/// One device found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub target: Target,
    /// The advertised addresses the connection policy allows.
    pub urls: Vec<DeviceUrl>,
    /// Advertised addresses that were dropped by the policy (public, host names, malformed).
    pub dropped_xaddrs: usize,
    /// Where the answering datagram came from (an IPv6 link-local peer keeps its scope id).
    pub from: SocketAddr,
    pub app_sequence: Option<AppSequence>,
}

/// Why datagrams were not used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ignored {
    /// Not a parseable discovery message.
    pub malformed: usize,
    /// A well-formed message that answers nothing we asked (wrong `RelatesTo`, wrong kind).
    pub unrelated: usize,
    /// Sent from an address the policy refuses.
    pub disallowed_source: usize,
    /// Older than what we already know about that device.
    pub stale: usize,
    /// Did not satisfy the query.
    pub unmatched: usize,
    /// Dropped because the device cap or the per-slice datagram cap was reached.
    pub over_limit: usize,
}

/// The result of [`discover`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub devices: Vec<DiscoveredDevice>,
    pub ignored: Ignored,
}

enum Expecting<'a> {
    Probe(&'a str),
    /// The message id and the endpoint address of the `Resolve` that was sent.
    Resolve(&'a str, &'a str),
}

struct Collector<'a> {
    opts: &'a DiscoverOptions,
    report: DiscoveryReport,
    /// Per endpoint address, the sequence of the last accepted `Bye` (or, if it carried none, of
    /// the device it removed). Kept for the rest of the run so that a replayed or reordered older
    /// message cannot bring the device back.
    byes: Vec<(String, AppSequence)>,
}

impl Collector<'_> {
    /// Record a found device, or update the one with the same address. Older sequences and lower
    /// metadata versions are counted as stale, as is anything replaying a remembered `Bye`; past
    /// `max_devices`, new devices are counted as over the limit.
    fn add(&mut self, target: Target, seq: Option<AppSequence>, from: SocketAddr) {
        let gone = self.byes.iter().find(|(a, _)| a.eq_ignore_ascii_case(&target.address)).map(|(_, s)| s);
        if matches!((&seq, gone), (Some(n), Some(g)) if !n.supersedes(g)) {
            self.report.ignored.stale += 1;
            return;
        }
        let (urls, dropped_xaddrs) = target.device_urls(from.ip());
        if let Some(existing) = self.report.devices.iter_mut().find(|d| d.target.address.eq_ignore_ascii_case(&target.address)) {
            let newer_seq = match (&seq, &existing.app_sequence) {
                (Some(n), Some(o)) => n.supersedes(o),
                _ => true,
            };
            let older_meta = matches!((target.metadata_version, existing.target.metadata_version), (Some(n), Some(o)) if n < o);
            if !newer_seq || older_meta {
                self.report.ignored.stale += 1;
                return;
            }
            // Keep addresses we already had if the newer message carries none, but only within the
            // same instance of the device: after a restart (a new instance id) the old addresses
            // may be stale, and keeping them would also stop the directed Resolve that fills them in.
            let same_instance = matches!((&seq, &existing.app_sequence), (Some(n), Some(o)) if n.instance_id == o.instance_id);
            let keep = same_instance && urls.is_empty() && !existing.urls.is_empty();
            let (u, d, t) = if keep {
                (existing.urls.clone(), existing.dropped_xaddrs, Target { xaddrs: existing.target.xaddrs.clone(), ..target })
            } else {
                (urls, dropped_xaddrs, target)
            };
            *existing = DiscoveredDevice { target: t, urls: u, dropped_xaddrs: d, from, app_sequence: seq };
            return;
        }
        if self.report.devices.len() >= self.opts.max_devices {
            self.report.ignored.over_limit += 1;
            return;
        }
        self.report.devices.push(DiscoveredDevice { target, urls, dropped_xaddrs, from, app_sequence: seq });
    }

    /// Sort one datagram: matches correlated with `expecting` (and announcements, if accepted) are
    /// added or removed; everything else only increments a counter in [`Ignored`].
    fn handle(&mut self, d: Datagram, expecting: &Expecting<'_>) {
        if !address_allowed(d.from.ip()) {
            self.report.ignored.disallowed_source += 1;
            return;
        }
        let Ok(msg) = parse(&d.payload) else {
            self.report.ignored.malformed += 1;
            return;
        };
        let relates = msg.header.relates_to.as_deref().map(|r| r.strip_prefix("urn:uuid:").unwrap_or(r));
        let correlated = |want: &str| relates.is_some_and(|r| r.eq_ignore_ascii_case(want));
        match (msg.body, expecting) {
            (Body::ProbeMatches(targets), Expecting::Probe(id)) if correlated(id) => {
                for t in targets {
                    if self.opts.query.matches(&t) {
                        self.add(t, msg.header.app_sequence.clone(), d.from);
                    } else {
                        self.report.ignored.unmatched += 1;
                    }
                }
            }
            (Body::ResolveMatches(Some(t)), Expecting::Resolve(id, address)) if correlated(id) && t.address.eq_ignore_ascii_case(address) => {
                self.add(t, msg.header.app_sequence, d.from);
            }
            (Body::Hello(t), _) if self.opts.accept_announcements => {
                if self.opts.query.matches(&t) {
                    self.add(t, msg.header.app_sequence, d.from);
                } else {
                    self.report.ignored.unmatched += 1;
                }
            }
            (Body::Bye(t), _) if self.opts.accept_announcements => {
                let seq = msg.header.app_sequence;
                let mut stale = false;
                let mut removed = None;
                self.report.devices.retain(|dev| {
                    if !dev.target.address.eq_ignore_ascii_case(&t.address) {
                        return true;
                    }
                    // A `Bye` older than what is known (a reordered datagram from before a restart)
                    // must not remove the live device.
                    let newer = match (&seq, &dev.app_sequence) {
                        (Some(n), Some(o)) => n.supersedes(o),
                        _ => true,
                    };
                    stale |= !newer;
                    if newer {
                        removed.clone_from(&dev.app_sequence);
                    }
                    !newer
                });
                if stale {
                    self.report.ignored.stale += 1;
                } else if let Some(gone) = seq.or(removed) {
                    self.remember_bye(&t.address, gone);
                }
            }
            _ => self.report.ignored.unrelated += 1,
        }
    }

    /// Remember the sequence of a `Bye` for `address` so an older message cannot revive the device;
    /// at most `MAX_BYES` addresses are kept.
    fn remember_bye(&mut self, address: &str, seq: AppSequence) {
        if let Some(known) = self.byes.iter_mut().find(|(a, _)| a.eq_ignore_ascii_case(address)) {
            if seq.supersedes(&known.1) {
                known.1 = seq;
            } else {
                self.report.ignored.stale += 1;
            }
        } else if self.byes.len() < MAX_BYES {
            self.byes.push((address.to_owned(), seq));
        }
    }
}

/// [`Error::Cancelled`] once the options' cancel token is set.
fn check_cancel(opts: &DiscoverOptions) -> Result<()> {
    if opts.cancel.as_ref().is_some_and(CancelToken::is_cancelled) { Err(Error::Cancelled) } else { Ok(()) }
}

/// Receive and handle datagrams until `until`, the socket reports none, or the run is cancelled.
/// Datagrams past `MAX_DATAGRAMS_PER_SLICE` are counted as over the limit, not parsed.
fn listen<S: DiscoverySocket + ?Sized>(socket: &S, until: Instant, c: &mut Collector<'_>, expecting: &Expecting<'_>) -> Result<()> {
    let mut seen = 0usize;
    loop {
        check_cancel(c.opts)?;
        if Instant::now() >= until {
            return Ok(());
        }
        let Some(d) = socket.recv(until)? else { return Ok(()) };
        seen += 1;
        if seen > MAX_DATAGRAMS_PER_SLICE {
            c.report.ignored.over_limit += 1;
            continue;
        }
        c.handle(d, expecting);
    }
}

/// Probe for devices and collect the answers. Nothing is sent except through `socket`.
///
/// The probe is sent `probe_sends` times over `window`; matches (and, if enabled, announcements)
/// are collected, then devices that came back without a usable address are asked directly with a
/// `Resolve`. The result lists each device once, with its policy-checked addresses.
pub fn discover<S: DiscoverySocket + ?Sized>(socket: &S, opts: &DiscoverOptions) -> Result<DiscoveryReport> {
    if opts.window.is_zero() || opts.window > Duration::from_secs(60) {
        return Err(Error::Invalid("discovery window: use more than 0 and at most 60 seconds".into()));
    }
    if !(1..=5).contains(&opts.probe_sends) || !(1..=1024).contains(&opts.max_devices) {
        return Err(Error::Invalid("discovery: probe_sends is 1 to 5 and max_devices 1 to 1024".into()));
    }
    if opts.resolve_missing_xaddrs && (opts.resolve_window.is_zero() || opts.resolve_window > Duration::from_secs(60)) {
        return Err(Error::Invalid("discovery resolve window: use more than 0 and at most 60 seconds".into()));
    }
    check_cancel(opts)?;
    let id = new_uuid();
    let bytes = probe(&id, &opts.query);
    if bytes.len() > MAX_DATAGRAM {
        return Err(Error::TooLarge("the probe", MAX_DATAGRAM));
    }
    let mut c = Collector { opts, report: DiscoveryReport::default(), byes: Vec::new() };
    let start = Instant::now();
    for i in 0..opts.probe_sends {
        check_cancel(opts)?;
        socket.send(&Destination::Multicast, bytes.as_bytes())?;
        let until = start + opts.window * (i + 1) / opts.probe_sends;
        listen(socket, until, &mut c, &Expecting::Probe(&id))?;
    }
    if opts.resolve_missing_xaddrs {
        let pending: Vec<(String, SocketAddr)> = c
            .report
            .devices
            .iter()
            .filter(|d| d.urls.is_empty())
            .map(|d| {
                // Same host and (link-local) scope id, at the discovery port.
                let mut to = d.from;
                to.set_port(DISCOVERY_PORT);
                (d.target.address.clone(), to)
            })
            .take(MAX_RESOLVES)
            .collect();
        for (address, to) in pending {
            check_cancel(opts)?;
            let rid = new_uuid();
            let Ok(dest) = Destination::unicast(to) else { continue };
            // A host that cannot be reached is skipped: it must not cost the devices already found.
            if socket.send(&dest, resolve(&rid, &address).as_bytes()).is_err() {
                continue;
            }
            let until = Instant::now().checked_add(opts.resolve_window).ok_or_else(|| Error::Invalid("discovery resolve window".into()))?;
            listen(socket, until, &mut c, &Expecting::Resolve(&rid, &address))?;
        }
    }
    Ok(c.report)
}

#[cfg(any(test, feature = "testing"))]
pub mod fake {
    //! An in-memory [`DiscoverySocket`] and a device-side responder, for tests of everything that
    //! sits on discovery. No real socket is ever opened.

    use std::collections::VecDeque;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::{
        AppSequence, Body, Datagram, Destination, DiscoverySocket, Instant, Result, SocketAddr, Target, new_uuid, parse, probe_matches,
        resolve_matches,
    };

    /// Lock `m`, taking the guard back from a poisoned mutex: one panicking responder must not
    /// make every later use of the socket panic too.
    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A device that answers probes and resolves (and nothing else) from memory.
    pub struct FakeResponder {
        pub target: Target,
        pub from: SocketAddr,
        pub instance_id: u64,
        message_number: Mutex<u64>,
    }

    impl FakeResponder {
        /// A responder announcing `target` from `from`, instance 1, counting messages from 1.
        pub fn new(target: Target, from: SocketAddr) -> Self {
            Self { target, from, instance_id: 1, message_number: Mutex::new(0) }
        }

        /// The `AppSequence` for this device's next message; numbers increase by one per call.
        fn next_seq(&self) -> AppSequence {
            let mut n = lock(&self.message_number);
            *n += 1;
            AppSequence::new(self.instance_id, *n)
        }

        /// The datagrams this device sends in answer to `request`.
        pub fn answer(&self, request: &[u8]) -> Vec<Datagram> {
            let Ok(msg) = parse(request) else { return Vec::new() };
            let Some(id) = msg.header.message_id.as_deref().map(|m| m.strip_prefix("urn:uuid:").unwrap_or(m).to_owned()) else { return Vec::new() };
            let payload = match msg.body {
                Body::Probe(q) if q.matches(&self.target) => probe_matches(&new_uuid(), &id, &self.next_seq(), std::slice::from_ref(&self.target)),
                Body::Resolve { address } if address.eq_ignore_ascii_case(&self.target.address) => {
                    resolve_matches(&new_uuid(), &id, &self.next_seq(), &self.target)
                }
                _ => return Vec::new(),
            };
            vec![Datagram { from: self.from, payload: payload.into_bytes() }]
        }
    }

    type Responder = Box<dyn Fn(&Destination, &[u8]) -> Vec<Datagram> + Send + Sync>;

    /// A socket whose "network" is a queue: datagrams you `push` are received in order, and
    /// responders (devices) react to what is sent.
    #[derive(Default)]
    pub struct FakeSocket {
        inbox: Mutex<VecDeque<Datagram>>,
        sent: Mutex<Vec<(Destination, Vec<u8>)>>,
        responders: Mutex<Vec<Responder>>,
    }

    impl FakeSocket {
        /// An empty socket: nothing queued, nothing sent, no responders.
        pub fn new() -> Self {
            Self::default()
        }

        /// Queue a datagram for the next `recv`.
        pub fn push(&self, d: Datagram) {
            lock(&self.inbox).push_back(d);
        }

        /// React to everything sent through this socket with the datagrams `responder` returns.
        pub fn add_responder(&self, responder: impl Fn(&Destination, &[u8]) -> Vec<Datagram> + Send + Sync + 'static) {
            lock(&self.responders).push(Box::new(responder));
        }

        /// Let `device` answer every probe and resolve sent through this socket.
        pub fn add_device(&self, device: FakeResponder) {
            lock(&self.responders).push(Box::new(move |_, payload| device.answer(payload)));
        }

        /// Everything sent so far.
        pub fn sent(&self) -> Vec<(Destination, Vec<u8>)> {
            lock(&self.sent).clone()
        }
    }

    impl DiscoverySocket for FakeSocket {
        /// Record the datagram, then queue whatever every responder answers to it. Never fails.
        fn send(&self, to: &Destination, payload: &[u8]) -> Result<()> {
            lock(&self.sent).push((to.clone(), payload.to_vec()));
            let replies: Vec<Datagram> = lock(&self.responders).iter().flat_map(|r| r(to, payload)).collect();
            lock(&self.inbox).extend(replies);
            Ok(())
        }

        /// The next queued datagram, or `None` at once when the queue is empty; the deadline is ignored.
        fn recv(&self, _deadline: Instant) -> Result<Option<Datagram>> {
            Ok(lock(&self.inbox).pop_front())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeResponder, FakeSocket};
    use super::*;

    #[test]
    fn a_responder_that_panicked_does_not_make_the_fake_socket_panic_afterwards() {
        let socket = FakeSocket::new();
        socket.add_responder(|_, _| panic!("a responder failed"));
        let to = Destination::unicast("192.168.1.9:3702".parse().unwrap()).unwrap();
        // The panic happens while the responder list is locked, which poisons it.
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| socket.send(&to, b"x"))).is_err());
        // Later use of the socket still works.
        socket.add_responder(|_, _| Vec::new());
        assert_eq!(socket.sent().len(), 1);
        assert!(matches!(socket.recv(Instant::now()), Ok(None)));
    }

    #[test]
    fn a_link_local_scope_id_survives_destination_and_datagram() {
        use std::net::{Ipv6Addr, SocketAddrV6};
        let addr = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1), 3702, 0, 7));
        match Destination::unicast(addr) {
            Ok(Destination::Unicast(SocketAddr::V6(v6))) => assert_eq!(v6.scope_id(), 7),
            other => panic!("{other:?}"),
        }
        let d = Datagram { from: addr, payload: Vec::new() };
        assert!(matches!(d.from, SocketAddr::V6(v6) if v6.scope_id() == 7));
        // A public IPv6 address is still refused, scope id or not.
        let public = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 3702, 0, 7));
        assert!(matches!(Destination::unicast(public), Err(Error::Policy(_))));
    }

    /// The device-level type print devices announce.
    fn print_dev() -> QName {
        ServiceKind::Print.descriptor().device_type_qname()
    }

    /// A target `urn:uuid:{uuid}` of type `ty` with one scope, metadata version 7 and these `xaddrs`.
    fn target(uuid: &str, ty: QName, xaddrs: &[&str]) -> Target {
        Target {
            address: format!("urn:uuid:{uuid}"),
            types: vec![QName::new(ns::WSDP, "Device"), ty],
            scopes: vec!["onvif://www.example.org/place/lab".into()],
            xaddrs: xaddrs.iter().map(|s| (*s).to_owned()).collect(),
            metadata_version: Some(7),
        }
    }

    /// The discovery address `10.0.0.{last}:3702`.
    fn peer(last: u8) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, last], 3702))
    }

    #[test]
    fn every_message_kind_round_trips_through_builder_and_parser() {
        let t = target("aaaa", print_dev(), &["http://10.0.0.5:5357/dev", "http://10.0.0.5:5357/dev2"]);
        let seq = AppSequence { instance_id: 9, sequence_id: Some("urn:uuid:seq".into()), message_number: 3 };
        let hello = parse(hello("m1", &seq, &t).as_bytes()).unwrap();
        assert_eq!(
            (hello.header.action.as_str(), hello.header.message_id.as_deref(), hello.header.to.as_deref()),
            (ACTION_HELLO, Some("urn:uuid:m1"), Some(ns::DISCOVERY_TO))
        );
        assert_eq!(hello.header.app_sequence, Some(seq.clone()));
        assert_eq!(hello.body, Body::Hello(t.clone()));
        let bye_msg = parse(bye("m2", &seq, &t).as_bytes()).unwrap();
        assert_eq!(bye_msg.body, Body::Bye(Target { metadata_version: None, ..t.clone() }), "Bye carries no MetadataVersion");
        let q = ProbeQuery { types: vec![print_dev(), QName::new("urn:other", "T")], scopes: vec!["a://b/c".into()], match_by: MatchBy::Strcmp0 };
        let p = parse(probe("m3", &q).as_bytes()).unwrap();
        assert_eq!(p.body, Body::Probe(q));
        assert_eq!(parse(probe("m3", &ProbeQuery::default()).as_bytes()).unwrap().body, Body::Probe(ProbeQuery::default()));
        let r = parse(resolve("m4", "urn:uuid:aaaa").as_bytes()).unwrap();
        assert_eq!(r.body, Body::Resolve { address: "urn:uuid:aaaa".into() });
        let pm = parse(probe_matches("m5", "m3", &seq, &[t.clone(), t.clone()]).as_bytes()).unwrap();
        assert_eq!(pm.header.relates_to.as_deref(), Some("urn:uuid:m3"));
        assert_eq!(pm.body, Body::ProbeMatches(vec![t.clone(), t.clone()]));
        let rm = parse(resolve_matches("m6", "m4", &seq, &t).as_bytes()).unwrap();
        assert_eq!(rm.body, Body::ResolveMatches(Some(t)));
        assert_eq!(parse(probe_matches("m5", "m3", &seq, &[]).as_bytes()).unwrap().body, Body::ProbeMatches(vec![]));
    }

    #[test]
    fn builders_escape_and_stay_datagram_sized() {
        let t = Target { address: "urn:uuid:<&>".into(), xaddrs: vec!["http://10.0.0.1/a?b=<c>".into()], ..Default::default() };
        assert!(parse(hello("m", &AppSequence::new(1, 1), &t).as_bytes()).is_ok());
        let p = probe("m", &ProbeQuery::for_service(ServiceKind::Scan));
        assert!(p.len() < 1400, "{} bytes", p.len());
        // Types that cannot be written are skipped, not emitted malformed.
        let q = ProbeQuery { types: vec![QName::new("", "NoNs"), QName::new("urn:x", "bad name"), QName::new("urn:x", "Ok")], ..Default::default() };
        let Body::Probe(back) = parse(probe("m", &q).as_bytes()).unwrap().body else { panic!("not a probe") };
        assert_eq!(back.types, vec![QName::new("urn:x", "Ok")]);
    }

    #[test]
    fn scope_matching_follows_the_rules() {
        let m = |rule: &MatchBy, p: &str, t: &str| scope_matches(rule, p, t);
        let r = MatchBy::Rfc3986;
        assert!(m(&r, "http://Example.org/a", "HTTP://example.ORG/a/b/c"));
        assert!(m(&r, "http://example.org/a/", "http://example.org/a"));
        assert!(!m(&r, "http://example.org/a", "http://example.org/ab"), "a prefix of the text is not a prefix of the path");
        assert!(!m(&r, "http://example.org/a/..", "http://example.org/a"));
        assert!(!m(&r, "http://example.org/a", "http://example.org/a/../b"));
        assert!(!m(&r, "http://example.org/a/b", "http://example.org/a"));
        assert!(m(&MatchBy::Strcmp0, "X", "X") && !m(&MatchBy::Strcmp0, "x", "X"));
        assert!(m(&MatchBy::Uuid, "urn:uuid:ABC", "urn:uuid:abc"));
        assert!(!m(&MatchBy::None, "x", "x") && !m(&MatchBy::Unknown("u".into()), "x", "x"));
        assert_eq!(MatchBy::from_uri(Some("urn:made-up")), MatchBy::Unknown("urn:made-up".into()));
        for rule in [MatchBy::Rfc3986, MatchBy::Uuid, MatchBy::Ldap, MatchBy::Strcmp0, MatchBy::None] {
            assert_eq!(MatchBy::from_uri(Some(rule.uri())), rule);
        }
    }

    #[test]
    fn probes_match_by_all_types_and_all_scopes() {
        let t = target("a", print_dev(), &[]);
        assert!(ProbeQuery::default().matches(&t), "an empty probe matches everything");
        assert!(ProbeQuery::for_service(ServiceKind::Print).matches(&t));
        assert!(!ProbeQuery::for_service(ServiceKind::Scan).matches(&t));
        let both = ProbeQuery { types: vec![QName::new(ns::WSDP, "Device"), print_dev()], ..Default::default() };
        assert!(both.matches(&t));
        let missing = ProbeQuery { types: vec![QName::new(ns::WSDP, "Device"), QName::new("urn:x", "Absent")], ..Default::default() };
        assert!(!missing.matches(&t));
        let scoped = ProbeQuery { scopes: vec!["onvif://www.example.org/place".into()], ..Default::default() };
        assert!(scoped.matches(&t));
        assert!(!ProbeQuery { scopes: vec!["onvif://www.example.org/other".into()], ..Default::default() }.matches(&t));
        assert!(!scoped.matches(&Target { scopes: vec![], ..t }));
    }

    #[test]
    fn xaddrs_are_filtered_through_the_device_url_policy() {
        let t = target(
            "a",
            print_dev(),
            &[
                "http://10.0.0.5/a",
                "http://8.8.8.8/b",
                "http://printer.local/c",
                "https://10.0.0.5/d",
                "http://10.0.0.5/a",
                "http://[fe80::1%25eth0]/e",
                "http://[fe80::2]:80/f",
                "garbage",
            ],
        );
        let (urls, dropped) = t.device_urls(peer(5).ip());
        assert_eq!(urls.iter().map(DeviceUrl::as_url).collect::<Vec<_>>(), ["http://10.0.0.5:80/a", "http://[fe80::2]:80/f"]);
        assert_eq!(dropped, 5);
    }

    #[test]
    fn loopback_addresses_are_kept_only_from_a_loopback_source() {
        let t = target("a", print_dev(), &["http://127.0.0.1:631/", "http://[::1]:631/", "http://10.0.0.5/p"]);
        for from in ["10.0.0.9", "fe80::9"] {
            let (urls, dropped) = t.device_urls(from.parse().unwrap());
            assert_eq!(urls.iter().map(DeviceUrl::as_url).collect::<Vec<_>>(), ["http://10.0.0.5:80/p"], "from {from}");
            assert_eq!(dropped, 2, "from {from}");
        }
        for from in ["127.0.0.1", "::1", "::ffff:127.0.0.1"] {
            let (urls, dropped) = t.device_urls(from.parse().unwrap());
            assert_eq!(urls.len(), 3, "from {from}");
            assert_eq!(dropped, 0, "from {from}");
        }
    }

    #[test]
    fn an_announcement_from_the_network_cannot_aim_the_client_at_this_machine() {
        let t = target("a", print_dev(), &["http://127.0.0.1:631/", "http://[::1]:631/", "http://10.0.0.5/p"]);
        let opts = DiscoverOptions { probe_sends: 1, accept_announcements: true, ..Default::default() };
        let run = |from: SocketAddr| {
            let socket = FakeSocket::new();
            socket.push(Datagram { from, payload: hello("m1", &AppSequence::new(1, 1), &t).into_bytes() });
            discover(&socket, &opts).unwrap().devices.remove(0)
        };
        let remote = run(peer(5));
        assert_eq!(remote.urls.iter().map(DeviceUrl::as_url).collect::<Vec<_>>(), ["http://10.0.0.5:80/p"]);
        assert_eq!(remote.dropped_xaddrs, 2);
        let local = run("127.0.0.1:3702".parse().unwrap());
        assert_eq!((local.urls.len(), local.dropped_xaddrs), (3, 0));
    }

    #[test]
    fn app_sequences_order_by_instance_then_number() {
        let s = |i, n| AppSequence::new(i, n);
        assert!(s(1, 2).supersedes(&s(1, 1)));
        assert!(!s(1, 1).supersedes(&s(1, 1)));
        assert!(!s(1, 5).supersedes(&s(2, 1)));
        assert!(s(3, 1).supersedes(&s(2, 99)));
        let other = AppSequence { sequence_id: Some("x".into()), ..s(1, 1) };
        assert!(other.supersedes(&s(9, 9)), "different sequence ids are incomparable and accepted");
    }

    #[test]
    #[allow(clippy::cognitive_complexity, reason = "a table of hostile inputs, each one line")]
    fn hostile_datagrams_are_errors_not_panics() {
        let good = hello("m", &AppSequence::new(1, 1), &target("a", print_dev(), &["http://10.0.0.1/"]));
        assert!(parse(good.as_bytes()).is_ok());
        assert!(matches!(parse(&vec![b' '; MAX_DATAGRAM + 1]), Err(Error::TooLarge(..))));
        assert!(matches!(parse(b"\xff\xfe"), Err(Error::Xml(_))));
        assert!(matches!(parse(b"<x/>"), Err(Error::Protocol(_))));
        let dup = |what: &str| good.replace("</wsd:Hello>", &format!("{what}</wsd:Hello>"));
        for extra in [
            "<wsd:Types>wsdp:Device</wsd:Types>",
            "<wsd:XAddrs>http://10.0.0.9/</wsd:XAddrs>",
            "<wsd:Scopes>a</wsd:Scopes><wsd:Scopes>b</wsd:Scopes>",
            "<wsd:MetadataVersion>1</wsd:MetadataVersion>",
        ] {
            assert!(matches!(parse(dup(extra).as_bytes()), Err(Error::Protocol(_))), "{extra}");
        }
        let seq2 = good.replace("</s:Header>", "<wsd:AppSequence InstanceId=\"1\" MessageNumber=\"2\"/></s:Header>");
        assert!(matches!(parse(seq2.as_bytes()), Err(Error::Protocol(_))));
        for bad_seq in [
            "InstanceId=\"x\" MessageNumber=\"1\"",
            "MessageNumber=\"1\"",
            "InstanceId=\"-1\" MessageNumber=\"1\"",
            "InstanceId=\"99999999999999999999\" MessageNumber=\"1\"",
        ] {
            let m = good.replace("InstanceId=\"1\" MessageNumber=\"1\"", bad_seq);
            assert!(matches!(parse(m.as_bytes()), Err(Error::Protocol(_))), "{bad_seq}");
        }
        assert!(matches!(
            parse(good.replace("<wsd:MetadataVersion>7</wsd:MetadataVersion>", "<wsd:MetadataVersion>seven</wsd:MetadataVersion>").as_bytes()),
            Err(Error::Protocol(_))
        ));
        // Too many list entries, an over-long list, and an unknown body.
        let many = (0..MAX_XADDRS + 1).map(|i| format!("http://10.0.0.1/{i}")).collect::<Vec<_>>().join(" ");
        assert!(matches!(parse(good.replace("http://10.0.0.1/", &many).as_bytes()), Err(Error::Protocol(_))));
        let long = format!("http://10.0.0.1/{}", "a".repeat(5000));
        assert!(matches!(parse(good.replace("http://10.0.0.1/", &long).as_bytes()), Err(Error::Protocol(_))));
        assert!(matches!(parse(good.replace("wsd:Hello", "wsd:Frobnicate").as_bytes()), Err(Error::Protocol(_))));
        // A target without an endpoint reference; a probe type with an undeclared prefix.
        assert!(matches!(parse(good.replace("EndpointReference", "Other").as_bytes()), Err(Error::Protocol(_))));
        let bad_probe = probe("m", &ProbeQuery { types: vec![QName::new("urn:x", "T")], ..Default::default() }).replace("xmlns:t0=\"urn:x\"", "");
        assert!(matches!(parse(bad_probe.as_bytes()), Err(Error::Protocol(_))));
        // The OASIS 1.1 namespace reads too.
        let v11 = good.replace(ns::WSD, ns::WSD_11);
        assert!(matches!(parse(v11.as_bytes()).unwrap().body, Body::Hello(_)));
        // Bounded depth.
        let deep = good.replace("<wsd:Hello>", &format!("<wsd:Hello>{}{}", "<a>".repeat(60), "</a>".repeat(60)));
        assert!(matches!(parse(deep.as_bytes()), Err(Error::Xml(_))));
    }

    #[test]
    fn destinations_are_policy_checked() {
        assert!(Destination::unicast(peer(5)).is_ok());
        assert!(Destination::unicast("127.0.0.1:3702".parse().unwrap()).is_ok());
        assert!(Destination::unicast("8.8.8.8:3702".parse().unwrap()).is_err());
        assert!(Destination::unicast("10.0.0.5:0".parse().unwrap()).is_err());
        assert!(Destination::unicast("[ff02::c]:3702".parse().unwrap()).is_err(), "multicast is never a unicast destination");
    }

    /// A [`FakeSocket`] that has every responder in `devices` attached.
    fn socket_with(devices: Vec<FakeResponder>) -> FakeSocket {
        let s = FakeSocket::new();
        for d in devices {
            s.add_device(d);
        }
        s
    }

    #[test]
    fn discover_finds_matching_devices_once_and_filters_addresses() {
        let print = FakeResponder::new(target("p1", print_dev(), &["http://10.0.0.5:5357/dev", "http://8.8.8.8/evil"]), peer(5));
        let scan = FakeResponder::new(target("s1", ServiceKind::Scan.descriptor().device_type_qname(), &["http://10.0.0.6:3920/dev"]), peer(6));
        let socket = socket_with(vec![print, scan]);
        let report = discover(&socket, &DiscoverOptions::for_service(ServiceKind::Print)).unwrap();
        assert_eq!(report.devices.len(), 1, "{report:?}");
        let d = &report.devices[0];
        assert_eq!(d.target.address, "urn:uuid:p1");
        assert_eq!(d.urls.iter().map(DeviceUrl::as_url).collect::<Vec<_>>(), ["http://10.0.0.5:5357/dev"]);
        assert_eq!(d.dropped_xaddrs, 1);
        assert_eq!(d.from, peer(5));
        // Two probe sends answered twice: still one device, and the repeat is not "stale" noise only.
        let sent = socket.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|(dest, _)| *dest == Destination::Multicast));
        // Everything (no type filter) finds both.
        assert_eq!(
            discover(
                &socket_with(vec![
                    FakeResponder::new(target("a", print_dev(), &["http://10.0.0.5/x"]), peer(5)),
                    FakeResponder::new(target("b", print_dev(), &["http://10.0.0.6/x"]), peer(6))
                ]),
                &DiscoverOptions::default()
            )
            .unwrap()
            .devices
            .len(),
            2
        );
    }

    #[test]
    fn discover_ignores_noise_strangers_and_uncorrelated_replies() {
        let socket = FakeSocket::new();
        let id_unknown = "00000000-0000-4000-8000-000000000000";
        let t = target("x", print_dev(), &["http://10.0.0.9/x"]);
        let seq = AppSequence::new(1, 1);
        let push = |from: SocketAddr, payload: Vec<u8>| socket.push(Datagram { from, payload });
        push(peer(9), b"garbage".to_vec());
        push(peer(9), probe_matches("m", id_unknown, &seq, std::slice::from_ref(&t)).into_bytes());
        push("8.8.8.8:3702".parse().unwrap(), probe_matches("m", id_unknown, &seq, std::slice::from_ref(&t)).into_bytes());
        push(peer(9), hello("m", &seq, &t).into_bytes());
        let report = discover(&socket, &DiscoverOptions { probe_sends: 1, ..Default::default() }).unwrap();
        assert!(report.devices.is_empty());
        assert_eq!(report.ignored, Ignored { malformed: 1, unrelated: 2, disallowed_source: 1, ..Default::default() });
    }

    #[test]
    fn announcements_are_taken_only_on_request_and_bye_removes() {
        let t = target("h", print_dev(), &["http://10.0.0.7/x"]);
        let socket = FakeSocket::new();
        socket.push(Datagram { from: peer(7), payload: hello("m1", &AppSequence::new(1, 1), &t).into_bytes() });
        socket.push(Datagram {
            from: peer(8),
            payload: hello("m2", &AppSequence::new(1, 1), &target("o", ServiceKind::Scan.descriptor().device_type_qname(), &[])).into_bytes(),
        });
        let opts =
            DiscoverOptions { probe_sends: 1, accept_announcements: true, query: ProbeQuery::for_service(ServiceKind::Print), ..Default::default() };
        let r = discover(&socket, &opts).unwrap();
        assert_eq!(r.devices.len(), 1);
        assert_eq!(r.ignored.unmatched, 1);
        socket.push(Datagram { from: peer(7), payload: hello("m3", &AppSequence::new(1, 2), &t).into_bytes() });
        socket.push(Datagram { from: peer(7), payload: bye("m4", &AppSequence::new(1, 3), &t).into_bytes() });
        assert!(discover(&socket, &opts).unwrap().devices.is_empty(), "the Bye removes the device");
        let off = FakeSocket::new();
        off.push(Datagram { from: peer(7), payload: hello("m1", &AppSequence::new(1, 1), &t).into_bytes() });
        assert!(discover(&off, &DiscoverOptions { probe_sends: 1, ..Default::default() }).unwrap().devices.is_empty());
    }

    #[test]
    fn stale_sequences_and_metadata_versions_do_not_replace_newer_knowledge() {
        let socket = FakeSocket::new();
        let new = Target { metadata_version: Some(10), ..target("d", print_dev(), &["http://10.0.0.7/new"]) };
        let old = Target { metadata_version: Some(5), ..target("d", print_dev(), &["http://10.0.0.7/old"]) };
        let opts = DiscoverOptions { probe_sends: 1, accept_announcements: true, ..Default::default() };
        for (m, seq, t) in [
            ("a", AppSequence::new(2, 5), &new),
            ("b", AppSequence::new(2, 4), &old),
            ("c", AppSequence::new(3, 1), &old),
            ("d", AppSequence::new(3, 2), &new),
        ] {
            socket.push(Datagram { from: peer(7), payload: hello(m, &seq, t).into_bytes() });
        }
        let r = discover(&socket, &opts).unwrap();
        assert_eq!(r.devices.len(), 1);
        assert_eq!(r.devices[0].urls[0].path(), "/new");
        assert_eq!(r.ignored.stale, 2, "message number going back, and metadata version going back");
    }

    #[test]
    fn a_match_without_addresses_is_resolved_directly_not_by_multicast() {
        let socket = socket_with(vec![FakeResponder::new(target("r", print_dev(), &[]), peer(5))]);
        let r = discover(&socket, &DiscoverOptions { probe_sends: 1, ..Default::default() }).unwrap();
        assert_eq!(r.devices.len(), 1);
        assert!(r.devices[0].urls.is_empty(), "nothing to resolve to: the fake has no addresses to give");
        let sent = socket.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].0, Destination::Unicast(peer(5)));
        assert!(matches!(parse(&sent[1].1).unwrap().body, Body::Resolve { address } if address == "urn:uuid:r"));
        // A device that lists no address in its probe match but gives one when resolved.
        let bare = FakeResponder::new(target("r", print_dev(), &[]), peer(5));
        let mut full = FakeResponder::new(target("r", print_dev(), &["http://10.0.0.5:80/found"]), peer(5));
        full.instance_id = 2; // a later announcement than the probe match

        let staged = FakeSocket::new();
        staged.add_responder(move |_, payload| match parse(payload).map(|m| m.body) {
            Ok(Body::Probe(_)) => bare.answer(payload),
            Ok(Body::Resolve { .. }) => full.answer(payload),
            _ => Vec::new(),
        });
        let r2 = discover(&staged, &DiscoverOptions { probe_sends: 1, resolve_window: Duration::from_millis(10), ..Default::default() }).unwrap();
        assert_eq!(r2.devices.len(), 1, "{r2:?}");
        assert_eq!(r2.devices[0].urls.iter().map(DeviceUrl::as_url).collect::<Vec<_>>(), ["http://10.0.0.5:80/found"]);
    }

    #[test]
    fn a_resolve_match_for_another_address_is_not_accepted() {
        let bare = FakeResponder::new(target("r", print_dev(), &[]), peer(5));
        let evil = target("evil", print_dev(), &["http://10.0.0.99:80/x"]);
        let staged = FakeSocket::new();
        staged.add_responder(move |_, payload| match parse(payload) {
            Ok(m) => match m.body {
                Body::Probe(_) => bare.answer(payload),
                Body::Resolve { .. } => {
                    let id = m.header.message_id.as_deref().map(|m| m.strip_prefix("urn:uuid:").unwrap_or(m).to_owned()).unwrap();
                    let p = resolve_matches(&new_uuid(), &id, &AppSequence::new(1, 1), &evil);
                    vec![Datagram { from: peer(5), payload: p.into_bytes() }]
                }
                _ => Vec::new(),
            },
            Err(_) => Vec::new(),
        });
        let opts = DiscoverOptions { probe_sends: 1, resolve_window: Duration::from_millis(10), ..Default::default() };
        let r = discover(&staged, &opts).unwrap();
        assert_eq!(r.devices.len(), 1, "no second device from the foreign match: {r:?}");
        assert!(r.devices[0].urls.is_empty());
        assert_eq!(r.ignored.unrelated, 1);
    }

    #[test]
    fn a_bye_older_than_the_known_announcement_does_not_remove_the_device() {
        let t = target("h", print_dev(), &["http://10.0.0.7/x"]);
        let socket = FakeSocket::new();
        // The restarted device's Hello (instance 2) arrives before the old instance's Bye.
        socket.push(Datagram { from: peer(7), payload: hello("m1", &AppSequence::new(2, 1), &t).into_bytes() });
        socket.push(Datagram { from: peer(7), payload: bye("m2", &AppSequence::new(1, 9), &t).into_bytes() });
        let opts = DiscoverOptions { probe_sends: 1, accept_announcements: true, ..Default::default() };
        let r = discover(&socket, &opts).unwrap();
        assert_eq!(r.devices.len(), 1, "{r:?}");
        assert_eq!(r.ignored.stale, 1);
    }

    #[test]
    fn addresses_are_kept_only_within_the_same_instance_of_the_device() {
        let with_url = target("h", print_dev(), &["http://10.0.0.7/x"]);
        let without = target("h", print_dev(), &[]);
        let opts = DiscoverOptions { probe_sends: 1, accept_announcements: true, ..Default::default() };
        let run = |first: &AppSequence, second: &AppSequence| {
            let socket = FakeSocket::new();
            socket.push(Datagram { from: peer(7), payload: hello("m1", first, &with_url).into_bytes() });
            socket.push(Datagram { from: peer(7), payload: hello("m2", second, &without).into_bytes() });
            discover(&socket, &opts).unwrap()
        };
        // A later message of the same instance without addresses keeps the ones already known.
        let same = run(&AppSequence::new(1, 1), &AppSequence::new(1, 2));
        assert_eq!(same.devices.len(), 1, "{same:?}");
        assert!(!same.devices[0].urls.is_empty(), "same instance: the known addresses stay");
        // After a restart (a new instance id) they may be stale: they go.
        let restarted = run(&AppSequence::new(1, 1), &AppSequence::new(2, 1));
        assert_eq!(restarted.devices.len(), 1, "{restarted:?}");
        assert!(restarted.devices[0].urls.is_empty(), "a new instance must not keep the old instance's addresses");
    }

    #[test]
    fn limits_options_and_cancellation_are_enforced() {
        let socket = FakeSocket::new();
        let bad = |o: DiscoverOptions| matches!(discover(&socket, &o), Err(Error::Invalid(_)));
        assert!(bad(DiscoverOptions { window: Duration::ZERO, ..Default::default() }));
        assert!(bad(DiscoverOptions { window: Duration::from_secs(61), ..Default::default() }));
        assert!(bad(DiscoverOptions { probe_sends: 0, ..Default::default() }));
        assert!(bad(DiscoverOptions { probe_sends: 6, ..Default::default() }));
        assert!(bad(DiscoverOptions { max_devices: 0, ..Default::default() }));
        assert!(bad(DiscoverOptions { resolve_window: Duration::ZERO, ..Default::default() }));
        assert!(bad(DiscoverOptions { resolve_window: Duration::MAX, ..Default::default() }));
        assert!(socket.sent().is_empty(), "invalid options send nothing");
        // The resolve window only matters when resolving is on.
        let off = DiscoverOptions { probe_sends: 1, resolve_missing_xaddrs: false, resolve_window: Duration::MAX, ..Default::default() };
        assert!(discover(&FakeSocket::new(), &off).is_ok());
        let token = CancelToken::new();
        token.cancel();
        assert_eq!(discover(&socket, &DiscoverOptions { cancel: Some(token), ..Default::default() }), Err(Error::Cancelled));
        assert!(socket.sent().is_empty());
        // The device cap holds.
        let many = FakeSocket::new();
        for i in 0..10u8 {
            many.add_device(FakeResponder::new(target(&format!("d{i}"), print_dev(), &["http://10.0.0.1/x"]), peer(i + 10)));
        }
        let r = discover(&many, &DiscoverOptions { probe_sends: 1, max_devices: 3, ..Default::default() }).unwrap();
        assert_eq!((r.devices.len(), r.ignored.over_limit), (3, 7));
    }

    /// A [`FakeSocket`] whose unicast sends to one address fail.
    struct FailingUnicast {
        inner: FakeSocket,
        fail_to: SocketAddr,
    }

    impl DiscoverySocket for FailingUnicast {
        /// Fail unicast sends to `fail_to` as unreachable; pass everything else to the inner socket.
        fn send(&self, to: &Destination, payload: &[u8]) -> Result<()> {
            if *to == Destination::Unicast(self.fail_to) {
                return Err(Error::transport(crate::error::TransportKind::Unreachable, "no route"));
            }
            self.inner.send(to, payload)
        }

        fn recv(&self, deadline: Instant) -> Result<Option<Datagram>> {
            self.inner.recv(deadline)
        }
    }

    #[test]
    fn a_failed_resolve_send_skips_that_device_and_keeps_the_others() {
        let socket = FailingUnicast {
            inner: socket_with(vec![
                FakeResponder::new(target("a", print_dev(), &[]), peer(5)),
                FakeResponder::new(target("b", print_dev(), &[]), peer(6)),
                FakeResponder::new(target("c", print_dev(), &["http://10.0.0.7/x"]), peer(7)),
            ]),
            fail_to: peer(5),
        };
        let opts = DiscoverOptions { probe_sends: 1, resolve_window: Duration::from_millis(10), ..Default::default() };
        let r = discover(&socket, &opts).unwrap();
        assert_eq!(r.devices.len(), 3, "{r:?}");
        let sent = socket.inner.sent();
        assert!(sent.iter().any(|(dest, _)| *dest == Destination::Unicast(peer(6))), "the resolve after the failed one is still sent");
    }

    #[test]
    fn a_replayed_hello_after_a_bye_does_not_bring_the_device_back() {
        let t = target("h", print_dev(), &["http://10.0.0.7/x"]);
        let opts = DiscoverOptions { probe_sends: 1, accept_announcements: true, ..Default::default() };
        let run = |last: &AppSequence| {
            let socket = FakeSocket::new();
            socket.push(Datagram { from: peer(7), payload: hello("m1", &AppSequence::new(1, 1), &t).into_bytes() });
            socket.push(Datagram { from: peer(7), payload: bye("m2", &AppSequence::new(1, 2), &t).into_bytes() });
            socket.push(Datagram { from: peer(7), payload: hello("m3", last, &t).into_bytes() });
            discover(&socket, &opts).unwrap()
        };
        let replayed = run(&AppSequence::new(1, 1));
        assert!(replayed.devices.is_empty(), "{replayed:?}");
        assert_eq!(replayed.ignored.stale, 1);
        assert_eq!(run(&AppSequence::new(2, 1)).devices.len(), 1, "a new instance comes back");
        assert_eq!(run(&AppSequence::new(1, 3)).devices.len(), 1, "a later message of the same instance comes back");
    }

    #[test]
    fn an_older_bye_does_not_replace_a_newer_remembered_bye() {
        let t = target("h", print_dev(), &["http://10.0.0.7/x"]);
        let opts = DiscoverOptions { probe_sends: 1, accept_announcements: true, ..Default::default() };
        // Hello(1,1), then two Byes in the given order, then a final Hello.
        let run = |byes: [u64; 2], last: &AppSequence| {
            let socket = FakeSocket::new();
            socket.push(Datagram { from: peer(7), payload: hello("m1", &AppSequence::new(1, 1), &t).into_bytes() });
            for (i, n) in byes.into_iter().enumerate() {
                socket.push(Datagram { from: peer(7), payload: bye(&format!("b{i}"), &AppSequence::new(1, n), &t).into_bytes() });
            }
            socket.push(Datagram { from: peer(7), payload: hello("m9", last, &t).into_bytes() });
            discover(&socket, &opts).unwrap()
        };
        // A reordered older Bye (1,2) after Bye (1,5) must not lower the remembered sequence, so
        // the reordered Hello (1,3) cannot bring the device back.
        let reordered = run([5, 2], &AppSequence::new(1, 3));
        assert!(reordered.devices.is_empty(), "{reordered:?}");
        assert_eq!(reordered.ignored.stale, 2, "the older Bye and the older Hello");
        // In order, the newer Bye replaces the older one: Hello (1,3) is stale, Hello (1,6) is not.
        let in_order = run([2, 5], &AppSequence::new(1, 3));
        assert!(in_order.devices.is_empty(), "{in_order:?}");
        assert_eq!(run([2, 5], &AppSequence::new(1, 6)).devices.len(), 1, "a Hello after the newest Bye comes back");
        assert_eq!(run([5, 2], &AppSequence::new(1, 6)).devices.len(), 1);
        // A new instance id resets the order.
        assert_eq!(run([5, 2], &AppSequence::new(2, 1)).devices.len(), 1, "a new instance comes back");
    }

    #[test]
    fn a_link_local_responder_is_resolved_on_its_own_interface() {
        use std::net::{Ipv6Addr, SocketAddrV6};
        let from = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 5), 50_000, 0, 7));
        let socket = socket_with(vec![FakeResponder::new(target("ll", print_dev(), &[]), from)]);
        let r = discover(&socket, &DiscoverOptions { probe_sends: 1, resolve_window: Duration::from_millis(10), ..Default::default() }).unwrap();
        assert_eq!(r.devices.len(), 1, "{r:?}");
        assert_eq!(r.devices[0].from, from);
        let sent = socket.sent();
        assert_eq!(sent.len(), 2);
        let want = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 5), DISCOVERY_PORT, 0, 7));
        assert_eq!(sent[1].0, Destination::Unicast(want));
    }

    #[test]
    fn a_datagram_flood_is_cut_off() {
        let socket = FakeSocket::new();
        for _ in 0..MAX_DATAGRAMS_PER_SLICE + 50 {
            socket.push(Datagram { from: peer(9), payload: b"x".to_vec() });
        }
        let r = discover(&socket, &DiscoverOptions { probe_sends: 1, ..Default::default() }).unwrap();
        assert_eq!(r.ignored.malformed, MAX_DATAGRAMS_PER_SLICE);
        assert_eq!(r.ignored.over_limit, 50);
    }
}
