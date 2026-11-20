//! Reading the pages of a PDF document.

use crate::content::{TypedIter, UntypedIter};
use crate::object::Array;
use crate::object::Dict;
use crate::object::Name;
use crate::object::Rect;
use crate::object::Stream;
use crate::object::dict::keys::*;
use crate::object::{MaybeRef, Object, ObjectLike};
use crate::object::array::ArrayIter;
use crate::reader::ReaderContext;
use crate::sync::OnceLock;
use crate::transform::Transform;
use crate::util::FloatExt;
use crate::xref::XRef;
use crate::filter::{FlateState, MAX_DECODED_STREAM};
use crate::object::stream::Incremental;
use alloc::borrow::Cow;
use alloc::boxed::Box;
use crate::object::ObjectIdentifier;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Deref;

/// Attributes that can be inherited.
#[derive(Debug, Clone)]
struct PagesContext {
    media_box: Option<Rect>,
    crop_box: Option<Rect>,
    rotate: Option<i32>,
}

impl PagesContext {
    fn new() -> Self {
        Self {
            media_box: None,
            crop_box: None,
            rotate: None,
        }
    }
}

/// A structure holding the pages of a PDF document.
pub struct Pages<'a> {
    pages: Vec<Page<'a>>,
    xref: &'a XRef,
    /// PdfCraft patch (59): the walk of the page tree stepped over some `/Kids` entry that is not
    /// a dictionary (a reference to a missing object, say): the tree names pages this walk did
    /// not reach, so an empty result is no sign that it has none.
    incomplete: bool,
}

impl<'a> Pages<'a> {
    /// Create a new `Pages` object within a bounded amount of page-tree work.
    fn new(
        pages_dict: &Dict<'a>,
        ctx: &ReaderContext<'a>,
        xref: &'a XRef,
        work_limit: usize,
    ) -> Result<Self, PageTreeError> {
        let mut pages = vec![];
        let mut incomplete = false;
        resolve_pages(pages_dict, &mut pages, ctx, work_limit, &mut incomplete)?;

        Ok(Self { pages, xref, incomplete })
    }

    /// Create a new `Pages` object by bruteforce-searching.
    ///
    /// Of course this could result in the order of pages being messed up, but
    /// this is still better than nothing.
    pub(crate) fn new_brute_force(ctx: &ReaderContext<'a>, xref: &'a XRef) -> Option<Self> {
        Self::scan(ctx, xref, false)
    }

    /// PdfCraft patch (59): as [`Self::new_brute_force`], taking only dictionaries that say they
    /// are pages (`/Type /Page`, ISO 32000-2 §7.7.3.3 Table 31) and have content.
    /// `new_brute_force` takes any dictionary with a `/Contents` entry, which annotations have
    /// too (a text string, Table 166); this scan runs on files whose page tree leads to no page,
    /// with nothing to check what it finds against, so it takes pages only.
    pub(crate) fn new_brute_force_typed(ctx: &ReaderContext<'a>, xref: &'a XRef) -> Option<Self> {
        Self::scan(ctx, xref, true)
    }

    /// PdfCraft patch (59): the page dictionaries found by scanning, each with what it inherits
    /// through its `/Parent` entries (see [`inherited_through_parents`]).
    fn scan(ctx: &ReaderContext<'a>, xref: &'a XRef, typed: bool) -> Option<Self> {
        crate::cancel::check(ctx.control())?;
        let mut pages = vec![];
        let mut known = BTreeMap::new();

        for object in xref.objects() {
            crate::cancel::check(ctx.control())?;
            // `Page::new` takes no dictionary without `/Contents` here; that is checked first,
            // so what other dictionaries would inherit is never looked up.
            if let Some(dict) = object.into_dict()
                && dict.contains_key(CONTENTS)
                && (!typed || dict.get::<Name<'_>>(TYPE).as_deref() == Some(PAGE))
                && let Some(page) = {
                    let (inherited, resources) = inherited_through_parents(&dict, ctx, &mut known);
                    Page::new(&dict, &inherited, resources, true)
                }
            {
                pages.push(page);
            }
        }

        crate::cancel::check(ctx.control())?;
        if pages.is_empty() {
            return None;
        }

        Some(Self { pages, xref, incomplete: false })
    }

    /// Return the xref table (of the document the pages belong to).   
    pub fn xref(&self) -> &'a XRef {
        self.xref
    }
}

impl<'a> Deref for Pages<'a> {
    type Target = [Page<'a>];

    fn deref(&self) -> &Self::Target {
        &self.pages
    }
}

/// Craft patch: bound traversal work, not valid indirect page-tree depth. The root
/// and every raw `/Kids` entry consume one unit, including malformed, direct and
/// repeated entries. This also bounds the stack, visited set and returned pages.
/// Budget exhaustion or failed frame/page reservation rejects the whole traversal;
/// it must not trigger the unbounded brute-force page-repair fallback.
const MAX_PAGE_TREE_WORK: usize = 100_000;

#[derive(Debug, PartialEq, Eq)]
enum PageTreeError {
    Malformed,
    Limit,
    Cancelled,
}

struct PageTreeFrame<'a> {
    kids: ArrayIter<'a>,
    ctx: PagesContext,
    resources: Resources<'a>,
}

