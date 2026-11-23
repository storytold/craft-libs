//! Legacy form fields (`FORMTEXT`, `FORMCHECKBOX`, `FORMDROPDOWN`): their `FFData` in the Data
//! stream ([MS-DOC] §2.9.78), built here byte by byte, comes back as the field's `w:ffData`, and
//! survives conversion to `.docx`.
use super::{StructSpec, grpprl, struct_doc, structured_spec};
use crate::{Cost, Work, ffdata, read};
use craft_wordmodel::para::InlineObject;

/// An `FFData` to encode; the strings not named stay empty.
#[derive(Default)]
struct Ff {
    /// The bit field after the version: type, result, flags.
    bits: u16,
    max_len: u16,
    hps: u16,
    name: &'static str,
    /// `xstzTextDef` of a text field.
    text_default: &'static str,
    /// `wDef` of a check box or drop-down.
    default: u16,
    help: &'static str,
    exit_macro: &'static str,
    /// The drop-down's entries.
    entries: &'static [&'static str],
}

fn xstz(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut v = (units.len() as u16).to_le_bytes().to_vec();
    v.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    v.extend_from_slice(&[0, 0]);
    v
}

fn ffdata_bytes(ff: &Ff) -> Vec<u8> {
    let mut v = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
    v.extend_from_slice(&ff.bits.to_le_bytes());
    v.extend_from_slice(&ff.max_len.to_le_bytes());
    v.extend_from_slice(&ff.hps.to_le_bytes());
    v.extend(xstz(ff.name));
    if ff.bits & 3 == 0 {
        v.extend(xstz(ff.text_default));
    } else {
        v.extend_from_slice(&ff.default.to_le_bytes());
    }
    v.extend(xstz("")); // format
    v.extend(xstz(ff.help));
    v.extend(xstz("")); // status text
    v.extend(xstz("")); // entry macro
    v.extend(xstz(ff.exit_macro));
    if ff.bits & 3 == 2 {
        v.extend_from_slice(&[0xFF, 0xFF]);
        v.extend_from_slice(&(ff.entries.len() as u16).to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        for e in ff.entries {
            v.extend(&(e.encode_utf16().count() as u16).to_le_bytes());
            v.extend(e.encode_utf16().flat_map(|u| u.to_le_bytes()));
        }
    }
    v
}

/// The definition behind a picture header, as Word writes it.
fn with_picf(ff: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 0x44];
    v[..4].copy_from_slice(&((0x44 + ff.len()) as u32).to_le_bytes());
    v[4..6].copy_from_slice(&0x44u16.to_le_bytes());
    v.extend_from_slice(ff);
    v
}

/// One field whose code holds the form character: CP 11 is U+0001 in
/// `\u{13} FORMxxxx \u{1}\u{14}`.
fn doc_with(instr: &'static str, data: Vec<u8>, flagged: bool, location: u32) -> Vec<u8> {
    let text: &'static str = match instr {
        "FORMTEXT" => "\u{13} FORMTEXT \u{1}\u{14}Result\u{15}\r",
        "FORMCHECKBOX" => "\u{13} FORMCHECKBOX \u{1}\u{14}\u{15}\r",
        _ => "\u{13} FORMDROPDOWN \u{1}\u{14}\u{15}\r",
    };
    let at = text.chars().position(|c| c == '\u{1}').unwrap_or(0) as u32;
    let ccp = text.chars().count() as u32;
    let mut form = vec![(0x6A03u16, location.to_le_bytes().to_vec())];
    if flagged {
        form.push((0x0806, vec![1]));
    }
    form.sort_by_key(|(op, _)| *op);
    let operands: Vec<(u16, &[u8])> = form.iter().map(|(op, b)| (*op, b.as_slice())).collect();
    let spec = StructSpec {
        data_stream: data,
        chpx_runs: vec![(0, Vec::new()), (at, grpprl(&operands)), (at + 1, Vec::new())],
        ..structured_spec(text, ccp)
    };
    struct_doc(&spec)
}

fn field_of(doc: &craft_wordmodel::Document) -> (String, String, String) {
    for b in &doc.body {
        for o in b.as_para().map(|p| p.objects.as_slice()).unwrap_or_default() {
            if let InlineObject::Field { instr, result, form, .. } = o {
                return (instr.clone(), result.clone(), form.clone());
            }
        }
    }
    panic!("no field in {:?}", doc.body.len());
}

fn form_of(instr: &'static str, data: Vec<u8>) -> String {
    let doc = read(&doc_with(instr, data, true, 0)).expect("opens");
    let (i, _, form) = field_of(&doc);
    assert_eq!(i, instr);
    form
}

