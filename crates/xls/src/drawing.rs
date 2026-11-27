//! Pictures, from the Office Drawing records ([MS-ODRAW]): the workbook's
//! MsoDrawingGroup holds the pictures (a BStore of FBSE entries, each with its BLIP), and each
//! sheet's MsoDrawing records hold its shapes; a shape whose properties name a BLIP (`pib`,
//! 0x0104) and that has a client anchor (cells and fractions of them) is a picture.

use std::sync::Arc;

use craft_officeart::blip::{Decode, Format, parse, picture, store_entries};
use craft_officeart::props::{entries, prop};
use craft_officeart::record::{MAX_DEPTH, Rec, Records};
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
    'stores: for dgg in Records::new(dgg).filter(|r| r.kind == rt::DGG_CONTAINER) {
        for store in Records::new(dgg.data).filter(|r| r.kind == rt::BSTORE_CONTAINER) {
            for fbse in store_entries(store.data) {
                if out.len() == craft_officeart::blip::MAX_ENTRIES {
                    break 'stores;
                }
                out.push(fbse.and_then(|f| fbse_blip(f, &mut work)).map(|(b, m)| (Arc::new(b), m)));
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
/// The BLIP is the record embedded after the name, whatever the FBSE's `size` says (a workbook
/// has no delay stream): unlike a `.doc`, a size of 0 doesn't mean there is no picture.
fn fbse_blip(fbse: &[u8], work: &mut Decode) -> Bytes {
    let name = usize::from(*fbse.get(33)?);
    let rec = Rec::at(fbse, 36usize.checked_add(name)?)?;
    let blip = parse(&rec)?;
    if matches!(blip.format, Format::Pict | Format::Tiff) {
        return None;
    }
    let p = picture(&blip, usize::MAX, work)?;
    Some((p.data, p.content_type))
}

/// Where a picture shape sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// Its own client anchor: cells and fractions of them.
    Cells(ClientAnchor),
    /// Inside a shape group: its rectangle as fractions (left, top, right, bottom) of the box of
    /// the top-level group, whose client anchor is `group`. [`Grid::anchor`] makes it cells.
    InGroup { group: ClientAnchor, frac: [f64; 4] },
}

/// A picture shape: its BLIP (1-based) and where it sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PictureShape {
    pub pib: u32,
    pub place: Place,
}

/// The picture shapes in a sheet's drawing (its MsoDrawing records joined), reading at most
/// `budget` records more (one budget for the whole workbook,
/// [`craft_officeart::record::MAX_RECORDS`] to start with). A picture inside a shape group is
/// placed through the group's coordinate spaces, nested groups included. Also whether the budget
/// ran out before the drawing's last record or a container nested past [`MAX_DEPTH`] was left
/// unread, so shapes may be missing.
pub(crate) fn picture_shapes(dg: &[u8], budget: &mut usize) -> (Vec<PictureShape>, bool) {
    let mut out = Vec::new();
    let mut allowance = Allowance { left: budget, cut: false };
    walk(dg, 0, None, &mut out, &mut allowance);
    (out, allowance.cut)
}

/// The records a drawing may still read, and whether it has had to leave one unread.
struct Allowance<'b> {
    left: &'b mut usize,
    cut: bool,
}

/// A group's coordinate space mapped to its top-level group's: per axis, a coordinate `v` is
/// `v * scale + offset` there. `space` is the top-level group's own space (its `OfficeArtFSPGR`)
/// and `anchor` its client anchor.
#[derive(Clone, Copy, Debug)]
struct Frame {
    anchor: ClientAnchor,
    space: [f64; 4],
    scale: [f64; 2],
    offset: [f64; 2],
}

impl Frame {
    /// The frame of a group nested in this one: its child anchor `child` (in this frame's space)
    /// holds its own coordinate space `space`.
    fn nested(&self, child: [f64; 4], space: [f64; 4]) -> Frame {
        let mut f = *self;
        for axis in 0..2 {
            let (c0, c1, s0, s1) = (child[axis], child[axis + 2], space[axis], space[axis + 2]);
            let k = if s1 != s0 { (c1 - c0) / (s1 - s0) } else { 0.0 };
            f.scale[axis] = self.scale[axis] * k;
            f.offset[axis] = self.scale[axis] * (c0 - s0 * k) + self.offset[axis];
        }
        f
    }

