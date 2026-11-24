use crate::GraphAccelerator;

pub(crate) struct Headless {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

impl Headless {
    /// `Ok(None)` skips the GPU parity tests: there is no adapter, or only a software or virtual one.
    pub(crate) fn new() -> Result<Option<Self>, String> {
        Self::with_limits(wgpu::Limits::default())
    }

    /// A device with `limits` on the test adapter. An eligible adapter that cannot create it is an error, not a skip.
    pub(crate) fn with_limits(limits: wgpu::Limits) -> Result<Option<Self>, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipping the GPU parity tests: no GPU adapter");
            return Ok(None);
        };
        let info = adapter.get_info();
        eprintln!("Material graph test adapter: {info:?}");
        // The tests compare the GPU with the CPU reference. Software rasterizers (llvmpipe, WARP) and paravirtual Metal sample and round
        // differently, so these tests only run on a hardware adapter unless MATERIAL_GRAPH_TEST_ANY_ADAPTER is set.
        let software = matches!(info.device_type, wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu) || info.name.contains("Paravirtual");
        if software && std::env::var_os("MATERIAL_GRAPH_TEST_ANY_ADAPTER").is_none() {
            eprintln!("skipping the GPU parity tests on the software or virtual adapter {}", info.name);
            return Ok(None);
        }

        let descriptor = wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))
            .map_err(|error| format!("no GPU device on the test adapter {}: {error}", info.name))?;
        Ok(Some(Self { device, queue }))
    }

    pub(crate) fn accelerator(&self) -> GraphAccelerator {
        GraphAccelerator::new(&self.device, &self.queue, || 4 << 30)
    }
}

#[test]
fn an_eligible_adapter_that_cannot_create_its_device_fails_instead_of_skipping() {
    let Some(_) = Headless::new().unwrap() else { return };
    let impossible = wgpu::Limits { max_texture_dimension_2d: u32::MAX, ..wgpu::Limits::default() };

    let result = Headless::with_limits(impossible);

    assert!(result.is_err(), "a device the adapter cannot create must fail the parity tests");
}
