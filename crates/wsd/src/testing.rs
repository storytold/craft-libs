//! Test support (cargo feature `testing`): a tiny loopback HTTP server.
//!
//! `Server` listens on 127.0.0.1 on an ephemeral port and answers each request with the raw bytes a
//! closure returns, which lets a test play a well-behaved device, a slow one, or a hostile one.
//! Test code only: it unwraps freely and is not part of the default build.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One request as the server read it.
pub struct HttpRequest {
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// The value of the first header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A server on 127.0.0.1 on an ephemeral port; `respond` returns the raw bytes to write back.
pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub hits: Arc<Mutex<u32>>,
}

impl Server {
    /// Bind 127.0.0.1 on an ephemeral port and serve connections one at a time on a background thread: read one
    /// request, write what `respond` returns, close. Panics if the port cannot be bound.
    pub fn start(respond: impl Fn(&HttpRequest) -> Vec<u8> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let hits = Arc::new(Mutex::new(0));
        let (s, h) = (stop.clone(), hits.clone());
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                *h.lock().unwrap() += 1;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                if let Some(req) = read_request(&mut stream) {
                    let reply = respond(&req);
                    let _ = stream.write_all(&reply);
                    let _ = stream.flush();
                }
            }
        });
        Self { port, stop, thread: Some(thread), hits }
    }

    /// `http://127.0.0.1:{port}{path}`.
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

impl Drop for Server {
    /// Stop accepting (a connection of our own wakes the accept loop) and join the server thread.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A complete HTTP/1.1 response with this status, `Content-Type`, a matching `Content-Length` and
/// `Connection: close`.
pub fn raw_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out =
        format!("HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

/// Read one request: the headers, then up to `Content-Length` bytes of body. `None` if the client
/// closes or errors before the headers end.
fn read_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let headers: Vec<(String, String)> =
        head.lines().skip(1).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(HttpRequest { headers, body })
}

// ---- an in-memory WSD device -------------------------------------------------------------

pub use device::{FakeDevice, Logged};

/// A WSD device that lives in memory: it answers WS-Transfer `Get`, WS-MetadataExchange
/// `GetMetadata` and WS-Eventing on its service endpoint, and any other operation in the service
/// namespace with a canned `<Op>Response`. One implementation shaped like a printer or a scanner,
/// chosen by [`ServiceKind`](crate::ServiceKind), so the same core code can be driven against both. It is a
/// [`Transport`](crate::Transport), so no socket is involved.
pub mod device {
    use std::sync::Mutex;

    use crate::addressing::{EndpointRef, ReferenceParameter};
    use crate::envelope::new_uuid;
    use crate::error::{Error, Result, TransportKind};
    use crate::eventing::{self, Expiry};
    use crate::metadata::{DIALECT_RELATIONSHIP, DIALECT_THIS_DEVICE, DIALECT_THIS_MODEL, RELATIONSHIP_HOST};
    use crate::policy::DeviceUrl;
    use crate::service::{ServiceDescriptor, ServiceKind};
    use crate::soap::reply_envelope;
    use crate::transport::{Request, Response, Transport};
    use crate::{ns, xml};

    /// One request the device received.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Logged {
        pub path: String,
        pub action: String,
        /// The `wse:Identifier` header, if the request had one.
        pub identifier: Option<String>,
    }

    struct Sub {
        id: String,
        notify_to: EndpointRef,
        end_to: Option<EndpointRef>,
    }

    #[derive(Default)]
    struct State {
        subs: Vec<Sub>,
        log: Vec<Logged>,
        next: u32,
    }

    /// See the [module](self) docs.
    pub struct FakeDevice {
        kind: ServiceKind,
        host: String,
        port: u16,
        state: Mutex<State>,
    }

    impl FakeDevice {
        pub const DEVICE_PATH: &'static str = "/device";
        pub const SERVICE_PATH: &'static str = "/service";

        /// A device of this kind "at" `127.0.0.1:port` (nothing listens there; it is a label).
        pub fn new(kind: ServiceKind, port: u16) -> Self {
            Self { kind, host: "127.0.0.1".into(), port, state: Mutex::new(State::default()) }
        }

        /// The descriptor of the kind this device plays.
        fn descriptor(&self) -> &'static ServiceDescriptor {
            self.kind.descriptor()
        }

