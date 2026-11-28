use craft_raster_codecs::{ChannelLayout, CodecError, Image, Limits, xpm};

fn decode(b: &[u8]) -> Result<Image, CodecError> {
    xpm::decode(b, &Limits::default())
}

const SAMPLE: &[u8] = br#"/* XPM */
static char *icon[] = {
"2 2 3 1",
". c #ff0000",
"  c None",
"+ c #00ff00",
".+",
" +"
};
"#;

#[test]
fn exact_x11_lookup_preserves_x11_values_without_css_overrides() {
    for (name, rgb) in [
        ("black", [0u8, 0, 0]),
        ("white", [255, 255, 255]),
        ("gray", [190, 190, 190]),
        ("maroon", [176, 48, 96]),
        ("purple", [160, 32, 240]),
        ("gray42", [107, 107, 107]),
        ("steelblue4", [54, 100, 139]),
        ("navyblue", [0, 0, 128]),
    ] {
        assert_eq!(xpm::x11_color_rgb(name), Some(rgb.map(|channel| f32::from(channel) / 255.0)), "{name}");
    }
}

#[test]
fn exact_x11_lookup_rejects_unnormalized_names_and_xpm_aliases() {
    for name in [
        "",
        "Gray",
        "ghost white",
        "ghost-white",
        "ghost_white",
        " gray",
        "gray ",
        "gray\0",
        "é",
        "ｇｒａｙ",
        "gray01",
        "gray101",
        "notacolor",
        "silver",
        "olive",
        "teal",
        "fuchsia",
        "aqua",
        "none",
        "transparent",
        "#ffffff",
        "rgb:ff/ff/ff",
    ] {
        assert_eq!(xpm::x11_color_rgb(name), None, "{name:?}");
    }
    assert_eq!(xpm::x11_color_rgb(&"a".repeat(64)), None, "unknown name at the byte limit");
    assert_eq!(xpm::x11_color_rgb(&"a".repeat(65)), None, "oversized name");
}

#[test]
fn xpm_palette_keeps_its_aliases_and_name_normalization() {
    let bytes = b"! XPM2\n7 1 7 1\na c silver\nb c olive\nc c teal\nd c fuchsia\ne c Ghost-White\nf c ghost_white\ng c None\nabcdefg\n";
    let image = decode(bytes).unwrap();
    assert_eq!(
        image.data(),
        &[192, 192, 192, 255, 128, 128, 0, 255, 0, 128, 128, 255, 255, 0, 255, 255, 248, 248, 255, 255, 248, 248, 255, 255, 0, 0, 0, 0]
    );
}

#[test]
fn xpm3_decodes_palette_transparency_and_spaces_in_keys() {
    assert!(xpm::looks_like_xpm(SAMPLE));
    let img = decode(SAMPLE).unwrap();
    assert_eq!(img.layout(), ChannelLayout::Rgba);
    assert_eq!(img.dimensions(), (2, 2));
    assert_eq!(img.data(), &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0, 255, 0, 255]);
}

#[test]
fn rgb_precisions_and_x11_name() {
    let xpm = br#"/* XPM */
static char *i[] = {
"3 1 3 1",
"a c #f00",
"b c #ffff00000000",
"c c gray70",
"abc"
};"#;
    let img = decode(xpm).unwrap();
    assert_eq!(img.data(), &[255, 0, 0, 255, 255, 0, 0, 255, 179, 179, 179, 255]);
}

