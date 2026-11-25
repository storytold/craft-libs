//! Styles, swatches, stroke styles, colour groups and preferences.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_idml::model::{
    ColorSpace, GradientStop, StrokeStyleKind, StyleGroup, StyleKind, StyleNode, SwatchKind, color_groups, named, preference, stroke_styles,
    style_groups, swatches,
};
use craft_idml::{El, parse_part, part_root, part_xml};

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="16.0">
	<RootCharacterStyleGroup Self="u7a">
		<CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]" />
		<CharacterStyle Self="CharacterStyle/Em" Name="Em" FontStyle="Italic"><Properties><BasedOn type="object">CharacterStyle/$ID/[No character style]</BasedOn></Properties></CharacterStyle>
		<CharacterStyleGroup Self="CharacterStyleGroup/Inline" Name="Inline">
			<CharacterStyleGroup Self="CharacterStyleGroup/Inline%3aCode" Name="Code">
				<CharacterStyle Self="CharacterStyle/Inline%3aCode%3aMono" Name="Inline:Code:Mono" />
			</CharacterStyleGroup>
		</CharacterStyleGroup>
	</RootCharacterStyleGroup>
	<RootParagraphStyleGroup Self="u79">
		<ParagraphStyle Self="ParagraphStyle/Head" Name="Head" PointSize="18" NextStyle="ParagraphStyle/Body">
			<Properties><BasedOn type="string">$ID/[No paragraph style]</BasedOn><Leading type="unit">22</Leading></Properties>
		</ParagraphStyle>
		<ParagraphStyle Self="ParagraphStyle/Body" Name="Body" />
	</RootParagraphStyleGroup>
	<RootObjectStyleGroup Self="u8b">
		<ObjectStyle Self="ObjectStyle/Frame" Name="Frame" FillColor="Color/Paper">
			<TextFramePreference TextColumnCount="3" />
			<TransparencySetting><BlendingSetting Opacity="50" /></TransparencySetting>
		</ObjectStyle>
	</RootObjectStyleGroup>
	<RootCellStyleGroup Self="u8c"><CellStyle Self="CellStyle/$ID/[None]" Name="$ID/[None]" /></RootCellStyleGroup>
	<RootTableStyleGroup Self="u8d"><TableStyle Self="TableStyle/$ID/[Basic Table]" Name="$ID/[Basic Table]" /></RootTableStyleGroup>
	<TOCStyle Self="TOCStyle/$ID/DefaultTOCStyleName" Name="$ID/DefaultTOCStyleName" />
</idPkg:Styles>
"#;

fn top(xml: &str) -> Vec<El> {
    parse_part(xml.as_bytes()).unwrap().elements().cloned().collect()
}

#[test]
fn style_groups_give_their_styles_with_their_group_path() {
    let groups = style_groups(&top(STYLES));
    let kinds: Vec<StyleKind> = groups.iter().map(|g| g.kind).collect();
    assert_eq!(kinds, [StyleKind::Character, StyleKind::Paragraph, StyleKind::Object, StyleKind::Cell, StyleKind::Table]);
    assert!(groups.iter().all(|g| g.root));
    let chars = groups[0].styles();
    assert_eq!(
        chars.iter().map(|(path, s)| (path.join("/"), s.values.name().unwrap())).collect::<Vec<_>>(),
        [(String::new(), "$ID/[No character style]"), (String::new(), "Em"), ("Inline/Code".to_string(), "Inline:Code:Mono"),]
    );
    let em = chars[1].1;
    assert_eq!(
        (em.kind, em.based_on().as_deref(), em.values.get("FontStyle")),
        (StyleKind::Character, Some("CharacterStyle/$ID/[No character style]"), Some("Italic"))
    );
    let head = groups[1].styles()[0].1;
    assert_eq!((head.based_on().as_deref(), head.next_style().as_deref()), (Some("$ID/[No paragraph style]"), Some("ParagraphStyle/Body")));
    assert_eq!((head.values.num("PointSize"), head.values.num("Leading")), (Some(18.0), Some(22.0)));
    let frame = groups[2].styles()[0].1;
    assert_eq!(frame.other("TextFramePreference").and_then(|p| p.attr("TextColumnCount")), Some("3"));
    assert!(frame.other("TransparencySetting").is_some());
    // A group inside a group is its own node, with its own values.
    let StyleNode::Group(inline) = &groups[0].children[2] else { panic!() };
    assert!(!inline.root);
    assert_eq!(inline.styles()[0].0, ["Inline", "Code"], "a group's own name starts its path");
    // Not a root style group.
    assert_eq!(StyleGroup::from_el(&El::new("TOCStyle")), None);
}

