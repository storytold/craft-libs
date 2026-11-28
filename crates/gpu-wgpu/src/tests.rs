#![allow(clippy::expect_used)]

//! RFC-0022 acceptance requirement 2: real backends beside the null one. `craft-gpu-null` stays
//! mandatory for every consumer (it cannot validate shader compilation, resource rules, or
//! numerical results), so these tests run representative jobs on a real wgpu device when one is
//! available. Runs against a software Vulkan adapter in a local container are local testing, not
//! hosted CI coverage: hosted lanes and the pilot consumers' browser and hardware validation
//! remain their own, separate requirements. The suite skips loudly when the host has no adapter
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

/// The staging path promises "same shape as `craft-gpu-null`": an empty upload is accepted and
/// reads back empty rather than being refused.
#[test]
fn staging_path_accepts_an_empty_buffer_and_reads_it_back_empty() {
    let mut ctx = WgpuContext::with_caps(WorkerKind::Main, craft_gpu::DeviceCaps::default());
    let handle = ctx.upload_buffer("empty", b"").expect("upload empty");
    assert_eq!(ctx.buffer_len(handle).expect("buffer_len"), 0);
    assert_eq!(ctx.read_buffer(handle).expect("read_buffer"), Vec::<u8>::new());
}

/// `read_buffer` must not eat completions that belong to another outstanding `request_read`.
#[test]
fn staging_read_buffer_keeps_another_outstanding_requests_completion_queued() {
    let mut ctx = WgpuContext::with_caps(WorkerKind::Main, craft_gpu::DeviceCaps::default());
    let a = ctx.upload_buffer("a", b"aa").expect("upload a");
    let b = ctx.upload_buffer("b", b"bb").expect("upload b");
    let request_a = ctx.request_read(a).expect("request_read a");
    assert_eq!(ctx.read_buffer(b).expect("read_buffer b"), b"bb".to_vec());
    let outcomes = ctx.poll_reads();
    assert_eq!(outcomes.len(), 1, "a's completion must survive b's read_buffer");
    assert_eq!(outcomes[0].request, request_a);
    assert_eq!(outcomes[0].result.as_deref(), Ok(b"aa".as_slice()));
}

/// CodeRabbit finding on lib.rs:228: if `request_adapter` succeeds but `request_device` fails,
/// the fallback must be the conservative fixed value, never a second, successful `probe_caps()`
/// adapter query -- which would report the adapter's advertised limits as if they came from a
/// granted device, for a context that holds none. Forcing `request_device` itself to fail is
/// hardware-dependent and not reliably reproducible in CI, so this tests the fallback value
/// `WgpuContext::new` falls back to directly: it must be the plain, fixed value, never one that
/// looks like a real probe result.
#[test]
fn the_no_device_fallback_caps_are_the_plain_fixed_value_not_a_probe_result() {
    let caps = crate::probe_fallback("wgpu", "no-device");
    assert_eq!(caps.backend_name, "wgpu");
    assert_eq!(caps.device_name, "no-device");
    assert_eq!(caps, craft_gpu::DeviceCaps { backend_name: "wgpu".into(), device_name: "no-device".into(), ..craft_gpu::DeviceCaps::default() });
}

/// CodeRabbit finding on lib.rs:299: an allocation failure used to be able to reach wgpu's
/// default uncaptured-error handler, which panics. An error scope around buffer creation now
/// converts that into a `GpuFault` instead. A genuine out-of-memory condition is not reliably
/// reproducible in CI (it depends on how much free VRAM the host actually has), so this is the
/// regression half: the added error scope must not misfire on an entirely ordinary allocation,
/// across both the create scope and the write scope that now follows it (see the next doc
/// comment for why those are two separate scopes rather than one spanning both calls).
#[test]
fn upload_buffer_succeeds_normally_with_the_allocation_error_scope_in_place() {
    let Some(mut ctx) = real_context_or_skip("upload_buffer_succeeds_normally_with_the_allocation_error_scope_in_place") else {
        return;
    };
    let handle = ctx.upload_buffer("scope-regression", b"ok").expect("a normal allocation must not report a false allocation error");
    assert_eq!(ctx.read_buffer(handle).expect("read back"), b"ok".to_vec());
}