        /// The device endpoint (`/device`), which answers metadata requests only.
        pub fn device_url(&self) -> DeviceUrl {
            DeviceUrl::parse(&format!("http://{}:{}{}", self.host, self.port, Self::DEVICE_PATH)).unwrap()
        }

        /// The hosted-service endpoint (`/service`), which answers metadata and eventing requests.
        pub fn service_url(&self) -> DeviceUrl {
            DeviceUrl::parse(&format!("http://{}:{}{}", self.host, self.port, Self::SERVICE_PATH)).unwrap()
        }

        /// Requests received so far.
        pub fn log(&self) -> Vec<Logged> {
            self.state.lock().unwrap().log.clone()
        }

        /// Live subscriptions: `(identifier, NotifyTo)`.
        pub fn subscriptions(&self) -> Vec<(String, EndpointRef)> {
            self.state.lock().unwrap().subs.iter().map(|s| (s.id.clone(), s.notify_to.clone())).collect()
        }

        /// The metadata document this device serves.
        pub fn metadata_xml(&self) -> String {
            let d = self.descriptor();
            let svc = self.service_url().as_url();
            format!(
                "<wsx:Metadata xmlns:wsx=\"{wsx}\" xmlns:wsdp=\"{wsdp}\" xmlns:wsa=\"{wsa}\" xmlns:{p}=\"{sns}\">\
<wsx:MetadataSection Dialect=\"{DIALECT_THIS_MODEL}\"><wsdp:ThisModel><wsdp:Manufacturer>Example Works</wsdp:Manufacturer><wsdp:ModelName>Fake {name}</wsdp:ModelName></wsdp:ThisModel></wsx:MetadataSection>\
<wsx:MetadataSection Dialect=\"{DIALECT_THIS_DEVICE}\"><wsdp:ThisDevice><wsdp:FriendlyName>Fake {name} device</wsdp:FriendlyName></wsdp:ThisDevice></wsx:MetadataSection>\
<wsx:MetadataSection Dialect=\"{DIALECT_RELATIONSHIP}\"><wsdp:Relationship Type=\"{RELATIONSHIP_HOST}\">\
<wsdp:Host><wsa:EndpointReference><wsa:Address>urn:uuid:00000000-0000-4000-8000-000000000001</wsa:Address></wsa:EndpointReference><wsdp:Types>wsdp:Device {p}:{dev}</wsdp:Types><wsdp:ServiceId>uri:fake-host</wsdp:ServiceId></wsdp:Host>\
<wsdp:Hosted><wsa:EndpointReference><wsa:Address>{svc}</wsa:Address></wsa:EndpointReference><wsdp:Types>{p}:{svct}</wsdp:Types><wsdp:ServiceId>uri:fake-{name}</wsdp:ServiceId></wsdp:Hosted>\
</wsdp:Relationship></wsx:MetadataSection></wsx:Metadata>",
                wsx = ns::WSX,
                wsdp = ns::WSDP,
                wsa = ns::WSA,
                p = d.prefix,
                sns = d.namespace,
                name = d.name,
                dev = d.device_type,
                svct = d.service_type,
            )
        }

        /// Event notification envelopes, one per subscription, as `(NotifyTo, bytes)`: the NotifyTo
        /// reference parameters are carried as headers, as the protocol says.
        pub fn notify(&self, action: &str, payload: &str) -> Vec<(EndpointRef, Vec<u8>)> {
            let d = self.descriptor();
            self.state
                .lock()
                .unwrap()
                .subs
                .iter()
                .map(|s| {
                    let text = format!(
                        "<s:Envelope xmlns:s=\"{}\" xmlns:wsa=\"{}\" xmlns:{}=\"{}\"><s:Header><wsa:To>{}</wsa:To><wsa:Action>{}</wsa:Action><wsa:MessageID>urn:uuid:{}</wsa:MessageID>{}</s:Header><s:Body>{payload}</s:Body></s:Envelope>",
                        ns::SOAP,
                        ns::WSA,
                        d.prefix,
                        d.namespace,
                        xml::esc(&s.notify_to.address),
                        xml::esc(action),
                        new_uuid(),
                        s.notify_to.header_xml()
                    );
                    (s.notify_to.clone(), text.into_bytes())
                })
                .collect()
        }

