//! A book's content documents read into the craft-flow model (feature `doc`).
//!
//! [`read_book`] opens the package, walks the spine and reads each XHTML content document with
//! craft-html's reader, one [`Chapter`] each. Images resolve against the chapter that names them
//! and are read once for the whole book. Bookmarks are made unique across the book by the
//! chapter's [`Chapter::key`], and links between chapters are rewritten to them, so the chapters
//! can be joined into one flow ([`BookFlow::join`]) without one chapter's `#n1` landing in another.
//! What is left out is said in [`BookFlow::warnings`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_flow::{FBlock, Flow, Fmt, Inline, MAX_DEPTH, Meta, Para};

use crate::read::percent_decode;
use crate::{Epub, Error, Item, Limits, Package, TocEntry, dir_of, resolve};

/// How a book is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BookOptions {
    /// The package's limits: every chapter and image is read inside its byte budgets.
    pub limits: Limits,
    /// Most content documents read; the rest of the spine is left out
    /// ([`Warning::ChaptersLeftOut`]). Default 10,000.
    pub max_chapters: usize,
}

impl Default for BookOptions {
    fn default() -> BookOptions {
        BookOptions { limits: Limits::DEFAULT, max_chapters: 10_000 }
    }
}

/// Something [`read_book`] left out or could not keep; the rest of the book is read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// A spine item whose file the archive does not have. When no manifest item has its
    /// `idref`, `path` is the `idref`.
    MissingChapter { path: String },
    /// A spine item that is not an XHTML or HTML document (an image, an SVG). Its manifest
    /// `fallback` is not followed.
    NotXhtml { path: String, media_type: String },
    /// A part the archive has but could not give (a broken entry), or a table of contents that
    /// could not be read.
    Unreadable { path: String, msg: String },
    /// An image (or the cover) the package does not have, or one outside it (`path` is then its
    /// `src`), named in the document at `from`.
    MissingImage { path: String, from: String },
    /// An image (or the cover) larger than [`Limits::max_entry_bytes`], named in the document at
    /// `from`.
    ImageTooLarge { path: String, from: String },
    /// An image in a format the flow cannot hold (an SVG, say: the formats are those
    /// `craft_flow::sniff_image` knows), named in the document at `from`. It is left out.
    UnsupportedImage { path: String, from: String },
    /// The package declares the whole book fixed layout (pre-paginated); it is read as flowing
    /// text and its pages' geometry is lost. `path` is the package document. A spine item's own
    /// `rendition:layout` is not read.
    FixedLayout { path: String },
    /// The spine has more content documents than [`BookOptions::max_chapters`]: they are left
    /// out from `path` on.
    ChaptersLeftOut { path: String },
}

/// One content document of the spine, read.
#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    /// Its archive entry.
    pub path: String,
    /// Its bookmark prefix, unique in the book: `c` and its position among the content documents
    /// planned from the spine (`c1`, `c2`, …), so one that turns out missing leaves a gap. Its
    /// bookmarks are `{key}-{id}`, and a bookmark `{key}` marks its start.
    pub key: String,
    /// `false` for a `linear="no"` spine item (auxiliary content).
    pub linear: bool,
    /// The label of the first table of contents entry that points into it.
    pub title: Option<String>,
    pub flow: Flow,
}

/// A book read into the flow model ([`read_book`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BookFlow {
    /// The package document as read: metadata, manifest and spine.
    pub package: Package,
    pub toc: Vec<TocEntry>,
    /// The cover image's bytes, when the package names one and has it.
    pub cover: Option<Arc<Vec<u8>>>,
    /// In spine order, each content document once.
    pub chapters: Vec<Chapter>,
    /// Grouped by kind, not in spine order.
    pub warnings: Vec<Warning>,
    targets: Targets,
}

/// What [`BookFlow::join`] puts between chapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// Each chapter after the first starts on a new page.
    PageBreak,
    /// Chapters follow on.
    Continuous,
}

