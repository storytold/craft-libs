//! An in-process fake WS-Print printer for tests of apps and of this crate. Cargo feature `testing`
//! (together with `wsd`); not for production use, and it has no effect on default builds.
//!
//! The protocol logic is [`FakePrinter::handle`], [`MemTransport`] is a socket-free
//! [`Transport`] over it, and [`printer_server`] puts it behind the
//! loopback HTTP server of `craft-wsd`'s `testing` feature (127.0.0.1, ephemeral port). It
//! implements just enough of the service to exercise a client: printer elements (description,
//! status, capabilities), create / send / status / cancel / history of jobs, with scripted endings
//! ([`Ending`]) and failures ([`FakeConfig`]). It stores every job's ticket and document
//! ([`FakeJob`]) and logs every request ([`Recorded`]) so a test can assert on what was sent.
//!
//! ```
//! use std::sync::Arc;
//! use craft_print::wsd::WsdBackend;
//! use craft_print::wsd::testing::{FakeConfig, FakePrinter, MemTransport};
//!
//! let printer = FakePrinter::new(FakeConfig::default());
//! let backend = WsdBackend::new().with_transport(MemTransport(Arc::clone(&printer)));
//! let id = backend.add("http://127.0.0.1:5357/print")?;
//! assert_eq!(printer.jobs(), 0);
//! # let _ = id;
//! # Ok::<(), craft_print::PrintError>(())
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::adapter::{DeviceUrl, Request, Response, Transport, WireError, param, xml};
pub use super::adapter::{HttpRequest, Server, raw_response};

pub const SOAP: &str = "http://www.w3.org/2003/05/soap-envelope";
pub const WSA: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing";
pub const WPRT: &str = "http://schemas.microsoft.com/windows/2006/08/wdp/print";
const ACTION_BASE: &str = "http://schemas.microsoft.com/windows/2006/08/wdp/print";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the fake does with a job as it is polled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Pending, Processing, then Completed with the given reason.
    Completes(&'static str),
    /// Aborted with the given reason once the document is in.
    Aborts(&'static str),
    /// Never finishes by itself.
    Hangs,
    /// The job vanishes (JobIdNotFound) at the third poll.
    Vanishes,
}

/// How the fake printer behaves; the default is a colour printer that accepts every job and completes it
/// after two polls.
#[derive(Clone, Debug)]
pub struct FakeConfig {
    pub formats: Vec<&'static str>,
    pub compressions: Vec<&'static str>,
    pub sides: Vec<&'static str>,
    pub max_copies: u32,
    pub color: bool,
    pub accept_jobs: bool,
    pub ending: Ending,
    /// Polls before the job is Processing / terminal.
    pub polls_to_finish: u32,
    /// Answer every request with this HTTP status and an empty body (a misbehaving device).
    pub http_error: Option<u16>,
    /// SendDocument fails with an internal error.
    pub fail_send: bool,
    /// CancelJob on a finished job answers `OperationFailed` (as fakeprint does) instead of `JobIdNotFound`.
    pub cancel_ended_is_operation_failed: bool,
    /// GetJobHistory answers a fault with this sub-code (`prefix:Local`, e.g.
    /// `wsa:ActionNotSupported` or `wprt:ServerErrorTemporaryError`) instead of the history.
    pub history_fault: Option<&'static str>,
    /// Extra finished jobs of other people, older than any real job, listed first in GetJobHistory
    /// (a busy printer's long history).
    pub history_filler: u32,
    /// The id of the first job the printer creates (after the filler's).
    pub first_job_id: u32,
    /// `PrinterState` and its reasons.
    pub state: &'static str,
    pub state_reasons: Vec<&'static str>,
    pub media: Vec<&'static str>,
    /// `PrintQuality` values the printer lists; empty lists none.
    pub qualities: Vec<&'static str>,
    /// Resolutions the printer lists; empty lists none.
    pub resolutions: Vec<(u32, u32)>,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            formats: vec!["application/pdf", "image/pwg-raster"],
            compressions: vec!["None"],
            sides: vec!["OneSided", "TwoSidedLongEdge"],
            max_copies: 10,
            color: true,
            accept_jobs: true,
            ending: Ending::Completes("JobCompletedSuccessfully"),
            polls_to_finish: 2,
            http_error: None,
            fail_send: false,
            cancel_ended_is_operation_failed: false,
            history_fault: None,
            history_filler: 0,
            first_job_id: 1,
            state: "Idle",
            state_reasons: Vec::new(),
            media: vec!["iso_a4_210x297mm"],
            qualities: Vec::new(),
            resolutions: Vec::new(),
        }
    }
}

