//! PPTX import: package → [`Presentation`].

pub(crate) mod chart;
mod shapes;
mod timing;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_drawingml::emu_to_pt;
use craft_drawingml::part::{self, Source};
use craft_drawingml::read as dml;
use craft_drawingml::xmlx::{self, has_rel_refs, keep_with};
use craft_opc::Rels;
use craft_xml::{El, Local, value};

use crate::package::{Doc, Package};
use crate::placeholder::{Owner, parents};
use crate::text::TextBody;
use crate::{
    Background, Comment, CustomShow, Defaults, EmbeddedFont, Error, Geom, HeaderFooter, Layout, LayoutId, LayoutType, Master, MasterId, MediaId,
    MediaItem, PhType, Presentation, Properties, Section, ShowSettings, Size, Slide, SlideId,
};

/// Import state shared by all parts.
pub struct Imp<'a> {
    pub pkg: &'a Package,
    pub defaults: &'a dyn Defaults,
    pub media: Vec<MediaItem>,
    pub media_by_part: HashMap<String, MediaId>,
    pub next: u32,
    pub slide_by_part: HashMap<String, SlideId>,
    pub custom_show_names: HashMap<String, String>,
    /// GUIDs (upper case) of the table styles the file defines.
    pub table_style_ids: Vec<String>,
    /// Per legacy VML drawing part: shape id → preview picture part (see `shapes::vml_preview`).
    pub vml_previews: HashMap<String, HashMap<String, String>>,
    /// Aggregate byte count of retained chart sidecars across the presentation.
    pub kept_sidecar_bytes: usize,
    /// Data slots of the charts decoded so far (see `MAX_PRESENTATION_CHART_POINTS`).
    pub chart_points: usize,
    /// Points the presentation may hold across modelled charts.
    pub chart_points_cap: usize,
    /// Cached kept sidecars by normalized chart part name.
    pub kept_charts: HashMap<String, crate::chart::ChartKept>,
    /// Chart parts whose sidecar capture was refused, with the retained-byte total it started from:
    /// the same start gives the same refusal, so the copy is not repeated.
    pub rejected_kept: HashMap<String, usize>,
    /// Bytes of chart parts decoded so far, rejected ones included (see `MAX_CHART_DECODE_BYTES`).
    pub chart_decode_bytes: usize,
    /// Add-in tags parts, parsed once each, by normalized part name.
    pub tags_by_part: HashMap<String, Arc<[(String, String)]>>,
    /// Bytes of tags copied into the model so far (see [`MAX_TAG_BYTES`]).
    pub tag_bytes: usize,
    /// The tag budget ran out and the warning was given.
    pub tags_capped: bool,
    /// Whether a part has relationships of its own, by normalized part name: looked up once
    /// however many embedded objects point at the part.
    pub has_rels: HashMap<String, bool>,
}

impl<'a> Imp<'a> {
    fn new(pkg: &'a Package, defaults: &'a dyn Defaults, next: u32, chart_points_cap: usize) -> Self {
        Imp {
            pkg,
            defaults,
            media: vec![],
            media_by_part: HashMap::new(),
            next,
            slide_by_part: HashMap::new(),
            custom_show_names: HashMap::new(),
            table_style_ids: Vec::new(),
            vml_previews: HashMap::new(),
            kept_sidecar_bytes: 0,
            chart_points: 0,
            chart_points_cap,
            kept_charts: HashMap::new(),
            rejected_kept: HashMap::new(),
            chart_decode_bytes: 0,
            tags_by_part: HashMap::new(),
            tag_bytes: 0,
            tags_capped: false,
            has_rels: HashMap::new(),
        }
    }
    pub fn alloc(&mut self) -> u32 {
        self.next = self.next.saturating_add(1);
        self.next
    }
    /// Media item for an internal part, read once.
    pub fn media_for(&mut self, part: &str) -> Option<MediaId> {
        let key = craft_opc::normalize(part);
        if let Some(m) = self.media_by_part.get(&key) {
            return Some(*m);
        }
        let data = self.pkg.get_shared(part)?.clone();
        let name = craft_opc::file_of(self.pkg.original_name(part).unwrap_or(part)).to_string();
        let content_type = self.pkg.content_type(part).filter(|c| !c.is_empty()).unwrap_or_else(|| crate::mime_for(&name).to_string());
        let id = MediaId(self.alloc());
        self.media.push(MediaItem { id, name, content_type, data, link: None });
        self.media_by_part.insert(key, id);
        Some(id)
    }
    /// Media item for an external link.
    pub fn media_link(&mut self, url: &str) -> MediaId {
        if let Some(m) = self.media.iter().find(|m| m.link.as_deref() == Some(url)) {
            return m.id;
        }
        let id = MediaId(self.alloc());
        let name = url.rsplit('/').next().unwrap_or(url).to_string();
        let content_type = crate::mime_for(&name).to_string();
        self.media.push(MediaItem { id, name, content_type, data: Arc::new(vec![]), link: Some(url.to_string()) });
        id
    }
}

/// One part being read: its name, relationships and root namespace declarations.
pub struct Part {
    pub name: String,
    pub rels: Rels,
    pub ns: Vec<(String, String)>,
    /// Prefixes bound below the root only (on a wrapper such as `mc:Choice`), each to one namespace.
    pub inner_ns: Vec<(String, String)>,
    /// Bytes of namespace declarations this part may still copy onto kept fragments.
    ns_budget: Cell<usize>,
}

impl Part {
    pub(crate) fn new(pkg: &Package, name: &str, doc: &Doc) -> Self {
        let ns = doc.ns_decls();
        Part {
            name: name.to_string(),
            rels: pkg.rels(name),
            inner_ns: xmlx::inner_decls(&doc.root, &ns),
            ns,
            ns_budget: Cell::new(xmlx::MAX_COPIED_NS_BYTES),
        }
    }
    /// Serialize an element with the namespace declarations it needs from the part root.
    pub fn keep(&self, el: &El) -> String {
        keep_with(el, &[&self.ns], Some(&self.ns_budget))
    }
    /// Like [`Part::keep`], also declaring prefixes that only an enclosing wrapper binds.
    pub fn keep_inner(&self, el: &El) -> String {
        keep_with(el, &[&self.ns, &self.inner_ns], Some(&self.ns_budget))
    }
}

