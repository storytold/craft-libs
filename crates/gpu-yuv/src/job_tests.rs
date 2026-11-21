//! `craft.yuv` through the job contract: on `craft-gpu-null`, which needs no hardware, and with a
//! device when the machine has a graphics adapter.

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, GpuJob, GpuJobParams, HandleId, ProviderRegistry, ReadOutcome, ReadRequest, WorkerKind};
use craft_gpu_null::NullContext;

use super::*;

const ALL: [Format; 3] = [Format::Nv12, Format::P010, Format::I420];
const MATRICES: [Matrix; 3] = [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020];
const RANGES: [Range; 2] = [Range::Limited, Range::Full];

/// A null context with caps of the test's choosing, which counts the buffers read.
struct Capped {
    inner: NullContext,
    caps: DeviceCaps,
    reads: usize,
}

impl Capped {
    fn new(caps: DeviceCaps) -> Capped {
        Capped { inner: NullContext::new(WorkerKind::Main), caps, reads: 0 }
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
        self.inner.upload_buffer(label, bytes)
    }
    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.inner.buffer_len(handle)
    }
    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
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
        self.reads += 1;
        self.inner.read_buffer(handle)
    }
}

fn registry(provider: YuvProvider) -> ProviderRegistry {
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

#[test]
fn the_registry_runs_craft_yuv_on_the_null_context() {
    let r = registry(YuvProvider::new());
    let mut ctx = NullContext::new(WorkerKind::Main);
    let px = tests::noise(64, 36);
    let input = ctx.upload_buffer("rgba", &px).unwrap();
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                for (w, h) in [(64, 36), (32, 18), (100, 50)] {
                    let frame = dispatch(&r, &mut ctx, &YuvJob::new(c, 64, 36, w, h).job(input)).unwrap();
                    assert_eq!(frame, cpu::convert(&px, 64, 36, c, w, h).unwrap(), "{c:?} {w}×{h}");
                    assert_eq!(frame.len(), FrameLayout::new(format, w, h).unwrap().frame_bytes());
                }
            }
        }
    }
}

#[test]
fn parameters_read_back_as_written() {
    let job = YuvJob::new(Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited), 1920, 1080, 1280, 720);
    assert_eq!(job.params().0, b"fmt:nv12;matrix:bt709;range:limited;sw:1920;sh:1080;w:1280;h:720");
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let job = YuvJob::new(Conversion::new(format, matrix, range), 7, u32::MAX, 0, 4);
                assert_eq!(YuvJob::parse(&job.params()), Ok(job));
            }
        }
    }
    let job = YuvJob::new(Conversion::new(Format::P010, Matrix::Bt2020, Range::Full), 3, 5, 8, 6).job(HandleId(9));
    assert_eq!((job.provider_id.as_str(), job.inputs.as_slice(), job.outputs.len()), (PROVIDER_ID, [HandleId(9)].as_slice(), 0));
}

