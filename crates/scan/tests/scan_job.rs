//! `ScanJob` driven the way an app drives it: plan an attempt, run the backend, record the result, ask what
//! may happen next. The fake backends are the shared test-only ones (WIA-, SANE- and eSCL-shaped); a small
//! scripted helper covers the failures none of them produces.

mod common;

use std::time::Duration;

use craft_job::{DeviceConnection, DeviceState, JobKind, JobState, RetryAdvice, RetryPolicy, RetrySafety, Severity, TagSupport, Timestamp, Tri};
use craft_scan::job::{
    AttemptOutcome, AttemptPlan, Continuation, ExitClass, Outcome, PageInfo, ScanJob, ScanJobConfig, ScanJobError, ScanJobState, device_view,
};
use craft_scan::{
    Acquired, Backend, Cause, HandlingStatus, InterruptKind, NativeCode, Page, Presence, Registry, ScanError, ScanSettings, Source,
    feeder_shortfall_message,
};

use common::escl_like::{AdfState, ESCL_ID, EsclLike};
use common::sane_like::SaneLike;
use common::wia_like::{WIA_ID, WiaLike};

/// A timestamp `secs` seconds in.
fn t(secs: u64) -> Timestamp {
    Timestamp::from_secs(secs)
}

/// A feeder request for `pages` pages.
fn feeder_request(pages: u32) -> ScanSettings {
    ScanSettings { source: Source::Feeder, pages, ..ScanSettings::default() }
}

/// A 16 by 8 JPEG page whose data is `bytes` zero bytes.
fn page(bytes: usize) -> Page {
    Page {
        bytes: vec![0; bytes],
        format: "jpeg",
        width: 16,
        height: 8,
        dpi: 200,
        dpi_y: 200,
        requested_dpi: 200,
        thresholded: false,
        length: Default::default(),
    }
}

/// What a backend does for a script: push `push` pages through the token, then end with `end`.
fn scripted(plan: &AttemptPlan, push: usize, source: Source, end: Option<ScanError>) -> Result<Acquired, ScanError> {
    let mut pages = Vec::new();
    for _ in 0..push {
        let p = page(8);
        if let Some(progress) = &plan.settings.progress {
            progress.set_source(source);
            progress.push_page(p.clone());
        }
        pages.push(p);
    }
    match end {
        None => Ok(Acquired { source, pages }),
        Some(error) if pages.is_empty() => Err(error),
        Some(error) => Err(ScanError::Incomplete { message: error.to_string(), acquired: Acquired { source, pages }, cause: error.cause() }),
    }
}

/// An error of `kind` with a native code, as a backend would raise it.
fn native(kind: InterruptKind) -> ScanError {
    ScanError::native_error(kind, NativeCode::new("test", 1, None), kind.sentence())
}

// ---- the motivating case: a large feeder scan -----------------------------------------------------------

#[test]
fn a_large_feeder_scan_interrupted_at_page_n_is_continued_by_appending() {
    let mut job = ScanJob::new(feeder_request(20), ScanJobConfig::default().with_title("Contract"));
    assert_eq!(job.state(), ScanJobState::Preparing);
    assert_eq!(job.retry_advice(), RetryAdvice::RetryNow);

    // Attempt 1: the feeder holds 12 sheets, the person asked for 20.
    let plan = job.begin_attempt(t(1)).expect("first attempt");
    assert_eq!((plan.number, plan.settings.pages, plan.kept_pages), (1, 20, 0));
    assert_eq!(job.state(), ScanJobState::Acquiring);
    let result = SaneLike::new(12).acquire(&plan.settings);
    // The app also polled the token while the transfer ran.
    assert_eq!(job.record_progress(&plan.progress), 12, "pages streamed through the token are kept as they arrive");
    job.finish_attempt(result, t(40)).expect("finish");

    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Interrupted);
    assert_eq!((snap.pages.len(), snap.expected_pages), (12, Some(20)), "twelve pages, not twenty-four: streamed pages are not counted twice");
    assert_eq!(snap.last_error.as_ref().map(|f| f.info.kind), Some(InterruptKind::FeederEmpty));
    assert_eq!(snap.advice, RetryAdvice::DoNotRetry, "a person reloads the feeder; nothing is retried by itself");
    assert_eq!(job.retry_advice(), RetryAdvice::DoNotRetry);
    assert!(snap.settled);

    // The default is to ask: pages are kept, so a bare begin_attempt refuses.
    assert_eq!(job.begin_attempt(t(41)).map(|_| ()), Err(ScanJobError::ContinuationRequired { kept: 12 }));

    // The person reloaded eight more sheets and chose to continue.
    let plan = job.begin_attempt_with(t(90), Continuation::Append).expect("append");
    assert_eq!((plan.number, plan.settings.pages, plan.kept_pages, plan.continuation), (2, 8, 12, Continuation::Append));
    let result = SaneLike::new(8).acquire(&plan.settings);
    job.finish_attempt(result, t(120)).expect("finish");

    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::Completed));
    assert_eq!(snap.pages.len(), 20);
    assert_eq!(snap.pages.iter().map(|p| p.number).collect::<Vec<_>>(), (1..=20).collect::<Vec<_>>());
    assert_eq!(snap.pages.iter().filter(|p| p.attempt == 1).count(), 12);
    assert_eq!(snap.pages.iter().filter(|p| p.attempt == 2).count(), 8);
    assert_eq!(snap.attempts_total, 2);
    assert_eq!(snap.attempts[0].outcome, AttemptOutcome::Interrupted);
    assert_eq!((snap.attempts[0].pages_before, snap.attempts[0].pages_gained), (0, 12));
    assert_eq!((snap.attempts[1].pages_before, snap.attempts[1].pages_gained), (12, 8));
    assert_eq!(snap.attempts[1].outcome, AttemptOutcome::Completed);
    assert!(snap.last_error.is_none(), "a clean finish clears the earlier interruption");
    assert_eq!((snap.times.started, snap.times.finished), (Some(t(1)), Some(t(120))));
    assert_eq!(job.take_pages().expect("taken").len(), 20);
    assert_eq!(job.snapshot().pages.len(), 20, "the snapshot still lists pages that were handed over");
}

#[test]
fn an_append_that_delivers_no_pages_fails_and_can_be_started_again() {
    let mut job = ScanJob::new(feeder_request(20), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("first attempt");
    job.finish_attempt(SaneLike::new(12).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted);

    // The feeder was not reloaded: the backend answers Ok with nothing.
    let plan = job.begin_attempt_with(t(3), Continuation::Append).expect("append");
    job.finish_attempt(scripted(&plan, 0, Source::Feeder, None), t(4)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::Failed), "an empty append is a failure, not a short success");
    assert_eq!(snap.attempts[1].outcome, AttemptOutcome::Failed);
    assert_eq!(snap.attempts[1].pages_gained, 0);
    assert_eq!(snap.pages.len(), 12, "the earlier pages are still kept");
    assert!(snap.warnings.is_empty());

    // It can still be continued once the feeder is reloaded.
    let plan = job.begin_attempt_with(t(5), Continuation::Append).expect("append again");
    job.finish_attempt(SaneLike::new(8).acquire(&plan.settings), t(6)).expect("finish");
    assert_eq!(job.snapshot().state, ScanJobState::Terminal(Outcome::Completed));
    assert_eq!(job.snapshot().pages.len(), 20);
}

#[test]
fn an_append_that_adds_nothing_is_not_completed_by_the_feeder_empty_answer() {
    // A whole-feeder job (pages = 0) jams after five sheets.
    let mut job = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("first attempt");
    job.finish_attempt(scripted(&plan, 5, Source::Feeder, Some(native(InterruptKind::Jam))), t(2)).expect("finish");
    assert_eq!((job.state(), job.page_count()), (ScanJobState::Interrupted, 5));

    // The jam is cleared, nothing is reloaded: the backend says the feeder is empty and delivers no page.
    let plan = job.begin_attempt_with(t(3), Continuation::Append).expect("append");
    job.finish_attempt(scripted(&plan, 0, Source::Feeder, Some(native(InterruptKind::FeederEmpty))), t(4)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Interrupted, "the person decides; the unscanned sheets are not written off as a finished scan");
    assert_eq!(snap.advice, RetryAdvice::DoNotRetry);
    assert_eq!(snap.attempts[1].pages_gained, 0);
    assert_ne!(snap.attempts[1].outcome, AttemptOutcome::Completed);

    // A first attempt that empties the feeder still completes.
    let mut job = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(scripted(&plan, 4, Source::Feeder, Some(native(InterruptKind::FeederEmpty))), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Completed));
}

