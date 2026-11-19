//! The drawing model's entities: common properties, geometry, text, hatches, dimensions.

use craft_dxf::model::text::{MTextColumns, decode_contents, decode_line, encode_contents, encode_line, map_text_groups};
use craft_dxf::model::{Color, Document, Entity, EntityKind, HatchEdge, HatchPath, Lineweight, Transparency, entity_records, records};
use craft_dxf::{Tag, parse};

fn tags(text: &str) -> Vec<Tag> {
    let lines: Vec<&str> = text.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).map(|l| if l == "~" { "" } else { l }).collect();
    parse((lines.join("\n") + "\n").as_bytes()).expect("the test drawing parses")
}

/// The entities of `text`, the records of an ENTITIES section.
fn entities(text: &str) -> Vec<Entity> {
    let t = tags(text);
    let (recs, strays) = entity_records(&records(&t));
    assert_eq!(strays, 0);
    recs.iter().map(Entity::read).collect()
}

fn one(text: &str) -> Entity {
    let mut e = entities(text);
    assert_eq!(e.len(), 1);
    e.remove(0)
}

#[test]
fn common_properties() {
    let c = one("0|LINE|5|2A|330|1F|100|AcDbEntity|8|Walls|6|DASHED|62|1|420|65280|370|50|48|2.5|60|1|67|1|440|33554687|390|B|100|AcDbLine|39|1.5|210|0.0|220|0.0|230|-1.0").common;
    assert_eq!((c.handle, c.owner, c.layer.as_str(), c.linetype.as_deref()), (Some(0x2A), Some(0x1F), "Walls", Some("DASHED")));
    assert_eq!((c.color, c.lineweight, c.ltscale), (Color::Rgb([0, 255, 0]), Lineweight::Hundredths(50), 2.5));
    assert!(!c.visible && c.paper_space);
    assert_eq!((c.transparency, c.thickness, c.extrusion, c.plot_style), (Transparency::Alpha(255), 1.5, [0.0, 0.0, -1.0], Some(0xB)));
    let d = one("0|LINE").common;
    assert_eq!((d.handle, d.owner, d.layer.as_str(), d.linetype.as_deref(), d.color), (None, None, "0", None, Color::ByLayer));
    assert_eq!((d.lineweight, d.ltscale, d.visible, d.paper_space, d.extrusion), (Lineweight::ByLayer, 1.0, true, false, [0.0, 0.0, 1.0]));
    assert_eq!(one("0|LINE|6|BYLAYER|48|-1.0|62|-2").common, d, "ByLayer by name, a negative scale and a negative colour are the defaults");
}

#[test]
fn entity_properties_are_not_read_from_the_entitys_own_subclass() {
    // An MTEXT's 48 is its column width, not the linetype scale, and a marker in an
    // application's braces is the application's: it does not end the entity's groups. A record with no AcDbEntity marker has its
    // layer wherever it is.
    let c = one("0|MTEXT|102|{APP|100|AcDbMText|102|}|100|AcDbEntity|8|A|100|AcDbMText|48|7.0|62|3").common;
    assert_eq!((c.ltscale, c.color, c.layer.as_str()), (1.0, Color::ByLayer, "A"));
    let c = one("0|LINE|100|AcDbLine|8|B|62|3").common;
    assert_eq!((c.color, c.layer.as_str()), (Color::Index(3), "B"));
    // An embedded object ends the entity's groups.
    assert_eq!(one("0|ATTDEF|8|C|101|Embedded Object|62|5").common.color, Color::ByLayer);
}

