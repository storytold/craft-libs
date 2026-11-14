//! Ogg Vorbis encoding (RFC 5334): the reference libvorbis encoder, built from its C sources by
//! `vorbis_rs`. Stereo or mono, variable bitrate at the requested kbit/s (64–500), with a
//! fallback to a quality preset when the bitrate is not supported for the rate and channel count.

use crate::{AudioBuffer, AudioError, Result};

/// The bitrates an Ogg Vorbis export is asked for, in kbit/s.
pub const OGG_BITRATES: (u32, u32) = (64, 500);

/// The block size the encoder processes at a time: small enough that memory stays bounded
/// regardless of file length.
#[cfg(not(target_arch = "wasm32"))]
const BLOCK: usize = 4096;

#[cfg(not(target_arch = "wasm32"))]
fn encode_error(e: vorbis_rs::VorbisError) -> AudioError {
    AudioError::Encode(e.to_string())
}

/// Encode `buf` as an Ogg Vorbis stream at about `kbps` (variable, as Vorbis is): stereo, or mono
/// for a mono buffer. A bitrate the encoder has no setup for at that rate and channel count falls
/// back to a quality that suits it. `Err` says why there is nothing.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(buf: &AudioBuffer, kbps: u32) -> Result<Vec<u8>> {
    use std::num::{NonZeroU8, NonZeroU32};

    let channels = buf.channels.len();
    if !(1..=2).contains(&channels) {
        return Err(AudioError::Unsupported("Ogg Vorbis export requires mono or stereo audio".into()));
    }
    if buf.channels.iter().any(|c| c.len() != buf.frames() || c.iter().any(|s| !s.is_finite())) {
        return Err(AudioError::Encode("Vorbis samples must be finite and channels equally long".into()));
    }
    let (Some(rate), Some(count)) = (NonZeroU32::new(buf.sample_rate), NonZeroU8::new(channels as u8)) else {
        return Err(AudioError::Encode("there is no audio to encode".into()));
    };
    let target = NonZeroU32::new(kbps.clamp(OGG_BITRATES.0, OGG_BITRATES.1) * 1000).unwrap_or(NonZeroU32::MIN);

    let encode = |strategy: vorbis_rs::VorbisBitrateManagementStrategy| -> Result<Vec<u8>> {
        let mut encoder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(rate, count, Vec::new(), 0x4C43_5246)
            .bitrate_management_strategy(strategy)
            .build()
            .map_err(encode_error)?;
        let frames = buf.frames();
        let mut at = 0usize;
        while at < frames {
            let end = (at + BLOCK).min(frames);
            let block: Vec<&[f32]> = buf.channels.iter().take(channels).map(|side| side.get(at..end).unwrap_or_default()).collect();
            encoder.encode_audio_block(&block).map_err(encode_error)?;
            at = end;
        }
        encoder.finish().map_err(encode_error)
    };

    // Convert Rust unwinds to an error. This does not catch native crashes or aborts.
    let attempt = |strategy| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(strategy)))
            .unwrap_or_else(|_| Err(AudioError::Encode("the Vorbis encoder failed".into())))
    };
    match attempt(vorbis_rs::VorbisBitrateManagementStrategy::Vbr { target_bitrate: target }) {
        Ok(bytes) => Ok(bytes),
        Err(_) => attempt(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr { target_quality: 0.6 })
            .map_err(|e| AudioError::Encode(format!("the Vorbis encoder refused the audio: {e}"))),
    }
}

/// Ogg Vorbis encoding is unavailable on WASM; returns an unsupported-format error.
#[cfg(target_arch = "wasm32")]
pub fn encode(_buf: &AudioBuffer, _kbps: u32) -> Result<Vec<u8>> {
    Err(AudioError::Unsupported("Ogg Vorbis files are exported by the desktop app".into()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn rejects_unsupported_or_malformed_channels_without_dropping_audio() {
        for channels in [0, 3, 6] {
            assert!(matches!(encode(&AudioBuffer::new(48_000, channels, 16), 192), Err(AudioError::Unsupported(_))));
        }
        let ragged = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.0; 16], vec![0.0; 15]] };
        assert!(encode(&ragged, 192).is_err());
        assert!(encode(&AudioBuffer::new(0, 1, 16), 192).is_err());
        let invalid = AudioBuffer { sample_rate: 48_000, channels: vec![vec![f32::NAN]] };
        assert!(encode(&invalid, 192).is_err());
    }

    #[test]
    fn generic_and_direct_encoders_write_mono_and_stereo_ogg() {
        for channels in [1, 2] {
            let audio = AudioBuffer::new(48_000, channels, 512);
            let opts = crate::EncodeOptions::new(crate::EncodeFormat::Ogg);
            assert!(opts.check(48_000, channels).is_ok());
            assert_eq!(opts.bitrate(48_000, channels).unwrap(), None);
            let direct = encode(&audio, 192).unwrap();
            let generic = crate::encode(&audio, &opts).unwrap();
            assert!(direct.starts_with(b"OggS"));
            assert_eq!(direct, generic);
            #[cfg(feature = "decode")]
            {
                let (info, decoded) = crate::decode(&generic, Some("ogg")).unwrap();
                assert_eq!(info.channels, channels);
                assert_eq!(decoded.num_channels(), channels);
                assert_eq!(decoded.frames(), audio.frames());
            }
        }
    }
    #[cfg(feature = "decode")]
    #[test]
    fn distinct_signals_survive_full_and_partial_blocks() {
        for frames in [4096, 4097, 8193] {
            let channels: Vec<Vec<f32>> = [440.0f32, 880.0]
                .into_iter()
                .map(|hz| (0..frames).map(|i| (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin() * 0.4).collect())
                .collect();
            let audio = AudioBuffer { sample_rate: 48_000, channels };
            let bytes = encode(&audio, 192).unwrap();
            let (_, decoded) = crate::decode(&bytes, Some("ogg")).unwrap();
            assert_eq!(decoded.frames(), frames);
            assert_eq!(decoded.channels.len(), 2);
            for (expected, actual) in audio.channels.iter().zip(&decoded.channels) {
                let mse = expected.iter().zip(actual).map(|(a, b)| (a - b).powi(2)).sum::<f32>() / frames as f32;
                // Vorbis is lossy; this tolerates codec error but rejects silence or swapped channels.
                assert!(mse.sqrt() < 0.08, "{frames} frames: RMS error {}", mse.sqrt());
            }
        }
    }
}
