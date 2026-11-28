//! Original synthetic records from [MS-DOC] sections 2.6.4 (section properties),
//! 2.2.5 (property storage), and 2.8.26 (PlcfSed), using the existing StructSpec
//! compound-file builder. No proprietary application or predecessor fixture is used.
//! https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/46c3ec54-53ff-4c0a-b0d6-07ad15d2546e

use craft_wordmodel::Document;
use craft_wordmodel::section::NumFormat;

use super::{grpprl, one_section, struct_doc};
use crate::read;

fn two_sections(first: Vec<u8>, last: Vec<u8>) -> Document {
    // A nonfinal section ends in a section mark, not an ordinary paragraph mark.
    let mut spec = one_section("Lead\u{C}Body\r", last, vec![(0, 0, Vec::new()), (5, 0, Vec::new())]);
    spec.sections.insert(0, (5, first));
    read(&struct_doc(&spec)).expect("original two-section DOC opens")
}

#[test]
fn restart_and_format_survive_both_section_positions() {
    let first = grpprl(&[(0x300E, &[2]), (0x3011, &[1]), (0x501C, &13u16.to_le_bytes()), (0x7044, &70_007u32.to_le_bytes())]);
    // The legacy-only start is unsigned, even above the writer's recommended range.
    let last = grpprl(&[(0x3011, &[1]), (0x501C, &40_000u16.to_le_bytes()), (0x300E, &[3])]);
    let doc = two_sections(first, last);
    assert_eq!(super::body_texts(&doc), ["Lead", "Body"]);
    let first = doc.body[0].as_para().and_then(|p| p.section.as_ref()).expect("first section is attached to its last paragraph");
    assert_eq!(first.page_num_start, Some(70_007), "the later full-width start overrides the legacy value");
    assert_eq!(first.page_num_format, NumFormat::LowerRoman);
    assert_eq!(doc.last_section.page_num_start, Some(40_000), "the final section keeps its own unsigned legacy start");
    assert_eq!(doc.last_section.page_num_format, NumFormat::UpperLetter);
}

#[test]
fn continuation_does_not_apply_start_operands() {
    for (label, properties) in [
        ("no restart property", grpprl(&[(0x501C, &17u16.to_le_bytes()), (0x7044, &70_007u32.to_le_bytes())])),
        ("explicit continuation", grpprl(&[(0x3011, &[0]), (0x7044, &70_007u32.to_le_bytes())])),
        ("later continuation cancels restart", grpprl(&[(0x3011, &[1]), (0x501C, &17u16.to_le_bytes()), (0x3011, &[0])])),
    ] {
        let doc = two_sections(grpprl(&[(0x3011, &[1]), (0x501C, &13u16.to_le_bytes())]), properties);
        assert_eq!(doc.last_section.page_num_start, None, "{label}");
        assert_eq!(doc.last_section.page_num_format, NumFormat::Decimal, "{label}");
    }
}

#[test]
fn format_only_continuation_and_restart_without_start_are_independent() {
    let doc = two_sections(grpprl(&[(0x300E, &[4]), (0x7044, &70_007u32.to_le_bytes())]), grpprl(&[(0x3011, &[1])]));
    let first = doc.body[0].as_para().and_then(|p| p.section.as_ref()).expect("first section");
    assert_eq!(first.page_num_start, None, "a numbering format alone does not restart the section");
    assert_eq!(first.page_num_format, NumFormat::LowerLetter);
    assert_eq!(doc.last_section.page_num_start, Some(0), "an explicit restart without a start uses the section-property default");
    assert_eq!(doc.last_section.page_num_format, NumFormat::Decimal, "the previous section's format does not leak");
}

#[test]
fn only_valid_properties_in_encounter_order_choose_the_restart() {
    for (label, properties, expected) in [
        ("start before restart flag", grpprl(&[(0x501C, &13u16.to_le_bytes()), (0x3011, &[1])]), Some(13)),
        // A nonconforming version order uses the local last-valid-value recovery policy.
        // It must not become an unconditional preference for the 32-bit opcode.
        (
            "last valid start survives an invalid full-width value",
            grpprl(&[(0x3011, &[1]), (0x7044, &70_007u32.to_le_bytes()), (0x501C, &17u16.to_le_bytes()), (0x7044, &0x7FFF_FFFFu32.to_le_bytes())]),
            Some(17),
        ),
        ("invalid Bool8 does not cancel a valid restart", grpprl(&[(0x3011, &[1]), (0x501C, &23u16.to_le_bytes()), (0x3011, &[2])]), Some(23)),
        ("invalid Bool8 does not turn continuation into a restart", grpprl(&[(0x3011, &[0]), (0x501C, &23u16.to_le_bytes()), (0x3011, &[2])]), None),
    ] {
        let doc = two_sections(Vec::new(), properties);
        assert_eq!(doc.last_section.page_num_start, expected, "{label}");
    }
}
