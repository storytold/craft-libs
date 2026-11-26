//! PowerPoint 95 (PowerPoint 7.0) presentations, learned from files PowerPoint 95 wrote.
//!
//! Its records have a 16-byte header: u32 type, u32 instance (0xFFFF for none), u32 body length
//! and a u32 fourth word (a tag). There is no container flag: a container's body is records, and
//! the reader knows which types those are. A saved `PowerPoint Document` stream is one record tree
//! (a type-3 root holding the Document, 1000), with no edit chain or persist directory.
//!
//! Read so far: the slide size, the masters with their colour schemes and backgrounds, and the
//! slides in order with their master, layout type and background, holding their placeholders, text
//! boxes, shapes, lines, freeforms and groups, with the text, its character and paragraph
//! formatting (spacing and justified alignment included) and its anchoring ([`text`]).
//!
//! A page's object list (3000) holds shapes (3008), placeholders (3010, which add a 3011 atom: the
//! type, the size, then the index), lines (3014), freeforms (3016) and groups (3001). A shape or
//! placeholder has its geometry (3009): the shape code (its low byte; the byte above orients the
//! outline) and its adjust value, the box as left, top, right and bottom in master units (576 per
//! inch) measured from the slide's centre, then what is left of the angle after whole
//! quarter-turns, in sixteenths of a degree, counter-clockwise. A line holds 3015: its start and
//! end points, in the same units, and 3007: its arrowheads. Shapes and lines have a format (3005)
//! with their fill, line width, dashes and colour, and shadow with its offset ([`shape`]). The 24
//! shapes of the object model read as DrawingML presets, turned and flipped as drawn; a code no
//! probe showed is a rectangle. A shape that draws nothing but its text is a text box.
//!
//! A freeform holds 3017: its box (left, top, right, bottom, as a shape's), the same box again,
//! the point count and, in its low byte, 1 when it is closed; 3007, its arrowheads; its format; and
//! 3035, its points: a u16 count, then x and y as i32s, in master units from the slide's centre. A
//! closed freeform leaves out the point that repeats its first. It reads as a custom path in its
//! box ([`shape::freeform_path`]). The format's angle is not read for freeforms: no probe turned or
//! flipped one, and the two boxes of 3017 matched in every file PowerPoint 95 wrote.
//!
//! A group (3001) holds a 3002 atom and a list (2000) whose count (2001) is followed by its
//! members, read as the page's objects are, groups included. Its members keep their places on the
//! slide; the group has no box or transform of its own (PowerPoint 95 can't turn or flip one), so
//! it reads as a group whose box is the union of its members' and whose child space is that same
//! box. Groups deeper than [`MAX_DEPTH`] are left out, and members count towards the page's object
//! cap.
//!
//! PowerPoint 95 numbers placeholders its own way: on the master 1 title, 2 body, 6 date, 7 slide
//! number and 8 footer; on slides 0x0C title, 0x0D body, 0x0F centred title and 0x10 subtitle. A
//! placeholder of any other type keeps its text as a text box. The master's placeholders place the
//! caller's default master's, and the footer row places every layout's, as the PowerPoint 97 path
//! does.
//!
//! The master's text styles (the Environment's 4052 atoms, one per text type) become the master's
//! title, body and other styles. Slide text carries its own formatting in full, so these matter
//! for text added later.
//!
//! Not read yet: pictures and notes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_pptx::{
    Background, ColorRef, Defaults, Effects, Fill, Geom, LayoutType, Line, Master, PhType, Placeholder, Presentation, Rgba, Shape, ShapeId,
    ShapeKind, Size, Slide, SlideId, TextBody, Xfrm,
};

use crate::convert::{Scheme, layout_kind, layout_table, page_side, place_placeholder, same_family, scheme_theme};
use crate::record::u32_at;
use shape::Look;
use text::Env;

mod shape;
mod text;

