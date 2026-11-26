//! Pictures ([MS-ODRAW] §2.2.23–2.2.32, §2.2.32–2.2.45): a BLIP record holds one picture, a
//! metafile (EMF, WMF, PICT; usually deflated) or a bitmap file (JPEG, PNG, DIB, TIFF). The drawing
//! group's BLIP store lists every picture (`FBSE`), each holding its BLIP or pointing into a delay
//! stream; shapes name a picture by its 1-based index in the store.
//!
//! Every byte a picture produces, inflated or copied, is charged to a [`Decode`] budget before it
//! is produced. Each reader sets its budget's total: that is its format's policy.

use crate::dib;
use crate::record::{Rec, u32_at};
use crate::rt;

/// Largest picture inflated from one compressed metafile.
pub const MAX_INFLATED: usize = 64 << 20;
/// Most BLIP store entries read.
pub const MAX_ENTRIES: usize = 4096;
/// The `.ppt` reader's budget: 2 GiB, the total a `.pptx` package may inflate to
/// (`craft_pptx::MAX_TOTAL`). Pictures are the only part of a `.ppt` that inflates, so a small
/// hostile file can't make the import hold more than a `.pptx` could. It also fits a 32-bit
/// `usize` (wasm32), where a 4 GiB constant wraps to 0.
pub const MAX_TOTAL: usize = 2 << 30;
// A zero or tiny budget would drop every picture without a word, and the shift above can wrap on
// a 32-bit target (wasm32) without any error, so the build checks it on every target.
const _: () = assert!(MAX_TOTAL >= MAX_INFLATED);

/// A budget charged before the work is done: bytes of inflating, copying and pixel decoding left,
/// and how many attempts it has refused (only the first few are logged).
pub struct Decode {
    pub left: usize,
    refused: usize,
}

impl Decode {
    pub fn new(left: usize) -> Self {
        Decode { left, refused: 0 }
    }

    /// Take `cost` off the budget; `false` (and a log line) when it doesn't fit.
    pub fn charge(&mut self, cost: usize, what: &str) -> bool {
        match self.left.checked_sub(cost) {
            Some(left) if cost > 0 => {
                self.left = left;
                true
            }
            _ => {
                if self.refused < 10 {
                    log::warn!(
                        "officeart: a {what} needing {cost} bytes of decode work is over the decode budget ({} left) and was not decoded",
                        self.left
                    );
                }
                self.refused = self.refused.saturating_add(1);
                false
            }
        }
    }
}

/// The kind of picture a BLIP holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Emf,
    Wmf,
    /// Macintosh PICT.
    Pict,
    Jpeg,
    Png,
    /// A device-independent bitmap: a `BITMAPINFOHEADER` and its pixels, without a file header.
    Dib,
    Tiff,
}

impl Format {
    /// The file extension and content type of the picture as a file (a DIB once it is a BMP file).
    pub fn file(self) -> (&'static str, &'static str) {
        match self {
            Format::Emf => ("emf", "image/x-emf"),
            Format::Wmf => ("wmf", "image/x-wmf"),
            Format::Pict => ("pict", "image/pict"),
            Format::Jpeg => ("jpg", "image/jpeg"),
            Format::Png => ("png", "image/png"),
            Format::Dib => ("bmp", "image/bmp"),
            Format::Tiff => ("tif", "image/tiff"),
        }
    }
}

/// A BLIP record, read: its format and where its picture's bytes are.
#[derive(Clone, Copy, Debug)]
pub struct Blip<'a> {
    pub format: Format,
    /// A metafile's 34-byte `OfficeArtMetafileHeader` (uncompressed size, bounds, size in EMUs,
    /// saved size, compression, filter); `None` for a bitmap.
    pub header: Option<&'a [u8]>,
    /// The saved bytes: a metafile's (deflated when `compressed`), cut to its saved size, or the
    /// bitmap file.
    data: &'a [u8],
    compressed: bool,
}

