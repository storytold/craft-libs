//! Legacy form fields: FFData records ([MS-DOC] §2.9.75) hand-built for these tests from the
//! specification, laid out as Word 2016 writes them (the location on the 0x01 character that
//! ends the instruction, with `sprmCFData` on). No file produced by Word is used.

use craft_wordmodel::legacy_form::LegacyForm;
use craft_wordmodel::para::InlineObject;

use super::{StructSpec, grpprl, read, struct_doc, structured_spec};

/// An Xstz: the count, the UTF-16 text and the 0 terminator.
fn xstz(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut v = (units.len() as u16).to_le_bytes().to_vec();
    units.iter().for_each(|u| v.extend_from_slice(&u.to_le_bytes()));
    v.extend_from_slice(&[0, 0]);
    v
}

/// The settings of one form field.
#[derive(Default)]
struct Ff {
    /// `FFDataBits`: iType (bits 0-1), iRes (2-6), fOwnHelp (7), fOwnStat (8), fProt (9),
    /// iSize (10), iTypeTxt (11-13), fRecalc (14), fHasListBox (15).
    bits: u16,
    cch: u16,
    hps: u16,
    name: &'static str,
    text_def: &'static str,
    w_def: u16,
    help: &'static str,
    entries: Vec<&'static str>,
}

const CHECK_BOX: u16 = 1;
const DROP_DOWN: u16 = 2 | (1 << 15);
const UNDEFINED: u16 = 25 << 2;

impl Ff {
    /// The FFData, behind the 68-byte header of a NilPICFAndBinData.
    fn record(&self) -> Vec<u8> {
        let kind = self.bits & 0b11;
        let mut ff = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
        ff.extend_from_slice(&self.bits.to_le_bytes());
        ff.extend_from_slice(&self.cch.to_le_bytes());
        ff.extend_from_slice(&self.hps.to_le_bytes());
        ff.extend(xstz(self.name));
        if kind == 0 {
            ff.extend(xstz(self.text_def));
        } else {
            ff.extend_from_slice(&self.w_def.to_le_bytes());
        }
        ff.extend(xstz("")); // format
        ff.extend(xstz(self.help));
        for _ in 0..3 {
            ff.extend(xstz("")); // status text, entry and exit macros
        }
        if kind == 2 {
            ff.extend_from_slice(&0xFFFFu16.to_le_bytes());
            ff.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
            ff.extend_from_slice(&0u16.to_le_bytes());
            for e in &self.entries {
                let units: Vec<u16> = e.encode_utf16().collect();
                ff.extend_from_slice(&(units.len() as u16).to_le_bytes());
                units.iter().for_each(|u| ff.extend_from_slice(&u.to_le_bytes()));
            }
        }
        let mut header = vec![0u8; 0x44];
        header[0..4].copy_from_slice(&((0x44 + ff.len()) as u32).to_le_bytes());
        header[4..6].copy_from_slice(&0x44u16.to_le_bytes());
        [header, ff].concat()
    }
}

/// A document with one paragraph holding `fields`, each `(instruction, result, data)`: the
/// field's 0x01 character points at `data` in the Data stream with `sprmCFData` on.
fn form_doc(fields: &[(&str, &str, Vec<u8>)]) -> Vec<u8> {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut data = Vec::new();
    for (instr, result, record) in fields {
        text.push_str("x\u{13} ");
        text.push_str(instr);
        text.push(' ');
        let at = text.chars().count() as u32;
        runs.push((at, grpprl(&[(0x6A03, &(data.len() as u32).to_le_bytes()), (0x0806, &[1])])));
        runs.push((at + 1, Vec::new()));
        data.extend_from_slice(record);
        text.push('\u{1}');
        if !result.is_empty() {
            text.push('\u{14}');
            text.push_str(result);
        }
        text.push('\u{15}');
    }
    text.push('\r');
    let text: &'static str = Box::leak(text.into_boxed_str());
    let ccp = text.chars().count() as u32;
    let spec = StructSpec { text, data_stream: data, chpx_runs: [vec![(0, Vec::new())], runs].concat(), ..structured_spec(text, ccp) };
    struct_doc(&spec)
}

