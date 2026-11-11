#![allow(clippy::expect_used)]

//! RFC-0022 acceptance requirement 2: real backends beside the null one. `craft-gpu-null` stays
//! mandatory for every consumer (it cannot validate shader compilation, resource rules, or
//! numerical results), so these tests run representative jobs on a real wgpu device when one is
//! available, and skip loudly rather than failing the suite when the host genuinely has none
//! (`cargo test` without `--features adapter-probe,native-vulkan` already exercises the masked
//! path: no adapter is ever requested, and every context here falls back to the staging path,
//! same shape as `craft-gpu-null`).
//!
//! Tolerance: every job below is byte/structural equality, not a numeric comparison with slack.
//! That is honest for what runs today: no provider dispatches a real WGSL compute job yet
//! (`craft-gpu-providers` ships CPU stand-ins by design; see `docs/gpu-providers.md`), so the only
//! thing a real device adds over the null one here is the upload → real `wgpu::Buffer` → copy →
//! `map_async` → read round trip itself. A provider that dispatches actual WGSL will need a
//! stated numeric tolerance of its own when it lands; this suite is not that provider.

use craft_gpu::{GpuContext, GpuJob, GpuJobParams, GpuProvider, WorkerKind};
use craft_gpu_providers::{EchoProvider, PremulCompositeProvider, ReadbackProvider};

use crate::WgpuContext;

/// A real device if this host has one; otherwise `None` with a loud, honest message instead of a
/// failed assertion. Call at the top of every test in this module.
fn real_context_or_skip(test_name: &str) -> Option<WgpuContext> {
    let ctx = WgpuContext::new(WorkerKind::Main);
    if ctx.has_device() {
        return Some(ctx);
    }
    eprintln!(
        "SKIP {test_name}: no real wgpu device in this environment (no backend feature compiled \
         in, or `request_adapter` found nothing). This is RFC-0022's defined no-adapter path, not \
         a failure; craft-gpu-null already covers the portable contract. Run with \
         `--features adapter-probe,native-vulkan` against a Vulkan adapter (a software one, such \
         as lavapipe/llvmpipe via mesa-vulkan-drivers, is enough) to exercise this suite for real."
    );
    None
}

#[test]
fn real_device_reports_a_non_null_backend_and_empty_features_it_actually_requested() {
    let Some(ctx) = real_context_or_skip("real_device_reports_a_non_null_backend_and_empty_features_it_actually_requested") else {
        return;
    };
    let caps = ctx.caps();
    assert_ne!(caps.backend_name, "null", "a real device must not report the null backend name");
    assert_ne!(caps.device_name, "no-adapter", "has_device() was true; caps must not be the no-adapter fallback");
    assert_ne!(caps.device_name, "probe-not-run", "has_device() was true; caps must not be the probe-skipped fallback");
    // This context asks the device for no features yet (no provider dispatches real WGSL): the
    // device reports exactly what was requested, per `Device::features`' own contract, so this is
    // device-truth agreeing with an intentionally empty request, not an unprobed default.
    assert!(caps.features.is_empty(), "no feature was requested, so none should be reported enabled: {:?}", caps.features);
}

#[test]
fn real_device_upload_and_async_readback_round_trips_bytes_exactly() {
    let Some(mut ctx) = real_context_or_skip("real_device_upload_and_async_readback_round_trips_bytes_exactly") else {
        return;
    };
    let payload: Vec<u8> = (0..256u32).map(|b| b as u8).collect();
    let handle = ctx.upload_buffer("round-trip", &payload).expect("upload_buffer on a real device");
    assert_eq!(ctx.buffer_len(handle).expect("buffer_len"), payload.len());

    // Exercise request_read + poll_reads directly (the primitive every backend implements),
    // not just the read_buffer convenience, so the async shape itself is under test.
    let request = ctx.request_read(handle).expect("request_read");
    let mut got = None;
    // Non-blocking poll_reads() in a bounded loop, exercising the exact primitive a worker's own
    // message/poll loop would call; read_buffer_convenience (a separate test) covers the blocking
    // path. The bound is generous for a software rasterizer on a 256-byte buffer, never relying
    // on wall-clock sleeps.
    for _ in 0..1_000_000 {
        if let Some(outcome) = ctx.poll_reads().into_iter().find(|o| o.request == request) {
            got = Some(outcome.result.expect("map_async + Device::poll round trip"));
            break;
        }
    }
    assert_eq!(got.expect("the real device never completed the mapped read within the bounded poll loop"), payload);
}

