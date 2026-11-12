use super::bounds::{describe, is_container, mask_extent, nodes_bounds, pages, rect, tree_bounds, union};
use super::{
    Affine, Archive, BTreeMap, Blend, Document, Effect, Error, HashSet, Image, Kind, Limits, Node, ObjId, Paint, Path, Point, Rect, Spread, Stream,
    Tag, paint, stream,
};

/// Largest embedded colour profile kept (real CMYK profiles are a few megabytes).
const MAX_PROFILE_BYTES: usize = 32 << 20;

/// Deepest layer nesting followed; deeper content is reported, not imported.
pub(super) const MAX_TREE_DEPTH: usize = 128;

/// Total layer nodes imported.
const MAX_NODES: usize = 500_000;

/// Pixels of pixel layers a reader starts with; [`read`] sets what [`Limits::max_pixel_bytes`]
/// allows (this many by default: 2 GiB as RGBA).
const MAX_LAYER_PIXELS: u64 = 512 << 20;

/// Embedded documents inside embedded documents are followed this deep.
const MAX_EMBED_DEPTH: usize = 4;

/// Characters of all stories together (a story may be shared by many text layers).
const MAX_DOCUMENT_CHARS: usize = 16 << 20;

/// Table cells of all tables together (each becomes a text object with its edge lines).
const MAX_DOCUMENT_CELLS: usize = 1 << 18;

/// Points of all strokes' pressure curves together (a stroke style may be shared by many layers).
const MAX_DOCUMENT_PRESSURE_POINTS: usize = 1 << 20;

/// Child references inspected, page rectangles produced, and outline segments retained, in all
/// (shared objects are visited once per link, so each visit is charged). Real documents stay
/// within a few times [`MAX_NODES`]; this leaves ~30x headroom over that.
const MAX_DOCUMENT_WORK: usize = 1 << 24;

/// Owned layer-name characters kept in all (each visit of a shared object keeps its own copy).
const MAX_DOCUMENT_NAME_CHARS: usize = 16 << 20;

/// What a whole read may still use. Shared objects can be linked from many layers, and embedded
/// documents are read with what their embedding document left, so linking or embedding the
/// same content many times can't multiply the work.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    /// Layer nodes ([`MAX_NODES`] in all).
    pub(crate) nodes: usize,
    /// Pixels the pixel layers may decode ([`Limits::max_pixel_bytes`] as RGBA in all).
    pub(crate) pixels: u64,
    /// Characters of stories ([`MAX_DOCUMENT_CHARS`] in all).
    pub(crate) chars: usize,
    /// Table cells ([`MAX_DOCUMENT_CELLS`] in all).
    pub(crate) cells: usize,
    /// Points of pressure curves ([`MAX_DOCUMENT_PRESSURE_POINTS`] in all).
    pub(crate) pressure: usize,
    /// Child references, page rectangles and outline segments ([`MAX_DOCUMENT_WORK`] in all).
    pub(crate) work: usize,
    /// Layer-name characters ([`MAX_DOCUMENT_NAME_CHARS`] in all).
    pub(crate) name_chars: usize,
}

impl Budget {
    pub(crate) const FULL: Budget = Budget {
        nodes: MAX_NODES,
        pixels: MAX_LAYER_PIXELS,
        chars: MAX_DOCUMENT_CHARS,
        cells: MAX_DOCUMENT_CELLS,
        pressure: MAX_DOCUMENT_PRESSURE_POINTS,
        work: MAX_DOCUMENT_WORK,
        name_chars: MAX_DOCUMENT_NAME_CHARS,
    };

    /// Take `n` units of work, or fail the read.
    pub(crate) fn spend(&mut self, n: usize) -> Result<(), Error> {
        self.work = self.work.checked_sub(n).ok_or(Error::Limit("too much shared geometry or page data"))?;
        Ok(())
    }
}

/// Read a document: archive, `doc.dat` and the model.
pub fn read(bytes: &[u8], limits: Limits) -> Result<Document, Error> {
    let pixels = u64::try_from(limits.max_pixel_bytes / 4).unwrap_or(u64::MAX);
    read_at(bytes, limits, &mut Budget { pixels, ..Budget::FULL }, Affine::IDENTITY, 0).document
}

