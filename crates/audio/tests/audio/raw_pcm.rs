//! Integer PCM in and out of WAV and AIFF files bit for bit (`wav_pcm`, `aiff_pcm`, `pcm_*`).

use craft_audio::{
    AudioBuffer, BitDepth, EncodeFormat, EncodeOptions, RawPcm, aiff_pcm, encode, pcm_aiff_file, pcm_aiff_header, pcm_wav_file, pcm_wav_header,
    wav_pcm,
};

fn raw(data: &[u8], channels: u16, sample_rate: u32, bits: u16) -> Option<RawPcm> {
    Some(RawPcm { data: data.to_vec(), channels, sample_rate, bits })
}

#[test]
fn wav_and_aiff_round_trip_bit_for_bit() {
    let pcm: Vec<u8> = (0..48u8).collect(); // 8 stereo 24-bit frames
    let w = pcm_wav_file(&pcm, 2, 48_000, 24);
    assert_eq!(wav_pcm(&w), raw(&pcm, 2, 48_000, 24));
    let a = pcm_aiff_file(&pcm, 2, 44_100, 24);
    assert_eq!(aiff_pcm(&a), raw(&pcm, 2, 44_100, 24));
    let p16: Vec<u8> = (0..32u8).collect();
    assert_eq!(aiff_pcm(&pcm_aiff_file(&p16, 1, 96_000, 16)), raw(&p16, 1, 96_000, 16));
    // 8-bit: unsigned in WAVE and in `RawPcm`, signed in AIFF
    let p8 = [0u8, 0x7F, 0x80, 0xFF, 3];
    assert_eq!(wav_pcm(&pcm_wav_file(&p8, 1, 8_000, 8)), raw(&p8, 1, 8_000, 8));
    let a8 = pcm_aiff_file(&p8, 1, 8_000, 8);
    assert_eq!(a8.get(54..59), Some(&[0x80, 0xFF, 0x00, 0x7F, 0x83][..]));
    assert_eq!(a8.len(), 60, "odd sound data is padded");
    assert_eq!(aiff_pcm(&a8), raw(&p8, 1, 8_000, 8));
    // neither reads the other's container
    assert_eq!(wav_pcm(&a), None);
    assert_eq!(aiff_pcm(&w), None);
}

/// A descriptor's summary is the header alone: the format is read, with no samples.
#[test]
fn headers_without_data_give_the_format() {
    assert_eq!(wav_pcm(&pcm_wav_header(4800, 2, 48_000, 16)), raw(&[], 2, 48_000, 16));
    assert_eq!(aiff_pcm(&pcm_aiff_header(1200, 2, 48_000, 16)), raw(&[], 2, 48_000, 16));
}

#[test]
fn files_cut_short_never_panic_and_give_what_is_there() {
    let pcm: Vec<u8> = (0..48u8).collect();
    let w = pcm_wav_file(&pcm, 2, 48_000, 24);
    let a = pcm_aiff_file(&pcm, 2, 44_100, 24);
    for cut in 0..w.len().max(a.len()) {
        if let Some(p) = w.get(..cut).and_then(wav_pcm) {
            assert_eq!(Some(&p.data[..]), pcm.get(..cut.saturating_sub(44)), "wav cut at {cut}");
        }
        if let Some(p) = a.get(..cut).and_then(aiff_pcm) {
            assert!(pcm.starts_with(&p.data) && p.data.len() <= cut.saturating_sub(54), "aiff cut at {cut}");
        }
    }
    assert_eq!(wav_pcm(b"RIFF"), None);
    assert_eq!(aiff_pcm(a.get(..30).unwrap()), None);
}

#[test]
fn only_integer_pcm_is_taken() {
    let buf = AudioBuffer::new(48_000, 2, 16);
    let float = encode(&buf, &EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Float32)).unwrap();
    assert_eq!(wav_pcm(&float), None);
    let float = encode(&buf, &EncodeOptions::new(EncodeFormat::Aiff).with_bit_depth(BitDepth::Float32)).unwrap();
    assert_eq!(aiff_pcm(&float), None);
    // the full writer's 24-bit WAV (extensible `fmt `, a JUNK chunk before it) is integer PCM too
    let int = encode(&buf, &EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24)).unwrap();
    assert_eq!(wav_pcm(&int), raw(&[0; 96], 2, 48_000, 24));
    let int = encode(&buf, &EncodeOptions::new(EncodeFormat::Aiff).with_bit_depth(BitDepth::Int16)).unwrap();
    assert_eq!(aiff_pcm(&int), raw(&[0; 64], 2, 48_000, 16));
    // zero channels or a zero rate is not a format
    let mut bad = pcm_wav_file(&[0; 8], 1, 48_000, 16);
    bad[24..28].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(wav_pcm(&bad), None);
}
