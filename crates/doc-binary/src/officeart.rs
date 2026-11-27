//! OfficeArt (Escher) drawing records, as Word 97-2003 stores them ([MS-ODRAW], public Microsoft
//! Open Specifications): the BLIP store and its pictures, shape containers with their property
//! tables, and the `OfficeArtContent` of the Table stream ([MS-DOC] §2.9.172). The record framing,
//! the property-table walk, the decode budget and DIB conversion are `craft-officeart`'s.
//!
//! Everything here walks hostile bytes: record lengths are clamped to their parent, nesting depth
//! and record counts are capped, and decompressed metafiles are size-limited.

use std::collections::HashMap;

use craft_officeart::dib::{self, Dib};
use craft_officeart::props::entries;
use craft_officeart::record::{Records, u32_at};
use craft_wordmodel::props::Rgb;

pub(crate) use craft_officeart::blip::{Decode, fbse_record};
pub(crate) use craft_officeart::record::Rec;
pub(crate) use craft_officeart::rt;

/// Deepest container nesting walked (groups inside groups…).
const MAX_DEPTH: usize = 12;
/// Most records walked in one drawing tree or BLIP store.
const MAX_RECORDS: usize = 200_000;
/// Most BLIP store entries kept.
const MAX_BSE: usize = 10_000;
/// Most drawings (main document, headers) read from `OfficeArtContent`.
const MAX_DRAWINGS: usize = 8;
/// Largest decompressed metafile (bytes).
const MAX_METAFILE: usize = 64 << 20;

/// The record at the start of `buf` and the bytes it spans (header included). A body longer
/// than `buf` is cut to what is there.
pub(crate) fn record(buf: &[u8]) -> Option<(Rec<'_>, usize)> {
    let rec = Rec::at(buf, 0)?;
    Some((rec, rec.span()))
}

/// The sibling records of a container body, at most `budget` of them (the budget is shared
/// across a whole walk, so a deep or wide tree cannot multiply the work).
pub(crate) fn children<'a>(body: &'a [u8], budget: &mut usize) -> Vec<Rec<'a>> {
    let mut out = Vec::new();
    let mut recs = Records::new(body);
    while *budget > 0 {
        let Some(rec) = recs.next() else { break };
        *budget -= 1;
        out.push(rec);
    }
    out
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    u32_at(b, at).map(|v| v as i32)
}

// ---------------------------------------------------------------------------------------------
// Pictures (BLIPs).

/// A decoded picture: bytes ready for `Document::media` and their file extension.
pub(crate) struct Blip {
    pub(crate) bytes: Vec<u8>,
    pub(crate) ext: &'static str,
    /// The format cannot be drawn (Macintosh PICT): the picture is kept as a placeholder of
    /// the right size.
    pub(crate) placeholder: bool,
}

/// The picture of an `OfficeArtFBSE` body: its embedded BLIP, or the BLIP stored at `foDelay`
/// in `delay` (the WordDocument stream for the document's BLIP store, the Data stream for
/// inline pictures). `limit` caps the bytes produced.
#[cfg(test)]
pub(crate) fn fbse_blip(fbse: &[u8], delay: &[u8], limit: usize, work: &mut Decode) -> Option<Blip> {
    let (rec, _) = fbse_record(fbse, delay)?;
    blip(&rec, limit, work)
}