/// A document read by [`read_at`] (or why it couldn't be), with the bytes its archive extracted
/// either way.
struct ReadAt {
    document: Result<Document, Error>,
    extracted: usize,
}

/// Read a document whose spreads are placed by `base` (for embedded documents, nested `depth`
/// deep). What it uses is taken from `budget`, also when it fails part of the way.
/// The most characters of a layer's name kept.
const MAX_NAME_CHARS: usize = 1024;

fn read_at(bytes: &[u8], limits: Limits, budget: &mut Budget, base: Affine, depth: usize) -> ReadAt {
    let mut archive = match Archive::open(bytes, limits) {
        Ok(a) => a,
        Err(e) => return ReadAt { document: Err(e), extracted: 0 },
    };
    let document = read_archive(&mut archive, budget, base, depth);
    ReadAt { document, extracted: archive.extracted() }
}

fn read_archive(archive: &mut Archive, budget: &mut Budget, base: Affine, depth: usize) -> Result<Document, Error> {
    let doc = archive.read("doc.dat")?;
    let s = stream::parse(&doc)?;
    // The parsed stream owns copies of all it needs: the inflated bytes (hundreds of megabytes
    // for a large document) are not held while the model is built.
    drop(doc);
    let mut reader = Reader::new(&s, archive, base, depth);
    reader.budget = *budget;
    let document = reader.document();
    *budget = reader.budget;
    document
}

pub(crate) struct Reader<'s, 'a, 'b> {
    pub(crate) s: &'s Stream,
    pub(crate) archive: &'b mut Archive<'a>,
    pub(crate) dpi: f64,
    pub(crate) warnings: BTreeMap<String, usize>,
    active: HashSet<ObjId>,
    /// Where the spreads go: the identity, or an embedding document's placement.
    base: Affine,
    /// The linear part of `base`, which also scales frame text.
    pub(crate) type_base: Affine,
    /// How deep inside embedded documents this one is.
    depth: usize,
    /// An embedded document's content, waiting to become its node's children.
    pending: Option<Vec<Node>>,
    /// The master spreads, in the order of [`Document::masters`].
    master_ids: Vec<ObjId>,
    /// What the rest of the read may use.
    pub(crate) budget: Budget,
}

impl<'s, 'a, 'b> Reader<'s, 'a, 'b> {
    pub(crate) fn new(s: &'s Stream, archive: &'b mut Archive<'a>, base: Affine, depth: usize) -> Self {
        let [a, b, c, d, ..] = base.0;
        Self {
            s,
            archive,
            dpi: 72.0,
            warnings: BTreeMap::new(),
            active: HashSet::new(),
            base,
            type_base: Affine([a, b, c, d, 0.0, 0.0]),
            depth,
            pending: None,
            master_ids: Vec::new(),
            budget: Budget::FULL,
        }
    }

    /// The objects of an array field (nulls skipped). Its raw slots, null ones included, are
    /// charged to the document's work first: a shared object's arrays are scanned again on every
    /// visit, and a long array of nulls yields nothing a node or segment limit would count. Past
    /// the work left, the array is not scanned: a warning, and no objects.
    pub(crate) fn objs(&mut self, id: ObjId, tag: &[u8; 4]) -> Vec<ObjId> {
        if self.budget.spend(self.s.array_len(id, tag).unwrap_or(0)).is_err() {
            self.warn("lists beyond the work one document may take (left out)");
            return Vec::new();
        }
        self.s.objs(id, tag)
    }

    pub(crate) fn warn(&mut self, what: impl Into<String>) {
        *self.warnings.entry(what.into()).or_default() += 1;
    }

