//! Windows metafiles: reads Windows Metafiles (WMF), Enhanced Metafiles (EMF) and EMF+ into a
//! neutral vector model, and (feature `write`) writes EMF and WMF.
//!
//! [`parse`] plays a file's records on a model of the device context and returns a [`Picture`]:
//! filled and stroked paths with their line styles, pattern fills, RGBA bitmaps and text strings,
//! each with the clip it was drawn under, in a picture frame with y down. Nothing in the model
//! belongs to an application: mapping it onto a document (or drawing it) is the caller's.
//! [`Picture::place`] fits a picture into a rectangle for callers that draw it.
//!
//! Text stays font-agnostic: a [`Text`] item names the font the file asks for and carries the
//! string, its advances and alignment; the caller resolves the font and draws the glyphs.
//!
//! The reader is pure Rust over byte slices (no file system), so it builds for wasm. Every read is
//! bounds-checked and every count is capped; hostile input yields a shorter picture or an [`Error`],
//! never a panic. Records follow MS-WMF, MS-EMF and MS-EMFPLUS; the README lists the records read
//! and the limits. Records of a type the reader does not know are counted in
//! [`Picture::skipped_records`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod bytes;
mod canvas;
mod clip;
mod cliprect;
mod dib;
mod emf;
mod emfplus;
mod geom;
mod place;
mod size;
mod text;
mod wmf;

#[cfg(feature = "write")]
pub mod write;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use canvas::Raw;
use geom::{Op, P};
pub use place::{MAX_BITMAP_SIDE, PlacedClip, PlacedItem, bez_path, crop_window};
pub use size::size;
pub use text::{Font, HAlign, SYMBOL_CHARSET, Text, VAlign};

/// Items kept from one file. Past the cap the parse stops and keeps what it has.
pub(crate) const MAX_ITEMS: usize = 200_000;
/// Object table slots (WMF handles, EMF object indices).
pub(crate) const MAX_OBJECTS: usize = 4096;
/// Largest bitmap in pixels (about 40 megapixels, 160 MB of RGBA).
pub(crate) const MAX_PIXELS: usize = 40_000_000;
/// DC save-stack depth.
pub(crate) const MAX_STACK: usize = 1024;
/// Points in one open path bracket.
pub(crate) const MAX_PATH_OPS: usize = 2_000_000;
/// Bytes of path ops and pixels kept from one file (about 256 MB).
pub(crate) const MAX_BYTES: usize = 256 * 1024 * 1024;

/// A parsed metafile. Items use coordinates in `0..width` and `0..height`, y down.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picture {
    /// Size of the drawing in its own units: points for placeable WMF and EMF (from the frame), device
    /// or logical units otherwise.
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    /// Records of a type the reader does not know (neither drawn nor known to change nothing that
    /// is kept). A caller can tell its user that part of the file was not read.
    pub skipped_records: u32,
}

/// How the ends of an open stroke and of its dashes are drawn (`PS_ENDCAP_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Round,
    Square,
    Flat,
}

/// How the corners of a stroke are drawn (`PS_JOIN_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Round,
    Bevel,
    Miter,
}

/// Caps, joins and dashes of a stroke. The default is GDI's: round caps and joins, a miter limit
/// of 10, a solid line.
#[derive(Debug, Clone, PartialEq)]
pub struct LineStyle {
    pub cap: Cap,
    pub join: Join,
    /// Longest miter as a multiple of the stroke width (for [`Join::Miter`]).
    pub miter_limit: f32,
    /// Dash, gap, dash, gap… lengths in the units of the stroke width, starting at the path's
    /// start. Empty is a solid line.
    pub dash: Vec<f32>,
}

impl Default for LineStyle {
    fn default() -> Self {
        LineStyle { cap: Cap::Round, join: Join::Round, miter_limit: 10.0, dash: Vec::new() }
    }
}

impl LineStyle {
    /// The same style with its dash lengths times `k`. None when a length stops being finite.
    fn scaled(&self, k: f64) -> Option<LineStyle> {
        let dash = self.dash.iter().map(|d| f(f64::from(*d) * k)).collect::<Option<Vec<f32>>>()?;
        Some(LineStyle { dash, ..self.clone() })
    }
}

