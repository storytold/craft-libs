use super::{
    Arc, Blocks, Comment, DocxError, El, HashMap, MAX_PARTS, MAX_SOURCES, MAX_TEMPLATE_PATH, NoteKind, NumFormat, Paragraph, PartKind, Reader, Rels,
    Seen, StoryCtx, Zoom, ZoomFit, int, math, on_off, para_block, props, tw, u32_of,
};

impl Reader<'_> {
    pub(super) fn read_settings(&mut self, path: &str) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let mail_merge = root.child("w:mailMerge").is_some();
        if let Some(b) = self.pkg.get(path).filter(|b| mail_merge || crate::write::settings_has_unowned(b)) {
            self.doc.passthrough.insert(crate::package::SETTINGS_ORIG.into(), Arc::new(b.to_vec()));
        }
        let rels = self.pkg.rels(path);
        if mail_merge {
            // Relationship limits must not stop import of the owned settings below.
            match crate::write::mail_merge_rels(&rels) {
                Ok(kept) => {
                    self.doc.passthrough.insert(crate::package::MAIL_MERGE_RELS.into(), Arc::new(kept));
                }
                Err(e) => log::warn!("docx: not keeping mail merge relationships: {e}"),
            }
        }
        let s = &mut self.doc.settings;
        for k in root.els() {
            match k.name.as_str() {
                // §17.15.1.6: the template, by a relationship (usually external) of the settings part.
                "w:attachedTemplate" => {
                    s.attached_template = k
                        .attr("r:id")
                        .and_then(|id| rels.by_id(id))
                        .map(|r| r.target.trim().to_string())
                        .filter(|t| !t.is_empty() && t.len() <= MAX_TEMPLATE_PATH);
                }
                "w:linkStyles" => s.link_styles = on_off(k),
                "w:removePersonalInformation" => s.remove_personal_info = on_off(k),
                "w:removeDateAndTime" => s.remove_date_time = on_off(k),
                "w:hideSpellingErrors" => s.hide_spelling_errors = on_off(k),
                "w:hideGrammaticalErrors" => s.hide_grammar_errors = on_off(k),
                "w:compat" => {
                    s.allow_space_of_same_style_in_table = k.child("w:allowSpaceOfSameStyleInTable").is_some_and(on_off);
                    // Only Word's own setting: another `w:uri` names a different application's.
                    let mode = k
                        .children("w:compatSetting")
                        .find(|c| {
                            c.attr("w:name") == Some("compatibilityMode")
                                && c.attr("w:uri").is_none_or(|u| u == "http://schemas.microsoft.com/office/word")
                        })
                        .and_then(|c| c.attr("w:val"));
                    if let Some(m) = mode.and_then(u32_of) {
                        s.compat_mode = Some(m.clamp(11, 99));
                    }
                    s.sum_paragraph_spacing = k.children("w:doNotUseHTMLParagraphAutoSpacing").any(on_off);
                    s.do_not_expand_shift_return = k.children("w:doNotExpandShiftReturn").any(on_off);
                }
                "w:trackRevisions" => s.track_changes = on_off(k),
                "w:doNotTrackMoves" => s.track_moves = !on_off(k),
                "w:defaultTabStop" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| *v > 0.0) {
                        s.default_tab = v.clamp(1.0, 1584.0);
                    }
                }
                "w:evenAndOddHeaders" => s.even_odd_headers = on_off(k),
                "w:drawingGridHorizontalSpacing" | "w:drawingGridVerticalSpacing" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| *v > 0.0) {
                        let v = v.clamp(0.5, 1584.0);
                        if k.name == "w:drawingGridHorizontalSpacing" { s.grid_h = v } else { s.grid_v = v }
                    }
                }
                "m:mathPr" => s.math = Some(math::read_math_pr(k)),
                "w:mirrorMargins" => s.mirror_margins = on_off(k),
                "w:gutterAtTop" => s.gutter_at_top = on_off(k),
                "w:autoHyphenation" => s.auto_hyphenation = on_off(k),
                "w:doNotHyphenateCaps" => s.hyphenate_caps = !on_off(k),
                "w:hyphenationZone" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| v.is_finite() && *v >= 0.0) {
                        s.hyphenation_zone = v.min(1584.0);
                    }
                }
                "w:consecutiveHyphenLimit" => {
                    if let Some(v) = k.attr("w:val").and_then(u32_of) {
                        s.consecutive_hyphen_limit = v.min(32_767);
                    }
                }
                "w:footnotePr" => {
                    if let Some(f) = k.child_val("w:numFmt") {
                        s.footnote_format = NumFormat::from_ooxml(f);
                    }
                    s.footnote_pr = props::note_pr(k, false);
                }
                "w:endnotePr" => {
                    if let Some(f) = k.child_val("w:numFmt") {
                        s.endnote_format = NumFormat::from_ooxml(f);
                    }
                    s.endnote_pr = props::note_pr(k, false);
                }
                "w:documentProtection" => read_protection(k, s),
                "w:zoom" => {
                    let percent = k.attr("w:percent").and_then(|v| u32_of(v.trim_end_matches('%'))).unwrap_or(100).clamp(10, 500);
                    let fit = match k.attr("w:val") {
                        Some("fullPage") => Some(ZoomFit::OnePage),
                        Some("bestFit") => Some(ZoomFit::PageWidth),
                        Some("textFit") => Some(ZoomFit::TextWidth),
                        _ => None,
                    };
                    s.zoom = Some(Zoom { percent, fit }).filter(|z| z.percent != 100 || z.fit.is_some());
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn read_notes(&mut self, path: &str, foot: bool) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let rels = self.pkg.rels(path);
        let tag = if foot { "w:footnote" } else { "w:endnote" };
        for n in root.children(tag) {
            match n.attr("w:type") {
                None | Some("normal") => {}
                Some(t @ ("separator" | "continuationSeparator" | "continuationNotice")) => {
                    self.read_note_sep(n, &rels, foot, t);
                    continue;
                }
                Some(_) => continue,
            }
            let Some(fid) = n.attr("w:id").and_then(int) else { continue };
            if self.doc.parts.len() >= MAX_PARTS {
                break;
            }
            // Reserve the id first: the note's own reference mark points at it.
            let (pk, kind) = if foot { (PartKind::Footnote, NoteKind::Footnote) } else { (PartKind::Endnote, NoteKind::Endnote) };
            let id = self.doc.add_part(pk, Blocks::new());
            self.doc.missing_pictures.part_names.insert(id, self.story.clone());
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.current_note = Some((kind, id));
            self.read_blocks(&mut sc, n, &rels, &mut blocks, self.chunks.depth);
            self.flush_pending(&mut sc, &mut blocks);
            self.current_note = None;
            if let Some(p) = self.doc.parts.get_mut(&id)
                && !blocks.is_empty()
            {
                p.blocks = blocks;
            }
            if foot {
                self.footnotes.insert(fid, id);
            } else {
                self.endnotes.insert(fid, id);
            }
        }
        Ok(())
    }

    /// A document's own separator, continuation separator or continuation notice (`kind`, the
    /// note's `w:type`) for its footnotes or endnotes. The first of each kind counts.
    fn read_note_sep(&mut self, n: &El, rels: &Rels, foot: bool, kind: &str) {
        let seps = if foot { &self.doc.footnote_separators } else { &self.doc.endnote_separators };
        let taken = match kind {
            "separator" => seps.separator.is_some(),
            "continuationSeparator" => seps.continuation_separator.is_some(),
            _ => seps.continuation_notice.is_some(),
        };
        if taken {
            return;
        }
        let mut sc = StoryCtx::default();
        let mut blocks = Blocks::new();
        self.in_note_sep = true;
        self.read_blocks(&mut sc, n, rels, &mut blocks, self.chunks.depth);
        self.flush_pending(&mut sc, &mut blocks);
        self.in_note_sep = false;
        if blocks.is_empty() {
            blocks.push(para_block(Paragraph::new()));
        }
        let seps = if foot { &mut self.doc.footnote_separators } else { &mut self.doc.endnote_separators };
        let slot = match kind {
            "separator" => &mut seps.separator,
            "continuationSeparator" => &mut seps.continuation_separator,
            _ => &mut seps.continuation_notice,
        };
        *slot = Some(blocks);
    }

    /// The extended properties a save carries over ([`crate::package::APP_KEPT_PROPS`]).
    pub(super) fn read_app(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        let mut kept = String::new();
        for e in root.els() {
            let name = e.local();
            let value = e.text();
            if crate::package::APP_KEPT_PROPS.contains(&name) && value.len() <= 2048 && !value.contains(['\t', '\n']) {
                kept.push_str(&format!("{name}\t{value}\n"));
            }
        }
        if !kept.is_empty() {
            self.doc.passthrough.insert(crate::package::APP_KEPT.into(), Arc::new(kept.into_bytes()));
        }
    }

    pub(super) fn read_comments(&mut self, path: &str, ext: Option<&str>, ids: Option<&str>, cex: Option<&str>) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let rels = self.pkg.rels(path);
        let mut by_para: HashMap<String, u32> = HashMap::new();
        let mut next_id = 0u32;
        // Straight into the document, beside the comments chunks in notes brought in.
        for c in root.children("w:comment") {
            if self.doc.parts.len() >= MAX_PARTS {
                break;
            }
            let file_id = c.attr("w:id").unwrap_or("").to_string();
            if self.comment_map.contains_key(&file_id) {
                continue;
            }
            let id = match file_id.parse::<u32>() {
                Ok(v) if !self.doc.comments.contains_key(&v) => v,
                _ => {
                    // Every id handed out so far is a key of `doc.comments` (inserted below).
                    while self.doc.comments.contains_key(&next_id) {
                        next_id += 1;
                    }
                    next_id
                }
            };
            // Taken before its text is read: a chunk there brings comments of its own.
            self.doc.comments.insert(id, Comment::default());
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.read_blocks(&mut sc, c, &rels, &mut blocks, self.chunks.depth);
            self.flush_pending(&mut sc, &mut blocks);
            let part = self.doc.add_part(PartKind::Comment, blocks);
            self.doc.missing_pictures.part_names.insert(part, self.story.clone());
            if let Some(pid) = c.children("w:p").last().and_then(|p| p.attr("w14:paraId")) {
                by_para.insert(pid.to_string(), id);
            }
            self.doc.comments.insert(
                id,
                Comment {
                    author: c.attr("w:author").unwrap_or("").to_string(),
                    initials: c.attr("w:initials").unwrap_or("").to_string(),
                    date: c.attr("w:date").unwrap_or("").to_string(),
                    parent: None,
                    resolved: false,
                    part,
                    ..Default::default()
                },
            );
            self.comment_map.insert(file_id, id);
        }
        if let Some(ext) = ext
            && let Ok(Some(x)) = self.xml(ext)
        {
            for e in x.els().filter(|e| e.local() == "commentEx") {
                let Some(cid) = e.attr("w15:paraId").and_then(|p| by_para.get(p)).copied() else { continue };
                let parent = e.attr("w15:paraIdParent").and_then(|p| by_para.get(p)).copied().filter(|p| *p != cid);
                let done = e.attr("w15:done").is_some_and(|v| v == "1" || v == "true");
                if let Some(c) = self.doc.comments.get_mut(&cid) {
                    c.parent = parent;
                    c.resolved = done;
                }
            }
        }
        // Durable ids (by paragraph id) and UTC dates (by durable id), which the writer
        // carries over (`write_comment_ids`). Their namespaces (w16cid, w16cex) have no canonical
        // prefix here, so attributes are found by local name.
        let local = |e: &crate::xml::El, name: &str| -> Option<String> {
            e.attrs.iter().find(|(k, _)| k.rsplit(':').next() == Some(name)).map(|(_, v)| v.clone())
        };
        let mut by_durable: HashMap<String, u32> = HashMap::new();
        if let Some(ids) = ids
            && let Ok(Some(x)) = self.xml(ids)
        {
            for e in x.els().filter(|e| e.local() == "commentId") {
                let Some(cid) = local(e, "paraId").and_then(|p| by_para.get(&p).copied()) else { continue };
                let Some(durable) = local(e, "durableId").filter(|d| crate::write::durable_id_ok(d)) else { continue };
                if let Some(c) = self.doc.comments.get_mut(&cid) {
                    c.durable_id = Some(durable.to_ascii_uppercase());
                    by_durable.insert(durable.to_ascii_uppercase(), cid);
                }
            }
        }
        // Read with the names as written: each comment's extension list is kept whole, in
        // whatever namespaces it uses.
        if let Some(cex) = cex
            && let Ok(Some(x)) = self.pkg.xml_as_written(cex)
        {
            let mut kept = 0usize;
            for e in x.elements().filter(|e| e.local() == "commentExtensible") {
                let Some(cid) = e.attr_local("durableId").and_then(|d| by_durable.get(&d.to_ascii_uppercase()).copied()) else { continue };
                let Some(c) = self.doc.comments.get_mut(&cid) else { continue };
                let date = e.attr_local("dateUtc").filter(|d| d.len() <= 40 && d.chars().all(|c| c.is_ascii_alphanumeric() || ":-.+".contains(c)));
                if let Some(date) = date {
                    c.date_utc = Some(date.to_string());
                }
                if let Some(ext) = e.elements().find(|k| k.local() == "extLst")
                    && let Some(markup) = standalone(ext, &[&x, e])
                {
                    kept += markup.len();
                    if kept <= MAX_COMMENT_EXTENSIONS {
                        c.extensions = Some(markup);
                    }
                }
            }
        }
        Ok(())
    }

    /// Sources from a `b:Sources` custom XML part (other custom XML is left alone); at most
    /// [`MAX_SOURCES`] in all, and the document's citation style from `StyleName`.
    pub(super) fn read_bibliography(&mut self, path: &str, seen: &mut Seen) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        if root.local() != "Sources" {
            return Ok(());
        }
        let room = MAX_SOURCES.saturating_sub(self.doc.sources.len());
        let bib = crate::bibliography::parse(&root, seen, room);
        if self.doc.settings.bib_style.is_none() {
            self.doc.settings.bib_style = bib.style;
        }
        self.doc.sources.extend(bib.sources);
        Ok(())
    }

    pub(super) fn read_core(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        let c = &mut self.doc.core;
        for k in root.els() {
            let t = k.text().trim().to_string();
            match k.name.as_str() {
                "dc:title" => c.title = t,
                "dc:subject" => c.subject = t,
                "dc:creator" => c.creator = t,
                "cp:keywords" => c.keywords = t,
                "dc:description" => c.description = t,
                "cp:lastModifiedBy" => c.last_modified_by = t,
                "cp:revision" => c.revision = u32_of(&t).unwrap_or(0),
                "dcterms:created" => c.created = t,
                "dcterms:modified" => c.modified = t,
                "cp:category" => c.category = t,
                _ => {}
            }
        }
    }
}