const TEXT: Ff = Ff {
    bits: 1 << 7,
    max_len: 40,
    hps: 20,
    name: "Full<Name>&",
    text_default: "Type \"a\" name",
    default: 0,
    help: "Enter name",
    exit_macro: "Check",
    entries: &[],
};
const TEXT_XML: &str = concat!(
    r#"<w:ffData><w:name w:val="Full&lt;Name&gt;&amp;"/><w:enabled/><w:calcOnExit w:val="0"/><w:exitMacro w:val="Check"/>"#,
    r#"<w:helpText w:type="text" w:val="Enter name"/>"#,
    r#"<w:textInput><w:default w:val="Type &quot;a&quot; name"/><w:maxLength w:val="40"/></w:textInput></w:ffData>"#
);

#[test]
fn a_text_field_keeps_its_name_default_and_limit() {
    assert_eq!(form_of("FORMTEXT", with_picf(&ffdata_bytes(&TEXT))), TEXT_XML);
}

#[test]
fn a_definition_without_a_picture_header_is_read_too() {
    assert_eq!(form_of("FORMTEXT", ffdata_bytes(&TEXT)), TEXT_XML);
}

#[test]
fn a_check_box_keeps_its_state_and_size() {
    let checked = Ff { bits: 1 | (1 << 2) | (1 << 9) | (1 << 10), hps: 24, name: "Agree", default: 1, ..Ff::default() };
    assert_eq!(
        form_of("FORMCHECKBOX", with_picf(&ffdata_bytes(&checked))),
        concat!(
            r#"<w:ffData><w:name w:val="Agree"/><w:enabled w:val="0"/><w:calcOnExit w:val="0"/>"#,
            r#"<w:checkBox><w:size w:val="24"/><w:default w:val="1"/><w:checked/></w:checkBox></w:ffData>"#
        )
    );
    // Word writes 0x19 as the result of a check box nobody ticked, and automatic size.
    let unticked = Ff { bits: 1 | (0x19 << 2), hps: 20, name: "Prior", ..Ff::default() };
    assert_eq!(
        form_of("FORMCHECKBOX", with_picf(&ffdata_bytes(&unticked))),
        concat!(
            r#"<w:ffData><w:name w:val="Prior"/><w:enabled/><w:calcOnExit w:val="0"/>"#,
            r#"<w:checkBox><w:sizeAuto/><w:default w:val="0"/></w:checkBox></w:ffData>"#
        )
    );
}

#[test]
fn an_unticked_check_box_whose_default_is_ticked_says_so() {
    // Without w:checked a reader uses w:default, which would tick the box.
    for res in [0u16, 0x19] {
        let unticked = Ff { bits: 1 | (res << 2), hps: 20, name: "Box", default: 1, ..Ff::default() };
        let form = form_of("FORMCHECKBOX", with_picf(&ffdata_bytes(&unticked)));
        assert!(form.contains(r#"<w:default w:val="1"/><w:checked w:val="0"/>"#), "{form}");
    }
}

#[test]
fn a_first_entry_choice_is_written_when_the_default_is_another_entry() {
    // An absent w:result would select the default entry.
    let first = Ff { bits: 2, default: 1, name: "S", entries: &["One", "Two"], ..Ff::default() };
    let form = form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&first)));
    assert!(form.contains(r#"<w:ddList><w:result w:val="0"/><w:default w:val="1"/>"#), "{form}");
    // With the first entry as the default Word writes neither.
    let plain = Ff { bits: 2, name: "S", entries: &["One", "Two"], ..Ff::default() };
    let form = form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&plain)));
    assert!(form.contains("<w:ddList><w:listEntry"), "{form}");
}

#[test]
fn a_definition_of_another_kind_is_not_attached_to_a_field() {
    let check = Ff { bits: 1, name: "Box", ..Ff::default() };
    assert_eq!(form_of("FORMTEXT", with_picf(&ffdata_bytes(&check))), "");
    assert_eq!(form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&check))), "");
    assert!(form_of("FORMCHECKBOX", with_picf(&ffdata_bytes(&check))).contains("<w:checkBox>"));
}

#[test]
fn a_picture_header_that_is_not_what_word_writes_gives_no_definition() {
    let ff = ffdata_bytes(&TEXT);
    // A header of another size, a total that is not past the header, and a record clipped by the stream.
    let mut size = with_picf(&ff);
    size[4..6].copy_from_slice(&0x40u16.to_le_bytes());
    let mut empty = with_picf(&ff);
    empty[..4].copy_from_slice(&0x44u32.to_le_bytes());
    // The record says it is longer than the stream, though a whole definition is there.
    let mut clipped = with_picf(&ff);
    let total = (clipped.len() + 10) as u32;
    clipped[..4].copy_from_slice(&total.to_le_bytes());
    for (what, data) in [("size", size), ("empty", empty), ("clipped", clipped)] {
        assert_eq!(form_of("FORMTEXT", data), "", "{what}");
    }
}

