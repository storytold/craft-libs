//! WS-Eventing, the client side: subscribe a sink to a service's events, keep the subscription
//! alive, and read what the device pushes back.
//!
//! This is the 2004/08 submission as Windows devices speak it (`wse:Expires`, the subscription
//! manager's `wse:Identifier` as a reference parameter that every follow-up message carries as a
//! header). There is no listening server here: a service crate or application owns the HTTP
//! listener and hands each received POST body to [`dispatch`] with a [`NotificationSink`].
//!
//! * Requests: [`SubscribeRequest`] (push delivery to a `NotifyTo` sink, optional `EndTo`,
//!   [`Expiry`], [`Filter`], and service extensions such as a scan destination list), [`renew`],
//!   [`get_status`], [`unsubscribe`]. The `NotifyTo` and `EndTo` addresses must pass the same
//!   private-network policy as device URLs: we never ask a device to call out to the internet.
//! * Replies: [`Subscription`] (manager reference, identifier, granted expiry).
//! * Incoming: [`Incoming`] is either a [`Notification`] (action, headers, the body kept for a
//!   service-specific reader) or a [`SubscriptionEnd`].

use std::sync::Mutex;
use std::time::Duration;

use roxmltree::Node;

use crate::addressing::{EndpointRef, ReferenceParameter};
use crate::envelope::addressing;
use crate::error::{Error, Result};
use crate::fault::parse_fault;
use crate::soap::{Call, SoapClient};
use crate::transport::Transport;
use crate::xml::{self, QName};
use crate::{SubCode, ns};

pub const ACTION_SUBSCRIBE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/Subscribe";
pub const ACTION_SUBSCRIBE_RESPONSE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/SubscribeResponse";
pub const ACTION_RENEW: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/Renew";
pub const ACTION_RENEW_RESPONSE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/RenewResponse";
pub const ACTION_GET_STATUS: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/GetStatus";
pub const ACTION_GET_STATUS_RESPONSE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/GetStatusResponse";
pub const ACTION_UNSUBSCRIBE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/Unsubscribe";
pub const ACTION_UNSUBSCRIBE_RESPONSE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/UnsubscribeResponse";
pub const ACTION_SUBSCRIPTION_END: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/SubscriptionEnd";
/// `Delivery/@Mode` for push delivery, the only mode used.
pub const DELIVERY_MODE_PUSH: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing/DeliveryModes/Push";
/// The DPWS filter dialect: whitespace-separated action URIs.
pub const DIALECT_ACTION: &str = "http://schemas.xmlsoap.org/ws/2006/02/devprof/Action";

const MAX_FILTER_ACTIONS: usize = 32;
const MAX_ACTION: usize = 512;
const MAX_EXTENSION: usize = 16 * 1024;
const MAX_HEADERS: usize = 16;

// ---- time ----

/// A subscription lifetime as the protocol writes it: a duration (`PT1H`) or an absolute UTC time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// Relative to now.
    After(Duration),
    /// Seconds since the Unix epoch (UTC).
    At(i64),
}

impl Expiry {
    /// Read an `xs:duration` or `xs:dateTime`. Years count 365 days and months 30; negative
    /// durations, out-of-range dates and anything else malformed are `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim();
        if t.starts_with('P') { parse_duration(t).map(Self::After) } else { parse_datetime(t).map(Self::At) }
    }

    /// The lexical form to put on the wire.
    pub fn to_text(&self) -> String {
        match self {
            Self::After(d) => format_duration(*d),
            Self::At(secs) => format_datetime(*secs),
        }
    }
}

/// Seconds in a non-negative `xs:duration` (`P1DT2H`, `PT0.5S`); years count 365 days, months 30.
/// Components out of order, fractions anywhere but seconds, and totals of 1e10 seconds or more are `None`.
fn parse_duration(t: &str) -> Option<Duration> {
    let rest = t.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    if date.is_empty() && time.is_none_or(str::is_empty) {
        return None;
    }
    let mut secs: f64 = 0.0;
    let mut part = |s: &str, units: &[(char, f64)]| -> Option<()> {
        let mut num = String::new();
        let mut last = 0usize;
        for c in s.chars() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
            } else {
                let idx = units.iter().position(|(u, _)| *u == c)?;
                if idx < last || num.is_empty() || num.len() > 12 {
                    return None;
                }
                last = idx + 1;
                let v: f64 = num.parse().ok()?;
                // Fractions are only legal on the last (seconds) component.
                if num.contains('.') && c != 'S' {
                    return None;
                }
                secs += v * units.get(idx)?.1;
                num.clear();
            }
        }
        num.is_empty().then_some(())
    };
    part(date, &[('Y', 365.0 * 86400.0), ('M', 30.0 * 86400.0), ('D', 86400.0)])?;
    if let Some(t) = time {
        if t.is_empty() {
            return None;
        }
        part(t, &[('H', 3600.0), ('M', 60.0), ('S', 1.0)])?;
    }
    (secs.is_finite() && secs < 1e10).then(|| Duration::from_secs_f64(secs))
}

/// `d` as `PT{secs}S`, with milliseconds as a three-digit fraction when there are any.
fn format_duration(d: Duration) -> String {
    let ms = d.subsec_millis();
    if ms == 0 { format!("PT{}S", d.as_secs()) } else { format!("PT{}.{ms:03}S", d.as_secs()) }
}