/// The password hash attributes of `w:documentProtection` that are kept (ECMA-376 §17.15.1.29:
/// the current `algorithmName`/`hashValue`/`saltValue`/`spinCount`, and the older transitional
/// `crypt*`/`hash`/`salt` set).
pub(crate) const PROTECTION_HASH_ATTRS: [&str; 16] = [
    "w:algorithmName",
    "w:hashValue",
    "w:saltValue",
    "w:spinCount",
    "w:cryptProviderType",
    "w:cryptAlgorithmClass",
    "w:cryptAlgorithmType",
    "w:cryptAlgorithmSid",
    "w:cryptSpinCount",
    "w:cryptProvider",
    "w:algIdExt",
    "w:algIdExtSource",
    "w:cryptProviderTypeExt",
    "w:cryptProviderTypeExtSource",
    "w:hash",
    "w:salt",
];

/// Restrict Editing (`w:documentProtection`, §17.15.1.29): the editing restriction, whether
/// formatting is limited to unlocked styles, and the password hash. Only enforced protection is
/// kept; a hostile attribute value can't be longer than a hash needs.
fn read_protection(k: &El, s: &mut craft_wordmodel::Settings) {
    let on = |v: &str| !matches!(v, "0" | "false" | "off");
    // Word enforces protection when `w:enforcement` is missing ([MS-OI29500] §2.1.402 (c)), the
    // opposite of the standard's literal reading, so only an explicit off value unlocks the
    // document; a file whose generator omits the attribute keeps its restriction and its hash.
    if k.attr("w:enforcement").is_some_and(|v| !on(v)) {
        return;
    }
    let edit = k.attr("w:edit").filter(|e| matches!(*e, "readOnly" | "comments" | "trackedChanges" | "forms"));
    let formatting = k.attr("w:formatting").is_some_and(on);
    if edit.is_none() && !formatting {
        return;
    }
    s.protection = Some(edit.unwrap_or("none").to_string());
    s.protect_formatting = formatting;
    s.protect_hash = PROTECTION_HASH_ATTRS
        .iter()
        .filter_map(|n| k.attr(n).filter(|v| !v.is_empty() && v.len() <= 1024).map(|v| (n.to_string(), v.to_string())))
        .collect();
}

