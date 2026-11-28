//! Truncated, malformed and hostile parts are refused with a typed error; nothing panics.

use craft_ink::inkml::{read, write};
use craft_ink::stroke::{Brush, Color, Stroke, StrokePoint};
use craft_ink::{Ink, InkmlError, Limits};

const OPEN: &str = r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat><inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/></inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"><inkml:brushProperty name="width" value="0.1"/><inkml:brushProperty name="height" value="0.1"/><inkml:brushProperty name="color" value="#000000"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#c" brushRef="#b">"##;

fn trace(body: &str) -> String {
    format!("{OPEN}{body}</inkml:trace></inkml:ink>")
}

fn ink() -> Ink {
    Ink {
        strokes: vec![Stroke::new(
            vec![StrokePoint::new(0.0, 0.0), StrokePoint::new(2.5, 3.25), StrokePoint::new(6.0, 1.0)],
            Brush::new(craft_ink::Tool::Pen),
            Color::BLACK,
        )],
        timestamp: None,
    }
}

#[test]
fn a_truncated_part_is_an_xml_error() {
    let text = write(&ink(), &Limits::default()).expect("writes");
    for cut in [1, text.len() / 4, text.len() / 2, text.len() - 3] {
        let bytes = &text.as_bytes()[..cut];
        assert!(matches!(read(bytes, &Limits::default()), Err(InkmlError::Xml(_))), "cut at {cut}");
    }
}

#[test]
fn a_part_that_is_not_inkml_is_refused() {
    for body in [
        "<foo/>".to_string(),
        r#"<inkml:ink xmlns:inkml="http://example.com/not-inkml"/>"#.to_string(),
        r#"<ink xmlns="http://example.com/not-inkml"/>"#.to_string(),
    ] {
        assert!(matches!(read(body.as_bytes(), &Limits::default()), Err(InkmlError::NotInkml(_))), "{body}");
    }
}

#[test]
fn a_doctype_or_an_undefined_entity_is_refused() {
    let doctype = r#"<!DOCTYPE inkml:ink [<!ENTITY x "y">]><inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"/>"#;
    assert!(matches!(read(doctype.as_bytes(), &Limits::default()), Err(InkmlError::Xml(_))));
    let entity = r#"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions>&boom;</inkml:definitions></inkml:ink>"#;
    assert!(matches!(read(entity.as_bytes(), &Limits::default()), Err(InkmlError::Xml(_))));
}

#[test]
fn dangling_references_are_refused_rather_than_guessed() {
    let body = trace("!0 0");
    let missing_brush = body.replace("brushRef=\"#b\"", "brushRef=\"#nope\"");
    assert!(matches!(read(missing_brush.as_bytes(), &Limits::default()), Err(InkmlError::DanglingRef(_))));
    let no_hash = body.replace("brushRef=\"#b\"", "brushRef=\"b\"");
    assert!(matches!(read(no_hash.as_bytes(), &Limits::default()), Err(InkmlError::DanglingRef(_))));
    let missing_context = body.replace("contextRef=\"#c\"", "contextRef=\"#nope\"");
    assert!(matches!(read(missing_context.as_bytes(), &Limits::default()), Err(InkmlError::DanglingRef(_))));
    // A trace with no context at all falls back to the part's first one, as Office writes it.
    let no_refs = body.replace("contextRef=\"#c\" brushRef=\"#b\"", "brushRef=\"#b\"");
    assert!(read(no_refs.as_bytes(), &Limits::default()).is_ok());
}