#[test]
fn values_a_form_field_cannot_hold_give_no_definition() {
    // 26 entries: 25 is "no choice", not the 26th entry.
    let names: Vec<&'static str> = (0..26).map(|_| "x").collect();
    let entries: &'static [&'static str] = Box::leak(names.into_boxed_slice());
    let none = Ff { bits: 2 | (25 << 2), name: "D", entries, ..Ff::default() };
    let form = form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&none)));
    assert!(form.contains("<w:ddList>") && !form.contains("<w:result"), "{form}");
    // A check box default other than 0 or 1, a state other than 0, 1 or 25, and a reserved text type.
    let default = Ff { bits: 1, name: "C", default: 2, ..Ff::default() };
    assert_eq!(form_of("FORMCHECKBOX", with_picf(&ffdata_bytes(&default))), "");
    let state = Ff { bits: 1 | (7 << 2), name: "C", ..Ff::default() };
    assert_eq!(form_of("FORMCHECKBOX", with_picf(&ffdata_bytes(&state))), "");
    let reserved = Ff { bits: 6 << 11, name: "T", ..Ff::default() };
    assert_eq!(form_of("FORMTEXT", with_picf(&ffdata_bytes(&reserved))), "");
}

#[test]
fn a_definition_is_measured_with_the_escapes_the_docx_reader_uses() {
    // An apostrophe is six bytes (&apos;) once the .docx reader serialises it.
    let entry: &'static str = Box::leak("'".repeat(1000).into_boxed_str());
    let many: &'static [&'static str] = Box::leak(vec![entry; 11].into_boxed_slice());
    let over = Ff { bits: 2, name: "A", entries: many, ..Ff::default() };
    assert_eq!(form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&over))), "");
    let few: &'static [&'static str] = Box::leak(vec![entry; 9].into_boxed_slice());
    let under = Ff { bits: 2, name: "A", entries: few, ..Ff::default() };
    let form = form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&under)));
    assert!(form.contains("&apos;") && form.len() <= 64 * 1024, "{}", form.len());
}

#[test]
fn tab_and_line_breaks_in_form_text_are_kept_as_character_references() {
    let f = Ff { name: "T", text_default: "A\tB\nC\rD\u{1}E", ..Ff::default() };
    let form = form_of("FORMTEXT", with_picf(&ffdata_bytes(&f)));
    // The other control character is still dropped.
    assert!(form.contains(r#"<w:default w:val="A&#9;B&#10;C&#13;DE"/>"#), "{form}");
}

#[test]
fn a_text_field_keeps_its_kind() {
    for (n, name) in [(1u16, "number"), (2, "date"), (3, "currentDate"), (4, "currentTime"), (5, "calculated")] {
        let f = Ff { bits: n << 11, name: "T", ..Ff::default() };
        let form = form_of("FORMTEXT", with_picf(&ffdata_bytes(&f)));
        assert!(form.contains(&format!(r#"<w:textInput><w:type w:val="{name}"/></w:textInput>"#)), "{form}");
    }
    let regular = form_of("FORMTEXT", with_picf(&ffdata_bytes(&Ff { name: "T", ..Ff::default() })));
    assert!(!regular.contains("w:type"), "{regular}");
}

#[test]
fn a_definition_the_docx_reader_would_drop_is_not_produced() {
    // 25 entries of 1,000 ampersands escape to about 122 KiB, past what the .docx reader keeps.
    let entry: &'static str = Box::leak("&".repeat(1000).into_boxed_str());
    let entries: &'static [&'static str] = Box::leak(vec![entry; 25].into_boxed_slice());
    let big = Ff { bits: 2, name: "Big", entries, ..Ff::default() };
    assert_eq!(form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&big))), "");
    // Just under the limit is kept.
    let entries: &'static [&'static str] = Box::leak(vec![entry; 10].into_boxed_slice());
    let small = Ff { bits: 2, name: "Small", entries, ..Ff::default() };
    assert!(form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&small))).contains("<w:ddList>"));
}

const DROP: Ff = Ff {
    bits: 2 | (2 << 2) | (1 << 14) | (1 << 15),
    max_len: 0,
    hps: 20,
    name: "Status",
    text_default: "",
    default: 1,
    help: "",
    exit_macro: "",
    entries: &["One", "Two", "Th\u{e9}ree & <more>"],
};

