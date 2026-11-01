//! [`Client`]: one method per IPP operation over any [`Transport`]. It builds the request
//! messages (RFC 8011 section 4.2), sends them as `POST application/ipp`, and reads the replies:
//! a non-success status becomes [`IppError::Status`], a success that ignored attributes carries
//! them as warnings.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use super::adapter::{NetworkLimits, Request, Transport};
use super::codec::{Attribute, Group, GroupTag, Limits, Message, Value};
use super::error::{IppError, Result};
use super::model::{JobAttrs, PrinterAttrs};
use super::policy::PrinterUri;
use super::protocol::{Operation, StatusCode};

/// The slowest upload the time limit allows for, in bytes a second. The limit covers the whole
/// request, body included, so a large document (`Print-Job`, `Send-Document`) over a slow link
/// must not use up the budget meant for the printer's reply: that would end as an outcome-unknown
/// timeout with the job possibly half made. The same floor as the WS-Print client's
/// (`craft_wsd::SoapClient`).
const UPLOAD_FLOOR_BYTES_PER_SEC: u64 = 64 * 1024;

/// `timeout` for the exchange plus the time to send `body_len` bytes at the upload floor.
pub(super) fn with_upload_allowance(timeout: Duration, body_len: usize) -> Duration {
    timeout.saturating_add(Duration::from_secs(body_len as u64 / UPLOAD_FLOOR_BYTES_PER_SEC))
}

/// The media type of every request.
pub const CONTENT_TYPE: &str = "application/ipp";

/// The IPP version a client speaks. Ordered: 1.1 is lower than 2.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IppVersion {
    V1_1,
    V2_0,
}

impl IppVersion {
    /// The (major, minor) pair written in a request header.
    pub const fn parts(self) -> (u8, u8) {
        match self {
            Self::V1_1 => (1, 1),
            Self::V2_0 => (2, 0),
        }
    }

    /// The version spelled as `ipp-versions-supported` spells it (`"2.0"`); `None` for any other
    /// version (1.0, 2.1, 2.2: this client speaks 1.1 and 2.0 only).
    pub fn from_keyword(text: &str) -> Option<Self> {
        match text.trim() {
            "1.1" => Some(Self::V1_1),
            "2.0" => Some(Self::V2_0),
            _ => None,
        }
    }

    /// The highest version this client speaks that a printer's `ipp-versions-supported` lists
    /// (2.0, else 1.1); `None` when the list names neither (or is empty).
    pub fn highest_common(advertised: &[String]) -> Option<Self> {
        advertised.iter().filter_map(|v| Self::from_keyword(v)).max()
    }
}

/// The IPP version each printer turned out to speak: shared by the clients of one backend, keyed by
/// the printer's canonical URI, and only ever lowered (a printer that rejected 2.0 is not asked 2.0
/// again for the life of the memory).
#[derive(Debug, Default)]
pub struct VersionMemory {
    versions: RwLock<HashMap<String, IppVersion>>,
}

impl VersionMemory {
    /// The version learned for a printer, if any.
    pub fn get(&self, printer: &str) -> Option<IppVersion> {
        self.versions.read().unwrap_or_else(PoisonError::into_inner).get(printer).copied()
    }

    /// Note that a printer speaks at most `version`; a version already known to be lower stays.
    pub fn lower(&self, printer: &str, version: IppVersion) {
        let mut versions = self.versions.write().unwrap_or_else(PoisonError::into_inner);
        let entry = versions.entry(printer.to_owned()).or_insert(version);
        *entry = (*entry).min(version);
    }
}

/// The printer attributes asked for by [`Client::get_printer_attributes`] when the caller names none.
pub const PRINTER_ATTRIBUTES: &[&str] = &[
    "printer-name",
    "printer-info",
    "printer-make-and-model",
    "printer-state",
    "printer-state-reasons",
    "printer-is-accepting-jobs",
    "operations-supported",
    "document-format-supported",
    "pwg-raster-document-type-supported",
    "sides-supported",
    "print-color-mode-supported",
    "color-supported",
    "media-supported",
    "media-size-supported",
    "copies-supported",
    "printer-resolution-supported",
    "print-quality-supported",
    "print-content-optimize-supported",
    "print-scaling-supported",
    "print-scaling-default",
    "orientation-requested-supported",
    "multiple-operation-time-out",
    "printer-up-time",
    "ipp-versions-supported",
];