    /// A rectangle in this frame's space as fractions of the top-level group's box.
    fn fractions(&self, r: [f64; 4]) -> Option<[f64; 4]> {
        let mut out = [0.0; 4];
        for (i, v) in r.iter().enumerate() {
            let axis = i % 2;
            let (lo, hi) = (self.space[axis], self.space[axis + 2]);
            out[i] = (v * self.scale[axis] + self.offset[axis] - lo) / (hi - lo);
        }
        out.iter().all(|v| v.is_finite()).then_some(out)
    }
}

/// The picture shapes in containers nested at most [`MAX_DEPTH`] deep, in `frame` (`None` outside
/// any group), reading at most `budget` records more (shared by the whole workbook); it is marked
/// cut when a record is left unread, by the budget or the depth limit.
fn walk(b: &[u8], depth: usize, frame: Option<Frame>, out: &mut Vec<PictureShape>, budget: &mut Allowance<'_>) {
    if depth > MAX_DEPTH {
        budget.cut |= !b.is_empty();
        return;
    }
    let mut recs = Records::new(b);
    while let Some(r) = take(&mut recs, budget) {
        match r.kind {
            rt::SP_CONTAINER => out.extend(place(&shape(r.data, budget), frame)),
            rt::SPGR_CONTAINER => group(r.data, depth + 1, frame, out, budget),
            _ if r.is_container() => walk(r.data, depth + 1, frame, out, budget),
            _ => {}
        }
    }
}

/// The next record, if the budget has one left for it; a record left unread is noted.
fn take<'a>(recs: &mut Records<'a>, budget: &mut Allowance<'_>) -> Option<craft_officeart::record::Rec<'a>> {
    if *budget.left == 0 {
        budget.cut |= recs.clone().next().is_some();
        return None;
    }
    let r = recs.next()?;
    *budget.left -= 1;
    Some(r)
}

/// A group container: its first shape is the group itself (its coordinate space and its anchor),
/// the rest its members. The drawing's top group (its patriarch) and a container whose first shape
/// isn't a group's hold their shapes in the frame they are in.
fn group(b: &[u8], depth: usize, frame: Option<Frame>, out: &mut Vec<PictureShape>, budget: &mut Allowance<'_>) {
    if depth > MAX_DEPTH {
        budget.cut |= !b.is_empty();
        return;
    }
    let mut recs = Records::new(b);
    let mut inner = frame;
    if let Some(head) = recs.clone().next().filter(|r| r.kind == rt::SP_CONTAINER) {
        let _ = take(&mut recs, budget);
        let s = shape(head.data, budget);
        match s.space.filter(|_| !s.patriarch) {
            Some(space) => {
                inner = match frame {
                    None => s.client.map(|anchor| Frame { anchor, space, scale: [1.0; 2], offset: [0.0; 2] }),
                    Some(f) => s.child.map(|child| f.nested(child, space)),
                };
                // A group that can't be placed leaves out the members placed by it; a member with
                // its own cell anchor keeps it.
            }
            None => out.extend(place(&s, frame)),
        }
    }
    while let Some(r) = take(&mut recs, budget) {
        match r.kind {
            rt::SP_CONTAINER => out.extend(place(&shape(r.data, budget), inner)),
            rt::SPGR_CONTAINER => group(r.data, depth + 1, inner, out, budget),
            _ if r.is_container() => walk(r.data, depth + 1, inner, out, budget),
            _ => {}
        }
    }
}

/// What a shape container says about where it is.
#[derive(Default)]
struct Sp {
    pib: Option<u32>,
    client: Option<ClientAnchor>,
    /// Its child anchor (left, top, right, bottom) in its group's space.
    child: Option<[f64; 4]>,
    /// A group's own coordinate space (`OfficeArtFSPGR`).
    space: Option<[f64; 4]>,
    /// The drawing's top group (`fPatriarch`).
    patriarch: bool,
}

/// A picture shape in `frame`: a picture with its client anchor outside any group, or its child
/// anchor inside one.
fn place(s: &Sp, frame: Option<Frame>) -> Option<PictureShape> {
    let pib = s.pib.filter(|p| *p > 0)?;
    let place = match (frame, s.client, s.child) {
        (_, Some(anchor), _) => Place::Cells(anchor),
        (Some(f), None, Some(child)) => Place::InGroup { group: f.anchor, frac: f.fractions(child)? },
        _ => return None,
    };
    Some(PictureShape { pib, place })
}

