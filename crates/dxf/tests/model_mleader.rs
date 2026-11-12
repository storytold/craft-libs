//! MULTILEADER and MLEADERSTYLE.

use craft_dxf::model::mleader::{MAX_ROOTS, MAX_VERTICES, MLeader, MLeaderStyle};
use craft_dxf::model::write::{Drawing, Format};
use craft_dxf::model::{Document, Entity, EntityKind};
use craft_dxf::{Tag, parse};

fn tags(text: &str) -> Vec<Tag> {
    let lines: Vec<&str> = text.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).map(|l| if l == "~" { "" } else { l }).collect();
    parse((lines.join("\n") + "\n").as_bytes()).expect("the test drawing parses")
}

const MLEADER: &str = "0|MULTILEADER|5|80|100|AcDbEntity|8|0|100|AcDbMLeader|270|2
300|CONTEXT_DATA{|40|1.0|10|5.0|20|5.0|30|0.0|41|2.5|140|0.18|145|0.09|174|1|175|1|290|1|304|first^Jsecond|11|0.0|21|0.0|31|1.0|340|11
12|6.0|22|5.0|32|0.0|13|1.0|23|0.0|33|0.0|42|0.0|43|20.0|45|1.25|170|2|171|1|296|0|110|0.0|120|0.0|130|0.0
302|LEADER{|290|1|291|1|10|4.0|20|5.0|30|0.0|11|1.0|21|0.0|31|0.0|90|0|40|0.36
304|LEADER_LINE{|10|0.0|20|0.0|10|2.0|20|3.0|30|1.0|91|0|305|}
304|LEADER_LINE{|10|0.0|20|9.0|305|}
303|}
302|LEADER{|10|8.0|20|5.0|40|0.5|303|}
301|}
340|1A|90|0|170|1|91|0|171|0|290|1|291|1|41|0.36|42|0.18|172|2|343|11|173|1|95|1|174|0|175|0|176|0|178|0|179|1|45|1.0|271|0|272|9|273|9
330|AD|177|1|44|0.0|302|value one|330|AE|177|2|44|0.0|302|value two
";

#[test]
fn a_multileader_by_level() {
    let t = tags(MLEADER);
    let EntityKind::MLeader(m) = Entity::read(&craft_dxf::model::entity_records(&craft_dxf::model::records(&t)).0[0]).kind else {
        panic!("a multileader")
    };
    // Entity level, after the context block.
    assert_eq!(
        (m.style, m.leader_type, m.dogleg_on, m.dogleg, m.arrow_size, m.content_type),
        (Some(0x1A), Some(1), Some(true), Some(0.36), Some(0.18), Some(2))
    );
    assert_eq!((m.text_style, m.left_attachment, m.right_attachment), (Some(0x11), Some(1), Some(1)));
    assert_eq!(m.attribs, [(0xAD, "value one".to_string()), (0xAE, "value two".to_string())]);
    // The context's own 10, 41, 340 and 170 are not the entity's.
    assert_eq!((m.base, m.text_height, m.context_arrow_size, m.has_text), (Some([5.0, 5.0, 0.0]), Some(2.5), Some(0.18), Some(true)));
    assert_eq!(
        (m.text.as_deref(), m.context_text_style, m.text_location, m.text_direction),
        (Some("first\nsecond"), Some(0x11), Some([6.0, 5.0, 0.0]), Some([1.0, 0.0, 0.0]))
    );
    assert_eq!(
        (m.text_width, m.line_spacing, m.line_spacing_style, m.text_alignment, m.has_block),
        (Some(20.0), Some(1.25), Some(2), Some(1), Some(false))
    );
    assert_eq!((m.context_left_attachment, m.context_right_attachment, m.block_scale), (Some(1), Some(1), None));
    // Roots with their landings, doglegs and lines; the second line's vertex has no z.
    assert_eq!(m.roots.len(), 2);
    let r = &m.roots[0];
    assert_eq!((r.landing, r.direction, r.dogleg), (Some([4.0, 5.0, 0.0]), Some([1.0, 0.0, 0.0]), Some(0.36)));
    assert_eq!(r.lines, [vec![[0.0, 0.0, 0.0], [2.0, 3.0, 1.0]], vec![[0.0, 9.0, 0.0]]]);
    assert_eq!((m.roots[1].landing, m.roots[1].dogleg, m.roots[1].lines.len()), (Some([8.0, 5.0, 0.0]), Some(0.5), 0));
}

