//! HEIF / HEIC / AVIF metadata from the box structure alone (feature `metadata`): the `Exif` and
//! XMP items of the image file format built on the ISO base media file format (ISO/IEC 23008-12
//! on ISO/IEC 14496-12). It needs no HEVC decoder and no heic-rs, so a build that only catalogues
//! photos can read their capture time, camera and lens.
//!
//! - `ftyp` names a HEIF or AVIF brand ([`is_heif`]).
//! - The top-level `meta` box lists the items (`iinf`, one `infe` each), says where each item's
//!   bytes are (`iloc`: extents in the file, construction method 0, or in the `meta` box's own
//!   `idat`, method 1), which item is the photo (`pitm`) and how items relate (`iref`).
//! - EXIF is an item of type `Exif`: a 4-byte big-endian offset to the TIFF header, then the
//!   payload (ISO/IEC 23008-12 annex A). iPhones write offset 6 and an `Exif\0\0` prefix; libheif
//!   and avif-serialize write offset 0 and the TIFF stream ([`exif_tiff`]).
//! - XMP is an item of type `mime` with content type `application/rdf+xml`.
//! - A `cdsc` ("content describes") reference ties a metadata item to the image it describes. The
//!   photo's items are those tied to the primary image or to an image it is made from (its `dimg`
//!   inputs: a grid's tiles, the picture an `iden` turns, an `iovl` overlay's or a `tmap` tone
//!   map's inputs), else those tied to no image (they describe the file). An item tied only to
//!   other images (a thumbnail, a depth map, another photo of a collection) is not this photo's.
//!
//! [`read`] returns the photo's own EXIF and XMP, as the decoder's [`crate::read_metadata`] does.
//! [`scan`] returns every EXIF and XMP item instead, whichever image it describes, and says when
//! it can't be sure it saw them all ([`Scan::complete`], [`Unreadable`]). An app that must prove a
//! file holds no location before copying it out (export's "remove location") refuses when unsure.
//!
//! The ICC profile is left to the decoder, which reads it for rendering. Every size and offset is
//! checked against the buffer; the boxes read, the items considered and the bytes gathered are
//! bounded, and `iloc` is walked once. Moved from LightCraft's `lightcraft-meta`.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};

use crate::{Error, SEQUENCE};