#[test]
fn restart_discards_the_kept_pages_and_asks_for_everything_again() {
    let mut job = ScanJob::new(feeder_request(5), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(2).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.page_count(), 2);
    let plan = job.begin_attempt_with(t(3), Continuation::Restart).expect("restart");
    assert_eq!((plan.settings.pages, plan.kept_pages, plan.continuation), (5, 0, Continuation::Restart));
    assert_eq!(job.page_count(), 0);
    job.finish_attempt(SaneLike::new(5).acquire(&plan.settings), t(4)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Completed));
    assert_eq!(job.page_count(), 5);
}

#[test]
fn the_person_can_finish_with_a_partial_stack() {
    let mut job = ScanJob::new(feeder_request(5), ScanJobConfig::default());
    assert_eq!(job.finish(t(1)), Err(ScanJobError::NotWaiting(ScanJobState::Preparing)));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(3).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.finish(t(3)), Ok(Outcome::CompletedWithWarnings));
    let view = job.view();
    assert_eq!(view.state, JobState::CompletedWithErrors);
    assert!(view.reasons.iter().any(|r| r.contains("3 of 5")));
    assert_eq!(
        job.begin_attempt_with(t(4), Continuation::Append).map(|_| ()),
        Err(ScanJobError::NotStartable(ScanJobState::Terminal(Outcome::CompletedWithWarnings)))
    );
}

#[test]
fn finishing_with_nothing_or_appending_with_everything_is_refused() {
    let mut job = ScanJob::new(feeder_request(2), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(Err(native(InterruptKind::CoverOpen)), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted, "a cover is for a person to close");
    assert_eq!(job.finish(t(3)), Err(ScanJobError::NothingToFinish));
    drop(plan);
    // Two pages kept by an attempt that nevertheless ended in an error: nothing is left to append.
    let plan = job.begin_attempt(t(4)).expect("retry with nothing kept");
    let end = native(InterruptKind::Attention);
    job.finish_attempt(scripted(&plan, 2, Source::Feeder, Some(end)), t(5)).expect("finish");
    assert_eq!(job.page_count(), 2);
    assert_eq!(job.begin_attempt_with(t(6), Continuation::Append).map(|_| ()), Err(ScanJobError::NothingLeft { kept: 2 }));
    assert_eq!(job.finish(t(7)), Ok(Outcome::Completed));
}

// ---- reasons --------------------------------------------------------------------------------------------

#[test]
fn a_jam_in_the_middle_stays_an_error_after_pages() {
    let mut esc = EsclLike::new(AdfState::Loaded, 10);
    esc.jam_after = Some(4);
    let mut job = ScanJob::new(feeder_request(8), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(esc.acquire(&plan.settings), t(5)).expect("finish");

    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Interrupted, "four pages do not make a jam a success");
    assert_eq!(snap.pages.len(), 4);
    let failure = snap.last_error.expect("the jam is the last error");
    assert_eq!(failure.info.kind, InterruptKind::Jam);
    assert_eq!(failure.safety, RetrySafety::Safe, "the device said what stopped it");
    assert!(!failure.transient);
    assert_eq!(snap.advice, RetryAdvice::DoNotRetry);
    let view = job.view();
    assert_eq!(view.state, JobState::NeedsAttention);
    assert_eq!(view.reasons, ["jam"]);
    assert_eq!(view.last_error.as_ref().and_then(|e| e.native).map(|n| n.code), Some(503));

    // The jam is cleared; four sheets are put back and the person continues.
    let plan = job.begin_attempt_with(t(60), Continuation::Append).expect("append");
    assert_eq!(plan.settings.pages, 4);
    job.finish_attempt(EsclLike::new(AdfState::Loaded, 10).acquire(&plan.settings), t(70)).expect("finish");
    assert_eq!((job.state(), job.page_count()), (ScanJobState::Terminal(Outcome::Completed), 8));
}

#[test]
fn cover_open_and_multi_feed_are_interruptions_for_a_person() {
    for (adf, kind) in
        [(AdfState::DoorOpen, InterruptKind::CoverOpen), (AdfState::Multipick, InterruptKind::MultiFeed), (AdfState::Jam, InterruptKind::Jam)]
    {
        let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default());
        let plan = job.begin_attempt(t(1)).expect("attempt");
        job.finish_attempt(EsclLike::new(adf, 3).acquire(&plan.settings), t(2)).expect("finish");
        let snap = job.snapshot();
        assert_eq!((snap.state, snap.last_error.map(|f| f.info.kind)), (ScanJobState::Interrupted, Some(kind)), "{adf:?}");
        assert_eq!(snap.advice, RetryAdvice::DoNotRetry);
    }
}

#[test]
fn an_empty_feeder_with_pages_in_hand_and_nothing_more_wanted_is_a_normal_end() {
    // The whole feeder was asked for (pages = 0): running out is the end.
    let mut job = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    let end = ScanError::native_error(InterruptKind::FeederEmpty, NativeCode::new("sane", 6, None), "no more documents");
    job.finish_attempt(scripted(&plan, 6, Source::Feeder, Some(end)), t(2)).expect("finish");
    assert_eq!((job.state(), job.page_count()), (ScanJobState::Terminal(Outcome::Completed), 6));
    assert!(job.snapshot().last_error.is_none());
    assert_eq!(job.view().state, JobState::Completed);

    // The wanted count was reached, and the driver still said "empty" after it.
    let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    let end = native(InterruptKind::BusyOrFeederEmpty);
    job.finish_attempt(scripted(&plan, 3, Source::Feeder, Some(end)), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Completed));
}

