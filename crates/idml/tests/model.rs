//! The layout model: spreads, pages, guides, page items, graphics and geometry.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_idml::model::{
    GraphicKind, IDENTITY, ItemChild, ItemKind, MAX_PATH_POINTS, PageChild, PathGeometry, PathPoint, Spread, SpreadChild, SubPath, Values, apply,
    concat, spreads,
};
use craft_idml::{El, Package, designmap_xml, parse_part, part_root, part_xml, write_package};

/// A spread shaped as InDesign writes one: a page with margins and guides, then the page items
/// back to front (an image frame, a threaded text frame, a group, an open line), with elements the
/// model does not type in between.
const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="16.0">
	<Spread Self="ud3" PageCount="1" BindingLocation="0" AllowPageShuffle="true" ItemTransform="1 0 0 1 0 0" ShowMasterItems="true">
		<FlattenerPreference LineArtAndTextResolution="300" />
		<Page Self="ud8" Name="1" AppliedMaster="ud1" OverrideList="ue1 ue2" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 -306 -396">
			<Properties><PageColor type="enumeration">UseMasterColor</PageColor></Properties>
			<MarginPreference ColumnCount="2" ColumnGutter="12" Top="36" Bottom="36" Left="36" Right="36" ColumnsPositions="0 264 276 540" />
			<Guide Self="ug1" Orientation="Horizontal" Location="100" FitToPage="false" ItemLayer="ua" Locked="true" />
			<Guide Self="ug2" Orientation="Vertical" Location="50.5" GuideType="Liquid" />
		</Page>
		<Rectangle Self="ur1" ItemLayer="ua" ItemTransform="1 0 0 1 10 20" AppliedObjectStyle="ObjectStyle/$ID/[Normal Graphics Frame]" FillColor="Swatch/None">
			<Properties>
				<PathGeometry>
					<GeometryPathType PathOpen="false">
						<PathPointArray>
							<PathPointType Anchor="0 0" LeftDirection="0 0" RightDirection="0 0" />
							<PathPointType Anchor="0 100" LeftDirection="0 100" RightDirection="0 100" />
							<PathPointType Anchor="200 100" LeftDirection="200 100" RightDirection="200 100" />
							<PathPointType Anchor="200 0" LeftDirection="200 0" RightDirection="200 0" />
						</PathPointArray>
					</GeometryPathType>
				</PathGeometry>
			</Properties>
			<TextWrapPreference TextWrapMode="None" />
			<Image Self="ui1" ItemTransform="0.5 0 0 0.5 0 0" ActualPpi="72 72">
				<Properties>
					<GraphicBounds Left="0" Top="0" Right="400" Bottom="200" />
					<Contents><![CDATA[aGVsbG8gd29y
bGQ=]]></Contents>
				</Properties>
				<TextWrapPreference TextWrapMode="None" />
				<Link Self="ul1" LinkResourceURI="file:/Users/a/My%20Pictures/p.jpg" LinkResourceFormat="$ID/JPEG" StoredState="Normal" />
				<ClippingPathSettings ClippingType="None" />
			</Image>
		</Rectangle>
		<TextFrame Self="ut1" ParentStory="us1" PreviousTextFrame="n" NextTextFrame="ut2" ItemLayer="ua" ItemTransform="1 0 0 1 0 300" Visible="false">
			<Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="0 0" /><PathPointType Anchor="0 50" /><PathPointType Anchor="100 50" /><PathPointType Anchor="100 0" /></PathPointArray></GeometryPathType></PathGeometry></Properties>
			<TextFramePreference TextColumnCount="2" />
		</TextFrame>
		<Group Self="ug9" ItemTransform="1 0 0 1 100 100" OverriddenPageItem="ue1">
			<Oval Self="uo1" ItemTransform="2 0 0 2 5 5"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray>
				<PathPointType Anchor="0 -1" LeftDirection="-0.55 -1" RightDirection="0.55 -1" />
				<PathPointType Anchor="1 0" LeftDirection="1 -0.55" RightDirection="1 0.55" />
			</PathPointArray></GeometryPathType></PathGeometry></Properties></Oval>
			<Group Self="ug10" ItemTransform="1 0 0 1 1 1"><GraphicLine Self="ul2" /></Group>
		</Group>
		<GraphicLine Self="ul3"><Properties><PathGeometry><GeometryPathType PathOpen="true"><PathPointArray><PathPointType Anchor="0 0" /><PathPointType Anchor="10 10" /></PathPointArray></GeometryPathType></PathGeometry></Properties></GraphicLine>
		<MultiStateObject Self="um1"><State Self="us9"><Rectangle Self="ur9" /></State></MultiStateObject>
	</Spread>
