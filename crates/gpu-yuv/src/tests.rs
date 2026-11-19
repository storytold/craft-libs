use super::*;

const ALL: [Format; 3] = [Format::Nv12, Format::P010, Format::I420];
const MATRICES: [Matrix; 3] = [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020];
const RANGES: [Range; 2] = [Range::Limited, Range::Full];

fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect()
}

/// The first luma sample and the first Cb and Cr samples of a converted frame, as values.
fn first(frame: &[u8], format: Format, w: u32, h: u32) -> [u16; 3] {
    let s = samples(frame, format);
    let luma = (w * h) as usize;
    let quarter = luma / 4;
    match format {
        Format::Nv12 | Format::P010 => [s[0], s[luma], s[luma + 1]],
        Format::I420 => [s[0], s[luma], s[luma + quarter]],
    }
}

/// Every sample of a frame as its value (P010 words shifted down by 6).
fn samples(frame: &[u8], format: Format) -> Vec<u16> {
    match format {
        Format::P010 => frame.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b) >> 6).collect(),
        Format::Nv12 | Format::I420 => frame.iter().map(|&b| u16::from(b)).collect(),
    }
}

fn cpu(rgb: [u8; 3], c: Conversion) -> [u16; 3] {
    let frame = cpu::convert(&solid(2, 2, rgb), 2, 2, c, 2, 2).unwrap();
    first(&frame, c.format, 2, 2)
}

#[test]
fn bt709_limited_colors_have_their_standard_codes() {
    let c = Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited);
    assert_eq!(cpu([255, 255, 255], c), [235, 128, 128]);
    assert_eq!(cpu([0, 0, 0], c), [16, 128, 128]);
    assert_eq!(cpu([255, 0, 0], c), [63, 102, 240]);
    assert_eq!(cpu([0, 0, 255], c), [32, 240, 118]);
}

#[test]
fn each_matrix_weights_red_by_its_kr() {
    let red = |m| cpu([255, 0, 0], Conversion::new(Format::Nv12, m, Range::Limited));
    assert_eq!(red(Matrix::Bt601), [81, 90, 240]);
    assert_eq!(red(Matrix::Bt2020), [74, 97, 240]);
}

#[test]
fn full_range_spans_every_code() {
    let c = Conversion::new(Format::I420, Matrix::Bt709, Range::Full);
    assert_eq!(cpu([255, 255, 255], c), [255, 128, 128]);
    assert_eq!(cpu([0, 0, 0], c), [0, 128, 128]);
    assert_eq!(cpu([255, 0, 0], c), [54, 99, 255]);
}

#[test]
fn p010_holds_ten_bit_codes_in_the_high_bits() {
    let limited = Conversion::new(Format::P010, Matrix::Bt2020, Range::Limited);
    assert_eq!(cpu([255, 255, 255], limited), [940, 512, 512]);
    assert_eq!(cpu([0, 0, 0], limited), [64, 512, 512]);
    let full = Conversion::new(Format::P010, Matrix::Bt2020, Range::Full);
    assert_eq!(cpu([255, 255, 255], full), [1023, 512, 512]);
    let frame = cpu::convert(&solid(2, 2, [255, 255, 255]), 2, 2, limited, 2, 2).unwrap();
    assert_eq!(&frame[..2], &(940u16 << 6).to_le_bytes());
    assert_eq!(frame.len(), 12);
}

#[test]
fn planes_follow_each_format_s_order() {
    let red = solid(4, 2, [255, 0, 0]);
    let nv12 = cpu::convert(&red, 4, 2, Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited), 4, 2).unwrap();
    assert_eq!(nv12, [63, 63, 63, 63, 63, 63, 63, 63, 102, 240, 102, 240]);
    let i420 = cpu::convert(&red, 4, 2, Conversion::new(Format::I420, Matrix::Bt709, Range::Limited), 4, 2).unwrap();
    assert_eq!(i420, [63, 63, 63, 63, 63, 63, 63, 63, 102, 102, 240, 240]);
}

#[test]
fn chroma_at_the_same_size_averages_the_four_pixels_it_covers() {
    // Red over blue: chroma is the code of their average, magenta at half strength.
    let mut px = solid(2, 1, [255, 0, 0]);
    px.extend(solid(2, 1, [0, 0, 255]));
    let c = Conversion::new(Format::Nv12, Matrix::Bt709, Range::Full);
    let frame = cpu::convert(&px, 2, 2, c, 2, 2).unwrap();
    let half = cpu([128, 0, 128], c);
    assert!(frame[4].abs_diff(half[1] as u8) <= 1 && frame[5].abs_diff(half[2] as u8) <= 1, "{frame:?} {half:?}");
}