/// Read a BLIP record ([MS-ODRAW] §2.2.23–2.2.31). Each type has two instances, the second
/// carrying a second 16-byte UID; any other instance, a metafile whose compression is neither
/// deflate (0) nor none (0xFE), or a picture with no bytes, is no picture.
pub fn parse<'a>(rec: &Rec<'a>) -> Option<Blip<'a>> {
    let (format, first) = match (rec.kind, rec.instance) {
        (rt::BLIP_EMF, _) => (Format::Emf, 0x3D4),
        (rt::BLIP_WMF, _) => (Format::Wmf, 0x216),
        (rt::BLIP_PICT, _) => (Format::Pict, 0x542),
        (rt::BLIP_JPEG | rt::BLIP_JPEG_CMYK, 0x46A | 0x46B) => (Format::Jpeg, 0x46A),
        (rt::BLIP_JPEG | rt::BLIP_JPEG_CMYK, _) => (Format::Jpeg, 0x6E2),
        (rt::BLIP_PNG, _) => (Format::Png, 0x6E0),
        (rt::BLIP_DIB, _) => (Format::Dib, 0x7A8),
        (rt::BLIP_TIFF, _) => (Format::Tiff, 0x6E4),
        _ => return None,
    };
    let uids = match rec.instance.checked_sub(first)? {
        0 => 16,
        1 => 32,
        _ => return None,
    };
    let blip = if matches!(format, Format::Emf | Format::Wmf | Format::Pict) {
        let header = rec.data.get(uids..uids + 34)?;
        let saved = u32_at(header, 28)? as usize;
        let compressed = match *header.get(32)? {
            0 => true,
            0xFE => false,
            _ => return None,
        };
        let raw = rec.data.get(uids + 34..)?;
        Blip { format, header: Some(header), data: raw.get(..saved.min(raw.len()))?, compressed }
    } else {
        // A one-byte tag, then the file.
        Blip { format, header: None, data: rec.data.get(uids + 1..)?, compressed: false }
    };
    Some(blip).filter(|b| !b.data.is_empty())
}

impl<'a> Blip<'a> {
    /// The saved bytes as they are: a bitmap file, a DIB, or a metafile's (deflated or not).
    pub fn raw(&self) -> &'a [u8] {
        self.data
    }

    /// The picture's bytes: a metafile inflated or stored, cut to its uncompressed size and at most
    /// [`MAX_INFLATED`] bytes, or a bitmap file as it is; a DIB
    /// stays a DIB ([`dib`] makes it a file). At most `limit` bytes, all charged to `work` before
    /// they are made. Inflating is charged the compressed bytes it reads and the size it may
    /// produce; what it didn't produce is given back when it succeeds, and kept when it fails, so
    /// a damaged stream shown by many entries can't be inflated again and again for free. A
    /// compressed metafile whose header gives no uncompressed size inflates to nothing.
    pub fn bytes(&self, limit: usize, work: &mut Decode) -> Option<Vec<u8>> {
        let cap = limit.min(MAX_INFLATED);
        // The uncompressed size, when the header gives one.
        let size = self.header.and_then(|h| u32_at(h, 0)).map(|s| s as usize).filter(|&s| s > 0);
        let out = if self.compressed {
            let want = size?.min(cap);
            if !work.charge(self.data.len().saturating_add(want), "metafile") {
                return None;
            }
            let out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(self.data, want).ok()?;
            work.left = work.left.saturating_add(want - out.len().min(want));
            out
        } else {
            let stored = self.data.get(..size.map_or(self.data.len(), |s| s.min(self.data.len())))?;
            let most = if self.header.is_some() { cap } else { limit };
            if stored.len() > most || !work.charge(stored.len(), "picture") {
                return None;
            }
            stored.to_vec()
        };
        Some(out).filter(|b| !b.is_empty())
    }
}

/// A decoded picture: file extension, content type and bytes as a file of that type.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub ext: &'static str,
    pub content_type: &'static str,
    pub data: Vec<u8>,
}

/// A BLIP as a picture file within `limit` bytes and `work`: a DIB becomes a BMP file. `None` for a
/// Macintosh PICT, which has no decoder here.
pub fn picture(blip: &Blip, limit: usize, work: &mut Decode) -> Option<Picture> {
    if blip.format == Format::Pict {
        return None;
    }
    // A DIB is copied once, into its BMP file, charged whole.
    let data = if blip.format == Format::Dib { dib::to_bmp_within(blip.raw(), limit, work)? } else { blip.bytes(limit, work)? };
    let (ext, content_type) = blip.format.file();
    Some(Picture { ext, content_type, data })
}

/// A WMF BLIP as a `.wmf` file: its records, with the placeable header the BLIP leaves out put
/// back so the picture keeps its size, within `limit` bytes and `work`. Putting the header in
/// front copies the records again, so that copy is charged whole. When it doesn't fit, or the
/// bounds can't make a placeable header, the records are kept as they are: a WMF without one.
pub fn with_placeable_header(blip: &Blip, limit: usize, work: &mut Decode) -> Option<Vec<u8>> {
    let records = blip.bytes(limit, work)?;
    let Some(header) = blip.header.filter(|h| crate::wmf::gets_header(h)) else { return Some(records) };
    let whole = records.len().checked_add(crate::wmf::PLACEABLE_HEADER)?;
    if whole > limit || !work.charge(whole, "metafile") {
        return Some(records);
    }
    crate::wmf::with_placeable_header(header, records)
}