/// Days from 1970-01-01 for a proleptic Gregorian date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The `(year, month, day)` of a day count from 1970-01-01; the inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Seconds since the Unix epoch of an `xs:dateTime` with a `Z`, `+hh:mm`/`-hh:mm` or no zone (taken
/// as UTC). Fractional seconds are accepted and dropped; invalid dates and times are `None`.
fn parse_datetime(t: &str) -> Option<i64> {
    let (date, time) = t.split_once('T')?;
    let mut d = date.split('-');
    let (y, mo, da) = (d.next()?, d.next()?, d.next()?);
    if d.next().is_some() || y.len() != 4 || mo.len() != 2 || da.len() != 2 {
        return None;
    }
    let (y, mo, da): (i64, i64, i64) = (y.parse().ok()?, mo.parse().ok()?, da.parse().ok()?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + i64::from(leap),
        _ => return None,
    };
    if da < 1 || da > dim || y < 1 {
        return None;
    }
    // Split off the zone: `Z`, `+hh:mm`, `-hh:mm` or nothing (UTC).
    let (clock, offset) = if let Some(c) = time.strip_suffix('Z') {
        (c, 0)
    } else if let Some(i) = time.rfind(['+', '-']).filter(|i| *i >= 8) {
        let (c, z) = time.split_at(i);
        let sign = if z.starts_with('-') { -1 } else { 1 };
        let (h, m) = z.get(1..)?.split_once(':')?;
        if h.len() != 2 || m.len() != 2 {
            return None;
        }
        let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
        if h > 14 || m > 59 {
            return None;
        }
        (c, sign * (h * 3600 + m * 60))
    } else {
        (time, 0)
    };
    let mut c = clock.split(':');
    let (h, mi, s) = (c.next()?, c.next()?, c.next()?);
    if c.next().is_some() || h.len() != 2 || mi.len() != 2 {
        return None;
    }
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if whole.len() != 2 || (s.contains('.') && frac.is_empty()) || !frac.bytes().all(|b| b.is_ascii_digit()) || frac.len() > 9 {
        return None;
    }
    let (h, mi, s): (i64, i64, i64) = (h.parse().ok()?, mi.parse().ok()?, whole.parse().ok()?);
    if h > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some(days_from_civil(y, mo, da) * 86_400 + h * 3600 + mi * 60 + s - offset)
}

/// `secs` since the Unix epoch as `YYYY-MM-DDThh:mm:ssZ`.
fn format_datetime(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

// ---- requests ----

/// A `wse:Filter`: a dialect URI and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub dialect: String,
    pub text: String,
}

impl Filter {
    /// The DPWS action filter: events whose action URI is in `actions`.
    pub fn actions(actions: &[&str]) -> Result<Self> {
        if actions.is_empty() || actions.len() > MAX_FILTER_ACTIONS {
            return Err(Error::Invalid(format!("filter: give 1 to {MAX_FILTER_ACTIONS} actions")));
        }
        if let Some(bad) = actions.iter().find(|a| a.len() > MAX_ACTION || !a.contains(':') || a.chars().any(|c| c.is_whitespace() || c.is_control()))
        {
            return Err(Error::Invalid(format!("filter action {bad:?}: not an absolute URI")));
        }
        Ok(Self { dialect: DIALECT_ACTION.to_owned(), text: actions.join(" ") })
    }
}

/// What to put in a `Subscribe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscribeRequest {
    /// Where the device pushes events. Must be an address the connection policy allows.
    pub notify_to: EndpointRef,
    /// Where the device reports the end of the subscription.
    pub end_to: Option<EndpointRef>,
    pub expires: Option<Expiry>,
    pub filter: Option<Filter>,
    /// Service-specific child elements of `Subscribe` (a scan destination list, for example). Each
    /// must be one well-formed element that declares its own namespaces; they are checked, not trusted.
    pub extensions: Vec<String>,
}

impl SubscribeRequest {
    /// A request delivering to `notify_to`, with no `EndTo`, expiry, filter or extensions.
    pub fn new(notify_to: EndpointRef) -> Self {
        Self { notify_to, end_to: None, expires: None, filter: None, extensions: Vec::new() }
    }

    /// The `wse:Subscribe` element. Refuses sink addresses outside the connection policy, a filter
    /// dialect or extension that is not well-formed, and oversized extensions.
    pub fn to_body(&self) -> Result<String> {
        self.notify_to.url().map_err(|e| Error::Invalid(format!("NotifyTo: {e}")))?;
        let mut out = String::from("<wse:Subscribe>");
        if let Some(end) = &self.end_to {
            end.url().map_err(|e| Error::Invalid(format!("EndTo: {e}")))?;
            out.push_str(&end.to_xml("wse:EndTo"));
        }
        out.push_str(&format!("<wse:Delivery Mode=\"{DELIVERY_MODE_PUSH}\">{}</wse:Delivery>", self.notify_to.to_xml("wse:NotifyTo")));
        if let Some(e) = &self.expires {
            out.push_str(&format!("<wse:Expires>{}</wse:Expires>", e.to_text()));
        }
        if let Some(f) = &self.filter {
            out.push_str(&format!("<wse:Filter Dialect=\"{}\">{}</wse:Filter>", xml::esc(&f.dialect), xml::esc(&f.text)));
        }
        for ext in &self.extensions {
            if ext.len() > MAX_EXTENSION {
                return Err(Error::Invalid("subscribe extension: too large".into()));
            }
            xml::parse(ext).map_err(|e| Error::Invalid(format!("subscribe extension: {e}")))?;
            out.push_str(ext);
        }
        out.push_str("</wse:Subscribe>");
        Ok(out)
    }

