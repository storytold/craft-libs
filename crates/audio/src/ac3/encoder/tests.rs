use super::*;

#[test]
fn analysis_synthesis_reconstructs_overlap() {
    let input: Vec<f32> = (0..1024).map(|n| (n as f32 * 0.071).sin() * 0.4).collect();
    let mut overlap = [0.0; 256];
    for offset in (0..768).step_by(256) {
        let mut block = [0.0; 512];
        block.copy_from_slice(&input[offset..offset + 512]);
        let coeffs = transform::analysis(&block, false);
        let mut decoded = [0.0; 512];
        super::super::imdct(&coeffs, false, &mut decoded);
        if offset > 0 {
            for i in 0..256 {
                assert!((2.0 * (decoded[i] + overlap[i]) - input[offset + i]).abs() < 1e-5);
            }
        }
        overlap.copy_from_slice(&decoded[256..]);
    }
}

fn config(codec: Codec, mode: CodingMode, lfe: bool) -> Config {
    Config { codec, sample_rate: 48_000, bitrate_kbps: 448, mode, lfe }
}

#[test]
fn encode_tone_headers_crc_and_level() -> Result<(), Box<dyn std::error::Error>> {
    for codec in [Codec::Ac3, Codec::Eac3] {
        for (mode, lfe) in [(CodingMode::Mono, false), (CodingMode::Stereo, false), (CodingMode::ThreeFrontTwoSurround, true)] {
            let cfg = config(codec, mode, lfe);
            let input: Vec<Vec<f32>> = (0..cfg.channels())
                .map(|ch| (0..3072).map(|n| (n as f32 * (if lfe && ch == 3 { 0.01 } else { 0.04 + ch as f32 * 0.015 })).sin() * 0.2).collect())
                .collect();
            let mut encoder = Encoder::new(cfg)?;
            let mut packets = encoder.push(&input.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
            let finish = encoder.finish()?;
            packets.extend(finish.packets);
            let mut decoder = super::super::Decoder::new();
            let mut decoded = vec![Vec::new(); cfg.channels()];
            for packet in &packets {
                let header = super::super::parse_header(&packet.data)?;
                assert_eq!(header.channels(), cfg.channels());
                assert_eq!(header.samples(), 1536);
                assert_eq!(bitstream::crc(&packet.data[2..]), 0);
                if codec == Codec::Ac3 {
                    let words = packet.data.len() / 2;
                    let split = (words / 2 + words / 8) * 2;
                    assert_eq!(bitstream::crc(&packet.data[2..split]), 0);
                }
                let frame = decoder.decode(&packet.data)?;
                for (dst, src) in decoded.iter_mut().zip(frame.channels) {
                    dst.extend(src);
                }
            }
            for ch in 0..cfg.channels() {
                if lfe && ch == 3 {
                    continue;
                } // The causal LFE low-pass intentionally changes phase.
                let mse = input[ch].iter().zip(&decoded[ch][256..]).map(|(&a, &b)| (a - b).powi(2)).sum::<f32>() / input[ch].len() as f32;
                assert!(mse < 0.0001, "{codec:?} {mode:?} ch{ch}: mse={mse}");
            }
        }
    }
    Ok(())
}

#[test]
fn long_short_transitions_reconstruct_without_gain_or_phase_steps() {
    let mut seed = 23u32;
    let input: Vec<f32> = (0..2048)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed as f32 / u32::MAX as f32 - 0.5) * 0.8
        })
        .collect();
    for switches in [[false; 6], [true; 6], [false, true, false, true, true, false]] {
        let mut overlap = [0.0; 256];
        for (block, short) in switches.into_iter().enumerate() {
            let offset = block * 256;
            let mut samples = [0.0; 512];
            samples.copy_from_slice(&input[offset..offset + 512]);
            let coefficients = transform::analysis(&samples, short);
            let mut decoded = [0.0; 512];
            super::super::imdct(&coefficients, short, &mut decoded);
            if block > 0 {
                for i in 0..256 {
                    assert!((2.0 * (decoded[i] + overlap[i]) - input[offset + i]).abs() < 1e-5);
                }
            }
            overlap.copy_from_slice(&decoded[256..]);
        }
    }
}

#[test]
fn lfe_lowpass_preserves_bass_and_rejects_full_bandwidth_content() {
    for rate in [32000, 44100, 48000] {
        for (frequency, minimum, maximum) in [(60.0, 0.98, 1.01), (120.0, 0.69, 0.72), (1000.0, 0.0, 0.00001)] {
            let mut filter = transform::LowPass::new(rate);
            let mut power = 0.0;
            let mut source_power = 0.0;
            for n in 0..rate {
                let input = (n as f32 * std::f32::consts::TAU * frequency / rate as f32).sin() * 0.5;
                let output = filter.process(input);
                if n > rate / 2 {
                    power += f64::from(output).powi(2);
                    source_power += f64::from(input).powi(2);
                }
            }
            let gain = (power / source_power).sqrt();
            assert!((minimum..=maximum).contains(&gain), "rate={rate} frequency={frequency} gain={gain}");
        }
    }
}

