//! Request bodies and reply readers for the operations, one function each.
//!
//! Builders return the fragment that goes inside `<s:Body>`; readers take the response element
//! that `craft_wsd::envelope::read_reply` hands over. Readers are lenient about vendor additions
//! (unknown elements are skipped) and strict about what they need (a missing `JobId` is an error).
//! Every list they build is capped.

use roxmltree::Node;

use super::adapter::xml::{self, child, children, is, text};
use super::error::{Result, WsdError};
use super::model::{
    Capabilities, JobElements, JobReason, JobState, JobStatus, JobSummary, PrinterDescription, PrinterElements, PrinterStatus, Section,
};
use super::ns;
use super::ticket::{DocumentDescription, Orientation, PrintTicket, Quality, Sides};

/// The most list entries read from one reply.
pub const MAX_ITEMS: usize = 256;
const MAX_VALUE: usize = 255;

/// The sections GetJobElements can return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobSection {
    Status,
    Ticket,
    Documents,
}

impl JobSection {
    /// The section's element name as GetJobElements requests it (`JobStatus`, `PrintTicket`, `Documents`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Status => "JobStatus",
            Self::Ticket => "PrintTicket",
            Self::Documents => "Documents",
        }
    }
}

/// The `GetPrinterElementsRequest` body asking for `sections`, in the order given.
pub fn get_printer_elements_body(sections: &[Section]) -> String {
    let names: String = sections.iter().map(|s| format!("<wprt:Name>wprt:{}</wprt:Name>", s.name())).collect();
    format!("<wprt:GetPrinterElementsRequest><wprt:RequestedElements>{names}</wprt:RequestedElements></wprt:GetPrinterElementsRequest>")
}

/// The `CreatePrintJobRequest` body carrying `ticket`; fails if the ticket cannot be written.
pub fn create_print_job_body(ticket: &PrintTicket) -> Result<String> {
    Ok(format!("<wprt:CreatePrintJobRequest>{}</wprt:CreatePrintJobRequest>", ticket.to_xml()?))
}

/// `document_data` is the `DocumentData` element from [`super::mtom`].
pub fn send_document_body(
    job_id: u32,
    description: &DocumentDescription,
    document_processing: &str,
    last: bool,
    document_data: &str,
) -> Result<String> {
    check_job_id(job_id)?;
    Ok(format!(
        "<wprt:SendDocumentRequest><wprt:JobId>{job_id}</wprt:JobId>{}{document_processing}<wprt:LastDocument>{last}</wprt:LastDocument>{document_data}</wprt:SendDocumentRequest>",
        description.to_xml()?
    ))
}

/// The `CancelJobRequest` body for `job_id`; an id above `i32::MAX` is refused as invalid.
pub fn cancel_job_body(job_id: u32) -> Result<String> {
    check_job_id(job_id)?;
    Ok(format!("<wprt:CancelJobRequest><wprt:JobId>{job_id}</wprt:JobId></wprt:CancelJobRequest>"))
}

/// The `GetJobElementsRequest` body asking for `sections` of job `job_id`; an id above `i32::MAX`
/// is refused as invalid.
pub fn get_job_elements_body(job_id: u32, sections: &[JobSection]) -> Result<String> {
    check_job_id(job_id)?;
    let names: String = sections.iter().map(|s| format!("<wprt:Name>wprt:{}</wprt:Name>", s.name())).collect();
    Ok(format!(
        "<wprt:GetJobElementsRequest><wprt:JobId>{job_id}</wprt:JobId><wprt:RequestedElements>{names}</wprt:RequestedElements></wprt:GetJobElementsRequest>"
    ))
}

/// The (empty) `GetActiveJobsRequest` body.
pub fn get_active_jobs_body() -> String {
    "<wprt:GetActiveJobsRequest/>".to_owned()
}

/// The (empty) `GetJobHistoryRequest` body.
pub fn get_job_history_body() -> String {
    "<wprt:GetJobHistoryRequest/>".to_owned()
}

/// Refuse a job id the printer cannot have issued (above `i32::MAX`).
fn check_job_id(id: u32) -> Result<()> {
    if id > i32::MAX as u32 { Err(WsdError::Invalid(format!("job id {id}: the printer issues 1 to {}", i32::MAX))) } else { Ok(()) }
}

/// A [`WsdError::Protocol`] error with `what` as its message.
fn protocol<T>(what: impl Into<String>) -> Result<T> {
    Err(WsdError::Protocol(what.into()))
}

/// A non-negative decimal integer (digits only), as the schema's `int` types are used here.
fn number(node: Node<'_, '_>) -> Option<u64> {
    let t = text(node);
    if t.is_empty() || t.len() > 18 || !t.bytes().all(|b| b.is_ascii_digit()) { None } else { t.parse().ok() }
}

/// [`number`], refused when it does not fit a `u32`.
fn number32(node: Node<'_, '_>) -> Option<u32> {
    number(node).and_then(|n| u32::try_from(n).ok())
}

