#![allow(clippy::expect_used, clippy::panic)]

use craft_gpu::{GpuJob, GpuJobOutcome, GpuJobParams, WorkerKind};
use craft_tier_store::StoreKey;

use crate::{FlushAckResult, FlushSink, GpuJobResult, HostMessage, Runtime, WorkerMessage};

#[test]
fn init_then_gpu_jobs_batch_done() {
    let mut runtime = Runtime::builder().with_default_providers().with_worker_kind(WorkerKind::SharedWorker).null_context().build();

    let ready = runtime.handle_host_message(HostMessage::Init { app_id: "test-app".into(), capabilities: vec!["gpu".into()] });
    assert_eq!(ready.len(), 1);
    let WorkerMessage::Ready { caps } = &ready[0] else {
        panic!("expected Ready, got {:?}", ready[0]);
    };
    assert_eq!(caps.backend_name, "null");

    let jobs = vec![GpuJob { provider_id: "craft.noop".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] }];
    let done = runtime.handle_host_message(HostMessage::GpuJobs { batch_id: 42, jobs });
    assert_eq!(done.len(), 1);
    let WorkerMessage::GpuBatchDone { batch_id, results } = &done[0] else {
        panic!("expected GpuBatchDone, got {:?}", done[0]);
    };
    assert_eq!(*batch_id, 42);
    assert_eq!(results, &[GpuJobResult::Done { outcome: GpuJobOutcome { outputs: vec![], data: None } }]);
}