/// The `ftyp` brands of HEIF still images and image sequences, and of AVIF.
const IMAGE_BRANDS: [&[u8; 4]; 12] = [b"mif1", b"msf1", b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs", b"avif", b"avis"];
/// Brands of other formats in the same container (Canon CR3, JPEG XL), which win when listed too.
const OTHER_BRANDS: [&[u8; 4]; 2] = [b"crx ", b"jxl "];

/// How far into the `ftyp` box brands are looked for.
pub const MAX_FTYP: usize = 4096;
/// Boxes read in one file. An iPhone photo has about a hundred items, a large grid a few hundred.
pub const MAX_BOXES: usize = 1 << 16;
/// Metadata items [`read`] considers for the photo's own EXIF and XMP (a real file has one or two).
pub const MAX_CANDIDATES: usize = 64;
/// Metadata items [`scan`] returns; a file with more is not [`Scan::complete`].
pub const MAX_SCANNED: usize = 1024;
/// The most bytes one metadata item may gather (EXIF and XMP are kilobytes).
pub const MAX_ITEM: usize = 16 << 20;
/// The most bytes [`scan`] gathers over all items.
pub const MAX_SCANNED_BYTES: usize = 64 << 20;

/// Does the `ftyp` box name a HEIF or AVIF brand (and not CR3 or JPEG XL)?
pub fn is_heif(bytes: &[u8]) -> bool {
    if bytes.get(4..8) != Some(b"ftyp".as_slice()) {
        return false;
    }
    // major brand at 8, minor version at 12, compatible brands from 16 to the end of the box
    let size = be32(bytes, 0).and_then(|s| usize::try_from(s).ok()).unwrap_or(0);
    let end = size.clamp(12, MAX_FTYP).min(bytes.len());
    let compatible = (16..end).step_by(4).filter_map(|at| bytes.get(at..at.checked_add(4).filter(|&e| e <= end)?));
    let mut heif = false;
    for brand in bytes.get(8..12).into_iter().chain(compatible) {
        if OTHER_BRANDS.iter().any(|b| b.as_slice() == brand) {
            return false;
        }
        heif |= IMAGE_BRANDS.iter().any(|b| b.as_slice() == brand);
    }
    heif
}

/// The photo's EXIF (its TIFF stream, see [`exif_tiff`]) and XMP packet, as
/// [`crate::read_metadata`] returns them: the primary image's items first, then those tied to no
/// image; the first that reads well wins. An error when the file is not HEIF ([`is_heif`]) or has
/// no `meta` box (an image sequence); an item that can't be read is left out rather than reported.
pub fn read(bytes: &[u8]) -> Result<(Option<Vec<u8>>, Option<String>), Error> {
    let c = Container::read(bytes, MAX_CANDIDATES).ok_or_else(not_heif)?;
    if !c.has_meta {
        return Err(Error::Unsupported(SEQUENCE));
    }
    let ranked = |kind: Kind| {
        let mut order: Vec<(u8, usize)> =
            c.items.iter().enumerate().filter(|(_, it)| it.kind == kind).filter_map(|(i, it)| Some((it.rank(c.primary.is_some())?, i))).collect();
        order.sort_by_key(|&(rank, _)| rank);
        order.into_iter().filter_map(|(_, i)| match c.gather(i, MAX_ITEM) {
            Gathered::Bytes(b) => Some(b),
            _ => None,
        })
    };
    let exif = ranked(Kind::Exif).find_map(|item| exif_tiff(&item).map(<[u8]>::to_vec));
    let xmp = ranked(Kind::Xmp).next().map(|item| String::from_utf8_lossy(&item).trim_end_matches('\0').to_string());
    Ok((exif, xmp))
}

/// Every EXIF and XMP item of a HEIF file, whichever image it describes, in `iinf` order. An error
/// only when the file is not HEIF ([`is_heif`]); a file without a `meta` box has no items.
///
/// The scan says when it may be missing something: [`Scan::complete`] is false when the item
/// catalog can't be read in full, and an item whose bytes can't be read comes back as
/// [`Unreadable`]. An item a complete `iloc` doesn't place is left out: it has no bytes.
pub fn scan(bytes: &[u8]) -> Result<Scan<'_>, Error> {
    scan_within(bytes, MAX_SCANNED_BYTES)
}

/// [`scan`], gathering at most `budget` bytes over all items.
fn scan_within(bytes: &[u8], budget: usize) -> Result<Scan<'_>, Error> {
    let container = Container::read(bytes, MAX_SCANNED).ok_or_else(not_heif)?;
    Ok(Scan { container, next: 0, budget })
}

fn not_heif() -> Error {
    Error::Malformed("not a HEIF file".into())
}

/// The metadata items of a HEIF file, from [`scan`]: an iterator over [`Item`]s.
pub struct Scan<'a> {
    container: Container<'a>,
    next: usize,
    /// Bytes still to gather, of [`MAX_SCANNED_BYTES`].
    budget: usize,
}

impl Scan<'_> {
    /// Whether the iterator returns every metadata item in the file: the boxes up to and through
    /// the item catalog were read to their ends, there was one `meta` box with one `iinf`, every
    /// catalog entry was understood, and there were at most [`MAX_SCANNED`] metadata items. A
    /// complete file without a `meta` box is complete, with no items.
    pub fn complete(&self) -> bool {
        self.container.catalog_complete && !self.container.more
    }
}

