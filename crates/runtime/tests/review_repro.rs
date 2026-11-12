//! Review reproductions for PR #662 (storytold/craft-contrib#39, skylartaylor), covering the
//! `craft-runtime` <-> `FlushSink` integration for failure 2 (acknowledgment without
//! persistence). The gist's own `Accepting` sink (a bare `usize` count) and its RSS-via-`ps`
//! probe don't fit `FlushSink`'s new per-key `FlushOutcome` contract or a sandboxed CI host; this
//! adapts the same intent to that contract and to `MirrorStore::retained_bytes()` /
//! `over_budget()`, which `crates/tier-store/src/tests.rs` exercises directly for failure 3
//! (unbounded eviction retention).
//!
//! Run: cargo test -p craft-runtime --test review_repro

use craft_runtime::{FlushOutcome, FlushSink, HostMessage, PendingWrite, Runtime, WorkerMessage};

/// A sink whose per-key outcome is scripted by the test.
struct Scripted {
    outcomes: Vec<FlushOutcome>,
}

impl FlushSink for Scripted {
    fn write_all(&mut self, pending: &[PendingWrite]) -> Vec<FlushOutcome> {
        // Mirror a real sink's contract: report at most one outcome per entry given.
        self.outcomes.iter().copied().take(pending.len()).collect()
    }
}

#[test]
fn a_fully_failed_flush_reports_error_and_keeps_the_write_queued_for_retry() {
    let mut runtime = Runtime::builder().null_context().flush_sink(Box::new(Scripted { outcomes: vec![FlushOutcome::Failed] })).build();

    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"1".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(
        matches!(flushed.as_slice(), [WorkerMessage::Error { .. }]),
        "a flush that persisted nothing must report Error, never StorageFlushed: {flushed:?}"
    );

    // The write must still be queued. The builder does not expose swapping sinks mid-flight, so
    // prove it the only way the public API allows: flushing again against the same
    // always-failing sink reports the identical failure, which is only possible if the write
    // was still there to flush (a dropped write would instead flush nothing and trivially
    // succeed, per `an_empty_flush_against_a_configured_sink_is_a_trivial_success_not_an_error`).
    let flushed_again = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(
        matches!(flushed_again.as_slice(), [WorkerMessage::Error { .. }]),
        "the write must still be queued (and still fail against the same sink) on a second flush: {flushed_again:?}"
    );
}

#[derive(Default)]
struct AcceptAll;
impl FlushSink for AcceptAll {
    fn write_all(&mut self, pending: &[PendingWrite]) -> Vec<FlushOutcome> {
        vec![FlushOutcome::Persisted; pending.len()]
    }
}

#[test]
fn a_partially_failed_flush_reports_only_what_persisted_and_retries_the_rest() {
    let mut runtime =
        Runtime::builder().null_context().flush_sink(Box::new(Scripted { outcomes: vec![FlushOutcome::Persisted, FlushOutcome::Failed] })).build();

    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"1".to_vec() });
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/b".into(), bytes: b"2".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending_count, pending: None }] = flushed.as_slice() else {
        panic!("expected a partial StorageFlushed, got {flushed:?}");
    };
    assert_eq!(*pending_count, 1, "only the persisted entry should be confirmed: {flushed:?}");
}

#[test]
fn a_fully_successful_flush_never_reports_error() {
    let mut runtime = Runtime::builder().null_context().flush_sink(Box::new(AcceptAll)).build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"1".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(matches!(flushed.as_slice(), [WorkerMessage::StorageFlushed { pending_count: 1, pending: None }]), "{flushed:?}");
}

#[test]
fn an_empty_flush_against_a_configured_sink_is_a_trivial_success_not_an_error() {
    let mut runtime = Runtime::builder().null_context().flush_sink(Box::new(AcceptAll)).build();
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(
        matches!(flushed.as_slice(), [WorkerMessage::StorageFlushed { pending_count: 0, pending: None }]),
        "nothing pending must not be confused with nothing persisted: {flushed:?}"
    );
}
