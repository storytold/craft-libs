//! Windows: the backends to initialize and the DirectX 12 shader compiler, applied before wgpu can initialize any
//! driver.

use egui_wgpu::wgpu::{Backends, DeviceType, Dx12Compiler};

/// What the start-up probe learned about one DirectX 12 adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dx12Adapter {
    /// `Cpu` is WARP (Microsoft's software rasteriser): it works, but every frame is drawn on the
    /// processor, which makes the app lag on older machines (#316).
    pub device_type: DeviceType,
    /// Whether the adapter offers the limits eframe will ask for when it creates the device.
    pub meets_limits: bool,
}

/// The backends to initialize on Windows.
///
/// An explicit `backend_override` (normally `Backends::from_env()`, i.e. `WGPU_BACKEND`) always
/// wins. Otherwise DirectX 12 only, as long as `dx12_graphics` is set and `dx12_adapters` (what a
/// DirectX 12-only probe found) contains a usable hardware GPU. Without one, OpenGL is the way to
/// the GPU: graphics chips whose driver has no DirectX 12 support (Intel 4th generation on Windows
/// 11, #316) still have an OpenGL driver, and GL can't reach the AMD driver crash described in
/// `configure`, because an AMD GPU with a driver shows up as a DirectX 12 adapter and
/// keeps us on DirectX 12. When DirectX 12 offers only WARP, it stays enabled next to GL as a last
/// resort for machines without an OpenGL driver either (virtual machines): wgpu prefers any GPU
/// over a software adapter, so it's only picked when GL has nothing better.
///
/// `dx12_graphics` is false on 32-bit Windows. wgpu creates the graphics pipeline through
/// `ID3D12Device2::CreatePipelineState`, and that stream is aligned to 8 bytes. The 32-bit runtime
/// expects pointer alignment (4), so creation returns `E_INVALIDARG` ("The parameter is incorrect")
/// and the process exits before the first frame. Compute pipelines use the older API and succeed,
/// which is why startup gets as far as the egui pipeline and then quits. OpenGL does not use that
/// stream.
pub(crate) fn choose_backends(backend_override: Option<Backends>, dx12_adapters: &[Dx12Adapter], dx12_graphics: bool) -> Backends {
    if let Some(backends) = backend_override {
        return backends;
    }
    if !dx12_graphics {
        return Backends::GL;
    }
    let hardware = |a: &&Dx12Adapter| a.device_type != DeviceType::Cpu;
    if dx12_adapters.iter().filter(hardware).any(|a| a.meets_limits) {
        Backends::DX12
    } else if !dx12_adapters.is_empty() && !dx12_adapters.iter().any(|a| hardware(&a)) {
        Backends::DX12 | Backends::GL
    } else {
        // No DirectX 12 adapter at all, or only GPUs that can't run the app: GL alone, so a
        // DirectX 12 GPU that would fail device creation isn't picked over the OpenGL one.
        Backends::GL
    }
}

/// The DirectX 12 shader compiler: FXC (`d3dcompiler_47.dll`, part of Windows) unless `env`
/// (normally `WGPU_DX12_COMPILER`, wgpu's own variable: `fxc`, `dxc`, `auto`…) names another.
///
/// #603: wgpu's default (`Auto`) uses DXC, loading whichever `dxcompiler.dll` the DLL search path
/// finds first. The app ships none, so that copy belongs to another program. Without a matching
/// `dxil.dll` beside it, DXC warns that the DXIL is unsigned; wgpu takes the warning for a compile
/// error, its own indirect-validation pipeline fails, the device is lost and eframe exits before
/// the first frame. FXC is always there and compiles everything egui's shaders need.
pub(crate) fn dx12_compiler(env: Option<&str>) -> Dx12Compiler {
    let Some(value) = env else { return Dx12Compiler::Fxc };
    value.parse().unwrap_or_else(|e| {
        log::warn!("graphics: WGPU_DX12_COMPILER={value:?} ignored ({e}), using FXC");
        Dx12Compiler::Fxc
    })
}

/// Sets the DirectX 12 shader compiler ([`dx12_compiler`]) and the backends ([`choose_backends`]) on `setup`.
/// Without a `WGPU_BACKEND` override this first probes DirectX 12 with a short-lived instance of its own, which
/// is dropped before eframe creates the real one.
#[cfg(windows)]
pub(crate) fn configure(setup: &mut egui_wgpu::WgpuSetupCreateNew) {
    // First, so the probe's instance doesn't load a stray `dxcompiler.dll` either.
    let compiler = dx12_compiler(std::env::var("WGPU_DX12_COMPILER").ok().as_deref());
    log::info!("graphics: DirectX 12 shader compiler {compiler:?}");
    setup.instance_descriptor.backend_options.dx12.shader_compiler = compiler;
    let backends = match Backends::from_env() {
        Some(backends) => {
            log::info!("graphics: WGPU_BACKEND override, backends {backends:?}");
            backends
        }
        None => {
            // 32-bit: skip the DirectX 12 probe. A capable GPU would be selected, then
            // CreatePipelineState fails and the app exits with no window.
            let dx12_graphics = cfg!(not(target_arch = "x86"));
            let adapters = if dx12_graphics { probe_dx12(setup) } else { Vec::new() };
            let backends = choose_backends(None, &adapters, dx12_graphics);
            if dx12_graphics {
                log::info!("graphics: {} DirectX 12 adapter(s) {adapters:?}, backends {backends:?}", adapters.len());
            } else {
                log::info!("graphics: 32-bit Windows uses OpenGL; DirectX 12 graphics pipeline creation fails");
            }
            backends
        }
    };
    // eframe's default includes GL. Creating that backend can crash inside AMD's
    // atio6axx.dll before adapter selection or any Rust error handling runs, so the
    // window never appears. Choosing an adapter afterwards is too late: only leaving GL
    // out of the instance avoids it. This matches the verified WGPU_BACKEND=dx12
    // workaround without mutating process environment variables.
    setup.instance_descriptor.backends = backends;
}

/// Lists the DirectX 12 adapters, checking each against the limits `setup`'s device descriptor
/// will require. Empty when this build has no DirectX 12 backend.
#[cfg(windows)]
fn probe_dx12(setup: &egui_wgpu::WgpuSetupCreateNew) -> Vec<Dx12Adapter> {
    use egui_wgpu::wgpu::{Instance, InstanceDescriptor};
    // `Instance::new` panics when the build has no backend for the platform.
    if !Instance::enabled_backend_features().contains(Backends::DX12) {
        return Vec::new();
    }
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::DX12,
        backend_options: setup.instance_descriptor.backend_options.clone(),
        ..InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(instance.enumerate_adapters(Backends::DX12))
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            let required = (setup.device_descriptor)(adapter).required_limits;
            let meets_limits = required.check_limits(&adapter.limits());
            log::info!("graphics: DirectX 12 adapter {:?} ({:?}, driver {:?}), limits ok: {meets_limits}", info.name, info.device_type, info.driver);
            Dx12Adapter { device_type: info.device_type, meets_limits }
        })
        .collect()
}
