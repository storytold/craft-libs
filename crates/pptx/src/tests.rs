use super::*;

#[test]
fn serde_roundtrip() {
    let mut p = Presentation::default();
    p.masters.push(Arc::new(Master { layouts: vec![Layout { id: LayoutId(2), ..Default::default() }], ..Default::default() }));
    p.slides.push(Arc::new(Slide { id: SlideId(3), layout: LayoutId(2), ..Default::default() }));
    let j = serde_json::to_string(&p).unwrap();
    let q: Presentation = serde_json::from_str(&j).unwrap();
    assert_eq!(p, q);
    // Unknown and missing fields are tolerated.
    let q: Presentation = serde_json::from_str(r#"{"slides":[],"zzz":1}"#).unwrap();
    assert!(q.slides.is_empty());
    assert_eq!(q.slide_size, Size::new(960.0, 540.0));
    // A side that is missing, null or not a number reads as NaN instead of failing the file.
    let q: Presentation = serde_json::from_str(r#"{"slideSize":{"width":720,"height":null}}"#).unwrap();
    assert_eq!(q.slide_size.width, 720.0);
    assert!(q.slide_size.height.is_nan());
}

/// A deck whose ids are used up (a hostile file can arrive that way) yields no further ids:
/// allocating never repeats one.
#[test]
fn exhausted_id_space_allocates_nothing() {
    let mut p = Presentation { next_id: u32::MAX, ..Default::default() };
    assert_eq!(p.alloc_id(), None);
    assert_eq!(p.alloc_id(), None);
    assert!(p.add_media("x.png", "image/png", vec![9]).is_none(), "fresh media bytes are refused too");
}

#[test]
fn ids_are_unique_and_fix_next_id() {
    let mut p = Presentation::default();
    let shape = Shape { id: ShapeId(40), ..Default::default() };
    p.slides.push(Arc::new(Slide { id: SlideId(30), layout: LayoutId(50), shapes: vec![shape], ..Default::default() }));
    p.masters.push(Arc::new(Master { id: MasterId(7), layouts: vec![Layout { id: LayoutId(50), ..Default::default() }], ..Default::default() }));
    p.fix_next_id();
    assert_eq!(p.next_id, 50);
    assert_eq!(p.alloc_id(), Some(51));
    let m = p.add_media("a.png", "image/png", vec![1, 2]).unwrap();
    assert_eq!(p.add_media("b.png", "image/png", vec![1, 2]), Some(m), "identical bytes are shared");
    assert!(p.validate().is_empty(), "{:?}", p.validate());
    Arc::make_mut(&mut p.slides[0]).layout = LayoutId(99);
    assert_eq!(p.validate(), ["slide 30 uses missing layout 99"]);
}

#[test]
fn group_find_and_walk() {
    let child = Shape { id: ShapeId(900), ..Default::default() };
    let g = Shape { id: ShapeId(901), kind: ShapeKind::Group { children: vec![child], child: Xfrm::new(0.0, 0.0, 1.0, 1.0) }, ..Default::default() };
    let mut slide = Slide { shapes: vec![g], ..Default::default() };
    assert!(slide.shape(ShapeId(900)).is_some());
    assert!(slide.shape_mut(ShapeId(900)).is_some());
    let mut n = 0;
    walk(&slide.shapes, &mut |_, _| n += 1);
    assert_eq!(n, 2);
}

#[test]
fn walks_stop_at_depth_64() {
    let mut s = Shape { id: ShapeId(1), ..Default::default() };
    for i in 2..100 {
        s = Shape { id: ShapeId(i), kind: ShapeKind::Group { children: vec![s], child: Xfrm::default() }, ..Default::default() };
    }
    let tree = vec![s];
    let mut n = 0;
    walk(&tree, &mut |_, _| n += 1);
    assert_eq!(n, 65);
    assert!(find_shape(&tree, ShapeId(1)).is_none());
    assert!(find_shape(&tree, ShapeId(40)).is_some());
}

#[test]
fn sibling_lists_and_parents_reach_into_groups() {
    let leaf = |id: u32| Shape { id: ShapeId(id), ..Default::default() };
    let group = |id: u32, children: Vec<Shape>| Shape {
        id: ShapeId(id),
        kind: ShapeKind::Group { children, child: Default::default() },
        ..Default::default()
    };
    let mut tree = vec![leaf(1), group(2, vec![leaf(3), group(4, vec![leaf(5)])]), leaf(6)];
    assert_eq!(parent_of(&tree, ShapeId(1)), Some(None));
    assert_eq!(parent_of(&tree, ShapeId(3)), Some(Some(ShapeId(2))));
    assert_eq!(parent_of(&tree, ShapeId(5)), Some(Some(ShapeId(4))));
    assert_eq!(parent_of(&tree, ShapeId(9)), None);
    let ids = |v: &Vec<Shape>| v.iter().map(|s| s.id.0).collect::<Vec<_>>();
    assert_eq!(sibling_list_mut(&mut tree, ShapeId(6)).map(|v| ids(v)), Some(vec![1, 2, 6]));
    assert_eq!(sibling_list_mut(&mut tree, ShapeId(4)).map(|v| ids(v)), Some(vec![3, 4]));
    assert_eq!(sibling_list_mut(&mut tree, ShapeId(5)).map(|v| ids(v)), Some(vec![5]));
    assert!(sibling_list_mut(&mut tree, ShapeId(9)).is_none());
}

#[test]
fn ink_width_follows_pressure_and_keeps_the_old_placeholder() {
    assert!(ink_pressure_is_legacy(&[(0.0, 0.0, 0.5), (1.0, 1.0, 0.5)]));
    assert!(!ink_pressure_is_legacy(&[(0.0, 0.0, 0.5), (1.0, 1.0, 1.0)]));
    assert!(!ink_pressure_is_legacy(&[]));
    assert!((ink_sample_width(8.0, 0.5, true) - 8.0).abs() < 1e-9);
    assert!((ink_sample_width(8.0, 1.0, false) - 8.0).abs() < 1e-9);
    assert!((ink_sample_width(8.0, 0.25, false) - 2.0).abs() < 1e-9);
    let floor = ink_sample_width(8.0, 0.0, false);
    assert!((floor - 0.64).abs() < 1e-4, "a light touch stays visible, got {floor}");
    assert!((ink_sample_width(8.0, f32::NAN, false) - 8.0).abs() < 1e-9);
}

#[test]
fn colour_maps_alias_the_background_and_text_slots() {
    let mut m = Master::default();
    let base = m.theme.colors.clone();
    assert_eq!(m.map_slot(SchemeSlot::Bg1), SchemeSlot::Lt1, "the default mapping when empty");
    assert_eq!(m.map_slot(SchemeSlot::Accent2), SchemeSlot::Accent2);
    let dark: Vec<(String, String)> = [("bg1", "dk1"), ("tx1", "lt1")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let s = m.scheme_with(&dark);
    assert_eq!(s.get(SchemeSlot::Lt1), base.get(SchemeSlot::Dk1));
    assert_eq!(s.get(SchemeSlot::Dk1), base.get(SchemeSlot::Lt1));
    assert_eq!(s.get(SchemeSlot::Lt2), base.get(SchemeSlot::Lt2), "bg2 keeps its default mapping");
    m.color_map = dark;
    assert_eq!(m.scheme(), s);
}

#[test]
fn enum_names_round_trip_through_their_xml() {
    for k in [PhType::Title, PhType::CtrTitle, PhType::Table, PhType::Diagram, PhType::SlideImage, PhType::SlideNum] {
        assert_eq!(PhType::from_xml(k.xml()), k);
    }
    assert_eq!(PhType::from_xml("zzz"), PhType::Body);
    for k in [LayoutType::Title, LayoutType::Comparison, LayoutType::VerticalTitleAndText, LayoutType::Custom] {
        assert_eq!(LayoutType::from_xml(k.xml()), k);
    }
    assert_eq!(LayoutType::from_xml("twoColTx"), LayoutType::TwoContent);
}
