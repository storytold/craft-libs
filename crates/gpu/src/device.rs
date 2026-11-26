//! The GPU start-up choices, one module per platform.
//!
//! The Windows and Linux choices are compiled under tests on every platform too, so their trials run on every CI
//! host; the parts that talk to the system stay gated to their platform. The macOS module is only wiring.

#[cfg(any(target_os = "linux", test))]
pub(crate) mod drm;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(any(windows, test))]
pub(crate) mod windows;

/// Sets the GPU start-up choices of the running platform on `setup`: the backends, the power preference, the DX12
/// shader compiler and, on Linux, the DRM device to render on. Call it before eframe creates the wgpu instance.
pub fn configure(setup: &mut egui_wgpu::WgpuSetupCreateNew) {
    #[cfg(windows)]
    windows::configure(setup);
    #[cfg(target_os = "macos")]
    macos::configure(setup);
    // egui-wgpu draws on the discrete GPU by default; draw on the GPU that drives the screen instead.
    #[cfg(target_os = "linux")]
    {
        setup.native_adapter_selector = Some(std::sync::Arc::new(linux::pick_adapter));
    }
    // Other platforms keep egui-wgpu's own choices.
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let _ = setup;
}
