//! PowerPoint 95 (PowerPoint 7.0) presentations, learned from files PowerPoint 95 wrote.
//!
//! Its records have a 16-byte header: u32 type, u32 instance (0xFFFF for none), u32 body length
//! and a u32 fourth word (a tag). There is no container flag: a container's body is records, and
//! the reader knows which types those are. A saved `PowerPoint Document` stream is one record tree
//! (a type-3 root holding the Document, 1000), with no edit chain or persist directory.
//!
//! Read so far: the slide size, the masters with their colour schemes and backgrounds, and the
//! slides in order with their master, layout type and background, holding their placeholders and
//! text boxes with the text, its character and paragraph formatting (spacing and justified
//! alignment included) and its anchoring ([`text`]).
//!
//! A page's object list (3000) holds shapes (3008) and placeholders (3010, which add a 3011 atom:
//! the type, the size, then the index). Each has its geometry (3009): the box as left, top, right
//! and bottom in master units (576 per inch) measured from the slide's centre, then the angle in
//! sixteenths of a degree. PowerPoint 95 numbers placeholders its own way: on the master 1 title,
//! 2 body, 6 date, 7 slide number and 8 footer; on slides 0x0C title, 0x0D body, 0x0F centred
//! title and 0x10 subtitle. A placeholder of any other type keeps its text as a text box. The
//! master's placeholders place the caller's default master's, and the footer row places every
//! layout's, as the PowerPoint 97 path does.
//!
//! The master's text styles (the Environment's 4052 atoms, one per text type) become the master's
//! title, body and other styles. Slide text carries its own formatting in full, so these matter
//! for text added later.
//!
//! Not read yet: shapes without text, and shapes' own fill, line and shadow; pictures; and notes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_pptx::{
    Background, ColorRef, Defaults, Fill, LayoutType, Line, Master, PhType, Placeholder, Presentation, Rgba, Shape, ShapeId, Size, Slide, SlideId,
    TextBody, Xfrm,
};

use crate::convert::{Scheme, layout_kind, layout_table, page_side, place_placeholder, same_family, scheme_theme};
use crate::record::u32_at;
use text::Env;

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
    pub const FORMAT: u32 = 3005;
    pub const SHAPE: u32 = 3008;
    pub const GEOMETRY: u32 = 3009;
    pub const PLACEHOLDER: u32 = 3010;
    pub const PLACEHOLDER_ATOM: u32 = 3011;
    pub const TEXT: u32 = 4001;
}

/// "No instance".
const NO_INSTANCE: u32 = 0xFFFF;
/// Slides, and masters, read at most: beyond any real presentation, so a file of empty slide
/// records can't make an unbounded presentation.
const MAX_SLIDES: usize = 1 << 16;
/// Masters read at most. Each becomes a copy of the caller's default master with its layouts, so
/// the cap is lower than for slides; PowerPoint 95 writes one.
const MAX_MASTERS: usize = 256;
/// Objects read from one slide or master at most.
const MAX_OBJECTS: usize = 1 << 14;
/// Master units (576 per inch) per point.
const MU: f64 = 8.0;
/// The fill colour's fourth byte when the colour is the RGB given, not a scheme slot.
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
/// a type-3 record whose first child is a Document (1000) with no instance. `None` for anything
/// else, PowerPoint 97 files included (their 8-byte headers don't read this way).
pub fn document(stream: &[u8]) -> Option<Rec<'_>> {
    let root = Rec::at(stream, 0).filter(|r| r.kind == rt::ROOT)?;
    root.children().next().filter(|d| d.kind == rt::DOCUMENT && d.instance == NO_INSTANCE)
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
    let (r, g, b, index) = (format.u8(20)?, format.u8(21)?, format.u8(22)?, format.u8(23)?);
    let color = if index == RGB_GIVEN { Rgba::rgb(r, g, b) } else { *scheme.get(usize::from(index))? };
    Some(Fill::solid(ColorRef::rgb(color)))
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
}

/// The shapes and placeholders of a page's object list.
fn objects<'a>(page: &Rec<'a>) -> impl Iterator<Item = Rec<'a>> {
    page.child(rt::OBJECTS).into_iter().flat_map(|l| l.children()).filter(|r| matches!(r.kind, rt::SHAPE | rt::PLACEHOLDER)).take(MAX_OBJECTS)
}

/// A shape or placeholder: its box, from master units about the slide's centre to points from
/// its top left corner, its placeholder atom and its text.
fn object(r: &Rec, env: &Env, scheme: &Scheme, size: Size) -> Option<Object> {
    let g = r.child(rt::GEOMETRY)?;
    let sides = [g.i32(12)?, g.i32(16)?, g.i32(20)?, g.i32(24)?];
    let [l, t, right, b] = sides;
    let x = |v: i32| f64::from(v) / MU + size.width / 2.0;
    let y = |v: i32| f64::from(v) / MU + size.height / 2.0;
    // The box is the unturned one; the angle is in sixteenths of a degree.
    let rot = g.i32(28).map_or(0.0, |v| (f64::from(v) / 16.0).rem_euclid(360.0));
    let xfrm =
        Xfrm { x: x(l).min(x(right)), y: y(t).min(y(b)), w: (x(right) - x(l)).abs(), h: (y(b) - y(t)).abs(), rot, flip_h: false, flip_v: false };
    let placeholder = r.kind == rt::PLACEHOLDER;
    let ph = r.child(rt::PLACEHOLDER_ATOM).filter(|_| placeholder).and_then(|a| {
        let size = match a.u8(1) {
            Some(1) => Some("half".into()),
            Some(2) => Some("quarter".into()),
            _ => None,
        };
        Some(Placeholder { kind: ph_type(a.u8(0)?)?, idx: a.u32(4).unwrap_or(0), size, ..Default::default() })
    });
    let container = r.child(rt::TEXT);
    let text = container.and_then(|c| text::text_body(&c, env, scheme, Some(sides), ph.is_none()));
    let ruler = container.and_then(|c| text::ruler_slot(&c));
    Some(Object { placeholder, ph, xfrm, text, ruler })
}

/// An object as a shape: a placeholder of a known type, else a text box when it has text (a shape
/// with no text is not read yet). A placeholder of an unknown type keeps its text as a text box.
fn shape(p: &mut Presentation, o: Object) -> Option<Shape> {
    let Some(ph) = o.ph else {
        let text = o.text.filter(|t| !t.is_empty())?;
        return Some(Shape {
            id: ShapeId(p.alloc_id()?),
            name: "Text Box".into(),
            xfrm: Some(o.xfrm),
            text_box: true,
            fill: Some(Fill::None),
            line: Some(Line::none()),
            text: Some(text),
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
        for o in objects(&m.rec).filter_map(|r| object(&r, &env, &scheme, size)) {
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
    for s in &slides {
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
        for o in objects(&s.rec).filter_map(|r| object(&r, &env, &scheme, size)) {
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
