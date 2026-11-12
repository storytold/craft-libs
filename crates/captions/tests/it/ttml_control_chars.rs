use craft_captions::{Cue, Document, Format, WriteOptions, parse, write};
use craft_captions::{TICKS_PER_SECOND, Tick};

/// Control characters that XML 1.0 forbids must not make the export unreadable.
#[test]
fn ttml_export_drops_xml_illegal_control_chars() {
    // every forbidden character, each between two letters that must survive
    let forbidden = (0u32..0x20).filter(|c| ![0x9, 0xA, 0xD].contains(c)).chain([0xFFFE, 0xFFFF]).filter_map(char::from_u32);
    let text: String = forbidden.flat_map(|c| ['a', c]).chain(['e']).collect();
    let want = format!("{}e", "a".repeat(31));
    for format in [Format::Ttml, Format::Dfxp] {
        let cue = Cue { start: Tick(TICKS_PER_SECOND), end: Tick(2 * TICKS_PER_SECOND), text: text.clone(), ..Default::default() };
        let doc = Document { cues: vec![cue], ..Default::default() };
        let xml = String::from_utf8(write(&doc, format, WriteOptions::default())).unwrap();
        let back = parse(xml.as_bytes(), format);
        assert!(back.is_ok(), "{format:?}: {xml}");
        assert_eq!(back.unwrap().cues[0].text, want, "{format:?}");
    }
}
