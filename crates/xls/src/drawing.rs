//! Pictures, from the Office Drawing records ([MS-ODRAW]): the workbook's
//! MsoDrawingGroup holds the pictures (a BStore of FBSE entries, each with its BLIP), and each
//! sheet's MsoDrawing records hold its shapes; a shape whose properties name a BLIP (`pib`,
//! 0x0104) and that has a client anchor (cells and fractions of them) is a picture.

use std::sync::Arc;

use craft_officeart::blip::{Decode, Format, fbse_record, parse, picture, store_entries};
use craft_officeart::props::{entries, prop};
use craft_officeart::record::{MAX_DEPTH, Records};
use craft_officeart::rt;

use crate::model::ClientAnchor;

/// Bytes all of a workbook's pictures may produce together, inflated or copied.
const MAX_PICTURE_BYTES: usize = 256 << 20;

/// One stored picture: its bytes (shared by every shape that shows it) and MIME type, or `None`
/// for a format that is not read (PICT, TIFF) or a damaged one, so that `pib` indexes still
/// line up.
pub(crate) type Blip = Option<(Arc<Vec<u8>>, &'static str)>;

/// The pictures of the workbook's MsoDrawingGroup, in BStore order (`pib` 1 is the first).
pub(crate) fn blip_store(dgg: &[u8]) -> Vec<Blip> {
    let mut out = Vec::new();
    let mut work = Decode::new(MAX_PICTURE_BYTES);
    for dgg in Records::new(dgg).filter(|r| r.kind == rt::DGG_CONTAINER) {
        for store in Records::new(dgg.data).filter(|r| r.kind == rt::BSTORE_CONTAINER) {
            for fbse in store_entries(store.data) {
                if out.len() < craft_officeart::blip::MAX_ENTRIES {
                    out.push(fbse.and_then(|f| fbse_blip(f, &mut work)).map(|(b, m)| (Arc::new(b), m)));
                }
            }
        }
    }
    out
}

/// A picture's bytes and MIME type.
type Bytes = Option<(Vec<u8>, &'static str)>;

/// The picture of one FBSE (its BLIP record after its description and name), within `work`: a
/// metafile inflated, a WMF with its placeable header put back so the picture keeps its size, a
/// DIB decoded and made a PNG (or a BMP when it can't be decoded). PICT and TIFF are not read.
fn fbse_blip(fbse: &[u8], work: &mut Decode) -> Bytes {
    let (rec, _) = fbse_record(fbse, &[])?;
    let blip = parse(&rec)?;
    if matches!(blip.format, Format::Pict | Format::Tiff) {
        return None;
    }
    let p = picture(&blip, usize::MAX, work)?;
    Some((p.data, p.content_type))
}

/// A picture shape: its BLIP (1-based) and client anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PictureShape {
    pub pib: u32,
    pub anchor: ClientAnchor,
}

/// The picture shapes in a sheet's drawing (its MsoDrawing records joined), reading at most
/// `budget` records more (one budget for the whole workbook, [`craft_officeart::record::MAX_RECORDS`] to start with),
/// and whether the budget ran out before the drawing's last record or a container nested past
/// [`MAX_DEPTH`] was left unread, so shapes may be missing.
pub(crate) fn picture_shapes(dg: &[u8], budget: &mut usize) -> (Vec<PictureShape>, bool) {
    let mut out = Vec::new();
    let mut cut = false;
    walk(dg, 0, &mut out, budget, &mut cut);
    (out, cut)
}

/// The picture shapes in containers nested at most [`MAX_DEPTH`] deep, reading at most `budget`
/// records more (shared by the whole workbook); `cut` is set when a record is left unread, by the
/// budget or the depth limit.
fn walk(b: &[u8], depth: usize, out: &mut Vec<PictureShape>, budget: &mut usize, cut: &mut bool) {
    if depth > MAX_DEPTH {
        *cut |= !b.is_empty();
        return;
    }
    for r in Records::new(b) {
        if *budget == 0 {
            *cut = true;
            return;
        }
        *budget -= 1;
        if r.kind == rt::SP_CONTAINER {
            if let Some(p) = shape(r.data, budget, cut) {
                out.push(p);
            }
        } else if r.is_container() {
            walk(r.data, depth + 1, out, budget, cut);
        }
    }
}

/// A shape container's BLIP and anchor, when it has both and the budget reads all its records.
fn shape(sp: &[u8], budget: &mut usize, cut: &mut bool) -> Option<PictureShape> {
    let (mut pib, mut anchor) = (None, None);
    for r in Records::new(sp) {
        if *budget == 0 {
            *cut = true;
            return None;
        }
        *budget -= 1;
        let body = r.data;
        match r.kind {
            // Properties: `instance` of them, six bytes each (id with flags, value).
            rt::FOPT => {
                // The last `pib` entry counts; a complex one (its operand a length) is no index.
                if let Some(e) = entries(body, usize::from(r.instance)).filter(|e| e.pid == prop::PIB).last() {
                    pib = (!e.is_complex).then_some(e.op);
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
    use craft_officeart::record::MAX_RECORDS;

    fn rec(vi: u16, ty: u16, body: &[u8]) -> Vec<u8> {
        [&vi.to_le_bytes()[..], &ty.to_le_bytes(), &(body.len() as u32).to_le_bytes(), body].concat()
    }

    /// An FBSE body holding `blip`: 36 bytes of description (its size at 20), then the BLIP.
    fn fbse_of(blip: &[u8]) -> Vec<u8> {
        let mut d = [0u8; 36];
        d[20..24].copy_from_slice(&(blip.len() as u32).to_le_bytes());
        [&d[..], blip].concat()
    }

    /// A metafile BLIP: one identifier, the 34-byte header (`fields` as eight u32s, then the
    /// compression and filter bytes), then `saved`.
    fn metafile_blip(ty: u16, instance: u16, fields: [u32; 8], compression: u8, saved: &[u8]) -> Vec<u8> {
        let mut header: Vec<u8> = fields.iter().flat_map(|v| v.to_le_bytes()).collect();
        header.extend_from_slice(&[compression, 0xFE]);
        rec(instance << 4, ty, &[&[0u8; 16][..], &header, saved].concat())
    }

    #[test]
    fn pictures_and_their_anchors_are_found_and_hostile_drawings_are_harmless() {
        let png = [0x89, b'P', b'N', b'G'];
        let blip = rec(0x6E00, 0xF01E, &[&[0u8; 17][..], &png].concat());
        let fbse = rec(0x0062, 0xF007, &fbse_of(&blip));
        let wmf = rec(0x0062, 0xF007, &fbse_of(&rec(0x2160, 0xF01B, &[0; 60])));
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
        assert_eq!(picture_shapes(&dg, &mut MAX_RECORDS.clone()), (vec![PictureShape { pib: 1, anchor }], false));
        // Truncated or nonsense drawings give nothing, never a panic.
        for bad in [&[][..], &[0xFF; 8][..], &[0; 70][..], &fbse_of(&[0xFF; 60])[..]] {
            let _ = fbse_blip(bad, &mut Decode::new(usize::MAX));
        }
        for bad in [&dg[..dg.len() - 5], &[0xFF; 40][..], &[][..], &dgg[..20]] {
            let _ = picture_shapes(bad, &mut MAX_RECORDS.clone());
            let _ = blip_store(bad);
        }
    }

    /// A store child that isn't an FBSE (here a BLIP stored in the store itself) keeps its `pib`
    /// slot, so the FBSE after it is still `pib` 2.
    #[test]
    fn a_store_child_that_isnt_an_fbse_keeps_its_slot() {
        let png = [0x89, b'P', b'N', b'G'];
        let blip = rec(0x6E00, 0xF01E, &[&[0u8; 17][..], &png].concat());
        let fbse = rec(0x0062, 0xF007, &fbse_of(&blip));
        let dgg = rec(0x000F, 0xF000, &rec(0x002F, 0xF001, &[blip, fbse].concat()));
        let store = blip_store(&dgg);
        assert_eq!(store.len(), 2);
        assert_eq!(store[1], Some((Arc::new(png.to_vec()), "image/png")));
    }

    #[test]
    fn metafiles_share_one_budget() {
        // A zlib metafile of 1000 zero bytes, as an EMF BLIP with one identifier.
        let raw = vec![0u8; 1000];
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let fbse = fbse_of(&metafile_blip(0xF01A, 0x3D4, [1000, 0, 0, 10, 10, 0, 0, z.len() as u32], 0, &z));
        let mut work = Decode::new(1500);
        assert_eq!(fbse_blip(&fbse, &mut work).map(|(b, _)| b.len()), Some(1000));
        assert_eq!(work.left, 500 - z.len(), "its compressed bytes and what they inflated to");
        // The second would inflate past what is left: refused, not cut.
        assert_eq!(fbse_blip(&fbse, &mut work), None);
    }

    #[test]
    fn a_wmf_gets_its_placeable_header_back_and_a_dib_becomes_a_png() {
        // Bounds 0,0–2000×1000 units over 2 × 1 inches (EMUs): 1000 units an inch; stored as is.
        let wmf = b"WMF records".to_vec();
        let n = wmf.len() as u32;
        let fbse = fbse_of(&metafile_blip(0xF01B, 0x216, [n, 0, 0, 2000, 1000, 2 * 914_400, 914_400, n], 0xFE, &wmf));
        let (bytes, mime) = fbse_blip(&fbse, &mut Decode::new(usize::MAX)).expect("wmf");
        assert_eq!(mime, "image/x-wmf");
        assert_eq!(&bytes[..4], &0x9AC6_CDD7u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 1000, "units per inch");
        let sum = bytes[..20].chunks(2).fold(0u16, |x, w| x ^ u16::from_le_bytes([w[0], w[1]]));
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), sum);
        assert_eq!(&bytes[22..], &wmf[..]);
        // A 1 × 1, 24-bit DIB, decoded and made a PNG.
        let mut dib = Vec::new();
        for v in [40u32, 1, 1] {
            dib.extend_from_slice(&v.to_le_bytes());
        }
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        dib.extend_from_slice(&[0, 0, 255, 0]);
        let fbse = fbse_of(&rec(0x7A80, 0xF01F, &[&[0u8; 17][..], &dib].concat()));
        let (png, mime) = fbse_blip(&fbse, &mut Decode::new(usize::MAX)).expect("png");
        assert_eq!((mime, &png[..4]), ("image/png", &[0x89, b'P', b'N', b'G'][..]));
    }

    /// A workbook's drawings read at most the shared limit of records together, and containers
    /// nested at most the shared depth.
    #[test]
    fn a_drawing_walk_is_bounded() {
        let opt = rec(0x0013, 0xF00B, &[&0x4104u16.to_le_bytes()[..], &1u32.to_le_bytes()].concat());
        let anchor_body: Vec<u8> = [0u16; 9].iter().flat_map(|v| v.to_le_bytes()).collect();
        let picture = rec(0x000F, 0xF004, &[opt, rec(0x0000, 0xF010, &anchor_body)].concat());
        // Three records a picture: half as many pictures as the limit need half again its records.
        let count = MAX_RECORDS / 4;
        let many = rec(0x000F, 0xF003, &vec![picture.clone(); count].concat());
        let mut budget = MAX_RECORDS;
        let (first, first_cut) = picture_shapes(&many, &mut budget);
        let (second, second_cut) = picture_shapes(&many, &mut budget);
        let (first, second) = (first.len(), second.len());
        assert!(first == count && second < count, "{first} then {second}: the second sheet gets what is left");
        // The second sheet is told its drawing was cut short; the first, read whole, isn't.
        assert!(!first_cut && second_cut);
        // A budget used up exactly by the drawing's last record cuts nothing.
        let one = rec(0x000F, 0xF003, &picture);
        assert!(!picture_shapes(&one, &mut 4).1);
        assert!(picture_shapes(&one, &mut 3).1);
        let nested = |levels: usize| (0..levels).fold(picture.clone(), |inner, _| rec(0x000F, 0xF003, &inner));
        assert_eq!(
            picture_shapes(&nested(MAX_DEPTH), &mut MAX_RECORDS.clone()),
            (vec![PictureShape { pib: 1, anchor: ClientAnchor::default() }], false)
        );
        let (deep, deep_cut) = picture_shapes(&nested(MAX_DEPTH + 2), &mut MAX_RECORDS.clone());
        assert!(deep.is_empty() && deep_cut, "past the depth limit, and told so");
    }
}
