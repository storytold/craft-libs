//! The codec round-trips every field the crate models, and the private properties survive.

use craft_ink::stroke::{Brush, Color, Effect, Stroke, StrokePoint, Tool};
use craft_ink::{Ink, Limits};

fn point(x: f64, y: f64) -> StrokePoint {
    StrokePoint::new(x, y)
}

fn ink(strokes: Vec<Stroke>) -> Ink {
    Ink { strokes, timestamp: None }
}

#[test]
fn brush_colour_thickness_pressure_and_order_round_trip() {
    let brush = Brush {
        tool: Tool::FountainPen,
        width_cm: 0.35,
        height_cm: 0.35,
        effect: Some(Effect::Galaxy),
        anchor: [-1_303.866_2, -1016.0],
        scale: 0.5,
        extra: Vec::new(),
    };
    // Pressure is recorded for every point or for none, so the F channel round-trips exactly.
    let pressured: Vec<StrokePoint> = vec![point(1.234, -5.678), point(2.345, -5.5), point(3.456, -5.0), point(4.567, -4.5)]
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            p.with_pressure(if i == 0 {
                0
            } else if i == 3 {
                32_767
            } else {
                1234
            })
        })
        .collect();
    let first = Stroke {
        points: pressured,
        brush,
        color: Color::from_hex("#A12BCD").expect("a colour"),
        order: 7,
        creation_id: Some("{11111111-2222-4333-8444-555555555555}".into()),
    };
    let second = Stroke::new(vec![point(0.001, 0.0), point(0.0, 0.001)], Brush::new(Tool::Highlighter), Color::rgb(0xF2, 0xE3, 0x3A));
    let original = ink(vec![first, second]);
    let text = craft_ink::inkml::write(&original, &Limits::default()).expect("writes");
    let back = craft_ink::inkml::read(text.as_bytes(), &Limits::default()).expect("reads");
    let mut expected = original.clone();
    // The creation id belongs to the drawing's content part, not to the InkML part.
    expected.strokes[0].creation_id = None;
    assert_eq!(back, expected);
    assert_eq!(back.strokes[0].brush.tool, Tool::FountainPen);
    assert_eq!(back.strokes[0].brush.effect, Some(Effect::Galaxy));
    assert!((back.strokes[0].brush.width_cm - 0.35).abs() < f32::EPSILON);
    assert_eq!(back.strokes[0].color, Color::rgb(0xA1, 0x2B, 0xCD));
    assert_eq!(back.strokes[0].order, 7);
    assert_eq!(back.strokes[0].points[3].pressure, Some(32_767));
    assert_eq!(back.strokes[1].brush.tool, Tool::Highlighter);
    assert!(back.strokes[1].points.iter().all(|p| p.pressure.is_none()));
}

#[test]
fn writing_is_deterministic_and_repeating_it_changes_nothing() {
    let stroke = Stroke::new(vec![point(0.0, 0.0), point(10.0, 4.5), point(20.0, 9.25)], Brush::new(Tool::BrushPen), Color::BLACK);
    let ink = ink(vec![stroke]);
    let first = craft_ink::inkml::write(&ink, &Limits::default()).expect("writes");
    let second = craft_ink::inkml::write(&ink, &Limits::default()).expect("writes");
    assert_eq!(first, second);
    let back = craft_ink::inkml::read(first.as_bytes(), &Limits::default()).expect("reads");
    let again = craft_ink::inkml::write(&back, &Limits::default()).expect("writes");
    assert_eq!(first, again);
}

#[test]
fn every_tool_and_texture_round_trips_its_private_marker() {
    let mut strokes = Vec::new();
    for tool in Tool::ALL {
        for effect in std::iter::once(None).chain(Effect::ALL.map(Some)) {
            let mut brush = Brush::new(tool);
            brush.effect = effect;
            strokes.push(Stroke::new(vec![point(0.0, 0.0), point(1.0, 1.0)], brush, Color::BLACK));
        }
    }
    let ink = ink(strokes);
    let text = craft_ink::inkml::write(&ink, &Limits::default()).expect("writes");
    let back = craft_ink::inkml::read(text.as_bytes(), &Limits::default()).expect("reads");
    assert_eq!(back.strokes.len(), Tool::ALL.len() * (Effect::ALL.len() + 1));
    let mut expected = Vec::new();
    for tool in Tool::ALL {
        for effect in std::iter::once(None).chain(Effect::ALL.map(Some)) {
            expected.push((tool, effect));
        }
    }
    for (stroke, (tool, effect)) in back.strokes.iter().zip(expected) {
        assert_eq!((stroke.brush.tool, stroke.brush.effect), (tool, effect));
    }
}

