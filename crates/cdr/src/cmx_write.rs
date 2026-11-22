//! Writer for `.cmx` exchange files with 32-bit coordinates, in the layouts [`crate::read_cmx`]
//! reads: the RIFF form `CMX1` (R53), its header (R54), the color, pen, line type, dash, outline and
//! arrowhead tables (R55 to R59), the screen table (R74) and one `page` chunk of instructions for
//! each page (R60): pages, layers, groups and curves (R61, R62) with uniform fills and outlines
//! (R63). `PROVENANCE.md`'s "Written fields" lists every field the writer sets and where its layout
//! comes from.
//!
//! Rectangles and ellipses are written as curves. A clipping container is written as a group of
//! its fill, the objects it clips, unclipped, and its outline. Hidden layers, groups and shapes,
//! guide and grid layers, guidelines and master layers are left out. Opacity, blend modes, fill
//! rules (a curve fills even-odd) and locked states are not written, and [`Written`] reports each
//! of these that the drawing held. The drawing's version is not written either; the file opens
//! with version 0.

use std::collections::HashMap;
use std::hash::Hash;

use crate::Error;
use crate::model::{Blend, Cap, Color, Drawing, Geometry, Join, Layer, LayerKind, Matrix, Object, Outline, Path, Pt, Segment, Shape, SubPath};

/// The size of a coordinate unit written: 0.1 µm, the unit of every sample file with 32-bit
/// coordinates (R54).
const UNIT_METERS: f64 = 1e-7;
/// Coordinate units per point.
const UNITS_PER_PT: f64 = 0.0254 / 72.0 / UNIT_METERS;
/// The most points one curve holds: its points field is at most 65,535 bytes long with its 3-byte
/// header, and holds a u16 count and 9 bytes for each point (R55, R62).
pub const MAX_CURVE_POINTS: usize = (u16::MAX as usize - 3 - 2) / 9;
/// The most UTF-16 units of a layer name or a color name written, as many as the reader takes in
/// (it keeps the first [`crate::records::MAX_NAME`] and a hash of the rest).
const MAX_NAME_UNITS: usize = 4096;
/// The most 8-bit characters of a layer name written, as many as the reader keeps.
const MAX_NAME_BYTES: usize = crate::records::MAX_NAME;
/// The most entries one table holds: its count is a u16 (R55), and objects name entries by a u16
/// from 1.
const MAX_ENTRIES: usize = u16::MAX as usize;
/// The screen entry's field 1, as the sample files with 32-bit coordinates write it (R74).
const SCREEN: [u8; 13] = [0x00, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0xa5, 0xae, 0x02, 0x00];

// Instruction codes (R60).
const BEGIN_PAGE: i16 = 9;
const END_PAGE: i16 = 10;
const BEGIN_LAYER: i16 = 11;
const END_LAYER: i16 = 12;
const BEGIN_GROUP: i16 = 13;
const END_GROUP: i16 = 14;
const CURVE: i16 = 67;

// Point type bytes (R16): bits 7 and 6 give the operation, bit 3 closes the subpath. Bit 2, which
// the reader does not read, is set on every node and left clear on control points, as the test
// files of `synth_cmx` set it.
const MOVE: u8 = 0x04;
const LINE: u8 = 0x44;
const CURVE_END: u8 = 0x84;
const CONTROL: u8 = 0xC0;
const CLOSES: u8 = 0x08;

/// A written file and what it changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Written {
    /// The file.
    pub bytes: Vec<u8>,
    /// A path held more than [`MAX_CURVE_POINTS`] points and was written as several curves in a
    /// group, so overlapping parts of its fill may draw differently.
    pub split_paths: bool,
    /// A filled or outlined shape had a subpath of more than [`MAX_CURVE_POINTS`] points, which was
    /// cut into open pieces, so its fill draws differently and its outline gets caps, and starts
    /// its dashes again, where the pieces meet.
    pub cut_subpaths: bool,
    /// A dash or gap length was rounded to a whole number of outline widths, the unit the dash
    /// table stores (R57), held to the 65,535 widths a length there holds, or, when it is not a
    /// number, stored as 0.
    pub rounded_dashes: bool,
    /// A layer or spot color name is longer than the 256 UTF-16 units the reader keeps whole, or
    /// ends in NUL characters, which the reader drops, so it opens shortened. Past 4,096 units it
    /// is also cut when written. Two spot colors whose names differ only in what is cut or dropped
    /// open as one. A layer name that holds a NUL opens cut there, and one of only white space
    /// opens with no name.
    pub long_names: bool,
    /// A spot color's tint is too low for its ink's look to open again: the record stores the color
    /// with its tint applied in 8-bit values (R14), so at 0 % it is white and at a low tint the
    /// look opens changed.
    pub zero_tints: bool,
    /// A shape filled with the nonzero rule may fill differently with the even-odd rule every
    /// curve is written with: one of its cubics loops, one of its subpaths crosses or touches
    /// itself or loops over itself, or two turn the same way and their boxes overlap, measured
    /// with each cubic flattened to 16 lines, with two lines that are not neighbors taken to cross
    /// when they are closer than the cubics may stray from them, and with two neighbors taken to
    /// cross when the curves along them may meet away from their shared point. Past 50,000,000 comparisons of edges and boxes in one
    /// file, a shape is taken to fill differently.
    pub nonzero_fills: bool,
    /// A spot color has no name, so it opens as a process color, its look at its tint: the reader
    /// takes a color without a name as a process color.
    pub unnamed_spots: bool,
    /// A spot color's look is unknown in every use of its name (or, for a spot color with no name,
    /// in that use), so it is written, and opens, as black at its tint.
    pub unknown_looks: bool,
    /// A spot color name was used with different looks, and a use was written with the look of
    /// the use at the strongest tint among the uses whose look is known, so that it opens more than 2/255 off what its own look would
    /// open as, or off its own look where its own look would not.
    pub merged_looks: bool,
    /// The drawing held what the file does not hold: hidden layers, groups or shapes, guide and
    /// grid layers, guidelines and master layers (left out), opacity below 1 or a blend mode
    /// (written opaque and normal), clipping (written unclipped), locked layers, groups or shapes
    /// (written unlocked), art on a desktop layer (written on a drawing layer), shapes whose paths
    /// have no segments (left out) or a layer with nothing on it (written, and left out by the
    /// reader).
    pub left_out: bool,
}

