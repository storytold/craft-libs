//! `word/settings.xml` merged into the file's original settings part.
//!
//! The model owns only some settings (`super::settings_xml`). Regenerating the part from the
//! model alone drops everything else Word stores there: compatibility options, rsids, document
//! variables, proofing state, the colour scheme mapping, "remove personal information on save"…
//! So when a file brought settings the model doesn't hold ([`has_unowned`]; the reader keeps its
//! bytes under `SETTINGS_ORIG`), the writer merges instead ([`merge`]): the original root start
//! tag (namespace declarations, `mc:Ignorable`) and every child the model doesn't own are kept
//! byte for byte; owned children come from the generated part (or go, when the model turned
//! them off); owned children the original lacks are inserted at their CT_Settings position; in
//! `w:compat` only the `compatibilityMode` setting is replaced. Kept children that refer to a
//! relationship are dropped unless owned: mail merge is regenerated with remapped external
//! sources by `mail_merge`. Anything unexpected returns `None`, and the generated part is written as before.

/// Children of `w:settings` the model writes (`super::settings_xml`).
const OWNED: &[&str] = &[
    "w:zoom",
    "w:removePersonalInformation",
    "w:removeDateAndTime",
    "w:displayBackgroundShape",
    "w:mirrorMargins",
    "w:gutterAtTop",
    "w:hideSpellingErrors",
    "w:hideGrammaticalErrors",
    "w:attachedTemplate",
    "w:linkStyles",
    "w:mailMerge",
    "w:trackRevisions",
    "w:doNotTrackMoves",
    "w:documentProtection",
    "w:defaultTabStop",
    "w:autoHyphenation",
    "w:consecutiveHyphenLimit",
    "w:hyphenationZone",
    "w:doNotHyphenateCaps",
    "w:evenAndOddHeaders",
    "w:drawingGridHorizontalSpacing",
    "w:drawingGridVerticalSpacing",
    "w:characterSpacingControl",
    "w:footnotePr",
    "w:endnotePr",
    "m:mathPr",
];

/// Owned children whose original, when there is one, is kept as is.
const KEEP_ORIGINAL: &[&str] = &["m:mathPr", "w:displayBackgroundShape"];

/// CT_Settings' sequence (ECMA-376 §17.15.1.78), for placing inserted children.
const ORDER: &[&str] = &[
    "w:writeProtection",
    "w:view",
    "w:zoom",
    "w:removePersonalInformation",
    "w:removeDateAndTime",
    "w:doNotDisplayPageBoundaries",
    "w:displayBackgroundShape",
    "w:printPostScriptOverText",
    "w:printFractionalCharacterWidth",
    "w:printFormsData",
    "w:embedTrueTypeFonts",
    "w:embedSystemFonts",
    "w:saveSubsetFonts",
    "w:saveFormsData",
    "w:mirrorMargins",
    "w:alignBordersAndEdges",
    "w:bordersDoNotSurroundHeader",
    "w:bordersDoNotSurroundFooter",
    "w:gutterAtTop",
    "w:hideSpellingErrors",
    "w:hideGrammaticalErrors",
    "w:activeWritingStyle",
    "w:proofState",
    "w:formsDesign",
    "w:attachedTemplate",
    "w:linkStyles",
    "w:stylePaneFormatFilter",
    "w:stylePaneSortMethod",
    "w:documentType",
    "w:mailMerge",
    "w:revisionView",
    "w:trackRevisions",
    "w:doNotTrackMoves",
    "w:doNotTrackFormatting",
    "w:documentProtection",
    "w:autoFormatOverride",
    "w:styleLockTheme",
    "w:styleLockQFSet",
    "w:defaultTabStop",
    "w:autoHyphenation",
    "w:consecutiveHyphenLimit",
    "w:hyphenationZone",
    "w:doNotHyphenateCaps",
    "w:showEnvelope",
    "w:summaryLength",
    "w:clickAndTypeStyle",
    "w:defaultTableStyle",
    "w:evenAndOddHeaders",
    "w:bookFoldRevPrinting",
    "w:bookFoldPrinting",
    "w:bookFoldPrintingSheets",
    "w:drawingGridHorizontalSpacing",
    "w:drawingGridVerticalSpacing",
    "w:displayHorizontalDrawingGridEvery",
    "w:displayVerticalDrawingGridEvery",
    "w:doNotUseMarginsForDrawingGridOrigin",
    "w:drawingGridHorizontalOrigin",
    "w:drawingGridVerticalOrigin",
    "w:doNotShadeFormData",
    "w:noPunctuationKerning",
    "w:characterSpacingControl",
    "w:printTwoOnOne",
    "w:strictFirstAndLastChars",
    "w:noLineBreaksAfter",
    "w:noLineBreaksBefore",
    "w:savePreviewPicture",
    "w:doNotValidateAgainstSchema",
    "w:saveInvalidXml",
    "w:ignoreMixedContent",
    "w:alwaysShowPlaceholderText",
    "w:doNotDemarcateInvalidXml",
    "w:saveXmlDataOnly",
    "w:useXSLTWhenSaving",
    "w:saveThroughXslt",
    "w:showXMLTags",
    "w:alwaysMergeEmptyNamespace",
    "w:updateFields",
    "w:hdrShapeDefaults",
    "w:footnotePr",
    "w:endnotePr",
    "w:compat",
    "w:docVars",
    "w:rsids",
    "m:mathPr",
    "w:attachedSchema",
    "w:themeFontLang",
    "w:clrSchemeMapping",
    "w:doNotIncludeSubdocsInStats",
    "w:doNotAutoCompressPictures",
    "w:forceUpgrade",
    "w:captions",
    "w:readModeInkLockDown",
    "w:smartTagType",
    "sl:schemaLibrary",
    "w:shapeDefaults",
    "w:doNotEmbedSmartTags",
    "w:decimalSymbol",
    "w:listSeparator",
];

fn rank(name: &str) -> usize {
    ORDER.iter().position(|n| *n == name).unwrap_or(ORDER.len())
}

/// A child element of the root: its qualified name and byte span.
struct Child {
    name: String,
    start: usize,
    end: usize,
}

/// Byte index of the `>` that ends the tag at the start of `s`, skipping quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// The root start tag's span start, span end, close tag start, and the root's direct children,
/// or `None` for anything the scanner doesn't expect (CDATA, DOCTYPE, a different root).
fn children(xml: &str, root: &str) -> Option<(usize, usize, usize, Vec<Child>)> {
    let open = xml.find(&format!("<{root}"))?;
    let open_end = open + tag_end(xml.get(open..)?)? + 1;
    if xml.get(open_end - 2..open_end)? == "/>" {
        return None;
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = open_end;
    let mut cur: Option<(String, usize)> = None;
    let close = loop {
        let rest = xml.get(i..)?;
        let lt = rest.find('<')?;
        let at = i + lt;
        let tail = xml.get(at..)?;
        if tail.starts_with("<!--") {
            i = at + tail.find("-->")? + 3;
            continue;
        }
        if tail.starts_with("<![CDATA[") || tail.starts_with("<!") {
            return None;
        }
        if tail.starts_with("<?") {
            i = at + tail.find("?>")? + 2;
            continue;
        }
        let gt = at + tag_end(tail)? + 1;
        let tag = xml.get(at..gt)?;
        if tag.starts_with("</") {
            if depth == 0 {
                let inside = tag.get(2..tag.len().checked_sub(1)?)?.trim();
                if inside == root {
                    break at;
                }
                return None;
            }
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let (name, s) = cur.take()?;
                out.push(Child { name, start: s, end: gt });
            }
        } else {
            let name: String = tag.get(1..)?.chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
            let selfclose = tag.ends_with("/>");
            if depth == 0 {
                if selfclose {
                    out.push(Child { name, start: at, end: gt });
                } else {
                    cur = Some((name, at));
                    depth = 1;
                }
            } else if !selfclose {
                depth += 1;
            }
        }
        i = gt;
    };
    if depth != 0 {
        return None;
    }
    Some((open, open_end, close, out))
}