#[test]
fn an_empty_feeder_under_a_whole_feeder_request_is_feeder_empty_not_a_scan_of_nothing() {
    // pages = 0 asks for the whole feeder; SANE's NO_DOCS on the first frame is how the backend learns it is empty.
    let mut job = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    let result = SaneLike::new(0).acquire(&plan.settings);
    assert!(matches!(&result, Err(e) if e.interrupt_kind() == InterruptKind::FeederEmpty), "{result:?}");
    job.finish_attempt(result, t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!((snap.state, snap.pages.len()), (ScanJobState::Interrupted, 0));
    assert_eq!(snap.last_error.map(|f| f.info.kind), Some(InterruptKind::FeederEmpty));

    // A loaded feeder, whole feeder wanted: the end of the stack is the end of the job.
    let mut job = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(4).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!((job.state(), job.page_count()), (ScanJobState::Terminal(Outcome::Completed), 4));
}

#[test]
fn a_page_writer_that_follows_the_documented_loop_keeps_every_page_and_survives_a_restart() {
    // The cursor of docs/scan-job.md section 1: `written` indexes job.pages().
    let mut job = ScanJob::new(feeder_request(6), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(plan.kept_pages, 0);
    let progress = plan.settings.progress.clone().expect("a progress token");
    for _ in 0..2 {
        progress.push_page(page(8));
    }
    assert_eq!(job.record_progress(&plan.progress), 2);
    let mut written = job.pages().len();
    // The worker ends between two polls: two more pages were pushed and the result carries all four.
    let result = scripted(&plan, 2, Source::Feeder, Some(native(InterruptKind::FeederEmpty)));
    assert!(matches!(&result, Err(ScanError::Incomplete { acquired, .. }) if acquired.pages.len() == 2));
    let polled_state = job.snapshot().state;
    job.finish_attempt(result, t(2)).expect("finish");
    assert_eq!(polled_state, ScanJobState::Acquiring, "what the poll loop published while the worker ran");
    assert_eq!(job.snapshot().state, ScanJobState::Interrupted, "finish_attempt changed the state: the loop must publish it once more");
    assert_eq!(job.pages().len(), 4, "finish_attempt added pages the last poll never saw");
    assert_eq!(job.pages()[written..].len(), 2, "so the writer must flush once more after it");
    written = job.pages().len();

    // The person starts over: the plan has no kept pages and the pages are gone, so the cursor must start again.
    let plan = job.begin_attempt_with(t(3), Continuation::Restart).expect("restart");
    assert_eq!(plan.kept_pages, 0);
    assert!(job.pages().is_empty() && written > job.pages().len(), "a stale cursor would index past the end");
    written = 0;
    assert!(job.pages()[written..].is_empty());

    // An app that spills pages between attempts: take_pages() empties job.pages() but the pages stay kept, so an Append
    // plan says kept_pages > 0 and the reset above does not run. The app resets the cursor when it takes the pages.
    job.finish_attempt(scripted(&plan, 3, Source::Feeder, Some(native(InterruptKind::FeederEmpty))), t(4)).expect("finish");
    written = job.pages().len();
    assert_eq!(written, 3);
    assert_eq!(job.take_pages().expect("taken").len(), 3);
    let plan = job.begin_attempt_with(t(5), Continuation::Append).expect("append");
    assert!(plan.kept_pages > 0 && job.pages().is_empty(), "kept_pages does not show that the taken pages are gone");
    assert!(written > job.pages().len(), "a cursor that is not reset after take_pages would index past the end");
    written = 0;
    assert!(job.pages()[written..].is_empty());
}

#[test]
fn a_short_stack_under_a_counted_request_is_an_interruption_the_person_resolves() {
    // The WIA-shaped fake: one sheet in the tray, five wanted; the driver answers busy after page one.
    let mut job = ScanJob::new(feeder_request(5), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    let result = WiaLike::new().acquire(&plan.settings);
    assert!(matches!(&result, Err(ScanError::Incomplete { acquired, .. }) if acquired.pages.len() == 1));
    job.finish_attempt(result, t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!((snap.state, snap.pages.len()), (ScanJobState::Interrupted, 1));
    assert_eq!(snap.last_error.map(|f| f.info.kind), Some(InterruptKind::FeederEmpty));
    assert_eq!(snap.attempts[0].phase, Some(craft_scan::Phase::BetweenPages));
}

// ---- Auto ---------------------------------------------------------------------------------------------

#[test]
fn a_restart_after_an_auto_fallback_asks_for_the_original_request_again() {
    // Auto, 5 pages wanted, the feeder is empty: the job falls back to the glass for one page by itself.
    let request = ScanSettings { source: Source::Auto, pages: 5, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(0).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.snapshot().attempts[0].outcome, AttemptOutcome::FellBackToGlass);
    let plan = job.begin_attempt(t(3)).expect("the fallback attempt");
    assert_eq!((plan.settings.source, plan.settings.pages), (Source::Glass, 1), "the fallback is the glass, one page");

    // The glass attempt stops with the cover open: a person has to act.
    let cover = ScanError::native_error(InterruptKind::CoverOpen, NativeCode::new("test", 1, None), InterruptKind::CoverOpen.sentence());
    job.finish_attempt(Err(cover), t(4)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted);

    // They close it, reload the feeder and start over: that is the whole job again, not the glass fallback.
    let plan = job.begin_attempt_with(t(5), Continuation::Restart).expect("restart");
    assert_eq!((plan.settings.source, plan.settings.pages), (Source::Auto, 5), "the original request");
    assert_eq!(plan.source_guess, Some(Source::Feeder));
    assert_eq!(job.snapshot().expected_pages, Some(5), "the view's total is the original count again");
    assert!(!plan.fell_back_to_glass);
}

#[test]
fn an_empty_feeder_with_no_pages_under_auto_falls_back_to_the_glass_and_records_it() {
    let sane_empty = SaneLike::new(0);
    let request = ScanSettings { source: Source::Auto, pages: 5, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));

    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!((plan.source_requested, plan.source_guess), (Source::Auto, Some(Source::Feeder)), "unknown tray: the feeder is tried first");
    assert_eq!(plan.settings.source, Source::Auto, "Auto is left to the backend");
    job.finish_attempt(sane_empty.acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted);
    assert_eq!(job.retry_advice(), RetryAdvice::RetryNow, "no person needed: the glass is the answer");
    assert_eq!(job.snapshot().attempts[0].outcome, AttemptOutcome::FellBackToGlass);
    assert!(!job.snapshot().settled);
    assert_eq!(job.view().state, JobState::Pending);

    let plan = job.begin_attempt(t(3)).expect("the fallback attempt");
    assert!(plan.fell_back_to_glass);
    assert_eq!((plan.settings.source, plan.settings.pages, plan.number), (Source::Glass, 1, 2));
    job.finish_attempt(sane_empty.acquire(&plan.settings), t(4)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::Completed), "one page from the glass is the job");
    assert_eq!((snap.pages.len(), snap.expected_pages, snap.source_in_use), (1, Some(1), Some(Source::Glass)));
    assert_eq!(snap.pages[0].source, Some(Source::Glass));
    assert_eq!((snap.attempts[1].source_requested, snap.attempts[1].source_used), (Source::Glass, Some(Source::Glass)));
}

#[test]
fn an_auto_that_the_backend_answered_from_the_glass_is_recorded_as_a_fallback() {
    use craft_scan::job::AUTO_FELL_BACK_REASON;
    // The WIA-shaped fake answers Auto from the glass (the driver switched inside the call).
    let wia = WiaLike { bits: 0x3, fail_listing: false };
    let request = ScanSettings { source: Source::Auto, pages: 5, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(plan.source_guess, Some(Source::Feeder));
    assert_eq!(job.snapshot().source_in_use, None, "nothing is known before the attempt ends");
    job.finish_attempt(wia.acquire(&plan.settings), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.source_in_use, Some(Source::Glass));
    assert_eq!((snap.attempts[0].source_used, snap.attempts[0].fell_back), (Some(Source::Glass), true));
    assert_eq!(snap.pages[0].source, Some(Source::Glass));
    assert!(job.view().reasons.iter().any(|r| r == AUTO_FELL_BACK_REASON), "{:?}", job.view().reasons);

    // An Auto that used the feeder, and an explicit glass request, are not fallbacks.
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(1).acquire(&plan.settings), t(2)).expect("finish");
    assert!(!job.snapshot().attempts[0].fell_back);
    let glass = ScanSettings { source: Source::Glass, ..ScanSettings::default() };
    let mut job = ScanJob::new(glass, ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(wia.acquire(&plan.settings), t(2)).expect("finish");
    assert!(!job.snapshot().attempts[0].fell_back);
}

#[test]
fn a_one_page_feeder_request_to_the_wia_fake_is_not_recorded_as_a_glass_fallback() {
    let wia = WiaLike { bits: 0x3, fail_listing: false };
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(wia.acquire(&plan.settings), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!((snap.attempts[0].source_used, snap.attempts[0].fell_back), (Some(Source::Feeder), false));
    assert_eq!(snap.pages[0].source, Some(Source::Feeder));
}

#[test]
fn an_auto_that_the_backend_answered_from_the_glass_completes_with_the_one_page_the_glass_gives() {
    // Five pages wanted, the backend resolves Auto to the glass itself and returns one page: that is the whole scan,
    // exactly as when the job's own fallback sends the retry to the glass.
    let wia = WiaLike { bits: 0x3, fail_listing: false };
    let request = ScanSettings { source: Source::Auto, pages: 5, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(wia.acquire(&plan.settings), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::Completed), "{:?}", snap.state);
    assert_eq!(snap.pages.len(), 1);
    assert_eq!(snap.expected_pages, Some(1));
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
    assert_ne!(snap.state.exit_class(), ExitClass::NeedsAttention);

    // An explicit glass request for several pages keeps its expectation: one page is a shortfall there.
    let glass = ScanSettings { source: Source::Glass, pages: 5, ..ScanSettings::default() };
    let mut job = ScanJob::new(glass, ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(wia.acquire(&plan.settings), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.expected_pages, Some(5));
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::CompletedWithWarnings));
}

#[test]
fn an_append_answered_from_the_glass_after_a_feeder_stop_is_not_a_complete_scan() {
    // Auto wants 20 pages, the feeder stops after 12. The person chooses Append without reloading it, the backend
    // answers Auto from the glass and gives one page: 13 of 20 is a shortfall, never a quiet Completed.
    let request = ScanSettings { source: Source::Auto, pages: 20, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));
    let plan = job.begin_attempt(t(1)).expect("first attempt");
    job.finish_attempt(SaneLike::new(12).acquire(&plan.settings), t(40)).expect("finish");
    assert_eq!(job.snapshot().state, ScanJobState::Interrupted);
    assert_eq!(job.snapshot().pages.len(), 12);

    let plan = job.begin_attempt_with(t(90), Continuation::Append).expect("append");
    let wia = WiaLike { bits: 0x3, fail_listing: false };
    job.finish_attempt(wia.acquire(&plan.settings), t(120)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.pages.len(), 13);
    assert_eq!(snap.expected_pages, Some(20), "the glass answer to an append does not shrink what was asked for");
    assert_ne!(snap.state, ScanJobState::Terminal(Outcome::Completed), "{:?}", snap.state);
}

#[test]
fn a_journal_from_a_newer_format_seeds_an_unknown_outcome() {
    use craft_scan::job::ScanJobJournal;
    let journal_after = |have: usize, wanted: u32| {
        let mut job = ScanJob::new(feeder_request(wanted), ScanJobConfig::default());
        let plan = job.begin_attempt(t(1)).expect("attempt");
        job.finish_attempt(SaneLike::new(have).acquire(&plan.settings), t(2)).expect("finish");
        job.snapshot().into_journal()
    };
    let seed = |journal: ScanJobJournal| ScanJob::continuing_from(feeder_request(20), ScanJobConfig::default(), journal);

    // At our own version an interrupted journal resumes as interrupted, as before.
    let current = journal_after(12, 20);
    assert_eq!((current.version, current.state), (ScanJobJournal::VERSION, ScanJobState::Interrupted));
    let job = seed(current.clone());
    assert_eq!(job.state(), ScanJobState::Interrupted);
    assert_ne!(job.retry_advice(), RetryAdvice::CheckDeviceFirst);

    // A newer version is never trusted: Interrupted and Terminal both seed NeedsAttention, check the device first.
    let mut newer = current;
    newer.version = ScanJobJournal::VERSION + 1;
    let job = seed(newer);
    assert_eq!(job.state(), ScanJobState::NeedsAttention);
    assert_eq!(job.retry_advice(), RetryAdvice::CheckDeviceFirst);
    assert_eq!(job.page_count(), 12);

    let mut done = journal_after(3, 3);
    assert_eq!(done.state, ScanJobState::Terminal(Outcome::Completed));
    done.version = ScanJobJournal::VERSION + 1;
    let job = seed(done);
    assert_eq!(job.state(), ScanJobState::NeedsAttention);
    assert_eq!(job.retry_advice(), RetryAdvice::CheckDeviceFirst);
}

#[test]
fn a_glass_guess_alone_does_not_shrink_the_expected_count() {
    // The feeder is known empty, so Auto is guessed to use the glass; the attempt then fails before the backend says
    // which source it used. The guess is not an answer: the request for 20 pages stands for the retry.
    let request = ScanSettings { source: Source::Auto, pages: 20, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));
    job.observe_status(EsclLike::new(AdfState::Empty, 0).status(ESCL_ID));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(plan.source_guess, Some(Source::Glass));
    job.finish_attempt(Err(ScanError::Failed("the scanner stopped answering".into())), t(2)).expect("finish");
    assert_eq!(job.snapshot().expected_pages, Some(20), "a guessed glass is not a reported glass");
}

#[test]
fn a_glass_answer_that_kept_no_page_does_not_shrink_the_expected_count() {
    use craft_scan::Acquired;
    // The backend names the glass but the attempt ends with nothing kept: there is no one-page scan to complete, so
    // the request for 20 pages stands for the retry.
    let request = ScanSettings { source: Source::Auto, pages: 20, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default().with_capabilities(true, true, None));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    let error =
        ScanError::Incomplete { message: "the glass stopped".into(), acquired: Acquired { source: Source::Glass, pages: Vec::new() }, cause: None };
    job.finish_attempt(Err(error), t(2)).expect("finish");
    let _ = plan;
    assert_eq!(job.snapshot().expected_pages, Some(20), "no page was kept from the glass");
}

#[test]
fn a_journal_with_no_pages_keeps_its_attempt_count_error_and_warnings() {
    use craft_scan::job::ScanJobJournal;
    // The first attempt found an empty feeder and no glass: nothing kept, interrupted, one attempt made.
    let request = ScanSettings { source: Source::Auto, pages: 5, ..ScanSettings::default() };
    let mut job = ScanJob::new(request.clone(), ScanJobConfig::default().with_capabilities(true, false, None));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(0).acquire(&plan.settings), t(2)).expect("finish");
    let mut journal: ScanJobJournal = job.snapshot().into_journal();
    assert_eq!((journal.state, journal.pages.len(), journal.attempts_total), (ScanJobState::Interrupted, 0, 1));
    assert!(journal.last_error.is_some());
    journal.warnings.push("kept for the next run".into());

    let resumed = ScanJob::continuing_from(request, ScanJobConfig::default(), journal);
    let snap = resumed.snapshot();
    assert_eq!(snap.state, ScanJobState::Interrupted, "an attempt is on record: it reads as the journal did, not as a fresh job");
    assert_eq!(resumed.retry_advice(), RetryAdvice::DoNotRetry, "a person decides; nothing starts by itself");
    assert_eq!(snap.attempts_total, 1, "the next attempt must not reuse number 1");
    assert!(snap.last_error.is_some());
    assert_eq!(snap.warnings, ["kept for the next run"]);
    // Written again before any attempt, the journal still reads Interrupted, not as cut off mid-scan.
    assert_eq!(snap.into_journal().effective_state(), ScanJobState::Interrupted);
}

