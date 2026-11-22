//! Sizes, budgets, options and values out of range are refused with an error before allocating.

use craft_pbr::{
    ColorEncoding, EnvImage, Environment, Limits, MAX_RADIANCE, PbrError, PrefilterOptions, hammersley, irradiance, sh_basis, try_prefilter, try_sh9,
};

use crate::{env_from, limits, opts};

/// Limits raised as far as they go, so only the `isize::MAX` bound is left.
fn unbounded() -> Limits {
    limits(|l| {
        l.max_side = u32::MAX;
        l.max_pixels = u64::MAX;
        l.max_bytes = u64::MAX;
        l.max_levels = u32::MAX;
        l.max_samples = u32::MAX;
        l.max_work = u64::MAX;
    })
}

#[test]
fn zero_sizes_are_refused() {
    let limits = Limits::default();
    assert_eq!(EnvImage::try_from_linear(0, 4, Vec::new(), &limits), Err(PbrError::BadSize(0, 4)));
    assert_eq!(EnvImage::try_from_linear(4, 0, Vec::new(), &limits), Err(PbrError::BadSize(4, 0)));
    assert_eq!(EnvImage::try_from_rgba(0, 0, &[], ColorEncoding::Linear, &limits), Err(PbrError::BadSize(0, 0)));
    assert_eq!(Environment::Night.try_render(1, &limits), Err(PbrError::BadSize(1, 0)));
    assert_eq!(Environment::Night.try_render(0, &limits), Err(PbrError::BadSize(0, 0)));
}

#[test]
fn huge_sizes_are_refused_before_allocating() {
    let default = Limits::default();
    assert_eq!(Environment::Night.try_render(4096, &default), Err(PbrError::BadSize(4096, 2048)));
    let few = limits(|l| l.max_pixels = 100);
    assert_eq!(Environment::Night.try_render(16, &few), Err(PbrError::TooLarge(16, 8)));
    // With every limit raised, a size over isize::MAX bytes is still refused, and the size is
    // checked before the data length.
    let huge = unbounded();
    assert_eq!(Environment::Night.try_render(u32::MAX, &huge), Err(PbrError::TooLarge(u32::MAX, u32::MAX / 2)));
    assert_eq!(EnvImage::try_from_rgba(u32::MAX, u32::MAX, &[], ColorEncoding::Linear, &huge), Err(PbrError::TooLarge(u32::MAX, u32::MAX)));
    assert_eq!(EnvImage::try_from_linear(u32::MAX, u32::MAX, Vec::new(), &huge), Err(PbrError::TooLarge(u32::MAX, u32::MAX)));
    // 2^60 pixels are 1.5 × 2^63 bytes: within u64, over isize::MAX.
    assert_eq!(EnvImage::try_from_linear(1 << 30, 1 << 30, Vec::new(), &huge), Err(PbrError::TooLarge(1 << 30, 1 << 30)));
    assert_eq!(EnvImage::try_from_linear(4, 2, vec![[0.0; 3]; 7], &default), Err(PbrError::BadDataLength { expected: 8, found: 7 }));
}