#[test]
fn style_groups_written_and_read_again_are_the_same() {
    let groups = style_groups(&top(STYLES));
    let mut part = part_root("Styles");
    for g in &groups {
        part.push(g.to_el());
    }
    assert_eq!(style_groups(&top(&String::from_utf8(part_xml(&part)).unwrap())), groups);
}

const GRAPHIC: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="16.0">
	<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black" />
	<Color Self="Color/Brand" Model="Spot" Space="RGB" ColorValue="255 128 0" Name="Brand" />
	<Color Self="Color/Lab" Model="Process" Space="LAB" ColorValue="50 -20 30" Name="Lab" />
	<Color Self="Color/Broken" Model="Process" Space="CMYK" ColorValue="0 NaN 0 0" Name="Broken" />
	<Ink Self="Ink/$ID/Process Cyan" Name="$ID/Process Cyan" />
	<Ink Self="Ink/Brand" Name="Brand" ConvertToProcess="true" AliasInkName="Other" />
	<Tint Self="Tint/Brand 40" BaseColor="Color/Brand" TintValue="40" Name="" />
	<Gradient Self="Gradient/Fade" Type="Radial" Name="Fade">
		<GradientStop Self="Gradient/FadeGradientStop0" StopColor="Color/Black" Location="0" />
		<GradientStop Self="Gradient/FadeGradientStop1" StopColor="Color/Brand" Location="100" Midpoint="30" />
		<GradientStop Self="Gradient/FadeGradientStop2" Location="50" />
	</Gradient>
	<Swatch Self="Swatch/None" Name="None" />
	<DashedStrokeStyle Self="DashedStrokeStyle/Dash" Name="Dash" DashArray="4 2 1 2" />
	<StripedStrokeStyle Self="StripedStrokeStyle/Rails" Name="Rails" StripeArray="0 25 75 100" />
	<DottedStrokeStyle Self="DottedStrokeStyle/Dots" Name="Dots" />
	<ColorGroup Self="ColorGroup/[Root Color Group]" Name="[Root Color Group]" IsRootColorGroup="true">
		<ColorGroupSwatch Self="u18ColorGroupSwatch0" SwatchItemRef="Swatch/None" />
		<ColorGroupSwatch Self="u18ColorGroupSwatch1" SwatchItemRef="Color/Black" />
	</ColorGroup>
</idPkg:Graphic>
"#;

