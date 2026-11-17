//! The conversion as render passes: one pass draws the luma plane, one draws the chroma plane or
//! planes, each into unsigned-integer textures that hold the exact stored words.

use wgpu::util::DeviceExt;

use crate::layout::FrameLayout;
use crate::spec::{Coefficients, Conversion, Error, Format};

/// The render pipelines of one [`Conversion`]. Make it once per device and conversion; it holds
/// no per-frame state.
pub struct Converter {
    conversion: Conversion,
    plane_layout: wgpu::BindGroupLayout,
    source_layout: wgpu::BindGroupLayout,
    luma: wgpu::RenderPipeline,
    chroma: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

/// A source texture view bound for a [`Converter`]. Keep one per texture that is converted more
/// than once.
pub struct Source {
    bind: wgpu::BindGroup,
}

/// The plane textures of one output size, and their read-back layout.
pub struct Planes {
    layout: FrameLayout,
    textures: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    luma_bind: wgpu::BindGroup,
    chroma_bind: wgpu::BindGroup,
}

fn plane_formats(format: Format) -> &'static [wgpu::TextureFormat] {
    use wgpu::TextureFormat as T;
    match format {
        Format::Nv12 => &[T::R8Uint, T::Rg8Uint],
        Format::P010 => &[T::R16Uint, T::Rg16Uint],
        Format::I420 => &[T::R8Uint, T::R8Uint, T::R8Uint],
    }
}

fn attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
    })
}

fn vec3(w: [f32; 3]) -> String {
    format!("vec3<f32>({:?}, {:?}, {:?})", w[0], w[1], w[2])
}

/// The WGSL of `conversion`, with its weights and scale written in as constants.
pub(crate) fn shader(conversion: Conversion) -> String {
    let k = Coefficients::of(conversion);
    let chroma = match conversion.format {
        Format::Nv12 | Format::P010 => {
            "@fragment fn fs_chroma(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {
    let c = area(pos.xy);
    return vec4<u32>(code(dot(c, CB) * CS + CO), code(dot(c, CR) * CS + CO), 0u, 0u);
}"
        }
        Format::I420 => {
            "struct Chroma { @location(0) cb: vec4<u32>, @location(1) cr: vec4<u32> }
@fragment fn fs_chroma(@builtin(position) pos: vec4<f32>) -> Chroma {
    let c = area(pos.xy);
    var o: Chroma;
    o.cb = vec4<u32>(code(dot(c, CB) * CS + CO), 0u, 0u, 0u);
    o.cr = vec4<u32>(code(dot(c, CR) * CS + CO), 0u, 0u, 0u);
    return o;
}"
        }
    };
    format!(
        "const KY = {ky};
const CB = {cb};
const CR = {cr};
const YS = {ys:?};
const YO = {yo:?};
const CS = {cs:?};
const CO = {co:?};
const MAX = {max:?};
const SHIFT = {shift}u;

struct Plane {{ size: vec2<f32>, pad: vec2<f32> }}
@group(0) @binding(0) var<uniform> plane: Plane;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {{
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}}

fn taps(r: f32) -> i32 {{
    return clamp(i32(ceil(r * 0.5 - 0.001)), 1, 4);
}}

// The average color under the texel centered at `pos`: up to 4 x 4 bilinear taps over its
// footprint in the source.
fn area(pos: vec2<f32>) -> vec3<f32> {{
    let src = vec2<f32>(textureDimensions(tex));
    let r = src / plane.size;
    let n = vec2<i32>(taps(r.x), taps(r.y));
    let nf = vec2<f32>(n);
    let step = r / nf;
    let center = pos / plane.size * src;
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < n.y; j++) {{
        for (var i = 0; i < n.x; i++) {{
            let p = center + (vec2<f32>(f32(i), f32(j)) + 0.5 - nf * 0.5) * step;
            sum += textureSampleLevel(tex, samp, p / src, 0.0).rgb;
        }}
    }}
    return sum / (nf.x * nf.y);
}}

fn code(v: f32) -> u32 {{
    return u32(clamp(round(v), 0.0, MAX)) << SHIFT;
}}

@fragment fn fs_luma(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {{
    return vec4<u32>(code(dot(area(pos.xy), KY) * YS + YO), 0u, 0u, 0u);
}}

{chroma}
",
        ky = vec3(k.y),
        cb = vec3(k.cb),
        cr = vec3(k.cr),
        ys = k.y_scale,
        yo = k.y_offset,
        cs = k.c_scale,
        co = k.c_offset,
        max = k.max,
        shift = conversion.format.shift(),
    )
}