impl Iterator for Scan<'_> {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        loop {
            let i = self.next;
            let (id, kind) = self.container.items.get(i).map(|c| (c.id, c.kind))?;
            self.next = i.saturating_add(1);
            let data = match self.container.gather(i, MAX_ITEM.min(self.budget)) {
                Gathered::Bytes(b) => {
                    self.budget = self.budget.saturating_sub(b.len());
                    Ok(b)
                }
                Gathered::NotPlaced => continue,
                Gathered::Unreadable => Err(Unreadable::Placement),
                Gathered::TooLarge => Err(Unreadable::TooLarge),
                Gathered::Protected => Err(Unreadable::Protected),
            };
            return Some(Item { id, kind, data });
        }
    }
}

/// One metadata item from [`scan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Its item id.
    pub id: u32,
    pub kind: Kind,
    /// Its bytes as stored (for EXIF, with the offset field: see [`exif_tiff`]), or why there are
    /// none.
    pub data: Result<Vec<u8>, Unreadable>,
}

/// What a metadata item holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// An `Exif` item.
    Exif,
    /// An XMP packet: a `mime` item of type `application/rdf+xml`.
    Xmp,
    /// An XMP packet with a content encoding (compressed). Its bytes are returned as stored; this
    /// crate doesn't decode them.
    EncodedXmp,
}

/// Why a metadata item's bytes can't be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unreadable {
    /// `iloc` is missing or damaged before the item's entry, or the entry points outside the file
    /// (or `idat`), into another file, or builds the item from other items (method 2).
    Placement,
    /// More than [`MAX_ITEM`] bytes, or more than [`scan`] has left of [`MAX_SCANNED_BYTES`].
    TooLarge,
    /// The item is protected (encrypted): its `infe` entry names a protection scheme, so its
    /// stored bytes are not the metadata in the clear.
    Protected,
}

/// The TIFF stream of an `Exif` item: past its 4-byte offset field and the bytes that offset
/// skips. An item whose offset is wrong, or that has no offset field, is read too when an
/// `Exif\0\0` prefix or the TIFF header itself shows where the stream starts. `None` when no TIFF
/// header is found.
pub fn exif_tiff(item: &[u8]) -> Option<&[u8]> {
    let tiff = |b: &&[u8]| b.starts_with(b"II*\0") || b.starts_with(b"MM\0*");
    let stated = be32(item, 0).and_then(|n| usize::try_from(n).ok()?.checked_add(4)).and_then(|at| item.get(at..));
    let after_field = item.get(4..).map(strip_exif_header);
    [stated, after_field, Some(strip_exif_header(item))].into_iter().flatten().find(tiff)
}

/// `b` without a leading `Exif\0` and the byte after it (the JPEG APP1 prefix), if it has one.
fn strip_exif_header(b: &[u8]) -> &[u8] {
    match b.get(..5) {
        Some(b"Exif\0") => b.get(6..).unwrap_or(b),
        _ => b,
    }
}

/// A metadata item and the images it describes.
#[derive(Debug)]
struct Candidate {
    id: u32,
    kind: Kind,
    /// Tied to some image by a `cdsc` reference.
    describes: bool,
    /// Tied to the primary image or an image it is made from.
    describes_primary: bool,
    /// Protected (encrypted): there, but not readable.
    protected: bool,
}

impl Candidate {
    /// 0 for the photo's own, 1 for one tied to no image (or any, when the file names no primary
    /// image that exists); `None` for one that describes other images only.
    fn rank(&self, has_primary: bool) -> Option<u8> {
        if self.describes_primary {
            Some(0)
        } else if !self.describes || !has_primary {
            Some(1)
        } else {
            None
        }
    }
}

/// What the `meta` box says about the metadata items, and where their bytes are.
struct Container<'a> {
    file: &'a [u8],
    /// A top-level `meta` box was found.
    has_meta: bool,
    idat: Option<&'a [u8]>,
    iloc: Option<Iloc<'a>>,
    /// The primary image, when `pitm` names an item `iinf` lists.
    primary: Option<u32>,
    /// The EXIF and XMP items, in `iinf` order (the first `cap` of them).
    items: Vec<Candidate>,
    /// More metadata items than `cap`.
    more: bool,
    /// Discovery read the whole item catalog, including each entry's type.
    catalog_complete: bool,
    /// Where each of `items` is (from one walk of `iloc`).
    places: Vec<Option<Place>>,
    /// A complete `iloc` walk established that any unplaced item has no entry.
    iloc_complete: bool,
}

