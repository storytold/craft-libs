#![allow(clippy::expect_used)]

use craft_gpu::{GpuContext, GpuJob, GpuJobParams, ProviderRegistry, WorkerKind, f16_to_f32, f16_to_f32_fast};
use craft_gpu_null::NullContext;

use crate::{EchoProvider, MaskProvider, NoopProvider, PremulCompositeProvider, ReadbackProvider, register_all, register_all_into};

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
    assert_eq!(ids, vec!["craft.noop", "craft.test.echo", "craft.mask", "craft.composite.premul", "craft.readback",]);
}

#[test]
fn gpu_providers_cargo_has_no_app_deps() {
    let manifest = include_str!("../Cargo.toml");
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with("effectcraft") || t.starts_with("filmcraft") || t.contains("effectcraft-") || t.contains("filmcraft-") {
            panic!("forbidden app dependency line: {t}");
        }
    }
}

#[test]
fn mask_provider_multiplies_rgba_by_mask_on_cpu() {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(MaskProvider));
    let mut ctx = NullContext::new(WorkerKind::Main);
    let base = ctx.upload_buffer("base", &[255u8, 0, 0, 255, 100, 100, 100, 255]).expect("upload base");
    let mask = ctx.upload_buffer("mask", &[128u8, 255, 255, 255, 0, 255, 255, 255]).expect("upload mask");
    let job =
        craft_gpu::GpuJob { provider_id: "craft.mask".into(), params: GpuJobParams(b"w:1,h:2".to_vec()), inputs: vec![base, mask], outputs: vec![] };
    let out = registry.dispatch(&mut ctx, &job).expect("mask dispatch");
    let handle = out.outputs[0];
    let bytes = ctx.read_buffer(handle).expect("read mask out");
    assert_eq!(bytes[0], 128);
    assert_eq!(bytes[4], 0);
}

#[test]
fn premul_composite_over_on_cpu() {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(PremulCompositeProvider));
    let mut ctx = NullContext::new(WorkerKind::Main);
    // backdrop: green opaque; layer: half-alpha blue
    let dst = ctx.upload_buffer("dst", &[0, 255, 0, 255]).expect("upload dst");
    let src = ctx.upload_buffer("src", &[0, 0, 255, 128]).expect("upload src");
    let job =
        GpuJob { provider_id: "craft.composite.premul".into(), params: GpuJobParams(b"w:1,h:1".to_vec()), inputs: vec![dst, src], outputs: vec![] };
    let out = registry.dispatch(&mut ctx, &job).expect("premul composite");
    let bytes = ctx.read_buffer(out.outputs[0]).expect("read out");
    assert_eq!(bytes[0], 0);
    assert_eq!(bytes[1], 127);
    assert_eq!(bytes[2], 128);
    assert_eq!(bytes[3], 255);
}

#[test]
fn readback_decodes_f16_rgba_to_f32() {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(ReadbackProvider));
    let mut ctx = NullContext::new(WorkerKind::Main);
    let h = half_float_one();
    let mut raw = Vec::new();
    raw.extend_from_slice(&h.to_le_bytes());
    raw.extend_from_slice(&h.to_le_bytes());
    raw.extend_from_slice(&h.to_le_bytes());
    raw.extend_from_slice(&0u16.to_le_bytes());
    let inp = ctx.upload_buffer("accum", &raw).expect("upload f16");
    let job = GpuJob { provider_id: "craft.readback".into(), params: GpuJobParams(b"w:1,h:1".to_vec()), inputs: vec![inp], outputs: vec![] };
    let out = registry.dispatch(&mut ctx, &job).expect("readback");
    let decoded = ctx.read_buffer(out.outputs[0]).expect("read f32 out");
    assert_eq!(decoded.len(), 16);
    let r = f32::from_le_bytes(decoded[0..4].try_into().expect("r"));
    assert!((r - 1.0).abs() < 1e-3);
    assert_eq!(f16_to_f32(h), f16_to_f32_fast(h));
}

fn half_float_one() -> u16 {
    f32_to_f16(1.0)
}

fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 31) & 1) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x007f_ffff;
    if exp <= 0 {
        return sign << 15;
    }
    if exp >= 0x1f {
        return (sign << 15) | 0x7c00;
    }
    (sign << 15) | ((exp as u16) << 10) | ((mant >> 13) as u16)
}

#[test]
fn register_all_into_populates_registry() {
    let mut registry = ProviderRegistry::new();
    register_all_into(&mut registry);
    let mut ctx = NullContext::new(WorkerKind::SharedWorker);
    let job = GpuJob { provider_id: "craft.noop".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] };
    registry.dispatch(&mut ctx, &job).expect("noop via register_all_into");
}