#[test]
fn swatches_give_their_colours_tints_and_gradients() {
    let t = top(GRAPHIC);
    let sw = swatches(&t);
    assert_eq!(
        sw.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [SwatchKind::Color, SwatchKind::Color, SwatchKind::Color, SwatchKind::Color, SwatchKind::Tint, SwatchKind::Gradient, SwatchKind::Swatch]
    );
    assert_eq!((sw[0].spot(), sw[0].space(), sw[0].color_value()), (false, ColorSpace::Cmyk, vec![0.0, 0.0, 0.0, 100.0]));
    assert_eq!((sw[1].spot(), sw[1].space(), sw[1].color_value()), (true, ColorSpace::Rgb, vec![255.0, 128.0, 0.0]));
    assert_eq!(sw[2].space(), ColorSpace::Lab);
    assert_eq!(sw[3].color_value(), Vec::<f64>::new(), "a value that is not finite is no value");
    assert_eq!((sw[4].base_color(), sw[4].tint_value()), (Some("Color/Brand"), Some(40.0)));
    assert!(sw[5].radial());
    assert_eq!(
        sw[5].stops(),
        [
            GradientStop { color: "Color/Black".into(), location: 0.0, midpoint: 50.0 },
            GradientStop { color: "Color/Brand".into(), location: 100.0, midpoint: 30.0 },
        ]
    );
    let again: Vec<El> = top(&String::from_utf8(part_xml(&sw.iter().fold(part_root("Graphic"), |p, s| p.with_child(s.to_el())))).unwrap());
    assert_eq!(swatches(&again), sw);
}

#[test]
fn inks_stroke_styles_and_colour_groups() {
    let t = top(GRAPHIC);
    let inks = named(&t, "Ink");
    assert_eq!(inks.len(), 2);
    assert_eq!((inks[1].boolean("ConvertToProcess"), inks[1].get("AliasInkName")), (Some(true), Some("Other")));
    let strokes = stroke_styles(&t);
    assert_eq!(strokes.iter().map(|s| s.kind).collect::<Vec<_>>(), [StrokeStyleKind::Dashed, StrokeStyleKind::Striped, StrokeStyleKind::Dotted]);
    assert_eq!(strokes[0].dash_array(), [4.0, 2.0, 1.0, 2.0]);
    assert_eq!(strokes[1].stripe_array(), [0.0, 25.0, 75.0, 100.0]);
    let groups = color_groups(&t);
    assert!(groups[0].is_root());
    assert_eq!(groups[0].members(), ["Swatch/None", "Color/Black"]);
    let again = top(&String::from_utf8(part_xml(&part_root("Graphic").with_child(groups[0].to_el()).with_child(strokes[1].to_el()))).unwrap());
    assert_eq!(color_groups(&again), groups);
    assert_eq!(stroke_styles(&again), [strokes[1].clone()]);
}

#[test]
fn preferences_are_found_by_name() {
    let prefs = top(
        r#"<idPkg:Preferences><DocumentPreference PageWidth="612" PageHeight="792" FacingPages="true"><Properties><ColumnDirection type="enumeration">Horizontal</ColumnDirection></Properties></DocumentPreference><DocumentPreference PageWidth="1"/><GridPreference BaselineDivision="12"/></idPkg:Preferences>"#,
    );
    let doc = preference(&prefs, "DocumentPreference").unwrap();
    assert_eq!(
        (doc.num("PageWidth"), doc.boolean("FacingPages"), doc.value("ColumnDirection").as_deref()),
        (Some(612.0), Some(true), Some("Horizontal"))
    );
    assert_eq!(preference(&prefs, "GridPreference").and_then(|g| g.num("BaselineDivision")), Some(12.0));
    assert_eq!(preference(&prefs, "MarginPreference"), None);
    assert_eq!(named(&prefs, "DocumentPreference").len(), 2);
}

/// Style groups nest as deep as the XML reader allows; their styles are gathered without
/// recursion.
#[test]
fn deeply_nested_style_groups_are_read() {
    let depth = 200;
    let xml = format!(
        "<RootParagraphStyleGroup>{}<ParagraphStyle Name=\"deep\"/>{}</RootParagraphStyleGroup>",
        "<ParagraphStyleGroup Name=\"g\">".repeat(depth),
        "</ParagraphStyleGroup>".repeat(depth)
    );
    let g = StyleGroup::from_el(&parse_part(xml.as_bytes()).unwrap()).unwrap();
    let styles = g.styles();
    assert_eq!((styles.len(), styles[0].0.len(), styles[0].1.values.name()), (1, depth, Some("deep")));
}
