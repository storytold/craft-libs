//! AVIF (AV1 Image File Format) decoding, behind the non-default `avif-decode` feature, native
//! targets only.
//!
//! The container is HEIF (ISO/IEC 23008-12) over ISO-BMFF. This module reads the boxes itself
//! (`meta`, `pitm`, `iloc`, `iinf`, `iref`, `iprp`/`ipco`/`ipma`, `idat`), and `craft-video`'s
//! clean-room AV1 decoder (`av1-decode`, pure Rust, no `unsafe`) decodes the AV1 still pictures.
//! The container reader, colour conversion and transforms are adapted from @emmertex's LightCraft
//! AVIF importer (storytold/lightcraft#3017).
//!
//! Supported: the primary item as one `av01` image or a `grid` of them; 8-, 10- and 12-bit;
//! 4:2:0, 4:2:2, 4:4:4 and monochrome; full or limited range; the `nclx` colour description (or the
//! AV1 sequence header's) and an ICC profile (`prof` / `rICC`); an alpha auxiliary image (`auxl`,
//! premultiplied with `prem` too); the primary item's Exif and XMP items. `clap`, `irot` and `imir`
//! are applied unless the caller keeps the stored orientation, as for HEIF. PQ and HLG files
//! open as linear light in 32-bit float, as HEIF does. Not read: image sequences (animated AVIF),
//! depth and gain-map auxiliary images, layered (`lsel`/`a1lx`) progressive images.
//!
//! Bounded for hostile input: box nesting, item, property and extent counts are capped; item data
//! can't add up to more than the file; the [`Limits`] are checked against the declared size
//! before any picture is decoded, against every decoded picture and against each output buffer;
//! the AV1 decoder's picture store and the pictures one chunk may show are capped by `max_alloc`
//! too; repeated transforms are refused; and a panic in the decoder is
//! a malformed-file error. wasm32 builds leave AVIF unreadable: a panic there aborts instead of
//! unwinding.
//!
//! Sources: ISO/IEC 23008-12 (HEIF) and the AV1 Image File Format specification v1.1.0 (box
//! layouts), ITU-T H.273 (colour code points).

use std::panic::{AssertUnwindSafe, catch_unwind};

use craft_video::av1::decode as av1;

use crate::Format;
use crate::cicp::Cicp;
use crate::error::CodecError;
use crate::image::{ChannelLayout, Image, Metadata, SampleType};
use crate::options::Limits;

const F: Format = Format::Avif;
/// Deepest box nesting followed (meta → iprp → ipco → property is 4).
const MAX_DEPTH: u32 = 8;
/// Most boxes, items, properties, associations or extents accepted: far above real files.
const MAX_ITEMS: usize = 1 << 16;
/// The AV1 decoder's own picture-store cap (`craft-video`'s default on 64-bit targets).
const AV1_MEMORY_BUDGET: u64 = 4 << 30;
/// The `auxC` types of an alpha plane (AVIF and the older HEVC-in-HEIF spelling).
const ALPHA_URNS: [&[u8]; 2] = [b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha", b"urn:mpeg:hevc:2015:auxid:1"];

type Result<T> = std::result::Result<T, CodecError>;

fn bad(m: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, m)
}

/// A big-endian reader over a byte slice that never panics.
struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Rd { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or_else(|| bad("box overflows"))?;
        let s = self.b.get(self.p..end).ok_or_else(|| bad("box truncated"))?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(self.uint(2)? as u16)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(self.uint(4)? as u32)
    }
    /// An unsigned big-endian number of `n` (0 to 8) bytes.
    fn uint(&mut self, n: usize) -> Result<u64> {
        if n > 8 {
            return Err(bad("field too wide"));
        }
        Ok(self.take(n)?.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
    }
    /// A full box's version, its flags skipped.
    fn version(&mut self) -> Result<u8> {
        let v = self.u8()?;
        self.take(3)?;
        Ok(v)
    }
    /// An item ID: 16 bits in version 0 boxes, 32 bits otherwise.
    fn id(&mut self, wide: bool) -> Result<u32> {
        if wide { self.u32() } else { self.u16().map(u32::from) }
    }
    fn cstr(&mut self) -> Result<&'a [u8]> {
        let rest = self.rest();
        let n = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
        let s = self.take(n)?;
        if self.p < self.b.len() {
            self.p += 1;
        }
        Ok(s)
    }
    fn rest(&self) -> &'a [u8] {
        self.b.get(self.p..).unwrap_or(&[])
    }
}

/// The child boxes of `b`: (type, payload).
fn boxes(b: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < b.len() {
        let mut r = Rd::new(b.get(p..).unwrap_or(&[]));
        if r.b.len() < 8 {
            break; // trailing padding
        }
        let size32 = r.u32()?;
        let t = r.take(4)?;
        let ty = [t[0], t[1], t[2], t[3]];
        let (hdr, size) = match size32 {
            0 => (8usize, r.b.len() as u64),
            1 => (16, r.uint(8)?),
            s => (8, u64::from(s)),
        };
        let size = usize::try_from(size).map_err(|_| bad("box too large"))?;
        if size < hdr || size > r.b.len() {
            return Err(bad(format!("box {} has a bad size", String::from_utf8_lossy(&ty))));
        }
        out.push((ty, r.b.get(hdr..size).unwrap_or(&[])));
        if out.len() > MAX_ITEMS {
            return Err(bad("too many boxes"));
        }
        p = p.checked_add(size).ok_or_else(|| bad("box overflows"))?;
    }
    Ok(out)
}

#[derive(Clone, Debug, Default)]
struct Extent {
    offset: u64,
    length: u64,
}

#[derive(Clone, Debug, Default)]
struct Loc {
    method: u8,
    base: u64,
    extents: Vec<Extent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Transform {
    /// Clean aperture: width, height, horizontal and vertical offset of the centre.
    Clap { w: f64, h: f64, dx: f64, dy: f64 },
    /// Anticlockwise rotation by `n` × 90°.
    Rot(u8),
    /// Mirror, as HEIF amendment 2, libheif and libavif read it: 0 = top and bottom exchanged,
    /// 1 = left and right exchanged.
    Mir(u8),
}

#[derive(Clone, Debug)]
enum Prop {
    Av1c(Vec<u8>),
    Nclx(Cicp),
    Icc(Vec<u8>),
    Ispe(u32, u32),
    AuxC(Vec<u8>),
    T(Transform),
    Other,
}

struct Item {
    id: u32,
    kind: [u8; 4],
    /// `mime` items: the content type.
    content_type: Option<Vec<u8>>,
}

#[derive(Default)]
struct Container<'a> {
    file: &'a [u8],
    primary: u32,
    items: Vec<Item>,
    locs: Vec<(u32, Loc)>,
    idat: &'a [u8],
    props: Vec<Prop>,
    /// item → 1-based property indices, in order.
    assoc: Vec<(u32, Vec<usize>)>,
    /// (type, from, to…)
    refs: Vec<([u8; 4], u32, Vec<u32>)>,
    /// `iloc` extents read so far, across all items (capped at [`MAX_ITEMS`]).
    extents: usize,
}

fn parse(file: &[u8]) -> Result<Container<'_>> {
    let top = boxes(file)?;
    let meta = top.iter().find(|(t, _)| t == b"meta").map(|(_, b)| *b).ok_or_else(|| {
        if top.iter().any(|(t, _)| t == b"moov") {
            CodecError::unsupported(F, "AVIF image sequences (animated AVIF) aren't read")
        } else {
            bad("no meta box")
        }
    })?;
    let mut c = Container { file, ..Default::default() };
    let meta = meta.get(4..).ok_or_else(|| bad("meta box truncated"))?;
    for (t, b) in boxes(meta)? {
        let mut r = Rd::new(b);
        match &t {
            b"pitm" => {
                let v = r.version()?;
                c.primary = r.id(v != 0)?;
            }
            b"iloc" => parse_iloc(&mut r, &mut c)?,
            b"iinf" => parse_iinf(&mut r, &mut c)?,
            b"iref" => parse_iref(&mut r, &mut c)?,
            b"idat" => c.idat = b,
            b"iprp" => parse_iprp(b, &mut c, 0)?,
            _ => {}
        }
    }
    if c.primary == 0 {
        return Err(bad("no primary item"));
    }
    Ok(c)
}