</idPkg:Spread>
"#;

fn spread() -> Spread {
    let part = parse_part(SPREAD.as_bytes()).unwrap();
    Spread::from_el(part.child("Spread").unwrap())
}

#[test]
fn a_spread_reads_as_pages_and_items_in_order() {
    let s = spread();
    assert!(!s.master);
    assert_eq!(s.values.self_id(), Some("ud3"));
    assert_eq!(s.values.item_transform(), IDENTITY);
    let kinds: Vec<String> = s
        .children
        .iter()
        .map(|c| match c {
            SpreadChild::Page(_) => "Page".into(),
            SpreadChild::Item(i) => i.kind.name().to_string(),
            SpreadChild::Other(e) => format!("other {}", e.name),
        })
        .collect();
    assert_eq!(kinds, ["other FlattenerPreference", "Page", "Rectangle", "TextFrame", "Group", "GraphicLine", "MultiStateObject"]);
    let page = s.pages().next().unwrap();
    assert_eq!(page.values.name(), Some("1"));
    assert_eq!(page.geometric_bounds(), Some([0.0, 0.0, 792.0, 612.0]));
    assert_eq!(page.values.item_transform(), [1.0, 0.0, 0.0, 1.0, -306.0, -396.0]);
    assert_eq!(page.applied_master(), Some("ud1"));
    assert_eq!(page.override_list(), ["ue1", "ue2"]);
    assert_eq!(page.values.value("PageColor").as_deref(), Some("UseMasterColor"));
    let margins = page.margin_preference().unwrap();
    assert_eq!((margins.num("ColumnCount"), margins.num("Top"), margins.get("ColumnsPositions")), (Some(2.0), Some(36.0), Some("0 264 276 540")));
    let guides: Vec<_> = page.guides().collect();
    assert_eq!(guides.len(), 2);
    assert!(guides[0].horizontal() && !guides[0].fit_to_page() && guides[0].values.locked() && !guides[0].liquid());
    assert_eq!((guides[0].location(), guides[0].values.item_layer()), (Some(100.0), Some("ua")));
    assert!(!guides[1].horizontal() && guides[1].fit_to_page() && guides[1].liquid());
    assert_eq!(guides[1].location(), Some(50.5));
}

#[test]
fn page_items_give_their_geometry_links_and_threads() {
    let s = spread();
    let items: Vec<_> = s.items().collect();
    let frame = items[0];
    assert_eq!(frame.kind, ItemKind::Rectangle);
    assert_eq!(frame.applied_object_style(), Some("ObjectStyle/$ID/[Normal Graphics Frame]"));
    assert_eq!(frame.values.get("FillColor"), Some("Swatch/None"));
    assert_eq!(frame.geometry().bounds(), Some([0.0, 0.0, 100.0, 200.0]));
    assert_eq!(frame.other("TextWrapPreference").and_then(|w| w.attr("TextWrapMode")), Some("None"));
    let image = frame.graphics().next().unwrap();
    assert_eq!(image.kind, GraphicKind::Image);
    assert_eq!(image.graphic_bounds(), Some([0.0, 0.0, 200.0, 400.0]));
    assert_eq!(image.contents().as_deref(), Some(&b"hello world"[..]));
    let link = image.link().unwrap();
    assert_eq!((link.uri(), link.format()), (Some("file:/Users/a/My%20Pictures/p.jpg"), Some("$ID/JPEG")));
    assert_eq!(craft_idml::value::uri_to_path(link.uri().unwrap()), "/Users/a/My Pictures/p.jpg");

    let text = items[1];
    assert_eq!(text.kind, ItemKind::TextFrame);
    assert_eq!((text.parent_story(), text.previous_text_frame(), text.next_text_frame()), (Some("us1"), None, Some("ut2")));
    assert!(!text.values.visible() && !text.values.locked());
    assert_eq!(text.geometry().paths[0].points[1], PathPoint::corner([0.0, 50.0]), "a missing direction point is the anchor");

    let group = items[2];
    assert_eq!(group.overridden_page_item(), Some("ue1"));
    let oval = group.items().next().unwrap();
    let g = oval.geometry();
    assert_eq!(g.paths[0].points[0], PathPoint { anchor: [0.0, -1.0], left: [-0.55, -1.0], right: [0.55, -1.0] });
    assert_eq!(g.bounds(), Some([-1.0, -0.55, 0.55, 1.0]), "direction points count");

    let line = items[3];
    assert!(line.geometry().paths[0].open);
    // A page item kind the model does not type keeps its content as written.
    assert_eq!(items[4].kind, ItemKind::Other("MultiStateObject".into()));
    assert!(matches!(&items[4].children[..], [ItemChild::Other(e)] if e.name == "State"));
}