impl BookFlow {
    /// Every chapter, in order (non-linear ones too), as one flow with the book's title and
    /// creators. A list that ends one chapter does not run on into a list that starts the next.
    pub fn join(&self, at: Boundary) -> Flow {
        let meta = Meta { title: self.package.title.clone(), author: self.package.creators.join(", "), ..Meta::default() };
        let mut flow = Flow { blocks: Vec::new(), meta };
        let mut lists = 0;
        for (n, chapter) in self.chapters.iter().enumerate() {
            let start = flow.blocks.len();
            flow.blocks.extend(chapter.flow.blocks.iter().cloned());
            if let Some(blocks) = flow.blocks.get_mut(start..) {
                lists = own_lists(blocks, lists);
            }
            if n > 0
                && at == Boundary::PageBreak
                && let Some(FBlock::Para(p)) = flow.blocks.get_mut(start)
            {
                p.page_break = true;
            }
        }
        flow
    }

    /// The bookmark in the chapters' flows that archive entry `path` with `fragment` (as written,
    /// or percent-encoded) names: a [`TocEntry`]'s target, say. The chapter's start when it has
    /// no such bookmark; `None` when `path` is not a chapter that was read. Links in the flows
    /// are rewritten the same way.
    pub fn bookmark(&self, path: &str, fragment: Option<&str>) -> Option<String> {
        self.targets.bookmark(path, fragment)
    }
}

/// Gives the top-level list paragraphs of one chapter's `blocks` list identities above `used`,
/// which no other chapter has, keeping each list together: a list the source numbers
/// ([`craft_flow::ListInfo::list`]) keeps one identity, and each run of unnumbered items, as
/// `craft_flow::to_doc` groups them, gets its own. (Unnumbered items of two chapters that meet
/// would otherwise be one list.) Returns the highest identity given.
fn own_lists(blocks: &mut [FBlock], used: u32) -> u32 {
    let numbered = blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { p.list.map(|l| l.list) } else { None }).max().unwrap_or(0);
    let mut next = used.saturating_add(numbered);
    // The unnumbered run being given an identity: that identity, and whether its top-level items
    // are numbered (a change of kind there starts another list).
    let mut run: Option<(u32, Option<bool>)> = None;
    for block in blocks {
        let Some(li) = (match block {
            FBlock::Para(p) => p.list.as_mut(),
            FBlock::Table(_) => None,
        }) else {
            run = None;
            continue;
        };
        if li.list != 0 {
            li.list = li.list.saturating_add(used);
            run = None;
            continue;
        }
        let kind_changes = li.level == 0 && run.is_some_and(|(_, top)| top.is_some_and(|t| t != li.ordered));
        let id = match run {
            Some((id, _)) if !kind_changes => id,
            _ => {
                next = next.saturating_add(1);
                next
            }
        };
        let top = if li.level == 0 { Some(li.ordered) } else { run.and_then(|(_, t)| t) };
        run = Some((id, top));
        li.list = id;
    }
    next
}

/// Each chapter read, by archive entry: its key and its bookmarks as written.
#[derive(Clone, Debug, Default, PartialEq)]
struct Targets(HashMap<String, (String, HashSet<String>)>);

impl Targets {
    fn bookmark(&self, path: &str, fragment: Option<&str>) -> Option<String> {
        let (key, anchors) = self.0.get(path)?;
        let found = fragment.and_then(|f| {
            if anchors.contains(f) {
                return Some(f.to_string());
            }
            Some(percent_decode(f)).filter(|d| anchors.contains(d))
        });
        Some(match found {
            Some(name) => format!("{key}-{name}"),
            None => key.clone(),
        })
    }
}

/// Whether `media_type` is a font's.
fn is_font(media_type: &str) -> bool {
    let m = media_type.trim().to_ascii_lowercase();
    m.starts_with("font/") || m.starts_with("application/font-") || m.starts_with("application/x-font-") || m == "application/vnd.ms-opentype"
}