fn parse_iinf(r: &mut Rd, c: &mut Container) -> Result<()> {
    let v = r.version()?;
    let _count = r.id(v != 0)?;
    for (t, b) in boxes(r.rest())? {
        if &t != b"infe" {
            continue;
        }
        let mut r = Rd::new(b);
        let v = r.version()?;
        if v < 2 {
            continue;
        }
        let id = r.id(v != 2)?;
        r.u16()?; // protection index
        let kind = r.take(4)?;
        let kind = [kind[0], kind[1], kind[2], kind[3]];
        let _name = r.cstr()?;
        let content_type = (&kind == b"mime").then(|| r.cstr().ok().map(<[u8]>::to_vec)).flatten();
        c.items.push(Item { id, kind, content_type });
        if c.items.len() > MAX_ITEMS {
            return Err(bad("too many items"));
        }
    }
    Ok(())
}

fn parse_iref(r: &mut Rd, c: &mut Container) -> Result<()> {
    let v = r.version()?;
    for (t, b) in boxes(r.rest())? {
        let mut r = Rd::new(b);
        let from = r.id(v != 0)?;
        let n = r.u16()?;
        let mut to = Vec::with_capacity(usize::from(n).min(r.rest().len() / 2 + 1));
        for _ in 0..n {
            to.push(r.id(v != 0)?);
        }
        c.refs.push((t, from, to));
        if c.refs.len() > MAX_ITEMS {
            return Err(bad("too many item references"));
        }
    }
    Ok(())
}

fn parse_iloc(r: &mut Rd, c: &mut Container) -> Result<()> {
    let v = r.version()?;
    if v > 2 {
        return Err(CodecError::unsupported(F, "iloc version"));
    }
    let a = r.u8()?;
    let b = r.u8()?;
    let (off_size, len_size, base_size) = (usize::from(a >> 4), usize::from(a & 15), usize::from(b >> 4));
    let index_size = if v >= 1 { usize::from(b & 15) } else { 0 };
    let count = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
    for _ in 0..count {
        let id = r.id(v >= 2)?;
        let method = if v >= 1 { (r.u16()? & 15) as u8 } else { 0 };
        r.u16()?; // data reference index
        let base = r.uint(base_size)?;
        let n = r.u16()?;
        let mut extents = Vec::with_capacity(usize::from(n).min(64));
        for _ in 0..n {
            r.uint(index_size)?;
            let offset = r.uint(off_size)?;
            let length = r.uint(len_size)?;
            extents.push(Extent { offset, length });
            // Zero-width offset and length fields read no bytes, so the file size alone doesn't
            // bound this.
            c.extents += 1;
            if c.extents > MAX_ITEMS {
                return Err(bad("too many item extents"));
            }
        }
        c.locs.push((id, Loc { method, base, extents }));
        if c.locs.len() > MAX_ITEMS {
            return Err(bad("too many items"));
        }
    }
    Ok(())
}

fn parse_iprp(b: &[u8], c: &mut Container, depth: u32) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(bad("boxes nested too deep"));
    }
    for (t, b) in boxes(b)? {
        match &t {
            b"ipco" => {
                for (t, b) in boxes(b)? {
                    c.props.push(parse_prop(&t, b)?);
                    if c.props.len() > MAX_ITEMS {
                        return Err(bad("too many properties"));
                    }
                }
            }
            b"ipma" => parse_ipma(b, c)?,
            _ => {}
        }
    }
    Ok(())
}

fn parse_ipma(b: &[u8], c: &mut Container) -> Result<()> {
    let mut r = Rd::new(b);
    let v = r.u8()?;
    let flags = r.take(3)?;
    let wide = flags.get(2).is_some_and(|f| f & 1 != 0);
    let n = r.u32()?;
    for _ in 0..n {
        let id = r.id(v != 0)?;
        let k = r.u8()?;
        let mut list = Vec::with_capacity(usize::from(k));
        for _ in 0..k {
            let idx = if wide { usize::from(r.u16()? & 0x7fff) } else { usize::from(r.u8()? & 0x7f) };
            list.push(idx);
        }
        c.assoc.push((id, list));
        if c.assoc.len() > MAX_ITEMS {
            return Err(bad("too many property associations"));
        }
    }
    Ok(())
}

fn parse_prop(t: &[u8; 4], b: &[u8]) -> Result<Prop> {
    let mut r = Rd::new(b);
    Ok(match t {
        b"av1C" => Prop::Av1c(b.get(4..).unwrap_or(&[]).to_vec()),
        b"colr" => match r.take(4)? {
            b"nclx" => Prop::Nclx(Cicp { primaries: r.u16()?, transfer: r.u16()?, matrix: r.u16()?, full_range: r.u8()? & 0x80 != 0 }),
            b"prof" | b"rICC" => Prop::Icc(r.rest().to_vec()),
            _ => Prop::Other,
        },
        b"ispe" => {
            r.version()?;
            Prop::Ispe(r.u32()?, r.u32()?)
        }
        b"auxC" => {
            r.version()?;
            Prop::AuxC(r.cstr()?.to_vec())
        }
        b"irot" => Prop::T(Transform::Rot(r.u8()? & 3)),
        b"imir" => Prop::T(Transform::Mir(r.u8()? & 1)),
        b"clap" => {
            // Width and height are unsigned fractions; the centre offsets have signed numerators
            // (and unsigned denominators), as libavif and libheif read them.
            let mut q = |signed: bool| -> Result<f64> {
                let raw = r.u32()?;
                let n = if signed { f64::from(raw as i32) } else { f64::from(raw) };
                let d = r.u32()?;
                if d == 0 {
                    return Err(bad("clap with a zero denominator"));
                }
                Ok(n / f64::from(d))
            };
            let (w, h, dx, dy) = (q(false)?, q(false)?, q(true)?, q(true)?);
            Prop::T(Transform::Clap { w, h, dx, dy })
        }
        _ => Prop::Other,
    })
}

