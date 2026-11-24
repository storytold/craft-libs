//! The compute passes against the CPU reference on a real device, with the tolerances the README
//! states: the blur within 1 in every channel (the two paths add the same weights in the same
//! order, but a device may fuse a multiply and an add, and a sum that lands on a half may round
//! either way), the lookup tables and the composite exactly (both are integer arithmetic). A
//! machine without a graphics adapter skips these tests with a printed note.

use super::*;
use crate::tests::{device, device_with, hostile, max_diff, noise, random_lut};

/// The sizes the passes run at: one pixel, single rows and columns, sizes that are not a multiple
/// of the 16 × 16 workgroup, and one larger than a workgroup in both directions.
const SIZES: [(u32, u32); 6] = [(1, 1), (1, 40), (57, 1), (17, 15), (64, 48), (129, 70)];

#[test]
fn the_blur_on_the_device_is_the_cpu_blur_within_one() {
    let Some((device, queue, name)) = device() else { return };
    let gpu = GpuFilters::new(device, queue);
    let mut worst = 0;
    for (k, &(w, h)) in SIZES.iter().enumerate() {
        // Sigmas from none through sub-pixel to a kernel wider than the image.
        for sigma in [0.0f32, 0.3, 1.0, 2.5, 7.0, 40.0] {
            let filter = Filter::GaussianBlur { sigma };
            for px in [noise(w, h, k as u64), hostile(w, h, 100 + k as u64)] {
                let on_device = gpu.apply(&filter, w, h, &[&px]).expect("device blur");
                let on_cpu = cpu::apply(&filter, w, h, &[&px]).expect("cpu blur");
                let d = max_diff(&on_device, &on_cpu);
                assert!(d <= 1, "{w} × {h}, sigma {sigma}: the device differs by {d}");
                worst = worst.max(d);
            }
        }
    }
    eprintln!("blur on {name}: largest channel difference {worst}");
}

#[test]
fn the_lookup_tables_and_the_composite_on_the_device_are_the_cpu_bytes() {
    let Some((device, queue, name)) = device() else { return };
    let gpu = GpuFilters::new(device, queue);
    for (k, &(w, h)) in SIZES.iter().enumerate() {
        let seed = k as u64;
        for px in [noise(w, h, seed), hostile(w, h, 200 + seed)] {
            for lut in [Lut::identity(), random_lut(seed)] {
                let filter = Filter::Lut(Box::new(lut));
                assert_eq!(
                    gpu.apply(&filter, w, h, &[&px]).expect("device lut"),
                    cpu::apply(&filter, w, h, &[&px]).expect("cpu lut"),
                    "lut {w} × {h}"
                );
            }
        }
        let (backdrop, layer) = (noise(w, h, 300 + seed), hostile(w, h, 400 + seed));
        for opacity in [0.0f32, 0.25, 0.5, 0.999, 1.0] {
            let filter = Filter::Composite { opacity };
            let inputs: [&[u8]; 2] = [&backdrop, &layer];
            assert_eq!(
                gpu.apply(&filter, w, h, &inputs).expect("device over"),
                cpu::apply(&filter, w, h, &inputs).expect("cpu over"),
                "over {w} × {h} at {opacity}"
            );
        }
    }
    eprintln!("lookup tables and composite on {name}: identical to the CPU");
}

#[test]
fn the_device_refuses_a_filter_over_its_buffer_limit_without_running_it() {
    // A device whose storage bindings hold 64 KiB: a 128 × 128 image is exactly that, and its blur
    // needs twice as much.
    let Some((device, queue, _)) = device_with(|l| wgpu::Limits { max_storage_buffer_binding_size: 64 * 1024, ..l }) else { return };
    let gpu = GpuFilters::new(device, queue);
    assert_eq!(gpu.buffer_limit(), 64 * 1024);
    let px = noise(128, 128, 9);
    let refused = gpu.apply(&Filter::GaussianBlur { sigma: 1.0 }, 128, 128, &[&px]);
    assert_eq!(refused, Err(Error::BufferLimit { needed: 128 * 1024, limit: 64 * 1024 }));
    // The same device still runs a filter that fits, so the refusal left nothing broken.
    let lut = Filter::Lut(Box::default());
    assert_eq!(gpu.apply(&lut, 128, 128, &[&px]).expect("lut fits"), px);
}

#[test]
fn the_device_refuses_bad_parameters_as_the_cpu_does() {
    let Some((device, queue, _)) = device() else { return };
    let gpu = GpuFilters::new(device, queue);
    let px = noise(4, 4, 10);
    assert!(matches!(gpu.apply(&Filter::GaussianBlur { sigma: f32::NAN }, 4, 4, &[&px]), Err(Error::Sigma(_))));
    assert!(matches!(gpu.apply(&Filter::Composite { opacity: 2.0 }, 4, 4, &[&px, &px]), Err(Error::Opacity(_))));
    assert!(matches!(gpu.apply(&Filter::GaussianBlur { sigma: 1.0 }, 4, 5, &[&px]), Err(Error::Source { .. })));
    assert!(matches!(gpu.apply(&Filter::GaussianBlur { sigma: 1.0 }, 0, 4, &[&px]), Err(Error::Empty { .. })));
}
