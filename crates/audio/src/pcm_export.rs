//! Audio-only files: WAV (RIFF, little-endian PCM; RF64 past 4 GiB) and AIFF (big-endian PCM),
//! 16, 24 or 32-bit integer or 32-bit float ([`SampleFormat`]), for streaming export.
//! Float WAV is `WAVE_FORMAT_IEEE_FLOAT` with a `fact` chunk; float AIFF is AIFF-C (`fl32`).
//!
//! The header is written first from the known length (`header`), then the samples in chunks
//! (`encode`) and the pad byte (`pad`), so an export streams to disk instead of building the
//! file in memory.

/// PCM representation in an exported file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// 16-bit signed integer.
    Int16,
    /// 24-bit signed integer.
    Int24,
    /// 32-bit signed integer.
    Int32,
    /// 32-bit IEEE float.
    Float32,
}

impl SampleFormat {
    /// Bits per sample.
    pub fn bits(self) -> u16 {
        match self {
            Self::Int16 => 16,
            Self::Int24 => 24,
            Self::Int32 | Self::Float32 => 32,
        }
    }
    /// Bytes per sample.
    pub fn bytes(self) -> usize {
        usize::from(self.bits() / 8)
    }
    /// Whether this is a floating point format.
    pub fn is_float(self) -> bool {
        self == Self::Float32
    }
    /// Clip and quantize one sample with symmetric integer full scale; NaN becomes silence.
    pub fn quantise(self, s: f32) -> i32 {
        let s = s.clamp(-1.0, 1.0);
        match self {
            Self::Int16 => (s * 32_767.0).round() as i32,
            Self::Int24 => (s * 8_388_607.0).round() as i32,
            Self::Int32 => (f64::from(s) * 2_147_483_647.0).round() as i32,
            Self::Float32 => 0,
        }
    }
    /// Sanitize a floating sample: NaN becomes silence, infinity becomes the largest finite value.
    pub fn float(s: f32) -> f32 {
        if s.is_nan() { 0.0 } else { s.clamp(f32::MIN, f32::MAX) }
    }
}

/// The container of an audio-only export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmContainer {
    /// RIFF/WAVE or RF64.
    Wav,
    /// AIFF or AIFF-C.
    Aiff,
}

/// Bytes of sample data for `frames` frames, or None when that does not fit 64 bits.
fn data_len(frames: u64, channels: u16, format: SampleFormat) -> Option<u64> {
    frames.checked_mul(u64::from(channels))?.checked_mul(format.bytes() as u64)
}

/// The pad byte after an odd-sized data chunk (RIFF and AIFF chunks are word-aligned).
pub fn pad(frames: u64, channels: u16, format: SampleFormat) -> &'static [u8] {
    if data_len(frames, channels, format).is_some_and(|n| n & 1 == 1) { &[0] } else { &[] }
}

/// `KSDATAFORMAT_SUBTYPE_PCM` / `_IEEE_FLOAT` after their first two bytes (the format tag).
const SUBTYPE_TAIL: [u8; 14] = [0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];

/// AIFF-C format version 1 (`FVER`).
const AIFC_VERSION_1: u32 = 0xA280_5140;

/// The AIFF-C compression name of `fl32`, as a Pascal string of even length.
const FL32_NAME: &[u8] = b"\x1532-bit floating point";

