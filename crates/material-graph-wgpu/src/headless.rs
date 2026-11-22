use crate::GraphAccelerator;

pub(crate) struct Headless {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

impl Headless {
    pub(crate) fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|error| format!("no GPU adapter: {error}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|error| format!("no GPU device: {error}"))?;
        let info = adapter.get_info();
        eprintln!("Material graph test adapter: {info:?}");
        // The tests compare the GPU with the CPU reference. Software rasterizers (llvmpipe, WARP) and paravirtual Metal sample and round
        // differently, so these tests only run on a hardware adapter unless MATERIAL_GRAPH_TEST_ANY_ADAPTER is set.
        let software = matches!(info.device_type, wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu) || info.name.contains("Paravirtual");
        if software && std::env::var_os("MATERIAL_GRAPH_TEST_ANY_ADAPTER").is_none() {
            return Err(format!("skipping the GPU parity tests on the software or virtual adapter {}", info.name));
        }
        Ok(Self { device, queue })
    }

    pub(crate) fn accelerator(&self) -> GraphAccelerator {
        GraphAccelerator::new(&self.device, &self.queue, || 4 << 30)
    }
}
