#![allow(clippy::expect_used)]

use crate::{
    DeviceCaps, GpuContext, GpuFault, GpuJob, GpuJobParams, HandleId, ProviderRegistry, WorkerKind, chunk_buffer, f16_rgba_to_f32_bytes, f16_to_f32,
    f16_to_f32_fast,
};

struct MemContext {
    caps: DeviceCaps,
    buffers: Vec<Vec<u8>>,
}

impl GpuContext for MemContext {
    fn caps(&self) -> DeviceCaps {
        self.caps.clone()
    }

    fn worker_kind(&self) -> WorkerKind {
        WorkerKind::Main
    }

    fn upload_buffer(&mut self, _label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault> {
        let id = HandleId(u32::try_from(self.buffers.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?);
        self.buffers.push(bytes.to_vec());
        Ok(id)
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.buffers.get(handle.0 as usize).map(|b| b.len()).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }

    fn read_buffer(&self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        self.buffers.get(handle.0 as usize).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }
}

#[test]
fn unknown_provider_is_fault() {
    let registry = ProviderRegistry::new();
    let mut ctx = MemContext { caps: DeviceCaps::default(), buffers: Vec::new() };
    let job = GpuJob { provider_id: "missing".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] };
    assert!(matches!(registry.dispatch(&mut ctx, &job), Err(GpuFault::UnknownProvider(_))));
}

#[test]
#[test]
fn f16_to_f32_matches_fast_for_one() {
    let h = 0x3c00u16;
    assert!((f16_to_f32(h) - 1.0).abs() < 1e-6);
    assert_eq!(f16_to_f32(h).to_bits(), f16_to_f32_fast(h).to_bits());
}

#[test]
fn f16_rgba_to_f32_bytes_expands_texels() {
    let src = 0x3c00u16.to_le_bytes();
    let mut raw = Vec::new();
    for _ in 0..4 {
        raw.extend_from_slice(&src);
    }
    let out = f16_rgba_to_f32_bytes(&raw).expect("decode");
    assert_eq!(out.len(), 16);
    assert!((f32::from_le_bytes(out[0..4].try_into().expect("le")) - 1.0).abs() < 1e-6);
}

#[test]
fn chunk_buffer_splits_on_storage_binding_limit() {
    let caps = DeviceCaps { max_storage_buffer_binding_size: 4, ..DeviceCaps::default() };
    let chunks = chunk_buffer(&caps, b"abcdef").expect("chunk_buffer");
    assert_eq!(chunks, vec![&b"abcd"[..], &b"ef"[..]]);
}