/// The fields of the first paragraph, as (instruction, `w:ffData`).
fn fields(bytes: &[u8]) -> Vec<(String, String)> {
    let doc = read(bytes).expect("opens");
    let p = doc.body.first().and_then(|b| b.as_para()).expect("paragraph");
    assert!(!p.objects.iter().any(|o| matches!(o, InlineObject::Image { .. })), "the FFData is not a picture: {:?}", p.objects);
    p.objects
        .iter()
        .filter_map(|o| match o {
            InlineObject::Field { instr, form, .. } => Some((instr.clone(), form.clone())),
            _ => None,
        })
        .collect()
}

fn form_of(xml: &str) -> Option<LegacyForm> {
    LegacyForm::parse(xml)
}

#[test]
fn check_boxes_keep_their_state_and_size() {
    let bytes = form_doc(&[
        ("FORMCHECKBOX", "", Ff { bits: CHECK_BOX | UNDEFINED, hps: 20, name: "Off", ..Default::default() }.record()),
        ("FORMCHECKBOX", "", Ff { bits: CHECK_BOX | (1 << 2), hps: 20, name: "On", ..Default::default() }.record()),
        ("FORMCHECKBOX", "", Ff { bits: CHECK_BOX | UNDEFINED, hps: 20, name: "ByDefault", w_def: 1, ..Default::default() }.record()),
        ("FORMCHECKBOX", "", Ff { bits: CHECK_BOX | UNDEFINED | (1 << 10), hps: 40, name: "Big", ..Default::default() }.record()),
        ("FORMCHECKBOX", "", Ff { bits: CHECK_BOX, hps: 20, name: "Cleared", w_def: 1, ..Default::default() }.record()),
    ]);
    let got: Vec<Option<LegacyForm>> = fields(&bytes).iter().map(|(_, xml)| form_of(xml)).collect();
    let check = |name: &str, checked: bool, size: Option<f32>| Some(LegacyForm::CheckBox { name: name.into(), checked, size });
    assert_eq!(
        got,
        [
            check("Off", false, None),
            check("On", true, None),
            check("ByDefault", true, None),
            check("Big", false, Some(20.0)),
            check("Cleared", false, None)
        ]
    );
}

/// The XML is the one Word writes for the same check box in a `.docx`, so a `.doc` saved as a
/// `.docx` keeps it as Word would.
#[test]
fn a_check_box_is_written_as_word_writes_it() {
    let bytes = form_doc(&[("FORMCHECKBOX", "", Ff { bits: CHECK_BOX | (1 << 2), hps: 20, name: "On", ..Default::default() }.record())]);
    assert_eq!(
        fields(&bytes),
        [(
            "FORMCHECKBOX".to_string(),
            r#"<w:ffData><w:name w:val="On"/><w:enabled/><w:calcOnExit w:val="0"/><w:checkBox><w:sizeAuto/><w:default w:val="0"/><w:checked/></w:checkBox></w:ffData>"#
                .to_string()
        )]
    );
}