#[test]
fn curves() {
    let k = |text: &str| one(text).kind;
    assert_eq!(k("0|LINE|10|1.0|20|2.0|30|3.0|11|4.0|21|5.0"), EntityKind::Line { start: [1.0, 2.0, 3.0], end: [4.0, 5.0, 0.0] });
    assert_eq!(k("0|POINT|10|1.0|50|30.0"), EntityKind::Point { location: [1.0, 0.0, 0.0], angle: 30.0 });
    assert_eq!(k("0|CIRCLE|10|1.0|20|2.0|40|3.0"), EntityKind::Circle { center: [1.0, 2.0, 0.0], radius: 3.0 });
    assert_eq!(k("0|ARC|10|1.0|40|3.0|50|10.0"), EntityKind::Arc { center: [1.0, 0.0, 0.0], radius: 3.0, start_angle: 10.0, end_angle: 360.0 });
    assert_eq!(
        k("0|ELLIPSE|10|1.0|11|2.0|21|0.0|40|0.5|41|0.0|42|3.0"),
        EntityKind::Ellipse { center: [1.0, 0.0, 0.0], major: [2.0, 0.0, 0.0], ratio: 0.5, start: 0.0, end: 3.0 }
    );
    assert_eq!(k("0|RAY|10|1.0|11|0.0|21|1.0"), EntityKind::Ray { base: [1.0, 0.0, 0.0], direction: [0.0, 1.0, 0.0] });
    assert_eq!(k("0|XLINE|10|1.0|11|0.0|21|1.0"), EntityKind::XLine { base: [1.0, 0.0, 0.0], direction: [0.0, 1.0, 0.0] });
    let EntityKind::Spline(s) = k(
        "0|SPLINE|70|8|71|2|40|0.0|40|0.0|40|1.0|10|0.0|20|0.0|10|1.0|20|1.0|30|5.0|10|2.0|41|1.0|41|2.0|41|1.0|11|9.0|21|9.0|12|1.0|22|0.0|44|0.01",
    ) else {
        panic!("a spline")
    };
    assert_eq!((s.degree, s.flags, s.knots.len(), s.weights.as_slice(), s.fit_tolerance), (2, 8, 3, [1.0, 2.0, 1.0].as_slice(), 0.01));
    assert_eq!(s.control, [[0.0, 0.0, 0.0], [1.0, 1.0, 5.0], [2.0, 0.0, 0.0]], "the third point has no y or z of its own");
    assert_eq!((s.fit.as_slice(), s.start_tangent, s.end_tangent), ([[9.0, 9.0, 0.0]].as_slice(), Some([1.0, 0.0, 0.0]), None));
}

#[test]
fn polylines() {
    let EntityKind::LwPolyline(p) = one("0|LWPOLYLINE|90|3|70|129|43|0.5|38|2.0|10|0.0|20|0.0|42|1.0|10|4.0|20|0.0|40|0.1|41|0.2|10|4.0|20|3.0").kind
    else {
        panic!("a lightweight polyline")
    };
    assert!(p.closed && p.plinegen);
    assert_eq!((p.const_width, p.elevation, p.vertices.len()), (0.5, 2.0, 3));
    assert_eq!((p.vertices[0].bulge, p.vertices[1].start_width, p.vertices[1].end_width, p.vertices[2].y), (1.0, 0.1, 0.2, 3.0));
    // A polyface mesh: three vertices and a face record that names them.
    let e = one(
        "0|POLYLINE|66|1|70|64|71|3|72|1|10|0.0|20|0.0|30|7.0|0|VERTEX|10|0.0|20|0.0|30|0.0|70|192|0|VERTEX|10|1.0|70|192|0|VERTEX|10|1.0|20|1.0|70|192|0|VERTEX|70|128|71|1|72|2|73|-3|0|SEQEND",
    );
    let EntityKind::Polyline(m) = e.kind else { panic!("a polyline") };
    assert!(m.is_polyface() && !m.is_mesh() && !m.is_3d() && !m.closed());
    assert_eq!((m.m, m.n, m.elevation, m.vertices.len()), (3, 1, 7.0, 4));
    assert!(m.vertices[3].is_face() && !m.vertices[0].is_face());
    assert_eq!(m.vertices[3].face, [1, 2, -3, 0]);
    let EntityKind::Polyline(h) = one("0|POLYLINE|70|4|40|0.5|41|0.25|0|VERTEX|10|1.0|42|0.5|40|0.1|41|0.2|70|16|0|SEQEND").kind else {
        panic!("a polyline")
    };
    assert_eq!((h.start_width, h.end_width), (0.5, 0.25));
    let v = h.vertices[0];
    assert!(v.is_spline_frame());
    assert_eq!((v.bulge, v.start_width, v.end_width), (0.5, 0.1, 0.2));
}

