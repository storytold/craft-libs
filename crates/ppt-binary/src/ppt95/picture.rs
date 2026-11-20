//! PowerPoint 95 pictures, learned from files PowerPoint 95 wrote.
//!
//! The Environment (1010) holds the document's pictures in a slot list (2006), as it holds its
//! fonts and rulers: each slot is what comes before a 2017 record. A slot's picture is a 4028
//! container holding a 4037, which holds a header (4038) and the picture itself (4033): always a
//! Windows metafile without a placeable header, whatever was inserted (a BMP, a JPEG and a PNG all
//! became metafiles drawing the bitmap). Inserting the same file again reuses its slot.
//!
//! A picture on a page is a shape (3008, a rectangle drawn with its format as any shape is) holding
//! a 4014 container: 4015, the crop (left, top, right and bottom as i32s, in ten-thousandths of the
//! picture), and 4053 → 4036, the picture's slot number. The shape's box is the cropped picture's.
//! (On a notes page, 4014 → 4054 → 4032 names the slide its slide image shows instead.) Each of the
//! four crops was seen set: left and top in one file, right and bottom in another.
//!
//! The metafile is kept as it is, as `image/x-wmf` media, as the PowerPoint 97 path keeps the
//! metafiles of its blip store. Each slot is read once, the first time a shape names it, and all of
//! them together within [`crate::blip::MAX_TOTAL`] bytes, the PowerPoint 97 path's budget.

use std::collections::HashMap;

use craft_pptx::{MediaId, Presentation};

use super::Rec;
use super::text::slots;
use crate::record::u16_at;

/// Record types.
pub(super) mod rt {
    pub const ENVIRONMENT: u32 = 1010;
    pub const PICTURES: u32 = 2006;
    pub const PICTURE: u32 = 4028;
    pub const PICTURE_BODY: u32 = 4037;
    pub const METAFILE: u32 = 4033;
    pub const LINK: u32 = 4014;
    pub const CROP: u32 = 4015;
    pub const PICTURE_LINK: u32 = 4053;
    pub const PICTURE_INDEX: u32 = 4036;
}

/// A WMF's standard header: 9 words.
const WMF_HEADER: usize = 18;
/// Crop values are ten-thousandths of the picture.
const CROP_UNIT: f64 = 10_000.0;

/// What a shape says about its picture: the slot it names and its crop (left, top, right, bottom,
/// as fractions of the picture).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PictureRef {
    pub slot: u32,
    pub crop: [f64; 4],
}

/// A shape's picture link (4014 → 4053 → 4036), with its crop. `None` for a shape that shows no
/// picture, a notes page's slide image included.
pub(super) fn picture_ref(shape: &Rec) -> Option<PictureRef> {
    let link = shape.child(rt::LINK)?;
    let slot = link.child(rt::PICTURE_LINK)?.child(rt::PICTURE_INDEX)?.u32(0)?;
    let mut crop = [0.0; 4];
    if let Some(c) = link.child(rt::CROP) {
        for (i, v) in crop.iter_mut().enumerate() {
            *v = c.i32(i * 4).map_or(0.0, |n| (f64::from(n) / CROP_UNIT).clamp(-1.0, 1.0));
        }
    }
    Some(PictureRef { slot, crop })
}

/// The document's pictures: each slot's metafile record, made media only when a shape first names
/// it, every slot at most once, and all of them within a byte budget.
pub(super) struct Pictures<'a> {
    slots: Vec<Option<Rec<'a>>>,
    media: HashMap<u32, Option<MediaId>>,
    budget: usize,
}

impl<'a> Pictures<'a> {
    pub(super) fn new(document: &Rec<'a>, budget: usize) -> Pictures<'a> {
        let slots = document
            .child(rt::ENVIRONMENT)
            .and_then(|e| e.child(rt::PICTURES))
            .map(|list| slots(&list, rt::PICTURE, |p| p.child(rt::PICTURE_BODY)?.child(rt::METAFILE)))
            .unwrap_or_default();
        Pictures { slots, media: HashMap::new(), budget }
    }

    /// The media of the picture in `slot`. `None` for a slot that is missing or empty, whose
    /// metafile can't be read, or that would pass the budget.
    pub(super) fn media(&mut self, p: &mut Presentation, slot: u32) -> Option<MediaId> {
        if let Some(m) = self.media.get(&slot) {
            return *m;
        }
        let id = self.read(p, slot);
        self.media.insert(slot, id);
        id
    }

    fn read(&mut self, p: &mut Presentation, slot: u32) -> Option<MediaId> {
        let record = self.slots.get(usize::try_from(slot).ok()?).copied().flatten()?;
        let bytes = metafile(record.data)?;
        if bytes.len() > self.budget {
            log::warn!(
                "ppt: a {}-byte PowerPoint 95 picture passes the {} bytes left of the pictures' budget; it is left out",
                bytes.len(),
                self.budget
            );
            return None;
        }
        self.budget -= bytes.len();
        p.add_media(&format!("image{}.wmf", u64::from(slot) + 1), "image/x-wmf", bytes.to_vec())
    }
}

/// A metafile record's bytes as a WMF file: a standard header (a type of 1 or 2 and a header of 9
/// words), cut to the size the header gives when the record holds more. `None` for anything else,
/// a header giving a size smaller than itself included.
fn metafile(data: &[u8]) -> Option<&[u8]> {
    let (kind, words) = (u16_at(data, 0)?, u16_at(data, 2)?);
    if !matches!(kind, 1 | 2) || words != 9 || data.len() < WMF_HEADER {
        return None;
    }
    // The file's size in words.
    let size = crate::record::u32_at(data, 6).and_then(|w| usize::try_from(w).ok()).and_then(|w| w.checked_mul(2))?;
    if size < WMF_HEADER {
        return None;
    }
    Some(data.get(..size).unwrap_or(data))
}