/// A rectangle of four i32s (left, top, right, bottom).
fn rect(b: &[u8]) -> Option<[f64; 4]> {
    let v = |at: usize| Some(f64::from(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?)));
    Some([v(0)?, v(4)?, v(8)?, v(12)?])
}

/// A shape container's picture, anchors and group space.
fn shape(sp: &[u8], budget: &mut Allowance<'_>) -> Sp {
    let mut s = Sp::default();
    let mut recs = Records::new(sp);
    while let Some(r) = take(&mut recs, budget) {
        let body = r.data;
        match r.kind {
            // Properties: `instance` of them, six bytes each (id with flags, value).
            rt::FOPT => {
                // The last `pib` entry counts; a complex one (its operand a length) is no index.
                if let Some(e) = entries(body, usize::from(r.instance)).filter(|e| e.pid == prop::PIB).last() {
                    s.pib = (!e.is_complex).then_some(e.op);
                }
            }
            rt::FSP => s.patriarch = body.get(4).is_some_and(|f| f & 0x04 != 0),
            rt::FSPGR => s.space = rect(body),
            rt::CHILD_ANCHOR => s.child = rect(body),
            // Client anchor: flags, then eight 16-bit values.
            rt::CLIENT_ANCHOR => {
                let v: Option<Vec<u16>> = (0..8).map(|i| Some(u16::from_le_bytes([*body.get(2 + i * 2)?, *body.get(3 + i * 2)?]))).collect();
                if let Some([col1, dx1, row1, dy1, col2, dx2, row2, dy2]) = v.as_deref().and_then(|v| <[u16; 8]>::try_from(v).ok()) {
                    s.client = Some(ClientAnchor { col1, dx1, row1, dy1, col2, dx2, row2, dy2 });
                }
            }
            _ => {}
        }
    }
    // A shape the budget cut short is no shape: its anchor may be the part left unread.
    if recs.next().is_some() {
        return Sp::default();
    }
    s
}

/// A sheet's grid, to make a picture inside a group cells: where each column and row starts, in
/// column-width units (1/256 of a character) and twips, as running totals built once.
pub(crate) struct Grid {
    cols: Vec<f64>,
    rows: Vec<f64>,
}

impl Grid {
    /// The grid of a sheet whose column `c` is `col_width(c)` wide and row `r` `row_height(r)` high.
    pub(crate) fn new(col_width: impl Fn(u16) -> f64, row_height: impl Fn(u16) -> f64) -> Grid {
        let sums = |n: usize, size: &dyn Fn(usize) -> f64| {
            let mut v = Vec::with_capacity(n + 1);
            let mut at = 0.0;
            for i in 0..n {
                v.push(at);
                at += size(i).max(0.0);
            }
            v.push(at);
            v
        };
        let width = |i: usize| u16::try_from(i).map_or(0.0, &col_width);
        let height = |i: usize| u16::try_from(i).map_or(0.0, &row_height);
        // A BIFF8 sheet has 256 columns and 65,536 rows: a picture's cells stay among them.
        Grid { cols: sums(256, &width), rows: sums(1 << 16, &height) }
    }

    /// A picture's cells: its own, or those its fractions of its group's box fall in.
    pub(crate) fn anchor(&self, place: &Place) -> ClientAnchor {
        let (g, f) = match place {
            Place::Cells(a) => return *a,
            Place::InGroup { group, frac } => (group, frac),
        };
        let (x0, x1) = (at(&self.cols, g.col1, g.dx1, 1024.0), at(&self.cols, g.col2, g.dx2, 1024.0));
        let (y0, y1) = (at(&self.rows, g.row1, g.dy1, 256.0), at(&self.rows, g.row2, g.dy2, 256.0));
        let (col1, dx1) = cell(&self.cols, x0 + f[0] * (x1 - x0), 1024.0);
        let (row1, dy1) = cell(&self.rows, y0 + f[1] * (y1 - y0), 256.0);
        let (col2, dx2) = cell(&self.cols, x0 + f[2] * (x1 - x0), 1024.0);
        let (row2, dy2) = cell(&self.rows, y0 + f[3] * (y1 - y0), 256.0);
        ClientAnchor { col1, dx1, row1, dy1, col2, dx2, row2, dy2 }
    }
}

