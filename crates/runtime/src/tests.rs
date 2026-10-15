#![allow(clippy::expect_used, clippy::panic)]

use craft_gpu::{GpuJob, GpuJobOutcome, GpuJobParams, HandleId, WorkerKind};
use craft_tier_store::StoreKey;

use crate::{HostMessage, Runtime, WorkerMessage};

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
    let WorkerMessage::GpuBatchDone { batch_id, outcomes } = &done[0] else {
        panic!("expected GpuBatchDone, got {:?}", done[0]);
    };
    assert_eq!(*batch_id, 42);
    assert_eq!(outcomes, &[GpuJobOutcome { outputs: vec![], data: None }]);
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
    let WorkerMessage::StorageFlushed { pending_count } = &flushed[0] else {
        panic!("expected StorageFlushed, got {:?}", flushed[0]);
    };
    assert_eq!(*pending_count, 2);

    let clip = runtime.upload_buffer("footage", b"clip").expect("upload in test");

    // Exercise echo provider through the same path as SharedWorker GPU batch.
    let jobs = vec![GpuJob {
        provider_id: "craft.test.echo".into(),
        params: GpuJobParams(b"pre:".to_vec()),
        inputs: vec![clip],
        outputs: vec![],
    }];
    let done = runtime.handle_host_message(HostMessage::GpuJobs { batch_id: 7, jobs });
    let WorkerMessage::GpuBatchDone { outcomes, .. } = &done[0] else {
        panic!("expected GpuBatchDone, got {:?}", done[0]);
    };
    assert_eq!(outcomes[0].data.as_deref(), Some(b"pre:clip".as_slice()));
}