impl Container<'_> {
    fn item_type(&self, id: u32) -> Option<[u8; 4]> {
        self.items.iter().find(|t| t.id == id).map(|t| t.kind)
    }
    fn props_of(&self, id: u32) -> impl Iterator<Item = &Prop> {
        self.assoc.iter().filter(move |a| a.0 == id).flat_map(|a| a.1.iter()).filter_map(|&i| i.checked_sub(1).and_then(|i| self.props.get(i)))
    }
    fn data(&self, id: u32) -> Result<Vec<u8>> {
        let loc = self.locs.iter().find(|l| l.0 == id).map(|l| &l.1).ok_or_else(|| bad(format!("item {id} has no location")))?;
        let src = match loc.method {
            0 => self.file,
            1 => self.idat,
            _ => return Err(CodecError::unsupported(F, "item data by reference to another item")),
        };
        let mut out = Vec::new();
        for e in &loc.extents {
            let start = loc.base.checked_add(e.offset).ok_or_else(|| bad("item offset overflows"))?;
            let start = usize::try_from(start).map_err(|_| bad("item offset out of range"))?;
            let end = if e.length == 0 {
                src.len()
            } else {
                usize::try_from(e.length).ok().and_then(|l| start.checked_add(l)).ok_or_else(|| bad("item length overflows"))?
            };
            let chunk = src.get(start..end).ok_or_else(|| bad("item data runs past the end of the file"))?;
            // Every byte comes from the file, so a real item is never larger than it; repeated or
            // overlapping extents that add up to more are refused before they allocate.
            if out.len().saturating_add(chunk.len()) > self.file.len() {
                return Err(bad("item data is larger than the file"));
            }
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }
    fn refs_from(&self, kind: &[u8; 4], from: u32) -> Vec<u32> {
        self.refs.iter().filter(|r| &r.0 == kind && r.1 == from).flat_map(|r| r.2.iter().copied()).collect()
    }
    /// Items with a `kind` reference to `to`.
    fn refs_to(&self, kind: &[u8; 4], to: u32) -> impl Iterator<Item = u32> {
        self.refs.iter().filter(move |r| &r.0 == kind && r.2.contains(&to)).map(|r| r.1)
    }
    fn ispe(&self, id: u32) -> Option<(u32, u32)> {
        self.props_of(id).find_map(|p| if let Prop::Ispe(w, h) = p { Some((*w, *h)) } else { None })
    }
    /// The item's transformative properties in association order. HEIF allows each of `clap`,
    /// `irot` and `imir` at most once per item; a file that associates one more often (repeated
    /// `ipma` entries could otherwise ask for a quarter of a million full-image rotations) is
    /// refused before any pixels are touched.
    fn transforms(&self, id: u32) -> Result<Vec<Transform>> {
        // Three is the most a valid item has, so a fourth is already a repeat.
        let ts: Vec<Transform> = self.props_of(id).filter_map(|p| if let Prop::T(t) = p { Some(*t) } else { None }).take(4).collect();
        for (i, t) in ts.iter().enumerate() {
            if ts[..i].iter().any(|u| std::mem::discriminant(u) == std::mem::discriminant(t)) {
                return Err(bad("an image item has a transformative property (clap, irot or imir) more than once"));
            }
        }
        Ok(ts)
    }
    fn nclx(&self, id: u32) -> Option<Cicp> {
        self.props_of(id).find_map(|p| if let Prop::Nclx(n) = p { Some(*n) } else { None })
    }
    fn icc(&self, id: u32) -> Option<Vec<u8>> {
        self.props_of(id).find_map(|p| if let Prop::Icc(b) = p { Some(b.clone()) } else { None })
    }
    /// The primary item's alpha plane: an auxiliary image (`auxl`) whose `auxC` says alpha.
    fn alpha_item(&self) -> Option<u32> {
        self.refs_to(b"auxl", self.primary).find(|&id| {
            matches!(self.item_type(id), Some(t) if &t == b"av01" || &t == b"grid")
                && self.props_of(id).any(|p| matches!(p, Prop::AuxC(urn) if ALPHA_URNS.contains(&urn.as_slice())))
        })
    }
    /// The primary item's colour is premultiplied by its alpha (`prem` from colour to alpha).
    fn premultiplied(&self, alpha: u32) -> bool {
        self.refs_from(b"prem", self.primary).contains(&alpha)
    }
    /// The first item of a metadata kind that describes (`cdsc`) the primary item.
    fn metadata_item(&self, is: impl Fn(&Item) -> bool) -> Option<u32> {
        let described: Vec<u32> = self.refs_to(b"cdsc", self.primary).collect();
        self.items.iter().filter(|i| is(i)).map(|i| i.id).find(|id| described.contains(id))
    }
}

/// The size of an image item (`av01` or `grid`) before its transforms.
fn item_size(c: &Container, id: u32) -> Result<(u32, u32)> {
    let kind = c.item_type(id).ok_or_else(|| bad("an image item is not described"))?;
    let (w, h) = match (c.ispe(id), &kind) {
        (_, b"grid") => grid_header(&c.data(id)?)?.2,
        (Some(s), b"av01") => s,
        (None, b"av01") => return Err(bad("an AV1 image item has no size (ispe)")),
        _ => return Err(CodecError::unsupported(F, "the image item is not an AV1 image or a grid")),
    };
    if w == 0 || h == 0 {
        return Err(bad("zero dimension"));
    }
    Ok((w, h))
}

/// The size after `clap` / `irot`, from a stored size.
fn displayed(mut w: u32, mut h: u32, ts: &[Transform]) -> (u32, u32) {
    for t in ts {
        match *t {
            Transform::Clap { w: cw, h: ch, .. } if cw >= 1.0 && ch >= 1.0 && cw.is_finite() && ch.is_finite() => {
                w = (cw.round() as u32).clamp(1, w);
                h = (ch.round() as u32).clamp(1, h);
            }
            Transform::Rot(n) if n % 2 == 1 => std::mem::swap(&mut w, &mut h),
            _ => {}
        }
    }
    (w, h)
}

/// (rows, columns, output size) of a `grid` item.
fn grid_header(d: &[u8]) -> Result<(u32, u32, (u32, u32))> {
    let mut r = Rd::new(d);
    r.u8()?; // version
    let flags = r.u8()?;
    let rows = u32::from(r.u8()?) + 1;
    let cols = u32::from(r.u8()?) + 1;
    let n = if flags & 1 != 0 { 4 } else { 2 };
    let w = r.uint(n)? as u32;
    let h = r.uint(n)? as u32;
    Ok((rows, cols, (w, h)))
}

/// The upright size [`decode`] returns (`keep_orientation` off), from the boxes alone.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<(u32, u32)> {
    let c = parse(bytes)?;
    let (w, h) = item_size(&c, c.primary)?;
    limits.check_size(w, h)?;
    Ok(displayed(w, h, &c.transforms(c.primary)?))
}

/// One decoded AV1 picture.
struct Pic {
    w: usize,
    h: usize,
    bits: u8,
    ssx: usize,
    ssy: usize,
    mono: bool,
    planes: [Vec<u16>; 3],
    cicp: Cicp,
}

fn av1_error(e: av1::Error) -> CodecError {
    match e {
        av1::Error::Unsupported(w) => CodecError::unsupported(F, format!("AV1: {w}")),
        other => bad(format!("AV1: {other}")),
    }
}

/// Decodes the `av01` item `id`, refusing pictures over `limits`.
fn decode_av1(c: &Container, id: u32, limits: &Limits) -> Result<Pic> {
    if c.item_type(id) != Some(*b"av01") {
        return Err(CodecError::unsupported(F, "an image tile is not an AV1 image"));
    }
    // A tile's declared size counts before its picture is decoded too.
    if let Some((w, h)) = c.ispe(id) {
        limits.check(w, h, ChannelLayout::Rgb, SampleType::U16)?;
    }
    let data = c.data(id)?;
    let config = c.props_of(id).find_map(|p| if let Prop::Av1c(o) = p { Some(o.clone()) } else { None });
    let mut dec = av1::Decoder::new();
    dec.set_memory_budget(limits.max_alloc.min(AV1_MEMORY_BUDGET));
    // Only the first picture is kept, so the pictures a chunk may show are held to the same budget
    // before they are built (the decoder allows up to 1 GiB of them otherwise).
    dec.set_shown_budget(limits.max_alloc);
    let mut pics = catch_unwind(AssertUnwindSafe(|| -> std::result::Result<Vec<av1::Picture>, av1::Error> {
        let mut pics = Vec::new();
        // The sequence header normally travels in the item too; the av1C copy covers files
        // where it doesn't.
        if let Some(cfg) = config.filter(|o| !o.is_empty()) {
            pics.extend(dec.decode(&cfg)?);
        }
        pics.extend(dec.decode(&data)?);
        pics.extend(dec.flush_result()?);
        Ok(pics)
    }))
    .map_err(|_| bad("the AV1 decoder failed on this file"))?
    .map_err(av1_error)?;
    if pics.is_empty() {
        return Err(bad("the AV1 data holds no picture"));
    }
    let p = pics.swap_remove(0);
    limits.check(p.width, p.height, ChannelLayout::Rgb, SampleType::U16)?;
    let (w, h) = (p.width as usize, p.height as usize);
    let (ssx, ssy) = (usize::from(p.subsampling_x.min(1)), usize::from(p.subsampling_y.min(1)));
    let need = w.checked_mul(h).ok_or_else(|| bad("picture too large"))?;
    let cneed = ((w + ssx) >> ssx) * ((h + ssy) >> ssy);
    if need == 0 || p.planes[0].len() < need || (!p.mono_chrome && (p.planes[1].len() < cneed || p.planes[2].len() < cneed)) {
        return Err(bad("AV1 picture planes are short"));
    }
    let cicp = Cicp {
        primaries: u16::from(p.color_primaries),
        transfer: u16::from(p.transfer_characteristics),
        matrix: u16::from(p.matrix_coefficients),
        full_range: p.full_range,
    };
    Ok(Pic { w, h, bits: p.bit_depth.clamp(8, 16), ssx, ssy, mono: p.mono_chrome, planes: p.planes, cicp })
}

