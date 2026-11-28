use craft_audio::{
    AudioBuffer, BitDepth, BwfInfo, EncodeFormat, EncodeOptions, SampleFormat, decode, detect_container, encode, probe, wav_float_header,
    wav_float_header_with_bwf,
};

fn sine(sr: u32, channels: usize, frames: usize) -> AudioBuffer {
    let mut b = AudioBuffer::new(sr, channels as u16, frames);
    for (c, ch) in b.channels.iter_mut().enumerate() {
        let f = 220.0 * (c as f32 + 1.0);
        for (i, s) in ch.iter_mut().enumerate() {
            *s = 0.8 * (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32 + c as f32).sin();
        }
    }
    b
}

fn tolerance(depth: BitDepth, dither: bool) -> f32 {
    let lsb = match depth {
        BitDepth::Int16 => 1.0 / 32768.0,
        BitDepth::Int24 => 1.0 / 8_388_608.0,
        BitDepth::Int32 => 1e-7,
        BitDepth::Float32 => 0.0,
    };
    lsb * if dither { 1.6 } else { 0.6 } + 1e-7
}

fn max_err(a: &AudioBuffer, b: &AudioBuffer) -> f32 {
    assert_eq!(a.num_channels(), b.num_channels());
    assert_eq!(a.frames(), b.frames());
    a.channels.iter().zip(&b.channels).flat_map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p - q).abs())).fold(0.0, f32::max)
}

const DEPTHS: [BitDepth; 4] = [BitDepth::Int16, BitDepth::Int24, BitDepth::Int32, BitDepth::Float32];

fn expected_sf(d: BitDepth) -> SampleFormat {
    match d {
        BitDepth::Int16 => SampleFormat::Int16,
        BitDepth::Int24 => SampleFormat::Int24,
        BitDepth::Int32 => SampleFormat::Int32,
        BitDepth::Float32 => SampleFormat::Float32,
    }
}

#[test]
fn wav_and_aiff_roundtrip_all_depths_and_layouts() {
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff] {
        for depth in DEPTHS {
            for channels in [1, 2, 6] {
                for dither in [false, true] {
                    let src = sine(48_000, channels, 4801);
                    let opts = EncodeOptions::new(format).with_bit_depth(depth).with_dither(dither);
                    let bytes = encode(&src, &opts).unwrap();
                    assert_eq!(detect_container(&bytes, None), Some(format.container()));
                    let info = probe(&bytes, None).unwrap();
                    assert_eq!(info.container, format.container());
                    assert_eq!(info.sample_rate, 48_000);
                    assert_eq!(info.channels as usize, channels);
                    assert_eq!(info.frames, 4801);
                    assert_eq!(info.sample_format, expected_sf(depth));
                    let (dinfo, out) = decode(&bytes, None).unwrap();
                    assert_eq!(dinfo, info);
                    assert_eq!(out.sample_rate, 48_000);
                    let err = max_err(&src, &out);
                    assert!(err <= tolerance(depth, dither), "{format:?} {depth:?} {channels}ch dither={dither}: err {err}");
                }
            }
        }
    }
}

#[test]
fn float_wav_header_with_bwf_keeps_its_bext_as_the_file_grows() {
    let take = sine(48_000, 2, 1_000);
    let bwf = BwfInfo::default()
        .with_description("take 3")
        .with_originator("recorder")
        .with_origination("2026-11-20", "18:30:00")
        .with_time_reference(48_000 * 66_600);
    let mut file = wav_float_header_with_bwf(48_000, 2, 0, &bwf).unwrap();
    let len = file.len();
    assert!(len > wav_float_header(48_000, 2, 0).unwrap().len());
    file.extend(take.interleaved().iter().flat_map(|s| s.to_le_bytes()));
    file[..len].copy_from_slice(&wav_float_header_with_bwf(48_000, 2, 600, &bwf).unwrap());
    let (info, decoded) = decode(&file, None).unwrap();
    assert_eq!((info.bwf.as_ref(), decoded.frames()), (Some(&bwf), 600));
    file[..len].copy_from_slice(&wav_float_header_with_bwf(48_000, 2, 1_000, &bwf).unwrap());
    let opts = EncodeOptions::default().with_bit_depth(BitDepth::Float32).with_bwf(bwf.clone());
    assert_eq!(file, encode(&take, &opts).unwrap());
    let big = wav_float_header_with_bwf(48_000, 2, 600_000_000, &bwf).unwrap();
    assert_eq!((&big[..4], big.len()), (&b"RF64"[..], len));
    assert!(wav_float_header_with_bwf(48_000, 0, 0, &bwf).is_err());
}
