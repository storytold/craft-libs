//! Original paragraph-shading operands assembled from [MS-DOC] §§2.9.247-249 and
//! the sprmPShd/PShd80 tables, not documents or observations from another application.

use std::sync::Arc;

use craft_wordmodel::props::{CharProps, ParaProps};
use craft_wordmodel::styles::Style;
use craft_wordmodel::{Document, Paragraph, Rgb, para_block};

use super::{RichSpec, grpprl, rich_doc};
use crate::{fmt, read, sprm, write};

const P_SHD80: u16 = 0x442D;
const P_SHD: u16 = 0xC64D;
const AUTO: [u8; 11] = [10, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0, 0];
const NIL: [u8; 11] = [10, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0];
const RED: Rgb = Rgb(0xC0, 0x20, 0x30);
const BLUE: Rgb = Rgb(0x10, 0x40, 0xB0);

fn shd(fore: Rgb, back: Rgb, ipat: u16) -> [u8; 11] {
    let [lo, hi] = ipat.to_le_bytes();
    [10, fore.0, fore.1, fore.2, 0, back.0, back.1, back.2, 0, lo, hi]
}

fn apply(p: &mut ParaProps, prls: &[(u16, &[u8])]) {
    for prl in sprm::iter(&grpprl(prls)) {
        fmt::apply_para(p, &prl);
    }
}

fn state(p: &ParaProps, fill: Option<Rgb>, automatic: bool) {
    assert_eq!(p.shading, fill);
    assert_eq!(p.shading_auto, automatic);
}

#[test]
fn paragraph_shading_reads_spec_operands_and_last_valid_values() {
    let red = shd(BLUE, RED, 0);
    let blue = shd(RED, BLUE, 0);
    let mut p = ParaProps::default();
    apply(&mut p, &[(P_SHD, &red)]);
    state(&p, Some(RED), false);
    // Both exact no-shading structures are paragraph overrides, unlike absent sprms.
    for clear in [AUTO, NIL] {
        apply(&mut p, &[(P_SHD, &clear)]);
        state(&p, None, true);
        apply(&mut p, &[(P_SHD, &red)]);
        state(&p, Some(RED), false);
    }
    apply(&mut p, &[(P_SHD80, &[0x40, 0])]); // icoBack 2, blue; clear pattern.
    state(&p, Some(Rgb(0, 0, 255)), false);
    for clear in [[0, 0], [0xFF, 0xFF]] {
        apply(&mut p, &[(P_SHD80, &clear)]);
        state(&p, None, true);
    }
    // Solid uses the foreground, not the clear pattern's background.
    apply(&mut p, &[(P_SHD, &shd(RED, BLUE, 1))]);
    state(&p, Some(RED), false);
    apply(&mut p, &[(P_SHD80, &(6u16 | (2 << 5) | (1 << 10)).to_le_bytes())]);
    state(&p, Some(Rgb(255, 0, 0)), false);
    // The flat model retains the background of a supported non-solid pattern.
    apply(&mut p, &[(P_SHD, &shd(RED, BLUE, 4))]);
    state(&p, Some(BLUE), false);
    apply(&mut p, &[(P_SHD, &red), (P_SHD, &AUTO), (P_SHD, &blue)]);
    state(&p, Some(BLUE), false);
    apply(&mut p, &[(P_SHD80, &[0x40, 0]), (P_SHD, &AUTO)]);
    state(&p, None, true);
}

#[test]
fn unsupported_paragraph_shading_does_not_clear_existing_fill() {
    let red = shd(BLUE, RED, 0);
    let blue = shd(RED, BLUE, 0);
    let mut bad_fore = AUTO;
    bad_fore[4] = 1; // COLORREF fAuto must be 0 or 0xFF.
    let mut bad_back = AUTO;
    bad_back[8] = 1;
    let mut short = AUTO[..10].to_vec();
    short[0] = 9;
    let mut long = AUTO.to_vec();
    long[0] = 11;
    long.push(0);
    // A lone ipatNil is not the paragraph ShdNil structure. Reserved patterns and
    // palette values must not turn a failed colour decode into an automatic override.
    let invalid: &[(u16, &[u8])] = &[
        (P_SHD, &shd(RED, BLUE, 0xFFFF)),
        (P_SHD, &shd(RED, BLUE, 0x1A)),
        (P_SHD, &bad_fore),
        (P_SHD, &bad_back),
        (P_SHD, &short),
        (P_SHD, &long),
        (P_SHD80, &(17u16 | (2 << 5)).to_le_bytes()),
        (P_SHD80, &(6u16 | (2 << 5) | (0x3F << 10)).to_le_bytes()),
    ];
    for &(op, operand) in invalid {
        let mut p = ParaProps::default();
        apply(&mut p, &[(P_SHD, &red), (op, operand)]);
        state(&p, Some(RED), false);
        apply(&mut p, &[(op, operand), (P_SHD, &blue)]);
        state(&p, Some(BLUE), false);
    }
    // Truncated tails are dropped by the actual grpprl iterator, retaining earlier fill.
    for (op, tail) in [(P_SHD, &AUTO[..10]), (P_SHD80, &[0x40][..])] {
        let mut p = ParaProps::default();
        apply(&mut p, &[(P_SHD, &red), (op, tail)]);
        state(&p, Some(RED), false);
    }
    // An invalid later property must also preserve an earlier explicit automatic fill.
    let mut p = ParaProps::default();
    apply(&mut p, &[(P_SHD, &AUTO), (P_SHD, &bad_back)]);
    state(&p, None, true);
}

