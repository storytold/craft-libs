//! Processing, parameter changes and resets do not allocate.

use craft_audio_dsp::create_effect;

/// The voice pitch shifter runs a voiced tone through 10,000 blocks of 480 frames, with its
/// pitch, its fine tune, Keep Formants and a return to no shift changed between blocks, and a
/// reset, without one heap allocation.
#[test]
fn voice_pitch_does_not_allocate_while_processing() {
    let Some(mut fx) = create_effect("voice_pitch", 48_000.0, 2) else { panic!("voice_pitch is not registered") };
    let tone: Vec<f32> = (0..480).map(|i| (0.4 * (std::f64::consts::TAU * 150.0 * i as f64 / 48_000.0).sin()) as f32).collect();
    let (mut l, mut r) = (tone.clone(), tone.clone());
    let info = allocation_counter::measure(|| {
        for block in 0..10_000u32 {
            match block % 500 {
                0 => {
                    fx.set_param("semitones", (block / 500 % 7) as f32 * 4.0 - 12.0);
                }
                100 => {
                    fx.set_param("formant_follow", 1.0);
                }
                250 => {
                    fx.set_param("cents", -35.0);
                }
                400 => {
                    fx.set_param("formant_follow", 0.0);
                }
                499 => {
                    fx.set_param("semitones", 0.0);
                    fx.set_param("cents", 0.0);
                }
                _ => {}
            }
            if block == 7_777 {
                fx.reset();
            }
            l.copy_from_slice(&tone);
            r.copy_from_slice(&tone);
            fx.process(&mut [&mut l, &mut r]);
        }
    });
    assert_eq!(info.count_total, 0, "{info:?}");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

/// The voice formant shifter runs a voiced tone through 10,000 blocks of 480 frames, with its
/// shift changed between blocks and set back to none, a stretch of silence, and a reset, without
/// one heap allocation.
#[test]
fn voice_formant_does_not_allocate_while_processing() {
    let Some(mut fx) = create_effect("voice_formant", 48_000.0, 2) else { panic!("voice_formant is not registered") };
    let tone: Vec<f32> = (0..480)
        .map(|i| {
            let t = std::f64::consts::TAU * 150.0 * i as f64 / 48_000.0;
            (0.3 * t.sin() + 0.2 * (4.0 * t).sin() + 0.1 * (13.0 * t).sin()) as f32
        })
        .collect();
    let (mut l, mut r) = (tone.clone(), tone.clone());
    let info = allocation_counter::measure(|| {
        for block in 0..10_000u32 {
            match block % 500 {
                0 => {
                    fx.set_param("semitones", (block / 500 % 7) as f32 * 4.0 - 12.0);
                }
                499 => {
                    fx.set_param("semitones", 0.0);
                }
                _ => {}
            }
            if block == 7_777 {
                fx.reset();
            }
            if (3_000..3_050).contains(&block) {
                l.fill(0.0);
                r.fill(0.0);
            } else {
                l.copy_from_slice(&tone);
                r.copy_from_slice(&tone);
            }
            fx.process(&mut [&mut l, &mut r]);
        }
    });
    assert_eq!(info.count_total, 0, "{info:?}");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

/// Noise suppression runs a voiced tone in noise through 10,000 blocks of 480 frames, with its
/// reduction changed between blocks and set to 0, a stretch of silence, blocks that leave the
/// second channel out, a NaN sample and a reset, without one heap allocation.
#[test]
fn noise_suppression_does_not_allocate_while_processing() {
    let Some(mut fx) = create_effect("noise_suppression", 48_000.0, 2) else { panic!("noise_suppression is not registered") };
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let input: Vec<f32> = (0..480)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let noise = (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
            (0.3 * (std::f64::consts::TAU * 150.0 * i as f64 / 48_000.0).sin()) as f32 + 0.05 * noise
        })
        .collect();
    let (mut l, mut r) = (input.clone(), input.clone());
    let info = allocation_counter::measure(|| {
        for block in 0..10_000u32 {
            match block % 500 {
                0 => {
                    fx.set_param("reduction", (block / 500 % 5) as f32 * 10.0);
                }
                499 => {
                    fx.set_param("reduction", 0.0);
                }
                _ => {}
            }
            if block == 7_777 {
                fx.reset();
            }
            if (3_000..3_050).contains(&block) {
                l.fill(0.0);
                r.fill(0.0);
            } else {
                l.copy_from_slice(&input);
                r.copy_from_slice(&input);
            }
            if block == 5_000 {
                l[100] = f32::NAN;
            }
            if (6_000..6_020).contains(&block) {
                fx.process(&mut [&mut l]);
            } else {
                fx.process(&mut [&mut l, &mut r]);
            }
        }
    });
    assert_eq!(info.count_total, 0, "{info:?}");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

/// The bitcrusher runs a voiced tone through 10,000 blocks of 480 frames, with its bits, rate
/// divide and mix changed between blocks, blocks that leave the second channel out, a NaN sample
/// and a reset, without one heap allocation.
#[test]
fn bitcrusher_does_not_allocate_while_processing() {
    let Some(mut fx) = create_effect("bitcrusher", 48_000.0, 2) else { panic!("bitcrusher is not registered") };
    let tone: Vec<f32> = (0..480).map(|i| (0.4 * (std::f64::consts::TAU * 150.0 * i as f64 / 48_000.0).sin()) as f32).collect();
    let (mut l, mut r) = (tone.clone(), tone.clone());
    let info = allocation_counter::measure(|| {
        for block in 0..10_000u32 {
            match block % 500 {
                0 => {
                    fx.set_param("bits", 1.0 + (block / 500 % 16) as f32);
                }
                100 => {
                    fx.set_param("rate_divide", 1.0 + (block / 500 % 7) as f32 * 7.5);
                }
                250 => {
                    fx.set_param("mix", 40.0);
                }
                499 => {
                    fx.set_param("mix", 100.0);
                }
                _ => {}
            }
            if block == 7_777 {
                fx.reset();
            }
            l.copy_from_slice(&tone);
            r.copy_from_slice(&tone);
            if block == 5_000 {
                l[100] = f32::NAN;
            }
            if (6_000..6_020).contains(&block) {
                fx.process(&mut [&mut l]);
            } else {
                fx.process(&mut [&mut l, &mut r]);
            }
        }
    });
    assert_eq!(info.count_total, 0, "{info:?}");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

/// The ring modulator runs a voiced tone through 10,000 blocks of 480 frames, with its effect,
/// modulation source, frequency and depth changed between blocks, blocks that leave the second
/// channel out, a NaN sample and a reset, without one heap allocation.
#[test]
fn ring_mod_does_not_allocate_while_processing() {
    let Some(mut fx) = create_effect("ring_mod", 48_000.0, 2) else { panic!("ring_mod is not registered") };
    let tone: Vec<f32> = (0..480).map(|i| (0.4 * (std::f64::consts::TAU * 150.0 * i as f64 / 48_000.0).sin()) as f32).collect();
    let (mut l, mut r) = (tone.clone(), tone.clone());
    let info = allocation_counter::measure(|| {
        for block in 0..10_000u32 {
            match block % 500 {
                0 => {
                    fx.set_param("effect", (block / 500 % 4) as f32);
                }
                100 => {
                    fx.set_param("mod_source", (block / 500 % 3) as f32);
                }
                250 => {
                    fx.set_param("frequency", 30.0 + (block / 500 % 5) as f32 * 400.0);
                    fx.set_param("mod_depth", 60.0);
                }
                499 => {
                    fx.set_param("mod_depth", 0.0);
                }
                _ => {}
            }
            if block == 7_777 {
                fx.reset();
            }
            l.copy_from_slice(&tone);
            r.copy_from_slice(&tone);
            if block == 5_000 {
                l[100] = f32::NAN;
            }
            if (6_000..6_020).contains(&block) {
                fx.process(&mut [&mut l]);
            } else {
                fx.process(&mut [&mut l, &mut r]);
            }
        }
    });
    assert_eq!(info.count_total, 0, "{info:?}");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}