/// Everything before the samples of a file of `frames` frames. A WAV whose RIFF size would not
/// fit 32 bits (4 GiB: 24-bit 5.1 at 96 kHz after about 41 minutes) is written as RF64 (EBU Tech
/// 3306), with the 64-bit sizes in a `ds64` chunk. AIFF has no 64-bit form, so a longer AIFF is
/// an error naming WAV instead.
pub fn header(container: PcmContainer, channels: u16, sample_rate: u32, format: SampleFormat, frames: u64) -> Result<Vec<u8>, String> {
    let too_long = || "the audio is too long to write".to_string();
    if channels == 0 {
        return Err("audio needs at least one channel".into());
    }
    if sample_rate == 0 {
        return Err("audio needs a sample rate".into());
    }
    let bits = format.bits();
    let bps = bits / 8;
    let float = format.is_float();
    let data = data_len(frames, channels, format).ok_or_else(too_long)?;
    let block_align = channels.checked_mul(bps).ok_or("too many audio channels")?;
    match container {
        PcmContainer::Wav => {
            let byte_rate =
                u32::try_from(u64::from(sample_rate) * u64::from(block_align)).map_err(|_| "the audio sample rate is too high".to_string())?;
            let ext = channels > 2;
            // plain PCM has no cbSize; IEEE float has an empty one
            let fmt_len: u32 = if ext {
                40
            } else if float {
                18
            } else {
                16
            };
            // every format but integer PCM has a `fact` chunk (the frame count; RF64: in `ds64`)
            let fact_len: u64 = if float { 12 } else { 0 };
            let padded = data.checked_add(data & 1).ok_or_else(too_long)?;
            // "WAVE" + fmt chunk + fact chunk + data chunk header + samples
            let riff = (4 + 8 + u64::from(fmt_len) + fact_len + 8).checked_add(padded).ok_or_else(too_long)?;
            let rf64 = riff > u64::from(u32::MAX);
            let mut v = Vec::with_capacity(100);
            if rf64 {
                const DS64: u64 = 8 + 28;
                v.extend_from_slice(b"RF64");
                v.extend_from_slice(&u32::MAX.to_le_bytes());
                v.extend_from_slice(b"WAVEds64");
                v.extend_from_slice(&28u32.to_le_bytes());
                v.extend_from_slice(&riff.checked_add(DS64).ok_or_else(too_long)?.to_le_bytes());
                v.extend_from_slice(&data.to_le_bytes());
                v.extend_from_slice(&frames.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes()); // no table entries
            } else {
                v.extend_from_slice(b"RIFF");
                v.extend_from_slice(&(riff as u32).to_le_bytes());
                v.extend_from_slice(b"WAVE");
            }
            let tag: u16 = if float { 3 } else { 1 };
            v.extend_from_slice(b"fmt ");
            v.extend_from_slice(&fmt_len.to_le_bytes());
            v.extend_from_slice(&(if ext { 0xFFFE } else { tag }).to_le_bytes());
            v.extend_from_slice(&channels.to_le_bytes());
            v.extend_from_slice(&sample_rate.to_le_bytes());
            v.extend_from_slice(&byte_rate.to_le_bytes());
            v.extend_from_slice(&block_align.to_le_bytes());
            v.extend_from_slice(&bits.to_le_bytes());
            if ext {
                // cbSize, valid bits, channel mask, KSDATAFORMAT_SUBTYPE_PCM / _IEEE_FLOAT
                v.extend_from_slice(&22u16.to_le_bytes());
                v.extend_from_slice(&bits.to_le_bytes());
                let mask: u32 = if channels == 6 { 0x3F } else { 0 };
                v.extend_from_slice(&mask.to_le_bytes());
                v.extend_from_slice(&tag.to_le_bytes());
                v.extend_from_slice(&SUBTYPE_TAIL);
            } else if float {
                v.extend_from_slice(&0u16.to_le_bytes());
            }
            if float {
                v.extend_from_slice(b"fact");
                v.extend_from_slice(&4u32.to_le_bytes());
                v.extend_from_slice(&(if rf64 { u32::MAX } else { u32::try_from(frames).unwrap_or(u32::MAX) }).to_le_bytes());
            }
            v.extend_from_slice(b"data");
            v.extend_from_slice(&(if rf64 { u32::MAX } else { data as u32 }).to_le_bytes());
            Ok(v)
        }
        PcmContainer::Aiff => {
            let aiff_too_long = || "AIFF holds at most 4 GB of audio: export WAV, which has no such limit".to_string();
            let frames32 = u32::try_from(frames).map_err(|_| aiff_too_long())?;
            // SSND: offset + block size + samples
            let ssnd = u32::try_from(data.checked_add(8).ok_or_else(aiff_too_long)?).map_err(|_| aiff_too_long())?;
            // floats need AIFF-C: a version chunk, and the compression type and name in COMM
            let comm: u32 = if float { 18 + 4 + FL32_NAME.len() as u32 } else { 18 };
            let fver: u32 = if float { 8 + 4 } else { 0 };
            let form = (4u32 + fver + (8 + comm))
                .checked_add(8)
                .and_then(|n| n.checked_add(ssnd))
                .and_then(|n| n.checked_add(ssnd & 1))
                .ok_or_else(aiff_too_long)?;
            let mut v = Vec::with_capacity(90);
            v.extend_from_slice(b"FORM");
            v.extend_from_slice(&form.to_be_bytes());
            if float {
                v.extend_from_slice(b"AIFCFVER");
                v.extend_from_slice(&4u32.to_be_bytes());
                v.extend_from_slice(&AIFC_VERSION_1.to_be_bytes());
            } else {
                v.extend_from_slice(b"AIFF");
            }
            v.extend_from_slice(b"COMM");
            v.extend_from_slice(&comm.to_be_bytes());
            v.extend_from_slice(&channels.to_be_bytes());
            v.extend_from_slice(&frames32.to_be_bytes());
            v.extend_from_slice(&bits.to_be_bytes());
            v.extend_from_slice(&extended(sample_rate));
            if float {
                v.extend_from_slice(b"fl32");
                v.extend_from_slice(FL32_NAME);
            }
            v.extend_from_slice(b"SSND");
            v.extend_from_slice(&ssnd.to_be_bytes());
            v.extend_from_slice(&0u32.to_be_bytes()); // offset
            v.extend_from_slice(&0u32.to_be_bytes()); // block size
            Ok(v)
        }
    }
}