#[test]
fn end_to_end_storage_flush_and_echo_gpu() {
    let mut runtime = Runtime::builder().with_default_providers().with_worker_kind(WorkerKind::SharedWorker).null_context().build();

    assert!(matches!(
        runtime
            .handle_host_message(HostMessage::Init { app_id: "effectcraft".into(), capabilities: vec!["gpu".into(), "tier_store".into()] })
            .as_slice(),
        [WorkerMessage::Ready { .. }]
    ));

    runtime.handle_host_message(HostMessage::StoragePut { key: "config/prefs.json".into(), bytes: br#"{"v":1}"#.to_vec() });
    runtime.handle_host_message(HostMessage::StoragePut { key: "files/project.ecproj".into(), bytes: b"proj-bytes".to_vec() });

    let key = StoreKey::new("config/prefs.json");
    assert_eq!(runtime.store_get(&key).as_deref(), Some(br#"{"v":1}"#.as_slice()));

    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    assert_eq!(flushed.len(), 1);
    let WorkerMessage::StorageFlushed { pending_count, pending } = &flushed[0] else {
        panic!("expected StorageFlushed, got {:?}", flushed[0]);
    };
    assert_eq!(*pending_count, 2);
    let exported = pending.as_ref().expect("pending export without FlushSink");
    assert_eq!(exported.len(), 2);
    assert_eq!(exported[0].key, "config/prefs.json");
    assert_eq!(exported[1].key, "files/project.ecproj");

    let clip = runtime.upload_buffer("footage", b"clip").expect("upload in test");

    // Exercise echo provider through the same path as SharedWorker GPU batch.
    let jobs = vec![GpuJob { provider_id: "craft.test.echo".into(), params: GpuJobParams(b"pre:".to_vec()), inputs: vec![clip], outputs: vec![] }];
    let done = runtime.handle_host_message(HostMessage::GpuJobs { batch_id: 7, jobs });
    let WorkerMessage::GpuBatchDone { results, .. } = &done[0] else {
        panic!("expected GpuBatchDone, got {:?}", done[0]);
    };
    let GpuJobResult::Done { outcome } = &results[0] else {
        panic!("expected a done outcome, got {:?}", results[0]);
    };
    assert_eq!(outcome.data.as_deref(), Some(b"pre:clip".as_slice()));
}

#[test]
fn storage_flush_uses_flush_sink() {
    use craft_tier_store::PendingWrite;
    use std::sync::{Arc, Mutex};

    let captured: Arc<Mutex<Vec<PendingWrite>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = captured.clone();
    struct Capture(Arc<Mutex<Vec<PendingWrite>>>);
    impl FlushSink for Capture {
        fn write_all(&mut self, pending: &[PendingWrite]) -> Vec<crate::FlushOutcome> {
            self.0.lock().expect("test lock").extend_from_slice(pending);
            vec![crate::FlushOutcome::Persisted; pending.len()]
        }
    }

    let mut runtime = Runtime::builder().null_context().flush_sink(Box::new(Capture(cap))).build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"1".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(matches!(flushed.as_slice(), [WorkerMessage::StorageFlushed { pending_count: 1, pending: None }]));
    let writes = captured.lock().expect("test lock");
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].key.0, "config/a");
}

// --- review_repro: CodeRabbit findings on PR #662's review ------------------------------------

/// StoragePut must backpressure instead of growing the mirror without bound (CodeRabbit finding
/// on runtime.rs:212; RFC-0023 acceptance requirement 2). A write larger than the whole budget
/// used to land in the mirror unconditionally: `config/` keys are never LRU-evicted, so nothing
/// ever reclaimed that memory.
#[test]
fn storage_put_over_budget_is_refused_not_silently_grown() {
    let mut runtime = Runtime::builder().null_context().mirror_byte_budget(10).build();

    let reply = runtime.handle_host_message(HostMessage::StoragePut { key: "config/big".into(), bytes: vec![0u8; 20] });

    assert!(matches!(reply.as_slice(), [WorkerMessage::Error { .. }]), "a write past the budget must error, got {reply:?}");
    assert_eq!(runtime.store_get(&StoreKey::new("config/big")), None, "a refused write must never land in the mirror");
}

/// A write that fits the remaining budget still succeeds (the backpressure check is strictly
/// "would exceed", not an unconditional refusal).
#[test]
fn storage_put_within_budget_still_succeeds() {
    let mut runtime = Runtime::builder().null_context().mirror_byte_budget(10).build();

    let reply = runtime.handle_host_message(HostMessage::StoragePut { key: "config/small".into(), bytes: vec![0u8; 5] });

    assert_eq!(reply, Vec::new());
    assert_eq!(runtime.store_get(&StoreKey::new("config/small")), Some(vec![0u8; 5]));
}

/// StorageDelete carries a tombstone through to the exported flush batch (CodeRabbit finding on
/// worker-host/protocol.rs:17): `MirrorStore::delete` already produces one, but nothing on the
/// wire could represent it before `PendingFlushEntry::deleted` and `HostMessage::StorageDelete`
/// existed.
#[test]
fn storage_delete_exports_a_tombstone_entry() {
    let mut runtime = Runtime::builder().null_context().build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "files/old".into(), bytes: b"bytes".to_vec() });
    runtime.handle_host_message(HostMessage::StorageFlush);

    runtime.handle_host_message(HostMessage::StorageDelete { key: "files/old".into() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending, .. }] = flushed.as_slice() else {
        panic!("expected StorageFlushed, got {flushed:?}");
    };
    let exported = pending.as_ref().expect("pending export without FlushSink");
    assert_eq!(exported.len(), 1);
    assert!(exported[0].deleted, "a delete must export a tombstone entry, not a normal write");
    assert_eq!(exported[0].key, "files/old");
}

/// A failed StorageFlushAck restores the write for retry with its original bytes intact
/// (CodeRabbit CRITICAL finding on runtime.rs:226). `take_pending` already removed the write
/// from the mirror by the time it was exported; without `exported_pending` holding the real
/// bytes, a restore after a failed host persistence had nothing to put back, and a naive restore
/// that does not keep the real bytes would requeue an empty write instead of the lost one.
#[test]
fn a_failed_flush_ack_restores_the_write_with_its_original_bytes() {
    let mut runtime = Runtime::builder().null_context().build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"original".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending, .. }] = flushed.as_slice() else {
        panic!("expected StorageFlushed, got {flushed:?}");
    };
    let exported = pending.as_ref().expect("pending export without FlushSink");
    let revision = exported[0].revision;

    // The host failed to persist it: ack with persisted: false.
    runtime
        .handle_host_message(HostMessage::StorageFlushAck { results: vec![FlushAckResult { key: "config/a".into(), revision, persisted: false }] });

    // The write must be queued again, with the real bytes, for the next flush attempt.
    let retried = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending_count, pending }] = retried.as_slice() else {
        panic!("expected StorageFlushed, got {retried:?}");
    };
    assert_eq!(*pending_count, 1, "the failed write must come back on the next flush");
    let retried_entry = &pending.as_ref().expect("pending export without FlushSink")[0];
    assert_eq!(retried_entry.bytes, b"original", "the restored write must carry its real bytes, not an empty placeholder");
}