/// Record types.
mod rt {
    pub const ROOT: u32 = 3;
    pub const DOCUMENT: u32 = 1000;
    pub const DOCUMENT_ATOM: u32 = 1001;
    pub const SLIDE_BOX: u32 = 1005;
    pub const SLIDE: u32 = 1006;
    pub const SLIDE_ATOM: u32 = 1007;
    pub const SCHEME: u32 = 1012;
    pub const LAYOUT: u32 = 1015;
    pub const MAIN_MASTER: u32 = 1016;
    pub const LIST: u32 = 2000;
    pub const COLOR_SCHEME: u32 = 2031;
    pub const OBJECTS: u32 = 3000;
    pub const GROUP: u32 = 3001;
    pub const FORMAT: u32 = 3005;
    pub const SHAPE: u32 = 3008;
    pub const GEOMETRY: u32 = 3009;
    pub const PLACEHOLDER: u32 = 3010;
    pub const PLACEHOLDER_ATOM: u32 = 3011;
    pub const LINE: u32 = 3014;
    pub const ARROWHEADS: u32 = 3007;
    pub const LINE_ATOM: u32 = 3015;
    pub const FREEFORM: u32 = 3016;
    pub const FREEFORM_ATOM: u32 = 3017;
    pub const POINTS: u32 = 3035;
    pub const TEXT: u32 = 4001;
}

/// "No instance".
const NO_INSTANCE: u32 = 0xFFFF;
/// Slides, and masters, read at most: beyond any real presentation, so a file of empty slide
/// records can't make an unbounded presentation.
const MAX_SLIDES: usize = 1 << 16;
/// Where the DocumentAtom holds the number of slides.
const SLIDE_COUNT: usize = 32;
/// Masters read at most. Each becomes a copy of the caller's default master with its layouts, so
/// the cap is lower than for slides; PowerPoint 95 writes one.
const MAX_MASTERS: usize = 256;
/// Objects read from one slide or master at most, group members included.
const MAX_OBJECTS: usize = 1 << 14;
/// Groups nest at most this deep, as the PowerPoint 97 path's containers do; deeper ones are left
/// out, so a damaged file can't make the conversion recurse without bound.
const MAX_DEPTH: usize = crate::record::MAX_DEPTH;
/// Points read from one freeform at most, as for the PowerPoint 97 path's freeforms.
const MAX_POINTS: usize = 8192;
/// Master units (576 per inch) per point.
const MU: f64 = 8.0;
/// A colour's fourth byte when the colour is the RGB given, not a scheme slot.
const RGB_GIVEN: u8 = 0xFE;

/// PowerPoint 95's default colour scheme, for a slide or master without one.
const DEFAULT_SCHEME: Scheme = [
    Rgba::rgb(0xFF, 0xFF, 0xFF),
    Rgba::rgb(0, 0, 0),
    Rgba::rgb(0x96, 0x96, 0x96),
    Rgba::rgb(0, 0, 0),
    Rgba::rgb(0x00, 0xCC, 0x99),
    Rgba::rgb(0x33, 0x33, 0xCC),
    Rgba::rgb(0xCC, 0xCC, 0xFF),
    Rgba::rgb(0xB2, 0xB2, 0xB2),
];

/// One record over borrowed bytes.
#[derive(Clone, Copy, Debug)]
pub struct Rec<'a> {
    pub kind: u32,
    pub instance: u32,
    /// The fourth word: a per-type tag, which tells some records of one type apart.
    pub tag: u32,
    /// The body, clipped to the input (a truncated last record keeps what is there).
    pub data: &'a [u8],
}

impl<'a> Rec<'a> {
    pub fn at(bytes: &'a [u8], at: usize) -> Option<Rec<'a>> {
        let word = |k: usize| u32_at(bytes, at.checked_add(k * 4)?);
        let (kind, instance, len, tag) = (word(0)?, word(1)?, word(2)?, word(3)?);
        let start = at.checked_add(16)?;
        let end = start.saturating_add(len as usize).min(bytes.len());
        Some(Rec { kind, instance, tag, data: bytes.get(start..end)? })
    }

    /// The body read as records. Only called on the types that are containers; the walk never
    /// recurses, so nesting depth can't grow the stack.
    pub fn children(&self) -> Records<'a> {
        Records { bytes: self.data, at: 0 }
    }

    pub fn child(&self, kind: u32) -> Option<Rec<'a>> {
        self.children().find(|r| r.kind == kind)
    }

    pub fn u8(&self, at: usize) -> Option<u8> {
        self.data.get(at).copied()
    }

    pub fn u32(&self, at: usize) -> Option<u32> {
        u32_at(self.data, at)
    }

    pub fn i32(&self, at: usize) -> Option<i32> {
        self.u32(at).map(|v| v as i32)
    }
}