/// Reads the EPUB package `bytes` and its content documents.
///
/// Refused: what [`Epub::open_with`] refuses, a package with anything encrypted but fonts the
/// manifest lists as fonts under a font obfuscation algorithm ([`Error::Encrypted`]), a chapter
/// larger than [`Limits::max_entry_bytes`], and parts past the total byte budget
/// ([`Error::TooLarge`]). A package whose `encryption.xml` lists more than the crate reads of
/// it (10,000 records) is refused too, as [`Error::Encrypted`] with that file's path: what is
/// past the limit is not known to be fonts. A missing or broken chapter, a missing, oversized or
/// unsupported image, a spine
/// item that is not XHTML and a fixed-layout book are [`Warning`]s. Links into documents that
/// were not read (missing, not XHTML, past [`BookOptions::max_chapters`]) are left as written.
pub fn read_book(bytes: &[u8], opts: &BookOptions) -> Result<BookFlow, Error> {
    craft_flow::begin_import();
    let mut epub = Epub::open_with(bytes, opts.limits)?;
    let (encrypted, cut) = epub.encryption_telling()?;
    if cut {
        return Err(Error::Encrypted { path: "META-INF/encryption.xml".into() });
    }
    for e in encrypted {
        let font = epub.package.manifest.iter().find(|i| i.path == e.path).is_some_and(|i| is_font(&i.media_type));
        if !(font && e.is_font_obfuscation()) {
            return Err(Error::Encrypted { path: e.path });
        }
    }
    let mut warnings = Vec::new();
    if epub.package.pre_paginated {
        warnings.push(Warning::FixedLayout { path: epub.opf_path.clone() });
    }
    // The table of contents as `Epub::toc` reads it, but with the navigation document's bytes
    // kept: when it is also in the spine it is not read (and charged to the budget) twice.
    let mut nav: Option<(String, Vec<u8>)> = None;
    let mut toc = Vec::new();
    if let Some(path) = epub.package.nav().map(|i| i.path.clone())
        && let Some(Some(xml)) = soft(epub.read(&path), &mut warnings)?
    {
        toc = soft(crate::parse_nav(&xml, &path), &mut warnings)?.unwrap_or_default();
        nav = Some((path, xml));
    }
    if toc.is_empty()
        && let Some(path) = epub.package.ncx.clone()
        && let Some(Some(xml)) = soft(epub.read(&path), &mut warnings)?
    {
        toc = soft(crate::parse_ncx(&xml, &path), &mut warnings)?.unwrap_or_default();
    }
    let package = std::mem::take(&mut epub.package);
    let opf_path = epub.opf_path.clone();

    // The spine's content documents, each once, in the order they first appear.
    let mut by_id: HashMap<&str, &Item> = HashMap::new();
    for item in &package.manifest {
        by_id.entry(&item.id).or_insert(item);
    }
    let mut seen = HashSet::new();
    let mut planned: Vec<(&str, bool)> = Vec::new();
    for s in &package.spine {
        let Some(item) = by_id.get(s.idref.as_str()) else {
            warnings.push(Warning::MissingChapter { path: s.idref.clone() });
            continue;
        };
        if !seen.insert(item.path.as_str()) {
            continue;
        }
        let media_type = item.media_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if !matches!(media_type.as_str(), "application/xhtml+xml" | "text/html") {
            warnings.push(Warning::NotXhtml { path: item.path.clone(), media_type: item.media_type.clone() });
            continue;
        }
        if planned.len() == opts.max_chapters {
            warnings.push(Warning::ChaptersLeftOut { path: item.path.clone() });
            break;
        }
        planned.push((&item.path, s.linear));
    }
    let mut nav = nav.filter(|(path, _)| planned.iter().any(|(p, _)| p == path));

    let images = Images {
        epub: RefCell::new(epub),
        cache: RefCell::default(),
        unsupported: RefCell::default(),
        warnings: RefCell::default(),
        error: RefCell::default(),
    };
    let cover = match package.cover_image() {
        Some(item) => images.get(&item.path, &opf_path)?,
        None => None,
    };
    let mut titles: HashMap<&str, &str> = HashMap::new();
    for entry in &toc {
        if let Some(path) = &entry.path {
            titles.entry(path).or_insert(&entry.label);
        }
    }

    let mut chapters = Vec::new();
    let mut targets = Targets::default();
    for (n, (path, linear)) in planned.into_iter().enumerate() {
        let read = match nav.take_if(|(p, _)| p == path) {
            Some((_, bytes)) => Read::Bytes(bytes),
            None => images.read(path)?,
        };
        let bytes = match read {
            Read::Bytes(bytes) => bytes,
            Read::Missing => {
                warnings.push(Warning::MissingChapter { path: path.to_string() });
                continue;
            }
            Read::Broken(msg) => {
                warnings.push(Warning::Unreadable { path: path.to_string(), msg });
                continue;
            }
            Read::TooLarge(e) => return Err(e),
        };
        let text = craft_html::doc::decode(&bytes);
        drop(bytes);
        let load = |src: &str| images.load(path, src);
        let mut flow = craft_html::doc::parse_with(&text, &load);
        if let Some(e) = images.error.try_borrow_mut().ok().and_then(|mut e| e.take()) {
            return Err(e);
        }
        let key = format!("c{}", n + 1);
        targets.0.insert(path.to_string(), (key.clone(), prefix_anchors(&mut flow.blocks, &key)));
        match flow.blocks.first_mut() {
            Some(FBlock::Para(p)) => p.inlines.insert(0, Inline::Anchor(key.clone())),
            _ => flow.blocks.insert(0, FBlock::Para(Para { inlines: vec![Inline::Anchor(key.clone())], ..Para::default() })),
        }
        let title = titles.get(path).map(|t| t.to_string());
        chapters.push(Chapter { path: path.to_string(), key, linear, title, flow });
    }
    warnings.extend(images.warnings.take());

    for chapter in &mut chapters {
        let from = chapter.path.as_str();
        each_inline(&mut chapter.flow.blocks, 0, &mut |i| {
            if let Some(fmt) = fmt_of(i)
                && let Some(link) = fmt.link.as_deref().and_then(|l| target(l, from, &targets))
            {
                fmt.link = Some(link.into());
            }
        });
    }
    Ok(BookFlow { package, toc, cover, chapters, warnings, targets })
}