// ---- CreatePrintJob ----

/// The `JobId` a `CreatePrintJobResponse` assigns; missing, non-numeric or above `i32::MAX` is a
/// protocol error.
pub fn parse_create_print_job(response: Node<'_, '_>) -> Result<u32> {
    let id = child(response, ns::WPRT, "JobId").ok_or_else(|| WsdError::Protocol("CreatePrintJobResponse has no JobId".into()))?;
    match number32(id) {
        Some(n) if n <= i32::MAX as u32 => Ok(n),
        _ => protocol(format!("the JobId {:?} is not a valid job id", text(id))),
    }
}

// ---- GetPrinterElements ----

/// Read a `GetPrinterElementsResponse`: description, status and capabilities where present, and the
/// names of sections the printer marked `Valid="false"`. Unknown sections are skipped.
pub fn parse_printer_elements(response: Node<'_, '_>) -> Result<PrinterElements> {
    let list = child(response, ns::WPRT, "PrinterElements").ok_or_else(|| WsdError::Protocol("no PrinterElements in the reply".into()))?;
    let mut out = PrinterElements::default();
    for data in children(list, ns::WPRT, "ElementData").take(MAX_ITEMS) {
        let Some(local) = section_name(data) else { continue };
        let valid = data.attribute("Valid").and_then(xml::parse_bool).unwrap_or(true);
        if !valid {
            out.invalid.push(local);
            continue;
        }
        match local.as_str() {
            "PrinterDescription" => out.description = child(data, ns::WPRT, "PrinterDescription").map(read_description),
            "PrinterStatus" => out.status = child(data, ns::WPRT, "PrinterStatus").map(read_status),
            "PrinterCapabilities" => out.capabilities = child(data, ns::WPRT, "PrinterCapabilities").map(read_capabilities),
            _ => {}
        }
    }
    Ok(out)
}

/// The local name of the wprt section an `ElementData` carries (from its `Name` QName).
fn section_name(data: Node<'_, '_>) -> Option<String> {
    let (uri, local) = xml::resolve_qname(data, data.attribute("Name")?)?;
    (uri.as_deref() == Some(ns::WPRT)).then_some(local)
}

/// Read `PrinterDescription`: the name (capped), colour and multi-document support, and pages per minute.
fn read_description(n: Node<'_, '_>) -> PrinterDescription {
    PrinterDescription {
        name: child(n, ns::WPRT, "PrinterName").map(text).unwrap_or_default().chars().take(MAX_VALUE).collect(),
        color_supported: child(n, ns::WPRT, "ColorSupported").and_then(|c| xml::parse_bool(&text(c))),
        multiple_documents: child(n, ns::WPRT, "MultipleDocumentJobsSupported").and_then(|c| xml::parse_bool(&text(c))),
        pages_per_minute: child(n, ns::WPRT, "PagesPerMinute").and_then(number32),
    }
}

/// Read `PrinterStatus`: the state, queued job count, and the primary and listed state reasons,
/// deduplicated and capped.
fn read_status(n: Node<'_, '_>) -> PrinterStatus {
    let mut reasons: Vec<String> = Vec::new();
    // Schema names: PrinterPrimaryStateReason (one) and PrinterStateReasons (a list of PrinterStateReason).
    for tag in ["PrinterPrimaryStateReason", "PrinterStateReasons"] {
        if let Some(r) = child(n, ns::WPRT, tag) {
            for v in r.descendants().filter(|d| d.is_element() && d.children().all(|c| !c.is_element())).take(MAX_ITEMS) {
                let t = text(v);
                if !t.is_empty() && t.len() <= MAX_VALUE && !reasons.contains(&t) && reasons.len() < MAX_ITEMS {
                    reasons.push(t);
                }
            }
        }
    }
    PrinterStatus {
        state: child(n, ns::WPRT, "PrinterState").map(text).unwrap_or_default().chars().take(MAX_VALUE).collect(),
        queued_jobs: child(n, ns::WPRT, "QueuedJobCount").and_then(number32),
        reasons,
    }
}

/// `AllowedValue` entries below `parent/<name>`: each element's text, split on white space when
/// `tokens` (NMTOKENS lists), else whole.
fn allowed(parent: Node<'_, '_>, name: &str, tokens: bool) -> Vec<String> {
    let mut out = Vec::new();
    let Some(list) = child(parent, ns::WPRT, name) else { return out };
    for v in children(list, ns::WPRT, "AllowedValue").take(MAX_ITEMS) {
        let t = text(v);
        let parts: Vec<&str> = if tokens { t.split_whitespace().collect() } else { vec![t.as_str()] };
        for p in parts {
            if !p.is_empty() && p.len() <= MAX_VALUE && out.len() < MAX_ITEMS && !out.iter().any(|o| o == p) {
                out.push(p.to_owned());
            }
        }
    }
    out
}