#[test]
fn a_journal_that_never_attempted_anything_resumes_as_a_fresh_job() {
    use craft_scan::job::ScanJobJournal;
    let request = ScanSettings { pages: 5, ..ScanSettings::default() };
    let journal: ScanJobJournal = ScanJob::new(request.clone(), ScanJobConfig::default()).snapshot().into_journal();
    assert_eq!((journal.state, journal.attempts_total), (ScanJobState::Preparing, 0));
    let resumed = ScanJob::continuing_from(request, ScanJobConfig::default(), journal);
    assert_eq!(resumed.state(), ScanJobState::Preparing);
    assert_eq!(resumed.retry_advice(), RetryAdvice::RetryNow);
}

#[test]
fn auto_does_not_fall_back_without_a_glass_or_after_pages_or_for_a_jam() {
    let auto = || ScanSettings { source: Source::Auto, pages: 5, ..ScanSettings::default() };

    let mut job = ScanJob::new(auto(), ScanJobConfig::default().with_capabilities(true, false, None));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(0).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.snapshot().attempts[0].outcome, AttemptOutcome::Interrupted, "no glass to fall back to");
    assert_eq!(job.retry_advice(), RetryAdvice::DoNotRetry);

    let mut job = ScanJob::new(auto(), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(SaneLike::new(2).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.snapshot().attempts[0].outcome, AttemptOutcome::Interrupted, "a shortfall after pages stays an error");

    let mut job = ScanJob::new(auto(), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(EsclLike::new(AdfState::Jam, 3).acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.snapshot().attempts[0].outcome, AttemptOutcome::Interrupted, "a jam is never answered with the glass");
}

#[test]
fn a_known_empty_feeder_makes_auto_start_on_the_glass_guess() {
    let request = ScanSettings { source: Source::Auto, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, ScanJobConfig::default());
    job.observe_status(EsclLike::new(AdfState::Empty, 0).status(ESCL_ID));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(plan.source_guess, Some(Source::Glass));
    // Neither source at all: the job fails without an attempt.
    let mut job = ScanJob::new(ScanSettings::default(), ScanJobConfig::default().with_capabilities(false, false, None));
    assert!(matches!(job.begin_attempt(t(1)), Err(ScanJobError::NoSource(_))));
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Failed));
}

// ---- unknown outcomes -----------------------------------------------------------------------------------

#[test]
fn an_error_after_pages_with_no_job_end_needs_attention_and_is_never_retried_by_itself() {
    // A policy that would retry anything transient, many times.
    let eager = RetryPolicy { max_attempts: 50, base: Duration::ZERO, ..RetryPolicy::default() };
    let mut job = ScanJob::new(feeder_request(10), ScanJobConfig::default().with_retry(eager));
    let plan = job.begin_attempt(t(1)).expect("attempt");
    // The link dropped after three pages were delivered and before the job ended.
    job.finish_attempt(scripted(&plan, 3, Source::Feeder, Some(native(InterruptKind::Offline))), t(2)).expect("finish");

    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::NeedsAttention);
    assert_eq!(snap.pages.len(), 3, "pages survive");
    let failure = snap.last_error.expect("error");
    assert_eq!((failure.safety, failure.transient), (RetrySafety::OutcomeUnknown, false));
    assert_eq!(job.retry_advice(), RetryAdvice::CheckDeviceFirst);
    assert!(!job.retry_advice().is_automatic_retry());
    assert_eq!(snap.attempts[0].outcome, AttemptOutcome::OutcomeUnknown);
    let view = job.view();
    assert_eq!(view.state, JobState::NeedsAttention);
    assert!(view.reasons.contains(&"outcome_unknown".to_string()));
    assert_eq!(view.last_error.and_then(|e| e.retry_advice), Some(RetryAdvice::CheckDeviceFirst));

    // Only an explicit choice goes on; the person looked at the scanner and says what they found.
    assert_eq!(job.begin_attempt(t(3)).map(|_| ()), Err(ScanJobError::ContinuationRequired { kept: 3 }));
    let plan = job.begin_attempt_with(t(4), Continuation::Append).expect("append");
    assert_eq!(plan.settings.pages, 7);
}