impl<'a> PageTreeFrame<'a> {
    fn new(dict: &Dict<'a>, mut ctx: PagesContext, resources: Resources<'a>) -> Option<Self> {
        ctx.media_box = dict.get::<Rect>(MEDIA_BOX).or(ctx.media_box);
        ctx.crop_box = dict.get::<Rect>(CROP_BOX).or(ctx.crop_box);
        ctx.rotate = dict.get::<i32>(ROTATE).or(ctx.rotate);
        let resources = Resources::from_scope(dict.get::<Dict<'a>>(RESOURCES), resources);

        Some(Self { kids: dict.get::<Array<'a>>(KIDS)?.raw_iter(), ctx, resources })
    }
}

/// PdfCraft patch: the largest `/UserUnit` honoured, as in Acrobat (a larger value counts as
/// this one).
const MAX_USER_UNIT: f64 = 75_000.0;

/// PdfCraft patch: whether a `/Kids` entry is an intermediate node of the page tree. A node is
/// `/Type /Pages`, or has a `/Kids` array and is not `/Type /Page`: ISO 32000-2 §7.7.3.2
/// requires both keys of a node, but writers leave `/Type` out, and a page has no `/Kids`
/// (§7.7.3.3), so the array tells them apart. A `/Kids` that is null (the same as absent,
/// §7.3.9) or not an array leaves an untyped dictionary a page.
fn is_page_tree_node(dict: &Dict<'_>) -> bool {
    match dict.get::<Name<'_>>(TYPE).as_deref() {
        Some(PAGES) => true,
        Some(PAGE) => false,
        _ => dict.get::<Array<'_>>(KIDS).is_some(),
    }
}

fn resolve_pages<'a>(
    pages_dict: &Dict<'a>,
    entries: &mut Vec<Page<'a>>,
    reader_ctx: &ReaderContext<'a>,
    work_limit: usize,
    incomplete: &mut bool,
) -> Result<(), PageTreeError> {
    crate::cancel::check(reader_ctx.control()).ok_or(PageTreeError::Cancelled)?;
    let mut remaining = work_limit.checked_sub(1).ok_or(PageTreeError::Limit)?;
    let root =
        PageTreeFrame::new(pages_dict, PagesContext::new(), Resources::new(Dict::empty(), None, reader_ctx)).ok_or(PageTreeError::Malformed)?;
    let mut visited = BTreeSet::new();
    if let Some(id) = pages_dict.obj_id() {
        visited.insert(id);
    }
    let mut stack = Vec::new();
    stack.try_reserve(1).map_err(|_| PageTreeError::Limit)?;
    stack.push(root);

    while let Some(frame) = stack.last_mut() {
        crate::cancel::check(reader_ctx.control()).ok_or(PageTreeError::Cancelled)?;
        let Some(kid) = frame.kids.next() else {
            stack.pop();
            continue;
        };
        remaining = remaining.checked_sub(1).ok_or(PageTreeError::Limit)?;

        let dict = match kid {
            MaybeRef::Ref(id) => {
                // Check raw references before resolution: resolving a cyclic or
                // invalid reference through a typed iterator can end that iterator
                // and hide all later siblings. Leaves are deduplicated as well.
                if !visited.insert(id.into()) {
                    continue;
                }
                // Resolve each indirect child with the original document context,
                // preserving decode limits without accumulating an ancestor chain.
                reader_ctx.xref().get_with::<Dict<'a>>(id.into(), reader_ctx)
            }
            MaybeRef::NotRef(object) => object.into_dict(),
        };
        let Some(dict) = dict else {
            // PdfCraft patch (59): an entry that is no dictionary may stand for pages this walk
            // cannot reach. One that refers back to an entry already walked (an ancestor, read as
            // a cycle, or a repeated leaf) was stepped over above and leads to no page missed.
            *incomplete = true;
            continue;
        };

        if is_page_tree_node(&dict) {
            if let Some(child) = PageTreeFrame::new(&dict, frame.ctx.clone(), frame.resources.clone()) {
                stack.try_reserve(1).map_err(|_| PageTreeError::Limit)?;
                stack.push(child);
            }
        } else if let Some(page) = Page::new(&dict, &frame.ctx, frame.resources.clone(), false) {
            // Let's be lenient and assume it's a `Page` in case it's `None` or something else
            // (see corpus test case 0083781).
            entries.try_reserve(1).map_err(|_| PageTreeError::Limit)?;
            entries.push(page);
        }
    }

    crate::cancel::check(reader_ctx.control()).ok_or(PageTreeError::Cancelled)?;
    Ok(())
}

/// PdfCraft patch (59): the most `/Parent` links [`inherited_through_parents`] follows up from one
/// page, as deep as the page-tree walk is tested to go.
const MAX_SCANNED_PAGE_PARENTS: usize = 1024;

/// PdfCraft patch (59): what a page found by scanning inherits from the page-tree nodes its
/// `/Parent` entries lead to (ISO 32000-2 §7.7.3.3 Table 31 makes `/Parent` required, and §7.7.3.4
/// has `/Resources`, `/MediaBox`, `/CropBox` and `/Rotate` inherited from those ancestors), applied
/// from the root down as [`resolve_pages`] applies them. A page whose tree leads no reader to it
/// (the root's only kid is the root) still names its parent, and takes its size and fonts from
/// there. The walk up stops at a node already seen, at one that is not a dictionary, and after
/// `MAX_SCANNED_PAGE_PARENTS` nodes.
///
/// `known` holds what each indirect node passes on, as worked out for an earlier page of the same
/// scan, so the pages under one node look it up once and the scan reads each node about once.
fn inherited_through_parents<'a>(
    page: &Dict<'a>,
    ctx: &ReaderContext<'a>,
    known: &mut BTreeMap<ObjectIdentifier, (PagesContext, Resources<'a>)>,
) -> (PagesContext, Resources<'a>) {
    let mut chain: Vec<Dict<'a>> = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(id) = page.obj_id() {
        seen.insert(id);
    }
    let mut base = None;
    let mut next = page.get::<Dict<'a>>(PARENT);
    while let Some(node) = next {
        if chain.len() >= MAX_SCANNED_PAGE_PARENTS {
            break;
        }
        if let Some(id) = node.obj_id() {
            if let Some(found) = known.get(&id) {
                base = Some(found.clone());
                break;
            }
            if !seen.insert(id) {
                break;
            }
        }
        next = node.get::<Dict<'a>>(PARENT);
        chain.push(node);
    }
    let (mut inherited, mut resources) =
        base.unwrap_or_else(|| (PagesContext::new(), Resources::new(Dict::empty(), None, ctx)));
    for node in chain.iter().rev() {
        inherited.media_box = node.get::<Rect>(MEDIA_BOX).or(inherited.media_box);
        inherited.crop_box = node.get::<Rect>(CROP_BOX).or(inherited.crop_box);
        inherited.rotate = node.get::<i32>(ROTATE).or(inherited.rotate);
        resources = Resources::from_scope(node.get::<Dict<'a>>(RESOURCES), resources);
        if let Some(id) = node.obj_id() {
            known.insert(id, (inherited.clone(), resources.clone()));
        }
    }
    (inherited, resources)
}

/// The rotation of the page.
#[derive(Debug, Copy, Clone)]
pub enum Rotation {
    /// No rotation.
    None,
    /// A rotation of 90 degrees.
    Horizontal,
    /// A rotation of 180 degrees.
    Flipped,
    /// A rotation of 270 degrees.
    FlippedHorizontal,
}

/// PdfCraft patch: the cut-off for content that expands from its compressed streams. The decoded
/// content of a page may be `MAX_CONTENT_EXPANSION` times the size of those streams, and never less
/// than `MAX_DECODED_STREAM`. Real content expands a few times, and a stream that inflates a
/// thousandfold from a few kilobytes is a decompression bomb.
const MAX_CONTENT_EXPANSION: usize = 64;

/// PdfCraft patch: the decoded content of a page, produced a piece at a time, so that the whole of
/// it need not be in memory at once (see `interpret_page`). Plain streams and Flate streams without
/// a predictor are decoded as they are read. Any other stream is decoded whole when the reader is
/// made, as `page_stream` does, and the decoded streams share one `MAX_DECODED_STREAM`.
///
/// The decoded bytes of the page's streams are also cut off at a budget: `MAX_CONTENT_EXPANSION`
/// times their compressed size, counting each distinct stream once however many times `/Contents`
/// names it. Content that reaches the budget is cut short, and [`ContentReader::truncated`] says so.
pub struct ContentReader<'a> {
    pieces: VecDeque<Piece<'a>>,
    budget: usize,
    produced: usize,
    materialized: usize,
    truncated: bool,
}

/// One part of the content of a page, in order.
enum Piece<'a> {
    /// A stream's own bytes, which are not decoded.
    Plain { data: Cow<'a, [u8]>, pos: usize },
    /// A Flate stream, decoded as it is read.
    Flate { data: Cow<'a, [u8]>, state: FlateState },
    /// A stream that was decoded whole.
    Whole { data: Vec<u8>, pos: usize },
    /// The space that separates the streams of a `/Contents` array.
    Separator { written: bool },
}

impl Piece<'_> {
    fn exhausted(&self) -> bool {
        match self {
            Piece::Plain { data, pos } => *pos >= data.len(),
            Piece::Flate { data, state } => state.finished(data),
            Piece::Whole { data, pos } => *pos >= data.len(),
            Piece::Separator { written } => *written,
        }
    }

    fn incomplete(&self) -> bool {
        matches!(self, Piece::Flate { state, .. } if !state.complete())
    }

    /// Appends the next bytes of the piece to `out`, which may end at `limit` bytes.
    fn append_to(&mut self, out: &mut Vec<u8>, limit: usize) {
        match self {
            Piece::Plain { data, pos } => copy_from(data, pos, out, limit),
            Piece::Flate { data, state } => state.decode_into(data, out, limit),
            Piece::Whole { data, pos } => copy_from(data, pos, out, limit),
            Piece::Separator { written } => {
                out.push(b' ');
                *written = true;
            }
        }
    }
}

/// Copies the bytes of `data` from `pos` to `out`, up to `limit` bytes in all.
fn copy_from(data: &[u8], pos: &mut usize, out: &mut Vec<u8>, limit: usize) {
    let rest = data.get(*pos..).unwrap_or(&[]);
    let take = rest.len().min(limit.saturating_sub(out.len()));
    out.extend_from_slice(rest.get(..take).unwrap_or(&[]));
    *pos += take;
}

impl<'a> ContentReader<'a> {
    /// A reader of no content.
    fn empty() -> Self {
        Self {
            pieces: VecDeque::new(),
            budget: 0,
            produced: 0,
            materialized: 0,
            truncated: false,
        }
    }

    /// A reader of `streams`, the content of a page in order. `separated` says whether they come
    /// from an array, whose streams are separated by spaces. `cap` is the most bytes that streams
    /// decoded whole may make in all, and at least the budget's floor.
    fn new(streams: Vec<Stream<'a>>, separated: bool, cap: usize) -> Self {
        let mut counted = BTreeSet::new();
        let mut compressed = 0_usize;
        let mut pieces = VecDeque::new();
        let mut materialized = 0_usize;
        // The bytes that the streams decoded whole take up, separators included, as `page_stream`
        // counted them against `MAX_DECODED_STREAM`.
        let mut whole_used = 0_usize;
        let mut accepting_pieces = true;
        let mut truncated = false;
        for stream in streams {
            let newly_counted = counted.insert(stream.obj_id());
            if !accepting_pieces {
                truncated = true;
                if newly_counted {
                    compressed = compressed.saturating_add(stream.raw_data().len());
                }
                continue;
            }

            let incremental = stream.incremental();
            let left = cap.saturating_sub(whole_used);
            if matches!(&incremental, Incremental::Whole) && left == 0 {
                accepting_pieces = false;
                truncated = true;
                if newly_counted {
                    compressed = compressed.saturating_add(stream.raw_data().len());
                }
                continue;
            }

            let raw = stream.raw_data();
            if newly_counted {
                compressed = compressed.saturating_add(raw.len());
            }

            match incremental {
                Incremental::Plain => pieces.push_back(Piece::Plain {
                    data: raw,
                    pos: 0,
                }),
                Incremental::Flate => {
                    let state = FlateState::new(&raw);
                    pieces.push_back(Piece::Flate { data: raw, state });
                }
                Incremental::Whole => {
                    let limit = left.min(stream.dict().ctx().decode_limits().max_decoded_stream());
                    let Ok(data) = stream.decoded_raw_within(raw, limit) else {
                        truncated = true;
                        continue;
                    };
                    let mut data = data.into_owned();
                    // Whole-stream decoders may stop at the requested limit without exposing
                    // completion. Treat reaching it as incomplete rather than certifying a prefix.
                    if data.len() >= limit {
                        truncated = true;
                        data.truncate(limit);
                    }
                    whole_used = whole_used.saturating_add(data.len());
                    materialized = materialized.saturating_add(data.len());
                    pieces.push_back(Piece::Whole { data, pos: 0 });
                }
            }

            if separated {
                whole_used = whole_used.saturating_add(1);
                pieces.push_back(Piece::Separator { written: false });
            }
        }
        let budget = cap.max(MAX_CONTENT_EXPANSION.saturating_mul(compressed));

        Self {
            pieces,
            budget,
            produced: 0,
            materialized,
            truncated,
        }
    }