/// The records laid end to end in a byte slice.
#[derive(Clone, Debug)]
pub struct Records<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Records<'a> {
    type Item = Rec<'a>;
    fn next(&mut self) -> Option<Rec<'a>> {
        let r = Rec::at(self.bytes, self.at)?;
        // Always past the header, so a zero-length record can't stall the walk.
        self.at = self.at.checked_add(16)?.checked_add(r.data.len())?;
        Some(r)
    }
}

/// The Document container of a PowerPoint 95 `PowerPoint Document` stream: the stream starts with
/// a type-3 record whose first child is a Document (1000) with no instance, holding a DocumentAtom
/// (1001) long enough for its slide count. `None` for anything else, PowerPoint 97 files included
/// (their 8-byte headers don't read this way).
pub fn document(stream: &[u8]) -> Option<Rec<'_>> {
    let root = Rec::at(stream, 0).filter(|r| r.kind == rt::ROOT)?;
    let document = root.children().next().filter(|d| d.kind == rt::DOCUMENT && d.instance == NO_INSTANCE)?;
    document.child(rt::DOCUMENT_ATOM)?.u32(SLIDE_COUNT)?;
    Some(document)
}

/// Whether a slide record holds what PowerPoint 95 writes first in every slide, its SlideAtom
/// (1007), whole enough to give the slide's id and its master's. One without it kept nothing (a
/// file cut short just after the slide's header, or the atom's): it is left out and named as lost.
fn kept(slide: &Rec) -> bool {
    slide.child(rt::SLIDE_ATOM).and_then(|a| a.u32(4)).is_some()
}

/// The slides a truncated or damaged file lost, by position ("slide 3"): those listed without their
/// SlideAtom, and those the DocumentAtom counts past the ones listed. Empty for a whole file.
pub fn lost(document: &Rec) -> Vec<String> {
    let count = document.child(rt::DOCUMENT_ATOM).and_then(|a| a.u32(SLIDE_COUNT)).map_or(0, |c| c as usize);
    let listed: Vec<bool> = document
        .children()
        .filter(|r| r.kind == rt::LIST)
        .flat_map(|list| list.children().filter(|r| r.kind == rt::SLIDE))
        .take(MAX_SLIDES)
        .map(|r| kept(&r))
        .collect();
    let empty = listed.iter().enumerate().filter(|(_, k)| !**k).map(|(i, _)| i + 1);
    // At most as many as could have been read, so a damaged count can't make a huge list.
    let missing = listed.len() + 1..=count.min(listed.len() + MAX_SLIDES);
    empty.chain(missing).map(|n| format!("slide {n}")).collect()
}

/// What a slide or master says about itself.
struct Page<'a> {
    rec: Rec<'a>,
    /// Its id (a master's is what its slides name).
    id: u32,
    master_id: u32,
    /// The [MS-PPT] `SlideLayoutType` number.
    layout: Option<u32>,
    /// The slide box's flags (1005 +16, +17, +20): whether the slide shows the master's objects,
    /// uses the master's colour scheme and uses the master's background. A missing box follows.
    follow_objects: bool,
    follow_scheme: bool,
    follow_background: bool,
    scheme: Option<Scheme>,
}

