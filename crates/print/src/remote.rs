//! The backend-neutral view of the jobs a printer or queue holds: [`RemoteJob`], the question a
//! caller asks ([`JobFilter`], [`Fingerprint`]), the answer to "did my earlier attempt create a job?"
//! ([`Reconciled`]) and what a backend can do ([`JobQueryCaps`]).
//!
//! This is the common base for every job source: a spooler queue, an IPP printer, the Windows
//! spooler and WS-Print each have a way to list jobs, and each maps onto these types. A backend
//! opts in through [`Backend::list_jobs`](crate::backends::Backend::list_jobs) and says what it can
//! fill in with [`Backend::job_query_caps`](crate::backends::Backend::job_query_caps); a caller reads
//! those capabilities before relying on a field. See `docs/job-tracking.md` for the mapping of each
//! source.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use craft_job::{Fingerprint, JobId, JobState, Tag, TagSupport, Timestamp};

use crate::tag::{TagExt as _, TagSupportExt as _};

/// One job in a printer's or queue's listing, in neutral terms.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RemoteJob {
    /// The job's id; namespaced when it comes from a [`Registry`](crate::Registry).
    pub id: JobId,
    /// The job name as the printer lists it, tag suffix included when the tag travels in the name.
    pub title: String,
    /// The user the printer lists, when it lists one.
    pub user: Option<String>,
    /// When the job was created, as a wall-clock time, when the source says. Not every source does
    /// (WS-Print job summaries have no creation time); IPP's relative `time-at-creation` is converted
    /// by the adapter or left out.
    pub created: Option<SystemTime>,
    pub state: JobState,
    /// The device's reasons, as reported.
    pub reasons: Vec<String>,
    /// The tag the job carries, if it carries one (any tag, ours or another print's).
    pub tag: Option<Tag>,
    /// The job carries the tag of the [`JobFilter`] it was listed with. `false` for every other job:
    /// foreign jobs are listed too and are never ours.
    pub ours: bool,
    pub sheets_done: Option<u32>,
    pub pages_done: Option<u32>,
}

impl RemoteJob {
    /// A job with this id, title and state. The tag is read from the title's suffix; a backend that
    /// carries it elsewhere sets [`Self::tag`] itself.
    pub fn new(id: JobId, title: impl Into<String>, state: JobState) -> Self {
        let title = title.into();
        let tag = Tag::find_in(&title);
        Self { id, title, user: None, created: None, state, reasons: Vec::new(), tag, ours: false, sheets_done: None, pages_done: None }
    }

    /// Set the owner the listing reports.
    #[must_use]
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set the creation time the listing reports.
    #[must_use]
    pub fn with_created(mut self, created: SystemTime) -> Self {
        self.created = Some(created);
        self
    }

    /// Set the idempotency tag the job carries in a native attribute.
    #[must_use]
    pub fn with_tag(mut self, tag: Tag) -> Self {
        self.tag = Some(tag);
        self
    }

    /// Set the state reasons the listing reports, replacing any already set.
    #[must_use]
    pub fn with_reasons(mut self, reasons: Vec<String>) -> Self {
        self.reasons = reasons;
        self
    }

    /// The title without a tag suffix: what a person typed.
    pub fn display_title(&self) -> &str {
        Tag::strip(&self.title)
    }
}

/// What to list. Everything is optional; an empty filter lists everything the source offers.
///
/// `user`, `title`, `since` and `states` **restrict** the result. `tag` never hides a job: it decides
/// which jobs are marked [`RemoteJob::ours`], because a caller that shows a queue needs the foreign
/// jobs too.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct JobFilter {
    /// Only this user's jobs (jobs whose listing has no user pass: the source does not say).
    pub user: Option<String>,
    /// Only jobs with this title (compared without any tag suffix). A listed title matches when it
    /// equals this one, or when it is a title the printer cut short: a prefix of this one with at
    /// least `MIN_CUT_CHARS` (8) characters. A shorter prefix ("R" for "Report") is some other job.
    pub title: Option<String>,
    /// Only jobs created at or after this time (jobs with no creation time pass).
    pub since: Option<SystemTime>,
    /// Mark the jobs carrying this tag as [`RemoteJob::ours`].
    pub tag: Option<Tag>,
    /// Only jobs in one of these states.
    pub states: Option<Vec<JobState>>,
}

