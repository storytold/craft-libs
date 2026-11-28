use craft_odf::style::Styles;
use craft_odf::units;

use crate::{NS, tree};

fn styles_part(styles: &str, automatic: &str, masters: &str) -> craft_xml::El {
    tree(&format!(
        r##"<office:document-styles {NS}><office:font-face-decls><style:font-face style:name="Liberation Sans" svg:font-family="'Liberation Sans'" style:font-family-generic="swiss"/></office:font-face-decls><office:styles>{styles}</office:styles><office:automatic-styles>{automatic}</office:automatic-styles><office:master-styles>{masters}</office:master-styles></office:document-styles>"##
    ))
}

fn content(automatic: &str) -> craft_xml::El {
    tree(&format!(r##"<office:document-content {NS}><office:automatic-styles>{automatic}</office:automatic-styles></office:document-content>"##))
}

#[test]
fn a_style_shows_its_default_its_ancestors_and_itself_nearest_last() {
    let part = styles_part(
        r##"<style:default-style style:family="table-cell"><style:text-properties fo:font-size="10pt" style:font-name="Liberation Sans"/></style:default-style>
           <style:style style:name="Default" style:family="table-cell"><style:table-cell-properties fo:border-left="0.74pt solid #000000" fo:padding-left="2pt"/></style:style>
           <style:style style:name="Heading" style:display-name="Main Heading" style:family="table-cell" style:parent-style-name="Default"><style:text-properties fo:font-weight="bold" fo:font-size="14pt"/></style:style>"##,
        "",
        "",
    );
    let auto = content(
        r##"<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Main Heading" style:data-style-name="N2"><style:table-cell-properties fo:border="none" fo:padding="1pt"/></style:style>
           <style:style style:name="Heading" style:family="table-cell"><style:text-properties fo:color="#ff0000"/></style:style>"##,
    );
    let s = Styles::read(Some(&part), Some(&auto));
    let p = s.resolve("table-cell", Some("ce1"));
    // The nearest wins, and what it does not state comes from farther up.
    assert_eq!(p.get("style:text-properties", "fo:font-size"), Some("14pt"));
    assert_eq!(p.get("style:text-properties", "fo:font-weight"), Some("bold"));
    assert_eq!(p.get("style:text-properties", "style:font-name"), Some("Liberation Sans"));
    // A shorthand replaces the sides it sets: no inherited left border or padding survives.
    let cell = p.group("style:table-cell-properties").unwrap();
    assert_eq!((cell.attr("fo:border-left"), cell.attr("fo:padding-left")), (None, None));
    assert!(units::border(cell.attr("fo:border").unwrap()).is_none());
    assert_eq!(cell.attr("fo:padding").and_then(units::length_pt), Some(1.0));
    // An automatic style of a common style's name is found first; a parent is always common.
    assert_eq!(s.style("table-cell", "Heading").and_then(|h| h.properties.get("style:text-properties", "fo:color")), Some("#ff0000"));
    assert_eq!(s.chain("table-cell", "ce1").iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["ce1", "Heading", "Default"]);
    assert_eq!(s.style("table-cell", "ce1").and_then(|c| c.data_style.as_deref()), Some("N2"));
    assert_eq!(s.font("Liberation Sans").map(|f| (f.family.as_str(), f.generic.as_deref())), Some(("Liberation Sans", Some("swiss"))));
    // No style at all is the default style's.
    assert_eq!(s.resolve("table-cell", None).get("style:text-properties", "fo:font-size"), Some("10pt"));
}

#[test]
fn a_circular_or_missing_parent_ends_the_chain() {
    let part = styles_part(
        r##"<style:style style:name="A" style:family="paragraph" style:parent-style-name="B"><style:text-properties fo:color="#000001"/></style:style>
           <style:style style:name="B" style:family="paragraph" style:parent-style-name="A"><style:text-properties fo:font-size="9pt"/></style:style>
           <style:style style:name="C" style:family="paragraph" style:parent-style-name="Nowhere"/>"##,
        "",
        "",
    );
    let s = Styles::read(Some(&part), None);
    assert_eq!(s.chain("paragraph", "A").len(), 2);
    assert_eq!(s.resolve("paragraph", Some("A")).get("style:text-properties", "fo:font-size"), Some("9pt"));
    assert_eq!(s.chain("paragraph", "C").len(), 1);
    assert!(s.style("table-cell", "A").is_none(), "a name is looked up within its family");
}

#[test]
fn page_layouts_master_pages_conditions_and_data_styles_are_found() {
    let part = styles_part(
        r##"<style:style style:name="Good" style:family="table-cell"/>
           <style:style style:name="Cond" style:family="table-cell"><style:map style:condition="cell-content()&gt;0" style:apply-style-name="Good" style:base-cell-address="Sheet1.A1"/></style:style>
           <number:number-style style:name="N2"><number:number number:decimal-places="2" number:min-integer-digits="1"/></number:number-style>"##,
        r##"<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" style:print-orientation="portrait"/><style:header-style><style:header-footer-properties fo:min-height="0.75cm"/></style:header-style></style:page-layout>"##,
        r##"<style:master-page style:name="Default" style:page-layout-name="pm1"><style:header><text:p xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">Top</text:p></style:header></style:master-page>"##,
    );
    let s = Styles::read(Some(&part), None);
    let layout = s.page_layout("pm1").unwrap();
    assert_eq!(layout.page.as_ref().and_then(|p| p.attr("fo:page-width")).and_then(units::length_pt).map(f64::round), Some(595.0));
    assert!(layout.header.is_some() && layout.footer.is_none());
    let master = s.master_page("Default").unwrap();
    assert_eq!(master.attr("style:page-layout-name"), Some("pm1"));
    assert!(master.find("text:p").is_some());
    let map = &s.style("table-cell", "Cond").unwrap().maps[0];
    assert_eq!((map.condition.as_str(), map.apply.as_str(), map.base_cell.as_deref()), ("cell-content()>0", "Good", Some("Sheet1.A1")));
    assert_eq!(s.data_styles().format_code("N2").as_deref(), Some("0.00"));
}
