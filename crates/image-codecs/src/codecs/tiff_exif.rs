//! EXIF in a TIFF file: the Exif (34665) and GPS (34853) directories that IFD0 points to, plus
//! IFD0's descriptive tags (make, model, software, artist, …), the way cameras and Photoshop
//! write them.
//!
//! An EXIF block (`Metadata::exif`) is itself a small TIFF stream, so both directions go through
//! the same reader ([`super::tiff_ifd::File`]): reading a TIFF builds a block from the file's own
//! directories, writing one copies the block's directories into the file. Values are copied as
//! bytes (re-ordered when the byte orders differ), so tags PhotoCraft doesn't know survive.
//! Classic TIFF only on the reading side: an EXIF block has 4-byte offsets. What is copied is
//! bounded by the stream's own size and a fixed cap, because entries of a hostile file can
//! overlap (see [`read`]).

use tiff::encoder::{DirectoryEncoder, TiffEncoder, TiffKind};
use tiff::tags::{Tag, Type};
use tiff::{Directory, TiffResult};

use super::tiff_ifd::{File, Ifd};

/// IFD0's pointer to the Exif directory.
pub(crate) const TAG_EXIF_IFD: u16 = 34665;
/// IFD0's pointer to the GPS directory.
pub(crate) const TAG_GPS_IFD: u16 = 34853;
/// The Exif directory's PixelXDimension and PixelYDimension: the size of the image the
/// directory describes.
const TAG_PIXEL_X_DIMENSION: u16 = 40962;
const TAG_PIXEL_Y_DIMENSION: u16 = 40963;
/// The Exif directory's pointer to the Interoperability directory (not carried: it holds only
/// the interoperability index, and a copied pointer would point nowhere).
const TAG_INTEROP_IFD: u16 = 40965;
/// IFD0 tags an EXIF block carries next to its directories: ImageDescription, Make, Model,
/// Software, DateTime, Artist, Copyright. Structure, resolution and orientation belong to the
/// image and are written from it.
const IFD0_TAGS: [u16; 7] = [270, 271, 272, 305, 306, 315, 33432];
/// At most this many entries are copied from one directory.
const MAX_FIELDS: usize = 1024;
/// At most this many value bytes are copied from one stream, over all its directories. Real EXIF
/// is far smaller (the largest Nikon maker note seen is under 1 MB); the cap leaves room for
/// that and for a few such notes. A stream never yields more than its own length either (see
/// [`read`]).
const MAX_COPIED_BYTES: usize = 4 * 1024 * 1024;

/// One entry with its value bytes in the byte order of the stream it came from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub tag: u16,
    pub ty: u16,
    pub count: u32,
    pub value: Vec<u8>,
}

/// The EXIF parts of a TIFF stream, in its byte order (`le`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ExifDirs {
    pub le: bool,
    pub ifd0: Vec<Field>,
    pub exif: Vec<Field>,
    pub gps: Vec<Field>,
}

impl ExifDirs {
    /// Whether there is an Exif or GPS directory (IFD0's descriptive tags alone are not EXIF).
    pub fn has_directories(&self) -> bool {
        !self.exif.is_empty() || !self.gps.is_empty()
    }
}

/// The entries of `ifd` that `keep` accepts, with their values copied. An entry's value only has
/// to lie inside the file, so entries may overlap: `budget` is the number of value bytes that may
/// still be copied (shared by all the directories of one stream), and an entry whose value does
/// not fit in what is left is skipped, so later, smaller entries can still be kept.
fn fields(f: &File<'_>, ifd: &Ifd, budget: &mut usize, keep: impl Fn(u16) -> bool) -> Vec<Field> {
    let mut out = Vec::new();
    for e in ifd.entries.iter().filter(|e| keep(e.tag) && !matches!(e.tag, TAG_EXIF_IFD | TAG_GPS_IFD | TAG_INTEROP_IFD)) {
        if out.len() >= MAX_FIELDS {
            break;
        }
        let (Ok(count), Some(value)) = (u32::try_from(e.count), f.data(e)) else { continue };
        let Some(left) = budget.checked_sub(value.len()) else { continue };
        *budget = left;
        out.push(Field { tag: e.tag, ty: e.ty, count, value: value.to_vec() });
    }
    out
}

fn sub(f: &File<'_>, ifd: &Ifd, tag: u16) -> Option<Ifd> {
    let at = f.tag_uint(ifd, tag)?;
    (at != ifd.at).then(|| f.ifd(at)).flatten()
}

/// The EXIF parts of the classic TIFF stream `b` (a TIFF file, or an EXIF block without its
/// `Exif\0\0` prefix): IFD0's descriptive tags and the Exif and GPS directories.
///
/// The values copied add up to at most [`MAX_COPIED_BYTES`] and to at most `b.len()`: the values
/// of a well-formed stream are distinct bytes of it, while a hostile one can have many entries
/// point at the same bytes and so ask for far more than its size. Entries that do not fit in what
/// is left are left out (the small descriptive directories come first, so a huge maker note
/// cannot crowd out the GPS position).
pub(crate) fn read(b: &[u8]) -> Option<ExifDirs> {
    read_file(File::parse(b)?)
}

/// The EXIF parts of a camera raw built on TIFF (NEF, ARW, CR2, DNG, Panasonic RW2), as
/// [`read`] gives them, without the Exif directory's PixelXDimension and PixelYDimension: in a
/// raw they give the sensor's or the preview's size, not the developed image's.
pub(crate) fn read_raw(b: &[u8]) -> Option<ExifDirs> {
    let mut dirs = read_file(File::parse_raw(b)?)?;
    dirs.exif.retain(|f| !matches!(f.tag, TAG_PIXEL_X_DIMENSION | TAG_PIXEL_Y_DIMENSION));
    Some(dirs)
}