impl JobFilter {
    /// An empty filter, which lists everything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Only jobs of `user` (jobs listed without a user still pass).
    #[must_use]
    pub fn for_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Only jobs titled `title`, ignoring tag suffixes and accepting a printer's cut-short title.
    #[must_use]
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Only jobs created at or after `since` (jobs without a creation time still pass).
    #[must_use]
    pub fn since(mut self, since: SystemTime) -> Self {
        self.since = Some(since);
        self
    }

    /// Mark jobs carrying `tag` as ours; this hides nothing.
    #[must_use]
    pub fn marking(mut self, tag: Tag) -> Self {
        self.tag = Some(tag);
        self
    }

    /// Only jobs in one of `states`.
    #[must_use]
    pub fn in_states(mut self, states: Vec<JobState>) -> Self {
        self.states = Some(states);
        self
    }

    /// Whether `job` passes the restricting parts of the filter (the tag only marks).
    pub fn matches(&self, job: &RemoteJob) -> bool {
        self.user.as_deref().is_none_or(|wanted| job.user.as_deref().is_none_or(|user| user == wanted))
            && self.title.as_deref().is_none_or(|wanted| title_matches(&job.title, wanted))
            && self.since.is_none_or(|since| job.created.is_none_or(|created| created >= since))
            && self.states.as_ref().is_none_or(|states| states.contains(&job.state))
    }
}

/// The fewest characters a listed title needs to count as a printer's cut-short version of a longer
/// one. A printer cuts a title at its own limit, which is tens of characters; a one or two letter
/// title that happens to begin the wanted one is some other job (matching it could make a
/// reconcile accept a foreign job and never resend the lost document).
const MIN_CUT_CHARS: usize = 8;

/// Whether a listed title is the title that was sent: equal without tag suffixes, or a printer's
/// cut-short version of it (at least [`MIN_CUT_CHARS`] characters of it).
pub(crate) fn title_matches(listed: &str, wanted: &str) -> bool {
    let (listed, wanted) = (Tag::strip(listed), Tag::strip(wanted));
    listed == wanted || (listed.chars().count() >= MIN_CUT_CHARS && wanted.starts_with(listed))
}

/// Apply a filter to a listing: marks [`RemoteJob::ours`] from [`JobFilter::tag`] and drops what the
/// restricting parts exclude. Backends use it so every source filters the same way.
pub fn filter_jobs(jobs: Vec<RemoteJob>, filter: &JobFilter) -> Vec<RemoteJob> {
    jobs.into_iter()
        .map(|mut job| {
            job.ours = filter.tag.is_some() && job.tag == filter.tag;
            job
        })
        .filter(|job| filter.matches(job))
        .collect()
}

/// A [`Timestamp`] as the wall-clock time a [`RemoteJob::created`] uses.
fn system_time(at: Timestamp) -> SystemTime {
    // Beyond what the platform's clock can hold (a corrupted journal's timestamp) it saturates to
    // a late time it can hold: a conversion must not panic.
    let mut millis = at.millis();
    loop {
        if let Some(time) = UNIX_EPOCH.checked_add(Duration::from_millis(millis)) {
            return time;
        }
        millis /= 2;
    }
}

/// Printing's reading of a [`Fingerprint`] (`craft-job`'s record of what was submitted): the
/// listing to ask for. The fingerprint's `title`, `user`, `tag`, `submitted` and `window` are
/// `craft-job`'s fields; `title` is the one the app gave, without a tag suffix.
pub trait FingerprintExt {
    /// The earliest creation time to accept: the submission time less the `window` (an allowance for
    /// clock skew between this machine and the printer).
    fn since(&self) -> SystemTime;

    /// The listing to ask for. When the tag can be matched the listing is unrestricted and the tag
    /// decides; otherwise title, user and time narrow it.
    fn filter(&self, tag_matchable: bool) -> JobFilter;
}

impl FingerprintExt for Fingerprint {
    /// The submission time less the fingerprint's window, as wall-clock time.
    fn since(&self) -> SystemTime {
        let window = u64::try_from(self.window.as_millis()).unwrap_or(u64::MAX);
        system_time(Timestamp::from_millis(self.submitted.millis().saturating_sub(window)))
    }

    /// With a matchable tag, an unrestricted listing that marks the tag; otherwise title, user and
    /// creation time restrict it.
    fn filter(&self, tag_matchable: bool) -> JobFilter {
        match (&self.tag, tag_matchable) {
            (Some(tag), true) => JobFilter::new().marking(*tag),
            _ => JobFilter { user: self.user.clone(), title: Some(self.title.clone()), since: Some(self.since()), tag: None, states: None },
        }
    }
}

