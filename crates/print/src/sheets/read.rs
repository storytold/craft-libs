//! The sheets of a document: the page tree and each page's boxes and rotation.

use super::*;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One sheet of the print-ready PDF.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sheet {
    /// The page object (number, generation); `None` for a page written inside its parent.
    pub(super) obj: Option<(u32, u16)>,
    pub(super) dict: Dict,
    /// The MediaBox, inherited or not.
    pub(super) media: [f64; 4],
    /// The visible box (CropBox within MediaBox), in the page's own unrotated space.
    pub(crate) bounds: [f64; 4],
    /// `/Resources` inherited from the page tree; `None` when the page has its own or none.
    pub(super) resources: Option<Arc<Obj>>,
    /// `/Rotate`, 0, 90, 180 or 270.
    pub(crate) rotate: u32,
    /// Annotation flags match the renderer's visible printing behavior.
    pub(super) print_annotations_safe: bool,
    /// `/Annots` lists at least one annotation.
    pub(super) annotated: bool,
}

impl Sheet {
    /// Width and height as the sheet is shown and printed (rotation applied), in points.
    pub(crate) fn size(&self) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.bounds;
        if self.rotate % 180 == 90 { (y1 - y0, x1 - x0) } else { (x1 - x0, y1 - y0) }
    }
}

/// A print-ready PDF's sheets, and what an update to it needs.
pub(crate) struct Sheets<'a> {
    pub(super) data: &'a [u8],
    pub(crate) sheets: Vec<Sheet>,
    pub(super) trailer: Dict,
    pub(super) catalog: Dict,
    /// The newest cross-reference section is a stream (so is the update's).
    pub(super) xref_stream: bool,
    pub(super) startxref: usize,
    /// The first unused object number.
    pub(super) size: u32,
}

/// What [`read`] inherits down the page tree.
#[derive(Clone, Default)]
pub(super) struct Inherited {
    pub(super) media: Option<Arc<Obj>>,
    crop: Option<Arc<Obj>>,
    pub(super) rotate: Option<Arc<Obj>>,
    pub(super) resources: Option<Arc<Obj>>,
}

