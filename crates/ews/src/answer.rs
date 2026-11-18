//! Reading an answer: the SOAP envelope, a fault, and the response messages of an operation with
//! their classes and codes.
//!
//! The answer is read with `craft-xml` into a tree whose names carry fixed prefixes whatever the
//! document used: `s:` for the envelope ([`SOAP`]), `m:` for the messages ([`MESSAGES`]), `t:` for
//! the types ([`TYPES`]), `e:` for the errors ([`ERRORS`]); a name in any other namespace starts
//! with `?:`. Elements are found by namespace and local name, never by the document's prefix;
//! everything else in an answer (an element not asked about, a header, a remark) is skipped.

use std::fmt;

use craft_xml::{Canonical, Doctype, El, Limits, Local, Options, UnknownEntity};

use crate::codes::{DEFAULT_BACK_OFF_MS, MAX_BACK_OFF_MS, MIN_BACK_OFF_MS, ResponseClass, ResponseCode};
use crate::soap::{ERRORS, MESSAGES, SOAP, TYPES};

/// Deepest the elements of an answer may nest.
pub const MAX_DEPTH: usize = 40;
/// Most nodes (elements and stretches of text) in one answer.
pub const MAX_NODES: usize = 100_000;
/// Most bytes of an answer: a `GetItem` answer carries whole messages in base64.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

const NAMESPACES: [(&str, &str); 4] = [("s", SOAP), ("m", MESSAGES), ("t", TYPES), ("e", ERRORS)];
const CANONICAL: Canonical<'static> = Canonical::new(&NAMESPACES);

/// Why an answer says nothing an operation can use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not XML, or XML past the limits, or with a document type declaration.
    Malformed,
    /// XML, but not the envelope or the answer that was asked for.
    Unexpected,
    /// The server is busy: ask again after this many milliseconds.
    Busy { back_off_ms: u32 },
    /// The server said no, with this code.
    Server(ResponseCode),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Malformed => f.write_str("the answer is not well-formed XML within the limits"),
            Error::Unexpected => f.write_str("the answer is not the one asked for"),
            Error::Busy { back_off_ms } => write!(f, "the server is busy: ask again in {back_off_ms} ms"),
            Error::Server(code) => write!(f, "the server answered {code}"),
        }
    }
}

impl std::error::Error for Error {}

/// Read `bytes` into a tree with the fixed prefixes (see the module's documentation). No document
/// type declaration is read, and a reference to an entity XML does not define is left out.
pub fn parse_xml(bytes: &[u8]) -> Result<El, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::Malformed);
    }
    let limits = Limits { max_input: MAX_BYTES, max_depth: MAX_DEPTH, max_nodes: MAX_NODES, ..Limits::DEFAULT };
    let options =
        Options { doctype: Doctype::Reject, unknown_entity: UnknownEntity::Drop, ..Options::canonical(&CANONICAL).strict().with_limits(limits) };
    options.parse(bytes).map_err(|_| Error::Malformed)
}

/// The body of a SOAP answer that was not a fault.
pub struct Answer {
    body: El,
}

impl fmt::Debug for Answer {
    /// Nothing of the answer: it may hold someone's mail.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Answer(..)")
    }
}

/// One response message of an operation's answer.
pub struct ResponseMessage<'a> {
    class: ResponseClass,
    code: ResponseCode,
    /// The server says it is busy (`ErrorServerBusy`, or inside `ErrorInternalServerError`).
    busy: bool,
    back_off_ms: Option<u32>,
    el: &'a El,
}

impl fmt::Debug for ResponseMessage<'_> {
    /// The class and the code: the rest is the server's.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseMessage").field("class", &self.class).field("code", &self.code).finish()
    }
}

impl Answer {
    /// Reads the document `bytes`: a SOAP envelope whose body is not a fault. A fault is the
    /// error it names.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let root = parse_xml(bytes)?;
        if root.name != "s:Envelope" {
            return Err(Error::Unexpected);
        }
        let body = root.child("s:Body").ok_or(Error::Unexpected)?;
        if let Some(fault) = body.child("s:Fault") {
            return Err(read_fault(fault));
        }
        Ok(Self { body: body.clone() })
    }

    /// The `s:Body` element.
    pub fn body(&self) -> &El {
        &self.body
    }

    /// The busy error, when any response message of the answer says the server is busy: it is
    /// the whole request that is to be made again, whatever the operation, and a busy server
    /// answers one message however many questions there were.
    pub fn busy(&self) -> Option<Error> {
        let response = self.body.elements().next()?;
        let all = response.child("m:ResponseMessages")?;
        all.elements().find_map(|child| ResponseMessage::read(child).ok()?.ok().err().filter(|error| matches!(error, Error::Busy { .. })))
    }

    /// The response messages of the answer to `operation` (`GetFolder`: the
    /// `GetFolderResponseMessage`s inside `GetFolderResponse`), in order.
    pub fn messages(&self, operation: &str) -> Result<Vec<ResponseMessage<'_>>, Error> {
        let response = self.body.child(format!("m:{operation}Response").as_str()).ok_or(Error::Unexpected)?;
        let all = response.child("m:ResponseMessages").ok_or(Error::Unexpected)?;
        let name = format!("m:{operation}ResponseMessage");
        all.elements().filter(|child| child.name == name).map(ResponseMessage::read).collect()
    }
}