#[test]
fn a_failure_in_the_middle_of_a_page_is_unknown_even_with_nothing_kept() {
    let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    plan.progress.set_transfer(Some(40), 4096, None); // a page was part-way across
    job.record_progress(&plan.progress);
    assert_eq!(job.snapshot().in_flight.map(|f| f.bytes_received), Some(4096));
    job.finish_attempt(Err(native(InterruptKind::Offline)), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::NeedsAttention);
    assert_eq!(snap.attempts[0].phase, Some(craft_scan::Phase::MidPage));
    assert!(snap.in_flight.is_none());
    assert_eq!(job.retry_advice(), RetryAdvice::CheckDeviceFirst);
}

#[test]
fn a_start_over_by_a_person_gets_a_fresh_retry_budget_even_with_nothing_kept() {
    let policy = RetryPolicy { max_attempts: 3, base: Duration::from_millis(100), jitter_percent: 0, ..RetryPolicy::default() };
    let first_retry = RetryAdvice::RetryAfter(Duration::from_millis(100));
    // The budget is used up and the job is Failed; the person starts it again by hand.
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default().with_retry(policy));
    for n in 1..=3 {
        job.begin_attempt(t(n)).expect("attempt");
        job.finish_attempt(Err(native(InterruptKind::Busy)), t(n)).expect("finish");
    }
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Failed));
    job.begin_attempt(t(10)).expect("manual start");
    job.finish_attempt(Err(native(InterruptKind::Busy)), t(10)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted, "a busy scanner is retried again, not given up on at once");
    assert_eq!(job.retry_advice(), first_retry);

    // Two failures on record, then the person names Restart: the count starts again.
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default().with_retry(policy));
    for n in 1..=2 {
        job.begin_attempt(t(n)).expect("attempt");
        job.finish_attempt(Err(native(InterruptKind::Busy)), t(n)).expect("finish");
    }
    assert_eq!(job.state(), ScanJobState::Interrupted);
    job.begin_attempt_with(t(10), Continuation::Restart).expect("restart");
    job.finish_attempt(Err(native(InterruptKind::Busy)), t(10)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted);
    assert_eq!(job.retry_advice(), first_retry);
}

#[test]
fn an_unreachable_scanner_before_any_page_is_safe_to_retry_within_the_budget() {
    let policy = RetryPolicy { max_attempts: 3, base: Duration::from_millis(100), jitter_percent: 0, ..RetryPolicy::default() };
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default().with_retry(policy));
    for (n, expected) in [(1, RetryAdvice::RetryAfter(Duration::from_millis(100))), (2, RetryAdvice::RetryAfter(Duration::from_millis(200)))] {
        let plan = job.begin_attempt(t(n)).expect("attempt");
        job.finish_attempt(Err(native(InterruptKind::Busy)), t(n)).expect("finish");
        assert_eq!(job.state(), ScanJobState::Interrupted);
        assert_eq!(job.retry_advice(), expected, "after failure {n}");
        assert_eq!(plan.number, n as u32);
    }
    // The third failure uses the budget up: RetryPolicy::max_attempts counts tries, the first included.
    job.begin_attempt(t(3)).expect("attempt");
    job.finish_attempt(Err(native(InterruptKind::Busy)), t(3)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Failed));
    assert_eq!(job.retry_advice(), RetryAdvice::DoNotRetry);
    assert_eq!(job.view().state, JobState::Aborted);
    // Failed can be started over by hand; the history stays.
    let plan = job.begin_attempt(t(10)).expect("manual retry");
    assert_eq!(plan.number, 4);
    assert_eq!(job.snapshot().attempts.len(), 4);
}

#[test]
fn a_disabled_policy_never_retries_and_a_device_hint_is_honoured() {
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default().with_retry(RetryPolicy::none()));
    job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(Err(native(InterruptKind::Busy)), t(1)).expect("finish");
    assert_eq!((job.state(), job.retry_advice()), (ScanJobState::Terminal(Outcome::Failed), RetryAdvice::DoNotRetry));

    // Busy with pages kept is never repeated on its own: paper position is unknown.
    let mut job = ScanJob::new(feeder_request(5), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(scripted(&plan, 2, Source::Feeder, Some(native(InterruptKind::Busy))), t(2)).expect("finish");
    assert_eq!((job.state(), job.retry_advice()), (ScanJobState::Interrupted, RetryAdvice::DoNotRetry));
}

#[test]
fn a_failure_nothing_can_cure_is_failed_at_once() {
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default());
    job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(Err(ScanError::Unsupported), t(1)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Failed));
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default());
    job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(Err(ScanError::Failed("dpi is 50 to 1200".into())), t(1)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Failed));
    assert_eq!(job.snapshot().last_error.map(|f| f.info.kind), Some(InterruptKind::Other));
}

// ---- cancel -------------------------------------------------------------------------------------------

#[test]
fn cancel_during_an_attempt_requests_the_token_and_ends_cancelled_with_the_pages() {
    let mut job = ScanJob::new(feeder_request(10), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    scripted(&plan, 2, Source::Feeder, None).expect("two pages arrive");
    job.cancel(t(2));
    assert!(plan.cancel.is_requested(), "the worker's token is set");
    assert_eq!(job.state(), ScanJobState::Acquiring, "the attempt has not returned yet");
    assert!(job.snapshot().cancel_requested);
    // The worker returns whatever it returns; the job is cancelled either way.
    job.finish_attempt(Err(ScanError::Failed("the scan was cancelled".into())), t(3)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::Cancelled));
    assert_eq!((snap.pages.len(), snap.attempts[0].outcome), (2, AttemptOutcome::Cancelled), "pages stream in through the token and are kept");
    assert_eq!(job.view().state, JobState::Cancelled);
    job.cancel(t(4));
    assert_eq!(job.snapshot().times.finished, Some(t(3)), "cancel on an ended job does nothing");
}

#[test]
fn cancel_while_waiting_ends_at_once_and_a_device_cancel_is_a_different_end() {
    let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default());
    job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(Err(native(InterruptKind::Jam)), t(2)).expect("finish");
    job.cancel(t(3));
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Cancelled));
    assert_eq!(job.begin_attempt(t(4)).map(|_| ()), Err(ScanJobError::NotStartable(ScanJobState::Terminal(Outcome::Cancelled))));

    let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    scripted(&plan, 1, Source::Feeder, None).expect("one page");
    job.device_cancelled(t(2)).expect("device cancel");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::CancelledByDevice));
    assert_eq!((snap.pages.len(), snap.attempts[0].outcome), (1, AttemptOutcome::CancelledByDevice));
    assert!(!snap.cancel_requested, "this side did not ask for it");
    let view = job.view();
    assert_eq!(view.state, JobState::Cancelled);
    assert_eq!(view.reasons, ["cancelled_by_device"]);
    assert_eq!(job.device_cancelled(t(3)), Err(ScanJobError::NotAcquiring));

    let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default());
    job.cancel(t(1));
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Cancelled), "before the first attempt");
}

// ---- pages the app drains itself ------------------------------------------------------------------------

