//! Matroska CodecID → codec mapping (Matroska codec specifications, RFC 9559 §5.1.4.1.21 and the
//! codec mappings registry).

/// Codec of a track, derived from `CodecID` and `CodecPrivate`.
///
/// Configuration records are passed through as the raw bytes found in `CodecPrivate`, which for
/// the ISO codecs are exactly the ISOBMFF configuration boxes' payloads (avcC, hvcC, av1C, ASC…).
#[derive(Clone, Debug, PartialEq)]
pub enum Codec {
    /// `V_MPEG4/ISO/AVC`; payload is an `AVCDecoderConfigurationRecord` (avcC).
    Avc { avcc: Vec<u8> },
    /// `V_MPEGH/ISO/HEVC`; payload is an `HEVCDecoderConfigurationRecord` (hvcC).
    Hevc { hvcc: Vec<u8> },
    /// `V_VP8`.
    Vp8,
    /// `V_VP9`; `CodecPrivate` (optional) holds VP9 codec feature metadata.
    Vp9 { private: Vec<u8> },
    /// `V_AV1`; payload is an `AV1CodecConfigurationRecord` (av1C).
    Av1 { av1c: Vec<u8> },
    /// `V_APV`; payload is an `APVDecoderConfigurationRecord` (apvC), if present.
    Apv { apvc: Vec<u8> },
    /// `V_PRORES`; `fourcc` from `CodecPrivate` (e.g. `apcn`), if present.
    ProRes { fourcc: Option<[u8; 4]> },
    /// `V_MJPEG`, or `V_MS/VFW/FOURCC` with an `MJPG` compression FourCC.
    Mjpeg,
    /// `V_MS/VFW/FOURCC` with any other FourCC; `bitmap_info_header` is the whole `CodecPrivate`.
    VfwFourcc { fourcc: [u8; 4], bitmap_info_header: Vec<u8> },
    /// `A_AAC` (and the legacy `A_AAC/MPEG{2,4}/<profile>` IDs); `asc` is the
    /// `AudioSpecificConfig` (synthesized for legacy IDs without `CodecPrivate`).
    Aac { asc: Vec<u8> },
    /// `A_OPUS`; `head` is the `OpusHead` identification header.
    Opus { head: Vec<u8> },
    /// `A_VORBIS`; the three Vorbis headers (identification, comment, setup), split from the
    /// Xiph-laced `CodecPrivate`.
    Vorbis { headers: Vec<Vec<u8>> },
    /// `A_FLAC`; `private` is `fLaC` + metadata blocks (STREAMINFO first).
    Flac { private: Vec<u8> },
    /// `A_PCM/INT/LIT`, `A_PCM/INT/BIG`, `A_PCM/FLOAT/IEEE`. Bit depth comes from the Audio element.
    Pcm { float: bool, big_endian: bool, bits: u16 },
    /// `A_AC3`.
    Ac3,
    /// `A_EAC3`.
    Eac3,
    /// `A_MPEG/L3`.
    Mp3,
    /// `A_MPEG/L2`.
    Mp2,
    /// `A_MPEG/L1`.
    Mp1,
    /// `S_TEXT/UTF8` (SubRip-style plain text).
    SubRip,
    /// `S_TEXT/WEBVTT`, or WebM's `D_WEBVTT/{SUBTITLES,CAPTIONS,DESCRIPTIONS,METADATA}`.
    WebVtt,
    /// `S_TEXT/ASS` / `S_TEXT/SSA`; `header` is the script header from `CodecPrivate`.
    Ass { header: Vec<u8>, ssa: bool },
    /// Any other CodecID (kept verbatim along with `Track::codec_private`).
    Other(String),
}