#[test]
fn a_preserved_brush_property_survives_a_round_trip() {
    let mut brush = Brush::new(Tool::Pen);
    brush.extra.push(("vendorPhase".into(), "2".into()));
    brush.extra.push(("vendorName".into(), "a & b".into()));
    let ink = ink(vec![Stroke::new(vec![point(0.0, 0.0), point(1.0, 0.0)], brush, Color::BLACK)]);
    let text = craft_ink::inkml::write(&ink, &Limits::default()).expect("writes");
    let back = craft_ink::inkml::read(text.as_bytes(), &Limits::default()).expect("reads");
    assert_eq!(back.strokes[0].brush.extra, vec![("vendorPhase".into(), "2".into()), ("vendorName".into(), "a & b".into())]);
}

#[test]
fn the_legacy_private_property_names_are_read() {
    // GridCraft wrote `gridcraftTool`/`gridcraftOrder` before this crate existed; a file it saved
    // must keep its tool and order.
    let text = br##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat><inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/></inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"><inkml:brushProperty name="width" value="0.1"/><inkml:brushProperty name="height" value="0.1"/><inkml:brushProperty name="color" value="#112233"/><inkml:brushProperty name="gridcraftTool" value="fountainPen"/><inkml:brushProperty name="gridcraftOrder" value="9"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#c" brushRef="#b">!0 0,!1000 1000</inkml:trace></inkml:ink>"##;
    let ink = craft_ink::inkml::read(text, &Limits::default()).expect("reads");
    assert_eq!(ink.strokes[0].brush.tool, Tool::FountainPen);
    assert_eq!(ink.strokes[0].order, 9);
    assert_eq!(ink.strokes[0].color, Color::rgb(0x11, 0x22, 0x33));
}

#[test]
fn brush_groups_inherit_trace_references_and_keep_document_order() {
    let text = br##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions>
<inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat>
<inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/>
</inkml:traceFormat><inkml:channelProperties>
<inkml:channelProperty channel="X" name="resolution" value="100"/><inkml:channelProperty channel="Y" name="resolution" value="100"/>
</inkml:channelProperties></inkml:inkSource></inkml:context>
<inkml:brush xml:id="pencil"><inkml:brushProperty name="width" value="0.1"/><inkml:brushProperty name="height" value="0.6"/><inkml:brushProperty name="inkEffects" value="pencil"/><inkml:brushProperty name="craftOrder" value="1"/></inkml:brush>
</inkml:definitions><inkml:traceGroup brushRef="#pencil" contextRef="#c">
<inkml:trace>!0 0,!100 100</inkml:trace>
<inkml:trace>!100 100,'0'100</inkml:trace>
</inkml:traceGroup></inkml:ink>"##;
    let ink = craft_ink::inkml::read(text, &Limits::default()).expect("reads");
    assert_eq!(ink.strokes.len(), 2);
    for stroke in &ink.strokes {
        assert_eq!(stroke.brush.tool, Tool::Pencil);
        assert_eq!(stroke.order, 1);
    }
    // Resolution 100 means 100 values per centimetre.
    assert!((ink.strokes[0].points[1].x - 1.0).abs() < 1e-12);
    assert_eq!(ink.strokes[1].points[1].y, 2.0);
}

#[test]
fn a_timestamp_is_kept_and_an_empty_part_round_trips() {
    let empty = Ink { strokes: Vec::new(), timestamp: Some("2026-11-07T10:00:00Z".into()) };
    let text = craft_ink::inkml::write(&empty, &Limits::default()).expect("writes");
    let back = craft_ink::inkml::read(text.as_bytes(), &Limits::default()).expect("reads");
    assert_eq!(back, empty);
    let bare = Ink::default();
    let text = craft_ink::inkml::write(&bare, &Limits::default()).expect("writes");
    assert_eq!(craft_ink::inkml::read(text.as_bytes(), &Limits::default()).expect("reads"), bare);
}