#[test]
fn a_reduction_averages_the_area_under_each_texel() {
    // One white column in four: a 4:1 reduction gives a quarter of white, where one bilinear tap
    // at the texel's center would land between two black columns.
    let px: Vec<u8> = (0..16).flat_map(|i| if i % 4 == 0 { [255, 255, 255, 255] } else { [0, 0, 0, 255] }).collect();
    let c = Conversion::new(Format::Nv12, Matrix::Bt709, Range::Full);
    let frame = cpu::convert(&px, 8, 2, c, 2, 2).unwrap();
    assert_eq!(&frame[..4], &[64, 64, 64, 64]);
}

#[test]
fn the_tap_count_grows_with_the_reduction_up_to_four() {
    assert_eq!([0.5, 1.0, 2.0, 2.01, 4.0, 4.5, 8.0, 100.0].map(cpu::taps), [1, 1, 1, 2, 2, 3, 4, 4]);
}

#[test]
fn bad_sizes_are_errors() {
    let c = Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited);
    let px = solid(2, 2, [0, 0, 0]);
    assert_eq!(cpu::convert(&px, 2, 2, c, 3, 2).unwrap_err(), Error::OutputSize { width: 3, height: 2 });
    assert_eq!(cpu::convert(&px, 2, 2, c, 0, 0).unwrap_err(), Error::OutputSize { width: 0, height: 0 });
    assert_eq!(cpu::convert(&px, 2, 3, c, 2, 2).unwrap_err(), Error::Source { width: 2, height: 3, bytes: 16 });
    assert_eq!(cpu::convert(&[], 0, 0, c, 2, 2).unwrap_err(), Error::Source { width: 0, height: 0, bytes: 0 });
    assert_eq!(FrameLayout::new(Format::P010, 4, 5).unwrap_err(), Error::OutputSize { width: 4, height: 5 });
    let big = cpu::MAX_SIDE + 2;
    assert_eq!(cpu::convert(&px, 2, 2, c, big, 2).unwrap_err(), Error::TooLarge { width: big, height: 2, limit: cpu::MAX_SIDE });
    // Byte sizes past the integer types are an error, not an overflow.
    let huge = u32::MAX - 1;
    assert_eq!(FrameLayout::new(Format::P010, huge, 2).unwrap_err(), Error::Overflow { width: huge, height: 2 });
    assert_eq!(FrameLayout::new(Format::Nv12, huge, 2).unwrap_err(), Error::Overflow { width: huge, height: 2 });
    assert_eq!(Error::OutputSize { width: 3, height: 2 }.to_string(), "a 4:2:0 picture of 3×2 needs both sides even and not zero");
}

#[test]
fn a_frame_buffer_that_cannot_be_allocated_is_an_error() {
    // A request past `isize::MAX` bytes fails on every allocator, so the failure path runs
    // without exhausting memory.
    assert_eq!(cpu::reserve::<u8>(usize::MAX).unwrap_err(), Error::Memory { bytes: usize::MAX });
    assert_eq!(cpu::reserve::<u16>(usize::MAX / 2).unwrap_err(), Error::Memory { bytes: usize::MAX - 1 });
    let room = cpu::reserve::<u8>(4096).unwrap();
    assert!(room.is_empty() && room.capacity() >= 4096);
    assert_eq!(Error::Memory { bytes: 16 }.to_string(), "a buffer of 16 bytes could not be allocated");
}

