//! The GPU start-up choices, one module per platform.

#[cfg(target_os = "linux")]
mod drm;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Sets the GPU start-up choices of the running platform on `setup`: the backends, the power preference, the DX12
/// shader compiler and, on Linux, the DRM device to render on.
pub fn configure(setup: &mut egui_wgpu::WgpuSetupCreateNew) {
    let _ = setup;
    todo!("pf.C-G1")
}