/// Read the sheets of a print-ready PDF.
pub(crate) fn read(data: &[u8]) -> Result<Sheets<'_>, Error> {
    let start = startxref(data).ok_or_else(|| unreadable("no cross-reference table"))?;
    let mut entries = HashMap::new();
    let mut budget = Budget::default();
    let mut trailer = None;
    let mut xref_stream = false;
    let mut seen = HashSet::new();
    let mut next = Some(start);
    while let Some(offset) = next {
        if !seen.insert(offset) || seen.len() > MAX_SECTIONS {
            break;
        }
        let Some((section, is_stream)) = xref_section(data, offset, &mut entries, &mut budget) else {
            // An older section that can't be read only loses objects the newer ones replaced.
            if budget.exhausted || trailer.is_none() {
                return Err(unreadable("the cross-reference table is damaged"));
            }
            break;
        };
        next = get(&section, b"Prev").and_then(Obj::int).and_then(|v| usize::try_from(v).ok());
        if trailer.is_none() {
            xref_stream = is_stream;
            trailer = Some(section);
        }
    }
    let trailer = trailer.ok_or_else(|| unreadable("no trailer"))?;
    let size = entries
        .keys()
        .max()
        .map_or(0, |m| m.saturating_add(1))
        .max(get(&trailer, b"Size").and_then(Obj::int).and_then(|s| u32::try_from(s).ok()).unwrap_or(0));
    let mut reader = Reader { data, entries, objstms: HashMap::new(), budget };
    let root = get(&trailer, b"Root").cloned().ok_or_else(|| unreadable("no document catalog"))?;
    let catalog = reader.resolve(&root).and_then(|c| c.dict().cloned()).ok_or_else(|| unreadable("no document catalog"))?;
    let pages_root = get(&catalog, b"Pages").cloned().ok_or_else(|| unreadable("no page tree"))?;

    let mut sheets = Vec::new();
    let mut visited = HashSet::new();
    // Depth-first in page order: kids are pushed last to first.
    let mut stack = vec![(pages_root, Inherited::default(), 0usize)];
    while let Some((node, inherited, depth)) = stack.pop() {
        if depth > MAX_DEPTH || sheets.len() >= MAX_PAGES {
            return Err(unreadable("the page tree is too deep or too large"));
        }
        let obj = match &node {
            Obj::Ref(num, generation) => {
                if !visited.insert(*num) {
                    return Err(unreadable("the page tree loops"));
                }
                Some((*num, *generation))
            }
            _ => None,
        };
        let mut dict = reader.resolve(&node).and_then(|d| d.dict().cloned()).ok_or_else(|| unreadable("a page is missing"))?;
        // A null value is the same as no entry (ISO 32000-1, 7.3.9), so it inherits.
        let own = |key: &[u8]| get(&dict, key).filter(|v| **v != Obj::Null).cloned().map(Arc::new);
        let parent_resources = inherited.resources.clone();
        let here = Inherited {
            media: own(b"MediaBox").or(inherited.media),
            crop: own(b"CropBox").or(inherited.crop),
            rotate: own(b"Rotate").or(inherited.rotate),
            resources: own(b"Resources").or(inherited.resources),
        };
        let kids = get(&dict, b"Kids").cloned().and_then(|k| reader.resolve(&k));
        let is_node = get(&dict, b"Type").and_then(Obj::name) == Some(b"Pages") || (get(&dict, b"Type").is_none() && kids.is_some());
        if is_node {
            let kids = kids.and_then(|k| k.array().map(<[Obj]>::to_vec)).unwrap_or_default();
            if stack.len().checked_add(kids.len()).is_none_or(|n| n > MAX_PAGES) {
                return Err(unreadable("the page tree is too large"));
            }
            stack.extend(kids.into_iter().rev().map(|kid| (kid, here.clone(), depth + 1)));
            continue;
        }
        // Windows.Data.Pdf ignores nondefault UserUnit. Refuse it before submitting a
        // job rather than silently printing the wrong physical dimensions.
        if let Some(unit) = get(&dict, b"UserUnit") {
            let unit = reader.resolve(unit).and_then(|u| u.num());
            if unit != Some(1.0) {
                return Err(unreadable("nondefault or invalid UserUnit is not supported"));
            }
        }
        if let Some(contents) = get(&dict, b"Contents").cloned() {
            let resolved = reader.resolve(&contents).ok_or_else(|| unreadable("page content is missing"))?;
            // An indirect array can live in an object stream. Keep only its stream
            // references in the wrapper; an array reference there renders blank.
            if let Obj::Array(items) = resolved {
                if !items.iter().all(|item| matches!(item, Obj::Ref(..))) {
                    return Err(unreadable("a content array must contain stream references"));
                }
                dict.retain(|(key, _)| key != b"Contents");
                dict.push((b"Contents".to_vec(), Obj::Array(items)));
            } else if resolved == Obj::Null {
                dict.retain(|(key, _)| key != b"Contents");
            } else if !matches!(contents, Obj::Ref(..)) || !matches!(resolved, Obj::Dict(_)) {
                return Err(unreadable("page content must be a stream or an array of streams"));
            }
        }
        let media = reader.rect(here.media.as_deref()).filter(|r| r[2] > r[0] && r[3] > r[1]).unwrap_or(DEFAULT_MEDIA);
        let bounds = reader
            .rect(here.crop.as_deref())
            .map(|c| [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])])
            .filter(|c| c[2] > c[0] && c[3] > c[1])
            .unwrap_or(media);
        let rotate = here.rotate.as_ref().and_then(|r| reader.resolve_borrowed(r).and_then(|r| r.int())).unwrap_or(0).rem_euclid(360);
        // Not a multiple of 90 is invalid; such a page is shown unrotated.
        let rotate = if rotate % 90 == 0 { u32::try_from(rotate).unwrap_or(0) } else { 0 };
        // Windows renders viewer visibility, not annotation print visibility. Only a plain
        // visible printable annotation is safe; flatten other flags before native submission.
        let (print_annotations_safe, annotated) = match get(&dict, b"Annots") {
            None | Some(Obj::Null) => (true, false),
            Some(annotations) => match reader.resolve(annotations) {
                Some(Obj::Null) => (true, false),
                Some(Obj::Array(items)) => (
                    items.iter().all(|item| {
                        reader
                            .resolve(item)
                            .and_then(|a| a.dict().cloned())
                            .is_some_and(|annotation| get(&annotation, b"F").and_then(|f| reader.resolve(f)).and_then(|f| f.int()) == Some(4))
                    }),
                    !items.is_empty(),
                ),
                _ => (false, true),
            },
        };
        // A reference to a missing object is null too. Only the cross-reference table is
        // looked at: resolving a shared Resources dictionary for every page would parse it again.
        let own_resources = match get(&dict, b"Resources") {
            None | Some(Obj::Null) => false,
            Some(Obj::Ref(num, _)) => matches!(reader.entries.get(num), Some(Entry::At(_) | Entry::In(..))),
            Some(_) => true,
        };
        let resources = if own_resources { None } else { parent_resources };
        sheets.push(Sheet { obj, dict, media, bounds, resources, rotate, print_annotations_safe, annotated });
    }
    if reader.budget.exhausted {
        return Err(unreadable("the parser memory budget was exceeded"));
    }
    if sheets.is_empty() {
        return Err(Error::Spool("the print-ready PDF has no pages".into()));
    }
    Ok(Sheets { data, sheets, trailer, catalog, xref_stream, startxref: start, size })
}
