#![allow(clippy::expect_used)]

use crate::{
    DeviceCaps, GpuContext, GpuFault, GpuJob, GpuJobParams, HandleId, ProviderRegistry, ReadOutcome, ReadRequest, WorkerKind, chunk_buffer,
    f16_rgba_to_f32_bytes, f16_to_f32, f16_to_f32_fast,
};

struct MemContext {
    caps: DeviceCaps,
    /// `None` means freed (or never allocated): handles are stable indices, so a freed slot
    /// cannot simply be removed without shifting every later handle's index.
    buffers: Vec<Option<Vec<u8>>>,
    next_request: u64,
    ready: Vec<ReadOutcome>,
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
        self.buffers.push(Some(bytes.to_vec()));
        Ok(id)
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.buffers.get(handle.0 as usize).and_then(Option::as_ref).map(Vec::len).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }

    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
        let bytes = self.buffers.get(handle.0 as usize).and_then(Option::as_ref).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
        let request = ReadRequest(self.next_request);
        self.next_request = self.next_request.wrapping_add(1);
        self.ready.push(ReadOutcome { request, handle, result: Ok(bytes) });
        Ok(request)
    }

    fn poll_reads(&mut self) -> Vec<ReadOutcome> {
        std::mem::take(&mut self.ready)
    }

    fn free_buffer(&mut self, handle: HandleId) {
        if let Some(slot) = self.buffers.get_mut(handle.0 as usize) {
            *slot = None;
        }
    }
}

fn mem_context() -> MemContext {
    MemContext { caps: DeviceCaps::default(), buffers: Vec::new(), next_request: 0, ready: Vec::new() }
}

#[test]
fn unknown_provider_is_fault() {
    let registry = ProviderRegistry::new();
    let mut ctx = mem_context();
    let job = GpuJob { provider_id: "missing".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] };
    assert!(matches!(registry.dispatch(&mut ctx, &job), Err(GpuFault::UnknownProvider(_))));
}

#[test]
fn dispatch_batch_keeps_completed_outcomes_when_a_later_job_fails() {
    struct Echo;
    impl crate::GpuProvider for Echo {
        fn id(&self) -> &'static str {
            "test.echo"
        }
        fn run(&self, _ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<crate::GpuJobOutcome, GpuFault> {
            Ok(crate::GpuJobOutcome { outputs: vec![], data: Some(job.params.0.clone()) })
        }
    }
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(Echo));
    let mut ctx = mem_context();
    let ok = GpuJob { provider_id: "test.echo".into(), params: GpuJobParams(b"first".to_vec()), inputs: vec![], outputs: vec![] };
    let bad = GpuJob { provider_id: "missing".into(), params: GpuJobParams(Vec::new()), inputs: vec![], outputs: vec![] };
    let ok2 = GpuJob { provider_id: "test.echo".into(), params: GpuJobParams(b"third".to_vec()), inputs: vec![], outputs: vec![] };
    let results = registry.dispatch_batch(&mut ctx, &[ok, bad, ok2]);
    assert_eq!(results.len(), 3, "one entry per job, in input order");
    assert_eq!(results[0].as_ref().expect("first job succeeded").data.as_deref(), Some(b"first".as_slice()));
    assert!(matches!(results[1], Err(GpuFault::UnknownProvider(_))));
    assert_eq!(results[2].as_ref().expect("job after the failure still ran").data.as_deref(), Some(b"third".as_slice()));
}

#[test]
fn read_buffer_default_resolves_against_request_read_and_poll_reads() {
    let mut ctx = mem_context();
    let handle = ctx.upload_buffer("in", b"hello").expect("upload_buffer");
    assert_eq!(ctx.read_buffer(handle).expect("read_buffer"), b"hello".to_vec());
}

#[test]
fn request_read_on_bad_handle_is_invalid_job() {
    let mut ctx = mem_context();
    assert!(matches!(ctx.request_read(HandleId(99)), Err(GpuFault::InvalidJob(_))));
}

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