impl Codec {
    /// Short codec name, matching FFmpeg's `codec_name` where one exists (useful for logs/UI).
    pub fn name(&self) -> String {
        match self {
            Codec::Avc { .. } => "h264".into(),
            Codec::Hevc { .. } => "hevc".into(),
            Codec::Vp8 => "vp8".into(),
            Codec::Vp9 { .. } => "vp9".into(),
            Codec::Av1 { .. } => "av1".into(),
            Codec::Apv { .. } => "apv".into(),
            Codec::ProRes { .. } => "prores".into(),
            Codec::Mjpeg => "mjpeg".into(),
            Codec::VfwFourcc { fourcc, .. } => String::from_utf8_lossy(fourcc).trim().to_ascii_lowercase(),
            Codec::Aac { .. } => "aac".into(),
            Codec::Opus { .. } => "opus".into(),
            Codec::Vorbis { .. } => "vorbis".into(),
            Codec::Flac { .. } => "flac".into(),
            Codec::Pcm { float, big_endian, bits } => {
                let k = if *float {
                    'f'
                } else if *bits == 8 {
                    'u'
                } else {
                    's'
                };
                if *bits == 8 { format!("pcm_{k}8") } else { format!("pcm_{k}{bits}{}", if *big_endian { "be" } else { "le" }) }
            }
            Codec::Ac3 => "ac3".into(),
            Codec::Eac3 => "eac3".into(),
            Codec::Mp3 => "mp3".into(),
            Codec::Mp2 => "mp2".into(),
            Codec::Mp1 => "mp1".into(),
            Codec::SubRip => "subrip".into(),
            Codec::WebVtt => "webvtt".into(),
            Codec::Ass { ssa, .. } => if *ssa { "ssa" } else { "ass" }.into(),
            Codec::Other(id) => id.clone(),
        }
    }
}

const AAC_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

/// Build a 2-byte (or 5-byte with SBR extension) AudioSpecificConfig for legacy `A_AAC/...` IDs.
fn synth_asc(object_type: u8, rate: f64, out_rate: Option<f64>, channels: u64, sbr: bool) -> Vec<u8> {
    let rate_index = |r: f64| AAC_RATES.iter().position(|&x| x as f64 == r).unwrap_or(4) as u8;
    let ri = rate_index(rate);
    let ch = channels.min(7) as u8;
    let mut v = vec![(object_type << 3) | (ri >> 1), ((ri & 1) << 7) | (ch << 3)];
    if sbr {
        // backward-compatible explicit signaling: sync 0x2b7, SBR object type 5, flag, ext rate index
        let ext = rate_index(out_rate.unwrap_or(rate * 2.0));
        v.push(0x56);
        v.push(0xE5);
        v.push(0x80 | (ext << 3));
    }
    v
}

/// Split a Xiph-laced `CodecPrivate` (Vorbis/Theora headers).
fn split_xiph(p: &[u8]) -> Vec<Vec<u8>> {
    let Some(&n) = p.first() else { return Vec::new() };
    let mut pos = 1;
    let mut sizes = Vec::new();
    for _ in 0..n {
        let mut s = 0usize;
        loop {
            let Some(&b) = p.get(pos) else { return Vec::new() };
            pos += 1;
            s += b as usize;
            if b != 255 {
                break;
            }
        }
        sizes.push(s);
    }
    let mut out = Vec::new();
    for s in sizes {
        let Some(h) = p.get(pos..pos + s) else { return Vec::new() };
        out.push(h.to_vec());
        pos += s;
    }
    out.push(p[pos.min(p.len())..].to_vec());
    out
}