fn page(rec: Rec<'_>) -> Page<'_> {
    let atom = rec.child(rt::SLIDE_ATOM);
    let flag = |at: usize| rec.child(rt::SLIDE_BOX).and_then(|b| b.u8(at)).is_none_or(|f| f != 0);
    Page {
        rec,
        id: atom.and_then(|a| a.u32(0)).unwrap_or(0),
        master_id: atom.and_then(|a| a.u32(4)).unwrap_or(0),
        layout: rec.child(rt::LAYOUT).and_then(|l| l.u32(0)),
        follow_objects: flag(16),
        follow_scheme: flag(17),
        follow_background: flag(20),
        scheme: rec.child(rt::SCHEME).and_then(|s| s.child(rt::COLOR_SCHEME)).map(|s| scheme_of(&s)),
    }
}

/// A `ColorSchemeAtom`: a count, then that many RGBX colours (the first eight are used).
fn scheme_of(r: &Rec) -> Scheme {
    let mut s = DEFAULT_SCHEME;
    let count = r.u32(0).map_or(0, |c| c.min(8) as usize);
    for (i, c) in s.iter_mut().enumerate().take(count) {
        if let (Some(red), Some(g), Some(b)) = (r.u8(4 + i * 4), r.u8(5 + i * 4), r.u8(6 + i * 4)) {
            *c = Rgba::rgb(red, g, b);
        }
    }
    s
}

/// The fill of a page's background shape: its own shape before the object list.
fn background(rec: &Rec, scheme: &Scheme) -> Option<Fill> {
    let shape = rec.children().take_while(|r| r.kind != rt::OBJECTS).find(|r| r.kind == rt::SHAPE)?;
    let format = shape.child(rt::FORMAT)?;
    if format.u8(12)? == 0xFF {
        return Some(Fill::None);
    }
    Some(Fill::solid(ColorRef::rgb(shape::color(&format, 20, scheme)?)))
}

/// A PowerPoint 95 placeholder number as a placeholder type. PowerPoint 95 numbers them its own
/// way, not as [MS-PPT] does: 1 to 8 on the master, 0x0C and up on slides.
fn ph_type(kind: u8) -> Option<PhType> {
    Some(match kind {
        0x01 | 0x0C => PhType::Title,
        0x02 | 0x0D => PhType::Body,
        0x06 => PhType::Date,
        0x07 => PhType::SlideNum,
        0x08 => PhType::Footer,
        0x0F => PhType::CtrTitle,
        0x10 => PhType::SubTitle,
        _ => return None,
    })
}

/// One object of a page's list, read but not given an id yet.
struct Object {
    /// A placeholder (3010), whether or not its type is known.
    placeholder: bool,
    /// The placeholder it is, when its type is known.
    ph: Option<Placeholder>,
    xfrm: Xfrm,
    text: Option<TextBody>,
    /// The ruler slot its text names.
    ruler: Option<u32>,
    /// A shape's preset (a rectangle for a shape code no probe showed): `None` for a placeholder.
    geom: Option<Geom>,
    /// How a shape or line is drawn (a placeholder's is left to its layout).
    look: Look,
    kind: Kind,
}

/// What an object is, beyond its box, text, geometry and look.
enum Kind {
    /// A shape (3008) or placeholder (3010).
    Shape,
    /// A line (3014).
    Line,
    /// A freeform (3016).
    Freeform,
    /// A group (3001) and its members.
    Group(Vec<Object>),
}

/// Reads a page's objects, counting them against [`MAX_OBJECTS`] across its groups.
struct Reader<'e> {
    env: &'e Env,
    scheme: &'e Scheme,
    size: Size,
    /// Objects still to be read on this page.
    left: usize,
}

impl Reader<'_> {
    /// The objects of a page's object list.
    fn page(env: &Env, scheme: &Scheme, size: Size, page: &Rec) -> Vec<Object> {
        let mut reader = Reader { env, scheme, size, left: MAX_OBJECTS };
        page.child(rt::OBJECTS).map(|l| reader.list(&l, 0)).unwrap_or_default()
    }

    /// The shapes, placeholders, lines, freeforms and groups of a list `depth` groups down.
    fn list(&mut self, list: &Rec, depth: usize) -> Vec<Object> {
        let mut out = vec![];
        for r in list.children().filter(|r| matches!(r.kind, rt::SHAPE | rt::PLACEHOLDER | rt::LINE | rt::FREEFORM | rt::GROUP)) {
            let Some(left) = self.left.checked_sub(1) else {
                log::warn!("ppt: a PowerPoint 95 page has more than {MAX_OBJECTS} objects; the rest are left out");
                break;
            };
            self.left = left;
            out.extend(match r.kind {
                rt::GROUP => self.group(&r, depth),
                rt::FREEFORM => freeform(&r, self.env, self.scheme, self.size),
                _ => object(&r, self.env, self.scheme, self.size),
            });
        }
        out
    }

    /// A group `depth` groups down, with its members read as the page's objects are. The walk
    /// recurses once per group, so a group nested deeper than [`MAX_DEPTH`] is left out.
    fn group(&mut self, r: &Rec, depth: usize) -> Option<Object> {
        if depth >= MAX_DEPTH {
            log::warn!("ppt: PowerPoint 95 groups nest deeper than {MAX_DEPTH}; the deeper ones are left out");
            return None;
        }
        let members = self.list(&r.child(rt::LIST)?, depth + 1);
        Some(Object {
            placeholder: false,
            ph: None,
            xfrm: Xfrm::default(),
            text: None,
            ruler: None,
            geom: None,
            look: Look::none(),
            kind: Kind::Group(members),
        })
    }
}