    /// The `Subscribe` call: its actions, expected reply element and body. Fails as [`Self::to_body`] does.
    fn call(&self) -> Result<Call<'static>> {
        Ok(Call::new(ACTION_SUBSCRIBE, ACTION_SUBSCRIBE_RESPONSE)
            .reply_element(ns::WSE, "SubscribeResponse")
            .namespace("wse", ns::WSE)
            .body(self.to_body()?))
    }
}

/// A live subscription: where to manage it and when it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub manager: EndpointRef,
    pub expires: Option<Expiry>,
}

impl Subscription {
    /// The `wse:Identifier` reference parameter of the manager reference, if the device gave one.
    pub fn identifier(&self) -> Option<&str> {
        self.manager.parameter(ns::WSE, "Identifier")
    }
}

/// Read a `wse:SubscribeResponse`.
pub fn parse_subscribe_response(node: Node<'_, '_>) -> Result<Subscription> {
    let mut managers = xml::children(node, ns::WSE, "SubscriptionManager");
    let manager = managers.next().ok_or_else(|| Error::Protocol("SubscribeResponse has no SubscriptionManager".into()))?;
    if managers.next().is_some() {
        return Err(Error::Protocol("SubscribeResponse has more than one SubscriptionManager".into()));
    }
    Ok(Subscription { manager: EndpointRef::parse(manager)?, expires: read_expires(node)? })
}

/// The `wse:Expires` child of a response, if any. A second one, or one that is neither a duration
/// nor a date-time, is a protocol error.
fn read_expires(node: Node<'_, '_>) -> Result<Option<Expiry>> {
    let mut all = xml::children(node, ns::WSE, "Expires");
    let Some(first) = all.next() else { return Ok(None) };
    if all.next().is_some() {
        return Err(Error::Protocol("more than one Expires".into()));
    }
    Expiry::parse(&xml::text(first))
        .map(Some)
        .ok_or_else(|| Error::Protocol(format!("Expires {:?} is neither a duration nor a date-time", xml::text(first))))
}

/// Subscribe on the client's endpoint (the event source: a service address).
pub fn subscribe<T: Transport>(client: &SoapClient<T>, request: &SubscribeRequest) -> Result<Subscription> {
    subscribe_with(client, request, |_| Ok(())).map(|(s, ())| s)
}

/// Subscribe and let `extra` read service-specific parts of the response (a scan destination token
/// list, for example) from the `SubscribeResponse` element.
pub fn subscribe_with<T: Transport, X>(
    client: &SoapClient<T>,
    request: &SubscribeRequest,
    extra: impl FnOnce(Node<'_, '_>) -> Result<X>,
) -> Result<(Subscription, X)> {
    client.call(&request.call()?, |node| Ok((parse_subscribe_response(node)?, extra(node)?)))
}

/// A manager-addressed call: reference parameters become headers, and the manager's address (which
/// must pass the connection policy) is the target. The subscribe reply came from the client's own
/// endpoint, so a manager at a loopback address is refused unless that endpoint is loopback too.
fn manager_call<'a, T: Transport>(client: &'a SoapClient<T>, sub: &Subscription) -> Result<SoapClient<&'a T>> {
    Ok(client.at(sub.manager.url_from(client.endpoint().host())?))
}

/// Renew; the granted expiry if the device states one.
pub fn renew<T: Transport>(client: &SoapClient<T>, sub: &Subscription, expires: Option<Expiry>) -> Result<Option<Expiry>> {
    let body = match expires {
        Some(e) => format!("<wse:Renew><wse:Expires>{}</wse:Expires></wse:Renew>", e.to_text()),
        None => "<wse:Renew/>".to_owned(),
    };
    let call = Call::new(ACTION_RENEW, ACTION_RENEW_RESPONSE)
        .reply_element(ns::WSE, "RenewResponse")
        .namespace("wse", ns::WSE)
        .headers(sub.manager.header_xml())
        .body(body);
    manager_call(client, sub)?.call(&call, read_expires)
}

/// GetStatus; the remaining lifetime if the device states one.
pub fn get_status<T: Transport>(client: &SoapClient<T>, sub: &Subscription) -> Result<Option<Expiry>> {
    let call = Call::new(ACTION_GET_STATUS, ACTION_GET_STATUS_RESPONSE)
        .reply_element(ns::WSE, "GetStatusResponse")
        .namespace("wse", ns::WSE)
        .headers(sub.manager.header_xml())
        .body("<wse:GetStatus/>");
    manager_call(client, sub)?.call(&call, read_expires)
}

/// Unsubscribe.
pub fn unsubscribe<T: Transport>(client: &SoapClient<T>, sub: &Subscription) -> Result<()> {
    let call = Call::new(ACTION_UNSUBSCRIBE, ACTION_UNSUBSCRIBE_RESPONSE)
        .namespace("wse", ns::WSE)
        .headers(sub.manager.header_xml())
        .body("<wse:Unsubscribe/>");
    manager_call(client, sub)?.call_ack(&call)
}

/// The WS-Eventing fault sub-codes, for services that want to react to a specific refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventingFault {
    DeliveryModeRequestedUnavailable,
    InvalidExpirationTime,
    UnsupportedExpirationType,
    FilteringNotSupported,
    FilteringRequestedUnavailable,
    EventSourceUnableToProcess,
    UnableToRenew,
    InvalidMessage,
}