#[test]
fn the_caps() {
    let mut text = String::from("0|MULTILEADER|100|AcDbMLeader|300|CONTEXT_DATA{");
    for _ in 0..MAX_ROOTS + 5 {
        text.push_str("|302|LEADER{|10|1.0|303|}");
    }
    text.push_str("|302|LEADER{|304|LEADER_LINE{");
    for i in 0..MAX_VERTICES + 5 {
        text.push_str(&format!("|10|{i}.0"));
    }
    text.push_str("|305|}|303|}|301|}");
    let t = tags(&text);
    let m = MLeader::read(&t[1..]);
    assert_eq!(m.roots.len(), MAX_ROOTS);
    assert!(m.roots.iter().all(|r| r.lines.is_empty()), "the root past the cap and its lines are left out");
    let mut text = String::from("0|MULTILEADER|100|AcDbMLeader|300|CONTEXT_DATA{|302|LEADER{|304|LEADER_LINE{");
    for i in 0..MAX_VERTICES + 5 {
        text.push_str(&format!("|10|{i}.0"));
    }
    let t = tags(&text);
    let m = MLeader::read(&t[1..]);
    assert_eq!(m.roots[0].lines[0].len(), MAX_VERTICES, "a file that ends inside a line keeps what it has");
}

#[test]
fn damaged_multileaders() {
    for text in [
        "0|MULTILEADER",
        "0|MULTILEADER|300|CONTEXT_DATA{|302|LEADER{|10|1.0",
        "0|MULTILEADER|305|}|303|}|301|}|304|x",
        "0|MULTILEADER|330|0|302|orphan",
    ] {
        let t = tags(text);
        let m = MLeader::read(&t[1..]);
        assert!(m.attribs.is_empty());
    }
    let t = tags("0|MULTILEADER|300|CONTEXT_DATA{|302|LEADER{|10|1.0|20|2.0");
    assert_eq!(MLeader::read(&t[1..]).roots[0].landing, Some([1.0, 2.0, 0.0]), "a root the file ends in keeps its landing");
}

#[test]
fn a_multileader_style() {
    let t = tags(
        "0|SECTION|2|OBJECTS|0|MLEADERSTYLE|5|1A|100|AcDbMLeaderStyle|179|2|170|2|171|1|172|0|90|2|40|0.0|41|0.0|173|1|91|-1056964608|92|-2|290|1|42|2.0|291|1|43|8.0|3|Standard|341|0|44|2.5|300|~|342|11|174|1|175|1|176|0|178|1|45|2.5|343|0|296|1|0|ENDSEC|0|EOF",
    );
    let d = Document::read(&t);
    let s = MLeaderStyle::read(d.objects_of("MLEADERSTYLE").next().expect("the style"));
    assert_eq!((s.handle, s.description.as_str(), s.content_type, s.leader_type), (Some(0x1A), "Standard", 2, 1));
    assert_eq!(
        (s.landing_gap, s.dogleg, s.arrow_size, s.text_height, s.text_style, s.block, s.annotative),
        (2.0, 8.0, 2.5, 2.5, Some(0x11), None, true)
    );
}

#[test]
fn the_writer_leaves_a_multileader_out_before_2007() {
    let t = tags(MLEADER);
    let e = Entity::read(&craft_dxf::model::entity_records(&craft_dxf::model::records(&t)).0[0]);
    for (format, left_out) in [(Format::R2004, 1), (Format::R2007, 0)] {
        let (_, skipped) = Drawing { format, entities: vec![e.clone()], ..Default::default() }.tags();
        assert_eq!(skipped, left_out, "{format:?}");
    }
}