/// The most kept comment extension markup, all comments together: many times what a busy
/// document's reactions take, and a hostile part can't grow a document past it.
const MAX_COMMENT_EXTENSIONS: usize = 16 << 20;

const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// `ext`, a descendant of `parents` (outermost first), as markup that stands on its own: it
/// gets the parents' namespace declarations for the prefixes it uses (Markup Compatibility's
/// attributes included, [`compatibility_prefixes`]), and the namespaces they mark ignorable
/// (`mc:Ignorable`) join its own, each under a prefix that still names it there. `None`
/// when it uses a prefix nothing declares, which a save could only write as markup no reader
/// accepts.
fn standalone(ext: &craft_xml::El, parents: &[&craft_xml::El]) -> Option<String> {
    let mut scope: Vec<(&str, &str)> = Vec::new();
    // Each namespace a parent marks ignorable, with the prefix that named it there: the mark is
    // on the namespace, not on the prefix's spelling.
    let mut marked: Vec<(&str, &str)> = Vec::new();
    for e in parents {
        declare(&mut scope, e);
        let tokens = e.attrs.iter().find(|(k, _)| is_ignorable(k, &scope)).map(|(_, v)| v.split_whitespace());
        for t in tokens.into_iter().flatten() {
            if let Some(&binding) = scope.iter().find(|(p, _)| *p == t)
                && !marked.iter().any(|(_, u)| *u == binding.1)
            {
                marked.push(binding);
            }
        }
    }
    let mut needed: Vec<&str> = Vec::new();
    if !uses(ext, &scope, &mut Vec::new(), &mut needed) {
        return None;
    }
    let own: Vec<&str> = ext.ns_decls().map(|(p, _)| p).collect();
    let mut el = ext.clone();
    let mut ext_scope = scope.clone();
    declare(&mut ext_scope, ext);
    // A marked namespace keeps the prefix that marked it while that prefix still names it, and
    // otherwise takes another prefix that does. When only a default namespace or markup inside
    // `ext` names it, `ext` declares it under a prefix free there, so the mark still reaches that
    // markup; with none, nothing in `ext` can name it.
    let inner: Vec<(&str, &str)> = ext.descendants().flat_map(craft_xml::El::ns_decls).collect();
    let mut ignorable: Vec<String> = Vec::new();
    let mut fresh: Vec<(String, &str)> = Vec::new();
    for &(p, uri) in &marked {
        let bound = if ext_scope.contains(&(p, uri)) { Some(p) } else { ext_scope.iter().find(|(o, u)| *u == uri && !o.is_empty()).map(|(o, _)| *o) };
        if let Some(o) = bound {
            ignorable.push(o.to_string());
            needed.push(o);
        } else if let Some(&(o, _)) = ext_scope.iter().chain(&inner).find(|(_, u)| *u == uri) {
            // `Ignorable` lists prefixes, so a default namespace gets one.
            let name = free_prefix(if o.is_empty() { "ns" } else { o }, uri, &ext_scope, &inner, &fresh);
            ignorable.push(name.clone());
            fresh.push((name, uri));
        }
    }
    for (name, uri) in &fresh {
        el.attrs.push((format!("xmlns:{name}"), (*uri).to_string()));
    }
    if !ignorable.is_empty() {
        match el.attrs.iter_mut().find(|(k, _)| is_ignorable(k, &ext_scope)) {
            Some((_, v)) => {
                let mut tokens: Vec<&str> = v.split_whitespace().collect();
                tokens.extend(ignorable.iter().map(String::as_str).filter(|t| !v.split_whitespace().any(|o| o == *t)));
                *v = tokens.join(" ");
            }
            None => match ext_scope.iter().find(|(p, u)| *u == MCE && !p.is_empty()).map(|(p, _)| *p) {
                Some(p) => {
                    el.attrs.push((format!("{p}:Ignorable"), ignorable.join(" ")));
                    needed.push(p);
                }
                // `ext` binds every prefix that named Markup Compatibility to something else, so
                // it gets a prefix of its own for it.
                None => {
                    let free = free_prefix("mc", MCE, &ext_scope, &inner, &fresh);
                    el.attrs.push((format!("xmlns:{free}"), MCE.to_string()));
                    el.attrs.push((format!("{free}:Ignorable"), ignorable.join(" ")));
                }
            },
        }
    }
    for (prefix, uri) in scope.iter().filter(|(p, _)| needed.contains(p) && !own.contains(p)) {
        let key = if prefix.is_empty() { "xmlns".to_string() } else { format!("xmlns:{prefix}") };
        if el.attr(&key).is_none() {
            el.attrs.push((key, (*uri).to_string()));
        }
    }
    Some(el.to_xml())
}