/// Decode one `OfficeArtBlip*` record ([MS-ODRAW] §2.2.23-§2.2.31). `work` is the
/// document's decode budget: inflating a metafile or decoding a DIB is charged to it before
/// it runs, whether it succeeds or not, and nothing is decoded once it is spent.
pub(crate) fn blip(rec: &Rec, limit: usize, work: &mut Decode) -> Option<Blip> {
    // Each type has two instance values; the odd one carries a second 16-byte UID.
    let uids = if rec.instance & 1 == 1 { 32 } else { 16 };
    match rec.kind {
        rt::BLIP_PNG | rt::BLIP_JPEG | rt::BLIP_JPEG_CMYK | rt::BLIP_DIB | rt::BLIP_TIFF => {
            let data = rec.data.get(uids + 1..)?; // + tag byte
            if data.is_empty() || data.len() > limit {
                return None;
            }
            match rec.kind {
                rt::BLIP_PNG => Some(Blip { bytes: data.to_vec(), ext: "png", placeholder: false }),
                rt::BLIP_JPEG | rt::BLIP_JPEG_CMYK => Some(Blip { bytes: data.to_vec(), ext: "jpeg", placeholder: false }),
                rt::BLIP_TIFF => Some(Blip { bytes: data.to_vec(), ext: "tiff", placeholder: false }),
                _ => match dib::to_png(data, limit, work)? {
                    Dib::Png(bytes) => Some(Blip { bytes, ext: "png", placeholder: false }),
                    // Kept as a BMP, which renders as a grey placeholder.
                    Dib::Bmp(bytes) => Some(Blip { bytes, ext: "bmp", placeholder: true }),
                },
            }
        }
        rt::BLIP_EMF | rt::BLIP_WMF | rt::BLIP_PICT => {
            // OfficeArtMetafileHeader: cbSize, rcBounds, ptSize, cbSave, compression, filter.
            let hdr = rec.data.get(uids..uids + 34)?;
            let cb_size = u32_at(hdr, 0)? as usize;
            let cb_save = u32_at(hdr, 28)? as usize;
            let compression = *hdr.get(32)?;
            let raw = rec.data.get(uids + 34..)?;
            let raw = raw.get(..cb_save.min(raw.len()))?;
            let cap = limit.min(MAX_METAFILE);
            let bytes = if compression == 0 {
                let want = if cb_size > 0 { cb_size.min(cap) } else { cap };
                // The compressed stream is read whole whatever it inflates to: a record that
                // claims to inflate to nothing still costs its input (#3871).
                if !work.charge(want.max(raw.len()), "metafile") {
                    return None;
                }
                miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(raw, want).ok()?
            } else {
                if raw.len() > cap {
                    return None;
                }
                raw.to_vec()
            };
            if bytes.is_empty() {
                return None;
            }
            match rec.kind {
                rt::BLIP_EMF => Some(Blip { bytes, ext: "emf", placeholder: false }),
                rt::BLIP_WMF => Some(Blip { bytes, ext: "wmf", placeholder: false }),
                _ => Some(Blip { bytes, ext: "pict", placeholder: true }),
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Shapes.

/// The shape properties we use, from `OfficeArtFOPT` and its secondary/tertiary tables
/// ([MS-ODRAW] §2.3).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Props {
    /// `pib`: 1-based index of the picture in the BLIP store.
    pub(crate) pib: Option<u32>,
    /// Rotation, degrees clockwise.
    pub(crate) rot: f32,
    /// Crop fractions (left, top, right, bottom).
    pub(crate) crop: [f32; 4],
    pub(crate) fill_color: Option<Rgb>,
    pub(crate) filled: Option<bool>,
    pub(crate) line_color: Option<Rgb>,
    /// Line width in EMUs.
    pub(crate) line_width: Option<u32>,
    pub(crate) line: Option<bool>,
    /// An arrowhead at either end of a line.
    pub(crate) arrowhead: bool,
    /// `posh`/`posrelh`/`posv`/`posrelv`: alignment and reference frame (Word 2000 and later).
    pub(crate) posh: Option<u32>,
    pub(crate) posrelh: Option<u32>,
    pub(crate) posv: Option<u32>,
    pub(crate) posrelv: Option<u32>,
    /// Distance from wrapped text (left, top, right, bottom), EMUs.
    pub(crate) wrap_dist: [Option<u32>; 4],
    /// `fLayoutInCell` of the Group Shape Boolean Properties, when its use bit is set: Layout in
    /// table cell (None = the default, on).
    pub(crate) layout_in_cell: Option<bool>,
    /// `wzDescription`: the alternative text.
    pub(crate) alt: String,
}

impl Props {
    /// Apply one property table (`OfficeArtFOPT` or a secondary/tertiary one): `count` 6-byte
    /// entries, then the complex data of the entries flagged complex, in order.
    pub(crate) fn apply(&mut self, body: &[u8], count: u16) {
        // The complex data is looked for after the entries the table holds whole.
        for e in entries(body, (count as usize).min(body.len() / 6)) {
            let (pid, op) = (e.pid, e.op);
            if e.is_complex {
                if pid == 0x0381
                    && let Some(data) = e.complex
                {
                    let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).take(4096).collect();
                    self.alt = String::from_utf16_lossy(&units);
                }
                continue;
            }
            let fixed = |v: u32| (v as i32) as f32 / 65_536.0;
            match pid {
                0x0004 => self.rot = fixed(op),
                0x0100..=0x0103 => {
                    // cropFromTop, cropFromBottom, cropFromLeft, cropFromRight.
                    let slot = [1, 3, 0, 2][(pid - 0x0100) as usize];
                    let v = fixed(op);
                    self.crop[slot] = if v.is_finite() { v.clamp(0.0, 0.99) } else { 0.0 };
                }
                0x0104 => self.pib = Some(op),
                0x0181 => self.fill_color = color(op),
                // fFilled (bit 4) is valid when fUsefFilled (bit 20) is set.
                0x01BF if op & (1 << 20) != 0 => self.filled = Some(op & (1 << 4) != 0),
                0x01BF => {}
                0x01C0 => self.line_color = color(op),
                0x01CB => self.line_width = Some(op),
                0x01D0 | 0x01D1 => self.arrowhead |= op != 0,
                // fLine (bit 3) is valid when fUsefLine (bit 19) is set.
                0x01FF if op & (1 << 19) != 0 => self.line = Some(op & (1 << 3) != 0),
                0x01FF => {}
                0x0384..=0x0387 => self.wrap_dist[(pid - 0x0384) as usize] = Some(op),
                0x038F => self.posh = Some(op),
                0x0390 => self.posrelh = Some(op),
                0x0391 => self.posv = Some(op),
                0x0392 => self.posrelv = Some(op),
                // Group Shape Boolean Properties: fLayoutInCell (bit 15) is valid when
                // fUsefLayoutInCell (bit 31) is set.
                0x03BF if op & (1 << 31) != 0 => self.layout_in_cell = Some(op & (1 << 15) != 0),
                _ => {}
            }
        }
    }
}

/// An `OfficeArtCOLORREF` holding a literal RGB value (palette, scheme and system colour
/// references are not resolved).
fn color(v: u32) -> Option<Rgb> {
    let [r, g, b, flags] = v.to_le_bytes();
    // fPaletteIndex, fSchemeIndex, fSysIndex.
    if flags & 0x19 != 0 {
        return None;
    }
    Some(Rgb(r, g, b))
}

/// One shape (`OfficeArtSpContainer`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Sp {
    pub(crate) spid: u32,
    /// Shape type (`MSOSPT`), from the FSP's instance.
    pub(crate) spt: u16,
    /// FSP flags: fGroup, fChild, fPatriarch, fDeleted, fOleShape, fHaveMaster, fFlipH, fFlipV…
    pub(crate) flags: u32,
    pub(crate) props: Props,
    /// `OfficeArtChildAnchor` (left, top, right, bottom) in the parent group's space.
    pub(crate) child: Option<[i32; 4]>,
    /// `OfficeArtFSPGR`: a group's own coordinate space.
    pub(crate) group_space: Option<[i32; 4]>,
    /// Has an `OfficeArtClientTextbox` (its text is in the text box story).
    pub(crate) textbox: bool,
}

impl Sp {
    pub(crate) fn flip_h(&self) -> bool {
        self.flags & (1 << 6) != 0
    }
    pub(crate) fn flip_v(&self) -> bool {
        self.flags & (1 << 7) != 0
    }
    fn deleted(&self) -> bool {
        self.flags & (1 << 3) != 0
    }
}

/// A shape or a group of shapes.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    Shape(Sp),
    /// A group: its own shape record (FSPGR, anchor) and members.
    Group(Sp, Vec<Node>),
}

