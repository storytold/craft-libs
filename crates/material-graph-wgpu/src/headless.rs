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

    /// Exercise the native policy and also the Metal fallback on hosts with another backend.
    pub(crate) fn spatial_sampling_modes(&self) -> impl Iterator<Item = (GraphAccelerator, bool)> {
        let metal = self.device.adapter_info().backend == wgpu::Backend::Metal;
        std::iter::once((self.accelerator(), metal)).chain((!metal).then(|| (self.accelerator().with_spatial_sampling_on_gpu(false), true)))
    }

    /// An accelerator that samples transforms and warps on the GPU even on Metal, to check the shaders there.
    pub(crate) fn gpu_spatial_sampling(&self) -> GraphAccelerator {
        self.accelerator().with_spatial_sampling_on_gpu(true)
    }

    /// Run the `main` entry of `source` once per workgroup for `invocations` workgroups. It reads `input` at binding 0 and writes
    /// `output_bytes` at binding 1, which come back.
    pub(crate) fn run_compute(&self, source: &str, input: &[u8], output_bytes: u64, invocations: u32) -> Vec<u8> {
        use wgpu::util::DeviceExt;
        let device = &self.device;
        let module = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("test compute"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("test compute"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test input"),
            contents: input,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let buffer =
            |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: output_bytes, usage, mapped_at_creation: false });
        let output = buffer("test output", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let readback = buffer("test readback", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test compute"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
            ],
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(invocations, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
        let submission = self.queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).unwrap();

        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        readback.unmap();
        bytes
    }
}

#[test]
fn an_eligible_adapter_that_cannot_create_its_device_fails_instead_of_skipping() {
    let Some(_) = Headless::new().unwrap() else { return };
    let impossible = wgpu::Limits { max_texture_dimension_2d: u32::MAX, ..wgpu::Limits::default() };

    let result = Headless::with_limits(impossible);

    assert!(result.is_err(), "a device the adapter cannot create must fail the parity tests");
}