/// The answer to "did an earlier attempt already create the job?".
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reconciled {
    /// No job matches. Whether that proves the document did not arrive depends on the source:
    /// one that forgets finished jobs ([`JobQueryCaps::history`] is `false`) may have printed it.
    NotFound,
    /// Exactly one job matches: the earlier attempt did create it.
    Found(RemoteJob),
    /// Several match: the same title was printed before and is still listed, or two attempts both
    /// got through. Newest first when the source says when jobs were created.
    Ambiguous(Vec<RemoteJob>),
}

impl Reconciled {
    /// The matching jobs, whichever way they were found.
    pub fn jobs(&self) -> &[RemoteJob] {
        match self {
            Self::NotFound => &[],
            Self::Found(job) => std::slice::from_ref(job),
            Self::Ambiguous(jobs) => jobs,
        }
    }
}

/// Decide a [`Reconciled`] from a listing. With a tag the source can match
/// ([`TagSupport::Carried`]), the jobs carrying our tag are the answer (and are marked `ours`), and
/// none of them is `NotFound`. With a tag the device may rewrite ([`TagSupport::Partial`]), a job
/// that still carries it is the answer too, and a same-title job does not make it ambiguous; only
/// when none does, jobs match by title, user and time, and are never `ours`. Without a tag, the
/// title, user and time are all there is.
pub fn reconcile_jobs(jobs: Vec<RemoteJob>, fingerprint: &Fingerprint, support: TagSupport) -> Reconciled {
    let tag = fingerprint.tag.filter(|_| support.carries());
    let tagged = |jobs: &[RemoteJob]| -> Vec<RemoteJob> {
        jobs.iter()
            .filter(|job| tag.is_some() && job.tag == tag)
            .cloned()
            .map(|mut job| {
                job.ours = true;
                job
            })
            .collect()
    };
    let mut found = tagged(&jobs);
    if found.is_empty() && !(tag.is_some() && support.matchable()) {
        let filter = fingerprint.filter(false);
        found = jobs
            .into_iter()
            .filter(|job| filter.matches(job))
            .map(|mut job| {
                job.ours = false;
                job
            })
            .collect();
    }
    found.sort_by_key(|job| std::cmp::Reverse(job.created));
    match found.len() {
        0 => Reconciled::NotFound,
        1 => found.pop().map_or(Reconciled::NotFound, Reconciled::Found),
        _ => Reconciled::Ambiguous(found),
    }
}

/// What a backend can answer about the jobs in its queue. Read it before relying on a field of
/// [`RemoteJob`]; the default is "nothing".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct JobQueryCaps {
    /// [`Backend::list_jobs`](crate::backends::Backend::list_jobs) works.
    pub list: bool,
    /// Listed jobs carry [`RemoteJob::created`].
    pub created_time: bool,
    /// Listed jobs carry [`RemoteJob::user`].
    pub user: bool,
    /// Listed jobs carry the tag a request put on them ([`RemoteJob::tag`]).
    pub tag: bool,
    /// Finished jobs are listed too, for a while. When `false`, a job that is not listed may have
    /// printed and been forgotten.
    pub history: bool,
}

impl JobQueryCaps {
    /// Nothing can be asked (the default).
    pub const NONE: Self = Self { list: false, created_time: false, user: false, tag: false, history: false };
    /// Everything can be asked.
    pub const ALL: Self = Self { list: true, created_time: true, user: true, tag: true, history: true };
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use craft_job::TagPlacement;

    #[test]
    fn a_timestamp_beyond_the_platforms_clock_saturates_instead_of_panicking() {
        assert_eq!(system_time(Timestamp::from_millis(1_500)), UNIX_EPOCH + Duration::from_millis(1_500));
        assert!(system_time(Timestamp::from_millis(u64::MAX)) > UNIX_EPOCH);
    }

    /// A pending job `p#{n}` of a test backend titled `title`.
    fn job(n: u32, title: &str) -> RemoteJob {
        RemoteJob::new(JobId::new("t", format!("p#{n}")), title, JobState::Pending)
    }

    /// `text` as a tag, or a random one if it does not parse.
    fn tag(text: &str) -> Tag {
        Tag::parse(text).unwrap_or_else(|_| Tag::random())
    }

    /// A fingerprint of `title` submitted at the epoch, with no user or tag.
    fn fingerprint(title: &str) -> Fingerprint {
        Fingerprint::new(title, Timestamp::default())
    }

    #[test]
    fn a_title_suffix_is_read_as_the_jobs_tag() {
        let j = job(1, "Report [ref abc123de]");
        assert_eq!(j.tag, Some(tag("abc123de")));
        assert_eq!(j.display_title(), "Report");
        assert_eq!(job(2, "Report").tag, None);
    }