#[test]
fn a_drop_down_keeps_its_entries_and_choice() {
    assert_eq!(
        form_of("FORMDROPDOWN", with_picf(&ffdata_bytes(&DROP))),
        concat!(
            r#"<w:ffData><w:name w:val="Status"/><w:enabled/><w:calcOnExit w:val="1"/><w:ddList><w:result w:val="2"/><w:default w:val="1"/>"#,
            "<w:listEntry w:val=\"One\"/><w:listEntry w:val=\"Two\"/><w:listEntry w:val=\"Th\u{e9}ree &amp; &lt;more&gt;\"/></w:ddList></w:ffData>"
        )
    );
}

/// Every cut of a definition, however short, leaves the field without one and does not panic.
#[test]
fn a_truncated_definition_leaves_the_form_empty() {
    for (instr, ff) in [("FORMTEXT", &TEXT), ("FORMDROPDOWN", &DROP)] {
        let full = with_picf(&ffdata_bytes(ff));
        assert!(!form_of(instr, full.clone()).is_empty());
        for n in 1..full.len() {
            // The picture header still claims the whole definition.
            let doc = read(&doc_with(instr, full[..n].to_vec(), true, 0)).expect("opens");
            assert_eq!(field_of(&doc).2, "", "{instr} cut at {n}");
        }
    }
}

#[test]
fn a_hostile_definition_leaves_the_form_empty() {
    let ok = ffdata_bytes(&DROP);
    // Declared lengths far past the stream: the name, the entry count, an entry, the picture header.
    let mut name = ok.clone();
    name[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut count = ok.clone();
    let sttb = 6 + DROP.entries.iter().map(|e| 2 + 2 * e.encode_utf16().count()).sum::<usize>();
    let entries_at = count.len() - sttb;
    count[entries_at + 2..entries_at + 4].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut entry = ok.clone();
    entry[entries_at + 6..entries_at + 8].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut picf = with_picf(&ok);
    picf[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    picf[4..6].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut version = ok.clone();
    version[..4].copy_from_slice(&[1, 0, 0, 0]);
    let mut kind = ok.clone();
    kind[4] |= 3; // iType 3 is none of the three
    for (what, data) in [("name", name), ("count", count), ("entry", entry), ("picf", picf), ("version", version), ("kind", kind)] {
        assert_eq!(form_of("FORMDROPDOWN", data), "", "{what}");
    }
    // A location past the end of the stream.
    for location in [u32::MAX, 0x7FFF_FFFF, 5000] {
        let doc = read(&doc_with("FORMTEXT", with_picf(&ffdata_bytes(&TEXT)), true, location)).expect("opens");
        assert_eq!(field_of(&doc), ("FORMTEXT".into(), "Result".into(), String::new()), "location {location}");
    }
    // Every byte of the stream set: not an FFData, and not a panic.
    let doc = read(&doc_with("FORMTEXT", vec![0xFF; 4096], true, 0)).expect("opens");
    assert_eq!(field_of(&doc).2, "");
}

/// Without `sprmCFData` the location names a picture, so the definition is not read.
#[test]
fn a_character_not_marked_as_form_data_gets_no_form() {
    let doc = read(&doc_with("FORMTEXT", with_picf(&ffdata_bytes(&TEXT)), false, 0)).expect("opens");
    assert_eq!(field_of(&doc).2, "");
}

#[test]
fn a_definition_is_charged_to_the_work_budget() {
    let data = with_picf(&ffdata_bytes(&DROP));
    let plenty = Work::new(u64::MAX);
    assert!(ffdata::read(&data, 0, &plenty).is_some());
    let read_and_copied = u64::MAX - plenty.left.get();
    assert!(read_and_copied > ffdata_bytes(&DROP).len() as u64 - 4, "{read_and_copied}");
    for budget in [0, 10, read_and_copied - 1] {
        let work = Work::new(budget);
        assert_eq!(ffdata::read(&data, 0, &work), None, "budget {budget}");
        assert_eq!(work.dropped(), [Cost::FormData]);
    }
    assert!(ffdata::read(&data, 0, &Work::new(read_and_copied)).is_some());
}

/// A converted file keeps the definitions: the `.docx` written from the `.doc` holds the same
/// `w:ffData` in each field's begin, and reading that `.docx` gives the fields their forms back.
#[test]
fn form_fields_survive_conversion_to_docx() {
    let doc = read(&doc_with("FORMTEXT", with_picf(&ffdata_bytes(&TEXT)), true, 0)).expect("opens");
    let docx = craft_docx::write(&doc).expect("writes");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&docx)).expect("a package");
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").expect("document part"), &mut xml).expect("text");
    assert!(xml.contains(&format!(r#"<w:fldChar w:fldCharType="begin">{TEXT_XML}</w:fldChar>"#)), "{xml}");
    let (instr, result, form) = field_of(&craft_docx::read(&docx).expect("reads back"));
    assert_eq!((instr.as_str(), result.as_str(), form.as_str()), ("FORMTEXT", "Result", TEXT_XML));
}