#[test]
fn layouts_pad_rows_for_copies_and_unpack_without_the_padding() {
    let l = FrameLayout::new(Format::Nv12, 1920, 1080).unwrap();
    assert_eq!(
        l.planes.iter().map(|p| (p.width, p.height, p.offset, p.bytes_per_row)).collect::<Vec<_>>(),
        [(1920, 1080, 0, 2048), (960, 540, 2048 * 1080, 2048)]
    );
    assert_eq!((l.buffer_size, l.frame_bytes()), (2048 * 1620, 1920 * 1620));
    let p = FrameLayout::new(Format::P010, 640, 360).unwrap();
    assert_eq!((p.planes[0].bytes_per_row, p.planes[1].texel_bytes, p.frame_bytes()), (1280, 4, 640 * 360 * 3));
    let i = FrameLayout::new(Format::I420, 4, 2).unwrap();
    assert_eq!(i.planes.iter().map(|p| p.offset).collect::<Vec<_>>(), [0, 512, 768]);
    let mut padded = vec![0u8; i.buffer_size as usize];
    padded[..4].copy_from_slice(&[1, 2, 3, 4]);
    padded[256..260].copy_from_slice(&[5, 6, 7, 8]);
    padded[512..514].copy_from_slice(&[9, 10]);
    padded[768..770].copy_from_slice(&[11, 12]);
    let mut frame = vec![0u8; i.frame_bytes()];
    i.unpack(&padded, &mut frame).unwrap();
    assert_eq!(frame, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    assert_eq!(i.unpack(&padded[..100], &mut frame).unwrap_err(), Error::BufferSize { needed: 1024, actual: 100 });
    assert_eq!(i.unpack(&padded, &mut [0; 3]).unwrap_err(), Error::BufferSize { needed: 12, actual: 3 });
}

#[test]
fn unpack_refuses_a_layout_that_was_changed_after_it_was_made() {
    let l = FrameLayout::new(Format::I420, 4, 2).unwrap();
    let padded = vec![0u8; l.buffer_size as usize];
    let changes: [fn(&mut FrameLayout); 6] = [
        |l| l.planes[0].bytes_per_row = 0,
        |l| l.planes[0].width = 0,
        |l| l.planes[0].texel_bytes = 0,
        |l| l.planes[0].offset = 768,
        |l| l.planes[0].bytes_per_row = 2,
        |l| l.planes.truncate(1),
    ];
    for (n, change) in changes.iter().enumerate() {
        let mut bad = l.clone();
        change(&mut bad);
        let mut frame = vec![0u8; bad.frame_bytes()];
        assert_eq!(bad.unpack(&padded, &mut frame), Err(Error::Layout), "change {n}");
    }
    assert_eq!(Error::Layout.to_string(), "the frame layout does not match its format and size");
}

#[test]
fn an_enlarged_solid_color_keeps_its_codes() {
    for format in ALL {
        let c = Conversion::new(format, Matrix::Bt709, Range::Limited);
        let frame = cpu::convert(&solid(3, 1, [10, 200, 90]), 3, 1, c, 8, 6).unwrap();
        let expect = cpu([10, 200, 90], c);
        let s = samples(&frame, format);
        assert_eq!(s.len(), 8 * 6 * 3 / 2);
        assert!(s[..48].iter().all(|&v| v == expect[0]), "{format}");
    }
}

#[test]
fn shaders_write_no_value_wgsl_cannot_read() {
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let s = gpu::shader(Conversion::new(format, matrix, range));
                assert!(!s.contains("NaN") && !s.contains("inf"), "{s}");
                assert!(s.contains("fn fs_chroma") && s.contains("fn fs_luma"));
            }
        }
    }
}

// The GPU half: the passes against the CPU reference. A machine without a graphics adapter skips
// these comparisons and says so on stderr.

pub(crate) fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    device_with(|l| l)
}

/// A device with the adapter's limits as `limits` changes them.
pub(crate) fn device_with(limits: impl FnOnce(wgpu::Limits) -> wgpu::Limits) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no graphics adapter: the GPU half of this test is skipped");
        return None;
    };
    let desc = wgpu::DeviceDescriptor { required_limits: limits(adapter.limits()), ..Default::default() };
    match pollster::block_on(adapter.request_device(&desc)) {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("the graphics adapter did not open ({e}): the GPU half of this test is skipped");
            None
        }
    }
}

/// `src` (its pixels, width and height) converted on the GPU to `out` (width and height).
pub(crate) fn gpu_convert(device: &wgpu::Device, queue: &wgpu::Queue, src: (&[u8], u32, u32), c: Conversion, out: (u32, u32)) -> Vec<u8> {
    let ((rgba, sw, sh), (w, h)) = (src, out);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sw * 4), rows_per_image: Some(sh) },
        wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
    );
    let converter = Converter::new(device, c);
    let source = converter.bind(device, &texture.create_view(&wgpu::TextureViewDescriptor::default()));
    let planes = converter.planes(device, w, h).unwrap();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 + planes.layout().buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    converter.draw(&mut enc, &source, &planes).unwrap();
    planes.copy_to_buffer(&mut enc, &buffer, 256).unwrap();
    queue.submit([enc.finish()]);
    buffer.map_async(wgpu::MapMode::Read, 256.., |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let view = buffer.get_mapped_range(256..).unwrap();
    let mut frame = vec![0; planes.layout().frame_bytes()];
    planes.layout().unpack(&view, &mut frame).unwrap();
    frame
}

/// The tolerance in code values: 2 at 8 bits and 6 at 10 bits. GPUs filter with fewer bits of
/// weight than `f32` and may round a half the other way.
fn tolerance(format: Format) -> u16 {
    if format.bits() == 10 { 6 } else { 2 }
}