/// The job attributes asked for by [`Client::get_job_attributes`] and [`Client::get_jobs`].
pub const JOB_ATTRIBUTES: &[&str] = &[
    "job-id",
    "job-uri",
    "job-state",
    "job-state-reasons",
    "job-name",
    "job-originating-user-name",
    "time-at-creation",
    "date-time-at-creation",
    "job-impressions",
    "job-impressions-completed",
    "job-media-sheets",
    "job-media-sheets-completed",
];

/// `which-jobs` of `Get-Jobs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    NotCompleted,
    Completed,
}

impl Which {
    /// The `which-jobs` keyword Get-Jobs sends.
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::NotCompleted => "not-completed",
            Self::Completed => "completed",
        }
    }
}

/// What a job request carries beyond the printer and the user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobRequest {
    pub job_name: String,
    pub document_name: Option<String>,
    /// `document-format`, a MIME type.
    pub document_format: Option<String>,
    /// `ipp-attribute-fidelity`: refuse the job when an attribute cannot be honoured.
    pub fidelity: bool,
    /// The job template attributes (the job group).
    pub attributes: Vec<Attribute>,
}

/// A reply that carried out the operation.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub message: Message,
    pub status: StatusCode,
    /// The attributes the printer said it ignored or substituted, as sentences.
    pub warnings: Vec<String>,
}

impl Reply {
    /// The first group of this kind.
    pub fn group(&self, tag: GroupTag) -> Option<&Group> {
        self.message.group(tag)
    }
}

/// A job the printer accepted.
#[derive(Clone, Debug, PartialEq)]
pub struct Accepted {
    pub job_id: i32,
    pub job: JobAttrs,
    pub warnings: Vec<String>,
    pub status: StatusCode,
}

/// An IPP client for one printer. Cheap to build; it opens no connection until a method is called.
pub struct Client<T> {
    transport: T,
    uri: PrinterUri,
    limits: NetworkLimits,
    codec: Limits,
    user: String,
    version: IppVersion,
    fallback: bool,
    memory: Option<Arc<VersionMemory>>,
    next_id: Arc<AtomicU32>,
}

impl<T: Transport> Client<T> {
    /// A client for the printer at `uri`: IPP 1.1 without fallback, user `craft`, default limits.
    /// Nothing is sent until an operation is called.
    pub fn new(transport: T, uri: PrinterUri) -> Self {
        Self {
            transport,
            uri,
            limits: NetworkLimits::default(),
            codec: Limits::default(),
            user: "craft".to_owned(),
            version: IppVersion::V1_1,
            fallback: false,
            memory: None,
            next_id: Arc::new(AtomicU32::new(1)),
        }
    }