/// A box from master units about the slide's centre (left, top, right, bottom, either way round)
/// to points from its top left corner.
fn xfrm_of([l, t, right, b]: [i32; 4], size: Size) -> Xfrm {
    let x = |v: i32| f64::from(v) / MU + size.width / 2.0;
    let y = |v: i32| f64::from(v) / MU + size.height / 2.0;
    Xfrm { x: x(l).min(x(right)), y: y(t).min(y(b)), w: (x(right) - x(l)).abs(), h: (y(b) - y(t)).abs(), rot: 0.0, flip_h: false, flip_v: false }
}

/// A line: the box its ends span, flipped so that it runs from its start to its end, drawn as its
/// format says (1 pt in the scheme's text and lines colour without one), with its arrowheads.
fn line(r: &Rec, scheme: &Scheme, size: Size) -> Option<Object> {
    let a = r.child(rt::LINE_ATOM)?;
    let (x1, y1, x2, y2) = (a.i32(0)?, a.i32(4)?, a.i32(8)?, a.i32(12)?);
    let mut xfrm = xfrm_of([x1, y1, x2, y2], size);
    (xfrm.flip_h, xfrm.flip_v) = (x2 < x1, y2 < y1);
    let mut look = match r.child(rt::FORMAT) {
        Some(f) => shape::look(&f, scheme),
        None => Look { line: shape::line_of(scheme[1], None), ..Look::none() },
    };
    // A line has no inside to fill.
    look.fill = Fill::None;
    if let Some(heads) = r.child(rt::ARROWHEADS).filter(|_| look.line != Line::none()) {
        shape::arrowheads(&mut look.line, &heads);
    }
    let geom = Some(Geom::Preset { name: "line".into(), adj: vec![] });
    Some(Object { placeholder: false, ph: None, xfrm, text: None, ruler: None, geom, look, kind: Kind::Line })
}

/// A freeform: its points as a path in its box, drawn as its format says (1 pt in the scheme's
/// text and lines colour without one), with its arrowheads when it is open. One of fewer than two
/// points draws nothing and is left out; one flagged closed with fewer than three is drawn open.
fn freeform(r: &Rec, env: &Env, scheme: &Scheme, size: Size) -> Option<Object> {
    let a = r.child(rt::FREEFORM_ATOM)?;
    let sides = [a.i32(0)?, a.i32(4)?, a.i32(8)?, a.i32(12)?];
    let points = r.child(rt::POINTS).map(|p| points(&p)).unwrap_or_default();
    if points.len() < 2 {
        return None;
    }
    // Only the low byte is the flag: PowerPoint 95 leaves whatever was there in the bytes above.
    let closed = a.u8(36) == Some(1) && points.len() >= 3;
    let mut look = match r.child(rt::FORMAT) {
        Some(f) => shape::look(&f, scheme),
        None => Look { line: shape::line_of(scheme[1], None), ..Look::none() },
    };
    if let Some(heads) = r.child(rt::ARROWHEADS).filter(|_| !closed && look.line != Line::none()) {
        shape::arrowheads(&mut look.line, &heads);
    }
    let geom = Some(shape::freeform_path(&points, sides, closed));
    let container = r.child(rt::TEXT);
    let text = container.and_then(|c| text::text_body(&c, env, scheme, Some(sides), true));
    let ruler = container.and_then(|c| text::ruler_slot(&c));
    Some(Object { placeholder: false, ph: None, xfrm: xfrm_of(sides, size), text, ruler, geom, look, kind: Kind::Freeform })
}

/// A 3035 atom's points: a u16 count, then x and y as i32s. No more than the record holds, nor
/// than [`MAX_POINTS`].
fn points(r: &Rec) -> Vec<(i32, i32)> {
    let count = crate::record::u16_at(r.data, 0).map_or(0, usize::from);
    (0..count.min(MAX_POINTS)).map_while(|i| Some((r.i32(2 + i * 8)?, r.i32(6 + i * 8)?))).collect()
}