#[test]
fn rates_modes_and_crc_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    for codec in [Codec::Ac3, Codec::Eac3] {
        for rate in [32000, 44100, 48000] {
            for mode in [
                CodingMode::DualMono,
                CodingMode::Mono,
                CodingMode::Stereo,
                CodingMode::ThreeFront,
                CodingMode::StereoSurround,
                CodingMode::ThreeFrontSurround,
                CodingMode::StereoTwoSurround,
                CodingMode::ThreeFrontTwoSurround,
            ] {
                for lfe in [false, true] {
                    let cfg = Config { sample_rate: rate, ..config(codec, mode, lfe) };
                    let mut encoder = Encoder::new(cfg)?;
                    let planes = vec![vec![0.0; 1537]; cfg.channels()];
                    let mut packets = encoder.push(&planes.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
                    let finish = encoder.finish()?;
                    packets.extend(finish.packets);
                    assert_eq!(packets.len(), 2);
                    assert_eq!(finish.padding_samples, 1279);
                    for packet in packets {
                        let h = super::super::parse_header(&packet.data)?;
                        assert_eq!(h.sample_rate, rate);
                        assert_eq!(h.acmod, mode as u8);
                        assert_eq!(h.lfeon, lfe);
                        assert_eq!(h.frame_bytes, packet.data.len());
                        assert_eq!(bitstream::crc(&packet.data[2..]), 0);
                        let decoded = super::super::Decoder::new().decode(&packet.data)?;
                        assert!(decoded.channels.iter().flatten().all(|&v| v == 0.0));
                        if codec == Codec::Ac3 {
                            let words = packet.data.len() / 2;
                            assert_eq!(bitstream::crc(&packet.data[2..(words / 2 + words / 8) * 2]), 0);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn input_validation_and_terminal_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config(Codec::Ac3, CodingMode::Stereo, false);
    let mut encoder = Encoder::new(cfg)?;
    assert!(encoder.push(&[&[0.0]]).is_err());
    assert!(encoder.push(&[&[0.0], &[]]).is_err());
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.01, -1.01] {
        assert!(encoder.push(&[&[value], &[0.0]]).is_err());
    }
    assert_eq!(encoder.input_samples, 0);
    assert!(encoder.push(&[&[], &[]])?.is_empty());
    let finish = encoder.finish()?;
    assert!(finish.packets.is_empty());
    assert_eq!(finish.priming_samples, 0);
    assert_eq!(finish.padding_samples, 0);
    assert_eq!(encoder.push(&[&[], &[]]).err(), Some(Error::Finished));
    assert_eq!(encoder.finish().err(), Some(Error::Finished));
    // Force an internal impossible budget to exercise terminal handling without
    // relying on a public configuration that new() should have rejected.
    let mut failing = Encoder::new(cfg)?;
    failing.config.bitrate_kbps = 1;
    assert_eq!(failing.push(&[&[0.0; 1536], &[0.0; 1536]]).err(), Some(Error::BitBudget));
    assert_eq!(failing.push(&[&[], &[]]).err(), Some(Error::Finished));
    assert_eq!(failing.finish().err(), Some(Error::Finished));
    let mut overflow = Encoder::new(cfg)?;
    overflow.input_samples = u64::MAX - 2000;
    assert!(overflow.push(&[&[0.0; 1000], &[0.0; 1000]]).is_err());
    assert_eq!(overflow.input_samples, u64::MAX - 2000);
    Ok(())
}

#[test]
fn chunk_boundaries_and_exact_trim_counts() -> Result<(), Box<dyn std::error::Error>> {
    for codec in [Codec::Ac3, Codec::Eac3] {
        for length in [1, 255, 256, 1280, 1535, 1536, 1537, 3071] {
            let cfg = config(codec, CodingMode::Stereo, false);
            let input: Vec<f32> = (0..length).map(|n| (n as f32 * 0.071).sin() * 0.2).collect();
            let mut contiguous = Encoder::new(cfg)?;
            let mut whole = contiguous.push(&[&input, &input])?;
            let finish = contiguous.finish()?;
            whole.extend(finish.packets);
            assert_eq!(whole.len() as u64 * 1536, length as u64 + u64::from(finish.priming_samples) + u64::from(finish.padding_samples));
            let mut chunked = Encoder::new(cfg)?;
            let mut pieces = Vec::new();
            for chunk in input.chunks(37) {
                pieces.extend(chunked.push(&[chunk, chunk])?);
            }
            let end = chunked.finish()?;
            pieces.extend(end.packets);
            assert_eq!(whole.iter().map(|p| &p.data).collect::<Vec<_>>(), pieces.iter().map(|p| &p.data).collect::<Vec<_>>());
            assert_eq!(finish.padding_samples, end.padding_samples);
        }
    }
    Ok(())
}

#[test]
fn configuration_validates_real_frame_bounds_and_explicit_layout() {
    for rate in [32000, 44100] {
        assert!(Encoder::new(Config { codec: Codec::Eac3, sample_rate: rate, bitrate_kbps: 1024, mode: CodingMode::Stereo, lfe: false }).is_err());
    }
    assert!(Encoder::new(Config { codec: Codec::Eac3, sample_rate: 48000, bitrate_kbps: 1024, mode: CodingMode::Stereo, lfe: false }).is_ok());
    assert!(Encoder::new(config(Codec::Ac3, CodingMode::Surround71, true)).is_err());
    assert!(Encoder::new(config(Codec::Eac3, CodingMode::Surround71, false)).is_err());
    assert_eq!(config(Codec::Eac3, CodingMode::Surround71, true).channel_layout(), ChannelLayout::SURROUND_7_1);
    assert!(!config(Codec::Ac3, CodingMode::DualMono, false).channel_layout().is_ordered());
}

#[test]
fn dec3_describes_dependent_rear_pair_and_aggregate_rate() -> Result<(), Box<dyn std::error::Error>> {
    // Independently verified against a Dolby-hosted 48k/768k 7.1 sample's
    // six-byte dec3 descriptor and ETSI Annex F; media is not redistributed.
    let encoder = Encoder::new(Config { codec: Codec::Eac3, sample_rate: 48000, bitrate_kbps: 768, mode: CodingMode::Surround71, lfe: true })?;
    assert_eq!(encoder.codec_configuration(), [0x18, 0x00, 0x20, 0x0f, 0x02, 0x02]);
    for rate in [32000, 44100, 48000] {
        let cfg = Config { codec: Codec::Eac3, sample_rate: rate, bitrate_kbps: 640, mode: CodingMode::Surround71, lfe: true };
        let mut encoder = Encoder::new(cfg)?;
        let meta = encoder.codec_configuration();
        let declared = (u32::from(meta[0]) << 5) | (u32::from(meta[1]) >> 3);
        let pcm = vec![vec![0.0; 3072]; 8];
        for packet in encoder.push(&pcm.iter().map(Vec::as_slice).collect::<Vec<_>>())? {
            let actual = (packet.data.len() as u64 * 8 * u64::from(rate)).div_ceil(1_536_000);
            assert!(u64::from(declared) >= actual);
            let first = super::super::parse_header(&packet.data)?;
            let second = super::super::parse_header(&packet.data[first.frame_bytes..])?;
            assert_eq!((first.strmtyp, first.acmod, first.lfeon), (0, 7, true));
            assert_eq!((second.strmtyp, second.acmod, second.lfeon), (1, 6, false));
            assert_eq!(first.frame_bytes + second.frame_bytes, packet.data.len());
        }
    }
    Ok(())
}

#[test]
fn actual_decoder_positions_satisfy_all_three_ac3_bit_budgets() -> Result<(), Box<dyn std::error::Error>> {
    for rate in [32000, 44100, 48000] {
        for bitrate in super::super::BITRATES {
            for mode in [CodingMode::Mono, CodingMode::ThreeFrontTwoSurround] {
                let cfg =
                    Config { codec: Codec::Ac3, sample_rate: rate, bitrate_kbps: bitrate, mode, lfe: mode == CodingMode::ThreeFrontTwoSurround };
                if cfg.validate().is_err() {
                    continue;
                }
                for stimulus in 0..3 {
                    let mut seed = 17u32;
                    let input: Vec<Vec<f32>> = (0..cfg.channels())
                        .map(|_| {
                            (0..1536)
                                .map(|n| {
                                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                                    match stimulus {
                                        0 => 0.0,
                                        1 => {
                                            if n == 513 {
                                                0.8
                                            } else {
                                                0.0
                                            }
                                        }
                                        _ => ((seed >> 8) as f32 / 16777216.0 - 0.5) * 0.9,
                                    }
                                })
                                .collect()
                        })
                        .collect();
                    let mut encoder = Encoder::new(cfg)?;
                    for packet in encoder.push(&input.iter().map(Vec::as_slice).collect::<Vec<_>>())? {
                        let mut decoder = super::super::Decoder::new();
                        decoder.decode(&packet.data)?;
                        assert_eq!(decoder.block_bit_positions.len(), 6);
                        let words = packet.data.len() / 2;
                        let split = (words / 2 + words / 8) * 16;
                        assert!(decoder.block_bit_positions[1].1 <= split, "first5/8: {cfg:?} stimulus{stimulus}");
                        assert!(decoder.block_bit_positions[5].0 >= split, "final3/8: {cfg:?} stimulus{stimulus}");
                        assert!(decoder.block_bit_positions[5].1 + 18 <= words * 16, "wholeframe: {cfg:?} stimulus{stimulus}");
                        assert_eq!(bitstream::crc(&packet.data[2..split / 8]), 0);
                        assert_eq!(bitstream::crc(&packet.data[2..]), 0);
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn lfe_final_nonzero_input_is_chunk_invariant() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config(Codec::Eac3, CodingMode::ThreeFrontTwoSurround, true);
    let mut input = vec![vec![0.0; 1537]; 6];
    input[3][1536] = 0.75;
    let mut one = Encoder::new(cfg)?;
    let mut a = one.push(&input.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
    a.extend(one.finish()?.packets);
    let mut two = Encoder::new(cfg)?;
    let mut b = Vec::new();
    for range in [0..1, 1..255, 255..1536, 1536..1537] {
        b.extend(two.push(&input.iter().map(|p| &p[range.clone()]).collect::<Vec<_>>())?);
    }
    b.extend(two.finish()?.packets);
    assert_eq!(a.iter().map(|p| &p.data).collect::<Vec<_>>(), b.iter().map(|p| &p.data).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn transient_adjacent_tone_refreshes_exponents_within_the_frame() -> Result<(), Box<dyn std::error::Error>> {
    // Independent QC localized a previous shared-six-block exponent envelope's
    // degradation to the first 32 ms after this onset. Preserve that regression.
    let rate = 32000u32;
    let input: Vec<f32> = (0..rate)
        .map(|n| {
            if n == 3200 || n == 1535 {
                0.85
            } else if (4800..11200).contains(&n) {
                (std::f32::consts::TAU * 301.0 * n as f32 / rate as f32).sin() * 0.4
            } else {
                0.0
            }
        })
        .collect();
    let mut encoder = Encoder::new(Config { codec: Codec::Ac3, sample_rate: rate, bitrate_kbps: 96, mode: CodingMode::Mono, lfe: false })?;
    let mut packets = encoder.push(&[&input])?;
    packets.extend(encoder.finish()?.packets);
    let mut decoder = super::super::Decoder::new();
    let mut decoded = Vec::new();
    for packet in packets {
        decoded.extend(decoder.decode(&packet.data)?.channels.remove(0));
    }
    let mut signal = 0.0f64;
    let mut error = 0.0f64;
    for n in 5440..10560 {
        signal += f64::from(input[n]).powi(2);
        error += f64::from(input[n] - decoded[n + 256]).powi(2);
    }
    let snr = 10.0 * (signal / error).log10();
    assert!(snr > 55.0, "transient-adjacent tone SNR {snr} dB");
    Ok(())
}

#[test]
fn surround_compatibility_downmix_retains_correlated_signal_headroom() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config { codec: Codec::Eac3, sample_rate: 48000, bitrate_kbps: 640, mode: CodingMode::Surround71, lfe: true };
    let mut pcm = vec![vec![0.0; 3072]; 8];
    for ch in [4, 5, 6, 7] {
        for n in 0..3072 {
            pcm[ch][n] = (n as f32 * std::f32::consts::TAU * 440.0 / 48000.0).sin() * 0.99;
        }
    }
    let mut encoder = Encoder::new(cfg)?;
    let mut packets = encoder.push(&pcm.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
    packets.extend(encoder.finish()?.packets);
    let mut decoder = super::super::Decoder::new();
    let mut left = Vec::new();
    for packet in packets {
        let h = super::super::parse_header(&packet.data)?;
        let frame = decoder.decode(&packet.data[..h.frame_bytes])?;
        left.extend_from_slice(&frame.channels[4]);
    }
    let mse = (256..3072).map(|n| (left[n + 256] - pcm[6][n]).powi(2)).sum::<f32>() / (3072 - 256) as f32;
    assert!(mse < 0.0002, "compatibility downmix mse={mse}");
    assert!(left.iter().all(|v| v.abs() < 1.05));
    Ok(())
}

#[test]
fn encoder_errors_convert_to_audio_errors() {
    let bad = Config { codec: Codec::Ac3, sample_rate: 22_050, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false };
    let made: crate::Result<Encoder> = Encoder::new(bad).map_err(Into::into);
    assert!(matches!(made, Err(crate::AudioError::Encode(m)) if m.contains("sample rate")));
}
