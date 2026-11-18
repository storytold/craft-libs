//! Compressed rich text ([MS-OXRTFCP] section 3 samples) and the text of RTF.

use proptest::prelude::*;

use super::{Error, MAX_RTF_DEPTH, crc32, decompress_rtf, rtf_html, rtf_text};

/// [MS-OXRTFCP] 3.1.1: a simple compressed RTF.
const EXAMPLE_1: [u8; 49] = [
    0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70, 0x67,
    0x31, 0x32, 0x35, 0x42, 0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64, 0x7d, 0x0a, 0x80, 0x0f,
    0xa0,
];

/// [MS-OXRTFCP] 3.1.2: a reference that reads past the write position.
const EXAMPLE_2: [u8; 30] = [
    0x1a, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xe2, 0xd4, 0x4b, 0x51, 0x41, 0x00, 0x04, 0x20, 0x57, 0x58, 0x59, 0x5a,
    0x0d, 0x6e, 0x7d, 0x01, 0x0e, 0xb0,
];

fn mela(rtf: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u32::try_from(rtf.len() + 12).unwrap().to_le_bytes());
    v.extend_from_slice(&u32::try_from(rtf.len()).unwrap().to_le_bytes());
    v.extend_from_slice(b"MELA");
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(rtf);
    v
}

#[test]
fn spec_example_1_decompresses() {
    let out = decompress_rtf(&EXAMPLE_1, 1 << 20).unwrap();
    assert_eq!(out, b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n");
    assert_eq!(crc32(&EXAMPLE_1[16..]), 0xa7c7_c5f1);
    assert_eq!(rtf_text(&out, 1000).unwrap(), ("hello world".to_string(), false));
}

#[test]
fn spec_example_2_reads_past_the_write_position() {
    let out = decompress_rtf(&EXAMPLE_2, 1 << 20).unwrap();
    assert_eq!(out, b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}");
    assert_eq!(out.len(), 0x1c);
}

#[test]
fn uncompressed_is_taken_as_it_is() {
    let rtf = b"{\\rtf1 plain}";
    assert_eq!(decompress_rtf(&mela(rtf), 100).unwrap(), rtf);
    // A header that lies about its size is held to what is there.
    let mut lying = mela(rtf);
    lying.get_mut(4..8).unwrap().copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(decompress_rtf(&lying, 100).unwrap(), rtf);
    assert!(matches!(decompress_rtf(&mela(rtf), 3), Err(Error::TooLarge)));
}

#[test]
fn a_bad_crc_is_an_error() {
    let mut bad = EXAMPLE_1;
    bad[12] ^= 1;
    assert!(matches!(decompress_rtf(&bad, 1 << 20), Err(Error::BadChecksum)));
    let mut bad = EXAMPLE_1;
    bad[20] ^= 0x40;
    assert!(decompress_rtf(&bad, 1 << 20).is_err());
}

#[test]
fn truncation_is_an_error_at_every_length() {
    for n in 0..EXAMPLE_1.len() {
        assert!(decompress_rtf(&EXAMPLE_1[..n], 1 << 20).is_err(), "{n}");
    }
    // A header whose size is right but whose runs have no end mark.
    let mut v = EXAMPLE_1[..40].to_vec();
    v[..4].copy_from_slice(&28u32.to_le_bytes());
    assert!(decompress_rtf(&v, 1 << 20).is_err());
}

#[test]
fn a_header_that_lies_about_the_size_costs_nothing() {
    let mut huge = EXAMPLE_1;
    huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(decompress_rtf(&huge, 1 << 20), Err(Error::TooLarge)));
    // Under the cap it is the stream that decides.
    let mut small = EXAMPLE_1;
    small[4..8].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(decompress_rtf(&small, 1 << 20).unwrap().len(), 0x2b);
}