    /// Appends up to `max` bytes of the decoded content to `out`, and returns how many it appended.
    /// That is fewer than `max` only at the end of the content, or where it is cut short.
    pub fn read_into(&mut self, out: &mut Vec<u8>, max: usize) -> usize {
        let start = out.len();
        let end = start.saturating_add(max);

        while out.len() < end {
            let Some(piece) = self.pieces.front_mut() else {
                break;
            };
            if piece.exhausted() {
                self.truncated |= piece.incomplete();
                self.pieces.pop_front();
                continue;
            }

            let allowed = self.budget.saturating_sub(self.produced);
            if allowed == 0 {
                self.truncated = true;
                self.pieces.clear();
                break;
            }

            let before = out.len();
            piece.append_to(out, end.min(before.saturating_add(allowed)));
            self.produced += out.len() - before;
        }

        out.len() - start
    }

    /// The bytes of the content that were decoded whole when the reader was made. The caller
    /// charges these against its own budget, as it did for the whole content.
    pub fn materialized_len(&self) -> usize {
        self.materialized
    }

    /// Whether a stream was omitted, failed to decode, or reached a content budget.
    /// This reports known incomplete input; it does not certify rendering fidelity.
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// A PDF page.
pub struct Page<'a> {
    inner: Dict<'a>,
    media_box: Rect,
    crop_box: Rect,
    rotation: Rotation,
    user_unit: f32,
    page_streams: OnceLock<Option<Vec<u8>>>,
    resources: Resources<'a>,
    ctx: ReaderContext<'a>,
}

impl<'a> Page<'a> {
    fn new(
        dict: &Dict<'a>,
        ctx: &PagesContext,
        resources: Resources<'a>,
        brute_force: bool,
    ) -> Option<Self> {
        // In general, pages without content are allowed, but in case we are brute-forcing
        // we ignore them.
        if brute_force && !dict.contains_key(CONTENTS) {
            return None;
        }

        // PdfCraft patch (38): a box that encloses no area is no box. A `/MediaBox` without
        // area (`[0 0 0 0]`) is read as a missing one, and a `/CropBox` that has none, or
        // none in common with the media box, as a missing one (the media box, ISO 32000-2
        // §14.11.2, Table 33). Kept, the page's visible region had no area: nothing was drawn
        // and no text was extracted, where other readers show the page.
        // A page's own box without area gives way to one it inherits that has some. Each box
        // set aside is logged, as other repairs of this reader are.
        // PdfCraft patch (44): a page with no `/MediaBox`, its own or inherited, or only ones
        // without area, is US Letter, not A4. The box is required (ISO 32000-2 §7.7.3.3,
        // Table 31) and has no default; other readers use Letter, as do PdfCraft's page model
        // and inspector. The page is then sized, placed and clipped by the same Letter region
        // and its content shows.
        let own_media = dict.get::<Rect>(MEDIA_BOX);
        let media_box = own_media
            .filter(has_area)
            .or(ctx.media_box.filter(has_area))
            .unwrap_or(LETTER);
        if own_media.or(ctx.media_box).is_some_and(|m| !has_area(&m)) {
            warn!("a /MediaBox without area was read as missing");
        }

        let shows = |c: &Rect| has_area(&c.intersect(media_box));
        let own_crop = dict.get::<Rect>(CROP_BOX);
        let crop_box = own_crop
            .filter(shows)
            .or(ctx.crop_box.filter(shows))
            .unwrap_or(media_box);
        if own_crop.or(ctx.crop_box).is_some_and(|c| !shows(&c)) {
            warn!("a /CropBox with no area in the media box was read as missing");
        }

        let rotation = match dict
            .get::<i32>(ROTATE)
            .or(ctx.rotate)
            .unwrap_or(0)
            .rem_euclid(360)
        {
            0 => Rotation::None,
            90 => Rotation::Horizontal,
            180 => Rotation::Flipped,
            270 => Rotation::FlippedHorizontal,
            _ => Rotation::None,
        };

        // PdfCraft patch: the page's own `/UserUnit` (it isn't inherited), read as Acrobat reads
        // it: a number from 1 up, at most `MAX_USER_UNIT`; anything else counts as 1.
        let user_unit = dict
            .get::<f64>(USER_UNIT)
            .filter(|u| *u >= 1.0)
            .map_or(1.0, |u| u.min(MAX_USER_UNIT) as f32);

        let ctx = resources.ctx.clone();
        let resources = Resources::from_scope(
            dict.get::<Dict<'_>>(RESOURCES),
            resources,
        );

        Some(Self {
            inner: dict.clone(),
            media_box,
            crop_box,
            rotation,
            user_unit,
            page_streams: OnceLock::new(),
            resources,
            ctx,
        })
    }

    fn operations_impl(&self) -> Option<UntypedIter<'_>> {
        let stream = self.page_stream()?;
        let iter = UntypedIter::new(stream);

        Some(iter)
    }

    /// PdfCraft patch: the content of the page, to be read a piece at a time (see
    /// [`ContentReader`]). Bare Flate streams use the separate incremental content budget,
    /// even when the document's ordinary materialization limit is lower. Consequently this
    /// can yield more bytes than [`Page::page_stream`] for the same stream. Whole-stream
    /// fallback decoders retain the document's ordinary limit.
    pub fn content_reader(&self) -> ContentReader<'a> {
        self.content_reader_within(MAX_DECODED_STREAM)
    }

    /// PdfCraft patch: [`Page::content_reader`], with `cap` in place of `MAX_DECODED_STREAM`.
    fn content_reader_within(&self, cap: usize) -> ContentReader<'a> {
        if let Some(stream) = self.inner.get::<Stream<'a>>(CONTENTS) {
            ContentReader::new(vec![stream], false, cap)
        } else if let Some(array) = self.inner.get::<Array<'a>>(CONTENTS) {
            ContentReader::new(array.iter::<Stream<'a>>().collect(), true, cap)
        } else {
            warn!("contents entry of page was neither stream nor array of streams");

            ContentReader::empty()
        }
    }

    /// Return the decoded content stream of the page.
    pub fn page_stream(&self) -> Option<&[u8]> {
        let convert_single = |s: Stream<'_>| {
            let data = s.decoded().ok()?;
            // PdfCraft patch: take the decoded buffer instead of copying it (up to
            // `MAX_DECODED_STREAM` twice over).
            Some(data.into_owned())
        };

        self.page_streams
            .get_or_init(|| {
                if let Some(stream) = self.inner.get::<Stream<'_>>(CONTENTS) {
                    convert_single(stream)
                } else if let Some(array) = self.inner.get::<Array<'_>>(CONTENTS) {
                    let mut collected = vec![];

                    for stream in array.iter::<Stream<'_>>() {
                        // PdfCraft patch: the streams share one `MAX_DECODED_STREAM` (an array
                        // naming one inflating stream twenty times decoded 5 GB).
                        let left = MAX_DECODED_STREAM.saturating_sub(collected.len());
                        if left == 0 {
                            break;
                        }
                        let Ok(data) = stream.decoded_within(left) else {
                            continue;
                        };
                        // Exactly what this stream and its separator need: growing by doubling
                        // here held 1 GB for a 256 MiB stream.
                        collected.reserve_exact(data.len() + 1);
                        collected.extend_from_slice(&data);
                        // Streams must have at least one whitespace in-between.
                        collected.push(b' ');
                    }

                    Some(collected)
                } else {
                    warn!("contents entry of page was neither stream nor array of streams");

                    None
                }
            })
            .as_ref()
            .map(|d| d.as_slice())
    }

    /// Get the resources of the page.
    pub fn resources(&self) -> &Resources<'a> {
        &self.resources
    }

    /// Get the media box of the page.
    pub fn media_box(&self) -> Rect {
        self.media_box
    }

    /// Get the rotation of the page.
    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// Get the crop box of the page.
    pub fn crop_box(&self) -> Rect {
        self.crop_box
    }

    /// PdfCraft patch: the size of the page's user-space unit in points (`/UserUnit`).
    pub fn user_unit(&self) -> f32 {
        self.user_unit
    }

    /// Return the intersection of crop box and media box.
    pub fn intersected_crop_box(&self) -> Rect {
        self.crop_box().intersect(self.media_box())
    }

    /// Return the base dimensions of the page (same as `intersected_crop_box`, but with special
    /// handling applied for zero-area pages).
    pub fn base_dimensions(&self) -> (f32, f32) {
        let crop_box = self.intersected_crop_box();

        if (crop_box.width() as f32).is_nearly_zero() || (crop_box.height() as f32).is_nearly_zero()
        {
            // PdfCraft patch (44): the same US Letter default as a missing `/MediaBox`.
            (LETTER.width() as f32, LETTER.height() as f32)
        } else {
            // PdfCraft patch: retain a one-point floor after UserUnit, not one raw unit.
            // The stored unit is finite and within 1..=75000. Round upward only if the
            // reciprocal's f32 product would fall below one and trigger a placeholder.
            let mut raw_floor = 1.0 / self.user_unit;
            if raw_floor * self.user_unit < 1.0 {
                raw_floor = raw_floor.next_up();
            }
            let raw_floor = f64::from(raw_floor);
            (
                crop_box.width().max(raw_floor) as f32,
                crop_box.height().max(raw_floor) as f32,
            )
        }
    }

    /// Return the with and height of the page that should be assumed when rendering the page.
    ///
    /// Depending on the document, it is either based on the media box or the crop box
    /// of the page. In addition to that, it also takes the rotation of the page into account.
    pub fn render_dimensions(&self) -> (f32, f32) {
        let (base_width, base_height) = self.base_dimensions();
        // PdfCraft patch: in points, after `/UserUnit`.
        let (mut base_width, mut base_height) =
            (base_width * self.user_unit, base_height * self.user_unit);

        if matches!(
            self.rotation(),
            Rotation::Horizontal | Rotation::FlippedHorizontal
        ) {
            core::mem::swap(&mut base_width, &mut base_height);
        }

        (base_width, base_height)
    }

    /// Return an untyped iterator over the operators of the page's content stream.
    pub fn operations(&self) -> UntypedIter<'_> {
        self.operations_impl().unwrap_or(UntypedIter::empty())
    }

    /// Get the raw dictionary of the page.
    pub fn raw(&self) -> &Dict<'a> {
        &self.inner
    }

    /// Get the xref table (of the document the page belongs to).
    pub fn xref(&self) -> &'a XRef {
        self.ctx.xref()
    }

    /// Return a typed iterator over the operators of the page's content stream.
    pub fn typed_operations(&self) -> TypedIter<'_> {
        TypedIter::from_untyped(self.operations())
    }

    /// Return the initial transform that should be applied when rendering.
    ///
    /// This accounts for the mismatch between PDF's y-up and most renderers'
    /// y-down coordinate system, the rotation of the page and the offset of
    /// the crop box.
    pub fn initial_transform(&self, invert_y: bool) -> Transform {
        let crop_box = self.intersected_crop_box();
        let (_, base_height) = self.base_dimensions();
        // PdfCraft patch: the page in points, after `/UserUnit`.
        let base_height = base_height * self.user_unit;
        let (width, height) = self.render_dimensions();

        let horizontal_t = Transform::ROTATE_CW_90 * Transform::translate((0.0, -width as f64));
        let flipped_horizontal_t =
            Transform::translate((0.0, height as f64)) * Transform::ROTATE_CCW_90;

        let rotation_transform = match self.rotation() {
            Rotation::None => Transform::IDENTITY,
            Rotation::Horizontal => {
                if invert_y {
                    horizontal_t
                } else {
                    flipped_horizontal_t
                }
            }
            Rotation::Flipped => {
                Transform::scale(-1.0) * Transform::translate((-width as f64, -height as f64))
            }
            Rotation::FlippedHorizontal => {
                if invert_y {
                    flipped_horizontal_t
                } else {
                    horizontal_t
                }
            }
        };

        let inversion_transform = if invert_y {
            Transform::new([1.0, 0.0, 0.0, -1.0, 0.0, base_height as f64])
        } else {
            Transform::IDENTITY
        };

        let user_space = Transform::translate((-crop_box.x0, -crop_box.y0));
        // PdfCraft patch: user space in units of `/UserUnit` points. A page without it keeps
        // exactly the transform it had.
        let user_space = if self.user_unit == 1.0 {
            user_space
        } else {
            Transform::scale(f64::from(self.user_unit)) * user_space
        };

        rotation_transform * inversion_transform * user_space
    }
}