/// A job the fake printer stores: the ticket and the document as they arrived.
#[derive(Clone, Debug)]
pub struct FakeJob {
    /// `JobName` and `JobOriginatingUserName` of the ticket.
    pub name: String,
    pub user: String,
    pub state: &'static str,
    pub reason: &'static str,
    pub ticket: String,
    pub format: Option<String>,
    pub document: Option<Vec<u8>>,
    pub last_sent: bool,
    polls: u32,
}

/// One request the fake received.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub action: String,
    pub content_type: String,
    pub body_len: usize,
}

#[derive(Default)]
struct State {
    next_id: u32,
    jobs: BTreeMap<u32, FakeJob>,
    log: Vec<Recorded>,
}

/// The fake printer. Share it with [`Arc`]: [`MemTransport`] and [`printer_server`] take a clone.
pub struct FakePrinter {
    pub config: Mutex<FakeConfig>,
    state: Mutex<State>,
}

pub fn envelope(action: &str, relates: &str, body: &str) -> String {
    format!(
        "<s:Envelope xmlns:s=\"{SOAP}\" xmlns:wsa=\"{WSA}\" xmlns:wprt=\"{WPRT}\"><s:Header><wsa:Action>{action}</wsa:Action><wsa:RelatesTo>{relates}</wsa:RelatesTo></s:Header><s:Body>{body}</s:Body></s:Envelope>"
    )
}

pub fn fault(relates: &str, role: &str, sub: &str, reason: &str) -> String {
    envelope(
        "http://schemas.xmlsoap.org/ws/2004/08/addressing/fault",
        relates,
        &format!(
            "<s:Fault><s:Code><s:Value>s:{role}</s:Value><s:Subcode><s:Value>{sub}</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang=\"en\">{reason}</s:Text></s:Reason></s:Fault>"
        ),
    )
}

fn local<'a>(n: roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child_text_deep(n: roxmltree::Node<'_, '_>, name: &str) -> String {
    n.descendants().find(|c| c.is_element() && local(*c) == name).map(|c| c.text().unwrap_or("").trim().to_owned()).unwrap_or_default()
}

fn child_text(n: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    n.children().find(|c| c.is_element() && local(*c) == name).map(|c| c.text().unwrap_or("").trim().to_owned())
}

/// Split a multipart body into (headers, content) parts.
fn multipart(content_type: &str, body: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Some(boundary) = param(content_type, "boundary") else { return Vec::new() };
    let delim = format!("--{boundary}");
    let text = body;
    let mut parts = Vec::new();
    let mut pos = 0;
    let find =
        |hay: &[u8], needle: &[u8], from: usize| hay.get(from..).and_then(|h| h.windows(needle.len()).position(|w| w == needle)).map(|p| p + from);
    while let Some(d) = find(text, delim.as_bytes(), pos) {
        let after = d + delim.len();
        if text.get(after..after + 2) == Some(b"--") {
            break;
        }
        let start = after + 2; // past CRLF
        let Some(next) = find(text, format!("\r\n{delim}").as_bytes(), start) else { break };
        let whole = &text[start..next];
        let split = find(whole, b"\r\n\r\n", 0).unwrap_or(0);
        parts.push((String::from_utf8_lossy(&whole[..split]).into_owned(), whole[(split + 4).min(whole.len())..].to_vec()));
        pos = next + 2;
    }
    parts
}

