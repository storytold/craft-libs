//! The high-level flow: discover, negotiate, create, send, poll, and map the end state.
//!
//! [`print_document`] does the whole job and needs a [`UserConfirmed`] token, so an agent or a
//! script cannot print without the person's explicit yes (the registry checks it too). Everything before the
//! first request that changes anything (planning the ticket, choosing the format, producing the
//! bytes) is plain functions that are tested on their own.

use std::time::{Duration, Instant};

use super::client::Client;
use super::encoder::{ColorMode, DocumentEncoder, EncodeContext};
use super::error::{Op, Result, WsdError};
use super::fault::FaultCode;
use super::model::{Capabilities, JobReason, JobState, JobStatus, PrinterDescription, PrinterElements, Section};
use super::mtom::Transfer;
use super::ticket::{Compression, DocumentDescription, Orientation, PrintTicket, Quality, Sides, TicketShape};
use crate::{CancelToken, RetrySafety, UserConfirmed};

use super::adapter::Transport;

/// How to poll for the end of a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollPolicy {
    pub interval: Duration,
    /// Give up waiting (the job keeps going on the printer) after this long.
    pub max_wait: Duration,
}

impl Default for PollPolicy {
    fn default() -> Self {
        Self { interval: Duration::from_secs(1), max_wait: Duration::from_secs(300) }
    }
}

/// What the person asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintOptions {
    pub job_name: String,
    /// `JobOriginatingUserName`. Defaults to a fixed `craft` rather than the account name.
    pub user_name: String,
    pub document_name: Option<String>,
    pub copies: Option<u32>,
    pub sides: Option<Sides>,
    pub orientation: Option<Orientation>,
    pub media_size: Option<String>,
    pub quality: Option<Quality>,
    pub resolution: Option<(u32, u32)>,
    pub color: ColorMode,
    /// Refuse (before anything is sent) a setting the printer lists as unsupported, and mark every
    /// setting `MustHonor` so the printer refuses what it cannot do. Off: such settings are
    /// dropped or clamped and reported in [`JobResult::warnings`].
    pub strict: bool,
    /// How much of the ticket to send. Anything but [`TicketShape::Full`] is a quirk mode for
    /// printers whose firmware stalls on the ticket's processing elements (`JobProcessing`,
    /// `DocumentProcessing`); [`TicketShape::Description`] is the recommended one. Settings that
    /// need those elements (copies, sides, orientation, quality, media size, resolution) are then
    /// refused up front with [`WsdError::Unsupported`], unless [`Self::drop_ticket_settings`] is set.
    /// Off by default.
    pub ticket_shape: TicketShape,
    /// With a reduced ticket, drop the settings it cannot carry (a warning each) instead of refusing.
    pub drop_ticket_settings: bool,
    pub transfer: Transfer,
    pub poll: PollPolicy,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            job_name: "Craft".into(),
            user_name: "craft".into(),
            document_name: None,
            copies: None,
            sides: None,
            orientation: None,
            media_size: None,
            quality: None,
            resolution: None,
            color: ColorMode::Auto,
            strict: false,
            ticket_shape: TicketShape::Full,
            drop_ticket_settings: false,
            transfer: Transfer::Mtom,
            poll: PollPolicy::default(),
        }
    }
}

/// How the job ended (or why we stopped watching).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobOutcome {
    Completed,
    CompletedWithWarnings,
    CompletedWithErrors,
    Canceled {
        at_device: bool,
    },
    Aborted {
        reason: Option<JobReason>,
    },
    /// The printer no longer knows the job (finished jobs are forgotten); most likely it finished.
    Forgotten,
    /// Still running when the wait ran out; the job continues on the printer.
    StillRunning,
}

impl JobOutcome {
    /// A stable lowercase name for logs and tool results.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithWarnings => "completed_with_warnings",
            Self::CompletedWithErrors => "completed_with_errors",
            Self::Canceled { .. } => "canceled",
            Self::Aborted { .. } => "aborted",
            Self::Forgotten => "forgotten",
            Self::StillRunning => "still_running",
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed | Self::CompletedWithWarnings)
    }
}

