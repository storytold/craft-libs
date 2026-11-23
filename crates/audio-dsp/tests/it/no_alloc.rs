//! Processing, parameter changes and resets do not allocate.

use craft_audio_dsp::create_effect;

/// The voice pitch shifter runs a voiced tone through 10,000 blocks of 480 frames, with its
/// pitch, its fine tune and a return to no shift changed between blocks, and a reset, without one
/// heap allocation.
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
                250 => {
                    fx.set_param("cents", -35.0);
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