#[test]
fn drop_downs_and_text_boxes_keep_their_settings() {
    let bytes = form_doc(&[
        (
            "FORMDROPDOWN",
            "",
            Ff { bits: DROP_DOWN | (1 << 2), hps: 20, name: "List", entries: vec!["Alpha", "Beta", "Gamma"], ..Default::default() }.record(),
        ),
        ("FORMTEXT", "Ann", Ff { cch: 40, name: "Who", text_def: "Ann", ..Default::default() }.record()),
    ]);
    let got = fields(&bytes);
    let list = form_of(&got[0].1).expect("drop-down");
    assert_eq!(list.shown_text(), Some("Beta"), "{}", got[0].1);
    assert_eq!(form_of(&got[1].1), Some(LegacyForm::Text { name: "Who".into(), max_len: Some(40) }));
    assert!(got[1].1.contains(r#"<w:default w:val="Ann"/>"#), "{}", got[1].1);
}

/// Names, entries and help text are escaped; a protected field is not enabled; a recalculated
/// one calculates on exit, and a number box says so.
#[test]
fn form_field_text_is_escaped_and_flags_are_kept() {
    let bytes = form_doc(&[(
        "FORMTEXT",
        "1",
        Ff { bits: (1 << 7) | (1 << 9) | (1 << 11) | (1 << 14), name: "A&B", help: "Type <a> \"number\"", text_def: "1", ..Default::default() }
            .record(),
    )]);
    let got = fields(&bytes);
    assert_eq!(
        got[0].1,
        r#"<w:ffData><w:name w:val="A&amp;B"/><w:enabled w:val="0"/><w:calcOnExit/><w:helpText w:type="text" w:val="Type &lt;a&gt; &quot;number&quot;"/><w:textInput><w:type w:val="number"/><w:default w:val="1"/></w:textInput></w:ffData>"#
    );
    assert_eq!(form_of(&got[0].1).map(|f| f.name().to_string()), Some("A&B".into()));
}

/// Only form fields keep the data: another field with the same marks gets none.
#[test]
fn only_form_fields_keep_ff_data() {
    let bytes = form_doc(&[("PAGE", "1", Ff { bits: CHECK_BOX | (1 << 2), name: "On", ..Default::default() }.record())]);
    assert_eq!(fields(&bytes), [("PAGE".to_string(), String::new())]);
}

/// Truncated, wrongly versioned or oversized records leave the field without settings, never
/// a panic.
#[test]
fn malformed_ff_data_leaves_the_field_without_settings() {
    let good = Ff { bits: DROP_DOWN, name: "List", entries: vec!["Alpha", "Beta"], ..Default::default() }.record();
    for len in 0..good.len() {
        let mut cut = good[..len].to_vec();
        if cut.len() >= 4 {
            // Keep `lcb` honest about the cut, so the record itself is short.
            cut[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        }
        let got = fields(&form_doc(&[("FORMDROPDOWN", "", cut)]));
        assert_eq!(got.len(), 1, "cut at {len}");
        assert!(got[0].1.is_empty() || form_of(&got[0].1).is_some(), "cut at {len}: {}", got[0].1);
    }
    let mut bad_version = good.clone();
    bad_version[0x44] = 0;
    assert_eq!(fields(&form_doc(&[("FORMDROPDOWN", "", bad_version)])), [("FORMDROPDOWN".to_string(), String::new())]);
    let mut long_name = good.clone();
    long_name[0x44 + 10..0x44 + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(fields(&form_doc(&[("FORMDROPDOWN", "", long_name)])), [("FORMDROPDOWN".to_string(), String::new())]);
    let mut past_the_stream = good;
    past_the_stream[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    let got = fields(&form_doc(&[("FORMDROPDOWN", "", past_the_stream)]));
    assert_eq!(form_of(&got[0].1).and_then(|f| f.shown_text().map(str::to_string)), Some("Alpha".into()), "lcb is clamped to the stream");
}

/// [MS-DOC] caps the number of drop-down entries, not their length: a long entry is kept.
#[test]
fn a_long_drop_down_entry_is_kept() {
    let long: &'static str = "x".repeat(300).leak();
    let bytes = form_doc(&[(
        "FORMDROPDOWN",
        "",
        Ff { bits: DROP_DOWN | (1 << 2), name: "List", entries: vec!["Alpha", long], ..Default::default() }.record(),
    )]);
    let got = fields(&bytes);
    let list = form_of(&got[0].1).expect("drop-down");
    assert_eq!(list.shown_text(), Some(long));
}

/// `sprmCFData` and `sprmCPicLocation` come from the CHPX, then from the piece's Prm, which
/// applies after it; the last of each wins.
#[test]
fn form_data_sprms_follow_the_chpx_then_the_piece_prm() {
    use crate::piece::PrmRef;
    let work = crate::Work::new(1 << 20);
    let at = |v: u32| v.to_le_bytes();
    let on = grpprl(&[(0x0806, &[1]), (0x6A03, &at(10))]);
    let off = grpprl(&[(0x0806, &[0])]);
    assert_eq!(crate::form_sprms(&on, PrmRef::None, &work), (true, Some(10)));
    // The Prm turns it off, or on with its own location.
    assert_eq!(crate::form_sprms(&on, PrmRef::Grpprl { index: 0, grpprl: Some(&off) }, &work), (false, Some(10)));
    let prm = grpprl(&[(0x0806, &[1]), (0x6A03, &at(20))]);
    assert_eq!(crate::form_sprms(&[], PrmRef::Grpprl { index: 0, grpprl: Some(&prm) }, &work), (true, Some(20)));
    assert_eq!(crate::form_sprms(&off, PrmRef::Grpprl { index: 0, grpprl: Some(&prm) }, &work), (true, Some(20)));
    // Within one grpprl the last location wins, for form data and pictures alike.
    let twice = grpprl(&[(0x6A03, &at(10)), (0x0806, &[1]), (0x6A03, &at(30))]);
    assert_eq!(crate::form_sprms(&twice, PrmRef::None, &work), (true, Some(30)));
    assert_eq!(crate::pic_location(&twice), Some(30));
    // Once the budget is spent, a Prm1 grpprl is skipped and the CHPX alone counts.
    let spent = crate::Work::new(0);
    assert_eq!(crate::form_sprms(&on, PrmRef::Grpprl { index: 0, grpprl: Some(&off) }, &spent), (true, Some(10)));
}

/// A Prm0 with isprm 0x47 is `sprmCFData` ([MS-DOC] Prm0): it turns the flag on for a CHPX that
/// gives only the location.
#[test]
fn a_prm0_turns_form_data_on() {
    use crate::piece::PrmRef;
    let work = crate::Work::new(1 << 20);
    let located = grpprl(&[(0x6A03, &10u32.to_le_bytes())]);
    assert_eq!(crate::form_sprms(&located, PrmRef::None, &work), (false, Some(10)));
    assert_eq!(crate::form_sprms(&located, PrmRef::Sprm0 { isprm: 0x47, val: 1 }, &work), (true, Some(10)));
}

/// A Prm0 `sprmCFData` with val 0 turns off the flag the CHPX set, since the Prm applies after
/// the CHPX.
#[test]
fn a_prm0_turns_the_chpx_form_data_off() {
    use crate::piece::PrmRef;
    let work = crate::Work::new(1 << 20);
    let on = grpprl(&[(0x0806, &[1]), (0x6A03, &10u32.to_le_bytes())]);
    assert_eq!(crate::form_sprms(&on, PrmRef::Sprm0 { isprm: 0x47, val: 0 }, &work), (false, Some(10)));
}

/// The bytes the decoder may read are charged before decoding, even when the record turns out
/// unreadable, so markers repeating one record stop once the budget is spent.
#[test]
fn form_data_decoding_is_charged_before_it_is_done() {
    let record = Ff { bits: CHECK_BOX | (1 << 2), hps: 20, name: "On", ..Default::default() }.record();
    let mut bad = record.clone();
    bad[0x44] = 0; // wrong version: read, then rejected
    let work = crate::Work::new(10_000);
    assert_eq!(crate::form_xml(&bad, 0, &work), None);
    assert_eq!(work.left.get(), 10_000 - bad.len() as u64, "a failed decode still costs its bytes");
    let xml = crate::form_xml(&record, 0, &work).expect("decoded");
    assert_eq!(work.left.get(), 10_000 - 2 * record.len() as u64 - xml.len() as u64);
    // Too little left for the record: nothing is decoded or kept.
    let short = crate::Work::new(record.len() as u64 - 1);
    assert_eq!(crate::form_xml(&record, 0, &short), None);
    // A hostile `lcb` costs no more than the bytes there are.
    let mut huge = record.clone();
    huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(crate::form::decode_cost(&huge, 0), huge.len() as u64);
}