        /// End every subscription that gave an `EndTo`, with the given status URI; returns the
        /// `SubscriptionEnd` messages as `(EndTo, bytes)`. All subscriptions are removed.
        pub fn end_all(&self, status: &str, reason: &str) -> Vec<(EndpointRef, Vec<u8>)> {
            let svc = self.service_url().as_url();
            let subs = std::mem::take(&mut self.state.lock().unwrap().subs);
            subs.into_iter()
                .filter_map(|s| {
                    let end_to = s.end_to?;
                    let manager = manager_epr(&svc, &s.id);
                    let text = format!(
                        "<s:Envelope xmlns:s=\"{}\" xmlns:wsa=\"{}\" xmlns:wse=\"{}\"><s:Header><wsa:To>{}</wsa:To><wsa:Action>{}</wsa:Action><wsa:MessageID>urn:uuid:{}</wsa:MessageID></s:Header><s:Body><wse:SubscriptionEnd>{}<wse:Status>{}</wse:Status><wse:Reason>{}</wse:Reason></wse:SubscriptionEnd></s:Body></s:Envelope>",
                        ns::SOAP,
                        ns::WSA,
                        ns::WSE,
                        xml::esc(&end_to.address),
                        eventing::ACTION_SUBSCRIPTION_END,
                        new_uuid(),
                        manager.to_xml("wse:SubscriptionManager"),
                        xml::esc(status),
                        xml::esc(reason)
                    );
                    Some((end_to, text.into_bytes()))
                })
                .collect()
        }

