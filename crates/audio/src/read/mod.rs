//! Format detection, probing and decoding: native WAV and AIFF readers, `symphonia` for the rest.

mod aiff;
mod caf;
mod flac;
mod mp4;
mod native;
mod pcm;
mod symph;
mod wav;

use crate::{AudioBuffer, AudioInfo, FileFormat, Result};

/// Size of an ID3v2 tag at the start of `b`, if there is one. The 10-byte footer flag (`0x10`)
/// exists only in ID3v2.4; in v2.2/v2.3 that bit is reserved and must not extend the skip.
pub(crate) fn id3v2_len(b: &[u8]) -> Option<usize> {
    if b.get(..3)? != b"ID3" {
        return None;
    }
    let version = *b.get(3)?;
    let flags = *b.get(5)?;
    let size = b.get(6..10)?.iter().fold(0usize, |acc, &x| (acc << 7) | usize::from(x & 0x7F));
    let footer = version == 4 && flags & 0x10 != 0;
    Some(10 + size + if footer { 10 } else { 0 })
}

fn detect_by_magic(b: &[u8], depth: u32) -> Option<FileFormat> {
    let head = |n: usize| b.get(..n);
    if wav::is_wav(b) {
        return Some(FileFormat::Wav);
    }
    if aiff::is_aiff(b) {
        return Some(FileFormat::Aiff);
    }
    match head(4)? {
        b"fLaC" => return Some(FileFormat::Flac),
        b"OggS" => return Some(FileFormat::Ogg),
        b"caff" => return Some(FileFormat::Caf),
        [0x1A, 0x45, 0xDF, 0xA3] => return Some(FileFormat::Other),
        _ => {}
    }
    if b.get(4..8) == Some(b"ftyp") {
        // MP4 / M4A: the sample entry inside `stsd` names the codec.
        let found = b.windows(4).position(|w| w == b"stsd");
        let codec = found.and_then(|p| b.get(p.checked_add(16)?..p.checked_add(20)?));
        return Some(if codec == Some(b"alac") { FileFormat::Alac } else { FileFormat::Aac });
    }
    if let Some(len) = id3v2_len(b) {
        if depth == 0
            && let Some(f) = b.get(len..).and_then(|rest| detect_by_magic(rest, 1))
        {
            return Some(f);
        }
        return Some(FileFormat::Mp3);
    }
    let (b0, b1) = (*b.first()?, *b.get(1)?);
    if b0 == 0xFF && b1 & 0xE0 == 0xE0 {
        let layer = (b1 >> 1) & 0x3;
        if b1 & 0xF0 == 0xF0 && layer == 0 {
            return Some(FileFormat::Aac); // ADTS
        }
        if layer != 0 {
            return Some(FileFormat::Mp3);
        }
    }
    None
}

fn detect_by_ext(ext: &str) -> FileFormat {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "wav" | "wave" | "bwf" | "rf64" => FileFormat::Wav,
        "aif" | "aiff" | "aifc" => FileFormat::Aiff,
        "flac" => FileFormat::Flac,
        "mp3" | "mp2" | "mp1" => FileFormat::Mp3,
        "ogg" | "oga" => FileFormat::Ogg,
        "aac" | "m4a" | "m4b" | "mp4" | "adts" => FileFormat::Aac,
        "caf" => FileFormat::Caf,
        _ => FileFormat::Other,
    }
}

/// Identify a file by its magic bytes, falling back to the extension hint (`"wav"`, `".flac"`).
pub fn detect_format(bytes: &[u8], ext_hint: Option<&str>) -> FileFormat {
    detect_by_magic(bytes, 0).unwrap_or_else(|| ext_hint.map(detect_by_ext).unwrap_or(FileFormat::Other))
}

/// Read a file's format, sample rate, channel count, length and BWF metadata.
pub fn probe(bytes: &[u8], ext_hint: Option<&str>) -> Result<AudioInfo> {
    let format = detect_format(bytes, ext_hint);
    match format {
        FileFormat::Wav if wav::is_wav(bytes) => match wav::parse(bytes)? {
            wav::WavParse::Native(l) => Ok(l.info),
            wav::WavParse::Foreign { bwf } => {
                let mut info = symph::probe(bytes, format)?;
                info.bwf = bwf;
                Ok(info)
            }
        },
        FileFormat::Aiff if aiff::is_aiff(bytes) => match aiff::parse(bytes)? {
            aiff::AiffParse::Native(l) => Ok(l.info),
            aiff::AiffParse::Foreign => symph::probe(bytes, format),
        },
        _ => native::probe(bytes, format).unwrap_or_else(|| symph::probe(bytes, format)),
    }
}

/// Decode a whole file to planar `f32`.
pub fn decode(bytes: &[u8], ext_hint: Option<&str>) -> Result<(AudioInfo, AudioBuffer)> {
    let format = detect_format(bytes, ext_hint);
    match format {
        FileFormat::Wav if wav::is_wav(bytes) => match wav::parse(bytes)? {
            wav::WavParse::Native(l) => {
                let buf = wav::decode(bytes, &l)?;
                let mut info = l.info;
                info.frames = buf.frames() as u64;
                Ok((info, buf))
            }
            wav::WavParse::Foreign { bwf } => {
                let (mut info, buf) = symph::decode(bytes, format)?;
                info.bwf = bwf;
                Ok((info, buf))
            }
        },
        FileFormat::Aiff if aiff::is_aiff(bytes) => match aiff::parse(bytes)? {
            aiff::AiffParse::Native(l) => {
                let buf = aiff::decode(bytes, &l)?;
                let mut info = l.info;
                info.frames = buf.frames() as u64;
                Ok((info, buf))
            }
            aiff::AiffParse::Foreign => symph::decode(bytes, format),
        },
        _ => native::decode(bytes, format, false).unwrap_or_else(|| symph::decode(bytes, format)),
    }
}

/// The WAVE_FORMAT_EXTENSIBLE speaker mask of a WAV file, if it has one.
pub fn wav_channel_mask(bytes: &[u8]) -> Option<u32> {
    wav::read_channel_mask(bytes)
}