    #[test]
    fn the_tag_marks_and_never_hides_and_the_other_parts_restrict() {
        let jobs = vec![job(1, "Report [ref abc123de]"), job(2, "Other [ref 00000000]"), job(3, "Report").with_user("someone")];
        let marked = filter_jobs(jobs.clone(), &JobFilter::new().marking(tag("abc123de")));
        assert_eq!(marked.iter().map(|j| j.ours).collect::<Vec<_>>(), [true, false, false]);
        let titled = filter_jobs(jobs.clone(), &JobFilter::new().titled("Report"));
        assert_eq!(titled.len(), 2);
        let user = filter_jobs(jobs, &JobFilter::new().for_user("someone"));
        assert_eq!(user.len(), 3, "jobs with no listed user pass; the source does not say");
    }

    #[test]
    fn time_and_state_restrict_and_an_unknown_creation_time_passes() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let old = job(1, "a").with_created(t0 - Duration::from_secs(10));
        let new = job(2, "a").with_created(t0 + Duration::from_secs(10));
        let none = job(3, "a");
        let f = JobFilter::new().since(t0);
        assert!(!f.matches(&old) && f.matches(&new) && f.matches(&none));
        let s = JobFilter::new().in_states(vec![JobState::Completed]);
        assert!(!s.matches(&new));
    }

    #[test]
    fn a_title_the_printer_cut_short_still_matches() {
        assert!(title_matches("Quarterly", "Quarterly report"));
        assert!(!title_matches("", "Quarterly report"));
        // A tiny prefix is another job's title, not a cut-short one; an exact short title still matches.
        assert!(!title_matches("Q", "Quarterly report"));
        assert!(!title_matches("Quarter", "Quarterly report"));
        assert!(title_matches("Report", "Report"));
        assert!(title_matches("Quarterl", "Quarterly report"));
        assert!(!title_matches("Quarterly reports", "Quarterly report"));
        assert!(title_matches("Report [ref abc123de]", "Report"));
    }

    #[test]
    fn reconcile_by_tag_finds_ours_among_foreign_jobs() {
        let fp = fingerprint("Report").with_tag(tag("abc123de"));
        let support = TagSupport::Carried(TagPlacement::JobNameSuffix);
        let listing = vec![job(1, "Report"), job(2, "Report [ref abc123de]"), job(3, "Report [ref ffffffff]")];
        let result = reconcile_jobs(listing, &fp, support);
        assert!(matches!(&result, Reconciled::Found(found) if found.ours && found.id.native() == "p#2"), "{result:?}");
    }

    #[test]
    fn reconcile_without_a_matchable_tag_falls_back_to_the_title() {
        let fp = fingerprint("Report").with_tag(tag("abc123de"));
        let none = TagSupport::None;
        assert_eq!(reconcile_jobs(vec![job(1, "Other")], &fp, none), Reconciled::NotFound);
        let two = reconcile_jobs(vec![job(1, "Report"), job(2, "Report")], &fp, none);
        assert!(matches!(&two, Reconciled::Ambiguous(jobs) if jobs.len() == 2 && jobs.iter().all(|j| !j.ours)));
        assert!(matches!(reconcile_jobs(vec![job(1, "Report")], &fp, none), Reconciled::Found(j) if !j.ours));
    }

    #[test]
    fn the_listing_asked_for_depends_on_whether_the_tag_can_be_matched() {
        let fp = fingerprint("Report").with_user("u").with_tag(tag("abc123de"));
        let by_tag = fp.filter(true);
        assert_eq!((by_tag.title.is_none(), by_tag.user.is_none(), by_tag.tag.is_some()), (true, true, true));
        let by_title = fp.filter(false);
        assert_eq!((by_title.title.as_deref(), by_title.user.as_deref(), by_title.tag.is_none()), (Some("Report"), Some("u"), true));
        assert_eq!(by_title.since, Some(UNIX_EPOCH), "a submission at the epoch less the window is the epoch");
    }

    #[test]
    fn the_time_bound_is_the_submission_less_the_window() {
        let fp = Fingerprint::new("Report", Timestamp::from_secs(1000)).with_window(Duration::from_secs(120));
        assert_eq!(fp.since(), UNIX_EPOCH + Duration::from_secs(880));
        let early = Fingerprint::new("Report", Timestamp::from_secs(10)).with_window(Duration::from_secs(120));
        assert_eq!(early.since(), UNIX_EPOCH, "saturates at the epoch");
    }
}