/// A clip path (`SELECTCLIPPATH`), in picture units. Items clipped by the same path share it, so
/// `Arc::ptr_eq` tells which items a clip covers.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipPath {
    pub segs: Vec<Seg>,
    pub even_odd: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A filled and/or stroked path, in picture coordinates.
    Path {
        segs: Vec<Seg>,
        fill: Option<[u8; 4]>,
        /// Colour and width in picture units.
        stroke: Option<([u8; 4], f32)>,
        /// Caps, joins and dashes of the stroke (dash lengths in picture units).
        line: LineStyle,
        even_odd: bool,
        /// Clip region in picture units, each rectangle `[x0, y0, x1, y1]`. None is the default clip.
        region: Option<Vec<[f32; 4]>>,
        /// Clip paths: the item shows inside all of them (and inside `region`). Outermost first.
        clips: Vec<Arc<ClipPath>>,
    },
    /// A bitmap stretched into `rect` = `[x, y, w, h]` (picture units), RGBA8 rows top-down, straight
    /// alpha.
    Bitmap {
        rect: [f32; 4],
        /// The unit square (the whole bitmap, y down) to picture space as affine coefficients
        /// `[a, b, c, d, e, f]` (`x' = a·x + c·y + e`, `y' = b·x + d·y + f`, kurbo's order). For an
        /// upright bitmap this is `rect` again (`[w, 0, 0, h, x, y]`); under a world transform that
        /// turns or skews the bitmap it carries the turn, and `rect` is the box around the turned bitmap.
        transform: [f64; 6],
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        /// Clip region in picture units. None is the default clip.
        region: Option<Vec<[f32; 4]>>,
        /// Clip paths, as on [`Item::Path`].
        clips: Vec<Arc<ClipPath>>,
    },
    /// A path filled with a repeating tile (pattern and hatch brushes). Items filled with the same
    /// brush share its tile.
    Pattern {
        segs: Vec<Seg>,
        even_odd: bool,
        tile: Arc<Tile>,
        /// Clip region in picture units. None is the default clip.
        region: Option<Vec<[f32; 4]>>,
        /// Clip paths, as on [`Item::Path`].
        clips: Vec<Arc<ClipPath>>,
    },
    /// A text string.
    Text(Box<Text>),
}

/// A brush tile: RGBA8 rows top-down (transparent where the brush leaves the background), each
/// pixel `cell` picture units, with a tile's top-left corner at `origin`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub cell: [f32; 2],
    pub origin: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("not a WMF or EMF file")]
    NotMetafile,
    #[error("metafile header is truncated or invalid")]
    BadHeader,
    #[error("metafile has no readable records")]
    NoRecords,
}

/// The picture frame in device units (`rect` = `[x, y, w, h]`) and, when the file states one, its
/// size in picture units.
pub(crate) struct Frame {
    pub rect: [f64; 4],
    pub size: Option<(f64, f64)>,
}

/// Parses a WMF (placeable or plain) or EMF file.
pub fn parse(bytes: &[u8]) -> Result<Picture, Error> {
    dib::set_decode_budget(dib::MAX_DECODE_PIXELS);
    parse_nested(bytes)
}

/// [`parse`] for a metafile inside the one being parsed: it draws on the same budgets.
pub(crate) fn parse_nested(bytes: &[u8]) -> Result<Picture, Error> {
    let b = Bytes(bytes);
    let (cv, frame) = if emf::is_emf(b) {
        emf::parse(b)?
    } else if wmf::is_wmf(b) {
        // A WMF converted from an EMF can carry the original in MFCOMMENT escapes; Office draws
        // the WMF's own records (its dx rounding shows), so the copy is not used.
        wmf::parse(b)?
    } else {
        return Err(Error::NotMetafile);
    };
    let skipped = cv.skipped;
    let mut pic = build(cv.items, frame);
    pic.skipped_records = skipped;
    Ok(pic)
}

/// True when the bytes start like a WMF (placeable or plain header) or an EMF.
pub fn is_metafile(bytes: &[u8]) -> bool {
    sniff(bytes).is_some()
}

/// A metafile format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Enhanced Metafile (32-bit coordinates, Bézier curves, paths and clipping), with or without
    /// EMF+ records.
    Emf,
    /// Windows Metafile, with or without a placeable header (16-bit coordinates, polygons).
    Wmf,
}

/// The metafile format `bytes` start like, if any.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    let b = Bytes(bytes);
    if emf::is_emf(b) {
        Some(Kind::Emf)
    } else if wmf::is_wmf(b) {
        Some(Kind::Wmf)
    } else {
        None
    }
}