#[test]
fn output_is_capped_while_it_is_made() {
    assert!(matches!(decompress_rtf(&EXAMPLE_1, 10), Err(Error::TooLarge)));
    // A long stream (no end mark) is stopped at the cap, not at its end.
    let mut body = Vec::new();
    for _ in 0..200_000 {
        body.push(0x00);
        body.extend_from_slice(b"abcdefgh");
    }
    let mut v = Vec::new();
    v.extend_from_slice(&u32::try_from(body.len() + 12).unwrap().to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(b"LZFu");
    v.extend_from_slice(&crc32(&body).to_le_bytes());
    v.extend_from_slice(&body);
    assert!(matches!(decompress_rtf(&v, 1 << 16), Err(Error::TooLarge)));
}

#[test]
fn an_unknown_kind_is_an_error() {
    let mut v = EXAMPLE_1;
    v[8..12].copy_from_slice(b"XXXX");
    assert!(matches!(decompress_rtf(&v, 100), Err(Error::Corrupt(_))));
}

fn text(rtf: &str) -> String {
    rtf_text(rtf.as_bytes(), 1 << 20).unwrap().0
}

#[test]
fn paragraphs_and_tabs() {
    assert_eq!(text("{\\rtf1\\ansi\\deff0 One\\par Two\\line Three\\tab x\\par}"), "One\nTwo\nThree\tx");
    assert_eq!(text("{\\rtf1 a\\par\\par\\par\\par\\par b}"), "a\n\nb");
    assert_eq!(text("{\\rtf1 braces \\{ \\} and \\\\ end}"), "braces { } and \\ end");
    assert_eq!(text("{\\rtf1 line\r\nbreaks in the file are not breaks}"), "linebreaks in the file are not breaks");
}

#[test]
fn hex_escapes_use_the_code_page() {
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg1252 caf\\'e9 \\'80}"), "caf\u{e9} \u{20ac}");
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg1251 \\'cf\\'f0\\'e8\\'e2\\'e5\\'f2}"), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
    // Two bytes of one character (Shift JIS).
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg932 \\'82\\'a0}"), "\u{3042}");
    // No code page: Windows-1252.
    assert_eq!(text("{\\rtf1 \\'e9}"), "\u{e9}");
}

#[test]
fn unicode_escapes_skip_their_fallback() {
    assert_eq!(text("{\\rtf1\\uc1\\u8364? x}"), "\u{20ac} x");
    assert_eq!(text("{\\rtf1\\uc2\\u8364\\'80\\'80 x}"), "\u{20ac} x");
    assert_eq!(text("{\\rtf1\\uc0\\u8364 x}"), "\u{20ac}x");
    assert_eq!(text("{\\rtf1\\u-3913 ?}"), "\u{f0b7}");
    assert_eq!(text("{\\rtf1\\u-10179?\\u-8704? x}"), "\u{1f600} x");
    // A lone surrogate is a replacement character, not a panic.
    assert_eq!(text("{\\rtf1\\u-10179? x}"), "\u{fffd} x");
    // \ucN belongs to its group.
    assert_eq!(text("{\\rtf1{\\uc0}\\u65?B}"), "AB");
}

#[test]
fn nested_groups_and_skipped_destinations() {
    let rtf = "{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}{\\colortbl;\\red0\\green0\\blue0;}{\\stylesheet{\\s1 Heading;}}{\\*\\generator Hidden;}{\\info{\\title T}}\
               {\\pict\\wmetafile8 0102030405}{\\header Head}\\pard{\\b bold {\\i both}} plain{\\*\\htmltag64 <b>}}";
    assert_eq!(text(rtf), "bold both plain");
}

#[test]
fn encapsulated_text_and_html_give_what_the_document_shows() {
    assert_eq!(text("{\\rtf1\\ansi\\fromtext\\deff0{\\fonttbl{\\f0\\fswiss Arial;}}\\pard Hello\\par}"), "Hello");
    let html = "{\\rtf1\\ansi\\fromhtml1 {\\*\\htmltag19 <html>}\\htmlrtf {\\htmlrtf0 Hello {\\*\\htmltag84 <b>}there\\htmlrtf \\par\\htmlrtf0 }}";
    assert_eq!(text(html), "Hello there");
}

#[test]
fn bin_data_is_skipped_by_its_count() {
    let mut rtf = b"{\\rtf1 a{\\pict\\bin5 }}{b".to_vec();
    let at = rtf.iter().position(|&b| b == b'5').unwrap() + 2;
    rtf.splice(at..at, *b"}{\\par");
    assert_eq!(rtf_text(&rtf, 100).unwrap().0, "ab");
    // A count past the end is the end.
    assert_eq!(rtf_text(b"{\\rtf1 a\\bin99999999999 bc", 100).unwrap().0, "a");
}