    /// The document.
    fn document(&mut self) -> Result<Document, Error> {
        let s = self.s;
        let root = s.root;
        self.dpi = s.obj(root, b"UVCn").and_then(|u| s.f64(u, b"UPPI")).filter(|d| *d > 0.0 && *d < 100_000.0).unwrap_or(72.0);
        let saved_by = s.obj(root, b"NVer").map(|v| {
            let product = s.str(v, b"Prod").unwrap_or("Affinity");
            match (s.int(v, b"Majr"), s.int(v, b"Minr"), s.int(v, b"Bild")) {
                (Some(a), Some(b), Some(c)) => format!("{product} {a}.{b}.{c}"),
                _ => product.to_string(),
            }
        });
        let doc = s.obj(root, b"DocR").ok_or(Error::Malformed("document has no root node"))?;
        self.master_ids = s.objs(doc, b"MpCh").into_iter().filter(|m| s.is(*m, b"Sprd")).collect();
        let mut spreads = Vec::new();
        for spread in s.objs(doc, b"Chld") {
            if let Some(sp) = self.spread(doc, spread)? {
                spreads.push(sp);
            }
        }
        if spreads.is_empty() {
            return Err(Error::Malformed("document has no pages"));
        }
        let mut masters = Vec::new();
        for master in self.master_ids.clone() {
            match self.spread(doc, master)? {
                Some(sp) => masters.push(sp),
                // Keep the indices of `MasterInstance::master` valid.
                None => masters.push(Spread {
                    bounds: Rect { x0: 0.0, y0: 0.0, x1: 0.0, y1: 0.0 },
                    pages: Vec::new(),
                    transparent: true,
                    nodes: Vec::new(),
                }),
            }
        }
        let cmyk_profile = s
            .obj(root, b"CMYP")
            .and_then(|p| s.bytes(p, b"Data"))
            .filter(|b| b.len() <= MAX_PROFILE_BYTES && b.get(16..20) == Some(b"CMYK".as_slice()))
            .map(<[u8]>::to_vec);
        let warnings = std::mem::take(&mut self.warnings).into_iter().map(|(w, n)| if n > 1 { format!("{w} ({n}×)") } else { w }).collect();
        Ok(Document { dpi: self.dpi, spreads, masters, saved_by, warnings, cmyk_profile })
    }