#[test]
fn limits_are_enforced_while_writing() {
    let many = ink(vec![Stroke::new(vec![point(0.0, 0.0)], Brush::default(), Color::BLACK); 3]);
    let tight = Limits { max_strokes: 2, ..Limits::default() };
    assert!(matches!(craft_ink::inkml::write(&many, &tight), Err(craft_ink::InkmlError::TooManyStrokes { max: 2 })));
    let long = ink(vec![Stroke::new((0..4).map(|i| point(f64::from(i), 0.0)).collect(), Brush::default(), Color::BLACK)]);
    let tight = Limits { max_points: 3, ..Limits::default() };
    assert!(matches!(craft_ink::inkml::write(&long, &tight), Err(craft_ink::InkmlError::TooManyPoints { max: 3 })));
    let empty_stroke = ink(vec![Stroke::default()]);
    assert!(matches!(craft_ink::inkml::write(&empty_stroke, &Limits::default()), Err(craft_ink::InkmlError::InvalidStroke { index: 0, .. })));
    let mut partial = Stroke::new(vec![point(0.0, 0.0), point(1.0, 1.0)], Brush::default(), Color::BLACK);
    partial.points[0].pressure = Some(4);
    assert!(matches!(craft_ink::inkml::write(&ink(vec![partial]), &Limits::default()), Err(craft_ink::InkmlError::InvalidStroke { .. })));
}

#[test]
fn pressure_round_trips_through_a_second_difference_run() {
    // A long, wobbly, densely sampled stroke exercises the inherited `"` prefix on both axes and
    // on the pressure channel.
    let points: Vec<StrokePoint> = (0..500)
        .map(|i| {
            let t = f64::from(i);
            StrokePoint::new(0.001 * t + (t * 0.7).sin() * 0.05, 0.002 * t + (t * 0.3).cos() * 0.02).with_pressure((i * 37 % 32_768) as u16)
        })
        .collect();
    let stroke = Stroke { points: points.clone(), ..Stroke::new(Vec::new(), Brush::new(Tool::Pencil), Color::BLACK) };
    let text = craft_ink::inkml::write(&ink(vec![stroke]), &Limits::default()).expect("writes");
    let back = craft_ink::inkml::read(text.as_bytes(), &Limits::default()).expect("reads");
    assert_eq!(back.strokes[0].points.len(), points.len());
    for (want, got) in points.iter().zip(&back.strokes[0].points) {
        // Coordinates are stored at 1/1000 cm, so compare at that resolution.
        assert!((want.x - got.x).abs() <= 0.0005 + 1e-12, "{} vs {}", want.x, got.x);
        assert!((want.y - got.y).abs() <= 0.0005 + 1e-12);
        assert_eq!(want.pressure, got.pressure);
    }
}

#[test]
fn the_difference_encoding_decodes_compact_values_stars_and_inherited_prefixes() {
    let part = |trace: &str| {
        format!(
            r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat><inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/></inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"><inkml:brushProperty name="width" value="0.1"/><inkml:brushProperty name="height" value="0.1"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#c" brushRef="#b">{trace}</inkml:trace></inkml:ink>"##
        )
    };
    // Compact values share one point: `'2117'529` is two channel values.
    let ink = craft_ink::inkml::read(part("!1000 1000,'100'100").as_bytes(), &Limits::default()).expect("reads");
    assert_eq!(ink.strokes[0].points.len(), 2);
    assert!((ink.strokes[0].points[1].x - 1.1).abs() < 1e-12);
    assert!((ink.strokes[0].points[1].y - 1.1).abs() < 1e-12);
    // Second differences, then an unprefixed value inheriting the `"` prefix.
    let ink = craft_ink::inkml::read(part("!1000 1000,'100'100,\"2 2,2 2").as_bytes(), &Limits::default()).expect("reads");
    let xs: Vec<f64> = ink.strokes[0].points.iter().map(|p| p.x).collect();
    assert_eq!(xs, vec![1.0, 1.1, 1.202, 1.306]);
    // `*` repeats the value and zeroes the difference order for that channel.
    let ink = craft_ink::inkml::read(part("!1000 1000,'100'100,* *").as_bytes(), &Limits::default()).expect("reads");
    assert_eq!((ink.strokes[0].points[2].x, ink.strokes[0].points[2].y), (1.1, 1.1));
}

#[test]
fn the_point_limit_counts_the_whole_part_not_one_trace() {
    let part = r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat><inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/></inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"/></inkml:definitions><inkml:trace contextRef="#c" brushRef="#b">!0 0,!1 1</inkml:trace><inkml:trace contextRef="#c" brushRef="#b">!0 0,!1 1</inkml:trace></inkml:ink>"##;
    let limits = Limits { max_points: 3, ..Limits::default() };
    let result = craft_ink::inkml::read(part.as_bytes(), &limits);
    assert!(matches!(result, Err(craft_ink::InkmlError::TooManyPoints { max: 3 })), "{result:?}");
    let limits = Limits { max_points: 4, ..Limits::default() };
    assert_eq!(craft_ink::inkml::read(part.as_bytes(), &limits).expect("reads").strokes.len(), 2);
}