#[test]
fn bad_input_is_an_error_or_what_could_be_read() {
    assert!(rtf_text(b"plain text", 100).is_err());
    assert!(rtf_text(b"", 100).is_err());
    assert_eq!(text("{\\rtf1 open {group {never closed"), "open group never closed");
    assert_eq!(text("{\\rtf1 closed}} too} many} text"), "closed too many text");
    assert_eq!(text("{\\rtf1 ends in a slash\\"), "ends in a slash");
    assert_eq!(text("{\\rtf1 \\'zz bad hex}"), "zz bad hex");
}

#[test]
fn depth_is_capped() {
    let deep = format!("{{\\rtf1 {}x{}", "{".repeat(MAX_RTF_DEPTH + 10), "}".repeat(MAX_RTF_DEPTH + 10));
    assert!(matches!(rtf_text(deep.as_bytes(), 100), Err(Error::Corrupt(_))));
    let ok = format!("{{\\rtf1 {}x{}", "{".repeat(100), "}".repeat(101));
    assert_eq!(text(&ok), "x");
}

#[test]
fn text_is_capped() {
    let long = format!("{{\\rtf1 {}}}", "word ".repeat(100_000));
    let (t, cut) = rtf_text(long.as_bytes(), 1000).unwrap();
    assert!(t.len() <= 1000 && cut);
    assert!(!rtf_text(b"{\\rtf1 short}", 1000).unwrap().1);
    // Folded blank lines still say the text was cut.
    let folded = format!("{{\\rtf1 {}}}", "x\\par\\par\\par\\par ".repeat(1000));
    let (t, cut) = rtf_text(folded.as_bytes(), 1000).unwrap();
    assert!(cut && t.len() < 1000, "{}", t.len());
    let multibyte = format!("{{\\rtf1 {}}}", "\\u233?".repeat(10_000));
    let t = rtf_text(multibyte.as_bytes(), 101).unwrap().0;
    assert!(t.len() <= 102 && t.chars().all(|c| c == '\u{e9}'));
}

#[test]
fn each_font_has_its_code_page() {
    // RichEdit's shape: ANSI document, a font of its own character set.
    assert_eq!(
        text("{\\rtf1\\ansi\\ansicpg1252{\\fonttbl{\\f0\\fnil\\fcharset128 Yu Gothic;}}\\f0\\'93\\'fa\\'96\\'7b\\'8c\\'ea}"),
        "\u{65e5}\u{672c}\u{8a9e}"
    );
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg1252{\\fonttbl{\\f0\\fnil\\fcharset204 Calibri;}}\\f0\\'cf\\'f0\\'e8 x}"), "\u{41f}\u{440}\u{438} x");
    // The font goes with its group.
    assert_eq!(
        text("{\\rtf1\\ansi\\ansicpg1252{\\fonttbl{\\f0\\fcharset204 A;}{\\f1\\fcharset161 B;}}{\\f0 \\'cf}\\'cf{\\f1\\'e1}\\f0\\'cf}"),
        "\u{41f}\u{cf}\u{3b1}\u{41f}"
    );
    // A change of font in the middle of a run of bytes.
    assert_eq!(text("{\\rtf1\\ansi{\\fonttbl{\\f0\\fcharset204 A;}{\\f1\\fcharset0 B;}}\\f0\\'cf\\f1\\'cf\\f0\\'cf}"), "\u{41f}\u{cf}\u{41f}");
    // A flat font table, \deff and no charset at all.
    assert_eq!(text("{\\rtf1\\ansi\\deff1{\\fonttbl\\f0\\fcharset0 A;\\f1\\fcharset204 B;}\\'cf}"), "\u{41f}");
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg1251{\\fonttbl{\\f0 A;}}\\f0\\'a8\\f9\\'a8}"), "\u{401}\u{401}");
}

#[test]
fn the_sample_of_ms_oxrtfex_reads_its_font_charset() {
    // [MS-OXRTFEX] 3.1: a 1251 document, and one font marked \fcharset0 for &uml; (0xA8, U+00A8).
    let rtf = "{\\rtf1\\ANSI\\ansicpg1251\\fromhtml1 \\deff0\r\n{\\fonttbl {\\f0\\fmodern Courier New;}{\\f1\\fswiss Arial;}{\\f2\\fswiss\\fcharset0 Arial;}}\
               \\uc1\\pard\\plain\\deftab360 \\f0\\fs24 {\\htmlrtf \\f1 \\htmlrtf0 \\'cf {\\*\\htmltag &uml;}\\htmlrtf {\\f2\\'a8}\\htmlrtf0 \\'a8}}";
    assert_eq!(text(rtf), "\u{41f} \u{a8}\u{401}");
}