/// The part being read, as the DrawingML readers see it.
pub(crate) struct Src<'s, 'a>(pub &'s mut Imp<'a>, pub &'s Part);

impl Source for Src<'_, '_> {
    fn rel(&self, id: &str) -> Option<part::Rel> {
        self.1.rels.by_id(id).map(|r| part::Rel { target: r.target.clone(), external: r.external })
    }
    fn media(&mut self, target: &str, external: bool) -> Option<MediaId> {
        if external { Some(self.0.media_link(target)) } else { self.0.media_for(target) }
    }
    fn slide(&self, target: &str) -> Option<SlideId> {
        self.0.slide_by_part.get(&craft_opc::normalize(target)).copied()
    }
    fn custom_show(&self, id: &str) -> Option<String> {
        self.0.custom_show_names.get(id).cloned()
    }
    fn keep(&self, el: &El) -> String {
        self.1.keep(el)
    }
    fn keep_inner(&self, el: &El) -> String {
        self.1.keep_inner(el)
    }
}

/// A relationship to an embedded 3-D model (`.glb`).
pub(crate) fn is_model3d(rel: &craft_opc::Rel) -> bool {
    !rel.external && (rel.kind() == "model3d" || rel.target.to_ascii_lowercase().ends_with(".glb"))
}

fn choice_has_model3d(el: &El, rels: &Rels) -> bool {
    fn rec(el: &El, rels: &Rels, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        if el.is(Local("contentPart"))
            && let Some(id) = el.attr_local("r:id").filter(|s| !s.is_empty())
            && rels.by_id(id).is_some_and(is_model3d)
        {
            return true;
        }
        el.elements().any(|c| rec(c, rels, depth + 1))
    }
    rec(el, rels, 0)
}

/// Bytes of tags one import copies into the model, every tags part and reference together: a
/// small compressed tags part listed by many shapes, or many times by one, must not grow without
/// bound. The package budgets ([`MAX_XML_PART`](crate::MAX_XML_PART), [`MAX_TOTAL`](crate::MAX_TOTAL)) bound what is
/// parsed; this bounds what is copied from it.
pub const MAX_TAG_BYTES: usize = 64 << 20;
/// What each copied pair costs besides its text, so empty pairs count too.
const TAG_PAIR_COST: usize = std::mem::size_of::<(String, String)>();

/// Add-in tags named by `holder`'s `p:custDataLst`: the name/value pairs of each tags part.
/// Each part is parsed once per import and counted once per holder. Once [`MAX_TAG_BYTES`] is
/// reached, no further tag parts are parsed or cached.
pub fn tags(imp: &mut Imp, part: &Part, holder: &El) -> Vec<(String, String)> {
    if imp.tags_capped {
        return vec![];
    }
    let Some(list) = holder.child(Local("custDataLst")) else { return vec![] };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for t in list.children(Local("tags")).take(100) {
        if imp.tags_capped {
            return out;
        }
        let Some(target) = t.attr_local("r:id").and_then(|id| part.rels.by_id(id)).map(|r| craft_opc::normalize(&r.target)) else { continue };
        if !seen.insert(target.clone()) {
            continue;
        }
        let parsed = match imp.tags_by_part.get(&target) {
            Some(p) => p.clone(),
            None => {
                if imp.tags_capped {
                    return out;
                }
                let p: Arc<[(String, String)]> = imp
                    .pkg
                    .xml(&target)
                    .map(|doc| {
                        doc.root
                            .children(Local("tag"))
                            .take(10_000)
                            .filter_map(|tag| Some((tag.attr_local("name")?.to_string(), tag.attr_local("val").unwrap_or("").to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                imp.tags_by_part.insert(target, p.clone());
                p
            }
        };
        for (name, val) in parsed.iter() {
            let n = imp.tag_bytes.saturating_add(TAG_PAIR_COST + name.len() + val.len());
            if n > MAX_TAG_BYTES {
                if !imp.tags_capped {
                    log::warn!("pptx: add-in tags over {MAX_TAG_BYTES} bytes; the rest are dropped");
                    imp.tags_capped = true;
                }
                return out;
            }
            imp.tag_bytes = n;
            out.push((name.clone(), val.clone()));
        }
    }
    out
}

/// Relationship types whose parts are kept verbatim, and whether they hang off the package root.
const KEPT_RELS: [(&str, bool); 3] = [
    ("http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml", false),
    ("http://schemas.microsoft.com/office/2020/02/relationships/classificationlabels", true),
    ("http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties", true),
];

/// A part we may keep: outside `ppt/` and the properties the writer makes itself.
fn keepable(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !n.is_empty()
        && !n.contains("..")
        && !n.starts_with("ppt/")
        && n != "docprops/core.xml"
        && n != "docprops/app.xml"
        && !n.starts_with("[content_types]")
}

/// Custom XML data, sensitivity labels and custom properties, with the parts they relate to
/// (`itemProps`) and their relationships parts, as written.
fn kept_parts(pkg: &Package, pres_path: &str) -> Vec<crate::KeptPart> {
    use crate::KeptPart;
    let mut out: Vec<KeptPart> = Vec::new();
    let text = |name: &str| pkg.get(name).and_then(|b| std::str::from_utf8(b).ok()).map(str::to_string);
    for (holder, from_root) in [("", true), (pres_path, false)] {
        let Some(doc) = pkg.xml(&craft_opc::rels_path(holder)) else { continue };
        let base = craft_opc::dir_of(holder);
        for r in doc.root.children(Local("Relationship")) {
            let (Some(ty), Some(target)) = (r.attr_local("Type"), r.attr_local("Target")) else { continue };
            if r.attr_local("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("external"))
                || !KEPT_RELS.iter().any(|(t, root)| *t == ty && *root == from_root)
            {
                continue;
            }
            let name = craft_opc::resolve(base, target);
            if !keepable(&name) || out.iter().any(|k| k.name.eq_ignore_ascii_case(&name)) || out.len() >= 400 {
                continue;
            }
            let Some(xml) = text(&name) else { continue };
            out.push(KeptPart {
                content_type: pkg.content_type(&name).unwrap_or_default(),
                name: name.clone(),
                xml,
                rel_type: ty.to_string(),
                from_root,
            });
            // The part's own relationships (customXml item → itemProps) and their targets.
            let rp = craft_opc::rels_path(&name);
            if let Some(rels_xml) = text(&rp) {
                let mut capped = false;
                for sub in pkg.rels(&name).list.iter().filter(|s| !s.external && keepable(&s.target)) {
                    // Checked before the part is read: repeated relationships to one large part
                    // must not copy it again each time.
                    if out.iter().any(|k| k.name.eq_ignore_ascii_case(&sub.target)) {
                        continue;
                    }
                    if out.len() >= 400 {
                        capped = true;
                        continue;
                    }
                    if let Some(x) = text(&sub.target) {
                        out.push(KeptPart {
                            content_type: pkg.content_type(&sub.target).unwrap_or_default(),
                            name: sub.target.clone(),
                            xml: x,
                            ..Default::default()
                        });
                    }
                }
                if !capped && out.len() < 400 {
                    out.push(KeptPart { name: rp, xml: rels_xml, ..Default::default() });
                }
            }
        }
    }
    out
}

fn load(pkg: &Package, name: &str) -> Option<Doc> {
    let mut d = pkg.xml(name)?;
    let rels = pkg.rels(name);
    // A 3-D model choice stays (even when its `Requires` prefix is not otherwise understood), so
    // the shape reader can keep the model and its fallback picture.
    xmlx::resolve_mc_keeping(&mut d.root, &|ac| ac.children(Local("Choice")).any(|ch| choice_has_model3d(ch, &rels)));
    Some(d)
}

fn max_cnvpr(el: &El, m: &mut u32) {
    let v = el.find_all(Local("cNvPr"));
    for c in v {
        if let Some(id) = c.attr_local("id").and_then(value::to_u32_lossy)
            && id < shapes::MAX_KEPT_ID
        {
            *m = (*m).max(id);
        }
    }
}

/// The theme of a `.thmx` theme file (its `officeDocument` is a theme part), or of the first
/// master of a `.pptx` / `.potx`, with the media its format scheme uses (ids from 1).
pub fn import_theme(bytes: &[u8]) -> Result<(crate::theme::Theme, Vec<MediaItem>), Error> {
    let pkg = Package::open(bytes)?;
    let target = pkg.rels("").first("officeDocument").map(|r| r.target.clone());
    let theme_path = match &target {
        Some(t) if pkg.xml(t).is_some_and(|d| d.root.is(Local("theme"))) => t.clone(),
        _ => {
            // A presentation: its first master's theme.
            let pres = target.unwrap_or_else(|| "ppt/presentation.xml".into());
            let pres_doc = load(&pkg, &pres).ok_or_else(|| Error::NotPptx("neither a theme nor a presentation".into()))?;
            let pres_part = Part::new(&pkg, &pres, &pres_doc);
            let master = pres_doc
                .root
                .child(Local("sldMasterIdLst"))
                .and_then(|l| l.children(Local("sldMasterId")).next())
                .and_then(|m| pres_part.rels.target(m.attr_local("r:id").unwrap_or("")).map(String::from))
                .or_else(|| pres_part.rels.all("slideMaster").next().map(|r| r.target.clone()))
                .ok_or_else(|| Error::NotPptx("the presentation has no slide master".into()))?;
            pkg.rels(&master).first("theme").map(|r| r.target.clone()).ok_or_else(|| Error::NotPptx("the slide master has no theme".into()))?
        }
    };
    let doc = load(&pkg, &theme_path).ok_or_else(|| Error::NotPptx(format!("missing or unreadable {theme_path}")))?;
    if !doc.root.is(Local("theme")) {
        return Err(Error::NotPptx(format!("{theme_path} is not a theme part")));
    }
    let mut imp = Imp::new(&pkg, &crate::Plain, 0, chart::MAX_PRESENTATION_CHART_POINTS);
    let part = Part::new(&pkg, &theme_path, &doc);
    let theme = craft_drawingml::theme::read_theme(&mut Src(&mut imp, &part), &doc.root);
    Ok((theme, imp.media))
}

pub fn import(bytes: &[u8], defaults: &dyn Defaults) -> Result<Presentation, Error> {
    import_checked(bytes, defaults).map(|(p, _)| p)
}

/// [`import`], plus the parts of the slide list the file names but that couldn't be read (a
/// missing or truncated slide): those slides are left out, so a caller can say so.
pub fn import_checked(bytes: &[u8], defaults: &dyn Defaults) -> Result<(Presentation, Vec<String>), Error> {
    import_with_chart_points(bytes, defaults, chart::MAX_PRESENTATION_CHART_POINTS)
}

/// [`import_checked`] with the presentation's chart point budget set (tests lower it).
pub(crate) fn import_with_chart_points(bytes: &[u8], defaults: &dyn Defaults, chart_points_cap: usize) -> Result<(Presentation, Vec<String>), Error> {
    let mut lost_slides: Vec<String> = vec![];
    let pkg = Package::open(bytes)?;
    let root_rels = pkg.rels("");
    let pres_path = root_rels.first("officeDocument").map(|r| r.target.clone()).unwrap_or_else(|| "ppt/presentation.xml".into());
    let pres_doc = load(&pkg, &pres_path).ok_or_else(|| Error::NotPptx(format!("missing or unreadable {pres_path}")))?;
    let pres_el = &pres_doc.root;
    if !pres_el.is(Local("presentation")) {
        return Err(Error::NotPptx(format!("{pres_path} is not a presentation part")));
    }
    let pres_part = Part::new(&pkg, &pres_path, &pres_doc);

    let (master_paths, slide_entries) = master_and_slide_paths(pres_el, &pres_part);
    let (docs, max_id) = load_part_docs(&pkg, &master_paths, &slide_entries);

    let mut imp = Imp::new(&pkg, defaults, max_id.max(1), chart_points_cap);
    // Slide ids first so hyperlinks anywhere can point at slides.
    let slide_ids = register_slides(&mut imp, &docs, &slide_entries, &mut lost_slides);
    read_custom_show_names(&mut imp, pres_el);

    let mut p = Presentation::default();

    read_presentation_settings(&mut imp, &mut p, &pres_part, pres_el);
    let layout_by_part = read_masters(&mut imp, &mut p, &master_paths, &docs);
    let file_slide_ids = read_slides(&mut imp, &mut p, &pres_part, &docs, &slide_ids, &layout_by_part);
    read_sections(&mut p, &pres_part, pres_el, &file_slide_ids);
    p.tags = tags(&mut imp, &pres_part, pres_el);
    read_custom_shows(&mut p, &imp, &pres_part, pres_el);
    read_guides_and_show(&mut p, &imp, &pres_part);
    read_notes_master(&mut imp, &mut p, &pres_part);
    read_handout_master(&mut imp, &mut p, &pres_part);
    read_document_properties(&mut p, &pkg, &root_rels);

    p.kept_parts = kept_parts(&pkg, &pres_path);

    inherit_placeholder_geometry(&mut p);
    default_text_anchor(&mut p);
    p.header_footer = header_footer(&p);
    p.embedded_fonts = read_embedded_fonts(&mut imp, &pres_part, pres_el);
    p.media = std::mem::take(&mut imp.media);
    p.next_id = imp.next;
    p.fix_next_id();
    Ok((p, lost_slides))
}

/// Masters, layouts and slides to read, in file order and without duplicates: (master paths,
/// (file slide id, slide path)).
fn master_and_slide_paths(pres_el: &El, pres_part: &Part) -> (Vec<String>, Vec<(u32, String)>) {
    let master_paths: Vec<String> = pres_el
        .child(Local("sldMasterIdLst"))
        .map(|l| {
            l.children(Local("sldMasterId")).filter_map(|m| pres_part.rels.target(m.attr_local("r:id").unwrap_or(""))).map(String::from).collect()
        })
        .unwrap_or_default();
    let mut master_paths: Vec<String> =
        if master_paths.is_empty() { pres_part.rels.all("slideMaster").map(|r| r.target.clone()).collect() } else { master_paths };
    let mut slide_entries: Vec<(u32, String)> = pres_el
        .child(Local("sldIdLst"))
        .map(|l| {
            l.children(Local("sldId"))
                .take(10_000)
                .filter_map(|s| {
                    Some((
                        s.attr_local("id").and_then(value::to_u32_lossy).unwrap_or(0),
                        pres_part.rels.target(s.attr_local("r:id").unwrap_or(""))?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut seen_masters = HashSet::new();
    master_paths.retain(|m| seen_masters.insert(craft_opc::normalize(m)));
    let mut seen_slides = HashSet::new();
    slide_entries.retain(|(_, p)| seen_slides.insert(craft_opc::normalize(p)));
    (master_paths, slide_entries)
}

/// Loads every master, layout and slide part once. Returns them by normalized path and the
/// largest shape id found in them.
fn load_part_docs(pkg: &Package, master_paths: &[String], slide_entries: &[(u32, String)]) -> (HashMap<String, Doc>, u32) {
    let mut all_parts: Vec<String> = master_paths.to_vec();
    let mut seen_layouts = HashSet::new();
    for m in master_paths {
        for r in pkg.rels(m).all("slideLayout") {
            let key = craft_opc::normalize(&r.target);
            if seen_layouts.insert(key) {
                all_parts.push(r.target.clone());
            }
        }
    }
    all_parts.extend(slide_entries.iter().map(|(_, p)| p.clone()));
    let mut docs: HashMap<String, Doc> = HashMap::new();
    let mut max_id = 0u32;
    for p in &all_parts {
        let key = craft_opc::normalize(p);
        if docs.contains_key(&key) {
            continue;
        }
        if let Some(d) = load(pkg, p) {
            max_cnvpr(&d.root, &mut max_id);
            docs.insert(key, d);
        }
    }
    (docs, max_id)
}

/// Allocates an id for every readable slide; slides that are missing or unreadable are recorded in
/// `lost_slides` and left out.
fn register_slides(
    imp: &mut Imp,
    docs: &HashMap<String, Doc>,
    slide_entries: &[(u32, String)],
    lost_slides: &mut Vec<String>,
) -> Vec<(u32, String, SlideId)> {
    let mut slide_ids: Vec<(u32, String, SlideId)> = vec![];
    for (file_id, path) in slide_entries {
        if !docs.contains_key(&craft_opc::normalize(path)) {
            log::warn!("pptx: slide {path} is missing or unreadable; skipped");
            lost_slides.push(path.clone());
            continue;
        }
        let id = SlideId(imp.alloc());
        imp.slide_by_part.insert(craft_opc::normalize(path), id);
        slide_ids.push((*file_id, path.clone(), id));
    }
    slide_ids
}

fn read_custom_show_names(imp: &mut Imp, pres_el: &El) {
    if let Some(l) = pres_el.child(Local("custShowLst")) {
        for cs in l.children(Local("custShow")) {
            if let (Some(id), Some(name)) = (cs.attr_local("id"), cs.attr_local("name")) {
                imp.custom_show_names.insert(id.to_string(), name.to_string());
            }
        }
    }
}

/// Slide and notes size, first slide number, default text style and table styles.
fn read_presentation_settings(imp: &mut Imp, p: &mut Presentation, pres_part: &Part, pres_el: &El) {
    if let Some(sz) = pres_el.child(Local("sldSz")) {
        let w = sz.attr_local("cx").and_then(value::to_i64_lossy).map(emu_to_pt).unwrap_or(960.0);
        let h = sz.attr_local("cy").and_then(value::to_i64_lossy).map(emu_to_pt).unwrap_or(540.0);
        p.slide_size = Size::new(sane_len(w, 960.0), sane_len(h, 540.0));
    }
    if let Some(sz) = pres_el.child(Local("notesSz")) {
        let w = sz.attr_local("cx").and_then(value::to_i64_lossy).map(emu_to_pt).unwrap_or(540.0);
        let h = sz.attr_local("cy").and_then(value::to_i64_lossy).map(emu_to_pt).unwrap_or(720.0);
        p.notes_size = Size::new(sane_len(w, 540.0), sane_len(h, 720.0));
    }
    p.first_slide_number = pres_el.attr_local("firstSlideNum").and_then(value::to_u32_lossy).unwrap_or(1).min(9999);
    if let Some(dts) = pres_el.child(Local("defaultTextStyle")) {
        p.default_text_style = dml::list_style(&mut Src(imp, pres_part), dts);
    }
    if let Some(r) = pres_part.rels.first("tableStyles")
        && let Some(d) = load(imp.pkg, &r.target)
    {
        let part = Part::new(imp.pkg, &r.target, &d);
        p.table_styles = table_styles(imp, &part, &d.root);
        imp.table_style_ids = p.table_styles.iter().map(|s| s.id.clone()).collect();
    }
}

/// Reads the slide masters and their layouts; a file without a readable master gets a default one.
fn read_masters(imp: &mut Imp, p: &mut Presentation, master_paths: &[String], docs: &HashMap<String, Doc>) -> HashMap<String, LayoutId> {
    let mut layout_by_part: HashMap<String, LayoutId> = HashMap::new();
    for mp in master_paths {
        let Some(doc) = docs.get(&craft_opc::normalize(mp)) else {
            log::warn!("pptx: master {mp} missing; skipped");
            continue;
        };
        let master = read_master(imp, mp, doc, docs, &mut layout_by_part);
        p.masters.push(Arc::new(master));
    }
    if p.masters.is_empty() {
        log::warn!("pptx: no readable slide master; using a default one");
        let (m, last) = imp.defaults.master(p.slide_size, imp.next);
        imp.next = imp.next.max(last);
        p.masters.push(Arc::new(m));
    }
    layout_by_part
}

/// Reads every registered slide with its notes and comments. Returns the slide ids by file id.
fn read_slides(
    imp: &mut Imp,
    p: &mut Presentation,
    pres_part: &Part,
    docs: &HashMap<String, Doc>,
    slide_ids: &[(u32, String, SlideId)],
    layout_by_part: &HashMap<String, LayoutId>,
) -> HashMap<u32, SlideId> {
    let pkg = imp.pkg;
    let fallback_layout = p.masters.first().and_then(|m| m.layouts.first()).map(|l| l.id).unwrap_or_default();
    let mut file_slide_ids: HashMap<u32, SlideId> = HashMap::new();
    // A notes or comments part belongs to one slide; one shared by many must not be read (and
    // copied) once per slide that names it.
    let mut owned_parts: HashSet<String> = HashSet::new();
    for (file_id, path, id) in slide_ids {
        let Some(doc) = docs.get(&craft_opc::normalize(path)) else { continue };
        let part = Part::new(pkg, path, doc);
        let layout =
            part.rels.first("slideLayout").and_then(|r| layout_by_part.get(&craft_opc::normalize(&r.target)).copied()).unwrap_or(fallback_layout);
        let mut slide = read_slide(imp, &part, doc, *id, layout);
        if let Some(notes) = slide_notes(imp, &part, &mut owned_parts) {
            slide.notes = notes;
        }
        read_slide_comments(pkg, pres_part, &part, &mut slide, &mut owned_parts);
        file_slide_ids.insert(*file_id, *id);
        p.slides.push(Arc::new(slide));
    }
    file_slide_ids
}

/// The slide's notes text. A notes part is the notes of the first slide that names it.
fn slide_notes(imp: &mut Imp, part: &Part, owned: &mut HashSet<String>) -> Option<TextBody> {
    let n = part.rels.first("notesSlide")?;
    if !owned.insert(craft_opc::normalize(&n.target)) {
        log::warn!("pptx: notes part {} is named by more than one slide; kept on the first", n.target);
        return None;
    }
    let pkg = imp.pkg;
    let nd = load(pkg, &n.target)?;
    let np = Part::new(pkg, &n.target, &nd);
    Some(read_notes(imp, &np, &nd.root))
}

/// Comments. A modern part and the legacy part can describe the same threads (PowerPoint
/// writes both): the modern one wins, its flattened legacy copy is skipped.
/// Each comments part is read once, for the first slide that names it.
fn read_slide_comments(pkg: &Package, pres_part: &Part, part: &Part, slide: &mut Slide, owned: &mut HashSet<String>) {
    let mut parts: Vec<(Doc, bool, bool)> = vec![];
    for c in part.rels.all("comments") {
        // An already-owned part is not re-read, but its kind still decides modern precedence.
        let fresh = owned.insert(craft_opc::normalize(&c.target));
        if let Some(cd) = pkg.xml(&c.target) {
            // The content type names the modern part; a package that types it only through a
            // Default entry instead of an Override is still modern when its root is p188.
            let modern = pkg.content_type(&c.target).as_deref() == Some(crate::write::CT_COMMENTS_MODERN) || cd.root.prefix() == "p188";
            parts.push((cd, modern, fresh));
        }
    }
    let has_modern = parts.iter().any(|(_, m, _)| *m);
    for (cd, modern, fresh) in parts {
        if !fresh || (has_modern && !modern) {
            continue;
        }
        slide.comments.extend(read_comments(pkg, pres_part, &cd.root, modern));
    }
}

/// Sections, and the extension elements the model does not understand (kept verbatim).
fn read_sections(p: &mut Presentation, pres_part: &Part, pres_el: &El, file_slide_ids: &HashMap<u32, SlideId>) {
    let Some(ext) = pres_el.child(Local("extLst")) else { return };
    if let Some(sl) = ext.find(Local("sectionLst")) {
        for s in sl.children(Local("section")).take(10_000) {
            let slides = s
                .child(Local("sldIdLst"))
                .map(|l| {
                    l.children(Local("sldId"))
                        .filter_map(|x| x.attr_local("id").and_then(value::to_u32_lossy).and_then(|i| file_slide_ids.get(&i).copied()))
                        .collect()
                })
                .unwrap_or_default();
            p.sections.push(Section { name: s.attr_local("name").unwrap_or("").to_string(), slides });
        }
    }
    let kept: String = ext
        .children(Local("ext"))
        .filter(|e| e.attr_local("uri") != Some(crate::SECTION_EXT_URI) && !has_rel_refs(e))
        .map(|e| pres_part.keep(e))
        .collect();
    if !kept.is_empty() {
        p.raw_ext = Some(kept);
    }
}

fn read_custom_shows(p: &mut Presentation, imp: &Imp, pres_part: &Part, pres_el: &El) {
    let Some(l) = pres_el.child(Local("custShowLst")) else { return };
    for cs in l.children(Local("custShow")) {
        let slides = cs
            .child(Local("sldLst"))
            .map(|l| {
                l.children(Local("sld"))
                    .filter_map(|s| pres_part.rels.target(s.attr_local("r:id").unwrap_or("")))
                    .filter_map(|t| imp.slide_by_part.get(&craft_opc::normalize(t)).copied())
                    .collect()
            })
            .unwrap_or_default();
        p.custom_shows.push(CustomShow { name: cs.attr_local("name").unwrap_or("").to_string(), slides });
    }
}

/// Drawing guides (eighths of a point) and slide show settings.
fn read_guides_and_show(p: &mut Presentation, imp: &Imp, pres_part: &Part) {
    let pkg = imp.pkg;
    if let Some(r) = pres_part.rels.first("viewProps")
        && let Some(d) = pkg.xml(&r.target)
        && let Some(lst) = d.root.find(Local("slideViewPr")).and_then(|v| v.find(Local("guideLst")))
    {
        p.guides = lst
            .children(Local("guide"))
            .filter_map(|g| {
                let pos = g.attr_local("pos").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 8.0;
                pos.is_finite().then_some(crate::DrawGuide { vertical: g.attr_local("orient") != Some("horz"), pos })
            })
            .take(1000)
            .collect();
    }
    if let Some(r) = pres_part.rels.first("presProps")
        && let Some(d) = pkg.xml(&r.target)
        && let Some(sp) = d.root.child(Local("showPr"))
    {
        p.show = read_show(sp, &imp.custom_show_names);
    }
}

/// Theme, background, shapes and colour map shared by the notes and handout masters.
fn read_aux_master(imp: &mut Imp, part: &Part, doc: &Doc, m: &mut Master) {
    let mut ctx = shapes::IdCtx::default();
    if let Some(csld) = doc.root.child(Local("cSld")) {
        m.background = background(imp, part, csld);
        if let Some(tree) = csld.child(Local("spTree")) {
            m.shapes = shapes::sp_tree(imp, part, &mut ctx, tree, 0);
        }
    }
    if let Some(cm) = doc.root.child(Local("clrMap")) {
        m.color_map = color_map(cm);
    }
}

/// Reads the part's own theme into the master, if it has one.
fn read_master_theme(imp: &mut Imp, part: &Part, m: &mut Master) -> bool {
    let pkg = imp.pkg;
    if let Some(t) = part.rels.first("theme")
        && let Some(td) = pkg.xml(&t.target)
    {
        let tp = Part::new(pkg, &t.target, &td);
        m.theme = craft_drawingml::theme::read_theme(&mut Src(imp, &tp), &td.root);
        return true;
    }
    false
}

fn read_notes_master(imp: &mut Imp, p: &mut Presentation, pres_part: &Part) {
    let pkg = imp.pkg;
    let Some(r) = pres_part.rels.first("notesMaster") else { return };
    let Some(d) = load(pkg, &r.target) else { return };
    let part = Part::new(pkg, &r.target, &d);
    let mut m = Master { id: MasterId(imp.alloc()), name: "Notes Master".into(), theme: imp.defaults.theme(), ..Default::default() };
    read_master_theme(imp, &part, &mut m);
    read_aux_master(imp, &part, &d, &mut m);
    // Notes text styles are the body's list style; slide masters use title/body/other instead.
    if let Some(ns) = d.root.child(Local("notesStyle")) {
        m.body_style = dml::list_style(&mut Src(imp, &part), ns);
    }
    p.notes_master = Some(Arc::new(m));
}

/// Handout master (header, footer, date and slide number for printed handouts).
fn read_handout_master(imp: &mut Imp, p: &mut Presentation, pres_part: &Part) {
    let pkg = imp.pkg;
    let Some(r) = pres_part.rels.first("handoutMaster") else { return };
    let Some(d) = load(pkg, &r.target) else { return };
    let part = Part::new(pkg, &r.target, &d);
    let mut m = Master { id: MasterId(imp.alloc()), name: "Handout Master".into(), theme: imp.defaults.theme(), ..Default::default() };
    if !read_master_theme(imp, &part, &mut m)
        && let Some(first) = p.masters.first()
    {
        m.theme = first.theme.clone();
    }
    if let Some(n) = d.root.child(Local("cSld")).and_then(|c| c.attr_local("name")).filter(|s| !s.is_empty()) {
        m.name = n.to_string();
    }
    read_aux_master(imp, &part, &d, &mut m);
    p.handout_master = Some(Arc::new(m));
}

fn read_document_properties(p: &mut Presentation, pkg: &Package, root_rels: &Rels) {
    if let Some(r) = root_rels.first("core-properties")
        && let Some(d) = pkg.xml(&r.target)
    {
        read_core(&d.root, &mut p.props);
    }
    if let Some(r) = root_rels.first("extended-properties")
        && let Some(d) = pkg.xml(&r.target)
    {
        p.props.company = d.root.child(Local("Company")).map(|c| c.deep_text()).unwrap_or_default();
    }
}

/// Embedded font faces in one file. Past this, further typefaces are dropped.
const MAX_EMBEDDED_FONTS: usize = 64;
/// One font part. A larger part is skipped; the rest of the file is still read.
const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;

/// `p:embeddedFontLst`: each typeface and the font parts its four face slots point at.
fn read_embedded_fonts(imp: &mut Imp, pres: &Part, root: &El) -> Vec<EmbeddedFont> {
    let Some(list) = root.child(Local("embeddedFontLst")) else { return Vec::new() };
    let mut out = Vec::new();
    for e in list.children(Local("embeddedFont")).take(MAX_EMBEDDED_FONTS) {
        let Some(font) = e.child(Local("font")) else { continue };
        let Some(typeface) = font.attr_local("typeface").map(str::trim).filter(|t| !t.is_empty()) else { continue };
        let typeface: String = typeface.chars().take(255).collect();
        let slot = |imp: &mut Imp, name: &str| -> Option<MediaId> {
            let id = e.child(Local(name))?.attr_local("r:id")?;
            let part = pres.rels.target(id)?;
            let data = imp.pkg.get(part)?;
            if data.is_empty() || data.len() > MAX_FONT_BYTES {
                log::warn!("pptx: embedded font {part} skipped ({} bytes)", data.len());
                return None;
            }
            imp.media_for(part)
        };
        let regular = slot(imp, "regular");
        let bold = slot(imp, "bold");
        let italic = slot(imp, "italic");
        let bold_italic = slot(imp, "boldItalic");
        if regular.is_none() && bold.is_none() && italic.is_none() && bold_italic.is_none() {
            continue;
        }
        out.push(EmbeddedFont {
            typeface,
            panose: panose_attr(font),
            pitch_family: byte_attr(font, "pitchFamily"),
            charset: byte_attr(font, "charset"),
            regular,
            bold,
            italic,
            bold_italic,
        });
    }
    out
}

fn byte_attr(el: &El, name: &str) -> Option<u8> {
    u8::try_from(el.attr_local(name).and_then(value::to_u32_lossy)?).ok()
}

/// `panose` is 20 hex digits (ST_Panose). Anything else is dropped.
fn panose_attr(el: &El) -> Option<String> {
    let v = el.attr_local("panose")?.trim();
    (v.len() == 20 && v.chars().all(|c| c.is_ascii_hexdigit())).then(|| v.to_string())
}

fn sane_len(v: f64, dflt: f64) -> f64 {
    if v.is_finite() && (1.0..=20_000.0).contains(&v) { v } else { dflt }
}

/// Does the element (or a descendant) reference a relationship?
pub fn color_map(cm: &El) -> Vec<(String, String)> {
    cm.attrs.iter().filter(|(k, _)| !k.starts_with("xmlns")).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// A shape's text without an anchor of its own sits at the top, as the standard has it and as
/// PowerPoint draws it. The theme's shape defaults (`a:objectDefaults/a:spDef/a:bodyPr`, centred in
/// Office's themes) are for shapes inserted later, not for the file's own shapes: PowerPoint for
/// Mac draws `<a:bodyPr/>` at the top under an `spDef` with `anchor="ctr"`. The model's own default
/// for shapes is the middle, so a top or bottom anchor is made explicit here and a middle one left
/// unset (placeholders and text boxes keep inheriting).
fn default_text_anchor(p: &mut Presentation) {
    fn fix(shapes: &mut [crate::Shape]) {
        walk_shapes_mut(shapes, &mut |s| {
            if s.ph.is_none()
                && !s.text_box
                && !matches!(s.kind, crate::ShapeKind::Table(_))
                && let Some(t) = &mut s.text
            {
                // Middle is the model's own default for shapes, so it is kept as unset.
                let got = t.body.anchor.unwrap_or(crate::text::Anchor::Top);
                t.body.anchor = (got != crate::text::Anchor::Middle).then_some(got);
            }
        });
    }
    for m in &mut p.masters {
        let m = Arc::make_mut(m);
        fix(&mut m.shapes);
        for l in &mut m.layouts {
            fix(&mut l.shapes);
        }
    }
    for s in &mut p.slides {
        fix(&mut Arc::make_mut(s).shapes);
    }
}

/// Placeholders read without a geometry (an empty custom geometry, see `shapes::sp`) take the
/// geometry of the layout or master placeholder they inherit from, as PowerPoint draws them: a
/// layout's picture placeholder cut into a shape keeps that shape on the slide. Anything left
/// is a rectangle.
fn inherit_placeholder_geometry(p: &mut Presentation) {
    let unset = |g: &Geom| matches!(g, Geom::Custom { paths } if paths.is_empty());
    // Masters first, then layouts (from their masters), then slides (from layouts and masters).
    for m in &mut p.masters {
        let m = Arc::make_mut(m);
        walk_shapes_mut(&mut m.shapes, &mut |s| {
            if unset(&s.geom) {
                s.geom = Geom::default();
            }
        });
    }
    let mut layout_geoms: Vec<(LayoutId, usize, Geom)> = vec![];
    for m in &p.masters {
        for l in &m.layouts {
            for (i, s) in l.shapes.iter().enumerate() {
                if unset(&s.geom) {
                    let (_, mp) = parents(m, Some(l), Owner::Layout, s);
                    layout_geoms.push((l.id, i, mp.map(|m| m.geom.clone()).filter(|g| !unset(g)).unwrap_or_default()));
                }
            }
        }
    }
    for m in &mut p.masters {
        let m = Arc::make_mut(m);
        for l in &mut m.layouts {
            for (id, i, g) in layout_geoms.iter().filter(|(id, _, _)| *id == l.id) {
                let _ = id;
                if let Some(s) = l.shapes.get_mut(*i) {
                    s.geom = g.clone();
                }
            }
        }
    }
    let mut slide_geoms: Vec<(usize, usize, Geom)> = vec![];
    for (si, sl) in p.slides.iter().enumerate() {
        let Some((m, l)) = p.master_for(sl) else { continue };
        for (i, s) in sl.shapes.iter().enumerate() {
            if unset(&s.geom) {
                let (lp, mp) = parents(m, l, Owner::Slide, s);
                let g = lp.map(|l| l.geom.clone()).filter(|g| !unset(g)).or_else(|| mp.map(|m| m.geom.clone()).filter(|g| !unset(g)));
                slide_geoms.push((si, i, g.unwrap_or_default()));
            }
        }
    }
    for (si, i, g) in slide_geoms {
        if let Some(sl) = p.slides.get_mut(si)
            && let Some(s) = Arc::make_mut(sl).shapes.get_mut(i)
        {
            s.geom = g;
        }
    }
    // Placeholders inside groups (rare) and anything still unset: rectangles.
    for sl in &mut p.slides {
        walk_shapes_mut(&mut Arc::make_mut(sl).shapes, &mut |s| {
            if unset(&s.geom) {
                s.geom = Geom::default();
            }
        });
    }
    for m in &mut p.masters {
        for l in &mut Arc::make_mut(m).layouts {
            walk_shapes_mut(&mut l.shapes, &mut |s| {
                if unset(&s.geom) {
                    s.geom = Geom::default();
                }
            });
        }
    }
}

/// Every shape, groups included.
fn walk_shapes_mut(shapes: &mut [crate::Shape], f: &mut dyn FnMut(&mut crate::Shape)) {
    for s in shapes {
        f(s);
        if let crate::ShapeKind::Group { children, .. } = &mut s.kind {
            walk_shapes_mut(children, f);
        }
    }
}

/// `a:tblStyleLst`: the table styles the file defines.
fn table_styles(imp: &mut Imp, part: &Part, root: &El) -> Vec<crate::table::TableStyleDef> {
    use crate::table::{TableBorder, TablePart, TablePartStyle, TableStyleDef};
    root.children(Local("tblStyle"))
        .take(256)
        .filter_map(|st| {
            let id = st.attr_local("styleId")?.to_ascii_uppercase();
            let mut def = TableStyleDef { id, name: st.attr_local("styleName").unwrap_or("").to_string(), raw: part.keep(st), ..Default::default() };
            if let Some(bg) = st.child(Local("tblBg")) {
                let mut ps = TablePartStyle::default();
                if let Some(f) = bg.elements().find(|e| e.local().ends_with("Fill")) {
                    ps.fill = dml::fill_el(&mut Src(imp, part), f);
                }
                if let Some(fr) = bg.child(Local("fillRef")) {
                    ps.fill_ref = Some((
                        fr.attr_local("idx").and_then(value::to_u32_lossy).unwrap_or(0),
                        dml::color(fr).unwrap_or(crate::ColorRef::scheme(craft_drawingml::color::SchemeSlot::Accent1)),
                    ));
                }
                def.background = Some(ps);
            }
            for el in st.elements() {
                let Some(kind) = TablePart::from_xml(el.local()) else { continue };
                let mut ps = TablePartStyle::default();
                if let Some(tx) = el.child(Local("tcTxStyle")) {
                    ps.text_color = dml::color(tx);
                    ps.bold = tx.attr_local("b").map(|v| v == "on");
                    ps.italic = tx.attr_local("i").map(|v| v == "on");
                }
                if let Some(tc) = el.child(Local("tcStyle")) {
                    if let Some(f) = tc.child(Local("fill")).and_then(|f| f.elements().next()) {
                        ps.fill = dml::fill_el(&mut Src(imp, part), f);
                    }
                    if let Some(fr) = tc.child(Local("fillRef")) {
                        ps.fill_ref = Some((
                            fr.attr_local("idx").and_then(value::to_u32_lossy).unwrap_or(0),
                            dml::color(fr).unwrap_or(crate::ColorRef::scheme(craft_drawingml::color::SchemeSlot::Accent1)),
                        ));
                    }
                    if let Some(bd) = tc.child(Local("tcBdr")) {
                        for (i, side) in ["left", "right", "top", "bottom", "insideH", "insideV"].iter().enumerate() {
                            let Some(b) = bd.child(Local(side)) else { continue };
                            let border = if let Some(ln) = b.child(Local("ln")) {
                                Some(TableBorder::Line { line: dml::line(&mut Src(imp, part), ln) })
                            } else {
                                b.child(Local("lnRef")).map(|r| TableBorder::Ref {
                                    idx: r.attr_local("idx").and_then(value::to_u32_lossy).unwrap_or(0),
                                    color: dml::color(r).unwrap_or(crate::ColorRef::scheme(craft_drawingml::color::SchemeSlot::Accent1)),
                                })
                            };
                            if let Some(slot) = ps.borders.get_mut(i) {
                                *slot = border;
                            }
                        }
                    }
                }
                def.parts.push((kind, ps));
            }
            Some(def)
        })
        .collect()
}

fn background(imp: &mut Imp, part: &Part, csld: &El) -> Option<Background> {
    let bg = csld.child(Local("bg"))?;
    if let Some(pr) = bg.child(Local("bgPr")) {
        return Some(Background::Fill { fill: dml::fill_of(&mut Src(imp, part), pr).unwrap_or(crate::Fill::None) });
    }
    let r = bg.child(Local("bgRef"))?;
    Some(Background::Ref {
        idx: r.attr_local("idx").and_then(value::to_u32_lossy).unwrap_or(1001),
        color: dml::color(r).unwrap_or(crate::ColorRef::scheme(craft_drawingml::color::SchemeSlot::Bg1)),
    })
}

fn read_master(imp: &mut Imp, path: &str, doc: &Doc, docs: &HashMap<String, Doc>, layout_by_part: &mut HashMap<String, LayoutId>) -> Master {
    let pkg = imp.pkg;
    let part = Part::new(pkg, path, doc);
    let root = &doc.root;
    let mut m = Master {
        id: MasterId(0),
        theme: imp.defaults.theme(),
        preserve: root.attr_local("preserve").and_then(value::to_bool).unwrap_or(false),
        ..Default::default()
    };
    if let Some(t) = part.rels.first("theme")
        && let Some(td) = pkg.xml(&t.target)
    {
        let tp = Part::new(pkg, &t.target, &td);
        m.theme = craft_drawingml::theme::read_theme(&mut Src(imp, &tp), &td.root);
    }
    m.id = MasterId(imp.alloc());
    if let Some(cm) = root.child(Local("clrMap")) {
        m.color_map = color_map(cm);
    }
    if let Some(csld) = root.child(Local("cSld")) {
        m.name = csld.attr_local("name").unwrap_or("").to_string();
        m.background = background(imp, &part, csld);
        if let Some(tree) = csld.child(Local("spTree")) {
            let mut ctx = shapes::IdCtx::default();
            m.shapes = shapes::sp_tree(imp, &part, &mut ctx, tree, 0);
        }
    }
    if m.name.is_empty() {
        m.name = m.theme.name.clone();
    }
    if let Some(ts) = root.child(Local("txStyles")) {
        if let Some(s) = ts.child(Local("titleStyle")) {
            m.title_style = dml::list_style(&mut Src(imp, &part), s);
        }
        if let Some(s) = ts.child(Local("bodyStyle")) {
            m.body_style = dml::list_style(&mut Src(imp, &part), s);
        }
        if let Some(s) = ts.child(Local("otherStyle")) {
            m.other_style = dml::list_style(&mut Src(imp, &part), s);
        }
    }
    // Layouts in list order, then any other related layouts.
    let mut lpaths: Vec<String> = root
        .child(Local("sldLayoutIdLst"))
        .map(|l| l.children(Local("sldLayoutId")).filter_map(|x| part.rels.target(x.attr_local("r:id").unwrap_or(""))).map(String::from).collect())
        .unwrap_or_default();
    for r in part.rels.all("slideLayout") {
        if !lpaths.iter().any(|x| craft_opc::normalize(x) == craft_opc::normalize(&r.target)) {
            lpaths.push(r.target.clone());
        }
    }
    for lp in lpaths.iter().take(1000) {
        let key = craft_opc::normalize(lp);
        if layout_by_part.contains_key(&key) {
            continue;
        }
        let Some(ld) = docs.get(&key) else { continue };
        let lpart = Part::new(pkg, lp, ld);
        let l = read_layout(imp, &lpart, ld);
        layout_by_part.insert(key, l.id);
        m.layouts.push(l);
    }
    if m.layouts.is_empty() {
        // A master must have a layout for slides to use.
        imp.next = imp.next.saturating_add(1);
        m.layouts.push(Layout {
            id: LayoutId(imp.next),
            name: "Blank".into(),
            kind: LayoutType::Blank,
            show_master_shapes: true,
            ..Default::default()
        });
    }
    if let Some(ext) = root.child(Local("extLst")).filter(|e| !has_rel_refs(e)) {
        m.raw_ext = Some(ext.elements().map(|e| part.keep(e)).collect());
    }
    m
}

fn read_layout(imp: &mut Imp, part: &Part, doc: &Doc) -> Layout {
    let root = &doc.root;
    let mut l = Layout {
        id: LayoutId(imp.alloc()),
        kind: LayoutType::from_xml(root.attr_local("type").unwrap_or("cust")),
        show_master_shapes: root.attr_local("showMasterSp").and_then(value::to_bool).unwrap_or(true),
        preserve: root.attr_local("preserve").and_then(value::to_bool).unwrap_or(false),
        ..Default::default()
    };
    if let Some(csld) = root.child(Local("cSld")) {
        l.name = csld.attr_local("name").unwrap_or("").to_string();
        l.background = background(imp, part, csld);
        if let Some(tree) = csld.child(Local("spTree")) {
            let mut ctx = shapes::IdCtx::default();
            l.shapes = shapes::sp_tree(imp, part, &mut ctx, tree, 0);
        }
    }
    l.color_map = color_map_override(root);
    if let Some(ext) = root.child(Local("extLst")).filter(|e| !has_rel_refs(e)) {
        l.raw_ext = Some(ext.elements().map(|e| part.keep(e)).collect());
    }
    l
}

/// A layout's or slide's `clrMapOvr/overrideClrMapping` (None for `masterClrMapping` or none).
fn color_map_override(root: &El) -> Option<Vec<(String, String)>> {
    root.child(Local("clrMapOvr"))?.child(Local("overrideClrMapping")).map(color_map)
}

fn read_slide(imp: &mut Imp, part: &Part, doc: &Doc, id: SlideId, layout: LayoutId) -> Slide {
    let root = &doc.root;
    let mut s = Slide {
        id,
        layout,
        hidden: root.attr_local("show").and_then(value::to_bool) == Some(false),
        show_master_shapes: root.attr_local("showMasterSp").and_then(value::to_bool).unwrap_or(true),
        color_map: color_map_override(root),
        ..Default::default()
    };
    let mut ctx = shapes::IdCtx::default();
    if let Some(csld) = root.child(Local("cSld")) {
        s.name = csld.attr_local("name").unwrap_or("").to_string();
        s.tags = tags(imp, part, csld);
        s.background = background(imp, part, csld);
        if let Some(tree) = csld.child(Local("spTree")) {
            s.shapes = shapes::sp_tree(imp, part, &mut ctx, tree, 0);
        }
        if let Some(c) = csld.child(Local("controls")) {
            let pics = shapes::controls(imp, part, &mut ctx, c);
            s.shapes.extend(pics);
        }
    }
    if let Some(t) = root.child(Local("transition")) {
        s.transition = timing::transition(imp, part, t);
    }
    if let Some(t) = root.child(Local("timing")) {
        s.animations = timing::animations(imp, part, &ctx, t, &s.shapes);
        timing::media_settings(t, &ctx, &mut s.shapes);
    }
    if let Some(ext) = root.child(Local("extLst")).filter(|e| !has_rel_refs(e)) {
        s.raw_ext = Some(ext.elements().map(|e| part.keep(e)).collect());
    }
    s
}

fn read_notes(imp: &mut Imp, part: &Part, root: &El) -> TextBody {
    let Some(tree) = root.path(&[Local("cSld"), Local("spTree")]) else { return TextBody::default() };
    for sp in tree.children(Local("sp")) {
        let ph = sp.path(&[Local("nvSpPr"), Local("nvPr"), Local("ph")]);
        let is_body = ph.is_some_and(|ph| matches!(ph.attr_local("type"), Some("body") | None) && ph.attr_local("type") != Some("sldImg"));
        if is_body && let Some(tx) = sp.child(Local("txBody")) {
            let mut t = dml::text_body(&mut Src(imp, part), tx);
            // Notes keep only content: drop body formatting from the notes master placeholder.
            t.list_style = Default::default();
            if t.is_empty() {
                return TextBody::default();
            }
            return t;
        }
    }
    TextBody::default()
}

fn read_comments(pkg: &Package, pres: &Part, root: &El, modern: bool) -> Vec<Comment> {
    // Authors: legacy commentAuthors.xml (numeric ids) and modern authors.xml (GUIDs).
    let mut authors: HashMap<String, (String, String)> = HashMap::new();
    for kind in ["commentAuthors", "authors"] {
        if let Some(r) = pres.rels.first(kind)
            && let Some(d) = pkg.xml(&r.target)
        {
            for a in d.root.elements() {
                if let Some(id) = a.attr_local("id") {
                    authors
                        .insert(id.to_string(), (a.attr_local("name").unwrap_or("").to_string(), a.attr_local("initials").unwrap_or("").to_string()));
                }
            }
        }
    }
    // Legacy positions are 1/8 pt; the modern part's p188:pos is an a:CT_Point2D in EMU.
    let scale = if modern { 12_700.0 } else { 8.0 };
    fn one(e: &El, authors: &HashMap<String, (String, String)>, depth: usize, scale: f64) -> Comment {
        let (author, initials) = e.attr_local("authorId").and_then(|a| authors.get(a).cloned()).unwrap_or_default();
        let text = if let Some(t) = e.child(Local("text")) {
            t.deep_text()
        } else if let Some(tb) = e.child(Local("txBody")) {
            tb.children(Local("p")).map(|p| p.deep_text()).collect::<Vec<_>>().join("\n")
        } else {
            String::new()
        };
        let pos = e.child(Local("pos"));
        let replies = if depth < 4 {
            e.child(Local("replyLst"))
                .map(|l| l.children(Local("reply")).take(1000).map(|r| one(r, authors, depth + 1, scale)).collect())
                .unwrap_or_default()
        } else {
            vec![]
        };
        Comment {
            author,
            initials,
            text,
            date: e.attr_local("dt").or(e.attr_local("created")).unwrap_or("").to_string(),
            x: pos.and_then(|p| p.attr_local("x").and_then(value::to_f64)).unwrap_or(0.0) / scale,
            y: pos.and_then(|p| p.attr_local("y").and_then(value::to_f64)).unwrap_or(0.0) / scale,
            resolved: e.attr_local("status") == Some("resolved"),
            replies,
            shape: None,
        }
    }
    root.children(Local("cm")).take(10_000).map(|c| one(c, &authors, 0, scale)).collect()
}

fn read_show(sp: &El, shows: &HashMap<String, String>) -> ShowSettings {
    let mut s = ShowSettings {
        loop_until_esc: sp.attr_local("loop").and_then(value::to_bool).unwrap_or(false),
        without_narration: sp.attr_local("showNarration").and_then(value::to_bool) == Some(false),
        without_animation: sp.attr_local("showAnimation").and_then(value::to_bool) == Some(false),
        use_timings: sp.attr_local("useTimings").and_then(value::to_bool).unwrap_or(true),
        show_media_controls: sp.find(Local("showMediaCtrls")).and_then(|e| e.attr_local("val").and_then(value::to_bool)).unwrap_or(true),
        ..Default::default()
    };
    if sp.child(Local("browse")).is_some() {
        s.show_type = "browsed".into();
    } else if sp.child(Local("kiosk")).is_some() {
        s.show_type = "kiosk".into();
    }
    if let Some(r) = sp.child(Local("sldRg")) {
        s.range = Some((
            r.attr_local("st").and_then(value::to_u32_lossy).unwrap_or(1).max(1),
            r.attr_local("end").and_then(value::to_u32_lossy).unwrap_or(1).max(1),
        ));
    }
    if let Some(c) = sp.child(Local("custShow")) {
        let id = c.attr_local("id").unwrap_or("");
        s.custom_show = Some(shows.get(id).cloned().unwrap_or_else(|| id.to_string()));
    }
    if let Some(c) = sp.child(Local("penClr")).and_then(dml::color)
        && let crate::ColorBase::Rgb { rgb } = c.base
    {
        s.pen_color = rgb;
    }
    s
}

fn read_core(root: &El, p: &mut Properties) {
    for e in root.elements() {
        let t = e.deep_text();
        match e.local() {
            "title" => p.title = t,
            "subject" => p.subject = t,
            "creator" => p.author = t,
            "keywords" => p.keywords = t,
            "description" => p.comments = t,
            "category" => p.category = t,
            "lastModifiedBy" => p.last_modified_by = t,
            "revision" => p.revision = t.trim().parse().unwrap_or(0),
            "created" => p.created = t,
            "modified" => p.modified = t,
            _ => {}
        }
    }
}

/// Which footer placeholders the slides show.
fn header_footer(p: &Presentation) -> HeaderFooter {
    let mut hf = HeaderFooter::default();
    for s in &p.slides {
        for sh in &s.shapes {
            match sh.ph_type() {
                Some(PhType::SlideNum) => hf.slide_number = true,
                Some(PhType::Date) => {
                    hf.date = true;
                    if let Some(t) = &sh.text
                        && !t.paragraphs.iter().flat_map(|p| &p.runs).any(|r| matches!(r.kind, crate::text::RunKind::Field { .. }))
                        && hf.date_text.is_empty()
                    {
                        hf.date_text = t.text();
                    }
                }
                Some(PhType::Footer) => {
                    hf.footer = true;
                    if hf.footer_text.is_empty()
                        && let Some(t) = &sh.text
                    {
                        hf.footer_text = t.text();
                    }
                }
                _ => {}
            }
        }
    }
    hf
}