/// A structure keeping track of the resources of a page.
#[derive(Clone, Debug)]
pub struct Resources<'a> {
    parent: Option<Box<Self>>,
    ctx: ReaderContext<'a>,
    // PdfCraft patch: legacy forms without Resources use the page, not their caller.
    page_properties: Dict<'a>,
    /// The raw dictionary of external graphics states.
    pub ext_g_states: Dict<'a>,
    /// The raw dictionary of fonts.
    pub fonts: Dict<'a>,
    /// The effective property dictionary selected at page or form entry.
    pub properties: Dict<'a>,
    /// The raw dictionary of color spaces.
    pub color_spaces: Dict<'a>,
    /// The raw dictionary of x objects.
    pub x_objects: Dict<'a>,
    /// The raw dictionary of patterns.
    pub patterns: Dict<'a>,
    /// The raw dictionary of shadings.
    pub shadings: Dict<'a>,
}

impl<'a> Resources<'a> {
    /// Use an object's entire resource dictionary, or inherit the enclosing scope when absent.
    ///
    /// An explicit dictionary, including an empty one, replaces the enclosing resources;
    /// missing names do not fall back individually. The enclosing reader context is retained
    /// so indirect resources keep the document's cross-references and decode limits.
    pub fn from_scope(resources: Option<Dict<'a>>, parent: Self) -> Self {
        match resources {
            Some(resources) => Self::new(resources, None, &parent.ctx),
            None => parent,
        }
    }

    /// Create a new `Resources` object from a dictionary with a parent.
    pub fn from_parent(resources: Dict<'a>, parent: Self) -> Self {
        let ctx = parent.ctx.clone();

        Self::new(resources, Some(parent), &ctx)
    }