/// A shape, placeholder or line: its box, its placeholder atom, its text, and a shape's preset and
/// look.
fn object(r: &Rec, env: &Env, scheme: &Scheme, size: Size) -> Option<Object> {
    if r.kind == rt::LINE {
        return line(r, scheme, size);
    }
    let g = r.child(rt::GEOMETRY)?;
    let sides = [g.i32(12)?, g.i32(16)?, g.i32(20)?, g.i32(24)?];
    let mut xfrm = xfrm_of(sides, size);
    let format = r.child(rt::FORMAT);
    let placeholder = r.kind == rt::PLACEHOLDER;
    // The shape code is the low byte; for codes of 0 and up the byte above it orients the outline.
    let code = g.u8(0).map(|c| c as i8);
    let drawn = g.i32(28).unwrap_or(0);
    match code {
        Some(c) if c >= 0 && !placeholder => shape::orient_drawn(&mut xfrm, g.u8(1).unwrap_or(0), drawn),
        // The angle the user set is the format's; the geometry's is what quarter-turns leave of it.
        _ => shape::orient(&mut xfrm, drawn, format.and_then(|f| f.i32(48)).unwrap_or(drawn)),
    }
    let ph = r.child(rt::PLACEHOLDER_ATOM).filter(|_| placeholder).and_then(|a| {
        let size = match a.u8(1) {
            Some(1) => Some("half".into()),
            Some(2) => Some("quarter".into()),
            _ => None,
        };
        Some(Placeholder { kind: ph_type(a.u8(0)?)?, idx: a.u32(4).unwrap_or(0), size, ..Default::default() })
    });
    let container = r.child(rt::TEXT);
    let mut text = container.and_then(|c| text::text_body(&c, env, scheme, Some(sides), ph.is_none()));
    // The quarter-turns between the box the geometry holds and the shape's own.
    let quarters = shape::quarter_turns(&xfrm, drawn);
    let ruler = container.and_then(|c| text::ruler_slot(&c));
    let (mut geom, mut look) = (None, Look::none());
    if !placeholder {
        let height = xfrm.h;
        geom = code.map(|c| shape::geometry(c, g.i32(4), &mut xfrm));
        // A balloon's box is its body, above its tail: the text's edge there stays where it was. The
        // cut is the shape's bottom, which is the held box's edge `quarters` on from the bottom.
        if let Some(b) = text.as_mut().map(|t| &mut t.body).filter(|_| height != xfrm.h) {
            let inset = [&mut b.inset_l, &mut b.inset_t, &mut b.inset_r, &mut b.inset_b].into_iter().nth((3 + quarters) % 4);
            if let Some(inset) = inset {
                *inset = inset.map(|v| (v - (height - xfrm.h)).max(0.0));
            }
        }
        if let Some(f) = format {
            look = shape::look(&f, scheme);
        }
    }
    // PowerPoint 95 lays a shape's text out in the box the geometry holds, turned by the text's own
    // angle (4003 +16, the geometry's own angle in every probe): quarter-turns folded into that box
    // don't turn the text. When the shape ends up at another angle (its box turned back, or the
    // half-turn a mirrored trapezoid takes), the text is upright on the slide, at its own angle,
    // with its insets on the held box's edges by name, as PowerPoint lays out `upright` text
    // (checked in PowerPoint 16 against PowerPoint 95's renderings).
    let angle = shape::clockwise(i64::from(container.and_then(|c| text::angle(&c)).unwrap_or(drawn)));
    if let Some(t) = text.as_mut().filter(|_| (xfrm.rot - angle).rem_euclid(360.0) != 0.0) {
        t.body.upright = Some(true);
        t.body.rot = Some(angle).filter(|r| *r != 0.0);
    }
    Some(Object { placeholder, ph, xfrm, text, ruler, geom, look, kind: Kind::Shape })
}