/// Maps device-space output into the picture frame. Without a usable frame the frame is the bounding
/// box of the drawing, so the origin is still the top-left of what was drawn.
fn build(raw: Vec<Raw>, frame: Option<Frame>) -> Picture {
    let frame = frame.filter(|f| f.rect.iter().all(|v| v.is_finite()) && f.rect[2] > 0.0 && f.rect[3] > 0.0);
    let (rect, size) = match frame {
        Some(f) => (f.rect, f.size.unwrap_or((f.rect[2], f.rect[3]))),
        None => match bbox(&raw) {
            Some([x0, y0, x1, y1]) => ([x0, y0, x1 - x0, y1 - y0], (x1 - x0, y1 - y0)),
            None => return Picture::default(),
        },
    };
    let sx = if rect[2] > 0.0 { size.0 / rect[2] } else { 1.0 };
    let sy = if rect[3] > 0.0 { size.1 / rect[3] } else { 1.0 };
    let tf = move |p: P| ((p.0 - rect[0]) * sx, (p.1 - rect[1]) * sy);
    let stroke_scale = (sx + sy) / 2.0;
    // No stroke is wider than the picture's diagonal: anything wider is a mapping artefact.
    let diag = size.0.hypot(size.1);
    // Tiles shared by several fills stay shared.
    let mut tiles: HashMap<*const canvas::RawTile, (Arc<canvas::RawTile>, Arc<Tile>)> = HashMap::new();
    // So do clip paths. None is a path with a point that is not finite: its items are left out.
    let mut clip_paths: ClipCache = HashMap::new();
    let mut clip = |region: canvas::RawClip| -> Option<ItemClip> {
        let mut paths = Vec::with_capacity(region.paths.len());
        for raw in region.paths {
            let key = Arc::as_ptr(&raw);
            let mapped = match clip_paths.get(&key) {
                Some((_, c)) => c.clone(),
                None => {
                    let c = raw
                        .ops
                        .iter()
                        .map(|op| seg(op, tf))
                        .collect::<Option<Vec<Seg>>>()
                        .map(|segs| Arc::new(ClipPath { segs, even_odd: raw.even_odd }));
                    // The raw path is kept with its key, so the address is never reused by another.
                    clip_paths.insert(key, (raw, c.clone()));
                    c
                }
            };
            paths.push(mapped?);
        }
        Some((map_region(region.rects, &tf)?, paths))
    };
    let items = raw.into_iter().filter_map(|r| match r {
        Raw::Path { ops, fill, stroke, line, even_odd, region } => {
            let segs = ops.iter().map(|op| seg(op, tf)).collect::<Option<Vec<Seg>>>()?;
            if segs.len() < 2 {
                return None;
            }
            let stroke = stroke.and_then(|(c, w)| Some((c, f((w * stroke_scale).min(diag))?)));
            let (region, clips) = clip(region)?;
            Some(Item::Path { segs, fill, stroke, line: line.scaled(stroke_scale)?, even_odd, region, clips })
        }
        Raw::Bitmap { rect: r, xf, img, region } => {
            let (x, y) = tf((r[0], r[1]));
            let rect = [f(x)?, f(y)?, f(r[2] * sx)?, f(r[3] * sy)?];
            let (e, g) = tf((xf[4], xf[5]));
            let transform = [xf[0] * sx, xf[1] * sy, xf[2] * sx, xf[3] * sy, e, g];
            if !transform.iter().all(|v| v.is_finite()) {
                return None;
            }
            let (region, clips) = clip(region)?;
            Some(Item::Bitmap { rect, transform, width: img.w as u32, height: img.h as u32, rgba: img.px, region, clips })
        }
        Raw::Pattern { ops, even_odd, tile, region } => {
            let segs = ops.iter().map(|op| seg(op, tf)).collect::<Option<Vec<Seg>>>()?;
            if segs.len() < 2 {
                return None;
            }
            let key = Arc::as_ptr(&tile);
            let tile = match tiles.get(&key) {
                Some((_, t)) => t.clone(),
                None => {
                    let (ox, oy) = tf((0.0, 0.0));
                    let t = Arc::new(Tile {
                        width: u32::try_from(tile.w).ok()?,
                        height: u32::try_from(tile.h).ok()?,
                        rgba: tile.px.clone(),
                        cell: [f(tile.cell * sx)?, f(tile.cell * sy)?],
                        origin: [f(ox)?, f(oy)?],
                    });
                    // The raw tile is kept with its key, so the address is never reused by another.
                    tiles.insert(key, (tile.clone(), t.clone()));
                    t
                }
            };
            let (region, clips) = clip(region)?;
            Some(Item::Pattern { segs, even_odd, tile, region, clips })
        }
        Raw::Text { mut text, region } => {
            // Device to picture: scale by (sx, sy) after moving the frame's corner to the origin.
            let [a, b, c, d, e, g] = text.transform;
            let (e, g) = tf((e, g));
            text.transform = [a * sx, b * sy, c * sx, d * sy, e, g];
            if !text.transform.iter().all(|v| v.is_finite()) {
                return None;
            }
            if let Some([x0, y0, x1, y1]) = text.clip {
                let (p, q) = (tf((f64::from(x0), f64::from(y0))), tf((f64::from(x1), f64::from(y1))));
                text.clip = Some([f(p.0.min(q.0))?, f(p.1.min(q.1))?, f(p.0.max(q.0))?, f(p.1.max(q.1))?]);
            }
            (text.region, text.clips) = clip(region)?;
            Some(Item::Text(text))
        }
    });
    let items = items.collect();
    Picture { width: f(size.0).unwrap_or(0.0), height: f(size.1).unwrap_or(0.0), items, skipped_records: 0 }
}

