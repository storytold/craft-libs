use craft_audio::{EncodeFormat, encode_format_for};

#[test]
fn only_formats_the_encoder_writes_are_named() {
    assert_eq!(encode_format_for("wav"), Some(EncodeFormat::Wav));
    assert_eq!(encode_format_for("WAV"), Some(EncodeFormat::Wav));
    assert_eq!(encode_format_for(".aif"), Some(EncodeFormat::Aiff));
    assert_eq!(encode_format_for("aiff"), Some(EncodeFormat::Aiff));
    for other in ["flac", "mp3", "ogg", "m4a", "xyz", ""] {
        assert_eq!(encode_format_for(other), None, "{other}");
    }
}