    // PdfCraft patch: page-tree inheritance stops at any present Resources dictionary.
    fn for_page(resources: Option<Dict<'a>>, parent: Self) -> Self {
        let properties = resources.as_ref().map_or_else(|| parent.properties.clone(), |dict| dict.get::<Dict<'_>>(PROPERTIES).unwrap_or_default());
        let mut result = Self::from_parent(resources.unwrap_or_default(), parent);
        result.properties = properties.clone();
        result.page_properties = properties;
        result
    }

    /// Create form resources, using page Properties only when Resources is absent.
    /// Other resource categories retain their existing parent-chain lookup.
    // PdfCraft patch: ISO 32000-2, 7.8.3 and Table 93 require the page origin.
    pub fn for_form(resources: Option<Dict<'a>>, parent: Self) -> Self {
        let properties =
            resources.as_ref().map_or_else(|| parent.page_properties.clone(), |dict| dict.get::<Dict<'_>>(PROPERTIES).unwrap_or_default());
        let mut result = Self::from_parent(resources.unwrap_or_default(), parent);
        result.properties = properties;
        result
    }

    /// Create a new `Resources` object.
    pub(crate) fn new(resources: Dict<'a>, parent: Option<Self>, ctx: &ReaderContext<'a>) -> Self {
        let ext_g_states = resources.get::<Dict<'_>>(EXT_G_STATE).unwrap_or_default();
        let fonts = resources.get::<Dict<'_>>(FONT).unwrap_or_default();
        let color_spaces = resources.get::<Dict<'_>>(COLORSPACE).unwrap_or_default();
        let x_objects = resources.get::<Dict<'_>>(XOBJECT).unwrap_or_default();
        let patterns = resources.get::<Dict<'_>>(PATTERN).unwrap_or_default();
        let shadings = resources.get::<Dict<'_>>(SHADING).unwrap_or_default();
        let properties = resources.get::<Dict<'_>>(PROPERTIES).unwrap_or_default();

        // PdfCraft patch: generic child resources retain the page origin.
        let page_properties = parent.as_ref().map_or_else(|| properties.clone(), |parent| parent.page_properties.clone());
        let parent = parent.map(Box::new);

        Self {
            parent,
            ext_g_states,
            fonts,
            color_spaces,
            properties,
            page_properties,
            x_objects,
            patterns,
            shadings,
            ctx: ctx.clone(),
        }
    }

    fn get_resource<T: ObjectLike<'a>>(&self, name: &Name<'_>, dict: &Dict<'a>) -> Option<T> {
        dict.get::<T>(name.deref())
    }

    /// Get the parent in the resource, chain, if available.
    pub fn parent(&self) -> Option<&Self> {
        self.parent.as_deref()
    }

    /// Get an external graphics state by name.
    pub fn get_ext_g_state(&self, name: &Name<'_>) -> Option<Dict<'a>> {
        self.get_resource::<Dict<'_>>(name, &self.ext_g_states)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_ext_g_state(name)))
    }

    /// Get a color space by name.
    pub fn get_color_space(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.get_resource::<Object<'_>>(name, &self.color_spaces)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_color_space(name)))
    }

    /// Get a font by name.
    pub fn get_font(&self, name: &Name<'_>) -> Option<Dict<'a>> {
        self.get_resource::<Dict<'_>>(name, &self.fonts)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_font(name)))
    }

    /// Get a pattern by name.
    pub fn get_pattern(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.get_resource::<Object<'_>>(name, &self.patterns)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_pattern(name)))
    }

    /// Get an x object by name.
    pub fn get_x_object(&self, name: &Name<'_>) -> Option<Stream<'a>> {
        self.get_resource::<Stream<'_>>(name, &self.x_objects)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_x_object(name)))
    }

    /// Get a shading by name.
    pub fn get_shading(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.get_resource::<Object<'_>>(name, &self.shadings)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_shading(name)))
    }
}

// <https://github.com/apache/pdfbox/blob/a53a70db16ea3133994120bcf1e216b9e760c05b/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/common/PDRectangle.java#L38>
const POINTS_PER_INCH: f64 = 72.0;
const POINTS_PER_MM: f64 = 1.0 / (10.0 * 2.54) * POINTS_PER_INCH;

/// PdfCraft patch (38): whether `r` encloses some area: finite, and wider and taller than
/// nothing.
fn has_area(r: &Rect) -> bool {
    let (w, h) = (r.width() as f32, r.height() as f32);
    w.is_finite()
        && h.is_finite()
        && w > 0.0
        && h > 0.0
        && !w.is_nearly_zero()
        && !h.is_nearly_zero()
}

/// The dimension of an A4 page.
pub const A4: Rect = Rect {
    x0: 0.0,
    y0: 0.0,
    x1: 210.0 * POINTS_PER_MM,
    y1: 297.0 * POINTS_PER_MM,
};

/// PdfCraft patch (44): the dimension of a US Letter page (8.5 × 11 in), the size of a page
/// that has no `/MediaBox`.
pub const LETTER: Rect = Rect {
    x0: 0.0,
    y0: 0.0,
    x1: 8.5 * POINTS_PER_INCH,
    y1: 11.0 * POINTS_PER_INCH,
};

pub(crate) mod cached {
    use crate::page::{MAX_PAGE_TREE_WORK, PageTreeError, Pages};
    use crate::reader::ReaderContext;
    use crate::xref::XRef;
    use core::ops::Deref;

    // Keep in sync with the implementation in `sync`. We duplicate it here
    // to make it more visible since we have unsafe code here.
    #[cfg(feature = "std")]
    pub(crate) use std::sync::Arc;

    #[cfg(not(feature = "std"))]
    pub(crate) use alloc::rc::Rc as Arc;

    pub(crate) struct CachedPages {
        pages: Pages<'static>,
        // NOTE: `pages` references the data in `xref`, so it's important that `xref`
        // appears after `pages` in the struct definition to ensure correct drop order.
        _xref: Arc<XRef>,
    }

    impl CachedPages {
        pub(crate) fn new(xref: Arc<XRef>) -> Option<Self> {
            Self::build(xref, MAX_PAGE_TREE_WORK, false)
        }

        /// PdfCraft patch (59): as [`Self::new`], and a page tree that leads to no page (its
        /// `/Kids` are empty or loop back to itself) gives way to the page dictionaries
        /// `Pages::new_brute_force_typed` finds, as one that cannot be read at all does. ISO
        /// 32000-2 §7.7.3.1 has every page reached through the tree; a root whose only kid is
        /// itself lost the one page that names it as its `/Parent`. When the scan finds no page
        /// either, the empty tree stands.
        ///
        /// Not when the walk stepped over a `/Kids` entry it could not read: that tree names
        /// pages the walk did not reach. Not used while trying a table rebuilt over a cut-off
        /// update (patch 43), whose empty tree means the older table is read instead.
        pub(crate) fn new_or_scan(xref: Arc<XRef>) -> Option<Self> {
            Self::build(xref, MAX_PAGE_TREE_WORK, true)
        }

        pub(super) fn new_with_limit(xref: Arc<XRef>, work_limit: usize) -> Option<Self> {
            Self::build(xref, work_limit, false)
        }

        fn build(xref: Arc<XRef>, work_limit: usize, scan_if_empty: bool) -> Option<Self> {
            // SAFETY:
            // - The XRef's location is stable in memory:
            //   - We wrapped it in a `Arc` (or `Rc` in `no_std`), which implements `StableDeref`.
            //   - The struct owns the `Arc`, ensuring that the inner value is not dropped during the whole
            //     duration.
            // - The internal 'static lifetime is not leaked because its rewritten
            //   to the self-lifetime in `pages()`.
            let xref_reference: &'static XRef = unsafe { core::mem::transmute(xref.deref()) };

            let ctx = ReaderContext::new(xref_reference, false);
            let pages = match xref_reference.get_with(xref.trailer_data().pages_ref, &ctx) {
                Some(p) => match Pages::new(&p, &ctx, xref_reference, work_limit) {
                    Ok(pages) => pages,
                    Err(PageTreeError::Malformed) => Pages::new_brute_force(&ctx, xref_reference)?,
                    // A limit is a failed load, never permission to bypass the bound
                    // or return a partially walked page tree through repair.
                    Err(PageTreeError::Limit | PageTreeError::Cancelled) => return None,
                },
                None => Pages::new_brute_force(&ctx, xref_reference)?,
            };
            let pages = if scan_if_empty && pages.is_empty() && !pages.incomplete {
                match Pages::new_brute_force_typed(&ctx, xref_reference) {
                    Some(found) => {
                        warn!("the page tree leads to no page; using the {} page objects found by scanning", found.len());
                        found
                    }
                    None => {
                        // A cancelled scan fails the load, as a cancelled walk does.
                        crate::cancel::check(ctx.control())?;
                        pages
                    }
                }
            } else {
                pages
            };

            Some(Self { pages, _xref: xref })
        }

        pub(crate) fn get(&self) -> &Pages<'_> {
            &self.pages
        }
    }
}

#[cfg(test)]
mod page_tree_tests {
    use super::{MAX_PAGE_TREE_WORK, Rotation, cached};
    use crate::object::{FromBytes, Name};
    use crate::{DecodeLimits, Pdf};
    use core::fmt::Write;

    // Real xref offsets keep repair from masking page-tree traversal failures.
    fn pdf_bytes(objects: &[String]) -> Vec<u8> {
        let mut data = String::from("%PDF-1.7\n");
        let mut offsets = vec![0];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(data.len());
            writeln!(data, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let xref = data.len();
        writeln!(data, "xref\n0 {}\n0000000000 65535 f ", offsets.len()).unwrap();
        for offset in offsets.iter().skip(1) {
            writeln!(data, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(data, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF", offsets.len()).unwrap();
        data.into_bytes()
    }

    fn cached_with_limit(objects: &[String], limit: usize) -> Option<cached::CachedPages> {
        let (xref, _) = crate::xref::root_xref(pdf_bytes(objects).into(), b"", DecodeLimits::default()).unwrap();
        cached::CachedPages::new_with_limit(cached::Arc::new(xref), limit)
    }

    #[test]
    fn cancellation_interrupts_initial_page_tree_construction() {
        use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let armed = alloc::sync::Arc::new(AtomicBool::new(false));
        let polls = alloc::sync::Arc::new(AtomicUsize::new(0));
        let flag = armed.clone();
        let counter = polls.clone();
        let cancellation = crate::Cancellation::new(move || {
            flag.load(Ordering::Relaxed) && counter.fetch_add(1, Ordering::Relaxed) + 1 == 100
        });
        let control = crate::cancel::Control::new(&cancellation);
        let kids = (3..1003).map(|i| format!("{i} 0 R ")).collect::<String>();
        let mut objects = vec![
            String::from("<< /Type /Catalog /Pages 2 0 R >>"),
            format!("<< /Type /Pages /Kids [{kids}] /Count 1000 >>"),
        ];
        objects.extend((0..1000).map(|_| String::from("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>")));
        let (xref, _) = crate::xref::root_xref_controlled(pdf_bytes(&objects).into(), b"", DecodeLimits::default(), Some(control)).unwrap();
        armed.store(true, Ordering::Relaxed);
        assert!(cached::CachedPages::new(cached::Arc::new(xref)).is_none());
        assert_eq!(polls.load(Ordering::Relaxed), 100);
        assert_eq!(cached_with_limit(&objects, 100_000).unwrap().get().len(), 1000);
    }

    fn assert_deep_tree() {
        let depth = 1024;
        let font_id = depth + 3;
        let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
        for level in 0..depth {
            let attributes = if level == 0 {
                "/MediaBox [0 0 100 200] /CropBox [2 3 90 180] /Rotate 90 /Resources << /Font << /Old << /BaseFont /Helvetica >> >> /ExtGState << /G << /CA 0.5 >> >> >>".to_string()
            } else if level == depth / 2 {
                format!("/MediaBox [0 0 300 400] /Rotate 180 /Resources << /Font << /New {font_id} 0 R >> >>")
            } else {
                String::new()
            };
            objects.push(format!("<< /Type /Pages /Kids [{} 0 R] /Count 1 {attributes} >>", level + 3));
        }
        objects.push(format!("<< /Type /Page /Parent {} 0 R >>", depth + 1));
        objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string());
        let limits = DecodeLimits::default().with_max_decoded_stream(31);
        let pdf = Pdf::new_with_limits(pdf_bytes(&objects), limits).unwrap();
        assert_eq!(pdf.pages().len(), 1);
        let page = &pdf.pages()[0];
        assert_eq!(page.raw().obj_id().unwrap().obj_number, (depth + 2) as i32);
        let media = page.media_box();
        assert_eq!((media.x0, media.y0, media.x1, media.y1), (0.0, 0.0, 300.0, 400.0));
        let crop = page.crop_box();
        assert_eq!((crop.x0, crop.y0, crop.x1, crop.y1), (2.0, 3.0, 90.0, 180.0));
        assert!(matches!(page.rotation(), Rotation::Flipped));
        let old = Name::from_bytes(b"/Old").unwrap();
        let new = Name::from_bytes(b"/New").unwrap();
        let state = Name::from_bytes(b"/G").unwrap();
        assert!(page.resources().get_font(&old).is_none());
        assert!(page.resources().get_ext_g_state(&state).is_none());
        assert_eq!(page.resources().get_font(&new).unwrap().get::<Name<'_>>(b"BaseFont").unwrap().as_ref(), b"Courier");
        assert_eq!(page.ctx.decode_limits().max_decoded_stream(), 31);
    }

    #[test]
    fn deeply_nested_indirect_page_tree_preserves_inheritance() {
        assert_deep_tree();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new().stack_size(1 << 20).spawn(assert_deep_tree).unwrap().join().unwrap();
    }

    #[test]
    fn cycles_invalid_children_and_duplicates_keep_later_siblings_in_order() {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R 3 0 R 7 0 R 5 0 R] /Count 4 /MediaBox [0 0 200 300] /Rotate 90 >>",
            "<< /Type /Pages /Kids [2 0 R 999 0 R null 5 0 R 6 0 R] /Rotate 180 >>",
            "<< /Type /Page /MediaBox [0 0 40 40] >>",
            "<< /Type /Page /MediaBox [0 0 50 50] >>",
            "<< /Type /Pages /Kids [3 0 R 8 0 R] /Rotate 270 >>",
            "<< /Type /Page /MediaBox [0 0 70 70] >>",
            "<< /Type /Page /MediaBox [0 0 80 80] >>",
        ]
        .map(str::to_string);
        let pages = cached_with_limit(&objects, MAX_PAGE_TREE_WORK).unwrap();
        let ids: Vec<_> = pages.get().iter().map(|page| page.raw().obj_id().unwrap().obj_number).collect();
        assert_eq!(ids, [5, 8, 4, 7]);
        assert!(matches!(pages.get()[0].rotation(), Rotation::Flipped));
        assert!(matches!(pages.get()[1].rotation(), Rotation::FlippedHorizontal));
        assert!(matches!(pages.get()[2].rotation(), Rotation::Horizontal));
        assert!(matches!(pages.get()[3].rotation(), Rotation::Horizontal));
    }

    #[test]
    fn work_limit_counts_root_and_every_entry_and_never_repairs_partial_pages() {
        for kids in ["3 0 R 3 0 R", "999 0 R 3 0 R", "null 3 0 R", "7 << /Type /Page /MediaBox [0 0 40 40] /Contents [] >>"] {
            let objects = [
                "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
                format!("<< /Type /Pages /Kids [{kids}] /Count 1 >>"),
                // The fallback would find this page even after an exhausted walk.
                "<< /Type /Page /MediaBox [0 0 40 40] /Contents [] >>".to_string(),
            ];
            for limit in 0..=2 {
                assert!(cached_with_limit(&objects, limit).is_none(), "{kids}: limit {limit} must fail closed");
            }
            assert_eq!(cached_with_limit(&objects, 3).unwrap().get().len(), 1, "{kids}");
        }
    }

    #[test]
    fn malformed_root_keeps_existing_brute_force_repair() {
        let objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Count 1 >>", "<< /Type /Page /MediaBox [0 0 40 40] /Contents [] >>"]
            .map(str::to_string);
        let repaired = cached_with_limit(&objects, 1).unwrap();
        assert_eq!(repaired.get().len(), 1);
        assert_eq!(repaired.get()[0].raw().obj_id().unwrap().obj_number, 3);
    }

    /// PdfCraft patch (storytold/pdfcraft#6498): intermediate nodes without `/Type` are told from
    /// pages by their `/Kids` array (ISO 32000-2 §7.7.3.2, §7.7.3.3), so the pages under them are
    /// read in order with what they inherit, and an untyped leaf is still a page.
    #[test]
    fn pages_under_nodes_without_a_type_are_read_in_order() {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 19 0 R] /Count 5 /MediaBox [0 0 612 792] >>",
            // Node 3: no /Type, its own box.
            "<< /Parent 2 0 R /Kids [4 0 R 9 0 R] /Count 4 /MediaBox [0 0 300 200] >>",
            // Node 4: no /Type, its /Kids reached through two references.
            "<< /Parent 3 0 R /Kids 16 0 R /Count 3 >>",
            "null",
            "<< /Type /Page /Parent 4 0 R /Contents [] >>",
            // A `/Type /Page` given by reference, with a stray empty `/Kids`, is a page.
            "<< /Parent 4 0 R /Type 18 0 R /Kids [] /Contents [] >>",
            "<< /Type /Page /Parent 4 0 R /Contents [] >>",
            // No /Type and a null /Kids (the same as no /Kids, §7.3.9): a page.
            "<< /Parent 3 0 R /Kids null /Contents [] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Contents [] >>",
            "null",
            "null",
            "null",
            "null",
            "null",
            "17 0 R",
            "[6 0 R 7 0 R 8 0 R]",
            "/Page",
            "10 0 R",
        ]
        .map(str::to_string);
        let pdf = Pdf::new(pdf_bytes(&objects)).unwrap();
        let pages = pdf.pages();
        let ids: Vec<_> = pages.iter().map(|page| page.raw().obj_id().unwrap().obj_number).collect();
        assert_eq!(ids, [6, 7, 8, 9, 10]);
        let sizes: Vec<_> = pages.iter().map(|page| page.base_dimensions()).collect();
        assert_eq!(sizes, [(300.0, 200.0), (300.0, 200.0), (300.0, 200.0), (300.0, 200.0), (400.0, 400.0)]);
    }

    /// PdfCraft patch (51, storytold/pdfcraft#7391): a page dictionary holding a stray delimiter
    /// between its entries keeps its page, the entries after the byte and the pages after it in
    /// the tree (ISO 32000-2 §7.3.7, §7.7.3), also when the byte is in a nested dictionary.
    #[test]
    fn a_stray_delimiter_in_a_page_dictionary_keeps_the_page_and_the_pages_after_it() {
        let font = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";
        for stray in [")", "]", "{", "}", ">", ") ] )"] {
            for nested in [false, true] {
                let mut objects = vec![
                    "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
                    "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R] /Count 4 >>".to_string(),
                ];
                for page in 0..4 {
                    let (junk, resources) = match (page, nested) {
                        (1, false) => (stray, format!("<< {font} >>")),
                        (1, true) => ("", format!("<< {stray} {font} >>")),
                        _ => ("", format!("<< {font} >>")),
                    };
                    objects.push(format!(
                        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Resources {resources} /Contents [] {junk} /Rotate 90 >>"
                    ));
                }
                let pdf = Pdf::new(pdf_bytes(&objects)).unwrap();
                let pages = pdf.pages();
                let ids: Vec<_> = pages.iter().map(|page| page.raw().obj_id().unwrap().obj_number).collect();
                assert_eq!(ids, [3, 4, 5, 6], "{stray:?}, nested {nested}");
                let second = &pages[1];
                assert!(matches!(second.rotation(), Rotation::Horizontal), "{stray:?}, nested {nested}: the entry after the byte");
                let f1 = Name::from_bytes(b"/F1").unwrap();
                assert!(second.resources().get_font(&f1).is_some(), "{stray:?}, nested {nested}: the page keeps its font");
            }
        }
    }

    const HELLO: &str = "BT /F1 24 Tf 20 100 Td (Hello) Tj ET";

    /// Objects 1-6: the catalog, a root whose `/Kids` are `kids` and which carries the media box
    /// and the font, a page under it, the font, the page's content, and a note (an annotation
    /// with a `/Contents` text string, which is no page).
    fn tree_leading_to(kids: &str) -> Vec<String> {
        vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!("<< /Type /Pages /Kids [{kids}] /Count 1 /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> >>"),
            "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!("<< /Length {} >>\nstream\n{HELLO}\nendstream", HELLO.len()),
            "<< /Type /Annot /Subtype /Text /Rect [0 0 20 20] /Contents (a note) >>".to_string(),
        ]
    }

    /// PdfCraft patch (59, storytold/pdfcraft#7500): a page tree that leads to no page (its root's
    /// only kid is the root, or its `/Kids` are empty) gives way to the page dictionaries found by
    /// scanning, with what they inherit through `/Parent` (ISO 32000-2 §7.7.3.1-7.7.3.4). The note
    /// is not taken for a page although it has `/Contents`.
    #[test]
    fn a_page_tree_leading_to_no_page_gives_way_to_the_pages_found_by_scanning() {
        let f1 = Name::from_bytes(b"/F1").unwrap();
        for kids in ["2 0 R", ""] {
            let pdf = Pdf::new(pdf_bytes(&tree_leading_to(kids))).unwrap();
            let pages = pdf.pages();
            let ids: Vec<_> = pages.iter().map(|page| page.raw().obj_id().unwrap().obj_number).collect();
            assert_eq!(ids, [3], "{kids:?}: the page, not the note");
            assert_eq!(pages[0].base_dimensions(), (300.0, 200.0), "{kids:?}: the media box of its /Parent");
            assert!(pages[0].resources().get_font(&f1).is_some(), "{kids:?}: the font of its /Parent");
            assert_eq!(pages[0].page_stream(), Some(HELLO.as_bytes()), "{kids:?}");
        }
    }

    /// PdfCraft patch (59): a walk that stepped over a kid it could not read (object 9 is missing)
    /// has not shown that the tree leads to no page, so the scan does not stand in for it: the
    /// orphan page 8 is never one of the pages.
    #[test]
    fn a_tree_with_a_missing_kid_is_not_replaced_by_the_scan() {
        for (kids, expected) in [("9 0 R", &[][..]), ("9 0 R 3 0 R", &[3][..]), ("null", &[][..])] {
            let mut objects = tree_leading_to(kids);
            objects.push("<< /Length 0 >>\nstream\n\nendstream".to_string());
            objects.push("<< /Type /Page /MediaBox [0 0 500 500] /Contents 7 0 R >>".to_string());
            let pdf = Pdf::new(pdf_bytes(&objects)).unwrap();
            let ids: Vec<_> = pdf.pages().iter().map(|page| page.raw().obj_id().unwrap().obj_number).collect();
            assert_eq!(ids, expected, "{kids:?}");
        }
    }

    /// PdfCraft patch (59): pages found because the root cannot be read at all (it has no `/Kids`)
    /// inherit through `/Parent` too, and the pages under one node share what it passes on, also
    /// when the chain of parents loops.
    #[test]
    fn pages_found_by_scanning_inherit_through_their_parents() {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Count 3 /MediaBox [0 0 300 200] /Rotate 90 >>",
            "<< /Type /Pages /Parent 2 0 R /MediaBox [0 0 100 50] >>",
            "<< /Type /Page /Parent 3 0 R /Contents [] >>",
            "<< /Type /Page /Parent 3 0 R /Contents [] >>",
            "<< /Type /Page /Parent 2 0 R /Contents [] >>",
            // Its parents loop back to each other.
            "<< /Type /Page /Parent 8 0 R /Contents [] >>",
            "<< /Type /Pages /Parent 9 0 R /MediaBox [0 0 70 70] >>",
            "<< /Type /Pages /Parent 8 0 R /Rotate 180 >>",
        ]
        .map(str::to_string);
        let pdf = Pdf::new(pdf_bytes(&objects)).unwrap();
        let pages = pdf.pages();
        let found: Vec<_> = pages
            .iter()
            .map(|page| (page.raw().obj_id().unwrap().obj_number, page.base_dimensions(), page.rotation() as u8))
            .collect();
        let (horizontal, flipped) = (Rotation::Horizontal as u8, Rotation::Flipped as u8);
        assert_eq!(
            found,
            [
                (4, (100.0, 50.0), horizontal),
                (5, (100.0, 50.0), horizontal),
                (6, (300.0, 200.0), horizontal),
                (7, (70.0, 70.0), flipped)
            ]
        );
    }

    /// PdfCraft patch (48, storytold/pdfcraft#7608): a reference whose generation number does not
    /// fit 32 bits still names its object (a generation is at most 65,535, ISO 32000-2 §7.5.4, so
    /// the object number is what identifies it), so a page's content streams named that way are
    /// part of its content, in a `/Contents` array or alone (§7.3.10).
    #[test]
    fn a_content_stream_named_by_a_generation_past_32_bits_is_read() {
        for (header, contents) in [
            ("4285904753", "[4 0 R 5 4285904753 R]"),
            ("4285904753", "5 4285904753 R"),
            ("0", "[4 0 R 5 4285904753 R]"),
            ("18446744073709551616", "[4 0 R 5 18446744073709551616 R]"),
        ] {
            // No table, so the objects are found by scanning the file.
            let pdf = format!(
                "%PDF-1.4\n\
                 1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
                 2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
                 3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents {contents} >> endobj\n\
                 4 0 obj << /Length 8 >> stream\n1 0 0 rg\nendstream endobj\n\
                 5 {header} obj << /Length 25 >> stream\n0 0 0 rg 0 0 200 100 re f\nendstream endobj\n\
                 trailer << /Root 1 0 R >>\n\
                 %%EOF\n"
            );
            let pdf = Pdf::new(pdf.into_bytes()).unwrap();
            let content = pdf.pages()[0].page_stream().map(<[u8]>::to_vec).unwrap_or_default();
            let content = String::from_utf8_lossy(&content);
            assert!(content.contains("0 0 0 rg 0 0 200 100 re f"), "{header} / {contents}: {content:?}");
            assert_eq!(content.contains("1 0 0 rg"), contents.starts_with('['), "{header} / {contents}: {content:?}");
        }
    }

    fn raw_crop_unit_page(unit: f64, crop: [f64; 4]) -> Vec<u8> {
        let bounds = crop.map(|n| n.to_string()).join(" ");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [{bounds}] /CropBox [{bounds}] /UserUnit {unit} /Resources << >> /Contents [] >>"),
        ];
        pdf_bytes(&objects)
    }

