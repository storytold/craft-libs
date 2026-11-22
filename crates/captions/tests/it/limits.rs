//! Limits and hostile input.

use craft_captions::{Cue, Document, Error, Format, MAX_INPUT, TICKS_PER_SECOND, Tick, WriteOptions, parse, write};

fn doc() -> Document {
    let cue = |s: i64, text: &str| Cue {
        start: Tick(s * TICKS_PER_SECOND),
        end: Tick((s + 2) * TICKS_PER_SECOND),
        text: text.into(),
        ..Default::default()
    };
    Document { cues: vec![cue(1, "Hello <i>there</i>"), cue(4, "Second\nline")], ..Default::default() }
}

#[test]
fn input_over_the_size_limit_is_refused() {
    let big = vec![b' '; MAX_INPUT + 1];
    for format in Format::ALL {
        assert_eq!(parse(&big, format), Err(Error::TooLarge(MAX_INPUT)), "{format:?}");
    }
    assert!(parse(big.get(..MAX_INPUT).unwrap(), Format::Srt).is_ok());
}

/// TTML is read without entity expansion: a document type with entities defines nothing.
#[test]
fn ttml_does_not_expand_entities() {
    let bomb = r#"<?xml version="1.0"?><!DOCTYPE tt [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;">]>
<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="1s" end="2s">&b;</p></div></body></tt>"#;
    assert_eq!(parse(bomb.as_bytes(), Format::Ttml), Err(Error::NotFormat("TTML")));
}

/// Every single-byte change to a written file reads or is refused; none panics.
#[test]
fn mutated_files_never_panic() {
    // CRAFT_HOSTILE_DIVISOR=N (pull requests) cuts at every N-th byte, from 0, and changes every
    // N-th byte, from a different offset for each format.
    let step = craft_media_testkit::hostile::divisor();
    for (k, format) in Format::ALL.into_iter().enumerate() {
        let bytes = write(&doc(), format, WriteOptions::default());
        // every byte of a text file, and the header and first blocks of the (sparse) binary one
        for i in 0..bytes.len().min(2048) {
            if i % step == k % step {
                for v in [0u8, 0x20, 0x3c, 0x7f, 0xff] {
                    let mut m = bytes.clone();
                    if let Some(b) = m.get_mut(i) {
                        *b = v;
                    }
                    let _ = parse(&m, format);
                }
            }
            if i % step == 0 {
                let _ = parse(bytes.get(..i).unwrap_or(&[]), format);
            }
        }
    }
}

/// The creating program is the caller's, cut to what the header field can hold.
#[test]
fn mcc_and_stl_name_the_given_program() {
    let opts = WriteOptions { program: "My\nApp \u{e9}", ..Default::default() };
    let mcc = String::from_utf8(write(&doc(), Format::Mcc, opts)).unwrap();
    assert!(mcc.contains("\nCreation Program=MyApp \u{e9}\n"), "{mcc}");
    let stl = write(&doc(), Format::Stl, opts);
    assert_eq!(stl.get(16..48), Some(&b"MyApp                           "[..]));
    assert_eq!(parse(&stl, Format::Stl).unwrap().cues.len(), 2);
    let default = String::from_utf8(write(&doc(), Format::Mcc, WriteOptions::default())).unwrap();
    assert!(default.contains("Creation Program=craft-captions\n"));
}
