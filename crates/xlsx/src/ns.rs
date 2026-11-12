//! Namespaces: every part is read with one fixed prefix per namespace, whatever prefix the file
//! used, so a tree can be matched by name and written back with [`declare`].
//!
//! SpreadsheetML's main namespace (transitional or strict) has no prefix: `sheetData`, `row`.
//! The others have the prefixes in [`NAMESPACES`]: `r:id`, `mc:AlternateContent`,
//! `x14:dataValidation`, `xm:sqref`, `a:solidFill`. A name in a namespace that is not listed
//! reads as `?:local`; [`craft_xml::El::write_xml`] leaves such names out.

use craft_xml::{AttrValueHook, Canonical, El, Node, Scope};

/// SpreadsheetML, transitional (ECMA-376 Part 1, 18).
pub const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// SpreadsheetML, strict (ISO/IEC 29500-1).
pub const MAIN_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
/// Relationship references (`r:id`).
pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Markup compatibility (ECMA-376 Part 3).
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// (prefix, namespace URI) for every namespace read with a prefix of its own. Where a prefix has
/// several URIs (transitional and strict), the first is the one [`declare`] writes.
pub const NAMESPACES: &[(&str, &str)] = &[
    ("", MAIN),
    ("", MAIN_STRICT),
    ("r", R),
    ("r", "http://purl.oclc.org/ooxml/officeDocument/relationships"),
    ("mc", MC),
    ("x12ac", "http://schemas.microsoft.com/office/spreadsheetml/2011/1/ac"),
    ("x14ac", "http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac"),
    ("x14", "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"),
    ("x15ac", "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"),
    ("x15", "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"),
    ("x16r2", "http://schemas.microsoft.com/office/spreadsheetml/2015/02/main"),
    ("xr", "http://schemas.microsoft.com/office/spreadsheetml/2014/revision"),
    ("xr2", "http://schemas.microsoft.com/office/spreadsheetml/2015/revision2"),
    ("xr3", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision3"),
    ("xr4", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision4"),
    ("xr5", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision5"),
    ("xr6", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision6"),
    ("xr7", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision7"),
    ("xr8", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision8"),
    ("xr9", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision9"),
    ("xr10", "http://schemas.microsoft.com/office/spreadsheetml/2016/revision10"),
    ("xm", "http://schemas.microsoft.com/office/excel/2006/main"),
    ("xda", "http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"),
    ("rv", "http://schemas.microsoft.com/office/spreadsheetml/2017/richdata"),
    ("rvrel", "http://schemas.microsoft.com/office/spreadsheetml/2022/richvaluerel"),
    ("xcalcf", "http://schemas.microsoft.com/office/spreadsheetml/2018/calcfeatures"),
    ("x18tc", "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"),
    ("xpdl", "http://schemas.microsoft.com/office/spreadsheetml/2016/pivotdefaultlayout"),
    ("xfpb", "http://schemas.microsoft.com/office/spreadsheetml/2022/featurepropertybag"),
    ("xpy", "http://schemas.microsoft.com/office/spreadsheetml/2023/python"),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    ("a", "http://purl.oclc.org/ooxml/drawingml/main"),
    ("xdr", "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"),
    ("xdr", "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing"),
    ("c", "http://schemas.openxmlformats.org/drawingml/2006/chart"),
    ("c", "http://purl.oclc.org/ooxml/drawingml/chart"),
    ("cx", "http://schemas.microsoft.com/office/drawing/2014/chartex"),
    ("pic", "http://schemas.openxmlformats.org/drawingml/2006/picture"),
    ("a14", "http://schemas.microsoft.com/office/drawing/2010/main"),
    ("a16", "http://schemas.microsoft.com/office/drawing/2014/main"),
    ("asvg", "http://schemas.microsoft.com/office/drawing/2016/SVG/main"),
    ("aink", "http://schemas.microsoft.com/office/drawing/2016/ink"),
    ("am3d", "http://schemas.microsoft.com/office/drawing/2017/model3d"),
    ("cx1", "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex"),
    ("cx2", "http://schemas.microsoft.com/office/drawing/2015/10/21/chartex"),
    ("cx3", "http://schemas.microsoft.com/office/drawing/2016/5/9/chartex"),
    ("cx4", "http://schemas.microsoft.com/office/drawing/2016/5/10/chartex"),
    ("dgm", "http://schemas.openxmlformats.org/drawingml/2006/diagram"),
    ("v", "urn:schemas-microsoft-com:vml"),
    ("o", "urn:schemas-microsoft-com:office:office"),
    ("x", "urn:schemas-microsoft-com:office:excel"),
    ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
];

/// The markup-compatibility attributes whose values are lists of prefixes (or of `prefix:name`).
const PREFIX_LISTS: &[&str] = &["mc:Ignorable", "mc:ProcessContent", "mc:MustUnderstand", "mc:PreserveElements", "mc:PreserveAttributes"];

/// Rewrites the prefixes in `mc:Ignorable` (and its kin) and `mc:Choice/@Requires` to the
/// canonical ones. In a list, a prefix of an unlisted namespace is left out; in `Requires` it
/// becomes `?`, so that the choice is known to need something this crate cannot carry.
fn mc_prefixes(el: &str, attr: &str, value: &str, scope: &Scope<'_>) -> Option<String> {
    let requires = el == "mc:Choice" && attr == "Requires";
    if !requires && !PREFIX_LISTS.contains(&attr) {
        return None;
    }
    let canon = |p: &str| scope.uri(p).map_or("?", |uri| CANONICAL.prefix(uri));
    let tokens = value.split_whitespace().filter_map(|t| {
        let (p, rest) = t.split_once(':').map_or((t, None), |(p, l)| (p, Some(l)));
        let c = canon(p);
        if requires {
            return Some(c.to_string());
        }
        (c != "?" && !c.is_empty()).then(|| rest.map_or_else(|| c.to_string(), |l| format!("{c}:{l}")))
    });
    Some(tokens.collect::<Vec<_>>().join(" "))
}

const MC_HOOK: &AttrValueHook<'static> = &mc_prefixes;

/// The canonical names parts are read with.
pub const CANONICAL: Canonical<'static> = Canonical { namespaces: NAMESPACES, unknown: "?", element: None, attr_value: Some(MC_HOOK) };

/// Prepares a tree read with [`CANONICAL`] for writing: declares on `root` every namespace its
/// names and markup-compatibility attributes use (the main namespace as the default one), and
/// drops each `mc:Choice` that requires a namespace this crate does not know, since its content
/// was not kept. Declarations already on `root` are replaced.
pub fn declare(root: &mut El) {
    declare_with(root, &[]);
}

/// [`declare`], also declaring the `prefixes` of names written outside the tree (streamed rows).
pub(crate) fn declare_with(root: &mut El, prefixes: &[&str]) {
    drop_unknown_choices(root);
    let mut used: Vec<&str> = prefixes.to_vec();
    collect(root, &mut used);
    let mut decls = vec![("xmlns".to_string(), MAIN.to_string())];
    for (p, uri) in NAMESPACES {
        if !p.is_empty() && used.contains(p) && !decls.iter().any(|(k, _)| k.strip_prefix("xmlns:") == Some(p)) {
            decls.push((format!("xmlns:{p}"), uri.to_string()));
        }
    }
    root.attrs.retain(|(k, _)| k != "xmlns" && !k.starts_with("xmlns:"));
    decls.append(&mut root.attrs);
    root.attrs = decls;
}

/// Drops each `mc:Choice` that requires an unlisted namespace, and an `mc:AlternateContent` left
/// with no choice and no fallback.
fn drop_unknown_choices(e: &mut El) {
    e.children
        .retain(|n| !matches!(n, Node::El(c) if c.name == "mc:Choice" && c.attr("Requires").is_some_and(|r| r.split_whitespace().any(|p| p == "?"))));
    for c in e.elements_mut() {
        drop_unknown_choices(c);
    }
    e.children.retain(|n| !matches!(n, Node::El(c) if c.name == "mc:AlternateContent" && c.elements().next().is_none()));
}

fn collect<'a>(e: &'a El, used: &mut Vec<&'a str>) {
    if let Some((p, _)) = e.name.split_once(':') {
        used.push(p);
    }
    for (k, v) in &e.attrs {
        if let Some((p, _)) = k.split_once(':') {
            used.push(p);
        }
        if PREFIX_LISTS.contains(&k.as_str()) || (e.name == "mc:Choice" && k == "Requires") {
            used.extend(v.split_whitespace().map(|t| t.split_once(':').map_or(t, |(p, _)| p)));
        }
    }
    for c in e.elements() {
        collect(c, used);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str) -> El {
        craft_xml::Options::canonical(&CANONICAL).parse(xml.as_bytes()).unwrap_or_default()
    }

    #[test]
    fn names_get_fixed_prefixes_and_write_back() {
        let x = format!(
            r#"<s:worksheet xmlns:s="{MAIN_STRICT}" xmlns:rel="http://purl.oclc.org/ooxml/officeDocument/relationships" xmlns:m="{MC}" xmlns:e="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:z="urn:future" m:Ignorable="e z"><s:sheetData/><s:drawing rel:id="rId1"/><m:AlternateContent><m:Choice Requires="z"><z:new/></m:Choice><m:Choice Requires="e"><s:a/></m:Choice><m:Fallback><s:b/></m:Fallback></m:AlternateContent></s:worksheet>"#
        );
        let mut root = read(&x);
        assert_eq!(root.attr("mc:Ignorable"), Some("x14ac"));
        assert_eq!(root.child("drawing").and_then(|d| d.attr("r:id")), Some("rId1"));
        let choices: Vec<_> = root.find_all("mc:Choice").filter_map(|c| c.attr("Requires")).collect();
        assert_eq!(choices, ["?", "x14ac"]);
        declare(&mut root);
        let out = root.to_xml();
        assert_eq!(
            out,
            format!(
                r#"<worksheet xmlns="{MAIN}" xmlns:r="{R}" xmlns:mc="{MC}" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" mc:Ignorable="x14ac"><sheetData/><drawing r:id="rId1"/><mc:AlternateContent><mc:Choice Requires="x14ac"><a/></mc:Choice><mc:Fallback><b/></mc:Fallback></mc:AlternateContent></worksheet>"#
            )
        );
        // What was written reads back to the same tree.
        let mut again = read(&out);
        declare(&mut again);
        assert_eq!(again, root);
    }

    #[test]
    fn extension_lists_keep_their_namespaces() {
        let x = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:dataValidations xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" count="1"><x14:dataValidation type="list"><x14:formula1><xm:f>Other!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>G1:G4</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst></worksheet>"#;
        let mut root = read(x);
        assert!(root.find("dataValidation").is_none(), "an extension's element is not the main one");
        assert_eq!(root.find("xm:sqref").map(|e| e.text().into_owned()), Some("G1:G4".into()));
        declare(&mut root);
        assert!(root.to_xml().starts_with(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="#));
        assert_eq!(read(&root.to_xml()), read(x));
    }
}
