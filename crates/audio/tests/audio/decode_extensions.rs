//! Every extension `DECODE_EXTENSIONS` names is a hint `detect_container` knows, and a file of
//! that kind probes and decodes.

use craft_audio::{AudioBuffer, BitDepth, DECODE_EXTENSIONS, EncodeFormat, EncodeOptions, decode, detect_container, encode, probe};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn generated(opts: EncodeOptions) -> Vec<u8> {
    let tone: Vec<f32> = (0..800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
    encode(&AudioBuffer { sample_rate: 8000, channels: vec![tone] }, &opts.with_bit_depth(BitDepth::Int16)).unwrap()
}

fn bytes_for(ext: &str) -> Vec<u8> {
    match ext {
        "wav" | "wave" | "bwf" => generated(EncodeOptions::new(EncodeFormat::Wav)),
        "rf64" => generated(EncodeOptions::new(EncodeFormat::Wav).with_force_rf64(true)),
        "aif" | "aiff" => generated(EncodeOptions::new(EncodeFormat::Aiff)),
        "aifc" => fixture("tone.aifc"),
        "flac" => generated(EncodeOptions::new(EncodeFormat::Flac)),
        "mp3" => fixture("tone.mp3"),
        "mp2" | "mpa" => fixture("tone.mp2"),
        "mp1" => fixture("silence.mp1"),
        "ogg" | "oga" => fixture("tone.ogg"),
        "opus" => fixture("tone.opus"),
        "m4a" | "m4b" | "mp4" => fixture("tone.m4a"),
        "mov" => fixture("tone.mov"),
        "aac" | "adts" => fixture("tone.aac"),
        "caf" => fixture("tone.caf"),
        "mkv" => fixture("tone.mkv"),
        "mka" => fixture("tone.mka"),
        "webm" => fixture("tone.webm"),
        "ac3" => fixture("tone.ac3"),
        "eac3" | "ec3" => fixture("tone.eac3"),
        other => panic!("no fixture for the extension {other:?}: add one"),
    }
}

#[test]
fn every_decode_extension_probes_and_decodes() {
    for ext in DECODE_EXTENSIONS {
        // No magic bytes here: the hint alone names the container.
        assert!(detect_container(&[0; 16], Some(ext)).is_some(), ".{ext} is not a known hint");
        assert_eq!(ext.to_ascii_lowercase(), *ext, "extensions are listed in lower case");
        let bytes = bytes_for(ext);
        let info = probe(&bytes, Some(ext)).unwrap_or_else(|e| panic!("probe .{ext}: {e}"));
        let (_, buf) = decode(&bytes, Some(ext)).unwrap_or_else(|e| panic!("decode .{ext}: {e}"));
        assert!(info.channels >= 1, ".{ext}");
        assert!(buf.frames() > 0, ".{ext}: no frames");
    }
    for known in ["wav", "aif", "flac", "mp3", "ogg", "m4a", "caf", "mkv", "webm", "opus", "ac3"] {
        assert!(DECODE_EXTENSIONS.contains(&known), "{known}");
    }
    for other in ["mid", "txt", "wma", ""] {
        assert!(!DECODE_EXTENSIONS.contains(&other), "{other}");
    }
}

#[test]
fn every_encode_format_has_an_extension_it_is_written_and_read_under() {
    for &format in EncodeFormat::ALL {
        let ext = format.extension();
        assert_eq!(craft_audio::encode_format_for(ext), Some(format), "{ext}");
        assert!(craft_audio::ENCODE_EXTENSIONS.contains(&ext), "{ext}");
        assert!(DECODE_EXTENSIONS.contains(&ext), "{ext}: what is written reads back");
        assert!(format.mime_type().starts_with("audio/"), "{format:?}");
    }
    assert_eq!(EncodeFormat::ALL.iter().map(|f| f.extension()).collect::<Vec<_>>(), ["wav", "aif", "flac"]);
    assert_eq!(EncodeFormat::Aiff.mime_type(), "audio/aiff");
}
