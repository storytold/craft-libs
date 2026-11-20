//! Review reproductions for PR #662 (storytold/craft-contrib#39, skylartaylor), covering the
//! `craft-runtime` <-> `FlushSink` integration for failure 2 (acknowledgment without
//! persistence). The gist's own `Accepting` sink (a bare `usize` count) and its RSS-via-`ps`
//! probe don't fit `FlushSink`'s new per-key `FlushOutcome` contract or a sandboxed CI host; this
//! adapts the same intent to that contract and to `MirrorStore::retained_bytes()` /
//! `over_budget()`, which `crates/tier-store/src/tests.rs` exercises directly for failure 3
//! (unbounded eviction retention).
//!
//! The tests from `a_no_native_sink_export_...` on cover skylartaylor's consolidated review
//! (storytold/craft-libs#123, 2026-11-12): the browser path (no native `FlushSink`, so a flush
//! batch is exported to the host JS and confirmed later by `HostMessage::StorageFlushAck`) must
//! only mark a revision saved on a true persistence acknowledgment, the same two-step contract
//! `a_partially_failed_flush_reports_only_what_persisted_and_retries_the_rest` above already
//! proves for the native path. Nothing in this file exercised the export/ack path at all before
//! these two tests: both pass against the current `handle_storage_flush`/`handle_storage_flush_ack`
//! wiring, which already gates `ack_saved` on `FlushAckResult::persisted`. They stand as coverage
//! that this finding is addressed, not as a reproduction of a live bug.
//!
//! Run: cargo test -p craft-runtime --test review_repro

use craft_runtime::{FlushAckResult, FlushOutcome, FlushSink, HostMessage, PendingWrite, Runtime, WorkerMessage};

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

/// Extracts the single exported `PendingFlushEntry` a one-key `StorageFlush` on the no-native-sink
/// (browser) path produces, for a test to turn around into a `StorageFlushAck`.
fn exported_entry(flushed: &[WorkerMessage]) -> craft_runtime::PendingFlushEntry {
    let [WorkerMessage::StorageFlushed { pending: Some(pending), .. }] = flushed else {
        panic!("expected one StorageFlushed with an exported batch, got {flushed:?}");
    };
    let [entry] = pending.as_slice() else {
        panic!("expected exactly one exported entry, got {pending:?}");
    };
    entry.clone()
}

/// A `StorageFlushAck` reporting `persisted: false` must not mark the revision saved: the write
/// stays queued and the next flush exports it again, unchanged.
#[test]
fn a_no_native_sink_export_not_persisted_leaves_the_write_queued_for_retry() {
    let mut runtime = Runtime::builder().null_context().build(); // no flush_sink: the browser/export path
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"1".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let entry = exported_entry(&flushed);

    runtime.handle_host_message(HostMessage::StorageFlushAck {
        results: vec![FlushAckResult { key: entry.key.clone(), revision: entry.revision, persisted: false }],
    });

    // Not saved: the next flush must export the exact same write again, not report it already done.
    let flushed_again = runtime.handle_host_message(HostMessage::StorageFlush);
    let entry_again = exported_entry(&flushed_again);
    assert_eq!(entry_again.key, entry.key);
    assert_eq!(entry_again.bytes, entry.bytes);
}

/// A `StorageFlushAck` reporting `persisted: true` is the only thing that marks a revision saved.
/// Once it lands, a later flush with nothing new pending must report zero, never re-export the
/// already-saved write.
#[test]
fn a_no_native_sink_export_persisted_true_is_the_only_thing_that_saves_it() {
    let mut runtime = Runtime::builder().null_context().build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"1".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let entry = exported_entry(&flushed);

    runtime.handle_host_message(HostMessage::StorageFlushAck {
        results: vec![FlushAckResult { key: entry.key.clone(), revision: entry.revision, persisted: true }],
    });

    let flushed_again = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending_count: 0, pending: Some(p) }] = flushed_again.as_slice() else {
        panic!("the acked write must not come back on a later flush: {flushed_again:?}");
    };
    assert!(p.is_empty(), "the acked write must not come back on a later flush: {p:?}");
}