/// Append interleaved samples in `format` (little-endian for WAV, big-endian for AIFF).
pub fn encode(container: PcmContainer, interleaved: &[f32], format: SampleFormat, out: &mut Vec<u8>) {
    let bps = format.bytes();
    out.reserve(interleaved.len().saturating_mul(bps));
    for &s in interleaved {
        let q = if format.is_float() { SampleFormat::float(s).to_bits() as i32 } else { format.quantise(s) };
        match container {
            PcmContainer::Wav => out.extend_from_slice(q.to_le_bytes().get(..bps).unwrap_or_default()),
            PcmContainer::Aiff => out.extend_from_slice(q.to_be_bytes().get(4 - bps..).unwrap_or_default()),
        }
    }
}

/// A whole file in memory (short audio; exports stream with [`header`] and [`encode`]).
fn write_whole(container: PcmContainer, interleaved: &[f32], channels: u16, sample_rate: u32, format: SampleFormat) -> Result<Vec<u8>, String> {
    let per_frame = usize::from(channels.max(1));
    if !interleaved.len().is_multiple_of(per_frame) {
        return Err("the samples do not make whole frames".into());
    }
    let frames = (interleaved.len() / per_frame) as u64;
    let mut v = header(container, channels, sample_rate, format, frames)?;
    encode(container, interleaved, format, &mut v);
    v.extend_from_slice(pad(frames, channels, format));
    Ok(v)
}

/// A RIFF/WAVE file of interleaved samples. More than two channels are written as
/// `WAVE_FORMAT_EXTENSIBLE` with the speaker mask of the layout (6 channels: L, R, C, LFE, Ls, Rs
/// = `0x3F`).
pub fn write_wav(interleaved: &[f32], channels: u16, sample_rate: u32, format: SampleFormat) -> Result<Vec<u8>, String> {
    write_whole(PcmContainer::Wav, interleaved, channels, sample_rate, format)
}

/// IEEE 754 80-bit extended encoding of a sample rate (AIFF `COMM`).
pub fn extended(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let shift = rate.leading_zeros();
    let mant = (rate as u64) << (32 + shift);
    let exp = 16383 + 31 - shift as u16;
    out[..2].copy_from_slice(&exp.to_be_bytes());
    out[2..].copy_from_slice(&mant.to_be_bytes());
    out
}

/// An AIFF file (`FORM`/`AIFF`, `COMM` + `SSND`; AIFF-C for floats) of interleaved samples.
pub fn write_aiff(interleaved: &[f32], channels: u16, sample_rate: u32, format: SampleFormat) -> Result<Vec<u8>, String> {
    write_whole(PcmContainer::Aiff, interleaved, channels, sample_rate, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_formats_preserve_samples_and_bounds() {
        let samples = [1.0, -1.0, f32::NAN];
        let wav = write_wav(&samples, 1, 48_000, SampleFormat::Int24).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[44..53], &[255, 255, 127, 1, 0, 128, 0, 0, 0]);
        let h = header(PcmContainer::Wav, 6, 96_000, SampleFormat::Int24, 96_000 * 3600).unwrap();
        assert_eq!(&h[..4], b"RF64");
        assert!(header(PcmContainer::Aiff, 2, 48_000, SampleFormat::Int24, u64::MAX).is_err());
        let aiff = write_aiff(&[0.25, -3.0], 1, 44_100, SampleFormat::Float32).unwrap();
        assert_eq!(&aiff[8..12], b"AIFC");
        assert_eq!(&aiff[aiff.len() - 8..], [0.25f32.to_be_bytes(), (-3.0f32).to_be_bytes()].concat());
    }

    #[test]
    fn zero_channels_rates_and_partial_frames_are_refused() {
        for container in [PcmContainer::Wav, PcmContainer::Aiff] {
            assert!(header(container, 0, 48_000, SampleFormat::Int16, 1).is_err());
            assert!(header(container, 1, 0, SampleFormat::Int16, 1).is_err());
            assert!(header(container, 2, 48_000, SampleFormat::Int16, 0).is_ok(), "zero frames stay valid");
        }
        assert!(write_wav(&[0.5], 0, 48_000, SampleFormat::Int16).is_err());
        assert!(write_wav(&[0.1, 0.2, 0.3], 2, 48_000, SampleFormat::Int16).is_err());
        assert!(write_aiff(&[0.1, 0.2, 0.3], 2, 48_000, SampleFormat::Int16).is_err());
        assert!(write_aiff(&[0.1, 0.2], 2, 48_000, SampleFormat::Int16).is_ok());
    }
}