#[test]
fn unknown_elements_are_rejected_and_unknown_attributes_ignored() {
    let unknown = format!("{OPEN}!0 0</inkml:trace><inkml:traceView/></inkml:ink>");
    assert!(matches!(read(unknown.as_bytes(), &Limits::default()), Err(InkmlError::Unsupported(_))));
    let annotated = body_with(r#"<inkml:annotation type="comment">hi</inkml:annotation>"#);
    assert!(read(annotated.as_bytes(), &Limits::default()).is_ok(), "annotations are ignored");
    let attribute = body_with(r##"<inkml:trace contextRef="#c" brushRef="#b" unknown="1">!0 0</inkml:trace>"##);
    assert!(read(attribute.as_bytes(), &Limits::default()).is_ok(), "unknown attributes are ignored");
}

fn body_with(extra: &str) -> String {
    format!(
        r#"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat><inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/></inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"><inkml:brushProperty name="width" value="0.1"/><inkml:brushProperty name="height" value="0.1"/></inkml:brush></inkml:definitions>{extra}</inkml:ink>"#
    )
}

#[test]
fn limits_on_bytes_strokes_points_and_depth_hold() {
    let text = write(&ink(), &Limits::default()).expect("writes");
    let short = Limits { max_bytes: text.len() - 1, ..Limits::default() };
    assert!(matches!(read(text.as_bytes(), &short), Err(InkmlError::TooLarge { .. })));
    let strokes = body_with(
        r##"<inkml:trace contextRef="#c" brushRef="#b">!0 0</inkml:trace><inkml:trace contextRef="#c" brushRef="#b">!0 0</inkml:trace><inkml:trace contextRef="#c" brushRef="#b">!0 0</inkml:trace>"##,
    );
    let limits = Limits { max_strokes: 2, ..Limits::default() };
    assert!(matches!(read(strokes.as_bytes(), &limits), Err(InkmlError::TooManyStrokes { max: 2 })));
    let points = trace("!0 0,!1 1,!2 2,!3 3");
    let limits = Limits { max_points: 3, ..Limits::default() };
    assert!(matches!(read(points.as_bytes(), &limits), Err(InkmlError::TooManyPoints { max: 3 })));
    let deep = format!("{OPEN}!0 0</inkml:trace></inkml:traceGroup></inkml:traceGroup></inkml:traceGroup></inkml:ink>").replace(
        "<inkml:trace contextRef=\"#c\" brushRef=\"#b\">",
        "<inkml:traceGroup><inkml:traceGroup><inkml:traceGroup><inkml:trace contextRef=\"#c\" brushRef=\"#b\">",
    );
    let limits = Limits { max_depth: 2, ..Limits::default() };
    assert!(matches!(read(deep.as_bytes(), &limits), Err(InkmlError::TooDeep { max: 2 })));
}

#[test]
fn malformed_values_are_errors_not_defaults() {
    for body in [
        "!0 0,!1",             // a missing channel value
        "!0 0\x001",           // a value the grammar cannot scan
        "!0 0,!3.5 1",         // a decimal in an integer channel
        "!0 0,!99999999999 1", // past the format's coordinate range
        "!0 0 junk",           // trailing text
        "",
    ] {
        let part = trace(body);
        assert!(matches!(read(part.as_bytes(), &Limits::default()), Err(InkmlError::BadValue { .. }) | Err(InkmlError::Missing(_))), "{body:?}");
    }
    // A hostile channel set: no Y, an unknown type, or a non-integer grammar.
    for format in [r#"<inkml:channel name="X" type="integer"/>"#, r#"<inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="real"/>"#]
    {
        let part = format!(
            r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat>{format}</inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"/></inkml:definitions><inkml:trace contextRef="#c" brushRef="#b">!0 0</inkml:trace></inkml:ink>"##
        );
        assert!(matches!(read(part.as_bytes(), &Limits::default()), Err(InkmlError::Missing(_) | InkmlError::BadValue { .. })));
    }
    // Hostile brush values: a non-numeric size, a colour of the wrong shape, an unknown texture.
    for property in ["width", "height", "color", "inkEffects"] {
        let part = format!(
            r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="c"><inkml:inkSource><inkml:traceFormat><inkml:channel name="X" type="integer"/><inkml:channel name="Y" type="integer"/></inkml:traceFormat></inkml:inkSource></inkml:context><inkml:brush xml:id="b"><inkml:brushProperty name="{property}" value="NaN"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#c" brushRef="#b">!0 0</inkml:trace></inkml:ink>"##
        );
        let error = read(part.as_bytes(), &Limits::default());
        assert!(matches!(error, Err(InkmlError::BadValue { .. })), "{property}: {error:?}");
    }
}

#[test]
fn a_flood_of_input_is_bounded_not_fatal() {
    // Many points, a huge group nesting and a very long trace text all fail cleanly.
    let mut body = String::new();
    for i in 0..200_000 {
        if i > 0 {
            body.push(',');
        }
        body.push_str("!0 0");
    }
    let part = trace(&body);
    assert!(read(part.as_bytes(), &Limits::default()).is_err());
    // A trace with NaN written as text, and one with an i64-overflowing difference.
    let part = trace("!0 0,'9223372036854775807 0,\"9223372036854775807 0");
    assert!(read(part.as_bytes(), &Limits::default()).is_err());
}