#[test]
fn bytes_past_the_buffer_are_not_dropped() {
    let dbcs = format!("{{\\rtf1\\ansi\\ansicpg932 {}}}", "\\'82\\'a0".repeat(3000));
    assert_eq!(text(&dbcs), "\u{3042}".repeat(3000));
    // An odd byte first, so the 4096-byte mark falls inside a pair.
    let shifted = format!("{{\\rtf1\\ansi\\ansicpg932 \\'41{}}}", "\\'82\\'a0".repeat(3000));
    assert_eq!(text(&shifted), format!("A{}", "\u{3042}".repeat(3000)));
    let single = format!("{{\\rtf1\\ansi\\ansicpg1252 {}}}", "\\'e9".repeat(5000));
    assert_eq!(text(&single), "\u{e9}".repeat(5000));
    let raw: Vec<u8> = [b"{\\rtf1\\ansi\\ansicpg1252 ".as_slice(), &[0xE9u8; 5000], b"}"].concat();
    assert_eq!(rtf_text(&raw, 1 << 20).unwrap().0, "\u{e9}".repeat(5000));
}

#[test]
fn ansicpg_wins_over_mac_in_either_order() {
    assert_eq!(text("{\\rtf1\\ansicpg1251\\mac \\'cf}"), "\u{41f}");
    assert_eq!(text("{\\rtf1\\mac\\ansicpg1251 \\'cf}"), "\u{41f}");
    assert_eq!(text("{\\rtf1\\mac \\'a5}"), "\u{2022}");
}

#[test]
fn control_characters_are_not_text() {
    assert_eq!(text("{\\rtf1 a\\'00b\\u0? c\\'07\\u1\\u9? d\\u10?e}"), "ab c\t d\ne");
}

#[test]
fn a_literal_tab_and_a_backslash_line_end() {
    assert_eq!(text("{\\rtf1 a\tb\\\r\nc\\\nd\\\re}"), "a\tb\nc\nd\ne");
}

#[test]
fn the_code_pages_of_the_mail_are_the_code_pages_of_the_text() {
    // Pages the first version of the text reader lacked: KOI8-R, ISO 8859-2, GB 18030's, EUC-KR's other number.
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg20866 \\'d0\\'d2}"), "\u{43f}\u{440}");
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg28592 \\'b1}"), "\u{105}");
    assert_eq!(text("{\\rtf1\\ansi\\ansicpg51949 \\'b0\\'a1}"), "\u{ac00}");
    assert_eq!(craft_mime::charset::code_page_label(20866), Some("koi8-r"));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = decompress_rtf(&bytes, 1 << 16);
        let _ = rtf_text(&bytes, 1 << 12);
        let mut with_start = b"{\\rtf1".to_vec();
        with_start.extend_from_slice(&bytes);
        let _ = rtf_text(&with_start, 1 << 12);
    }

    #[test]
    fn changed_streams_never_panic(at in 0usize..49, to in any::<u8>(), cut in 0usize..49) {
        let mut v = EXAMPLE_1.to_vec();
        if let Some(b) = v.get_mut(at) { *b = to; }
        v.truncate(cut.max(1));
        if let Ok(rtf) = decompress_rtf(&v, 1 << 12) { let _ = rtf_text(&rtf, 100); }
    }

    #[test]
    fn rtf_with_valid_start_is_bounded(body in "[\\\\{}a-z0-9 '*-]{0,600}", max in 0usize..300) {
        let rtf = format!("{{\\rtf1 {body}");
        if let Ok((t, _)) = rtf_text(rtf.as_bytes(), max) { prop_assert!(t.len() <= max.saturating_add(4)); }
    }
}

/// The document of [MS-OXRTFEX] 2.1.3.1.3's kind: tags in groups, what isn't HTML suppressed.
const ENCAPSULATED: &[u8] =
    b"{\\rtf1\\ansi\\ansicpg1252\\fromhtml1 \\deff0{\\fonttbl{\\f0\\fswiss Arial;}{\\f1\\fcharset204 Cyr;}}{\\colortbl;\\red0;}\
{\\*\\htmltag64 <p>}{\\htmlrtf \\f1 \\htmlrtf0 caf\\'e9 {\\*\\htmltag <b>}{\\htmlrtf \\b \\htmlrtf0 bold{\\*\\htmltag </b>}}\
\\htmlrtf \\par \\htmlrtf0 {\\*\\htmltag &amp;}\\htmlrtf &\\htmlrtf0 \\u8364?{\\*\\mhtmltag <skipped>}{\\*\\other no}{\\*\\htmltag </p>}}}";

