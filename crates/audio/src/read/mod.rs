//! Format detection, probing and decoding: native WAV and AIFF readers, `symphonia` for the rest.

mod aiff;
mod caf;
mod flac;
mod mp4;
mod native;
mod pcm;
mod symph;
mod wav;

use crate::{AudioBuffer, AudioInfo, Container, Result};

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

fn detect_by_magic(b: &[u8], depth: u32) -> Option<Container> {
    let head = |n: usize| b.get(..n);
    if wav::is_wav(b) {
        return Some(Container::Wav);
    }
    if aiff::is_aiff(b) {
        return Some(Container::Aiff);
    }
    match head(4)? {
        b"fLaC" => return Some(Container::Flac),
        b"OggS" => return Some(Container::Ogg),
        b"caff" => return Some(Container::Caf),
        // Matroska and WebM differ only in the EBML header's DocType, within its first bytes.
        [0x1A, 0x45, 0xDF, 0xA3] => {
            let header = b.get(..64.min(b.len())).unwrap_or_default();
            return Some(if header.windows(4).any(|w| w == b"webm") { Container::WebM } else { Container::Matroska });
        }
        _ => {}
    }
    if b.get(4..8) == Some(b"ftyp") {
        return Some(if b.get(8..12) == Some(b"qt  ") { Container::QuickTime } else { Container::Mp4 });
    }
    if let Some(len) = id3v2_len(b) {
        if depth == 0
            && let Some(f) = b.get(len..).and_then(|rest| detect_by_magic(rest, 1))
        {
            return Some(f);
        }
        return Some(Container::MpegAudio);
    }
    let (b0, b1) = (*b.first()?, *b.get(1)?);
    if b0 == 0x0B && b1 == 0x77 {
        return Some(Container::Ac3);
    }
    if b0 == 0xFF && b1 & 0xE0 == 0xE0 {
        let layer = (b1 >> 1) & 0x3;
        if b1 & 0xF0 == 0xF0 && layer == 0 {
            return Some(Container::Adts);
        }
        if layer != 0 {
            return Some(Container::MpegAudio);
        }
    }
    None
}

fn detect_by_ext(ext: &str) -> Option<Container> {
    Some(match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "wav" | "wave" | "bwf" | "rf64" => Container::Wav,
        "aif" | "aiff" | "aifc" => Container::Aiff,
        "flac" => Container::Flac,
        "mp3" | "mp2" | "mp1" | "mpa" => Container::MpegAudio,
        "ogg" | "oga" | "opus" => Container::Ogg,
        "aac" | "adts" => Container::Adts,
        "m4a" | "m4b" | "mp4" => Container::Mp4,
        "mov" => Container::QuickTime,
        "mkv" | "mka" => Container::Matroska,
        "webm" => Container::WebM,
        "ac3" | "eac3" | "ec3" => Container::Ac3,
        "caf" => Container::Caf,
        _ => return None,
    })
}

/// The container of a file, by its magic bytes, falling back to the extension hint (`"wav"`,
/// `".flac"`); `None` when neither says.
pub fn detect_container(bytes: &[u8], ext_hint: Option<&str>) -> Option<Container> {
    detect_by_magic(bytes, 0).or_else(|| ext_hint.and_then(detect_by_ext))
}

/// Read a file's container, codec, sample rate, channel layout, length and BWF metadata, without
/// decoding the audio where the headers say enough.
pub fn probe(bytes: &[u8], ext_hint: Option<&str>) -> Result<AudioInfo> {
    let container = detect_container(bytes, ext_hint);
    match container {
        Some(Container::Wav) if wav::is_wav(bytes) => match wav::parse(bytes)? {
            wav::WavParse::Native(l) => Ok(l.info),
            wav::WavParse::Foreign { bwf } => {
                let mut info = symph::probe(bytes, container)?;
                info.bwf = bwf;
                Ok(info)
            }
        },
        Some(Container::Aiff) if aiff::is_aiff(bytes) => match aiff::parse(bytes)? {
            aiff::AiffParse::Native(l) => Ok(l.info),
            aiff::AiffParse::Foreign => symph::probe(bytes, container),
        },
        _ => native::probe(bytes, container).unwrap_or_else(|| symph::probe(bytes, container)),
    }
}

/// Decode the first audio track of a file to planar `f32`, trimmed to its presentation length, in
/// WAV channel order when [`AudioInfo::layout`] is ordered.
pub fn decode(bytes: &[u8], ext_hint: Option<&str>) -> Result<(AudioInfo, AudioBuffer)> {
    let container = detect_container(bytes, ext_hint);
    match container {
        Some(Container::Wav) if wav::is_wav(bytes) => match wav::parse(bytes)? {
            wav::WavParse::Native(l) => {
                let buf = wav::decode(bytes, &l)?;
                let mut info = l.info;
                info.frames = buf.frames() as u64;
                Ok((info, buf))
            }
            wav::WavParse::Foreign { bwf } => {
                let (mut info, buf) = symph::decode(bytes, container)?;
                info.bwf = bwf;
                Ok((info, buf))
            }
        },
        Some(Container::Aiff) if aiff::is_aiff(bytes) => match aiff::parse(bytes)? {
            aiff::AiffParse::Native(l) => {
                let buf = aiff::decode(bytes, &l)?;
                let mut info = l.info;
                info.frames = buf.frames() as u64;
                Ok((info, buf))
            }
            aiff::AiffParse::Foreign => symph::decode(bytes, container),
        },
        _ => native::decode(bytes, container, false).unwrap_or_else(|| symph::decode(bytes, container)),
    }
}