#[test]
fn rejects_truncation_unsupported_colours_and_unknown_keys() {
    for bytes in [
        &b"/* XPM */\nstatic char *i[] = { \"1 1 1 1\", \"a c None\" };"[..],
        &b"/* XPM */\nstatic char *i[] = { \"1 1 1 1\", \"a c NotAColor\", \"a\" };"[..],
        &b"/* XPM */\nstatic char *i[] = { \"1 1 1 1\", \"a c #a\xC3\xA9\", \"a\" };"[..],
        &b"/* XPM */\nstatic char *i[] = { \"1 1 1 1\", \"a c #+f0000\", \"a\" };"[..],
        &b"/* XPM */\nstatic char *i[] = { \"1 1 1 1\", \"a c black\", \"z\" };"[..],
        &b"/* XPM */\nstatic char *i[] = { \"1 1 1 1\", \"a c black\", \"aa\" };"[..],
        &b"/* XPM */\nstatic char *i[] = { \"1 1 2 1\", \"a c black\", \"a c white\", \"a\" };"[..],
    ] {
        assert!(decode(bytes).is_err(), "{:?}", bytes);
    }
    assert!(decode(b"/* XPM */\n\"4294967295 4294967295 1 1\"").is_err());
    assert!(decode(b"/* XPM */\n\"2 1 1 1\" \"a c black\" \"aa\" /* bad").is_err());
}

#[test]
fn x11_names_and_field_priority() {
    let xpm = br#"/* XPM */
static char *i[] = {
"4 1 4 1",
"a c LightGray",
"b c DarkBlue",
"c m black g white c light blue",
"d m black g4 red g green",
"abcd"
};"#;
    let image = decode(xpm).unwrap();
    assert_eq!(image.data(), &[211, 211, 211, 255, 0, 0, 139, 255, 173, 216, 230, 255, 0, 255, 0, 255]);
}

#[test]
fn named_x11_colors_use_x11_not_css_values() {
    let bytes = br#"/* XPM */
static char *icon[] = {
"6 1 6 1",
"a c gray",
"b c maroon",
"c c purple",
"d c LightGoldenrodYellow",
"e c DarkSlateGray4",
"f c Ghost White",
"abcdef"
};"#;
    let image = decode(bytes).unwrap();
    assert_eq!(image.data(), &[190, 190, 190, 255, 176, 48, 96, 255, 160, 32, 240, 255, 250, 250, 210, 255, 82, 139, 139, 255, 248, 248, 255, 255,]);
}

#[test]
fn xpm2_decodes_crlf_comments_spaces_and_multichar_keys() {
    let bytes = b"! XPM2\r\n! exported by an icon tool\r\n2 2 3 2\r\n.a c #f00\r\n!_ c None\r\nzz c DarkBlue\r\n.a!_\r\nzz.a\r\n";
    assert!(xpm::looks_like_xpm(bytes));
    let image = decode(bytes).unwrap();
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(image.data(), &[255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 139, 255, 255, 0, 0, 255]);
}

#[test]
fn xpm2_allows_exclamation_as_a_pixel_key_and_preserves_space_keys() {
    let bytes = b"! XPM2\n2 1 2 1\n! c red\n  c None\n! \n";
    let image = decode(bytes).unwrap();
    assert_eq!(image.data(), &[255, 0, 0, 255, 0, 0, 0, 0]);
}

#[test]
fn xpm2_malformed_inputs_return_errors() {
    for bytes in [
        &b"! XPM2\n1 1 1 1\na c black\n"[..],
        &b"! XPM2\n1 1 1 1\na c black\nb\n"[..],
        &b"! XPM2\n1 1 1 1\na c #a\xc3\xa9\na\n"[..],
        &b"! XPM2 invalid\n1 1 1 1\na c black\na\n"[..],
        &b"! XPM2\n100000 100000 1 1\na c red\na\n"[..],
    ] {
        assert!(decode(bytes).is_err(), "{bytes:?}");
    }
    assert!(!xpm::looks_like_xpm(b"! XPM23\n1 1 1 1\na c black\na\n"));
}

#[test]
fn xpm_rows_shorter_than_the_header_declares_are_refused_before_allocating() {
    let b = b"/* XPM */\nstatic char *x[] = {\n\"30000 30000 1 1\",\n\". c #000000\",\n\".\",\n};\n".to_vec();
    match decode(&b) {
        Err(CodecError::Malformed { .. }) => {}
        other => panic!("expected a malformed-row error, got {:?}", other.map(|i| (i.width(), i.height()))),
    }
}