#[test]
fn bad_parameters_are_invalid_jobs() {
    let good = "fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4";
    let cases: [(&[u8], &str); 14] = [
        (b"", "is not a key:value field"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4", "`h` is missing"),
        (b"fmt:nv12;fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4", "`fmt` is given twice"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4;alpha:1", "unknown key `alpha`"),
        (b"fmt:NV12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4", "`NV12` is not a value of `fmt`"),
        (b"fmt:nv12;matrix:bt2100;range:limited;sw:4;sh:4;w:4;h:4", "`bt2100` is not a value of `matrix`"),
        (b"fmt:nv12;matrix:bt709;range:studio;sw:4;sh:4;w:4;h:4", "`studio` is not a value of `range`"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:+4;sh:4;w:4;h:4", "`+4` is not a value of `sw`"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:-4;w:4;h:4", "`-4` is not a value of `sh`"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:;h:4", "`` is not a value of `w`"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4294967296", "`4294967296` is not a value of `h`"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4;", "`` is not a key:value field"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:4.0", "`4.0` is not a value of `h`"),
        (b"fmt:nv12;matrix:bt709;range:limited;sw:4;sh:4;w:4;h:\xff", "not UTF-8"),
    ];
    assert!(YuvJob::parse(&GpuJobParams(good.as_bytes().to_vec())).is_ok());
    for (params, want) in cases {
        let message = invalid(YuvJob::parse(&GpuJobParams(params.to_vec())));
        assert!(message.starts_with("craft.yuv: ") && message.contains(want), "{message:?} for {:?}", String::from_utf8_lossy(params));
    }
}

#[test]
fn inputs_outputs_and_sizes_are_checked_before_any_read() {
    let r = registry(YuvProvider::new());
    let mut ctx = Capped::new(DeviceCaps::default());
    let c = Conversion::new(Format::I420, Matrix::Bt601, Range::Limited);
    let input = ctx.upload_buffer("rgba", &tests::noise(8, 4)).unwrap();
    let mut job = YuvJob::new(c, 8, 4, 8, 4).job(input);
    job.inputs.clear();
    assert!(invalid(r.dispatch(&mut ctx, &job)).contains("expects one RGBA8 input, not 0"));
    job.inputs = vec![input, input];
    assert!(invalid(r.dispatch(&mut ctx, &job)).contains("not 2"));
    job.inputs = vec![input];
    job.outputs = vec![input];
    assert!(invalid(r.dispatch(&mut ctx, &job)).contains("names no outputs"));
    // A buffer whose size is not the source's, an odd or zero side, and a missing buffer.
    let wrong = [
        YuvJob::new(c, 8, 8, 8, 4).job(input),
        YuvJob::new(c, 0, 4, 8, 4).job(input),
        YuvJob::new(c, 8, 4, 7, 4).job(input),
        YuvJob::new(c, 8, 4, 8, 0).job(input),
        YuvJob::new(c, u32::MAX, u32::MAX, 8, 4).job(input),
    ];
    for job in &wrong {
        invalid(r.dispatch(&mut ctx, job));
    }
    assert!(matches!(r.dispatch(&mut ctx, &YuvJob::new(c, 8, 4, 8, 4).job(HandleId(99))), Err(GpuFault::InvalidJob(_))));
    assert_eq!(ctx.reads, 0, "no job above reads its input");
    assert!(r.dispatch(&mut ctx, &YuvJob::new(c, 8, 4, 8, 4).job(input)).is_ok());
    assert_eq!(ctx.reads, 1);
}

#[test]
fn jobs_past_the_context_caps_are_refused() {
    let r = registry(YuvProvider::new());
    let caps = DeviceCaps { max_texture_dimension_2d: 64, max_buffer_size: 16128, ..DeviceCaps::default() };
    let mut ctx = Capped::new(caps);
    let c = Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited);
    let small = ctx.upload_buffer("small", &tests::noise(64, 4)).unwrap();
    let wide = ctx.upload_buffer("wide", &tests::noise(66, 2)).unwrap();
    // The source and the output sides against the largest texture side.
    assert!(invalid(r.dispatch(&mut ctx, &YuvJob::new(c, 66, 2, 64, 2).job(wide))).contains("66×2 is larger than the largest texture side, 64"));
    assert!(invalid(r.dispatch(&mut ctx, &YuvJob::new(c, 64, 4, 66, 4).job(small))).contains("66×4"));
    // NV12 read-back rows at width 64 are padded to 256 bytes: 64 × 44 packs into 4224 bytes
    // but reads back through 256 × (44 + 22) = 16896, more than the 16128-byte limit; 64 × 42
    // reads back through exactly 16128.
    assert_eq!(FrameLayout::new(Format::Nv12, 64, 44).unwrap().frame_bytes(), 4224);
    let refused = invalid(r.dispatch(&mut ctx, &YuvJob::new(c, 64, 4, 64, 44).job(small)));
    assert!(refused.contains("16896 bytes is larger than the largest buffer, 16128"), "{refused}");
    assert!(dispatch(&r, &mut ctx, &YuvJob::new(c, 64, 4, 64, 42).job(small)).is_ok());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_provider_with_a_device_matches_the_cpu_through_the_registry() {
    let Some((device, queue)) = tests::device() else { return };
    let provider = YuvProvider::with_device(device, queue);
    assert!(provider.has_device() && !YuvProvider::new().has_device());
    let r = registry(provider);
    let mut ctx = NullContext::new(WorkerKind::Main);
    let (noise, ramps) = (tests::noise(64, 36), tests::ramps(160, 90));
    let (n, s) = (ctx.upload_buffer("noise", &noise).unwrap(), ctx.upload_buffer("ramps", &ramps).unwrap());
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                let frame = dispatch(&r, &mut ctx, &YuvJob::new(c, 64, 36, 64, 36).job(n)).unwrap();
                tests::assert_close(&frame, &cpu::convert(&noise, 64, 36, c, 64, 36).unwrap(), c, "64×36 through the registry");
            }
        }
        // Each conversion's converter is made once and used again: the second run of a
        // conversion gives the same frame.
        let c = Conversion::new(format, Matrix::Bt709, Range::Limited);
        for (w, h) in [(80, 46), (240, 136)] {
            let frame = dispatch(&r, &mut ctx, &YuvJob::new(c, 160, 90, w, h).job(s)).unwrap();
            tests::assert_close(&frame, &cpu::convert(&ramps, 160, 90, c, w, h).unwrap(), c, &format!("160×90 to {w}×{h} through the registry"));
            assert_eq!(frame, dispatch(&r, &mut ctx, &YuvJob::new(c, 160, 90, w, h).job(s)).unwrap());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_device_smaller_than_the_context_caps_refuses_the_job_without_a_device_error() {
    let Some((device, queue)) = tests::device_with(|l| wgpu::Limits { max_texture_dimension_2d: 64, ..l }) else { return };
    let r = registry(YuvProvider::with_device(device, queue));
    // The null context's caps allow 8192, so only the device's own limit stops these.
    let mut ctx = NullContext::new(WorkerKind::Main);
    let c = Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited);
    let big = ctx.upload_buffer("big", &tests::noise(128, 2)).unwrap();
    let small = ctx.upload_buffer("small", &tests::noise(8, 2)).unwrap();
    assert!(invalid(r.dispatch(&mut ctx, &YuvJob::new(c, 128, 2, 8, 2).job(big))).contains("128×2 is larger than the largest texture side, 64"));
    assert!(invalid(r.dispatch(&mut ctx, &YuvJob::new(c, 8, 2, 128, 2).job(small))).contains("128×2"));
    // The device still runs a job that fits after refusing two.
    let frame = dispatch(&r, &mut ctx, &YuvJob::new(c, 8, 2, 8, 2).job(small)).unwrap();
    tests::assert_close(&frame, &cpu::convert(&tests::noise(8, 2), 8, 2, c, 8, 2).unwrap(), c, "8×2 after two refusals");
}