/// Read `PrinterCapabilities`: the copies range (minimum 1 when unstated), media sizes and types,
/// orientations, qualities, sides, resolutions, document formats and compressions.
fn read_capabilities(n: Node<'_, '_>) -> Capabilities {
    let mut caps = Capabilities::default();
    if let Some(jobs) = child(n, ns::WPRT, "JobValues") {
        if let Some(copies) = child(jobs, ns::WPRT, "JobProcessing").and_then(|p| child(p, ns::WPRT, "Copies")) {
            let max = child(copies, ns::WPRT, "MaxValue").and_then(number32);
            let min = child(copies, ns::WPRT, "MinValue").and_then(number32).unwrap_or(1);
            caps.copies = max.map(|m| (min, m));
        }
        if let Some(dp) = child(jobs, ns::WPRT, "DocumentProcessing") {
            caps.media_sizes = allowed(dp, "MediaSizeName", true);
            caps.media_types = allowed(dp, "MediaType", true);
            caps.orientations = allowed(dp, "Orientation", true);
            caps.qualities = allowed(dp, "PrintQuality", true);
            caps.sides = allowed(dp, "Sides", true);
            if let Some(res) = child(dp, ns::WPRT, "Resolution") {
                caps.resolutions = read_resolutions(res);
            }
        }
    }
    if let Some(dd) = child(n, ns::WPRT, "DocumentValues").and_then(|d| child(d, ns::WPRT, "DocumentDescription")) {
        caps.formats = allowed(dd, "Format", false);
        caps.compressions = allowed(dd, "Compression", true);
    }
    caps
}

