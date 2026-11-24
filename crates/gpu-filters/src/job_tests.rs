//! `craft.filters` through the job contract: on `craft-gpu-null`, which needs no hardware, and
//! with a device when the machine has a graphics adapter.

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, GpuJob, GpuJobParams, HandleId, ProviderRegistry, ReadOutcome, ReadRequest, WorkerKind};
use craft_gpu_null::NullContext;

use super::*;
use crate::tests::{hostile, noise, random_lut};

/// A null context with caps of the test's choosing, which counts the buffers read and can fail
/// every buffer call as a device that has gone.
struct Capped {
    inner: NullContext,
    caps: DeviceCaps,
    reads: usize,
    gone: Option<GpuFault>,
}

impl Capped {
    fn new(caps: DeviceCaps) -> Capped {
        Capped { inner: NullContext::new(WorkerKind::Main), caps, reads: 0, gone: None }
    }

    fn check(&self) -> Result<(), GpuFault> {
        self.gone.clone().map_or(Ok(()), Err)
    }
}

impl GpuContext for Capped {
    fn caps(&self) -> DeviceCaps {
        self.caps.clone()
    }
    fn worker_kind(&self) -> WorkerKind {
        self.inner.worker_kind()
    }
    fn upload_buffer(&mut self, label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault> {
        self.check()?;
        self.inner.upload_buffer(label, bytes)
    }
    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.check()?;
        self.inner.buffer_len(handle)
    }
    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
        self.check()?;
        self.reads += 1;
        self.inner.request_read(handle)
    }
    fn poll_reads(&mut self) -> Vec<ReadOutcome> {
        self.inner.poll_reads()
    }
    fn free_buffer(&mut self, handle: HandleId) {
        self.inner.free_buffer(handle);
    }
    fn read_buffer(&mut self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        self.check()?;
        self.reads += 1;
        self.inner.read_buffer(handle)
    }
}

fn registry(provider: FilterProvider) -> ProviderRegistry {
    let mut r = ProviderRegistry::new();
    r.register(Box::new(provider));
    r
}