    /// PdfCraft patch: a tiny crop box keeps a one-point floor after `/UserUnit`, not one raw unit.
    #[test]
    fn raw_crop_unit_floor_is_one_point_after_user_unit() {
        for (unit, width, height) in [(1.0, 1.0, 1.0), (4096.0, 512.0, 1024.0), (75000.0, 9375.0, 18750.0)] {
            let pdf = Pdf::new(raw_crop_unit_page(unit, [10.0, 20.0, 10.125, 20.25])).unwrap();
            let page = &pdf.pages()[0];
            assert_eq!(page.base_dimensions(), ((width / unit) as f32, (height / unit) as f32), "unit={unit}");
        }
        // In f32, the nearest reciprocal of 41 or 47 multiplies back to 0.99999994.
        for unit in [1.0, 41.0, 47.0] {
            let pdf = Pdf::new(raw_crop_unit_page(unit, [0.0, 0.0, 0.01, 0.01])).unwrap();
            let page = &pdf.pages()[0];
            let (width, height) = page.base_dimensions();
            assert!(width * page.user_unit() >= 1.0 && height * page.user_unit() >= 1.0, "unit={unit}");
            let (width, height) = page.render_dimensions();
            assert!((1.0..=1.000001).contains(&width) && (1.0..=1.000001).contains(&height), "unit={unit}: {width} by {height}");
        }
        // A nearly-zero crop box still falls back to US Letter (PdfCraft patch 44).
        for unit in [1.0, 4096.0, 75000.0] {
            for side in [0.0, 0.0001] {
                let pdf = Pdf::new(raw_crop_unit_page(unit, [0.0, 0.0, side, side])).unwrap();
                let page = &pdf.pages()[0];
                let expected = (super::LETTER.width() as f32, super::LETTER.height() as f32);
                assert_eq!(page.base_dimensions(), expected, "unit={unit}, side={side}");
                assert_eq!(page.render_dimensions(), (expected.0 * unit as f32, expected.1 * unit as f32));
            }
        }
    }
}

