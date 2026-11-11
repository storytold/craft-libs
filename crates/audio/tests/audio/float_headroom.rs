use craft_audio::{AudioBuffer, BitDepth, EncodeFormat, EncodeOptions, FileEncoder, SampleFormat, decode, encode};
use std::io::Cursor;

fn encoded(audio: &AudioBuffer, options: &EncodeOptions, streaming: bool) -> Vec<u8> {
    if !streaming {
        return encode(audio, options).unwrap();
    }
    let mut writer = FileEncoder::new(Cursor::new(Vec::new()), options, audio.sample_rate, audio.num_channels()).unwrap();
    // Cross the writer's 4096-frame scratch boundary, then write a short final block.
    for start in (0..audio.frames()).step_by(4097) {
        let end = (start + 4097).min(audio.frames());
        let block: Vec<Vec<f32>> = audio.channels.iter().map(|channel| channel[start..end].to_vec()).collect();
        writer.write(&block, end - start).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn assert_float_headroom(streaming: bool) {
    let samples = [
        -f32::MAX,
        -2.0,
        -1.25,
        -1.0,
        -0.5,
        -f32::MIN_POSITIVE,
        -f32::from_bits(1),
        -0.0,
        0.0,
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        0.5,
        1.0,
        1.25,
        2.0,
        f32::MAX,
    ];
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff] {
        for dither in [false, true] {
            for channels in [1, 2] {
                let audio = AudioBuffer {
                    sample_rate: 48_000,
                    channels: (0..channels).map(|c| samples.iter().copied().cycle().skip(c).take(4103).collect()).collect(),
                };
                let options = EncodeOptions::new(format).with_bit_depth(BitDepth::Float32).with_dither(dither);
                let bytes = encoded(&audio, &options, streaming);
                let pcm: Vec<u8> =
                    audio.interleaved().iter().flat_map(|v| if format == EncodeFormat::Wav { v.to_le_bytes() } else { v.to_be_bytes() }).collect();
                assert!(bytes.ends_with(&pcm), "{format:?}, dither={dither}: stored sample bytes changed");
                let (info, decoded) = decode(&bytes, None).unwrap();
                assert_eq!(info.sample_format, SampleFormat::Float32);
                assert_eq!((decoded.sample_rate, decoded.num_channels(), decoded.frames()), (48_000, channels as u16, 4103));
                for (channel, (actual, expected)) in decoded.channels.iter().zip(&audio.channels).enumerate() {
                    for (frame, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                        assert_eq!(actual.to_bits(), expected.to_bits(), "{format:?}, dither={dither}, channel={channel}, frame={frame}");
                    }
                }
            }
        }
    }
}

#[test]
fn whole_file_float32_preserves_finite_headroom() {
    assert_float_headroom(false);
}