/// Resolution lists have no named list type in the schema, so read every `Width`/`Height` pair
/// under the element (directly, or one level down in `AllowedValue`-like entries).
fn read_resolutions(res: Node<'_, '_>) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut candidates: Vec<Node<'_, '_>> = vec![res];
    candidates.extend(res.children().filter(|c| c.is_element()).take(MAX_ITEMS));
    for c in candidates {
        let w = child(c, ns::WPRT, "Width").and_then(number32);
        let h = child(c, ns::WPRT, "Height").and_then(number32);
        if let Some(w) = w.filter(|w| *w > 0) {
            let pair = (w, h.filter(|h| *h > 0).unwrap_or(w));
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out
}

// ---- jobs ----

/// Read a job status or summary entry. A valid `JobId` and a non-empty `JobState` are required;
/// missing counters read as zero.
fn read_job_status(n: Node<'_, '_>) -> Result<JobStatus> {
    let job_id = child(n, ns::WPRT, "JobId").and_then(number32).filter(|id| *id <= i32::MAX as u32);
    let Some(job_id) = job_id else { return protocol("a job entry without a valid JobId") };
    let Some(state) = child(n, ns::WPRT, "JobState").map(text).filter(|s| !s.is_empty()) else { return protocol("a job entry without a JobState") };
    let reasons = child(n, ns::WPRT, "JobStateReasons")
        .map(|r| children(r, ns::WPRT, "JobStateReason").take(MAX_ITEMS).map(|x| JobReason::parse(&text(x))).collect())
        .unwrap_or_default();
    let count = |name: &str| child(n, ns::WPRT, name).and_then(number).unwrap_or(0);
    Ok(JobStatus {
        job_id,
        state: JobState::parse(&state),
        reasons,
        k_octets_processed: count("KOctetsProcessed"),
        sheets_completed: count("MediaSheetsCompleted"),
        documents: count("NumberOfDocuments"),
    })
}

/// Read a `GetJobElementsResponse`: the job status and print ticket where present, and the sections
/// the printer marked invalid. A malformed status fails the whole reply.
pub fn parse_job_elements(response: Node<'_, '_>) -> Result<JobElements> {
    let list = child(response, ns::WPRT, "JobElements").ok_or_else(|| WsdError::Protocol("no JobElements in the reply".into()))?;
    let mut out = JobElements::default();
    for data in children(list, ns::WPRT, "ElementData").take(MAX_ITEMS) {
        let Some(local) = section_name(data) else { continue };
        if !data.attribute("Valid").and_then(xml::parse_bool).unwrap_or(true) {
            out.invalid.push(local);
            continue;
        }
        match local.as_str() {
            "JobStatus" => {
                if let Some(s) = child(data, ns::WPRT, "JobStatus") {
                    out.status = Some(read_job_status(s)?);
                }
            }
            "PrintTicket" => {
                let t = child(data, ns::WPRT, "PrintTicket");
                out.has_ticket = t.is_some();
                out.ticket = t.map(read_ticket);
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The settings a `PrintTicket` element states; what is absent or unrecognised stays `None`.
fn read_ticket(n: Node<'_, '_>) -> PrintTicket {
    let short = |v: Node<'_, '_>| text(v).chars().take(MAX_VALUE).collect::<String>();
    let mut t = PrintTicket::default();
    if let Some(d) = child(n, ns::WPRT, "JobDescription") {
        t.job_name = child(d, ns::WPRT, "JobName").map(short).unwrap_or_default();
        t.user_name = child(d, ns::WPRT, "JobOriginatingUserName").map(short).unwrap_or_default();
    }
    t.copies = child(n, ns::WPRT, "JobProcessing").and_then(|p| child(p, ns::WPRT, "Copies")).and_then(number32);
    if let Some(dp) = child(n, ns::WPRT, "DocumentProcessing") {
        t.media_size = child(dp, ns::WPRT, "MediaSizeName").map(short).filter(|s| !s.is_empty());
        t.orientation = child(dp, ns::WPRT, "Orientation").and_then(|v| Orientation::parse(&text(v)));
        t.quality = child(dp, ns::WPRT, "PrintQuality").and_then(|v| Quality::parse(&text(v)));
        t.sides = child(dp, ns::WPRT, "Sides").and_then(|v| Sides::parse(&text(v)));
        t.resolution = child(dp, ns::WPRT, "Resolution").and_then(|r| {
            let w = child(r, ns::WPRT, "Width").and_then(number32)?;
            let h = child(r, ns::WPRT, "Height").and_then(number32)?;
            Some((w, h))
        });
    }
    t
}

/// GetActiveJobs (`container` = `ActiveJobs`) and GetJobHistory (`JobHistory`).
///
/// More than [`MAX_ITEMS`] active jobs is [`WsdError::TooLarge`]: the list is a queue, and a
/// partial one would hide jobs. A history, which a busy printer can make as long as it likes, is
/// not refused: it is cut to the [`MAX_ITEMS`] rows with the highest job ids. WS-Print does not
/// say in which order a history is listed, and job ids grow with submission, so the newest jobs
/// (the ones a caller looking for a job it just sent needs) are kept whatever the order; the rows
/// kept stay in the order the printer gave them.
pub fn parse_job_list(response: Node<'_, '_>, container: &str) -> Result<Vec<JobSummary>> {
    let list = child(response, ns::WPRT, container).ok_or_else(|| WsdError::Protocol(format!("no {container} in the reply")))?;
    let history = container == "JobHistory";
    let mut out = Vec::new();
    for s in list.children().filter(|c| is(*c, ns::WPRT, "JobSummary")) {
        if out.len() >= MAX_ITEMS {
            if !history {
                return Err(WsdError::TooLarge("the job list", MAX_ITEMS));
            }
            // Keep memory bounded: trim whenever the buffer has doubled.
            if out.len() >= 2 * MAX_ITEMS {
                keep_newest(&mut out);
            }
        }
        out.push(JobSummary {
            status: read_job_status(s)?,
            job_name: child(s, ns::WPRT, "JobName").map(text).unwrap_or_default(),
            user_name: child(s, ns::WPRT, "JobOriginatingUserName").map(text).unwrap_or_default(),
        });
    }
    if out.len() > MAX_ITEMS {
        keep_newest(&mut out);
    }
    Ok(out)
}

/// Cut `rows` to the [`MAX_ITEMS`] with the highest job ids, in their present order.
fn keep_newest(rows: &mut Vec<JobSummary>) {
    let mut ids: Vec<u32> = rows.iter().map(|r| r.status.job_id).collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    if let Some(&cut) = ids.get(MAX_ITEMS - 1) {
        rows.retain(|r| r.status.job_id >= cut);
    }
    rows.truncate(MAX_ITEMS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsd::ticket::Compression;

    /// Parse `body` inside a root element that declares `wprt` and a `q` prefix, and hand the root to `f`.
    fn with<T>(body: &str, f: impl FnOnce(Node<'_, '_>) -> Result<T>) -> Result<T> {
        let x = format!("<wprt:R xmlns:wprt=\"{}\" xmlns:q=\"urn:q\">{body}</wprt:R>", ns::WPRT);
        let doc = xml::parse(&x)?;
        f(doc.root_element())
    }

    #[test]
    fn request_bodies_are_the_documented_shapes() {
        let g = get_printer_elements_body(&[Section::Description, Section::Capabilities]);
        assert!(g.contains("<wprt:Name>wprt:PrinterDescription</wprt:Name><wprt:Name>wprt:PrinterCapabilities</wprt:Name>"));
        assert_eq!(cancel_job_body(5).unwrap(), "<wprt:CancelJobRequest><wprt:JobId>5</wprt:JobId></wprt:CancelJobRequest>");
        assert!(get_job_elements_body(5, &[JobSection::Status, JobSection::Ticket]).unwrap().contains("wprt:PrintTicket"));
        assert_eq!(get_active_jobs_body(), "<wprt:GetActiveJobsRequest/>");
        assert!(cancel_job_body(u32::MAX).is_err() && get_job_elements_body(i32::MAX as u32 + 1, &[]).is_err());
        let t = PrintTicket { job_name: "j".into(), user_name: "u".into(), ..Default::default() };
        assert!(create_print_job_body(&t).unwrap().starts_with("<wprt:CreatePrintJobRequest><wprt:PrintTicket>"));
        let d = DocumentDescription { id: 1, compression: Compression::None, format: "application/pdf".into(), name: None, must_honor_format: false };
        let s = send_document_body(2, &d, "", true, "<wprt:DocumentData/>").unwrap();
        let (a, b, c, e) = (s.find("<wprt:JobId>"), s.find("DocumentDescription"), s.find("LastDocument"), s.find("<wprt:DocumentData"));
        assert!(a < b && b < c && c < e, "{s}");
        assert!(s.contains("<wprt:LastDocument>true</wprt:LastDocument>"));
    }

    #[test]
    fn create_job_ids_are_validated() {
        let id = |v: &str| with(&format!("<wprt:JobId>{v}</wprt:JobId>"), parse_create_print_job);
        assert_eq!(id("17").unwrap(), 17);
        assert_eq!(id(" 2147483647 ").unwrap(), i32::MAX as u32);
        for bad in ["", "-1", "+3", "abc", "2147483648", "99999999999999999999", "1.5", "0x10"] {
            assert!(id(bad).is_err(), "{bad:?}");
        }
        assert!(with("", parse_create_print_job).is_err());
    }

    const ELEMENTS: &str = "<wprt:PrinterElements>\
<wprt:ElementData Name=\"wprt:PrinterDescription\" Valid=\"true\"><wprt:PrinterDescription><wprt:ColorSupported>true</wprt:ColorSupported><wprt:DeviceId>x</wprt:DeviceId><wprt:MultipleDocumentJobsSupported>false</wprt:MultipleDocumentJobsSupported><wprt:PagesPerMinute>20</wprt:PagesPerMinute><wprt:PrinterName>Lab</wprt:PrinterName></wprt:PrinterDescription></wprt:ElementData>\
<wprt:ElementData Name=\"wprt:PrinterStatus\" Valid=\"true\"><wprt:PrinterStatus><wprt:PrinterCurrentTime>2026-01-01T00:00:00Z</wprt:PrinterCurrentTime><wprt:PrinterState>Idle</wprt:PrinterState><wprt:QueuedJobCount>2</wprt:QueuedJobCount></wprt:PrinterStatus></wprt:ElementData>\
<wprt:ElementData Name=\"wprt:PrinterCapabilities\" Valid=\"true\"><wprt:PrinterCapabilities>\
<wprt:JobValues><wprt:JobProcessing><wprt:Copies><wprt:MaxValue>99</wprt:MaxValue></wprt:Copies></wprt:JobProcessing>\
<wprt:DocumentProcessing><wprt:MediaSizeName><wprt:AllowedValue>iso_a4_210x297mm na_letter_8.5x11in</wprt:AllowedValue></wprt:MediaSizeName>\
<wprt:Orientation><wprt:AllowedValue>Portrait</wprt:AllowedValue><wprt:AllowedValue>Landscape</wprt:AllowedValue></wprt:Orientation>\
<wprt:Sides><wprt:AllowedValue>OneSided TwoSidedLongEdge</wprt:AllowedValue></wprt:Sides>\
<wprt:Resolution><wprt:AllowedValue><wprt:Width>300</wprt:Width><wprt:Height>300</wprt:Height></wprt:AllowedValue><wprt:AllowedValue><wprt:Width>600</wprt:Width></wprt:AllowedValue></wprt:Resolution>\
</wprt:DocumentProcessing></wprt:JobValues>\
<wprt:DocumentValues><wprt:DocumentDescription><wprt:Compression><wprt:AllowedValue>None Gzip</wprt:AllowedValue></wprt:Compression><wprt:Format><wprt:AllowedValue>application/pdf</wprt:AllowedValue><wprt:AllowedValue>image/pwg-raster</wprt:AllowedValue><wprt:AllowedValue>application/pdf</wprt:AllowedValue></wprt:Format></wprt:DocumentDescription></wprt:DocumentValues>\
</wprt:PrinterCapabilities></wprt:ElementData>\
<wprt:ElementData Name=\"wprt:DefaultPrintTicket\" Valid=\"false\"/>\
<wprt:ElementData Name=\"q:Vendor\" Valid=\"true\"><q:Anything/></wprt:ElementData>\
</wprt:PrinterElements>";

    #[test]
    fn printer_elements_yield_description_status_and_capabilities() {
        let e = with(ELEMENTS, parse_printer_elements).unwrap();
        let d = e.description.unwrap();
        assert_eq!((d.name.as_str(), d.color_supported, d.multiple_documents, d.pages_per_minute), ("Lab", Some(true), Some(false), Some(20)));
        let s = e.status.unwrap();
        assert_eq!((s.state.as_str(), s.queued_jobs), ("Idle", Some(2)));
        assert_eq!(e.invalid, ["DefaultPrintTicket"]);
        let c = e.capabilities.unwrap();
        assert_eq!(c.formats, ["application/pdf", "image/pwg-raster"]);
        assert_eq!(c.compressions, ["None", "Gzip"]);
        assert_eq!(c.media_sizes, ["iso_a4_210x297mm", "na_letter_8.5x11in"]);
        assert_eq!(c.sides, ["OneSided", "TwoSidedLongEdge"]);
        assert_eq!(c.copies, Some((1, 99)));
        assert_eq!(c.resolutions, [(300, 300), (600, 600)]);
        assert_eq!(c.supports_sides(crate::wsd::ticket::Sides::TwoSidedShortEdge), Some(false));
        assert_eq!(c.supports_quality(crate::wsd::ticket::Quality::High), None);
        assert_eq!(c.supports_copies(100), Some(false));
        assert_eq!(c.supports_resolution((600, 600)), Some(true));
        assert_eq!(c.accepts_format("APPLICATION/PDF"), Some("application/pdf"));
        assert!(c.accepts_no_compression());
    }

    #[test]
    fn missing_sections_and_hostile_values_degrade() {
        assert!(with("", parse_printer_elements).is_err());
        let e = with("<wprt:PrinterElements/>", parse_printer_elements).unwrap();
        assert_eq!(e, PrinterElements::default());
        // Wrong namespace for the section name, junk numbers, absurd lists.
        let junk = "<wprt:PrinterElements><wprt:ElementData Name=\"q:PrinterDescription\"><wprt:PrinterDescription/></wprt:ElementData>\
<wprt:ElementData Name=\"nonsense\"/><wprt:ElementData/><wprt:ElementData Name=\"wprt:PrinterDescription\" Valid=\"maybe\"><wprt:PrinterDescription><wprt:PagesPerMinute>-4</wprt:PagesPerMinute><wprt:ColorSupported>yes</wprt:ColorSupported></wprt:PrinterDescription></wprt:ElementData></wprt:PrinterElements>";
        let e = with(junk, parse_printer_elements).unwrap();
        let d = e.description.unwrap();
        assert_eq!((d.pages_per_minute, d.color_supported, d.name.as_str()), (None, None, ""));
        let many: String = (0..1000).map(|i| format!("<wprt:AllowedValue>image/x{i}</wprt:AllowedValue>")).collect();
        let big = format!(
            "<wprt:PrinterElements><wprt:ElementData Name=\"wprt:PrinterCapabilities\"><wprt:PrinterCapabilities><wprt:DocumentValues><wprt:DocumentDescription><wprt:Format>{many}</wprt:Format></wprt:DocumentDescription></wprt:DocumentValues></wprt:PrinterCapabilities></wprt:ElementData></wprt:PrinterElements>"
        );
        assert_eq!(with(&big, parse_printer_elements).unwrap().capabilities.unwrap().formats.len(), MAX_ITEMS);
        let long = format!("<wprt:AllowedValue>{}</wprt:AllowedValue>", "x".repeat(400));
        let big = format!(
            "<wprt:PrinterElements><wprt:ElementData Name=\"wprt:PrinterCapabilities\"><wprt:PrinterCapabilities><wprt:DocumentValues><wprt:DocumentDescription><wprt:Format>{long}</wprt:Format></wprt:DocumentDescription></wprt:DocumentValues></wprt:PrinterCapabilities></wprt:ElementData></wprt:PrinterElements>"
        );
        assert!(with(&big, parse_printer_elements).unwrap().capabilities.unwrap().formats.is_empty());
    }

    #[test]
    fn printer_status_reads_the_schema_named_reasons() {
        let body = "<wprt:PrinterElements><wprt:ElementData Name=\"wprt:PrinterStatus\" Valid=\"true\"><wprt:PrinterStatus><wprt:PrinterCurrentTime>2026-01-01T00:00:00Z</wprt:PrinterCurrentTime><wprt:PrinterState>Stopped</wprt:PrinterState><wprt:PrinterPrimaryStateReason>MediaEmpty</wprt:PrinterPrimaryStateReason><wprt:PrinterStateReasons><wprt:PrinterStateReason>MediaEmpty</wprt:PrinterStateReason><wprt:PrinterStateReason>MarkerSupplyLow</wprt:PrinterStateReason><wprt:PrinterStateReason>vendor:x</wprt:PrinterStateReason></wprt:PrinterStateReasons><wprt:QueuedJobCount>0</wprt:QueuedJobCount></wprt:PrinterStatus></wprt:ElementData></wprt:PrinterElements>";
        let st = with(body, parse_printer_elements).unwrap().status.unwrap();
        assert_eq!(st.state, "Stopped");
        assert_eq!(st.reasons, ["MediaEmpty", "MarkerSupplyLow", "vendor:x"]);
        // Only the list, only the primary, or neither.
        let only = |inner: &str| {
            let b = format!(
                "<wprt:PrinterElements><wprt:ElementData Name=\"wprt:PrinterStatus\"><wprt:PrinterStatus><wprt:PrinterState>Idle</wprt:PrinterState>{inner}</wprt:PrinterStatus></wprt:ElementData></wprt:PrinterElements>"
            );
            with(&b, parse_printer_elements).unwrap().status.unwrap().reasons
        };
        assert_eq!(only("<wprt:PrinterPrimaryStateReason>DoorOpen</wprt:PrinterPrimaryStateReason>"), ["DoorOpen"]);
        assert_eq!(
            only("<wprt:PrinterStateReasons><wprt:PrinterStateReason>MediaJam</wprt:PrinterStateReason></wprt:PrinterStateReasons>"),
            ["MediaJam"]
        );
        assert!(only("").is_empty());
        // The old, wrong names are not read.
        assert!(only("<wprt:PrimaryStateReason>DoorOpen</wprt:PrimaryStateReason>").is_empty());
        // Hostile: a flood and an overlong value are capped.
        let flood: String = (0..(MAX_ITEMS * 2)).map(|i| format!("<wprt:PrinterStateReason>r{i}</wprt:PrinterStateReason>")).collect();
        assert_eq!(only(&format!("<wprt:PrinterStateReasons>{flood}</wprt:PrinterStateReasons>")).len(), MAX_ITEMS);
        let long = "x".repeat(400);
        assert!(
            only(&format!("<wprt:PrinterStateReasons><wprt:PrinterStateReason>{long}</wprt:PrinterStateReason></wprt:PrinterStateReasons>"))
                .is_empty()
        );
    }

    #[test]
    fn job_elements_read_back_the_effective_ticket() {
        let body = "<wprt:JobElements><wprt:ElementData Name=\"wprt:PrintTicket\" Valid=\"true\"><wprt:PrintTicket><wprt:JobDescription><wprt:JobName>n</wprt:JobName><wprt:JobOriginatingUserName>u</wprt:JobOriginatingUserName></wprt:JobDescription><wprt:JobProcessing><wprt:Copies>3</wprt:Copies></wprt:JobProcessing><wprt:DocumentProcessing><wprt:MediaSizeName>iso_a4_210x297mm</wprt:MediaSizeName><wprt:Orientation>Landscape</wprt:Orientation><wprt:Resolution><wprt:Width>600</wprt:Width><wprt:Height>300</wprt:Height></wprt:Resolution><wprt:PrintQuality>High</wprt:PrintQuality><wprt:Sides>TwoSidedLongEdge</wprt:Sides></wprt:DocumentProcessing></wprt:PrintTicket></wprt:ElementData></wprt:JobElements>";
        let t = with(body, parse_job_elements).unwrap().ticket.unwrap();
        assert_eq!((t.job_name.as_str(), t.user_name.as_str(), t.copies), ("n", "u", Some(3)));
        assert_eq!(t.media_size.as_deref(), Some("iso_a4_210x297mm"));
        assert_eq!(
            (t.orientation, t.quality, t.sides, t.resolution),
            (Some(Orientation::Landscape), Some(Quality::High), Some(Sides::TwoSidedLongEdge), Some((600, 300)))
        );
        // An empty ticket and unknown tokens leave everything unset.
        let empty = "<wprt:JobElements><wprt:ElementData Name=\"wprt:PrintTicket\"><wprt:PrintTicket><wprt:DocumentProcessing><wprt:Sides>Bogus</wprt:Sides></wprt:DocumentProcessing></wprt:PrintTicket></wprt:ElementData></wprt:JobElements>";
        let j = with(empty, parse_job_elements).unwrap();
        assert!(j.has_ticket && j.ticket.unwrap().sides.is_none());
    }

    /// The children of a job status entry with this id, state and reasons, and fixed counters.
    fn job(id: &str, state: &str, reasons: &[&str]) -> String {
        let r: String = reasons.iter().map(|r| format!("<wprt:JobStateReason>{r}</wprt:JobStateReason>")).collect();
        let r = if r.is_empty() { String::new() } else { format!("<wprt:JobStateReasons>{r}</wprt:JobStateReasons>") };
        format!(
            "<wprt:JobId>{id}</wprt:JobId><wprt:JobState>{state}</wprt:JobState>{r}<wprt:KOctetsProcessed>12</wprt:KOctetsProcessed><wprt:MediaSheetsCompleted>2</wprt:MediaSheetsCompleted><wprt:NumberOfDocuments>1</wprt:NumberOfDocuments>"
        )
    }

    #[test]
    fn job_elements_carry_status_and_reasons() {
        let body = format!(
            "<wprt:JobElements><wprt:ElementData Name=\"wprt:JobStatus\" Valid=\"true\"><wprt:JobStatus>{}</wprt:JobStatus></wprt:ElementData><wprt:ElementData Name=\"wprt:PrintTicket\" Valid=\"true\"><wprt:PrintTicket/></wprt:ElementData><wprt:ElementData Name=\"wprt:Documents\" Valid=\"false\"/></wprt:JobElements>",
            job("4", "Completed", &["JobCompletedWithWarnings", "vendor:thing"])
        );
        let j = with(&body, parse_job_elements).unwrap();
        let s = j.status.unwrap();
        assert_eq!((s.job_id, s.state.clone(), s.k_octets_processed, s.sheets_completed, s.documents), (4, JobState::Completed, 12, 2, 1));
        assert_eq!(s.reasons, [JobReason::JobCompletedWithWarnings, JobReason::Other("vendor:thing".into())]);
        assert!(j.has_ticket && s.state.is_terminal());
        assert_eq!(j.invalid, ["Documents"]);
    }

    #[test]
    fn job_entries_without_an_id_or_state_are_protocol_errors() {
        let wrap = |inner: &str| {
            format!(
                "<wprt:JobElements><wprt:ElementData Name=\"wprt:JobStatus\"><wprt:JobStatus>{inner}</wprt:JobStatus></wprt:ElementData></wprt:JobElements>"
            )
        };
        for inner in [
            "",
            "<wprt:JobState>Pending</wprt:JobState>",
            "<wprt:JobId>1</wprt:JobId>",
            "<wprt:JobId>-2</wprt:JobId><wprt:JobState>Pending</wprt:JobState>",
        ] {
            assert!(matches!(with(&wrap(inner), parse_job_elements), Err(WsdError::Protocol(_))), "{inner}");
        }
        assert!(with("", parse_job_elements).is_err());
    }

    #[test]
    fn active_job_lists_parse_and_are_capped() {
        let one = |id: u32| {
            format!(
                "<wprt:JobSummary>{}<wprt:JobName>n{id}</wprt:JobName><wprt:JobOriginatingUserName>u</wprt:JobOriginatingUserName></wprt:JobSummary>",
                job(&id.to_string(), "Processing", &["JobPrinting"])
            )
        };
        let list = with(&format!("<wprt:ActiveJobs>{}{}</wprt:ActiveJobs>", one(1), one(2)), |n| parse_job_list(n, "ActiveJobs")).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((list[1].job_name.as_str(), list[0].status.state.clone()), ("n2", JobState::Processing));
        assert!(with("<wprt:ActiveJobs/>", |n| parse_job_list(n, "ActiveJobs")).unwrap().is_empty());
        assert!(with("", |n| parse_job_list(n, "ActiveJobs")).is_err());
        // An active list over the cap is refused: a partial queue would hide jobs.
        let flood: String = (1..=(MAX_ITEMS as u32 + 5)).map(one).collect();
        assert!(matches!(
            with(&format!("<wprt:ActiveJobs>{flood}</wprt:ActiveJobs>"), |n| parse_job_list(n, "ActiveJobs")),
            Err(WsdError::TooLarge(..))
        ));
        // A history over the cap is cut to the newest MAX_ITEMS, not refused, whatever its order.
        let total = 300u32;
        let ids = |rows: &[JobSummary]| rows.iter().map(|r| r.status.job_id).collect::<Vec<_>>();
        let ascending: String = (1..=total).map(one).collect();
        let kept = with(&format!("<wprt:JobHistory>{ascending}</wprt:JobHistory>"), |n| parse_job_list(n, "JobHistory")).unwrap();
        assert_eq!(kept.len(), MAX_ITEMS);
        assert_eq!(ids(&kept), (total - MAX_ITEMS as u32 + 1..=total).collect::<Vec<_>>(), "oldest-first: the newest are kept");
        let descending: String = (1..=total).rev().map(one).collect();
        let kept = with(&format!("<wprt:JobHistory>{descending}</wprt:JobHistory>"), |n| parse_job_list(n, "JobHistory")).unwrap();
        assert_eq!(ids(&kept), (total - MAX_ITEMS as u32 + 1..=total).rev().collect::<Vec<_>>(), "newest-first: same rows, same order");
        // Far over the cap still ends at MAX_ITEMS.
        let many: String = (1..=(MAX_ITEMS as u32 * 5)).map(one).collect();
        let kept = with(&format!("<wprt:JobHistory>{many}</wprt:JobHistory>"), |n| parse_job_list(n, "JobHistory")).unwrap();
        assert_eq!((kept.len(), kept[0].status.job_id), (MAX_ITEMS, MAX_ITEMS as u32 * 4 + 1));
    }

    #[test]
    fn states_and_reasons_round_trip_their_names() {
        for s in ["Pending", "Pending-Held", "Processing", "ProcessingStopped", "Started", "Terminating", "Completed", "Canceled", "Aborted", "x:y"] {
            assert_eq!(JobState::parse(s).name(), s);
        }
        assert!(JobState::Aborted.is_terminal() && !JobState::Terminating.is_terminal() && !JobState::Other("x".into()).is_terminal());
        assert_eq!(JobReason::parse("DocumentFormatError"), JobReason::DocumentFormatError);
    }
}