fn find_close_tag(xml: &str, tag_name: &str) -> Option<usize> {
    children(xml, tag_name).map(|(_, _, close, _)| close)
}

const WML: &[u8] = b"http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const WML_STRICT: &[u8] = b"http://purl.oclc.org/ooxml/wordprocessingml/main";
const REL_NS: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_STRICT_NS: &[u8] = b"http://purl.oclc.org/ooxml/officeDocument/relationships";

fn is_rel_ns(ns: &[u8]) -> bool {
    let unesc = unescaped_ns(ns);
    unesc.as_ref() == REL_NS || unesc.as_ref() == REL_STRICT_NS
}

fn unescaped_ns(ns: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if ns.contains(&b'&')
        && let Ok(s) = std::str::from_utf8(ns)
        && let Ok(unesc) = quick_xml::escape::unescape(s)
    {
        return std::borrow::Cow::Owned(unesc.into_owned().into_bytes());
    }
    std::borrow::Cow::Borrowed(ns)
}

fn is_wml_ns(ns: &[u8], root_w: &[u8]) -> bool {
    let unesc_ns = unescaped_ns(ns);
    let unesc_root = unescaped_ns(root_w);
    (!unesc_root.is_empty() && unesc_ns.as_ref() == unesc_root.as_ref()) || unesc_ns.as_ref() == WML || unesc_ns.as_ref() == WML_STRICT
}

const OWNED_COMPAT_SWITCHES: &[&str] = &["doNotExpandShiftReturn", "doNotUseHTMLParagraphAutoSpacing", "allowSpaceOfSameStyleInTable"];

