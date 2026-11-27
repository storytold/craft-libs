//! Pictures, from the Office Drawing records ([MS-ODRAW]): the workbook's
//! MsoDrawingGroup holds the pictures (a BStore of FBSE entries, each with its BLIP), and each
//! sheet's MsoDrawing records hold its shapes; a shape whose properties name a BLIP (`pib`,
//! 0x0104) and that has a client anchor (cells and fractions of them) is a picture.

use std::sync::Arc;

use craft_officeart::props::{entries, prop};
use craft_officeart::record::{Rec, Records};
use craft_officeart::rt;

use crate::model::ClientAnchor;

/// Pictures read from one workbook at most.
const MAX_BLIPS: usize = 4096;
/// Bytes inflated from all of a workbook's metafiles together.
const MAX_METAFILES: usize = 256 << 20;
/// Nesting of drawing containers followed at most.
const MAX_DEPTH: usize = 16;

/// One stored picture: its bytes (shared by every shape that shows it) and MIME type, or `None`
/// for a format that is not read (PICT, TIFF) or a damaged one, so that `pib` indexes still
/// line up.
pub(crate) type Blip = Option<(Arc<Vec<u8>>, &'static str)>;

/// The pictures of the workbook's MsoDrawingGroup, in BStore order (`pib` 1 is the first).
pub(crate) fn blip_store(dgg: &[u8]) -> Vec<Blip> {
    let mut out = Vec::new();
    let mut inflate_left = MAX_METAFILES;
    for dgg in Records::new(dgg).filter(|r| r.kind == rt::DGG_CONTAINER) {
        for store in Records::new(dgg.data).filter(|r| r.kind == rt::BSTORE_CONTAINER) {
            for fbse in Records::new(store.data).filter(|r| r.kind == rt::FBSE) {
                if out.len() < MAX_BLIPS {
                    out.push(fbse_blip(fbse.data, &mut inflate_left).map(|(b, m)| (Arc::new(b), m)));
                }
            }
        }
    }
    out
}

/// A picture's bytes and MIME type.
type Bytes = Option<(Vec<u8>, &'static str)>;

/// The picture of one FBSE: 36 bytes of description, its name, then the BLIP record.
/// `inflate_left` is what the workbook's metafiles may still inflate to.
fn fbse_blip(fbse: &[u8], inflate_left: &mut usize) -> Bytes {
    let name_len = usize::from(*fbse.get(33)?);
    let blip = Rec::at(fbse, 36usize.checked_add(name_len)?)?;
    let (ty, instance, data) = (blip.kind, blip.instance, blip.data);
    // EMF and WMF: one or two identifiers, a metafile header, then the (usually zlib) bytes.
    match (ty, instance) {
        (0xF01A, 0x3D4) => return metafile(data, 1, false, inflate_left),
        (0xF01A, 0x3D5) => return metafile(data, 2, false, inflate_left),
        (0xF01B, 0x216) => return metafile(data, 1, true, inflate_left),
        (0xF01B, 0x217) => return metafile(data, 2, true, inflate_left),
        // DIB: identifiers, a tag byte, then a BITMAPINFOHEADER and the pixels.
        (0xF01F, 0x7A8) => return dib_as_bmp(data.get(17..)?),
        (0xF01F, 0x7A9) => return dib_as_bmp(data.get(33..)?),
        _ => {}
    }
    // PNG and JPEG keep one or two 16-byte identifiers and a tag byte before the bytes.
    let (uids, mime) = match (ty, instance) {
        (0xF01E, 0x6E0) => (1, "image/png"),
        (0xF01E, 0x6E1) => (2, "image/png"),
        (0xF01D | 0xF02A, 0x46A | 0x6E2) => (1, "image/jpeg"),
        (0xF01D | 0xF02A, 0x46B | 0x6E3) => (2, "image/jpeg"),
        _ => return None,
    };
    let bytes = data.get(16 * uids + 1..)?;
    (!bytes.is_empty()).then(|| (bytes.to_vec(), mime))
}

/// Largest metafile inflated from one BLIP.
const MAX_METAFILE: usize = 64 << 20;

/// An EMF or WMF BLIP's file: after `uids` identifiers comes the 34-byte OfficeArtMetafileHeader
/// (uncompressed size, bounds, size in EMUs, saved size, compression 0 = DEFLATE in a zlib wrapper
/// or 0xFE = none, filter), then the saved bytes. A WMF is stored without its placeable header;
/// one is put back from the bounds so the picture keeps its size.
fn metafile(data: &[u8], uids: usize, wmf: bool, inflate_left: &mut usize) -> Bytes {
    let h = data.get(16 * uids..16 * uids + 34)?;
    let u32_at = |at: usize| Some(u32::from_le_bytes(h.get(at..at + 4)?.try_into().ok()?));
    let size = usize::try_from(u32_at(0)?).ok()?;
    let saved = usize::try_from(u32_at(28)?).ok()?;
    let body = data.get(16 * uids + 34..)?;
    let body = body.get(..saved.min(body.len()))?;
    let bytes = match *h.get(32)? {
        0x00 => {
            let out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(body, size.min(MAX_METAFILE).min(*inflate_left)).ok()?;
            *inflate_left = inflate_left.saturating_sub(out.len());
            out
        }
        0xFE => body.to_vec(),
        _ => return None,
    };
    if bytes.is_empty() {
        return None;
    }
    if !wmf {
        return Some((bytes, "image/x-emf"));
    }
    Some((craft_officeart::wmf::with_placeable_header(h, bytes)?, "image/x-wmf"))
}

/// A packed DIB (header, palette, pixels) as a BMP file: the 14-byte file header in front, its
/// pixel offset counted from the info header's size and palette.
fn dib_as_bmp(dib: &[u8]) -> Bytes {
    let u32_at = |at: usize| Some(u32::from_le_bytes(dib.get(at..at + 4)?.try_into().ok()?));
    let header = u32_at(0)?;
    if !(12..=124).contains(&header) || dib.len() <= header as usize {
        return None;
    }
    let palette = if header == 12 {
        0
    } else {
        let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
        let used = u32_at(32)?;
        let colors = if used > 0 {
            used.min(256)
        } else if bits <= 8 {
            1 << bits
        } else {
            0
        };
        // BI_BITFIELDS with a 40-byte header keeps its three masks after it.
        let masks = if header == 40 && u32_at(16)? == 3 { 12 } else { 0 };
        colors * 4 + masks
    };
    let offset = 14 + header + palette;
    let total = u32::try_from(dib.len()).ok()?.checked_add(14)?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(dib);
    Some((out, "image/bmp"))
}

/// A picture shape: its BLIP (1-based) and client anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PictureShape {
    pub pib: u32,
    pub anchor: ClientAnchor,
}

/// The picture shapes in a sheet's drawing (its MsoDrawing records joined).
pub(crate) fn picture_shapes(dg: &[u8]) -> Vec<PictureShape> {
    let mut out = Vec::new();
    walk(dg, 0, &mut out);
    out
}

fn walk(b: &[u8], depth: usize, out: &mut Vec<PictureShape>) {
    if depth > MAX_DEPTH {
        return;
    }
    for r in Records::new(b) {
        if r.kind == rt::SP_CONTAINER {
            if let Some(p) = shape(r.data) {
                out.push(p);
            }
        } else if r.is_container() {
            walk(r.data, depth + 1, out);
        }
    }
}

/// A shape container's BLIP and anchor, when it has both.
fn shape(sp: &[u8]) -> Option<PictureShape> {
    let (mut pib, mut anchor) = (None, None);
    for r in Records::new(sp) {
        let body = r.data;
        match r.kind {
            // Properties: `instance` of them, six bytes each (id with flags, value).
            rt::FOPT => {
                if let Some(e) = entries(body, usize::from(r.instance)).filter(|e| e.pid == prop::PIB).last() {
                    pib = Some(e.op);
                }
            }
            // Client anchor: flags, then eight 16-bit values.
            rt::CLIENT_ANCHOR => {
                let mut v = [0u16; 8];
                for (i, slot) in v.iter_mut().enumerate() {
                    let at = 2 + i * 2;
                    *slot = u16::from_le_bytes([*body.get(at)?, *body.get(at + 1)?]);
                }
                let [col1, dx1, row1, dy1, col2, dx2, row2, dy2] = v;
                anchor = Some(ClientAnchor { col1, dx1, row1, dy1, col2, dx2, row2, dy2 });
            }
            _ => {}
        }
    }
    Some(PictureShape { pib: pib.filter(|p| *p > 0)?, anchor: anchor? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(vi: u16, ty: u16, body: &[u8]) -> Vec<u8> {
        [&vi.to_le_bytes()[..], &ty.to_le_bytes(), &(body.len() as u32).to_le_bytes(), body].concat()
    }

    #[test]
    fn pictures_and_their_anchors_are_found_and_hostile_drawings_are_harmless() {
        let png = [0x89, b'P', b'N', b'G'];
        let blip = rec(0x6E00, 0xF01E, &[&[0u8; 17][..], &png].concat());
        let fbse = rec(0x0062, 0xF007, &[&[0u8; 36][..], &blip].concat());
        let wmf = rec(0x0062, 0xF007, &[&[0u8; 36][..], &rec(0x2160, 0xF01B, &[0; 60])].concat());
        let dgg = rec(0x000F, 0xF000, &rec(0x002F, 0xF001, &[fbse, wmf].concat()));
        let store = blip_store(&dgg);
        assert_eq!(store.len(), 2);
        assert_eq!(store[0], Some((Arc::new(png.to_vec()), "image/png")));
        assert_eq!(store[1], None, "a damaged metafile keeps its slot");
        // A shape with pib 1 and an anchor from B2 (half way across) to D5.
        let opt = rec(0x0013, 0xF00B, &[&0x4104u16.to_le_bytes()[..], &1u32.to_le_bytes()].concat());
        let anchor_body: Vec<u8> = [0u16, 1, 512, 1, 0, 3, 0, 4, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let sp = rec(0x000F, 0xF004, &[opt, rec(0x0000, 0xF010, &anchor_body)].concat());
        let dg = rec(0x000F, 0xF002, &rec(0x000F, 0xF003, &sp));
        let anchor = ClientAnchor { col1: 1, dx1: 512, row1: 1, dy1: 0, col2: 3, dx2: 0, row2: 4, dy2: 0 };
        assert_eq!(picture_shapes(&dg), vec![PictureShape { pib: 1, anchor }]);
        // Truncated or nonsense drawings give nothing, never a panic.
        for bad in [&[][..], &[0xFF; 8][..], &[0; 70][..]] {
            assert_eq!(metafile(bad, 1, false, &mut usize::MAX.clone()), None);
            let _ = metafile(bad, 2, true, &mut usize::MAX.clone());
            let _ = dib_as_bmp(bad);
        }
        for bad in [&dg[..dg.len() - 5], &[0xFF; 40][..], &[][..], &dgg[..20]] {
            let _ = picture_shapes(bad);
            let _ = blip_store(bad);
        }
    }

    #[test]
    fn metafiles_share_one_inflate_allowance() {
        // A zlib metafile of 1000 zero bytes, as an EMF BLIP with one identifier.
        let raw = vec![0u8; 1000];
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let mut header = Vec::new();
        for v in [1000u32, 0, 0, 10, 10, 0, 0, z.len() as u32] {
            header.extend_from_slice(&v.to_le_bytes());
        }
        header.extend_from_slice(&[0x00, 0xFE]);
        let data = [&[0u8; 16][..], &header, &z].concat();
        let mut left = 1500;
        assert_eq!(metafile(&data, 1, false, &mut left).map(|(b, _)| b.len()), Some(1000));
        assert_eq!(left, 500);
        // The second would inflate past what is left: refused, not cut.
        assert_eq!(metafile(&data, 1, false, &mut left), None);
    }

    #[test]
    fn a_wmf_gets_its_placeable_header_back_and_a_dib_becomes_a_bmp() {
        // Bounds 0,0–2000×1000 units over 2 × 1 inches (EMUs): 1000 units an inch; stored as is.
        let wmf = b"WMF records".to_vec();
        let mut header = Vec::new();
        for v in [wmf.len() as u32, 0, 0, 2000, 1000, 2 * 914_400, 914_400, wmf.len() as u32] {
            header.extend_from_slice(&v.to_le_bytes());
        }
        header.extend_from_slice(&[0xFE, 0xFE]);
        let data = [&[0u8; 16][..], &header, &wmf].concat();
        let (bytes, mime) = metafile(&data, 1, true, &mut usize::MAX.clone()).expect("wmf");
        assert_eq!(mime, "image/x-wmf");
        assert_eq!(&bytes[..4], &0x9AC6_CDD7u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 1000, "units per inch");
        let sum = bytes[..20].chunks(2).fold(0u16, |x, w| x ^ u16::from_le_bytes([w[0], w[1]]));
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), sum);
        assert_eq!(&bytes[22..], &wmf[..]);
        // A 1 × 1, 24-bit DIB: the pixels start right after the file and info headers.
        let mut dib = Vec::new();
        for v in [40u32, 1, 1] {
            dib.extend_from_slice(&v.to_le_bytes());
        }
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        dib.extend_from_slice(&[0, 0, 255, 0]);
        let (bmp, mime) = dib_as_bmp(&dib).expect("bmp");
        assert_eq!(mime, "image/bmp");
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(bmp.len(), 14 + dib.len());
    }
}