impl EventingFault {
    /// Classify a fault sub-code; `None` for anything outside the WS-Eventing namespace.
    pub fn from_subcode(code: &SubCode) -> Option<Self> {
        if code.ns.as_deref() != Some(ns::WSE) {
            return None;
        }
        Some(match code.local.as_str() {
            "DeliveryModeRequestedUnavailable" => Self::DeliveryModeRequestedUnavailable,
            "InvalidExpirationTime" => Self::InvalidExpirationTime,
            "UnsupportedExpirationType" => Self::UnsupportedExpirationType,
            "FilteringNotSupported" => Self::FilteringNotSupported,
            "FilteringRequestedUnavailable" => Self::FilteringRequestedUnavailable,
            "EventSourceUnableToProcess" => Self::EventSourceUnableToProcess,
            "UnableToRenew" => Self::UnableToRenew,
            "InvalidMessage" => Self::InvalidMessage,
            _ => return None,
        })
    }
}

// ---- incoming messages ----

/// Why a subscription ended, from a `SubscriptionEnd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndStatus {
    DeliveryFailure,
    SourceShuttingDown,
    SourceCancelling,
    Other(String),
}

/// A `wse:SubscriptionEnd` message sent to the `EndTo` sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriptionEnd {
    pub manager: EndpointRef,
    pub status: EndStatus,
    pub reason: String,
}

impl SubscriptionEnd {
    /// The `wse:Identifier` the subscription manager's reference carries, if any.
    pub fn identifier(&self) -> Option<&str> {
        self.manager.parameter(ns::WSE, "Identifier")
    }
}

/// An event notification: the generic envelope, with the payload kept for the service to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub action: String,
    pub message_id: Option<String>,
    pub to: Option<String>,
    /// Header blocks other than SOAP and WS-Addressing ones: the `NotifyTo` reference parameters
    /// (such as a subscription identifier) the device echoes. Plain-text blocks only, capped.
    pub headers: Vec<ReferenceParameter>,
    /// The name of the payload's first element.
    pub body: QName,
    raw: Vec<u8>,
}

impl Notification {
    /// The first header block `{ns}local`, as text.
    pub fn header(&self, ns: &str, local: &str) -> Option<&str> {
        self.headers.iter().find(|h| h.ns == ns && h.local == local).map(|h| h.text.as_str())
    }

    /// Read the payload element (re-parsed from the bounded message) with a service-specific reader.
    pub fn with_body<R, E: From<Error>>(&self, read: impl FnOnce(Node<'_, '_>) -> std::result::Result<R, E>) -> std::result::Result<R, E> {
        let text = xml::checked_text(&self.raw)?;
        let doc = xml::parse(text)?;
        let body = xml::child(doc.root_element(), ns::SOAP, "Body").ok_or_else(|| Error::Protocol("the envelope has no Body".into()))?;
        let payload = body.children().find(|c| c.is_element()).ok_or_else(|| Error::Protocol("the Body is empty".into()))?;
        read(payload)
    }
}

/// Something a device pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    Event(Notification),
    SubscriptionEnd(SubscriptionEnd),
}

/// Parse a POSTed notification envelope. A SOAP fault is an [`Error::Fault`]; a message without an
/// action or body is a protocol error.
pub fn parse_incoming(bytes: &[u8]) -> Result<Incoming> {
    let text = xml::checked_text(bytes)?;
    let doc = xml::parse(text)?;
    let envelope = doc.root_element();
    if !xml::is(envelope, ns::SOAP, "Envelope") {
        return Err(Error::Protocol("not a SOAP 1.2 envelope".into()));
    }
    let body = xml::child(envelope, ns::SOAP, "Body").ok_or_else(|| Error::Protocol("the envelope has no Body".into()))?;
    let payload = body.children().find(|c| c.is_element()).ok_or_else(|| Error::Protocol("the Body is empty".into()))?;
    if xml::is(payload, ns::SOAP, "Fault") {
        return Err(Error::from_fault(parse_fault(payload)));
    }
    let header = xml::child(envelope, ns::SOAP, "Header").ok_or_else(|| Error::Protocol("the message has no Header".into()))?;
    let action = addressing(header, "Action").filter(|a| !a.is_empty()).ok_or_else(|| Error::Protocol("the message has no Action".into()))?;
    if xml::is(payload, ns::WSE, "SubscriptionEnd") {
        return read_end(payload).map(Incoming::SubscriptionEnd);
    }
    let mut headers = Vec::new();
    for h in header.children().filter(|c| c.is_element()) {
        let Some(hns) = h.tag_name().namespace() else { continue };
        if [ns::SOAP, ns::WSA, ns::WSA_FINAL].contains(&hns) || h.children().any(|c| c.is_element()) || headers.len() >= MAX_HEADERS {
            continue;
        }
        headers.push(ReferenceParameter::new(hns, h.tag_name().name(), xml::text(h)));
    }
    let body_name = QName::new(payload.tag_name().namespace().unwrap_or(""), payload.tag_name().name());
    Ok(Incoming::Event(Notification {
        action,
        message_id: addressing(header, "MessageID"),
        to: addressing(header, "To"),
        headers,
        body: body_name,
        raw: bytes.to_vec(),
    }))
}

/// Read a `wse:SubscriptionEnd` body. The manager and `Status` are required; a status that is not one
/// of the three WS-Eventing values is kept as [`EndStatus::Other`].
fn read_end(node: Node<'_, '_>) -> Result<SubscriptionEnd> {
    let manager =
        xml::child(node, ns::WSE, "SubscriptionManager").ok_or_else(|| Error::Protocol("SubscriptionEnd has no SubscriptionManager".into()))?;
    let status = xml::child_text(node, ns::WSE, "Status").ok_or_else(|| Error::Protocol("SubscriptionEnd has no Status".into()))?;
    // Only the exact `{WSE}/<Name>` URI is a standard status; look-alikes stay `Other`.
    let status = match status.strip_prefix(ns::WSE).and_then(|rest| rest.strip_prefix('/')) {
        Some("DeliveryFailure") => EndStatus::DeliveryFailure,
        Some("SourceShuttingDown") => EndStatus::SourceShuttingDown,
        Some("SourceCancelling") => EndStatus::SourceCancelling,
        _ => EndStatus::Other(status),
    };
    Ok(SubscriptionEnd { manager: EndpointRef::parse(manager)?, status, reason: xml::child_text(node, ns::WSE, "Reason").unwrap_or_default() })
}