impl<'a> Container<'a> {
    /// `None` when `bytes` is not HEIF; no items when there is no `meta` box.
    fn read(bytes: &'a [u8], cap: usize) -> Option<Container<'a>> {
        if !is_heif(bytes) {
            return None;
        }
        let mut c = Container {
            file: bytes,
            has_meta: false,
            idat: None,
            iloc: None,
            primary: None,
            items: Vec::new(),
            more: false,
            catalog_complete: false,
            places: Vec::new(),
            iloc_complete: false,
        };
        let mut budget = MAX_BOXES;
        let top = boxes(bytes, 0, bytes.len(), &mut budget);
        c.catalog_complete = top.last().is_some_and(|b| b.end == bytes.len());
        // A complete file without an item `meta` has no item catalog (e.g. a sequence).
        let Some(meta) = top.iter().find(|b| &b.kind == b"meta") else { return Some(c) };
        c.has_meta = true;
        // `meta` is a full box: version and flags, then the boxes inside it.
        let first = meta.body.saturating_add(4);
        let inside = boxes(bytes, first, meta.end, &mut budget);
        c.catalog_complete &= version(bytes, meta) == Some(0)
            && first <= meta.end
            && inside.last().map_or(first, |b| b.end) == meta.end
            && top.iter().filter(|b| &b.kind == b"meta").count() == 1
            && inside.iter().filter(|b| &b.kind == b"iinf").count() == 1;
        let child = |kind: &[u8; 4]| inside.iter().find(|b| &b.kind == kind);
        let payload = |kind: &[u8; 4]| child(kind).and_then(|b| bytes.get(b.body..b.end));
        let mut ids = Vec::new();
        if let Some(iinf) = child(b"iinf") {
            let catalog = item_infos(bytes, iinf, cap, &mut budget);
            ids = catalog.ids;
            c.items = catalog.items;
            c.more = catalog.more;
            c.catalog_complete &= catalog.complete;
        }
        c.primary = payload(b"pitm").and_then(primary_item).filter(|p| ids.binary_search(p).is_ok());
        if let Some(iref) = child(b"iref") {
            link(bytes, iref, c.primary, &mut c.items, &mut budget);
        }
        c.idat = payload(b"idat");
        c.iloc = payload(b"iloc").and_then(Iloc::parse);
        (c.places, c.iloc_complete) = match &c.iloc {
            Some(iloc) => iloc.locate(&c.items),
            None => (Vec::new(), false),
        };
        Some(c)
    }

    /// The bytes of `items[i]`, at most `limit` of them.
    fn gather(&self, i: usize, limit: usize) -> Gathered {
        if self.items.get(i).is_some_and(|c| c.protected) {
            return Gathered::Protected;
        }
        match (&self.iloc, self.places.get(i)) {
            (Some(iloc), Some(Some(place))) => iloc.gather(self.file, self.idat, place, limit),
            _ if self.iloc_complete => Gathered::NotPlaced,
            _ => Gathered::Unreadable,
        }
    }
}

/// An item's bytes, or why there are none.
#[derive(Debug)]
enum Gathered {
    Bytes(Vec<u8>),
    /// A complete `iloc` walk found no entry for the item.
    NotPlaced,
    /// Placement unresolved, outside the data, in another file, or built from other items (method 2).
    Unreadable,
    /// More than the limit.
    TooLarge,
    /// A protected (encrypted) item.
    Protected,
}

/// A full box's version: the first byte of its own payload (`None` for an empty box).
fn version(bytes: &[u8], b: &BoxRef) -> Option<u8> {
    bytes.get(b.body..b.end)?.first().copied()
}

/// `pitm`: the primary item's id (16 bits in version 0, else 32).
fn primary_item(pitm: &[u8]) -> Option<u32> {
    match pitm.first()? {
        0 => be16(pitm, 4).map(u32::from),
        _ => be32(pitm, 4),
    }
}

/// What `iinf` lists. Best-effort discovery is kept even when the catalog cannot establish that
/// all items were read.
#[derive(Default)]
struct ItemCatalog {
    /// Every item id listed, sorted.
    ids: Vec<u32>,
    items: Vec<Candidate>,
    more: bool,
    complete: bool,
}

/// `iinf` → its item ids and its metadata items (at most `cap`). The catalog is complete when the
/// table version is known, its count matches its entries, the entries fill the box, and each is
/// an `infe` this reader understands.
fn item_infos(bytes: &[u8], iinf: &BoxRef, cap: usize, budget: &mut usize) -> ItemCatalog {
    let mut catalog = ItemCatalog::default();
    let Some(data) = bytes.get(iinf.body..iinf.end) else { return catalog };
    // Only the supported versions establish the count and where the entries start.
    let (count, header) = match version(bytes, iinf) {
        Some(0) => (be16(data, 4).map(u32::from), 6),
        Some(1) => (be32(data, 4), 8),
        _ => return catalog,
    };
    let Some(count) = count else { return catalog };
    let first = iinf.body.saturating_add(header);
    let entries = boxes(bytes, first, iinf.end, budget);
    catalog.complete = entries.last().map_or(first, |b| b.end) == iinf.end && u32::try_from(entries.len()).ok() == Some(count);
    for b in &entries {
        let info = bytes.get(b.body..b.end).filter(|_| &b.kind == b"infe").and_then(item_info);
        let Some((id, kind, protected)) = info else {
            catalog.complete = false;
            continue;
        };
        catalog.ids.push(id);
        match kind {
            Some(_) if catalog.items.len() >= cap => catalog.more = true,
            Some(kind) => catalog.items.push(Candidate { id, kind, describes: false, describes_primary: false, protected }),
            None => {}
        }
    }
    catalog.ids.sort_unstable();
    catalog.ids.dedup();
    catalog
}

/// An `infe` entry's item id, its kind when it is an `Exif` or XMP item (`mime`,
/// `application/rdf+xml`), and whether it is protected (encrypted). A protected entry still names
/// its type in the clear, so protected metadata is found too. `None` when the entry can't be read
/// to its last field.
fn item_info(infe: &[u8]) -> Option<(u32, Option<Kind>, bool)> {
    let version = *infe.first()?;
    // version and flags, the item id (32 bits in version 3), the protection index
    let (id, at) = match version {
        0..=2 => (u32::from(be16(infe, 4)?), 6usize),
        3 => (be32(infe, 4)?, 8usize),
        _ => return None,
    };
    let protected = be16(infe, at)? != 0;
    let at = at.checked_add(2)?;
    // versions 0 and 1 have no item type and describe MIME items; 2 and 3 name the type
    let (kind, at) = if version >= 2 { (infe.get(at..at.checked_add(4)?)?, at.checked_add(4)?) } else { (b"mime".as_slice(), at) };
    let (_name, at) = string(infe, at)?;
    let kind = match kind {
        b"Exif" => Some(Kind::Exif),
        b"mime" => {
            let (content_type, at) = string(infe, at)?;
            // Content encoding may be absent, but a present string must terminate.
            let encoding = if at == infe.len() { b"".as_slice() } else { string(infe, at)?.0 };
            match (content_type.trim_ascii().eq_ignore_ascii_case(b"application/rdf+xml"), encoding.trim_ascii().is_empty()) {
                (true, true) => Some(Kind::Xmp),
                (true, false) => Some(Kind::EncodedXmp),
                (false, _) => None,
            }
        }
        b"uri " => {
            string(infe, at)?;
            None
        }
        _ => None,
    };
    Some((id, kind, protected))
}

/// The NUL-terminated string at `at` and where the next field starts.
fn string(b: &[u8], at: usize) -> Option<(&[u8], usize)> {
    let rest = b.get(at..)?;
    let n = rest.iter().position(|&c| c == 0)?;
    Some((rest.get(..n)?, at.checked_add(n)?.checked_add(1)?))
}

/// Mark the candidates that `cdsc` references tie to an image, and those tied to the primary image
/// or one it is made from (`dimg`, followed to any depth; each image once, so loops end).
fn link(bytes: &[u8], iref: &BoxRef, primary: Option<u32>, items: &mut [Candidate], budget: &mut usize) {
    // full box; item ids are 16 bits in version 0, 32 in version 1
    let width = match version(bytes, iref) {
        Some(0) => 2usize,
        Some(1) => 4,
        _ => return,
    };
    let index: HashMap<u32, usize> = items.iter().enumerate().rev().map(|(i, c)| (c.id, i)).collect();
    let mut inputs: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut described: Vec<(usize, Vec<u32>)> = Vec::new();
    for r in boxes(bytes, iref.body.saturating_add(4), iref.end, budget) {
        let cdsc = match &r.kind {
            b"cdsc" => true,
            b"dimg" => false,
            _ => continue,
        };
        let Some(body) = bytes.get(r.body..r.end) else { continue };
        let Some(from) = id_at(body, 0, width) else { continue };
        let item = index.get(&from).copied();
        if cdsc && item.is_none() {
            continue;
        }
        let count = be16(body, width).unwrap_or(0);
        let mut to = Vec::new();
        let mut at = width.saturating_add(2);
        for _ in 0..count {
            let Some(id) = id_at(body, at, width) else { break };
            to.push(id);
            at = at.saturating_add(width);
        }
        match item {
            Some(i) if cdsc => described.push((i, to)),
            _ => inputs.entry(from).or_default().extend(to),
        }
    }
    // the primary image and every image it is made from
    let mut photo = HashSet::new();
    let mut todo: Vec<u32> = primary.into_iter().collect();
    while let Some(id) = todo.pop() {
        if photo.insert(id) {
            todo.extend(inputs.get(&id).into_iter().flatten().filter(|i| !photo.contains(*i)));
        }
    }
    for (i, to) in described {
        if let Some(item) = items.get_mut(i)
            && !to.is_empty()
        {
            item.describes = true;
            item.describes_primary |= to.iter().any(|t| photo.contains(t));
        }
    }
}

fn id_at(b: &[u8], at: usize, width: usize) -> Option<u32> {
    if width == 2 { be16(b, at).map(u32::from) } else { be32(b, at) }
}

/// `iloc`: its field sizes, read once.
struct Iloc<'a> {
    data: &'a [u8],
    version: u8,
    offset_size: usize,
    length_size: usize,
    base_size: usize,
    index_size: usize,
}

