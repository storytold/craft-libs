//! Embedded PCM essence in and out of WAVE and AIFF wrappers (craft-audio does the containers).

/// PCM (little-endian, 8-bit unsigned), channels, sample rate and bits of a WAVE file.
pub(crate) fn parse_wav(b: &[u8]) -> Option<(Vec<u8>, u16, u32, u16)> {
    craft_audio::wav_pcm(b).map(|p| (p.data, p.channels, p.sample_rate, p.bits))
}

/// The same for an AIFF / AIFF-C (`NONE`, `twos`, `sowt`) file.
pub(crate) fn parse_aiff(b: &[u8]) -> Option<(Vec<u8>, u16, u32, u16)> {
    craft_audio::aiff_pcm(b).map(|p| (p.data, p.channels, p.sample_rate, p.bits))
}