/// Where received notifications go. The application's HTTP listener calls [`dispatch`] with each
/// POST body; the service crate's sink turns payloads into its own events. `&self`, so a sink can
/// be shared with the listener thread (use interior mutability).
pub trait NotificationSink {
    /// An event arrived. An `Err` tells the listener to answer with a failure status.
    fn notification(&self, notification: &Notification) -> Result<()>;
    /// The device ended a subscription. The default ignores it.
    fn subscription_end(&self, _end: &SubscriptionEnd) -> Result<()> {
        Ok(())
    }
}

/// Parse `bytes` and hand the result to `sink`.
pub fn dispatch(bytes: &[u8], sink: &dyn NotificationSink) -> Result<()> {
    match parse_incoming(bytes)? {
        Incoming::Event(n) => sink.notification(&n),
        Incoming::SubscriptionEnd(e) => sink.subscription_end(&e),
    }
}

/// A sink that keeps everything it is given (tests, simple tools).
#[derive(Debug, Default)]
pub struct CollectingSink {
    items: Mutex<Vec<Incoming>>,
}

impl CollectingSink {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything received so far, in order.
    pub fn items(&self) -> Vec<Incoming> {
        self.items.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Keep `i`; dropped if the lock is poisoned.
    fn push(&self, i: Incoming) {
        if let Ok(mut g) = self.items.lock() {
            g.push(i);
        }
    }
}

impl NotificationSink for CollectingSink {
    /// Keep a clone of the event; never refuses.
    fn notification(&self, n: &Notification) -> Result<()> {
        self.push(Incoming::Event(n.clone()));
        Ok(())
    }