#[test]
fn nested_items_carry_their_transform_into_the_spread() {
    let s = spread();
    let all: Vec<(&str, [f64; 6])> = s.all_items().into_iter().map(|(i, m)| (i.values.self_id().unwrap(), m)).collect();
    let ids: Vec<&str> = all.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, ["ur1", "ut1", "ug9", "uo1", "ug10", "ul2", "ul3", "um1"]);
    let oval = all[3].1;
    assert_eq!(oval, [2.0, 0.0, 0.0, 2.0, 105.0, 105.0]);
    assert_eq!(apply(&oval, [1.0, 0.0]), [107.0, 105.0]);
    assert_eq!(all[5].1, [1.0, 0.0, 0.0, 1.0, 101.0, 101.0]);
    assert_eq!(concat(&IDENTITY, &oval), oval);
}

#[test]
fn a_spread_written_and_read_again_is_the_same() {
    let s = spread();
    let bytes = part_xml(&part_root("Spread").with_child(s.to_el()));
    let again = Spread::from_el(parse_part(&bytes).unwrap().child("Spread").unwrap());
    assert_eq!(again, s);
    let xml = String::from_utf8(bytes).unwrap();
    // `<Properties>` stays the first child; the embedded contents and unknown elements survive.
    assert!(xml.contains("<Image Self=\"ui1\" ItemTransform=\"0.5 0 0 0.5 0 0\" ActualPpi=\"72 72\">\n"), "{xml}");
    assert!(
        xml.contains("<Image Self=\"ui1\" ItemTransform=\"0.5 0 0 0.5 0 0\" ActualPpi=\"72 72\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<GraphicBounds"),
        "{xml}"
    );
    assert!(xml.contains("<FlattenerPreference LineArtAndTextResolution=\"300\"/>"), "{xml}");
    assert!(xml.contains("<State Self=\"us9\">"), "{xml}");
}

#[test]
fn a_package_gives_its_spreads_and_parent_spreads() {
    let s = spread();
    let mut parent = Spread { master: true, ..Spread::default() };
    parent.values.set("Self", "ud1");
    parent.values.set("NamePrefix", "A");
    parent.values.set("BaseName", "Parent");
    let designmap = El::new("Document")
        .with_attr("xmlns:idPkg", craft_idml::PACKAGING_NS)
        .with_child(El::new("idPkg:MasterSpread").with_attr("src", "MasterSpreads/MasterSpread_ud1.xml"))
        .with_child(El::new("idPkg:Spread").with_attr("src", "Spreads/Spread_ud3.xml"));
    let bytes = write_package(&[
        ("designmap.xml", designmap_xml(&designmap)),
        ("MasterSpreads/MasterSpread_ud1.xml", part_xml(&part_root("MasterSpread").with_child(parent.to_el()))),
        ("Spreads/Spread_ud3.xml", SPREAD.as_bytes().to_vec()),
    ])
    .unwrap();
    let doc = Package::open(&bytes).unwrap().document().unwrap();
    let found = spreads(&doc);
    assert_eq!(found.len(), 2);
    assert!(found[0].master);
    assert_eq!(found[0].parent_name(), Some(("A", "Parent")));
    assert_eq!(found[1], s);
}