#[test]
fn pages_are_kept_when_the_result_has_none_and_not_doubled_when_it_has_them() {
    let mut job = ScanJob::new(feeder_request(4), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    // The app drained the token itself and gave the job the pages.
    plan.progress.push_page(page(8));
    plan.progress.push_page(page(8));
    let drained = plan.progress.take_pages();
    assert_eq!(job.absorb_pages(drained), 2);
    plan.progress.push_page(page(8)); // never drained by anyone
    job.finish_attempt(Err(ScanError::Failed("the scanner stopped answering".into())), t(2)).expect("finish");
    assert_eq!(job.page_count(), 3, "the undrained page is taken from the token at the end");
    assert_eq!(job.pages().len(), 3);
    assert_eq!(job.snapshot().attempts[0].pages_gained, 3);
}

#[test]
fn taking_pages_waits_for_the_attempt_and_append_still_counts_them() {
    let mut job = ScanJob::new(feeder_request(4), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(job.take_pages(), Err(ScanJobError::AttemptRunning));
    job.finish_attempt(SaneLike::new(2).acquire(&plan.settings), t(2)).expect("finish");
    let first = job.take_pages().expect("taken");
    assert_eq!(first.len(), 2);
    let plan = job.begin_attempt_with(t(3), Continuation::Append).expect("append");
    assert_eq!(plan.settings.pages, 2, "taken pages are still kept pages");
    job.finish_attempt(SaneLike::new(2).acquire(&plan.settings), t(4)).expect("finish");
    assert_eq!(job.take_pages().expect("taken").len(), 2);
    assert_eq!(job.page_count(), 4);
}

// ---- snapshot, view -------------------------------------------------------------------------------------

#[test]
fn snapshot_and_view_carry_no_page_bytes_and_the_fields_are_right() {
    let request = ScanSettings { device: format!("wia:{WIA_ID}"), source: Source::Feeder, pages: 5, ..ScanSettings::default() };
    let config = ScanJobConfig::default().with_title("Invoices").with_created(t(100)).with_tag_support(TagSupport::None);
    let mut job = ScanJob::new(request, config);
    let plan = job.begin_attempt(t(101)).expect("attempt");
    // Three pages of a megabyte each.
    let big = || Page {
        bytes: vec![7; 1 << 20],
        format: "png",
        width: 2000,
        height: 3000,
        dpi: 300,
        dpi_y: 300,
        requested_dpi: 300,
        thresholded: false,
        length: Default::default(),
    };
    for _ in 0..3 {
        plan.progress.push_page(big());
    }
    assert_eq!(job.record_progress(&plan.progress), 3);
    job.finish_attempt(Err(native(InterruptKind::Jam)), t(110)).expect("finish");

    let snap = job.snapshot();
    assert_eq!((snap.pages.len(), snap.bytes), (3, 3 << 20));
    assert_eq!(snap.pages[0].bytes, 1 << 20, "the size is listed, the bytes are not");
    assert!(format!("{snap:?}").len() < 8 * 1024, "a snapshot of 3 MiB of pages debug-prints small: {}", format!("{snap:?}").len());
    assert_eq!(snap.tag_support, TagSupport::None);
    assert_eq!(snap.fingerprint.title, "Invoices");
    assert!(snap.fingerprint.tag.is_none());

    let view = job.view();
    assert!(format!("{view:?}").len() < 8 * 1024);
    assert_eq!((view.id.backend(), view.id.native()), ("wia", format!("local-{}", job.id().get()).as_str()));
    assert_eq!((view.kind, view.title.as_str(), view.state), (JobKind::Scan, "Invoices", JobState::NeedsAttention));
    assert_eq!(view.progress.map(|p| (p.done, p.total)), Some((3, Some(5))));
    assert_eq!(view.progress.and_then(|p| p.fraction()), Some(0.6));
    assert_eq!(view.documents.len(), 3);
    assert_eq!(view.documents.total_pages(), Some(3));
    assert_eq!(view.documents.total_bytes(), Some(3 << 20));
    assert_eq!(view.documents.first().and_then(|d| d.format.as_deref()), Some("png"));
    assert_eq!(view.times.created, Some(t(100)));
    assert_eq!((view.times.started, view.times.finished), (Some(t(101)), None));
    assert_eq!(view.attempts, 1);
    assert!(view.tag.is_none(), "no backend carries a tag");
    let error = view.last_error.clone().expect("last error");
    assert_eq!((error.kind_label.as_str(), error.retry_advice), ("jam", Some(RetryAdvice::DoNotRetry)));
    assert_eq!(error.native.map(|n| (n.domain, n.code)), Some(("test", 1)));
    assert!(view.needs_attention());
}

#[test]
fn views_of_an_unbound_job_use_the_scan_prefix_and_an_unbounded_total() {
    let job = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    let view = job.view();
    assert_eq!(view.id.backend(), "scan");
    assert_eq!((view.state, view.progress.map(|p| (p.done, p.total))), (JobState::Pending, Some((0, None))));
    assert_eq!(view.progress.and_then(|p| p.fraction()), None);
    assert!(view.documents.is_empty());
    let other = ScanJob::new(feeder_request(0), ScanJobConfig::default());
    assert_ne!(job.id(), other.id(), "local ids are process-unique");
}

#[test]
fn the_timeline_is_bounded_but_the_count_is_not() {
    let mut job = ScanJob::new(
        feeder_request(1),
        ScanJobConfig::default().with_timeline_len(3).with_retry(RetryPolicy { max_attempts: 100, ..RetryPolicy::default() }),
    );
    for n in 1..=7u64 {
        job.begin_attempt(t(n)).expect("attempt");
        job.finish_attempt(Err(native(InterruptKind::Busy)), t(n)).expect("finish");
    }
    let snap = job.snapshot();
    assert_eq!((snap.attempts_total, snap.attempts.len()), (7, 3));
    assert_eq!(snap.attempts.iter().map(|a| a.number).collect::<Vec<_>>(), [5, 6, 7]);
    assert_eq!(job.view().attempts, 7);
}

#[test]
fn an_attempt_cannot_be_planned_or_finished_out_of_turn() {
    let mut job = ScanJob::new(feeder_request(1), ScanJobConfig::default());
    assert_eq!(job.finish_attempt(Err(ScanError::Unsupported), t(1)), Err(ScanJobError::NotAcquiring));
    job.begin_attempt(t(1)).expect("attempt");
    assert_eq!(job.begin_attempt(t(2)).map(|_| ()), Err(ScanJobError::NotStartable(ScanJobState::Acquiring)));
    assert_eq!(job.record_progress(&craft_scan::ProgressToken::new()), 0);
    assert_eq!(job.retry_advice(), RetryAdvice::DoNotRetry, "in flight");
}

#[test]
fn a_shortfall_message_still_reads_as_a_feeder_end() {
    // The sentence the shared helpers build is classified from its text when an error carries no kind.
    let mut job = ScanJob::new(feeder_request(4), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    let error = ScanError::Incomplete {
        message: feeder_shortfall_message(4, 2),
        acquired: Acquired { source: Source::Feeder, pages: vec![page(4), page(4)] },
        cause: None::<Cause>,
    };
    drop(plan);
    job.finish_attempt(Err(error), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Interrupted);
    assert_eq!(job.snapshot().last_error.map(|f| f.info.kind), Some(InterruptKind::FeederEmpty));
}

// ---- the registry route ---------------------------------------------------------------------------------

#[test]
fn a_job_runs_through_a_registry_and_reports_no_tag_support() {
    let registry = Registry::new();
    registry.register(Box::new(WiaLike::new())).expect("register");
    registry.register(Box::new(SaneLike::new(1))).expect("register");
    let device = format!("wia:{WIA_ID}");
    assert_eq!(registry.tag_support(&device), Ok(TagSupport::None));
    assert_eq!(registry.tag_support("nope:1").err().and_then(|e| e.native()), Some(craft_scan::UNKNOWN_BACKEND));
    let scanner = registry.scanner_details(&device).expect("details");
    let config = ScanJobConfig::default().with_scanner(&scanner);
    let request = ScanSettings { device, source: Source::Glass, ..ScanSettings::default() };
    let mut job = ScanJob::new(request, config);
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(registry.acquire(&plan.settings), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Completed));
    assert_eq!(job.view().device_name.as_deref(), Some("Fake WIA scanner"));
    assert_eq!(job.view().id.backend(), "wia");
}

#[test]
fn a_scanner_whose_units_were_not_read_may_still_have_a_feeder_or_glass() {
    // A backend that never read the handling bits leaves feeder and flatbed false: unknown, not absent.
    let mut unread = common::scanner("x:1", "Unread");
    unread.flatbed = false;
    assert_eq!((unread.feeder_presence(), unread.flatbed_presence()), (Presence::Unknown, Presence::Unknown));
    let config = ScanJobConfig::default().with_scanner(&unread);
    assert!(config.has_feeder && config.has_glass, "unknown is not absent");
    let mut job = ScanJob::new(ScanSettings { device: "x:1".into(), source: Source::Auto, ..ScanSettings::default() }, config);
    let plan = job.begin_attempt(t(1)).expect("Auto finds a source to try");
    assert_eq!(plan.source_guess, Some(Source::Feeder), "the feeder is tried first; an empty one is learned from the failure");

    // The bits were read and there is no feeder: the glass.
    let mut read = common::scanner("x:2", "Read");
    read.reported.units = true;
    assert_eq!((read.feeder_presence(), read.flatbed_presence()), (Presence::Absent, Presence::Present));
    let config = ScanJobConfig::default().with_scanner(&read);
    assert!(!config.has_feeder && config.has_glass);
    let mut job = ScanJob::new(ScanSettings { device: "x:2".into(), source: Source::Auto, ..ScanSettings::default() }, config);
    assert_eq!(job.begin_attempt(t(1)).expect("attempt").source_guess, Some(Source::Glass));

    // The bits were read and the scanner has neither: nothing to scan from, reported at the start.
    let mut neither = common::scanner("x:3", "Neither");
    neither.flatbed = false;
    neither.reported.units = true;
    let config = ScanJobConfig::default().with_scanner(&neither);
    assert!(!config.has_feeder && !config.has_glass);
    let mut job = ScanJob::new(ScanSettings { device: "x:3".into(), source: Source::Auto, ..ScanSettings::default() }, config);
    assert!(job.begin_attempt(t(1)).is_err() || job.state().is_terminal());
}

// ---- device_view -------------------------------------------------------------------------------------

/// The state of the indicator `key` in `view`.
fn state_of(view: &craft_job::DeviceView, key: craft_job::IndicatorKey) -> Option<Tri> {
    view.indicators.iter().find(|i| i.key == key).map(|i| i.state)
}

#[test]
fn a_wia_status_maps_bit_for_bit() {
    use craft_job::IndicatorKey as K;
    let wia = WiaLike { bits: 0x1 | 0x2 | 0x20 | 0x800, fail_listing: false };
    let mut scanner = wia.scanner_details(WIA_ID).expect("details");
    // A bare backend id has no registry prefix; the registry adds one.
    assert!(device_view(&scanner, None, None).id.to_string().starts_with("unknown:"));
    scanner.id = format!("wia:{WIA_ID}");
    let view = device_view(&scanner, wia.status(WIA_ID), Some(Source::Feeder));
    assert_eq!(view.kind, JobKind::Scan);
    assert_eq!(view.connection, DeviceConnection::Network, "the WSD transport is a network scanner");
    assert_eq!(view.name, "Fake WIA scanner");
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::On));
    assert_eq!(state_of(&view, K::FlatbedReady), Some(Tri::On));
    assert_eq!(state_of(&view, K::Jam), Some(Tri::On));
    assert_eq!(state_of(&view, K::LampError), Some(Tri::On));
    assert_eq!(state_of(&view, K::CoverOpen), Some(Tri::Off));
    assert_eq!(state_of(&view, K::MultiFeed), Some(Tri::Off));
    assert_eq!(state_of(&view, K::DuplexReady), None, "no duplexer, no indicator");
    assert_eq!(view.worst_severity(), Severity::Error);
    assert_eq!(view.state, DeviceState::Attention);
    let paper = view.paper.expect("paper path");
    assert_eq!((paper.feeder_loaded, paper.glass_available), (Tri::On, Tri::On));
    assert_eq!(paper.source_in_use, Some(craft_job::PaperSource::Feeder));
    assert_eq!(view.capabilities.has_feeder, Tri::On);
    assert_eq!(view.capabilities.has_flatbed, Tri::On);
    assert_eq!(view.capabilities.duplex, Tri::Off);
    assert_eq!(view.id.to_string(), format!("wia:{WIA_ID}"));
}