#[test]
fn prefiltering_over_the_byte_or_work_budget_is_refused() {
    let env = env_from(256, 128, |_, _| [1.0; 3]);
    let options = PrefilterOptions::default();
    // The source copy alone is 384 KiB.
    let tight = limits(|l| l.max_bytes = 512 << 10);
    assert_eq!(try_prefilter(&env, &options, &tight), Err(PbrError::TooLarge(256, 128)));
    // Levels 1 to 5 hold 128×64 + 64×32 + 32×16 + 16×8 + 8×4 = 10,912 pixels, at 128 samples each.
    let needed = 10_912 * 128;
    let short = limits(|l| l.max_work = needed - 1);
    assert_eq!(try_prefilter(&env, &options, &short), Err(PbrError::TooMuchWork { needed, limit: needed - 1 }));
    let exact = limits(|l| l.max_work = needed);
    assert!(try_prefilter(&env, &options, &exact).is_ok());
    // A raised sample count on a 1 × 1 source passes the work limit, but its sample table does
    // not fit the byte limit.
    let tiny = env_from(1, 1, |_, _| [1.0; 3]);
    let many = limits(|l| {
        l.max_samples = u32::MAX;
        l.max_work = u64::MAX;
    });
    assert_eq!(try_prefilter(&tiny, &opts(|o| o.samples = 100_000_000), &many), Err(PbrError::TooLarge(1, 1)));
    // One level is only a copy, so the sample table is not counted.
    let copy = try_prefilter(&tiny, &opts(|o| (o.levels, o.samples) = (1, 100_000_000)), &many).unwrap();
    assert_eq!(copy, vec![tiny.clone()]);
    // A raised level count is counted without listing the levels first: 2^32 − 2 levels of
    // 1 × 1 are refused for their bytes, and for their work when bytes are not limited.
    let deep = limits(|l| {
        l.max_levels = u32::MAX;
        l.max_work = u64::MAX;
    });
    assert_eq!(try_prefilter(&tiny, &opts(|o| o.levels = u32::MAX), &deep), Err(PbrError::TooLarge(1, 1)));
    let unbounded = limits(|l| {
        l.max_levels = u32::MAX;
        l.max_bytes = u64::MAX;
    });
    let needed = (u64::from(u32::MAX) - 1) * 128;
    assert_eq!(try_prefilter(&tiny, &opts(|o| o.levels = u32::MAX), &unbounded), Err(PbrError::TooMuchWork { needed, limit: 1 << 28 }));
}

#[test]
fn prefiltering_counts_the_source_pyramid_lists() {
    // A 16 × 8 source is halved once to 8 × 4 for the pyramid, and its second level is 8 × 4.
    let env = env_from(16, 8, |_, _| [1.0; 3]);
    let options = opts(|o| (o.levels, o.samples) = (2, 1));
    let (image, reference) = (size_of::<EnvImage>() as u64, size_of::<&EnvImage>() as u64);
    let pixels = (128 + 32 + 32) * 12;
    let sample = size_of::<([f32; 3], f32, &EnvImage)>() as u64;
    let level_lists = 2 * (size_of::<(u32, u32)>() as u64 + image);
    // The halved source level, and references to it and to the source.
    let total = pixels + sample + level_lists + image + 2 * reference;
    assert_eq!(try_prefilter(&env, &options, &limits(|l| l.max_bytes = total - 1)), Err(PbrError::TooLarge(16, 8)));
    assert!(try_prefilter(&env, &options, &limits(|l| l.max_bytes = total)).is_ok());
}

#[test]
fn sh9_over_the_byte_budget_for_its_column_table_is_refused() {
    // A 2048 × 1 source is 24 KiB; its column table holds six `f64` per column, 96 KiB.
    let env = env_from(2048, 1, |_, _| [1.0; 3]);
    let table = 2048 * 48;
    assert_eq!(try_sh9(&env, &limits(|l| l.max_bytes = table - 1)), Err(PbrError::TooLarge(2048, 1)));
    assert!(try_sh9(&env, &limits(|l| l.max_bytes = table)).is_ok());
}

#[test]
fn nan_infinite_and_negative_radiance_is_refused() {
    let limits = Limits::default();
    for (i, bad) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0].into_iter().enumerate() {
        let mut data = vec![[1.0; 3]; 8];
        data[i + 2][i % 3] = bad;
        assert_eq!(EnvImage::try_from_linear(4, 2, data, &limits), Err(PbrError::BadRadiance { index: i + 2 }), "{bad}");
    }
    // The RGBA import replaces them with 0 instead.
    let px = [[f32::NAN, f32::INFINITY, -5.0, 1.0], [1e9, 0.5, f32::NEG_INFINITY, f32::NAN]];
    let e = EnvImage::try_from_rgba(2, 1, &px, ColorEncoding::Linear, &limits).unwrap();
    assert_eq!(e.pixels(), &[[0.0, 0.0, 0.0], [65_000.0, 0.5, 0.0]]);
    let e = EnvImage::try_from_rgba(2, 1, &px, ColorEncoding::Srgb, &limits).unwrap();
    assert_eq!(e.pixels()[1][0], 1.0);
}

