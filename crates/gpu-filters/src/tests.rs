//! The CPU reference's behaviour, and the helpers the other test modules share.

use super::*;

/// A small deterministic generator, so every run tests the same pictures.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn byte(&mut self) -> u8 {
        (self.next() >> 32) as u8
    }
}

/// A premultiplied picture of `w × h` noise: one pixel in eight transparent, one in eight opaque,
/// the rest of any alpha, each colour channel at most its alpha.
pub(crate) fn noise(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        let a = match rng.byte() % 8 {
            0 => 0,
            1 => 255,
            _ => rng.byte(),
        };
        for _ in 0..3 {
            let c = if a == 0 { 0 } else { (u32::from(rng.byte()) * u32::from(a) / 255) as u8 };
            px.push(c);
        }
        px.push(a);
    }
    px
}

/// Noise with no rule between colour and alpha, so colour above alpha appears.
pub(crate) fn hostile(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    (0..w * h * 4).map(|_| rng.byte()).collect()
}

/// Lookup tables of noise.
pub(crate) fn random_lut(seed: u64) -> Lut {
    let mut rng = Rng::new(seed);
    let mut lut = Lut::identity();
    for table in &mut lut.tables {
        for v in table.iter_mut() {
            *v = rng.byte();
        }
    }
    lut
}

/// The largest difference between two images' channels.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// Whether every pixel's colour channels are at most its alpha.
fn premultiplied(px: &[u8]) -> bool {
    px.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
}

/// A `w × h` picture of one colour.
fn flat(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    rgba.iter().copied().cycle().take((w * h * 4) as usize).collect()
}

#[test]
fn the_kernel_sums_to_one_and_falls_off_from_the_centre() {
    for sigma in [0.0f32, 1e-30, 0.3, 1.0, 2.5, 17.0, MAX_SIGMA] {
        let w = kernel(sigma).expect("kernel");
        let total: f64 = f64::from(w[0]) + 2.0 * w[1..].iter().map(|&v| f64::from(v)).sum::<f64>();
        assert!((total - 1.0).abs() < 1e-5, "sigma {sigma}: the weights sum to {total}");
        assert!(w.windows(2).all(|p| p[0] >= p[1]), "sigma {sigma}: a weight rises away from the centre");
    }
    assert_eq!(kernel(0.0).expect("kernel"), vec![1.0]);
}

#[test]
fn a_blur_of_sigma_zero_changes_no_valid_pixel() {
    let px = noise(13, 7, 1);
    assert_eq!(cpu::gaussian_blur(&px, 13, 7, 0.0).expect("blur"), px);
}

#[test]
fn a_blur_keeps_the_picture_premultiplied_even_from_hostile_input() {
    for (sigma, seed) in [(0.7f32, 2u64), (3.0, 3), (12.0, 4)] {
        let out = cpu::gaussian_blur(&hostile(31, 17, seed), 31, 17, sigma).expect("blur");
        assert!(premultiplied(&out), "sigma {sigma}: a colour channel is above its alpha");
    }
}