/// CodeRabbit finding on lib.rs:329 (upload_buffer) and lib.rs:404 (request_read): the original
/// code popped its single error scope only after write_buffer (respectively, after the copy and
/// its submission), so a failed create_buffer would leave an invalid buffer in play across the
/// later call that used it. write_buffer against an invalid buffer raises a VALIDATION error, a
/// kind the OutOfMemory-only scope never catches, so it would reach wgpu's default
/// uncaptured-error handler and panic there instead of surfacing as a `GpuFault` -- the same
/// crash the scope was added to prevent in the first place.
///
/// A genuine allocation failure is not reliably reproducible in CI (it depends on how much free
/// VRAM/host memory lavapipe actually has to give), so this cannot be a runtime test that forces
/// `create_buffer` to fail and then observes `write_buffer` never ran. What can be verified
/// honestly, and is verified here, is the ordering itself: `upload_buffer`'s source now pops the
/// create scope and returns on error strictly before it pushes the write scope or calls
/// `write_buffer`, and `request_read`'s source pops the staging-buffer create scope and returns
/// on error strictly before it creates the copy encoder or calls `queue.submit`. Both orderings
/// are asserted directly against the source text below rather than left as an unverified claim in
/// a comment; the success-path test above is the runtime half, proving the added scope split does
/// not misfire on the ordinary case.
#[test]
fn a_failed_allocation_scope_pop_is_ordered_strictly_before_the_call_that_would_use_the_buffer() {
    let source = include_str!("lib.rs");
    let upload_create_pop = source.find("create_scope.pop()").expect("upload_buffer must pop a create scope");
    let upload_write_scope_push = source.find("let write_scope = d.device.push_error_scope").expect("upload_buffer must push a separate write scope");
    assert!(
        upload_create_pop < upload_write_scope_push,
        "upload_buffer must check the create scope's result before it ever pushes the write scope or calls write_buffer"
    );

    let read_create_pop =
        source.rfind("device reported an allocation error creating the staging buffer").expect("request_read must pop a create scope");
    let read_copy_scope_push = source.rfind("let copy_scope = d.device.push_error_scope").expect("request_read must push a separate copy scope");
    assert!(
        read_create_pop < read_copy_scope_push,
        "request_read must check the staging-buffer create scope's result before it ever pushes the copy scope or touches the encoder"
    );
}

/// CodeRabbit finding on lib.rs:313: nothing released an uploaded buffer's resources, so the
/// buffer table (and the GPU memory behind it) could only ever grow. free_buffer must actually
/// make the handle behave as invalid afterward, and must not disturb any other handle's index.
#[test]
fn staging_free_buffer_invalidates_the_handle_without_shifting_others() {
    let mut ctx = WgpuContext::with_caps(WorkerKind::Main, craft_gpu::DeviceCaps::default());
    let a = ctx.upload_buffer("a", b"aa").expect("upload a");
    let b = ctx.upload_buffer("b", b"bb").expect("upload b");

    ctx.free_buffer(a);

    assert!(matches!(ctx.buffer_len(a), Err(craft_gpu::GpuFault::InvalidJob(_))), "a freed handle must behave as invalid");
    assert_eq!(ctx.read_buffer(b).expect("b must be unaffected by freeing a"), b"bb".to_vec());
}

/// The same guarantee on a real device: freeing one handle must not disturb another's index or
/// its own buffer.
#[test]
fn real_device_free_buffer_invalidates_the_handle_without_shifting_others() {
    let Some(mut ctx) = real_context_or_skip("real_device_free_buffer_invalidates_the_handle_without_shifting_others") else {
        return;
    };
    let a = ctx.upload_buffer("a", b"aa").expect("upload a");
    let b = ctx.upload_buffer("b", b"bb").expect("upload b");

    ctx.free_buffer(a);

    assert!(matches!(ctx.buffer_len(a), Err(craft_gpu::GpuFault::InvalidJob(_))), "a freed handle must behave as invalid on a real device too");
    assert_eq!(ctx.read_buffer(b).expect("b must be unaffected by freeing a"), b"bb".to_vec());
}