/// A prefix for `uri` that a kept element can declare without changing what any of its names
/// mean: `base`, or the first of `base1`, `base2`, ... that names nothing in `scope`, isn't
/// already taken by `fresh`, and isn't declared inside the markup (`inner`) for another namespace.
fn free_prefix(base: &str, uri: &str, scope: &[(&str, &str)], inner: &[(&str, &str)], fresh: &[(String, &str)]) -> String {
    let taken = |p: &str| scope.iter().any(|(s, _)| *s == p) || fresh.iter().any(|(f, _)| f == p) || inner.iter().any(|(i, u)| *i == p && *u != uri);
    std::iter::once(base.to_string()).chain((1..).map(|i| format!("{base}{i}"))).find(|p| !taken(p)).unwrap_or_default()
}

/// `e`'s namespace declarations added to `scope`, each replacing what its prefix named before.
fn declare<'a>(scope: &mut Vec<(&'a str, &'a str)>, e: &'a craft_xml::El) {
    for (prefix, uri) in e.ns_decls() {
        scope.retain(|(p, _)| *p != prefix);
        scope.push((prefix, uri));
    }
}

/// Markup Compatibility's attributes whose values list prefixes.
const MC_PREFIX_LISTS: [&str; 2] = ["Ignorable", "MustUnderstand"];