/// Kr, Kb of an H.273 matrix code point; `None` = identity (GBR).
fn coefficients(matrix: u16, primaries: u16) -> Result<Option<(f32, f32)>> {
    Ok(match matrix {
        0 => None,
        1 => Some((0.2126, 0.0722)),
        4 => Some((0.30, 0.11)),
        7 => Some((0.212, 0.087)),
        // 9 is BT.2020 non-constant luminance; 10 (constant luminance) needs other equations and
        // is refused below.
        9 => Some((0.2627, 0.0593)),
        // Chromaticity-derived non-constant luminance; unspecified primaries are read as BT.709,
        // as `cicp::linearize` reads them for the HLG gain.
        12 => {
            let [kr, _, kb] = crate::cicp::luma_weights(crate::cicp::primaries_or_bt709(primaries));
            Some((kr, kb))
        }
        // 2 (unspecified: AVIF readers assume BT.601), 5, 6 (BT.601) and reserved values.
        2 | 5 | 6 | 3 | 15.. => Some((0.299, 0.114)),
        _ => return Err(CodecError::unsupported(F, format!("matrix coefficients {matrix} (YCgCo, constant luminance or ICtCp)"))),
    })
}

/// A chroma sample at luma position (x, y), bilinear between chroma sites centred on their
/// luma pairs.
#[inline]
fn chroma_at(plane: &[u16], cw: usize, ch: usize, ssx: usize, ssy: usize, x: usize, y: usize) -> f32 {
    let pos = |v: usize, ss: usize, n: usize| -> (usize, usize, f32) {
        if ss == 0 {
            return (v.min(n - 1), v.min(n - 1), 0.0);
        }
        let f = ((v as f32 + 0.5) / 2.0 - 0.5).max(0.0);
        let i = f as usize;
        (i.min(n - 1), (i + 1).min(n - 1), f - i as f32)
    };
    let (x0, x1, fx) = pos(x, ssx, cw);
    let (y0, y1, fy) = pos(y, ssy, ch);
    let s = |xx: usize, yy: usize| f32::from(plane.get(yy * cw + xx).copied().unwrap_or(0));
    let top = s(x0, y0) * (1.0 - fx) + s(x1, y0) * fx;
    let bot = s(x0, y1) * (1.0 - fx) + s(x1, y1) * fx;
    top * (1.0 - fy) + bot * fy
}

/// The sample scaling of a picture: luma offset and range, chroma range and midpoint.
struct Levels {
    yoff: f32,
    yrange: f32,
    crange: f32,
    mid: f32,
}

impl Levels {
    fn new(bits: u8, full_range: bool) -> Self {
        let max = ((1u32 << bits) - 1) as f32;
        let scale = f32::from(1u16 << (bits - 8).min(8));
        let (yoff, yrange, crange) = if full_range { (0.0, max, max) } else { (16.0 * scale, 219.0 * scale, 224.0 * scale) };
        Levels { yoff, yrange, crange, mid: f32::from(1u16 << (bits - 1).min(15)) }
    }
}

fn to_u16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

/// Where a decoded picture goes: an interleaved 16-bit canvas of `ch` channels.
struct Canvas {
    w: usize,
    h: usize,
    ch: usize,
    data: Vec<u16>,
}

impl Canvas {
    fn new(w: u32, h: u32, layout: ChannelLayout, limits: &Limits) -> Result<Self> {
        limits.check(w, h, layout, SampleType::U16)?;
        let (w, h, ch) = (w as usize, h as usize, layout.channels());
        let len = w.checked_mul(h).and_then(|n| n.checked_mul(ch)).ok_or_else(|| bad("image too large"))?;
        Ok(Canvas { w, h, ch, data: vec![0; len] })
    }

    /// The picture's YCbCr as non-linear R'G'B' into channels 0..3, its top-left at (x0, y0).
    fn put_colour(&mut self, p: &Pic, cicp: Cicp, x0: usize, y0: usize) -> Result<()> {
        let lv = Levels::new(p.bits, cicp.full_range);
        let k = coefficients(cicp.matrix, cicp.primaries)?;
        let (cw, chh) = ((p.w + p.ssx) >> p.ssx, (p.h + p.ssy) >> p.ssy);
        for y in 0..p.h.min(self.h.saturating_sub(y0)) {
            for x in 0..p.w.min(self.w.saturating_sub(x0)) {
                let luma = (f32::from(p.planes[0][y * p.w + x]) - lv.yoff) / lv.yrange;
                let (cb, cr) = if p.mono {
                    (lv.mid, lv.mid)
                } else {
                    (chroma_at(&p.planes[1], cw, chh, p.ssx, p.ssy, x, y), chroma_at(&p.planes[2], cw, chh, p.ssx, p.ssy, x, y))
                };
                let rgb = match k {
                    // Identity (GBR): Y = G, Cb = B, Cr = R, all three at the luma levels.
                    None => [(cr - lv.yoff) / lv.yrange, luma, (cb - lv.yoff) / lv.yrange],
                    Some((kr, kb)) => {
                        let (cb, cr) = ((cb - lv.mid) / lv.crange, (cr - lv.mid) / lv.crange);
                        let r = luma + 2.0 * (1.0 - kr) * cr;
                        let b = luma + 2.0 * (1.0 - kb) * cb;
                        [r, (luma - kr * r - kb * b) / (1.0 - kr - kb), b]
                    }
                };
                let o = ((y0 + y) * self.w + x0 + x) * self.ch;
                for (d, v) in self.data[o..o + 3].iter_mut().zip(rgb) {
                    *d = to_u16(v);
                }
            }
        }
        Ok(())
    }

    /// The picture's luma as alpha into channel 3, its top-left at (x0, y0).
    fn put_alpha(&mut self, p: &Pic, x0: usize, y0: usize) {
        let lv = Levels::new(p.bits, p.cicp.full_range);
        for y in 0..p.h.min(self.h.saturating_sub(y0)) {
            for x in 0..p.w.min(self.w.saturating_sub(x0)) {
                let a = (f32::from(p.planes[0][y * p.w + x]) - lv.yoff) / lv.yrange;
                self.data[((y0 + y) * self.w + x0 + x) * self.ch + 3] = to_u16(a);
            }
        }
    }
}

/// Decodes the image item `id` (one `av01` or a `grid` of them) into `canvas`, through `put`.
/// Returns the first picture's bit depth and colour description.
fn render(
    c: &Container,
    id: u32,
    limits: &Limits,
    canvas: &mut Canvas,
    put: &mut dyn FnMut(&mut Canvas, &Pic, usize, usize) -> Result<()>,
) -> Result<(u8, Cicp)> {
    if c.item_type(id) != Some(*b"grid") {
        let p = decode_av1(c, id, limits)?;
        if (p.w, p.h) != (canvas.w, canvas.h) {
            return Err(bad("the AV1 picture's size differs from its item's"));
        }
        put(canvas, &p, 0, 0)?;
        return Ok((p.bits, p.cicp));
    }
    let (rows, cols, _) = grid_header(&c.data(id)?)?;
    let tiles = c.refs_from(b"dimg", id);
    if tiles.is_empty() || tiles.len() as u64 != u64::from(rows) * u64::from(cols) {
        return Err(bad("grid tile count doesn't match its rows and columns"));
    }
    let mut first: Option<(usize, usize, u8, Cicp)> = None;
    for (i, &t) in tiles.iter().enumerate() {
        let p = decode_av1(c, t, limits)?;
        let (tw, th, ..) = *first.get_or_insert((p.w, p.h, p.bits, p.cicp));
        if (p.w, p.h) != (tw, th) {
            return Err(bad("grid tiles differ in size"));
        }
        if tw.saturating_mul(cols as usize) < canvas.w || th.saturating_mul(rows as usize) < canvas.h {
            return Err(bad("grid tiles don't cover the grid"));
        }
        put(canvas, &p, (i % cols as usize) * tw, (i / cols as usize) * th)?;
    }
    first.map(|(_, _, bits, cicp)| (bits, cicp)).ok_or_else(|| bad("grid without tiles"))
}

