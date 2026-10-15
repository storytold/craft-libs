#![allow(clippy::expect_used)]

use craft_gpu::{GpuContext, GpuJob, GpuJobParams, ProviderRegistry, WorkerKind};
use craft_gpu_null::NullContext;

use crate::{EchoProvider, NoopProvider, register_all, register_all_into};

#[test]
fn noop_provider_registered_by_id() {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(NoopProvider));
    let mut ctx = NullContext::new(WorkerKind::Main);
    let job = GpuJob { provider_id: "craft.noop".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] };
    let out = registry.dispatch(&mut ctx, &job).expect("noop dispatch");
    assert!(out.data.is_none());
}

#[test]
fn echo_provider_concatenates_params_and_first_input() {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(EchoProvider));
    let mut ctx = NullContext::new(WorkerKind::Main);
    let h = ctx.upload_buffer("in", b"xy").expect("upload_buffer in test");
    let job = GpuJob { provider_id: "craft.test.echo".into(), params: GpuJobParams(b"a".to_vec()), inputs: vec![h], outputs: vec![] };
    let out = registry.dispatch(&mut ctx, &job).expect("echo dispatch");
    assert_eq!(out.data.as_deref(), Some(b"axy".as_slice()));
}

#[test]
fn register_all_lists_shared_providers() {
    let ids: Vec<_> = register_all().iter().map(|p| p.id()).collect();
    assert_eq!(ids, vec!["craft.noop", "craft.test.echo"]);
}

#[test]
fn register_all_into_populates_registry() {
    let mut registry = ProviderRegistry::new();
    register_all_into(&mut registry);
    let mut ctx = NullContext::new(WorkerKind::SharedWorker);
    let job = GpuJob { provider_id: "craft.noop".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] };
    registry.dispatch(&mut ctx, &job).expect("noop via register_all_into");
}
