//! PowerPoint 95 (PowerPoint 7.0) presentations, learned from files PowerPoint 95 wrote.
//!
//! Its records have a 16-byte header: u32 type, u32 instance (0xFFFF for none), u32 body length
//! and a u32 fourth word (a tag). There is no container flag: a container's body is records, and
//! the reader knows which types those are. A saved `PowerPoint Document` stream is one record tree
//! (a type-3 root holding the Document, 1000), with no edit chain or persist directory.
//!
//! Read so far: the slide size, the masters with their colour schemes and backgrounds, and the
//! slides in order with their master, layout type and background. Shapes, text, pictures and
//! notes are not read yet, so slides come out empty.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_pptx::{Background, ColorRef, Defaults, Fill, LayoutType, Master, Presentation, Rgba, Size, Slide, SlideId};

use crate::convert::{Scheme, layout_kind, layout_table, page_side, scheme_theme};
use crate::record::u32_at;

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
    /// The body, clipped to the input (a truncated last record keeps what is there).
    pub data: &'a [u8],
}

impl<'a> Rec<'a> {
    pub fn at(bytes: &'a [u8], at: usize) -> Option<Rec<'a>> {
        let word = |k: usize| u32_at(bytes, at.checked_add(k * 4)?);
        // The fourth word, a tag, isn't needed yet; it must be there for the header to be whole.
        let (kind, instance, len, _tag) = (word(0)?, word(1)?, word(2)?, word(3)?);
        let start = at.checked_add(16)?;
        let end = start.saturating_add(len as usize).min(bytes.len());
        Some(Rec { kind, instance, data: bytes.get(start..end)? })
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
    let (r, g, b, index) = (format.u8(20)?, format.u8(21)?, format.u8(22)?, format.u8(23)?);
    let color = if index == RGB_GIVEN { Rgba::rgb(r, g, b) } else { *scheme.get(usize::from(index))? };
    Some(Fill::solid(ColorRef::rgb(color)))
}

/// Build the presentation from a PowerPoint 95 Document container.
pub fn convert(document: &Rec, app: &dyn Defaults) -> Presentation {
    let atom = document.child(rt::DOCUMENT_ATOM);
    let field = |at: usize| atom.and_then(|a| a.i32(at));
    let size = Size::new(page_side(field(0), 720.0), page_side(field(4), 540.0));
    let mut p = Presentation { slide_size: size, default_text_style: app.default_text_style(), next_id: 256, ..Default::default() };
    p.notes_size = Size::new(page_side(field(8), 540.0), page_side(field(12), 720.0));

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
        p.slides.push(Arc::new(Slide { id: SlideId(id), layout, background, show_master_shapes: s.follow_objects, ..Default::default() }));
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