#[test]
fn a_clear_lamp_bit_is_not_reported_never_off() {
    use craft_job::IndicatorKey as K;
    // Other bits are set, so the status is a real reading; the lamp bit is an error flag with no readback.
    let wia = WiaLike { bits: 0x1 | 0x2, fail_listing: false };
    let mut scanner = wia.scanner_details(WIA_ID).expect("details");
    scanner.id = format!("wia:{WIA_ID}");
    let view = device_view(&scanner, wia.status(WIA_ID), None);
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::On));
    assert_eq!(state_of(&view, K::Jam), Some(Tri::Off), "fault bits are still read as given");
    assert_eq!(state_of(&view, K::LampError), Some(Tri::NotReported));
}

#[test]
fn the_wsd_all_clear_status_is_not_reported_never_off() {
    use craft_job::IndicatorKey as K;
    let mut scanner = common::scanner("wia:{x}", "Network scanner");
    scanner.feeder = true;
    scanner.connection = craft_scan::Connection::Wsd;
    scanner.feeder_capacity = Some(50);
    // A driver that reports every bit clear, and no feeder reading.
    let view = device_view(&scanner, Some(HandlingStatus::default()), None);
    assert!(!view.indicators.is_empty());
    for indicator in &view.indicators {
        assert_eq!(indicator.state, Tri::NotReported, "{:?}", indicator.key);
        assert_eq!(indicator.severity, Severity::Ok, "a state that is not reported is never a problem: {:?}", indicator.key);
    }
    assert_eq!(view.state, DeviceState::Unknown);
    let paper = view.paper.clone().expect("paper");
    assert_eq!(
        (paper.feeder_loaded, paper.glass_available, paper.feeder_capacity, paper.source_in_use),
        (Tri::NotReported, Tri::NotReported, Some(50), None)
    );
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::NotReported));
    // A backend that cannot preflight at all says the same.
    let none = device_view(&scanner, None, None);
    assert_eq!(none.indicators, view.indicators);
    assert_eq!(none.state, DeviceState::Unknown);
}

#[test]
fn capabilities_are_absent_only_when_the_handling_bits_were_read() {
    use craft_job::IndicatorKey as K;
    let mut scanner = common::scanner("wia:{x}", "Network scanner");
    scanner.connection = craft_scan::Connection::Wsd;
    scanner.flatbed = false;
    // A backend that did not say what it read: a plain false is not "absent".
    let view = device_view(&scanner, None, None);
    assert_eq!(
        (view.capabilities.has_feeder, view.capabilities.has_flatbed, view.capabilities.duplex),
        (Tri::NotReported, Tri::NotReported, Tri::NotReported)
    );
    assert_eq!(view.paper.expect("paper").glass_available, Tri::NotReported);
    // The bits were read: now it is.
    scanner.reported.units = true;
    scanner.feeder = true;
    let view = device_view(&scanner, None, None);
    assert_eq!((view.capabilities.has_feeder, view.capabilities.has_flatbed, view.capabilities.duplex), (Tri::On, Tri::Off, Tri::Off));
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::NotReported));
    assert_eq!(view.paper.expect("paper").glass_available, Tri::Off);
    // A status that says the flatbed is ready is believed over a details record that listed none: the indicator and the
    // paper path agree.
    let status = HandlingStatus { flat_ready: true, ..HandlingStatus::default() };
    let view = device_view(&scanner, Some(status), None);
    assert_eq!(state_of(&view, K::FlatbedReady), Some(Tri::On));
    assert_eq!(view.paper.expect("paper").glass_available, Tri::On);
}

#[test]
fn ready_bits_are_read_by_connection_a_clear_one_is_off_only_on_usb() {
    use craft_job::IndicatorKey as K;
    let build = |connection| {
        let mut scanner = common::scanner("wia:{x}", "Scanner");
        scanner.feeder = true;
        scanner.flatbed = true;
        scanner.duplex = true;
        scanner.connection = connection;
        scanner
    };
    // Something is reported (a cover is open), every ready bit is clear.
    let status = HandlingStatus { cover_open: true, ..HandlingStatus::default() };
    for connection in [craft_scan::Connection::Wsd, craft_scan::Connection::Other, craft_scan::Connection::Unknown] {
        let view = device_view(&build(connection), Some(status), None);
        assert_eq!(state_of(&view, K::FlatbedReady), Some(Tri::NotReported), "{connection:?}");
        assert_eq!(state_of(&view, K::DuplexReady), Some(Tri::NotReported), "{connection:?}");
        assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::NotReported), "{connection:?}");
        assert_eq!(state_of(&view, K::CoverOpen), Some(Tri::On));
        assert_eq!(state_of(&view, K::Jam), Some(Tri::Off), "fault bits are read as given once the status says something");
        assert_eq!(view.paper.expect("paper").glass_available, Tri::NotReported);
    }
    // A set ready bit is a positive reading, and an explicit feeder reading is kept.
    let wsd = build(craft_scan::Connection::Wsd);
    let loaded = HandlingStatus { feeder_ready: Some(true), flat_ready: true, ..HandlingStatus::default() };
    let view = device_view(&wsd, Some(loaded), None);
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::On));
    assert_eq!(state_of(&view, K::FlatbedReady), Some(Tri::On));
    assert_eq!(state_of(&view, K::DuplexReady), Some(Tri::NotReported));
    let empty = HandlingStatus { feeder_ready: Some(false), flat_ready: true, ..HandlingStatus::default() };
    assert_eq!(state_of(&device_view(&wsd, Some(empty), None), K::FeederPaper), Some(Tri::Off));
    // USB: a clear ready bit is off.
    let view = device_view(&build(craft_scan::Connection::Usb), Some(status), None);
    assert_eq!(state_of(&view, K::FlatbedReady), Some(Tri::Off));
    assert_eq!(state_of(&view, K::DuplexReady), Some(Tri::Off));
    assert_eq!(view.paper.expect("paper").glass_available, Tri::Off);
}

