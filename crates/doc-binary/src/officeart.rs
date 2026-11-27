//! OfficeArt (Escher) drawing records, as Word 97-2003 stores them ([MS-ODRAW], public Microsoft
//! Open Specifications): the BLIP store and its pictures, shape containers with their property
//! tables, and the `OfficeArtContent` of the Table stream ([MS-DOC] §2.9.172). The record framing,
//! the property-table walk, the decode budget and DIB conversion are `craft-officeart`'s.
//!
//! Everything here walks hostile bytes: record lengths are clamped to their parent, nesting depth
//! and record counts are capped, and decompressed metafiles are size-limited.

use std::collections::HashMap;

use craft_officeart::blip::Format;
use craft_officeart::dib::{self, Dib};
use craft_officeart::record::{MAX_DEPTH, MAX_RECORDS, Records, u32_at};
use craft_wordmodel::props::Rgb;

pub(crate) use craft_officeart::blip::{Decode, fbse_record};
pub(crate) use craft_officeart::record::Rec;
pub(crate) use craft_officeart::rt;

/// Most drawings (main document, headers) read from `OfficeArtContent`.
const MAX_DRAWINGS: usize = 8;
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
    let b = craft_officeart::blip::parse(rec)?;
    let (ext, placeholder) = match b.format {
        Format::Dib => {
            return match dib::to_png(b.raw(), limit, work)? {
                Dib::Png(bytes) => Some(Blip { bytes, ext: "png", placeholder: false }),
                // Kept as a BMP, which renders as a grey placeholder.
                Dib::Bmp(bytes) => Some(Blip { bytes, ext: "bmp", placeholder: true }),
            };
        }
        Format::Png => ("png", false),
        Format::Jpeg => ("jpeg", false),
        Format::Tiff => ("tiff", false),
        Format::Emf => ("emf", false),
        Format::Wmf => ("wmf", false),
        // The format cannot be drawn: kept as a placeholder of the right size.
        Format::Pict => ("pict", true),
    };
    // A WMF gets back the placeable header the BLIP leaves out, so it keeps its size; the header
    // is part of the picture's share of `limit` and of `work`.
    let bytes = if b.format == Format::Wmf { craft_officeart::blip::with_placeable_header(&b, limit, work)? } else { b.bytes(limit, work)? };
    Some(Blip { bytes, ext, placeholder })
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
    /// The properties of a shape's merged tables (its primary table first, then its secondary and
    /// tertiary ones; boolean words merged flag by flag).
    pub(crate) fn from_set(p: &craft_officeart::props::Props) -> Props {
        let fixed = |v: u32| (v as i32) as f32 / 65_536.0;
        // Complex entries other than the alt text were left out of the set (`shape`), so every
        // value here is a simple operand.
        let mut out = Props { pib: p.get(0x0104), ..Props::default() };
        if let Some(v) = p.get(0x0004) {
            out.rot = fixed(v);
        }
        // cropFromTop, cropFromBottom, cropFromLeft, cropFromRight.
        for (pid, slot) in [(0x0100, 1), (0x0101, 3), (0x0102, 0), (0x0103, 2)] {
            if let Some(v) = p.get(pid).map(fixed) {
                out.crop[slot] = if v.is_finite() { v.clamp(0.0, 0.99) } else { 0.0 };
            }
        }
        out.fill_color = p.get(0x0181).and_then(color);
        // fFilled (bit 4) and fLine (bit 3), when their fUse bits are set.
        out.filled = p.flag(0x01BF, 4);
        out.line_color = p.get(0x01C0).and_then(color);
        out.line_width = p.get(0x01CB);
        out.arrowhead = [0x01D0, 0x01D1].iter().any(|&id| p.get(id).is_some_and(|v| v != 0));
        out.line = p.flag(0x01FF, 3);
        for (i, d) in out.wrap_dist.iter_mut().enumerate() {
            *d = p.get(0x0384 + i as u16);
        }
        (out.posh, out.posrelh, out.posv, out.posrelv) = (p.get(0x038F), p.get(0x0390), p.get(0x0391), p.get(0x0392));
        // Group Shape Boolean Properties: fLayoutInCell (bit 15), when its fUse bit is set.
        out.layout_in_cell = p.flag(0x03BF, 15);
        out.alt = p.string(0x0381).unwrap_or_default();
        out
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
    // Property tables in specificity order: primary (the last one first), then the secondary
    // ones and then the tertiary ones, wherever each sits in the container.
    let (mut primary, mut secondary, mut tertiary) = (vec![], vec![], vec![]);
    for rec in children(body, budget) {
        match rec.kind {
            rt::FSP => {
                sp.spt = rec.instance;
                sp.spid = u32_at(rec.data, 0).unwrap_or(0);
                sp.flags = u32_at(rec.data, 4).unwrap_or(0);
            }
            rt::FOPT => primary.push(rec),
            rt::SECONDARY_FOPT => secondary.push(rec),
            rt::TERTIARY_FOPT => tertiary.push(rec),
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
    let mut props = craft_officeart::props::Props::default();
    // The alt text is the one complex property read; any other complex entry's operand is a
    // length, not a value, so it is left out.
    for t in primary.iter().rev().chain(&secondary).chain(&tertiary) {
        props.add_table_where(t, |id, complex| !complex || id == 0x0381);
    }
    sp.props = Props::from_set(&props);
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
                out.bstore = children(rec.data, &mut budget)
                    .into_iter()
                    .filter(|r| r.kind == rt::FBSE)
                    .map(|r| r.data)
                    .take(craft_officeart::blip::MAX_ENTRIES)
                    .collect();
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