impl<'a> ResponseMessage<'a> {
    fn read(el: &'a El) -> Result<Self, Error> {
        let class = el.attr("ResponseClass").and_then(ResponseClass::from_name).ok_or(Error::Unexpected)?;
        let code = el.child("m:ResponseCode").map(|code| ResponseCode::from_name(&code.text())).ok_or(Error::Unexpected)?;
        // (`MessageXml` and its values are told by their local names: a server puts them in the
        // messages' namespace or the types'.)
        let wait = el.child(Local("BackOffMilliseconds")).map(|wait| wait.text());
        let (busy, back_off_ms) = busy_of(&code, el.child(Local("MessageXml")), wait.as_deref());
        Ok(Self { class, code, busy, back_off_ms, el })
    }

    /// How the server says it went.
    pub fn class(&self) -> ResponseClass {
        self.class
    }

    /// The code the server gave.
    pub fn code(&self) -> &ResponseCode {
        &self.code
    }

    /// The element, for whoever reads what it holds.
    pub fn el(&self) -> &'a El {
        self.el
    }

    /// The element when the message says it was done; the error it names when it was not.
    pub fn ok(&self) -> Result<&'a El, Error> {
        if self.class != ResponseClass::Error {
            return Ok(self.el);
        }
        Err(if self.busy { Error::Busy { back_off_ms: self.back_off_ms.unwrap_or(DEFAULT_BACK_OFF_MS) } } else { Error::Server(self.code.clone()) })
    }
}

/// The text of the `Value` called `name` inside a `MessageXml`.
fn value_of(message_xml: &El, name: &str) -> Option<String> {
    message_xml.children(Local("Value")).find(|value| value.attr("Name") == Some(name)).map(|value| value.text().trim().to_string())
}

/// Whether a message with this `code` and `MessageXml` says the server is busy, and the wait
/// it asks for. Busy is `ErrorServerBusy`, and also `ErrorInternalServerError` whose inner
/// error is `ErrorServerBusy` (Exchange's throttling says so in a 200 answer). The wait is a
/// `BackOffMilliseconds` of its own or a `Value` of that name.
fn busy_of(code: &ResponseCode, message_xml: Option<&El>, direct: Option<&str>) -> (bool, Option<u32>) {
    let inner = message_xml.and_then(|xml| value_of(xml, "InnerErrorResponseCode")).map(|c| ResponseCode::from_name(&c));
    let busy = *code == ResponseCode::ServerBusy || (*code == ResponseCode::InternalServerError && inner == Some(ResponseCode::ServerBusy));
    let wait = direct.map(back_off_ms).or_else(|| message_xml.and_then(|xml| value_of(xml, "BackOffMilliseconds")).map(|w| back_off_ms(&w)));
    (busy, wait)
}

/// A wait in milliseconds, as a server writes it: digits, at least [`MIN_BACK_OFF_MS`] and at
/// most [`MAX_BACK_OFF_MS`]; anything else is the default.
pub fn back_off_ms(text: &str) -> u32 {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return DEFAULT_BACK_OFF_MS;
    }
    // (A number of any length: too many digits is simply the most.)
    let ms = text.parse::<u64>().unwrap_or(u64::MAX);
    u32::try_from(ms.clamp(u64::from(MIN_BACK_OFF_MS), u64::from(MAX_BACK_OFF_MS))).unwrap_or(MAX_BACK_OFF_MS)
}

/// The error a `Fault` names: the response code of its `detail`, and for a busy server the wait.
/// (`ResponseCode` and `MessageXml` are told by their local names, and `Value` too: the service
/// writes `MessageXml` in the types' namespace as often as the errors'.)
fn read_fault(fault: &El) -> Error {
    let detail = fault.child(Local("detail"));
    let code = detail
        .and_then(|detail| detail.child(Local("ResponseCode")))
        .map_or_else(|| ResponseCode::Other(String::new()), |code| ResponseCode::from_name(&code.text()));
    let (busy, wait) = busy_of(&code, detail.and_then(|detail| detail.child(Local("MessageXml"))), None);
    if busy { Error::Busy { back_off_ms: wait.unwrap_or(DEFAULT_BACK_OFF_MS) } } else { Error::Server(code) }
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