/// Map a CodecID + CodecPrivate (+ audio params for legacy AAC / PCM) to a [`Codec`].
pub(crate) fn map_codec(id: &str, private: &[u8], audio: Option<&crate::matroska::AudioInfo>) -> Codec {
    let p = private.to_vec();
    match id {
        "V_MPEG4/ISO/AVC" => Codec::Avc { avcc: p },
        "V_MPEGH/ISO/HEVC" => Codec::Hevc { hvcc: p },
        "V_VP8" => Codec::Vp8,
        "V_VP9" => Codec::Vp9 { private: p },
        "V_AV1" => Codec::Av1 { av1c: p },
        "V_APV" => Codec::Apv { apvc: p },
        "V_PRORES" => Codec::ProRes { fourcc: private.get(..4).and_then(|f| f.try_into().ok()) },
        "V_MJPEG" => Codec::Mjpeg,
        "V_MS/VFW/FOURCC" => {
            let fourcc: [u8; 4] = private.get(16..20).and_then(|f| f.try_into().ok()).unwrap_or(*b"    ");
            match &fourcc {
                b"MJPG" | b"mjpg" | b"AVRn" | b"AVDJ" => Codec::Mjpeg,
                b"H264" | b"h264" | b"avc1" | b"X264" | b"x264" => Codec::Avc { avcc: Vec::new() },
                b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => Codec::ProRes { fourcc: Some(fourcc) },
                b"apv1" | b"APV1" => Codec::Apv { apvc: Vec::new() },
                _ => Codec::VfwFourcc { fourcc, bitmap_info_header: p },
            }
        }
        "A_AAC" => Codec::Aac { asc: p },
        "A_OPUS" => Codec::Opus { head: p },
        "A_VORBIS" => Codec::Vorbis { headers: split_xiph(private) },
        "A_FLAC" => Codec::Flac { private: p },
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => {
            let bits = audio.and_then(|a| a.bit_depth).unwrap_or(if id == "A_PCM/FLOAT/IEEE" { 32 } else { 16 }) as u16;
            Codec::Pcm { float: id == "A_PCM/FLOAT/IEEE", big_endian: id == "A_PCM/INT/BIG", bits }
        }
        "A_AC3" | "A_AC3/BSID9" | "A_AC3/BSID10" => Codec::Ac3,
        "A_EAC3" => Codec::Eac3,
        "A_MPEG/L3" => Codec::Mp3,
        "A_MPEG/L2" => Codec::Mp2,
        "A_MPEG/L1" => Codec::Mp1,
        "S_TEXT/UTF8" => Codec::SubRip,
        "S_TEXT/WEBVTT" | "D_WEBVTT/SUBTITLES" | "D_WEBVTT/CAPTIONS" | "D_WEBVTT/DESCRIPTIONS" | "D_WEBVTT/METADATA" => Codec::WebVtt,
        "S_TEXT/ASS" | "S_ASS" => Codec::Ass { header: p, ssa: false },
        "S_TEXT/SSA" | "S_SSA" => Codec::Ass { header: p, ssa: true },
        _ if id.starts_with("A_AAC/") => {
            if !private.is_empty() {
                return Codec::Aac { asc: p };
            }
            let (rate, out_rate, ch) = audio.map_or((48000.0, None, 2), |a| (a.sampling_frequency, a.output_sampling_frequency, a.channels));
            let object_type = if id.ends_with("/MAIN") {
                1
            } else if id.ends_with("/SSR") {
                3
            } else if id.ends_with("/LTP") {
                4
            } else {
                2
            };
            let sbr = id.ends_with("/SBR");
            Codec::Aac { asc: synth_asc(object_type, rate, out_rate, ch, sbr) }
        }
        _ => Codec::Other(id.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(map_codec("V_MPEG4/ISO/AVC", &[1, 2], None), Codec::Avc { avcc: vec![1, 2] });
        assert_eq!(map_codec("V_APV", &[1, 2, 3], None), Codec::Apv { apvc: vec![1, 2, 3] });
        assert_eq!(map_codec("V_APV", &[], None).name(), "apv");
        assert_eq!(map_codec("V_PRORES", b"apcn", None), Codec::ProRes { fourcc: Some(*b"apcn") });
        let mut bih = vec![0u8; 40];
        bih[16..20].copy_from_slice(b"MJPG");
        assert_eq!(map_codec("V_MS/VFW/FOURCC", &bih, None), Codec::Mjpeg);
        bih[16..20].copy_from_slice(b"FFV1");
        assert!(matches!(map_codec("V_MS/VFW/FOURCC", &bih, None), Codec::VfwFourcc { fourcc, .. } if &fourcc == b"FFV1"));
        let a = crate::matroska::AudioInfo { sampling_frequency: 44100.0, channels: 2, bit_depth: Some(24), output_sampling_frequency: None };
        assert_eq!(map_codec("A_PCM/INT/LIT", &[], Some(&a)).name(), "pcm_s24le");
        assert_eq!(map_codec("A_PCM/FLOAT/IEEE", &[], Some(&a)), Codec::Pcm { float: true, big_endian: false, bits: 24 });
        // legacy AAC LC 44.1k stereo → 0x12 0x10
        assert_eq!(map_codec("A_AAC/MPEG4/LC", &[], Some(&a)), Codec::Aac { asc: vec![0x12, 0x10] });
        assert_eq!(map_codec("S_TEXT/UTF8", &[], None).name(), "subrip");
        assert_eq!(map_codec("X_WHATEVER", &[], None), Codec::Other("X_WHATEVER".into()));
    }

    #[test]
    fn xiph_split() {
        let mut p = vec![2u8, 3, 2];
        p.extend_from_slice(&[1, 1, 1, 2, 2, 3, 3, 3, 3]);
        let h = split_xiph(&p);
        assert_eq!(h, vec![vec![1, 1, 1], vec![2, 2], vec![3, 3, 3, 3]]);
    }
}