/// CodeRabbit finding on lib.rs:438: `queue.write_buffer` only records its copy in wgpu's
/// PendingWrites; without some submission to flush that queue, wgpu-core keeps the destination
/// buffer (and the staging buffer `write_buffer` made internally) alive on its own, so repeated
/// upload_buffer/free_buffer cycles would retain GPU allocations with nothing ever releasing
/// them. `upload_buffer` now flushes with an empty `queue.submit` right after its own
/// write_buffer call. This cannot assert freed VRAM directly (wgpu exposes no such counter), so
/// it instead proves the behavioral guarantee this fix is actually for: many upload/free cycles
/// in a row complete without panicking and without ever running out of handles, which is exactly
/// what an unbounded, unflushed retention would eventually threaten on a long-lived worker.
#[test]
fn real_device_upload_and_free_many_cycles_in_a_row_does_not_panic_or_leak_a_growing_table() {
    let Some(mut ctx) = real_context_or_skip("real_device_upload_and_free_many_cycles_in_a_row_does_not_panic_or_leak_a_growing_table") else {
        return;
    };
    for i in 0..256u32 {
        let bytes = i.to_le_bytes();
        let handle = ctx.upload_buffer("cycle", &bytes).expect("each upload in the cycle must succeed");
        assert_eq!(ctx.read_buffer(handle).expect("read back this cycle's buffer"), bytes.to_vec());
        ctx.free_buffer(handle);
    }
}

/// Freeing a handle twice, or a handle that never existed, is a no-op: callers are not expected
/// to track whether they already freed one.
#[test]
fn free_buffer_is_a_harmless_no_op_when_repeated_or_given_an_unknown_handle() {
    let mut ctx = WgpuContext::with_caps(WorkerKind::Main, craft_gpu::DeviceCaps::default());
    let a = ctx.upload_buffer("a", b"aa").expect("upload a");
    ctx.free_buffer(a);
    ctx.free_buffer(a);
    ctx.free_buffer(craft_gpu::HandleId(999));
}

/// CodeRabbit finding on lib.rs:401: `poll_reads` can only attach a device-lost fault to a read
/// it has something to attach it to, so a caller whose event loop only polls, with nothing
/// outstanding, could never observe loss through it alone. `device_status` covers exactly that
/// gap: it is checked here with no buffer call and no pending read at all after the device is
/// destroyed.
#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
#[test]
fn device_status_surfaces_loss_even_with_nothing_pending() {
    use craft_gpu::GpuFault;

    use crate::Backing;

    let Some(ctx) = real_context_or_skip("device_status_surfaces_loss_even_with_nothing_pending") else {
        return;
    };
    assert_eq!(ctx.device_status(), Ok(()), "a freshly created device must not already report loss");

    if let Backing::Device(d) = &ctx.backing {
        d.device.destroy();
        let _ = d.device.poll(wgpu::PollType::Poll);
    }

    // No upload_buffer, no request_read, no poll_reads: device_status() alone must observe it.
    assert!(matches!(ctx.device_status(), Err(GpuFault::DeviceLost(_))), "device.destroy() must surface through device_status() alone");
}

/// Every context and probe requests its adapter from one process-wide instance, so opening
/// contexts on several threads does not load and unload the Vulkan drivers once per context. No
/// adapter is needed: an instance exists even on a host with no usable backend.
#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
#[test]
fn every_context_and_probe_shares_one_wgpu_instance() {
    let first = crate::native::instance();
    let again = std::thread::spawn(crate::native::instance).join().expect("instance() on another thread");
    assert_eq!(first, again, "a second call, from another thread, must return the same instance");
    assert_eq!(first, crate::native::instance(), "a later call must return the same instance");
}

/// Same two guarantees on a real device: empty buffers round-trip (the readback resolves without
/// mapping, since wgpu refuses an empty map range) and `read_buffer` re-queues completions that
/// belong to another outstanding request.
#[test]
fn real_device_empty_buffer_and_interleaved_read_buffer_behave_like_staging() {
    let Some(mut ctx) = real_context_or_skip("real_device_empty_buffer_and_interleaved_read_buffer_behave_like_staging") else {
        return;
    };
    let empty = ctx.upload_buffer("empty", b"").expect("upload empty");
    assert_eq!(ctx.buffer_len(empty).expect("buffer_len"), 0);
    assert_eq!(ctx.read_buffer(empty).expect("read_buffer empty"), Vec::<u8>::new());

    let a = ctx.upload_buffer("a", b"aa").expect("upload a");
    let b = ctx.upload_buffer("b", b"bb").expect("upload b");
    let request_a = ctx.request_read(a).expect("request_read a");
    assert_eq!(ctx.read_buffer(b).expect("read_buffer b"), b"bb".to_vec());
    let mut seen_a = false;
    for _ in 0..1000 {
        for outcome in ctx.poll_reads() {
            if outcome.request == request_a {
                assert_eq!(outcome.result.as_deref(), Ok(b"aa".as_slice()));
                seen_a = true;
            }
        }
        if seen_a {
            break;
        }
    }
    assert!(seen_a, "a's completion must still arrive through poll_reads after b's read_buffer");
}