/// Write `drawing` as a `.cmx` file that a read within `limits` takes all of, as
/// [`crate::write_cmx`] describes. Coordinates are rounded to 0.1 µm. A page without its own size takes the drawing's;
/// with neither, it opens with no size. An all-zero dash pattern is solid and stores no lengths.
pub fn write(drawing: &Drawing, limits: crate::Limits) -> Result<Written, Error> {
    let mut w = Writer {
        limits,
        // The RIFF header, the form type and the `cont` chunk.
        file_bytes: 12 + 8 + 180,
        looks: spot_looks(drawing),
        crossing_budget: CROSSING_BUDGET,
        left_out: !drawing.master.is_empty() || !drawing.guides.is_empty(),
        ..Writer::default()
    };
    // The RIFF chunk's size, counted as each child is made: the form type, then the children, each a
    // whole chunk with its pad byte.
    let mut size = 4u32;
    let mut children = Vec::new();
    let mut add = |c: Vec<u8>| -> Result<(), Error> {
        size = u32::try_from(c.len()).ok().and_then(|n| size.checked_add(n)).ok_or(Error::Limit("chunk size"))?;
        children.push(c);
        Ok(())
    };
    add(chunk(b"cont", &cont())?)?;
    for (n, page) in drawing.pages.iter().enumerate() {
        add(w.page(n, page.size.or(drawing.page_size), &page.layers)?)?;
    }
    // A drawing with no pages but a page size is written as one empty page of that size.
    if drawing.pages.is_empty() && drawing.page_size.is_some() {
        add(w.page(0, drawing.page_size, &[])?)?;
    }
    // The reader also counts every chunk (`cont`, the pages and the seven tables), every table entry
    // and the one arrowhead pair.
    let pages = if drawing.pages.is_empty() { usize::from(drawing.page_size.is_some()) } else { drawing.pages.len() };
    let t = &w.tables;
    let entries = t.colors.entries.len() + t.pens.entries.len() + t.line_types.entries.len() + t.dashes.entries.len() + t.outlines.entries.len() + 1;
    w.read_cost.chunks = w.read_cost.chunks.saturating_add(1 + pages + 7 + entries);
    w.read_cost.check(&w.limits)?;
    for c in w.tables.chunks(w.limits.input.saturating_sub(w.file_bytes))? {
        add(c)?;
    }
    let total = usize::try_from(size).ok().and_then(|n| n.checked_add(8)).ok_or(Error::Limit("chunk size"))?;
    if total > w.limits.input {
        return Err(Error::Limit("input size"));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(total).map_err(|_| Error::Limit("file size"))?;
    bytes.extend(b"RIFF");
    bytes.extend(size.to_le_bytes());
    bytes.extend(b"CMX1");
    for c in children {
        bytes.extend(c);
    }
    Ok(Written {
        bytes,
        split_paths: w.split_paths,
        cut_subpaths: w.cut_subpaths,
        rounded_dashes: w.rounded_dashes,
        long_names: w.long_names,
        zero_tints: w.zero_tints,
        nonzero_fills: w.nonzero_fills,
        unnamed_spots: w.unnamed_spots,
        unknown_looks: w.unknown_looks,
        merged_looks: w.merged_looks,
        left_out: w.left_out,
    })
}

/// A chunk: its id, a u32 size, the body and a pad byte after an odd body (R53). A body past
/// what the size holds is an [`Error::Limit`].
fn chunk(id: &[u8; 4], body: &[u8]) -> Result<Vec<u8>, Error> {
    let size = u32::try_from(body.len()).map_err(|_| Error::Limit("chunk size"))?;
    let mut out = Vec::with_capacity(body.len() + 9);
    out.extend(id);
    out.extend(size.to_le_bytes());
    out.extend(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    Ok(out)
}

/// The header (R54): the coordinate size digit `4` at byte 52 and the unit in meters at bytes 64
/// to 72. Bytes 48 and 54 hold the digit `2`, as in the sample files; the rest are zeros.
fn cont() -> Vec<u8> {
    let mut c = vec![0u8; 180];
    c[48] = b'2';
    c[52] = b'4';
    c[54] = b'2';
    c[64..72].copy_from_slice(&UNIT_METERS.to_le_bytes());
    c
}

/// A field: its tag, a u16 length counting the 3 header bytes, then `data` (R55). `None` when the
/// data does not fit the length.
fn field(tag: u8, data: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(data.len() + 3).ok()?;
    let mut f = Vec::with_capacity(data.len() + 3);
    f.push(tag);
    f.extend(len.to_le_bytes());
    f.extend(data);
    Some(f)
}

/// A field list: the fields, then 0xff (R55).
fn fields(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut l = fields.concat();
    l.push(0xff);
    l
}

/// An instruction: the short form when its size fits a u16 below 0xFFFF, else the long form
/// (R60).
fn instr(code: i16, body: &[u8]) -> Vec<u8> {
    let mut i = Vec::with_capacity(body.len() + 8);
    match u16::try_from(body.len() + 4).ok().filter(|n| *n != u16::MAX) {
        Some(n) => i.extend(n.to_le_bytes()),
        None => {
            i.extend(u16::MAX.to_le_bytes());
            i.extend(((body.len() + 8) as u32).to_le_bytes());
        }
    }
    i.extend(code.to_le_bytes());
    i.extend(body);
    i
}

/// A length in points as coordinate units.
fn units(v: f64) -> Result<i32, Error> {
    let u = (v * UNITS_PER_PT).round();
    if u.is_finite() && u >= f64::from(i32::MIN) && u <= f64::from(i32::MAX) { Ok(u as i32) } else { Err(Error::Limit("coordinate range")) }
}

/// A length that can't be negative (a width) in coordinate units.
fn width_units(v: f64) -> Result<u32, Error> {
    let u = (v.max(0.0) * UNITS_PER_PT).round();
    if u.is_finite() && u <= f64::from(u32::MAX) { Ok(u as u32) } else { Err(Error::Limit("coordinate range")) }
}

/// The distinct entries of one table, in the order they were first used, and the 1-based index of
/// each.
struct Table<K> {
    entries: Vec<K>,
    index: HashMap<K, u16>,
}

impl<K> Default for Table<K> {
    fn default() -> Self {
        Table { entries: Vec::new(), index: HashMap::new() }
    }
}

impl<K: Clone + Eq + Hash> Table<K> {
    /// The 1-based index of `k`, added when it is new.
    fn index(&mut self, k: K) -> Result<u16, Error> {
        if let Some(i) = self.index.get(&k) {
            return Ok(*i);
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::Limit("table entries"));
        }
        let i = u16::try_from(self.entries.len() + 1).map_err(|_| Error::Limit("table entries"))?;
        self.entries.push(k.clone());
        self.index.insert(k, i);
        Ok(i)
    }
}

/// A color as it is stored, so equal colors share an entry.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ColorKey {
    model: u16,
    values: [u8; 4],
    name: Option<String>,
}

/// The tables the instructions refer to (R55 to R59).
#[derive(Default)]
struct Tables {
    colors: Table<ColorKey>,
    /// Widths in coordinate units.
    pens: Table<u32>,
    /// Flags, and the caps and join.
    line_types: Table<(u8, u8)>,
    dashes: Table<Vec<u16>>,
    /// Line type, color, pen and dashes (the screen and the arrowheads are always the first).
    outlines: Table<[u16; 4]>,
}