#[test]
fn paragraph_shading_overrides_style_only_when_explicit() {
    let red = shd(BLUE, RED, 0);
    let blue = shd(RED, BLUE, 0);
    for (style, direct, fill, automatic) in [
        (0, Vec::new(), Some(RED), false),
        (0, grpprl(&[(P_SHD, &AUTO)]), None, true),
        (0, grpprl(&[(P_SHD, &blue)]), Some(BLUE), false),
        (1, Vec::new(), None, true),
        (1, grpprl(&[(P_SHD, &blue)]), Some(BLUE), false),
    ] {
        let spec = RichSpec {
            text: "Shaded paragraph\r",
            styles: vec![(1, 0x0FFF, "Normal", grpprl(&[(P_SHD, &red)]), Vec::new()), (1, 0, "Automatic", grpprl(&[(P_SHD, &AUTO)]), Vec::new())],
            fonts: vec!["Times New Roman"],
            chpx_runs: vec![(0, Vec::new())],
            papx_entries: vec![(0, style, direct)],
        };
        let doc = read(&rich_doc(&spec)).expect("original spec-built DOC");
        let p = doc.body[0].as_para().expect("paragraph");
        assert_eq!(p.plain_text(), "Shaded paragraph");
        let resolved = doc.styles.resolve_para(&p.props);
        assert_eq!(resolved.shading, fill);
        assert_eq!(resolved.shading_auto, automatic);
    }
}

#[test]
fn paragraph_shading_round_trips_without_losing_inheritance() {
    let mut doc = Document::new();
    Arc::make_mut(&mut doc.styles).upsert(Style {
        id: "Shaded".into(),
        name: "Shaded".into(),
        para: ParaProps { shading: Some(RED), ..Default::default() },
        ..Default::default()
    });
    Arc::make_mut(&mut doc.styles).upsert(Style {
        id: "Automatic".into(),
        name: "Automatic".into(),
        based_on: Some("Shaded".into()),
        para: ParaProps { shading_auto: true, ..Default::default() },
        ..Default::default()
    });
    let inherited = ParaProps { style: Some("Shaded".into()), ..Default::default() };
    let automatic_style = ParaProps { style: Some("Automatic".into()), ..Default::default() };
    let cases = [
        ("Inherits red", inherited.clone()),
        ("Explicit automatic", ParaProps { shading_auto: true, ..inherited.clone() }),
        ("Color wins over marker", ParaProps { shading: Some(BLUE), shading_auto: true, ..inherited }),
        ("Automatic style", automatic_style.clone()),
        ("Color overrides automatic style", ParaProps { shading: Some(BLUE), ..automatic_style }),
        ("Unspecified", ParaProps::default()),
    ];
    doc.body = cases
        .iter()
        .map(|(text, props)| {
            let mut p = Paragraph::with_text(text, CharProps::default());
            p.props = props.clone();
            para_block(p)
        })
        .collect();
    let first = read(&write(&doc).expect("write DOC")).expect("import DOC");
    let second = read(&write(&first).expect("save imported DOC")).expect("reopen DOC");
    for round in [&first, &second] {
        assert_eq!(round.body.len(), cases.len());
        for (i, (text, props)) in cases.iter().enumerate() {
            let p = round.body[i].as_para().expect("paragraph");
            assert_eq!(p.plain_text(), *text);
            let wanted = doc.styles.resolve_para(props);
            let got = round.styles.resolve_para(&p.props);
            assert_eq!(got.shading, wanted.shading, "{text}");
            assert_eq!(got.shading_auto, wanted.shading_auto, "{text}");
        }
        state(&round.body[0].as_para().expect("inherited").props, None, false);
        state(&round.body[1].as_para().expect("automatic").props, None, true);
        state(&round.body[2].as_para().expect("colored").props, Some(BLUE), false);
        state(&round.body[5].as_para().expect("unspecified").props, None, false);
    }
}