impl Node {
    pub(crate) fn sp(&self) -> &Sp {
        match self {
            Node::Shape(sp) | Node::Group(sp, _) => sp,
        }
    }
}

/// Parse one `OfficeArtSpContainer`.
pub(crate) fn shape(body: &[u8], budget: &mut usize) -> Sp {
    let mut sp = Sp::default();
    for rec in children(body, budget) {
        match rec.kind {
            rt::FSP => {
                sp.spt = rec.instance;
                sp.spid = u32_at(rec.data, 0).unwrap_or(0);
                sp.flags = u32_at(rec.data, 4).unwrap_or(0);
            }
            rt::FOPT | rt::SECONDARY_FOPT | rt::TERTIARY_FOPT => sp.props.apply(rec.data, rec.instance),
            rt::FSPGR | rt::CHILD_ANCHOR => {
                let r = [0, 4, 8, 12].map(|o| i32_at(rec.data, o));
                if let [Some(l), Some(t), Some(r), Some(b)] = r {
                    if rec.kind == rt::FSPGR {
                        sp.group_space = Some([l, t, r, b]);
                    } else {
                        sp.child = Some([l, t, r, b]);
                    }
                }
            }
            rt::CLIENT_TEXTBOX => sp.textbox = true,
            _ => {}
        }
    }
    sp
}