/// Applies the container's transforms in order to an interleaved canvas.
fn apply_transforms(mut cv: Canvas, ts: &[Transform]) -> Canvas {
    for t in ts {
        cv = match *t {
            Transform::Clap { w, h, dx, dy } => clap(cv, w, h, dx, dy),
            Transform::Rot(n) => (0..n).fold(cv, |cv, _| rotate(cv)),
            Transform::Mir(axis) => mirror(cv, axis),
        };
    }
    cv
}

fn clap(cv: Canvas, cw: f64, chh: f64, dx: f64, dy: f64) -> Canvas {
    if !(cw >= 1.0 && chh >= 1.0 && cw.is_finite() && chh.is_finite() && dx.is_finite() && dy.is_finite()) {
        return cv;
    }
    let (w, h, ch) = (cv.w, cv.h, cv.ch);
    let nw = (cw.round() as usize).clamp(1, w);
    let nh = (chh.round() as usize).clamp(1, h);
    // The centre of the aperture is the image centre plus the offset.
    let x0 = ((w as f64 - 1.0) / 2.0 + dx - (nw as f64 - 1.0) / 2.0).round().clamp(0.0, (w - nw) as f64) as usize;
    let y0 = ((h as f64 - 1.0) / 2.0 + dy - (nh as f64 - 1.0) / 2.0).round().clamp(0.0, (h - nh) as f64) as usize;
    let mut data = Vec::with_capacity(nw * nh * ch);
    for y in y0..y0 + nh {
        data.extend_from_slice(&cv.data[(y * w + x0) * ch..(y * w + x0 + nw) * ch]);
    }
    Canvas { w: nw, h: nh, ch, data }
}

/// 90° anticlockwise: (x, y) → (y, w - 1 - x), the new size h × w.
fn rotate(cv: Canvas) -> Canvas {
    let (w, h, ch) = (cv.w, cv.h, cv.ch);
    let mut data = vec![0u16; cv.data.len()];
    for y in 0..h {
        for x in 0..w {
            let d = ((w - 1 - x) * h + y) * ch;
            let s = (y * w + x) * ch;
            data[d..d + ch].copy_from_slice(&cv.data[s..s + ch]);
        }
    }
    Canvas { w: h, h: w, ch, data }
}

fn mirror(mut cv: Canvas, axis: u8) -> Canvas {
    let (w, h, ch) = (cv.w, cv.h, cv.ch);
    if axis == 0 {
        for y in 0..h / 2 {
            let (top, bottom) = cv.data.split_at_mut((h - 1 - y) * w * ch);
            top[y * w * ch..(y + 1) * w * ch].swap_with_slice(&mut bottom[..w * ch]);
        }
    } else {
        for row in cv.data.chunks_exact_mut(w * ch) {
            for x in 0..w / 2 {
                for c in 0..ch {
                    row.swap(x * ch + c, (w - 1 - x) * ch + c);
                }
            }
        }
    }
    cv
}

/// Straight colour from colour premultiplied by alpha.
fn unpremultiply(cv: &mut Canvas) {
    for px in cv.data.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a == 0 {
            continue;
        }
        for v in &mut px[..3] {
            *v = (u32::from(*v) * 65535 / a).min(65535) as u16;
        }
    }
}

/// The primary item's Exif (TIFF structure, its offset prefix dropped) and XMP.
fn read_metadata(c: &Container) -> (Option<Vec<u8>>, Option<String>) {
    let exif = c.metadata_item(|i| &i.kind == b"Exif").and_then(|id| c.data(id).ok()).and_then(|d| {
        let offset = usize::try_from(u32::from_be_bytes(d.get(..4)?.try_into().ok()?)).ok()?;
        let tiff = d.get(4usize.checked_add(offset)?..)?;
        (tiff.starts_with(b"II*\0") || tiff.starts_with(b"MM\0*")).then(|| tiff.to_vec())
    });
    let xmp = c
        .metadata_item(|i| &i.kind == b"mime" && i.content_type.as_deref() == Some(b"application/rdf+xml"))
        .and_then(|id| c.data(id).ok())
        .map(|d| String::from_utf8_lossy(&d).trim_end_matches('\0').to_owned());
    (exif, xmp)
}

/// Decodes the primary image. `keep_orientation` leaves the container's `clap`, `irot` and
/// `imir` unapplied (the stored pixels), as for HEIF.
pub(crate) fn decode(bytes: &[u8], limits: &Limits, keep_orientation: bool) -> Result<Image> {
    decode_with_depth(bytes, limits, keep_orientation).map(|(image, _)| image)
}