fn unbase64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter_map(val) {
        acc = (acc << 6) | u32::from(c);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

impl FakePrinter {
    pub fn new(config: FakeConfig) -> Arc<Self> {
        let next_id = config.first_job_id;
        Arc::new(Self { config: Mutex::new(config), state: Mutex::new(State { next_id, ..Default::default() }) })
    }

    pub fn log(&self) -> Vec<Recorded> {
        lock(&self.state).log.clone()
    }

    pub fn actions(&self) -> Vec<String> {
        self.log().into_iter().map(|r| r.action.rsplit('/').next().unwrap_or("").to_owned()).collect()
    }

    pub fn job(&self, id: u32) -> Option<FakeJob> {
        lock(&self.state).jobs.get(&id).cloned()
    }

    pub fn jobs(&self) -> usize {
        lock(&self.state).jobs.len()
    }

    /// Handle one POST: returns (status, content type, body).
    pub fn handle(&self, content_type: &str, body: &[u8]) -> (u16, String, Vec<u8>) {
        let cfg = lock(&self.config).clone();
        if let Some(code) = cfg.http_error {
            return (code, "text/plain".into(), Vec::new());
        }
        // The envelope is the whole body, or the first part of a multipart package.
        let (envelope_bytes, parts) = if content_type.to_ascii_lowercase().starts_with("multipart/related") {
            let parts = multipart(content_type, body);
            (parts.first().map(|p| p.1.clone()).unwrap_or_default(), parts)
        } else {
            (body.to_vec(), Vec::new())
        };
        let text = String::from_utf8_lossy(&envelope_bytes).into_owned();
        let Ok(doc) = roxmltree::Document::parse(&text) else {
            return (400, "text/plain".into(), b"bad xml".to_vec());
        };
        let env = doc.root_element();
        let header = env.children().find(|c| c.is_element() && local(*c) == "Header");
        let action = header
            .and_then(|h| h.children().find(|c| c.is_element() && local(*c) == "Action"))
            .map(|a| a.text().unwrap_or("").trim().to_owned())
            .unwrap_or_default();
        let msg_id = header
            .and_then(|h| h.children().find(|c| c.is_element() && local(*c) == "MessageID"))
            .map(|a| a.text().unwrap_or("").trim().to_owned())
            .unwrap_or_default();
        let op = action.strip_prefix(&format!("{ACTION_BASE}/")).unwrap_or("").to_owned();
        let payload = env.children().find(|c| c.is_element() && local(*c) == "Body").and_then(|b| b.children().find(|c| c.is_element()));
        lock(&self.state).log.push(Recorded { action: action.clone(), content_type: content_type.to_owned(), body_len: body.len() });
        let ok = |name: &str, inner: &str| {
            (
                200u16,
                "application/soap+xml; charset=utf-8".to_owned(),
                envelope(&format!("{ACTION_BASE}/{name}Response"), &msg_id, &format!("<wprt:{name}Response>{inner}</wprt:{name}Response>"))
                    .into_bytes(),
            )
        };
        let flt = |role: &str, sub: &str, reason: &str| {
            (500u16, "application/soap+xml; charset=utf-8".to_owned(), fault(&msg_id, role, sub, reason).into_bytes())
        };
        let Some(payload) = payload else { return flt("Sender", "wprt:InvalidArgs", "empty body") };
        let mut st = lock(&self.state);
        match op.as_str() {
            "GetPrinterElements" => {
                let wanted: Vec<String> = payload
                    .descendants()
                    .filter(|n| n.is_element() && local(*n) == "Name")
                    .map(|n| n.text().unwrap_or("").trim().trim_start_matches("wprt:").to_owned())
                    .collect();
                let mut inner = String::new();
                for w in wanted {
                    inner.push_str(&match w.as_str() {
                        "PrinterDescription" => format!("<wprt:ElementData Name=\"wprt:PrinterDescription\" Valid=\"true\"><wprt:PrinterDescription><wprt:ColorSupported>{}</wprt:ColorSupported><wprt:DeviceId>MFG:Fake;</wprt:DeviceId><wprt:MultipleDocumentJobsSupported>false</wprt:MultipleDocumentJobsSupported><wprt:PagesPerMinute>10</wprt:PagesPerMinute><wprt:PrinterName>Fake Printer</wprt:PrinterName></wprt:PrinterDescription></wprt:ElementData>", cfg.color),
                        "PrinterStatus" => {
                            let reasons: String = cfg.state_reasons.iter().map(|r| format!("<wprt:PrinterStateReason>{r}</wprt:PrinterStateReason>")).collect();
                            let reasons = if reasons.is_empty() { String::new() } else { format!("<wprt:PrinterStateReasons>{reasons}</wprt:PrinterStateReasons>") };
                            format!("<wprt:ElementData Name=\"wprt:PrinterStatus\" Valid=\"true\"><wprt:PrinterStatus><wprt:PrinterCurrentTime>2026-01-01T00:00:00Z</wprt:PrinterCurrentTime><wprt:PrinterState>{}</wprt:PrinterState>{reasons}<wprt:QueuedJobCount>0</wprt:QueuedJobCount></wprt:PrinterStatus></wprt:ElementData>", cfg.state)
                        }
                        "PrinterCapabilities" => {
                            let list = |v: &[&str]| v.iter().map(|x| format!("<wprt:AllowedValue>{x}</wprt:AllowedValue>")).collect::<String>();
                            let mut extra = String::new();
                            if !cfg.qualities.is_empty() {
                                extra.push_str(&format!("<wprt:PrintQuality>{}</wprt:PrintQuality>", list(&cfg.qualities)));
                            }
                            if !cfg.resolutions.is_empty() {
                                let each: String = cfg
                                    .resolutions
                                    .iter()
                                    .map(|(w, h)| format!("<wprt:AllowedValue><wprt:Width>{w}</wprt:Width><wprt:Height>{h}</wprt:Height></wprt:AllowedValue>"))
                                    .collect();
                                extra.push_str(&format!("<wprt:Resolution>{each}</wprt:Resolution>"));
                            }
                            format!("<wprt:ElementData Name=\"wprt:PrinterCapabilities\" Valid=\"true\"><wprt:PrinterCapabilities><wprt:JobValues><wprt:JobProcessing><wprt:Copies><wprt:MaxValue>{}</wprt:MaxValue></wprt:Copies></wprt:JobProcessing><wprt:DocumentProcessing><wprt:MediaSizeName>{}</wprt:MediaSizeName><wprt:Sides>{}</wprt:Sides>{}</wprt:DocumentProcessing></wprt:JobValues><wprt:DocumentValues><wprt:DocumentDescription><wprt:Compression>{}</wprt:Compression><wprt:Format>{}</wprt:Format></wprt:DocumentDescription></wprt:DocumentValues></wprt:PrinterCapabilities></wprt:ElementData>", cfg.max_copies, list(&cfg.media), list(&cfg.sides), extra, list(&cfg.compressions), list(&cfg.formats))
                        }
                        other => format!("<wprt:ElementData Name=\"wprt:{other}\" Valid=\"false\"/>"),
                    });
                }
                ok("GetPrinterElements", &format!("<wprt:PrinterElements>{inner}</wprt:PrinterElements>"))
            }
            "CreatePrintJob" => {
                if !cfg.accept_jobs {
                    return flt("Receiver", "wprt:ServerErrorNotAcceptingJobs", "service mode");
                }
                let Some(ticket) = payload.children().find(|c| c.is_element() && local(*c) == "PrintTicket") else {
                    return flt("Sender", "wprt:InvalidArgs", "no PrintTicket");
                };
                let ticket_text = &text[ticket.range()];
                // MustHonor on a value the printer cannot do: refuse the job.
                if let Some(copies) = ticket.descendants().find(|n| n.is_element() && local(*n) == "Copies")
                    && copies.attributes().any(|a| a.name() == "MustHonor" && a.value() == "true")
                    && copies.text().unwrap_or("0").trim().parse::<u32>().unwrap_or(0) > cfg.max_copies
                {
                    return flt("Sender", "wprt:InvalidArgs", "copies");
                }
                let text_of = |name: &str| child_text_deep(ticket, name);
                let id = st.next_id;
                st.next_id += 1;
                st.jobs.insert(
                    id,
                    FakeJob {
                        name: text_of("JobName"),
                        user: text_of("JobOriginatingUserName"),
                        state: "Pending",
                        reason: "JobIncoming",
                        ticket: ticket_text.to_owned(),
                        format: None,
                        document: None,
                        last_sent: false,
                        polls: 0,
                    },
                );
                ok("CreatePrintJob", &format!("<wprt:JobId>{id}</wprt:JobId>"))
            }
            "SendDocument" if cfg.fail_send => flt("Receiver", "wprt:ServerErrorInternalError", "boom"),
            "SendDocument" => {
                let id: u32 = child_text(payload, "JobId").and_then(|s| s.parse().ok()).unwrap_or(0);
                let Some(desc) = payload.children().find(|c| c.is_element() && local(*c) == "DocumentDescription") else {
                    return flt("Sender", "wprt:InvalidArgs", "no description");
                };
                let format = child_text(desc, "Format").unwrap_or_default();
                let compression = child_text(desc, "Compression").unwrap_or_default();
                let Some(job) = st.jobs.get_mut(&id) else { return flt("Sender", "wprt:ClientErrorJobIdNotFound", "no such job") };
                if job.last_sent {
                    return flt("Sender", "wprt:ClientErrorLastDocumentAlreadySent", "already sent");
                }
                if !cfg.formats.iter().any(|f| *f == format) {
                    return flt("Sender", "wprt:ClientErrorFormatNotSupported", "format");
                }
                if !cfg.compressions.iter().any(|c| *c == compression) {
                    return flt("Sender", "wprt:ClientErrorCompressionNotSupported", "compression");
                }
                let data = payload.children().find(|c| c.is_element() && local(*c) == "DocumentData");
                let bytes = match data.and_then(|d| d.children().find(|c| c.is_element() && local(*c) == "Include")) {
                    Some(inc) => {
                        let cid = inc.attribute("href").unwrap_or("").trim_start_matches("cid:").to_owned();
                        match parts.iter().find(|(h, _)| h.to_ascii_lowercase().contains(&format!("content-id: <{}>", cid.to_ascii_lowercase()))) {
                            Some((_, content)) => content.clone(),
                            None => return flt("Sender", "wprt:InvalidArgs", "xop include not resolved"),
                        }
                    }
                    None => unbase64(&data.map(|d| d.text().unwrap_or("").to_owned()).unwrap_or_default()),
                };
                job.format = Some(format);
                job.document = Some(bytes);
                job.last_sent = child_text(payload, "LastDocument").as_deref() == Some("true");
                if let Ending::Aborts(reason) = cfg.ending {
                    job.state = "Aborted";
                    job.reason = reason;
                }
                ok("SendDocument", "")
            }
            "GetJobElements" => {
                let id: u32 = child_text(payload, "JobId").and_then(|s| s.parse().ok()).unwrap_or(0);
                let Some(job) = st.jobs.get_mut(&id) else { return flt("Sender", "wprt:ClientErrorJobIdNotFound", "no such job") };
                job.polls += 1;
                if cfg.ending == Ending::Vanishes && job.polls >= 3 {
                    st.jobs.remove(&id);
                    return flt("Sender", "wprt:ClientErrorJobIdNotFound", "forgotten");
                }
                if let Ending::Completes(reason) = cfg.ending
                    && job.last_sent
                    && !matches!(job.state, "Completed" | "Canceled" | "Aborted")
                {
                    if job.polls >= cfg.polls_to_finish {
                        job.state = "Completed";
                        job.reason = reason;
                    } else {
                        job.state = "Processing";
                        job.reason = "JobPrinting";
                    }
                }
                let sheets = if job.state == "Completed" { 1 } else { 0 };
                ok(
                    "GetJobElements",
                    &format!(
                        "<wprt:JobElements><wprt:ElementData Name=\"wprt:JobStatus\" Valid=\"true\"><wprt:JobStatus><wprt:JobId>{id}</wprt:JobId><wprt:JobState>{}</wprt:JobState><wprt:JobStateReasons><wprt:JobStateReason>{}</wprt:JobStateReason></wprt:JobStateReasons><wprt:KOctetsProcessed>1</wprt:KOctetsProcessed><wprt:MediaSheetsCompleted>{sheets}</wprt:MediaSheetsCompleted><wprt:NumberOfDocuments>1</wprt:NumberOfDocuments></wprt:JobStatus></wprt:ElementData></wprt:JobElements>",
                        job.state, job.reason
                    ),
                )
            }
            "CancelJob" => {
                let id: u32 = child_text(payload, "JobId").and_then(|s| s.parse().ok()).unwrap_or(0);
                match st.jobs.get_mut(&id) {
                    Some(job) if !matches!(job.state, "Completed" | "Canceled" | "Aborted") => {
                        job.state = "Canceled";
                        job.reason = "JobCanceledByUser";
                        ok("CancelJob", "")
                    }
                    Some(_) if cfg.cancel_ended_is_operation_failed => {
                        flt("Receiver", "wprt:OperationFailed", "the job already ended; there is nothing to cancel")
                    }
                    _ => flt("Sender", "wprt:ClientErrorJobIdNotFound", "nothing to cancel"),
                }
            }
            "GetJobHistory" if cfg.history_fault.is_some() => flt("Receiver", cfg.history_fault.unwrap_or_default(), "the history is not available"),
            "GetActiveJobs" | "GetJobHistory" => {
                let active = op == "GetActiveJobs";
                let filler: String = (0..if active { 0 } else { cfg.history_filler })
                    .map(|i| format!("<wprt:JobSummary><wprt:JobId>{}</wprt:JobId><wprt:JobState>Completed</wprt:JobState><wprt:JobName>old{i}</wprt:JobName><wprt:JobOriginatingUserName>other</wprt:JobOriginatingUserName><wprt:KOctetsProcessed>0</wprt:KOctetsProcessed><wprt:MediaSheetsCompleted>0</wprt:MediaSheetsCompleted><wprt:NumberOfDocuments>1</wprt:NumberOfDocuments></wprt:JobSummary>", i + 1))
                    .collect();
                let live: String = st
                    .jobs
                    .iter()
                    .filter(|(_, j)| active != matches!(j.state, "Completed" | "Canceled" | "Aborted"))
                    .map(|(id, j)| format!("<wprt:JobSummary><wprt:JobId>{id}</wprt:JobId><wprt:JobState>{}</wprt:JobState><wprt:JobName>{}</wprt:JobName><wprt:JobOriginatingUserName>{}</wprt:JobOriginatingUserName><wprt:KOctetsProcessed>0</wprt:KOctetsProcessed><wprt:MediaSheetsCompleted>0</wprt:MediaSheetsCompleted><wprt:NumberOfDocuments>1</wprt:NumberOfDocuments></wprt:JobSummary>", j.state, xml::esc(&j.name), xml::esc(&j.user)))
                    .collect();
                let rows = filler + &live;
                let wrap = if active { "ActiveJobs" } else { "JobHistory" };
                ok(&op, &format!("<wprt:{wrap}>{rows}</wprt:{wrap}>"))
            }
            _ => flt("Sender", "wsa:ActionNotSupported", "unknown action"),
        }
    }
}

/// A socket-free transport straight into the fake.
pub struct MemTransport(pub Arc<FakePrinter>);

impl Transport for MemTransport {
    fn post(&self, _url: &DeviceUrl, request: Request) -> Result<Response, WireError> {
        let (status, content_type, body) = self.0.handle(&request.content_type, &request.body);
        Ok(Response::new(status, Some(content_type), body))
    }
}

/// A loopback server on an ephemeral port that plays `printer`.
pub fn printer_server(printer: Arc<FakePrinter>) -> Server {
    Server::start(move |req| {
        let (status, ct, body) = printer.handle(req.header("content-type").unwrap_or(""), &req.body);
        raw_response(status, &ct, &body)
    })
}