fn check_compat_child<F>(root_tag: &str, compat_s: &str, k: &Child, check: F) -> bool
where
    F: FnOnce(&quick_xml::events::BytesStart<'_>, &quick_xml::NsReader<&[u8]>, &[u8]) -> bool,
{
    let Some(child_str) = compat_s.get(k.start..k.end) else { return false };
    let root =
        if root_tag.starts_with('<') { root_tag } else { r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"# };
    let Some(compat_open_end) = tag_end(compat_s).map(|e| e + 1) else { return false };
    let Some(compat_open) = compat_s.get(..compat_open_end) else { return false };
    let mut xml = String::with_capacity(root.len() + compat_open.len() + child_str.len() + 32);
    xml.push_str(root);
    xml.push_str(compat_open);
    xml.push_str(child_str);
    xml.push_str("</w:compat></w:settings>");
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    let mut root_w_ns = None;
    loop {
        match reader.read_resolved_event() {
            Ok((_, quick_xml::events::Event::Start(_))) => {
                depth += 1;
                if depth == 1 {
                    root_w_ns = match reader.resolver().resolve_element(quick_xml::name::QName(b"w:dummy")).0 {
                        quick_xml::name::ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                        _ => None,
                    };
                } else if depth == 2 {
                    break;
                }
            }
            Ok((_, quick_xml::events::Event::Eof)) | Err(_) => return false,
            _ => {}
        }
    }
    if let Ok((res, quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e))) = reader.read_resolved_event() {
        let is_wml_elem = match res {
            quick_xml::name::ResolveResult::Bound(ref ns) => is_wml_ns(ns.as_ref(), root_w_ns.as_deref().unwrap_or(b"")),
            _ => false,
        };
        if !is_wml_elem {
            return false;
        }
        check(e, &reader, root_w_ns.as_deref().unwrap_or(b""))
    } else {
        false
    }
}

fn is_compat_mode_child(root_tag: &str, compat_s: &str, k: &Child) -> bool {
    check_compat_child(root_tag, compat_s, k, |e, reader, root_w| {
        if e.local_name().as_ref() != b"compatSetting" {
            return false;
        }
        let mut w_name = None;
        let mut un_name = None;
        let mut w_uri = None;
        let mut un_uri = None;

        for attr in e.attributes().with_checks(false).flatten() {
            let (ares, local) = reader.resolver().resolve_attribute(attr.key);
            let val = attr.normalized_value(quick_xml::XmlVersion::Explicit1_0).ok();
            let is_wml = match ares {
                quick_xml::name::ResolveResult::Bound(ref ns) => is_wml_ns(ns.as_ref(), root_w),
                _ => false,
            };
            let is_unbound = matches!(ares, quick_xml::name::ResolveResult::Unbound);

            if local.as_ref() == b"name" {
                if is_wml {
                    w_name = val;
                } else if is_unbound {
                    un_name = val;
                }
            } else if local.as_ref() == b"uri" {
                if is_wml {
                    w_uri = val;
                } else if is_unbound {
                    un_uri = val;
                }
            }
        }

        let name = w_name.or(un_name);
        let uri = w_uri.or(un_uri);
        name.as_deref() == Some("compatibilityMode") && uri.as_deref().is_none_or(|u| u == "http://schemas.microsoft.com/office/word")
    })
}

fn is_wml_switch_child(root_tag: &str, compat_s: &str, k: &Child, sw_local: &str) -> bool {
    check_compat_child(root_tag, compat_s, k, |e, _reader, _root_w| e.local_name().as_ref() == sw_local.as_bytes())
}

const CT_COMPAT_ORDER: &[&str] = &[
    "useSingleBorderforContiguousCells",
    "wpJustification",
    "noTabHangInd",
    "noLeading",
    "spaceForUL",
    "noColumnBalance",
    "balanceSingleByteDoubleByteWidth",
    "noExtraLineSpacing",
    "doNotLeaveBackslashAlone",
    "ulTrailSpace",
    "doNotExpandShiftReturn",
    "spacingInWholePoints",
    "lineWrapLikeWord6",
    "printBodyTextBeforeHeader",
    "printColBlack",
    "wpSpaceWidth",
    "showBreaksInFrames",
    "subFontBySize",
    "suppressBottomSpacing",
    "suppressTopSpacing",
    "suppressSpacingAtTopOfPage",
    "suppressTopSpacingWP",
    "suppressSpBfAfterPgBrk",
    "swapBordersFacingPages",
    "convMailMergeEsc",
    "truncateFontHeightsLikeWP6",
    "mwSmallCaps",
    "usePrinterMetrics",
    "doNotSuppressParagraphBorders",
    "wrapTrailSpaces",
    "footnoteLayoutLikeWW8",
    "shapeLayoutLikeWW8",
    "alignTablesRowByRow",
    "forgetLastTabAlignment",
    "adjustLineHeightInTable",
    "autoSpaceLikeWord95",
    "noSpaceRaiseLower",
    "doNotUseHTMLParagraphAutoSpacing",
    "layoutRawTableWidth",
    "layoutTableRowsApart",
    "useWord97LineBreakRules",
    "doNotBreakWrappedTables",
    "doNotSnapToGridInCell",
    "selectFldWithFirstOrLastChar",
    "applyBreakingRules",
    "doNotWrapTextWithPunct",
    "doNotUseEastAsianBreakRules",
    "useWord2002TableStyleRules",
    "growAutofit",
    "useFELayout",
    "useNormalStyleForList",
    "doNotUseIndentAsNumberingTabStop",
    "useAltKinsokuLineBreakRules",
    "allowSpaceOfSameStyleInTable",
    "doNotSuppressIndentation",
    "doNotAutofitConstrainedTables",
    "autofitToFirstFixedWidthCell",
    "underlineTabInNumList",
    "displayHangulFixedWidth",
    "splitPgBreakAndParaMark",
    "doNotVertAlignCellWithSp",
    "doNotBreakConstrainedForcedTable",
    "doNotVertAlignInTxbx",
    "useAnsiKerningPairs",
    "cachedColBalance",
    "compatSetting",
];

fn compat_child_rank(name: &str) -> usize {
    let local = name.rsplit(':').next().unwrap_or(name);
    CT_COMPAT_ORDER.iter().position(|&n| n == local).unwrap_or(CT_COMPAT_ORDER.len())
}

fn recognized_compat_child_rank(root_tag: &str, compat_s: &str, k: &Child) -> Option<usize> {
    let local = k.name.rsplit(':').next().unwrap_or(&k.name);
    let rank = CT_COMPAT_ORDER.iter().position(|&n| n == local)?;
    if check_compat_child(root_tag, compat_s, k, |_, _, _| true) { Some(rank) } else { None }
}

fn is_wml_compat_switch(root_tag: &str, compat_s: &str, k: &Child) -> bool {
    OWNED_COMPAT_SWITCHES.iter().any(|sw| is_wml_switch_child(root_tag, compat_s, k, sw))
}

fn is_wml_compat_container(root_tag: &str, orig: &str, k: &Child) -> bool {
    let Some(child_str) = orig.get(k.start..k.end) else { return false };
    let root =
        if root_tag.starts_with('<') { root_tag } else { r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"# };
    let Some(open_end) = tag_end(child_str).map(|e| e + 1) else { return false };
    let Some(open_tag) = child_str.get(..open_end) else { return false };
    let mut xml = String::with_capacity(root.len() + open_tag.len() + 64);
    xml.push_str(root);
    if open_tag.ends_with("/>") {
        xml.push_str(open_tag);
    } else {
        let tag_name = &k.name;
        xml.push_str(open_tag);
        xml.push_str(&format!("</{tag_name}>"));
    }
    xml.push_str("</w:settings>");
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    let mut root_w_ns = None;
    loop {
        match reader.read_resolved_event() {
            Ok((_, quick_xml::events::Event::Start(_))) if depth == 0 => {
                depth = 1;
                root_w_ns = match reader.resolver().resolve_element(quick_xml::name::QName(b"w:dummy")).0 {
                    quick_xml::name::ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
            }
            Ok((res, quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e))) if depth == 1 => {
                let is_wml_elem = match res {
                    quick_xml::name::ResolveResult::Bound(ref ns) => is_wml_ns(ns.as_ref(), root_w_ns.as_deref().unwrap_or(b"")),
                    _ => false,
                };
                return is_wml_elem && e.local_name().as_ref() == b"compat";
            }
            Ok((_, quick_xml::events::Event::Eof)) | Err(_) => return false,
            _ => {}
        }
    }
}

fn compat_destination_and_w_ns(root_tag: &str, compat_s: &str) -> (Vec<u8>, Option<Vec<u8>>) {
    let root = if root_tag.trim_start().starts_with('<') {
        root_tag
    } else {
        r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#
    };
    let Some(open_end) = tag_end(compat_s).map(|e| e + 1) else {
        return (WML.to_vec(), Some(WML.to_vec()));
    };
    let Some(compat_open) = compat_s.get(..open_end) else {
        return (WML.to_vec(), Some(WML.to_vec()));
    };
    let mut xml = String::with_capacity(root.len() + compat_open.len() + 64);
    xml.push_str(root);
    xml.push_str(compat_open);
    if !compat_open.ends_with("/>") {
        xml.push_str("</dummy>");
    }
    xml.push_str("</w:settings>");
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    let mut compat_ns = WML.to_vec();
    let mut w_ns = None;
    while let Ok((res, ev)) = reader.read_resolved_event() {
        match ev {
            quick_xml::events::Event::Start(_) => depth += 1,
            quick_xml::events::Event::Empty(_) if depth == 1 => depth = 2,
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
        if depth == 2 {
            if let quick_xml::name::ResolveResult::Bound(ns) = res {
                compat_ns = unescaped_ns(ns.as_ref()).into_owned();
            }
            if let quick_xml::name::ResolveResult::Bound(ns) = reader.resolver().resolve_element(quick_xml::name::QName(b"w:dummy")).0 {
                w_ns = Some(unescaped_ns(ns.as_ref()).into_owned());
            }
            break;
        }
    }
    (compat_ns, w_ns)
}

fn qualify_wml_child(g_text: &str, root_tag: &str, compat_s: &str) -> String {
    let (compat_ns, w_ns) = compat_destination_and_w_ns(root_tag, compat_s);
    if w_ns.as_deref() == Some(&compat_ns) {
        g_text.to_string()
    } else {
        let uri_str = std::str::from_utf8(&compat_ns).unwrap_or("http://schemas.openxmlformats.org/wordprocessingml/2006/main");
        if let Some(space_pos) = g_text.find(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            let mut s = g_text.to_string();
            s.insert_str(space_pos, &format!(r#" xmlns:w="{uri_str}""#));
            s
        } else {
            g_text.to_string()
        }
    }
}

fn enable_switch_element(elem_str: &str, root_tag: &str, compat_s: &str) -> String {
    let root = if root_tag.trim_start().starts_with('<') {
        root_tag
    } else {
        r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#
    };
    let Some(open_end) = tag_end(compat_s).map(|e| e + 1) else { return elem_str.to_string() };
    let Some(compat_open) = compat_s.get(..open_end) else { return elem_str.to_string() };
    let elem_offset_in_xml = root.len() + compat_open.len();
    let mut xml = String::with_capacity(root.len() + compat_open.len() + elem_str.len() + 64);
    xml.push_str(root);
    xml.push_str(compat_open);
    xml.push_str(elem_str);
    if !elem_str.ends_with("/>") {
        xml.push_str("</dummy>");
    }
    if !compat_open.ends_with("/>") {
        xml.push_str("</dummy>");
    }
    xml.push_str("</w:settings>");
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    let mut root_w_ns = None;
    let mut disabled_val_range: Option<(usize, usize)> = None;

    while let Ok((_, ev)) = reader.read_resolved_event() {
        match ev {
            quick_xml::events::Event::Start(_) if depth == 0 => {
                depth = 1;
                root_w_ns = match reader.resolver().resolve_element(quick_xml::name::QName(b"w:dummy")).0 {
                    quick_xml::name::ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
            }
            quick_xml::events::Event::Start(_) if depth == 1 => {
                depth = 2;
            }
            quick_xml::events::Event::Empty(ref e) | quick_xml::events::Event::Start(ref e) if depth >= 2 => {
                let mut w_val = None;
                let mut un_val = None;
                let root_w = root_w_ns.as_deref().unwrap_or(b"");
                for attr in e.attributes().with_checks(false).flatten() {
                    let (ares, local) = reader.resolver().resolve_attribute(attr.key);
                    if local.as_ref() == b"val" {
                        let is_wml = match ares {
                            quick_xml::name::ResolveResult::Bound(ref ns) => is_wml_ns(ns.as_ref(), root_w),
                            _ => false,
                        };
                        let is_unbound = matches!(ares, quick_xml::name::ResolveResult::Unbound);
                        let val_str = attr.normalized_value(quick_xml::XmlVersion::Explicit1_0).ok();
                        let start_in_xml = attr.value.as_ptr() as usize - xml.as_ptr() as usize;
                        if start_in_xml >= elem_offset_in_xml {
                            let start = start_in_xml - elem_offset_in_xml;
                            let end = start + attr.value.len();
                            if is_wml {
                                w_val = Some((val_str, (start, end)));
                            } else if is_unbound {
                                un_val = Some((val_str, (start, end)));
                            }
                        }
                    }
                }
                let chosen = w_val.or(un_val);
                if let Some((Some(v), span)) = chosen
                    && crate::units::is_off_val(&v)
                {
                    disabled_val_range = Some(span);
                }
                break;
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }

    if let Some((start, end)) = disabled_val_range
        && start < end
        && end <= elem_str.len()
    {
        let mut s = elem_str.to_string();
        s.replace_range(start..end, "1");
        return s;
    }
    elem_str.to_string()
}

fn sync_compat_switch(root_tag: &str, orig: &mut String, orig_tag: &str, generated: &str, sw_local: &str) -> Option<()> {
    let gen_sw = children(generated, "w:compat").and_then(|(_, _, _, g_kids)| {
        g_kids.into_iter().find(|k| is_wml_switch_child(root_tag, generated, k, sw_local)).and_then(|k| generated.get(k.start..k.end))
    });

    let (_, _, o_close, o_kids) = children(orig, orig_tag)?;
    let orig_matches: Vec<(usize, usize)> =
        o_kids.iter().filter(|k| is_wml_switch_child(root_tag, orig, k, sw_local)).map(|k| (k.start, k.end)).collect();

    match (orig_matches.as_slice(), gen_sw) {
        ([], None) => {}
        (matches, None) => {
            for &(a, b) in matches.iter().rev() {
                orig.replace_range(a..b, "");
            }
        }
        ([], Some(g_text)) => {
            let my_rank = compat_child_rank(sw_local);
            let at = o_kids
                .iter()
                .find(|k| recognized_compat_child_rank(root_tag, orig, k).is_some_and(|r| r > my_rank))
                .map(|k| k.start)
                .unwrap_or(o_close);
            let child = qualify_wml_child(g_text, root_tag, orig);
            orig.insert_str(at, &child);
        }
        (matches, Some(_g_text)) => {
            for &(a, b) in matches[1..].iter().rev() {
                orig.replace_range(a..b, "");
            }
            let (first_a, first_b) = matches[0];
            let orig_elem = &orig[first_a..first_b];
            let updated = enable_switch_element(orig_elem, root_tag, orig);
            orig.replace_range(first_a..first_b, &updated);
        }
    }
    Some(())
}

/// The original `w:compat` with its `compatibilityMode` taken from the generated one,
/// or removed if the generated settings omit it.
fn merge_compat(root_tag: &str, orig: &str, orig_tag: &str, generated: &str) -> Option<String> {
    let mode_spans = |s: &str, tag: &str| -> Vec<(usize, usize)> {
        children(s, tag)
            .map(|(_, _, _, kids)| kids.into_iter().filter(|k| is_compat_mode_child(root_tag, s, k)).map(|k| (k.start, k.end)).collect())
            .unwrap_or_default()
    };

    let has_any_gen_switch =
        children(generated, "w:compat").is_some_and(|(_, _, _, g_kids)| g_kids.iter().any(|k| is_wml_compat_switch(root_tag, generated, k)));

    let open_end = tag_end(orig)? + 1;
    let is_self_closing = orig.get(open_end - 2..open_end) == Some("/>");
    let compat_open = orig.get(..open_end)?;

    if tag_has_rel_id(root_tag, compat_open) {
        return children(generated, "w:compat").is_some().then(|| generated.to_string()).or_else(|| Some(String::new()));
    }

    let gen_modes = mode_spans(generated, "w:compat");
    if is_self_closing && gen_modes.is_empty() && !has_any_gen_switch {
        return Some(orig.to_string());
    }

    let mut out = if is_self_closing {
        let open = orig.get(..open_end - 2)?.trim_end();
        format!("{open}></{orig_tag}>")
    } else {
        orig.to_string()
    };

    // Omit relationship-bearing children whose relationships cannot be remapped in settings.xml.rels
    if let Some((_, _, _, kids)) = children(&out, orig_tag) {
        for k in kids.into_iter().rev() {
            if let Some(child_str) = out.get(k.start..k.end)
                && child_has_rel_id(root_tag, compat_open, child_str)
            {
                out.replace_range(k.start..k.end, "");
            }
        }
    }

    for sw in OWNED_COMPAT_SWITCHES {
        sync_compat_switch(root_tag, &mut out, orig_tag, generated, sw)?;
    }

    let o_modes = mode_spans(&out, orig_tag);
    match (o_modes.as_slice(), gen_modes.first()) {
        ([], None) => {}
        (matches, None) => {
            for &(a, b) in matches.iter().rev() {
                out.replace_range(a..b, "");
            }
        }
        ([], Some(&(c, d))) => {
            let g_text = generated.get(c..d)?;
            let close = find_close_tag(&out, orig_tag)?;
            let child = qualify_wml_child(g_text, root_tag, &out);
            out.insert_str(close, &child);
        }
        (matches, Some(&(c, d))) => {
            let g_text = generated.get(c..d)?;
            for &(a, b) in matches[1..].iter().rev() {
                out.replace_range(a..b, "");
            }
            let (first_a, first_b) = matches[0];
            let child = qualify_wml_child(g_text, root_tag, &out);
            out.replace_range(first_a..first_b, &child);
        }
    }

    if let Some((c_start, c_open, _, remaining_kids)) = children(&out, orig_tag)
        && remaining_kids.is_empty()
        && !compat_has_unmodeled_attrs(root_tag, &out[c_start..c_open])
    {
        return Some(String::new());
    }

    Some(out)
}

fn tag_has_rel_id(root_tag: &str, tag_str: &str) -> bool {
    let root =
        if root_tag.starts_with('<') { root_tag } else { r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"# };
    let mut xml = String::with_capacity(root.len() + tag_str.len() + 32);
    xml.push_str(root);
    xml.push_str(tag_str);
    if !tag_str.ends_with("/>") {
        xml.push_str("</dummy></w:settings>");
    } else {
        xml.push_str("</w:settings>");
    }
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    loop {
        match reader.read_resolved_event() {
            Ok((_, quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e))) => {
                depth += 1;
                if depth == 2 {
                    for attr in e.attributes().with_checks(false).flatten() {
                        let (res, local) = reader.resolver().resolve_attribute(attr.key);
                        if local.as_ref() == b"id"
                            && let quick_xml::name::ResolveResult::Bound(ns) = res
                            && is_rel_ns(ns.as_ref())
                        {
                            return true;
                        }
                    }
                    break;
                }
            }
            Ok((_, quick_xml::events::Event::Eof)) | Err(_) => break,
            _ => {}
        }
    }
    false
}

fn child_has_rel_id(root_tag: &str, compat_open: &str, child_str: &str) -> bool {
    let root =
        if root_tag.starts_with('<') { root_tag } else { r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"# };
    let mut xml = String::with_capacity(root.len() + compat_open.len() + child_str.len() + 32);
    xml.push_str(root);
    xml.push_str(compat_open);
    xml.push_str(child_str);
    xml.push_str("</w:compat></w:settings>");
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    loop {
        match reader.read_resolved_event() {
            Ok((_, quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e))) => {
                depth += 1;
                if depth >= 3 {
                    for attr in e.attributes().with_checks(false).flatten() {
                        let (res, local) = reader.resolver().resolve_attribute(attr.key);
                        if local.as_ref() == b"id"
                            && let quick_xml::name::ResolveResult::Bound(ns) = res
                            && is_rel_ns(ns.as_ref())
                        {
                            return true;
                        }
                    }
                }
            }
            Ok((_, quick_xml::events::Event::End(_))) => {
                depth -= 1;
            }
            Ok((_, quick_xml::events::Event::Eof)) | Err(_) => break,
            _ => {}
        }
    }
    false
}

fn compat_has_unmodeled_attrs(root_tag: &str, compat_tag: &str) -> bool {
    let root =
        if root_tag.starts_with('<') { root_tag } else { r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"# };
    let mut xml = String::with_capacity(root.len() + compat_tag.len() + 32);
    xml.push_str(root);
    xml.push_str(compat_tag);
    if !compat_tag.ends_with("/>") {
        xml.push_str("</dummy></w:settings>");
    } else {
        xml.push_str("</w:settings>");
    }
    let mut reader = quick_xml::NsReader::from_str(&xml);
    let mut depth = 0;
    loop {
        match reader.read_resolved_event() {
            Ok((_, quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e))) => {
                depth += 1;
                if depth == 2 {
                    for attr in e.attributes().with_checks(false).flatten() {
                        let key = attr.key.as_ref();
                        if key.starts_with(b"xmlns:") || key == b"xmlns" {
                            continue;
                        }
                        let (res, local) = reader.resolver().resolve_attribute(attr.key);
                        if local.as_ref() == b"id"
                            && let quick_xml::name::ResolveResult::Bound(ns) = res
                            && is_rel_ns(ns.as_ref())
                        {
                            continue;
                        }
                        return true;
                    }
                    break;
                }
            }
            Ok((_, quick_xml::events::Event::Eof)) | Err(_) => break,
            _ => {}
        }
    }
    false
}

/// Does `original` (a settings part) hold anything the model doesn't write? Then the reader keeps
/// it for [`merge`]. False for parts WordCraft wrote itself, and for anything it can't scan.
pub fn has_unowned(original: &[u8]) -> bool {
    let Some(xml) = std::str::from_utf8(original).ok() else { return false };
    let Some((o_start, open_end, _, kids)) = children(xml, "w:settings") else { return false };
    let root_tag = xml.get(o_start..open_end).unwrap_or("");
    if kids.iter().any(|child| child.name == "w:mailMerge") {
        let Some(parsed) = crate::xml::parse(original).ok() else { return false };
        if kids.len() == parsed.els().count()
            && kids.iter().zip(parsed.els()).any(|(child, element)| child.name == "w:mailMerge" && element.name != "w:mailMerge")
        {
            return true;
        }
    }
    kids.iter().any(|k| {
        if is_wml_compat_container(root_tag, xml, k) {
            xml.get(k.start..k.end).is_some_and(|c| {
                let open_end = tag_end(c).map(|e| e + 1).unwrap_or(0);
                let compat_open = c.get(..open_end).unwrap_or("");
                if tag_has_rel_id(root_tag, compat_open) {
                    return false;
                }
                if compat_has_unmodeled_attrs(root_tag, compat_open) {
                    return true;
                }
                let Some((_, _, _, compat_kids)) = children(c, &k.name) else { return false };
                compat_kids.iter().any(|kid| {
                    !is_compat_mode_child(root_tag, c, kid)
                        && c.get(kid.start..kid.end).is_some_and(|child_str| !child_has_rel_id(root_tag, compat_open, child_str))
                })
            })
        } else {
            !OWNED.contains(&k.name.as_str()) && !xml.get(k.start..k.end).is_some_and(|s| s.contains("r:id="))
        }
    })
}

/// The merged settings part, or `None` to fall back to `generated`.
pub fn merge(original: &[u8], generated: &[u8]) -> Option<Vec<u8>> {
    let orig = std::str::from_utf8(original).ok()?;
    let gen_s = std::str::from_utf8(generated).ok()?;
    let (o_start, o_open, o_close, mut o_kids) = children(orig, "w:settings")?;
    let (_, _, _, g_kids) = children(gen_s, "w:settings")?;
    // Mail merge is regenerated from the namespace-aware tree. Recognise the same children
    // here, including local/default aliases, so a recovered save can remove the original too.
    // Keep every other child's existing byte-preserving merge policy.
    if o_kids.iter().any(|child| child.name.rsplit(':').next() == Some("mailMerge")) {
        let parsed = crate::xml::parse(original).ok()?;
        if parsed.els().count() != o_kids.len() {
            return None;
        }
        for (child, element) in o_kids.iter_mut().zip(parsed.els()) {
            if element.name == "w:mailMerge" || child.name == "w:mailMerge" {
                child.name.clone_from(&element.name);
            }
        }
    }
    // Prefixes the generated children use must be declared by the original root.
    let root_tag = orig.get(o_start..o_open)?;
    let preamble = orig.get(..o_start)?;
    for p in ["w", "m", "r"] {
        if g_kids.iter().any(|k| k.name.starts_with(&format!("{p}:"))) && !root_tag.contains(&format!("xmlns:{p}=")) {
            return None;
        }
    }
    let mut reader = quick_xml::NsReader::from_str(root_tag);
    let strict = loop {
        let (namespace, event) = reader.read_resolved_event().ok()?;
        match event {
            quick_xml::events::Event::Start(_) => {
                break matches!(namespace, quick_xml::name::ResolveResult::Bound(ns) if ns.as_ref() == b"http://purl.oclc.org/ooxml/wordprocessingml/main");
            }
            quick_xml::events::Event::Eof => return None,
            _ => {}
        }
    };
    let g_text = |k: &Child| {
        let text = gen_s.get(k.start..k.end)?;
        // Only this generated child declares its own canonical aliases. Match the retained
        // settings root; a fallback to the generated part keeps its Transitional declarations.
        if strict && k.name == "w:mailMerge" {
            Some(
                text.replace(
                    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#,
                    r#"xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main""#,
                )
                .replace(
                    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#,
                    r#"xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships""#,
                ),
            )
        } else {
            Some(text.to_string())
        }
    };
    let mut used = vec![false; g_kids.len()];
    let mark = |name: &str, used: &mut Vec<bool>| -> Option<usize> {
        let gi = g_kids.iter().enumerate().position(|(i, g)| g.name == name && !used.get(i).copied().unwrap_or(true))?;
        if let Some(u) = used.get_mut(gi) {
            *u = true;
        }
        Some(gi)
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for k in &o_kids {
        let text = orig.get(k.start..k.end)?;
        if KEEP_ORIGINAL.contains(&k.name.as_str()) {
            // The model reads these only partly (math settings) or writes them only in one case
            // (the background shape flag, with a page colour): the original wins.
            mark(&k.name, &mut used);
            out.push((k.name.clone(), text.to_string()));
        } else if OWNED.contains(&k.name.as_str()) {
            if let Some(gi) = mark(&k.name, &mut used) {
                out.push((k.name.clone(), g_text(g_kids.get(gi)?)?));
            }
        } else if is_wml_compat_container(root_tag, orig, k) {
            let g_compat = mark("w:compat", &mut used).and_then(|gi| g_text(g_kids.get(gi)?));
            let merged = merge_compat(root_tag, text, &k.name, g_compat.as_deref().unwrap_or(""))?;
            if !merged.is_empty() {
                out.push(("w:compat".to_string(), merged));
            }
        } else if !text.contains("r:id=") {
            out.push((k.name.clone(), text.to_string()));
        }
    }
    for (gi, g) in g_kids.iter().enumerate() {
        let owned = OWNED.contains(&g.name.as_str()) || g.name == "w:compat";
        if used.get(gi).copied().unwrap_or(true) || !owned {
            continue;
        }
        let r = rank(&g.name);
        let at = out.iter().position(|(n, _)| rank(n) > r).unwrap_or(out.len());
        out.insert(at, (g.name.clone(), g_text(g)?));
    }
    let mut s = String::with_capacity(orig.len() + 256);
    s.push_str(preamble);
    s.push_str(root_tag);
    for (_, t) in &out {
        s.push_str(t);
    }
    s.push_str(orig.get(o_close..)?);
    Some(s.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEN: &str = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#;

    #[test]
    fn odd_or_hostile_originals_fall_back_or_merge_without_panicking() {
        let odd = [
            "",
            "<w:settings",
            "<w:settings/>",
            "<x:other/>",
            r#"<w:settings xmlns:w="w"><w:a><w:b></w:a></w:settings>"#,
            r#"<w:settings xmlns:w="w"><![CDATA[x]]></w:settings>"#,
            r#"<w:settings xmlns:w="w"></w:x></w:settings>"#,
            "<w:settings xmlns:w=\"w\">\u{1F600}<w:rsids/></w:settings>",
            r#"<w:settings xmlns:w="w"><!-- c --><?pi x?><w:rsids/></w:settings>"#,
        ];
        for o in odd {
            let _ = merge(o.as_bytes(), GEN.as_bytes());
            let _ = has_unowned(o.as_bytes());
        }
        assert!(merge(&[0xff, 0xfe], GEN.as_bytes()).is_none());
        // Missing declarations for a prefix the generated part uses: fall back.
        assert!(merge(br#"<w:settings xmlns:x="x"><w:rsids/></w:settings>"#, GEN.as_bytes()).is_none());
    }

    #[test]
    fn a_gt_inside_an_attribute_value_does_not_hide_the_kept_settings() {
        for v in [r#"w:val="x>y""#, "w:val='x>y'"] {
            let orig = format!(
                r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:docVar w:name="a" {v}/><w:docVar w:name="b" {v}></w:docVar></w:settings>"#
            );
            assert!(has_unowned(orig.as_bytes()), "{v}");
            let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
            assert!(out.contains(r#"w:name="a""#) && out.contains(r#"w:name="b""#), "{v}");
        }
    }

    #[test]
    fn a_compat_block_without_a_mode_gets_the_generated_one() {
        let orig = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat><w:useFELayout/></w:compat></w:settings>"#;
        let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
        assert!(out.contains("<w:useFELayout/>") && out.contains("compatibilityMode"), "{out}");
        assert!(has_unowned(orig.as_bytes()));
        assert!(!has_unowned(GEN.as_bytes()));
    }

    #[test]
    fn a_self_closing_compat_block_gets_the_generated_mode() {
        let orig = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat/><w:rsids><w:rsidRoot w:val="00112233"/></w:rsids></w:settings>"#;
        let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
        assert!(out.contains("<w:compat><w:compatSetting") && out.contains("</w:compat>"), "{out}");
        assert!(out.contains("<w:rsidRoot w:val=\"00112233\"/>"), "{out}");
    }

    #[test]
    fn zoom_comes_from_the_generated_part_while_foreign_settings_stay() {
        let original_zoom = r#"<w:zoom w:percent="120"/>"#;
        let updated_zoom = r#"<w:zoom w:val="bestFit" w:percent="145"/>"#;
        let original = format!(
            r#"<w:settings xmlns:w="w" xmlns:mc="mc" mc:Ignorable="w15" xmlns:w15="w15">{original_zoom}<w:proofState w:spelling="clean"/><w:docVars><w:docVar w:name="ClauseSet" w:val="B"/></w:docVars></w:settings>"#
        );
        let generated = format!(r#"<w:settings xmlns:w="w">{updated_zoom}</w:settings>"#);
        for source in [original.clone(), original.replace(original_zoom, "")] {
            assert!(has_unowned(source.as_bytes()));
            let out = String::from_utf8(merge(source.as_bytes(), generated.as_bytes()).unwrap()).unwrap();
            assert_eq!(out, original.replace(original_zoom, updated_zoom));
            assert_eq!(out.matches("<w:zoom ").count(), 1);
        }
    }

    #[test]
    fn gutter_at_top_comes_from_the_generated_part() {
        let orig =
            r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:mirrorMargins/><w:gutterAtTop/><w:proofState w:spelling="clean"/></w:settings>"#;
        assert!(has_unowned(orig.as_bytes()));
        // Turned off in the model: the original element goes, the foreign one stays.
        let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
        assert!(!out.contains("gutterAtTop") && out.contains("<w:proofState"), "{out}");
        // On in the model: written once, in place.
        let gen_on = GEN.replace("<w:defaultTabStop", "<w:mirrorMargins/><w:gutterAtTop/><w:defaultTabStop");
        let out = String::from_utf8(merge(orig.as_bytes(), gen_on.as_bytes()).unwrap()).unwrap();
        assert_eq!(out.matches("<w:gutterAtTop/>").count(), 1, "{out}");
        let at = |tag: &str| out.find(tag).unwrap_or_else(|| panic!("{tag} missing: {out}"));
        assert!(at("<w:mirrorMargins/>") < at("<w:gutterAtTop/>") && at("<w:gutterAtTop/>") < at("<w:proofState"), "{out}");
        // A file whose only extra setting is this one has nothing foreign to keep.
        assert!(!has_unowned(br#"<w:settings xmlns:w="w"><w:zoom w:percent="100"/><w:gutterAtTop/></w:settings>"#));
    }

    #[test]
    fn compatibility_mode_removal_handles_paired_tags_quotes_and_whitespace() {
        let gen_no_mode = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:zoom w:percent="100"/></w:settings>"#;

        // Paired tag with closing </w:compatSetting>
        let orig_paired = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"></w:compatSetting><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/></w:compat></w:settings>"#;
        assert!(has_unowned(orig_paired.as_bytes()));
        let out_paired = String::from_utf8(merge(orig_paired.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out_paired.contains("compatibilityMode"), "{out_paired}");
        assert!(!out_paired.contains("</w:compatSetting>"), "{out_paired}");
        assert!(out_paired.contains("overrideTableStyleFontSizeAndJustification"), "{out_paired}");

        // Single quotes
        let orig_single_quotes = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat><w:compatSetting w:name='compatibilityMode' w:uri='http://schemas.microsoft.com/office/word' w:val='15'/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:val="1"/></w:compat></w:settings>"#;
        let out_sq = String::from_utf8(merge(orig_single_quotes.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out_sq.contains("compatibilityMode"), "{out_sq}");
        assert!(out_sq.contains("overrideTableStyleFontSizeAndJustification"), "{out_sq}");

        // Whitespace around equals
        let orig_spaces = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat><w:compatSetting w:name = "compatibilityMode" w:uri = "http://schemas.microsoft.com/office/word" w:val = "15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:val="1"/></w:compat></w:settings>"#;
        let out_sp = String::from_utf8(merge(orig_spaces.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out_sp.contains("compatibilityMode"), "{out_sp}");
        assert!(out_sp.contains("overrideTableStyleFontSizeAndJustification"), "{out_sp}");

        // Foreign vendor compatSetting element is preserved
        let orig_foreign = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="m" xmlns:r="r" xmlns:x="urn:vendor"><w:compat><x:compatSetting x:name="compatibilityMode" x:uri="urn:vendor" x:val="1"/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#;
        let gen_foreign =
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let out_foreign = String::from_utf8(merge(orig_foreign.as_bytes(), gen_foreign.as_bytes()).unwrap()).unwrap();
        assert!(out_foreign.contains("x:compatSetting"), "{out_foreign}");
        assert!(!out_foreign.contains(r#"w:name="compatibilityMode""#), "{out_foreign}");

        // Namespace declared directly on w:compat (not root) with mode replacement
        let orig_compat_ns = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="m" xmlns:r="r"><w:compat xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><a:compatSetting a:name="compatibilityMode" a:uri="http://schemas.microsoft.com/office/word" a:val="15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:val="1"/></w:compat></w:settings>"#;
        let gen_new_mode = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14"/></w:compat></w:settings>"#;
        let out_ns = String::from_utf8(merge(orig_compat_ns.as_bytes(), gen_new_mode.as_bytes()).unwrap()).unwrap();
        assert_eq!(out_ns.matches("compatibilityMode").count(), 1, "{out_ns}");
        assert!(out_ns.contains(r#"w:val="14""#), "{out_ns}");

        // Escaped XML entities in attribute values
        let orig_escaped = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="m" xmlns:r="r"><w:compat><w:compatSetting w:name="compatibility&#x4d;ode" w:uri="http:&#47;&#47;schemas.microsoft.com/office/word" w:val="15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:val="1"/></w:compat></w:settings>"#;
        let out_esc = String::from_utf8(merge(orig_escaped.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out_esc.contains("compatibility"), "{out_esc}");
        assert!(out_esc.contains("overrideTableStyleFontSizeAndJustification"), "{out_esc}");

        // Leading whitespace before w:settings and root-declared alias
        let orig_ws = r#"
   <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><a:compatSetting a:name="compatibilityMode" a:uri="http://schemas.microsoft.com/office/word" a:val="15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:val="1"/></w:compat></w:settings>"#;
        let out_ws = String::from_utf8(merge(orig_ws.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out_ws.contains("compatibilityMode"), "{out_ws}");
        assert!(out_ws.contains("overrideTableStyleFontSizeAndJustification"), "{out_ws}");
    }

    #[test]
    fn compat_block_closing_tag_with_whitespace_preserves_unowned_children() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:useFELayout/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat ></w:settings>"#;
        assert!(has_unowned(orig.as_bytes()), "expected has_unowned with </w:compat >");
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("<w:useFELayout/>"), "{merged}");
        assert!(!merged.contains("compatibilityMode"), "{merged}");
    }

    #[test]
    fn compat_mode_matching_ignores_unqualified_attributes() {
        // A compatSetting with unqualified name="compatibilityMode" (no namespace) and w:name="vendorOption"
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:compatSetting w:name="vendorOption" name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#;
        // Since it's not a true compatibilityMode, has_unowned treats it as unowned vendor setting
        assert!(has_unowned(orig.as_bytes()));
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        // The vendor setting is preserved on save
        assert!(merged.contains(r#"w:name="vendorOption""#), "{merged}");
    }

    #[test]
    fn repeated_compat_switches_are_all_removed_when_disabled_and_deduplicated_when_enabled() {
        let orig_two = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/><w:useFELayout/><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        // Disabled in generated: both must be removed
        let gen_disabled =
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let out_disabled = String::from_utf8(merge(orig_two.as_bytes(), gen_disabled.as_bytes()).unwrap()).unwrap();
        assert!(!out_disabled.contains("doNotExpandShiftReturn"), "{out_disabled}");
        assert!(out_disabled.contains("<w:useFELayout/>"), "{out_disabled}");

        // Enabled in generated: exactly one occurrence emitted
        let gen_enabled = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        let out_enabled = String::from_utf8(merge(orig_two.as_bytes(), gen_enabled.as_bytes()).unwrap()).unwrap();
        assert_eq!(out_enabled.matches("doNotExpandShiftReturn").count(), 1, "{out_enabled}");
        assert!(out_enabled.contains("<w:useFELayout/>"), "{out_enabled}");
    }

    #[test]
    fn new_compat_switches_are_inserted_in_schema_order() {
        // spacingInWholePoints is ranked after doNotExpandShiftReturn in CT_Compat
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:spacingInWholePoints/></w:compat></w:settings>"#;
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        let idx_shift = merged.find("<w:doNotExpandShiftReturn/>").unwrap();
        let idx_spacing = merged.find("<w:spacingInWholePoints/>").unwrap();
        assert!(idx_shift < idx_spacing, "doNotExpandShiftReturn must precede spacingInWholePoints: {merged}");
    }

    #[test]
    fn aliased_compat_container_and_switches_synchronize() {
        // Document uses aliased prefix a: for WordprocessingML
        let orig = r#"<w:settings xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><a:compat><a:doNotExpandShiftReturn/><a:useFELayout/></a:compat></w:settings>"#;
        // User disabled switch
        let gen_disabled =
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let out_disabled = String::from_utf8(merge(orig.as_bytes(), gen_disabled.as_bytes()).unwrap()).unwrap();
        assert!(!out_disabled.contains("doNotExpandShiftReturn"), "{out_disabled}");
        assert!(out_disabled.contains("<a:useFELayout/>"), "{out_disabled}");
    }

    #[test]
    fn clearing_mode_removes_every_matching_word_mode() {
        // Multiple Word mode entries (e.g. 15 and 14) and one vendor entry
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:vendor"><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14"/><w:compatSetting w:name="compatibilityMode" w:uri="urn:vendor" w:val="99"/></w:compat></w:settings>"#;
        let gen_no_mode =
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let out = String::from_utf8(merge(orig.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out.contains(r#"w:val="15""#), "{out}");
        assert!(!out.contains(r#"w:val="14""#), "{out}");
        assert!(out.contains(r#"w:uri="urn:vendor""#), "{out}");
    }

    #[test]
    fn mode_attribute_lookup_precedence_matches_reader() {
        // Fallback to unqualified uri="urn:vendor" excludes vendor entry
        let orig_vendor_uri = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:compatSetting w:name="compatibilityMode" uri="urn:vendor" w:val="15"/></w:compat></w:settings>"#;
        assert!(has_unowned(orig_vendor_uri.as_bytes()));

        // Fallback to unqualified name="compatibilityMode" when w:name is absent recognizes mode
        let orig_unqual_name = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:useFELayout/><w:compatSetting name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#;
        let gen_no_mode =
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let out = String::from_utf8(merge(orig_unqual_name.as_bytes(), gen_no_mode.as_bytes()).unwrap()).unwrap();
        assert!(!out.contains("compatibilityMode"), "{out}");
        assert!(out.contains("<w:useFELayout/>"), "{out}");
    }

    #[test]
    fn aliased_compat_container_orders_before_math_pr() {
        let orig = r#"<w:settings xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><a:compat><a:useFELayout/></a:compat></w:settings>"#;
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:mathPr><m:mathFont m:val="Cambria Math"/></m:mathPr></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        let idx_compat = merged.find("<a:compat>").unwrap();
        let idx_math = merged.find("<m:mathPr>").unwrap();
        assert!(idx_compat < idx_math, "a:compat must precede m:mathPr: {merged}");
    }

    #[test]
    fn comment_with_closing_tag_syntax_does_not_interfere() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/><w:docVars><w:docVar w:name="V" w:val="1"/></w:docVars></w:settings><!-- </w:settings > -->"#;
        assert!(has_unowned(orig.as_bytes()));
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="120"/></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("<w:docVars>"), "{merged}");
    }

    #[test]
    fn foreign_child_is_ignored_when_finding_switch_insertion_point() {
        // x:spacingInWholePoints precedes w:ulTrailSpace
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:vendor"><w:compat><x:spacingInWholePoints/><w:ulTrailSpace/><w:spacingInWholePoints/></w:compat></w:settings>"#;
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        let idx_custom = merged.find("<x:spacingInWholePoints/>").unwrap();
        let idx_trail = merged.find("<w:ulTrailSpace/>").unwrap();
        let idx_shift = merged.find("<w:doNotExpandShiftReturn/>").unwrap();
        let idx_spacing = merged.find("<w:spacingInWholePoints/>").unwrap();
        assert!(idx_custom < idx_trail, "custom before trail: {merged}");
        assert!(idx_trail < idx_shift, "trail before shift: {merged}");
        assert!(idx_shift < idx_spacing, "shift before spacing: {merged}");
    }

    #[test]
    fn self_closing_compat_container_with_unmodeled_attr_is_retained() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:vendor"><w:compat x:layout="legacy"/></w:settings>"#;
        assert!(has_unowned(orig.as_bytes()), "expected self-closing compat with unmodeled attr to be retained");
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains(r#"x:layout="legacy""#), "{merged}");
    }

    #[test]
    fn unmodeled_attributes_on_enabled_switch_are_preserved() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:vendor"><w:compat><w:doNotExpandShiftReturn x:vendor="custom"/></w:compat></w:settings>"#;
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains(r#"x:vendor="custom""#), "{merged}");
    }

    #[test]
    fn generated_compat_child_in_vendor_rebound_w_prefix_declares_wml_namespace() {
        // Document binds prefix 'w' locally to urn:vendor on a:compat
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><a:compat xmlns:w="urn:vendor"><a:useFELayout/></a:compat></w:settings>"#;
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains(r#"<w:doNotExpandShiftReturn xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#), "{merged}");
        assert!(merged.contains("<a:useFELayout/>"), "{merged}");
    }

    #[test]
    fn aliased_compat_drops_unremapped_relationship_markup_while_preserving_relationship_free_markup() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:rel="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x="urn:vendor"><a:compat><a:useFELayout/><x:noteElem x:note="example r:id=text"/><x:markup r:id="rId5"/><x:aliasedMarkup rel:id="rId6"/><x:spaceMarkup r:id = "rId7"/></a:compat></w:settings>"#;
        assert!(has_unowned(orig.as_bytes()));
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("<a:useFELayout/>"), "{merged}");
        assert!(merged.contains(r#"x:note="example r:id=text""#), "text mentioning r:id= must be preserved: {merged}");
        assert!(!merged.contains("rId5"), "expected unremapped r:id to be omitted: {merged}");
        assert!(!merged.contains("rId6"), "expected unremapped rel:id to be omitted: {merged}");
        assert!(!merged.contains("rId7"), "expected unremapped spaced r:id to be omitted: {merged}");
    }

    #[test]
    fn switch_with_padded_zero_value_is_enabled_when_model_enables_it() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:vendor"><w:compat><w:doNotExpandShiftReturn w:val=" 0 " x:vendor="custom"/></w:compat></w:settings>"#;
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat><w:doNotExpandShiftReturn/></w:compat></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains(r#"w:val="1""#), "{merged}");
        assert!(merged.contains(r#"x:vendor="custom""#), "{merged}");
    }

    #[test]
    fn aliased_compat_container_with_bom_and_cleared_mode_removes_old_mode() {
        let orig = "\u{FEFF}<w:settings xmlns:a=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><a:compat><a:compatSetting a:name=\"compatibilityMode\" a:uri=\"http://schemas.microsoft.com/office/word\" a:val=\"15\"/><a:useFELayout/></a:compat></w:settings>";
        assert!(has_unowned(orig.as_bytes()));
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged_bytes = merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap();
        let merged = String::from_utf8(merged_bytes).unwrap();
        assert!(merged.starts_with('\u{FEFF}'), "expected BOM to be preserved in output: {merged}");
        assert!(merged.contains("<a:useFELayout/>"), "{merged}");
        assert!(!merged.contains("compatibilityMode"), "expected compatibilityMode to be removed: {merged}");
    }

    #[test]
    fn aliased_compat_drops_strict_relationship_markup() {
        let orig = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships"><a:compat><a:useFELayout/><x:markup r:id="rId8"/></a:compat></w:settings>"#;
        assert!(has_unowned(orig.as_bytes()));
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged = String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("<a:useFELayout/>"), "{merged}");
        assert!(!merged.contains("rId8"), "expected Strict r:id to be omitted: {merged}");
    }
}