    /// A spread (`Sprd`) of the document `doc`, or of its master spreads.
    fn spread(&mut self, doc: ObjId, spread: ObjId) -> Result<Option<Spread>, Error> {
        let s = self.s;
        if !s.is(spread, b"Sprd") {
            self.warn("an unknown kind of page");
            return Ok(None);
        }
        // The page list is scanned whole (null slots too) every time the spread is listed.
        self.budget.spend(s.obj(spread, b"SpMd").and_then(|md| s.array_len(md, b"PagR")).unwrap_or(0))?;
        let pages = pages(s, spread);
        self.budget.spend(pages.len() + 1)?;
        let default_size = s.floats::<2>(doc, b"DfSz").filter(|[w, h]| *w > 0.0 && *h > 0.0).map(|[w, h]| Rect { x0: 0.0, y0: 0.0, x1: w, y1: h });
        let bounds = s.floats::<4>(spread, b"SprB").map(rect).or_else(|| pages.iter().copied().reduce(union)).or(default_size).unwrap_or(Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
        let transparent = s.bool(spread, b"SprT").unwrap_or(false);
        let (nodes, mask) = self.children(spread, self.base, 0)?;
        if mask.is_some() {
            self.warn("mask layers directly on a page (imported without them)");
        }
        Ok(Some(Spread { bounds, pages, transparent, nodes }))
    }

    /// The children of `parent`, and the mask layer among them that masks the parent (with its
    /// placement), read later over just what it masks.
    #[allow(clippy::type_complexity)]
    fn children(&mut self, parent: ObjId, world: Affine, depth: usize) -> Result<(Vec<Node>, Option<(ObjId, Affine)>), Error> {
        let s = self.s;
        let mut out = Vec::new();
        let mut mask = None;
        // Invisible masks, skipped children and null slots cost no node, so the visit is charged
        // here, by the raw slots, before the list is scanned.
        self.budget.spend(s.array_len(parent, b"Chld").unwrap_or(0))?;
        let kids = s.objs(parent, b"Chld");
        for c in kids {
            if s.is(c, b"MRst") {
                // A mask layer inside a group or layer masks that container.
                if s.bool(c, b"Visi") == Some(false) {
                    continue;
                }
                let local = s.floats::<6>(c, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY);
                if mask.is_none() {
                    mask = Some((c, local.then(world)));
                } else {
                    self.warn("several pixel masks on one layer (only the first is used)");
                }
                continue;
            }
            if let Some(n) = self.node(c, world, depth + 1)? {
                out.push(n);
            }
        }
        Ok((out, mask))
    }

    fn node(&mut self, id: ObjId, parent: Affine, depth: usize) -> Result<Option<Node>, Error> {
        let s = self.s;
        if depth > MAX_TREE_DEPTH {
            self.warn("layers nested deeper than 128 levels");
            return Ok(None);
        }
        let Some(left) = self.budget.nodes.checked_sub(1) else {
            return Err(Error::Limit("too many layers"));
        };
        self.budget.nodes = left;
        // A node that contains itself (a shared object linked back into its own subtree).
        if !self.active.insert(id) {
            self.warn("a layer that contains itself");
            return Ok(None);
        }
        let local = s.floats::<6>(id, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY);
        let world = local.then(parent);
        let class = s.class(id).unwrap_or(Tag(0));
        // A layer name is short; a long one is cut, as every visit of a shared object keeps a copy.
        let name: String = s.str(id, b"Desc").unwrap_or_default().chars().take(MAX_NAME_CHARS.min(self.budget.name_chars)).collect();
        self.budget.name_chars -= name.chars().count();
        let visible = s.bool(id, b"Visi").unwrap_or(true);
        let locked = s.bool(id, b"Edtb").is_some_and(|e| !e);
        let opacity = s.f64(id, b"Opac").map_or(1.0, |o| o.clamp(0.0, 1.0));
        let blend = match s.enumeration(id, b"Blnd") {
            None if s.bool(id, b"PasT") != Some(false) && is_container(class) => Blend::PassThrough,
            None => Blend::Normal,
            Some((i, v)) => Blend::from_enum(i, v).unwrap_or_else(|| {
                self.warn("an unknown blend mode (imported as Normal)");
                Blend::Normal
            }),
        };
        let kind = if !world.is_finite() {
            self.warn("a layer with an invalid transform");
            Kind::Unsupported
        } else {
            self.kind(id, class, world, parent)?
        };
        // This node's embedded content, before its children's `node` calls can take it.
        let embedded = self.pending.take();
        // A compound's children are its operands, already merged into its outline.
        let (mut children, child_mask) = if class == Tag::of(b"Comp") { (Vec::new(), None) } else { self.children(id, world, depth)? };
        if let Some(embedded) = embedded {
            children.extend(embedded);
        }
        // Pixel masks are read only over what they mask (they can span a whole spread at print
        // resolution, a gigapixel).
        let over = mask_extent(&kind, &children);
        let child_mask = match child_mask {
            Some((c, at)) => crate::raster::mask(self, c, at, over)?,
            None => None,
        };
        let (mask, attached_mask) = self.attached_masks(id, world, over)?;
        let pixel_mask = attached_mask.or(child_mask);
        self.active.remove(&id);
        let mut node = Node { class, name, visible, locked, opacity, blend, kind, mask, pixel_mask, effects: Vec::new(), children };
        // Gradient overlays are laid out over the node's bounds, also while it is hidden.
        let bounds = tree_bounds(&node);
        node.effects = self.effects(id, world, bounds);
        Ok(Some(node))
    }

    fn kind(&mut self, id: ObjId, class: Tag, world: Affine, parent: Affine) -> Result<Kind, Error> {
        let s = self.s;
        Ok(match &class.0.to_be_bytes() {
            b"Scop" => Kind::Layer,
            b"Grup" => Kind::Group,
            b"PCrv" | b"ShpN" | b"Comp" | b"SNEN" | b"SNRR" | b"ShRN" => {
                // Affinity 3.2.3's two-board public fixture marks board shapes with an `aprp`
                // property object in `phrp`, while older documents use `ABEn`. Ordinary shapes
                // in that fixture have neither. See the pinned corpus and validation audit.
                if s.bool(id, b"ABEn") == Some(true) || s.obj(id, b"phrp").is_some_and(|p| s.is(p, b"aprp")) {
                    return Ok(self.artboard(id, world));
                }
                let Some((local, even_odd)) = crate::geometry::outline(self, id, class, world) else {
                    self.warn(format!("a {} that could not be read", describe(class)));
                    return Ok(Kind::Unsupported);
                };
                self.budget.spend(local.subpaths.iter().map(|sp| sp.segments.len() + 1).sum())?;
                let (fills, strokes) = paint::node_paint(self, id, world);
                Kind::Shape { path: local.transformed(world), fills, strokes, even_odd }
            }
            b"TxtT" => match crate::text::table(self, id, world) {
                Some(t) => Kind::Table(t),
                None => {
                    self.warn("tables that could not be read");
                    Kind::Unsupported
                }
            },
            b"TxtA" | b"TxtF" | b"TxtC" => match crate::text::read(self, id, world) {
                Some(t) => Kind::Text(t),
                None => {
                    self.warn("text whose story could not be read (master-page text, or an unknown layout)");
                    Kind::Unsupported
                }
            },
            b"ImgN" | b"Rstr" => match crate::raster::node_image(self, id, world)? {
                Some(img) => Kind::Image(img),
                None => Kind::Unsupported,
            },
            b"MPIN" => {
                // `SLnk` links the instance with the master spread it shows.
                let linked = s.obj(id, b"SLnk").map(|l| self.objs(l, b"ILOb")).unwrap_or_default();
                let master = linked.iter().find_map(|o| self.master_ids.iter().position(|m| m == o));
                let at = |t: &[u8; 4], default: usize| s.int(id, t).and_then(|v| usize::try_from(v).ok()).unwrap_or(default).min(4096);
                match master {
                    Some(master) => Kind::MasterInstance { master, page: at(b"PgOf", 0), master_page: at(b"MPOf", 0), count: at(b"PgCt", 1) },
                    None => {
                        self.warn("master page instances without their master");
                        Kind::Unsupported
                    }
                }
            }
            b"EmbN" => match self.embedded_document(id, world) {
                // The embedded document itself, as editable content.
                Some(nodes) => {
                    self.pending = Some(nodes);
                    Kind::Group
                }
                None => match crate::raster::embedded(self, id, world)? {
                    Some(img) => {
                        self.warn("embedded documents and symbols (imported as pictures of them)");
                        Kind::Image(img)
                    }
                    None => {
                        self.warn("embedded documents and symbols without a cached picture");
                        Kind::Unsupported
                    }
                },
            },
            b"FRst" => {
                // A fill layer paints its whole bitmap area; its gradient lives in page space.
                let b = s.obj(id, b"Bitm");
                let size = |t: &[u8; 4]| b.and_then(|b| s.int(b, t)).map(|v| v as f64).filter(|v| *v > 0.0);
                match (size(b"BmpW"), size(b"BmpH")) {
                    (Some(w), Some(h)) => {
                        let local = Path { subpaths: vec![crate::shapes::rectangle(0.0, 0.0, w, h)] };
                        let fills = paint::fills(self, id, parent, b"BFFl");
                        Kind::Shape { path: local.transformed(world), fills, strokes: Vec::new(), even_odd: false }
                    }
                    _ => {
                        self.warn("fill layers without an area");
                        Kind::Unsupported
                    }
                }
            }
            _ => {
                self.warn(format!("{} layers", describe(class)));
                Kind::Unsupported
            }
        })
    }

    /// The enabled layer effects (`FiEf`) of node `id`. The angle convention was measured on the
    /// public MIT fx fixture (Patchy): an angle of π/2 with offset 10 shadows 10 below.
    fn effects(&mut self, id: ObjId, world: Affine, bounds: Option<Rect>) -> Vec<Effect> {
        let s = self.s;
        let mut out = Vec::new();
        for e in self.objs(id, b"FiEf").into_iter().take(64) {
            if s.bool(e, b"Enab") == Some(false) {
                continue;
            }
            // "Scale with object": lengths follow the node's transform; otherwise they are absolute.
            let k = if s.bool(e, b"SclO") == Some(true) { world.scale() } else { 1.0 };
            let len = |t: &[u8; 4]| s.f64(e, t).filter(|v| v.is_finite() && *v >= 0.0 && *v < 1e6).unwrap_or(0.0) * k;
            let opacity = s.f64(e, b"Opac").filter(|v| v.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
            let angle = s.f64(e, b"Angl").filter(|v| v.is_finite()).unwrap_or(0.0);
            let color = s.obj(e, b"Colr").and_then(|c| crate::paint::color(self, c));
            let class = s.class(e).map(|c| c.0.to_be_bytes());
            match (class.as_ref(), color) {
                (Some(b"Shad"), Some(color)) => out.push(Effect::DropShadow { color, opacity, radius: len(b"Radi"), distance: len(b"Offs"), angle }),
                (Some(b"InnS"), Some(color)) => out.push(Effect::InnerShadow { color, opacity, radius: len(b"Radi"), distance: len(b"Offs"), angle }),
                (Some(b"OutG"), Some(color)) => out.push(Effect::OuterGlow { color, opacity, radius: len(b"Radi") }),
                (Some(b"Strk"), Some(color)) if s.enumeration(e, b"Ftyp").is_none_or(|(t, _)| t == 0) => {
                    let align = s.enumeration(e, b"Alig").map_or(0, |(a, _)| a);
                    out.push(Effect::Outline { color, opacity, width: len(b"Radi"), align });
                }
                (Some(b"Strk"), _) => self.warn("gradient outline effects (left out)"),
                (Some(b"BevE" | b"EmbE"), _) => {
                    // No radius, no bevel (as Affinity draws it).
                    let size = len(b"Radi");
                    if size > 0.0 {
                        let white = crate::paint::Color::Rgb { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
                        let black = crate::paint::Color::Rgb { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
                        let highlight = s.obj(e, b"HiCl").and_then(|c| crate::paint::color(self, c)).unwrap_or(white);
                        let shadow = s.obj(e, b"ShCl").and_then(|c| crate::paint::color(self, c)).unwrap_or(black);
                        let shadow_opacity = s.f64(e, b"ShOp").filter(|v| v.is_finite()).unwrap_or(0.75).clamp(0.0, 1.0);
                        let depth = s.f64(e, b"Dept").filter(|v| v.is_finite()).unwrap_or(5.0);
                        let azimuth = s.f64(e, b"Azim").filter(|v| v.is_finite()).unwrap_or(std::f64::consts::FRAC_PI_4 * 3.0);
                        let inner = s.enumeration(e, b"Beve").is_none_or(|(k, _)| k == 0);
                        out.push(Effect::Bevel { size, depth, angle: azimuth, highlight, highlight_opacity: opacity, shadow, shadow_opacity, inner });
                    }
                }
                (Some(b"ColO"), Some(color)) => {
                    let blend = self.effect_blend(e);
                    out.push(Effect::ColorOverlay { color, opacity, blend });
                }
                (Some(b"GrdO"), _) => {
                    let blend = self.effect_blend(e);
                    // The gradient's unit space spans the node's bounding box from -1 to 1, centred
                    // on it (y down): checked against the stored previews of real documents.
                    let space =
                        bounds.map(|b| Affine([(b.x1 - b.x0) / 2.0, 0.0, 0.0, (b.y1 - b.y0) / 2.0, (b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0]));
                    match (s.obj(e, b"GrFl"), space) {
                        (Some(fill), Some(space)) => match paint::descriptor(self, fill, space) {
                            Some(Paint::Gradient(gradient)) => out.push(Effect::GradientOverlay { gradient, opacity, blend }),
                            Some(Paint::Solid(color)) => out.push(Effect::ColorOverlay { color, opacity, blend }),
                            _ => {}
                        },
                        // Nothing visible to lay it over.
                        (Some(_), None) => {}
                        (None, _) => self.warn("gradient overlay effects that could not be read (left out)"),
                    }
                }
                (Some(b"ColO"), None) => self.warn("colour overlay effects that could not be read (left out)"),
                (Some(b"Gaus"), _) => self.warn("blur effects (left out)"),
                _ => self.warn("layer effects of other kinds (left out)"),
            }
        }
        for a in self.objs(id, b"AdCh") {
            if let Some(effect) = self.attached_recolor(a) {
                out.push(effect);
            }
        }
        out
    }
    fn attached_recolor(&mut self, a: ObjId) -> Option<Effect> {
        let s = self.s;
        if !s.is(a, b"RcRA") {
            return None;
        }
        if s.bool(a, b"Visi") == Some(false) {
            return None;
        }
        let adjp = s.obj(a, b"AdjP")?;
        if !s.is(adjp, b"RecP") {
            return None;
        }
        let h = s.f64(adjp, b"RecH").unwrap_or(0.0);
        let sat = s.f64(adjp, b"RecS").unwrap_or(1.0);
        let l = s.f64(adjp, b"RecL").unwrap_or(0.5);
        let (r, g, b) = crate::paint::hsl(h, sat, l);
        let color = crate::paint::Color::Rgb { r, g, b, a: 1.0 };
        let opacity = s.f64(a, b"Opac").filter(|v| v.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
        let blend = self.effect_blend(a);
        Some(Effect::ColorOverlay { color, opacity, blend })
    }

    /// A layer effect's blend mode (`BlnM`); unknown modes are Normal (warned).
    fn effect_blend(&mut self, e: ObjId) -> Blend {
        match self.s.enumeration(e, b"BlnM") {
            None => Blend::Normal,
            Some((i, v)) => Blend::from_enum(i, v).unwrap_or_else(|| {
                self.warn("an unknown blend mode (imported as Normal)");
                Blend::Normal
            }),
        }
    }

    /// Vector shapes attached to a node (`AdCh`) mask it: it shows only inside their outlines;
    /// an attached mask layer is a pixel mask. Adjustments attached the same way are reported.
    fn attached_masks(&mut self, id: ObjId, world: Affine, over: Option<Rect>) -> Result<(Option<Path>, Option<Image>), Error> {
        let s = self.s;
        self.budget.spend(s.array_len(id, b"AdCh").unwrap_or(0))?;
        let attached = s.objs(id, b"AdCh");
        let mut mask: Option<Path> = None;
        let mut pixels: Option<Image> = None;
        for a in attached {
            let class = s.class(a).unwrap_or(Tag(0));
            if s.bool(a, b"Visi") == Some(false) {
                continue;
            }
            if class == Tag::of(b"PCrv") || class == Tag::of(b"ShpN") || class == Tag::of(b"Comp") {
                let local = s.floats::<6>(a, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY).then(world);
                if let Some((p, _)) = crate::geometry::outline(self, a, class, local) {
                    mask.get_or_insert_with(Path::default).subpaths.extend(p.transformed(local).subpaths);
                }
            } else if class == Tag::of(b"MRst") {
                let local = s.floats::<6>(a, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY).then(world);
                match crate::raster::mask(self, a, local, over)? {
                    Some(m) if pixels.is_none() => pixels = Some(m),
                    Some(_) => self.warn("several pixel masks on one layer (only the first is used)"),
                    None => {}
                }
            } else if class == Tag::of(b"RcRA") {
                // Handled in `effects` as a colour overlay.
            } else {
                self.warn(format!("{} layers attached to other layers (imported without them)", describe(class)));
            }
        }
        Ok((mask, pixels))
    }

    /// The content of an Affinity document embedded in this one (`EmbN` whose `EmbC` entry holds
    /// `EmDc`, four more bytes and a complete Affinity document): its first spread's nodes,
    /// placed so the centre of its page, first artboard or content is the node's origin.
    fn embedded_document(&mut self, id: ObjId, world: Affine) -> Option<Vec<Node>> {
        let s = self.s;
        let entry = s.obj(id, b"Bitm").and_then(|b| s.obj(b, b"EmCn")).and_then(|c| s.entry(c, b"EmbC"))?.to_string();
        if entry.is_empty() {
            return None;
        }
        if self.depth >= MAX_EMBED_DEPTH {
            self.warn("embedded documents nested more than four deep");
            return None;
        }
        let data = self.archive.read(&entry).ok()?;
        let inner = data.strip_prefix(b"EmDc").and_then(|d| d.get(4..))?;
        if !crate::is_affinity(inner) {
            return None;
        }
        // A first reading finds its page and resolution, the second places it.
        // Both readings are charged to the budget and the archive's limits, also when they fail:
        // each does the work.
        let mut budget = self.budget;
        let probe = read_at(inner, self.archive.remaining(), &mut budget, Affine::IDENTITY, self.depth + 1);
        self.budget = budget;
        if self.archive.charge(probe.extracted).is_err() {
            self.warn("embedded documents beyond the size limit (left out)");
            return None;
        }
        let probe = match probe.document {
            Ok(d) => d,
            Err(e) => {
                self.warn(format!("embedded documents that could not be read ({e})"));
                return None;
            }
        };
        let spread = probe.spreads.first()?;
        // `PBBx` 2 places the content's own bounds (as with a cached picture); otherwise its page.
        let content = s.obj(id, b"Bitm").and_then(|b| s.enumeration(b, b"PBBx")).is_some_and(|(k, _)| k == 2);
        let first_board = spread.nodes.iter().find_map(|n| if let Kind::Artboard { rect, .. } = &n.kind { Some(*rect) } else { None });
        let page = if content { None } else { spread.pages.first().copied().or(first_board) };
        let page = page.or_else(|| nodes_bounds(&spread.nodes)).unwrap_or(spread.bounds);
        // Its pixels aren't needed any more: never hold both readings' at once.
        drop(probe);
        // Its document pixels are the node's units, whatever either document's resolution.
        let k = 1.0;
        let (cx, cy) = ((page.x0 + page.x1) / 2.0, (page.y0 + page.y1) / 2.0);
        let base = Affine([k, 0.0, 0.0, k, -cx * k, -cy * k]).then(world);
        if !base.is_finite() {
            self.warn("embedded documents with an invalid placement");
            return None;
        }
        let mut budget = self.budget;
        let placed = read_at(inner, self.archive.remaining(), &mut budget, base, self.depth + 1);
        self.budget = budget;
        if self.archive.charge(placed.extracted).is_err() {
            self.warn("embedded documents beyond the size limit (left out)");
            return None;
        }
        let doc = placed.document.ok()?;
        for w in doc.warnings {
            self.warn(format!("{w} [in embedded documents]"));
        }
        if doc.spreads.len() > 1 {
            self.warn("embedded documents with several pages (the first is used)");
        }
        let mut nodes = doc.spreads.into_iter().next()?.nodes;
        // Its master page instances count its own masters, which aren't kept: in this document
        // they would show other masters.
        fn strip_masters(nodes: &mut [Node], depth: usize) -> bool {
            let mut found = false;
            for n in nodes {
                if matches!(n.kind, Kind::MasterInstance { .. }) {
                    n.kind = Kind::Unsupported;
                    found = true;
                }
                if depth < MAX_TREE_DEPTH {
                    found |= strip_masters(&mut n.children, depth + 1);
                }
            }
            found
        }
        if strip_masters(&mut nodes, 0) {
            self.warn("master pages of embedded documents (left out)");
        }
        Some(nodes)
    }

    fn artboard(&mut self, id: ObjId, world: Affine) -> Kind {
        let s = self.s;
        let Some(b) = s.floats::<4>(id, b"ShpB").map(rect).filter(|b| b.x1 > b.x0 && b.y1 > b.y0) else {
            self.warn("an artboard without valid bounds (its children are kept without clipping)");
            return Kind::Unsupported;
        };
        let corners =
            [Point { x: b.x0, y: b.y0 }, Point { x: b.x1, y: b.y0 }, Point { x: b.x1, y: b.y1 }, Point { x: b.x0, y: b.y1 }].map(|p| world.apply(p));
        let [a, b2, c, d, ..] = world.0;
        if b2.abs() > 1e-9 || c.abs() > 1e-9 || a <= 0.0 || d <= 0.0 {
            self.warn("a rotated or flipped artboard (export bounds are its bounding box; artwork keeps its outline)");
        }
        let xs = corners.map(|p| p.x);
        let ys = corners.map(|p| p.y);
        let rect = Rect {
            x0: xs.iter().copied().fold(f64::INFINITY, f64::min),
            y0: ys.iter().copied().fold(f64::INFINITY, f64::min),
            x1: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            y1: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        };
        if ![rect.x0, rect.y0, rect.x1, rect.y1].iter().all(|v| v.is_finite()) || rect.x1 <= rect.x0 || rect.y1 <= rect.y0 {
            self.warn("an artboard with an invalid transformed size (its children are kept without clipping)");
            return Kind::Unsupported;
        }
        let class = s.class(id).unwrap_or(Tag::of(b"ShpN"));
        let (path, even_odd) = crate::geometry::outline(self, id, class, world).unwrap_or_else(|| {
            self.warn("an artboard whose outline could not be read (clipped to its rectangle)");
            (Path { subpaths: vec![crate::shapes::rectangle(b.x0, b.y0, b.x1, b.y1)] }, false)
        });
        let (background, strokes) = paint::node_paint(self, id, world);
        Kind::Artboard { rect, path: path.transformed(world), even_odd, background, strokes }
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn shared_work_is_bounded() {
        let mut b = Budget::FULL;
        assert!(b.spend(MAX_DOCUMENT_WORK).is_ok());
        assert_eq!(b.spend(1), Err(Error::Limit("too much shared geometry or page data")));
    }
}