/// `result`, with a broken part (`Error::Part`) turned into a warning and `None`; other errors
/// are the book's.
fn soft<T>(result: Result<T, Error>, warnings: &mut Vec<Warning>) -> Result<Option<T>, Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::Part { part, msg }) => {
            warnings.push(Warning::Unreadable { path: part, msg });
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// An archive entry, read.
enum Read {
    Bytes(Vec<u8>),
    Missing,
    /// There but broken: it costs the book only that part.
    Broken(String),
    /// Larger than [`Limits::max_entry_bytes`].
    TooLarge(Error),
}

/// The package while its chapters are read, with the images read so far: craft-html's image
/// loader is a `Fn`, so the reads go through cells.
struct Images<'a> {
    epub: RefCell<Epub<'a>>,
    /// By archive entry (or `src`, for one outside the package); `None` for one not read.
    cache: RefCell<HashMap<String, Option<Arc<Vec<u8>>>>>,
    /// Images read whose format the flow cannot hold, already said.
    unsupported: RefCell<HashSet<String>>,
    warnings: RefCell<Vec<Warning>>,
    /// A spent budget, met while craft-html was loading an image: the book's error.
    error: RefCell<Option<Error>>,
}

impl Images<'_> {
    /// Archive entry `path`; a spent total budget is the book's error.
    fn read(&self, path: &str) -> Result<Read, Error> {
        let Ok(mut epub) = self.epub.try_borrow_mut() else { return Ok(Read::Broken("the package is being read".into())) };
        match epub.read_telling(path) {
            Ok(Some(bytes)) => Ok(Read::Bytes(bytes)),
            Ok(None) => Ok(Read::Missing),
            Err((Error::Part { msg, .. }, _)) => Ok(Read::Broken(msg)),
            Err((e, true)) => Ok(Read::TooLarge(e)),
            Err((e, false)) => Err(e),
        }
    }

    fn warn(&self, warning: Warning) {
        if let Ok(mut w) = self.warnings.try_borrow_mut() {
            w.push(warning);
        }
    }

    fn remember(&self, key: &str, image: Option<Arc<Vec<u8>>>) {
        if let Ok(mut cache) = self.cache.try_borrow_mut() {
            cache.insert(key.to_string(), image);
        }
    }

    fn cached(&self, key: &str) -> Option<Option<Arc<Vec<u8>>>> {
        self.cache.try_borrow().ok().and_then(|c| c.get(key).cloned())
    }

    /// The image at archive entry `path`, named in the document at `from`, read once.
    fn get(&self, path: &str, from: &str) -> Result<Option<Arc<Vec<u8>>>, Error> {
        if let Some(cached) = self.cached(path) {
            return Ok(cached);
        }
        let (path, from) = (path.to_string(), from.to_string());
        let image = match self.read(&path)? {
            Read::Bytes(bytes) => Some(Arc::new(bytes)),
            Read::Missing | Read::Broken(_) => {
                self.warn(Warning::MissingImage { path: path.clone(), from });
                None
            }
            Read::TooLarge(_) => {
                self.warn(Warning::ImageTooLarge { path: path.clone(), from });
                None
            }
        };
        self.remember(&path, image.clone());
        Ok(image)
    }

    /// craft-html's image loader for the chapter at `from`: `src` as written there.
    fn load(&self, from: &str, src: &str) -> Option<Arc<Vec<u8>>> {
        if self.error.try_borrow().map_or(true, |e| e.is_some()) {
            return None;
        }
        let path = if has_scheme(src) { String::new() } else { resolve(dir_of(from), src) };
        if path.is_empty() {
            // Outside the package (a remote resource), or naming nothing: said once per `src`.
            if self.cached(src).is_none() {
                self.warn(Warning::MissingImage { path: src.to_string(), from: from.to_string() });
                self.remember(src, None);
            }
            return None;
        }
        let image = self.get(&path, from).unwrap_or_else(|e| {
            if let Ok(mut error) = self.error.try_borrow_mut() {
                *error = Some(e);
            }
            None
        })?;
        // craft-html would drop it without a word (`make_img` takes only what this knows); the
        // cover keeps its bytes whatever their format.
        if craft_flow::sniff_image(&image).is_none() {
            if self.unsupported.try_borrow_mut().is_ok_and(|mut said| said.insert(path.clone())) {
                self.warn(Warning::UnsupportedImage { path, from: from.to_string() });
            }
            return None;
        }
        Some(image)
    }
}