#[cfg(test)]
mod resources_scope_tests {
    use super::Resources;
    use crate::object::{Dict, FromBytes, Name};
    use crate::reader::ReaderContext;

    #[test]
    fn explicit_scope_replaces_all_categories_and_absence_inherits() {
        let enclosing = Dict::from_bytes(b"<< /Font << /F << /BaseFont /Helvetica >> >> /ExtGState << /G << /CA 0.5 >> >> >>").unwrap();
        let parent = Resources::new(enclosing, None, ReaderContext::dummy_ref());
        let font = Name::from_bytes(b"/F").unwrap();
        let state = Name::from_bytes(b"/G").unwrap();
        let inherited = Resources::from_scope(None, parent.clone());
        assert!(inherited.get_font(&font).is_some());
        assert!(inherited.get_ext_g_state(&state).is_some());

        for bytes in [b"<< >>".as_slice(), b"<< /ProcSet [/PDF] >>", b"<< /Font << /Other << /BaseFont /Courier >> >> >>"] {
            let own = Resources::from_scope(Some(Dict::from_bytes(bytes).unwrap()), parent.clone());
            assert!(own.get_font(&font).is_none(), "an explicit scope must not borrow a font");
            assert!(own.get_ext_g_state(&state).is_none(), "nor another missing resource category");
            assert!(own.parent().is_none());
        }
    }
}

#[cfg(test)]
mod content_reader_tests {
    use crate::Pdf;

    /// A zlib stream of `data` in stored blocks, which expands by nothing.
    fn stored_zlib(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let mut chunks = data.chunks(0xffff).peekable();
        if chunks.peek().is_none() {
            out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
        }
        while let Some(chunk) = chunks.next() {
            out.push(u8::from(chunks.peek().is_none()));
            let len = u16::try_from(chunk.len()).expect("a stored block holds at most 65535 bytes");
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(chunk);
        }
        out.extend_from_slice(&[0; 4]);
        out
    }