    /// Keep a clone of the end notice; never refuses.
    fn subscription_end(&self, e: &SubscriptionEnd) -> Result<()> {
        self.push(Incoming::SubscriptionEnd(e.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_and_format() {
        for (text, secs) in [
            ("PT1H", 3600.0),
            ("PT10M", 600.0),
            ("P1D", 86_400.0),
            ("P1DT2H3M4S", 93_784.0),
            ("PT0.5S", 0.5),
            ("PT30S", 30.0),
            ("P1M", 2_592_000.0),
            ("P1Y", 31_536_000.0),
        ] {
            assert_eq!(Expiry::parse(text), Some(Expiry::After(Duration::from_secs_f64(secs))), "{text}");
        }
        for bad in ["", "P", "PT", "1H", "PT1", "PT-1S", "-PT1S", "P1.5D", "PT1H1H", "PTS", "PT1S1M", "P1DT", "PT99999999999999S", "PXS", "P1D2"] {
            assert_eq!(Expiry::parse(bad), None, "{bad}");
        }
        assert_eq!(Expiry::After(Duration::from_secs(3600)).to_text(), "PT3600S");
        assert_eq!(Expiry::After(Duration::from_millis(1500)).to_text(), "PT1.500S");
        let round = Expiry::parse(&Expiry::After(Duration::from_secs(90)).to_text());
        assert_eq!(round, Some(Expiry::After(Duration::from_secs(90))));
    }

    #[test]
    fn date_times_parse_with_zones_and_round_trip() {
        assert_eq!(Expiry::parse("1970-01-01T00:00:00Z"), Some(Expiry::At(0)));
        assert_eq!(Expiry::parse("2000-03-01T00:00:00Z"), Some(Expiry::At(951_868_800)));
        assert_eq!(Expiry::parse("2000-03-01T02:00:00+02:00"), Some(Expiry::At(951_868_800)));
        assert_eq!(Expiry::parse("2000-02-29T22:00:00-02:00"), Some(Expiry::At(951_868_800)));
        assert_eq!(Expiry::parse("2000-03-01T00:00:00.250Z"), Some(Expiry::At(951_868_800)));
        assert_eq!(Expiry::parse("2000-03-01T00:00:00"), Some(Expiry::At(951_868_800)), "no zone means UTC");
        for secs in [0, 951_868_800, 4_102_444_800, -86_400, 1_700_000_000] {
            assert_eq!(Expiry::parse(&Expiry::At(secs).to_text()), Some(Expiry::At(secs)), "{secs}");
        }
        for bad in [
            "2001-02-29T00:00:00Z",
            "2000-13-01T00:00:00Z",
            "2000-00-10T00:00:00Z",
            "2000-01-32T00:00:00Z",
            "2000-01-01T24:00:00Z",
            "2000-01-01T00:60:00Z",
            "2000-01-01T00:00:60Z",
            "2000-01-01",
            "2000-01-01T00:00Z",
            "2000-01-01T00:00:00+15:00",
            "2000-01-01T00:00:00+1:00",
            "0000-01-01T00:00:00Z",
            "20000101T000000Z",
            "yesterday",
            "2000-01-01T00:00:00.Z",
        ] {
            assert_eq!(Expiry::parse(bad), None, "{bad}");
        }
    }

    /// A subscribe request to a loopback sink carrying one reference parameter.
    fn req() -> SubscribeRequest {
        SubscribeRequest::new(EndpointRef::new("http://127.0.0.1:9000/sink").with_parameter(ReferenceParameter::new("urn:x", "Ctx", "c1")))
    }

    /// `body` inside a root element that declares `wse` and `wsa`, so it parses on its own.
    fn wrap(body: &str) -> String {
        format!("<r xmlns:wse=\"{}\" xmlns:wsa=\"{}\">{body}</r>", ns::WSE, ns::WSA)
    }

    #[test]
    fn subscribe_requests_are_typed_and_well_formed() {
        let mut r = req();
        r.end_to = Some(EndpointRef::new("http://127.0.0.1:9000/end"));
        r.expires = Some(Expiry::After(Duration::from_secs(600)));
        r.filter = Some(Filter::actions(&["http://x/a", "http://x/b"]).unwrap());
        r.extensions.push("<d:Dest xmlns:d=\"urn:d\"><d:Name>Desk</d:Name></d:Dest>".into());
        let text = wrap(&r.to_body().unwrap());
        let doc = xml::parse(&text).unwrap();
        let sub = xml::find(doc.root_element(), ns::WSE, "Subscribe").unwrap();
        let kids: Vec<_> = sub.children().filter(|c| c.is_element()).map(|c| c.tag_name().name().to_owned()).collect();
        assert_eq!(kids, ["EndTo", "Delivery", "Expires", "Filter", "Dest"]);
        let delivery = xml::child(sub, ns::WSE, "Delivery").unwrap();
        assert_eq!(delivery.attribute("Mode"), Some(DELIVERY_MODE_PUSH));
        let notify = EndpointRef::parse(xml::child(delivery, ns::WSE, "NotifyTo").unwrap()).unwrap();
        assert_eq!(notify, r.notify_to);
        assert_eq!(xml::text(xml::child(sub, ns::WSE, "Expires").unwrap()), "PT600S");
        let f = xml::child(sub, ns::WSE, "Filter").unwrap();
        assert_eq!((f.attribute("Dialect"), xml::text(f).as_str()), (Some(DIALECT_ACTION), "http://x/a http://x/b"));
        // Minimal: only the delivery.
        let min = SubscribeRequest::new(EndpointRef::new("http://127.0.0.1:1/s")).to_body().unwrap();
        assert!(!min.contains("Expires") && !min.contains("Filter") && min.contains("NotifyTo"));
    }

    #[test]
    fn unsafe_sinks_filters_and_extensions_are_refused_before_anything_is_sent() {
        for bad in ["http://8.8.8.8/sink", "http://host.example/sink", "urn:uuid:1", "https://127.0.0.1/s"] {
            assert!(matches!(SubscribeRequest::new(EndpointRef::new(bad)).to_body(), Err(Error::Invalid(m)) if m.contains("NotifyTo")), "{bad}");
        }
        let mut r = req();
        r.end_to = Some(EndpointRef::new("http://8.8.8.8/e"));
        assert!(matches!(r.to_body(), Err(Error::Invalid(m)) if m.contains("EndTo")));
        for ext in ["<unclosed>", "not xml", "<a/><b/>", "<d:x/>", &format!("<a>{}</a>", "x".repeat(MAX_EXTENSION))] {
            let mut r = req();
            r.extensions.push(ext.to_owned());
            assert!(matches!(r.to_body(), Err(Error::Invalid(_))), "{ext}");
        }
        assert!(Filter::actions(&[]).is_err());
        assert!(Filter::actions(&["no-colon"]).is_err());
        assert!(Filter::actions(&["http://x/a b"]).is_err());
        assert!(Filter::actions(&vec!["http://x/a"; 33]).is_err());
        assert!(Filter::actions(&[&format!("http://{}", "a".repeat(600))]).is_err());
    }

    #[test]
    fn subscribe_responses_are_read_and_hostile_ones_refused() {
        let ok = format!(
            "<wse:SubscribeResponse xmlns:wse=\"{}\" xmlns:wsa=\"{}\"><wse:SubscriptionManager><wsa:Address>http://127.0.0.1:5357/svc</wsa:Address><wsa:ReferenceParameters><wse:Identifier>uuid:abc</wse:Identifier></wsa:ReferenceParameters></wse:SubscriptionManager><wse:Expires>PT1H</wse:Expires></wse:SubscribeResponse>",
            ns::WSE,
            ns::WSA
        );
        let read = |t: &str| {
            let doc = xml::parse(t).unwrap();
            parse_subscribe_response(doc.root_element())
        };
        let s = read(&ok).unwrap();
        assert_eq!(s.identifier(), Some("uuid:abc"));
        assert_eq!(s.expires, Some(Expiry::After(Duration::from_secs(3600))));
        assert_eq!(s.manager.url().unwrap().port(), 5357);
        // No expiry is fine; no manager, two managers, two expiries or garbage expiry are not.
        assert_eq!(read(&ok.replace("<wse:Expires>PT1H</wse:Expires>", "")).unwrap().expires, None);
        assert!(matches!(read(&format!("<wse:SubscribeResponse xmlns:wse=\"{}\"/>", ns::WSE)), Err(Error::Protocol(_))));
        let mgr = "<wse:SubscriptionManager><wsa:Address>http://127.0.0.1:1/s</wsa:Address></wse:SubscriptionManager>";
        assert!(matches!(read(&ok.replace("<wse:Expires>", &format!("{mgr}<wse:Expires>"))), Err(Error::Protocol(_))));
        assert!(matches!(
            read(&ok.replace("<wse:Expires>PT1H</wse:Expires>", "<wse:Expires>PT1H</wse:Expires><wse:Expires>PT2H</wse:Expires>")),
            Err(Error::Protocol(_))
        ));
        assert!(matches!(read(&ok.replace("PT1H", "tomorrow")), Err(Error::Protocol(_))));
        // A manager at an address we may not talk to still parses, but cannot be used.
        let public = read(&ok.replace("http://127.0.0.1:5357/svc", "http://8.8.8.8/svc")).unwrap();
        assert!(matches!(public.manager.url(), Err(Error::Policy(_))));
    }

    #[test]
    fn a_loopback_manager_from_a_device_on_the_network_is_never_posted_to() {
        use crate::TransportKind;
        use crate::policy::DeviceUrl;
        use crate::transport::{Request, Response};
        use std::cell::Cell;

        /// Counts posts and refuses them, so a call that gets through the policy ends in a
        /// transport error and one that does not ends in a policy error.
        struct Counting(Cell<usize>);
        impl Transport for Counting {
            fn post(&self, _: &DeviceUrl, _: Request) -> Result<Response, Error> {
                self.0.set(self.0.get() + 1);
                Err(Error::transport(TransportKind::Refused, "no network in tests"))
            }
        }

        let manager = |addr: &str| {
            let xml = format!(
                "<wse:SubscribeResponse xmlns:wse=\"{}\" xmlns:wsa=\"{}\"><wse:SubscriptionManager><wsa:Address>{addr}</wsa:Address></wse:SubscriptionManager></wse:SubscribeResponse>",
                ns::WSE,
                ns::WSA
            );
            parse_subscribe_response(xml::parse(&xml).unwrap().root_element()).unwrap()
        };
        let sub = manager("http://127.0.0.1:5357/m");
        let t = Counting(Cell::new(0));
        // The device is on the network: the loopback manager it named is refused before any post.
        let lan = SoapClient::new(&t, DeviceUrl::parse("http://192.168.1.20/svc").unwrap());
        assert!(matches!(renew(&lan, &sub, None), Err(Error::Policy(_))));
        assert!(matches!(get_status(&lan, &sub), Err(Error::Policy(_))));
        assert!(matches!(unsubscribe(&lan, &sub), Err(Error::Policy(_))));
        assert_eq!(t.0.get(), 0);
        // A device on this machine may name a loopback manager; a LAN manager is fine from the LAN.
        let local = SoapClient::new(&t, DeviceUrl::parse("http://127.0.0.1:3911/svc").unwrap());
        assert!(matches!(renew(&local, &sub, None), Err(Error::Transport { .. })));
        assert_eq!(t.0.get(), 1);
        let lan_sub = manager("http://192.168.1.20:5357/m");
        assert!(matches!(renew(&lan, &lan_sub, None), Err(Error::Transport { .. })));
        assert_eq!(t.0.get(), 2);
    }

    #[test]
    fn eventing_fault_codes_are_classified_by_namespace() {
        let code = |ns_uri: &str, local: &str| SubCode { ns: Some(ns_uri.to_owned()), local: local.to_owned() };
        assert_eq!(EventingFault::from_subcode(&code(ns::WSE, "UnableToRenew")), Some(EventingFault::UnableToRenew));
        assert_eq!(EventingFault::from_subcode(&code(ns::WSE, "InvalidMessage")), Some(EventingFault::InvalidMessage));
        assert_eq!(EventingFault::from_subcode(&code("urn:other", "UnableToRenew")), None);
        assert_eq!(EventingFault::from_subcode(&code(ns::WSE, "Unknown")), None);
        assert_eq!(EventingFault::from_subcode(&SubCode::default()), None);
    }

    /// An incoming envelope addressed to the test sink with `action`, extra header blocks and `body`.
    fn envelope(action: &str, header_extra: &str, body: &str) -> String {
        format!(
            "<s:Envelope xmlns:s=\"{}\" xmlns:wsa=\"{}\" xmlns:wse=\"{}\"><s:Header><wsa:To>http://127.0.0.1:9000/sink</wsa:To><wsa:Action>{action}</wsa:Action><wsa:MessageID>urn:uuid:m1</wsa:MessageID>{header_extra}</s:Header><s:Body>{body}</s:Body></s:Envelope>",
            ns::SOAP,
            ns::WSA,
            ns::WSE
        )
    }

    #[test]
    fn notifications_keep_headers_and_a_readable_payload() {
        let m = envelope(
            "urn:ev/Changed",
            "<x:Ctx xmlns:x=\"urn:x\">c1</x:Ctx><x:Nested xmlns:x=\"urn:x\"><y/></x:Nested>",
            "<e:Changed xmlns:e=\"urn:ev\"><e:Value>9</e:Value></e:Changed>",
        );
        let Incoming::Event(n) = parse_incoming(m.as_bytes()).unwrap() else { panic!("not an event") };
        assert_eq!(
            (n.action.as_str(), n.message_id.as_deref(), n.to.as_deref()),
            ("urn:ev/Changed", Some("urn:uuid:m1"), Some("http://127.0.0.1:9000/sink"))
        );
        assert_eq!(n.body, QName::new("urn:ev", "Changed"));
        assert_eq!(n.header("urn:x", "Ctx"), Some("c1"));
        assert_eq!(n.header("urn:x", "Nested"), None, "only plain-text blocks are kept");
        let v = n.with_body(|b| Ok::<_, Error>(xml::child_text(b, "urn:ev", "Value"))).unwrap();
        assert_eq!(v.as_deref(), Some("9"));
    }

    #[test]
    fn subscription_end_is_recognised_with_its_status() {
        for (status, want) in [
            (format!("{}/DeliveryFailure", ns::WSE), EndStatus::DeliveryFailure),
            (format!("{}/SourceShuttingDown", ns::WSE), EndStatus::SourceShuttingDown),
            (format!("{}/SourceCancelling", ns::WSE), EndStatus::SourceCancelling),
            ("urn:vendor/Odd".to_owned(), EndStatus::Other("urn:vendor/Odd".into())),
            // A look-alike under another namespace is not the standard status.
            ("urn:other/DeliveryFailure".to_owned(), EndStatus::Other("urn:other/DeliveryFailure".into())),
            // Neither a longer namespace nor a deeper path is the standard status.
            (format!("{}X/DeliveryFailure", ns::WSE), EndStatus::Other(format!("{}X/DeliveryFailure", ns::WSE))),
            (format!("{}/DeliveryModes/DeliveryFailure", ns::WSE), EndStatus::Other(format!("{}/DeliveryModes/DeliveryFailure", ns::WSE))),
        ] {
            let body = format!(
                "<wse:SubscriptionEnd><wse:SubscriptionManager><wsa:Address>http://127.0.0.1:5357/s</wsa:Address><wsa:ReferenceParameters><wse:Identifier>id1</wse:Identifier></wsa:ReferenceParameters></wse:SubscriptionManager><wse:Status>{status}</wse:Status><wse:Reason xml:lang=\"en\">gone</wse:Reason></wse:SubscriptionEnd>"
            );
            let Incoming::SubscriptionEnd(e) = parse_incoming(envelope(ACTION_SUBSCRIPTION_END, "", &body).as_bytes()).unwrap() else {
                panic!("not an end")
            };
            assert_eq!((&e.status, e.reason.as_str(), e.identifier()), (&want, "gone", Some("id1")));
        }
    }

    #[test]
    fn hostile_incoming_messages_are_errors() {
        let ev = "<e:E xmlns:e=\"urn:ev\"/>";
        assert!(matches!(parse_incoming(b"junk"), Err(Error::Xml(_))));
        assert!(matches!(parse_incoming(b"<a/>"), Err(Error::Protocol(_))));
        assert!(matches!(parse_incoming(envelope("", "", ev).as_bytes()), Err(Error::Protocol(_))));
        assert!(matches!(parse_incoming(envelope("urn:a", "", "").as_bytes()), Err(Error::Protocol(_))));
        let no_header = format!("<s:Envelope xmlns:s=\"{}\"><s:Body>{ev}</s:Body></s:Envelope>", ns::SOAP);
        assert!(matches!(parse_incoming(no_header.as_bytes()), Err(Error::Protocol(_))));
        let fault = "<s:Fault><s:Code><s:Value>s:Sender</s:Value></s:Code><s:Reason><s:Text>no</s:Text></s:Reason></s:Fault>";
        assert!(matches!(parse_incoming(envelope("urn:a", "", fault).as_bytes()), Err(Error::Fault(_))));
        // A SubscriptionEnd missing its parts.
        for end in ["<wse:SubscriptionEnd/>", "<wse:SubscriptionEnd><wse:Status>x</wse:Status></wse:SubscriptionEnd>"] {
            assert!(matches!(parse_incoming(envelope(ACTION_SUBSCRIPTION_END, "", end).as_bytes()), Err(Error::Protocol(_))), "{end}");
        }
        // Depth and DTD bombs.
        let deep = envelope("urn:a", "", &format!("{}{}", "<a>".repeat(60), "</a>".repeat(60)));
        assert!(matches!(parse_incoming(deep.as_bytes()), Err(Error::Xml(_))));
        let dtd = format!("<!DOCTYPE x [<!ENTITY e \"1\">]>{}", envelope("urn:a", "", ev));
        assert!(matches!(parse_incoming(dtd.as_bytes()), Err(Error::Xml(_))));
        // Header fan-out is capped.
        let many: String = (0..100).map(|i| format!("<h{i} xmlns=\"urn:h\">v</h{i}>")).collect();
        let Incoming::Event(n) = parse_incoming(envelope("urn:a", &many, ev).as_bytes()).unwrap() else { panic!("not an event") };
        assert_eq!(n.headers.len(), MAX_HEADERS);
    }

    #[test]
    fn a_sink_receives_events_and_ends_through_dispatch() {
        let sink = CollectingSink::new();
        dispatch(envelope("urn:a", "", "<e:E xmlns:e=\"urn:ev\"/>").as_bytes(), &sink).unwrap();
        let end = format!(
            "<wse:SubscriptionEnd><wse:SubscriptionManager><wsa:Address>urn:m</wsa:Address></wse:SubscriptionManager><wse:Status>{}/SourceCancelling</wse:Status></wse:SubscriptionEnd>",
            ns::WSE
        );
        dispatch(envelope(ACTION_SUBSCRIPTION_END, "", &end).as_bytes(), &sink).unwrap();
        assert!(dispatch(b"junk", &sink).is_err());
        let items = sink.items();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], Incoming::Event(_)) && matches!(items[1], Incoming::SubscriptionEnd(_)));
        // A sink that refuses makes dispatch fail so the listener can answer with an error status.
        struct Refuse;
        impl NotificationSink for Refuse {
            /// Refuse every event.
            fn notification(&self, _: &Notification) -> Result<()> {
                Err(Error::Invalid("full".into()))
            }
        }
        assert!(dispatch(envelope("urn:a", "", "<e:E xmlns:e=\"urn:ev\"/>").as_bytes(), &Refuse).is_err());
        assert!(dispatch(envelope(ACTION_SUBSCRIPTION_END, "", &end).as_bytes(), &Refuse).is_ok(), "the default end handler ignores it");
    }
}