#[test]
fn feeder_ready_is_taken_as_the_option_the_backend_gave() {
    use craft_job::IndicatorKey as K;
    let sane = SaneLike::new(3);
    let scanner = sane.scanner_details(common::sane_like::SANE_ID).expect("details");
    assert_eq!(scanner.feeder_ready, None);
    // Something is reported (a cover is open) but the tray is not.
    let status = HandlingStatus { cover_open: true, ..HandlingStatus::default() };
    let view = device_view(&scanner, Some(status), None);
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::NotReported));
    assert_eq!(state_of(&view, K::CoverOpen), Some(Tri::On));
    assert_eq!(state_of(&view, K::Jam), Some(Tri::Off));
    assert_eq!(view.connection, DeviceConnection::Usb);
    assert_eq!(view.state, DeviceState::Attention);

    let esc = EsclLike::new(AdfState::Empty, 0);
    let scanner = esc.scanner_details(ESCL_ID).expect("details");
    let view = device_view(&scanner, esc.status(ESCL_ID), None);
    assert_eq!(state_of(&view, K::FeederPaper), Some(Tri::Off), "the backend knows the feeder is empty");
    assert_eq!(view.connection, DeviceConnection::Other);
    assert_eq!(view.state, DeviceState::Online);
    assert!(view.problems().next().is_none(), "an empty feeder is information, not a fault");
    let loaded = EsclLike::new(AdfState::Loaded, 1);
    let view = device_view(&scanner, loaded.status(ESCL_ID), Some(Source::Auto));
    assert_eq!(view.paper.map(|p| (p.feeder_loaded, p.source_in_use)), Some((Tri::On, None)), "Auto names no source");
}

#[test]
fn connections_map_one_to_one_except_wsd_which_is_network() {
    use craft_scan::Connection as C;
    assert_eq!(DeviceConnection::from(C::Usb), DeviceConnection::Usb);
    assert_eq!(DeviceConnection::from(C::Wsd), DeviceConnection::Network);
    assert_eq!(DeviceConnection::from(C::Other), DeviceConnection::Other);
    assert_eq!(DeviceConnection::from(C::Unknown), DeviceConnection::Unknown);
}

// ---- device cancel through the error, offline, continuing, exit class ------------------------------------

#[test]
fn a_device_side_cancel_reported_by_the_backend_ends_cancelled_by_device() {
    let mut job = ScanJob::new(feeder_request(10), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(scripted(&plan, 3, Source::Feeder, Some(native(InterruptKind::Cancelled))), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!(snap.state, ScanJobState::Terminal(Outcome::CancelledByDevice));
    assert_eq!((snap.pages.len(), snap.attempts[0].outcome, snap.cancel_requested), (3, AttemptOutcome::CancelledByDevice, false));
    assert_eq!(snap.exit_class(), ExitClass::Cancelled);
    // A client cancel that raced with it still wins: this side asked first.
    let mut job = ScanJob::new(feeder_request(10), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.cancel(t(2));
    job.finish_attempt(scripted(&plan, 0, Source::Feeder, Some(native(InterruptKind::Cancelled))), t(3)).expect("finish");
    assert_eq!(job.state(), ScanJobState::Terminal(Outcome::Cancelled));
    assert_eq!(InterruptKind::parse("cancelled"), Some(InterruptKind::Cancelled));
    assert_eq!(craft_scan::interrupt_kind(InterruptKind::Cancelled.sentence()), InterruptKind::Cancelled);
}

#[test]
fn offline_before_any_page_is_an_unknown_outcome_and_never_retried() {
    let eager = RetryPolicy { max_attempts: 50, base: Duration::ZERO, ..RetryPolicy::default() };
    let mut job = ScanJob::new(feeder_request(3), ScanJobConfig::default().with_retry(eager));
    job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(Err(native(InterruptKind::Offline)), t(2)).expect("finish");
    assert_eq!(job.state(), ScanJobState::NeedsAttention);
    assert_eq!(job.retry_advice(), RetryAdvice::CheckDeviceFirst);
    assert_eq!(job.snapshot().last_error.map(|f| f.safety), Some(RetrySafety::OutcomeUnknown));
    // A person who looked may start again by hand; nothing is kept so no continuation is asked for.
    assert!(job.begin_attempt(t(3)).is_ok());
}

#[test]
fn continuing_after_12_of_40_feeder_pages_asks_for_the_28_left() {
    let kept: Vec<PageInfo> = (1..=12)
        .map(|n| PageInfo {
            number: 99 + n,
            attempt: 1,
            source: Some(Source::Feeder),
            format: "jpeg",
            width: 16,
            height: 8,
            dpi: 200,
            bytes: 1000,
            length: Default::default(),
        })
        .collect();
    let request = ScanSettings { device: "sane:x".into(), source: Source::Feeder, pages: 40, ..ScanSettings::default() };
    let mut job = ScanJob::continuing(request, ScanJobConfig::default().with_title("Archive"), kept);
    assert_eq!(job.state(), ScanJobState::Interrupted);
    let view = job.view();
    assert_eq!(view.state, JobState::NeedsAttention);
    assert_eq!(view.progress.map(|p| (p.done, p.total)), Some((12, Some(40))));
    assert_eq!(view.documents.len(), 12);
    assert_eq!(job.snapshot().pages.iter().map(|p| p.number).collect::<Vec<_>>(), (1..=12).collect::<Vec<_>>());
    assert_eq!(job.snapshot().summary_line(), "Archive: interrupted, 12 of 40 pages, 1 attempt");
    assert_eq!(job.snapshot().exit_class(), ExitClass::NeedsAttention);
    assert_eq!(job.begin_attempt(t(1)).map(|_| ()), Err(ScanJobError::ContinuationRequired { kept: 12 }));

    let plan = job.begin_attempt_with(t(1), Continuation::Append).expect("append");
    assert_eq!((plan.number, plan.settings.pages, plan.kept_pages), (2, 28, 12));
    job.finish_attempt(SaneLike::new(28).acquire(&plan.settings), t(2)).expect("finish");
    let snap = job.snapshot();
    assert_eq!((snap.state, snap.pages.len()), (ScanJobState::Terminal(Outcome::Completed), 40));
    assert_eq!(snap.exit_class(), ExitClass::Completed);
    assert_eq!(job.take_pages().expect("taken").len(), 28, "only pages received by this process are held");
    assert_eq!(job.view().progress.map(|p| (p.done, p.total)), Some((40, Some(40))));
    // Nothing kept is just a new job.
    let fresh = ScanJob::continuing(feeder_request(3), ScanJobConfig::default(), Vec::new());
    assert_eq!(fresh.state(), ScanJobState::Preparing);
    // Everything already kept: nothing to append, finish instead.
    let all: Vec<PageInfo> = (1..=3)
        .map(|n| PageInfo { number: n, attempt: 1, source: None, format: "png", width: 1, height: 1, dpi: 100, bytes: 1, length: Default::default() })
        .collect();
    let mut full = ScanJob::continuing(feeder_request(3), ScanJobConfig::default(), all);
    assert_eq!(full.begin_attempt_with(t(1), Continuation::Append).map(|_| ()), Err(ScanJobError::NothingLeft { kept: 3 }));
    assert_eq!(full.finish(t(2)), Ok(Outcome::Completed));
}

#[test]
fn exit_classes_follow_the_state() {
    let class = |s: ScanJobState| s.exit_class();
    assert_eq!(class(ScanJobState::Terminal(Outcome::Completed)), ExitClass::Completed);
    assert_eq!(class(ScanJobState::Terminal(Outcome::CompletedWithWarnings)), ExitClass::NeedsAttention);
    assert_eq!(class(ScanJobState::Terminal(Outcome::Failed)), ExitClass::Failed);
    assert_eq!(class(ScanJobState::Terminal(Outcome::Cancelled)), ExitClass::Cancelled);
    assert_eq!(class(ScanJobState::Terminal(Outcome::CancelledByDevice)), ExitClass::Cancelled);
    for s in [ScanJobState::Preparing, ScanJobState::Acquiring, ScanJobState::Interrupted, ScanJobState::NeedsAttention] {
        assert_eq!(class(s), ExitClass::NeedsAttention, "{s:?}");
    }
    assert_eq!(ExitClass::NeedsAttention.name(), "needs_attention");
    let mut job = ScanJob::new(feeder_request(2), ScanJobConfig::default());
    let plan = job.begin_attempt(t(1)).expect("attempt");
    job.finish_attempt(scripted(&plan, 1, Source::Feeder, Some(native(InterruptKind::Jam))), t(2)).expect("finish");
    assert_eq!(job.snapshot().summary_line(), "Scan: interrupted, 1 of 2 pages, 1 attempt, jam");
}