#[test]
fn a_blur_fades_the_edges_of_a_flat_picture_and_keeps_its_middle() {
    let (w, h) = (40, 40);
    let out = cpu::gaussian_blur(&flat(w, h, [200, 100, 50, 255]), w, h, 2.0).expect("blur");
    let at = |x: u32, y: u32| &out[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
    assert_eq!(at(20, 20), &[200, 100, 50, 255], "the middle, far from any edge, is unchanged");
    assert!(at(0, 0)[3] < at(0, 20)[3] && at(0, 20)[3] < 255, "the corner fades more than the edge, which fades");
}

#[test]
fn a_blur_spreads_a_square_symmetrically_and_keeps_its_mass() {
    let (w, h) = (33, 33);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 14..19 {
        for x in 14..19 {
            let i = ((y * w + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[255; 4]);
        }
    }
    let out = cpu::gaussian_blur(&px, w, h, 2.0).expect("blur");
    let alpha = |x: u32, y: u32| out[((y * w + x) * 4 + 3) as usize];
    assert!(alpha(16, 16) < 255 && alpha(16, 16) > alpha(19, 16));
    for d in 1..12 {
        assert_eq!(alpha(16 - d, 16), alpha(16 + d, 16));
        assert_eq!(alpha(16, 16 - d), alpha(16, 16 + d));
    }
    let mass: u32 = out.as_chunks::<4>().0.iter().map(|p| u32::from(p[3])).sum();
    assert!(mass.abs_diff(25 * 255) <= 25 * 255 / 50, "the square's alpha {mass} is not within 2% of the {} it started with", 25 * 255);
}

#[test]
fn identity_tables_give_back_every_valid_premultiplied_pixel() {
    let mut px = Vec::new();
    for a in 0..=255u8 {
        for c in 0..=a {
            px.extend_from_slice(&[c, a - c, c / 2, a]);
        }
    }
    // 32896 pixels, padded with transparent ones to 129 rows of 256.
    px.resize(256 * 129 * 4, 0);
    assert_eq!(cpu::lut_apply(&px, 256, 129, &Lut::identity()).expect("lut"), px);
}

#[test]
fn tables_work_on_straight_colour_and_premultiply_again() {
    let mut invert = Lut::identity();
    for ch in 0..3 {
        for (i, v) in invert.tables[ch].iter_mut().enumerate() {
            *v = 255 - i as u8;
        }
    }
    // Half-transparent white is straight white; inverted it is straight black, so no colour.
    assert_eq!(cpu::lut_apply(&[128, 128, 128, 128], 1, 1, &invert).expect("lut"), vec![0, 0, 0, 128]);
    // Half-transparent black inverts to half-transparent white.
    assert_eq!(cpu::lut_apply(&[0, 0, 0, 128], 1, 1, &invert).expect("lut"), vec![128, 128, 128, 128]);
    // An alpha table of zeros clears every pixel.
    let mut clear = Lut::identity();
    clear.tables[3] = [0; 256];
    assert!(cpu::lut_apply(&noise(9, 9, 5), 9, 9, &clear).expect("lut").iter().all(|&v| v == 0));
}

#[test]
fn tables_keep_hostile_input_premultiplied() {
    let out = cpu::lut_apply(&hostile(16, 16, 6), 16, 16, &random_lut(7)).expect("lut");
    assert!(premultiplied(&out));
}

#[test]
fn compositing_follows_source_over() {
    let backdrop = noise(8, 8, 8);
    let layer = noise(8, 8, 9);
    assert_eq!(cpu::composite(&backdrop, &layer, 8, 8, 0.0).expect("over"), backdrop, "no opacity leaves the backdrop");
    let opaque = flat(8, 8, [10, 20, 30, 255]);
    assert_eq!(cpu::composite(&backdrop, &opaque, 8, 8, 1.0).expect("over"), opaque, "an opaque layer covers the backdrop");
    let clear = flat(8, 8, [0; 4]);
    assert_eq!(cpu::composite(&backdrop, &clear, 8, 8, 1.0).expect("over"), backdrop, "a transparent layer leaves the backdrop");
    let out = cpu::composite(&backdrop, &layer, 8, 8, 0.6).expect("over");
    assert!(premultiplied(&out));
    for ((o, b), l) in out.as_chunks::<4>().0.iter().zip(backdrop.as_chunks::<4>().0.iter()).zip(layer.as_chunks::<4>().0.iter()) {
        assert!(o[3] >= b[3] && o[3] >= (f32::from(l[3]) * 0.6) as u8, "over never lowers alpha");
    }
    let hostile_out = cpu::composite(&hostile(8, 8, 10), &hostile(8, 8, 11), 8, 8, 0.5).expect("over");
    assert!(premultiplied(&hostile_out));
}

#[test]
fn the_cpu_filters_refuse_bad_sizes_and_parameters() {
    let px = noise(4, 4, 12);
    assert!(matches!(cpu::gaussian_blur(&px, 4, 4, f32::NAN), Err(Error::Sigma(_))));
    assert!(matches!(cpu::gaussian_blur(&px, 4, 4, -1.0), Err(Error::Sigma(_))));
    assert!(matches!(cpu::gaussian_blur(&px, 4, 4, MAX_SIGMA * 1.01), Err(Error::Sigma(_))));
    assert!(matches!(cpu::gaussian_blur(&px, 4, 4, f32::INFINITY), Err(Error::Sigma(_))));
    assert!(matches!(cpu::gaussian_blur(&[], 0, 4, 1.0), Err(Error::Empty { .. })));
    assert!(matches!(cpu::gaussian_blur(&px, 4, 5, 1.0), Err(Error::Source { .. })));
    assert!(matches!(cpu::gaussian_blur(&px, MAX_SIDE + 1, 1, 1.0), Err(Error::TooLarge { .. })));
    assert!(matches!(cpu::gaussian_blur(&px, MAX_SIDE, MAX_SIDE, 1.0), Err(Error::TooLarge { .. })), "over MAX_PIXELS");
    assert!(matches!(cpu::composite(&px, &px, 4, 4, f32::NAN), Err(Error::Opacity(_))));
    assert!(matches!(cpu::composite(&px, &px, 4, 4, 1.5), Err(Error::Opacity(_))));
    assert!(matches!(cpu::composite(&px, &px[..60], 4, 4, 0.5), Err(Error::Source { .. })));
    assert!(matches!(cpu::apply(&Filter::Composite { opacity: 1.0 }, 4, 4, &[&px]), Err(Error::Inputs { .. })));
}

// The device half: a machine without a graphics adapter skips it, the no-adapter path RFC-0022
// defines, and says so.

/// A device on the default adapter, or `None` with a printed note when there is none.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    device_with(|l| l)
}

/// A device with the adapter's limits as `limits` changes them.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn device_with(limits: impl FnOnce(wgpu::Limits) -> wgpu::Limits) -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no graphics adapter: the device half of this test is skipped");
        return None;
    };
    let info = adapter.get_info();
    let name = format!("{} ({:?})", info.name, info.backend);
    let desc = wgpu::DeviceDescriptor { required_limits: limits(adapter.limits()), ..Default::default() };
    match pollster::block_on(adapter.request_device(&desc)) {
        Ok((device, queue)) => {
            eprintln!("device half running on {name}");
            Some((device, queue, name))
        }
        Err(e) => {
            eprintln!("the graphics adapter did not open ({e}): the device half of this test is skipped");
            None
        }
    }
}