pub(crate) fn assert_close(gpu: &[u8], cpu: &[u8], c: Conversion, what: &str) {
    assert_eq!(gpu.len(), cpu.len(), "{what}");
    let (g, k) = (samples(gpu, c.format), samples(cpu, c.format));
    let worst = g.iter().zip(&k).enumerate().max_by_key(|(_, (a, b))| a.abs_diff(**b));
    if let Some((i, (a, b))) = worst {
        assert!(a.abs_diff(*b) <= tolerance(c.format), "{what} {c:?}: sample {i} is {a} on the GPU and {b} on the CPU");
    }
}

/// A picture with hard edges: each pixel's channels from a fixed pseudo-random sequence.
pub(crate) fn noise(w: u32, h: u32) -> Vec<u8> {
    let mut x = 0x2545_f491_u32;
    (0..w * h * 4)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            if i % 4 == 3 { 255 } else { (x >> 24) as u8 }
        })
        .collect()
}

/// A smooth picture: three ramps at different angles.
pub(crate) fn ramps(w: u32, h: u32) -> Vec<u8> {
    (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| {
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                [(u * 255.0) as u8, (v * 255.0) as u8, ((1.0 - (u + v) * 0.5) * 255.0) as u8, 255]
            })
        })
        .collect()
}

#[test]
fn the_gpu_matches_the_cpu_at_the_same_size() {
    let Some((device, queue)) = device() else { return };
    let px = noise(64, 36);
    for format in ALL {
        for matrix in MATRICES {
            for range in RANGES {
                let c = Conversion::new(format, matrix, range);
                let g = gpu_convert(&device, &queue, (&px, 64, 36), c, (64, 36));
                assert_close(&g, &cpu::convert(&px, 64, 36, c, 64, 36).unwrap(), c, "64×36 at the same size");
            }
        }
    }
}

#[test]
fn the_gpu_matches_the_cpu_when_scaling() {
    let Some((device, queue)) = device() else { return };
    let px = ramps(160, 90);
    for format in ALL {
        let c = Conversion::new(format, Matrix::Bt709, Range::Limited);
        for (w, h) in [(80, 46), (32, 18), (16, 10), (240, 136)] {
            let g = gpu_convert(&device, &queue, (&px, 160, 90), c, (w, h));
            assert_close(&g, &cpu::convert(&px, 160, 90, c, w, h).unwrap(), c, &format!("160×90 to {w}×{h}"));
        }
        let odd = ramps(37, 23);
        let g = gpu_convert(&device, &queue, (&odd, 37, 23), c, (20, 12));
        assert_close(&g, &cpu::convert(&odd, 37, 23, c, 20, 12).unwrap(), c, "37×23 to 20×12");
    }
}

#[test]
fn planes_larger_than_the_device_allows_are_errors() {
    // The queue stays alive: a device whose queue was dropped cannot make an encoder.
    let Some((device, _queue)) = device() else { return };
    let converter = Converter::new(&device, Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited));
    let limit = device.limits().max_texture_dimension_2d;
    let too_big = (limit + 2) & !1;
    assert_eq!(converter.planes(&device, too_big, 2).err(), Some(Error::TooLarge { width: too_big, height: 2, limit }));
    let planes = converter.planes(&device, 4, 2).unwrap();
    let other = Converter::new(&device, Conversion::new(Format::P010, Matrix::Bt709, Range::Limited));
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: 4, height: 2, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let source = other.bind(&device, &texture.create_view(&wgpu::TextureViewDescriptor::default()));
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    assert_eq!(other.draw(&mut enc, &source, &planes).unwrap_err(), Error::Mismatch { converter: Format::P010, planes: Format::Nv12 });
    let small =
        device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 256, usage: wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    assert_eq!(planes.copy_to_buffer(&mut enc, &small, 0).unwrap_err(), Error::BufferSize { needed: 768, actual: 256 });
    assert_eq!(planes.copy_to_buffer(&mut enc, &small, 4).unwrap_err(), Error::Offset(4));
}

#[test]
fn planes_whose_read_back_is_larger_than_the_largest_buffer_are_errors() {
    let Some((device, _queue)) = device_with(|l| wgpu::Limits { max_buffer_size: 1 << 20, ..l }) else { return };
    let converter = Converter::new(&device, Conversion::new(Format::Nv12, Matrix::Bt709, Range::Limited));
    // 1024 × 1024 NV12 reads back 1.5 MiB.
    assert_eq!(converter.planes(&device, 1024, 1024).err(), Some(Error::BufferLimit { needed: 1024 * 1536, limit: 1 << 20 }));
    assert!(converter.planes(&device, 512, 512).is_ok());
}
