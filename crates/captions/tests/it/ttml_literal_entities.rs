//! TTML text that looks like markup stays text, beside the cue's own style tags.

use craft_captions::ttml::{Flavor, write};
use craft_captions::{Format, TICKS_PER_SECOND, Tick, parse};

fn caption(xml: &str, format: Format) -> String {
    let doc = parse(xml.as_bytes(), format).unwrap();
    assert_eq!(doc.cues.len(), 1);
    assert_eq!((doc.cues[0].start, doc.cues[0].end), (Tick(TICKS_PER_SECOND), Tick(TICKS_PER_SECOND * 2)));
    doc.cues[0].text.clone()
}

/// A literal `<` in cue text is kept as `&lt;`, the way a VTT import keeps its entities, so the
/// style tags around it stay tags and plain text (burn-in, export) decodes the bracket instead of
/// cutting at it; a `>` starts no tag and stays literal.
#[test]
fn literal_brackets_keep_their_words_and_their_style() {
    for (xml, stored, plain) in [
        (
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling"><body tts:fontStyle="italic"><div><p begin="1s" end="2s">1 &lt; 2</p></div></body></tt>"#,
            "<i>1 &lt; 2</i>",
            "1 < 2",
        ),
        (
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling"><body><div><p begin="1s" end="2s">a <span tts:fontWeight="bold">b &gt; c</span> d</p></div></body></tt>"#,
            "a <b>b > c</b> d",
            "a b > c d",
        ),
        (
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling"><body><div><p begin="1s" end="2s">Use &lt;<span tts:fontWeight="bold">input</span>&gt;</p></div></body></tt>"#,
            "Use &lt;<b>input</b>>",
            "Use <input>",
        ),
        (
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling"><body tts:fontStyle="italic"><div><p begin="1s" end="2s">2 &lt; 3 &gt; 1</p></div></body></tt>"#,
            "<i>2 &lt; 3 > 1</i>",
            "2 < 3 > 1",
        ),
        (
            // a literal "&lt;" in the source text (an escaped ampersand) stays distinct from `<`
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling"><body><div><p begin="1s" end="2s">Type &lt; and &amp;lt;</p></div></body></tt>"#,
            "Type &lt; and &amp;lt;",
            "Type < and &lt;",
        ),
    ] {
        let text = caption(xml, Format::Ttml);
        assert_eq!(text, stored);
        assert_eq!(craft_captions::plain_text(&text), plain);
        // written back out, the XML text is the source text escaped once (not the stored entity
        // escaped again), and imported again, in both flavours, the cue reads the same
        let doc = parse(xml.as_bytes(), Format::Ttml).unwrap();
        for flavor in [Flavor::Imsc1, Flavor::Dfxp] {
            let again = write(&doc, flavor, None, "en");
            assert!(!again.contains("&amp;lt;") || stored.contains("&amp;lt;"), "{flavor:?}: {again}");
            if stored == "<i>1 &lt; 2</i>" {
                assert!(again.contains(">1 &lt; 2<"), "{flavor:?}: {again}");
            }
            assert_eq!(caption(&again, Format::Ttml), stored, "{flavor:?}: {again}");
        }
    }
}