/// Where cell `i` plus `d` of `parts` of it is, along running totals `sums`.
fn at(sums: &[f64], i: u16, d: u16, parts: f64) -> f64 {
    let i = usize::from(i);
    let (start, end) = (sums.get(i).copied().unwrap_or(0.0), sums.get(i + 1).copied().unwrap_or(0.0));
    start + (end - start) * f64::from(d.min(parts as u16)) / parts
}

/// The cell `v` falls in along running totals `sums`, and how many `parts` of it in.
fn cell(sums: &[f64], v: f64, parts: f64) -> (u16, u16) {
    let last = sums.len().saturating_sub(2);
    let i = sums.partition_point(|&s| s <= v).saturating_sub(1).min(last);
    let (start, end) = (sums.get(i).copied().unwrap_or(0.0), sums.get(i + 1).copied().unwrap_or(0.0));
    let d = if end > start { ((v - start) / (end - start) * parts).round().clamp(0.0, parts - 1.0) } else { 0.0 };
    (u16::try_from(i).unwrap_or(u16::MAX), d as u16)
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
        assert_eq!(picture_shapes(&dg, &mut MAX_RECORDS.clone()).0, vec![PictureShape { pib: 1, place: Place::Cells(anchor) }]);
        // Truncated or nonsense drawings give nothing, never a panic.
        for bad in [&[][..], &[0xFF; 8][..], &[0; 70][..], &fbse_of(&[0xFF; 60])[..]] {
            let _ = fbse_blip(bad, &mut Decode::new(usize::MAX));
        }
        for bad in [&dg[..dg.len() - 5], &[0xFF; 40][..], &[][..], &dgg[..20]] {
            let _ = picture_shapes(bad, &mut MAX_RECORDS.clone()).0;
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
            (vec![PictureShape { pib: 1, place: Place::Cells(ClientAnchor::default()) }], false)
        );
        let (deep, deep_cut) = picture_shapes(&nested(MAX_DEPTH + 2), &mut MAX_RECORDS.clone());
        assert!(deep.is_empty() && deep_cut, "past the depth limit, and told so");
    }

    fn rect_rec(ty: u16, r: [i32; 4]) -> Vec<u8> {
        rec(0x0000, ty, &r.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>())
    }

    fn client(a: [u16; 8]) -> Vec<u8> {
        rec(0x0000, 0xF010, &[0u16].iter().chain(&a).flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>())
    }

    fn pib(n: u32) -> Vec<u8> {
        rec(0x0013, 0xF00B, &[&0x4104u16.to_le_bytes()[..], &n.to_le_bytes()].concat())
    }

    fn fsp(flags: u32) -> Vec<u8> {
        rec(0x0002, 0xF00A, &[1024u32.to_le_bytes(), flags.to_le_bytes()].concat())
    }

    /// A picture inside a group is placed by its child anchor, as a fraction of the top-level
    /// group's box, through any nested group's space; one outside a group keeps its cells, and the
    /// drawing's patriarch holds its shapes as they are.
    #[test]
    fn pictures_inside_groups_are_placed_through_their_groups() {
        let anchor = [1, 0, 1, 0, 5, 0, 9, 0];
        let head = rec(0x000F, 0xF004, &[fsp(0x1), rect_rec(0xF009, [0, 0, 1000, 1000]), client(anchor)].concat());
        let a = rec(0x000F, 0xF004, &[pib(1), rect_rec(0xF00F, [0, 0, 500, 500])].concat());
        let inner_head = rec(0x000F, 0xF004, &[fsp(0x3), rect_rec(0xF009, [0, 0, 100, 100]), rect_rec(0xF00F, [500, 0, 1000, 500])].concat());
        let b = rec(0x000F, 0xF004, &[pib(2), rect_rec(0xF00F, [0, 0, 100, 100])].concat());
        let inner = rec(0x000F, 0xF003, &[inner_head, b].concat());
        let group = rec(0x000F, 0xF003, &[head, a, inner].concat());
        let outside = rec(0x000F, 0xF004, &[pib(3), client([7, 0, 7, 0, 8, 0, 8, 0])].concat());
        let patriarch = rec(0x000F, 0xF004, &[fsp(0x5), rect_rec(0xF009, [0, 0, 0, 0])].concat());
        let dg = rec(0x000F, 0xF002, &rec(0x000F, 0xF003, &[patriarch, group, outside].concat()));
        let [col1, dx1, row1, dy1, col2, dx2, row2, dy2] = anchor;
        let group = ClientAnchor { col1, dx1, row1, dy1, col2, dx2, row2, dy2 };
        let got = picture_shapes(&dg, &mut MAX_RECORDS.clone()).0;
        let outside = ClientAnchor { col1: 7, dx1: 0, row1: 7, dy1: 0, col2: 8, dx2: 0, row2: 8, dy2: 0 };
        assert_eq!(
            got,
            [
                PictureShape { pib: 1, place: Place::InGroup { group, frac: [0.0, 0.0, 0.5, 0.5] } },
                PictureShape { pib: 2, place: Place::InGroup { group, frac: [0.5, 0.0, 1.0, 0.5] } },
                PictureShape { pib: 3, place: Place::Cells(outside) },
            ]
        );
    }

    /// A group whose own box can't be placed (no anchor) leaves out the members placed by it, and
    /// keeps a member with its own cell anchor.
    #[test]
    fn a_group_that_cannot_be_placed_keeps_its_cell_anchored_members() {
        let head = rec(0x000F, 0xF004, &[fsp(0x1), rect_rec(0xF009, [0, 0, 1000, 1000])].concat());
        let child = rec(0x000F, 0xF004, &[pib(1), rect_rec(0xF00F, [0, 0, 500, 500])].concat());
        let cells = [2, 0, 2, 0, 4, 0, 4, 0];
        let own = rec(0x000F, 0xF004, &[pib(2), client(cells)].concat());
        let dg = rec(0x000F, 0xF002, &rec(0x000F, 0xF003, &[head, child, own].concat()));
        let got = picture_shapes(&dg, &mut MAX_RECORDS.clone()).0;
        let [col1, dx1, row1, dy1, col2, dx2, row2, dy2] = cells;
        assert_eq!(got, [PictureShape { pib: 2, place: Place::Cells(ClientAnchor { col1, dx1, row1, dy1, col2, dx2, row2, dy2 }) }]);
    }

    /// A picture's cells stay in a BIFF8 sheet's 256 columns, even when every column is hidden.
    #[test]
    fn grouped_pictures_stay_in_the_sheets_columns() {
        let group = ClientAnchor { col1: 1, dx1: 0, row1: 1, dy1: 0, col2: 5, dx2: 0, row2: 9, dy2: 0 };
        let place = Place::InGroup { group, frac: [0.0, 0.0, 0.5, 0.5] };
        let a = Grid::new(|c| if c < 256 { 0.0 } else { 1000.0 }, |_| 100.0).anchor(&place);
        assert!(a.col1 < 256 && a.col2 < 256, "{a:?}");
    }

    /// A fraction of a group's box falls in the cell the grid's column widths and row heights put
    /// it in: half way across columns of equal width is a column boundary, and a wider column
    /// moves it.
    #[test]
    fn the_grid_turns_fractions_into_cells() {
        let group = ClientAnchor { col1: 1, dx1: 0, row1: 1, dy1: 0, col2: 5, dx2: 0, row2: 9, dy2: 0 };
        let place = Place::InGroup { group, frac: [0.0, 0.0, 0.5, 0.5] };
        let even = Grid::new(|_| 1000.0, |_| 100.0).anchor(&place);
        assert_eq!(even, ClientAnchor { col1: 1, dx1: 0, row1: 1, dy1: 0, col2: 3, dx2: 0, row2: 5, dy2: 0 });
        // Column 1 three times as wide: half way (4000 of 1000..7000) is the start of column 2.
        let wide = Grid::new(|c| if c == 1 { 3000.0 } else { 1000.0 }, |_| 100.0).anchor(&place);
        assert_eq!((wide.col2, wide.dx2), (2, 0));
        // A quarter of the way into a column is 256 of its 1024 parts.
        let quarter = Place::InGroup { group, frac: [0.0, 0.0, 0.3125, 0.5] };
        assert_eq!(Grid::new(|_| 1000.0, |_| 100.0).anchor(&quarter).dx2, 256);
    }
}
