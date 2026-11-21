//! Synthetic per-section RTF header/footer preservation controls.
#![cfg(feature = "doc")]

use craft_flow as model;
use craft_wordmodel::{Block, Document};

/// A section export must carry the stories named by its section properties.
#[test]
fn rtf_export_keeps_section_headers_and_footers() {
    let input = br"{\rtf1\ansi\sectd\titlepg
{\header \pard\qc TEST HEADER ONE\par}
{\headerf \pard TEST FIRST HEADER\par}
{\footer \pard TEST FOOTER ONE\par}
\pard TEST BODY ONE\sect
\sectd{\header \pard TEST HEADER TWO\par}
{\footer \pard TEST FOOTER TWO\par}
\pard TEST BODY TWO\par}";
    let before = craft_rtf::doc::import(input).unwrap();
    let after = craft_rtf::doc::import(craft_rtf::doc::export(&before).as_bytes()).unwrap();
    let stories = |d: &Document| {
        d.sections()
            .into_iter()
            .map(|(_, sec)| {
                [sec.headers.default, sec.headers.first, sec.footers.default].map(|id| {
                    id.and_then(|id| d.parts.get(&id))
                        .map(|part| part.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("|"))
                })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(stories(&after), stories(&before));
    assert_eq!(model::from_doc(&after).blocks.len(), model::from_doc(&before).blocks.len());
}

/// All six section-story destinations retain their text, formatting and header tables.
#[test]
fn rtf_export_keeps_all_header_footer_destinations_and_shared_resources() {
    let input = br"{\rtf1\ansi\deff0
{\fonttbl{\f0 TEST BASE;}{\f1 TEST FONT;}}
{\colortbl;\red12\green34\blue56;}
\sectd\titlepg
{\header \pard\f1\cf1\b TEST HEADER\par
\trowd\cellx2000\pard\plain\intbl TEST CELL\cell\row}
{\headerf \pard TEST FIRST HEADER\par}
{\headerl \pard TEST EVEN HEADER\par}
{\footer \pard TEST FOOTER\par}
{\footerf \pard TEST FIRST FOOTER\par}
{\footerl \pard TEST EVEN FOOTER\par}
\pard\plain TEST BODY\par}";
    let mut before = craft_rtf::doc::import(input).unwrap();
    let original_header = before.parts.get(&before.last_section.headers.default.unwrap()).unwrap();
    assert!(original_header.blocks.iter().any(|b| matches!(&**b, Block::Table(_))), "synthetic input has a header table");
    before.settings.even_odd_headers = true;
    let encoded = craft_rtf::doc::export(&before);
    assert!(encoded.contains("\\facingp"), "even-page stories need the document flag: {encoded}");
    for destination in ["headerr", "headerf", "headerl", "footerr", "footerf", "footerl"] {
        assert!(encoded.contains(&format!("{{\\{destination} ")), "{destination}: {encoded}");
    }
    let after = craft_rtf::doc::import(encoded.as_bytes()).unwrap();
    let section = &after.last_section;
    for (id, expected) in [
        (section.headers.first, "TEST FIRST HEADER"),
        (section.headers.even, "TEST EVEN HEADER"),
        (section.footers.default, "TEST FOOTER"),
        (section.footers.first, "TEST FIRST FOOTER"),
        (section.footers.even, "TEST EVEN FOOTER"),
    ] {
        let part = after.parts.get(&id.unwrap()).unwrap();
        assert_eq!(part.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("|"), expected);
    }
    let part = after.parts.get(&section.headers.default.unwrap()).unwrap();
    let heading = part.blocks.iter().find_map(|b| b.as_para()).unwrap();
    assert!(heading.run_ranges().any(|(_, cp)| cp.bold == Some(true) && cp.font.as_deref() == Some("TEST FONT")));
    assert!(heading.run_ranges().any(|(_, cp)| cp.color == Some(craft_wordmodel::TextColor::Rgb(craft_wordmodel::Rgb(12, 34, 56)))));
    let table = part.blocks.iter().find_map(|b| if let Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
    assert_eq!(table.rows[0].cells[0].blocks[0].as_para().unwrap().plain_text(), "TEST CELL");
    assert_eq!(after.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("|"), "TEST BODY");
}

/// Omitted headers remain linked, while explicit empty headers clear inherited content.
#[test]
fn rtf_export_distinguishes_linked_and_explicitly_empty_headers() {
    let input = br"{\rtf1\ansi\sectd
{\header \pard TEST HEADER\par}\pard TEST ONE\sect
\sectd\pard TEST TWO\sect
\sectd{\header }\pard TEST THREE\par}";
    let before = craft_rtf::doc::import(input).unwrap();
    let encoded = craft_rtf::doc::export(&before);
    let after = craft_rtf::doc::import(encoded.as_bytes()).unwrap();
    let sections = after.sections();
    assert_eq!(sections.len(), 3);
    assert!(sections[0].1.headers.default.is_some());
    assert!(sections[1].1.headers.default.is_none(), "an omitted destination stays linked");
    let empty = sections[2].1.headers.default.unwrap();
    assert!(
        after.parts.get(&empty).unwrap().blocks.iter().all(|b| b.as_para().is_some_and(|p| p.plain_text().is_empty())),
        "an empty destination clears the inherited story"
    );
}

/// Facing-page activation survives RTF round trips without enabling dormant even stories.
#[test]
fn rtf_roundtrip_keeps_even_odd_header_setting() {
    for (control, enabled) in [("", false), ("\\facingp", true), ("\\facingp1", true), ("\\facingp0", false), ("\\facingp\\facingp0", false)] {
        let input = format!(
            "{{\\rtf1\\ansi{control}\\sectd{{\\header TEST DEFAULT HEADER\\par}}{{\\headerl TEST EVEN HEADER\\par}}\
             {{\\footer TEST DEFAULT FOOTER\\par}}{{\\footerl TEST EVEN FOOTER\\par}}\\pard TEST BODY\\page TEST SECOND PAGE\\par}}"
        );
        let before = craft_rtf::doc::import(input.as_bytes()).unwrap();
        assert_eq!(before.settings.even_odd_headers, enabled, "import {control:?}");
        let encoded = craft_rtf::doc::export(&before);
        assert_eq!(encoded.contains("\\facingp"), enabled, "export {control:?}");
        for destination in if enabled { ["headerr", "footerr"] } else { ["header", "footer"] } {
            assert!(encoded.contains(&format!("{{\\{destination} ")), "{encoded}");
        }
        let after = craft_rtf::doc::import(encoded.as_bytes()).unwrap();
        assert_eq!(after.settings.even_odd_headers, enabled, "reimport {control:?}");
        for (id, expected) in [
            (after.last_section.headers.default, "TEST DEFAULT HEADER"),
            (after.last_section.headers.even, "TEST EVEN HEADER"),
            (after.last_section.footers.default, "TEST DEFAULT FOOTER"),
            (after.last_section.footers.even, "TEST EVEN FOOTER"),
        ] {
            let part = after.parts.get(&id.unwrap()).unwrap();
            assert_eq!(part.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("|"), expected);
        }
    }
}

/// Header/footer and skipped destinations cannot change the document-wide facing-page setting.
#[test]
fn rtf_facing_pages_ignore_story_local_and_skipped_controls() {
    for (control, local, enabled) in [("", "\\facingp", false), ("\\facingp", "\\facingp0", true)] {
        let input = format!(
            "{{\\rtf1\\ansi{control}\\sectd{{\\header {local} TEST HEADER\\par}}\
             {{\\footer {local} TEST FOOTER\\par}}{{\\*\\testskip {local}}}\\pard TEST BODY\\par}}"
        );
        let doc = craft_rtf::doc::import(input.as_bytes()).unwrap();
        assert_eq!(doc.settings.even_odd_headers, enabled, "{input}");
    }
}