        /// Log the request and answer it as the device would: metadata on either path, eventing on the
        /// service path, and a sender fault for anything else.
        fn handle(&self, path: &str, request: &Request) -> Result<Response> {
            let text = std::str::from_utf8(&request.body)
                .map_err(|_| Error::transport(TransportKind::InvalidResponse, "the fake device got a non-text body"))?;
            let doc = xml::parse(text)?;
            let env = doc.root_element();
            let header = xml::child(env, ns::SOAP, "Header").ok_or_else(|| Error::Protocol("no Header".into()))?;
            let body = xml::child(env, ns::SOAP, "Body").ok_or_else(|| Error::Protocol("no Body".into()))?;
            let message_id = crate::envelope::addressing(header, "MessageID").unwrap_or_default().trim_start_matches("urn:uuid:").to_owned();
            let action = crate::envelope::addressing(header, "Action").unwrap_or_default();
            let identifier = xml::child_text(header, ns::WSE, "Identifier");
            let payload = body.children().find(|c| c.is_element());
            let mut st = self.state.lock().unwrap();
            st.log.push(Logged { path: path.to_owned(), action: action.clone(), identifier: identifier.clone() });
            let ok = |action: &str, ns_decls: &[(&str, &str)], body: &str| Ok(soap_ok(reply_envelope(action, &message_id, ns_decls, body)));
            let fault = |sub_ns: &str, sub_local: &str, reason: &str| {
                let body = format!(
                    "<s:Fault><s:Code><s:Value>s:Sender</s:Value><s:Subcode><s:Value xmlns:f=\"{sub_ns}\">f:{sub_local}</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang=\"en\">{reason}</s:Text></s:Reason></s:Fault>"
                );
                Ok(Response {
                    retry_after: None,
                    status: 500,
                    content_type: Some("application/soap+xml".into()),
                    body: reply_envelope(ns::FAULT_ACTION, &message_id, &[], &body).into_bytes(),
                })
            };
            let d = self.descriptor();
            match action.as_str() {
                crate::metadata::ACTION_TRANSFER_GET => ok(crate::metadata::ACTION_TRANSFER_GET_RESPONSE, &[], &self.metadata_xml()),
                crate::metadata::ACTION_GET_METADATA => ok(crate::metadata::ACTION_GET_METADATA_RESPONSE, &[], &self.metadata_xml()),
                _ if path != Self::SERVICE_PATH => fault(ns::WSA, "ActionNotSupported", "not served here"),
                eventing::ACTION_SUBSCRIBE => {
                    let sub = payload.ok_or_else(|| Error::Protocol("empty Subscribe".into()))?;
                    let notify = xml::child(sub, ns::WSE, "Delivery")
                        .and_then(|dl| xml::child(dl, ns::WSE, "NotifyTo"))
                        .map(EndpointRef::parse)
                        .transpose()?;
                    let Some(notify_to) = notify else { return fault(ns::WSE, "InvalidMessage", "no NotifyTo") };
                    let end_to = xml::child(sub, ns::WSE, "EndTo").map(EndpointRef::parse).transpose()?;
                    let requested = xml::child_text(sub, ns::WSE, "Expires").and_then(|t| Expiry::parse(&t));
                    st.next += 1;
                    let id = format!("uuid:fake-sub-{}", st.next);
                    // The device grants at most an hour, and always in duration form.
                    let granted = match requested {
                        Some(Expiry::After(d)) => d.min(std::time::Duration::from_secs(3600)),
                        _ => std::time::Duration::from_secs(3600),
                    };
                    st.subs.push(Sub { id: id.clone(), notify_to, end_to });
                    let mgr = manager_epr(&self.service_url().as_url(), &id).to_xml("wse:SubscriptionManager");
                    ok(
                        eventing::ACTION_SUBSCRIBE_RESPONSE,
                        &[("wse", ns::WSE)],
                        &format!(
                            "<wse:SubscribeResponse>{mgr}<wse:Expires>{}</wse:Expires></wse:SubscribeResponse>",
                            Expiry::After(granted).to_text()
                        ),
                    )
                }
                eventing::ACTION_RENEW | eventing::ACTION_GET_STATUS | eventing::ACTION_UNSUBSCRIBE => {
                    let known = identifier.as_deref().is_some_and(|i| st.subs.iter().any(|s| s.id == i));
                    if !known {
                        return fault(ns::WSE, "UnableToRenew", "unknown subscription");
                    }
                    match action.as_str() {
                        eventing::ACTION_RENEW => ok(
                            eventing::ACTION_RENEW_RESPONSE,
                            &[("wse", ns::WSE)],
                            "<wse:RenewResponse><wse:Expires>PT3600S</wse:Expires></wse:RenewResponse>",
                        ),
                        eventing::ACTION_GET_STATUS => ok(
                            eventing::ACTION_GET_STATUS_RESPONSE,
                            &[("wse", ns::WSE)],
                            "<wse:GetStatusResponse><wse:Expires>PT1800S</wse:Expires></wse:GetStatusResponse>",
                        ),
                        _ => {
                            st.subs.retain(|s| Some(s.id.as_str()) != identifier.as_deref());
                            ok(eventing::ACTION_UNSUBSCRIBE_RESPONSE, &[], "")
                        }
                    }
                }
                a if a.starts_with(d.namespace) => {
                    let op = payload.map(|p| p.tag_name().name().to_owned()).unwrap_or_default();
                    let op = op.strip_suffix("Request").unwrap_or(&op).to_owned();
                    ok(
                        &d.response_action(&op),
                        &[(d.prefix, d.namespace)],
                        &format!("<{p}:{op}Response><{p}:Handled>{name}</{p}:Handled></{p}:{op}Response>", p = d.prefix, name = d.name),
                    )
                }
                _ => fault(ns::WSA, "ActionNotSupported", "unknown action"),
            }
        }
    }

    /// The subscription manager reference: `service_url` with the subscription `id` as `wse:Identifier`.
    fn manager_epr(service_url: &str, id: &str) -> EndpointRef {
        EndpointRef::new(service_url).with_parameter(ReferenceParameter::new(ns::WSE, "Identifier", id))
    }

    /// A 200 reply carrying the SOAP envelope `body`.
    fn soap_ok(body: String) -> Response {
        Response { retry_after: None, status: 200, content_type: Some("application/soap+xml".into()), body: body.into_bytes() }
    }

    impl Transport for FakeDevice {
        /// Answer requests to this device's host and port in memory; any other address is refused as if
        /// nothing listened there. No network is used.
        fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response> {
            if url.host().to_string() != self.host || url.port() != self.port {
                return Err(Error::transport(TransportKind::Refused, "connection refused (this is not the fake device's address)"));
            }
            self.handle(url.path(), &request)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_answers_and_counts() {
        let server =
            Server::start(|req| raw_response(201, "text/plain", format!("{}:{}", req.header("X-Test").unwrap_or("-"), req.body.len()).as_bytes()));
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        s.write_all(b"POST /x HTTP/1.1\r\nX-Test: yes\r\nContent-Length: 3\r\n\r\nabc").unwrap();
        let mut reply = String::new();
        s.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 201") && reply.ends_with("yes:3"), "{reply}");
        assert_eq!(*server.hits.lock().unwrap(), 1);
        assert_eq!(server.url("/p"), format!("http://127.0.0.1:{}/p", server.port));
    }
}