pub(crate) fn decode_with_depth(bytes: &[u8], limits: &Limits, keep_orientation: bool) -> Result<(Image, u8)> {
    let c = parse(bytes)?;
    let id = c.primary;
    let (w, h) = item_size(&c, id)?;
    let alpha = c.alpha_item().filter(|&a| item_size(&c, a).is_ok_and(|s| s == (w, h)));
    let transforms = c.transforms(id)?;
    // The alpha plane is composed in stored orientation and turned with the colour, which is right
    // when it has no transforms of its own (what encoders write) or the colour's. Others would put
    // transparency in the wrong place, so they are refused rather than ignored.
    if let Some(a) = alpha {
        let own = c.transforms(a)?;
        if !own.is_empty() && own != transforms {
            return Err(CodecError::unsupported(F, "an alpha plane with transforms (clap, irot, imir) that differ from the image's"));
        }
    }
    let layout = if alpha.is_some() { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    // The declared size, before any picture is decoded.
    let mut canvas = Canvas::new(w, h, layout, limits)?;
    let file_cicp = c.nclx(id);
    let mut colour = |cv: &mut Canvas, p: &Pic, x: usize, y: usize| cv.put_colour(p, file_cicp.unwrap_or(p.cicp), x, y);
    let (bits, stream_cicp) = render(&c, id, limits, &mut canvas, &mut colour)?;
    if let Some(a) = alpha {
        render(&c, a, limits, &mut canvas, &mut |cv, p, x, y| {
            cv.put_alpha(p, x, y);
            Ok(())
        })?;
        if c.premultiplied(a) {
            unpremultiply(&mut canvas);
        }
    }
    if !keep_orientation {
        canvas = apply_transforms(canvas, &transforms);
    }
    let (w, h) = (canvas.w as u32, canvas.h as u32);
    let image = Image::from_u16(w, h, layout, &canvas.data)?;
    drop(canvas);
    let mut img = if bits > 8 { image } else { image.convert(layout, SampleType::U8) };
    let cicp = file_cicp.unwrap_or(stream_cicp);
    img.icc = c.icc(id);
    img.cicp = Some(cicp);
    let (exif, xmp) = read_metadata(&c);
    img.meta = Metadata {
        exif: exif.map(|e| if keep_orientation { e } else { crate::orientation::upright_exif(&e).into_owned() }),
        xmp: xmp.map(|x| if keep_orientation { x } else { crate::orientation::upright_xmp(&x).into_owned() }),
        ..Default::default()
    };
    // PQ or HLG: linear light in 32-bit float, as for HEIF.
    if cicp.is_hdr() {
        limits.check(w, h, layout, SampleType::F32)?;
        return crate::cicp::linearize(img, cicp).map(|image| (image, bits));
    }
    Ok((img, bits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DecodeOptions;

    const QUAD_420: &[u8] = include_bytes!("../../tests/fixtures/avif/quad-420-bt709.avif");
    const QUAD_420_DAV1D: &[u8] = include_bytes!("../../tests/fixtures/avif/quad-420-bt709.dav1d.yuv");
    const QUAD_GBR_LOSSLESS: &[u8] = include_bytes!("../../tests/fixtures/avif/quad-444-gbr-lossless.avif");
    const QUAD_SOURCE: &[u8] = include_bytes!("../../tests/fixtures/avif/quad-source.rgb");

    fn rgb8(img: &Image) -> Vec<u8> {
        img.to_rgba8().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
    }

    fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).abs()).sum::<f64>() / a.len() as f64
    }

    /// A box of type `t` around `body`.
    fn bx(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(body);
        v
    }

    /// A full box: version 0, no flags.
    fn fbx(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = vec![0, 0, 0, 0];
        b.extend_from_slice(body);
        bx(t, &b)
    }

    /// An `infe` (version 2) for item `id` of `kind`.
    fn infe(id: u16, kind: &[u8; 4]) -> Vec<u8> {
        let mut b = vec![2, 0, 0, 0];
        b.extend_from_slice(&id.to_be_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(kind);
        b.push(0);
        bx(b"infe", &b)
    }

    fn ispe(w: u32, h: u32) -> Vec<u8> {
        let mut b = w.to_be_bytes().to_vec();
        b.extend_from_slice(&h.to_be_bytes());
        fbx(b"ispe", &b)
    }

    /// The AV1 data and `av1C` of an encoded file's primary item.
    fn av1_of(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let c = parse(file).unwrap();
        let cfg = c.props_of(c.primary).find_map(|p| if let Prop::Av1c(o) = p { Some(o.clone()) } else { None }).unwrap();
        (c.data(c.primary).unwrap(), cfg)
    }

    /// An AVIF of `items` (id, type, data, property indices) with `props`, `refs` and primary 1.
    /// All item data sits in `idat` (construction method 1).
    /// An item: ID, type, data and 1-based property indices.
    type TestItem<'a> = (u16, &'a [u8; 4], &'a [u8], &'a [u8]);
    /// An item reference: type, from, to.
    type TestRef<'a> = (&'a [u8; 4], u16, &'a [u16]);

    fn build(items: &[TestItem], props: &[Vec<u8>], refs: &[TestRef]) -> Vec<u8> {
        let mut iinf = vec![0, 0, 0, 0];
        iinf.extend_from_slice(&(items.len() as u16).to_be_bytes());
        let mut iloc = vec![1, 0, 0, 0, 0x44, 0x00];
        iloc.extend_from_slice(&(items.len() as u16).to_be_bytes());
        let mut ipma = vec![0, 0, 0, 0];
        ipma.extend_from_slice(&(items.len() as u32).to_be_bytes());
        let mut idat = Vec::new();
        for &(id, kind, data, assoc) in items {
            iinf.extend(infe(id, kind));
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 1, 0, 0, 0, 1]); // method 1, data ref 0, one extent
            iloc.extend_from_slice(&(idat.len() as u32).to_be_bytes());
            iloc.extend_from_slice(&(data.len() as u32).to_be_bytes());
            idat.extend_from_slice(data);
            ipma.extend_from_slice(&id.to_be_bytes());
            ipma.push(assoc.len() as u8);
            ipma.extend(assoc.iter().map(|&p| p | 0x80));
        }
        let mut iref = vec![0, 0, 0, 0];
        for &(kind, from, to) in refs {
            let mut b = from.to_be_bytes().to_vec();
            b.extend_from_slice(&(to.len() as u16).to_be_bytes());
            to.iter().for_each(|t| b.extend_from_slice(&t.to_be_bytes()));
            iref.extend(bx(kind, &b));
        }
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(fbx(b"hdlr", b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0"));
        meta.extend(fbx(b"pitm", &[0, 1]));
        meta.extend(bx(b"iinf", &iinf));
        meta.extend(bx(b"iloc", &iloc));
        meta.extend(bx(b"iref", &iref));
        meta.extend(bx(b"iprp", &[bx(b"ipco", &props.concat()), bx(b"ipma", &ipma)].concat()));
        meta.extend(bx(b"idat", &idat));
        let mut file = bx(b"ftyp", b"avif\0\0\0\0avifmif1miaf");
        file.extend(bx(b"meta", &meta));
        file
    }

    #[test]
    fn libaom_420_limited_range_matches_dav1d() {
        let img = crate::decode(QUAD_420).unwrap();
        assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (33, 17, ChannelLayout::Rgb, SampleType::U8));
        assert_eq!(img.cicp.map(|c| (c.matrix, c.full_range)), Some((1, false)), "BT.709 matrix, limited range");
        assert_eq!(crate::probe_size(QUAD_420).unwrap(), (33, 17));
        // The AV1 picture is libdav1d's, sample for sample.
        let c = parse(QUAD_420).unwrap();
        let p = decode_av1(&c, c.primary, &Limits::default()).unwrap();
        assert_eq!((p.w, p.h, p.bits, p.ssx, p.ssy, p.mono), (33, 17, 8, 1, 1, false));
        let planes: Vec<u8> = p.planes.iter().flatten().map(|&v| v as u8).collect();
        assert_eq!(planes, QUAD_420_DAV1D);
        let ours = rgb8(&img);
        // The quadrant centres are the source's colours, give or take the lossy coding.
        for (x, y) in [(5usize, 4usize), (27, 4), (5, 13), (27, 13)] {
            let i = (y * 33 + x) * 3;
            let d = mean_abs_diff(&ours[i..i + 3], &QUAD_SOURCE[i..i + 3]);
            assert!(d < 12.0, "({x}, {y}): {:?} vs {:?}", &ours[i..i + 3], &QUAD_SOURCE[i..i + 3]);
        }
    }

    #[test]
    fn libaom_lossless_identity_matrix_is_exact() {
        let img = crate::decode(QUAD_GBR_LOSSLESS).unwrap();
        assert_eq!((img.width(), img.height(), img.sample_type()), (33, 17, SampleType::U8));
        assert_eq!(img.cicp.map(|c| (c.matrix, c.full_range)), Some((0, true)));
        assert_eq!(rgb8(&img), QUAD_SOURCE);
    }

    #[test]
    fn limits_are_checked_before_decoding() {
        let tight = |max_pixels, max_alloc| DecodeOptions { limits: Limits { max_pixels, max_alloc, ..Limits::default() }, ..Default::default() };
        assert!(matches!(crate::decode_with(QUAD_420, &tight(33 * 17 - 1, u64::MAX)), Err(CodecError::LimitExceeded(_))));
        // The 16-bit intermediate counts: 33 × 17 RGB at 2 bytes.
        assert!(matches!(crate::decode_with(QUAD_420, &tight(u64::MAX, 33 * 17 * 6 - 1)), Err(CodecError::LimitExceeded(_))));
        assert!(crate::decode_with(QUAD_420, &tight(33 * 17, Limits::default().max_alloc)).is_ok());
        // `max_alloc` is the AV1 decoder's picture-store budget too.
        assert!(crate::decode_with(QUAD_420, &tight(33 * 17, 33 * 17 * 6)).is_err());
        // A declared size over the limits fails from the boxes alone.
        let (av1, cfg) = av1_of(QUAD_420);
        let huge = build(&[(1, b"av01", &av1, &[1, 2])], &[ispe(1 << 20, 1 << 20), bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat())], &[]);
        assert!(matches!(crate::decode(&huge), Err(CodecError::LimitExceeded(_))));
        assert!(matches!(crate::probe_size(&huge), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn a_picture_that_differs_from_its_ispe_is_refused() {
        let (av1, cfg) = av1_of(QUAD_420);
        let file = build(&[(1, b"av01", &av1, &[1, 2])], &[ispe(32, 17), bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat())], &[]);
        assert!(matches!(crate::decode(&file), Err(CodecError::Malformed { .. })));
    }

    #[test]
    fn a_grid_is_assembled_from_its_tiles_and_cropped() {
        // Two copies of the 33 × 17 picture side by side, cropped to 60 × 15 by the grid.
        let (av1, cfg) = av1_of(QUAD_GBR_LOSSLESS);
        let av1c = bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat());
        let grid = [0u8, 0, 0, 1, 0, 60, 0, 15];
        let file = build(
            &[(1, b"grid", &grid, &[1]), (2, b"av01", &av1, &[2, 3]), (3, b"av01", &av1, &[2, 3])],
            &[ispe(60, 15), ispe(33, 17), av1c, fbx(b"pixi", &[3, 8, 8, 8])],
            &[(b"dimg", 1, &[2, 3])],
        );
        assert_eq!(crate::probe_size(&file).unwrap(), (60, 15));
        let img = crate::decode(&file).unwrap();
        assert_eq!((img.width(), img.height()), (60, 15));
        let got = rgb8(&img);
        for y in 0..15 {
            for x in 0..60 {
                let s = (y * 33 + x % 33) * 3;
                assert_eq!(&got[(y * 60 + x) * 3..][..3], &QUAD_SOURCE[s..s + 3], "({x}, {y})");
            }
        }
        // Tiles that can't cover the grid are refused.
        let wide = [0u8, 0, 0, 1, 0, 70, 0, 15];
        let file = build(
            &[(1, b"grid", &wide, &[1]), (2, b"av01", &av1, &[2, 3]), (3, b"av01", &av1, &[2, 3])],
            &[ispe(70, 15), ispe(33, 17), bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat())],
            &[(b"dimg", 1, &[2, 3])],
        );
        assert!(matches!(crate::decode(&file), Err(CodecError::Malformed { .. })));
    }

    #[test]
    fn container_rotation_and_mirror_are_applied_unless_kept() {
        let (av1, cfg) = av1_of(QUAD_GBR_LOSSLESS);
        let av1c = bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat());
        let file = build(&[(1, b"av01", &av1, &[1, 2, 3, 4])], &[ispe(33, 17), av1c, bx(b"irot", &[1]), bx(b"imir", &[0])], &[]);
        assert_eq!(crate::probe_size(&file).unwrap(), (17, 33));
        let img = crate::decode(&file).unwrap();
        assert_eq!((img.width(), img.height()), (17, 33));
        let got = rgb8(&img);
        // Rotated 90° anticlockwise, then top and bottom exchanged: a transpose, (x, y) from (y, x).
        for y in 0..33 {
            for x in 0..17 {
                let s = (x * 33 + y) * 3;
                assert_eq!(&got[(y * 17 + x) * 3..][..3], &QUAD_SOURCE[s..s + 3], "({x}, {y})");
            }
        }
        let kept = crate::decode_with(&file, &DecodeOptions { keep_orientation: true, ..Default::default() }).unwrap();
        assert_eq!((kept.width(), kept.height(), rgb8(&kept)), (33, 17, QUAD_SOURCE.to_vec()));
    }

    /// A `clap` box: width, height, horizontal and vertical offset, each numerator then denominator.
    fn clap_box(f: [u32; 8]) -> Vec<u8> {
        bx(b"clap", &f.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>())
    }

    #[test]
    fn clean_aperture_sizes_are_unsigned() {
        let (av1, cfg) = av1_of(QUAD_GBR_LOSSLESS);
        let av1c = bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat());
        // A 2 × 2 aperture with its width written as 0x80000000 / 0x40000000, centred one pixel
        // left of the image centre (a signed offset numerator: -1 / 1).
        let clap = clap_box([0x8000_0000, 0x4000_0000, 2, 1, (-1i32) as u32, 1, 0, 1]);
        let file = build(&[(1, b"av01", &av1, &[1, 2, 3])], &[ispe(33, 17), av1c, clap], &[]);
        assert_eq!(crate::probe_size(&file).unwrap(), (2, 2));
        let img = crate::decode(&file).unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
        // Centre (16, 8) moved to (15, 8): the 2 × 2 crop starts at (15, 8) (rounded from 14.5, 7.5).
        let got = rgb8(&img);
        let (x0, y0) = ((32.0f64 / 2.0 - 1.0 - 0.5).round() as usize, (16.0f64 / 2.0 - 0.5).round() as usize);
        for y in 0..2 {
            for x in 0..2 {
                let s = ((y0 + y) * 33 + x0 + x) * 3;
                assert_eq!(&got[(y * 2 + x) * 3..][..3], &QUAD_SOURCE[s..s + 3], "({x}, {y})");
            }
        }
    }

    #[test]
    fn repeated_transforms_are_refused() {
        let (av1, cfg) = av1_of(QUAD_GBR_LOSSLESS);
        let av1c = bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat());
        // the same irot associated twice, and through two ipma entries for the item
        let twice = build(&[(1, b"av01", &av1, &[1, 2, 3, 3])], &[ispe(33, 17), av1c.clone(), bx(b"irot", &[1])], &[]);
        assert!(matches!(crate::probe_size(&twice), Err(CodecError::Malformed { .. })));
        assert!(matches!(crate::decode(&twice), Err(CodecError::Malformed { .. })));
        let mut many = build(&[(1, b"av01", &av1, &[1, 2, 3])], &[ispe(33, 17), av1c.clone(), bx(b"irot", &[1])], &[]);
        let c = parse(&many).unwrap();
        assert_eq!(c.transforms(1).unwrap(), vec![Transform::Rot(1)]);
        // 1024 more ipma records for item 1, each repeating the irot 255 times
        let mut ipma = vec![0, 0, 0, 0];
        ipma.extend_from_slice(&1024u32.to_be_bytes());
        for _ in 0..1024 {
            ipma.extend_from_slice(&[0, 1, 255]);
            ipma.extend(std::iter::repeat_n(3u8, 255));
        }
        many = build(&[(1, b"av01", &av1, &[1, 2])], &[ispe(33, 17), av1c, bx(b"irot", &[1])], &[]);
        let mut c = parse(&many).unwrap();
        parse_ipma(&ipma, &mut c).unwrap();
        assert!(matches!(c.transforms(1), Err(CodecError::Malformed { .. })));
        // each kind once is fine, in any order
        let ok = build(
            &[(1, b"av01", &av1, &[1, 2, 3, 4, 5])],
            &[
                ispe(33, 17),
                bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat()),
                clap_box([33, 1, 17, 1, 0, 1, 0, 1]),
                bx(b"irot", &[1]),
                bx(b"imir", &[0]),
            ],
            &[],
        );
        assert_eq!(crate::probe_size(&ok).unwrap(), (17, 33));
    }

    #[test]
    fn alpha_with_transforms_of_its_own_is_refused() {
        let (av1, cfg) = av1_of(QUAD_GBR_LOSSLESS);
        let av1c = bx(b"av1C", &[&[0x81, 0, 0, 0][..], &cfg].concat());
        let auxc = fbx(b"auxC", b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha\0");
        let props = [ispe(33, 17), av1c, auxc, bx(b"imir", &[1])];
        let with = |colour: &[u8], alpha: &[u8]| build(&[(1, b"av01", &av1, colour), (2, b"av01", &av1, alpha)], &props, &[(b"auxl", 2, &[1])]);
        // no transforms on the alpha plane, or the colour's: composed, then turned together
        let plain = crate::decode(&with(&[1, 2], &[1, 2, 3])).unwrap();
        assert_eq!(plain.layout(), ChannelLayout::Rgba);
        let both = crate::decode(&with(&[1, 2, 4], &[1, 2, 3, 4])).unwrap();
        let alpha_free = crate::decode(&with(&[1, 2, 4], &[1, 2, 3])).unwrap();
        assert_eq!(both.to_rgba8(), alpha_free.to_rgba8());
        // an alpha-only mirror would put transparency on the wrong side
        assert!(matches!(crate::decode(&with(&[1, 2], &[1, 2, 3, 4])), Err(CodecError::Unsupported { .. })));
    }

    #[test]
    fn transforms_rotate_mirror_and_crop() {
        // 3 × 2, one channel, value = index
        let cv = || Canvas { w: 3, h: 2, ch: 1, data: (0..6).collect() };
        let r = apply_transforms(cv(), &[Transform::Rot(1)]);
        // anticlockwise: the right column becomes the top row
        assert_eq!((r.w, r.h, r.data), (2, 3, vec![2, 5, 1, 4, 0, 3]));
        // axis 0 exchanges top and bottom, axis 1 left and right (libheif's and libavif's reading)
        assert_eq!(apply_transforms(cv(), &[Transform::Mir(0)]).data, vec![3, 4, 5, 0, 1, 2]);
        assert_eq!(apply_transforms(cv(), &[Transform::Mir(1)]).data, vec![2, 1, 0, 5, 4, 3]);
        let c = apply_transforms(cv(), &[Transform::Clap { w: 1.0, h: 1.0, dx: 1.0, dy: 0.0 }]);
        assert_eq!((c.w, c.h), (1, 1));
        assert!(c.data == [2] || c.data == [5]);
        assert_eq!(displayed(3, 2, &[Transform::Rot(3)]), (2, 3));
        // A non-finite or empty aperture is ignored.
        assert_eq!(apply_transforms(cv(), &[Transform::Clap { w: f64::NAN, h: 1.0, dx: 0.0, dy: 0.0 }]).data.len(), 6);
    }

    #[test]
    fn hostile_files_are_errors_not_panics() {
        for file in [QUAD_420, QUAD_GBR_LOSSLESS] {
            // every truncation
            for n in 0..file.len() {
                let _ = crate::probe_size(&file[..n]);
                let _ = crate::decode(&file[..n]);
            }
            // byte flips through the container and the AV1 data
            for i in 0..file.len() {
                for x in [0x01u8, 0x80, 0xff] {
                    let mut m = file.to_vec();
                    m[i] ^= x;
                    let _ = crate::probe_size(&m);
                    let _ = crate::decode(&m);
                }
            }
        }
        // box sizes that point far outside the file
        let mut b = bx(b"ftyp", b"avif\0\0\0\0avif");
        b.extend_from_slice(b"\xff\xff\xff\xffmeta\0\0\0\0");
        assert!(crate::decode(&b).is_err());
        let mut b = bx(b"ftyp", b"avif\0\0\0\0avif");
        b.extend_from_slice(b"\0\0\0\x01meta\xff\xff\xff\xff\xff\xff\xff\xff");
        assert!(crate::probe_size(&b).is_err());
    }

    #[test]
    fn an_animated_avif_is_unsupported() {
        let mut b = bx(b"ftyp", b"avis\0\0\0\0avis");
        b.extend(bx(b"moov", &[]));
        assert!(matches!(crate::decode(&b), Err(CodecError::Unsupported { format: Format::Avif, .. })));
    }

    #[test]
    fn zero_width_iloc_extents_are_capped() {
        // iloc v2 with zero-byte offset/length/base/index fields: each extent costs no input bytes
        let mut iloc = vec![2, 0, 0, 0, 0, 0];
        iloc.extend_from_slice(&3u32.to_be_bytes());
        for id in 1..=3u32 {
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff]); // method, data ref, 65535 extents
        }
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(bx(b"iloc", &iloc));
        let mut b = bx(b"ftyp", b"avif\0\0\0\0avif");
        b.extend(bx(b"meta", &meta));
        assert!(matches!(crate::decode(&b), Err(CodecError::Malformed { message, .. }) if message.contains("extents")));
    }

    #[test]
    fn item_data_larger_than_the_file_is_refused() {
        // one item whose 1000 extents each repeat the whole file (length 0 = to the end)
        let mut iloc = vec![1, 0, 0, 0, 0x44, 0];
        iloc.extend_from_slice(&1u16.to_be_bytes());
        iloc.extend_from_slice(&[0, 1, 0, 0, 0, 0]); // id 1, method 0, data ref 0
        iloc.extend_from_slice(&1000u16.to_be_bytes());
        for _ in 0..1000 {
            iloc.extend_from_slice(&[0; 8]);
        }
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(fbx(b"pitm", &[0, 1]));
        meta.extend(bx(b"iloc", &iloc));
        let mut b = bx(b"ftyp", b"avif\0\0\0\0avif");
        b.extend(bx(b"meta", &meta));
        let c = parse(&b).unwrap();
        assert!(matches!(c.data(1), Err(CodecError::Malformed { message, .. }) if message.contains("larger than the file")));
    }

    #[test]
    fn unsupported_matrices_are_errors() {
        for m in [8u16, 10, 11, 13, 14] {
            assert!(matches!(coefficients(m, 1), Err(CodecError::Unsupported { .. })), "{m}");
        }
        assert_eq!(coefficients(0, 1).unwrap(), None);
        assert!(coefficients(12, 9).unwrap().is_some());
    }

    #[test]
    fn chromaticity_derived_matrix_reads_unspecified_primaries_as_bt709() {
        // Matrix 12 derives Kr and Kb from the primaries; `linearize` reads primaries 2 as BT.709
        // for the HLG gain, so reconstruction uses the same weights.
        let [kr, _, kb] = crate::cicp::luma_weights(crate::cicp::primaries_or_bt709(2));
        assert_eq!(coefficients(12, 2).unwrap(), Some((kr, kb)));
        assert_eq!(coefficients(12, 2).unwrap(), coefficients(12, 1).unwrap());
        assert_ne!(coefficients(12, 9).unwrap(), coefficients(12, 1).unwrap());
    }

    #[cfg(feature = "avif")]
    mod with_the_encoder {
        use super::*;
        use crate::codecs::avif::{HdrTransfer, encode_avif_hdr, encode_avif_with_speed};

        /// A smooth test card with a saturated band.
        fn card(w: usize, h: usize, ch: usize) -> Vec<u8> {
            (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let px =
                        [(x * 255 / (w - 1)) as u8, (y * 255 / (h - 1)) as u8, if x < w / 4 { 230 } else { 40 }, (255 - x * 200 / (w - 1)) as u8];
                    px.into_iter().take(ch)
                })
                .collect()
        }

        #[test]
        fn ravif_rgb_round_trip_is_10_bit() {
            let (w, h) = (48, 32);
            let src = Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, card(w, h, 3)).unwrap();
            let file = encode_avif_with_speed(&src, 95, 10).unwrap();
            let img = crate::decode(&file).unwrap();
            let (with_depth, depth) = crate::decode_avif_with_depth(&file, &crate::DecodeOptions::default()).unwrap();
            assert_eq!(depth, 10);
            assert_eq!(with_depth, img);
            assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (48, 32, ChannelLayout::Rgb, SampleType::U16));
            let diff = mean_abs_diff(&rgb8(&img), &card(w, h, 3));
            assert!(diff < 2.5, "mean difference {diff}");
        }

        #[test]
        fn ravif_alpha_round_trips() {
            let (w, h) = (40, 24);
            let src = Image::from_u8(w as u32, h as u32, ChannelLayout::Rgba, card(w, h, 4)).unwrap();
            let file = encode_avif_with_speed(&src, 95, 10).unwrap();
            let img = crate::decode(&file).unwrap();
            assert_eq!(img.layout(), ChannelLayout::Rgba);
            let got = img.to_rgba8();
            let want = card(w, h, 4);
            let alpha = |v: &[u8]| v.iter().skip(3).step_by(4).copied().collect::<Vec<_>>();
            let diff = mean_abs_diff(&alpha(&got), &alpha(&want));
            assert!(diff < 2.0, "alpha mean difference {diff}");
        }

        #[test]
        fn exif_is_read_from_its_item() {
            let mut src = Image::from_u8(16, 8, ChannelLayout::Rgb, card(16, 8, 3)).unwrap();
            let exif = b"II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec();
            src.meta.exif = Some(exif.clone());
            let file = encode_avif_with_speed(&src, 80, 10).unwrap();
            let img = crate::decode_with(&file, &DecodeOptions { keep_orientation: true, ..Default::default() }).unwrap();
            assert_eq!(img.meta.exif.as_deref(), Some(exif.as_slice()));
        }

        #[test]
        fn hdr_round_trips_keep_highlights_above_white() {
            let (w, h) = (32usize, 16usize);
            // linear BT.2020: left half grey 0.18, right half 4 × SDR white
            let rgb: Vec<f32> = (0..w * h).flat_map(|i| if i % w < w / 2 { [0.18f32; 3] } else { [4.0; 3] }).collect();
            for t in [HdrTransfer::Pq, HdrTransfer::Hlg] {
                let file = encode_avif_hdr(w as u32, h as u32, &rgb, 98, 10, None, t).unwrap();
                let img = crate::decode(&file).unwrap();
                assert_eq!(img.sample_type(), SampleType::F32, "{t:?}");
                assert_eq!(img.cicp.map(|c| c.transfer), Some(Cicp::TRANSFER_LINEAR), "{t:?}");
                let v = img.to_f32_samples().unwrap();
                let at = |x: usize| v[(8 * w + x) * 3 + 1];
                assert!((at(4) / 0.18 - 1.0).abs() < 0.08, "{t:?}: grey {}", at(4));
                assert!((at(w - 4) / 4.0 - 1.0).abs() < 0.08, "{t:?}: highlight {}", at(w - 4));
            }
        }
    }
}