/// Where an item is: how it is built, and its extents in `iloc`.
#[derive(Clone, Copy, Debug)]
struct Place {
    method: u16,
    data_reference: u16,
    base: u64,
    /// Where its first extent starts in `iloc`.
    extents_at: usize,
    extents: usize,
}

impl<'a> Iloc<'a> {
    fn parse(data: &'a [u8]) -> Option<Iloc<'a>> {
        let version = *data.first()?;
        if version > 2 {
            return None;
        }
        // version and flags, then four 4-bit field sizes: offset, length, base offset, extent index
        let (sizes, more) = (*data.get(4)?, *data.get(5)?);
        let (offset_size, length_size) = (usize::from(sizes >> 4), usize::from(sizes & 15));
        let base_size = usize::from(more >> 4);
        let index_size = if version == 0 { 0 } else { usize::from(more & 15) }; // reserved in version 0
        if [offset_size, length_size, base_size, index_size].into_iter().any(|n| !matches!(n, 0 | 4 | 8)) {
            return None;
        }
        Some(Iloc { data, version, offset_size, length_size, base_size, index_size })
    }

    /// One walk over the entries: where each of `items` is (the first entry for an id wins). It
    /// stops once all are placed, or where the entries stop making sense. The boolean is true
    /// only if the walk exhausted the entries, so an absent placement is known to be absent.
    fn locate(&self, items: &[Candidate]) -> (Vec<Option<Place>>, bool) {
        let mut places = vec![None; items.len()];
        if items.is_empty() {
            return (places, false);
        }
        let wanted: HashMap<u32, usize> = items.iter().enumerate().rev().map(|(i, c)| (c.id, i)).collect();
        let mut left = wanted.len();
        let complete = self.walk(|id, place| {
            if let Some(&i) = wanted.get(&id)
                && let Some(slot @ None) = places.get_mut(i)
            {
                *slot = Some(place);
                left = left.saturating_sub(1);
            }
            left > 0
        });
        (places, complete.unwrap_or(false))
    }

    /// Call `f` with each entry's item id and place. `Some(true)` means every entry was read,
    /// `Some(false)` means `f` stopped the walk, and `None` means an entry ran past the box.
    fn walk(&self, mut f: impl FnMut(u32, Place) -> bool) -> Option<bool> {
        let (version, data) = (self.version, self.data);
        let id_size = if version == 2 { 4 } else { 2 };
        let (count, mut at) = if version == 2 { (be32(data, 6)?, 10usize) } else { (u32::from(be16(data, 6)?), 8usize) };
        let extent_size = self.index_size.checked_add(self.offset_size)?.checked_add(self.length_size)?;
        // (every entry takes at least six bytes, so a count larger than the box ends at its end)
        for _ in 0..count {
            iloc_entry_visited();
            let id = uint(data, at, id_size)?;
            at = at.checked_add(id_size)?;
            let method = if version == 0 {
                0
            } else {
                let m = be16(data, at)? & 15;
                at = at.checked_add(2)?;
                m
            };
            let data_reference = be16(data, at)?;
            let base = uint(data, at.checked_add(2)?, self.base_size)?;
            at = at.checked_add(2)?.checked_add(self.base_size)?;
            let extents = usize::from(be16(data, at)?);
            at = at.checked_add(2)?;
            let place = Place { method, data_reference, base, extents_at: at, extents };
            at = at.checked_add(extents.checked_mul(extent_size)?)?;
            if at > data.len() {
                return None;
            }
            if let Ok(id) = u32::try_from(id)
                && !f(id, place)
            {
                return Some(false);
            }
        }
        Some(true)
    }

    /// An item's bytes, its extents in order from the file (construction method 0) or from the
    /// `meta` box's `idat` (method 1), at most `limit` of them. Every extent is checked and the
    /// lengths summed before anything is copied, so an item that is too large (say, the same
    /// region named by thousands of extents) costs no copying.
    fn gather(&self, file: &[u8], idat: Option<&[u8]>, place: &Place, limit: usize) -> Gathered {
        // data reference 0 is this file
        let source = match (place.data_reference, place.method, idat) {
            (0, 0, _) => file,
            (0, 1, Some(idat)) => idat,
            _ => return Gathered::Unreadable,
        };
        let pieces = || {
            let mut at = place.extents_at;
            (0..place.extents).map(move |_| {
                let (piece, next) = self.extent(source, place.base, at)?;
                at = next;
                Some(piece)
            })
        };
        let mut total = 0usize;
        for piece in pieces() {
            let Some(piece) = piece else { return Gathered::Unreadable };
            total = total.saturating_add(piece.len());
            if total > limit {
                return Gathered::TooLarge;
            }
        }
        let mut out = Vec::with_capacity(total);
        for piece in pieces().flatten() {
            bytes_copied(piece.len());
            out.extend_from_slice(piece);
        }
        Gathered::Bytes(out)
    }

    /// The extent at `at`: its bytes in `source` (a length of 0 runs to the end) and where the
    /// next extent starts.
    fn extent<'s>(&self, source: &'s [u8], base: u64, at: usize) -> Option<(&'s [u8], usize)> {
        let at = at.checked_add(self.index_size)?;
        let offset = uint(self.data, at, self.offset_size)?;
        let at = at.checked_add(self.offset_size)?;
        let length = uint(self.data, at, self.length_size)?;
        let at = at.checked_add(self.length_size)?;
        let rest = source.get(usize::try_from(base.checked_add(offset)?).ok()?..)?;
        let piece = if length == 0 { rest } else { rest.get(..usize::try_from(length).ok()?)? };
        Some((piece, at))
    }
}