/// Parse an `OfficeArtSpgrContainer`: the first shape is the group itself, the rest its members.
fn group(body: &[u8], depth: usize, budget: &mut usize) -> Option<Node> {
    let recs = children(body, budget);
    let mut it = recs.iter();
    let head = it.find(|r| r.kind == rt::SP_CONTAINER).map(|r| shape(r.data, budget))?;
    let mut members = Vec::new();
    for rec in it {
        if let Some(n) = node(rec, depth + 1, budget) {
            members.push(n);
        }
    }
    Some(Node::Group(head, members))
}

/// A shape or group record as a node (`None` for other records, deleted shapes or past the
/// depth cap).
fn node(rec: &Rec, depth: usize, budget: &mut usize) -> Option<Node> {
    match rec.kind {
        rt::SP_CONTAINER => Some(shape(rec.data, budget)).filter(|s| !s.deleted()).map(Node::Shape),
        rt::SPGR_CONTAINER if depth < MAX_DEPTH => group(rec.data, depth, budget),
        _ => None,
    }
}

/// The document's drawings: the BLIP store and the top-level shapes of each drawing by spid.
#[derive(Default)]
pub(crate) struct Content<'a> {
    /// `OfficeArtFBSE` bodies, in BLIP store order (`pib` 1 = the first).
    pub(crate) bstore: Vec<&'a [u8]>,
    /// The main document's shapes (`dgglbl` 0) and the headers' (`dgglbl` 1), by spid.
    pub(crate) main: HashMap<u32, Node>,
    pub(crate) header: HashMap<u32, Node>,
}

/// Parse `OfficeArtContent` (the `fcDggInfo` blob of the Table stream): an
/// `OfficeArtDggContainer` followed by (`dgglbl`, `OfficeArtDgContainer`) drawings.
pub(crate) fn content(blob: &[u8]) -> Content<'_> {
    let mut out = Content::default();
    let mut budget = MAX_RECORDS;
    let Some((dgg, mut at)) = record(blob) else { return out };
    if dgg.kind == rt::DGG_CONTAINER {
        for rec in children(dgg.data, &mut budget) {
            if rec.kind == rt::BSTORE_CONTAINER {
                out.bstore = children(rec.data, &mut budget).into_iter().filter(|r| r.kind == rt::FBSE).map(|r| r.data).take(MAX_BSE).collect();
            }
        }
    }
    for _ in 0..MAX_DRAWINGS {
        let Some(&label) = blob.get(at) else { break };
        let Some((dg, used)) = blob.get(at + 1..).and_then(record) else { break };
        at = at.saturating_add(1 + used);
        if dg.kind != rt::DG_CONTAINER {
            continue;
        }
        let into = if label == 0 { &mut out.main } else { &mut out.header };
        for rec in children(dg.data, &mut budget) {
            if rec.kind != rt::SPGR_CONTAINER {
                continue;
            }
            // The top group: its first shape is the patriarch; the members are the drawing's shapes.
            if let Some(Node::Group(_, members)) = group(rec.data, 0, &mut budget) {
                for n in members {
                    into.entry(n.sp().spid).or_insert(n);
                }
            }
        }
    }
    out
}