#[test]
fn encapsulated_html_comes_out_as_it_was() {
    let (html, cut) = rtf_html(ENCAPSULATED, 1 << 16).unwrap().unwrap();
    assert!(!cut);
    assert_eq!(html, "<p>caf\u{439} <b>bold</b>&amp;\u{20AC}</p>");
}

#[test]
fn a_tag_is_kept_even_where_the_text_round_it_is_suppressed() {
    let rtf = b"{\\rtf1\\fromhtml1 \\htmlrtf {\\*\\htmltag <i>}gone\\htmlrtf0 kept}";
    assert_eq!(rtf_html(rtf, 100).unwrap().unwrap().0, "<i>kept");
}

#[test]
fn only_a_document_that_says_so_is_html() {
    assert_eq!(rtf_html(b"{\\rtf1\\ansi plain}", 100).unwrap(), None);
    assert_eq!(rtf_html(b"{\\rtf1\\fromtext hello}", 100).unwrap(), None);
    assert_eq!(rtf_html(b"{\\rtf1 text\\fromhtml1 }", 100).unwrap(), None, "past a token that isn't a control word");
    assert_eq!(rtf_html(b"{\\rtf1\\a\\b\\c\\d\\e\\f\\g\\h\\i\\j\\fromhtml1 }", 100).unwrap(), None, "past the first ten tokens");
    assert_eq!(rtf_html(b"not rich text", 100).unwrap(), None);
}

#[test]
fn encapsulated_html_is_capped_and_a_deep_nest_refused() {
    let mut rtf = b"{\\rtf1\\fromhtml1 ".to_vec();
    rtf.extend(std::iter::repeat_n(b'x', 1000));
    let (html, cut) = rtf_html(&rtf, 10).unwrap().unwrap();
    assert!(cut && html.len() <= 10);
    let mut deep = b"{\\rtf1\\fromhtml1 ".to_vec();
    deep.extend(std::iter::repeat_n(b'{', MAX_RTF_DEPTH + 2));
    assert!(rtf_html(&deep, 100).is_err());
}

#[test]
fn the_text_of_encapsulated_html_is_still_what_it_shows() {
    let rtf = b"{\\rtf1\\fromhtml1 {\\*\\htmltag <p>}\\htmlrtf x\\htmlrtf0 hello}";
    assert_eq!(rtf_text(rtf, 100).unwrap().0, "xhello");
}

proptest! {
    #[test]
    fn any_bytes_read_as_html_never_panic(data in proptest::collection::vec(any::<u8>(), 0..2000)) {
        let mut rtf = b"{\\rtf1\\fromhtml1 ".to_vec();
        rtf.extend_from_slice(&data);
        let _ = rtf_html(&rtf, 1 << 12);
    }
}

/// The font character sets are the crate's table, but for ANSI (0), which is Windows-1252
/// whatever the document's code page, and the sets with no text code page here.
#[test]
fn font_character_sets_name_their_code_pages() {
    let want = [
        (0, 1252),
        (77, 10000),
        (128, 932),
        (129, 949),
        (134, 936),
        (136, 950),
        (161, 1253),
        (162, 1254),
        (163, 1258),
        (177, 1255),
        (178, 1256),
        (186, 1257),
        (204, 1251),
        (222, 874),
        (238, 1250),
    ];
    for charset in -1..=256 {
        let expected = want.iter().find(|(c, _)| *c == charset).map(|(_, cp)| *cp);
        assert_eq!(super::charset_code_page(charset), expected, "{charset}");
    }
}

#[test]
fn the_checksum_is_the_specifications() {
    // [MS-OXRTFCP] 3.2.3: the CRC of the compressed bytes of the first example is its header's.
    assert_eq!(crc32(&EXAMPLE_1[16..]), u32::from_le_bytes([0xf1, 0xc5, 0xc7, 0xa7]));
    assert_eq!(crc32(b""), 0);
}