fn read_file(f: File<'_>) -> Option<ExifDirs> {
    let b = f.b;
    if f.big {
        return None;
    }
    let ifd0 = f.ifd(f.first_ifd)?;
    let mut budget = MAX_COPIED_BYTES.min(b.len());
    let ifd0_fields = fields(&f, &ifd0, &mut budget, |t| IFD0_TAGS.contains(&t));
    let mut dir = |tag: u16| sub(&f, &ifd0, tag).map(|d| fields(&f, &d, &mut budget, |_| true)).unwrap_or_default();
    let gps = dir(TAG_GPS_IFD);
    let exif = dir(TAG_EXIF_IFD);
    Some(ExifDirs { le: f.le, ifd0: ifd0_fields, exif, gps })
}

/// Bytes per element whose order a byte-order change reverses: a RATIONAL is two LONGs.
fn element(ty: u16) -> usize {
    match ty {
        3 | 8 => 2,
        4 | 5 | 9 | 10 | 11 | 13 => 4,
        12 | 16 | 17 | 18 => 8,
        _ => 1,
    }
}

/// `value` of type `ty` in byte order `to_le`, from `from_le`.
pub(crate) fn reorder(ty: u16, value: &[u8], from_le: bool, to_le: bool) -> Vec<u8> {
    let n = element(ty);
    if from_le == to_le || n == 1 {
        return value.to_vec();
    }
    value.chunks(n).flat_map(|c| c.iter().rev().copied()).collect()
}

/// An EXIF block: a classic TIFF stream in `dirs.le` order whose IFD0 holds `dirs.ifd0` and
/// points to the Exif and GPS directories.
pub(crate) fn block(dirs: &ExifDirs) -> Vec<u8> {
    let le = dirs.le;
    let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut ifd0: Vec<Field> = dirs.ifd0.clone();
    // Placeholders for the pointers; their offsets are known once the layout is.
    let pointer = |tag: u16| Field { tag, ty: 4, count: 1, value: vec![0; 4] };
    if !dirs.exif.is_empty() {
        ifd0.push(pointer(TAG_EXIF_IFD));
    }
    if !dirs.gps.is_empty() {
        ifd0.push(pointer(TAG_GPS_IFD));
    }
    // Layout: header, then each directory followed by its out-of-line values.
    let size = |fs: &[Field]| 2 + 12 * fs.len() + 4 + fs.iter().filter(|f| f.value.len() > 4).map(|f| f.value.len().next_multiple_of(2)).sum::<usize>();
    let at0 = 8;
    let at_exif = at0 + size(&ifd0);
    let at_gps = at_exif + if dirs.exif.is_empty() { 0 } else { size(&dirs.exif) };
    for f in &mut ifd0 {
        match f.tag {
            TAG_EXIF_IFD => f.value = u32b(at_exif as u32).to_vec(),
            TAG_GPS_IFD => f.value = u32b(at_gps as u32).to_vec(),
            _ => {}
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(if le { b"II" } else { b"MM" });
    out.extend_from_slice(&u16b(42));
    out.extend_from_slice(&u32b(at0 as u32));
    let write_dir = |out: &mut Vec<u8>, fs: &[Field]| {
        let mut fs = fs.to_vec();
        fs.sort_by_key(|f| f.tag);
        let start = out.len();
        let mut extra = start + 2 + 12 * fs.len() + 4;
        let mut values = Vec::new();
        out.extend_from_slice(&u16b(fs.len() as u16));
        for f in &fs {
            out.extend_from_slice(&u16b(f.tag));
            out.extend_from_slice(&u16b(f.ty));
            out.extend_from_slice(&u32b(f.count));
            if f.value.len() <= 4 {
                let mut inline = f.value.clone();
                inline.resize(4, 0);
                out.extend_from_slice(&inline);
            } else {
                out.extend_from_slice(&u32b(extra as u32));
                values.extend_from_slice(&f.value);
                if f.value.len() % 2 == 1 {
                    values.push(0);
                }
                extra += f.value.len().next_multiple_of(2);
            }
        }
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&values);
    };
    write_dir(&mut out, &ifd0);
    if !dirs.exif.is_empty() {
        write_dir(&mut out, &dirs.exif);
    }
    if !dirs.gps.is_empty() {
        write_dir(&mut out, &dirs.gps);
    }
    out
}

/// Writes `fields` (in byte order `from_le`) into a directory of `enc` that is not part of the
/// page chain, and returns its offset for the pointer tag.
pub(crate) fn write_directory<W: std::io::Write + std::io::Seek, K: TiffKind>(
    enc: &mut TiffEncoder<W, K>,
    fields: &[Field],
    from_le: bool,
) -> TiffResult<K::OffsetType> {
    let mut sub = enc.extra_directory()?;
    write_fields(&mut sub, fields, from_le)?;
    Ok(sub.finish_with_offsets()?.offset)
}

/// Adds `fields` (in byte order `from_le`) to directory `d`, in the file's (native) byte order.
pub(crate) fn write_fields<W: std::io::Write + std::io::Seek, K: TiffKind>(
    d: &mut DirectoryEncoder<'_, W, K>,
    fields: &[Field],
    from_le: bool,
) -> TiffResult<()> {
    let mut dir = Directory::empty();
    for f in fields {
        let Some(ty) = Type::from_u16(f.ty) else { continue };
        let entry = d.write_entry_bytes(ty, &reorder(f.ty, &f.value, from_le, cfg!(target_endian = "little")))?;
        dir.extend([(Tag::from_u16_exhaustive(f.tag), entry)]);
    }
    d.extend_from(&dir);
    Ok(())
}