#[test]
fn geometry_reads_writes_and_keeps_only_usable_points() {
    let rect = PathGeometry::rectangle([10.0, 20.0, 30.0, 40.0]);
    assert_eq!(rect.bounds(), Some([10.0, 20.0, 30.0, 40.0]));
    assert_eq!(rect.paths[0].points[0], PathPoint::corner([20.0, 10.0]));
    assert_eq!(PathGeometry::from_el(&rect.to_el()), rect);
    assert_eq!(PathGeometry::default().bounds(), None);

    let pg = parse_part(
        br#"<PathGeometry>
        <GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="0 NaN"/><PathPointType Anchor="1 1"/></PathPointArray></GeometryPathType>
        <GeometryPathType PathOpen="true"><PathPointArray><PathPointType Anchor="0 0" RightDirection="inf 0"/></PathPointArray></GeometryPathType>
        <GeometryPathType><PathPointArray><PathPointType/><PathPointType Anchor="1"/><PathPointType Anchor="2 3" LeftDirection="x"/></PathPointArray></GeometryPathType>
        </PathGeometry>"#,
    )
    .unwrap();
    let g = PathGeometry::from_el(&pg);
    // Subpaths with a non-finite number are dropped whole; points without a usable anchor are
    // dropped; an unusable direction point is the anchor.
    assert_eq!(g, PathGeometry { paths: vec![SubPath { open: false, points: vec![PathPoint::corner([2.0, 3.0])] }] });
}

#[test]
fn geometry_keeps_at_most_the_point_limit() {
    let many = format!(
        "<PathGeometry>{}</PathGeometry>",
        "<GeometryPathType><PathPointArray><PathPointType Anchor=\"1 1\"/><PathPointType Anchor=\"2 2\"/><PathPointType Anchor=\"3 3\"/></PathPointArray></GeometryPathType>".repeat(4)
    );
    let pg = parse_part(many.as_bytes()).unwrap();
    assert_eq!(PathGeometry::from_el(&pg).point_count(), 12);
    let capped = PathGeometry::from_el_capped(&pg, 7);
    assert_eq!(capped.point_count(), 7);
    assert_eq!(capped.paths.iter().map(|p| p.points.len()).collect::<Vec<_>>(), [3, 3, 1]);
    assert_eq!(PathGeometry::from_el_capped(&pg, 0), PathGeometry::default());
    assert_eq!(MAX_PATH_POINTS, 1_000_000);
}

#[test]
fn values_set_and_replace_their_properties() {
    let mut s = spread();
    let SpreadChild::Item(frame) = &mut s.children[2] else { panic!() };
    frame.set_geometry(&PathGeometry::rectangle([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(frame.geometry().bounds(), Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(frame.values.properties.len(), 1, "replaced, not added");
    frame.values.set("FillColor", "Color/Black");
    assert_eq!(frame.values.get("FillColor"), Some("Color/Black"));
    let mut v = Values::default();
    v.set_prop_el("Leading", Some(El::new("Leading").with_attr("type", "unit").with_text("14")));
    assert_eq!(v.num("Leading"), Some(14.0));
    v.set_prop_el("Leading", None);
    assert_eq!(v.prop("Leading"), None);
    assert!(v.element("Thing").children.is_empty());
    // A page's own children other than guides stay where they were.
    let page = s.pages().next().unwrap();
    assert!(matches!(&page.children[0], PageChild::Other(e) if e.name == "MarginPreference"));
}

/// Groups nest as deep as the XML reader allows; reading and walking them does not recurse
/// without bound.
#[test]
fn deeply_nested_groups_are_read_and_walked() {
    let depth = 250;
    let xml =
        format!("<Spread>{}<Rectangle Self=\"deep\"/>{}</Spread>", "<Group ItemTransform=\"1 0 0 1 1 0\">".repeat(depth), "</Group>".repeat(depth));
    let s = Spread::from_el(&parse_part(xml.as_bytes()).unwrap());
    let all = s.all_items();
    assert_eq!(all.len(), depth + 1);
    let (deepest, m) = all.last().unwrap();
    assert_eq!(deepest.values.self_id(), Some("deep"));
    assert_eq!(m[4], depth as f64);
    let again = Spread::from_el(&parse_part(&part_xml(&s.to_el())).unwrap());
    assert_eq!(again, s);
}
