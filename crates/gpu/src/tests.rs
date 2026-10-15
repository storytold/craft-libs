#![allow(clippy::expect_used)]

use crate::{DeviceCaps, GpuContext, GpuFault, GpuJob, GpuJobParams, HandleId, ProviderRegistry, WorkerKind, chunk_buffer};

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
fn chunk_buffer_splits_on_storage_binding_limit() {
    let caps = DeviceCaps { max_storage_buffer_binding_size: 4, ..DeviceCaps::default() };
    let chunks = chunk_buffer(&caps, b"abcdef").expect("chunk_buffer");
    assert_eq!(chunks, vec![&b"abcd"[..], &b"ef"[..]]);
}