/// Run `job` on `ctx` and read its one output back.
fn dispatch(r: &ProviderRegistry, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<Vec<u8>, GpuFault> {
    let outcome = r.dispatch(ctx, job)?;
    assert_eq!(outcome.outputs.len(), 1);
    assert_eq!(outcome.data, None);
    ctx.read_buffer(outcome.outputs[0])
}

fn invalid(result: Result<impl std::fmt::Debug, GpuFault>) -> String {
    match result {
        Err(GpuFault::InvalidJob(message)) => message,
        other => panic!("expected an invalid job, got {other:?}"),
    }
}

fn filters() -> Vec<Filter> {
    vec![Filter::GaussianBlur { sigma: 2.5 }, Filter::Lut(Box::new(random_lut(1))), Filter::Composite { opacity: 0.4 }]
}

#[test]
fn the_registry_runs_every_filter_on_the_null_context_as_the_cpu_does() {
    let r = registry(FilterProvider::new());
    let mut ctx = NullContext::new(WorkerKind::Main);
    let (w, h) = (23, 11);
    let (a, b) = (noise(w, h, 2), noise(w, h, 3));
    let ha = ctx.upload_buffer("a", &a).expect("upload");
    let hb = ctx.upload_buffer("b", &b).expect("upload");
    for filter in filters() {
        let spec = FilterJob::new(filter.clone(), w, h);
        let (handles, images): (Vec<HandleId>, Vec<&[u8]>) = if filter.inputs() == 2 { (vec![ha, hb], vec![&a, &b]) } else { (vec![ha], vec![&a]) };
        let out = dispatch(&r, &mut ctx, &spec.job(&handles)).expect("dispatch");
        assert_eq!(out, cpu::apply(&filter, w, h, &images).expect("cpu"), "{filter:?}");
    }
}

#[test]
fn parameters_read_back_as_written() {
    for filter in filters().into_iter().chain([
        Filter::GaussianBlur { sigma: 1e-7 },
        Filter::GaussianBlur { sigma: MAX_SIGMA },
        Filter::Composite { opacity: 1.0 },
    ]) {
        let spec = FilterJob::new(filter, 1920, 1080);
        assert_eq!(FilterJob::parse(&spec.params()).expect("parse"), spec);
    }
}

#[test]
fn hostile_parameters_are_invalid_jobs() {
    let table = "00".repeat(256);
    let cases: Vec<Vec<u8>> = [
        "op:blur;w:4;h:4;sigma:NaN".to_string(),
        "op:blur;w:4;h:4;sigma:nan".into(),
        "op:blur;w:4;h:4;sigma:inf".into(),
        "op:blur;w:4;h:4;sigma:-1".into(),
        "op:blur;w:4;h:4;sigma:200.5".into(),
        "op:blur;w:4;h:4;sigma:1e39".into(),
        "op:blur;w:4;h:4;sigma:".into(),
        "op:blur;w:4;h:4".into(),
        "op:blur;w:4;h:4;sigma:1;sigma:2".into(),
        "op:blur;w:4;h:4;sigma:1;opacity:1".into(),
        "op:blur;w:4;h:4;sigma:1;depth:3".into(),
        "op:blur;w:0;h:4;sigma:1".into(),
        "op:blur;w:-4;h:4;sigma:1".into(),
        "op:blur;w:+4;h:4;sigma:1".into(),
        "op:blur;w:99999999999;h:4;sigma:1".into(),
        "op:blur;w:16385;h:1;sigma:1".into(),
        "op:blur;w:16384;h:16384;sigma:1".into(),
        "op:sharpen;w:4;h:4".into(),
        "w:4;h:4;sigma:1".into(),
        "op:over;w:4;h:4;opacity:1.5".into(),
        "op:over;w:4;h:4;opacity:NaN".into(),
        "op:over;w:4;h:4;opacity:-0.1".into(),
        format!("op:lut;w:4;h:4;r:{table};g:{table};b:{table}"),
        format!("op:lut;w:4;h:4;r:{table};g:{table};b:{table};a:{}", "0".repeat(511)),
        format!("op:lut;w:4;h:4;r:{table};g:{table};b:{table};a:{}", "zz".repeat(256)),
        format!("op:lut;w:4;h:4;r:{table};g:{table};b:{table};a:{table};sigma:1"),
        "".into(),
        ";".into(),
        "op".into(),
        "x".repeat(5000),
    ]
    .into_iter()
    .map(String::into_bytes)
    .chain([vec![0xff, 0xfe, b':', b'1']])
    .collect();
    let r = registry(FilterProvider::new());
    let mut ctx = NullContext::new(WorkerKind::Main);
    let input = ctx.upload_buffer("in", &noise(4, 4, 4)).expect("upload");
    for params in cases {
        let job = GpuJob { provider_id: PROVIDER_ID.into(), params: GpuJobParams(params.clone()), inputs: vec![input], outputs: Vec::new() };
        let message = invalid(r.dispatch(&mut ctx, &job));
        assert!(message.starts_with(PROVIDER_ID), "{:?}: {message}", String::from_utf8_lossy(&params));
    }
}

#[test]
fn a_job_with_the_wrong_buffers_is_invalid() {
    let r = registry(FilterProvider::new());
    let mut ctx = NullContext::new(WorkerKind::Main);
    let short = ctx.upload_buffer("short", &noise(4, 3, 5)).expect("upload");
    let good = ctx.upload_buffer("good", &noise(4, 4, 6)).expect("upload");
    let blur = FilterJob::new(Filter::GaussianBlur { sigma: 1.0 }, 4, 4);
    let over = FilterJob::new(Filter::Composite { opacity: 1.0 }, 4, 4);
    invalid(r.dispatch(&mut ctx, &blur.job(&[short])));
    invalid(r.dispatch(&mut ctx, &blur.job(&[good, good])));
    invalid(r.dispatch(&mut ctx, &blur.job(&[])));
    invalid(r.dispatch(&mut ctx, &over.job(&[good])));
    invalid(r.dispatch(&mut ctx, &over.job(&[good, short])));
    let mut named = blur.job(&[good]);
    named.outputs.push(good);
    invalid(r.dispatch(&mut ctx, &named));
    // A handle the context never issued is the context's own fault, passed through.
    assert!(r.dispatch(&mut ctx, &blur.job(&[HandleId(999)])).is_err());
}

#[test]
fn a_job_over_the_buffer_limits_is_refused_before_any_input_is_read() {
    // 16 × 16 RGBA8 is 1024 bytes; the blur's intermediate image is 2048.
    let caps = DeviceCaps { max_buffer_size: 1024, ..DeviceCaps::default() };
    let r = registry(FilterProvider::new());
    let mut ctx = Capped::new(caps);
    let input = ctx.upload_buffer("in", &noise(16, 16, 7)).expect("upload");
    let message = invalid(r.dispatch(&mut ctx, &FilterJob::new(Filter::GaussianBlur { sigma: 1.0 }, 16, 16).job(&[input])));
    assert!(message.contains("2048"), "{message}");
    assert_eq!(ctx.reads, 0, "the input was read before the job was refused");
    // The lookup tables need no buffer larger than the image, so they fit.
    dispatch(&r, &mut ctx, &FilterJob::new(Filter::Lut(Box::default()), 16, 16).job(&[input])).expect("lut fits");
    // The storage binding limit counts as much as the buffer limit.
    let mut ctx = Capped::new(DeviceCaps { max_storage_buffer_binding_size: 1023, ..DeviceCaps::default() });
    let input = ctx.upload_buffer("in", &noise(16, 16, 7)).expect("upload");
    invalid(r.dispatch(&mut ctx, &FilterJob::new(Filter::Lut(Box::default()), 16, 16).job(&[input])));
    assert_eq!(ctx.reads, 0);
}

#[test]
fn a_context_without_a_device_fails_the_job_with_its_own_fault() {
    let r = registry(FilterProvider::new());
    for gone in [GpuFault::DeviceUnavailable("no adapter".into()), GpuFault::DeviceLost("reset".into())] {
        let mut ctx = Capped::new(DeviceCaps::default());
        let input = ctx.upload_buffer("in", &noise(4, 4, 8)).expect("upload");
        ctx.gone = Some(gone.clone());
        let result = r.dispatch(&mut ctx, &FilterJob::new(Filter::GaussianBlur { sigma: 1.0 }, 4, 4).job(&[input]));
        assert_eq!(result, Err(gone));
    }
    assert!(!FilterProvider::new().has_device());
}

#[test]
fn hostile_pixels_run_through_every_filter() {
    let r = registry(FilterProvider::new());
    let mut ctx = NullContext::new(WorkerKind::Main);
    for (w, h) in [(1, 1), (1, 9), (9, 1), (17, 5)] {
        let a = ctx.upload_buffer("a", &hostile(w, h, 9)).expect("upload");
        let b = ctx.upload_buffer("b", &hostile(w, h, 10)).expect("upload");
        for filter in filters().into_iter().chain([Filter::GaussianBlur { sigma: MAX_SIGMA }]) {
            let handles = if filter.inputs() == 2 { vec![a, b] } else { vec![a] };
            let out = dispatch(&r, &mut ctx, &FilterJob::new(filter, w, h).job(&handles)).expect("dispatch");
            assert_eq!(out.len(), (w * h * 4) as usize);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_registry_runs_the_device_provider_as_it_runs_the_cpu_provider() {
    let Some((device, queue, _)) = tests::device() else { return };
    let provider = FilterProvider::with_device(device, queue);
    assert!(provider.has_device());
    let (gpu, cpu) = (registry(provider), registry(FilterProvider::new()));
    let mut ctx = NullContext::new(WorkerKind::Main);
    let (w, h) = (37, 21);
    let a = ctx.upload_buffer("a", &noise(w, h, 11)).expect("upload");
    let b = ctx.upload_buffer("b", &noise(w, h, 12)).expect("upload");
    for filter in filters() {
        let handles = if filter.inputs() == 2 { vec![a, b] } else { vec![a] };
        let job = FilterJob::new(filter.clone(), w, h).job(&handles);
        let on_device = dispatch(&gpu, &mut ctx, &job).expect("device");
        let on_cpu = dispatch(&cpu, &mut ctx, &job).expect("cpu");
        assert!(crate::tests::max_diff(&on_device, &on_cpu) <= 1, "{filter:?}");
    }
    // The device provider refuses what the CPU one refuses, as the same fault.
    let bad = GpuJob {
        provider_id: PROVIDER_ID.into(),
        params: GpuJobParams(b"op:blur;w:37;h:21;sigma:NaN".to_vec()),
        inputs: vec![a],
        outputs: Vec::new(),
    };
    invalid(gpu.dispatch(&mut ctx, &bad));
}
