//! The macro and the container impls, on small twin models.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use crate::{Convert, ConvertRef, Floats, convert_types};

/// One model.
mod left {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Point {
        pub x: f32,
        pub y: f32,
    }

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Empty {}

    #[derive(Clone, Debug, PartialEq)]
    pub enum Shape {
        None,
        Dot(Point),
        Line(Point, Point),
        Poly { pts: Vec<Point>, closed: bool },
        Label { text: String, at: Option<Box<Point>> },
    }

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Page {
        pub title: String,
        pub shapes: Vec<Arc<Shape>>,
        pub named: BTreeMap<u32, Point>,
        pub size: (f32, f32),
        pub margins: [f32; 4],
        /// `note` on the right.
        pub comment: Option<String>,
        /// Only this model has it.
        pub cache: u64,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Unit {
        Pt,
        Mm,
        /// `Inch` on the right.
        In,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum Old {
        Keep(u8),
        /// Only this model has it.
        Gone {
            why: String,
        },
    }
}

/// Its twin: the same shape, with a renamed field, a renamed variant and one member of its own.
mod right {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Point {
        pub x: f32,
        pub y: f32,
    }

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Empty {}

    #[derive(Clone, Debug, PartialEq)]
    pub enum Shape {
        None,
        Dot(Point),
        Line(Point, Point),
        Poly { pts: Vec<Point>, closed: bool },
        Label { text: String, at: Option<Box<Point>> },
    }

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Page {
        pub title: String,
        pub shapes: Vec<Arc<Shape>>,
        pub named: BTreeMap<u32, Point>,
        pub size: (f32, f32),
        pub margins: [f32; 4],
        pub note: Option<String>,
        /// Only this model has it.
        pub revision: u32,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Unit {
        Pt,
        Mm,
        Inch,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum New {
        Keep(u8),
        /// Only this model has it.
        #[allow(dead_code)]
        Added,
    }
}

convert_types! {
    struct left::Point = right::Point { x, y }
    struct left::Empty = right::Empty {}
    enum left::Shape = right::Shape {
        None,
        Dot(p),
        Line(a, b),
        Poly { pts, closed },
        Label { text, at },
    }
    struct left::Page = right::Page {
        title,
        shapes,
        named,
        size,
        margins,
        comment => note,
        // Only the left has `cache`: dropped going right, zero coming back.
        cache => _ = 0,
        // Only the right has `revision`: 1 going right, dropped coming back.
        _ => revision = 1,
    }
    enum left::Unit = right::Unit { Pt, Mm, In => Inch }
    // One way only, by value: a variant on each side has no partner.
    enum left::Old => right::New {
        Keep(n),
        Gone { why: _ } = right::New::Keep(0),
        _ => Added,
    }
}

fn left_page() -> left::Page {
    let dot = Arc::new(left::Shape::Dot(left::Point { x: 1.0, y: 2.0 }));
    left::Page {
        title: "Plan".into(),
        shapes: vec![
            dot.clone(),
            dot,
            Arc::new(left::Shape::None),
            Arc::new(left::Shape::Line(left::Point { x: 0.0, y: 0.0 }, left::Point { x: 3.0, y: 4.0 })),
            Arc::new(left::Shape::Poly { pts: vec![left::Point::default(); 3], closed: true }),
            Arc::new(left::Shape::Label { text: "a".into(), at: Some(Box::new(left::Point { x: 5.0, y: 6.0 })) }),
        ],
        named: BTreeMap::from([(7, left::Point { x: 7.0, y: 8.0 })]),
        size: (612.0, 792.0),
        margins: [72.0, 36.0, 72.0, 36.0],
        comment: Some("draft".into()),
        cache: 99,
    }
}

fn right_page() -> right::Page {
    let dot = Arc::new(right::Shape::Dot(right::Point { x: 1.0, y: 2.0 }));
    right::Page {
        title: "Plan".into(),
        shapes: vec![
            dot.clone(),
            dot,
            Arc::new(right::Shape::None),
            Arc::new(right::Shape::Line(right::Point { x: 0.0, y: 0.0 }, right::Point { x: 3.0, y: 4.0 })),
            Arc::new(right::Shape::Poly { pts: vec![right::Point::default(); 3], closed: true }),
            Arc::new(right::Shape::Label { text: "a".into(), at: Some(Box::new(right::Point { x: 5.0, y: 6.0 })) }),
        ],
        named: BTreeMap::from([(7, right::Point { x: 7.0, y: 8.0 })]),
        size: (612.0, 792.0),
        margins: [72.0, 36.0, 72.0, 36.0],
        note: Some("draft".into()),
        revision: 1,
    }
}

#[test]
fn a_twin_declaration_converts_both_ways_by_value_and_by_reference() {
    let left = left_page();
    let right: right::Page = left.convert_ref();
    assert_eq!(right, right_page());
    let moved: right::Page = left.clone().convert();
    assert_eq!(moved, right_page());

    // Coming back, the left-only field takes its declared default.
    let back: left::Page = right.convert_ref();
    assert_eq!(back, left::Page { cache: 0, ..left });
    let back_moved: left::Page = right.convert();
    assert_eq!(back_moved, back);

    assert_eq!(Convert::<right::Empty>::convert(left::Empty {}), right::Empty {});
    assert_eq!(ConvertRef::<left::Empty>::convert_ref(&right::Empty {}), left::Empty {});
}

#[test]
fn renamed_and_unit_variants_map_both_ways() {
    for (l, r) in [(left::Unit::Pt, right::Unit::Pt), (left::Unit::Mm, right::Unit::Mm), (left::Unit::In, right::Unit::Inch)] {
        assert_eq!(Convert::<right::Unit>::convert(l), r);
        assert_eq!(ConvertRef::<right::Unit>::convert_ref(&l), r);
        assert_eq!(Convert::<left::Unit>::convert(r), l);
        assert_eq!(ConvertRef::<left::Unit>::convert_ref(&r), l);
    }
}

#[test]
fn a_one_way_enum_maps_a_source_only_variant_with_its_own_expression() {
    assert_eq!(Convert::<right::New>::convert(left::Old::Keep(4)), right::New::Keep(4));
    assert_eq!(Convert::<right::New>::convert(left::Old::Gone { why: "x".into() }), right::New::Keep(0));
}

#[test]
fn an_unshared_arc_is_unwrapped_and_a_shared_one_is_cloned() {
    let lone = Arc::new(left::Point { x: 1.0, y: 2.0 });
    let out: Arc<right::Point> = Convert::<_, ()>::convert(lone);
    assert_eq!(*out, right::Point { x: 1.0, y: 2.0 });

    let shared = Arc::new(left::Point { x: 3.0, y: 4.0 });
    let keep = shared.clone();
    let out: Arc<right::Point> = Convert::<_, ()>::convert(shared);
    assert_eq!(*out, right::Point { x: 3.0, y: 4.0 });
    assert_eq!(*keep, left::Point { x: 3.0, y: 4.0 }, "the other owner still has its value");
    assert_eq!(Arc::strong_count(&keep), 1);

    let rc = Rc::new(left::Point { x: 5.0, y: 6.0 });
    let also = rc.clone();
    let out: Rc<right::Point> = Convert::<_, ()>::convert(rc);
    assert_eq!((out.x, also.x), (5.0, 5.0));
}

#[test]
fn containers_convert_element_by_element() {
    let p = || left::Point { x: 1.0, y: 2.0 };
    let q = || right::Point { x: 1.0, y: 2.0 };

    assert_eq!(Convert::<Option<right::Point>>::convert(Some(p())), Some(q()));
    assert_eq!(Convert::<Option<right::Point>>::convert(None::<left::Point>), None);
    assert_eq!(Convert::<Vec<right::Point>>::convert(vec![p(), p()]), vec![q(), q()]);
    assert_eq!(Convert::<VecDeque<right::Point>>::convert(VecDeque::from([p()])), VecDeque::from([q()]));
    assert_eq!(Convert::<Box<right::Point>>::convert(Box::new(p())), Box::new(q()));
    assert_eq!(Convert::<[right::Point; 2]>::convert([p(), p()]), [q(), q()]);
    assert_eq!(Convert::<(right::Point, u8, String)>::convert((p(), 3u8, "s".to_string())), (q(), 3, "s".to_string()));
    assert_eq!(
        Convert::<BTreeMap<String, right::Point>>::convert(BTreeMap::from([("k".to_string(), p())])),
        BTreeMap::from([("k".to_string(), q())])
    );
    assert_eq!(Convert::<HashMap<u8, right::Point>>::convert(HashMap::from([(1u8, p())])), HashMap::from([(1u8, q())]));
    assert_eq!(Convert::<BTreeSet<u8>>::convert(BTreeSet::from([1u8, 2])), BTreeSet::from([1u8, 2]));
    assert_eq!(Convert::<HashSet<char>>::convert(HashSet::from(['a'])), HashSet::from(['a']));

    assert_eq!(ConvertRef::<Option<right::Point>>::convert_ref(&Some(p())), Some(q()));
    assert_eq!(ConvertRef::<Vec<right::Point>>::convert_ref(&vec![p()]), vec![q()]);
    assert_eq!(ConvertRef::<Vec<right::Point>>::convert_ref(&[p(), p()][..]), vec![q(), q()]);
    assert_eq!(ConvertRef::<VecDeque<right::Point>>::convert_ref(&VecDeque::from([p()])), VecDeque::from([q()]));
    assert_eq!(ConvertRef::<Box<right::Point>>::convert_ref(&Box::new(p())), Box::new(q()));
    assert_eq!(ConvertRef::<Arc<right::Point>>::convert_ref(&Arc::new(p())), Arc::new(q()));
    assert_eq!(ConvertRef::<Rc<right::Point>>::convert_ref(&Rc::new(p())), Rc::new(q()));
    assert_eq!(ConvertRef::<[right::Point; 2]>::convert_ref(&[p(), p()]), [q(), q()]);
    assert_eq!(ConvertRef::<(right::Point, bool)>::convert_ref(&(p(), true)), (q(), true));
    assert_eq!(ConvertRef::<BTreeMap<u8, right::Point>>::convert_ref(&BTreeMap::from([(1u8, p())])), BTreeMap::from([(1u8, q())]));
    assert_eq!(ConvertRef::<HashMap<u8, right::Point>>::convert_ref(&HashMap::from([(1u8, p())])), HashMap::from([(1u8, q())]));
    assert_eq!(ConvertRef::<BTreeSet<u8>>::convert_ref(&BTreeSet::from([1u8])), BTreeSet::from([1u8]));
    assert_eq!(ConvertRef::<HashSet<u8>>::convert_ref(&HashSet::from([1u8])), HashSet::from([1u8]));
    assert_eq!(ConvertRef::<String>::convert_ref("text"), "text");
}

/// A second mapping between the same two models, kept apart by its marker.
struct Strict;

thread_local! {
    static SEEN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl Floats for Strict {
    fn f32(value: f32) -> f32 {
        if value.is_finite() {
            value
        } else {
            SEEN.with(|seen| seen.set(seen.get() + 1));
            0.0
        }
    }
}

convert_types! {
    marker = Strict;
    struct left::Point => right::Point {
        // A field can take its own expression; `x` is the source field.
        x = x * 2.0,
        y,
    }
    struct &left::Point => right::Point { x = *x * 2.0, y }
}

#[test]
fn a_marker_names_its_own_mapping_and_sees_every_float() {
    let p = left::Point { x: 2.0, y: f32::NAN };
    let plain: right::Point = Convert::<_, ()>::convert(p.clone());
    assert!(plain.x == 2.0 && plain.y.is_nan(), "the default mapping passes floats through");

    SEEN.with(|seen| seen.set(0));
    let strict: right::Point = Convert::<_, Strict>::convert(p.clone());
    assert_eq!((strict.x, strict.y), (4.0, 0.0));
    let nested: Vec<Option<right::Point>> = ConvertRef::<_, Strict>::convert_ref(&vec![Some(p)]);
    assert_eq!(nested, vec![Some(right::Point { x: 4.0, y: 0.0 })]);
    assert_eq!(SEEN.with(std::cell::Cell::get), 2);
}