#[cfg(test)]
thread_local! {
    /// `iloc` entries walked on this thread, for the tests that bound the work.
    static ILOC_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn iloc_entry_visited() {
    #[cfg(test)]
    ILOC_ENTRIES.with(|n| n.set(n.get().saturating_add(1)));
}

#[cfg(test)]
thread_local! {
    /// Item bytes copied on this thread, for the tests that bound the work.
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg_attr(not(test), allow(unused_variables))]
fn bytes_copied(n: usize) {
    #[cfg(test)]
    COPIED.with(|c| c.set(c.get().saturating_add(n)));
}

/// A big-endian unsigned field of `size` bytes (0 reads as 0).
fn uint(b: &[u8], at: usize, size: usize) -> Option<u64> {
    match size {
        0 => Some(0),
        2 => be16(b, at).map(u64::from),
        4 => be32(b, at).map(u64::from),
        8 => be64(b, at),
        _ => None,
    }
}

/// A box: its type, where its payload starts and where it ends.
struct BoxRef {
    kind: [u8; 4],
    /// Start of the payload (after the header and, for `uuid`, the 16-byte uuid).
    body: usize,
    end: usize,
}

/// The boxes directly inside `[from, to)`, at most `budget` of them (shared over the calls of one
/// read); stops at the first malformed header.
fn boxes(bytes: &[u8], from: usize, to: usize, budget: &mut usize) -> Vec<BoxRef> {
    let mut out = Vec::new();
    let to = to.min(bytes.len());
    let mut at = from;
    while *budget > 0 {
        let Some(header_end) = at.checked_add(8).filter(|&e| e <= to) else { break };
        *budget = budget.saturating_sub(1);
        let (Some(size), Some(kind)) = (be32(bytes, at), bytes.get(at.saturating_add(4)..header_end)) else { break };
        let Ok(kind) = <[u8; 4]>::try_from(kind) else { break };
        let (len, header) = match size {
            0 => (to.saturating_sub(at), 8usize),
            1 => match be64(bytes, header_end).and_then(|v| usize::try_from(v).ok()) {
                Some(len) => (len, 16),
                None => break,
            },
            n => match usize::try_from(n) {
                Ok(len) => (len, 8),
                Err(_) => break,
            },
        };
        // the 16-byte extended type of a `uuid` box is part of its header
        let header = if &kind == b"uuid" { header.saturating_add(16) } else { header };
        let Some(end) = at.checked_add(len).filter(|&e| e <= to && len >= header) else { break };
        let Some(body) = at.checked_add(header) else { break };
        out.push(BoxRef { kind, body, end });
        at = end;
    }
    out
}

/// A big-endian `u16` at `at`, or `None` when the buffer is shorter than that.
fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at.checked_add(2)?)?.try_into().ok().map(u16::from_be_bytes)
}

/// A big-endian `u32` at `at`, or `None` when the buffer is shorter than that.
fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?)?.try_into().ok().map(u32::from_be_bytes)
}

/// A big-endian `u64` at `at`, or `None` when the buffer is shorter than that.
fn be64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at.checked_add(8)?)?.try_into().ok().map(u64::from_be_bytes)
}

#[cfg(test)]
mod tests;
