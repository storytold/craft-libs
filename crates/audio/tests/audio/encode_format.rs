use craft_audio::{EncodeFormat, encode_format_for};

#[test]
fn only_formats_the_encoder_writes_are_named() {
    assert_eq!(encode_format_for("wav"), Some(EncodeFormat::Wav));
    assert_eq!(encode_format_for("WAV"), Some(EncodeFormat::Wav));
    assert_eq!(encode_format_for(".aif"), Some(EncodeFormat::Aiff));
    assert_eq!(encode_format_for("aiff"), Some(EncodeFormat::Aiff));
    assert_eq!(encode_format_for("wave"), Some(EncodeFormat::Wav));
    assert_eq!(encode_format_for("BWF"), Some(EncodeFormat::Wav));
    assert_eq!(encode_format_for(".aifc"), Some(EncodeFormat::Aiff));
    assert_eq!(encode_format_for("flac"), Some(EncodeFormat::Flac));
    assert!(craft_audio::ENCODE_EXTENSIONS.contains(&"flac"));
    for name in craft_audio::ENCODE_EXTENSIONS {
        assert!(encode_format_for(name).is_some(), "{name}");
    }
    for other in ["mp3", "ogg", "m4a", "xyz", ""] {
        assert_eq!(encode_format_for(other), None, "{other}");
    }
}
