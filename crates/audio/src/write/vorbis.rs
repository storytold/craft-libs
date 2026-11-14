//! Ogg Vorbis encoding (RFC 5334): the reference libvorbis encoder, built from its C sources by
//! `vorbis_rs`. Stereo or mono, variable bitrate at the requested kbit/s (64–500), with a
//! fallback to a quality preset when the bitrate is not supported for the rate and channel count.

use crate::{AudioBuffer, AudioError, Result};

/// The bitrates an Ogg Vorbis export is asked for, in kbit/s.
pub const OGG_BITRATES: (u32, u32) = (64, 500);

/// The block size the encoder processes at a time: small enough that memory stays bounded
/// regardless of file length.
const BLOCK: usize = 4096;

fn encode_error(e: vorbis_rs::VorbisError) -> AudioError {
    AudioError::Encode(e.to_string())
}

/// Encode `buf` as an Ogg Vorbis stream at about `kbps` (variable, as Vorbis is): stereo, or mono
/// for a mono buffer. A bitrate the encoder has no setup for at that rate and channel count falls
/// back to a quality that suits it. `Err` says why there is nothing.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(buf: &AudioBuffer, kbps: u32) -> Result<Vec<u8>> {
    use std::num::{NonZeroU8, NonZeroU32};

    let channels = buf.channels.len().clamp(1, 2);
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

    // The library is C: whatever it does wrong ends here as an error, never as a crash of the app.
    let attempt = |strategy| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(strategy))).unwrap_or_else(|_| Err(AudioError::Encode("the Vorbis encoder failed".into())));
    match attempt(vorbis_rs::VorbisBitrateManagementStrategy::Vbr { target_bitrate: target }) {
        Ok(bytes) => Ok(bytes),
        Err(_) => attempt(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr { target_quality: 0.6 }).map_err(|e| AudioError::Encode(format!("the Vorbis encoder refused the audio: {e}"))),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn encode(_buf: &AudioBuffer, _kbps: u32) -> Result<Vec<u8>> {
    Err("Ogg Vorbis files are exported by the desktop app".into())
}
