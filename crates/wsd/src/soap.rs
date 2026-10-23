//! The generic SOAP call layer: one [`SoapClient`] per endpoint, one [`Call`] per exchange.
//!
//! A service crate (print, scan) or a protocol module of this crate (metadata, eventing) describes
//! an exchange as a [`Call`]: the request action, the action and body element the reply must
//! have, the namespaces and body of the request, optional extra header blocks and an optional
//! MTOM attachment. The client does everything that is the same for every WSD service:
//!
//! * a fresh `MessageID` per call (replaceable, so tests are deterministic);
//! * the envelope, the `Content-Type` (`action` parameter included) and the MTOM package;
//! * the request-size cap, the per-request timeout and an overall deadline, and a cancel token
//!   checked before every exchange;
//! * the reply: the root part of a multipart answer, the body cap, the rule that a non-success
//!   HTTP status is an [`Error::Http`] (with the device's `Retry-After`) unless the body is a SOAP
//!   envelope carrying a fault (which keeps the `Retry-After` too), and
//!   the checked reading of the envelope ([`crate::envelope::read_reply`]) with faults mapped to
//!   [`Error::Fault`].
//!
//! It owns no socket: the [`Transport`] does the POST, so the same code runs against the real HTTP
//! transport, a scripted in-memory device or a test server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use roxmltree::Node;

use crate::cancel::CancelToken;
use crate::envelope::{self, Expect, build_envelope_with_headers, new_uuid, read_ack_hinted, read_reply_hinted};
use crate::error::{Error, Result};
use crate::mtom;
use crate::ns;
use crate::policy::{DeviceUrl, NetworkLimits};
use crate::transport::{Request, Response, Transport};

/// The slowest upload the time limit allows for, in bytes a second. The limit covers the whole
/// request, body included, so a large document (`SendDocument`) over a slow link must not use up
/// the budget meant for the printer's reply: that would end as an outcome-unknown timeout after the
/// job was created.
const UPLOAD_FLOOR_BYTES_PER_SEC: u64 = 64 * 1024;

/// `timeout` for the exchange plus the time to send `body_len` bytes at the upload floor.
fn with_upload_allowance(timeout: Duration, body_len: usize) -> Duration {
    timeout.saturating_add(Duration::from_secs(body_len as u64 / UPLOAD_FLOOR_BYTES_PER_SEC))
}
use crate::xml;

/// Produces message ids (without the `urn:uuid:` prefix).
pub type IdSource = Arc<dyn Fn() -> String + Send + Sync>;

/// A binary payload sent as an MTOM part; the call's body must hold the `xop:Include` for `cid`
/// ([`mtom::xop_include_ref`]).
#[derive(Clone, Copy, Debug)]
pub struct Attachment<'a> {
    pub cid: &'a str,
    pub data: &'a [u8],
}

/// What the reply's Body must hold.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ReplyShape {
    /// A first element `{ns}local`, handed to the caller's reader.
    Element(String, String),
    /// Nothing is read: an empty Body (or any non-fault payload) is acceptable.
    Ack,
}

/// One exchange: what to send and what to expect back.
#[derive(Clone, Debug)]
pub struct Call<'a> {
    action: String,
    response_action: String,
    reply: ReplyShape,
    namespaces: Vec<(String, String)>,
    headers: String,
    body: String,
    attachment: Option<(String, &'a [u8])>,
}

impl<'a> Call<'a> {
    /// A call with this request action and expected reply action. The reply is an acknowledgement
    /// (no payload read) until [`Self::reply_element`] says otherwise.
    pub fn new(action: impl Into<String>, response_action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            response_action: response_action.into(),
            reply: ReplyShape::Ack,
            namespaces: Vec::new(),
            headers: String::new(),
            body: String::new(),
            attachment: None,
        }
    }

    /// The reply's Body must start with `{ns}local`; it is handed to the reader.
    pub fn reply_element(mut self, ns: impl Into<String>, local: impl Into<String>) -> Self {
        self.reply = ReplyShape::Element(ns.into(), local.into());
        self
    }

    /// Declare `prefix` on the envelope for the body (and headers) to use. `s` and `wsa` always are.
    pub fn namespace(mut self, prefix: impl Into<String>, uri: impl Into<String>) -> Self {
        self.namespaces.push((prefix.into(), uri.into()));
        self
    }

    /// Extra, well-formed header blocks (for instance [`crate::addressing::EndpointRef::header_xml`]).
    pub fn headers(mut self, xml: impl Into<String>) -> Self {
        self.headers = xml.into();
        self
    }

    /// The Body's content: a well-formed fragment, empty for WS-Transfer `Get`.
    pub fn body(mut self, xml: impl Into<String>) -> Self {
        self.body = xml.into();
        self
    }

    /// Send `data` as an MTOM part with content id `cid`.
    pub fn attachment(mut self, att: Attachment<'a>) -> Self {
        self.attachment = Some((att.cid.to_owned(), att.data));
        self
    }

    /// The request's `wsa:Action`.
    pub fn action(&self) -> &str {
        &self.action
    }
}