    /// A zlib stream of one literal byte and then `count` copies of a 258-byte match from it, in
    /// fixed Huffman codes: about 160 bytes of output for each byte of input.
    fn fixed_run_bomb(count: usize) -> Vec<u8> {
        struct Bits {
            out: Vec<u8>,
            buf: u8,
            len: u32,
        }
        impl Bits {
            /// Bits are sent least significant first.
            fn put(&mut self, value: u32, bits: u32) {
                for i in 0..bits {
                    self.buf |= (((value >> i) & 1) as u8) << self.len;
                    self.len += 1;
                    if self.len == 8 {
                        self.out.push(self.buf);
                        self.buf = 0;
                        self.len = 0;
                    }
                }
            }

            /// Huffman codes are sent most significant bit first.
            fn code(&mut self, code: u32, bits: u32) {
                for i in (0..bits).rev() {
                    self.put((code >> i) & 1, 1);
                }
            }
        }

        let mut bits = Bits {
            out: vec![0x78, 0x01],
            buf: 0,
            len: 0,
        };
        bits.put(1, 1); // The final block.
        bits.put(1, 2); // Fixed Huffman codes.
        bits.code(0x30 + u32::from(b'a'), 8); // The literal 'a'.
        for _ in 0..count {
            bits.code(0xc5, 8); // Length 258 (code 285), no extra bits.
            bits.code(0, 5); // Distance 1 (code 0), no extra bits.
        }
        bits.code(0, 7); // End of block (code 256).
        if bits.len > 0 {
            bits.out.push(bits.buf);
        }
        bits.out.extend_from_slice(&[0; 4]);
        bits.out
    }

    /// A one-page PDF whose content is `stream`, with the stream dictionary entries `dict`.
    fn page_pdf(stream: &[u8], dict: &str) -> Vec<u8> {
        let mut pdf = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R >> endobj\n4 0 obj << {dict} /Length {} >> stream\n",
            stream.len()
        )
        .into_bytes();
        pdf.extend_from_slice(stream);
        pdf.extend_from_slice(b"\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n");
        pdf
    }

    /// A one-page PDF with an array of content references to `streams`.
    fn page_pdf_array(streams: &[(&[u8], &str)], refs: &[usize]) -> Vec<u8> {
        let contents = refs
            .iter()
            .map(|index| format!("{} 0 R", 4 + index))
            .collect::<Vec<_>>()
            .join(" ");
        let mut pdf = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents [{contents}] >> endobj\n"
        )
        .into_bytes();
        for (index, (stream, dict)) in streams.iter().enumerate() {
            let object = 4 + index;
            pdf.extend_from_slice(
                format!("{object} 0 obj << {dict} /Length {} >> stream\n", stream.len())
                    .as_bytes(),
            );
            pdf.extend_from_slice(stream);
            pdf.extend_from_slice(b"\nendstream endobj\n");
        }
        pdf.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF\n");
        pdf
    }

    /// The content of the first page, read `piece` bytes at a time, with a budget of `cap`.
    fn read_content(pdf: Vec<u8>, cap: usize, piece: usize) -> (Vec<u8>, bool) {
        let doc = Pdf::new(pdf).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader_within(cap);
        let mut out = Vec::new();
        while content.read_into(&mut out, piece) > 0 {}

        (out, content.truncated())
    }

    #[test]
    fn plain_and_whole_pieces_keep_their_bytes() {
        let plain = b"q 1 0 0 1 0 0 cm Q\n";
        let whole = b"q 0 0 m Q\n";
        let encoded: String = whole.iter().map(|byte| format!("{byte:02x}")).collect::<String>() + ">";

        for piece in [1, 7, 4096, 65_536] {
            let (plain_out, plain_truncated) = read_content(page_pdf(plain, ""), 1 << 20, piece);
            assert_eq!(plain_out, plain, "plain piece {piece}");
            assert!(!plain_truncated, "plain piece {piece}");

            let (whole_out, whole_truncated) = read_content(
                page_pdf(encoded.as_bytes(), "/Filter /ASCIIHexDecode"),
                1 << 20,
                piece,
            );
            assert_eq!(whole_out, whole, "whole piece {piece}");
            assert!(!whole_truncated, "whole piece {piece}");
        }
    }

    #[test]
    fn repeated_content_reference_counts_compressed_bytes_once() {
        let data = vec![b'q'; 1 << 16];
        let pdf = page_pdf_array(&[(data.as_slice(), "")], &[0, 0]);
        let doc = Pdf::new(pdf).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader_within(1 << 20);
        let expected_budget = 64 * data.len();
        assert_eq!(content.budget, expected_budget);

        let mut out = Vec::new();
        while content.read_into(&mut out, 4096) > 0 {}
        let mut expected = data.clone();
        expected.push(b' ');
        expected.extend_from_slice(&data);
        expected.push(b' ');
        assert!(
            out == expected,
            "expected {} bytes, got {}",
            expected.len(),
            out.len()
        );
        assert!(!content.truncated());
    }

    #[test]
    fn whole_cap_still_counts_later_unique_streams() {
        let first = vec![b'q'; 1 << 20];
        let later = vec![b'Q'; 12_000];
        let hex = |data: &[u8]| {
            data.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                + ">"
        };
        let first_raw = hex(&first);
        let later_raw = hex(&later);
        let pdf = page_pdf_array(
            &[
                (first_raw.as_bytes(), "/Filter /ASCIIHexDecode"),
                (later_raw.as_bytes(), "/Filter /ASCIIHexDecode"),
            ],
            &[0, 1],
        );
        let cap = 1 << 20;
        let doc = Pdf::new(pdf).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader_within(cap);
        let expected_budget = 64 * (first_raw.len() + later_raw.len());
        assert!(expected_budget > cap);
        assert_eq!(content.budget, expected_budget);
        assert_eq!(content.materialized_len(), cap);
        assert_eq!(content.pieces.len(), 2, "only the first whole stream and separator are kept");

        let mut out = Vec::new();
        while content.read_into(&mut out, 65_536) > 0 {}
        let mut expected = first;
        expected.push(b' ');
        assert_eq!(out, expected);
        assert!(content.truncated(), "omitted streams must not look complete");
    }

    #[test]
    fn malformed_flate_and_failed_whole_streams_report_incomplete_content() {
        for (data, filter) in [
            (vec![0x78, 0x9c, 0x07], "/Filter /FlateDecode"),
            (vec![0x78, 0x9c, 0x01, 10, 0, 245, 255, b'q'], "/Filter /FlateDecode"),
            (b"not an image".to_vec(), "/Filter /DCTDecode"),
        ] {
            let (_, incomplete) = read_content(page_pdf(&data, filter), 4096, 7);
            assert!(incomplete, "failed {filter} must not look like complete content");
        }
    }

    #[test]
    fn whole_stream_reports_a_smaller_document_decode_limit() {
        // RunLength repeat packet: 128 decoded bytes, followed by EOD.
        let data = page_pdf(&[129, b'A', 128], "/Filter /RunLengthDecode");
        let limits = crate::DecodeLimits::default().with_max_decoded_stream(32);
        let doc = Pdf::new_with_limits(data, limits).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader();
        let mut out = Vec::new();
        while content.read_into(&mut out, 7) > 0 {}
        assert_eq!(out, vec![b'A'; 32]);
        assert!(content.truncated(), "the smaller document limit cut this stream");
    }

    #[test]
    fn a_nonfinal_flate_block_is_not_complete_at_input_eof() {
        let mut data = stored_zlib(b"q Q");
        // Stored zlib fixture starts with its two-byte header then the final-block flag.
        data[2] &= !1;
        // Drop the checksum so the next block is absent rather than malformed checksum bytes.
        data.truncate(data.len() - 4);
        let (out, incomplete) = read_content(page_pdf(&data, "/Filter /FlateDecode"), 4096, 2);
        assert_eq!(out, b"q Q");
        assert!(incomplete);
    }

    #[test]
    fn content_past_the_cap_is_read_to_the_end() {
        let content: Vec<u8> = b"q 1 0 0 1 0 0 cm Q\n".iter().copied().cycle().take(3 << 20).collect();
        let pdf = page_pdf(&stored_zlib(&content), "/Filter /FlateDecode");
        // The cap is far below the content, but stored data expands by nothing, so nothing is cut.
        for piece in [7, 4096, 1 << 16] {
            let (out, truncated) = read_content(pdf.clone(), 1 << 20, piece);
            assert_eq!(out.len(), content.len(), "piece {piece}");
            assert!(out == content, "piece {piece}: the bytes are the content's");
            assert!(!truncated, "piece {piece}");
        }
    }

    #[test]
    fn a_stream_that_expands_past_the_budget_is_cut_there() {
        let cap = 1 << 20;
        let bomb = fixed_run_bomb(400_000);
        let (out, truncated) = read_content(page_pdf(&bomb, "/Filter /FlateDecode"), cap, 1 << 16);

        assert!(truncated, "the expansion is past the budget");
        // The budget is 64 times the compressed stream, which is more than the cap here.
        assert_eq!(out.len(), 64 * bomb.len());
        assert!(out.iter().all(|&b| b == b'a'));
    }

    #[test]
    fn a_small_expansion_under_the_cap_is_not_cut() {
        let bomb = fixed_run_bomb(1000);
        let (out, truncated) = read_content(page_pdf(&bomb, "/Filter /FlateDecode"), 1 << 20, 4096);

        assert!(!truncated);
        assert_eq!(out.len(), 1 + 258 * 1000);
    }
}
