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
//! `w:compat` only the switches the model writes and the `compatibilityMode` setting come from the
//! generated part, and a block left empty goes. Kept children that refer to a
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

/// The root start tag's span end and the root's direct children, or `None` for anything the
/// scanner doesn't expect (CDATA, DOCTYPE, a different root).
fn children(xml: &str, root: &str) -> Option<(usize, usize, Vec<Child>)> {
    let open = xml.find(&format!("<{root}"))?;
    let open_end = open + tag_end(xml.get(open..)?)? + 1;
    if xml.get(open_end - 2..open_end)? == "/>" {
        return None;
    }
    let close = xml.rfind(&format!("</{root}>"))?;
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = open_end;
    let mut cur: Option<(String, usize)> = None;
    while i < close {
        let rest = xml.get(i..close)?;
        let Some(lt) = rest.find('<') else { break };
        let at = i + lt;
        let tail = xml.get(at..close)?;
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
    }
    if depth != 0 {
        return None;
    }
    Some((open_end, close, out))
}

/// The `w:compat` switches the model writes (`super::settings`).
const OWNED_COMPAT_SWITCHES: &[&str] = &["w:doNotExpandShiftReturn", "w:doNotUseHTMLParagraphAutoSpacing", "w:allowSpaceOfSameStyleInTable"];

/// CT_Compat's sequence (ECMA-376 §17.15.3.9), for placing inserted switches.
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

/// Position of a `w:`-prefixed `w:compat` child in [`CT_COMPAT_ORDER`]; `None` for anything else.
fn compat_rank(name: &str) -> Option<usize> {
    let local = name.strip_prefix("w:")?;
    CT_COMPAT_ORDER.iter().position(|n| *n == local)
}

/// Is this `w:compat` child Word's `compatibilityMode` setting?
fn is_compat_mode(child: &str) -> bool {
    let mut reader = quick_xml::Reader::from_str(child);
    let Ok(quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e)) = reader.read_event() else { return false };
    e.name().as_ref() == b"w:compatSetting"
        && e.attributes().with_checks(false).flatten().any(|a| a.key.as_ref() == b"w:name" && a.value.as_ref() == b"compatibilityMode")
}

/// The original `w:compat` with what the model owns in it taken from the generated one (`""`
/// when the model writes none): its switches, and the `compatibilityMode` setting. A block left
/// without children goes (`""`).
fn merge_compat(orig: &str, generated: &str) -> Option<String> {
    let g_kids = children(generated, "w:compat").map(|(_, _, kids)| kids).unwrap_or_default();
    let g_child = |pred: &dyn Fn(&Child, &str) -> bool| g_kids.iter().find_map(|k| generated.get(k.start..k.end).filter(|text| pred(k, text)));
    let mut out = match orig.strip_suffix("/>") {
        Some(open) => format!("{}></w:compat>", open.trim_end()),
        None => orig.to_string(),
    };
    for sw in OWNED_COMPAT_SWITCHES {
        let (_, _, kids) = children(&out, "w:compat")?;
        for k in kids.iter().rev().filter(|k| k.name == *sw) {
            out.replace_range(k.start..k.end, "");
        }
        if let Some(g) = g_child(&|k, _| k.name == *sw) {
            let (_, close, kids) = children(&out, "w:compat")?;
            let rank = compat_rank(sw);
            let at = kids.iter().find(|k| compat_rank(&k.name) > rank).map_or(close, |k| k.start);
            out.insert_str(at, g);
        }
    }
    let (_, close, kids) = children(&out, "w:compat")?;
    let o_mode = kids.iter().find(|k| out.get(k.start..k.end).is_some_and(is_compat_mode)).map(|k| k.start..k.end);
    match (o_mode, g_child(&|_, text| is_compat_mode(text))) {
        (Some(span), Some(g)) => out.replace_range(span, g),
        (Some(span), None) => out.replace_range(span, ""),
        (None, Some(g)) => out.insert_str(close, g),
        (None, None) => {}
    }
    let (_, _, kids) = children(&out, "w:compat")?;
    Some(if kids.is_empty() { String::new() } else { out })
}

/// Does `original` (a settings part) hold anything the model doesn't write? Then the reader keeps
/// it for [`merge`]. False for parts WordCraft wrote itself, and for anything it can't scan.
pub fn has_unowned(original: &[u8]) -> bool {
    let Some(xml) = std::str::from_utf8(original).ok() else { return false };
    let Some((_, _, kids)) = children(xml, "w:settings") else { return false };
    if kids.iter().any(|child| child.name == "w:mailMerge") {
        let Some(parsed) = crate::xml::parse(original).ok() else { return false };
        if kids.len() == parsed.els().count()
            && kids.iter().zip(parsed.els()).any(|(child, element)| child.name == "w:mailMerge" && element.name != "w:mailMerge")
        {
            return true;
        }
    }
    kids.iter().any(|k| match k.name.as_str() {
        // Anything in `w:compat` beyond its own tag and the compatibility mode.
        "w:compat" => xml.get(k.start..k.end).is_some_and(|c| {
            let mode = usize::from(c.contains("w:name=\"compatibilityMode\""));
            c.matches("<w:").count().saturating_sub(1 + mode) > 0
        }),
        n => !OWNED.contains(&n),
    })
}

/// The merged settings part, or `None` to fall back to `generated`.
pub fn merge(original: &[u8], generated: &[u8]) -> Option<Vec<u8>> {
    let orig = std::str::from_utf8(original).ok()?;
    let gen_s = std::str::from_utf8(generated).ok()?;
    let (o_open, o_close, mut o_kids) = children(orig, "w:settings")?;
    let (_, _, g_kids) = children(gen_s, "w:settings")?;
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
    let root_tag = orig.get(..o_open)?;
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
        } else if k.name == "w:compat" {
            let g_compat = mark("w:compat", &mut used).and_then(|gi| g_text(g_kids.get(gi)?));
            let merged = merge_compat(text, g_compat.as_deref().unwrap_or(""))?;
            if !merged.is_empty() {
                out.push((k.name.clone(), merged));
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

    const GEN: &str = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="u" w:val="15"/></w:compat></w:settings>"#;

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
    fn a_bare_self_closing_compat_block_goes_like_an_empty_paired_one() {
        let gen_xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#;
        let merged_with = |compat: &str| {
            let orig = format!(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{compat}<w:rsids><w:rsidRoot w:val="00112233"/></w:rsids></w:settings>"#
            );
            String::from_utf8(merge(orig.as_bytes(), gen_xml.as_bytes()).unwrap()).unwrap()
        };
        let self_closing = merged_with("<w:compat/>");
        assert!(!self_closing.contains("compat"), "{self_closing}");
        assert!(self_closing.contains(r#"<w:rsidRoot w:val="00112233"/>"#), "{self_closing}");
        assert_eq!(self_closing, merged_with("<w:compat></w:compat>"));
    }
}