/// Markup Compatibility's attributes whose values list qualified names (`p:name` or `p:*`).
const MC_NAME_LISTS: [&str; 3] = ["ProcessContent", "PreserveElements", "PreserveAttributes"];

/// Does the prefix `p` name Markup Compatibility, by the declarations in `scope`?
fn is_mce(p: &str, scope: &[(&str, &str)]) -> bool {
    scope.iter().rev().find(|(sp, _)| *sp == p).is_some_and(|(_, u)| *u == MCE)
}

/// Is the attribute named `k` Markup Compatibility's `Ignorable`, by the prefixes in `scope`?
fn is_ignorable(k: &str, scope: &[(&str, &str)]) -> bool {
    k.split_once(':').is_some_and(|(p, local)| local == "Ignorable" && is_mce(p, scope))
}

/// The prefixes `e`'s Markup Compatibility attributes name, by the declarations in `scope`:
/// those `Ignorable` and `MustUnderstand` list, those of the names `ProcessContent`,
/// `PreserveElements` and `PreserveAttributes` list, and those an `mc:Choice`'s `Requires` lists.
/// A reader resolves each against the declarations in scope, so each must stay declared.
fn compatibility_prefixes<'a>(e: &'a craft_xml::El, scope: &[(&str, &str)]) -> Vec<&'a str> {
    let (p, local) = e.name.split_once(':').unwrap_or(("", e.name.as_str()));
    let choice = local == "Choice" && is_mce(p, scope);
    let mut out = Vec::new();
    for (k, v) in &e.attrs {
        let tokens = v.split_whitespace();
        match k.split_once(':') {
            Some((p, local)) if MC_PREFIX_LISTS.contains(&local) && is_mce(p, scope) => out.extend(tokens),
            Some((p, local)) if MC_NAME_LISTS.contains(&local) && is_mce(p, scope) => {
                out.extend(tokens.map(|t| t.split_once(':').map_or("", |(p, _)| p)));
            }
            None if choice && k == "Requires" => out.extend(tokens),
            _ => {}
        }
    }
    out
}

