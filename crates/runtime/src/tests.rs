#![allow(clippy::expect_used, clippy::panic)]

use craft_gpu::{GpuJob, GpuJobOutcome, GpuJobParams, WorkerKind};

use crate::{HostMessage, Runtime, WorkerMessage};

#[test]
fn init_then_gpu_jobs_batch_done() {
    let mut runtime = Runtime::builder()
        .with_default_providers()
        .with_worker_kind(WorkerKind::SharedWorker)
        .null_context()
        .build();

    let ready = runtime.handle_host_message(HostMessage::Init {
        app_id: "test-app".into(),
        capabilities: vec!["gpu".into()],
    });
    assert_eq!(ready.len(), 1);
    let WorkerMessage::Ready { caps } = &ready[0] else {
        panic!("expected Ready, got {:?}", ready[0]);
    };
    assert_eq!(caps.backend_name, "null");

    let jobs = vec![GpuJob {
        provider_id: "craft.noop".into(),
        params: GpuJobParams(Vec::new()),
        inputs: vec![],
        outputs: vec![],
    }];
    let done = runtime.handle_host_message(HostMessage::GpuJobs {
        batch_id: 42,
        jobs,
    });
    assert_eq!(done.len(), 1);
    let WorkerMessage::GpuBatchDone { batch_id, outcomes } = &done[0] else {
        panic!("expected GpuBatchDone, got {:?}", done[0]);
    };
    assert_eq!(*batch_id, 42);
    assert_eq!(
        outcomes,
        &[GpuJobOutcome {
            outputs: vec![],
            data: None,
        }]
    );
}