#[test]
fn radiance_over_the_maximum_is_refused() {
    let limits = Limits::default();
    for bad in [f32::MAX, 1e29, MAX_RADIANCE * 1.001] {
        let mut data = vec![[1.0; 3]; 8];
        data[5][1] = bad;
        assert_eq!(EnvImage::try_from_linear(4, 2, data, &limits), Err(PbrError::BadRadiance { index: 5 }), "{bad}");
    }
    assert!(EnvImage::try_from_linear(4, 2, vec![[MAX_RADIANCE; 3]; 8], &limits).is_ok());
}

#[test]
fn the_largest_radiance_averages_without_overflow() {
    let limits = Limits::default();
    let rel = |a: f32, b: f32| ((a - b) / b).abs();
    // One bright pixel in a 2 × 2 block, and in a 3 × 3 image.
    let block = env_from(2, 2, |x, y| if (x, y) == (1, 1) { [MAX_RADIANCE; 3] } else { [0.0; 3] });
    let half = block.try_half(&limits).unwrap();
    assert!(rel(half.pixels()[0][0], MAX_RADIANCE / 4.0) < 1e-6, "{:?}", half.pixels());
    let odd = env_from(3, 3, |x, y| if (x, y) == (2, 2) { [MAX_RADIANCE; 3] } else { [0.0; 3] });
    let half = odd.try_half(&limits).unwrap();
    assert!(rel(half.pixels()[0][0], MAX_RADIANCE / 9.0) < 1e-5, "{:?}", half.pixels());
    // A uniform image keeps its value through many samples; one bright pixel is spread out.
    let many = limits_with_samples(100_000);
    let uniform = env_from(8, 4, |_, _| [MAX_RADIANCE; 3]);
    let mips = try_prefilter(&uniform, &opts(|o| (o.levels, o.samples) = (2, 100_000)), &many).unwrap();
    assert!(mips[1].pixels().iter().flatten().all(|v| rel(*v, MAX_RADIANCE) < 1e-3), "{:?}", mips[1].pixels());
    let spot = env_from(32, 16, |x, y| if (x, y) == (8, 8) { [MAX_RADIANCE; 3] } else { [0.0; 3] });
    let mips = try_prefilter(&spot, &PrefilterOptions::default(), &limits).unwrap();
    let peak = mips[5].pixels().iter().flatten().fold(0.0f32, |a, v| a.max(*v));
    assert!(peak > 0.0 && peak < MAX_RADIANCE / 10.0, "{peak}");
    let c = try_sh9(&uniform, &limits).unwrap();
    let e = irradiance(&c, [0.3, 0.4, 0.5]).unwrap();
    assert!(e.iter().all(|v| rel(*v, MAX_RADIANCE) < 1e-4), "{e:?}");
    let mut c = [[0.0; 3]; 9];
    c[4][1] = f32::NAN;
    assert!(matches!(irradiance(&c, [0.0, 1.0, 0.0]), Err(PbrError::BadOption(_))));
}

fn limits_with_samples(samples: u32) -> Limits {
    limits(|l| {
        l.max_samples = samples;
        l.max_work = u64::MAX;
    })
}

#[test]
fn sources_over_the_size_limits_are_refused() {
    let env = env_from(16, 8, |_, _| [1.0; 3]);
    let few = limits(|l| l.max_pixels = 100);
    assert_eq!(try_prefilter(&env, &PrefilterOptions::default(), &few), Err(PbrError::TooLarge(16, 8)));
    assert_eq!(try_sh9(&env, &few), Err(PbrError::TooLarge(16, 8)));
    let narrow = limits(|l| l.max_side = 8);
    assert_eq!(try_prefilter(&env, &PrefilterOptions::default(), &narrow), Err(PbrError::BadSize(16, 8)));
    // Halving and resampling check the source too, not only the smaller result.
    assert_eq!(env.try_half(&few), Err(PbrError::TooLarge(16, 8)));
    assert_eq!(env.try_resampled(4, 2, &few), Err(PbrError::TooLarge(16, 8)));
    assert_eq!(env.try_half(&narrow), Err(PbrError::BadSize(16, 8)));
    assert_eq!(env.try_resampled(8, 4, &narrow), Err(PbrError::BadSize(16, 8)));
}