/// The BLIP record of an `OfficeArtFBSE` body: embedded after it, or in the delay stream, where
/// its (`foDelay`, record length) location is returned too (entries can share one record,
/// whatever sizes their FBSEs claim: the record's own header is what counts). An entry whose size
/// is 0 holds no picture.
pub fn fbse_record<'a>(fbse: &'a [u8], delay: &'a [u8]) -> Option<(Rec<'a>, Option<(usize, usize)>)> {
    // btWin32, btMacOS, rgbUid[16], tag, size, cRef, foDelay, unused, cbName, unused×2.
    if u32_at(fbse, 20)? == 0 {
        return None;
    }
    let fo_delay = u32_at(fbse, 28)? as usize;
    let cb_name = *fbse.get(33)? as usize;
    let at = 36usize.checked_add(cb_name)?;
    if let Some(rec) = Rec::at(fbse, at) {
        return Some((rec, None));
    }
    // Not embedded: the record sits in the delay stream.
    let rec = Rec::at(delay, fo_delay)?;
    Some((rec, Some((fo_delay, rec.span()))))
}

/// The `OfficeArtFBSE` bodies of a BLIP store container's body, in order (`pib` 1 is the first),
/// at most [`MAX_ENTRIES`].
pub fn store_entries(bstore: &[u8]) -> impl Iterator<Item = &[u8]> {
    crate::record::Records::new(bstore).filter(|r| r.kind == rt::FBSE).map(|r| r.data).take(MAX_ENTRIES)
}

/// The BLIP record of a `.ppt` store entry: embedded when bytes follow the FBSE's name, and in the
/// delay stream only when none do, so an embedded BLIP that can't be read is no picture (unlike
/// [`fbse_record`], `.doc`'s rule, which then looks in the delay stream).
fn entry_record<'a>(fbse: &'a [u8], delay: &'a [u8]) -> Option<Rec<'a>> {
    // btWin32, btMacOS, rgbUid[16], tag, size, cRef, foDelay, unused, cbName, unused×2.
    if u32_at(fbse, 20)? == 0 {
        return None;
    }
    let fo_delay = u32_at(fbse, 28)? as usize;
    let at = 36usize.checked_add(*fbse.get(33)? as usize)?;
    if fbse.len() > at { Rec::at(fbse, at) } else { Rec::at(delay, fo_delay) }
}

/// The BLIP store: each entry's BLIP record, decoded only when a shape first asks for it, every
/// entry at most once, and all of them together within a budget.
pub struct Store<'a> {
    entries: Vec<Option<Rec<'a>>>,
    work: Decode,
}

impl<'a> Store<'a> {
    /// The store of a drawing group (`OfficeArtDggContainer`), its delay stream (`.ppt`'s
    /// `Pictures` stream) and its budget.
    pub fn new(dgg: Option<Rec<'a>>, delay: &'a [u8], budget: usize) -> Self {
        let entries = match dgg.and_then(|d| d.child(rt::BSTORE_CONTAINER)) {
            // One slot per child, so a record that isn't an FBSE keeps its `pib` and the ones
            // after it don't shift.
            Some(bstore) => crate::record::Records::new(bstore.data)
                .take(MAX_ENTRIES)
                .map(|r| if r.kind == rt::FBSE { entry_record(r.data, delay) } else { None })
                .collect(),
            None => vec![],
        };
        Store { entries, work: Decode::new(budget) }
    }

    /// The picture at `index` (0-based; shapes use 1-based ids), taken out of the store. `None`
    /// for an entry that is empty, can't be read, was taken already, or would pass the budget.
    pub fn take(&mut self, index: usize) -> Option<Picture> {
        let blip = self.entries.get_mut(index)?.take()?;
        let limit = self.work.left;
        picture(&parse(&blip)?, limit, &mut self.work)
    }

    /// A BLIP held in a shape's own property ([MS-ODRAW] §2.2.37: a complex `fillBlip` or `pib`
    /// whose extra bytes are the BLIP record itself), decoded within the same budget.
    pub fn decode_inline(&mut self, bytes: &[u8]) -> Option<Picture> {
        let blip = Rec::at(bytes, 0)?;
        let limit = self.work.left;
        picture(&parse(&blip)?, limit, &mut self.work)
    }

    /// What is left of the budget.
    pub fn left(&self) -> usize {
        self.work.left
    }
}

#[cfg(test)]
mod tests;