/// An object as a shape: a placeholder of a known type; a text box when it draws nothing but its
/// text; else a shape or line that draws something. One that draws nothing is left out. A
/// placeholder of an unknown type keeps its text as a text box.
fn shape(p: &mut Presentation, o: Object) -> Option<Shape> {
    if let Kind::Group(members) = o.kind {
        return group(p, members);
    }
    let Some(ph) = o.ph else {
        let text = o.text.filter(|t| !t.is_empty());
        if text.is_some() && !o.look.drawn() {
            let mut xfrm = o.xfrm;
            (xfrm.flip_h, xfrm.flip_v) = (false, false);
            return Some(Shape {
                id: ShapeId(p.alloc_id()?),
                name: "Text Box".into(),
                xfrm: Some(xfrm),
                text_box: true,
                fill: Some(Fill::None),
                line: Some(Line::none()),
                text,
                ..Default::default()
            });
        }
        let geom = o.geom.filter(|_| o.look.drawn())?;
        let id = p.alloc_id()?;
        let Look { fill, line, shadow, .. } = o.look;
        return Some(Shape {
            id: ShapeId(id),
            name: match o.kind {
                Kind::Line => format!("Line {id}"),
                Kind::Freeform => format!("Freeform {id}"),
                _ => format!("Shape {id}"),
            },
            xfrm: Some(o.xfrm),
            geom,
            fill: Some(fill),
            line: Some(line),
            effects: shadow.map(|s| Effects { outer_shadow: Some(s), ..Default::default() }),
            text,
            ..Default::default()
        });
    };
    let name = match ph.kind {
        PhType::Title | PhType::CtrTitle => "Title",
        PhType::SubTitle => "Subtitle",
        PhType::Date => "Date",
        PhType::Footer => "Footer",
        PhType::SlideNum => "Slide Number",
        _ => "Text",
    };
    // An empty placeholder draws nothing (its prompt shows only while editing).
    let text = Some(o.text.unwrap_or_default());
    Some(Shape { id: ShapeId(p.alloc_id()?), name: name.into(), xfrm: Some(o.xfrm), ph: Some(ph), text, ..Default::default() })
}

/// A group of the shapes its members are, in the box they span, which is also its child space, so
/// the members keep their places on the slide. A group none of whose members draws anything is
/// left out.
///
/// A nested group recurses here directly, not through [`shape`], whose frame is far larger: the
/// recursion is bounded by [`MAX_DEPTH`], and each level should cost the stack little.
fn group(p: &mut Presentation, members: Vec<Object>) -> Option<Shape> {
    let id = p.alloc_id()?;
    let mut children = Vec::with_capacity(members.len());
    for m in members {
        let s = match m.kind {
            Kind::Group(inner) => group(p, inner),
            _ => shape(p, m),
        };
        children.extend(s);
    }
    let mut boxes = children.iter().filter_map(|c| c.xfrm);
    let first = boxes.next()?;
    let (mut l, mut t, mut r, mut b) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for x in boxes {
        (l, t, r, b) = (l.min(x.x), t.min(x.y), r.max(x.x + x.w), b.max(x.y + x.h));
    }
    let xfrm = Xfrm::new(l, t, r - l, b - t);
    Some(Shape {
        id: ShapeId(id),
        name: format!("Group {id}"),
        xfrm: Some(xfrm),
        kind: ShapeKind::Group { children, child: xfrm },
        ..Default::default()
    })
}