/// Whether every prefix `e` and its descendants use is declared, in the subtree (`path`, the
/// declarations on the way down) or in `outer`; the ones only `outer` declares are added to
/// `needed`. The prefixes Markup Compatibility's attributes name count as used
/// ([`compatibility_prefixes`]). An unprefixed element is in the default namespace, if one is
/// declared.
fn uses<'a>(e: &'a craft_xml::El, outer: &[(&'a str, &'a str)], path: &mut Vec<(&'a str, &'a str)>, needed: &mut Vec<&'a str>) -> bool {
    let mark = path.len();
    path.extend(e.ns_decls());
    let attrs = e.attrs.iter().map(|(k, _)| k.as_str()).filter(|k| *k != "xmlns" && !k.starts_with("xmlns:"));
    let mut prefixes: Vec<&str> =
        [e.name.split_once(':').map_or("", |(p, _)| p)].into_iter().chain(attrs.filter_map(|k| k.split_once(':').map(|(p, _)| p))).collect();
    // Only an element that may carry them pays for building the scope.
    let mc_attr = |k: &str| k.split_once(':').is_some_and(|(_, l)| MC_PREFIX_LISTS.contains(&l) || MC_NAME_LISTS.contains(&l));
    if e.local() == "Choice" || e.attrs.iter().any(|(k, _)| mc_attr(k)) {
        let scope: Vec<(&str, &str)> = outer.iter().chain(path.iter()).copied().collect();
        prefixes.extend(compatibility_prefixes(e, &scope));
    }
    let mut ok = true;
    for p in prefixes {
        if p == "xml" || path.iter().any(|(d, _)| *d == p) || needed.contains(&p) {
            continue;
        }
        if outer.iter().any(|(o, _)| *o == p) {
            needed.push(p);
        } else if !p.is_empty() {
            ok = false;
        }
    }
    ok = ok && e.elements().all(|k| uses(k, outer, path, needed));
    path.truncate(mark);
    ok
}
