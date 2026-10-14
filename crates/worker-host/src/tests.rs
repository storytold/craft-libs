#![allow(clippy::expect_used)]

use craft_gpu::{GpuJob, GpuJobParams};

use crate::{HostMessage, WorkerMessage};

#[test]
fn host_init_json_shape() {
    let msg = HostMessage::Init {
        app_id: "collab.photo".into(),
        capabilities: vec!["gpu".into(), "tier_store".into()],
    };
    let json = serde_json::to_string(&msg).expect("serialize HostMessage");
    assert_eq!(
        json,
        r#"{"type":"init","app_id":"collab.photo","capabilities":["gpu","tier_store"]}"#
    );
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize HostMessage");
    assert_eq!(msg, back);
}

#[test]
fn host_gpu_jobs_json_shape() {
    let msg = HostMessage::GpuJobs {
        batch_id: 42,
        jobs: vec![GpuJob {
            provider_id: "craft.test".into(),
            params: GpuJobParams(vec![1, 2]),
            inputs: vec![],
            outputs: vec![],
        }],
    };
    let json = serde_json::to_string(&msg).expect("serialize HostMessage");
    assert_eq!(
        json,
        r#"{"type":"gpu_jobs","batch_id":42,"jobs":[{"provider_id":"craft.test","params":[1,2],"inputs":[],"outputs":[]}]}"#
    );
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize HostMessage");
    assert_eq!(msg, back);
}

#[test]
fn host_gpu_jobs_roundtrip_json() {
    let msg = HostMessage::GpuJobs {
        batch_id: 1,
        jobs: vec![GpuJob {
            provider_id: "craft.test".into(),
            params: GpuJobParams(vec![1, 2]),
            inputs: vec![],
            outputs: vec![],
        }],
    };
    let json = serde_json::to_string(&msg).expect("serialize HostMessage");
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize HostMessage");
    assert_eq!(msg, back);
}

#[test]
fn worker_ready_roundtrip_json() {
    let msg = WorkerMessage::Ready {
        caps: craft_gpu::DeviceCaps::default(),
    };
    let json = serde_json::to_string(&msg).expect("serialize WorkerMessage");
    let back: WorkerMessage = serde_json::from_str(&json).expect("deserialize WorkerMessage");
    assert_eq!(msg, back);
}
