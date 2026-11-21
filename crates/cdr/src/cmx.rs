//! Reader for `.cmx` exchange files with 32-bit coordinates: the RIFF form `CMX1` (R53), its
//! header (R54), the color, pen, line type, dash, outline and arrowhead tables (R55 to R59) and the
//! instructions of each page (R60): pages, layers, groups and curves (R61, R62) with uniform fills
//! and outlines (R63). Text, images, rectangles, ellipses, clipping paths, other fills and lenses
//! are left out with a warning.

use crate::bytes::{Rd, u32_at};
use crate::geometry::path_of;
use crate::layout::{self, Profile};
use crate::limits::Budget;
use crate::model::{Cap, Color, Drawing, Geometry, Join, Layer, LayerKind, Matrix, Object, Outline, Page, Pt, Shape};
use crate::records::{name, property_color};
use crate::riff::{LIST, push};
use crate::{Error, Warning};

const FORM: [u8; 4] = *b"CMX1";
/// Meters per coordinate unit when the header gives no usable value: the 0.1 µm of every sample
/// file with 32-bit coordinates (R54).
const DEFAULT_UNIT: f64 = 1e-7;
/// How far past a chunk that runs short the next chunk id is looked for (R53).
const RESYNC: usize = 64;
/// The chunks the reader uses (R53 to R60).
const READ_IDS: [[u8; 4]; 9] = [*b"LIST", *b"cont", *b"page", *b"rclr", *b"rpen", *b"rott", *b"rdot", *b"rotl", *b"rota"];
/// The most UTF-16 units read of a layer's name; [`name`] keeps 256 of them.
const MAX_NAME_UNITS: usize = 4096;

// Instruction codes (R60).
const BEGIN_PAGE: i16 = 9;
const END_PAGE: i16 = 10;
const BEGIN_LAYER: i16 = 11;
const END_LAYER: i16 = 12;
const BEGIN_GROUP: i16 = 13;
const END_GROUP: i16 = 14;
const BEGIN_TEXT: i16 = 20;
const END_TEXT: i16 = 21;
const GLYPH_RUN: i16 = 65;
const ELLIPSE: i16 = 66;
const CURVE: i16 = 67;
const RECTANGLE: i16 = 68;
const IMAGE: i16 = 69;
const ADD_CLIP: i16 = 88;
const REMOVE_CLIP: i16 = 89;
const JUMP: i16 = 111;

/// Does `bytes` start as a CMX file: a RIFF or RIFX form of type `CMX1` (R53)?
pub(crate) fn is_cmx(bytes: &[u8]) -> bool {
    matches!(bytes.get(0..4), Some(b"RIFF" | b"RIFX")) && bytes.get(8..12) == Some(FORM.as_slice())
}

/// A chunk outside the `LIST` chunks: its id, its body and where the body starts in the file.
struct Chunk<'a> {
    id: [u8; 4],
    body: &'a [u8],
    start: usize,
}

/// The tables that page instructions refer to by 1-based index (R55 to R59).
#[derive(Default)]
struct Tables {
    colors: Vec<Option<Color>>,
    /// Outline widths in points.
    pens: Vec<Option<f64>>,
    /// Flags, and the caps and join.
    line_types: Vec<Option<(u8, u8)>>,
    dashes: Vec<Vec<u16>>,
    /// Line type, screen, color, arrowheads, pen and dashes.
    outlines: Vec<Option<[u16; 6]>>,
    arrowheads: Vec<(u16, u16)>,
}