impl Tables {
    /// The table chunks, or an [`Error::Limit`] once they take more than `left` bytes.
    fn chunks(&self, left: usize) -> Result<Vec<Vec<u8>>, Error> {
        // The bytes taken so far, charged as each entry is made and then for each table's header.
        let used = std::cell::Cell::new(0usize);
        let charge = |n: usize| {
            used.set(used.get().saturating_add(n));
            if used.get() > left { Err(Error::Limit("input size")) } else { Ok(()) }
        };
        let table = |id: &[u8; 4], entries: Vec<Vec<u8>>| -> Result<Vec<u8>, Error> {
            let mut d = (entries.len() as u16).to_le_bytes().to_vec();
            d.extend(entries.concat());
            let c = chunk(id, &d)?;
            charge(c.len() + 2 - d.len())?;
            Ok(c)
        };
        let one = |tag: u8, data: &[u8]| -> Result<Vec<u8>, Error> {
            let e = field(tag, data).map(|f| fields(&[f])).ok_or(Error::Limit("table entry size"))?;
            charge(e.len())?;
            Ok(e)
        };
        let colors = self
            .colors
            .entries
            .iter()
            .map(|k| -> Result<Vec<u8>, Error> {
                let e = color_entry(k)?;
                charge(e.len())?;
                Ok(e)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A pen: the width, an aspect of 100 percent, an angle of 0 and a matrix of type 1
        // (R57).
        let pens = self
            .pens
            .entries
            .iter()
            .map(|w| {
                let mut d = w.to_le_bytes().to_vec();
                d.extend(100u16.to_le_bytes());
                d.extend(0i32.to_le_bytes());
                d.extend(1u16.to_le_bytes());
                one(1, &d)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let line_types = self.line_types.entries.iter().map(|(f, cj)| one(1, &[*f, *cj])).collect::<Result<Vec<_>, _>>()?;
        let dashes = self
            .dashes
            .entries
            .iter()
            .map(|d| {
                let mut b = (d.len() as u16).to_le_bytes().to_vec();
                b.extend(d.iter().flat_map(|x| x.to_le_bytes()));
                one(1, &b)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // An outline: line type, screen, color, arrowhead pair, pen and dash pattern (R58).
        let outlines = self
            .outlines
            .entries
            .iter()
            .map(|[line, color, pen, dash]| one(1, &[*line, 1, *color, 1, *pen, *dash].map(u16::to_le_bytes).concat()))
            .collect::<Result<Vec<_>, _>>()?;
        // One arrowhead pair with no arrowheads, which every outline names (R59).
        let mut rota = 1u16.to_le_bytes().to_vec();
        rota.extend([0; 4]);
        Ok(vec![
            table(b"rclr", colors)?,
            table(b"rpen", pens)?,
            table(b"rott", line_types)?,
            table(b"rdot", dashes)?,
            table(b"rotl", outlines)?,
            chunk(b"rota", &rota)?,
            // One screen, which every outline names (R74).
            table(b"rscr", vec![one(1, &SCREEN)?])?,
        ])
    }
}

/// A color as a color record (R14): the model and four value bytes. A spot color is model 21, its
/// look at its tint in blue, green and red with the tint in percent, and its name; with no look it
/// is black at its tint.
fn color_key(c: &Color) -> ColorKey {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let pct = |v: f32| (v.clamp(0.0, 1.0) * 100.0).round() as u8;
    match c {
        Color::Rgb([r, g, b]) => ColorKey { model: 5, values: [byte(*b), byte(*g), byte(*r), 0], name: None },
        Color::Cmyk([c, m, y, k]) => ColorKey { model: 2, values: [pct(*c), pct(*m), pct(*y), pct(*k)], name: None },
        Color::Gray(k) => ColorKey { model: 9, values: [byte(1.0 - k), 0, 0, 0], name: None },
        Color::Registration => ColorKey { model: 20, values: [0; 4], name: None },
        Color::Spot { name, tint, look } => {
            // The look is mixed at the whole percent stored, the tint the reader takes it back
            // out at.
            let stored = pct(*tint);
            let t = f32::from(stored) / 100.0;
            let [r, g, b] = look.unwrap_or([0.0; 3]).map(|v| 1.0 - (1.0 - v.clamp(0.0, 1.0)) * t);
            ColorKey { model: 21, values: [byte(b), byte(g), byte(r), stored], name: Some(name.clone()) }
        }
    }
}

/// A color entry (R56): field 10 holds a property list, a u32 length then parts of a u8 kind, a u32
/// length and the body (R11): part 1 the color record (model, palette 5, 4 zero bytes, the
/// values; R14), and for a named color part 3, a u32 count and UTF-16 units.
fn color_entry(k: &ColorKey) -> Result<Vec<u8>, Error> {
    let mut list = vec![1u8];
    list.extend(12u32.to_le_bytes());
    list.extend(k.model.to_le_bytes());
    list.extend(5u16.to_le_bytes());
    list.extend([0; 4]);
    list.extend(k.values);
    if let Some(name) = &k.name {
        let units = utf16_prefix(name);
        list.push(3);
        list.extend((4 + 2 * units.len() as u32).to_le_bytes());
        list.extend((units.len() as u32).to_le_bytes());
        list.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    }
    let mut rec = (list.len() as u32).to_le_bytes().to_vec();
    rec.extend(list);
    Ok(fields(&[field(10, &rec).ok_or(Error::Limit("table entry size"))?]))
}

/// The counts [`crate::read_cmx`] holds to [`crate::Limits`] while it reads a file.
#[derive(Default)]
struct ReadCost {
    chunks: usize,
    objects: usize,
    coordinates: usize,
    dashes: usize,
}

impl ReadCost {
    /// An [`Error::Limit`] when a read within `limits` would refuse the file or leave out part of
    /// it.
    fn check(&self, limits: &crate::Limits) -> Result<(), Error> {
        if self.chunks > limits.chunks {
            return Err(Error::Limit("chunk count"));
        }
        if self.objects > limits.objects {
            return Err(Error::Limit("objects"));
        }
        if self.coordinates > limits.coordinates {
            return Err(Error::Limit("coordinates"));
        }
        if self.dashes > limits.dashes {
            return Err(Error::Limit("dashes"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct Writer {
    /// What the file must open within.
    limits: crate::Limits,
    /// The bytes of the chunks written so far, with the RIFF header.
    file_bytes: usize,
    /// At least the bytes the table entries added so far take in the tables written at the end:
    /// a dash pattern's lengths and a color's name.
    table_bytes: usize,
    tables: Tables,
    /// The look of each named spot color, from [`spot_looks`].
    looks: HashMap<String, [f32; 3]>,
    /// The comparisons [`may_fill_differently`] may still make.
    crossing_budget: u64,
    /// The groups open where the next instruction is written.
    open_groups: usize,
    /// The smallest and largest x and y the curves written on the current page reach, in
    /// coordinate units.
    bounds: Option<[i32; 4]>,
    /// What [`crate::read_cmx`] counts against [`crate::Limits`] in the instructions written so
    /// far: instructions (chunks), groups and curves (objects), curve points (coordinates) and the
    /// dash lengths of every curve's outline (dashes).
    read_cost: ReadCost,
    split_paths: bool,
    cut_subpaths: bool,
    rounded_dashes: bool,
    long_names: bool,
    zero_tints: bool,
    nonzero_fills: bool,
    unnamed_spots: bool,
    unknown_looks: bool,
    merged_looks: bool,
    left_out: bool,
}

/// Whether a spot color's `name`, as written, reads back as no name: the reader drops trailing NULs.
fn unnamed(name: &str) -> bool {
    utf16_prefix(name).iter().all(|u| *u == 0)
}

/// Whether `name` opens shortened: it is longer than the UTF-16 units the reader keeps whole, or
/// it ends in NULs, which the reader drops.
fn shortened(name: &str) -> bool {
    name.ends_with('\0') || name.encode_utf16().nth(crate::records::MAX_NAME).is_some()
}

/// `name` in UTF-16, cut after its last whole character within [`MAX_NAME_UNITS`] units.
fn utf16_prefix(name: &str) -> Vec<u16> {
    let mut units = Vec::new();
    let mut buf = [0u16; 2];
    for c in name.chars() {
        let enc = c.encode_utf16(&mut buf);
        if units.len() + enc.len() > MAX_NAME_UNITS {
            break;
        }
        units.extend_from_slice(enc);
    }
    units
}

/// The look of each named spot color in `drawing`, master layers included, from the use at the
/// strongest tint that gives one.
fn spot_looks(drawing: &Drawing) -> HashMap<String, [f32; 3]> {
    let mut looks: HashMap<String, (f32, [f32; 3])> = HashMap::new();
    let layers = drawing.pages.iter().flat_map(|p| &p.layers).chain(&drawing.master);
    let mut stack: Vec<&Object> = layers.flat_map(|l| &l.objects).collect();
    while let Some(o) = stack.pop() {
        let shape = match o {
            Object::Shape(s) => s,
            Object::Group { objects, .. } => {
                stack.extend(objects);
                continue;
            }
            Object::Clip { container, objects } => {
                stack.extend(objects);
                container
            }
        };
        for c in shape.fill.iter().chain(shape.outline.as_ref().map(|o| &o.color)) {
            if let Color::Spot { name, tint, look: Some(look) } = c
                && !unnamed(name)
            {
                let known = looks.entry(name.clone()).or_insert((*tint, *look));
                if *tint > known.0 {
                    *known = (*tint, *look);
                }
            }
        }
    }
    looks.into_iter().map(|(name, (_, look))| (name, look)).collect()
}

impl Writer {
    /// The 1-based color entry of `c`. A named spot color takes its ink's look: the look of the
    /// use of its name at the strongest tint among the uses whose look is known, hidden uses
    /// included.
    fn color(&mut self, c: &Color) -> Result<u16, Error> {
        let key = match c {
            Color::Spot { name, tint, .. } if self.looks.contains_key(name) => {
                color_key(&Color::Spot { name: name.clone(), tint: *tint, look: self.looks.get(name).copied() })
            }
            _ => color_key(c),
        };
        if let Color::Spot { name, look, .. } = c {
            self.unnamed_spots |= unnamed(name);
            self.long_names |= shortened(name);
            // The reader takes the tint back out of the stored color; at a low tint the 8-bit
            // values do not hold enough of the look for that.
            let opens = |k: &ColorKey| {
                let [b, g, r] = [k.values[0], k.values[1], k.values[2]].map(|v| f32::from(v) / 255.0);
                crate::records::untinted([r, g, b], f32::from(k.values[3]) / 100.0)
            };
            // More than 2/255 apart in some value, with room for the reader's rounding.
            let lost = |opened: &[f32; 3], want: [f32; 3]| opened.iter().zip(want).any(|(a, w)| (a - w.clamp(0.0, 1.0)).abs() > 2.0 / 255.0 + 1e-6);
            let back = opens(&key);
            let ink = self.looks.get(name).copied().or(*look).unwrap_or([0.0; 3]);
            self.zero_tints |= key.values[3] == 0 || lost(&back, ink);
            self.unknown_looks |= look.is_none() && !self.looks.contains_key(name);
            // A use with a look of its own: the ink's look written in its place changed it when the
            // use opens off what its own look would open as, or off its own look where its own
            // look would not. Otherwise the tint alone keeps its own look from opening again.
            if let Some(own) = look {
                let back_own = if self.looks.get(name) == Some(own) { back } else { opens(&color_key(c)) };
                if lost(&back, back_own) || (lost(&back, *own) && !lost(&back_own, *own)) {
                    self.merged_looks = true;
                } else {
                    self.zero_tints |= lost(&back_own, *own);
                }
            }
        }
        if let Some(name) = key.name.as_ref().filter(|_| !self.tables.colors.index.contains_key(&key)) {
            self.table_bytes = self.table_bytes.saturating_add(2 * utf16_prefix(name).len());
            self.check_bytes(0)?;
        }
        self.tables.colors.index(key)
    }

    /// A `page` chunk: begin page `n` (from 0), its shown layers back to front, end the page (R60).
    fn page(&mut self, n: usize, size: Option<(f64, f64)>, layers: &[Layer]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        self.bounds = None;
        // The model holds layers and objects front to back; the file lists them back to front.
        let mut written = 0;
        for layer in layers.iter().rev() {
            // A desktop layer is written as a drawing layer, which is how the reader takes it.
            self.left_out |= !layer.guides.is_empty() || layer.flags.locked || (layer.kind == LayerKind::Desktop && !layer.objects.is_empty());
            // The reader leaves out a layer that holds nothing.
            self.left_out |= layer.objects.is_empty();
            // Guide and grid layers hold construction art, which a reader takes as drawing art.
            if layer.flags.hidden || matches!(layer.kind, LayerKind::Guides | LayerKind::Grid) {
                self.left_out = true;
                continue;
            }
            // The reader also stops a layer's name at its first NUL and takes a blank name as none.
            let name = &layer.name;
            self.long_names |= shortened(name) || name.contains('\0') || (!name.is_empty() && name.trim().is_empty());
            out.extend(instr(BEGIN_LAYER, &layer_fields(n, written, &layer.name)?));
            written += 1;
            self.objects(&layer.objects, &mut out)?;
            out.extend(instr(END_LAYER, &[]));
            self.read_cost.chunks += 2;
            self.read_cost.check(&self.limits)?;
            self.check_bytes(out.len())?;
        }
        out.extend(instr(END_PAGE, &[]));
        // The begin and end page instructions.
        self.read_cost.chunks += 2;
        self.read_cost.check(&self.limits)?;
        let begin = begin_page(n, size, self.bounds.take())?;
        if u32::try_from(begin.len() + out.len()).is_err() {
            return Err(Error::Limit("chunk size"));
        }
        let mut page = begin;
        page.extend(out);
        let c = chunk(b"page", &page)?;
        self.check_bytes(c.len())?;
        self.file_bytes += c.len();
        Ok(c)
    }

    /// An [`Error::Limit`] when the file would be longer than the input size of the limits with
    /// `pending` more bytes.
    fn check_bytes(&self, pending: usize) -> Result<(), Error> {
        if self.file_bytes.saturating_add(self.table_bytes).saturating_add(pending) > self.limits.input {
            Err(Error::Limit("input size"))
        } else {
            Ok(())
        }
    }

    /// The shown `objects` (front to back) in the order they are drawn. Groups are walked with a
    /// stack, not by recursion.
    fn objects(&mut self, objects: &[Object], out: &mut Vec<u8>) -> Result<(), Error> {
        // Each entry: the objects of a list still to write, back to front, whether a group ends
        // after them, and a clipping container's outline drawn over them.
        type Level<'a> = (std::iter::Rev<std::slice::Iter<'a, Object>>, bool, Option<Shape>);
        let mut stack: Vec<Level<'_>> = vec![(objects.iter().rev(), false, None)];
        while let Some((iter, _, _)) = stack.last_mut() {
            // A page chunk holds at most what its 32-bit size counts.
            if u32::try_from(out.len()).is_err() {
                return Err(Error::Limit("chunk size"));
            }
            self.check_bytes(out.len())?;
            let Some(o) = iter.next() else {
                if let Some((_, ends, over)) = stack.pop() {
                    if let Some(s) = over {
                        self.shape(&s, out)?;
                    }
                    if ends {
                        self.end_group(out);
                    }
                }
                continue;
            };
            match o {
                // A hidden clipping container hides what it clips too.
                Object::Shape(s) | Object::Clip { container: s, .. } if s.flags.hidden => self.left_out = true,
                Object::Group { flags, .. } if flags.hidden => self.left_out = true,
                Object::Shape(s) => self.shape(s, out)?,
                Object::Group { objects, flags } => {
                    self.left_out |= flags.locked;
                    self.begin_group(out)?;
                    stack.push((objects.iter().rev(), true, None));
                }
                Object::Clip { container, objects } => {
                    self.left_out = true;
                    self.begin_group(out)?;
                    // The container's fill is drawn behind the objects it clips and its outline
                    // over them; an outline behind a fill is drawn behind them too, and an
                    // outline with no fill to be behind is drawn over them. A container with
                    // neither draws nothing and is not written.
                    let mut over = None;
                    match (&container.fill, &container.outline) {
                        (None, None) => {}
                        (None, Some(_)) => over = Some((**container).clone()),
                        (Some(_), Some(o)) if !o.behind_fill => {
                            self.shape(&Shape { outline: None, ..(**container).clone() }, out)?;
                            over = Some(Shape { fill: None, ..(**container).clone() });
                        }
                        _ => self.shape(container, out)?,
                    }
                    stack.push((objects.iter().rev(), true, over));
                }
            }
        }
        Ok(())
    }

    /// Begin a group. A group inside as many groups as the reader opens (the depth of the
    /// [`crate::Limits`] written for) is an [`Error::Limit`]: the reader leaves out what it holds.
    fn begin_group(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
        if self.open_groups >= self.limits.depth {
            return Err(Error::Limit("nesting depth"));
        }
        self.open_groups += 1;
        out.extend(group());
        self.read_cost.chunks += 1;
        self.read_cost.objects += 1;
        self.read_cost.check(&self.limits)
    }

    fn end_group(&mut self, out: &mut Vec<u8>) {
        self.open_groups = self.open_groups.saturating_sub(1);
        out.extend(instr(END_GROUP, &[]));
        self.read_cost.chunks += 1;
    }

    /// A shape as one curve, or as a group of curves when its path holds more than
    /// [`MAX_CURVE_POINTS`] points.
    fn shape(&mut self, s: &Shape, out: &mut Vec<u8>) -> Result<(), Error> {
        self.left_out |= s.opacity < 1.0 || s.blend != Blend::Normal || s.flags.locked;
        // A path's nodes are at least as many as its segments and starts; a path past what the
        // limits leave is refused before its points are made.
        if let Geometry::Path(p) = &s.geometry {
            let nodes = p.subpaths.iter().filter(|sp| !sp.segments.is_empty()).fold(0usize, |n, sp| {
                sp.segments.iter().fold(n.saturating_add(1), |n, seg| n.saturating_add(if matches!(seg, Segment::Line(_)) { 1 } else { 3 }))
            });
            // A path split into several curves is written once for its fill and once for its
            // outline.
            let passes = if s.fill.is_some() && s.outline.is_some() && nodes > MAX_CURVE_POINTS { 2 } else { 1 };
            let nodes = nodes.saturating_mul(passes);
            if self.read_cost.coordinates.saturating_add(nodes) > self.limits.coordinates {
                return Err(Error::Limit("coordinates"));
            }
            // A path with segments takes at least one object and one instruction.
            if nodes > 0 {
                let cost = ReadCost { chunks: self.read_cost.chunks + 1, objects: self.read_cost.objects + 1, ..ReadCost::default() };
                cost.check(&self.limits)?;
            }
            self.check_bytes(out.len().saturating_add(nodes.saturating_mul(9)))?;
        }
        let points = points_of(&s.geometry, &s.matrix)?;
        let curves = split(&points);
        // A path with no segments draws nothing, and the file has no curve for it.
        if curves.is_empty() {
            self.left_out = true;
            return Ok(());
        }
        // A shape written as several curves paints all its fill and then all its outline (the
        // other way round for an outline behind the fill); one curve carries both.
        let passes: Vec<(Option<&Color>, Option<&Outline>)> = match (&s.fill, &s.outline) {
            (Some(f), Some(o)) if curves.len() > 1 && o.behind_fill => vec![(None, Some(o)), (Some(f), None)],
            (Some(f), Some(o)) if curves.len() > 1 => vec![(Some(f), None), (None, Some(o))],
            (f, o) => vec![(f.as_ref(), o.as_ref())],
        };
        let mut attrs = Vec::with_capacity(passes.len());
        for (f, o) in passes {
            attrs.push(self.attributes(f, o, &s.matrix)?);
        }
        if !self.nonzero_fills
            && s.nonzero
            && s.fill.is_some()
            && let Geometry::Path(p) = &s.geometry
        {
            self.nonzero_fills = may_fill_differently(p, &mut self.crossing_budget);
        }
        self.cut_subpaths |= (s.fill.is_some() || s.outline.is_some()) && points.iter().any(|sp| sp.len() > MAX_CURVE_POINTS);
        if curves.len() > 1 {
            self.split_paths = true;
            self.begin_group(out)?;
        }
        // Each pass writes every curve, and the reader takes the outline's dashes again for every
        // curve that names it.
        let points_count = curves.iter().fold(0usize, |n, c| n.saturating_add(c.len()));
        for (_, dashes) in &attrs {
            let cost = &mut self.read_cost;
            cost.chunks = cost.chunks.saturating_add(curves.len());
            cost.objects = cost.objects.saturating_add(curves.len());
            cost.coordinates = cost.coordinates.saturating_add(points_count);
            cost.dashes = cost.dashes.saturating_add(dashes.saturating_mul(curves.len()));
        }
        self.read_cost.check(&self.limits)?;
        // Each curve instruction holds at least 9 bytes for each point.
        self.check_bytes(out.len().saturating_add(points_count.saturating_mul(9).saturating_mul(attrs.len())))?;
        let mut bodies = Vec::with_capacity(curves.len());
        for c in &curves {
            let mut g = (c.len() as u16).to_le_bytes().to_vec();
            // The points as written, in coordinate units.
            let mut written = Vec::with_capacity(c.len());
            for (p, t) in c.iter() {
                let (x, y) = (units(p.x)?, units(p.y)?);
                g.extend(x.to_le_bytes());
                g.extend(y.to_le_bytes());
                written.push((Pt { x: f64::from(x), y: f64::from(y) }, *t));
            }
            // The page's art edges take in the written curve itself: its anchors and each cubic's
            // extremes, rounded outward.
            for (i, (p, t)) in written.iter().enumerate() {
                let [x0, y0, x1, y1] = match (t >> 6, i.checked_sub(3).and_then(|s| written.get(s..i))) {
                    (2, Some([(a, _), (b, _), (d, _)])) => cubic_extent(*a, *b, *d, *p),
                    (3, _) => continue,
                    _ => [p.x, p.y, p.x, p.y],
                };
                // Within the anchors and control points, which are i32, so the casts keep them.
                let (x0, y0, x1, y1) = (x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32);
                let b = self.bounds.get_or_insert([x0, y0, x1, y1]);
                *b = [b[0].min(x0), b[1].min(y0), b[2].max(x1), b[3].max(y1)];
            }
            g.extend(c.iter().map(|(_, t)| *t));
            bodies.push(field(2, &g).ok_or(Error::Limit("curve size"))?);
        }
        for (a, _) in &attrs {
            let f1 = field(1, a).ok_or(Error::Limit("rendering attributes size"))?;
            for f2 in &bodies {
                let i = instr(CURVE, &fields(&[f1.clone(), f2.clone()]));
                self.check_bytes(out.len().saturating_add(i.len()))?;
                out.extend(i);
            }
        }
        if curves.len() > 1 {
            self.end_group(out);
        }
        Ok(())
    }

    /// Rendering attributes (R63): the mask, then the uniform fill's field list (fill type 1 and a
    /// field 1 of the color entry and screen entry 1, R74) and the outline's (its `rotl` entry),
    /// with the number of dash lengths the outline names.
    fn attributes(&mut self, fill: Option<&Color>, outline: Option<&Outline>, matrix: &Matrix) -> Result<(Vec<u8>, usize), Error> {
        let mask = u8::from(fill.is_some()) | (u8::from(outline.is_some()) << 1);
        let mut a = vec![mask];
        let mut dashes = 0;
        if let Some(c) = fill {
            let index = self.color(c)?;
            let mut v = 1u16.to_le_bytes().to_vec();
            v.extend(fields(&[field(1, &[index.to_le_bytes(), 1u16.to_le_bytes()].concat()).ok_or(Error::Limit("fill size"))?]));
            a.extend(fields(&[field(1, &v).ok_or(Error::Limit("fill size"))?]));
        }
        if let Some(o) = outline {
            // The points are written placed by the matrix, so an outline that scales with the
            // shape takes the matrix's scale into its width, as the engine applies it (R50).
            let [m0, m1, m2, m3, _, _] = matrix.0;
            let scale = if o.scale_with { (m0 * m3 - m1 * m2).abs().sqrt() } else { 1.0 };
            let (index, n) = self.outline(o, scale)?;
            dashes = n;
            a.extend(fields(&[field(1, &index.to_le_bytes()).ok_or(Error::Limit("outline size"))?]));
        }
        Ok((a, dashes))
    }

    /// The `rotl` entry of an outline (R57, R58): its line type (2 solid, 4 dashed, 8 behind the
    /// fill, 16 scaled with the object; the caps in the low 4 bits and the join in the high 4
    /// bits), color, pen and dash pattern, and the number of dash lengths stored. `scale`
    /// multiplies the width.
    fn outline(&mut self, o: &Outline, scale: f64) -> Result<(u16, usize), Error> {
        // A pattern with no positive length draws a solid line (R57).
        let lengths: &[f64] = if !o.dashes.iter().any(|d| *d > 0.0) { &[] } else { &o.dashes };
        // A length that is not a number is stored as 0.
        self.rounded_dashes |= o.dashes.iter().any(|d| d.is_nan());
        // One table entry holds a u16 count and the lengths in a field of at most 65,535 bytes.
        if lengths.len() > (usize::from(u16::MAX) - 3 - 2) / 2 {
            return Err(Error::Limit("table entry size"));
        }
        let mut dashes: Vec<u16> = lengths.iter().map(|d| d.max(0.0).round().min(f64::from(u16::MAX)) as u16).collect();
        if lengths.iter().zip(&dashes).any(|(d, r)| *d != f64::from(*r)) {
            self.rounded_dashes = true;
        }
        // Marks (the even entries) that all round to nothing would draw nothing, and lengths that
        // all round to nothing would draw a solid line: the positive ones are kept as short as the
        // table allows (gaps only in the second case).
        let all_zero = dashes.iter().all(|d| *d == 0);
        let marks_gone = dashes.iter().step_by(2).all(|d| *d == 0) && lengths.iter().step_by(2).any(|d| *d > 0.0);
        if !dashes.is_empty() && (all_zero || marks_gone) {
            for (i, (r, d)) in dashes.iter_mut().zip(lengths).enumerate() {
                if *r == 0 && *d > 0.0 && (all_zero || i % 2 == 0) {
                    *r = 1;
                }
            }
            self.rounded_dashes = true;
        }
        let dashed = !dashes.is_empty();
        let flags = 2 | if dashed { 4 } else { 0 } | if o.behind_fill { 8 } else { 0 } | if o.scale_with { 16 } else { 0 };
        let cap = match o.cap {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
        };
        let join = match o.join {
            Join::Miter => 0,
            Join::Round => 1,
            Join::Bevel => 2,
        };
        let line = self.tables.line_types.index((flags, cap | (join << 4)))?;
        let color = self.color(&o.color)?;
        // A pen of width 0 reads as a hairline, so a thinner positive width is written as one unit.
        if o.width.is_nan() || o.width < 0.0 {
            return Err(Error::Limit("outline width"));
        }
        let width = o.width * scale;
        if width.is_nan() {
            return Err(Error::Limit("outline width"));
        }
        let pen = self.tables.pens.index(if width > 0.0 { width_units(width)?.max(1) } else { 0 })?;
        // A solid outline names an empty pattern, as the sample files' solid outlines do (R58).
        let stored = dashes.len();
        if !self.tables.dashes.index.contains_key(&dashes) {
            self.table_bytes = self.table_bytes.saturating_add(2 * stored);
            self.check_bytes(0)?;
        }
        let dash = self.tables.dashes.index(dashes)?;
        Ok((self.tables.outlines.index([line, color, pen, dash])?, stored))
    }
}

/// Begin page `n` (from 0; R61): field 1 the page number from 1, a u32 of flags and the left,
/// top, right and bottom edges of `bounds`, the page's art (smallest x and y, largest x and y),
/// or of the page around its center when it has no art; field 4 its width and height. Without a
/// size the edges are all 0, which the reader takes as no size.
fn begin_page(n: usize, size: Option<(f64, f64)>, bounds: Option<[i32; 4]>) -> Result<Vec<u8>, Error> {
    let number = u16::try_from(n + 1).map_err(|_| Error::Limit("pages"))?;
    let mut d = number.to_le_bytes().to_vec();
    d.extend(0u32.to_le_bytes());
    let mut list = Vec::new();
    let mut size_field = None;
    let mut edges = [0i32; 4];
    if let Some((w, h)) = size {
        // A negative or NaN side counts as 0.
        let (wu, hu) = (width_units(w)?, width_units(h)?);
        // A side that rounds to nothing would make the reader drop the page's whole size.
        if wu == 0 || hu == 0 {
            return Err(Error::Limit("page size"));
        }
        edges = match bounds {
            Some([x0, y0, x1, y1]) => [x0, y1, x1, y0],
            None => {
                // The page around its center; an odd size keeps its extra unit on the left and
                // bottom, where the edge reaches -2^31.
                let edge = |v: i64| i32::try_from(v).map_err(|_| Error::Limit("page size"));
                let (w, h) = (i64::from(wu), i64::from(hu));
                [edge(w / 2 - w)?, edge(h / 2)?, edge(w / 2)?, edge(h / 2 - h)?]
            }
        };
        let mut s = wu.to_le_bytes().to_vec();
        s.extend(hu.to_le_bytes());
        size_field = Some(s);
    }
    for v in edges {
        d.extend(v.to_le_bytes());
    }
    list.push(field(1, &d).ok_or(Error::Limit("page size"))?);
    if let Some(s) = size_field {
        list.push(field(4, &s).ok_or(Error::Limit("page size"))?);
    }
    Ok(instr(BEGIN_PAGE, &fields(&list)))
}

/// `c` as a Windows-1252 byte, or `?` when the code page has no byte for it.
fn windows_1252_byte(c: char) -> u8 {
    match u8::try_from(u32::from(c)) {
        Ok(b) if !(0x80..0xa0).contains(&b) => b,
        _ => crate::records::CP1252_HIGH.iter().position(|h| *h == c && c != '\u{FFFD}').and_then(|i| u8::try_from(0x80 + i).ok()).unwrap_or(b'?'),
    }
}

/// A layer's fields (R62): field 1 the page number and the layer number (u16 each, from 1), 8 zero
/// bytes, then the name as a u16 count and 8-bit characters (Windows-1252: characters the code
/// page has no byte for are written as `?`); field 4 the name as a u32 count and UTF-16 units.
/// `page` and `n` count the pages and the page's layers written from 0; past 65,535 layers is an
/// [`Error::Limit`].
fn layer_fields(page: usize, n: usize, name: &str) -> Result<Vec<u8>, Error> {
    // A blank name, up to the first NUL where the reader stops, is written empty in both fields,
    // so it opens with no name.
    let blank = name.split('\0').next().unwrap_or_default().trim().is_empty();
    let narrow: Vec<u8> = if blank { Vec::new() } else { name.chars().map(windows_1252_byte).take(MAX_NAME_BYTES).collect() };
    let page = u16::try_from(page + 1).map_err(|_| Error::Limit("pages"))?;
    let number = u16::try_from(n + 1).map_err(|_| Error::Limit("layers"))?;
    let mut d = page.to_le_bytes().to_vec();
    d.extend(number.to_le_bytes());
    d.extend([0; 8]);
    d.extend((narrow.len() as u16).to_le_bytes());
    d.extend(&narrow);
    let units = if blank { Vec::new() } else { utf16_prefix(name) };
    let mut wide = (units.len() as u32).to_le_bytes().to_vec();
    wide.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    let f1 = field(1, &d).ok_or(Error::Limit("layer name"))?;
    let f4 = field(4, &wide).ok_or(Error::Limit("layer name"))?;
    Ok(fields(&[f1, f4]))
}

/// Begin a group: field 1 of 26 zero bytes, as the test files of `synth_cmx` hold.
fn group() -> Vec<u8> {
    instr(BEGIN_GROUP, &fields(&[field(1, &[0; 26]).unwrap_or_default()]))
}

/// A subpath's points and their type bytes (R16), placed by `m`.
type Points = Vec<(Pt, u8)>;

/// The subpaths of a geometry as points and type bytes, one list for each subpath. A closed
/// subpath ends back at its start, on a node that carries the closing bit.
fn points_of(g: &Geometry, m: &Matrix) -> Result<Vec<Points>, Error> {
    let (path, m) = match g {
        Geometry::Path(p) => (std::borrow::Cow::Borrowed(p), Matrix(m.0)),
        Geometry::Rect { w, h, radii } => {
            let (w, h, radii, m) = fold_scale(*w, *h, *radii, m);
            (std::borrow::Cow::Owned(rect_path(w, h, radii)), m)
        }
        Geometry::Ellipse { w, h } => (std::borrow::Cow::Owned(ellipse_path(*w, *h)), Matrix(m.0)),
    };
    path.subpaths.iter().filter(|s| !s.segments.is_empty()).map(|s| subpath_points(s, &m)).collect()
}

/// A rectangle placed by `m` with the scale of `m` taken into its size, when the matrix keeps right
/// angles: the width and height times the matrix's scale along each, the radii times the square
/// root of its area scale, and the matrix left with its rotation, mirroring and offset. The
/// corners then stay round. Other matrices are kept whole.
fn fold_scale(w: f64, h: f64, radii: [f64; 4], m: &Matrix) -> (f64, f64, [f64; 4], Matrix) {
    let [m0, m1, m2, m3, m4, m5] = m.0;
    let (sx, sy) = (m0.hypot(m1), m2.hypot(m3));
    let square = sx > 1e-12 && sy > 1e-12 && (m0 * m2 + m1 * m3).abs() <= 1e-9 * sx * sy;
    if !square || !m.0.iter().all(|c| c.is_finite()) {
        return (w, h, radii, Matrix(m.0));
    }
    let k = (sx * sy).sqrt();
    (w * sx, h * sy, radii.map(|r| r * k), Matrix([m0 / sx, m1 / sx, m2 / sy, m3 / sy, m4, m5]))
}

/// A subpath's points; more than memory holds is an [`Error::Limit`].
fn subpath_points(s: &SubPath, m: &Matrix) -> Result<Points, Error> {
    let size = |segs: &[Segment]| segs.iter().fold(1usize, |n, seg| n.saturating_add(if matches!(seg, Segment::Line(_)) { 1 } else { 3 }));
    // The closing bit on the last node closes a subpath that fits in one curve. One that is cut
    // into open pieces ends with a line back to its start instead.
    let open_end = s.closed && last_point(&s.segments).is_some_and(|p| p != s.start);
    let closing = (open_end && size(&s.segments) > MAX_CURVE_POINTS).then_some(Segment::Line(s.start));
    let count = size(&s.segments).saturating_add(usize::from(closing.is_some()));
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| Error::Limit("path size"))?;
    out.push((m.apply(s.start), if s.closed { MOVE | CLOSES } else { MOVE }));
    for seg in s.segments.iter().chain(closing.as_ref()) {
        match seg {
            Segment::Line(p) => out.push((m.apply(*p), LINE)),
            Segment::Cubic(c1, c2, p) => {
                out.push((m.apply(*c1), CONTROL));
                out.push((m.apply(*c2), CONTROL));
                out.push((m.apply(*p), CURVE_END));
            }
        }
    }
    if s.closed
        && let Some(end) = out.last_mut()
    {
        end.1 |= CLOSES;
    }
    Ok(out)
}

fn last_point(segments: &[Segment]) -> Option<Pt> {
    segments.last().map(|s| match s {
        Segment::Line(p) | Segment::Cubic(_, _, p) => *p,
    })
}

/// The subpaths gathered into curves of at most [`MAX_CURVE_POINTS`] points, whole subpaths kept
/// together where they fit. A subpath longer than that is cut into open pieces that meet end to
/// start, each piece cut after a node.
fn split(subpaths: &[Points]) -> Vec<Points> {
    let mut curves: Vec<Points> = vec![Vec::new()];
    for sp in subpaths {
        if sp.len() > MAX_CURVE_POINTS {
            let mut rest: &[(Pt, u8)] = sp;
            while rest.len() > 1 {
                // Cut after a node, never between a curve's control points and its end.
                let mut take = rest.len().min(MAX_CURVE_POINTS);
                while take > 1 && rest.get(take - 1).is_some_and(|(_, t)| *t == CONTROL) {
                    take -= 1;
                }
                if take < 2 {
                    break;
                }
                let piece: Points = rest[..take].iter().enumerate().map(|(i, (p, t))| if i == 0 { (*p, MOVE) } else { (*p, *t & !CLOSES) }).collect();
                if curves.last().is_some_and(|c| !c.is_empty()) {
                    curves.push(Vec::new());
                }
                curves.push(piece);
                rest = &rest[take - 1..];
            }
            continue;
        }
        if curves.last().is_some_and(|c| c.len() + sp.len() > MAX_CURVE_POINTS) {
            curves.push(Vec::new());
        }
        if let Some(c) = curves.last_mut() {
            c.extend_from_slice(sp);
        }
    }
    curves.retain(|c| !c.is_empty());
    curves
}

/// The lines each cubic is flattened to for [`may_fill_differently`].
const CURVE_STEPS: u32 = 16;

/// The pairs of edges and of subpath boxes [`may_fill_differently`] may compare in one export;
/// past it a shape is taken to fill differently.
const CROSSING_BUDGET: u64 = 50_000_000;

/// The pairs among `n` items, saturating.
fn pairs(n: u64) -> u64 {
    n.saturating_mul(n.saturating_sub(1)) / 2
}

/// Take `cost` comparisons off `budget`; false, and the budget emptied, when it does not cover
/// them.
fn charge(budget: &mut u64, cost: u64) -> bool {
    match budget.checked_sub(cost) {
        Some(left) => {
            *budget = left;
            true
        }
        None => {
            *budget = 0;
            false
        }
    }
}

/// The closed polygon of a subpath with each cubic flattened to [`CURVE_STEPS`] lines, without
/// repeated points, and for each edge (from a point to the next, and from the last point to the
/// first) how far the subpath may stray from that edge: 0 along a line, and along a piece of a
/// cubic `h² / 8` times the cubic's largest second derivative, `h` being the piece's share of the
/// cubic's parameter.
fn flatten(s: &SubPath) -> (Vec<Pt>, Vec<f64>) {
    // Each point with how far the subpath may stray from the edge that ends at it.
    let mut pts = vec![(s.start, 0.0)];
    let mut last = s.start;
    for seg in &s.segments {
        match *seg {
            Segment::Line(p) => {
                pts.push((p, 0.0));
                last = p;
            }
            Segment::Cubic(c1, c2, p) => {
                // The second derivative, 6 ((1 − t) (p0 − 2 c1 + c2) + t (c1 − 2 c2 + p3)), is at its
                // largest at an end.
                let second = |a: Pt, b: Pt, c: Pt| (a.x - 2.0 * b.x + c.x).hypot(a.y - 2.0 * b.y + c.y);
                let steps = f64::from(CURVE_STEPS);
                let stray = 6.0 * second(last, c1, c2).max(second(c1, c2, p)) / (8.0 * steps * steps);
                for i in 1..=CURVE_STEPS {
                    let t = f64::from(i) / steps;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    let q = Pt { x: a * last.x + b * c1.x + c * c2.x + d * p.x, y: a * last.y + b * c1.y + c * c2.y + d * p.y };
                    pts.push((q, stray));
                }
                last = p;
            }
        }
    }
    // A repeated point is dropped; the subpath strays no further than that from the point, which
    // is on the edge that ends at it.
    let mut kept: Vec<(Pt, f64)> = Vec::with_capacity(pts.len());
    for (q, stray) in pts {
        match kept.last_mut() {
            Some((prev, e)) if *prev == q => *e = e.max(stray),
            _ => kept.push((q, stray)),
        }
    }
    // A last point back on the first is dropped, and its edge becomes the edge back to the first.
    if kept.len() > 1 && kept.first().map(|f| f.0) == kept.last().map(|l| l.0) {
        let back = kept.pop().map_or(0.0, |(_, e)| e);
        if let Some(first) = kept.first_mut() {
            first.1 = first.1.max(back);
        }
    }
    let (points, ends): (Vec<Pt>, Vec<f64>) = kept.into_iter().unzip();
    // Edge k runs from point k to point k + 1.
    let n = ends.len();
    let strays = (0..n).map(|k| ends[(k + 1) % n]).collect();
    (points, strays)
}

/// Whether `p` may fill differently even-odd than nonzero: one of its cubics loops, a subpath
/// crosses itself or turns around other than once, measured with the cubics flattened (two edges
/// that are not neighbors count as crossing when they are closer than the cubics may stray from
/// their lines, and two neighbors when the curves along them may meet away from their shared
/// point), or two of its subpaths turn the same way and their boxes, control points
/// included, overlap. Subpaths
/// that turn opposite ways, such as a letter's counter inside the letter, and subpaths apart from
/// each other, fill the same either way. `budget` counts down the comparisons, and is charged for
/// a subpath's edges before the subpath is flattened.
fn may_fill_differently(p: &Path, budget: &mut u64) -> bool {
    let n = p.subpaths.len();
    // The boxes of every two subpaths are compared.
    if !charge(budget, pairs(n as u64)) {
        return true;
    }
    let mut polygons = Vec::with_capacity(n);
    for s in &p.subpaths {
        let points =
            s.segments.iter().fold(1u64, |k, seg| k.saturating_add(if matches!(seg, Segment::Line(_)) { 1 } else { u64::from(CURVE_STEPS) }));
        if !charge(budget, pairs(points)) {
            return true;
        }
        let mut last = s.start;
        for seg in &s.segments {
            match *seg {
                Segment::Line(p) => last = p,
                Segment::Cubic(c1, c2, p) => {
                    if cubic_loops(last, c1, c2, p) {
                        return true;
                    }
                    last = p;
                }
            }
        }
        let (pts, strays) = flatten(s);
        if loops(&pts) || crosses(&pts, &strays) || joints_meet(s, budget) {
            return true;
        }
        polygons.push(pts);
    }
    if n < 2 {
        return false;
    }
    // Each subpath's turn (`None` when its area sums to 0, which matches either way) and box.
    let shapes: Vec<(Option<bool>, [f64; 4])> = polygons
        .iter()
        .zip(&p.subpaths)
        .map(|(pts, s)| {
            // The area is summed from the first point, so far-off coordinates keep their precision.
            let o = pts.first().copied().unwrap_or(s.start);
            let area: f64 = pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| (a.x - o.x) * (b.y - o.y) - (b.x - o.x) * (a.y - o.y)).sum();
            // The box holds every point, control points included, so it holds the curves too.
            let controls = std::iter::once(s.start).chain(s.segments.iter().flat_map(|seg| match *seg {
                Segment::Line(a) => vec![a],
                Segment::Cubic(a, b, c) => vec![a, b, c],
            }));
            let bounds = controls.fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)]);
            ((area != 0.0).then_some(area > 0.0), bounds)
        })
        .collect();
    shapes.iter().enumerate().any(|(i, (turn, a))| {
        shapes[i + 1..]
            .iter()
            .any(|(other, b)| (turn.is_none() || other.is_none() || turn == other) && a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3])
    })
}

/// The smallest x and y and the largest x and y the cubic from `p0` through `c1` and `c2` to `p3`
/// reaches: its ends and the points where it turns along either axis.
fn cubic_extent(p0: Pt, c1: Pt, c2: Pt, p3: Pt) -> [f64; 4] {
    let at = |t: f64, a: f64, b: f64, c: f64, d: f64| {
        let u = 1.0 - t;
        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
    };
    // The parameters in 0 to 1 where one coordinate's derivative is 0.
    let turns = |a: f64, b: f64, c: f64, d: f64| -> Vec<f64> {
        let (qa, qb, qc) = (-a + 3.0 * b - 3.0 * c + d, 2.0 * (a - 2.0 * b + c), b - a);
        let mut ts = Vec::new();
        if qa.abs() < 1e-12 {
            if qb.abs() > 1e-12 {
                ts.push(-qc / qb);
            }
        } else {
            let disc = qb * qb - 4.0 * qa * qc;
            if disc >= 0.0 {
                // Each root from the form that adds numbers of one sign, so neither cancels.
                let q = -0.5 * (qb + qb.signum() * disc.sqrt());
                ts.push(q / qa);
                if q != 0.0 {
                    ts.push(qc / q);
                }
            }
        }
        ts.retain(|t| (0.0..=1.0).contains(t));
        ts
    };
    // An extreme between the ends is evaluated with rounding, so it is widened by a little more
    // than that rounding: the edges then hold the curve.
    let slack = |v: f64| v.abs() * 4.0 * f64::EPSILON + 1e-12;
    let mut e = [p0.x.min(p3.x), p0.y.min(p3.y), p0.x.max(p3.x), p0.y.max(p3.y)];
    for t in turns(p0.x, c1.x, c2.x, p3.x) {
        let x = at(t, p0.x, c1.x, c2.x, p3.x);
        e = [e[0].min(x - slack(x)), e[1], e[2].max(x + slack(x)), e[3]];
    }
    for t in turns(p0.y, c1.y, c2.y, p3.y) {
        let y = at(t, p0.y, c1.y, c2.y, p3.y);
        e = [e[0], e[1].min(y - slack(y)), e[2], e[3].max(y + slack(y))];
    }
    e
}

/// Whether the cubic from `p0` through `c1` and `c2` to `p3` crosses itself. With the cubic as
/// `a t³ + b t² + c t + p0`, two parameters `s` and `t` meet where `a (σ² − π) + b σ + c = 0`, with
/// `σ = s + t` and `π = s t`; the cubic loops when `s` and `t` are distinct and both in 0 to 1.
fn cubic_loops(p0: Pt, c1: Pt, c2: Pt, p3: Pt) -> bool {
    let cross = |u: (f64, f64), v: (f64, f64)| u.0 * v.1 - u.1 * v.0;
    let a = (-p0.x + 3.0 * c1.x - 3.0 * c2.x + p3.x, -p0.y + 3.0 * c1.y - 3.0 * c2.y + p3.y);
    let b = (3.0 * p0.x - 6.0 * c1.x + 3.0 * c2.x, 3.0 * p0.y - 6.0 * c1.y + 3.0 * c2.y);
    let r = (3.0 * p0.x - 3.0 * c1.x, 3.0 * p0.y - 3.0 * c1.y);
    let det = cross(a, b);
    if det == 0.0 || !det.is_finite() {
        return false;
    }
    // u = σ² − π.
    let (u, sum) = (cross(r, b) / det, cross(a, r) / det);
    let disc = 4.0 * u - 3.0 * sum * sum;
    if disc.is_nan() || disc <= 0.0 {
        return false;
    }
    let q = disc.sqrt();
    let (s, t) = ((sum - q) / 2.0, (sum + q) / 2.0);
    // The start meeting the end closes the curve; that is no crossing.
    let closes = s.abs() < 1e-9 && (t - 1.0).abs() < 1e-9;
    // Rounding can put a parameter of 0 or 1 just outside the range.
    let inside = |v: f64| (-1e-9..=1.0 + 1e-9).contains(&v);
    inside(s) && inside(t) && !closes
}

/// Whether the closed polygon through `pts` turns around other than once: the sum of its turning
/// angles is not one full turn either way.
fn loops(pts: &[Pt]) -> bool {
    let edges: Vec<(f64, f64)> =
        pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| (b.x - a.x, b.y - a.y)).filter(|(x, y)| x.hypot(*y) > 1e-9).collect();
    if edges.len() < 3 {
        return false;
    }
    let turning: f64 = edges.iter().zip(edges.iter().cycle().skip(1)).map(|(a, b)| (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1)).sum();
    (turning / std::f64::consts::TAU).round().abs() != 1.0
}

/// Whether two edges of the closed polygon through `pts` that are not neighbors cross, touch or run
/// along each other: the subpath comes back to a place it has been. `strays` holds, for each edge
/// from a point to the next, how far the subpath may stray from it; two edges closer than their
/// subpath may stray from them count as crossing.
fn crosses(pts: &[Pt], strays: &[f64]) -> bool {
    let n = pts.len();
    if n < 4 {
        return false;
    }
    // Which side of the line from `a` to `b` the point `c` is on: 1, -1, or 0 on the line.
    let side = |a: Pt, b: Pt, c: Pt| {
        let v = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        i8::from(v > 0.0) - i8::from(v < 0.0)
    };
    let edge = |i: usize| (pts[i], pts[(i + 1) % n]);
    (0..n).any(|i| {
        let (a, b) = edge(i);
        // Edge i shares a point with edges i - 1 and i + 1, and edge 0 with edge n - 1.
        (i + 2..n).filter(|j| !(i == 0 && *j == n - 1)).any(|j| {
            let (c, d) = edge(j);
            let (sc, sd) = (side(a, b, c), side(a, b, d));
            if sc == 0 && sd == 0 {
                // On one line: they meet when their spans along it meet.
                let along = |q: Pt| (q.x - a.x) * (b.x - a.x) + (q.y - a.y) * (b.y - a.y);
                let (tc, td, tb) = (along(c), along(d), along(b));
                if tc.min(td).max(0.0) <= tc.max(td).min(tb) {
                    return true;
                }
            } else if sc * sd <= 0 && side(c, d, a) * side(c, d, b) <= 0 {
                return true;
            }
            // A cubic's piece may cross an edge that its line passes close to.
            let stray = strays.get(i).copied().unwrap_or(0.0) + strays.get(j).copied().unwrap_or(0.0);
            stray > 0.0 && gap(a, b, c, d) <= stray * (1.0 + 1e-9)
        })
    })
}

/// The distance between the segment from `a` to `b` and the segment from `c` to `d`, which do not
/// cross: the shortest distance from an end of one to the other.
fn gap(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    let to = |p: Pt, a: Pt, b: Pt| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = dx * dx + dy * dy;
        let t = if len > 0.0 { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len).clamp(0.0, 1.0) } else { 0.0 };
        (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
    };
    to(a, c, d).min(to(b, c, d)).min(to(c, a, b)).min(to(d, a, b))
}

/// A segment as the start, the two control points and the end of a cubic; a line as the cubic
/// along it.
type Bez = [Pt; 4];

/// How many times [`meet_away_from_start`] halves two cubics around their shared start before
/// taking what is left beside it not to meet, and [`boxes_meet`] halves two cubics before taking
/// them to meet.
const NEAR_HALVINGS: u32 = 32;
const APART_HALVINGS: u32 = 24;

/// The point a share `t` of the way from `a` to `b`.
fn lerp(a: Pt, b: Pt, t: f64) -> Pt {
    Pt { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t }
}

/// The cubic `c` cut at `t` into the part before and the part after.
fn cut(c: Bez, t: f64) -> (Bez, Bez) {
    let (ab, bc, cd) = (lerp(c[0], c[1], t), lerp(c[1], c[2], t), lerp(c[2], c[3], t));
    let (abc, bcd) = (lerp(ab, bc, t), lerp(bc, cd, t));
    let m = lerp(abc, bcd, t);
    ([c[0], ab, abc, m], [m, bcd, cd, c[3]])
}

/// The directions from `c[0]` that the cubic `c` stays within away from that point, which its
/// control points and end bound: the angle halfway between the outermost two, and half the angle
/// between them. `None` when they span half a turn or more, or all lie on `c[0]`.
fn directions(c: &Bez) -> Option<(f64, f64)> {
    let o = c[0];
    let vs: Vec<(f64, f64)> = c[1..].iter().map(|q| (q.x - o.x, q.y - o.y)).filter(|v| v.0 != 0.0 || v.1 != 0.0).collect();
    let first = *vs.first()?;
    let angle = |v: &(f64, f64)| (first.0 * v.1 - first.1 * v.0).atan2(first.0 * v.0 + first.1 * v.1);
    let (lo, hi) = vs.iter().map(angle).fold((0.0_f64, 0.0_f64), |(lo, hi), a| (lo.min(a), hi.max(a)));
    (hi - lo < std::f64::consts::PI).then(|| (first.1.atan2(first.0) + (lo + hi) / 2.0, (hi - lo) / 2.0))
}

/// Whether the cubics `a` and `b`, which start at the same point, may meet away from it. Near that
/// point they stay within the directions their control points bound, so where those do not overlap
/// they do not meet there; the parts away from it are compared by their boxes. `halvings` counts
/// down how often the two are halved again. Two cubics that leave the point along the same tangent
/// keep overlapping directions however often they are halved; the parts left at the last halving
/// span 2⁻³² of each given cubic's parameter from the point, and are taken not to meet. `budget` counts
/// down the comparisons.
fn meet_away_from_start(a: Bez, b: Bez, halvings: u32, budget: &mut u64) -> bool {
    if !charge(budget, 1) {
        return true;
    }
    if let (Some((ca, wa)), Some((cb, wb))) = (directions(&a), directions(&b)) {
        let apart = (ca - cb).sin().atan2((ca - cb).cos()).abs();
        if apart - (wa + wb) > 1e-9 {
            return false;
        }
        // Each runs straight out along the same direction: one runs along the other.
        if wa == 0.0 && wb == 0.0 {
            return true;
        }
    }
    if halvings == 0 {
        return false;
    }
    let ((an, af), (bn, bf)) = (cut(a, 0.5), cut(b, 0.5));
    meet_away_from_start(an, bn, halvings - 1, budget)
        || [(an, bf), (af, bn), (af, bf)].into_iter().any(|(x, y)| boxes_meet(x, y, APART_HALVINGS, budget))
}

/// Whether the cubics `a` and `b` may meet: their boxes, control points included, overlap or touch
/// after `halvings` halvings of both. `budget` counts down the comparisons.
fn boxes_meet(a: Bez, b: Bez, halvings: u32, budget: &mut u64) -> bool {
    if !charge(budget, 1) {
        return true;
    }
    let bounds =
        |c: &Bez| c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |e, q| [e[0].min(q.x), e[1].min(q.y), e[2].max(q.x), e[3].max(q.y)]);
    let (p, q) = (bounds(&a), bounds(&b));
    if p[0] > q[2] || q[0] > p[2] || p[1] > q[3] || q[1] > p[3] {
        return false;
    }
    if halvings == 0 {
        return true;
    }
    let ((a0, a1), (b0, b1)) = (cut(a, 0.5), cut(b, 0.5));
    [(a0, b0), (a0, b1), (a1, b0), (a1, b1)].into_iter().any(|(x, y)| boxes_meet(x, y, halvings - 1, budget))
}

/// Whether, at a point where two segments of the subpath `s` meet (the last and the first
/// included, closing it), the subpath's pieces beside that point may meet away from it: a line
/// whole, and the first or last of a cubic's [`CURVE_STEPS`] pieces. These are the curves along
/// the neighboring edges of the flattened polygon, which [`crosses`] does not compare. Segments
/// that do not leave their start are skipped, and two lines are not compared. `budget` counts down
/// the comparisons.
fn joints_meet(s: &SubPath, budget: &mut u64) -> bool {
    let line = |a: Pt, b: Pt| [a, lerp(a, b, 1.0 / 3.0), lerp(a, b, 2.0 / 3.0), b];
    // Each segment as a cubic, and whether it is a cubic.
    let mut curves: Vec<(Bez, bool)> = Vec::with_capacity(s.segments.len() + 1);
    let mut last = s.start;
    for seg in &s.segments {
        let c = match *seg {
            Segment::Line(p) => (line(last, p), false),
            Segment::Cubic(c1, c2, p) => ([last, c1, c2, p], true),
        };
        last = c.0[3];
        if c.0.iter().any(|q| *q != c.0[0]) {
            curves.push(c);
        }
    }
    if last != s.start {
        curves.push((line(last, s.start), false));
    }
    let n = curves.len();
    if n < 2 {
        return false;
    }
    let piece = 1.0 / f64::from(CURVE_STEPS);
    (0..n).any(|k| {
        let ((a, a_cubic), (b, b_cubic)) = (curves[k], curves[(k + 1) % n]);
        // Two lines meet away from their shared point only by one running back along the other,
        // which wraps no area, so the rules fill the same.
        if !a_cubic && !b_cubic {
            return false;
        }
        // Both from the point they share.
        let a = if a_cubic { cut(a, 1.0 - piece).1 } else { a };
        let a = [a[3], a[2], a[1], a[0]];
        let b = if b_cubic { cut(b, piece).0 } else { b };
        // Turned about the shared point so that `b` leaves it along x: two cubics leaving it along
        // one tangent then have boxes thin across it, which part as soon as the cubics do.
        let o = b[0];
        let lead = b[1..].iter().map(|q| (q.x - o.x, q.y - o.y)).find(|v| v.0 != 0.0 || v.1 != 0.0).unwrap_or((1.0, 0.0));
        let (sin, cos) = lead.1.atan2(lead.0).sin_cos();
        let turn = |c: Bez| c.map(|q| Pt { x: (q.x - o.x) * cos + (q.y - o.y) * sin, y: (q.y - o.y) * cos - (q.x - o.x) * sin });
        meet_away_from_start(turn(a), turn(b), NEAR_HALVINGS, budget)
    })
}

/// A rectangle from the origin to (`w`, `h`) with its corners rounded by `radii`, counterclockwise
/// from the corner at (`w`, 0), as [`Geometry::Rect`] gives them. On each edge a corner's radius
/// reaches no further than where its neighbor's begins, unless that leaves it less than half the
/// edge.
fn rect_path(w: f64, h: f64, radii: [f64; 4]) -> Path {
    // A quarter circle as a cubic: the control points lie this share of the radius from the ends.
    const K: f64 = 0.552_284_749_830_793_6;
    // A negative or NaN radius leaves its corner square.
    let radii = radii.map(|r| if r.is_finite() { r.max(0.0) } else { 0.0 });
    // Edge i runs from corner i to corner i + 1: the height, the width, the height, the width.
    let len = |i: usize| if i.is_multiple_of(2) { h.abs() } else { w.abs() };
    // On each of its edges a corner reaches up to where its neighbor's corner begins, and at
    // least halfway.
    let room = |len: f64, other: f64| (len - other).max(len / 2.0);
    let radii: [f64; 4] = std::array::from_fn(|i| {
        let (next, prev) = ((i + 1) % 4, (i + 3) % 4);
        radii[i].min(room(len(i), radii[next])).min(room(len(prev), radii[prev]))
    });
    let corners = [Pt { x: w, y: 0.0 }, Pt { x: w, y: h }, Pt { x: 0.0, y: h }, Pt { x: 0.0, y: 0.0 }];
    // `r` along the edge from `c` toward `to`.
    let sign = |d: f64| {
        if d > 0.0 {
            1.0
        } else if d < 0.0 {
            -1.0
        } else {
            0.0
        }
    };
    let toward = |c: Pt, to: Pt, r: f64| Pt { x: c.x + sign(to.x - c.x) * r, y: c.y + sign(to.y - c.y) * r };
    let mut segments = Vec::new();
    let mut start = None;
    for (i, (&c, r)) in corners.iter().zip(radii).enumerate() {
        let (prev, next) = (corners[(i + 3) % 4], corners[(i + 1) % 4]);
        // The corner is entered along the edge from the previous corner and left toward the next.
        let (a, b) = (toward(c, prev, r), toward(c, next, r));
        match start {
            None => start = Some(a),
            Some(_) => segments.push(Segment::Line(a)),
        }
        if r > 0.0 {
            let c1 = Pt { x: a.x + (c.x - a.x) * K, y: a.y + (c.y - a.y) * K };
            let c2 = Pt { x: b.x + (c.x - b.x) * K, y: b.y + (c.y - b.y) * K };
            segments.push(Segment::Cubic(c1, c2, b));
        }
    }
    let start = start.unwrap_or(corners[0]);
    segments.push(Segment::Line(start));
    Path { subpaths: vec![SubPath { start, segments, closed: true }] }
}

/// The ellipse in the box from the origin to (`w`, `h`) as four cubics.
fn ellipse_path(w: f64, h: f64) -> Path {
    const K: f64 = 0.552_284_749_830_793_6;
    let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
    let p = |x: f64, y: f64| Pt { x: cx + x, y: cy + y };
    let start = p(rx, 0.0);
    let segments = vec![
        Segment::Cubic(p(rx, ry * K), p(rx * K, ry), p(0.0, ry)),
        Segment::Cubic(p(-rx * K, ry), p(-rx, ry * K), p(-rx, 0.0)),
        Segment::Cubic(p(-rx, -ry * K), p(-rx * K, -ry), p(0.0, -ry)),
        Segment::Cubic(p(rx * K, -ry), p(rx, -ry * K), start),
    ];
    Path { subpaths: vec![SubPath { start, segments, closed: true }] }
}