/// A successful StorageFlushAck marks the revision saved and clears it from the exported batch:
/// a second flush with nothing new pending reports zero, not a re-export of an already-persisted
/// write.
#[test]
fn a_successful_flush_ack_marks_the_write_saved() {
    let mut runtime = Runtime::builder().null_context().build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"ok".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending, .. }] = flushed.as_slice() else {
        panic!("expected StorageFlushed, got {flushed:?}");
    };
    let revision = pending.as_ref().expect("pending export without FlushSink")[0].revision;

    runtime.handle_host_message(HostMessage::StorageFlushAck { results: vec![FlushAckResult { key: "config/a".into(), revision, persisted: true }] });

    let second = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(
        matches!(second.as_slice(), [WorkerMessage::StorageFlushed { pending_count: 0, .. }]),
        "a saved write must not be re-exported, got {second:?}"
    );
}

/// Before any acknowledgment arrives, the exported write must stay recoverable across an
/// intervening flush that has nothing new to export: a second, empty `StorageFlush` must not
/// make the first export's bytes unreachable. Proven by acking the *original* revision after
/// that intervening flush and confirming it still restores the real bytes, not nothing.
#[test]
fn an_unacknowledged_export_survives_an_intervening_empty_flush() {
    let mut runtime = Runtime::builder().null_context().build();
    runtime.handle_host_message(HostMessage::StoragePut { key: "config/a".into(), bytes: b"pending".to_vec() });
    let flushed = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending, .. }] = flushed.as_slice() else {
        panic!("expected StorageFlushed, got {flushed:?}");
    };
    let revision = pending.as_ref().expect("pending export without FlushSink")[0].revision;

    // Nothing new to export: take_pending() is empty, but the first export is still outstanding.
    let second = runtime.handle_host_message(HostMessage::StorageFlush);
    assert!(matches!(second.as_slice(), [WorkerMessage::StorageFlushed { pending_count: 0, .. }]));

    // The host now reports the original export failed. It must still be recoverable, with its
    // real bytes, even though a flush happened in between.
    runtime
        .handle_host_message(HostMessage::StorageFlushAck { results: vec![FlushAckResult { key: "config/a".into(), revision, persisted: false }] });
    let retried = runtime.handle_host_message(HostMessage::StorageFlush);
    let [WorkerMessage::StorageFlushed { pending_count, pending }] = retried.as_slice() else {
        panic!("expected StorageFlushed, got {retried:?}");
    };
    assert_eq!(*pending_count, 1);
    assert_eq!(pending.as_ref().expect("pending export without FlushSink")[0].bytes, b"pending");
}

/// A batch with a middle failure reports the first and third jobs done and the second failed,
/// never discarding the two jobs that actually succeeded (PR 663, craft-gpu's per-job
/// dispatch_batch contract: GpuBatchDone must carry a result per job, in input order).
#[test]
fn gpu_batch_with_a_middle_failure_reports_per_job_results() {
    let mut runtime = Runtime::builder().with_default_providers().null_context().build();
    let jobs = vec![
        GpuJob { provider_id: "craft.noop".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] },
        GpuJob { provider_id: "craft.does.not.exist".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] },
        GpuJob { provider_id: "craft.noop".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] },
    ];
    let done = runtime.handle_host_message(HostMessage::GpuJobs { batch_id: 1, jobs });
    let [WorkerMessage::GpuBatchDone { results, .. }] = done.as_slice() else {
        panic!("expected GpuBatchDone, got {done:?}");
    };
    assert_eq!(results.len(), 3);
    assert!(matches!(results[0], GpuJobResult::Done { .. }), "job 1 must report done, got {:?}", results[0]);
    assert!(matches!(results[1], GpuJobResult::Failed { .. }), "job 2 must report failed, got {:?}", results[1]);
    assert!(matches!(results[2], GpuJobResult::Done { .. }), "job 3 must report done, got {:?}", results[2]);
}