pub(crate) fn drawing(bytes: &[u8], budget: &mut Budget) -> Result<Drawing, Error> {
    if !is_cmx(bytes) {
        return Err(Error::NotCmx);
    }
    if bytes.get(0..4) == Some(b"RIFX") {
        return Err(Error::Unsupported("big-endian CMX files"));
    }
    let mut warn = Vec::new();
    let chunks = chunks(bytes, budget, &mut warn)?;
    let cont = chunks.iter().find(|c| c.id == *b"cont").map(|c| c.body).ok_or(Error::Damaged("CMX header"))?;
    match cont.get(52) {
        Some(b'4') => {}
        Some(b'2') => return Err(Error::Unsupported("CMX files with 16-bit coordinates")),
        _ => return Err(Error::Damaged("CMX header")),
    }
    let meters = Rd::at(cont, 64).f64().filter(|m| *m > 0.0 && *m <= 1.0).unwrap_or(DEFAULT_UNIT);
    let unit = meters / 0.0254 * 72.0;
    let tables = Tables::read(&chunks, unit, budget, &mut warn)?;
    let mut pages = Vec::new();
    for c in chunks.iter().filter(|c| c.id == *b"page") {
        pages.push(PageRead::new(&tables, unit, budget, &mut warn).read(c)?);
    }
    Ok(Drawing { version: 0, page_size: None, pages, master: Vec::new(), guides: Vec::new(), warnings: warn })
}

/// The chunks of the file in order, those inside `LIST` chunks in place (R53).
fn chunks<'a>(b: &'a [u8], budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<Vec<Chunk<'a>>, Error> {
    let declared = u32_at(b, 4).and_then(|n| (n as usize).checked_add(8)).unwrap_or(b.len());
    if declared > b.len() {
        // The file was cut short of the size its header gives.
        push(warn, Warning::Damaged);
    }
    let end = declared.min(b.len());
    let mut out = Vec::new();
    // (position, end, depth) of the lists still to walk; a list's children go before the rest of
    // the list that holds it.
    let mut stack = vec![(12usize, end, 0usize)];
    while let Some((mut pos, end, depth)) = stack.pop() {
        while pos.saturating_add(8) <= end {
            let Some(id) = Rd::at(b, pos).fourcc() else { break };
            if !is_id(&id) {
                match resync(b, pos, end) {
                    Some(next) => {
                        pos = next;
                        continue;
                    }
                    None => {
                        push(warn, Warning::Damaged);
                        break;
                    }
                }
            }
            budget.take_chunk()?;
            let size = u32_at(b, pos + 4).map_or(0, |s| s as usize);
            let start = pos + 8;
            let stop = start.saturating_add(size);
            if stop > end {
                // Stray bytes that happen to start with four letters give a length past the list:
                // look for the next chunk id after them. A chunk the reader uses is kept, cut at
                // the end of the list.
                if !READ_IDS.contains(&id)
                    && let Some(next) = resync(b, pos, end)
                {
                    pos = next;
                    continue;
                }
                push(warn, Warning::Damaged);
            }
            let stop = stop.min(end);
            let next = stop.saturating_add(size & 1);
            if id == LIST {
                if depth >= budget.limits.depth {
                    push(warn, Warning::TooDeep);
                    pos = next;
                    continue;
                }
                stack.push((next, end, depth));
                stack.push((start + 4, stop, depth + 1));
                break;
            }
            if let Some(body) = b.get(start..stop) {
                out.push(Chunk { id, body, start });
            }
            pos = next;
        }
    }
    Ok(out)
}

/// Is `t` a chunk id: an ASCII letter, then three ASCII letters, digits or spaces (R53)?
fn is_id(t: &[u8]) -> bool {
    matches!(t, [first, rest @ ..] if t.len() == 4 && first.is_ascii_alphabetic() && rest.iter().all(|c| c.is_ascii_alphanumeric() || *c == b' '))
}

/// Where the next chunk after `pos` lies: the first chunk id at an even distance of 2 to [`RESYNC`]
/// bytes that names a chunk the reader uses, or another chunk whose length keeps it inside the list
/// that ends at `end` (R53).
fn resync(b: &[u8], pos: usize, end: usize) -> Option<usize> {
    let fits = |p: usize| u32_at(b, p + 4).and_then(|s| (p + 8).checked_add(s as usize)).is_some_and(|stop| stop <= end);
    let chunk_at = |p: usize| Rd::at(b, p).fourcc().is_some_and(|id| is_id(&id) && (READ_IDS.contains(&id) || fits(p)));
    (pos + 2..=pos.saturating_add(RESYNC).min(end.saturating_sub(8))).step_by(2).find(|&p| chunk_at(p))
}