#[test]
fn options_out_of_range_are_refused() {
    let env = env_from(16, 8, |_, _| [1.0; 3]);
    let limits = Limits::default();
    let bad: [PrefilterOptions; 9] = [
        opts(|o| o.levels = 0),
        opts(|o| o.levels = 9),
        opts(|o| o.samples = 0),
        opts(|o| o.samples = 1025),
        opts(|o| o.lod_bias = f32::NAN),
        opts(|o| o.lod_bias = f32::INFINITY),
        opts(|o| o.firefly_cap = Some(0.0)),
        opts(|o| o.firefly_cap = Some(-1.0)),
        opts(|o| o.firefly_cap = Some(f32::NAN)),
    ];
    for options in bad {
        assert!(matches!(try_prefilter(&env, &options, &limits), Err(PbrError::BadOption(_))), "{options:?}");
    }
    assert_eq!(hammersley(0, 0), None);
    assert_eq!(hammersley(4, 4), None);
    let [u, v] = hammersley(1, 4).unwrap();
    assert!((u - 0.25).abs() < 1e-6 && (v - 0.5).abs() < 1e-6, "{u} {v}");
}

#[test]
fn directions_that_are_not_finite_or_zero_are_refused() {
    let env = env_from(16, 8, |_, _| [2.0; 3]);
    let c = try_sh9(&env, &Limits::default()).unwrap();
    for d in [[0.0; 3], [f32::NAN, 0.0, 1.0], [f32::INFINITY, 0.0, 0.0]] {
        assert_eq!(env.sample(d), Err(PbrError::BadDirection), "{d:?}");
        assert_eq!(sh_basis(d), Err(PbrError::BadDirection), "{d:?}");
        assert_eq!(irradiance(&c, d), Err(PbrError::BadDirection), "{d:?}");
    }
    // Directions need not be unit length.
    assert_eq!(env.sample([1e30, 0.0, 0.0]), Ok([2.0; 3]));
    assert_eq!(env.sample([0.0, 1e-30, 0.0]), Ok([2.0; 3]));
    assert_eq!(env.sample([0.0, 0.0, 1e-40]), Ok([2.0; 3]));
    assert_eq!(sh_basis([f32::from_bits(1), 0.0, 0.0]), sh_basis([1.0, 0.0, 0.0]));
    assert_eq!(sh_basis([0.0, 0.0, 5.0]), sh_basis([0.0, 0.0, 1.0]));
}

#[test]
fn tiny_and_odd_sources_give_levels_that_never_grow() {
    let limits = Limits::default();
    for (w, h) in [(1, 1), (3, 1), (2, 3), (5, 3), (37, 19), (9, 4)] {
        let env = env_from(w, h, |x, y| [x as f32, y as f32, 1.0]);
        let mips = try_prefilter(&env, &opts(|o| o.levels = 8), &limits).unwrap();
        assert_eq!(mips.len(), 8);
        for pair in mips.windows(2) {
            assert!(pair[1].width() <= pair[0].width() && pair[1].height() <= pair[0].height(), "{w}x{h}");
            assert!(pair[1].width() >= pair[0].width().min(4) && pair[1].height() >= pair[0].height().min(2), "{w}x{h}");
        }
        assert!(mips.iter().all(|m| m.pixels().iter().all(|p| p.iter().all(|v| v.is_finite() && *v >= 0.0))));
        assert!(try_sh9(&env, &limits).is_ok());
    }
}