/// A client bound to one endpoint of one device.
pub struct SoapClient<T: Transport> {
    transport: T,
    endpoint: DeviceUrl,
    limits: NetworkLimits,
    cancel: Option<CancelToken>,
    deadline: Option<Instant>,
    ids: IdSource,
    reply_label: &'static str,
}

/// Message ids as random v4 UUIDs.
fn default_ids() -> IdSource {
    Arc::new(new_uuid)
}

impl<T: Transport> SoapClient<T> {
    /// A client for `endpoint` with the default limits, no cancel token, no deadline and random message ids.
    /// Nothing is sent until a call.
    pub fn new(transport: T, endpoint: DeviceUrl) -> Self {
        Self {
            transport,
            endpoint,
            limits: NetworkLimits::default(),
            cancel: None,
            deadline: None,
            ids: default_ids(),
            reply_label: "the device's reply",
        }
    }

    /// Use `limits` for the timeout and the request and response size caps.
    pub fn with_limits(mut self, limits: NetworkLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Refuse to start any further exchange once `token` is set ([`Error::Cancelled`]).
    pub fn with_cancel(mut self, token: CancelToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// No exchange may start after `deadline` ([`Error::Timeout`]), and none may run past it: the
    /// request timeout is cut to the time left.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// [`Self::with_deadline`] `budget` from now.
    pub fn with_budget(self, budget: Duration) -> Self {
        let deadline = Instant::now().checked_add(budget);
        match deadline {
            Some(d) => self.with_deadline(d),
            None => self,
        }
    }

    /// Replace the message-id source (tests; ids are otherwise random UUIDs).
    pub fn with_id_source(mut self, ids: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.ids = Arc::new(ids);
        self
    }

    /// How an oversized reply is named in [`Error::TooLarge`] ("the printer's reply").
    pub fn with_reply_label(mut self, label: &'static str) -> Self {
        self.reply_label = label;
        self
    }

    /// The endpoint every call is posted to.
    pub fn endpoint(&self) -> &DeviceUrl {
        &self.endpoint
    }

    /// The limits calls are made with.
    pub fn limits(&self) -> NetworkLimits {
        self.limits
    }

    /// The transport calls go through.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// A client for another endpoint of the same device (the metadata endpoint, a subscription
    /// manager) that shares this one's transport, limits, cancel token, deadline and id source.
    pub fn at(&self, endpoint: DeviceUrl) -> SoapClient<&T> {
        SoapClient {
            transport: &self.transport,
            endpoint,
            limits: self.limits,
            cancel: self.cancel.clone(),
            deadline: self.deadline,
            ids: Arc::clone(&self.ids),
            reply_label: self.reply_label,
        }
    }

    /// [`Error::Cancelled`] once the client's cancel token is set.
    fn check_cancel(&self) -> Result<()> {
        if self.cancel.as_ref().is_some_and(CancelToken::is_cancelled) { Err(Error::Cancelled) } else { Ok(()) }
    }

    /// The time one request of `body_len` bytes may take: the configured timeout plus the upload
    /// allowance, then cut to the deadline (the deadline is the last word).
    fn budget(&self, body_len: usize) -> Result<Duration> {
        self.check_cancel()?;
        let mut timeout = with_upload_allowance(self.limits.timeout, body_len);
        if let Some(deadline) = self.deadline {
            let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(Error::Timeout)?;
            timeout = timeout.min(left);
        }
        Ok(timeout)
    }

    /// Send `call` and hand the reply's first Body element to `read`. `E` is the caller's error
    /// type; protocol errors convert into it.
    pub fn call<R, E: From<Error>>(&self, call: &Call<'_>, read: impl FnOnce(Node<'_, '_>) -> Result<R, E>) -> Result<R, E> {
        let ReplyShape::Element(body_ns, body_local) = &call.reply else {
            return Err(Error::Invalid("call: this call expects no reply payload; use call_ack".into()).into());
        };
        let (id, bytes, retry_after) = self.exchange(call)?;
        read_reply_hinted(&bytes, Expect { action: &call.response_action, message_id: &id, body_ns, body_local }, retry_after, read)
    }

    /// Send `call` and check only that the answer is not a fault and belongs to the request.
    pub fn call_ack<E: From<Error>>(&self, call: &Call<'_>) -> Result<(), E> {
        let (id, bytes, retry_after) = self.exchange(call)?;
        read_ack_hinted(&bytes, &call.response_action, &id, retry_after)
    }

    /// Like [`Self::call`], for replies that may carry binary parts (scanned images): `read` also
    /// gets the parts of a successful `multipart/related` reply (empty for a plain reply), to
    /// resolve `xop:Include` references with [`mtom::resolve_href`].
    pub fn call_with_parts<R, E: From<Error>>(
        &self,
        call: &Call<'_>,
        read: impl FnOnce(Node<'_, '_>, &[mtom::Part<'_>]) -> Result<R, E>,
    ) -> Result<R, E> {
        let ReplyShape::Element(body_ns, body_local) = &call.reply else {
            return Err(Error::Invalid("call: this call expects no reply payload; use call_ack".into()).into());
        };
        let (id, response) = self.post(call)?;
        let ok = (200..300).contains(&response.status);
        let ct = response.content_type.clone().unwrap_or_default();
        if ok && mtom::is_multipart(&ct) && response.body.len() <= self.limits.max_response {
            let parts = mtom::parts(&ct, &response.body)?;
            let root = mtom::root_of(&ct, &parts)?;
            let expect = Expect { action: &call.response_action, message_id: &id, body_ns, body_local };
            return read_reply_hinted(root.data, expect, response.retry_after, |n| read(n, &parts));
        }
        let retry_after = response.retry_after;
        let xml = self.reply_xml(response)?;
        read_reply_hinted(&xml, Expect { action: &call.response_action, message_id: &id, body_ns, body_local }, retry_after, |n| read(n, &[]))
    }

    /// Post the call and return its message id with the reply's XML bytes and `Retry-After`.
    fn exchange(&self, call: &Call<'_>) -> Result<(String, Vec<u8>, Option<Duration>)> {
        let (id, response) = self.post(call)?;
        let retry_after = response.retry_after;
        Ok((id, self.reply_xml(response)?, retry_after))
    }

    /// Post the call: its message id and the raw reply.
    fn post(&self, call: &Call<'_>) -> Result<(String, Response)> {
        self.check_cancel()?;
        let id = (self.ids)();
        let namespaces: Vec<(&str, &str)> = call.namespaces.iter().map(|(p, u)| (p.as_str(), u.as_str())).collect();
        let envelope = build_envelope_with_headers(&self.endpoint.as_url(), &call.action, &id, &namespaces, &call.headers, &call.body);
        let (content_type, body) = match &call.attachment {
            Some((cid, data)) => {
                let package = mtom::package(&envelope, cid, data);
                (package.content_type, package.body)
            }
            None => (format!("application/soap+xml; charset=utf-8; action=\"{}\"", call.action), envelope.into_bytes()),
        };
        if body.len() > self.limits.max_request {
            return Err(Error::TooLarge("the request", self.limits.max_request));
        }
        let timeout = self.budget(body.len())?;
        let request = Request { content_type, soap_action: call.action.clone(), body, timeout, max_response: self.limits.max_response };
        Ok((id, self.transport.post(&self.endpoint, request)?))
    }

    /// The XML of a reply: the root part of a multipart reply, else the body. A non-success
    /// status is an [`Error::Http`] unless the body is a SOAP envelope carrying a fault (an HTML
    /// maintenance page, other XML, an envelope without a fault, junk, or a body over the limit
    /// all keep the status); the fault is then read later by `read_reply`.
    fn reply_xml(&self, response: Response) -> Result<Vec<u8>> {
        let ok = (200..300).contains(&response.status);
        let (status, retry_after) = (response.status, response.retry_after);
        let http = || Error::http_retry_after(status, retry_after);
        if response.body.len() > self.limits.max_response {
            return Err(if ok { Error::TooLarge(self.reply_label, self.limits.max_response) } else { http() });
        }
        let ct = response.content_type.clone().unwrap_or_default();
        let xml = if mtom::is_multipart(&ct) {
            mtom::root_part(&ct, &response.body).map(<[u8]>::to_vec).map_err(|e| if ok { e } else { http() })?
        } else {
            response.body
        };
        if !ok && !is_fault_envelope(&xml) {
            return Err(http());
        }
        Ok(xml)
    }
}

/// Is this a SOAP envelope whose Body starts with a Fault? Anything that does not parse is not.
fn is_fault_envelope(bytes: &[u8]) -> bool {
    let first = bytes.iter().find(|b| !b.is_ascii_whitespace());
    if first != Some(&b'<') && first != Some(&0xEF) {
        return false;
    }
    let Ok(text) = xml::checked_text(bytes) else { return false };
    let Ok(doc) = xml::parse(text) else { return false };
    let root = doc.root_element();
    xml::is(root, ns::SOAP, "Envelope")
        && xml::child(root, ns::SOAP, "Body").and_then(|b| b.children().find(|c| c.is_element())).is_some_and(|p| xml::is(p, ns::SOAP, "Fault"))
}

/// Build a reply envelope as a device would (test helper, also used by the in-memory fakes): the
/// reply `action`, `RelatesTo` the request's message id, and `body`.
pub fn reply_envelope(action: &str, relates_to: &str, extra_ns: &[(&str, &str)], body: &str) -> String {
    let extra: String = extra_ns.iter().map(|(p, u)| format!(" xmlns:{p}=\"{}\"", xml::esc(u))).collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{soap}\" xmlns:wsa=\"{wsa}\"{extra}><s:Header><wsa:Action>{action}</wsa:Action><wsa:MessageID>urn:uuid:{id}</wsa:MessageID><wsa:RelatesTo>urn:uuid:{rel}</wsa:RelatesTo></s:Header><s:Body>{body}</s:Body></s:Envelope>",
        soap = ns::SOAP,
        wsa = ns::WSA,
        action = xml::esc(action),
        id = envelope::new_uuid(),
        rel = xml::esc(relates_to),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::error::TransportKind;

    const SVC: &str = "urn:example:service";
    const ACT: &str = "urn:example:service/DoIt";
    const ACT_RESP: &str = "urn:example:service/DoItResponse";

    type Replier = Box<dyn Fn(&Request) -> Result<Response> + Send + Sync>;

    /// Answers every post by calling `reply(request)`; keeps what it was sent.
    struct Script {
        seen: Mutex<Vec<(String, Request)>>,
        reply: Replier,
    }

    impl Script {
        /// A script that answers every post with `reply`.
        fn new(reply: impl Fn(&Request) -> Result<Response> + Send + Sync + 'static) -> Self {
            Self { seen: Mutex::new(Vec::new()), reply: Box::new(reply) }
        }
        /// A script that answers every post with a `DoItResponse` of value 42 related to the request's message id.
        fn echo() -> Self {
            Self::new(|req| {
                let text = String::from_utf8_lossy(&req.body).into_owned();
                let id = text.split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned();
                Ok(Response {
                    retry_after: None,
                    status: 200,
                    content_type: Some("application/soap+xml".into()),
                    body: reply_envelope(ACT_RESP, &id, &[("svc", SVC)], "<svc:DoItResponse><svc:Value>42</svc:Value></svc:DoItResponse>")
                        .into_bytes(),
                })
            })
        }
        /// How many posts the script has received.
        fn count(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
    }

    impl Transport for Script {
        /// Record the URL and request, then answer with the script's reply.
        fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response> {
            self.seen.lock().unwrap().push((url.as_url(), request.clone()));
            (self.reply)(&request)
        }
    }

    /// The loopback endpoint the tests post to.
    fn url() -> DeviceUrl {
        DeviceUrl::parse("http://127.0.0.1:5357/svc").unwrap()
    }

    /// A `DoIt` call expecting a `DoItResponse` element.
    fn call() -> Call<'static> {
        Call::new(ACT, ACT_RESP).reply_element(SVC, "DoItResponse").namespace("svc", SVC).body("<svc:DoItRequest/>")
    }

    /// The text of the reply's `Value` child; empty when there is none.
    fn value(n: Node<'_, '_>) -> Result<String> {
        Ok(xml::child_text(n, SVC, "Value").unwrap_or_default())
    }

    #[test]
    fn a_call_posts_a_checked_envelope_and_reads_the_reply() {
        let script = Script::echo();
        let client = SoapClient::new(&script, url()).with_id_source(|| "fixed-id".to_owned());
        let c = call().headers("<svc:Token xmlns:svc=\"urn:example:service\">t</svc:Token>");
        assert_eq!(client.call(&c, value).unwrap(), "42");
        let seen = script.seen.lock().unwrap();
        let (to, req) = &seen[0];
        assert_eq!(to, "http://127.0.0.1:5357/svc");
        assert_eq!(req.soap_action, ACT);
        assert_eq!(req.content_type, format!("application/soap+xml; charset=utf-8; action=\"{ACT}\""));
        assert_eq!(req.timeout, NetworkLimits::default().timeout);
        let text = String::from_utf8(req.body.clone()).unwrap();
        let doc = xml::parse(&text).unwrap();
        let h = xml::child(doc.root_element(), ns::SOAP, "Header").unwrap();
        assert_eq!(xml::text(xml::child(h, ns::WSA, "MessageID").unwrap()), "urn:uuid:fixed-id");
        assert_eq!(xml::text(xml::child(h, ns::WSA, "To").unwrap()), "http://127.0.0.1:5357/svc");
        assert!(xml::child(h, SVC, "Token").is_some());
    }

    #[test]
    fn ids_are_fresh_by_default() {
        let script = Script::echo();
        let client = SoapClient::new(&script, url());
        client.call(&call(), value).unwrap();
        client.call(&call(), value).unwrap();
        let seen = script.seen.lock().unwrap();
        assert_ne!(seen[0].1.body, seen[1].1.body);
    }

    #[test]
    fn a_reader_error_type_of_the_callers_own_is_supported() {
        #[derive(Debug, PartialEq)]
        enum Mine {
            Wire(Error),
            Odd,
        }
        impl From<Error> for Mine {
            fn from(e: Error) -> Self {
                Self::Wire(e)
            }
        }
        let script = Script::echo();
        let client = SoapClient::new(&script, url());
        assert_eq!(client.call(&call(), |_| Err::<(), _>(Mine::Odd)), Err(Mine::Odd));
        let down = Script::new(|_| Err(Error::transport(TransportKind::Refused, "down")));
        assert_eq!(SoapClient::new(&down, url()).call(&call(), value), Err(Error::transport(TransportKind::Refused, "down")));
    }

    #[test]
    fn faults_http_errors_and_mismatches_map_to_errors() {
        let fault = |status: u16| {
            Script::new(move |req| {
                let text = String::from_utf8_lossy(&req.body).into_owned();
                let id = text.split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned();
                let body = "<s:Fault><s:Code><s:Value>s:Receiver</s:Value><s:Subcode><s:Value xmlns:v=\"urn:example:service\">v:Busy</s:Value></s:Subcode></s:Code><s:Reason><s:Text>busy</s:Text></s:Reason></s:Fault>";
                Ok(Response {
                    retry_after: None,
                    status,
                    content_type: Some("application/soap+xml".into()),
                    body: reply_envelope(ns::FAULT_ACTION, &id, &[], body).into_bytes(),
                })
            })
        };
        for status in [200, 500] {
            let s = fault(status);
            match SoapClient::new(&s, url()).call(&call(), value) {
                Err(Error::Fault(f)) => assert!(f.code.is(SVC, "Busy")),
                other => panic!("{other:?}"),
            }
        }
        let html = |status: u16| {
            Script::new(move |_| {
                Ok(Response { retry_after: None, status, content_type: Some("text/html".into()), body: b"<html>maintenance</html>".to_vec() })
            })
        };
        assert_eq!(SoapClient::new(&html(503), url()).call(&call(), value), Err(Error::http(503)));
        assert!(SoapClient::new(&html(503), url()).call(&call(), value).unwrap_err().is_retryable());
        // A 200 with the wrong reply action is a protocol error.
        let wrong = Script::new(|req| {
            let text = String::from_utf8_lossy(&req.body).into_owned();
            let id = text.split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned();
            Ok(Response {
                retry_after: None,
                status: 200,
                content_type: None,
                body: reply_envelope("urn:other", &id, &[("svc", SVC)], "<svc:DoItResponse/>").into_bytes(),
            })
        });
        assert!(matches!(SoapClient::new(&wrong, url()).call(&call(), value), Err(Error::Protocol(_))));
    }

    #[test]
    fn oversized_replies_and_requests_are_refused() {
        let big = |status: u16| {
            Script::new(move |_| {
                Ok(Response { retry_after: None, status, content_type: None, body: vec![b' '; NetworkLimits::default().max_response + 1] })
            })
        };
        let c = SoapClient::new(big(200), url()).with_reply_label("the printer's reply");
        match c.call(&call(), value) {
            Err(e @ Error::TooLarge(..)) => assert!(e.to_string().contains("the printer's reply"), "{e}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(SoapClient::new(big(500), url()).call(&call(), value), Err(Error::http(500)));
        let script = Script::echo();
        let tiny = NetworkLimits { max_request: 100, ..NetworkLimits::default() };
        assert!(matches!(SoapClient::new(&script, url()).with_limits(tiny).call(&call(), value), Err(Error::TooLarge("the request", 100))));
        assert_eq!(script.count(), 0, "an oversized request is never sent");
    }

    #[test]
    fn cancel_and_deadline_stop_calls_before_they_are_sent() {
        let script = Script::echo();
        let token = CancelToken::new();
        let client = SoapClient::new(&script, url()).with_cancel(token.clone());
        assert!(client.call(&call(), value).is_ok());
        token.cancel();
        assert_eq!(client.call(&call(), value), Err(Error::Cancelled));
        assert_eq!(client.call_ack::<Error>(&call()), Err(Error::Cancelled));
        assert_eq!(script.count(), 1);
        let past = SoapClient::new(&script, url()).with_deadline(Instant::now());
        assert_eq!(past.call(&call(), value), Err(Error::Timeout));
        assert_eq!(script.count(), 1);
    }

    #[test]
    fn the_request_timeout_is_cut_to_the_deadline() {
        let script = Script::echo();
        let client = SoapClient::new(&script, url()).with_budget(Duration::from_secs(5));
        client.call(&call(), value).unwrap();
        let t = script.seen.lock().unwrap()[0].1.timeout;
        assert!(t <= Duration::from_secs(5) && t > Duration::from_secs(1), "{t:?}");
        // A far deadline does not lengthen the configured timeout.
        let far = SoapClient::new(&script, url()).with_budget(Duration::from_secs(3600));
        far.call(&call(), value).unwrap();
        assert_eq!(script.seen.lock().unwrap()[1].1.timeout, NetworkLimits::default().timeout);
    }

    #[test]
    fn the_time_limit_grows_with_the_body_it_has_to_send() {
        let base = Duration::from_secs(30);
        assert_eq!(with_upload_allowance(base, 0), base, "a small request keeps its limit");
        assert_eq!(with_upload_allowance(base, 10 * 1024), base);
        // 6.4 MiB at 64 KiB/s needs 100 s more.
        assert_eq!(with_upload_allowance(base, 6_553_600), base + Duration::from_secs(100));
        assert_eq!(with_upload_allowance(Duration::MAX, usize::MAX), Duration::MAX, "no overflow");
    }

    #[test]
    fn a_deadline_bounds_the_timeout_of_a_large_body() {
        let script = Script::echo();
        let data = vec![0u8; 10 * 1024 * 1024];
        let big = || call().attachment(Attachment { cid: "doc@x", data: &data });
        let bounded = SoapClient::new(&script, url()).with_budget(Duration::from_secs(60));
        bounded.call(&big(), value).unwrap();
        let t = script.seen.lock().unwrap()[0].1.timeout;
        assert!(t <= Duration::from_secs(60), "the upload allowance must not outrun the deadline: {t:?}");
        // Without a deadline the allowance applies in full.
        let free = SoapClient::new(&script, url());
        free.call(&big(), value).unwrap();
        let t = script.seen.lock().unwrap()[1].1.timeout;
        assert!(t > NetworkLimits::default().timeout + Duration::from_secs(150), "{t:?}");
    }

    #[test]
    fn at_shares_the_transport_and_settings_but_targets_another_endpoint() {
        let script = Script::echo();
        let token = CancelToken::new();
        let client = SoapClient::new(&script, url()).with_cancel(token.clone()).with_id_source(|| "shared".into());
        let other = client.at(DeviceUrl::parse("http://127.0.0.1:5357/manager").unwrap());
        other.call(&call(), value).unwrap();
        assert_eq!(script.seen.lock().unwrap()[0].0, "http://127.0.0.1:5357/manager");
        assert!(String::from_utf8_lossy(&script.seen.lock().unwrap()[0].1.body).contains("urn:uuid:shared"));
        token.cancel();
        assert_eq!(other.call(&call(), value), Err(Error::Cancelled));
        assert_eq!(client.endpoint().path(), "/svc");
    }

    #[test]
    fn acks_accept_an_empty_body_and_a_payload_call_refuses_ack_shape() {
        let empty = Script::new(|req| {
            let text = String::from_utf8_lossy(&req.body).into_owned();
            let id = text.split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned();
            Ok(Response { retry_after: None, status: 200, content_type: None, body: reply_envelope(ACT_RESP, &id, &[], "").into_bytes() })
        });
        let client = SoapClient::new(&empty, url());
        assert!(client.call_ack::<Error>(&Call::new(ACT, ACT_RESP)).is_ok());
        assert!(matches!(client.call(&Call::new(ACT, ACT_RESP), value), Err(Error::Invalid(_))));
        // Asking for an element from an empty body is a protocol error.
        assert!(matches!(client.call(&call(), value), Err(Error::Protocol(_))));
    }

    #[test]
    fn an_attachment_is_packaged_as_mtom_and_a_multipart_reply_is_unwrapped() {
        let seen_ct = Arc::new(Mutex::new(String::new()));
        let seen = Arc::clone(&seen_ct);
        let script = Script::new(move |req| {
            *seen.lock().unwrap() = req.content_type.clone();
            let ct = req.content_type.clone();
            let root = mtom::root_part(&ct, &req.body).unwrap();
            assert!(String::from_utf8_lossy(root).contains("xop:Include"));
            assert!(req.body.windows(5).any(|w| w == b"BYTES"));
            let id = String::from_utf8_lossy(root).split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned();
            let inner = reply_envelope(ACT_RESP, &id, &[("svc", SVC)], "<svc:DoItResponse><svc:Value>7</svc:Value></svc:DoItResponse>");
            let pkg = mtom::package(&inner, "x@y", b"unused");
            Ok(Response { retry_after: None, status: 200, content_type: Some(pkg.content_type), body: pkg.body })
        });
        let c = Call::new(ACT, ACT_RESP)
            .reply_element(SVC, "DoItResponse")
            .namespace("svc", SVC)
            .body(format!("<svc:DoItRequest>{}</svc:DoItRequest>", mtom::xop_include_ref("doc@x")))
            .attachment(Attachment { cid: "doc@x", data: b"BYTES" });
        assert_eq!(SoapClient::new(&script, url()).call(&c, value).unwrap(), "7");
        assert!(seen_ct.lock().unwrap().starts_with("multipart/related"));
    }

    #[test]
    fn broken_multipart_replies_keep_the_http_status_when_not_successful() {
        let bad = |status: u16| {
            Script::new(move |_| {
                Ok(Response { retry_after: None, status, content_type: Some("multipart/related; boundary=b".into()), body: b"<html>".to_vec() })
            })
        };
        assert_eq!(SoapClient::new(bad(502), url()).call(&call(), value), Err(Error::http(502)));
        assert!(matches!(SoapClient::new(bad(200), url()).call(&call(), value), Err(Error::Protocol(_) | Error::Xml(_))));
    }

    #[test]
    fn binary_parts_of_a_multipart_reply_are_handed_to_the_reader() {
        let script = Script::new(|req| {
            let text = String::from_utf8_lossy(&req.body).into_owned();
            let id = text.split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned();
            let inner = reply_envelope(
                ACT_RESP,
                &id,
                &[("svc", SVC)],
                &format!("<svc:DoItResponse><svc:Image>{}</svc:Image></svc:DoItResponse>", mtom::xop_include_ref("img@x")),
            );
            let pkg = mtom::package(&inner, "img@x", b"PIXELS");
            Ok(Response { retry_after: None, status: 200, content_type: Some(pkg.content_type), body: pkg.body })
        });
        let read = |n: Node<'_, '_>, parts: &[mtom::Part<'_>]| -> Result<Vec<u8>> {
            let inc = xml::find(n, ns::XOP, "Include").ok_or_else(|| Error::Protocol("no include".into()))?;
            mtom::resolve_href(parts, inc.attribute("href").unwrap_or(""))
                .map(<[u8]>::to_vec)
                .ok_or_else(|| Error::Protocol("dangling include".into()))
        };
        assert_eq!(SoapClient::new(&script, url()).call_with_parts(&call(), read).unwrap(), b"PIXELS");
        // A plain (non-multipart) reply gives the reader no parts, so the include dangles.
        let plain = Script::echo();
        assert!(
            matches!(SoapClient::new(&plain, url()).call_with_parts(&call(), |n, parts| { assert!(parts.is_empty()); value(n) }), Ok(v) if v == "42")
        );
        // Faults still win.
        let busy = Script::new(|_| Ok(Response { retry_after: None, status: 503, content_type: Some("text/html".into()), body: b"x".to_vec() }));
        assert_eq!(SoapClient::new(&busy, url()).call_with_parts(&call(), |_, _| Ok::<_, Error>(())), Err(Error::http(503)));
    }

    const BUSY_FAULT: &str = "<s:Fault><s:Code><s:Value>s:Receiver</s:Value><s:Subcode><s:Value xmlns:v=\"urn:example:service\">v:Busy</s:Value><s:Subcode><s:Value xmlns:v=\"urn:example:service\">v:PaperOut</s:Value></s:Subcode></s:Subcode></s:Code><s:Reason><s:Text>busy</s:Text></s:Reason></s:Fault>";

    /// The message id (without `urn:uuid:`) of a request the client sent.
    fn id_of(req: &Request) -> String {
        let text = String::from_utf8_lossy(&req.body).into_owned();
        text.split("urn:uuid:").nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_owned()
    }

    #[test]
    fn retry_after_reaches_http_errors_and_faults_on_every_call_path() {
        let wait = Some(Duration::from_secs(7));
        // A non-SOAP 503: the status error carries the wait.
        let html = Script::new(move |_| Ok(Response::new(503, Some("text/html".into()), b"<html/>".to_vec()).with_retry_after(wait)));
        let e = SoapClient::new(&html, url()).call(&call(), value).unwrap_err();
        assert_eq!(e, Error::http_retry_after(503, wait));
        assert_eq!((e.retry_after(), e.http_status(), e.is_retryable()), (wait, Some(503), true));
        assert_eq!(e.retry_safety(false), crate::RetrySafety::Safe, "a 503 is a decline, not a half-done job");
        let e = SoapClient::new(&html, url()).call_ack::<Error>(&call()).unwrap_err();
        assert_eq!(e.retry_after(), wait);
        let e = SoapClient::new(&html, url()).call_with_parts(&call(), |_, _| Ok::<_, Error>(())).unwrap_err();
        assert_eq!(e.retry_after(), wait);
        // An oversized error body keeps the status and the wait too.
        let big = Script::new(move |_| Ok(Response::new(429, None, vec![b' '; NetworkLimits::default().max_response + 1]).with_retry_after(wait)));
        assert_eq!(SoapClient::new(&big, url()).call(&call(), value), Err(Error::http_retry_after(429, wait)));
        // A fault in a 503 keeps the wait on the typed fault.
        let fault = Script::new(move |req| {
            Ok(Response::new(503, Some("application/soap+xml".into()), reply_envelope(ns::FAULT_ACTION, &id_of(req), &[], BUSY_FAULT).into_bytes())
                .with_retry_after(wait))
        });
        for path in 0..3 {
            let c = SoapClient::new(&fault, url());
            let e = match path {
                0 => c.call(&call(), value).unwrap_err(),
                1 => c.call_ack::<Error>(&call()).unwrap_err(),
                _ => c.call_with_parts(&call(), |_, _| Ok::<_, Error>(())).unwrap_err(),
            };
            let f = e.fault().unwrap();
            assert_eq!((f.retry_after, e.retry_after()), (wait, wait), "path {path}");
            assert!(e.is_fault(SVC, "Busy") && f.has_code(SVC, "PaperOut") && !f.is(SVC, "PaperOut"), "path {path}");
            assert_eq!(f.nested, vec![crate::SubCode::new(Some(SVC), "PaperOut")]);
        }
        // Without a Retry-After there is none, and a success never carries one into the result.
        let plain = Script::new(|req| Ok(Response::new(500, None, reply_envelope(ns::FAULT_ACTION, &id_of(req), &[], BUSY_FAULT).into_bytes())));
        assert_eq!(SoapClient::new(&plain, url()).call(&call(), value).unwrap_err().retry_after(), None);
        let ok = Script::echo();
        assert!(SoapClient::new(&ok, url()).call(&call(), value).is_ok());
    }

    #[test]
    fn a_third_party_transport_reports_failures_with_the_public_constructors() {
        // An in-process transport (no sockets) that fails like a real one at different stages.
        for (kind, stage, submit, query) in [
            (TransportKind::Refused, crate::Stage::Connect, crate::RetrySafety::Safe, crate::RetrySafety::Safe),
            (TransportKind::ConnectTimeout, crate::Stage::Connect, crate::RetrySafety::Safe, crate::RetrySafety::Safe),
            (TransportKind::ConnectionClosed, crate::Stage::Reply, crate::RetrySafety::OutcomeUnknown, crate::RetrySafety::Idempotent),
            (TransportKind::Timeout, crate::Stage::Reply, crate::RetrySafety::OutcomeUnknown, crate::RetrySafety::Idempotent),
        ] {
            let t = Script::new(move |_| Err(Error::transport(kind, "in-process failure")));
            let e = SoapClient::new(&t, url()).call(&call(), value).unwrap_err();
            assert_eq!((e.transport_kind(), e.stage(), e.retry_safety(false), e.retry_safety(true)), (Some(kind), stage, submit, query), "{kind}");
            assert_eq!(t.count(), 1, "the client itself never retries");
        }
        // Cancellation and the deadline are raised before anything is sent: always safe.
        let script = Script::echo();
        let token = CancelToken::new();
        token.cancel();
        let e = SoapClient::new(&script, url()).with_cancel(token).call(&call(), value).unwrap_err();
        assert_eq!((e.stage(), e.retry_safety(false)), (crate::Stage::Prepare, crate::RetrySafety::Safe));
        let e = SoapClient::new(&script, url()).with_deadline(Instant::now()).call(&call(), value).unwrap_err();
        assert_eq!((e.stage(), e.retry_safety(false)), (crate::Stage::Prepare, crate::RetrySafety::Safe));
        assert_eq!(script.count(), 0);
    }
}
