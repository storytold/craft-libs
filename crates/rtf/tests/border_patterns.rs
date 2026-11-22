//! Distinct small-gap border preservation in the word-model RTF format.
#![cfg(feature = "doc")]
use craft_flow::FBlock;
use craft_rtf::doc::{export, import, parse};

/// Small-gap paragraph borders keep their style through parsing, export and re-import.
#[test]
fn dash_small_gap_border_round_trips() {
    use craft_wordmodel::props::BorderStyle;
    let rtf = br"{\rtf1\ansi\pard\brdrb\brdrdashsm\brdrw10 Dash\par}";
    let flow = parse(rtf).unwrap();
    let FBlock::Para(para) = &flow.blocks[0] else { panic!() };
    assert_eq!(para.borders.unwrap().bottom.unwrap().style, BorderStyle::DashSmallGap);
    let doc = import(rtf).unwrap();
    let exported = export(&doc);
    assert!(exported.contains("\\brdrb\\brdrdashsm\\brdrw10"), "{exported}");
    let back = import(exported.as_bytes()).unwrap();
    let para = back.body.iter().find_map(|b| if let craft_wordmodel::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
    let border = para.props.borders.unwrap().bottom.unwrap();
    assert_eq!(border.style, BorderStyle::DashSmallGap);
    assert_eq!(border.width, 0.5);
}