#[test]
fn real_device_read_buffer_convenience_blocks_to_completion() {
    let Some(mut ctx) = real_context_or_skip("real_device_read_buffer_convenience_blocks_to_completion") else {
        return;
    };
    let handle = ctx.upload_buffer("blocking-read", b"craft-gpu-wgpu").expect("upload_buffer");
    assert_eq!(ctx.read_buffer(handle).expect("read_buffer"), b"craft-gpu-wgpu".to_vec());
}

/// RFC-0022 requirement 2's "representative jobs": the same `craft-gpu-providers` CPU stand-ins
/// `craft-runtime` dispatches against the null context in CI also run, unmodified, against a
/// real device, through the same `&mut dyn GpuContext` seam.
#[test]
fn shared_providers_run_against_a_real_device_through_the_same_seam_as_the_null_context() {
    let Some(mut ctx) = real_context_or_skip("shared_providers_run_against_a_real_device_through_the_same_seam_as_the_null_context") else {
        return;
    };

    let echo = EchoProvider;
    let input = ctx.upload_buffer("echo-in", b"xy").expect("upload_buffer");
    let job = GpuJob { provider_id: "craft.test.echo".into(), params: GpuJobParams(b"a".to_vec()), inputs: vec![input], outputs: vec![] };
    let out = echo.run(&mut ctx, &job).expect("craft.test.echo on a real device");
    assert_eq!(out.data.as_deref(), Some(b"axy".as_slice()));

    let composite = PremulCompositeProvider;
    let dst = ctx.upload_buffer("dst", &[0, 255, 0, 255]).expect("upload dst");
    let src = ctx.upload_buffer("src", &[0, 0, 255, 128]).expect("upload src");
    let job =
        GpuJob { provider_id: "craft.composite.premul".into(), params: GpuJobParams(b"w:1,h:1".to_vec()), inputs: vec![dst, src], outputs: vec![] };
    let out = composite.run(&mut ctx, &job).expect("craft.composite.premul on a real device");
    let bytes = ctx.read_buffer(out.outputs[0]).expect("read composite output off a real device");
    assert_eq!(bytes, vec![0, 127, 128, 255]);

    let readback = ReadbackProvider;
    let rgba = ctx.upload_buffer("rgba", &[10, 20, 30, 40]).expect("upload rgba");
    let job = GpuJob { provider_id: "craft.readback".into(), params: GpuJobParams(b"w:1,h:1,fmt:u8".to_vec()), inputs: vec![rgba], outputs: vec![] };
    let out = readback.run(&mut ctx, &job).expect("craft.readback fmt:u8 on a real device");
    let bytes = ctx.read_buffer(out.outputs[0]).expect("read readback output off a real device");
    assert_eq!(bytes, vec![10, 20, 30, 40]);
}

/// RFC-0022 requirement 1's device-loss half: a stated fault, not a panic, and a documented
/// recovery. `Backing`/`DeviceBacking` are crate-private, so this test reaches `ctx.backing`
/// directly to force the one cause it can trigger deterministically (`device.destroy()`) rather
/// than waiting on a real driver reset.
#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
#[test]
fn device_destroy_surfaces_as_device_lost_and_stops_further_buffer_calls() {
    use craft_gpu::GpuFault;

    use crate::Backing;

    let Some(mut ctx) = real_context_or_skip("device_destroy_surfaces_as_device_lost_and_stops_further_buffer_calls") else {
        return;
    };
    assert!(!ctx.is_device_lost(), "a freshly created device must not already report loss");

    if let Backing::Device(d) = &ctx.backing {
        d.device.destroy();
        // The lost closure can run during destroy() or on the next poll, depending on backend;
        // give it the chance either way.
        let _ = d.device.poll(wgpu::PollType::Poll);
    }

    // poll_reads() is the other place the trait docs promise this surfaces, for a caller that
    // polls instead of issuing a buffer call first.
    let _ = ctx.poll_reads();
    assert!(ctx.is_device_lost(), "device.destroy() must surface through is_device_lost(), per the GpuContext trait docs");

    let after = ctx.upload_buffer("after-loss", b"x");
    assert!(matches!(after, Err(GpuFault::DeviceLost(_))), "a call after device loss must return GpuFault::DeviceLost, got {after:?}");
}