#[test]
fn text_and_attributes() {
    let EntityKind::Text(t) = one("0|TEXT|10|1.0|20|2.0|11|3.0|21|4.0|40|2.5|1|10^ 2^Im|50|90.0|41|0.8|51|15.0|7|Notes|71|2|72|1|73|2").kind else {
        panic!("text")
    };
    assert_eq!((t.insert, t.align, t.height, t.value.as_str()), ([1.0, 2.0, 0.0], Some([3.0, 4.0, 0.0]), 2.5, "10^2\tm"));
    assert_eq!((t.rotation, t.width_factor, t.oblique, t.style.as_str(), t.generation, t.halign, t.valign), (90.0, 0.8, 15.0, "Notes", 2, 1, 2));
    let EntityKind::Text(d) = one("0|TEXT|1|x").kind else { panic!("text") };
    assert_eq!((d.align, d.width_factor, d.style.as_str(), d.halign, d.valign), (None, 1.0, "Standard", 0, 0));

    let EntityKind::AttDef(a) =
        one("0|ATTDEF|100|AcDbText|10|1.0|1|default|100|AcDbAttributeDefinition|280|0|3|Enter^Ia|2|TAG|70|12|74|3|280|1").kind
    else {
        panic!("an attribute definition")
    };
    assert_eq!((a.tag.as_str(), a.prompt.as_str(), a.flags, a.text.valign, a.lock_position), ("TAG", "Enter\ta", 12, 3, true));
    assert!(a.multiline.is_none());

    // A block reference with two attributes; the second is multiline.
    let EntityKind::Insert(i) = one("0|INSERT|66|1|2|DOOR|10|1.0|20|2.0|41|2.0|43|-1.0|50|45.0|70|3|71|0|44|5.0|45|6.0
         0|ATTRIB|8|Tags|62|2|1|one|2|A|70|1|280|0
         0|ATTRIB|1|~|2|B|101|Embedded Object|10|7.0|40|2.0|71|5|3|line one^J|1|line two
         0|SEQEND")
    .kind
    else {
        panic!("an insert")
    };
    assert_eq!((i.block.as_str(), i.insert, i.scale, i.rotation), ("DOOR", [1.0, 2.0, 0.0], [2.0, 1.0, -1.0], 45.0));
    assert_eq!((i.columns, i.rows, i.column_spacing, i.row_spacing), (3, 1, 5.0, 6.0));
    assert_eq!(i.attribs.len(), 2);
    let (c, a) = &i.attribs[0];
    assert_eq!(
        (c.layer.as_str(), c.color, a.tag.as_str(), a.text.value.as_str(), a.flags, a.lock_position),
        ("Tags", Color::Index(2), "A", "one", 1, false)
    );
    assert!(a.prompt.is_empty(), "an ATTRIB has no prompt");
    let m = i.attribs[1].1.multiline.as_ref().expect("embedded MTEXT");
    assert_eq!((m.contents.as_str(), m.insert, m.height, m.attach), ("line one\nline two", [7.0, 0.0, 0.0], 2.0, 5));
}

#[test]
fn caret_encoding() {
    assert_eq!(decode_contents("a^Ib^Jc^ d^2^"), "a\tb\nc^d^2^");
    assert_eq!(decode_line("a^Ib^Jc^ d"), "a\tb^Jc^d", "a single line has no line feed");
    assert_eq!(encode_contents("a\tb\nc^d\u{7}"), "a^Ib^Jc^ d", "other control characters are dropped");
    assert_eq!(encode_line("10^2\t"), "10^ 2^I");
    for s in ["", "plain", "10^2", "tab\there", "^^ ^I"] {
        assert_eq!(decode_line(&encode_line(s)), s);
        assert_eq!(decode_contents(&encode_contents(s)), s);
    }
    let mut t = vec![Tag::s(1, "a^ b"), Tag::s(3, "p^ q"), Tag::i(101, 0), Tag::s(1, "e^ f")];
    map_text_groups("attdef", &mut t, decode_line);
    assert_eq!(t.iter().map(Tag::str).collect::<Vec<_>>(), ["a^b", "p^q", "0", "e^ f"], "the embedded object's text is left alone");
    let mut t = vec![Tag::s(1, "a^ b"), Tag::s(3, "p^ q")];
    map_text_groups("TEXT", &mut t, decode_line);
    assert_eq!(t[1].str(), "p^ q", "only an ATTDEF's 3 is a prompt");
    map_text_groups("LINE", &mut t, |_| String::new());
    assert_eq!(t[0].str(), "a^b");
}

#[test]
fn mtext() {
    let EntityKind::MText(m) = one("0|MTEXT|10|1.0|20|2.0|40|2.5|41|40.0|71|5|72|1|3|first ^Ipart, |3|second, |1|last\\Pline|7|Notes|11|0.0|21|1.0|50|30.0|44|1.2|73|2|90|1|63|4|45|1.25").kind else {
        panic!("multiline text")
    };
    assert_eq!((m.insert, m.height, m.width, m.attach, m.style.as_str()), ([1.0, 2.0, 0.0], 2.5, 40.0, 5, "Notes"));
    assert_eq!(m.contents, "first \tpart,second,last\\Pline");
    assert_eq!((m.direction, m.rotation, m.line_spacing, m.line_spacing_exact), (Some([0.0, 1.0, 0.0]), 30.0, 1.2, true));
    assert!((m.angle() - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "the direction wins over the rotation");
    assert_eq!(m.background, Some((1, Color::Index(4), 1.25)));
    assert!(m.columns.is_none());
    let EntityKind::MText(r) = one("0|MTEXT|1|x|50|90.0|11|0.0|21|0.0|71|99").kind else { panic!("multiline text") };
    assert!((r.angle() - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "a zero direction is none");
    assert_eq!((r.attach, r.background, r.line_spacing), (9, None, 1.0));
}

#[test]
fn mtext_columns() {
    // Legacy groups: static, 2 columns, the first 50 after 75 counts the heights; the 50 before
    // 75 is the rotation.
    let t = tags("0|MTEXT|10|0.0|50|15.0|1|x|75|1|76|2|78|1|79|0|48|30.0|49|5.0|50|2|50|80.0|50|80.0");
    let c = MTextColumns::read(&t[1..]).expect("columns");
    assert_eq!((c.dynamic, c.count, c.width, c.gutter, c.height, c.auto_height, c.flow_reversed), (false, 2, 30.0, 5.0, 80.0, false, true));
    assert!(c.heights.is_empty(), "only manual dynamic columns keep each height");
    let EntityKind::MText(m) = one("0|MTEXT|10|0.0|50|15.0|1|x|75|1|76|2|48|30.0|49|5.0|50|2|50|80.0|50|80.0").kind else { panic!("multiline text") };
    assert_eq!(m.rotation, 15.0);
    // Dynamic with manual heights, and no count before them.
    let t = tags("0|MTEXT|1|x|75|2|76|3|79|0|48|30.0|49|5.0|50|10.0|50|40.0|50|20.0");
    let c = MTextColumns::read(&t[1..]).expect("columns");
    assert_eq!((c.dynamic, c.count, c.height, c.heights.as_slice()), (true, 3, 40.0, [10.0, 40.0, 20.0].as_slice()));
    // R2018: the embedded object; 72 is 0 for dynamic auto-height, the count follows the widths.
    let t = tags("0|MTEXT|1|x|46|50.0|101|Embedded Object|70|1|10|1.0|11|0.0|40|0.0|41|0.0|42|65.0|43|0.0|71|2|72|0|44|30.0|45|5.0|73|1|74|0");
    let c = MTextColumns::read(&t[1..]).expect("columns");
    assert_eq!((c.dynamic, c.count, c.width, c.gutter, c.height, c.auto_height), (true, 2, 30.0, 5.0, 50.0, true));
    // No columns: type 0, or no column groups at all.
    assert!(MTextColumns::read(&tags("0|MTEXT|1|x|75|0|76|0")[1..]).is_none());
    assert!(MTextColumns::read(&tags("0|MTEXT|1|x|101|Embedded Object|71|0")[1..]).is_none());
}

#[test]
fn the_column_height_limit() {
    let mut text = String::from("0|MTEXT|1|x|75|2|76|5000|79|0");
    for _ in 0..5000 {
        text.push_str("|50|1.0");
    }
    let t = tags(&text);
    let c = MTextColumns::read(&t[1..]).expect("columns");
    assert_eq!((c.heights.len(), c.count), (1000, 1000));
    let mut text = String::from("0|MTEXT|1|x|101|Embedded Object|71|2|72|40000|73|0");
    for _ in 0..5000 {
        text.push_str("|46|1.0");
    }
    let t = tags(&text);
    let c = MTextColumns::read(&t[1..]).expect("columns");
    assert_eq!((c.heights.len(), c.count), (1000, 1000));
    // A total width that asks for a count beyond any number.
    let t = tags("0|MTEXT|1|x|101|Embedded Object|71|2|72|0|73|1|42|1e300|44|1e-300|45|0.0");
    assert_eq!(MTextColumns::read(&t[1..]).expect("columns").count, 1000);
}

#[test]
fn dimensions_and_leaders() {
    let EntityKind::Dimension(d) =
        one("0|DIMENSION|2|*D1|3|ISO-25|10|1.0|20|2.0|11|3.0|21|4.0|12|0.5|13|5.0|14|6.0|15|7.0|16|8.0|70|161|1|<> ^Im|42|12.5|50|30.0|52|60.0|53|45.0|40|2.0").kind
    else {
        panic!("a dimension")
    };
    assert_eq!((d.kind(), d.flags & 128, d.flags & 32), (1, 128, 32));
    assert_eq!((d.block.as_deref(), d.style.as_str(), d.text.as_str(), d.measurement), (Some("*D1"), "ISO-25", "<> \tm", Some(12.5)));
    assert_eq!((d.defpt, d.text_mid, d.insertion[0]), ([1.0, 2.0, 0.0], [3.0, 4.0, 0.0], 0.5));
    assert_eq!((d.p13[0], d.p14[0], d.p15[0], d.p16[0]), (5.0, 6.0, 7.0, 8.0));
    assert_eq!((d.rotation, d.oblique, d.text_rotation, d.leader_length), (30.0, 60.0, 45.0, 2.0));
    let EntityKind::Dimension(n) = one("0|DIMENSION|70|6").kind else { panic!("a dimension") };
    assert_eq!((n.block.as_deref(), n.style.as_str(), n.measurement, n.kind()), (None, "Standard", None, 6));

    // The second vertex has no z: it does not take the third's.
    let EntityKind::Leader(l) = one("0|LEADER|3|Arrows|71|0|72|1|73|0|340|4F|10|1.0|20|2.0|30|3.0|10|4.0|20|5.0|10|6.0|20|7.0|30|8.0").kind else {
        panic!("a leader")
    };
    assert_eq!(l.vertices, [[1.0, 2.0, 3.0], [4.0, 5.0, 0.0], [6.0, 7.0, 8.0]]);
    assert!(!l.arrow && l.spline);
    assert_eq!((l.style.as_str(), l.annotation_type, l.annotation), ("Arrows", 0, Some(0x4F)));
}

#[test]
fn faces_viewports_images_and_tolerances() {
    let k = |text: &str| one(text).kind;
    let three = "|10|0.0|11|1.0|12|1.0|22|1.0";
    assert_eq!(k(&format!("0|SOLID{three}")), EntityKind::Solid { corners: [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 0.0]] });
    assert!(matches!(k(&format!("0|TRACE{three}|13|9.0")), EntityKind::Trace { corners } if corners[3] == [9.0, 0.0, 0.0]));
    assert!(matches!(k(&format!("0|3DFACE{three}|70|5")), EntityKind::Face3d { hidden_edges: 5, .. }));
    let EntityKind::Viewport(v) =
        k("0|VIEWPORT|10|5.0|20|4.0|40|8.0|41|6.0|69|2|90|81920|12|1.0|22|2.0|17|9.0|45|30.0|51|90.0|340|A1|331|B1|331|B2|341|B3")
    else {
        panic!("a viewport")
    };
    assert_eq!((v.center, v.width, v.height, v.id, v.status & 16384, v.status & 131_072), ([5.0, 4.0, 0.0], 8.0, 6.0, 2, 16384, 0));
    assert_eq!((v.view_center, v.view_target, v.view_direction, v.view_height, v.twist), ([1.0, 2.0], [9.0, 0.0, 0.0], [0.0, 0.0, 1.0], 30.0, 90.0));
    assert_eq!((v.clip, v.frozen_layers.as_slice()), (Some(0xA1), [0xB1, 0xB2, 0xB3].as_slice()));
    let EntityKind::Image(i) =
        k("0|IMAGE|10|1.0|11|0.5|12|0.0|22|0.5|13|640.0|23|480.0|340|C1|70|7|280|1|281|60|71|2|14|0.0|24|0.0|14|10.0|24|0.0|14|10.0|24|10.0|290|1")
    else {
        panic!("an image")
    };
    assert_eq!((i.insert[0], i.u[0], i.v[1], i.size, i.image_def), (1.0, 0.5, 0.5, [640.0, 480.0], Some(0xC1)));
    assert_eq!((i.display, i.clipping, i.brightness, i.contrast, i.fade, i.clip_type, i.clip_inverted), (7, true, 60, 50, 0, 2, true));
    assert_eq!(i.clip, [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    assert!(matches!(k("0|WIPEOUT|10|1.0|14|-0.5|24|-0.5|14|0.5|24|0.5"), EntityKind::Wipeout(w) if w.clip.len() == 2 && w.clip_type == 1));
    assert_eq!(
        k("0|TOLERANCE|3|Tol|10|1.0|1|{\\Fgdt;j}%%v0.1"),
        EntityKind::Tolerance { insert: [1.0, 0.0, 0.0], direction: [1.0, 0.0, 0.0], style: "Tol".into(), text: "{\\Fgdt;j}%%v0.1".into() }
    );
    assert_eq!(k("0|ARC_DIMENSION|10|1.0"), EntityKind::Other, "a type the model does not have is read from its tags");
}

#[test]
fn hatch_with_a_polyline_loop_and_a_pattern() {
    let EntityKind::Hatch(h) = one("0|HATCH|10|0.0|20|0.0|30|2.5|2|ANSI31|70|0|71|1|91|1
         92|7|72|1|73|1|93|3|10|0.0|20|0.0|42|0.5|10|4.0|20|0.0|42|0.0|10|4.0|20|3.0|42|0.0|97|2|330|A1|330|A2
         75|1|76|1|52|45.0|41|2.0|77|0|78|2
         53|45.0|43|0.0|44|0.0|45|-1.4|46|1.4|79|0
         53|135.0|43|1.0|44|2.0|45|1.0|46|1.0|79|2|49|3.0|49|-1.5
         98|2|10|1.0|20|1.0|10|2.0|20|2.0")
    .kind
    else {
        panic!("a hatch")
    };
    assert_eq!((h.pattern.as_str(), h.solid, h.associative, h.style, h.pattern_type), ("ANSI31", false, true, 1, 1));
    assert_eq!((h.angle, h.scale, h.double, h.elevation), (45.0, 2.0, false, 2.5));
    assert_eq!(h.loops.len(), 1);
    let l = &h.loops[0];
    assert_eq!((l.flags, l.sources.as_slice()), (7, [0xA1, 0xA2].as_slice()));
    assert_eq!(l.path, HatchPath::Polyline { closed: true, vertices: vec![[0.0, 0.0, 0.5], [4.0, 0.0, 0.0], [4.0, 3.0, 0.0]] });
    assert_eq!(h.pattern_lines.len(), 2);
    assert_eq!((h.pattern_lines[0].angle, h.pattern_lines[0].offset, h.pattern_lines[0].dashes.len()), (45.0, [-1.4, 1.4], 0));
    assert_eq!((h.pattern_lines[1].base, h.pattern_lines[1].dashes.as_slice()), ([1.0, 2.0], [3.0, -1.5].as_slice()));
    assert_eq!(h.seeds, [[1.0, 1.0], [2.0, 2.0]]);
    assert!(h.gradient.is_none());
}

#[test]
fn hatch_with_edge_loops_and_a_gradient() {
    let EntityKind::Hatch(h) = one(
        "0|HATCH|2|SOLID|70|1|71|0|91|2
         92|1|93|4
         72|1|10|0.0|20|0.0|11|4.0|21|0.0
         72|2|10|4.0|20|1.0|40|1.0|50|270.0|51|360.0|73|1
         72|3|10|2.0|20|2.0|11|2.0|21|0.0|40|0.5|50|0.0|51|180.0|73|0
         72|4|94|3|73|1|74|0|95|8|96|4|40|0.0|40|0.0|40|0.0|40|0.0|40|1.0|40|1.0|40|1.0|40|1.0|10|0.0|20|2.0|42|1.0|10|0.0|20|1.5|42|2.0|10|0.0|20|0.5|42|2.0|10|0.0|20|0.0|42|1.0|97|2|11|0.0|21|2.0|11|0.0|21|0.0|12|0.0|22|-1.0|13|0.0|23|-1.0
         97|0
         92|16|93|1
         72|2|10|2.0|20|1.0|40|0.25|50|0.0|51|360.0|73|1
         97|1|330|B1
         75|0|76|1|98|0
         450|1|451|0|460|0.785|461|0.0|452|0|462|0.0|453|2|463|0.0|63|5|421|255|463|1.0|63|2|470|LINEAR
         1001|ACAD|1010|0.0",
    )
    .kind
    else {
        panic!("a hatch")
    };
    assert!(h.solid && !h.associative);
    assert_eq!(h.loops.len(), 2);
    let HatchPath::Edges(e) = &h.loops[0].path else { panic!("edges") };
    assert_eq!(e.len(), 4);
    assert_eq!(e[0], HatchEdge::Line { start: [0.0, 0.0], end: [4.0, 0.0] });
    assert_eq!(e[1], HatchEdge::Arc { center: [4.0, 1.0], radius: 1.0, start_angle: 270.0, end_angle: 360.0, ccw: true });
    assert_eq!(e[2], HatchEdge::Ellipse { center: [2.0, 2.0], major: [2.0, 0.0], ratio: 0.5, start_angle: 0.0, end_angle: 180.0, ccw: false });
    let HatchEdge::Spline { degree, rational, periodic, knots, control, weights, fit, start_tangent, end_tangent } = &e[3] else {
        panic!("a spline edge")
    };
    assert_eq!((*degree, *rational, *periodic, knots.len(), control.len()), (3, true, false, 8, 4));
    assert_eq!(
        (weights.as_slice(), fit.len(), *start_tangent, *end_tangent),
        ([1.0, 2.0, 2.0, 1.0].as_slice(), 2, Some([0.0, -1.0]), Some([0.0, -1.0]))
    );
    // The spline's own 97 (fit points) is not the loop's source count.
    assert!(h.loops[0].sources.is_empty());
    assert_eq!((h.loops[1].flags, h.loops[1].sources.as_slice()), (16, [0xB1].as_slice()));
    let g = h.gradient.expect("a gradient");
    assert_eq!((g.name.as_str(), g.angle, g.shift, g.single), ("LINEAR", 0.785, 0.0, false));
    assert_eq!(g.colors, [Color::Rgb([0, 0, 255]), Color::Index(2)], "a true colour wins over its index");
}

#[test]
fn damaged_hatches() {
    let k = |text: &str| one(text).kind;
    assert_eq!(k("0|HATCH|2|SOLID"), EntityKind::Other, "no boundary data");
    assert_eq!(k("0|HATCH|91|2|92|2|93|1|10|0.0|20|0.0"), EntityKind::Other, "the second loop is missing");
    assert_eq!(k("0|HATCH|91|1|92|2|8|x|93|1"), EntityKind::Other, "a foreign group in the loop header");
    // Counts larger than the data: what is there is read, nothing is reserved for the rest.
    let EntityKind::Hatch(h) = k("0|HATCH|91|1|92|2|93|2000000000|10|1.0|20|2.0|97|2000000000|330|A|98|2000000000|10|5.0|20|6.0") else {
        panic!("a hatch")
    };
    assert_eq!(h.loops[0].path, HatchPath::Polyline { closed: false, vertices: vec![[1.0, 2.0, 0.0]] });
    assert_eq!((h.loops[0].sources.as_slice(), h.seeds.as_slice()), ([0xA].as_slice(), [[5.0, 6.0]].as_slice()));
    let EntityKind::Hatch(h) = k("0|HATCH|91|1|92|0|93|9|72|9|10|1.0|72|1|10|1.0|20|1.0|11|2.0|21|2.0|72|4|95|-5|96|2000000000|10|1.0|20|1.0") else {
        panic!("a hatch")
    };
    let HatchPath::Edges(e) = &h.loops[0].path else { panic!("edges") };
    assert_eq!(e.len(), 2, "an edge type the format does not have is passed over");
    assert!(matches!(&e[1], HatchEdge::Spline { knots, control, .. } if knots.is_empty() && control.len() == 1));
    assert_eq!(k("0|HATCH|91|-4").clone(), k("0|HATCH|91|0"), "a negative loop count is none");
    // A 450 that says no gradient.
    assert!(matches!(k("0|HATCH|91|0|450|0|463|0.0|63|1"), EntityKind::Hatch(h) if h.gradient.is_none()));
}

#[test]
fn entities_of_a_whole_drawing() {
    let t = tags(
        "0|SECTION|2|BLOCKS|0|BLOCK|2|B|0|CIRCLE|40|2.0|0|ENDBLK|0|ENDSEC|0|SECTION|2|ENTITIES|0|INSERT|2|B|0|POLYLINE|0|VERTEX|10|1.0|0|SEQEND|0|ENDSEC|0|EOF",
    );
    let d = Document::read(&t);
    let kinds: Vec<EntityKind> = d.entities.iter().chain(&d.blocks[0].entities).map(|e| Entity::read(e).kind).collect();
    assert!(matches!(kinds.as_slice(), [EntityKind::Insert(_), EntityKind::Polyline(p), EntityKind::Circle { .. }] if p.vertices.len() == 1));
}

#[test]
fn a_spline_edge_without_fit_data_leaves_the_loops_source_count() {
    // Before R2010 a spline edge has no 97 of its own: the one after it counts the loop's sources.
    let EntityKind::Hatch(h) = one(
        "0|HATCH|91|1|92|1|93|1|72|4|94|1|73|0|74|0|95|4|96|2|40|0.0|40|0.0|40|1.0|40|1.0|10|0.0|20|0.0|10|1.0|20|1.0|97|2|330|A1|330|A2|75|0|98|0",
    )
    .kind
    else {
        panic!("a hatch")
    };
    assert_eq!(h.loops[0].sources, [0xA1, 0xA2]);
    assert!(
        matches!(&h.loops[0].path, HatchPath::Edges(e) if matches!(&e[0], HatchEdge::Spline { fit, control, .. } if fit.is_empty() && control.len() == 2))
    );
}

#[test]
fn an_attributes_vertical_alignment_is_group_74_not_the_field_length() {
    // 73 is the field length of an ATTRIB or ATTDEF; a writer that emits it must not turn a
    // middle-aligned attribute into another alignment.
    let EntityKind::AttDef(a) = one("0|ATTDEF|10|1.0|1|x|2|TAG|70|0|73|8|74|2").kind else { panic!("an attribute definition") };
    assert_eq!(a.text.valign, 2);
    let EntityKind::AttDef(a) = one("0|ATTDEF|10|1.0|1|x|2|TAG|70|0|73|3").kind else { panic!("an attribute definition") };
    assert_eq!(a.text.valign, 0, "a field length alone is no alignment");
    let EntityKind::Text(t) = one("0|TEXT|10|1.0|1|x|73|3|74|1").kind else { panic!("text") };
    assert_eq!(t.valign, 3, "TEXT has its alignment in 73");
}

#[test]
fn entity_groups_written_after_the_entitys_own_marker() {
    // An AcDbEntity subclass without a layer: the run of entity codes after the LINE's own marker
    // is the entity's.
    let c = one("0|LINE|5|2A|100|AcDbEntity|100|AcDbLine|8|Walls|62|1|370|50|10|1.0|20|2.0|48|9.0").common;
    assert_eq!((c.layer.as_str(), c.color, c.lineweight), ("Walls", Color::Index(1), Lineweight::Hundredths(50)));
    assert_eq!(c.ltscale, 1.0, "the run ends at the entity's own first group");
    // With a layer in AcDbEntity, nothing after the entity's marker is taken.
    let c = one("0|MTEXT|100|AcDbEntity|8|A|100|AcDbMText|62|3|48|7.0").common;
    assert_eq!((c.layer.as_str(), c.color, c.ltscale), ("A", Color::ByLayer, 1.0));
}

#[test]
fn an_attributes_justification_is_not_its_multiline_header() {
    // A left-justified multiline attribute writes no 72 of its own; the 72 and 11 of its
    // AcDbAttribute subclass are the multiline header.
    let EntityKind::AttDef(a) = one(
        "0|ATTDEF|100|AcDbText|10|1.0|20|2.0|40|2.5|1|x|100|AcDbAttributeDefinition|280|0|3|p|2|TAG|70|0|74|0|280|0|71|4|72|1|11|9.0|21|9.0|31|0.0|101|Embedded Object|10|1.0|1|x",
    )
    .kind
    else {
        panic!("an attribute definition")
    };
    assert_eq!((a.text.halign, a.text.valign, a.text.align, a.text.generation), (0, 0, None, 0));
    // The text's own justification still counts.
    let EntityKind::AttDef(a) = one("0|ATTDEF|100|AcDbText|10|1.0|1|x|72|2|11|5.0|100|AcDbAttributeDefinition|2|TAG|74|3").kind else {
        panic!("an attribute definition")
    };
    assert_eq!((a.text.halign, a.text.valign, a.text.align), (2, 3, Some([5.0, 0.0, 0.0])));
}