    /// Use `limits` for the timeout and the request and reply size caps.
    #[must_use]
    pub fn with_limits(mut self, limits: NetworkLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Send `user` as `requesting-user-name`.
    #[must_use]
    pub fn with_user(mut self, user: &str) -> Self {
        self.user = user.to_owned();
        self
    }

    /// Send requests as `version` first.
    #[must_use]
    pub fn with_version(mut self, version: IppVersion) -> Self {
        self.version = version;
        self
    }

    /// Allow one downgrade to IPP 1.1 when the printer answers `server-error-version-not-supported`
    /// to a higher version. A version-rejected request creates nothing (the printer refuses it
    /// before acting), so repeating it at 1.1 is safe even for `Print-Job`, `Create-Job` and
    /// `Send-Document`. At most one extra attempt per request; off by default.
    #[must_use]
    pub fn with_fallback(mut self, fallback: bool) -> Self {
        self.fallback = fallback;
        self
    }

    /// Share what is learned about printers' versions (see [`VersionMemory`]). With fallback on, a
    /// version learned for this printer lowers the version of every later request.
    #[must_use]
    pub fn with_memory(mut self, memory: Arc<VersionMemory>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// The version the next request is sent at: the configured one, lowered to what was learned for
    /// this printer when fallback is allowed.
    pub fn version(&self) -> IppVersion {
        let learned = if self.fallback { self.memory.as_ref().and_then(|m| m.get(&self.uri.canonical())) } else { None };
        learned.map_or(self.version, |known| known.min(self.version))
    }

    /// Share one request-id counter between clients of the same backend.
    #[must_use]
    pub fn with_counter(mut self, counter: Arc<AtomicU32>) -> Self {
        self.next_id = counter;
        self
    }

    /// The printer this client talks to.
    pub fn uri(&self) -> &PrinterUri {
        &self.uri
    }

    /// The next request id from the shared counter; 0 is skipped, since the protocol reserves it.
    fn next_request_id(&self) -> u32 {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }

    /// The request message for an operation with its operation group: `attributes-charset` and
    /// `attributes-natural-language` first (RFC 8011 section 4.1.4), then the printer URI, a job id
    /// when there is one, and the user.
    pub fn request(&self, operation: Operation, job_id: Option<i32>, extra: Vec<Attribute>) -> Message {
        let (major, minor) = self.version().parts();
        let mut group = Group::new(GroupTag::Operation)
            .with(Attribute::new("attributes-charset", Value::Charset("utf-8".into())))
            .with(Attribute::new("attributes-natural-language", Value::NaturalLanguage("en".into())))
            .with(Attribute::uri("printer-uri", &self.uri.canonical()));
        if let Some(id) = job_id {
            group = group.with(Attribute::integer("job-id", id));
        }
        group = group.with(Attribute::name_value("requesting-user-name", &self.user));
        group.attributes.extend(extra);
        Message::new(major, minor, operation.code(), self.next_request_id()).with_group(group)
    }

    /// Send a request and read the reply. With fallback allowed ([`Self::with_fallback`]) and a
    /// request above 1.1, a `server-error-version-not-supported` answer is followed by exactly one
    /// more attempt at 1.1; whatever that attempt answers,
    /// including the same status again, is the result. There is no further retry. A printer that
    /// answers the 1.1 request with success or any status other than version-not-supported is
    /// remembered as speaking 1.1.
    pub fn exchange(&self, message: &Message) -> Result<Reply> {
        match self.exchange_once(message) {
            Err(IppError::Status { code: StatusCode::VERSION_NOT_SUPPORTED, .. }) if self.fallback && (message.major, message.minor) > (1, 1) => {
                let (major, minor) = IppVersion::V1_1.parts();
                let mut again = message.clone();
                (again.major, again.minor, again.request_id) = (major, minor, self.next_request_id());
                let result = self.exchange_once(&again);
                // An answer from the printer other than the same rejection shows it speaks 1.1.
                let answered = match &result {
                    Ok(_) => true,
                    Err(IppError::Status { code, .. }) => *code != StatusCode::VERSION_NOT_SUPPORTED,
                    Err(_) => false,
                };
                if answered && let Some(memory) = &self.memory {
                    memory.lower(&self.uri.canonical(), IppVersion::V1_1);
                }
                result
            }
            other => other,
        }
    }

    /// Send `message` once and read the reply. Fails on an oversized request, a non-200 HTTP status, a
    /// reply that is not `application/ipp` or answers another request id, and an IPP error status;
    /// attributes the printer marked unsupported become warnings.
    fn exchange_once(&self, message: &Message) -> Result<Reply> {
        let body = message.encode()?;
        if body.len() > self.limits.max_request {
            return Err(IppError::TooLarge("the request", self.limits.max_request));
        }
        // The limit covers the whole exchange, upload included: a document must not use up the time
        // meant for the printer's answer (see `with_upload_allowance`).
        let timeout = with_upload_allowance(self.limits.timeout, body.len());
        let request =
            Request { content_type: CONTENT_TYPE.to_owned(), soap_action: String::new(), body, timeout, max_response: self.limits.max_response };
        let response = self.transport.post(self.uri.as_device(), request)?;
        if response.status != 200 {
            return Err(IppError::Http { status: response.status, retry_after: response.retry_after });
        }
        if let Some(content_type) = &response.content_type
            && !content_type.trim().to_ascii_lowercase().starts_with(CONTENT_TYPE)
        {
            return Err(IppError::Protocol(format!("the reply is {content_type:?}, not an IPP message")));
        }
        let reply = Message::decode(&response.body, &self.codec.with_max_message(self.limits.max_response))?;
        if reply.request_id != message.request_id {
            return Err(IppError::Protocol(format!("the reply answers request {} where {} was sent", reply.request_id, message.request_id)));
        }
        let status = StatusCode(reply.code);
        let unsupported = unsupported_names(&reply);
        if !status.is_success() {
            let text = reply.group(GroupTag::Operation).and_then(|g| g.first_str("status-message")).unwrap_or("").chars().take(240).collect();
            return Err(IppError::Status { code: status, message: text, retry_after: response.retry_after, unsupported });
        }
        let warnings = if unsupported.is_empty() { Vec::new() } else { vec![format!("the printer ignored or changed: {}", unsupported.join(", "))] };
        Ok(Reply { message: reply, status, warnings })
    }

    /// The request's job attributes as a job group; `None` when there are none.
    fn job_group(request: &JobRequest) -> Option<Group> {
        if request.attributes.is_empty() {
            return None;
        }
        Some(Group { tag: GroupTag::Job, attributes: request.attributes.clone() })
    }

    /// The operation attributes that describe the job: its name, fidelity when asked, the document name,
    /// and the document format when `with_format`.
    fn create_attributes(request: &JobRequest, with_format: bool) -> Vec<Attribute> {
        let mut extra = vec![Attribute::name_value("job-name", &request.job_name)];
        if request.fidelity {
            extra.push(Attribute::boolean("ipp-attribute-fidelity", true));
        }
        if let Some(name) = &request.document_name {
            extra.push(Attribute::name_value("document-name", name));
        }
        if let (true, Some(format)) = (with_format, &request.document_format) {
            extra.push(Attribute::new("document-format", Value::MimeMediaType(format.clone())));
        }
        extra
    }

    /// The `Print-Job` request message.
    pub fn print_job_message(&self, request: &JobRequest, document: &[u8]) -> Message {
        let mut message = self.request(Operation::PrintJob, None, Self::create_attributes(request, true));
        message.groups.extend(Self::job_group(request));
        message.with_data(document.to_vec())
    }

    /// The `Validate-Job` request message.
    pub fn validate_job_message(&self, request: &JobRequest) -> Message {
        let mut message = self.request(Operation::ValidateJob, None, Self::create_attributes(request, true));
        message.groups.extend(Self::job_group(request));
        message
    }

    /// The `Create-Job` request message.
    pub fn create_job_message(&self, request: &JobRequest) -> Message {
        let mut message = self.request(Operation::CreateJob, None, Self::create_attributes(request, false));
        message.groups.extend(Self::job_group(request));
        message
    }

    /// The `Send-Document` request message.
    pub fn send_document_message(&self, job_id: i32, format: Option<&str>, last: bool, document: &[u8]) -> Message {
        let mut extra = Vec::new();
        if let Some(format) = format {
            extra.push(Attribute::new("document-format", Value::MimeMediaType(format.to_owned())));
        }
        extra.push(Attribute::boolean("last-document", last));
        self.request(Operation::SendDocument, Some(job_id), extra).with_data(document.to_vec())
    }

    /// `Get-Printer-Attributes`. With no names asked for, [`PRINTER_ATTRIBUTES`].
    pub fn get_printer_attributes(&self, requested: &[&str]) -> Result<PrinterAttrs> {
        let names = if requested.is_empty() { PRINTER_ATTRIBUTES } else { requested };
        let message = self.request(Operation::GetPrinterAttributes, None, vec![Attribute::keywords("requested-attributes", names)]);
        let reply = self.exchange(&message)?;
        let group = reply.group(GroupTag::Printer).ok_or_else(|| IppError::Protocol("the reply has no printer attributes".into()))?;
        Ok(PrinterAttrs::from_group(group))
    }

    /// `Validate-Job`: would the printer take this job? The reply's warnings say what it would change.
    pub fn validate_job(&self, request: &JobRequest) -> Result<Reply> {
        self.exchange(&self.validate_job_message(request))
    }

    /// Read a Print-Job or Create-Job reply; it must carry a job group with a positive `job-id`.
    fn accepted(reply: Reply) -> Result<Accepted> {
        let group = reply.group(GroupTag::Job).ok_or_else(|| IppError::Protocol("the reply has no job attributes".into()))?;
        let job = JobAttrs::from_group(group);
        let job_id = job.id.filter(|id| *id > 0).ok_or_else(|| IppError::Protocol("the reply names no job-id".into()))?;
        Ok(Accepted { job_id, job, warnings: reply.warnings, status: reply.status })
    }

    /// `Print-Job`: the job and its document in one request.
    pub fn print_job(&self, request: &JobRequest, document: &[u8]) -> Result<Accepted> {
        Self::accepted(self.exchange(&self.print_job_message(request, document))?)
    }

    /// `Create-Job`: a job without a document; follow with [`Self::send_document`].
    pub fn create_job(&self, request: &JobRequest) -> Result<Accepted> {
        Self::accepted(self.exchange(&self.create_job_message(request))?)
    }

    /// `Send-Document`.
    pub fn send_document(&self, job_id: i32, format: Option<&str>, last: bool, document: &[u8]) -> Result<Reply> {
        self.exchange(&self.send_document_message(job_id, format, last, document))
    }

    /// `Get-Job-Attributes`.
    pub fn get_job_attributes(&self, job_id: i32) -> Result<JobAttrs> {
        let message = self.request(Operation::GetJobAttributes, Some(job_id), vec![Attribute::keywords("requested-attributes", JOB_ATTRIBUTES)]);
        let reply = self.exchange(&message)?;
        let group = reply.group(GroupTag::Job).ok_or_else(|| IppError::Protocol("the reply has no job attributes".into()))?;
        Ok(JobAttrs::from_group(group))
    }

    /// `Get-Jobs`. With `my_jobs` only `user`'s jobs are listed; `limit` bounds the count.
    pub fn get_jobs(&self, which: Which, user: Option<&str>, my_jobs: bool, limit: u32) -> Result<Vec<JobAttrs>> {
        let mut message = self.request(
            Operation::GetJobs,
            None,
            vec![
                Attribute::integer("limit", i32::try_from(limit).unwrap_or(i32::MAX)),
                Attribute::keywords("requested-attributes", JOB_ATTRIBUTES),
                Attribute::keyword("which-jobs", which.keyword()),
                Attribute::boolean("my-jobs", my_jobs),
            ],
        );
        if let Some(user) = user
            && let Some(group) = message.groups.first_mut()
        {
            // requesting-user-name is already in the group; replace it with the asked-for user.
            if let Some(attribute) = group.attributes.iter_mut().find(|a| a.name == "requesting-user-name") {
                attribute.values = vec![Value::Name(user.to_owned())];
            }
        }
        let reply = self.exchange(&message)?;
        Ok(reply.message.groups_of(GroupTag::Job).map(JobAttrs::from_group).collect())
    }

    /// `Cancel-Job`.
    pub fn cancel_job(&self, job_id: i32) -> Result<()> {
        self.exchange(&self.request(Operation::CancelJob, Some(job_id), Vec::new())).map(|_| ())
    }
}

/// The names in the reply's unsupported-attributes groups, deduplicated, at most 32.
fn unsupported_names(reply: &Message) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for group in reply.groups_of(GroupTag::Unsupported) {
        for attribute in &group.attributes {
            if !names.contains(&attribute.name) {
                names.push(attribute.name.clone());
            }
        }
    }
    names.truncate(32);
    names
}