/// The end of the field list at `pos` of `b`: after its 0xff, or the end of `b` when a field runs
/// past it (R55).
fn list_end(b: &[u8], mut pos: usize) -> usize {
    while let Some(&tag) = b.get(pos) {
        if tag == 0xff {
            return pos + 1;
        }
        match Rd::at(b, pos + 1).u16().map(usize::from).filter(|n| *n >= 3) {
            Some(n) if pos + n <= b.len() => pos += n,
            _ => break,
        }
    }
    b.len()
}

/// The data of field `tag` in the field list `b` (R55).
fn field(b: &[u8], tag: u8) -> Option<&[u8]> {
    let mut pos = 0;
    while let Some(&t) = b.get(pos) {
        if t == 0xff {
            return None;
        }
        let n = usize::from(Rd::at(b, pos + 1).u16()?);
        let data = b.get(pos + 3..pos.checked_add(n)?).filter(|_| n >= 3)?;
        if t == tag {
            return Some(data);
        }
        pos += n;
    }
    None
}

/// The field lists of a table: a u16 count, then that many lists (R55). Each entry counts toward
/// the chunk limit.
fn entries<'a>(d: &'a [u8], budget: &mut Budget) -> Result<Vec<&'a [u8]>, Error> {
    let count = Rd::new(d).u16().map_or(0, usize::from);
    let mut out = Vec::new();
    let mut pos = 2;
    for _ in 0..count {
        if pos >= d.len() {
            break;
        }
        budget.take_chunk()?;
        let end = list_end(d, pos);
        out.push(d.get(pos..end).unwrap_or_default());
        pos = end;
    }
    Ok(out)
}