impl Converter {
    /// The pipelines of `conversion` on `device`.
    pub fn new(device: &wgpu::Device, conversion: Conversion) -> Converter {
        let plane_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv plane"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv source"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv"),
            bind_group_layouts: &[Some(&plane_layout), Some(&source_layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(conversion.format.name()),
            source: wgpu::ShaderSource::Wgsl(shader(conversion).into()),
        });
        let target = |format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
        let formats = plane_formats(conversion.format);
        let pipeline = |label: &str, fs: &str, targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fs), targets, compilation_options: Default::default() }),
                multiview_mask: None,
                cache: None,
            })
        };
        let luma = pipeline("yuv luma", "fs_luma", &[target(formats[0])]);
        let chroma_targets: Vec<_> = formats[1..].iter().map(|f| target(*f)).collect();
        let chroma = pipeline("yuv chroma", "fs_chroma", &chroma_targets);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yuv linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Converter { conversion, plane_layout, source_layout, luma, chroma, sampler }
    }

    pub fn conversion(&self) -> Conversion {
        self.conversion
    }

    /// Bind `view` as a source. The view must be a 2D view of a filterable float format, such as
    /// `Rgba8Unorm` or `Bgra8Unorm`; an sRGB view gives linear values, which the matrix does not
    /// expect.
    pub fn bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> Source {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv source"),
            layout: &self.source_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        Source { bind }
    }

    /// Plane textures for frames of `width × height`. Both sides must be even, not zero, and no
    /// larger than the device's largest 2D texture.
    pub fn planes(&self, device: &wgpu::Device, width: u32, height: u32) -> Result<Planes, Error> {
        let limit = device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(Error::TooLarge { width, height, limit });
        }
        let layout = FrameLayout::new(self.conversion.format, width, height)?;
        let max = device.limits().max_buffer_size;
        if layout.buffer_size > max {
            return Err(Error::BufferLimit { needed: layout.buffer_size, limit: max });
        }
        let formats = plane_formats(self.conversion.format);
        let textures: Vec<wgpu::Texture> = layout
            .planes
            .iter()
            .zip(formats)
            .map(|(p, format)| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("yuv plane"),
                    size: wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: *format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            })
            .collect();
        let views = textures.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect();
        let uniform = |w: u32, h: u32| {
            let mut bytes = [0u8; 16];
            bytes[..4].copy_from_slice(&(w as f32).to_le_bytes());
            bytes[4..8].copy_from_slice(&(h as f32).to_le_bytes());
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("yuv plane size"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yuv plane"),
                layout: &self.plane_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
            })
        };
        let luma_bind = uniform(width, height);
        let chroma_bind = uniform(width / 2, height / 2);
        Ok(Planes { layout, textures, views, luma_bind, chroma_bind })
    }

    /// Record the passes that convert `source` into `planes`.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, source: &Source, planes: &Planes) -> Result<(), Error> {
        if planes.layout.format != self.conversion.format {
            return Err(Error::Mismatch { converter: self.conversion.format, planes: planes.layout.format });
        }
        // No allocation per frame: one luma view, then one or two chroma views.
        let chroma_count = planes.views.len().saturating_sub(1).min(2);
        let luma = [planes.views.first().and_then(attachment)];
        let chroma = [planes.views.get(1).and_then(attachment), planes.views.get(2).and_then(attachment)];
        let passes: [(&wgpu::RenderPipeline, &wgpu::BindGroup, &[Option<wgpu::RenderPassColorAttachment<'_>>]); 2] =
            [(&self.luma, &planes.luma_bind, &luma), (&self.chroma, &planes.chroma_bind, &chroma[..chroma_count])];
        for (pipeline, plane, attachments) in passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yuv"),
                color_attachments: attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, plane, &[]);
            pass.set_bind_group(1, &source.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }
}

impl Planes {
    pub fn layout(&self) -> &FrameLayout {
        &self.layout
    }

    /// The plane textures, luma first.
    pub fn textures(&self) -> &[wgpu::Texture] {
        &self.textures
    }

    /// Record copies of every plane into `buffer` from `offset`, in the [`FrameLayout`]'s
    /// places. `offset` must be a multiple of `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`, and the
    /// buffer must hold `layout().buffer_size` bytes from it.
    pub fn copy_to_buffer(&self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, offset: u64) -> Result<(), Error> {
        if !offset.is_multiple_of(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)) {
            return Err(Error::Offset(offset));
        }
        let needed = offset.saturating_add(self.layout.buffer_size);
        if buffer.size() < needed {
            return Err(Error::BufferSize { needed, actual: buffer.size() });
        }
        for (texture, p) in self.textures.iter().zip(&self.layout.planes) {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: offset + p.offset,
                        bytes_per_row: Some(p.bytes_per_row),
                        rows_per_image: Some(p.height),
                    },
                },
                wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
            );
        }
        Ok(())
    }
}