/// Build the presentation from a PowerPoint 95 Document container.
pub fn convert(document: &Rec, app: &dyn Defaults) -> Presentation {
    let atom = document.child(rt::DOCUMENT_ATOM);
    let field = |at: usize| atom.and_then(|a| a.i32(at));
    let size = Size::new(page_side(field(0), 720.0), page_side(field(4), 540.0));
    let mut p = Presentation { slide_size: size, default_text_style: app.default_text_style(), next_id: 256, ..Default::default() };
    p.notes_size = Size::new(page_side(field(8), 540.0), page_side(field(12), 720.0));

    let env = Env::new(document);

    // The lists are told apart by what they hold, in whatever order they come.
    let (mut slides, mut masters) = (vec![], vec![]);
    for list in document.children().filter(|r| r.kind == rt::LIST) {
        for r in list.children() {
            match r.kind {
                rt::SLIDE if slides.len() < MAX_SLIDES => slides.push(page(r)),
                rt::MAIN_MASTER if masters.len() < MAX_MASTERS => masters.push(page(r)),
                _ => {}
            }
        }
    }

    // Each master: the caller's default master with the file's colours and background.
    let mut by_id: HashMap<u32, (usize, Scheme)> = HashMap::new();
    for m in &masters {
        let scheme = m.scheme.unwrap_or(DEFAULT_SCHEME);
        let mut master = default_master(&mut p, app, size, Some(scheme));
        let index = p.masters.len();
        // The scheme's background is what the master shows anyway.
        master.background = background(&m.rec, &scheme).filter(|f| *f != Fill::solid(ColorRef::rgb(scheme[0]))).map(|fill| Background::Fill { fill });
        // The master's placeholders set where new slides' placeholders go: the first of each
        // family places the default master's. Its other objects are the master's own shapes.
        let mut placed: Vec<PhType> = vec![];
        // The rulers of the master's title and body, which give their styles' margins.
        let (mut title_ruler, mut body_ruler) = (None, None);
        for o in Reader::page(&env, &scheme, size, &m.rec) {
            if o.placeholder {
                if let Some(kind) = o.ph.as_ref().map(|ph| ph.kind).filter(|k| !placed.iter().any(|d| same_family(*d, *k))) {
                    placed.push(kind);
                    match kind {
                        PhType::Title | PhType::CtrTitle => title_ruler = o.ruler,
                        PhType::Body => body_ruler = o.ruler,
                        _ => {}
                    }
                    let text = o.text;
                    place_placeholder(&mut master, kind, o.xfrm, || text.and_then(|t| t.paragraphs.into_iter().next()));
                }
                continue;
            }
            if let Some(s) = shape(&mut p, o) {
                master.shapes.push(s);
            }
        }
        // The master's text styles replace the defaults' first five levels.
        for (style, ty, ruler) in [
            (&mut master.title_style, text::TITLE, title_ruler),
            (&mut master.body_style, text::BODY, body_ruler),
            (&mut master.other_style, text::OTHER, None),
        ] {
            let Some(read) = text::master_style(&env, &scheme, ty, ruler) else { continue };
            for level in 0..5u8 {
                if let Some(l) = read.level(level) {
                    style.set(level, l.clone());
                }
            }
        }
        p.masters.push(Arc::new(master));
        by_id.entry(m.id).or_insert((index, scheme));
    }
    if p.masters.is_empty() {
        let m = default_master(&mut p, app, size, None);
        p.masters.push(Arc::new(m));
    }

    let layouts: Vec<_> = p.masters.iter().map(|m| layout_table(m, &HashSet::new())).collect();
    for s in slides.iter().filter(|s| kept(&s.rec)) {
        // An unknown master is the first one.
        let (index, master_scheme) =
            by_id.get(&s.master_id).copied().unwrap_or((0, masters.first().and_then(|m| m.scheme).unwrap_or(DEFAULT_SCHEME)));
        let Some((kinds, first)) = layouts.get(index) else { continue };
        let kind: Option<LayoutType> = s.layout.map(layout_kind);
        let Some(layout) = kind.and_then(|k| kinds.iter().find(|(lk, _)| *lk == k).map(|(_, id)| *id)).or(*first) else { continue };
        // A slide following the master's scheme holds a copy of it; its own is used only when it
        // doesn't follow.
        let scheme = if s.follow_scheme { master_scheme } else { s.scheme.unwrap_or(master_scheme) };
        let background = if s.follow_background { None } else { background(&s.rec, &scheme).map(|fill| Background::Fill { fill }) };
        let Some(id) = p.alloc_id() else { break };
        let mut shapes = vec![];
        for o in Reader::page(&env, &scheme, size, &s.rec) {
            shapes.extend(shape(&mut p, o));
        }
        p.slides.push(Arc::new(Slide { id: SlideId(id), layout, background, shapes, show_master_shapes: s.follow_objects, ..Default::default() }));
    }
    p
}

/// The caller's default master, with ids above those handed out so far, wearing `scheme`.
fn default_master(p: &mut Presentation, app: &dyn Defaults, size: Size, scheme: Option<Scheme>) -> Master {
    let (mut master, last) = app.master(size, p.next_id);
    p.next_id = p.next_id.max(last);
    if let Some(scheme) = scheme {
        let theme = scheme_theme(&scheme);
        master.name = theme.name.clone();
        master.theme = theme;
    }
    master
}

#[cfg(test)]
mod tests;