/// Devices from `craft-gpu-device`, on wgpu's noop backend, so these run without a GPU.
#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
mod device_from_craft_gpu_device {
    use craft_gpu::{GpuContext, GpuFault, WorkerKind};
    use craft_gpu_device::{DeviceHealth, Fault, Preference, Request};

    use crate::{Backing, WgpuContext, native};

    /// An instance whose only adapter is wgpu's noop one, which wgpu 30 lists as a software
    /// adapter.
    fn noop() -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::NOOP;
        desc.backend_options.noop = wgpu::NoopBackendOptions { enable: true, ..Default::default() };
        wgpu::Instance::new(desc)
    }

    fn opened() -> craft_gpu_device::Opened {
        craft_gpu_device::open(&noop(), None, &Preference::Software, &Request::default()).expect("the noop adapter opens")
    }

    #[test]
    fn the_default_device_is_the_one_craft_gpu_device_opens() {
        let (caps, backing) = native::create_device_on(&noop()).expect("the noop adapter opens");
        let Backing::Device(d) = &backing else { panic!("a device backing") };
        assert_eq!(caps, craft_gpu_device::device_caps(&d.device));
        assert!(caps.software, "{caps:?}");
        assert!(d.health.is_ok() && d.health.device_status().is_ok());
        // no adapter at all: the staging path
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::NOOP;
        assert!(native::create_device_on(&wgpu::Instance::new(desc)).is_none());
    }

    #[test]
    fn the_probe_reports_the_adapter_craft_gpu_device_picks() {
        let opened = opened();
        let caps = native::probe_caps_on(&noop());
        assert_eq!(caps.device_name, opened.candidate.name);
        assert!(caps.software && caps.features.is_empty(), "{caps:?}");
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::NOOP;
        assert_eq!(native::probe_caps_on(&wgpu::Instance::new(desc)).device_name, "no-adapter");
    }

    #[test]
    fn a_context_on_an_opened_device_reads_the_loss_from_its_health() {
        let opened = opened();
        let mut ctx = WgpuContext::from_opened(WorkerKind::Main, &opened);
        assert!(ctx.has_device());
        assert_eq!(ctx.caps(), opened.device_caps());
        assert_eq!(ctx.device_status(), Ok(()));
        // a loss another user of the device records is this context's loss too
        opened.health.mark(Fault::Lost("the driver reset the GPU".into()));
        assert!(ctx.health().is_some_and(|h| h.device_status().is_err()));
        assert_eq!(ctx.device_status(), Err(GpuFault::DeviceLost("the driver reset the GPU".into())));
        assert!(ctx.is_device_lost());
        assert!(matches!(ctx.upload_buffer("after-loss", b"x"), Err(GpuFault::DeviceLost(_))));
    }

    #[test]
    fn a_destroyed_device_is_lost_through_the_health_that_watches_it() {
        let opened = opened();
        let health = DeviceHealth::watch(&opened.device);
        let mut ctx = WgpuContext::with_device(WorkerKind::DedicatedWorker, opened.device.clone(), opened.queue.clone(), health.clone());
        assert_eq!(ctx.caps(), opened.device_caps());
        opened.device.destroy();
        let _ = ctx.poll_reads();
        assert!(health.device_status().is_err(), "{health:?}");
        assert!(matches!(ctx.device_status(), Err(GpuFault::DeviceLost(_))));
        assert!(matches!(ctx.request_read(craft_gpu::HandleId(0)), Err(GpuFault::DeviceLost(_))));
    }

    #[test]
    fn no_work_goes_to_a_device_whose_health_recorded_an_error() {
        let opened = opened();
        let mut ctx = WgpuContext::from_opened(WorkerKind::Main, &opened);
        let handle = ctx.upload_buffer("before", b"abcd").expect("upload on a healthy device");
        // an out-of-memory error stops new work until the app clears it
        opened.health.mark(Fault::Error(craft_gpu_device::OUT_OF_MEMORY.into()));
        assert!(matches!(ctx.upload_buffer("oom", b"x"), Err(GpuFault::Operation(_))));
        assert!(matches!(ctx.request_read(handle), Err(GpuFault::Operation(_))));
        assert_eq!(ctx.device_status(), Ok(()), "only a loss is a device_status fault");
        ctx.free_buffer(handle);
        opened.health.clear_out_of_memory(opened.health.out_of_memory_count());
        let handle = ctx.upload_buffer("after", b"abcd").expect("upload once the error is cleared");
        assert_eq!(ctx.buffer_len(handle), Ok(4));
        // another error stops it for good
        opened.health.mark(Fault::Error("Validation Error".into()));
        assert_eq!(ctx.upload_buffer("validation", b"x"), Err(GpuFault::Operation("Validation Error".into())));
        assert_eq!(ctx.device_status(), Ok(()));
    }

    #[test]
    fn a_read_waiting_when_the_device_reports_an_error_fails_instead_of_polling_forever() {
        let opened = opened();
        let mut ctx = WgpuContext::from_opened(WorkerKind::Main, &opened);
        let handle = ctx.upload_buffer("pending", b"abcd").expect("upload");
        let request = ctx.request_read(handle).expect("request_read");
        opened.health.mark(Fault::Error("Validation Error".into()));
        assert_eq!(ctx.wait_between_reads(request), Some(GpuFault::Operation("Validation Error".into())));
        let Backing::Device(d) = &ctx.backing else { panic!("a device backing") };
        assert!(d.pending.iter().all(|p| p.request != request), "the failed read is dropped");
        // a loss is left to poll_reads, which fails every pending read with it
        let other = ctx.request_read(handle);
        assert!(other.is_err());
        opened.health.mark(Fault::Lost("gone".into()));
        assert_eq!(ctx.wait_between_reads(request), None);
    }

    #[test]
    fn a_staging_context_has_no_health() {
        let ctx = WgpuContext::with_caps(WorkerKind::Main, craft_gpu::DeviceCaps::default());
        assert!(ctx.health().is_none());
    }

    #[test]
    fn a_pending_read_fails_when_validation_and_out_of_memory_coexist() {
        for oom_first in [true, false] {
            let device = opened();
            let mut ctx = WgpuContext::from_opened(WorkerKind::Main, &device);
            let handle = ctx.upload_buffer("pending", b"abcd").expect("upload");
            let request = ctx.request_read(handle).expect("read");
            let faults = [Fault::Error(craft_gpu_device::OUT_OF_MEMORY.into()), Fault::Error("Validation Error".into())];
            for index in if oom_first { [0, 1] } else { [1, 0] } {
                device.health.mark(faults[index].clone());
            }
            let outcomes = ctx.poll_reads();
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].request, request);
            assert_eq!(outcomes[0].result, Err(GpuFault::Operation("Validation Error".into())));
            assert!(ctx.poll_reads().is_empty());
        }
    }

    #[test]
    fn polling_faults_complete_all_pending_reads_once() {
        let device = opened();
        let mut ctx = WgpuContext::from_opened(WorkerKind::Main, &device);
        let handle = ctx.upload_buffer("pending", b"abcd").expect("upload");
        let first = ctx.request_read(handle).expect("first read");
        let second = ctx.request_read(handle).expect("second read");
        device.health.mark(Fault::Error("Validation Error".into()));
        let outcomes = ctx.poll_reads();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().any(|outcome| outcome.request == first));
        assert!(outcomes.iter().any(|outcome| outcome.request == second));
        for outcome in outcomes {
            assert_eq!(outcome.handle, handle);
            assert_eq!(outcome.result, Err(GpuFault::Operation("Validation Error".into())));
        }
        assert!(ctx.poll_reads().is_empty(), "failed requests complete only once");

        let device = opened();
        let mut ctx = WgpuContext::from_opened(WorkerKind::Main, &device);
        let handle = ctx.upload_buffer("oom", b"abcd").expect("upload");
        let request = ctx.request_read(handle).expect("read");
        device.health.mark(Fault::Error(craft_gpu_device::OUT_OF_MEMORY.into()));
        let outcomes = ctx.poll_reads();
        assert!(outcomes.iter().all(|outcome| outcome.request == request && outcome.result.is_ok()));
        let Backing::Device(backing) = &ctx.backing else { panic!("device backing") };
        assert_eq!(outcomes.len() + backing.pending.len(), 1, "out of memory does not discard or fail the read");
    }
}