impl Tables {
    /// The first chunk of each table; a table that is missing is empty.
    fn read(chunks: &[Chunk], unit: f64, budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<Tables, Error> {
        let first = |id: &[u8; 4]| chunks.iter().find(|c| c.id == *id).map_or(&[][..], |c| c.body);
        // A color record of field 10 holds a property list as a `.cdr` fill does (R56).
        let record = layout::for_version(1300);
        let mut t = Tables::default();
        for e in entries(first(b"rclr"), budget)? {
            t.colors.push(color(e, record.as_ref(), warn));
        }
        for e in entries(first(b"rpen"), budget)? {
            t.pens.push(field(e, 1).and_then(|f| Rd::new(f).u32()).map(|w| f64::from(w) * unit));
        }
        for e in entries(first(b"rott"), budget)? {
            t.line_types.push(field(e, 1).and_then(|f| Some((*f.first()?, f.get(1).copied().unwrap_or(0)))));
        }
        for e in entries(first(b"rdot"), budget)? {
            let mut r = Rd::new(field(e, 1).unwrap_or_default());
            let n = r.u16().map_or(0, usize::from);
            let mut pattern: Vec<u16> = (0..n).map_while(|_| r.u16()).collect();
            // A pattern of zero lengths draws as a solid line, as in a `.cdr` outline (R48).
            if pattern.iter().all(|x| *x == 0) {
                pattern.clear();
            }
            t.dashes.push(pattern);
        }
        for e in entries(first(b"rotl"), budget)? {
            t.outlines.push(field(e, 1).and_then(|f| {
                let mut r = Rd::new(f);
                Some([r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?])
            }));
        }
        // Arrowhead pairs are plain u16 pairs after the count, not field lists (R59).
        let mut r = Rd::new(first(b"rota"));
        let n = r.u16().map_or(0, usize::from);
        for _ in 0..n {
            let Some(pair) = r.u16().zip(r.u16()) else { break };
            budget.take_chunk()?;
            t.arrowheads.push(pair);
        }
        Ok(t)
    }

    /// Entry `index` (1-based) of `list`, or `None` with a warning when there is none.
    fn get<T: Clone>(list: &[Option<T>], index: u16, warn: &mut Vec<Warning>) -> Option<T> {
        let found = usize::from(index).checked_sub(1).and_then(|i| list.get(i)).cloned().flatten();
        if found.is_none() {
            push(warn, Warning::Damaged);
        }
        found
    }

    fn color(&self, index: u16, warn: &mut Vec<Warning>) -> Color {
        Tables::get(&self.colors, index, warn).unwrap_or(Color::Rgb([0.0; 3]))
    }

    /// The outline of `rotl` entry `index`, or `None` when its line type draws none (R57, R58).
    fn outline(&self, index: u16, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Outline> {
        let [line, _screen, color, arrows, pen, dash] = Tables::get(&self.outlines, index, warn)?;
        let (flags, caps_join) = Tables::get(&self.line_types, line, warn).unwrap_or((2, 0));
        if flags & 1 != 0 {
            return None;
        }
        let width = Tables::get(&self.pens, pen, warn).unwrap_or(0.0);
        let mut dashes = Vec::new();
        if flags & 4 != 0 {
            let stored = usize::from(dash).checked_sub(1).and_then(|i| self.dashes.get(i));
            match stored {
                Some(d) if d.is_empty() => {}
                Some(d) if budget.take_dashes(d.len()) => dashes = d.iter().map(|x| f64::from(*x)).collect(),
                Some(_) => push(warn, Warning::TooDeep),
                None => push(warn, Warning::Damaged),
            }
        }
        if usize::from(arrows).checked_sub(1).and_then(|i| self.arrowheads.get(i)).is_some_and(|(s, e)| *s != 0 || *e != 0) {
            push(warn, Warning::Arrowheads);
        }
        Some(Outline {
            width,
            color: self.color(color, warn),
            cap: match caps_join & 0x0f {
                1 => Cap::Round,
                2 => Cap::Square,
                _ => Cap::Butt,
            },
            join: match caps_join >> 4 {
                1 => Join::Round,
                2 => Join::Bevel,
                _ => Join::Miter,
            },
            dashes,
            behind_fill: flags & 8 != 0,
            scale_with: flags & 16 != 0,
        })
    }
}

/// A color entry (R56): the property list of field 10 when there is one, else the model and
/// palette bytes of field 1 and the values of field 2. Colors in other models read as black with a
/// warning.
fn color(e: &[u8], record: Option<&Profile>, warn: &mut Vec<Warning>) -> Option<Color> {
    if let (Some(f), Some(p)) = (field(e, 10), record) {
        let mut r = Rd::new(f);
        let len = r.u32()? as usize;
        return property_color(r.bytes(len.min(r.left()))?, p, warn);
    }
    let model = *field(e, 1)?.first()?;
    let v = field(e, 2)?;
    let byte = |i: usize| v.get(i).map(|b| f32::from(*b));
    let pct = |i: usize| byte(i).map(|x| (x / 100.0).clamp(0.0, 1.0));
    let full = |i: usize| byte(i).map(|x| x / 255.0);
    Some(match model {
        2 => Color::Cmyk([pct(0)?, pct(1)?, pct(2)?, pct(3)?]),
        3 => Color::Cmyk([full(0)?, full(1)?, full(2)?, full(3)?]),
        4 => Color::Cmyk([full(0)?, full(1)?, full(2)?, 0.0]),
        5 => Color::Rgb([full(0)?, full(1)?, full(2)?]),
        9 => Color::Gray(1.0 - full(0)?),
        _ => {
            push(warn, Warning::UnknownColors);
            Color::Rgb([0.0; 3])
        }
    })
}

/// The objects of one layer while it is read: the layer's own list, then one list for each group
/// left open.
struct LayerRead {
    name: String,
    lists: Vec<Vec<Object>>,
}

impl LayerRead {
    /// The layer, its open groups closed. Lists are drawn back to front (R60), and the model holds
    /// them front to back.
    fn finish(mut self) -> Layer {
        while self.lists.len() > 1 {
            close_group(&mut self.lists);
        }
        let mut objects = self.lists.pop().unwrap_or_default();
        objects.reverse();
        Layer { name: self.name, kind: LayerKind::Drawing, flags: Default::default(), objects, guides: Vec::new() }
    }
}

/// Close the innermost open group of `lists` into the one that holds it.
fn close_group(lists: &mut Vec<Vec<Object>>) {
    if lists.len() < 2 {
        return;
    }
    let mut objects = lists.pop().unwrap_or_default();
    objects.reverse();
    if let Some(top) = lists.last_mut() {
        top.push(Object::Group { objects, flags: Default::default() });
    }
}

/// The reading of one page chunk.
struct PageRead<'t, 'b> {
    tables: &'t Tables,
    unit: f64,
    budget: &'b mut Budget,
    warn: &'b mut Vec<Warning>,
    size: Option<(f64, f64)>,
    /// The page's center in the file's coordinates when the page takes its size from the
    /// drawing's bounds; points are moved by it so the art lies on the page.
    center: (f64, f64),
    layers: Vec<Layer>,
    open: Option<LayerRead>,
    /// Groups nested past the depth limit that are still open; their content is left out.
    skipped: usize,
}

impl<'t, 'b> PageRead<'t, 'b> {
    fn new(tables: &'t Tables, unit: f64, budget: &'b mut Budget, warn: &'b mut Vec<Warning>) -> Self {
        PageRead { tables, unit, budget, warn, size: None, center: (0.0, 0.0), layers: Vec::new(), open: None, skipped: 0 }
    }

    /// The instructions of a page chunk: a u16 size counting the whole instruction and an i16 code,
    /// or 0xFFFF, a u32 size and the code; then the code's field lists (R60).
    fn read(mut self, c: &Chunk) -> Result<Page, Error> {
        let d = c.body;
        let mut pos = 0usize;
        while pos.saturating_add(4) <= d.len() {
            self.budget.take_chunk()?;
            let mut r = Rd::at(d, pos);
            let (size, code, head) = match r.u16() {
                Some(0xFFFF) => match (r.u32(), r.i16()) {
                    (Some(size), Some(code)) => (size as usize, code, 8),
                    _ => break,
                },
                Some(size) => match r.i16() {
                    Some(code) => (usize::from(size), code, 4),
                    None => break,
                },
                None => break,
            };
            if size < head {
                push(self.warn, Warning::Damaged);
                break;
            }
            let end = pos.saturating_add(size);
            if end > d.len() {
                push(self.warn, Warning::Damaged);
            }
            let body = d.get(pos + head..end.min(d.len())).unwrap_or_default();
            if code == JUMP {
                // Field 1 is the file offset the instructions go on from; the bytes passed over
                // hold the page's images (R60).
                let target = field(body, 1).and_then(|f| Rd::new(f).u32()).and_then(|t| (t as usize).checked_sub(c.start));
                pos = target.filter(|t| *t >= end && *t <= d.len()).unwrap_or(end);
                if pos > end {
                    push(self.warn, Warning::UnsupportedObjects);
                }
                continue;
            }
            self.instruction(code, body);
            pos = end;
        }
        if pos < d.len() {
            // An instruction whose head runs past the page was not read.
            push(self.warn, Warning::Damaged);
        }
        self.close_layer();
        // A layer that holds nothing (the special layers every sample file lists empty) is left
        // out. The file lists layers back to front.
        let mut layers = self.layers;
        layers.retain(|l| !l.objects.is_empty());
        layers.reverse();
        Ok(Page { size: self.size, layers })
    }

    fn instruction(&mut self, code: i16, body: &[u8]) {
        match code {
            BEGIN_PAGE => {
                // Field 4: the page's width and height (R61).
                let mut r = Rd::new(field(body, 4).unwrap_or_default());
                if let (Some(w), Some(h)) = (r.u32(), r.u32()) {
                    let (w, h) = (f64::from(w) * self.unit, f64::from(h) * self.unit);
                    self.size = (w > 0.0 && h > 0.0).then_some((w, h));
                }
                if self.size.is_none() {
                    self.size_from_bounds(field(body, 1).unwrap_or_default());
                }
            }
            BEGIN_LAYER => {
                self.close_layer();
                // A layer without a name is left unnamed, as in a `.cdr` drawing.
                let name = self.layer_name(body).unwrap_or_default();
                self.open = Some(LayerRead { name, lists: vec![Vec::new()] });
            }
            END_LAYER => self.close_layer(),
            BEGIN_GROUP => {
                let open = self.layer().lists.len();
                if self.skipped > 0 || open > self.budget.limits.depth {
                    self.skipped += 1;
                    push(self.warn, Warning::TooDeep);
                } else if self.budget.take_object() {
                    self.layer().lists.push(Vec::new());
                } else {
                    // Past the object limit the group's content joins the group that holds it.
                    self.skipped += 1;
                    push(self.warn, Warning::TooDeep);
                }
            }
            END_GROUP => {
                if self.skipped > 0 {
                    self.skipped -= 1;
                } else {
                    close_group(&mut self.layer().lists);
                }
            }
            CURVE if self.skipped == 0 => {
                if let Some(shape) = self.curve(body) {
                    if self.budget.take_object() {
                        if let Some(list) = self.layer().lists.last_mut() {
                            list.push(Object::Shape(Box::new(shape)));
                        }
                    } else {
                        push(self.warn, Warning::TooDeep);
                    }
                }
            }
            BEGIN_TEXT | GLYPH_RUN | ELLIPSE | RECTANGLE | IMAGE | ADD_CLIP => push(self.warn, Warning::UnsupportedObjects),
            // These and the other codes (text styles and characters, color contexts) draw nothing
            // on their own.
            END_PAGE | END_TEXT | REMOVE_CLIP => {}
            _ => {}
        }
    }

    /// A page without a size of its own is sized to the drawing's bounds in field 1 (R61): a u16
    /// page number, a u32 of flags, then the left, top, right and bottom edges as i32 in
    /// coordinate units. The page is centered on those bounds.
    fn size_from_bounds(&mut self, f: &[u8]) {
        let mut r = Rd::new(f);
        if r.u16().is_none() || r.u32().is_none() {
            return;
        }
        let (Some(l), Some(t), Some(rt), Some(b)) = (r.i32(), r.i32(), r.i32(), r.i32()) else {
            return;
        };
        let (l, t, rt, b) = (f64::from(l) * self.unit, f64::from(t) * self.unit, f64::from(rt) * self.unit, f64::from(b) * self.unit);
        let (w, h) = ((rt - l).abs(), (t - b).abs());
        if w > 0.0 && h > 0.0 {
            self.size = Some((w, h));
            self.center = ((l + rt) / 2.0, (t + b) / 2.0);
        }
    }

    /// The open layer, or a new unnamed one when objects come before any layer begins.
    fn layer(&mut self) -> &mut LayerRead {
        self.open.get_or_insert_with(|| LayerRead { name: String::new(), lists: vec![Vec::new()] })
    }

    fn close_layer(&mut self) {
        self.skipped = 0;
        if let Some(l) = self.open.take() {
            self.layers.push(l.finish());
        }
    }

    /// A layer's name: field 4 as a u32 count and UTF-16 units, else field 1 from byte 12 as a
    /// u16 count and 8-bit characters (R62).
    fn layer_name(&mut self, body: &[u8]) -> Option<String> {
        // A field 4 that can't be read leaves the name to field 1.
        let wide = field(body, 4).and_then(|f| {
            let mut r = Rd::new(f);
            let n = (r.u32()? as usize).min(MAX_NAME_UNITS).min(r.left() / 2);
            Some((r.bytes(n * 2)?, layout::for_version(1200)?))
        });
        if let Some((units, l)) = wide {
            let s = name(units, &l, self.warn);
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
        let mut r = Rd::at(field(body, 1)?, 12);
        let n = usize::from(r.u16()?).min(r.left());
        let s = name(r.bytes(n)?, &layout::for_version(1100)?, self.warn);
        (!s.trim().is_empty()).then_some(s)
    }

    /// A curve (R62): field 1 its rendering attributes, field 2 a u16 point count, the points (i32
    /// x and y) and one type byte for each. Points are from the page's center, y up.
    fn curve(&mut self, body: &[u8]) -> Option<Shape> {
        let mut r = Rd::new(field(body, 2).unwrap_or_default());
        let Some(count) = r.u16().map(usize::from) else {
            // A curve without its points is lost.
            push(self.warn, Warning::Damaged);
            return None;
        };
        // `stored` points are read, so the type bytes are found after all of them, and the first
        // `n`, those that have a type byte, are kept. When the field holds every point, a short
        // field has lost type bytes. When it holds whole points and their types for fewer points,
        // the count is too large. Otherwise the field lost points, and its type bytes can't be
        // found: the curve is lost.
        let left = r.left();
        let (stored, n) = match count.checked_mul(8).filter(|c| *c <= left) {
            Some(c) => (count, count.min(left - c)),
            None if left.is_multiple_of(9) => (left / 9, left / 9),
            None => {
                push(self.warn, Warning::Damaged);
                return None;
            }
        };
        if n < count {
            push(self.warn, Warning::Damaged);
        }
        if !self.budget.take_coordinates(stored) {
            push(self.warn, Warning::TooDeep);
            return None;
        }
        let mut points = Vec::with_capacity(stored);
        for _ in 0..stored {
            let (x, y) = (f64::from(r.i32()?) * self.unit, f64::from(r.i32()?) * self.unit);
            points.push(Pt { x: x - self.center.0, y: y - self.center.1 });
        }
        points.truncate(n);
        let path = path_of(&points, r.bytes(n)?);
        if path.subpaths.is_empty() {
            return None;
        }
        let (fill, outline) = self.attributes(field(body, 1).unwrap_or_default());
        Some(Shape::new(Geometry::Path(path), Matrix::IDENTITY, fill, outline))
    }

    /// Rendering attributes (R63): a mask (1 fill, 2 outline, 4 lens), then a field list for the
    /// fill and one for the outline when the mask names them.
    fn attributes(&mut self, a: &[u8]) -> (Option<Color>, Option<Outline>) {
        let Some(&mask) = a.first() else { return (None, None) };
        let mut pos = 1;
        let mut fill = None;
        if mask & 1 != 0 {
            let end = list_end(a, pos);
            let list = a.get(pos..end).unwrap_or_default();
            pos = end;
            // Field 1: the fill type (u16), then its own field list; type 1, uniform, names its
            // color in field 1.
            let v = field(list, 1).unwrap_or_default();
            match Rd::new(v).u16() {
                Some(0) | None => {}
                Some(1) => {
                    let index = field(v.get(2..).unwrap_or_default(), 1).and_then(|c| Rd::new(c).u16());
                    fill = Some(match index {
                        Some(i) => self.tables.color(i, self.warn),
                        None => {
                            push(self.warn, Warning::Damaged);
                            Color::Rgb([0.0; 3])
                        }
                    });
                }
                Some(_) => push(self.warn, Warning::UnsupportedFills),
            }
        }
        let mut outline = None;
        if mask & 2 != 0 {
            let list = a.get(pos..list_end(a, pos)).unwrap_or_default();
            match field(list, 1).and_then(|f| Rd::new(f).u16()) {
                Some(i) => outline = self.tables.outline(i, self.budget, self.warn),
                None => push(self.warn, Warning::Damaged),
            }
        }
        if mask & 4 != 0 {
            push(self.warn, Warning::Lenses);
        }
        // A canvas (8) or a container (16) is not read.
        if mask & (8 | 16) != 0 {
            push(self.warn, Warning::UnsupportedObjects);
        }
        (fill, outline)
    }
}