/// An item's clip in picture units: the region rectangles and the clip paths.
type ItemClip = (Option<Vec<[f32; 4]>>, Vec<Arc<ClipPath>>);

/// Device clip paths already mapped into the picture, by address.
type ClipCache = HashMap<*const canvas::RawClipPath, (Arc<canvas::RawClipPath>, Option<Arc<ClipPath>>)>;

fn map_region(region: Option<Vec<[f64; 4]>>, tf: &impl Fn(P) -> P) -> Option<Option<Vec<[f32; 4]>>> {
    let Some(rects) = region else { return Some(None) };
    let mut out = Vec::with_capacity(rects.len());
    for [x0, y0, x1, y1] in rects {
        let (p, q) = (tf((x0, y0)), tf((x1, y1)));
        out.push([f(p.0.min(q.0))?, f(p.1.min(q.1))?, f(p.0.max(q.0))?, f(p.1.max(q.1))?]);
    }
    Some(Some(out))
}

fn f(v: f64) -> Option<f32> {
    let v = v as f32;
    v.is_finite().then_some(v)
}

fn seg(op: &Op, tf: impl Fn(P) -> P) -> Option<Seg> {
    let pt = |p: P| {
        let (x, y) = tf(p);
        Some((f(x)?, f(y)?))
    };
    Some(match *op {
        Op::M(p) => {
            let (x, y) = pt(p)?;
            Seg::Move(x, y)
        }
        Op::L(p) => {
            let (x, y) = pt(p)?;
            Seg::Line(x, y)
        }
        Op::C(a, b, c) => {
            let (x1, y1) = pt(a)?;
            let (x2, y2) = pt(b)?;
            let (x3, y3) = pt(c)?;
            Seg::Cubic(x1, y1, x2, y2, x3, y3)
        }
        Op::Z => Seg::Close,
    })
}

/// Bounding box `[x0, y0, x1, y1]` over the finite points of the output.
fn bbox(raw: &[Raw]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut grow = |p: P| {
        if p.0.is_finite() && p.1.is_finite() {
            b = Some(match b {
                None => [p.0, p.1, p.0, p.1],
                Some([x0, y0, x1, y1]) => [x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1)],
            });
        }
    };
    for r in raw {
        match r {
            Raw::Path { ops, .. } | Raw::Pattern { ops, .. } => {
                for op in ops {
                    match *op {
                        Op::M(p) | Op::L(p) => grow(p),
                        Op::C(a, c, d) => {
                            grow(a);
                            grow(c);
                            grow(d);
                        }
                        Op::Z => {}
                    }
                }
            }
            Raw::Bitmap { rect, .. } => {
                grow((rect[0], rect[1]));
                grow((rect[0] + rect[2], rect[1] + rect[3]));
            }
            Raw::Text { text: t, .. } => grow((t.transform[4], t.transform[5])),
        }
    }
    b
}