/// The result of [`print_document`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobResult {
    pub job_id: u32,
    pub outcome: JobOutcome,
    pub state: Option<JobState>,
    pub reasons: Vec<JobReason>,
    /// The format the document was sent in, as the printer spells it.
    pub format: String,
    pub bytes_sent: usize,
    pub sheets_completed: u64,
    pub printer_name: String,
    /// Settings that were dropped or changed because the printer does not list them.
    pub warnings: Vec<String>,
}

/// Choose the first offered format the printer lists; returns the printer's spelling.
pub fn negotiate_format(offered: &[String], caps: &Capabilities) -> Result<String> {
    for f in offered {
        if let Some(spelled) = caps.accepts_format(f) {
            return Ok(spelled.to_owned());
        }
    }
    Err(WsdError::NoCommonFormat { offered: offered.to_vec(), accepted: caps.formats.clone() })
}

/// The ticket and picture settings for this printer, with the changes made along the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub ticket: PrintTicket,
    pub color: bool,
    pub dpi: (u32, u32),
    pub warnings: Vec<String>,
}

/// Check the options against what the printer lists. A dimension the printer does not list is not
/// checked (the printer will answer at CreatePrintJob; with `strict` it refuses under `MustHonor`).
pub fn plan(options: &PrintOptions, caps: &Capabilities, description: &PrinterDescription) -> Result<Plan> {
    let mut warnings = Vec::new();
    let mut ticket = PrintTicket {
        job_name: options.job_name.clone(),
        user_name: if options.user_name.is_empty() { "craft".into() } else { options.user_name.clone() },
        must_honor: options.strict,
        shape: options.ticket_shape,
        ..Default::default()
    };
    let strict = options.strict;
    let minimal = options.ticket_shape != TicketShape::Full;
    if minimal {
        let asked = [
            ("copies", options.copies.is_some()),
            ("sides", options.sides.is_some()),
            ("orientation", options.orientation.is_some()),
            ("quality", options.quality.is_some()),
            ("media size", options.media_size.is_some()),
            ("resolution", options.resolution.is_some()),
        ];
        for (what, set) in asked {
            if set && !options.drop_ticket_settings {
                return Err(WsdError::Unsupported(format!(
                    "{what} with a {} ticket (it has no room for settings; remove the setting or drop_ticket_settings)",
                    options.ticket_shape.token()
                )));
            }
            if set {
                warnings.push(format!("the minimal ticket ({}) leaves out {what}", options.ticket_shape.token()));
            }
        }
    }
    if let Some(n) = options.copies {
        // Zero copies is never a request a printer should be asked about, minimal ticket or not.
        if n == 0 {
            return Err(WsdError::Invalid("copies must be at least 1".into()));
        }
    }
    if let Some(n) = options.copies.filter(|_| !minimal) {
        // A count below the printer's published minimum is a mistake in the request, not a count
        // that is "too many": it is refused here and never turned into the maximum.
        if let Some((lo, _)) = caps.copies {
            let lo = lo.max(1);
            if n < lo {
                return Err(WsdError::Invalid(format!("{n} copies is below the printer's minimum of {lo}")));
            }
        }
        match caps.supports_copies(n) {
            Some(false) if !strict => {
                let max = caps.copies.map_or(1, |(_, m)| m).max(1);
                warnings.push(format!("the printer takes at most {max} copies; {n} was lowered to {max}"));
                ticket.copies = Some(max);
            }
            other => {
                if keep(strict, &format!("{n} copies"), other, &mut warnings)? {
                    ticket.copies = Some(n);
                }
            }
        }
    }
    if let Some(s) = options.sides.filter(|_| !minimal)
        && keep(strict, &format!("sides {}", s.token()), caps.supports_sides(s), &mut warnings)?
    {
        ticket.sides = Some(s);
    }
    if let Some(o) = options.orientation.filter(|_| !minimal)
        && keep(strict, &format!("orientation {}", o.token()), caps.supports_orientation(o), &mut warnings)?
    {
        ticket.orientation = Some(o);
    }
    if let Some(q) = options.quality.filter(|_| !minimal)
        && keep(strict, &format!("quality {}", q.token()), caps.supports_quality(q), &mut warnings)?
    {
        ticket.quality = Some(q);
    }
    if let Some(m) = options.media_size.as_ref().filter(|_| !minimal)
        && keep(strict, &format!("media size {m}"), caps.supports_media(m), &mut warnings)?
    {
        ticket.media_size = Some(m.clone());
    }
    if let Some(r) = options.resolution.filter(|_| !minimal)
        && keep(strict, &format!("resolution {}x{}", r.0, r.1), caps.supports_resolution(r), &mut warnings)?
    {
        ticket.resolution = Some(r);
    }
    let dpi = ticket.resolution.or(options.resolution).unwrap_or_else(|| default_dpi(caps));
    let can_color = description.color_supported == Some(true);
    let color = match options.color {
        ColorMode::Mono => false,
        ColorMode::Auto => can_color,
        ColorMode::Color if description.color_supported == Some(false) => {
            if options.strict {
                return Err(WsdError::Unsupported("colour (the printer is monochrome)".into()));
            }
            warnings.push("the printer is monochrome; the document is printed in grey".into());
            false
        }
        ColorMode::Color => true,
    };
    ticket.validate()?;
    Ok(Plan { ticket, color, dpi, warnings })
}

