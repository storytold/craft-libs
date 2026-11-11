#![allow(clippy::expect_used)]

use craft_gpu::{GpuJob, GpuJobParams};

use crate::{HostMessage, WorkerMessage};

#[test]
fn host_init_json_shape() {
    let msg = HostMessage::Init { app_id: "collab.photo".into(), capabilities: vec!["gpu".into(), "tier_store".into()] };
    let json = serde_json::to_string(&msg).expect("serialize HostMessage");
    assert_eq!(json, r#"{"type":"init","app_id":"collab.photo","capabilities":["gpu","tier_store"]}"#);
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize HostMessage");
    assert_eq!(msg, back);
}

#[test]
fn host_gpu_jobs_json_shape() {
    let msg = HostMessage::GpuJobs {
        batch_id: 42,
        jobs: vec![GpuJob { provider_id: "craft.test".into(), params: GpuJobParams(vec![1, 2]), inputs: vec![], outputs: vec![] }],
    };
    let json = serde_json::to_string(&msg).expect("serialize HostMessage");
    assert_eq!(json, r#"{"type":"gpu_jobs","batch_id":42,"jobs":[{"provider_id":"craft.test","params":[1,2],"inputs":[],"outputs":[]}]}"#);
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize HostMessage");
    assert_eq!(msg, back);
}

#[test]
fn host_gpu_jobs_roundtrip_json() {
    let msg = HostMessage::GpuJobs {
        batch_id: 1,
        jobs: vec![GpuJob { provider_id: "craft.test".into(), params: GpuJobParams(vec![1, 2]), inputs: vec![], outputs: vec![] }],
    };
    let json = serde_json::to_string(&msg).expect("serialize HostMessage");
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize HostMessage");
    assert_eq!(msg, back);
}

#[test]
fn host_cancel_json_shape() {
    let msg = HostMessage::Cancel { batch_id: 99 };
    let json = serde_json::to_string(&msg).expect("serialize Cancel");
    assert_eq!(json, r#"{"type":"cancel","batch_id":99}"#);
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize Cancel");
    assert_eq!(msg, back);
}

#[test]
fn host_storage_put_and_flush_json_shape() {
    let put = HostMessage::StoragePut { key: "config/prefs.json".into(), bytes: vec![1, 2] };
    let json = serde_json::to_string(&put).expect("serialize StoragePut");
    assert!(json.contains(r#""type":"storage_put""#));
    let flush = HostMessage::StorageFlush;
    let flushed = WorkerMessage::StorageFlushed { pending_count: 2, pending: None };
    let fj = serde_json::to_string(&flush).expect("serialize flush");
    assert_eq!(fj, r#"{"type":"storage_flush"}"#);
    let wj = serde_json::to_string(&flushed).expect("serialize flushed");
    assert!(wj.contains(r#""pending_count":2"#));
}

#[test]
fn storage_flushed_pending_export_json() {
    let msg = WorkerMessage::StorageFlushed {
        pending_count: 1,
        pending: Some(vec![crate::PendingFlushEntry { key: "config/a".into(), bytes: vec![1, 2], deleted: false, revision: 3 }]),
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    assert!(json.contains(r#""pending_count":1"#));
    assert!(json.contains(r#""key":"config/a""#));
    assert!(json.contains(r#""revision":3"#));
    let back: WorkerMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, back);
}

#[test]
fn storage_delete_json_shape() {
    let msg = HostMessage::StorageDelete { key: "files/old".into() };
    let json = serde_json::to_string(&msg).expect("serialize StorageDelete");
    assert_eq!(json, r#"{"type":"storage_delete","key":"files/old"}"#);
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize StorageDelete");
    assert_eq!(msg, back);
}

#[test]
fn storage_flush_ack_json_shape() {
    let msg = HostMessage::StorageFlushAck {
        results: vec![
            crate::FlushAckResult { key: "config/a".into(), revision: 3, persisted: true },
            crate::FlushAckResult { key: "config/b".into(), revision: 1, persisted: false },
        ],
    };
    let json = serde_json::to_string(&msg).expect("serialize StorageFlushAck");
    let back: HostMessage = serde_json::from_str(&json).expect("deserialize StorageFlushAck");
    assert_eq!(msg, back);
    assert!(json.contains(r#""type":"storage_flush_ack""#));
    assert!(json.contains(r#""persisted":true"#));
    assert!(json.contains(r#""persisted":false"#));
}

#[test]
fn pending_flush_entry_deleted_defaults_false_when_absent_from_json() {
    // A host built before the `deleted` field existed sends JSON without it; deserializing must
    // not fail, and must treat the entry as a normal write, never as an accidental tombstone.
    let json = r#"{"key":"config/a","bytes":[1,2],"revision":1}"#;
    let entry: crate::PendingFlushEntry = serde_json::from_str(json).expect("deserialize legacy PendingFlushEntry");
    assert!(!entry.deleted);
}

#[test]
fn worker_ready_roundtrip_json() {
    let msg = WorkerMessage::Ready { caps: craft_gpu::DeviceCaps::default() };
    let json = serde_json::to_string(&msg).expect("serialize WorkerMessage");
    let back: WorkerMessage = serde_json::from_str(&json).expect("deserialize WorkerMessage");
    assert_eq!(msg, back);
}
