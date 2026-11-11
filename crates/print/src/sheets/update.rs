//! The incremental-update writer that appends the changed pages.

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

/// The whole document printed `copies` times in one job, for collated copies the driver can't
/// make: each copy's pages follow the previous copy's. On two-sided paper, a document with an odd
/// number of sheets gets a blank page after every copy but the last, so each copy starts on a
/// sheet of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Repeat {
    pub(crate) copies: u32,
    pub(crate) two_sided: bool,
}

fn has_annotations(dict: &Dict) -> bool {
    get(dict, b"Annots").is_some_and(|a| !matches!(a, Obj::Null) && !matches!(a, Obj::Array(items) if items.is_empty()))
}

fn rect_obj(r: [f64; 4]) -> Obj {
    Obj::Array(r.into_iter().map(num_obj).collect())
}

/// Starts object `num` in an update, recording where it is.
fn new_object(out: &mut Vec<u8>, num: u32, generation: u16, written: &mut Vec<(u32, u16, usize)>) {
    written.push((num, generation, out.len()));
    out.extend_from_slice(format!("{num} {generation} obj\n").as_bytes());
}

/// Boxes a reframed page drops: they were in the old geometry.
const OLD_GEOMETRY: [&[u8]; 6] = [b"MediaBox", b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox", b"Rotate"];

impl Sheets<'_> {
    /// Refuse annotation print/view visibility the Windows renderer cannot preserve.
    #[cfg(windows)]
    pub(crate) fn check_print_annotations(&self) -> Result<(), Error> {
        if self.sheets.iter().all(|s| s.print_annotations_safe) {
            Ok(())
        } else {
            Err(Error::Spool("flatten annotation print visibility into page content before native Windows printing; nothing was sent".into()))
        }
    }

    /// The PDF with `edits` appended as an incremental update (ISO 32000-1, 7.5.6): the
    /// original bytes, then the new versions of the edited pages, the content streams a
    /// reframed page wraps its own in, and a cross-reference section of the same kind as the
    /// newest one, chained to it with `/Prev`.
    ///
    pub(crate) fn update(&self, edits: &[Edit]) -> Result<Vec<u8>, Error> {
        // Only a repeat can be turned down, so there is always a PDF.
        Ok(self.write(edits, None)?.unwrap_or_else(|| self.data.to_vec()))
    }

    /// [`update`](Self::update) with every copy of `repeat` in the one PDF. The edited pages
    /// are not written in place: the update adds a page tree of its own, every copy's pages as
    /// new page objects that share the original content and resources, and a new version of
    /// the catalog pointing to it. Each new page carries the boxes, rotation and resources it
    /// inherited, since its parent is the new tree. `None` when the copies can't be one PDF:
    /// the PDF is encrypted (strings are encrypted with keys made from their object's number),
    /// a page has annotations (an annotation belongs to one page; whether a renderer draws it on
    /// others is unknown), or the job would be too large.
    #[cfg(windows)]
    pub(crate) fn repeated(&self, edits: &[Edit], repeat: Repeat) -> Result<Option<Vec<u8>>, Error> {
        self.write(edits, Some(repeat))
    }

    pub(super) fn write(&self, edits: &[Edit], repeat: Option<Repeat>) -> Result<Option<Vec<u8>>, Error> {
        let refuse = |why: &str| Error::Spool(format!("the print-ready PDF can't be fitted to the paper: {why}"));
        let mut out = self.data.to_vec();
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        let mut next = self.size;
        // Each new object's number, generation and offset.
        let mut written: Vec<(u32, u16, usize)> = Vec::new();
        let repeat = repeat.filter(|r| r.copies > 1);
        let mut edited = HashMap::new();
        for edit in edits {
            let sheet = self.sheets.get(edit.sheet).ok_or_else(|| refuse("no such sheet"))?;
            let mut dict = sheet.dict.clone();
            dict.retain(|(k, _)| k != b"Rotate");
            if let Some(frame) = &edit.frame {
                if get(&self.trailer, b"Encrypt").is_some() {
                    return Err(refuse("it is encrypted"));
                }
                // Annotation appearances live outside /Contents. Transforming only content
                // would leave them at the old coordinates; deleting them loses printed marks.
                // Require callers to flatten before fitting until appearances can be reframed.
                if has_annotations(&dict) {
                    return Err(refuse("flatten page annotations before using shrink-to-fit"));
                }
                let [a, b, c, d, e, f] = frame.matrix;
                let [x, y, w, h] = frame.clip;
                let prefix = format!(
                    "q {} {} {} {} {} {} cm {} {} {} {} re W n\n",
                    number(a),
                    number(b),
                    number(c),
                    number(d),
                    number(e),
                    number(f),
                    number(x),
                    number(y),
                    number(w),
                    number(h)
                );
                let mut contents = Vec::new();
                for part in [prefix.as_bytes(), b"\nQ\n"] {
                    let stream = next;
                    next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
                    new_object(&mut out, stream, 0, &mut written);
                    out.extend_from_slice(format!("<</Length {}>>\nstream\n", part.len()).as_bytes());
                    out.extend_from_slice(part);
                    out.extend_from_slice(b"\nendstream\nendobj\n");
                    contents.push(Obj::Ref(stream, 0));
                }
                // The page's own content, in order, between the two.
                let own = match get(&dict, b"Contents") {
                    Some(Obj::Array(items)) => items.clone(),
                    Some(other) => vec![other.clone()],
                    None => Vec::new(),
                };
                contents.splice(1..1, own);
                dict.retain(|(k, _)| !OLD_GEOMETRY.contains(&k.as_slice()) && k != b"Contents" && k != b"Annots");
                // CropBox too: a parent's would otherwise be inherited, in the old geometry.
                let media = Obj::Array(vec![num_obj(0.0), num_obj(0.0), num_obj(frame.media.0), num_obj(frame.media.1)]);
                dict.push((b"MediaBox".to_vec(), media.clone()));
                dict.push((b"CropBox".to_vec(), media));
                dict.push((b"Contents".to_vec(), Obj::Array(contents)));
            }
            dict.push((b"Rotate".to_vec(), Obj::Num(edit.rotate.to_string().into_bytes())));
            if repeat.is_some() {
                edited.insert(edit.sheet, dict);
            } else {
                let (num, generation) = sheet.obj.ok_or_else(|| refuse("a page is not an object of its own"))?;
                new_object(&mut out, num, generation, &mut written);
                write_dict(&mut out, &dict);
                out.extend_from_slice(b"\nendobj\n");
            }
        }
        if let Some(Repeat { copies, two_sided }) = repeat {
            let refuse = |why: &str| Error::Spool(format!("the copies can't be written: {why}"));
            if get(&self.trailer, b"Encrypt").is_some() || self.sheets.iter().any(|s| s.annotated) {
                return Ok(None);
            }
            let Some(&Obj::Ref(root, root_generation)) = get(&self.trailer, b"Root") else {
                return Ok(None);
            };
            let blank = two_sided && self.sheets.len() % 2 == 1;
            let per_copy = self.sheets.len() + usize::from(blank);
            let count =
                usize::try_from(copies).ok().and_then(|c| per_copy.checked_mul(c)).map(|n| n - usize::from(blank)).filter(|&n| n <= MAX_PAGES);
            let Some(count) = count else { return Ok(None) };
            let tree = next;
            next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
            let parent = (b"Parent".to_vec(), Obj::Ref(tree, 0));
            // Bytes written once, and bytes written once per copy (a page, its xref entry and kid).
            let (mut once, mut each) = (0usize, 0usize);
            let within =
                |once: usize, each: usize| each.checked_mul(copies as usize).and_then(|n| n.checked_add(once)).is_some_and(|n| n <= MAX_REPEATED);
            // Inherited inline resources are written once, as an object of their own, and every
            // page that inherited them refers to it.
            let mut shared: HashMap<*const Obj, Obj> = HashMap::new();
            let mut bodies: Vec<Vec<u8>> = Vec::with_capacity(per_copy);
            let mut last = Dict::new();
            for (i, sheet) in self.sheets.iter().enumerate() {
                let mut dict = edited.remove(&i).unwrap_or_else(|| sheet.dict.clone());
                dict.retain(|(k, _)| k != b"Parent");
                // The boxes as read, not the inherited objects: those can be anything.
                for (key, value) in [(&b"MediaBox"[..], rect_obj(sheet.media)), (b"CropBox", rect_obj(sheet.bounds))] {
                    if get(&dict, key).is_none_or(|v| *v == Obj::Null) {
                        dict.retain(|(k, _)| k != key);
                        dict.push((key.to_vec(), value));
                    }
                }
                if get(&dict, b"Rotate").is_none_or(|v| *v == Obj::Null) {
                    dict.retain(|(k, _)| k != b"Rotate");
                    dict.push((b"Rotate".to_vec(), Obj::Num(sheet.rotate.to_string().into_bytes())));
                }
                if let Some(resources) = &sheet.resources {
                    let value = match &**resources {
                        Obj::Ref(..) => Obj::clone(resources),
                        _ => match shared.get(&Arc::as_ptr(resources)) {
                            Some(r) => r.clone(),
                            None => {
                                let num = next;
                                next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
                                let at = out.len();
                                new_object(&mut out, num, 0, &mut written);
                                write_obj(&mut out, resources);
                                out.extend_from_slice(b"\nendobj\n");
                                once += out.len() - at;
                                if !within(once, each) {
                                    return Ok(None);
                                }
                                shared.insert(Arc::as_ptr(resources), Obj::Ref(num, 0));
                                Obj::Ref(num, 0)
                            }
                        },
                    };
                    dict.retain(|(k, _)| k != b"Resources");
                    dict.push((b"Resources".to_vec(), value));
                }
                dict.push(parent.clone());
                let mut body = Vec::new();
                write_dict(&mut body, &dict);
                each += body.len() + 64;
                if !within(once, each) {
                    return Ok(None);
                }
                bodies.push(body);
                last = dict;
            }
            if blank {
                let mut page = vec![(b"Type".to_vec(), Obj::Name(b"Page".to_vec())), parent.clone(), (b"Resources".to_vec(), Obj::Dict(Vec::new()))];
                for key in [&b"MediaBox"[..], b"CropBox", b"Rotate"] {
                    if let Some(value) = get(&last, key) {
                        page.push((key.to_vec(), value.clone()));
                    }
                }
                let mut body = Vec::new();
                write_dict(&mut body, &page);
                bodies.push(body);
            }
            let mut kids = Vec::with_capacity(count);
            for body in bodies.iter().cycle().take(count) {
                let num = next;
                next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
                new_object(&mut out, num, 0, &mut written);
                out.extend_from_slice(body);
                out.extend_from_slice(b"\nendobj\n");
                kids.push(Obj::Ref(num, 0));
            }
            new_object(&mut out, tree, 0, &mut written);
            let node = vec![
                (b"Type".to_vec(), Obj::Name(b"Pages".to_vec())),
                (b"Kids".to_vec(), Obj::Array(kids)),
                (b"Count".to_vec(), Obj::Num(count.to_string().into_bytes())),
            ];
            write_dict(&mut out, &node);
            out.extend_from_slice(b"\nendobj\n");
            let mut catalog = self.catalog.clone();
            // Page labels and the open action name pages of the old tree.
            catalog.retain(|(k, _)| k != b"Pages" && k != b"PageLabels" && k != b"OpenAction");
            catalog.push((b"Pages".to_vec(), Obj::Ref(tree, 0)));
            new_object(&mut out, root, root_generation, &mut written);
            write_dict(&mut out, &catalog);
            out.extend_from_slice(b"\nendobj\n");
        }
        if written.is_empty() {
            return Ok(Some(self.data.to_vec()));
        }
        written.sort_by_key(|(num, _, _)| *num);
        let mut trailer: Dict = Vec::new();
        for key in [&b"Root"[..], b"Info", b"ID", b"Encrypt"] {
            if let Some(v) = get(&self.trailer, key) {
                trailer.push((key.to_vec(), v.clone()));
            }
        }
        trailer.push((b"Prev".to_vec(), Obj::Num(self.startxref.to_string().into_bytes())));
        if self.xref_stream {
            let xref = next;
            next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
            let at = out.len();
            written.push((xref, 0, at));
            let width = (usize::BITS - at.leading_zeros()).div_ceil(8).max(1) as usize;
            let mut data = Vec::new();
            let mut index = Vec::new();
            for (num, generation, offset) in &written {
                index.extend([Obj::Num(num.to_string().into_bytes()), Obj::Num(b"1".to_vec())]);
                data.push(1u8);
                data.extend_from_slice(offset.to_be_bytes().get(std::mem::size_of::<usize>() - width..).unwrap_or(&[]));
                data.extend_from_slice(&generation.to_be_bytes());
            }
            trailer.extend([
                (b"Type".to_vec(), Obj::Name(b"XRef".to_vec())),
                (b"Size".to_vec(), Obj::Num(next.to_string().into_bytes())),
                (b"W".to_vec(), Obj::Array(vec![Obj::Num(b"1".to_vec()), Obj::Num(width.to_string().into_bytes()), Obj::Num(b"2".to_vec())])),
                (b"Index".to_vec(), Obj::Array(index)),
                (b"Length".to_vec(), Obj::Num(data.len().to_string().into_bytes())),
            ]);
            out.extend_from_slice(format!("{xref} 0 obj\n").as_bytes());
            write_dict(&mut out, &trailer);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&data);
            out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{at}\n%%EOF\n").as_bytes());
        } else {
            let at = out.len();
            out.extend_from_slice(b"xref\n");
            for (num, generation, offset) in &written {
                out.extend_from_slice(format!("{num} 1\n{offset:010} {generation:05} n\r\n").as_bytes());
            }
            trailer.push((b"Size".to_vec(), Obj::Num(next.to_string().into_bytes())));
            out.extend_from_slice(b"trailer\n");
            write_dict(&mut out, &trailer);
            out.extend_from_slice(format!("\nstartxref\n{at}\n%%EOF\n").as_bytes());
        }
        Ok(Some(out))
    }
}
