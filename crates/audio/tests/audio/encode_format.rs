use craft_audio::{EncodeFormat, encode_format_for};

#[test]
fn ogg_names_follow_the_feature() {
    let built = cfg!(feature = "vorbis-encode");
    assert_eq!(EncodeFormat::Ogg.is_built(), built);
    assert_eq!(encode_format_for("ogg"), built.then_some(EncodeFormat::Ogg));
    assert_eq!(craft_audio::ENCODE_EXTENSIONS.contains(&"ogg"), built);
}