/// Keep a setting? `Some(false)` is "the printer lists this dimension and not this value".
fn keep(strict: bool, what: &str, supported: Option<bool>, warnings: &mut Vec<String>) -> Result<bool> {
    match supported {
        Some(false) if strict => Err(WsdError::Unsupported(what.to_owned())),
        Some(false) => {
            warnings.push(format!("the printer does not list {what}; it was left out"));
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// The listed resolution nearest 300 dpi, else 300.
fn default_dpi(caps: &Capabilities) -> (u32, u32) {
    caps.resolutions.iter().copied().min_by_key(|(w, _)| w.abs_diff(300)).unwrap_or((300, 300))
}

/// Map a terminal state and its reasons to an outcome.
pub fn outcome_of(state: &JobState, reasons: &[JobReason]) -> JobOutcome {
    let has = |r: JobReason| reasons.contains(&r);
    match state {
        JobState::Completed if has(JobReason::JobCompletedWithErrors) => JobOutcome::CompletedWithErrors,
        JobState::Completed if has(JobReason::JobCompletedWithWarnings) => JobOutcome::CompletedWithWarnings,
        JobState::Completed => JobOutcome::Completed,
        JobState::Canceled => JobOutcome::Canceled { at_device: has(JobReason::JobCanceledAtDevice) },
        _ => JobOutcome::Aborted { reason: reasons.iter().find(|r| **r != JobReason::None).cloned() },
    }
}

/// Read what the printer is and what it accepts (no job, no confirmation needed).
pub fn printer_info<T: Transport>(client: &Client<T>) -> Result<PrinterElements> {
    client.get_printer_elements(&[Section::Description, Section::Status, Section::Capabilities])
}

/// The number of phases [`submit_document`] reports: capabilities read, job created, document
/// accepted.
pub const SUBMIT_PHASES: usize = 3;

/// The number of phases [`print_document_with_progress`] reports: the three of [`submit_document`]
/// and the job ended.
pub const PHASES: usize = 4;

/// What a printer accepted: the result of [`submit_document`], before anything is known about the
/// printing itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub job_id: u32,
    /// The format the document was sent in, as the printer spells it.
    pub format: String,
    pub bytes_sent: usize,
    pub printer_name: String,
    /// Settings that were dropped or changed because the printer does not list them.
    pub warnings: Vec<String>,
}

/// Print one document. See the module docs; `confirmed` is the person's yes.
pub fn print_document<T: Transport>(
    client: &Client<T>,
    encoder: &dyn DocumentEncoder,
    options: &PrintOptions,
    confirmed: UserConfirmed,
    cancel: &CancelToken,
) -> Result<JobResult> {
    print_document_with_progress(client, encoder, options, confirmed, cancel, &mut |_, _| {})
}

/// [`print_document`] that also calls `progress(done, PHASES)` after each phase: the printer's
/// capabilities were read (1), the job exists (2), the document is accepted (3), the job reached an
/// end state or the wait ended (4). These are protocol phases, not sheets. It is
/// [`submit_document`] followed by a blocking watch; the backend uses the first half only.
///
/// `cancel` is polled before every phase and while the job is watched. Before the document is
/// sent, a set token is [`WsdError::Cancelled`] (a job that was already created is cancelled on the
/// printer first); once the document is in, a set token sends CancelJob and the printer's end
/// state is the result.
pub fn print_document_with_progress<T: Transport>(
    client: &Client<T>,
    encoder: &dyn DocumentEncoder,
    options: &PrintOptions,
    confirmed: UserConfirmed,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<JobResult> {
    let mut op = Op::Query;
    let accepted = submit_document(client, encoder, options, confirmed, cancel, &mut |done, _| progress(done, PHASES), &mut op)?;
    let watched = watch(client, accepted.job_id, options.poll, cancel)?;
    progress(PHASES, PHASES);
    Ok(JobResult {
        job_id: accepted.job_id,
        outcome: watched.outcome,
        state: watched.status.as_ref().map(|s| s.state.clone()),
        reasons: watched.status.as_ref().map(|s| s.reasons.clone()).unwrap_or_default(),
        format: accepted.format,
        bytes_sent: accepted.bytes_sent,
        sheets_completed: watched.status.as_ref().map_or(0, |s| s.sheets_completed),
        printer_name: accepted.printer_name,
        warnings: accepted.warnings,
    })
}

/// Create the job and send the document, and return as soon as the printer has **accepted** it. No
/// waiting for the job: following it is a separate step ([`Client::job_status`], [`watch_job`]).
/// Calls `progress(done, SUBMIT_PHASES)` after each phase. `op` is set to the operation in flight,
/// so that on an error the caller can classify it ([`WsdError::classify`]): a failed create or send
/// whose outcome is unknown must not be repeated blindly.
///
/// `cancel` is polled before every phase. A job that exists when the token is set, or whose document
/// the printer **refused**, is cancelled on the printer (it would sit there until the document
/// timeout). When the failure leaves the outcome unknown (the connection closed or timed out after
/// the document was written) the job is left alone: cancelling could abort a job that is printing,
/// and a reconcile can find it.
pub fn submit_document<T: Transport>(
    client: &Client<T>,
    encoder: &dyn DocumentEncoder,
    options: &PrintOptions,
    _confirmed: UserConfirmed,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(usize, usize),
    op: &mut Op,
) -> Result<Accepted> {
    let cancelled = || if cancel.is_cancelled() { Err(WsdError::Cancelled) } else { Ok(()) };
    *op = Op::Query;
    cancelled()?;
    let info = printer_info(client)?;
    progress(1, SUBMIT_PHASES);
    let caps = info.capabilities.clone().ok_or_else(|| WsdError::Protocol("the printer did not return its capabilities".into()))?;
    let description = info.description.clone().unwrap_or_default();
    let format = negotiate_format(&encoder.formats(), &caps)?;
    if !caps.accepts_no_compression() {
        return Err(WsdError::Unsupported(format!("uncompressed documents (the printer lists {:?})", caps.compressions)));
    }
    let plan = plan(options, &caps, &description)?;
    let ctx = EncodeContext { color: plan.color, dpi: plan.dpi, sides: plan.ticket.sides, media_size: plan.ticket.media_size.clone() };
    let data = encoder.encode(&format, &ctx)?;
    cancelled()?;
    *op = Op::Create;
    let job_id = client.create_print_job(&plan.ticket)?;
    progress(2, SUBMIT_PHASES);
    if cancel.is_cancelled() {
        // The job exists until the document timeout: do not leave it behind.
        let _ = cancel_job_tolerant(client, job_id);
        return Err(WsdError::Cancelled);
    }
    let desc = DocumentDescription {
        id: 1,
        compression: Compression::None,
        format: format.clone(),
        name: options.document_name.clone(),
        must_honor_format: false,
    };
    *op = Op::Send;
    if let Err(e) = client.send_document(job_id, &desc, &plan.ticket, true, &data, options.transfer) {
        // The job exists until the document timeout. Tidy up when the printer refused the document,
        // not when it may have received it. The real error is kept either way.
        if e.classify(Op::Send).safety != RetrySafety::OutcomeUnknown {
            let _ = client.cancel_job(job_id);
        }
        return Err(e);
    }
    progress(3, SUBMIT_PHASES);
    Ok(Accepted { job_id, format, bytes_sent: data.len(), printer_name: description.name, warnings: plan.warnings })
}

struct Watched {
    outcome: JobOutcome,
    status: Option<JobStatus>,
}

/// CancelJob, treating "too late" as success. A printer that has forgotten the job answers
/// `ClientErrorJobIdNotFound`; one that still remembers a finished job answers `OperationFailed`
/// ("the job already ended"). The first is accepted as it is; the second only when the job's
/// status confirms it is terminal (otherwise the refusal is real and is returned). A transport
/// failure or any other fault is returned.
pub fn cancel_job_tolerant<T: Transport>(client: &Client<T>, job_id: u32) -> Result<()> {
    match client.cancel_job(job_id) {
        Ok(()) => Ok(()),
        Err(WsdError::Fault(f)) if f.code == FaultCode::ClientErrorJobIdNotFound => Ok(()),
        Err(WsdError::Fault(f)) if f.code == FaultCode::OperationFailed => match client.job_status(job_id) {
            Ok(s) if s.state.is_terminal() => Ok(()),
            Err(WsdError::Fault(g)) if g.code == FaultCode::ClientErrorJobIdNotFound => Ok(()),
            _ => Err(WsdError::Fault(f)),
        },
        Err(e) => Err(e),
    }
}

/// Poll until the job reaches a terminal state, the printer forgets it, or the wait runs out.
/// A set cancel token sends CancelJob once and keeps watching for the end state.
pub fn watch_job<T: Transport>(client: &Client<T>, job_id: u32, poll: PollPolicy, cancel: &CancelToken) -> Result<(JobOutcome, Option<JobStatus>)> {
    watch(client, job_id, poll, cancel).map(|w| (w.outcome, w.status))
}

fn watch<T: Transport>(client: &Client<T>, job_id: u32, poll: PollPolicy, cancel: &CancelToken) -> Result<Watched> {
    let started = Instant::now();
    let mut last: Option<JobStatus> = None;
    let mut cancel_sent = false;
    let mut failures = 0u32;
    loop {
        if cancel.is_cancelled() && !cancel_sent {
            cancel_sent = true;
            // A cancel that loses the race with the end of the job is not a failure: the status
            // read below reports how the job really ended.
            cancel_job_tolerant(client, job_id)?;
        }
        match client.job_status(job_id) {
            Ok(status) => {
                failures = 0;
                if status.state.is_terminal() {
                    let outcome = outcome_of(&status.state, &status.reasons);
                    return Ok(Watched { outcome, status: Some(status) });
                }
                last = Some(status);
            }
            Err(WsdError::Fault(f)) if f.code == FaultCode::ClientErrorJobIdNotFound => {
                let outcome = if cancel_sent { JobOutcome::Canceled { at_device: false } } else { JobOutcome::Forgotten };
                return Ok(Watched { outcome, status: last });
            }
            // A transient failure (the condition, not the operation: a status read can always be
            // repeated) is tried a few more times, waiting at least as long as the printer asks.
            Err(e) if e.is_retryable() => {
                failures += 1;
                if failures > 3 {
                    return Err(e);
                }
                if let Some(wait) = e.classify(Op::Status).retry_after {
                    let waited = started.elapsed();
                    std::thread::sleep(wait.min(poll.max_wait.saturating_sub(waited)));
                }
            }
            Err(e) => return Err(e),
        }
        let waited = started.elapsed();
        if waited >= poll.max_wait {
            return Ok(Watched { outcome: JobOutcome::StillRunning, status: last });
        }
        std::thread::sleep(poll.interval.min(poll.max_wait - waited));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Capabilities {
        Capabilities {
            formats: vec!["image/pwg-raster".into(), "application/pdf".into()],
            compressions: vec!["None".into()],
            sides: vec!["OneSided".into(), "TwoSidedLongEdge".into()],
            orientations: vec!["Portrait".into()],
            media_sizes: vec!["iso_a4_210x297mm".into()],
            copies: Some((1, 5)),
            resolutions: vec![(150, 150), (600, 600)],
            ..Default::default()
        }
    }

    fn desc(color: Option<bool>) -> PrinterDescription {
        PrinterDescription { name: "P".into(), color_supported: color, ..Default::default() }
    }

    #[test]
    fn format_negotiation_follows_the_encoders_preference_and_the_printers_spelling() {
        let offered = vec!["application/pdf".to_owned(), "image/pwg-raster".to_owned()];
        assert_eq!(negotiate_format(&offered, &caps()).unwrap(), "application/pdf");
        let only_raster = Capabilities { formats: vec!["Image/PWG-Raster".into()], ..Default::default() };
        assert_eq!(negotiate_format(&offered, &only_raster).unwrap(), "Image/PWG-Raster");
        let e = negotiate_format(&offered, &Capabilities { formats: vec!["image/jpeg".into()], ..Default::default() }).unwrap_err();
        assert!(matches!(e, WsdError::NoCommonFormat { ref accepted, .. } if accepted == &["image/jpeg"]));
        // A printer that lists nothing is not assumed to take PDF.
        assert!(negotiate_format(&offered, &Capabilities::default()).is_err());
    }

    #[test]
    fn a_plan_keeps_supported_settings_and_reports_the_rest() {
        let o = PrintOptions {
            copies: Some(9),
            sides: Some(Sides::TwoSidedShortEdge),
            orientation: Some(Orientation::Portrait),
            media_size: Some("na_letter_8.5x11in".into()),
            resolution: Some((600, 600)),
            quality: Some(Quality::High),
            ..Default::default()
        };
        let p = plan(&o, &caps(), &desc(Some(true))).unwrap();
        assert_eq!(p.ticket.copies, Some(5));
        assert_eq!((p.ticket.sides, p.ticket.media_size.clone()), (None, None));
        assert_eq!(p.ticket.orientation, Some(Orientation::Portrait));
        assert_eq!(p.ticket.resolution, Some((600, 600)));
        assert_eq!(p.ticket.quality, Some(Quality::High)); // not listed: left to the printer
        assert_eq!(p.dpi, (600, 600));
        assert!(p.color);
        assert_eq!(p.warnings.len(), 3, "{:?}", p.warnings);
        assert!(!p.ticket.must_honor);
    }

    #[test]
    fn strict_plans_refuse_instead_and_mark_must_honor() {
        let strict = |o: PrintOptions| plan(&PrintOptions { strict: true, ..o }, &caps(), &desc(Some(true)));
        for o in [
            PrintOptions { copies: Some(6), ..Default::default() },
            PrintOptions { sides: Some(Sides::TwoSidedShortEdge), ..Default::default() },
            PrintOptions { media_size: Some("x".into()), ..Default::default() },
            PrintOptions { resolution: Some((1200, 1200)), ..Default::default() },
        ] {
            assert!(matches!(strict(o), Err(WsdError::Unsupported(_))));
        }
        let ok = strict(PrintOptions { copies: Some(2), ..Default::default() }).unwrap();
        assert!(ok.ticket.must_honor && ok.warnings.is_empty());
        assert!(matches!(
            plan(&PrintOptions { strict: true, color: ColorMode::Color, ..Default::default() }, &caps(), &desc(Some(false))),
            Err(WsdError::Unsupported(_))
        ));
    }

    #[test]
    fn colour_resolves_against_the_printer() {
        let c = |mode, supported| plan(&PrintOptions { color: mode, ..Default::default() }, &caps(), &desc(supported)).unwrap();
        assert!(c(ColorMode::Auto, Some(true)).color);
        assert!(!c(ColorMode::Auto, Some(false)).color && !c(ColorMode::Auto, None).color);
        assert!(!c(ColorMode::Mono, Some(true)).color);
        assert!(c(ColorMode::Color, None).color);
        let forced = c(ColorMode::Color, Some(false));
        assert!(!forced.color && forced.warnings.len() == 1);
    }

    #[test]
    fn the_default_resolution_is_the_listed_one_nearest_300() {
        assert_eq!(default_dpi(&caps()), (150, 150));
        assert_eq!(default_dpi(&Capabilities::default()), (300, 300));
        assert_eq!(default_dpi(&Capabilities { resolutions: vec![(600, 600), (360, 360)], ..Default::default() }), (360, 360));
    }

    #[test]
    fn copies_are_mapped_against_the_published_range() {
        let with = |copies: Option<(u32, u32)>, n: u32, strict: bool| {
            let c = Capabilities { copies, ..Default::default() };
            plan(&PrintOptions { copies: Some(n), strict, ..Default::default() }, &c, &desc(None))
        };
        let invalid = |r: Result<Plan>| matches!(r, Err(WsdError::Invalid(_)));
        // Zero is refused locally with any range, strict or not, and never becomes the maximum.
        for range in [Some((1, 99)), Some((0, 99)), Some((2, 9)), None] {
            for strict in [false, true] {
                assert!(invalid(with(range, 0, strict)), "{range:?} strict={strict}");
            }
        }
        // Below the published minimum: refused, not lowered to the maximum.
        for strict in [false, true] {
            assert!(invalid(with(Some((3, 9)), 2, strict)));
        }
        // Above the maximum: lowered with a warning, or refused in strict mode.
        let p = with(Some((1, 99)), 100, false).unwrap();
        assert_eq!(p.ticket.copies, Some(99));
        assert!(p.warnings.iter().any(|w| w.contains("lowered to 99")), "{:?}", p.warnings);
        assert!(matches!(with(Some((1, 99)), 100, true), Err(WsdError::Unsupported(_))));
        // No published range: sent as is, the printer decides.
        let p = with(None, 7, false).unwrap();
        assert_eq!((p.ticket.copies, p.warnings.len()), (Some(7), 0));
        assert_eq!(with(None, 1, false).unwrap().ticket.copies, Some(1));
        // In range, including the ends.
        assert_eq!(with(Some((2, 9)), 2, true).unwrap().ticket.copies, Some(2));
        assert_eq!(with(Some((2, 9)), 9, true).unwrap().ticket.copies, Some(9));
        assert_eq!(with(Some((1, 99)), 1, false).unwrap().ticket.copies, Some(1));
    }

    #[test]
    fn invalid_values_in_the_options_are_refused() {
        let bad = PrintOptions { copies: Some(0), ..Default::default() };
        assert!(plan(&bad, &Capabilities::default(), &desc(None)).is_err());
        let bad = PrintOptions { media_size: Some("a b".into()), ..Default::default() };
        assert!(plan(&bad, &Capabilities::default(), &desc(None)).is_err());
    }

    #[test]
    fn end_states_map_to_typed_outcomes() {
        use JobReason as R;
        assert_eq!(outcome_of(&JobState::Completed, &[R::JobCompletedSuccessfully]), JobOutcome::Completed);
        assert_eq!(outcome_of(&JobState::Completed, &[]), JobOutcome::Completed);
        assert_eq!(outcome_of(&JobState::Completed, &[R::JobCompletedWithWarnings]), JobOutcome::CompletedWithWarnings);
        assert_eq!(outcome_of(&JobState::Completed, &[R::JobCompletedWithErrors, R::JobCompletedWithWarnings]), JobOutcome::CompletedWithErrors);
        assert_eq!(outcome_of(&JobState::Canceled, &[R::JobCanceledByUser]), JobOutcome::Canceled { at_device: false });
        assert_eq!(outcome_of(&JobState::Canceled, &[R::JobCanceledAtDevice]), JobOutcome::Canceled { at_device: true });
        assert_eq!(outcome_of(&JobState::Aborted, &[R::None, R::DocumentFormatError]), JobOutcome::Aborted { reason: Some(R::DocumentFormatError) });
        assert_eq!(outcome_of(&JobState::Aborted, &[]), JobOutcome::Aborted { reason: None });
        assert_eq!(JobOutcome::Aborted { reason: None }.name(), "aborted");
        assert_eq!(JobReason::parse("vendor:x").name(), "vendor:x");
        assert_eq!(JobReason::parse("JobPrinting").name(), "JobPrinting");
        assert!(
            JobOutcome::CompletedWithWarnings.is_success() && !JobOutcome::CompletedWithErrors.is_success() && !JobOutcome::Forgotten.is_success()
        );
    }

    #[test]
    fn the_cancel_token_is_shared_between_clones() {
        let t = CancelToken::new();
        let u = t.clone();
        assert!(!u.is_cancelled());
        t.cancel();
        assert!(u.is_cancelled());
    }
}
