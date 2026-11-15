//! Borrowed views of native XFL shape-hint records.
//!
//! These parse the native point encodings and hint ordering. Snapping points onto consumer
//! geometry and pairing start/end keyframes remain application responsibilities.

use crate::shape::{XflElement, integer, number};

/// A native XFL point in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// The optional `<shapeHints>` wrapper on a keyframe.
#[derive(Clone, Copy, Debug)]
pub struct ShapeHints<'a, E: XflElement> {
    wrapper: Option<&'a E>,
}

impl<'a, E: XflElement> ShapeHints<'a, E> {
    pub fn new(keyframe: &'a E) -> Self {
        Self { wrapper: keyframe.child("shapeHints") }
    }

    pub fn is_empty(&self) -> bool {
        self.wrapper.is_none_or(|wrapper| wrapper.elements().next().is_none())
    }

    /// Records in source order; unrecognized child names are retained because the legacy reader
    /// interpreted their point and ordering attributes too.
    pub fn iter(&self) -> impl Iterator<Item = ShapeHint<'a, E>> + 'a {
        let wrapper = self.wrapper;
        wrapper.into_iter().flat_map(XflElement::elements).map(|element| ShapeHint { element })
    }
}

/// One native shape-hint record.
#[derive(Clone, Copy, Debug)]
pub struct ShapeHint<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> ShapeHint<'a, E> {
    /// Native numeric order, or its single-letter `name` fallback (`a` is zero).
    pub fn index(&self) -> Option<usize> {
        integer(self.element, "index").map(|value| value as usize).or_else(|| self.element.attr("name").and_then(letter_index))
    }

    /// The keyframe's point, in either joined or separate native coordinate attributes.
    pub fn start_point(&self) -> Option<Point> {
        read_point(self.element, "startPoint", "startX", "startY").or_else(|| read_point(self.element, "point", "x", "y"))
    }

    /// The next keyframe's point, when the record contains one.
    pub fn end_point(&self) -> Option<Point> {
        read_point(self.element, "endPoint", "endX", "endY")
    }
}

fn letter_index(name: &str) -> Option<usize> {
    let [byte] = name.as_bytes() else { return None };
    match byte.to_ascii_lowercase() {
        b'a'..=b'z' => Some((byte.to_ascii_lowercase() - b'a') as usize),
        _ => None,
    }
}

fn read_point(element: &impl XflElement, joined: &str, x: &str, y: &str) -> Option<Point> {
    let point = match element.attr(joined) {
        Some(value) => {
            let mut values = value.split([',', ' ']).filter(|part| !part.trim().is_empty()).map(|part| part.trim().parse::<f32>().ok());
            Point { x: values.next()??, y: values.next()?? }
        }
        None => Point { x: number(element, x)?, y: number(element, y)? },
    };
    (point.x.is_finite() && point.y.is_finite()).then_some(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::parse(source).expect("well-formed test XML")
    }

    #[test]
    fn reads_joined_and_split_native_points_in_source_order() {
        let keyframe = parse(br#"<DOMFrame><shapeHints><Hint name="B" startPoint="12.5, -2" endX="30" endY="40"/><Hint index="0" startX="1" startY="2" endPoint="3 4"/></shapeHints></DOMFrame>"#);
        let hints = ShapeHints::new(&keyframe);
        assert!(!hints.is_empty());
        let records: Vec<_> = hints.iter().collect();
        assert_eq!(records[0].index(), Some(1));
        assert_eq!(records[0].start_point(), Some(Point { x: 12.5, y: -2.0 }));
        assert_eq!(records[0].end_point(), Some(Point { x: 30.0, y: 40.0 }));
        assert_eq!(records[1].index(), Some(0));
        assert_eq!(records[1].start_point(), Some(Point { x: 1.0, y: 2.0 }));
        assert_eq!(records[1].end_point(), Some(Point { x: 3.0, y: 4.0 }));
    }

    #[test]
    fn malformed_joined_values_fall_back_to_the_other_native_point_field() {
        let keyframe = parse(br#"<DOMFrame><shapeHints><Any name="z" startPoint="bad" x="7" y="8" endPoint="nan, 2"/></shapeHints></DOMFrame>"#);
        let hint = ShapeHints::new(&keyframe).iter().next().expect("hint record");
        assert_eq!(hint.index(), Some(25));
        assert_eq!(hint.start_point(), Some(Point { x: 7.0, y: 8.0 }));
        assert_eq!(hint.end_point(), None);
    }

    #[test]
    fn missing_lists_and_non_letter_names_remain_optional() {
        let keyframe = parse(b"<DOMFrame/>");
        assert!(ShapeHints::new(&keyframe).is_empty());
        let keyframe = parse(br#"<DOMFrame><shapeHints><Hint name="AB" startX="1" startY="2"/></shapeHints></DOMFrame>"#);
        let hint = ShapeHints::new(&keyframe).iter().next().expect("hint record");
        assert_eq!(hint.index(), None);
    }
}