/// Whether `href` starts with a URI scheme (`http:`, `mailto:`): it leaves the package.
fn has_scheme(href: &str) -> bool {
    href.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Renames the bookmarks of `blocks` to `{key}-{name}`; their names as written.
fn prefix_anchors(blocks: &mut [FBlock], key: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    each_inline(blocks, 0, &mut |i| {
        if let Inline::Anchor(name) | Inline::AnchorEnd(name) = i {
            *name = format!("{key}-{}", std::mem::take(name));
            if let Some(original) = name.get(key.len() + 1..) {
                names.insert(original.to_string());
            }
        }
    });
    names
}

/// The bookmark link (`#c2-n3`) a link written in the chapter at `from` becomes, when it points
/// into a chapter of the book ([`Targets::bookmark`]). `None` for a link that leaves the book or
/// points at a part of the package that was not read as a chapter.
fn target(link: &str, from: &str, chapters: &Targets) -> Option<String> {
    if has_scheme(link) {
        return None;
    }
    let (file, fragment) = link.split_once('#').map_or((link, None), |(file, fragment)| (file, Some(fragment)));
    let path = if file.split('?').next().unwrap_or("").is_empty() { from.to_string() } else { resolve(dir_of(from), file) };
    chapters.bookmark(&path, fragment).map(|name| format!("#{name}"))
}

/// The formatting, and so the link, an inline carries.
fn fmt_of(inline: &mut Inline) -> Option<&mut Fmt> {
    match inline {
        Inline::Text(_, fmt) | Inline::Ruby { fmt, .. } | Inline::Field { fmt, .. } | Inline::Control { fmt, .. } | Inline::Note { fmt, .. } => {
            Some(fmt)
        }
        _ => None,
    }
}

/// Calls `f` on every inline of `blocks`: in paragraphs, table cells, notes, comments, text
/// boxes and groups, to the flow's nesting limit.
fn each_inline(blocks: &mut [FBlock], depth: usize, f: &mut dyn FnMut(&mut Inline)) {
    if depth > MAX_DEPTH {
        return;
    }
    for block in blocks {
        match block {
            FBlock::Para(p) => {
                for i in &mut p.inlines {
                    walk_inline(i, depth, f);
                }
            }
            FBlock::Table(t) => {
                for cell in t.rows.iter_mut().flatten() {
                    each_inline(&mut cell.blocks, depth + 1, f);
                }
            }
        }
    }
}

fn walk_inline(i: &mut Inline, depth: usize, f: &mut dyn FnMut(&mut Inline)) {
    f(i);
    match i {
        Inline::Note { blocks, .. } | Inline::Shape { blocks, .. } => each_inline(blocks, depth + 1, f),
        Inline::Comment { comment, .. } => each_inline(&mut comment.blocks, depth + 1, f),
        Inline::Group { members, .. } if depth < MAX_DEPTH => {
            for m in members {
                walk_inline(&mut m.inline, depth + 1, f);
            }
        }
        _ => {}
    }
}
